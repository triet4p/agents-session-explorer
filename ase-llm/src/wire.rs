//! Typed wire protocols for the five configured API kinds.
//!
//! This module only builds one request body and parses one response envelope.
//! It never opens a socket, never sees the credential value (the transport adds
//! the auth header from `ApiKey`), and every rejection is a stable code that
//! never echoes the prompt, the response body or the key.
//!
//! Wire rules kept here:
//!
//! - `chat-completions` sends `reasoning_effort` only when the profile has an
//!   explicit effort; `responses` sends `reasoning.effort` the same way;
//! - native Gemini generation and both decision kinds never carry an OpenAI
//!   effort field, and a configured effort is rejected instead of dropped;
//! - Console System One uses `state`, native Gemini Decisions uses `input`;
//!   both share the same typed `choice` question and answer envelope;
//! - auth location is protocol-defined: Bearer for OpenAI-compatible kinds and
//!   Console System One, `x-goog-api-key` for the two native Gemini kinds.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use crate::api::ApiError;
use crate::config::ApiKind;
use crate::url::Endpoint;

/// Version of the request/parse framing. Part of every API cache identity.
pub const WIRE_VERSION: &str = "ase-llm-wire/v1";
/// Version of the typed response mapping.
pub const PARSE_VERSION: &str = "ase-llm-parse/v1";
/// Stable prompt-cache/session header for the Console gateway.
pub const SESSION_HEADER: &str = "x-opencode-session";

/// Where a protocol expects its credential. The value is added by the
/// transport and is never part of a `WireRequest`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthStyle {
    Bearer,
    GoogleApiKey,
}

impl AuthStyle {
    pub fn for_api(api: ApiKind) -> Self {
        match api {
            ApiKind::Gemini | ApiKind::GeminiDecision => AuthStyle::GoogleApiKey,
            _ => AuthStyle::Bearer,
        }
    }

    pub fn header_name(self) -> &'static str {
        match self {
            AuthStyle::Bearer => "authorization",
            AuthStyle::GoogleApiKey => "x-goog-api-key",
        }
    }

    pub fn scheme(self) -> &'static str {
        match self {
            AuthStyle::Bearer => "bearer",
            AuthStyle::GoogleApiKey => "api-key",
        }
    }

    pub(crate) fn header_value(self, key: &str) -> String {
        match self {
            AuthStyle::Bearer => format!("Bearer {key}"),
            AuthStyle::GoogleApiKey => key.to_string(),
        }
    }
}

/// One protocol call. `instruction` is rubric text and `input` is the rendered
/// decision pair (or a fixed synthetic payload for the smoke path).
pub struct CallSpec<'a> {
    pub api: ApiKind,
    pub model: &'a str,
    pub effort: Option<&'a str>,
    pub instruction: &'a str,
    pub input: &'a str,
    pub question_id: &'a str,
    pub choices: &'a [(&'a str, &'a str)],
}

impl CallSpec<'_> {
    pub fn effort_sent(&self) -> bool {
        self.api.supports_openai_effort() && self.effort.is_some()
    }
}

/// A fully built request. The auth header is deliberately absent: it is added
/// from `ApiKey` at send time so the secret cannot reach logs or reports.
pub struct WireRequest {
    pub url: String,
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub path_and_query: String,
    pub api: ApiKind,
    pub auth: AuthStyle,
    /// Nonsecret headers only.
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
    pub body_keys: Vec<String>,
    pub effort_sent: bool,
    pub session_sent: bool,
}

