//! Build-side evidence reopening from pinned motif addresses.
//!
//! Each occurrence named by the pinned query is reopened from the frozen
//! publication: the pinned address span is re-sliced, the call block is
//! re-projected through its recorded selector, the paired tool result is
//! resolved again, and the redaction policy renders the excerpt that will be
//! published. A prepared occurrence carries no path and no raw result text.

use crate::bundle_digest::{canonical, value_digest};
use crate::bundle_evidence::occurrence_identity_hex;
use crate::bundle_redaction::{cap_signature, redaction_lines, render_excerpt};
use crate::bundle_schema::SELECTOR_VERSION;
use crate::bundle_selector::{
    derive_signature, derive_tool, parsed_rows, result_for_call, result_status_of, row_index_of,
    row_spans, selector_of,
};
use crate::export_policy::MAX_CALL_BYTES;
use ase_core::{err, RawStore, Result};
use ase_index::{sha256_hex, FsRawStore, Index};
use ase_ingest::util::fnv64;
use ase_motif::{align, AlignConfig, Motif, MotifConfig, Occurrence};
use ase_postings::Publication;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

pub(crate) struct PreparedOccurrence {
    pub(crate) occurrence_id: String,
    pub(crate) order_key: (u64, u64, u32),
    pub(crate) call_count: u64,
    pub(crate) divergence: u32,
    pub(crate) failure: bool,
    pub(crate) result_status: String,
    pub(crate) source_alias: String,
    pub(crate) evidence: Vec<Value>,
    pub(crate) identity: String,
    pub(crate) private: Vec<String>,
}

pub(crate) fn occurrence_value(occurrence: &Occurrence) -> Value {
    json!({
        "task": occurrence.task,
        "attempt": occurrence.attempt,
        "dispatch_id": occurrence.dispatch_id,
        "text_hash": occurrence.text_hash,
        "calls": occurrence.calls.iter().map(|call| json!({
            "file": call.evidence.span.file,
            "event": call.evidence.event,
            "offset": call.evidence.span.off,
            "length": call.evidence.span.len,
            "span": format!("span:{}:{}:{}", call.evidence.span.file, call.evidence.span.off, call.evidence.span.len),
            "selector": call.evidence.selector,
            "block_hash": call.evidence.block_hash,
        })).collect::<Vec<Value>>(),
    })
}

pub(crate) fn occurrence_id(value: &Value) -> Result<String> {
    let calls = value
        .get("calls")
        .and_then(Value::as_array)
        .ok_or_else(|| err("occurrence has no calls"))?;
    let mut keys = Vec::with_capacity(calls.len());
    for call in calls {
        let span = call.get("span").and_then(Value::as_str).unwrap_or("");
        let event = call
            .get("event")
            .and_then(Value::as_u64)
            .ok_or_else(|| err("occurrence call event missing"))?;
        let parts: Vec<&str> = span.split(':').collect();
        if parts.len() != 4 || parts[0] != "span" {
            return Err(err("occurrence call span malformed"));
        }
        keys.push(json!({
            "file": parts[1].parse::<u64>().map_err(|_| err("occurrence span file malformed"))?,
            "event": event,
            "offset": parts[2].parse::<u64>().map_err(|_| err("occurrence span offset malformed"))?,
            "length": parts[3].parse::<u64>().map_err(|_| err("occurrence span length malformed"))?,
            "selector": call.get("selector").and_then(Value::as_str).unwrap_or(""),
            "block_hash": call.get("block_hash").and_then(Value::as_str).unwrap_or(""),
        }));
    }
    let identity = json!({
        "task": value.get("task").and_then(Value::as_str).unwrap_or(""),
        "attempt": value.get("attempt").and_then(Value::as_str).unwrap_or(""),
        "dispatch_id": value.get("dispatch_id").and_then(Value::as_str).unwrap_or(""),
        "text_hash": value.get("text_hash").and_then(Value::as_str).unwrap_or(""),
        "calls": keys,
    });
    Ok(format!("occ-{}", &value_digest(&identity)[..16]))
}

pub(crate) fn source_alias(file_id: u64, sha: &str) -> String {
    let digest = value_digest(&json!({ "file_id": file_id, "sha256": sha }));
    format!("src-{}", &digest[..16])
}

fn parse_span(label: &str) -> Result<(u32, u64, u32)> {
    let parts: Vec<&str> = label.split(':').collect();
    if parts.len() != 4 || parts[0] != "span" {
        return Err(err("call span label malformed"));
    }
    Ok((
        parts[1].parse().map_err(|_| err("span file malformed"))?,
        parts[2].parse().map_err(|_| err("span offset malformed"))?,
        parts[3].parse().map_err(|_| err("span length malformed"))?,
    ))
}

