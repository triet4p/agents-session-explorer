//! Evidence-bundle export orchestration.
//!
//! The exporter runs the pinned motif query locally, selects representatives
//! deterministically, redacts result text with the pinned redaction policy,
//! writes only path-free shareable files, and keeps the alias-to-source map
//! outside the bundle. Nothing is published until the freshly written staging
//! bundle has been reopened and verified by `verify::verify`. No model,
//! network, verdict, catalog or gate measurement is involved.
//!
//! This module owns the build-side orchestration: argument and path handling,
//! the staging tree, assembly and publication. Query description, evidence
//! reopening, representative selection, leak scanning and the shared
//! span/selector/digest/redaction/view helpers live in focused modules.

use crate::bundle_digest::value_digest;
use crate::bundle_schema::LOCAL_MAP_SCHEMA;
use crate::cli_output::outln;
use crate::bundle_views::{render_evidence_view, render_questions_view};
use crate::export_draft::{build_bundle_base, build_motif_skeleton};
use crate::export_evidence::{
    occurrence_id, occurrence_value, prepare_occurrences, source_alias, PreparedOccurrence,
};
use crate::export_guard::scan_no_leaks;
use crate::export_policy::{MAX_BUNDLE_BYTES, MAX_OCCURRENCE_BYTES};
use crate::export_query::pinned_config;
use crate::export_selection::{cap_selection, removable_position, select_representatives};
use crate::index::require_publication;
use crate::motif::{motif_target, MotifTarget};
use crate::verify::verify;
use ase_core::{err, Result};
use ase_index::{sha256_hex, Index};
use ase_postings::Publication;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Component, Path, PathBuf};

const VALUE_FLAGS: [&str; 7] = [
    "--out",
    "--local-map",
    "--index",
    "--omp-root",
    "--codex-root",
    "--address",
    "--addresses",
];

struct Assembled {
    bundle: Value,
    motif: Value,
    files: Vec<(String, Vec<u8>)>,
    total_bytes: usize,
}

fn opt(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn flag_values(args: &[String], name: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == name {
            let value = args
                .get(i + 1)
                .ok_or_else(|| err(format!("{name} needs a value")))?;
            if value.starts_with("--") {
                return Err(err(format!(
                    "{name} needs a value, not the flag {value}; check the flag/value order"
                )));
            }
            out.push(value.clone());
            i += 2;
            continue;
        }
        i += 1;
    }
    Ok(out)
}

fn positional_target(args: &[String]) -> Result<String> {
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if VALUE_FLAGS.contains(&arg.as_str()) {
            i += 2;
            continue;
        }
        if arg.starts_with("--") {
            i += 1;
            continue;
        }
        return Ok(arg.clone());
    }
    Err(err("export needs one motif:<id> positional argument"))
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        normalize(path)
    } else {
        normalize(&std::env::current_dir().unwrap_or_default().join(path))
    }
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let body = serde_json::to_string_pretty(value).map_err(|e| err(format!("JSON render: {e}")))?;
    fs::write(path, body).map_err(|e| err(format!("write failed: {e}")))
}

fn digest_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|e| err(format!("read failed: {e}")))?;
    Ok(sha256_hex(&bytes))
}

fn address_file_entries(path: &str) -> Result<Vec<String>> {
    let bytes = fs::read(Path::new(path)).map_err(|e| {
        let trimmed = path.trim_start();
        if trimmed.starts_with('[') || trimmed.starts_with('{') || path.contains('|') {
            err(format!(
                "--addresses expects FILE, not an inline address list; pass a file path or repeat --address 'PROJECT|TASK' for inline keys"
            ))
        } else {
            err(format!("--addresses could not read FILE {path:?}: {e}"))
        }
    })?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|e| err(format!("--addresses FILE {path:?} is not JSON: {e}")))?;
    let list = match &value {
        Value::Array(rows) => Some(rows.as_slice()),
        Value::Object(_) => value.get("addresses").and_then(Value::as_array).map(Vec::as_slice),
        _ => None,
    };
    let Some(list) = list else {
        return Err(err(format!(
            "--addresses FILE {path:?} must be a JSON array of strings or an object with an `addresses` array of strings, not inline/CSV text"
        )));
    };
    let mut entries = Vec::with_capacity(list.len());
    for (index, row) in list.iter().enumerate() {
        let address = row.as_str().ok_or_else(|| {
            err(format!(
                "--addresses FILE {path:?} entry {index} must be a string PROJECT|TASK; inline/CSV text is not accepted"
            ))
        })?;
        entries.push(address.to_string());
    }
    Ok(entries)
}

