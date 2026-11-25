//! Bundle header and motif skeleton written before evidence is attached.
//!
//! Both documents are pure functions of the pinned publication, the motif
//! report and the engine/build identity. They carry every versioned policy
//! name the verifier later checks, so a draft is sealed only after the same
//! constants have been through the verify route.

use crate::bundle_schema::{
    BUNDLE_VERSION, EVIDENCE_RENDER, MOTIF_SCHEMA, QUESTIONS_RENDER, REDACTION_POLICY,
    REPRESENTATIVE_POLICY, SELECTOR_VERSION,
};
use crate::export_policy::{
    MAX_BUNDLE_BYTES, MAX_CALL_BYTES, MAX_OCCURRENCE_BYTES, MAX_REPRESENTATIVES, MAX_RESULT_LINES,
    QUESTION_TEXTS,
};
use crate::export_query::{build_query_digest, query_config};
use ase_core::{err, Result};
use ase_motif::{Motif, MotifConfig, MotifReport};
use ase_postings::Publication;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(crate) fn build_bundle_base(
    published: &Publication,
    report: &MotifReport,
    config: &MotifConfig,
    binary_sha256: &str,
) -> Result<Value> {
    let core = &published.manifest.core_digest;
    if core.len() < 16 {
        return Err(err("publication core digest is too short"));
    }
    let snapshot_id = format!("snap-{}", &core[..16]);
    let mut files = published.manifest.files.clone();
    files.sort_by_key(|row| row.id);
    let file_rows: Vec<Value> = files
        .iter()
        .map(|row| {
            json!({
                "file_id": row.id,
                "sha256": format!("sha256:{}", row.digest),
                "size": row.size,
            })
        })
        .collect();
    let total_bytes: u64 = files.iter().map(|row| row.size).sum();
    let query_result_digest = build_query_digest(report, config)?;
    Ok(json!({
        "schema": BUNDLE_VERSION,
        "bundle_version": BUNDLE_VERSION,
        "created_by": "ase-export",
        "snapshot": {
            "core_digest": core,
            "file_count": files.len(),
            "files": file_rows,
            "provider": "omp",
            "snapshot_id": snapshot_id,
            "total_bytes": total_bytes,
        },
        "query": {
            "config": query_config(config),
            "dedupe_rule": {
                "name": ase_motif::TEXT_HASH_RULE_NAME,
                "version": ase_motif::TEXT_HASH_RULE_VERSION,
            },
            "query_result_digest": query_result_digest,
            "rule": { "name": report.rule_name, "version": report.rule_version },
        },
        "engine": {
            "binary_sha256": format!("sha256:{binary_sha256}"),
            "evidence_render": EVIDENCE_RENDER,
            "name": "ase",
            "questions_render": QUESTIONS_RENDER,
            "redaction_policy": REDACTION_POLICY,
            "rule_versions": {
                "callsig": "callsig-v1",
                "motif_alignment": ase_motif::MOTIF_RULE_VERSION,
                "motif_dedupe": ase_motif::TEXT_HASH_RULE_VERSION,
                "projection": SELECTOR_VERSION,
            },
        },
        "redaction": {
            "allowed_fields": [
                "tool name",
                "callsig-v1 argument signature",
                "first N result text lines after redaction",
            ],
            "forbidden": [
                "real paths",
                "alias-to-source mapping",
                "raw tool output",
                "thinking or system text",
                "verdicts",
                "session identifiers",
            ],
            "markers": [
                "«url»", "«path»", "«email»", "«secret»", "«token»", "«absent»", "«nontext»", "«empty»",
            ],
            "policy": REDACTION_POLICY,
        },
        "citation_limits": {
            "max_bundle_bytes": MAX_BUNDLE_BYTES,
            "max_excerpt_bytes_per_call": MAX_CALL_BYTES,
            "max_excerpt_bytes_per_occurrence": MAX_OCCURRENCE_BYTES,
            "max_occurrences_per_motif": MAX_REPRESENTATIVES,
            "max_result_line_bytes": 160,
            "max_result_lines": MAX_RESULT_LINES,
        },
        "representative_policy": REPRESENTATIVE_POLICY,
    }))
}

pub(crate) fn build_motif_skeleton(motif: &Motif, reference: &str) -> Value {
    let mut relaxed = BTreeMap::new();
    for (gap, count) in motif.support_relaxed.iter().enumerate() {
        relaxed.insert(gap.to_string(), *count);
    }
    json!({
        "schema": MOTIF_SCHEMA,
        "motif_id": motif.id,
        "family_id": motif.family_id,
        "signatures": motif.signatures,
        "support": { "exact": motif.support_exact, "relaxed": relaxed },
        "denominator": {
            "tasks_total": motif.denominator.tasks_total,
            "tasks_in_family": motif.denominator.tasks_in_family,
            "tasks_with_occurrence": motif.denominator.tasks_with_occurrence,
            "sequences_in_family": motif.denominator.sequences_in_family,
            "sequences_with_occurrence": motif.denominator.sequences_with_occurrence,
            "sequences_deduplicated": motif.denominator.sequences_deduplicated,
            "excluded_count": motif.denominator.excluded.len(),
        },
        "reference": { "occurrence_id": reference },
        "representative_policy": REPRESENTATIVE_POLICY,
        "questions": QUESTION_TEXTS.iter().map(|(id, text)| json!({
            "question_id": id,
            "text": text,
        })).collect::<Vec<Value>>(),
        "representatives": [],
        "occurrences_total": 0,
        "representatives_selected": 0,
        "representatives_shortfall": false,
    })
}
