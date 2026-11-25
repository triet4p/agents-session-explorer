//! `ase llm smoke` — the one network route in the `llm` group.
//!
//! This module owns the fixed synthetic call and its success/failure reports.
//! The instruction, input and choices are constants: no session, journal,
//! database, recap or excerpt data is ever sent. Diagnostics expose only
//! allowlisted codes and structural facts, never a credential, endpoint text
//! or raw provider body. All non-secret settings and credential resolution
//! still come from `ase-llm`.

use ase_core::{err, Result};
use crate::{llm::llm_expect, opt};
use serde_json::json;

const SMOKE_INSTRUCTION: &str =
    "Synthetic protocol check. Choose the option that matches the two synthetic messages. \
     No real session, journal or user content is present.";
/// Text-generation APIs must return the rubric object as text; both decision
/// APIs keep the typed-choice instruction and their own answer contract.
const SMOKE_GENERATION_INSTRUCTION: &str =
    "Synthetic protocol check. Choose the option that matches the two synthetic messages. \
     No real session, journal or user content is present. \
     Answer with one JSON object only and no other text: \
     {\"label\":\"continue\",\"confidence\":\"medium\",\"abstain_reason\":null}. \
     Allowed labels: split, continue, unknown. \
     Allowed confidence: high, medium, low. \
     Use a short abstain_reason string only for unknown; otherwise use null.";
const SMOKE_INPUT: &str = "<|ase:prev|>\nsynthetic earlier task instruction\n<|ase:curr|>\nsynthetic later refinement of the same task\n";
const SMOKE_CHOICES: [(&str, &str); 3] = [
    ("split", "A distinct synthetic task starts."),
    ("continue", "The same synthetic task continues."),
    ("unknown", "The synthetic pair does not decide."),
];

/// Fixed codes accepted from `ApiError::detail()`. Transport errors may carry
/// OS text, so only an exact allowlist is ever echoed as a code.
const SMOKE_API_CODES: &[&str] = &[
    "api_kind_mismatch", "model_missing", "rubric_choice_missing", "rubric_choice_invalid",
    "question_id_invalid", "request_encode", "effort_not_supported_for_api",
    "bad_request", "auth_rejected", "endpoint_not_found", "request_timeout",
    "request_too_large", "rate_limited", "server_error",
    "redirect_refused", "response_json", "response_too_large",
    "chat_missing_content", "responses_missing_output_text", "gemini_missing_parts",
    "gemini_missing_text", "prompt_blocked", "decision_missing_answers",
    "decision_missing_answer", "decision_wrong_answer_type", "decision_missing_choice",
    "decision_missing_confidence", "decision_invalid_confidence", "decision_invalid_probability",
    "resolve_host", "connect", "read_timeout", "write_timeout", "write", "write_body",
    "read", "response_headers_missing", "status_unparsable", "response_truncated",
    "chunked_missing_size", "chunked_bad_size", "chunked_truncated", "chunked_missing_terminator",
    "https_transport_unavailable_on_platform",
];

/// Fixed codes accepted from `ase_judge::rubric::interpret`.
const SMOKE_RUBRIC_CODES: &[&str] = &[
    "generation_missing_json_object", "generation_invalid_answer", "generation_invalid_label",
    "generation_invalid_confidence", "decision_invalid_confidence", "decision_unknown_choice",
    "decision_returned_text", "generation_returned_choice",
];

fn allowlisted_code(value: &str, codes: &[&'static str], fallback: &'static str) -> &'static str {
    codes
        .iter()
        .copied()
        .find(|code| *code == value)
        .unwrap_or(fallback)
}

fn smoke_api_code(error: Option<&ase_llm::ApiError>) -> &'static str {
    match error {
        Some(error) => allowlisted_code(error.detail(), SMOKE_API_CODES, "api_error_unclassified"),
        None => "none",
    }
}

