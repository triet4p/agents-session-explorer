//! Small, deterministic text extractors for stock journals.
//!
//! These extractors never store prose. They return only the labels/tokens the
//! claim layer needs, together with the byte offset inside the supplied block
//! so evidence points at the exact occurrence.

const CONTEXT_KEYWORDS: &[&str] = &[
    "task", "attempt", "sprint", "handoff", "task_id", "target", "ticket", "work item",
];

const DENY_PREFIXES: &[&str] = &[
    "SHA", "MD5", "UTF", "ISO", "HTTP", "HTTPS", "LTS", "RGB", "RGBA", "HSL", "INT", "FP",
    "BF", "USD", "TCP", "UDP", "TLS", "SSL", "JSON", "YAML", "TOML", "CSS", "HTML", "URL",
    "API", "SDK", "JWT", "UUID", "AES", "RSA", "GPU", "CPU", "RAM", "SSD", "HDD", "PDF",
    "CSV", "PNG", "JPEG", "GIF", "MP4", "ZIP", "TAR", "GZ", "ARM", "X86", "AMD", "EG",
    "INVOKE", "GET", "SET", "NEW", "REMOVE", "START", "STOP",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenHit {
    pub token: String,
    pub offset: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnHit {
    pub name: String,
    pub offset: usize,
    /// `spawn` for `Spawned agent ...` / `(job ...)`, `task_result` for
    /// `<task-result id="...">`, `failed` for `Task NAME failed ...`.
    pub shape: &'static str,
    pub status: Option<String>,
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

fn is_date_like(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 3
        && parts[0].len() == 4
        && parts[0].bytes().all(|b| b.is_ascii_digit())
        && parts[1].len() == 2
        && parts[1].bytes().all(|b| b.is_ascii_digit())
        && parts[2].len() == 2
        && parts[2].bytes().all(|b| b.is_ascii_digit())
}

fn is_all_hex(s: &str) -> bool {
    s.len() >= 6 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn has_upper(s: &str) -> bool {
    s.bytes().any(|b| b.is_ascii_uppercase())
}

fn has_digit(s: &str) -> bool {
    s.bytes().any(|b| b.is_ascii_digit())
}

fn segments_ok(s: &str) -> bool {
    let parts: Vec<&str> = s.split(['-', '_']).filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 || parts.len() > 4 {
        return false;
    }
    if parts.iter().any(|p| !p.bytes().all(|b| b.is_ascii_alphanumeric())) {
        return false;
    }
    // Every segment is an identifier-like run; at least one carries a letter
    // and one carries a digit, and at least one segment mixes both (S3, T1).
    let letters = parts.iter().any(|p| has_upper(p) || p.bytes().any(|b| b.is_ascii_lowercase()));
    let digits = parts.iter().any(|p| has_digit(p));
    let mixed = parts.iter().any(|p| has_upper(p) && has_digit(p));
    letters && digits && mixed
}

fn short_label_ok(s: &str) -> bool {
    // P0a, P1b, S3a style: a short prefix, digits, one optional lowercase
    // suffix. Rejects FP16/BF16 because those have no uppercase after digits
    // and more than two digits.
    let bytes = s.as_bytes();
    if bytes.len() < 3 || bytes.len() > 8 {
        return false;
    }
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_uppercase() {
        i += 1;
    }
    if i == 0 || i > 3 {
        return false;
    }
    let digits_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == digits_start || i - digits_start > 2 {
        return false;
    }
    if i >= bytes.len() {
        // A no-separator label must carry a lowercase suffix (P0a, P0c);
        // uppercase+digit abbreviations such as A02/EG4 are too generic.
        return false;
    }
    i + 1 == bytes.len() && bytes[i].is_ascii_lowercase()
}

fn denied(token: &str) -> bool {
    if is_date_like(token) || is_all_hex(token) {
        return true;
    }
    let head = token
        .split(['-', '_'])
        .next()
        .unwrap_or(token)
        .to_ascii_uppercase();
    if DENY_PREFIXES.iter().any(|d| *d == head) {
        return true;
    }
    // Pure numeric or pure alphabetic runs are not task identifiers.
    if !has_upper(token) || !has_digit(token) {
        return true;
    }
    false
}

fn plausible_token(token: &str) -> bool {
    if token.len() < 3 || token.len() > 48 || denied(token) {
        return false;
    }
    let separated = token.contains('-') || token.contains('_');
    if separated {
        segments_ok(token)
    } else {
        short_label_ok(token)
    }
}

fn context_near(text: &str, offset: usize, len: usize) -> bool {
    let mut start = offset.saturating_sub(160);
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (offset + len + 160).min(text.len());
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }
    let window = text[start..end].to_ascii_lowercase();
    CONTEXT_KEYWORDS.iter().any(|k| window.contains(k))
}

/// Extract conservative task identifier candidates. The extractor requires a
/// nearby assignment/mention keyword so colour codes, hashes and library
/// version fragments in handoffs do not become task clues.
pub fn extract_task_tokens(text: &str) -> Vec<TokenHit> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !(bytes[i] as char).is_ascii_alphanumeric() {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_token_char(bytes[i] as char) {
            i += 1;
        }
        let raw = &text[start..i];
        let token = raw.trim_matches(['-', '_']);
        if token.len() != raw.len() {
            continue;
        }
        if plausible_token(token) && context_near(text, start, token.len()) {
            out.push(TokenHit { token: token.to_string(), offset: start });
        }
    }
    out
}

/// Extract `task_id=` / `task_id:` markers. Values use the same conservative
/// identifier rule, but do not require a second keyword because the marker
/// itself is the assignment cue.
pub fn extract_task_id_markers(text: &str) -> Vec<TokenHit> {
    let mut out = Vec::new();
    let lower = text.to_ascii_lowercase();
    let mut from = 0usize;
    while let Some(pos) = lower[from..].find("task_id") {
        let at = from + pos;
        let mut j = at + "task_id".len();
        let tail = &text[j..];
        let skip = tail
            .char_indices()
            .find(|(_, c)| !matches!(c, ' ' | '\t' | '`' | '"' | '\'' | ':' | '='))
            .map(|(i, _)| i)
            .unwrap_or(tail.len());
        j += skip;
        let rest = &text[j..];
        let end = rest
            .char_indices()
            .find(|(_, c)| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_')))
            .map(|(i, _)| i)
            .unwrap_or(rest.len());
        let raw = rest[..end].trim_matches(['-', '_']);
        if plausible_token(raw) {
            out.push(TokenHit { token: raw.to_string(), offset: j });
        }
        from = at + "task_id".len();
        if from >= lower.len() {
            break;
        }
    }
    out
}

fn valid_spawn_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn push_unique(out: &mut Vec<SpawnHit>, hit: SpawnHit) {
    if valid_spawn_name(&hit.name) && !out.iter().any(|h| h.name == hit.name) {
        out.push(hit);
    }
}

/// Parse child names out of a `task` tool result. Stock OMP has used at least
/// three result shapes:
///
/// - `Spawned agent \`Name\` (job \`Name\`)`
/// - `Spawned N background agents ... - \`Name\` (job \`Name\`)`
/// - `<task-result id="Name" agent="..." status="...">`
///
/// Failed preflight results name the intended task but spawn no child.
pub fn extract_spawns(text: &str) -> Vec<SpawnHit> {
    let mut out = Vec::new();

    // (job `Name`) is the most stable field across spawn summaries.
    let mut from = 0usize;
    while let Some(pos) = text[from..].find("(job `") {
        let at = from + pos + "(job `".len();
        if let Some(end) = text[at..].find('`') {
            let name = text[at..at + end].trim();
            if !name.is_empty() {
                push_unique(
                    &mut out,
                    SpawnHit {
                        name: name.to_string(),
                        offset: at,
                        shape: "spawn",
                        status: None,
                    },
                );
            }
            from = at + end;
        } else {
            break;
        }
    }

    // `Spawned agent `Name`` (older single-child summary).
    let mut from = 0usize;
    while let Some(pos) = text[from..].find("Spawned agent `") {
        let at = from + pos + "Spawned agent `".len();
        if let Some(end) = text[at..].find('`') {
            let name = text[at..at + end].trim();
            if !name.is_empty() {
                push_unique(
                    &mut out,
                    SpawnHit {
                        name: name.to_string(),
                        offset: at,
                        shape: "spawn",
                        status: None,
                    },
                );
            }
            from = at + end;
        } else {
            break;
        }
    }

    // `<task-result id="Name" ... status="...">` (newer auto-delivered result).
    let mut from = 0usize;
    while let Some(pos) = text[from..].find("<task-result") {
        let tag_start = from + pos;
        let tag_end = text[tag_start..]
            .find('>')
            .map(|e| tag_start + e)
            .unwrap_or(text.len());
        let tag = &text[tag_start..tag_end];
        if let Some(id_at) = tag.find("id=\"") {
            let start = id_at + 4;
            if let Some(end) = tag[start..].find('"') {
                let name = tag[start..start + end].trim();
                let status = tag
                    .find("status=\"")
                    .and_then(|s| tag[s + 8..].find('"').map(|e| tag[s + 8..s + 8 + e].to_string()));
                if !name.is_empty() {
                    push_unique(
                        &mut out,
                        SpawnHit {
                            name: name.to_string(),
                            offset: tag_start + start,
                            shape: "task_result",
                            status,
                        },
                    );
                }
            }
        }
        from = tag_end.max(tag_start + 1);
    }

    // `Task Name failed preflight` names a task but creates no child journal.
    if let Some(pos) = text.find(" failed") {
        let head = &text[..pos];
        for prefix in ["Task ", "task "] {
            if let Some(p) = head.rfind(prefix) {
                let name = head[p + prefix.len()..].trim();
                if !name.is_empty()
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                {
                    push_unique(
                        &mut out,
                        SpawnHit {
                            name: name.to_string(),
                            offset: p + prefix.len(),
                            shape: "failed",
                            status: None,
                        },
                    );
                }
                break;
            }
        }
    }

    out
}

/// Classify a corroborated token so a task identity is never conflated with
/// an attempt, review, checkpoint or dependency label.
pub fn classify_token(token: &str) -> &'static str {
    let parts: Vec<&str> = token.split(['-', '_']).collect();
    let mut kind = "other";
    for part in &parts {
        let upper = part.to_ascii_uppercase();
        let digit_tail = |rest: &str| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit());
        if upper == "DEP" || upper.starts_with("DEP") && digit_tail(&upper[3..]) {
            return "dependency";
        }
        if upper.starts_with("DIAG") && digit_tail(&upper[4..]) {
            return "attempt";
        }
        if upper.starts_with("BSR") && digit_tail(&upper[3..]) {
            return "review";
        }
        if upper.starts_with("BS") && digit_tail(&upper[2..]) {
            return "baseline";
        }
        if upper.starts_with("DR") && digit_tail(&upper[2..]) {
            return "attempt";
        }
        if upper.starts_with('D') && digit_tail(&upper[1..]) {
            return "review";
        }
        if upper.starts_with("SPRINT") {
            return "other";
        }
        if upper.starts_with("CP") && digit_tail(&upper[2..]) {
            kind = "checkpoint";
        } else if upper.starts_with("CR") && digit_tail(&upper[2..]) {
            kind = "checkpoint_review";
        } else if upper.starts_with('R') && digit_tail(&upper[1..]) && kind == "other" {
            kind = "review";
        } else if upper.starts_with('A') && digit_tail(&upper[1..]) && kind == "other" {
            kind = "attempt";
        } else if upper.starts_with('C') && digit_tail(&upper[1..]) && kind == "other" {
            kind = "checkpoint";
        }
    }
    if kind != "other" {
        return kind;
    }
    if parts.iter().any(|p| *p == "T" || p.eq_ignore_ascii_case("Task")) || token.contains("-T") {
        return "task";
    }
    if token.bytes().any(|b| b.is_ascii_digit())
        && token.bytes().any(|b| b.is_ascii_alphabetic())
    {
        "task"
    } else {
        "other"
    }
}

/// Absolute path mentions used as repo/path context. A path mention alone is
/// never a repo claim; it is context for the workspace claim.
pub fn extract_path_mentions(text: &str) -> Vec<TokenHit> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        let start = if c.is_ascii_alphabetic()
            && i + 2 < bytes.len()
            && bytes[i + 1] == b':'
            && matches!(bytes[i + 2], b'\\' | b'/')
        {
            Some(i)
        } else if c == '/' && i + 1 < bytes.len() && (bytes[i + 1] as char).is_ascii_alphabetic() {
            // POSIX absolute path; skip URI prefixes below.
            Some(i)
        } else {
            None
        };
        let Some(start) = start else {
            i += 1;
            continue;
        };
        if start > 0 && bytes[start - 1] == b'/' {
            i += 1;
            continue;
        }
        let mut j = start;
        while j < bytes.len() {
            let b = bytes[j];
            if b.is_ascii_whitespace()
                || matches!(b, b'"' | b'\'' | b'`' | b'<' | b'>' | b'|' | b')' | b'(' | b',' | b';' | b']' | b'[')
            {
                break;
            }
            j += 1;
        }
        let mut raw = text[start..j].trim_end_matches(['.', ':', '!', '?']);
        while raw.ends_with('/') || raw.ends_with('\\') {
            raw = &raw[..raw.len() - 1];
        }
        if raw.contains("://") || raw.len() < 4 {
            i = j.max(start + 1);
            continue;
        }
        out.push(TokenHit { token: raw.to_string(), offset: start });
        i = j.max(start + 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_structured_task_ids_and_rejects_technical_tokens() {
        let text = "Target: ASE-S3-T1 attempt A1. Use SHA-256 and colour E2F2E9; P0a handoff.";
        let tokens: Vec<String> = extract_task_tokens(text)
            .into_iter()
            .map(|t| t.token)
            .collect();
        assert!(tokens.contains(&"ASE-S3-T1".to_string()));
        assert!(tokens.contains(&"P0a".to_string()));
        assert!(!tokens.contains(&"SHA-256".to_string()));
        assert!(!tokens.contains(&"E2F2E9".to_string()));
    }

    #[test]
    fn parses_spawn_and_failed_task_result_shapes() {
        let multi = "Spawned 2 background agents using designer.\n- `A1` (job `A1`)\n- `B2` (job `B2`)";
        let names: Vec<String> = extract_spawns(multi).into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["A1".to_string(), "B2".to_string()]);

        let single = "Spawned agent `C3` (job `C3`).";
        assert_eq!(extract_spawns(single)[0].name, "C3");

        let nested = "<task-result id=\"D4\" agent=\"gold-task\" status=\"completed\">ok</task-result>";
        let hits = extract_spawns(nested);
        assert_eq!(hits[0].name, "D4");
        assert_eq!(hits[0].shape, "task_result");
        assert_eq!(hits[0].status.as_deref(), Some("completed"));

        let failed = "Task S54EvidenceR1 failed preflight: Error: nope";
        let hits = extract_spawns(failed);
        assert_eq!(hits[0].name, "S54EvidenceR1");
        assert_eq!(hits[0].shape, "failed");
    }
}
