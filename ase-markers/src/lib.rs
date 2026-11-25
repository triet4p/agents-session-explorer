//! Evidence-backed observation of `task_id=` markers in allowlisted prose.
//!
//! This crate is pure text logic. It never opens files, reads raw bytes,
//! consults an index or a model, emits a task key, groups by session or
//! assigns anything. It classifies each marker occurrence inside one already
//! allowlisted prose block as an assignment-context signal, a mere mention or
//! ambiguous, and keeps its in-block position so the caller can attach the
//! versioned block selector and generation evidence. Reconciliation across
//! signals is deliberately absent here.

/// Stable identifier of the derivation rule recorded on every observation.
pub const RULE_NAME: &str = "task-id-marker";
/// Bump when the matcher, cue taxonomy, window or status meanings change.
pub const RULE_VERSION: &str = "marker-v1";
/// The exact prose key this rule scans for.
pub const MATCHER: &str = "task_id=";

/// Characters before the marker used for cue context, in Unicode scalar values.
const CONTEXT_BEFORE: usize = 160;
/// Characters after the marker used for cue context, in Unicode scalar values.
const CONTEXT_AFTER: usize = 80;
/// Upper bound on one marker value, in bytes.
const MAX_VALUE_BYTES: usize = 128;

/// Assignment cues, matched as token sequences in the lowercased local window.
/// "task" is intentionally absent: the marker itself tokenizes to `task id`.
const ASSIGNMENT_CUES: &[&[&str]] = &[
    &["please"],
    &["assign"],
    &["assigned"],
    &["assignment"],
    &["implement"],
    &["implements"],
    &["implemented"],
    &["implementation"],
    &["build"],
    &["building"],
    &["fix"],
    &["fixes"],
    &["fixed"],
    &["complete"],
    &["completed"],
    &["handle"],
    &["handles"],
    &["finish"],
    &["finished"],
    &["solve"],
    &["solved"],
    &["resolve"],
    &["resolved"],
    &["deliver"],
    &["develop"],
    &["deploy"],
    &["write"],
    &["create"],
    &["work", "on"],
    &["take", "on"],
    &["start"],
    &["begin"],
    &["proceed"],
    &["continue"],
    &["run"],
    &["own"],
];

/// Mention/reference cues. Their presence without an assignment cue makes the
/// occurrence a mention, never assignment evidence.
const MENTION_CUES: &[&[&str]] = &[
    &["mention"],
    &["mentioned"],
    &["mentions"],
    &["refer"],
    &["reference"],
    &["referenced"],
    &["fyi"],
    &["note"],
    &["noted"],
    &["notes"],
    &["previously"],
    &["already"],
    &["example"],
    &["examples"],
    &["related"],
    &["discussed"],
    &["background"],
    &["context"],
    &["read"],
    &["seen"],
    &["aware"],
    &["see", "also"],
    &["just", "so", "you", "know"],
    &["for", "reference"],
    &["in", "case"],
    &["e", "g"],
    &["i", "e"],
];

/// Strength of one marker occurrence. Every variant remains a candidate
/// signal; none of them is a task assignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Usage {
    /// An assignment cue was found without a mention cue in the local window.
    AssignmentContext,
    /// Only mention/reference cues were found in the local window.
    Mention,
    /// Neither or both cue classes were found; retain but abstain.
    Ambiguous,
}

impl Usage {
    pub fn label(self) -> &'static str {
        match self {
            Usage::AssignmentContext => "assignment_context",
            Usage::Mention => "mention",
            Usage::Ambiguous => "ambiguous",
        }
    }
}

/// One already-allowlisted prose block. The caller re-derives `text` from raw
/// bytes at the pinned span and attaches the block's versioned selector; no
/// locator is stored or invented by this crate.
#[derive(Clone, Debug)]
pub struct Block {
    pub text: String,
}

/// One `task_id=` occurrence with its local classification and in-block
/// position. `block_index` points into the audited block slice.
#[derive(Clone, Debug)]
pub struct Observation {
    pub block_index: usize,
    pub value: String,
    pub usage: Usage,
    pub reason: &'static str,
    pub assignment_cue: Option<String>,
    pub mention_cue: Option<String>,
    /// Occurrence order inside this block, starting at zero.
    pub occurrence_in_block: u32,
    /// Inclusive Unicode-scalar offset of the marker key inside the block.
    pub char_start: u32,
    /// Exclusive Unicode-scalar offset of the marker value inside the block.
    pub char_end: u32,
}

#[derive(Clone, Debug)]
pub struct RuleInfo {
    pub name: String,
    pub version: String,
    pub matcher: String,
    pub domain: String,
    pub abstention: String,
    pub task_boundary: String,
}

