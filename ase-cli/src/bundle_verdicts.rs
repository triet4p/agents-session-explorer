//! Read-only verdict cross-check for a verified bundle.
//!
//! A local `ase-verdicts-v1` file is compared against the bundle and its
//! evidence outcome map. Nothing is imported, stored or recomputed from a
//! model here: an assertion counts only when its bundle identity, occurrence,
//! question, evidence refs, input identity and content id all agree.

use crate::bundle_digest::value_digest;
use crate::bundle_input::read_json;
use crate::bundle_report::{bump, fail, push_failure};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

pub(crate) fn assertion_body(bundle: &Value, verdict: &Value) -> Value {
    let mut refs = verdict
        .get("evidence_refs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    refs.sort_by(|a, b| a.as_str().unwrap_or("").cmp(b.as_str().unwrap_or("")));
    json!({
        "bundle_id": bundle.get("bundle_id").cloned().unwrap_or(Value::Null),
        "occurrence_id": verdict.get("occurrence_id").cloned().unwrap_or(Value::Null),
        "question_id": verdict.get("question_id").cloned().unwrap_or(Value::Null),
        "answer": verdict.get("answer").cloned().unwrap_or(Value::Null),
        "should_codify": verdict.get("should_codify").cloned().unwrap_or(Value::Null),
        "waste_score": verdict.get("waste_score").cloned().unwrap_or(Value::Null),
        "catalog_kind": verdict.get("catalog_kind").cloned().unwrap_or(Value::Null),
        "catalog_key": verdict.get("catalog_key").cloned().unwrap_or(Value::Null),
        "evidence_refs": refs,
    })
}

pub(crate) fn check_verdicts(
    bundle: &Value,
    verdicts: &Value,
    occurrences: &HashMap<String, Value>,
    evidence_state: &HashMap<String, &'static str>,
    report: &mut Value,
) {
    report["assertion_source"] = json!("local verdict file (read-only cross-check)");
    let Some(rows) = verdicts.get("verdicts").and_then(Value::as_array) else {
        push_failure(report, None, None, "stale_bundle", "verdict rows missing");
        return;
    };
    report["assertions_total"] = json!(rows.len());
    let envelope_failure = if verdicts.get("schema").and_then(Value::as_str)
        != Some("ase-verdicts-v1")
    {
        Some(fail("schema_version_unsupported", "verdict file schema unsupported"))
    } else if verdicts.get("bundle_id").and_then(Value::as_str)
        != bundle.get("bundle_id").and_then(Value::as_str)
    {
        Some(fail("bundle_id_mismatch", "verdict bundle id differs"))
    } else if verdicts.get("bundle_digest").and_then(Value::as_str)
        != bundle.get("bundle_digest").and_then(Value::as_str)
        || verdicts.get("bundle_version").and_then(Value::as_str)
            != bundle.get("bundle_version").and_then(Value::as_str)
    {
        Some(fail("stale_bundle", "verdict bundle identity stale"))
    } else {
        None
    };
    if let Some(f) = envelope_failure {
        for verdict in rows {
            push_failure(
                report,
                verdict.get("assertion_id").and_then(Value::as_str),
                verdict.get("occurrence_id").and_then(Value::as_str),
                f.code,
                &f.detail,
            );
        }
        report["assertions_unverified"] = json!(rows.len());
        return;
    }
    for verdict in rows {
        let occ_id = verdict.get("occurrence_id").and_then(Value::as_str).unwrap_or("");
        let Some(occ) = occurrences.get(occ_id).cloned() else {
            push_failure(report, None, Some(occ_id), "occurrence_unknown", "occurrence unknown in bundle");
            bump(report, "assertions_unverified", 1);
            continue;
        };
        let refs = verdict.get("evidence_refs").and_then(Value::as_array).cloned().unwrap_or_default();
        let question_ok = occ.get("questions").and_then(Value::as_array).is_some_and(|qs| {
            qs.iter().any(|q| {
                q.get("question_id").and_then(Value::as_str)
                    == verdict.get("question_id").and_then(Value::as_str)
            })
        });
        let failure = if !question_ok {
            Some(fail("question_unknown", "question unknown for occurrence"))
        } else if refs.is_empty() {
            Some(fail("missing_evidence", "verdict has no evidence refs"))
        } else if refs.iter().any(|r| !evidence_state.contains_key(r.as_str().unwrap_or(""))) {
            Some(fail("missing_evidence", "evidence ref unknown in bundle"))
        } else if refs.iter().any(|r| {
            evidence_state.get(r.as_str().unwrap_or("")) != Some(&"verified")
                && evidence_state.get(r.as_str().unwrap_or("")) != Some(&"verified_span_after_append")
        }) {
            Some(fail("evidence_unverified", "referenced evidence not verified"))
        } else if verdict.get("input_identity").and_then(Value::as_str)
            != occ.get("input_identity").and_then(Value::as_str)
        {
            Some(fail("input_identity_mismatch", "verdict input identity stale"))
        } else {
            let expected = value_digest(&assertion_body(bundle, verdict));
            if verdict.get("assertion_id").and_then(Value::as_str)
                != Some(format!("as-{}", &expected[..16]).as_str())
            {
                Some(fail("assertion_id_conflict", "assertion id differs from content"))
            } else {
                None
            }
        };
        match failure {
            Some(f) => {
                push_failure(report, None, Some(occ_id), f.code, &f.detail);
                bump(report, "assertions_unverified", 1);
            }
            None => bump(report, "assertions_verified", 1),
        }
    }
}

/// Reads a local verdict file and cross-checks it against the report.
pub(crate) fn cross_check(
    path: &Path,
    bundle: &Value,
    occurrences: &HashMap<String, Value>,
    evidence_state: &HashMap<String, &'static str>,
    report: &mut Value,
) {
    match read_json(path) {
        Ok(verdicts) => check_verdicts(bundle, &verdicts, occurrences, evidence_state, report),
        Err(f) => push_failure(report, None, None, f.code, &f.detail),
    }
}