fn legacy_selector(selector: &str) -> Option<(usize, String)> {
    let parts: Vec<&str> = selector.split('|').collect();
    if parts.len() != 3 || parts[0] != SELECTOR_VERSION || parts[2] != "toolCall" {
        return None;
    }
    let content = parts[1]
        .strip_prefix("message.content[")?
        .strip_suffix(']')?
        .parse::<usize>()
        .ok()?;
    Some((content, parts[2].to_string()))
}

fn error_nonempty(value: Option<&Value>) -> bool {
    match value {
        None => false,
        Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::Number(_)) | Some(Value::Bool(true)) => true,
    }
}

fn result_failure(row: &Value, block: Option<&Value>) -> bool {
    let message = row.get("message").unwrap_or(&Value::Null);
    message.get("isError").and_then(Value::as_bool) == Some(true)
        || error_nonempty(message.get("error"))
        || block
            .map(|block| {
                block.get("isError").and_then(Value::as_bool) == Some(true)
                    || error_nonempty(block.get("error"))
                    || block.get("status").and_then(Value::as_str) == Some("error")
            })
            .unwrap_or(false)
}

fn build_evidence(
    occurrence: &Occurrence,
    occurrence_id: &str,
    idx: &Index,
    store: &FsRawStore,
    published: &Publication,
    bytes_by_file: &mut HashMap<u64, Arc<[u8]>>,
) -> Result<(Vec<Value>, bool, String, Vec<String>)> {
    let mut evidence = Vec::with_capacity(occurrence.calls.len());
    let mut any_failure = false;
    let mut statuses = BTreeSet::new();
    let mut files = BTreeSet::new();
    let mut private = Vec::new();
    private.push(occurrence.task.clone());
    private.push(occurrence.attempt.clone());
    private.push(occurrence.dispatch_id.clone());

    for (ordinal, call) in occurrence.calls.iter().enumerate() {
        if call.evidence.generation != published.generation {
            return Err(err("export call generation differs from the active publication"));
        }
        let (file_id, off, len) = parse_span(&format!("span:{}:{}:{}", call.evidence.span.file, call.evidence.span.off, call.evidence.span.len))?;
        if file_id != call.evidence.file {
            return Err(err("call span file differs from its evidence file"));
        }
        files.insert(file_id);
        let bytes = match bytes_by_file.get(&(file_id as u64)) {
            Some(bytes) => Arc::clone(bytes),
            None => {
                let bytes = store.bytes(file_id)?;
                let meta = published
                    .manifest
                    .files
                    .iter()
                    .find(|row| row.id == file_id)
                    .ok_or_else(|| err("call file absent from publication manifest"))?;
                if sha256_hex(&bytes) != meta.digest {
                    return Err(err("call file digest differs from the frozen publication"));
                }
                bytes_by_file.insert(file_id as u64, Arc::clone(&bytes));
                bytes
            }
        };
        if off as usize > bytes.len() || len as usize > bytes.len() - off as usize {
            return Err(err("call span outside the reopened source"));
        }
        let event = idx.event(call.evidence.event)?;
        let span = event.span();
        if span != call.evidence.span || fnv64(&bytes[off as usize..off as usize + len as usize]) != event.hash() {
            return Err(err("call span or checksum differs from the pinned address"));
        }
        let spans = row_spans(&bytes);
        let row = row_index_of(off as usize, &bytes);
        if spans.get(row).copied() != Some((off as usize, len as usize)) {
            return Err(err("call offset is not a pinned row boundary"));
        }
        let rows = parsed_rows(&bytes);
        let (content, kind) = legacy_selector(&call.evidence.selector)
            .ok_or_else(|| err("call selector is not the pinned toolCall form"))?;
        let row_value = rows
            .get(row)
            .and_then(Option::as_ref)
            .ok_or_else(|| err("call row is not JSON"))?;
        let block = row_value
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
            .and_then(|blocks| blocks.get(content))
            .ok_or_else(|| err("call selector content index absent"))?
            .clone();
        if block.get("type").and_then(Value::as_str) != Some(kind.as_str()) {
            return Err(err("call selector block type differs"));
        }
        let selector = selector_of(&bytes, row, content, &kind);
        let tool = derive_tool(&block);
        let full_signature = derive_signature(&block);
        let signature = cap_signature(&full_signature);
        let block_sha256 = format!("sha256:{}", sha256_hex(&canonical(&block)));
        let call_id = block
            .get("toolCallId")
            .or_else(|| block.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| err("reopened call has no tool call id"))?;

        let found = result_for_call(&rows, row, call_id);
        let (result, status, result_text, failure) = match found {
            None => (json!({ "status": "absent" }), "absent", String::new(), false),
            Some((result_row, result_content, result_block)) => {
                let status = result_status_of(&result_block);
                let result_kind = result_block
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("toolResult");
                let result_selector = selector_of(&bytes, result_row, result_content, result_kind);
                let result_sha256 = format!("sha256:{}", sha256_hex(&canonical(&result_block)));
                let text = if status == "present" {
                    result_block
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string()
                } else {
                    String::new()
                };
                let failure = result_failure(&rows[result_row].as_ref().cloned().unwrap_or(Value::Null), Some(&result_block));
                (
                    json!({
                        "status": status,
                        "selector": result_selector,
                        "block_sha256": result_sha256,
                    }),
                    status,
                    text,
                    failure,
                )
            }
        };
        statuses.insert(status.to_string());
        any_failure |= failure;

        let mut lines = redaction_lines(&result_text);
        let mut excerpt = render_excerpt(ordinal as u64, &tool, &signature, status, &lines);
        while excerpt.len() > MAX_CALL_BYTES && lines.len() > 1 {
            lines.pop();
            excerpt = render_excerpt(ordinal as u64, &tool, &signature, status, &lines);
        }
        if excerpt.len() > MAX_CALL_BYTES {
            return Err(err("redacted call exceeds the 512-byte citation limit"));
        }
        let evidence_id = {
            let digest = value_digest(&json!({
                "occurrence_id": occurrence_id,
                "call_ordinal": ordinal,
                "selector": selector,
                "block_sha256": block_sha256,
            }));
            format!("ev-{}", &digest[..16])
        };
        let excerpt_sha256 = format!("sha256:{}", sha256_hex(excerpt.as_bytes()));
        evidence.push(json!({
            "evidence_id": evidence_id,
            "call_ordinal": ordinal,
            "call": {
                "file_id": file_id,
                "selector": selector,
                "block_sha256": block_sha256,
                "tool": tool,
                "signature": signature,
            },
            "result": result,
            "result_lines": lines.len(),
            "excerpt": excerpt,
            "excerpt_sha256": excerpt_sha256,
        }));
    }

    if evidence.is_empty() {
        return Err(err("occurrence has no reopened call evidence"));
    }
    if files.len() != 1 {
        return Err(err("one occurrence must use one private source alias"));
    }
    let result_status = if any_failure {
        "failure".to_string()
    } else if statuses.len() == 1 {
        statuses.iter().next().cloned().unwrap_or_else(|| "absent".to_string())
    } else {
        "mixed".to_string()
    };
    Ok((evidence, any_failure, result_status, private))
}

