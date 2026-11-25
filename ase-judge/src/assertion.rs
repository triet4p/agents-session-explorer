//! Versioned assertion vocabulary for the offline evaluation layer.
//!
//! An assertion is a model or rule judgement about one adjacent pair of human
//! user messages on the active graph leaf of one pinned journal. It is *not* a
//! source fact: the source fact is the two byte spans it points at. Every
//! assertion carries those spans, the exact projected input digest it was
//! computed from, and the rule/prompt/model identity that produced it. The
//! store refuses to insert an assertion without evidence and never stores the
//! message prose.
//!
//! Cache identity is span/input based (INITIALIZATION.md §12): the key covers
//! the two line digests, the rendered input digest, the projection/render
//! versions and the generator identity, never the whole journal digest. An
//! append elsewhere in the journal therefore does not invalidate a decision.

use ase_core::{err, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const VIEW_VERSION: &str = "turn-episode-v1";
pub const INPUT_RENDER_VERSION: &str = "decision-input-v1";
pub const BASELINE_RULE_VERSION: &str = "baseline-rubric-v2";
pub const BASELINE_GENERATOR_ID: &str = "baseline-rules";
pub const SLM_PROMPT_VERSION: &str = "slm-boundary-v2";
pub const JUDGE_PROMPT_VERSION: &str = "judge-adjudicate-v2";
pub const LOCAL_MODEL_PROTOCOL: &str = "ase-local-model/v1";
/// Bytes of the earlier message shown to a generator (its tail).
pub const PREV_TAIL_BYTES: usize = 800;
/// Bytes of the later message shown to a generator (its head).
pub const CURR_HEAD_BYTES: usize = 1600;

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn canonical(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}

/// The only three outcomes. `unknown` is a measured outcome and is never
/// merged into `split`/`continue` by any layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Split,
    Continue,
    Unknown,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Split => "split",
            Decision::Continue => "continue",
            Decision::Unknown => "unknown",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "split" => Some(Decision::Split),
            "continue" => Some(Decision::Continue),
            "unknown" => Some(Decision::Unknown),
            _ => None,
        }
    }
}

/// The producing layer. The baseline is a deterministic rule engine; the SLM
/// and the Judge are separate local-model stages with independent versions and
/// independent cache namespaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Generator {
    Baseline,
    Slm,
    Judge,
}

impl Generator {
    pub fn as_str(self) -> &'static str {
        match self {
            Generator::Baseline => "baseline",
            Generator::Slm => "slm",
            Generator::Judge => "judge",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "baseline" => Some(Generator::Baseline),
            "slm" => Some(Generator::Slm),
            "judge" => Some(Generator::Judge),
            _ => None,
        }
    }
}

fn is_hex64(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// One addressable source line: ordinal within the journal (non-blank lines
/// counted from zero), entry id, byte offset/length of the retained line and
/// the SHA-256 of those exact bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub ordinal: u32,
    pub entry_id: String,
    pub off: u64,
    pub len: u32,
    pub line_sha256: String,
}

impl Anchor {
    fn validate(&self, side: &str) -> Result<()> {
        if self.entry_id.is_empty() {
            return Err(err(format!("assertion evidence {side} has no entry id")));
        }
        if self.len == 0 {
            return Err(err(format!("assertion evidence {side} has an empty span")));
        }
        if !is_hex64(&self.line_sha256) {
            return Err(err(format!(
                "assertion evidence {side} has no strong line digest"
            )));
        }
        Ok(())
    }
}

/// Evidence is exactly the pair of source lines the decision is about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub prev: Anchor,
    pub curr: Anchor,
}

impl Evidence {
    pub fn anchors(&self) -> [&Anchor; 2] {
        [&self.prev, &self.curr]
    }
}

fn head_bytes(text: &str, budget: usize) -> &str {
    if text.len() <= budget {
        return text;
    }
    let mut cut = budget;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    &text[..cut]
}

fn tail_bytes(text: &str, budget: usize) -> &str {
    if text.len() <= budget {
        return text;
    }
    let mut cut = text.len() - budget;
    while cut < text.len() && !text.is_char_boundary(cut) {
        cut += 1;
    }
    &text[cut..]
}

/// Deterministic, versioned render of the decision input. The earlier message
/// contributes its tail and the later message its head, so a very long
/// transcript cannot make the cache key unstable for an unrelated turn.
pub fn render_decision(prev_text: &str, curr_text: &str) -> String {
    format!(
        "<|ase:prev|>\n{}\n<|ase:curr|>\n{}\n",
        tail_bytes(prev_text, PREV_TAIL_BYTES),
        head_bytes(curr_text, CURR_HEAD_BYTES)
    )
}