fn smoke_rubric_code(code: &str) -> &'static str {
    allowlisted_code(code, SMOKE_RUBRIC_CODES, "rubric_error_unclassified")
}

/// Fixed structural facts about extracted text; the text itself is never
/// emitted, only its size and the presence/order of JSON object delimiters.
fn smoke_text_context(text: &str) -> serde_json::Value {
    let open = text.find('{');
    let close = text.find('}');
    let json_delimiters = match (open, close) {
        (Some(open), Some(close)) if open < close => "paired",
        (Some(_), Some(_)) => "reversed",
        (Some(_), None) => "open_only",
        (None, Some(_)) => "close_only",
        (None, None) => "none",
    };
    json!({
        "chars": text.chars().count(),
        "bytes": text.len(),
        "json_delimiters": json_delimiters,
    })
}

/// Chooses one fixed primary code. Explicit envelope state outranks parse and
/// rubric codes; no provider string participates in the decision output.
fn smoke_primary_code(
    stage: &str,
    api_code: &'static str,
    rubric_code: &'static str,
    http_status: Option<u16>,
    responses: Option<&ase_llm::ResponsesDiagnostic>,
) -> &'static str {
    let response_received = http_status.is_some_and(|status| (200..300).contains(&status));
    if response_received {
        if let Some(response) = responses {
            match response.status {
                "incomplete" => {
                    return match response.incomplete_reason {
                        "max_output_tokens" => "incomplete_max_output_tokens",
                        "content_filter" => "incomplete_content_filter",
                        "other" => "incomplete_other_reason",
                        _ => "incomplete_no_reason",
                    }
                }
                "failed" => return "response_failed_state",
                _ => {}
            }
            let has_output = response.output_text_part || response.top_level_output_text;
            if response.refusal_part && !has_output {
                return "refusal_without_output_text";
            }
            if !has_output {
                return "missing_output_text";
            }
        }
    }
    if stage == "rubric" {
        return rubric_code;
    }
    api_code
}

/// Failure-only machine-readable report. The successful smoke report schema
/// is untouched, and no field here contains provider text, a key or an endpoint.
fn print_smoke_diagnostic(
    api: &str,
    stage: &str,
    code: &'static str,
    api_code: &'static str,
    rubric_code: Option<&'static str>,
    http_status: Option<u16>,
    responses: Option<&ase_llm::ResponsesDiagnostic>,
    text: Option<serde_json::Value>,
) -> Result<()> {
    let response = responses.map(|value| {
        json!({
            "status": value.status,
            "incomplete_reason": value.incomplete_reason,
            "refusal_part": value.refusal_part,
            "output_text_part": value.output_text_part,
            "top_level_output_text": value.top_level_output_text,
            "output_item_types": value.output_item_types,
            "content_part_types": value.content_part_types,
        })
    });
    let report = json!({
        "schema": "ase-llm-smoke-diagnostic/v1",
        "synthetic": true,
        "api": api,
        "stage": stage,
        "code": code,
        "api_error_code": api_code,
        "rubric_code": rubric_code,
        "http_status": http_status,
        "response": response,
        "text": text,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&report)
            .map_err(|e| err(format!("serialize llm smoke diagnostic: {e}")))?
    );
    Ok(())
}