/// One parsed answer. Generation protocols return text; the two decision
/// protocols return a typed choice with a probability.
pub enum WireAnswer {
    Text(String),
    Choice {
        choice: String,
        /// Calibrated probability reported by the decision API. This is a
        /// different semantic from a generation model's verbal self-report.
        confidence: f64,
        probabilities: BTreeMap<String, f64>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

pub struct WireResponse {
    pub answer: WireAnswer,
    pub usage: Usage,
    pub model: Option<String>,
}

/// The Console gateway routes sessions through a stable header. The documented
/// production host is `opencode.ai`; the same wire path on a loopback endpoint
/// is recognized so a synthetic transport check exercises the real header
/// behavior. Native provider APIs and any other host do not receive it.
pub fn is_console_gateway(endpoint: &Endpoint) -> bool {
    endpoint.path_and_query.starts_with("/inference/")
        && (endpoint.host == "opencode.ai" || endpoint.is_loopback())
}

fn question_id_valid(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn instruction_text<'a>(spec: &CallSpec<'a>) -> &'a str {
    if spec.instruction.trim().is_empty() {
        "Decide the requested label from the supplied input."
    } else {
        spec.instruction
    }
}

fn criteria(spec: &CallSpec<'_>) -> Result<Map<String, Value>, ApiError> {
    if spec.choices.len() < 2 {
        return Err(ApiError::Unavailable("rubric_choice_missing".into()));
    }
    let mut map = Map::new();
    for (key, description) in spec.choices {
        if key.is_empty() || description.trim().is_empty() {
            return Err(ApiError::Unavailable("rubric_choice_invalid".into()));
        }
        map.insert((*key).to_string(), Value::String((*description).to_string()));
    }
    Ok(map)
}

fn decision_questions(spec: &CallSpec<'_>) -> Result<Value, ApiError> {
    if !question_id_valid(spec.question_id) {
        return Err(ApiError::Unavailable("question_id_invalid".into()));
    }
    let question = json!({
        "type": "choice",
        "instructions": instruction_text(spec),
        "criteria": criteria(spec)?,
    });
    let mut questions = Map::new();
    questions.insert(spec.question_id.to_string(), question);
    Ok(Value::Object(questions))
}

fn body_for(spec: &CallSpec<'_>) -> Result<Value, ApiError> {
    if spec.effort.is_some() && !spec.api.supports_openai_effort() {
        return Err(ApiError::Unavailable("effort_not_supported_for_api".into()));
    }
    Ok(match spec.api {
        ApiKind::ChatCompletions => {
            let mut body = json!({
                "model": spec.model,
                "messages": [
                    {"role": "system", "content": instruction_text(spec)},
                    {"role": "user", "content": spec.input},
                ],
            });
            if let Some(effort) = spec.effort {
                body["reasoning_effort"] = Value::String(effort.to_string());
            }
            body
        }
        ApiKind::Responses => {
            let mut body = json!({
                "model": spec.model,
                "input": [
                    {
                        "role": "system",
                        "content": [{"type": "input_text", "text": instruction_text(spec)}],
                    },
                    {
                        "role": "user",
                        "content": [{"type": "input_text", "text": spec.input}],
                    },
                ],
            });
            if let Some(effort) = spec.effort {
                body["reasoning"] = json!({"effort": effort});
            }
            body
        }
        ApiKind::Gemini => json!({
            "systemInstruction": {"parts": [{"text": instruction_text(spec)}]},
            "contents": [{"role": "user", "parts": [{"text": spec.input}]}],
        }),
        ApiKind::Decision => json!({
            "model": spec.model,
            "state": spec.input,
            "questions": decision_questions(spec)?,
        }),
        ApiKind::GeminiDecision => json!({
            "model": spec.model,
            "input": spec.input,
            "questions": decision_questions(spec)?,
        }),
    })
}

/// Builds the exact request for a typed API. The endpoint is the owner-supplied
/// full request URL; no path, model segment or host is inferred.
pub fn build_request(
    spec: &CallSpec<'_>,
    endpoint: &Endpoint,
    session: Option<&str>,
) -> Result<WireRequest, ApiError> {
    if spec.model.trim().is_empty() {
        return Err(ApiError::Unavailable("model_missing".into()));
    }
    let body = body_for(spec)?;
    let mut body_keys: Vec<String> = body
        .as_object()
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default();
    body_keys.sort();
    let body = serde_json::to_vec(&body)
        .map_err(|_| ApiError::Unavailable("request_encode".into()))?;
    let mut headers: Vec<(&'static str, String)> = vec![
        ("content-type", "application/json".to_string()),
        ("accept", "application/json".to_string()),
    ];
    let mut session_sent = false;
    if let Some(session) = session {
        if is_console_gateway(endpoint) {
            headers.push((SESSION_HEADER, session.to_string()));
            session_sent = true;
        }
    }
    Ok(WireRequest {
        url: endpoint.url.clone(),
        scheme: endpoint.scheme.clone(),
        host: endpoint.host.clone(),
        port: endpoint.port,
        path_and_query: endpoint.path_and_query.clone(),
        api: spec.api,
        auth: AuthStyle::for_api(spec.api),
        headers,
        body,
        body_keys,
        effort_sent: spec.effort_sent(),
        session_sent,
    })
}

fn response_object(body: &[u8], code: &'static str) -> Result<Value, ApiError> {
    serde_json::from_slice(body).map_err(|_| ApiError::Rejected(code.to_string()))
}

fn u64_at(value: &Value, path: &[&str]) -> Option<u64> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    cursor.as_u64()
}

