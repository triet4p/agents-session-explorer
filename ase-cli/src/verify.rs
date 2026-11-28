//! Evidence-bundle verification against a private local source map.
//!
//! The bundle is read as data only. Every evidence row is reopened from a
//! private local source map: source bytes are read again, the exact byte span
//! named by the selector is sliced, the versioned block selector is re-run, the
//! strong block digest is recomputed, the paired tool result is resolved again,
//! and the redaction policy is re-applied before the published excerpt is
//! accepted. Nothing here imports a verdict, joins a catalog, calls a model or
//! claims a gate result. `verify --all` lives in `cmd_verify` and is untouched.
//!
//! This module owns the read-side orchestration: it walks the bundle, checks
//! digests and held-out views in fail-closed order, and derives the final
//! report status. Span/selector reopening, digest helpers, redaction, views,
//! source-map access, report scaffolding, evidence checks and the verdict
//! cross-check live in their own focused modules.

use crate::bundle_digest::{sha_body, value_digest};
use crate::bundle_evidence::check_evidence;
use crate::bundle_input::{check_snapshot, load_local_map, read_json, snapshot_files};
use crate::bundle_report::{
    base_report, bump, count_evidence, fail, push_failure, top_failure, unverifiable, Check,
};
use crate::bundle_schema::{
    BUNDLE_VERSION, EVIDENCE_RENDER, MOTIF_SCHEMA, QUESTIONS_RENDER, REDACTION_POLICY,
    REPRESENTATIVE_POLICY, SELECTOR_VERSION,
};
use crate::bundle_verdicts::cross_check;
use crate::bundle_views::{render_evidence_view, render_questions_view};
use crate::cli_output::outln;
use ase_core::{err, Result};
use ase_index::sha256_hex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// CLI entry point. A positional argument selects the bundle form; the
/// legacy `--all` / `--sample` form never reaches this module.
pub fn run_bundle(bundle: &str, args: &[String]) -> Result<()> {
    let local_map = opt_value(args, "--local-map").map(PathBuf::from);
    let verdicts = opt_value(args, "--verdicts").map(PathBuf::from);
    let report = verify(
        Path::new(bundle),
        local_map.as_deref(),
        verdicts.as_deref(),
    );
    let rendered = serde_json::to_string_pretty(&report).map_err(|e| err(format!("verify report: {e}")))?;
    if args.iter().any(|a| a == "--json") {
        outln!("{rendered}");
    } else {
        outln!(
            "bundle verify: {}; evidence {}/{} ({} failed, {} unverified), motifs {}/{}, assertions {}/{}",
            report["status"].as_str().unwrap_or("unverified"),
            report["evidence_verified"].as_u64().unwrap_or(0),
            report["evidence_total"].as_u64().unwrap_or(0),
            report["evidence_failed"].as_u64().unwrap_or(0),
            report["evidence_unverified"].as_u64().unwrap_or(0),
            report["motifs_verified"].as_u64().unwrap_or(0),
            report["motifs_total"].as_u64().unwrap_or(0),
            report["assertions_verified"].as_u64().unwrap_or(0),
            report["assertions_total"].as_u64().unwrap_or(0),
        );
    }
    if report["status"].as_str() != Some("verified") {
        std::process::exit(3);
    }
    Ok(())
}

fn opt_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn check_file_view(bundle_dir: &Path, rel: &str, recorded: &str, rendered: Option<String>) -> Check<()> {
    let bytes = fs::read(bundle_dir.join(rel))
        .map_err(|_| fail("view_digest_mismatch", "view file not readable"))?;
    let recorded = sha_body(recorded)
        .ok_or_else(|| fail("view_digest_mismatch", "view digest malformed"))?;
    if sha256_hex(&bytes) != recorded {
        return Err(fail("view_digest_mismatch", "view bytes differ from recorded digest"));
    }
    if let Some(rendered) = rendered {
        let rendered = rendered.into_bytes();
        if bytes != rendered {
            let at = bytes
                .iter()
                .zip(rendered.iter())
                .position(|(a, b)| a != b)
                .unwrap_or(bytes.len().min(rendered.len()));
            return Err(fail(
                "view_digest_mismatch",
                format!("view is not the deterministic render at byte {at} ({} vs {} bytes)", bytes.len(), rendered.len()),
            ));
        }
    }
    Ok(())
}

