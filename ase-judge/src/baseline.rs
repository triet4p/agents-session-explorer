//! Deterministic no-model baseline for the segmentation decision.
//!
//! The rule order follows the frozen `ase-judge-rubric/v1` priorities for the
//! signals a lexical baseline can actually observe: explicit task reset or
//! hand-off, an independent named deliverable, same-deliverable refinement,
//! acknowledgement-only messages, then abstention. Topic-change inference and
//! any referenced context outside the two visible messages are not available
//! to this baseline, so it abstains instead of guessing. The baseline never
//! reads filenames, task markers, recaps, hand labels or model output.

use crate::assertion::Decision;

/// Versioned rule identity. Changing a rule requires a new version, which
/// invalidates cached baseline assertions through the cache key.
pub const RULE_VERSION: &str = crate::assertion::BASELINE_RULE_VERSION;
pub const GENERATOR_ID: &str = crate::assertion::BASELINE_GENERATOR_ID;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaselineOutcome {
    pub label: Decision,
    pub confidence: &'static str,
    pub abstain_reason: Option<&'static str>,
    /// Rubric rationale code retained for inspection (`RC1`..`RC6`).
    pub rationale: &'static str,
}

impl BaselineOutcome {
    fn decided(label: Decision, confidence: &'static str, rationale: &'static str) -> Self {
        Self {
            label,
            confidence,
            abstain_reason: None,
            rationale,
        }
    }

    fn abstained(reason: &'static str) -> Self {
        Self {
            label: Decision::Unknown,
            confidence: "low",
            abstain_reason: Some(reason),
            rationale: "RC6",
        }
    }
}

fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !last_space && !out.is_empty() {
                out.push(' ');
            }
            last_space = true;
        } else {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            last_space = false;
        }
    }
    out.trim().to_string()
}

fn tokens(normalized: &str) -> Vec<&str> {
    normalized
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect()
}

const SPLIT_DECLARATIONS: &[&str] = &[
    "new task",
    "next task",
    "different task",
    "another task",
    "separate task",
    "new objective",
    "new goal",
    "new assignment",
    "start over",
    "start a new",
    "start a fresh",
    "start fresh",
    "switch to a different",
    "move on to",
    "hand off",
    "handoff",
    "task id",
    "task:",
    "new prompt",
    // Vietnamese equivalents of the same explicit declarations. The cohort is
    // Vietnamese; a bilingual rule baseline is still model-free.
    "task mới",
    "task khác",
    "nhiệm vụ mới",
    "việc mới",
    "task tiếp theo",
    "chuyển sang",
    "sang task",
    "bắt đầu task",
    "làm task mới",
    "giờ sang",
    "bây giờ sang",
];

fn declares_split(normalized: &str) -> bool {
    SPLIT_DECLARATIONS
        .iter()
        .any(|phrase| normalized.contains(phrase))
}

const CREATE_VERBS: &[&str] = &[
    "create", "add", "implement", "build", "write", "generate", "produce", "deliver",
    "design", "draft", "export", "publish", "tạo", "thêm", "viết", "xây", "dựng",
    "triển", "khai", "xuất", "soạn", "làm", "sửa",
];

const DELIVERABLE_WORDS: &[&str] = &[
    "report", "readme", "changelog", "artifact", "deck", "slide", "slides", "diagram",
    "document", "doc", "file", "script", "test", "tests", "patch", "release", "summary",
    "liệu", "cáo", "biểu", "hoạch", "nháp", "kế",
];

fn artifact_tokens(normalized: &str) -> Vec<String> {
    tokens(normalized)
        .into_iter()
        .filter(|token| {
            token.contains('/')
                || token.contains('\\')
                || token.contains('.')
                || DELIVERABLE_WORDS.contains(token)
        })
        .map(str::to_string)
        .collect()
}

/// Rubric RC2: the later message introduces an independent, named deliverable
/// that the earlier message does not share, and it asks for it to be created.
fn introduces_deliverable(prev_normalized: &str, curr_normalized: &str) -> bool {
    let curr_tokens: Vec<&str> = tokens(curr_normalized);
    if !curr_tokens
        .iter()
        .any(|token| CREATE_VERBS.contains(token))
    {
        return false;
    }
    let prev_tokens: Vec<&str> = tokens(prev_normalized);
    artifact_tokens(curr_normalized)
        .into_iter()
        .any(|token| !prev_tokens.contains(&token.as_str()))
}

const CONTINUE_CUES: &[&str] = &[
    "yes", "no", "continue", "keep", "same", "still", "again", "instead", "also", "fix",
    "change", "update", "refine", "adjust", "correct", "improve", "review", "test", "run",
    "try", "this", "that", "it", "these", "those", "please", "add", "remove", "rename",
    "move", "use", "make", "need", "want", "now", "then", "next", "commit", "push",
    "merge", "build", "deploy",
    // Vietnamese refinement/action cues.
    "tiếp", "tục", "sửa", "đổi", "thêm", "bớt", "xoá", "xóa", "cập", "nhật", "chỉnh",
    "kiểm", "tra", "chạy", "thử", "lại", "giúp", "ổn", "chuẩn", "đúng",
];

const ACK_WORDS: &[&str] = &[
    "thanks", "thank", "thx", "great", "ok", "okay", "cool", "nice", "perfect", "got",
    "understood", "sure", "good", "fine", "awesome",
    "cảm", "ơn", "cám", "tốt", "hay", "tuyệt", "chuẩn", "ổn", "được", "ừ", "vâng",
    "dạ", "oke", "okie", "okela",
];

fn is_acknowledgement_only(normalized: &str) -> bool {
    let tokens = tokens(normalized);
    !tokens.is_empty() && tokens.len() <= 4 && tokens.iter().all(|t| ACK_WORDS.contains(t))
}

fn has_continue_cue(curr_normalized: &str) -> bool {
    tokens(curr_normalized)
        .iter()
        .any(|token| CONTINUE_CUES.contains(token))
}

/// Classifies one adjacent pair. Deterministic for the same input bytes and
/// rule version.
pub fn classify(prev_text: &str, curr_text: &str) -> BaselineOutcome {
    let prev = normalize(prev_text);
    let curr = normalize(curr_text);
    if curr.trim().is_empty() || prev.trim().is_empty() {
        return BaselineOutcome::abstained("unusable_message_text");
    }
    if declares_split(&curr) {
        return BaselineOutcome::decided(Decision::Split, "high", "RC1");
    }
    if introduces_deliverable(&prev, &curr) {
        return BaselineOutcome::decided(Decision::Split, "medium", "RC2");
    }
    if is_acknowledgement_only(&curr) {
        return BaselineOutcome::abstained("acknowledgement_only");
    }
    if has_continue_cue(&curr) {
        return BaselineOutcome::decided(Decision::Continue, "medium", "RC4");
    }
    BaselineOutcome::abstained("no_decisive_rule")
}
