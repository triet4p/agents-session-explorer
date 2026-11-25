//! Deterministic byte selectors and source reopening for bundle evidence.
//!
//! A selector names one message content block by its row span and content
//! index. Reopening slices that exact span again, re-decodes the row and
//! returns the block, so a verifier recomputes tool names, call signatures and
//! block digests from source bytes rather than from bundle metadata.

use crate::bundle_report::{fail, Check};
use crate::bundle_schema::SELECTOR_VERSION;
use serde_json::Value;

pub(crate) fn row_spans(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = 0usize;
    for (i, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            let mut end = i;
            if end > start && bytes[end - 1] == b'\r' {
                end -= 1;
            }
            spans.push((start, end - start));
            start = i + 1;
        }
    }
    if start < bytes.len() {
        let mut end = bytes.len();
        if end > start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
        spans.push((start, end - start));
    }
    spans
}

pub(crate) fn parsed_rows(bytes: &[u8]) -> Vec<Option<Value>> {
    row_spans(bytes)
        .iter()
        .map(|(off, len)| serde_json::from_slice(&bytes[*off..*off + *len]).ok())
        .collect()
}

pub(crate) fn row_index_of(off: usize, bytes: &[u8]) -> usize {
    bytes[..off.min(bytes.len())]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
}

pub(crate) fn selector_of(bytes: &[u8], row: usize, content: usize, kind: &str) -> String {
    let (off, len) = row_spans(bytes)
        .get(row)
        .copied()
        .unwrap_or((0, 0));
    format!("{SELECTOR_VERSION}|span:{off}:{len}|message.content[{content}]|{kind}")
}

fn parse_selector(selector: &str) -> Check<(usize, usize, usize, String)> {
    let parts: Vec<&str> = selector.split('|').collect();
    if parts.len() != 4 || parts[0] != SELECTOR_VERSION {
        return Err(fail("selector_mismatch", "selector version/shape unsupported"));
    }
    let span = parts[1]
        .strip_prefix("span:")
        .and_then(|s| s.split_once(':'))
        .ok_or_else(|| fail("selector_mismatch", "selector span malformed"))?;
    let off = span.0.parse::<usize>().map_err(|_| fail("selector_mismatch", "selector offset malformed"))?;
    let len = span.1.parse::<usize>().map_err(|_| fail("selector_mismatch", "selector length malformed"))?;
    let inner = parts[2]
        .strip_prefix("message.content[")
        .and_then(|s| s.strip_suffix(']'))
        .and_then(|s| s.parse::<usize>().ok())
        .ok_or_else(|| fail("selector_mismatch", "selector content index malformed"))?;
    Ok((off, len, inner, parts[3].to_string()))
}

pub(crate) fn reopen_block(bytes: &[u8], selector: &str) -> Check<(usize, Value)> {
    let (off, len, content, kind) = parse_selector(selector)?;
    if off > bytes.len() || len > bytes.len() - off {
        return Err(fail("span_out_of_range", "selector span outside source"));
    }
    let slice = &bytes[off..off + len];
    let row: Value = serde_json::from_slice(slice)
        .map_err(|_| fail("span_out_of_range", "spanned bytes are not a JSON row"))?;
    let blocks = row
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
        .ok_or_else(|| fail("span_out_of_range", "selector message content absent"))?;
    let Some(block) = blocks.get(content) else {
        return Err(fail("span_out_of_range", "selector content index absent"));
    };
    if block.get("type").and_then(Value::as_str) != Some(kind.as_str()) {
        return Err(fail("selector_mismatch", "selector block type differs"));
    }
    Ok((row_index_of(off, bytes), block.clone()))
}

pub(crate) fn derive_tool(block: &Value) -> String {
    block
        .get("toolName")
        .or_else(|| block.get("tool"))
        .or_else(|| block.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn value_class(value: &Value) -> &'static str {
    match value {
        Value::String(s) if looks_like_path(s) => "P",
        Value::String(_) => "S",
        Value::Number(_) => "N",
        Value::Bool(_) => "B",
        Value::Null => "Z",
        Value::Array(_) => "A",
        Value::Object(_) => "O",
    }
}

fn looks_like_path(text: &str) -> bool {
    text.starts_with('/')
        || text.starts_with('~')
        || text.contains('\\')
        || (text.len() > 2 && text.as_bytes()[1] == b':')
}

pub(crate) fn derive_signature(block: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(args) = block.get("arguments").and_then(Value::as_object) {
        for (key, value) in args {
            parts.push(format!("{}={}", key, value_class(value)));
        }
    }
    parts.sort();
    format!("{}|{}", derive_tool(block), parts.join(","))
}

pub(crate) fn result_status_of(block: &Value) -> &'static str {
    match block.get("status").and_then(Value::as_str) {
        Some("absent") => "absent",
        Some("nontext") => "nontext",
        _ if block.get("text").and_then(Value::as_str).is_some() => "present",
        _ => "nontext",
    }
}

pub(crate) fn result_for_call(rows: &[Option<Value>], call_row: usize, call_id: &str) -> Option<(usize, usize, Value)> {
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by_key(|&i| {
        let row = rows[i].as_ref();
        (
            row.and_then(|r| r.get("timestamp")).and_then(Value::as_str).unwrap_or("").to_string(),
            row.and_then(|r| r.get("ordinal")).and_then(Value::as_u64).unwrap_or(0),
        )
    });
    for i in order {
        if i <= call_row {
            continue;
        }
        let Some(row) = rows[i].as_ref() else { continue };
        let message = row.get("message").unwrap_or(&Value::Null);
        let role = message.get("role").and_then(Value::as_str);
        if role != Some("tool") && role != Some("toolResult") {
            continue;
        }
        let Some(content) = message.get("content").and_then(Value::as_array) else {
            continue;
        };
        // Fixture and older shapes carry the id on a toolResult content block.
        for (ci, block) in content.iter().enumerate() {
            if block.get("type").and_then(Value::as_str) == Some("toolResult")
                && block.get("toolCallId").and_then(Value::as_str) == Some(call_id)
            {
                return Some((i, ci, block.clone()));
            }
        }
        // Native OMP puts the id on the message and the text in a content block.
        if message.get("toolCallId").and_then(Value::as_str) == Some(call_id) {
            for (ci, block) in content.iter().enumerate() {
                if block.get("text").and_then(Value::as_str).is_some() {
                    return Some((i, ci, block.clone()));
                }
            }
        }
    }
    None
}