fn collect_addresses(args: &[String]) -> Result<Vec<MotifTarget>> {
    let mut raw = flag_values(args, "--address")?;
    let mut from_file = false;
    for path in flag_values(args, "--addresses")? {
        from_file = true;
        raw.extend(address_file_entries(&path)?);
    }
    raw.sort();
    raw.dedup();
    if raw.is_empty() {
        return Err(err(if from_file {
            "export needs at least one address; --addresses FILE listed no addresses and no --address was given"
        } else {
            "export needs at least one address: repeat --address 'PROJECT|TASK' or pass --addresses FILE with a JSON array of strings"
        }));
    }
    let mut targets = Vec::with_capacity(raw.len());
    for address in raw {
        let target = motif_target(&address)?;
        if target.project.is_none() {
            return Err(err(format!(
                "export requires full PROJECT|TASK keys; got {address:?}. Quote 'PROJECT|TASK' in PowerShell so the pipe stays in one argument"
            )));
        }
        targets.push(target);
    }
    Ok(targets)
}

fn finalize_bundle(mut bundle: Value) -> Value {
    bundle
        .as_object_mut()
        .expect("bundle object")
        .remove("bundle_id");
    bundle
        .as_object_mut()
        .expect("bundle object")
        .remove("bundle_digest");
    let id = format!("bun-{}", &value_digest(&bundle)[..16]);
    bundle["bundle_id"] = json!(id);
    let digest = format!("sha256:{}", value_digest(&bundle));
    bundle["bundle_digest"] = json!(digest);
    bundle
}

fn assemble(
    stage_bundle: &Path,
    bundle_base: &Value,
    motif_skeleton: &Value,
    prepared: &[PreparedOccurrence],
    selected: &[(usize, String)],
    occurrences_total: usize,
) -> Result<Assembled> {
    let mut representatives = Vec::with_capacity(selected.len());

    for &(index, ref label) in selected {
        let row = &prepared[index];
        let mut excerpt_bytes = 0usize;
        for evidence in &row.evidence {
            excerpt_bytes += evidence
                .get("excerpt")
                .and_then(Value::as_str)
                .unwrap_or("")
                .len();
        }
        if excerpt_bytes > MAX_OCCURRENCE_BYTES {
            return Err(err("occurrence excerpt exceeds the 4096-byte citation limit"));
        }

        representatives.push(json!({
            "occurrence_id": row.occurrence_id,
            "selection": label,
            "source_alias": row.source_alias,
            "call_count": row.call_count,
            "result_status": row.result_status,
            "input_identity": row.identity,
            "evidence": row.evidence,
        }));
    }
    let mut motif = motif_skeleton.clone();
    motif["representatives"] = json!(representatives);
    motif["occurrences_total"] = json!(occurrences_total);
    motif["representatives_selected"] = json!(selected.len());
    motif["representatives_shortfall"] = json!(selected.len() < occurrences_total);

    let motif_rel = format!("motifs/{}/motif.json", motif_skeleton["motif_id"].as_str().unwrap_or(""));
    let evidence_rel = format!("motifs/{}/evidence.md", motif_skeleton["motif_id"].as_str().unwrap_or(""));
    let questions_rel = format!("motifs/{}/questions.md", motif_skeleton["motif_id"].as_str().unwrap_or(""));
    let motif_body = serde_json::to_string_pretty(&motif).map_err(|e| err(format!("JSON render: {e}")))?;
    let evidence_body = render_evidence_view(&motif);
    let questions_body = render_questions_view(&motif);
    let hash = |bytes: &[u8]| format!("sha256:{}", sha256_hex(bytes));
    let mut bundle = bundle_base.clone();
    bundle["motifs"] = json!([{
        "evidence_path": evidence_rel,
        "evidence_sha256": hash(evidence_body.as_bytes()),
        "motif_id": motif_skeleton["motif_id"],
        "motif_path": motif_rel,
        "motif_sha256": hash(motif_body.as_bytes()),
        "questions_path": questions_rel,
        "questions_sha256": hash(questions_body.as_bytes()),
    }]);
    let bundle = finalize_bundle(bundle);
    let bundle_body = serde_json::to_string_pretty(&bundle).map_err(|e| err(format!("JSON render: {e}")))?;
    let files = vec![
        ("bundle.json".to_string(), bundle_body.into_bytes()),
        (motif_rel, motif_body.into_bytes()),
        (evidence_rel, evidence_body.into_bytes()),
        (questions_rel, questions_body.into_bytes()),
    ];
    let total_bytes: usize = files.iter().map(|(_, bytes)| bytes.len()).sum();
    for (rel, bytes) in &files {
        let path = stage_bundle.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| err(format!("create directory: {e}")))?;
        }
        fs::write(path, bytes).map_err(|e| err(format!("write failed: {e}")))?;
    }

    Ok(Assembled {
        bundle,
        motif,
        files,
        total_bytes,
    })
}