/// A judgement about one decision. The two source anchors are mandatory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assertion {
    pub unit_id: String,
    pub decision_index: u32,
    pub generator: Generator,
    /// Baseline rule version, or the SLM/Judge prompt version.
    pub rule_version: String,
    /// `baseline-rules`, or the declared local model identity.
    pub generator_id: String,
    pub evidence: Evidence,
    pub render_sha256: String,
    pub label: Decision,
    pub confidence: Option<String>,
    pub abstain_reason: Option<String>,
    /// Judge-only input context (the candidate labels it consumed). Codes
    /// only, never prose.
    pub context: Option<Value>,
    pub verification_only: bool,
}

impl Assertion {
    /// Digest of the decision input before any Judge candidate context.
    pub fn base_input_sha256(&self) -> String {
        sha256_hex(&canonical(&json!({
            "schema": "ase-judge-decision-input/v1",
            "unit_id": self.unit_id,
            "decision_index": self.decision_index,
            "render_version": INPUT_RENDER_VERSION,
            "render_sha256": self.render_sha256,
            "prev": {
                "ordinal": self.evidence.prev.ordinal,
                "entry_id": self.evidence.prev.entry_id,
                "off": self.evidence.prev.off,
                "len": self.evidence.prev.len,
                "line_sha256": self.evidence.prev.line_sha256,
            },
            "curr": {
                "ordinal": self.evidence.curr.ordinal,
                "entry_id": self.evidence.curr.entry_id,
                "off": self.evidence.curr.off,
                "len": self.evidence.curr.len,
                "line_sha256": self.evidence.curr.line_sha256,
            },
        })))
    }

    /// The Judge also depends on the candidate labels it was shown; every
    /// other generator depends on the base input alone.
    pub fn input_sha256(&self) -> String {
        if self.generator != Generator::Judge {
            return self.base_input_sha256();
        }
        sha256_hex(&canonical(&json!({
            "schema": "ase-judge-input/v1",
            "base_input_sha256": self.base_input_sha256(),
            "context": self.context.clone().unwrap_or(Value::Null),
            "prompt_version": self.rule_version,
        })))
    }

    /// Span/input-based cache key. Never contains a whole-journal digest.
    pub fn cache_key(&self) -> String {
        sha256_hex(&canonical(&json!({
            "schema": "ase-judge-assertion-cache/v1",
            "input_sha256": self.input_sha256(),
            "generator": self.generator.as_str(),
            "rule_version": self.rule_version,
            "generator_id": self.generator_id,
            "view_version": VIEW_VERSION,
            "render_version": INPUT_RENDER_VERSION,
            "verification_only": self.verification_only,
        })))
    }

    /// Content identity of the assertion itself, independent of the run.
    pub fn assertion_id(&self) -> String {
        sha256_hex(&canonical(&json!({
            "schema": "ase-judge-assertion/v1",
            "unit_id": self.unit_id,
            "decision_index": self.decision_index,
            "generator": self.generator.as_str(),
            "rule_version": self.rule_version,
            "generator_id": self.generator_id,
            "evidence": self.evidence,
            "render_sha256": self.render_sha256,
            "label": self.label.as_str(),
            "confidence": self.confidence,
            "abstain_reason": self.abstain_reason,
            "context": self.context,
        })))
    }

    /// Structural validation performed before any source read. A missing or
    /// malformed evidence pair is rejected here, at insert time.
    pub fn validate_shape(&self) -> Result<()> {
        if !is_hex64(&self.unit_id) {
            return Err(err("assertion unit id must be a 64-hex digest"));
        }
        if self.rule_version.is_empty() {
            return Err(err("assertion needs a rule or prompt version"));
        }
        if self.generator_id.is_empty() {
            return Err(err("assertion needs a generator identity"));
        }
        if !is_hex64(&self.render_sha256) {
            return Err(err("assertion needs an input render digest"));
        }
        self.evidence.prev.validate("prev")?;
        self.evidence.curr.validate("curr")?;
        if self.evidence.prev.ordinal >= self.evidence.curr.ordinal {
            return Err(err("assertion evidence is not an ordered pair"));
        }
        if self.generator == Generator::Judge && self.context.is_none() {
            return Err(err("judge assertion needs its candidate-label context"));
        }
        if self.label == Decision::Unknown && self.abstain_reason.is_none() {
            return Err(err("unknown assertion needs an abstain reason"));
        }
        Ok(())
    }
}