fn usage_from(value: &Value, input: &[&str], output: &[&str]) -> Usage {
    Usage {
        input_tokens: u64_at(value, input),
        output_tokens: u64_at(value, output),
    }
}

fn model_from(value: &Value) -> Option<String> {
    value
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn text_from_chat(value: &Value) -> Result<String, ApiError> {
    let content = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::Rejected("chat_missing_content".into()))?;
    Ok(content.to_string())
}

fn text_from_responses(value: &Value) -> Result<String, ApiError> {
    let mut text = String::new();
    if let Some(output) = value.get("output").and_then(Value::as_array) {
        for item in output {
            if item.get("type").and_then(Value::as_str) != Some("message") {
                continue;
            }
            let Some(parts) = item.get("content").and_then(Value::as_array) else {
                continue;
            };
            for part in parts {
                if part.get("type").and_then(Value::as_str) == Some("output_text") {
                    if let Some(chunk) = part.get("text").and_then(Value::as_str) {
                        text.push_str(chunk);
                    }
                }
            }
        }
    }
    if text.is_empty() {
        if let Some(chunk) = value.get("output_text").and_then(Value::as_str) {
            text.push_str(chunk);
        }
    }
    if text.is_empty() {
        return Err(ApiError::Rejected("responses_missing_output_text".into()));
    }
    Ok(text)
}

/// Bounded, non-text structural summary of a Responses envelope.
///
/// Only fixed allowlisted codes, presence flags and type names are retained.
/// No prompt, output text, refusal text, provider reason string, body or key
/// can reach this struct; it exists for `ase llm smoke` failure diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResponsesDiagnostic {
    /// Top-level `status` mapped to a fixed code.
    pub status: &'static str,
    /// `incomplete_details.reason` mapped to a fixed code.
    pub incomplete_reason: &'static str,
    /// An output part had type `refusal`; its text is never read.
    pub refusal_part: bool,
    /// An output part had type `output_text`; its text is not read here.
    pub output_text_part: bool,
    /// The convenience top-level `output_text` field was a nonempty string.
    pub top_level_output_text: bool,
    /// Allowlisted `output[].type` values, sorted and deduplicated.
    pub output_item_types: Vec<&'static str>,
    /// Allowlisted `output[].content[].type` values, sorted and deduplicated.
    pub content_part_types: Vec<&'static str>,
}

impl Default for ResponsesDiagnostic {
    fn default() -> Self {
        Self {
            status: "absent",
            incomplete_reason: "absent",
            refusal_part: false,
            output_text_part: false,
            top_level_output_text: false,
            output_item_types: Vec::new(),
            content_part_types: Vec::new(),
        }
    }
}

fn status_code(value: Option<&Value>) -> &'static str {
    match value {
        None => "absent",
        Some(value) => match value.as_str() {
            Some("completed") => "completed",
            Some("in_progress") => "in_progress",
            Some("queued") => "queued",
            Some("incomplete") => "incomplete",
            Some("failed") => "failed",
            Some("cancelled") => "cancelled",
            Some(_) | None => "other",
        },
    }
}