/// One fixed synthetic call through the configured protocol. This is the only
/// `ase llm` command that opens a socket; it never sends journal, database,
/// recap or excerpt data, and its success and failure reports contain no
/// credential or raw body.
pub fn run(args: &[String]) -> Result<()> {
    llm_expect(
        args,
        &["--profile", "--timeout-ms"],
        &["--json"],
    )?;
    let timeout_ms = match opt(args, "--timeout-ms") {
        Some(text) => text
            .parse::<u64>()
            .map_err(|_| err("invalid --timeout-ms"))?,
        None => ase_llm::DEFAULT_TIMEOUT_MS,
    };
    let env = ase_llm::capture_env();
    let resolution = ase_llm::resolve(&env, opt(args, "--profile").as_deref())?;
    let profile = &resolution.profile;
    let instruction = if profile.api.is_decision() {
        SMOKE_INSTRUCTION
    } else {
        SMOKE_GENERATION_INSTRUCTION
    };
    let spec = ase_llm::CallSpec {
        api: profile.api,
        model: &profile.model,
        effort: profile.effort.as_deref(),
        instruction,
        input: SMOKE_INPUT,
        question_id: ase_judge::rubric::QUESTION_ID,
        choices: &SMOKE_CHOICES,
    };
    let outcome = ase_llm::call_diagnostic(
        &resolution,
        &spec,
        std::time::Duration::from_millis(timeout_ms.max(1)),
    );
    let result = match outcome.result {
        Ok(result) => result,
        Err(api_error) => {
            let api_code = smoke_api_code(Some(&api_error));
            let primary = smoke_primary_code(
                "call",
                api_code,
                "none",
                outcome.http_status,
                outcome.responses.as_ref(),
            );
            print_smoke_diagnostic(
                profile.api.as_str(),
                "call",
                primary,
                api_code,
                None,
                outcome.http_status,
                outcome.responses.as_ref(),
                None,
            )?;
            return Err(err(format!(
                "llm smoke failed: {primary} (api_error: {api_code}; diagnostic on stdout)"
            )));
        }
    };
    let answer_kind = match &result.answer {
        ase_llm::WireAnswer::Text(_) => "text",
        ase_llm::WireAnswer::Choice { .. } => "choice",
    };
    let probabilities = match &result.answer {
        ase_llm::WireAnswer::Choice { probabilities, .. } => Some(probabilities.clone()),
        ase_llm::WireAnswer::Text(_) => None,
    };
    let text_context = match &result.answer {
        ase_llm::WireAnswer::Text(text) => Some(smoke_text_context(text)),
        ase_llm::WireAnswer::Choice { .. } => None,
    };
    let (label, confidence, abstain_reason) =
        match ase_judge::rubric::interpret(profile.api, result.answer) {
            Ok(interpreted) => interpreted,
            Err(code) => {
                let rubric_code = smoke_rubric_code(code);
                let primary = smoke_primary_code(
                    "rubric",
                    "none",
                    rubric_code,
                    outcome.http_status,
                    outcome.responses.as_ref(),
                );
                print_smoke_diagnostic(
                    profile.api.as_str(),
                    "rubric",
                    primary,
                    "none",
                    Some(rubric_code),
                    outcome.http_status,
                    outcome.responses.as_ref(),
                    text_context,
                )?;
                return Err(err(format!(
                    "llm smoke failed: {primary} (rubric: {rubric_code}; diagnostic on stdout)"
                )));
            }
        };
    let value = json!({
        "schema": "ase-llm-smoke/v1",
        "synthetic": true,
        "network": "used",
        "profile": profile.profile,
        "provider": profile.provider,
        "api": profile.api.as_str(),
        "endpoint": profile.endpoint.url,
        "origin": profile.endpoint.origin,
        "model": profile.model,
        "effort": {
            "value": profile.effort,
            "sent": result.effort_sent,
        },
        "auth": {
            "header": result.auth_header,
            "scheme": result.auth_scheme,
            "value_present": true,
            "value_printed": false,
        },
        "request": {
            "body_keys": result.body_keys,
            "session_header_sent": result.session_sent,
        },
        "answer": {
            "kind": answer_kind,
            "label": label.as_str(),
            "confidence": confidence.as_ref().map(|value| json!({
                "semantics": value.semantics(),
                "value": value.value_json(),
            })),
            "abstain_reason": abstain_reason,
            "probabilities": probabilities,
            "model": result.model,
            "usage": ase_llm::usage_json(&result.usage),
        },
        "status": result.status,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value)
            .map_err(|e| err(format!("serialize llm smoke report: {e}")))?
    );
    Ok(())
}
