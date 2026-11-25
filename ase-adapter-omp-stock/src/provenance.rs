//! Evidence and provenance construction for OMP stock adapter claims.
//!
//! Every claim field that can be re-verified against a source journal is built
//! here; no claim state decision is made in this module.

use serde_json::Value;
use std::collections::HashSet;

use crate::extract::extract_task_tokens;
use crate::journal::{AssignSource, TokenClue};
use crate::model::*;

pub(crate) fn make_evidence(
    file: &ViewFile,
    generation: &str,
    event: &ViewEvent,
    selector: &str,
    line: &[u8],
    block: &[u8],
) -> EvidenceRef {
    EvidenceRef {
        provider: file.provider.label().to_string(),
        generation_id: generation.to_string(),
        coordinate: file.encoding.coordinate().to_string(),
        file_id: Some(file.id),
        path: file.path.clone(),
        event: Some(event.event),
        selector: selector.to_string(),
        selector_version: SELECTOR_VERSION.to_string(),
        span: event.span.into(),
        span_checksum: Some(format!("{:016x}", event.hash)),
        line_digest: sha256_hex(line),
        block_digest: Some(sha256_hex(block)),
        manifest_digest: file.manifest_digest.clone(),
    }
}

pub(crate) fn path_evidence(file: &ViewFile, generation: &str, field: &str) -> EvidenceRef {
    let selector = format!("{SELECTOR_VERSION}|path.{field}");
    let normalized = path_key(&file.path);
    EvidenceRef {
        provider: file.provider.label().to_string(),
        generation_id: generation.to_string(),
        coordinate: file.encoding.coordinate().to_string(),
        file_id: Some(file.id),
        path: file.path.clone(),
        event: None,
        selector,
        selector_version: SELECTOR_VERSION.to_string(),
        span: SpanJson { file: file.id, off: 0, len: 0 },
        span_checksum: None,
        line_digest: sha256_hex(b""),
        block_digest: Some(sha256_hex(normalized.as_bytes())),
        manifest_digest: file.manifest_digest.clone(),
    }
}

pub(crate) fn sidecar_evidence(
    generation: &str,
    path: &str,
    token: &str,
    bytes: &[u8],
    offset: usize,
) -> EvidenceRef {
    // The sidecar is not indexed. Keep a bounded context window so reopen can
    // re-run the same context-sensitive token extractor, never a guessed line.
    let text = String::from_utf8_lossy(bytes);
    let mut start = offset.saturating_sub(160);
    while start > 0 && !text.is_char_boundary(start.min(text.len())) {
        start -= 1;
    }
    let mut end = (offset + token.len() + 160).min(text.len());
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }
    let window = &bytes[start..end];
    EvidenceRef {
        provider: "omp".to_string(),
        generation_id: generation.to_string(),
        coordinate: "raw_file_bytes".to_string(),
        file_id: None,
        path: path.to_string(),
        event: None,
        selector: selector_sidecar_token(token),
        selector_version: SELECTOR_VERSION.to_string(),
        span: SpanJson { file: 0, off: start as u64, len: (end - start) as u32 },
        span_checksum: None,
        line_digest: sha256_hex(window),
        block_digest: Some(sha256_hex(token.as_bytes())),
        manifest_digest: Some(sha256_hex(bytes)),
    }
}

pub(crate) fn assignment_token_selector(source: AssignSource, token: &str) -> String {
    match source {
        AssignSource::Content(index) => selector_arguments_token(index, token),
        AssignSource::DataArgs => selector_data_args_token(token),
    }
}

pub(crate) fn retarget(ev: &EvidenceRef, selector: String, block: &[u8]) -> EvidenceRef {
    let mut out = ev.clone();
    out.selector = selector;
    out.block_digest = Some(sha256_hex(block));
    out
}

pub(crate) fn values_in_json_strings(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => out.push(s.clone()),
            Value::Array(items) => {
                for item in items {
                    walk(item, out);
                }
            }
            Value::Object(map) => {
                for item in map.values() {
                    walk(item, out);
                }
            }
            _ => {}
        }
    }
    walk(value, &mut out);
    out
}

pub(crate) fn token_clues_from_value(
    value: &Value,
    source_field: &str,
    source_kind: &str,
    file: &ViewFile,
    generation: &str,
    event: &ViewEvent,
    line: &[u8],
    selector: &str,
    block: &[u8],
) -> Vec<TokenClue> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for text in values_in_json_strings(value) {
        for hit in extract_task_tokens(&text) {
            if !seen.insert(hit.token.clone()) {
                continue;
            }
            out.push(TokenClue {
                token: hit.token,
                source_field: source_field.to_string(),
                source_kind: source_kind.to_string(),
                strong: false,
                evidence: make_evidence(file, generation, event, selector, line, block),
            });
        }
    }
    out
}
