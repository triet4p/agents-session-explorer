//! Shared typed rubric mapping for every model transport.
//!
//! The deterministic baseline and the store keep one three-way vocabulary
//! (`split`/`continue`/`unknown`). Model transports differ in how they report
//! that decision:
//!
//! - an OpenAI-compatible or native Gemini generation model returns text that
//!   must contain one JSON object with a **verbal** self-report
//!   (`high`/`medium`/`low`);
//! - both decision APIs return a **calibrated probability** for the chosen
//!   option. That number is not a verbal level and is never converted into
//!   one; the code records which semantic produced the value.
//!
//! The rendered decision pair is always supplied as data (`messages`,
//! `contents` or `state`/`input`); instructions never embed it. Definitions of
//! the options and the mapping to `Decision` live here once.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::assertion::Decision;
use ase_llm::{ApiKind, WireAnswer};

/// Stable decision question id for the two decision protocols.
pub const QUESTION_ID: &str = "task_boundary";

/// Allowed rubric options, in a fixed order. The keys are the protocol choices
/// and the stored labels.
pub const CHOICES: [(&str, &str); 3] = [
    (
        "split",
        "A distinct task or objective starts in the LATER message.",
    ),
    (
        "continue",
        "The LATER message continues or refines the SAME task as the EARLIER one.",
    ),
    (
        "unknown",
        "The visible pair does not decide the relation.",
    ),
];

pub const VERBAL_VALUES: [&str; 3] = ["high", "medium", "low"];

/// A model-reported confidence with an explicit semantic. The variants are
/// never interchanged or thresholded into each other.
#[derive(Clone, Debug, PartialEq)]
pub enum ModelConfidence {
    /// Self-reported verbal level from a generation model.
    Verbal(String),
    /// Calibrated probability in `[0, 1]` from a decision API.
    Probability(f64),
}

impl ModelConfidence {
    pub fn semantics(&self) -> &'static str {
        match self {
            ModelConfidence::Verbal(_) => "verbal",
            ModelConfidence::Probability(_) => "probability",
        }
    }

    pub fn value_json(&self) -> Value {
        match self {
            ModelConfidence::Verbal(level) => Value::String(level.clone()),
            ModelConfidence::Probability(probability) => {
                json!(probability)
            }
        }
    }

    /// Store encoding. The prefix keeps the semantic explicit in the existing
    /// `confidence` column without changing the historical schema.
    pub fn encode(&self) -> String {
        match self {
            ModelConfidence::Verbal(level) => format!("verbal:{level}"),
            ModelConfidence::Probability(probability) => {
                format!("probability:{}", format_probability(*probability))
            }
        }
    }
}

/// Decodes a stored confidence value. Unprefixed legacy values were verbal
/// self-reports, which is how every pre-T5 assertion was written.
pub fn decode_confidence(text: &str) -> (&'static str, Value) {
    if let Some(value) = text.strip_prefix("verbal:") {
        return ("verbal", Value::String(value.to_string()));
    }
    if let Some(value) = text.strip_prefix("probability:") {
        if let Ok(probability) = value.parse::<f64>() {
            return ("probability", json!(probability));
        }
    }
    ("verbal", Value::String(text.to_string()))
}

fn format_probability(value: f64) -> String {
    let mut text = format!("{value:.6}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text.is_empty() {
        text.push('0');
    }
    text
}

/// The instruction seen by the SLM stage. It contains no session text.
pub fn slm_instruction() -> &'static str {
    "You are a local small language model for offline task-boundary detection.\n\
     Decide whether the LATER human message starts a distinct task relative to the EARLIER one.\n\
     Allowed labels: split, continue, unknown. split = a distinct task/objective starts; \
     continue = same task/objective; unknown = the visible pair does not decide.\n\
     Allowed confidence: high, medium, low. Put a short abstain reason only for unknown, otherwise null.\n\
     Answer with one JSON object only: {\"label\":\"continue\",\"confidence\":\"medium\",\"abstain_reason\":null}\n\
     Use only the two messages supplied as data; do not use outside context."
}