impl Default for RuleInfo {
    fn default() -> Self {
        Self {
            name: RULE_NAME.to_string(),
            version: RULE_VERSION.to_string(),
            matcher: MATCHER.to_string(),
            domain: "A task_id= marker is scanned only in already-allowlisted user/assistant prose blocks of one published generation. Every occurrence keeps its in-block position for the caller's versioned selector and block digest. Tool output, thinking, system, unknown blocks and call signatures are never scanned.".to_string(),
            abstention: "An occurrence with no cue, or with both assignment and mention cues in its local window, stays ambiguous/abstain. A marker is never promoted by another occurrence, by being first in a block, or by being the only marker in a session.".to_string(),
            task_boundary: "This rule emits occurrence-level observations only. It never emits a TaskKey, an assigned_to edge, a session-level assignment, a chronology or a merge across IDs; reconciling this signal with naming and delegation is a separate layer.".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Audit {
    pub rule: RuleInfo,
    pub blocks_scanned: usize,
    pub occurrences: usize,
    pub assignment_context: usize,
    pub mention: usize,
    pub ambiguous: usize,
    pub observations: Vec<Observation>,
}

fn is_value_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn key_at(bytes: &[u8], at: usize) -> bool {
    let key = MATCHER.as_bytes();
    if at + key.len() > bytes.len() {
        return false;
    }
    bytes[at..at + key.len()]
        .iter()
        .zip(key)
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn first_cue(tokens: &[&str], cues: &[&[&str]]) -> Option<String> {
    for cue in cues {
        if cue.len() <= tokens.len() && tokens.windows(cue.len()).any(|w| w == *cue) {
            return Some(cue.join(" "));
        }
    }
    None
}

fn classify(
    chars: &[char],
    char_start: usize,
    char_end: usize,
    window_limit: usize,
) -> (Usage, &'static str, Option<String>, Option<String>) {
    let mut start = char_start.saturating_sub(CONTEXT_BEFORE);
    // Only skip a leading fragment when the window was actually cut inside a
    // word; at offset 0 nothing was sliced and every token must stay visible.
    if start > 0
        && chars
            .get(start - 1)
            .is_some_and(|c| c.is_alphanumeric() || *c == '_')
    {
        while start < char_start
            && chars
                .get(start)
                .is_some_and(|c| c.is_alphanumeric() || *c == '_')
        {
            start += 1;
        }
    }
    let end = (char_end + CONTEXT_AFTER)
        .min(window_limit)
        .min(chars.len());
    let window: String = chars[start..end].iter().collect::<String>().to_lowercase();
    let tokens: Vec<&str> = window
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let assignment = first_cue(&tokens, ASSIGNMENT_CUES);
    let mention = first_cue(&tokens, MENTION_CUES);
    match (assignment.is_some(), mention.is_some()) {
        (true, true) => (
            Usage::Ambiguous,
            "assignment and mention cues both present in the local context; abstain",
            assignment,
            mention,
        ),
        (true, false) => (
            Usage::AssignmentContext,
            "assignment cue present without a mention cue; supported marker signal only",
            assignment,
            None,
        ),
        (false, true) => (
            Usage::Mention,
            "mention cue present without an assignment cue; not assignment evidence",
            None,
            mention,
        ),
        (false, false) => (
            Usage::Ambiguous,
            "no assignment or mention cue in the local context; abstain as an unclassified candidate",
            None,
            None,
        ),
    }
}

fn scan(text: &str, block_index: usize, out: &mut Vec<Observation>) {
    let bytes = text.as_bytes();
    let chars: Vec<char> = text.chars().collect();
    let mut sites: Vec<(String, usize, usize)> = Vec::new();
    let mut i = 0usize;
    let mut char_idx = 0usize;
    while i < bytes.len() {
        if key_at(bytes, i) && (i == 0 || !is_word_byte(bytes[i - 1])) {
            let value_start = i + MATCHER.len();
            let mut value_end = value_start;
            while value_end < bytes.len()
                && is_value_byte(bytes[value_end])
                && value_end - value_start < MAX_VALUE_BYTES
            {
                value_end += 1;
            }
            while value_end > value_start
                && matches!(bytes[value_end - 1], b'.' | b'-' | b'_')
            {
                value_end -= 1;
            }
            if value_end > value_start {
                let value =
                    String::from_utf8_lossy(&bytes[value_start..value_end]).into_owned();
                let char_start = char_idx;
                // The matcher and the value are ASCII, so byte length is char length.
                let char_end = char_start + MATCHER.len() + (value_end - value_start);
                sites.push((value, char_start, char_end));
                i = value_end;
                char_idx = char_end;
                continue;
            }
        }
        // UTF-8 continuation bytes do not advance the Unicode-scalar index.
        if bytes[i] & 0xC0 != 0x80 {
            char_idx += 1;
        }
        i += 1;
    }
    for (index, (value, char_start, char_end)) in sites.iter().enumerate() {
        // A later marker's own cue text must not leak into this window.
        let window_limit = sites.get(index + 1).map_or(chars.len(), |next| next.1);
        let (usage, reason, assignment_cue, mention_cue) =
            classify(&chars, *char_start, *char_end, window_limit);
        out.push(Observation {
            block_index,
            value: value.clone(),
            usage,
            reason,
            assignment_cue,
            mention_cue,
            occurrence_in_block: index as u32,
            char_start: *char_start as u32,
            char_end: *char_end as u32,
        });
    }
}

/// Classify every `task_id=` occurrence in already-allowlisted prose blocks.
pub fn audit(blocks: &[Block]) -> Audit {
    let mut out = Audit {
        rule: RuleInfo::default(),
        blocks_scanned: blocks.len(),
        occurrences: 0,
        assignment_context: 0,
        mention: 0,
        ambiguous: 0,
        observations: Vec::new(),
    };
    for (index, block) in blocks.iter().enumerate() {
        scan(&block.text, index, &mut out.observations);
    }
    out.occurrences = out.observations.len();
    for observation in &out.observations {
        match observation.usage {
            Usage::AssignmentContext => out.assignment_context += 1,
            Usage::Mention => out.mention += 1,
            Usage::Ambiguous => out.ambiguous += 1,
        }
    }
    out
}
