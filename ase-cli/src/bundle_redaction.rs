//! Redaction policy `ase-redact-v1` and excerpt rendering.
//!
//! The exporter applies this policy once when it writes evidence and the
//! verifier re-applies it to reopened source text, so both share one ordered
//! pattern table, one truncation rule and one excerpt layout.

use regex::Regex;
use std::sync::LazyLock;

fn patterns() -> &'static Vec<(Regex, &'static str)> {
    static PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
        let raw: [(&str, &str); 8] = [
            (r#"[A-Za-z][A-Za-z0-9+.-]*://[^\s"'<>\x60]+"#, "«url»"),
            (r#"[A-Za-z]:[\\/][^\s"'<>\x60|]+"#, "«path»"),
            (r#"\\\\[^\s"'<>\x60|]+"#, "«path»"),
            (r#"/(?:home|Users|root|tmp|var|etc|opt|mnt|workspace|data)/[^\s"'<>\x60|]+"#, "«path»"),
            (r#"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"#, "«email»"),
            (r#"(?:sk-|ghp_|github_pat_|AKIA|ASIA|xox[abp]-)[A-Za-z0-9_\-]{8,}"#, "«secret»"),
            (r#"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}"#, "«secret»"),
            (r#"[A-Za-z0-9+/=_-]{48,}"#, "«token»"),
        ];
        raw.iter()
            .map(|(source, marker)| {
                (
                    Regex::new(source).expect("fixed redaction pattern"),
                    *marker,
                )
            })
            .collect()
    });
    &PATTERNS
}

fn replace_filtered<F>(text: &str, re: &Regex, marker: &str, allowed: F) -> String
where
    F: Fn(&str, usize) -> bool,
{
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    for m in re.find_iter(text) {
        if !allowed(text, m.start()) {
            continue;
        }
        out.push_str(&text[last..m.start()]);
        out.push_str(marker);
        last = m.end();
    }
    out.push_str(&text[last..]);
    out
}

fn word_boundary(text: &str, at: usize) -> bool {
    text[..at]
        .chars()
        .next_back()
        .is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_')
}

fn strict_boundary(text: &str, at: usize) -> bool {
    text[..at].chars().next_back().is_none_or(|c| {
        !c.is_ascii_alphanumeric() && c != '_' && c != '.' && c != '-'
    })
}

fn redact(text: &str) -> String {
    let p = patterns();
    let mut out = p[0].0.replace_all(text, p[0].1).into_owned();
    // The table is applied in order, one left-to-right pass per pattern.
    out = replace_filtered(&out, &p[1].0, p[1].1, word_boundary);
    out = p[2].0.replace_all(&out, p[2].1).into_owned();
    out = replace_filtered(&out, &p[3].0, p[3].1, strict_boundary);
    for (re, marker) in &p[4..] {
        out = re.replace_all(&out, *marker).into_owned();
    }
    out
}


fn truncate_line(line: &str) -> String {
    if line.len() <= 160 {
        return line.to_string();
    }
    let mut cut = 0usize;
    for (i, ch) in line.char_indices() {
        if i + ch.len_utf8() > 157 {
            break;
        }
        cut = i + ch.len_utf8();
    }
    let mut out = line[..cut].to_string();
    out.push('…');
    out
}

pub(crate) fn cap_signature(signature: &str) -> String {
    truncate_line(signature)
}

pub(crate) fn redaction_lines(text: &str) -> Vec<String> {
    let redacted = redact(text);
    if redacted.is_empty() {
        return Vec::new();
    }
    redacted
        .split('\n')
        .take(3)
        .map(|line| truncate_line(line.strip_suffix('\r').unwrap_or(line)))
        .collect()
}

pub(crate) fn render_excerpt(
    ordinal: u64,
    tool: &str,
    signature: &str,
    status: &str,
    lines: &[String],
) -> String {
    let mut out = format!("call[{ordinal}] tool={tool} sig={signature}\n");
    match status {
        "absent" => out.push_str("result: «absent»"),
        "nontext" => out.push_str("result: «nontext»"),
        _ if lines.is_empty() => out.push_str("result: «empty»"),
        _ => {
            for (i, line) in lines.iter().enumerate() {
                if i == 0 {
                    out.push_str("result: ");
                } else {
                    out.push_str("        ");
                }
                out.push_str(line);
                if i + 1 < lines.len() {
                    out.push('\n');
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_applies_normative_markers_in_order() {
        let text = "go https://x.invalid/a C:\\tmp\\a /workspace/b a@b.invalid sk-abcdefghij\n";
        assert_eq!(
            redact(text),
            "go «url» «path» «path» «email» «secret»\n"
        );
    }

    #[test]
    fn truncation_keeps_utf8_boundary_at_or_below_157_bytes() {
        let line = format!("{}é", "x".repeat(160));
        let out = truncate_line(&line);
        assert_eq!(out.len(), 160);
        assert!(out.ends_with('…'));
    }
}