fn divergence(motif: &Motif, signatures: &[String], band: u32) -> u32 {
    align(&motif.signatures, signatures, &AlignConfig { band })
        .map(|a| a.changes())
        .unwrap_or(0)
}

pub(crate) fn prepare_occurrences(
    motif: &Motif,
    config: &MotifConfig,
    idx: &Index,
    store: &FsRawStore,
    published: &Publication,
    bundle: &Value,
    motif_skeleton: &Value,
) -> Result<Vec<PreparedOccurrence>> {
    let mut bytes_by_file = HashMap::new();
    let mut out = Vec::new();
    for occurrence in &motif.occurrences {
        if occurrence.duplicate_of.is_some() {
            continue;
        }
        let value = occurrence_value(occurrence);
        let id = occurrence_id(&value)?;
        let (evidence, failure, result_status, mut private) = build_evidence(
            occurrence,
            &id,
            idx,
            store,
            published,
            &mut bytes_by_file,
        )?;
        let first = occurrence
            .calls
            .first()
            .ok_or_else(|| err("occurrence has no first call"))?;
        let order_key = (
            first.evidence.span.file as u64,
            first.evidence.span.off,
            first.evidence.event,
        );
        let mut reopened = Vec::with_capacity(occurrence.calls.len());
        for call in &occurrence.calls {
            reopened.push(call.signature.clone());
            private.push(call.evidence.selector.clone());
        }
        let mut occurrence_record = json!({
            "occurrence_id": id,
            "source_alias": "",
            "evidence": evidence,
        });
        let alias = {
            let file_id = first.evidence.span.file as u64;
            let meta = published
                .manifest
                .files
                .iter()
                .find(|row| row.id as u64 == file_id)
                .ok_or_else(|| err("occurrence file absent from publication manifest"))?;
            source_alias(file_id, &format!("sha256:{}", meta.digest))
        };
        occurrence_record["source_alias"] = json!(alias);
        let identity = occurrence_identity_hex(bundle, motif_skeleton, &occurrence_record)
            .ok_or_else(|| err("input identity could not be recomputed"))?;
        out.push(PreparedOccurrence {
            occurrence_id: id,
            order_key,
            call_count: occurrence.calls.len() as u64,
            divergence: divergence(motif, &reopened, config.band),
            failure,
            result_status,
            source_alias: alias,
            evidence: occurrence_record
                .get("evidence")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            identity,
            private,
        });
    }
    Ok(out)
}