pub(crate) fn verify(bundle_dir: &Path, map_path: Option<&Path>, verdicts_path: Option<&Path>) -> Value {
    let mut report = base_report();
    let bytes = match fs::read(bundle_dir.join("bundle.json")) {
        Ok(bytes) => bytes,
        Err(_) => return top_failure(&mut report, "bundle_tampered", "bundle.json not readable", 0, 0, 0),
    };
    let bundle: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return top_failure(&mut report, "bundle_tampered", "bundle.json not JSON", 0, 0, 0),
    };
    let motifs: Vec<Value> = bundle
        .get("motifs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let (total_evidence, total_occurrences) = count_evidence(bundle_dir, &motifs);
    report["occurrences_total"] = json!(total_occurrences);

    let schema_ok = bundle.get("schema").and_then(Value::as_str) == Some(BUNDLE_VERSION)
        && bundle.get("bundle_version").and_then(Value::as_str) == Some(BUNDLE_VERSION);
    if !schema_ok {
        return top_failure(&mut report, "schema_version_unsupported", "bundle schema/version unsupported", total_evidence, motifs.len() as u64, total_occurrences);
    }
    let no_digest = {
        let mut value = bundle.clone();
        value.as_object_mut().expect("bundle object").remove("bundle_digest");
        value
    };
    if bundle.get("bundle_digest").and_then(Value::as_str)
        != Some(format!("sha256:{}", value_digest(&no_digest)).as_str())
    {
        return top_failure(&mut report, "bundle_digest_mismatch", "bundle digest does not match canonical body", total_evidence, motifs.len() as u64, total_occurrences);
    }
    let no_id = {
        let mut value = bundle.clone();
        value.as_object_mut().expect("bundle object").remove("bundle_id");
        value.as_object_mut().expect("bundle object").remove("bundle_digest");
        value
    };
    if bundle.get("bundle_id").and_then(Value::as_str)
        != Some(format!("bun-{}", &value_digest(&no_id)[..16]).as_str())
    {
        return top_failure(&mut report, "bundle_tampered", "bundle id does not match canonical body", total_evidence, motifs.len() as u64, total_occurrences);
    }

    let engine = bundle.get("engine").unwrap_or(&Value::Null);
    let version_check: [(&str, &str); 5] = [
        ("redaction_policy", REDACTION_POLICY),
        ("evidence_render", EVIDENCE_RENDER),
        ("questions_render", QUESTIONS_RENDER),
        ("representative_policy", REPRESENTATIVE_POLICY),
        ("name", "ase"),
    ];
    for (key, expected) in version_check {
        if bundle.get(key).and_then(Value::as_str) == Some(expected)
            || engine.get(key).and_then(Value::as_str) == Some(expected)
        {
            continue;
        }
        let code = match key {
            "redaction_policy" => "redaction_version_unsupported",
            "evidence_render" | "questions_render" => "view_digest_mismatch",
            "representative_policy" => "engine_version_unsupported",
            _ => "engine_version_unsupported",
        };
        return top_failure(&mut report, code, "bundle version unsupported", total_evidence, motifs.len() as u64, total_occurrences);
    }
    let rules = engine.get("rule_versions").and_then(Value::as_object);
    let required_rules: [(&str, &str); 4] = [
        ("callsig", "callsig-v1"),
        ("motif_alignment", "motif-align-v1"),
        ("motif_dedupe", "seq-text-v1"),
        ("projection", SELECTOR_VERSION),
    ];
    for (key, expected) in required_rules {
        if rules.and_then(|r| r.get(key)).and_then(Value::as_str) != Some(expected) {
            return top_failure(&mut report, "engine_version_unsupported", "rule version unsupported", total_evidence, motifs.len() as u64, total_occurrences);
        }
    }
    if bundle.get("redaction").and_then(|r| r.get("policy")).and_then(Value::as_str)
        != Some(REDACTION_POLICY)
    {
        return top_failure(&mut report, "redaction_version_unsupported", "redaction policy block unsupported", total_evidence, motifs.len() as u64, total_occurrences);
    }

    let (map, map_base) = match load_local_map(map_path) {
        Ok(value) => value,
        Err(f) => return top_failure(&mut report, f.code, &f.detail, total_evidence, motifs.len() as u64, total_occurrences),
    };
    if let Err(f) = check_snapshot(&bundle, map.as_ref()) {
        return top_failure(&mut report, f.code, &f.detail, total_evidence, motifs.len() as u64, total_occurrences);
    }
    let files = snapshot_files(&bundle);

    let mut occurrences: HashMap<String, Value> = HashMap::new();
    let mut evidence_state: HashMap<String, &'static str> = HashMap::new();
    let mut all_motifs_ok = true;
    for motif_row in &motifs {
        bump(&mut report, "motifs_total", 1);
        let motif_id = motif_row.get("motif_id").and_then(Value::as_str).unwrap_or("");
        let motif_rel = motif_row.get("motif_path").and_then(Value::as_str).unwrap_or("");
        let evidence_rel = motif_row.get("evidence_path").and_then(Value::as_str).unwrap_or("");
        let questions_rel = motif_row.get("questions_path").and_then(Value::as_str).unwrap_or("");
        let motif: Value = match read_json(&bundle_dir.join(motif_rel)) {
            Ok(v) => v,
            Err(_) => {
                push_failure(&mut report, None, None, "motif_digest_mismatch", "motif.json unreadable");
                all_motifs_ok = false;
                continue;
            }
        };
        if motif.get("schema").and_then(Value::as_str) != Some(MOTIF_SCHEMA)
            || motif.get("motif_id").and_then(Value::as_str) != Some(motif_id)
        {
            push_failure(&mut report, None, None, "motif_digest_mismatch", "motif schema/id mismatch");
            all_motifs_ok = false;
            continue;
        }
        let mut motif_ok = true;
        let recorded_motif_sha = motif_row.get("motif_sha256").and_then(Value::as_str).unwrap_or("");
        match fs::read(bundle_dir.join(motif_rel)) {
            Ok(bytes) if sha_body(recorded_motif_sha) == Some(sha256_hex(&bytes).as_str()) => {}
            _ => {
                push_failure(&mut report, None, None, "motif_digest_mismatch", "motif bytes differ from recorded digest");
                motif_ok = false;
            }
        }
        for (rel, recorded, rendered) in [
            (evidence_rel, motif_row.get("evidence_sha256").and_then(Value::as_str).unwrap_or(""), Some(render_evidence_view(&motif))),
            (questions_rel, motif_row.get("questions_sha256").and_then(Value::as_str).unwrap_or(""), Some(render_questions_view(&motif))),
        ] {
            if let Err(f) = check_file_view(bundle_dir, rel, recorded, rendered) {
                push_failure(&mut report, None, None, f.code, &f.detail);
                motif_ok = false;
            }
        }
        report["views_checked"] = json!(["evidence.md", "questions.md"]);

        let reps = motif.get("representatives").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut motif_all_accepted = motif_ok;
        for rep in reps {
            let occ_id = rep.get("occurrence_id").and_then(Value::as_str).unwrap_or("").to_string();
            let mut indexed = rep.clone();
            indexed["questions"] = motif.get("questions").cloned().unwrap_or_else(|| json!([]));
            occurrences.insert(occ_id.clone(), indexed);
            let rows = rep.get("evidence").and_then(Value::as_array).cloned().unwrap_or_default();
            for _row in &rows {
                bump(&mut report, "evidence_total", 1);
            }
            if !motif_ok {
                for row in &rows {
                    bump(&mut report, "evidence_failed", 1);
                    if let Some(id) = row.get("evidence_id").and_then(Value::as_str) {
                        evidence_state.insert(id.to_string(), "failed");
                    }
                }
                continue;
            }
            let mut outcomes: Vec<Check<bool>> = Vec::new();
            for row in &rows {
                outcomes.push(check_evidence(
                    row,
                    &rep,
                    &motif,
                    &bundle,
                    map.as_ref(),
                    &map_base,
                    &files,
                ));
            }
            for (row, outcome) in rows.iter().zip(outcomes.into_iter()) {
                let ev_id = row.get("evidence_id").and_then(Value::as_str).unwrap_or("");
                match outcome {
                    Ok(append) => {
                        if append {
                            bump(&mut report, "evidence_verified", 1);
                            evidence_state.insert(ev_id.to_string(), "verified_span_after_append");
                            report["watermarks"].as_array_mut().expect("watermarks array").push(json!({
                                "evidence_id": ev_id,
                                "occurrence_id": occ_id,
                                "code": "verified_span_after_append",
                                "note": "whole-file digest differs from the frozen pin; only reopened spans were rechecked, no current-file completeness claimed",
                            }));
                        } else {
                            bump(&mut report, "evidence_verified", 1);
                            evidence_state.insert(ev_id.to_string(), "verified");
                        }
                    }
                    Err(f) => {
                        motif_all_accepted = false;
                        bump(&mut report, if unverifiable(f.code) { "evidence_unverified" } else { "evidence_failed" }, 1);
                        evidence_state.insert(ev_id.to_string(), "failed");
                        push_failure(&mut report, Some(ev_id), Some(&occ_id), f.code, &f.detail);
                    }
                }
            }
        }
        if motif_ok && motif_all_accepted {
            bump(&mut report, "motifs_verified", 1);
        } else {
            all_motifs_ok = false;
        }
    }

    if report["evidence_total"].as_u64().unwrap_or(0) == 0 {
        report["notes"].as_array_mut().expect("notes").push(json!("empty bundle is never verified"));
    }
    if let Some(path) = verdicts_path {
        cross_check(path, &bundle, &occurrences, &evidence_state, &mut report);
    }

    let total_ev = report["evidence_total"].as_u64().unwrap_or(0);
    let verified = report["evidence_verified"].as_u64().unwrap_or(0);
    let failed = report["evidence_failed"].as_u64().unwrap_or(0);
    let unverified = report["evidence_unverified"].as_u64().unwrap_or(0);
    let assertions_ok = report["assertions_total"].as_u64().unwrap_or(0)
        == report["assertions_verified"].as_u64().unwrap_or(0);
    let ok = all_motifs_ok
        && total_ev > 0
        && verified == total_ev
        && failed == 0
        && unverified == 0
        && assertions_ok;
    report["status"] = json!(if ok { "verified" } else { "unverified" });
    report
}