fn build_local_map(
    published: &Publication,
    idx: &Index,
    aliases: &HashMap<u64, String>,
) -> Result<Value> {
    let core = &published.manifest.core_digest;
    let snapshot_id = format!("snap-{}", &core[..16.min(core.len())]);
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
    let mut alias_rows = BTreeMap::new();
    for row in &files {
        let alias = aliases
            .get(&(row.id as u64))
            .ok_or_else(|| err("snapshot file has no private alias"))?;
        let source_path = idx.file(row.id)?.path().to_string();
        let path = absolute(Path::new(&source_path)).to_string_lossy().replace('\\', "/");
        alias_rows.insert(alias.clone(), json!({
            "file_id": row.id,
            "path": path,
            "sha256": format!("sha256:{}", row.digest),
        }));
    }
    Ok(json!({
        "schema": LOCAL_MAP_SCHEMA,
        "snapshot": {
            "core_digest": core,
            "files": file_rows,
            "snapshot_id": snapshot_id,
        },
        "aliases": alias_rows,
    }))
}

pub fn run(args: &[String]) -> Result<()> {
    let target = positional_target(args)?;
    let motif_id = target
        .strip_prefix("motif:")
        .ok_or_else(|| err("export target must be motif:<id>"))?;
    if motif_id.is_empty() || motif_id.contains('/') || motif_id.contains('\\') {
        return Err(err("invalid motif id"));
    }
    let out = absolute(&PathBuf::from(
        opt(args, "--out").ok_or_else(|| err("export needs --out DIR"))?,
    ));
    let local_map = absolute(&PathBuf::from(
        opt(args, "--local-map").ok_or_else(|| err("export needs --local-map FILE"))?,
    ));
    if local_map.starts_with(&out) {
        return Err(err("private local map must live outside the bundle directory"));
    }
    if out.exists() {
        let entries = fs::read_dir(&out).map_err(|e| err(format!("read output directory: {e}")))?;
        if entries.count() != 0 {
            return Err(err("export output directory is not empty"));
        }
    }
    let targets = collect_addresses(args)?;
    let (idx, store, published) = require_publication(args)?;
    let config = pinned_config();
    let mut sessions = BTreeSet::new();
    let mut tasks = Vec::with_capacity(targets.len());
    for target in &targets {
        let member =
            ase_motif::membership_scoped(&idx, &store, &target.token, target.project.as_deref())?;
        for attempt in &member.attempts {
            sessions.insert(attempt.child_session_id.clone());
        }
        tasks.push(ase_motif::project(&published, &idx, &member)?);
    }
    let report = ase_motif::detect(&published, &idx, &store, &tasks, &config)?;
    let motif = report
        .motifs
        .iter()
        .find(|m| m.id == motif_id)
        .ok_or_else(|| {
            err(format!(
                "motif {motif_id:?} is not in the pinned query result for the selected addresses on the current publication; it is not a general occurrence id"
            ))
        })?;

    let exe = std::env::current_exe().map_err(|e| err(format!("current executable: {e}")))?;
    let binary_sha256 = digest_file(&exe)?;
    let bundle_base = build_bundle_base(&published, &report, &config, &binary_sha256)?;
    let reference_occurrence = motif
        .occurrences
        .iter()
        .find(|occ| {
            occ.task == motif.reference_task
                && occ.attempt == motif.reference_attempt
                && occ.sequence == motif.reference_sequence
        })
        .ok_or_else(|| err("motif reference occurrence is not reopenable"))?;
    let reference = occurrence_id(&occurrence_value(reference_occurrence))?;
    let motif_skeleton = build_motif_skeleton(motif, &reference);
    let prepared = prepare_occurrences(
        motif,
        &config,
        &idx,
        &store,
        &published,
        &bundle_base,
        &motif_skeleton,
    )?;
    if prepared.is_empty() {
        return Err(err("motif has no eligible non-duplicate occurrences"));
    }

    let stage = absolute(Path::new(".tmp/ase-export-stage"));
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(|e| err(format!("clear staging directory: {e}")))?;
    }
    let stage_bundle = stage.join("bundle");
    fs::create_dir_all(&stage_bundle).map_err(|e| err(format!("create staging directory: {e}")))?;

    let mut selected = select_representatives(&prepared);
    cap_selection(&mut selected);
    let mut assembled = assemble(
        &stage_bundle,
        &bundle_base,
        &motif_skeleton,
        &prepared,
        &selected,
        prepared.len(),
    )?;
    while assembled.total_bytes > MAX_BUNDLE_BYTES {
        let Some(position) = removable_position(&selected) else {
            return Err(err("bundle exceeds 32768 bytes with only earliest/latest left"));
        };
        selected.remove(position);
        assembled = assemble(
            &stage_bundle,
            &bundle_base,
            &motif_skeleton,
            &prepared,
            &selected,
            prepared.len(),
        )?;
    }

    let mut private_strings = BTreeSet::new();
    private_strings.insert(published.generation.clone());
    for session in &sessions {
        private_strings.insert(session.clone());
    }
    for motif_row in &report.motifs {
        for occurrence in &motif_row.occurrences {
            private_strings.insert(occurrence.task.clone());
            private_strings.insert(occurrence.attempt.clone());
            private_strings.insert(occurrence.dispatch_id.clone());
            for call in &occurrence.calls {
                private_strings.insert(call.evidence.selector.clone());
            }
        }
    }
    for row in &prepared {
        for value in &row.private {
            private_strings.insert(value.clone());
        }
    }
    for file in 0..idx.file_count {
        let path = absolute(Path::new(idx.file(file)?.path()))
            .to_string_lossy()
            .replace('\\', "/");
        private_strings.insert(path);
    }
    scan_no_leaks(&assembled.files, &assembled.motif, &private_strings)?;

    let mut aliases = HashMap::new();
    for file in &published.manifest.files {
        aliases.insert(
            file.id as u64,
            source_alias(file.id as u64, &format!("sha256:{}", file.digest)),
        );
    }
    let map = build_local_map(&published, &idx, &aliases)?;
    let stage_map = stage.join("local-map.json");
    write_json(&stage_map, &map)?;

    let report_value = verify(&stage_bundle, Some(&stage_map), None);
    if report_value["status"].as_str() != Some("verified") {
        let codes: Vec<String> = report_value["failures"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|failure| failure["code"].as_str().map(str::to_string))
            .collect();
        return Err(err(format!(
            "prepublication verification refused the bundle: {}",
            codes.join(",")
        )));
    }
    if !report_value["watermarks"].as_array().map_or(false, Vec::is_empty) {
        return Err(err("prepublication verification found an append watermark"));
    }

    if let Some(parent) = local_map.parent() {
        fs::create_dir_all(parent).map_err(|e| err(format!("create local map directory: {e}")))?;
    }
    write_json(&local_map, &map)?;
    fs::create_dir_all(&out).map_err(|e| err(format!("create output directory: {e}")))?;
    let mut published_paths = Vec::with_capacity(assembled.files.len());
    for (rel, bytes) in &assembled.files {
        let path = out.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| err(format!("create output directory: {e}")))?;
        }
        fs::write(&path, bytes).map_err(|e| err(format!("write failed: {e}")))?;
        published_paths.push(rel.clone());
    }

    let summary = json!({
        "command": "export",
        "status": "exported",
        "scope": "bundle export and prepublication reopen; no gate measurement",
        "motif_id": motif_id,
        "snapshot_id": assembled.bundle["snapshot"]["snapshot_id"],
        "core_digest": assembled.bundle["snapshot"]["core_digest"],
        "bundle_id": assembled.bundle["bundle_id"],
        "bundle_digest": assembled.bundle["bundle_digest"],
        "bundle_dir": out.to_string_lossy().replace('\\', "/"),
        "local_map": local_map.to_string_lossy().replace('\\', "/"),
        "files": published_paths,
        "occurrences_total": prepared.len(),
        "representatives_selected": selected.len(),
        "representatives_shortfall": selected.len() < prepared.len(),
        "evidence_total": report_value["evidence_total"],
        "evidence_verified": report_value["evidence_verified"],
        "verification": {
            "status": report_value["status"],
            "failures": report_value["failures"],
            "watermarks": report_value["watermarks"],
        },
        "note": "Only observations and questions are exported; verdicts, catalog entries, raw output, real paths and alias mappings stay out of the bundle.",
    });
    if has(args, "--json") {
        outln!("{}", serde_json::to_string_pretty(&summary).unwrap());
    } else {
        outln!(
            "bundle export: {}; bundle {}; representatives {}/{}; evidence {}/{}; verification {}",
            motif_id,
            assembled.bundle["bundle_id"].as_str().unwrap_or(""),
            selected.len(),
            prepared.len(),
            report_value["evidence_verified"].as_u64().unwrap_or(0),
            report_value["evidence_total"].as_u64().unwrap_or(0),
            report_value["status"].as_str().unwrap_or("unverified"),
        );
    }
    Ok(())
}