fn reason_code(value: Option<&Value>) -> &'static str {
    match value {
        None => "absent",
        Some(value) => match value.as_str() {
            Some("max_output_tokens") => "max_output_tokens",
            Some("content_filter") => "content_filter",
            Some(_) | None => "other",
        },
    }
}

fn push_type(types: &mut Vec<&'static str>, code: &'static str) {
    if !types.contains(&code) {
        types.push(code);
    }
}

fn item_type(value: &str) -> &'static str {
    match value {
        "message" => "message",
        "reasoning" => "reasoning",
        "function_call" => "function_call",
        "function_call_output" => "function_call_output",
        _ => "other",
    }
}

fn part_type(value: &str) -> &'static str {
    match value {
        "output_text" => "output_text",
        "refusal" => "refusal",
        "input_text" => "input_text",
        _ => "other",
    }
}

/// Reads one Responses envelope for smoke diagnostics. `None` means the body
/// was not a JSON object. The parse policy in `text_from_responses` is
/// unchanged: this summary never contributes to or alters a typed answer.
pub fn responses_diagnostic(body: &[u8]) -> Option<ResponsesDiagnostic> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let object = value.as_object()?;
    let mut diagnostic = ResponsesDiagnostic {
        status: status_code(object.get("status")),
        incomplete_reason: reason_code(
            object
                .get("incomplete_details")
                .and_then(|details| details.get("reason")),
        ),
        ..Default::default()
    };
    diagnostic.top_level_output_text = object
        .get("output_text")
        .and_then(Value::as_str)
        .is_some_and(|text| !text.is_empty());
    if let Some(items) = object.get("output").and_then(Value::as_array) {
        for item in items {
            if let Some(kind) = item.get("type").and_then(Value::as_str) {
                push_type(&mut diagnostic.output_item_types, item_type(kind));
            }
            let Some(parts) = item.get("content").and_then(Value::as_array) else {
                continue;
            };
            for part in parts {
                match part.get("type").and_then(Value::as_str) {
                    Some("output_text") => {
                        diagnostic.output_text_part = true;
                        push_type(&mut diagnostic.content_part_types, "output_text");
                    }
                    Some("refusal") => {
                        diagnostic.refusal_part = true;
                        push_type(&mut diagnostic.content_part_types, "refusal");
                    }
                    Some(kind) => push_type(&mut diagnostic.content_part_types, part_type(kind)),
                    None => {}
                }
            }
        }
    }
    diagnostic.output_item_types.sort_unstable();
    diagnostic.content_part_types.sort_unstable();
    Some(diagnostic)
}

fn text_from_gemini(value: &Value) -> Result<String, ApiError> {
    if let Some(reason) = value
        .get("promptFeedback")
        .and_then(|feedback| feedback.get("blockReason"))
        .and_then(Value::as_str)
    {
        if !reason.is_empty() {
            return Err(ApiError::Rejected("prompt_blocked".into()));
        }
    }
    let parts = value
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|candidates| candidates.first())
        .and_then(|candidate| candidate.get("content"))
        .and_then(|content| content.get("parts"))
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::Rejected("gemini_missing_parts".into()))?;
    let mut text = String::new();
    for part in parts {
        if let Some(chunk) = part.get("text").and_then(Value::as_str) {
            text.push_str(chunk);
        }
    }
    if text.is_empty() {
        return Err(ApiError::Rejected("gemini_missing_text".into()));
    }
    Ok(text)
}

fn probability(value: &Value, code: &'static str) -> Result<f64, ApiError> {
    let number = value
        .as_f64()
        .ok_or_else(|| ApiError::Rejected(code.to_string()))?;
    if !number.is_finite() || !(0.0..=1.0).contains(&number) {
        return Err(ApiError::Rejected(code.to_string()));
    }
    Ok(number)
}

