//! Evidence-row verification and occurrence input identity.
//!
//! One evidence row is accepted only after its selector reopens the mapped
//! source, the call block digest is recomputed, the paired tool result is
//! resolved again, the redaction policy reproduces the published excerpt and
//! the occurrence identity hashes back to the same value.

use crate::bundle_digest::{canonical, sha_body, value_digest};
use crate::bundle_input::{alias_source, read_source};
use crate::bundle_redaction::{cap_signature, redaction_lines, render_excerpt};
use crate::bundle_report::{fail, str_field, Check};
use crate::bundle_schema::BUNDLE_VERSION;
use crate::bundle_selector::{
    derive_signature, derive_tool, parsed_rows, reopen_block, result_for_call, result_status_of,
    selector_of,
};
use ase_index::sha256_hex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

pub(crate) fn check_evidence(
    row: &Value,
    occurrence: &Value,
    motif: &Value,
    bundle: &Value,
    map: Option<&Value>,
    map_base: &Path,
    files: &HashMap<u64, (String, u64)>,
) -> Check<bool> {
    let call = row.get("call").unwrap_or(&Value::Null);
    let file_id = call.get("file_id").and_then(Value::as_u64).ok_or_else(|| fail("alias_unknown", "call file id missing"))?;
    let (file_sha, _) = files.get(&file_id).ok_or_else(|| fail("alias_unknown", "call file id absent from snapshot"))?;
    let alias = str_field(occurrence, "source_alias")?;
    let source = alias_source(map, map_base, alias, file_id, file_sha)?;
    let bytes = read_source(&source)?;

    // A changed whole-file digest is only a watermark: the decision is made
    // from the re-opened spans below, never from the whole-file digest alone.
    let append = sha256_hex(&bytes) != *file_sha;

    let selector = str_field(call, "selector")?;
    let (call_row, call_block) = reopen_block(&bytes, selector)?;
    let recorded_block_sha = sha_body(str_field(call, "block_sha256")?)
        .ok_or_else(|| fail("digest_mismatch", "call block digest malformed"))?;
    if sha256_hex(&canonical(&call_block)) != recorded_block_sha {
        return Err(fail("digest_mismatch", "reopened call block digest differs"));
    }

    let expected_id = value_digest(&json!({
        "occurrence_id": str_field(occurrence, "occurrence_id")?,
        "call_ordinal": row.get("call_ordinal").and_then(Value::as_u64).unwrap_or(0),
        "selector": selector,
        "block_sha256": str_field(call, "block_sha256")?,
    }));
    if row.get("evidence_id").and_then(Value::as_str)
        != Some(format!("ev-{}", &expected_id[..16]).as_str())
    {
        return Err(fail("bundle_tampered", "evidence id does not match its coordinates"));
    }

    let tool = derive_tool(&call_block);
    let signature = cap_signature(&derive_signature(&call_block));
    if call.get("tool").and_then(Value::as_str) != Some(tool.as_str())
        || call.get("signature").and_then(Value::as_str) != Some(signature.as_str())
    {
        return Err(fail("selector_mismatch", "reopened tool/signature differs from bundle"));
    }

    let recorded_result = row.get("result").unwrap_or(&Value::Null);
    let status = recorded_result
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !matches!(status, "present" | "absent" | "nontext") {
        return Err(fail("result_status_mismatch", "published result status unsupported"));
    }
    let call_id = call_block
        .get("toolCallId")
        .or_else(|| call_block.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| fail("selector_mismatch", "reopened call has no tool call id"))?;
    let rows = parsed_rows(&bytes);
    let found = result_for_call(&rows, call_row, call_id);
    let (result_text, result_status) = match (found, status) {
        (None, "absent") => {
            if recorded_result.get("selector").is_some() || recorded_result.get("block_sha256").is_some() {
                return Err(fail("result_status_mismatch", "absent result carries a digest/selector"));
            }
            (String::new(), "absent")
        }
        (None, _) => return Err(fail("result_status_mismatch", "paired tool result absent")),
        (Some(_), "absent") => return Err(fail("result_status_mismatch", "paired tool result is present")),
        (Some((ri, ci, result_block)), expected) => {
            let actual_status = result_status_of(&result_block);
            if actual_status != expected {
                return Err(fail("result_status_mismatch", "result status differs from source"));
            }
            let kind = result_block.get("type").and_then(Value::as_str).unwrap_or("toolResult");
            let expected_selector = selector_of(&bytes, ri, ci, kind);
            if recorded_result.get("selector").and_then(Value::as_str) != Some(expected_selector.as_str()) {
                return Err(fail("selector_mismatch", "result selector differs from source"));
            }
            let recorded_sha = sha_body(
                recorded_result
                    .get("block_sha256")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            )
            .ok_or_else(|| fail("digest_mismatch", "result block digest malformed"))?;
            if sha256_hex(&canonical(&result_block)) != recorded_sha {
                return Err(fail("digest_mismatch", "reopened result block digest differs"));
            }
            let text = result_block
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            (text, expected)
        }
    };

    let ordinal = row.get("call_ordinal").and_then(Value::as_u64).unwrap_or(0);
    let mut lines = redaction_lines(&result_text);
    if let Some(kept) = row.get("result_lines").and_then(Value::as_u64) {
        if kept > 3 {
            return Err(fail("excerpt_mismatch", "result_lines exceeds the three-line limit"));
        }
        lines.truncate(kept as usize);
    }
    let excerpt = render_excerpt(ordinal, &tool, &signature, result_status, &lines);
    let published = row.get("excerpt").and_then(Value::as_str).unwrap_or("");
    let recorded_excerpt_sha = sha_body(
        row.get("excerpt_sha256").and_then(Value::as_str).unwrap_or(""),
    )
    .ok_or_else(|| fail("excerpt_digest_mismatch", "excerpt digest malformed"))?;
    if sha256_hex(published.as_bytes()) != recorded_excerpt_sha {
        return Err(fail("excerpt_digest_mismatch", "published excerpt digest differs"));
    }
    if published != excerpt {
        return Err(fail("excerpt_mismatch", "published excerpt differs from policy output"));
    }

    let expected_identity = occurrence_identity(bundle, motif, occurrence)?;
    if occurrence.get("input_identity").and_then(Value::as_str) != Some(expected_identity.as_str()) {
        return Err(fail("input_identity_mismatch", "occurrence input identity differs"));
    }
    Ok(append)
}

