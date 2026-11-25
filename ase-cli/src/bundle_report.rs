//! Verification report shape, failure classification and evidence counting.
//!
//! A bundle verify report is one flat record: monotonic counters, a failure
//! list in discovery order and a status derived only at the end. Failure codes
//! split sources that could not be examined (`unverifiable`) from source-backed
//! observations that disagreed with the bundle.

use crate::bundle_input::read_json;
use crate::bundle_schema::BUNDLE_VERSION;
use serde_json::{json, Value};
use std::path::Path;

#[derive(Clone)]
pub(crate) struct Failure {
    pub(crate) code: &'static str,
    pub(crate) detail: String,
}

pub(crate) fn fail(code: &'static str, detail: impl Into<String>) -> Failure {
    Failure {
        code,
        detail: detail.into(),
    }
}

pub(crate) type Check<T> = std::result::Result<T, Failure>;

/// Codes where the source could not be examined at all (as opposed to a
/// source-backed observation that disagreed with the bundle).
pub(crate) fn unverifiable(code: &str) -> bool {
    matches!(code, "alias_unknown" | "source_missing" | "source_unreadable")
}

pub(crate) fn str_field<'a>(value: &'a Value, key: &str) -> Check<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| fail("bundle_tampered", format!("missing string field {key}")))
}

pub(crate) fn push_failure(report: &mut Value, ev: Option<&str>, occ: Option<&str>, code: &str, detail: &str) {
    report["failures"].as_array_mut().expect("failures array").push(json!({
        "evidence_id": ev,
        "occurrence_id": occ,
        "code": code,
        "detail": detail,
    }));
}

pub(crate) fn bump(report: &mut Value, key: &str, by: u64) {
    let current = report[key].as_u64().unwrap_or(0);
    report[key] = json!(current + by);
}

pub(crate) fn base_report() -> Value {
    json!({
        "command": "verify",
        "mode": "bundle",
        "scope": "bundle evidence reopen; no gate measurement",
        "status": "unverified",
        "schema_checked": BUNDLE_VERSION,
        "evidence_total": 0,
        "evidence_verified": 0,
        "evidence_failed": 0,
        "evidence_unverified": 0,
        "motifs_total": 0,
        "motifs_verified": 0,
        "assertions_total": 0,
        "assertions_verified": 0,
        "assertions_unverified": 0,
        "assertion_source": null,
        "views_checked": [],
        "watermarks": [],
        "notes": [],
        "failures": []
    })
}

pub(crate) fn count_evidence(bundle_dir: &Path, motifs: &[Value]) -> (u64, u64) {
    let mut evidence = 0u64;
    let mut occurrences = 0u64;
    for row in motifs {
        let Some(rel) = row.get("motif_path").and_then(Value::as_str) else {
            continue;
        };
        if let Ok(motif) = read_json(&bundle_dir.join(rel)) {
            if let Some(reps) = motif.get("representatives").and_then(Value::as_array) {
                occurrences += reps.len() as u64;
                for rep in reps {
                    evidence += rep
                        .get("evidence")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len) as u64;
                }
            }
        }
    }
    (evidence, occurrences)
}

pub(crate) fn top_failure(
    report: &mut Value,
    code: &str,
    detail: &str,
    evidence: u64,
    motifs: u64,
    occurrences: u64,
) -> Value {
    report["evidence_total"] = json!(evidence);
    report["evidence_failed"] = json!(evidence);
    report["motifs_total"] = json!(motifs);
    report["occurrences_total"] = json!(occurrences);
    push_failure(report, None, None, code, detail);
    report["status"] = json!("unverified");
    report.clone()
}