/// The instruction seen by the Judge stage. Candidate labels are codes, never
/// prose.
pub fn judge_instruction(baseline: Decision, slm: Option<Decision>) -> String {
    let slm = slm.map(Decision::as_str).unwrap_or("unavailable");
    format!(
        "You are a separate local adjudicator for offline task-boundary evaluation.\n\
         A deterministic baseline answered \"{}\"; the local SLM answered \"{slm}\".\n\
         Using only the two human messages supplied as data, decide the final label.\n\
         Allowed labels: split, continue, unknown. Allowed confidence: high, medium, low.\n\
         Put a short abstain reason only for unknown, otherwise null.\n\
         You may keep or override either candidate. Use unknown when the visible pair does not decide.\n\
         Answer with one JSON object only: {{\"label\":\"continue\",\"confidence\":\"medium\",\"abstain_reason\":null}}",
        baseline.as_str(),
    )
}

/// One-string prompt used by the workspace-local process runner, which has a
/// single prompt channel.
pub fn slm_prompt(rendered: &str) -> String {
    format!("{}\n<|ase:decision|>\n{rendered}", slm_instruction())
}

pub fn judge_prompt(rendered: &str, baseline: Decision, slm: Option<Decision>) -> String {
    format!(
        "{}\n<|ase:decision|>\n{rendered}",
        judge_instruction(baseline, slm)
    )
}

#[derive(Deserialize)]
struct GenerationAnswer {
    label: Option<String>,
    confidence: Option<String>,
    abstain_reason: Option<String>,
}

fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(&text[start..=end])
}

fn unknown_reason(reason: Option<String>) -> Option<String> {
    reason
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| Some("model_chose_unknown".to_string()))
}

/// Maps generation text onto the shared rubric. Returns a stable rejection code
/// instead of guessing a label.
pub fn parse_generation(
    text: &str,
) -> Result<(Decision, Option<ModelConfidence>, Option<String>), &'static str> {
    let object = extract_json_object(text).ok_or("generation_missing_json_object")?;
    let answer: GenerationAnswer =
        serde_json::from_str(object).map_err(|_| "generation_invalid_answer")?;
    let label = answer
        .label
        .as_deref()
        .and_then(Decision::parse)
        .ok_or("generation_invalid_label")?;
    let confidence = match answer.confidence {
        Some(level) => {
            let normalized = level.trim().to_ascii_lowercase();
            if !VERBAL_VALUES.contains(&normalized.as_str()) {
                return Err("generation_invalid_confidence");
            }
            Some(ModelConfidence::Verbal(normalized))
        }
        None => None,
    };
    let abstain_reason = if label == Decision::Unknown {
        unknown_reason(answer.abstain_reason)
    } else {
        None
    };
    Ok((label, confidence, abstain_reason))
}

/// Maps a typed decision answer onto the shared rubric. The probability keeps
/// its own semantic and is never translated to `high`/`medium`/`low`.
pub fn map_choice(
    choice: &str,
    probability: f64,
) -> Result<(Decision, ModelConfidence, Option<String>), &'static str> {
    if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
        return Err("decision_invalid_confidence");
    }
    let label = Decision::parse(choice).ok_or("decision_unknown_choice")?;
    let abstain_reason = if label == Decision::Unknown {
        Some("model_chose_unknown".to_string())
    } else {
        None
    };
    Ok((
        label,
        ModelConfidence::Probability(probability),
        abstain_reason,
    ))
}

/// Interprets one transport answer. A generation API must return text and a
/// decision API must return a typed choice; the other shape is rejected rather
/// than re-mapped.
pub fn interpret(
    api: ApiKind,
    answer: WireAnswer,
) -> Result<(Decision, Option<ModelConfidence>, Option<String>), &'static str> {
    if api.is_decision() {
        match answer {
            WireAnswer::Choice {
                choice,
                confidence,
                ..
            } => {
                let (label, confidence, abstain_reason) = map_choice(&choice, confidence)?;
                Ok((label, Some(confidence), abstain_reason))
            }
            WireAnswer::Text(_) => Err("decision_returned_text"),
        }
    } else {
        match answer {
            WireAnswer::Text(text) => parse_generation(&text),
            WireAnswer::Choice { .. } => Err("generation_returned_choice"),
        }
    }
}

/// Prompt/version strings used by the stage identity and the store.
pub const RUBRIC_VERSION: &str = "rubric-map-v2";