fn occurrence_identity(bundle: &Value, motif: &Value, occurrence: &Value) -> Check<String> {
    let mut evidence = Vec::new();
    let rows = occurrence
        .get("evidence")
        .and_then(Value::as_array)
        .ok_or_else(|| fail("bundle_tampered", "occurrence evidence missing"))?;
    for row in rows {
        evidence.push(json!({
            "evidence_id": row.get("evidence_id").and_then(Value::as_str).unwrap_or(""),
            "call_block_sha256": row.get("call").and_then(|c| c.get("block_sha256")).and_then(Value::as_str).unwrap_or(""),
            "excerpt_sha256": row.get("excerpt_sha256").and_then(Value::as_str).unwrap_or(""),
        }));
    }
    let questions = motif
        .get("questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let question_ids: Vec<String> = questions
        .iter()
        .filter_map(|q| q.get("question_id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let engine = bundle.get("engine").unwrap_or(&Value::Null);
    Ok(value_digest(&json!({
        "bundle_version": meta_version(bundle, "bundle_version", BUNDLE_VERSION)?,
        "motif_id": motif.get("motif_id").and_then(Value::as_str).unwrap_or(""),
        "occurrence_id": str_field(occurrence, "occurrence_id")?,
        "question_ids": question_ids,
        "rule_versions": engine.get("rule_versions").cloned().unwrap_or(Value::Null),
        "redaction_policy": engine.get("redaction_policy").and_then(Value::as_str).unwrap_or(""),
        "evidence": evidence,
    })))
}

pub(crate) fn occurrence_identity_hex(bundle: &Value, motif: &Value, occurrence: &Value) -> Option<String> {
    occurrence_identity(bundle, motif, occurrence).ok()
}

fn meta_version(bundle: &Value, key: &str, expected: &str) -> Check<String> {
    let actual = str_field(bundle, key)?;
    if actual != expected {
        return Err(fail("redaction_version_unsupported", format!("{key} unsupported")));
    }
    Ok(actual.to_string())
}