fn choice_from_decisions(
    value: &Value,
    question_id: &str,
) -> Result<(String, f64, BTreeMap<String, f64>), ApiError> {
    for blocked in ["prompt_feedback", "promptFeedback"] {
        if let Some(reason) = value
            .get(blocked)
            .and_then(|feedback| feedback.get("block_reason").or_else(|| feedback.get("blockReason")))
        {
            if !reason.is_null() {
                return Err(ApiError::Rejected("prompt_blocked".into()));
            }
        }
    }
    let answers = value
        .get("answers")
        .and_then(Value::as_object)
        .ok_or_else(|| ApiError::Rejected("decision_missing_answers".into()))?;
    let answer = answers
        .get(question_id)
        .and_then(Value::as_object)
        .ok_or_else(|| ApiError::Rejected("decision_missing_answer".into()))?;
    if answer.get("type").and_then(Value::as_str) != Some("choice") {
        return Err(ApiError::Rejected("decision_wrong_answer_type".into()));
    }
    let choice = answer
        .get("choice")
        .and_then(Value::as_str)
        .filter(|choice| !choice.is_empty())
        .ok_or_else(|| ApiError::Rejected("decision_missing_choice".into()))?
        .to_string();
    let confidence = probability(
        answer
            .get("confidence")
            .ok_or_else(|| ApiError::Rejected("decision_missing_confidence".into()))?,
        "decision_invalid_confidence",
    )?;
    let mut probabilities = BTreeMap::new();
    if let Some(map) = answer.get("probabilities").and_then(Value::as_object) {
        for (key, value) in map {
            probabilities.insert(
                key.clone(),
                probability(value, "decision_invalid_probability")?,
            );
        }
    }
    Ok((choice, confidence, probabilities))
}

/// Parses one response envelope for the typed API. A shape mismatch is a
/// `Rejected` code, never a partial or guessed answer.
pub fn parse_response(
    api: ApiKind,
    question_id: &str,
    body: &[u8],
) -> Result<WireResponse, ApiError> {
    let value = response_object(body, "response_json")?;
    match api {
        ApiKind::ChatCompletions => Ok(WireResponse {
            answer: WireAnswer::Text(text_from_chat(&value)?),
            usage: usage_from(&value, &["usage", "prompt_tokens"], &["usage", "completion_tokens"]),
            model: model_from(&value),
        }),
        ApiKind::Responses => Ok(WireResponse {
            answer: WireAnswer::Text(text_from_responses(&value)?),
            usage: usage_from(&value, &["usage", "input_tokens"], &["usage", "output_tokens"]),
            model: model_from(&value),
        }),
        ApiKind::Gemini => Ok(WireResponse {
            answer: WireAnswer::Text(text_from_gemini(&value)?),
            usage: Usage {
                input_tokens: u64_at(&value, &["usageMetadata", "promptTokenCount"]),
                output_tokens: u64_at(&value, &["usageMetadata", "candidatesTokenCount"]),
            },
            model: value
                .get("modelVersion")
                .and_then(Value::as_str)
                .map(str::to_string),
        }),
        ApiKind::Decision => {
            let (choice, confidence, probabilities) = choice_from_decisions(&value, question_id)?;
            Ok(WireResponse {
                answer: WireAnswer::Choice {
                    choice,
                    confidence,
                    probabilities,
                },
                usage: usage_from(&value, &["usage", "input_tokens"], &["usage", "output_tokens"]),
                model: model_from(&value),
            })
        }
        ApiKind::GeminiDecision => {
            let (choice, confidence, probabilities) = choice_from_decisions(&value, question_id)?;
            Ok(WireResponse {
                answer: WireAnswer::Choice {
                    choice,
                    confidence,
                    probabilities,
                },
                usage: Usage {
                    input_tokens: u64_at(&value, &["usage", "total_input_tokens"]),
                    output_tokens: u64_at(&value, &["usage", "total_output_tokens"]),
                },
                model: model_from(&value),
            })
        }
    }
}
