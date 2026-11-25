//! P4 skill/template catalog join per `artifacts/sprint-7/ASE-S7-T1/contract-a1.md` §5 and §6.
//!
//! This module reads a local `ase-p4-assertions-v1` layer written by `ase import
//! verdicts`, reopens the cited bundle evidence through the private local map,
//! and compares only today's verified assertions against a user-supplied
//! `ase-skill-catalog-v1` file. Comparison is exact byte equality on
//! `(kind, key/alias)`; there is no inference, recommendation, model call, gate
//! measurement or write to any store. No private source path enters the output.

use ase_core::{err, Result};
use ase_index::sha256_hex;
use crate::p4_verify::{self, BUNDLE_VERSION};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const CATALOG_SCHEMA: &str = "ase-skill-catalog-v1";
const JOIN_SCHEMA: &str = "ase-p4-join-v1";
const ASSERTIONS_SCHEMA: &str = "ase-p4-assertions-v1";
const NO_MATCH_MEANING: &str = "no entry in the supplied catalog matched the declared key; this is not evidence that no skill exists or that one should be created";

struct Catalog {
    id: String,
    sha256: String,
    malformed: bool,
    entries: BTreeMap<(String, String), BTreeSet<String>>,
    aliases: BTreeMap<(String, String), BTreeSet<String>>,
    defects: BTreeMap<(String, String), BTreeSet<String>>,
    completeness: BTreeMap<String, bool>,
}

struct BundleView {
    bundle: Value,
    bundle_id: String,
    occurrences: HashMap<String, Value>,
    span_refs: HashMap<String, Vec<Value>>,
    evidence: HashSet<String>,
}

struct JoinRow {
    row: Value,
    status: &'static str,
}

// ---------------------------------------------------------------- CLI surface

pub fn run(args: &[String]) -> Result<()> {
    if args.first().map(String::as_str) != Some("skill-gap") {
        return Err(err("join needs a `skill-gap <options>` target"));
    }
    let mut store: Option<String> = None;
    let mut catalog: Option<String> = None;
    let mut bundle: Option<String> = None;
    let mut local_map: Option<String> = None;
    let mut verdicts: Option<String> = None;
    let mut as_json = false;
    let mut i = 1usize;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                as_json = true;
                i += 1;
            }
            flag @ ("--store" | "--catalog" | "--bundle" | "--local-map" | "--verdicts") => {
                let value = args
                    .get(i + 1)
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| err(format!("{flag} needs a value")))?
                    .clone();
                let slot = match flag {
                    "--store" => &mut store,
                    "--catalog" => &mut catalog,
                    "--bundle" => &mut bundle,
                    "--local-map" => &mut local_map,
                    _ => &mut verdicts,
                };
                if slot.is_some() {
                    return Err(err(format!("duplicate {flag} option")));
                }
                *slot = Some(value);
                i += 2;
            }
            other => return Err(err(format!("unknown join option: {other}"))),
        }
    }
    let store_path = PathBuf::from(store.ok_or_else(|| err("join skill-gap needs --store FILE"))?);
    let catalog_path = PathBuf::from(catalog.ok_or_else(|| err("join skill-gap needs --catalog FILE"))?);
    let bundle_dir = PathBuf::from(bundle.ok_or_else(|| err("join skill-gap needs --bundle DIR"))?);
    let map_path = PathBuf::from(local_map.ok_or_else(|| err("join skill-gap needs --local-map FILE"))?);

    let assertions = load_store(&store_path)?;
    let catalog = load_catalog(&catalog_path)?;
    let view = load_bundle_view(&bundle_dir)?;
    let verify = p4_verify::verify(&bundle_dir, Some(&map_path), None);
    let evidence_state = evidence_reopen(&verify, &view);
    let verdict_rows = match verdicts.as_deref() {
        Some(path) => Some(load_verdict_rows(Path::new(path))?),
        None => None,
    };
    let verdict_file_digest = verdicts
        .as_deref()
        .map(|path| {
            fs::read(path)
                .map(|bytes| format!("sha256:{}", sha256_hex(&bytes)))
                .map_err(|_| err("verdict file not readable"))
        })
        .transpose()?;

    let mut rows: Vec<JoinRow> = Vec::new();
    let mut excluded: Vec<Value> = Vec::new();
    let mut notes = vec![
        "only assertions whose bundle identity and cited evidence reopen today are joined".to_string(),
        "no_match is not evidence that a skill is absent or should be created".to_string(),
        "the assertion layer is read-only; this command writes no store, bundle or verdict file".to_string(),
    ];
    for (id, assertion) in &assertions {
        if let Some((code, detail)) = lineage_failure(
            assertion,
            &view,
            &evidence_state,
            verdict_rows.as_ref(),
            verdict_file_digest.as_deref(),
        ) {
            excluded.push(json!({
                "assertion_id": id,
                "occurrence_id": assertion.get("occurrence_id").cloned().unwrap_or(Value::Null),
                "code": code,
                "detail": detail,
            }));
            continue;
        }
        rows.push(join_row(assertion, &catalog));
    }
    rows.sort_by(|a, b| {
        a.row["assertion_id"]
            .as_str()
            .unwrap_or("")
            .cmp(b.row["assertion_id"].as_str().unwrap_or(""))
    });
    excluded.sort_by(|a, b| {
        a["assertion_id"].as_str().unwrap_or("").cmp(b["assertion_id"].as_str().unwrap_or(""))
    });

    let mut counts = BTreeMap::new();
    for key in ["match", "no_match", "ambiguous", "insufficient_input"] {
        counts.insert(key, 0usize);
    }
    for row in &rows {
        *counts.get_mut(row.status).expect("known status") += 1;
    }
    let status = if rows.is_empty() && !assertions.is_empty() {
        "failed_closed"
    } else {
        "joined"
    };
    if status == "failed_closed" {
        notes.push("no verified assertion survived the fresh lineage check; no catalog conclusion was drawn".to_string());
    }
    let report = json!({
        "command": "join",
        "action": "skill-gap",
        "scope": "P4 deterministic catalog join of verified assertions; no model, gate measurement or source path",
        "schema": JOIN_SCHEMA,
        "status": status,
        "catalog_id": catalog.id,
        "catalog_sha256": catalog.sha256,
        "bundle_id": view.bundle_id,
        "bundle_digest": view.bundle.get("bundle_digest").cloned().unwrap_or(Value::Null),
        "no_match_meaning": NO_MATCH_MEANING,
        "verify_status": verify["status"].as_str().unwrap_or("unverified"),
        "evidence_total": verify["evidence_total"],
        "evidence_verified": verify["evidence_verified"],
        "assertions_total": assertions.len(),
        "rows_total": rows.len(),
        "excluded_total": excluded.len(),
        "status_counts": counts,
        "rows": rows.iter().map(|item| &item.row).collect::<Vec<&Value>>(),
        "excluded": excluded,
        "notes": notes,
    });
    emit(&report, as_json)
}

fn emit(report: &Value, as_json: bool) -> Result<()> {
    if as_json {
        println!("{}", serde_json::to_string_pretty(report).map_err(|e| err(format!("report: {e}")))?);
    } else {
        println!(
            "p4 join skill-gap: {}; rows {} (match {}, no_match {}, ambiguous {}, insufficient_input {}), excluded {}; catalog {}, bundle {}",
            report["status"].as_str().unwrap_or("failed_closed"),
            report["rows_total"].as_u64().unwrap_or(0),
            report["status_counts"]["match"].as_u64().unwrap_or(0),
            report["status_counts"]["no_match"].as_u64().unwrap_or(0),
            report["status_counts"]["ambiguous"].as_u64().unwrap_or(0),
            report["status_counts"]["insufficient_input"].as_u64().unwrap_or(0),
            report["excluded_total"].as_u64().unwrap_or(0),
            report["catalog_id"].as_str().unwrap_or("unknown"),
            report["bundle_id"].as_str().unwrap_or("unknown"),
        );
    }
    Ok(())
}

// ------------------------------------------------------------- local inputs

/// Contract §5.3 treats malformed catalogs as `insufficient_input`, not as a
/// crashed command. Unreadable files are still a hard CLI error.
fn load_catalog(path: &Path) -> Result<Catalog> {
    let bytes = fs::read(path).map_err(|_| err("catalog file not readable"))?;
    let sha256 = format!("sha256:{}", sha256_hex(&bytes));
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let mut catalog = Catalog {
        id: value.get("catalog_id").and_then(Value::as_str).unwrap_or("").to_string(),
        sha256,
        malformed: false,
        entries: BTreeMap::new(),
        aliases: BTreeMap::new(),
        defects: BTreeMap::new(),
        completeness: BTreeMap::new(),
    };
    if value.get("schema").and_then(Value::as_str) != Some(CATALOG_SCHEMA) || catalog.id.is_empty() {
        catalog.malformed = true;
        return Ok(catalog);
    }
    if let Some(map) = value.get("completeness").and_then(Value::as_object) {
        for (kind, complete) in map {
            if let Some(flag) = complete.as_bool() {
                catalog.completeness.insert(kind.clone(), flag);
            }
        }
    }
    let Some(list) = value.get("entries").and_then(Value::as_array) else {
        catalog.malformed = true;
        return Ok(catalog);
    };
    for entry in list {
        let Some(entry_id) = entry.get("entry_id").and_then(Value::as_str) else {
            catalog.malformed = true;
            continue;
        };
        let Some(kind) = entry.get("kind").and_then(Value::as_str) else {
            catalog.malformed = true;
            continue;
        };
        if kind != "skill" && kind != "template" {
            catalog.malformed = true;
            continue;
        }
        let Some(key) = entry.get("key").and_then(Value::as_str) else {
            catalog.malformed = true;
            continue;
        };
        if !catalog_key_valid(key) {
            catalog.malformed = true;
            continue;
        }
        let aliases = match entry.get("aliases") {
            Some(Value::Array(values)) => {
                let mut valid = Vec::new();
                let mut ok = true;
                for alias in values {
                    match alias.as_str() {
                        Some(text) if catalog_key_valid(text) => valid.push(text.to_string()),
                        _ => ok = false,
                    }
                }
                if !ok {
                    catalog.malformed = true;
                }
                valid
            }
            Some(Value::Null) | None => Vec::new(),
            _ => {
                catalog.malformed = true;
                Vec::new()
            }
        };
        catalog
            .entries
            .entry((kind.to_string(), key.to_string()))
            .or_default()
            .insert(entry_id.to_string());
        for alias in aliases {
            catalog
                .aliases
                .entry((kind.to_string(), alias))
                .or_default()
                .insert(entry_id.to_string());
        }
    }
    for ((kind, key), ids) in &catalog.entries {
        if ids.len() > 1 {
            catalog.defects.entry((kind.clone(), key.clone())).or_default().insert("catalog_duplicate_key".to_string());
        }
    }
    for ((kind, alias), ids) in &catalog.aliases {
        if ids.len() > 1 {
            catalog.defects.entry((kind.clone(), alias.clone())).or_default().insert("catalog_alias_conflict".to_string());
        }
        if let Some(primary) = catalog.entries.get(&(kind.clone(), alias.clone())) {
            let union: BTreeSet<&String> = ids.iter().chain(primary.iter()).collect();
            if union.len() > 1 {
                catalog.defects.entry((kind.clone(), alias.clone())).or_default().insert("catalog_alias_conflict".to_string());
            }
        }
    }
    Ok(catalog)
}

fn load_store(path: &Path) -> Result<BTreeMap<String, Value>> {
    let bytes = fs::read(path).map_err(|_| err("assertion store not readable"))?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| err("assertion store not JSON"))?;
    if value.get("schema").and_then(Value::as_str) != Some(ASSERTIONS_SCHEMA) {
        return Err(err("assertion store schema unsupported"));
    }
    let mut body = value.clone();
    let recorded = body.get("store_digest").and_then(Value::as_str).unwrap_or("").to_string();
    body.as_object_mut().expect("store object").remove("store_digest");
    if recorded != format!("sha256:{}", p4_verify::value_digest(&body)) {
        return Err(err("assertion store digest mismatch"));
    }
    let mut assertions = BTreeMap::new();
    for row in value.get("assertions").and_then(Value::as_array).cloned().unwrap_or_default() {
        if let Some(id) = row.get("assertion_id").and_then(Value::as_str) {
            assertions.insert(id.to_string(), row);
        }
    }
    Ok(assertions)
}

fn load_verdict_rows(path: &Path) -> Result<HashMap<String, Value>> {
    let bytes = fs::read(path).map_err(|_| err("verdict file not readable"))?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| err("verdict file not JSON"))?;
    let mut rows = HashMap::new();
    for row in value.get("verdicts").and_then(Value::as_array).cloned().unwrap_or_default() {
        if let Some(id) = row.get("assertion_id").and_then(Value::as_str) {
            rows.insert(id.to_string(), row);
        }
    }
    Ok(rows)
}

fn load_bundle_view(dir: &Path) -> Result<BundleView> {
    let bytes = fs::read(dir.join("bundle.json")).map_err(|_| err("bundle.json not readable"))?;
    let bundle: Value = serde_json::from_slice(&bytes).map_err(|_| err("bundle.json not JSON"))?;
    let bundle_id = bundle.get("bundle_id").and_then(Value::as_str).unwrap_or("").to_string();
    let mut occurrences = HashMap::new();
    let mut span_refs = HashMap::new();
    let mut evidence = HashSet::new();
    let motifs = bundle.get("motifs").and_then(Value::as_array).cloned().unwrap_or_default();
    for motif_row in motifs {
        let rel = motif_row.get("motif_path").and_then(Value::as_str).unwrap_or("");
        let bytes = fs::read(dir.join(rel)).map_err(|_| err("motif.json not readable"))?;
        let motif: Value = serde_json::from_slice(&bytes).map_err(|_| err("motif.json not JSON"))?;
        let motif_id = motif.get("motif_id").cloned().unwrap_or(Value::Null);
        let reps = motif.get("representatives").and_then(Value::as_array).cloned().unwrap_or_default();
        for rep in reps {
            let occ_id = rep.get("occurrence_id").and_then(Value::as_str).unwrap_or("").to_string();
            let mut indexed = rep.clone();
            indexed["motif_id"] = motif_id.clone();
            let mut refs = Vec::new();
            for row in rep.get("evidence").and_then(Value::as_array).cloned().unwrap_or_default() {
                let ev_id = row.get("evidence_id").and_then(Value::as_str).unwrap_or("").to_string();
                evidence.insert(ev_id.clone());
                let call = row.get("call").unwrap_or(&Value::Null);
                refs.push(json!({
                    "evidence_id": ev_id,
                    "file_id": call.get("file_id").cloned().unwrap_or(Value::Null),
                    "selector": call.get("selector").cloned().unwrap_or(Value::Null),
                    "block_sha256": call.get("block_sha256").cloned().unwrap_or(Value::Null),
                }));
            }
            span_refs.insert(occ_id.clone(), refs);
            occurrences.insert(occ_id, indexed);
        }
    }
    Ok(BundleView { bundle, bundle_id, occurrences, span_refs, evidence })
}

/// Mirror of the T4 reopen rule: only a currently readable and digest-matching
/// span may support a join row. Bundle-level failures mark every span
/// unverifiable, so no assertion can be promoted from them.
fn evidence_reopen(verify: &Value, view: &BundleView) -> HashMap<String, &'static str> {
    let mut state: HashMap<String, &'static str> =
        view.evidence.iter().map(|id| (id.clone(), "unverified")).collect();
    if verify["status"].as_str() == Some("verified") {
        for id in &view.evidence {
            state.insert(id.clone(), "verified");
        }
        for watermark in verify["watermarks"].as_array().cloned().unwrap_or_default() {
            if let Some(id) = watermark.get("evidence_id").and_then(Value::as_str) {
                state.insert(id.to_string(), "verified_span_after_append");
            }
        }
        return state;
    }
    let failures = verify["failures"].as_array().cloned().unwrap_or_default();
    if failures.is_empty() {
        return state;
    }
    for failure in &failures {
        let code = failure.get("code").and_then(Value::as_str).unwrap_or("bundle_tampered");
        let kind = if matches!(code, "alias_unknown" | "source_missing" | "source_unreadable") {
            "unverified"
        } else {
            "failed"
        };
        match failure.get("evidence_id").and_then(Value::as_str) {
            Some(id) => {
                state.insert(id.to_string(), kind);
            }
            None => {
                for id in &view.evidence {
                    state.insert(id.clone(), kind);
                }
            }
        }
    }
    state
}

fn reject(code: &'static str, detail: &str) -> Option<(&'static str, String)> {
    Some((code, detail.to_string()))
}

// -------------------------------------------------------------- freshness

/// First failing check wins, mirroring the contract's ordered rejection rule.
/// An assertion that fails any check is excluded (reported, never joined).
fn lineage_failure(
    assertion: &Value,
    view: &BundleView,
    state: &HashMap<String, &'static str>,
    verdict_rows: Option<&HashMap<String, Value>>,
    verdict_file_digest: Option<&str>,
) -> Option<(&'static str, String)> {
    let id = assertion.get("assertion_id").and_then(Value::as_str).unwrap_or("");
    let occ_id = assertion.get("occurrence_id").and_then(Value::as_str).unwrap_or("");

    if assertion.get("bundle_id").and_then(Value::as_str) != Some(view.bundle_id.as_str()) {
        return reject("bundle_id_mismatch", "assertion bundle id differs from the reopened bundle");
    }
    if assertion.get("bundle_digest").and_then(Value::as_str)
        != view.bundle.get("bundle_digest").and_then(Value::as_str)
        || assertion.get("bundle_version").and_then(Value::as_str) != Some(BUNDLE_VERSION)
    {
        return reject("stale_bundle", "assertion bundle digest or version is stale");
    }
    let Some(occurrence) = view.occurrences.get(occ_id) else {
        return reject("occurrence_unknown", "occurrence is not in the reopened bundle");
    };
    if assertion.get("input_identity").and_then(Value::as_str)
        != occurrence.get("input_identity").and_then(Value::as_str)
    {
        return reject("input_identity_mismatch", "judged bundle content is stale");
    }
    let refs: Vec<String> = assertion
        .get("evidence_refs")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    if refs.is_empty() {
        return reject("missing_evidence", "assertion has no evidence refs");
    }
    for ev in &refs {
        if !view.evidence.contains(ev) {
            return reject("missing_evidence", "evidence ref is not in the reopened bundle");
        }
        if !matches!(state.get(ev).copied(), Some("verified" | "verified_span_after_append")) {
            return reject("evidence_unverified", "cited evidence did not reopen today");
        }
    }
    let expected = format!("as-{}", &p4_verify::value_digest(&p4_verify::assertion_body(&view.bundle, assertion))[..16]);
    if id != expected {
        return reject("assertion_id_conflict", "assertion id differs from the stored body");
    }
    if !sha256_field_valid(assertion.get("verdict_digest").and_then(Value::as_str)) {
        return reject("verdict_digest_malformed", "verdict digest is missing or malformed");
    }
    if !sha256_field_valid(assertion.get("verdict_file_digest").and_then(Value::as_str)) {
        return reject("verdict_file_digest_malformed", "verdict-file provenance is missing or malformed");
    }
    if assertion.get("judge").and_then(|j| j.get("kind")).and_then(Value::as_str)
        != Some("user")
        && assertion.get("judge").and_then(|j| j.get("kind")).and_then(Value::as_str)
            != Some("external_llm")
    {
        return reject("judge_missing", "assertion judge provenance is missing");
    }
    let stored_spans = assertion.get("evidence_span_refs").and_then(Value::as_array).cloned().unwrap_or_default();
    let current_spans = view.span_refs.get(occ_id).cloned().unwrap_or_default();
    if !same_spans(&stored_spans, &current_spans) {
        return reject("span_ref_mismatch", "path-free span references differ from the bundle");
    }
    if let Some(rows) = verdict_rows {
        let Some(verdict) = rows.get(id) else {
            return reject("verdict_missing", "assertion is absent from the supplied verdict file");
        };
        let digest = format!("sha256:{}", p4_verify::value_digest(verdict));
        if assertion.get("verdict_digest").and_then(Value::as_str) != Some(digest.as_str()) {
            return reject("verdict_digest_mismatch", "verdict digest differs from the supplied verdict file");
        }
        if verdict.get("input_identity").and_then(Value::as_str)
            != assertion.get("input_identity").and_then(Value::as_str)
        {
            return reject("verdict_input_identity_mismatch", "verdict file content is stale");
        }
        if let Some(file_digest) = verdict_file_digest {
            if assertion.get("verdict_file_digest").and_then(Value::as_str) != Some(file_digest) {
                return reject("verdict_file_digest_mismatch", "verdict file digest differs from the supplied file");
            }
        }
    }
    None
}

fn sha256_field_valid(value: Option<&str>) -> bool {
    matches!(value, Some(text) if text.strip_prefix("sha256:").is_some_and(|body| {
        body.len() == 64 && body.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }))
}

fn same_spans(stored: &[Value], current: &[Value]) -> bool {
    let key = |row: &Value| -> Value {
        json!({
            "evidence_id": row.get("evidence_id").cloned().unwrap_or(Value::Null),
            "file_id": row.get("file_id").cloned().unwrap_or(Value::Null),
            "selector": row.get("selector").cloned().unwrap_or(Value::Null),
            "block_sha256": row.get("block_sha256").cloned().unwrap_or(Value::Null),
        })
    };
    let stored: BTreeSet<String> = stored.iter().map(|row| p4_verify::value_digest(&key(row))).collect();
    let current: BTreeSet<String> = current.iter().map(|row| p4_verify::value_digest(&key(row))).collect();
    stored == current
}

// ------------------------------------------------------------------- join

fn catalog_key_valid(key: &str) -> bool {
    let bytes = key.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes[1..].iter().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b':' | b'-')
        })
}

fn join_row(assertion: &Value, catalog: &Catalog) -> JoinRow {
    let assertion_id = assertion.get("assertion_id").and_then(Value::as_str).unwrap_or("").to_string();
    let kind = assertion.get("catalog_kind").and_then(Value::as_str);
    let key = assertion.get("catalog_key").and_then(Value::as_str);
    let mut reasons: Vec<String> = Vec::new();
    let mut status: &'static str = "insufficient_input";
    let mut entry_ids: BTreeSet<String> = BTreeSet::new();

    if catalog.malformed {
        reasons.push("catalog_malformed".to_string());
    } else if catalog.entries.is_empty() && catalog.aliases.is_empty() {
        reasons.push("catalog_empty".to_string());
    } else {
        match kind {
            None => reasons.push("verdict_kind_missing".to_string()),
            Some(k) if k != "skill" && k != "template" => reasons.push("catalog_kind_invalid".to_string()),
            Some(_) => {}
        }
        match key {
            None => reasons.push("verdict_key_missing".to_string()),
            Some(k) if !catalog_key_valid(k) => reasons.push("catalog_key_invalid".to_string()),
            Some(_) => {}
        }
        if reasons.is_empty() {
            let kind = kind.unwrap_or("");
            let key = key.unwrap_or("");
            let lookup = (kind.to_string(), key.to_string());
            if catalog.completeness.get(kind) != Some(&true) {
                reasons.push("catalog_incomplete".to_string());
            }
            if let Some(ids) = catalog.entries.get(&lookup) {
                entry_ids.extend(ids.iter().cloned());
            }
            if let Some(ids) = catalog.aliases.get(&lookup) {
                entry_ids.extend(ids.iter().cloned());
            }
            if let Some(defects) = catalog.defects.get(&lookup) {
                reasons.extend(defects.iter().cloned());
                status = "ambiguous";
            } else if entry_ids.len() > 1 {
                reasons.push("catalog_key_alias_collision".to_string());
                status = "ambiguous";
            } else if entry_ids.len() == 1 && reasons.is_empty() {
                status = "match";
            } else if entry_ids.is_empty() && reasons.is_empty() {
                status = "no_match";
            }
        }
    }
    if reasons.iter().any(|reason| reason == "catalog_incomplete") {
        status = "insufficient_input";
    }
    reasons.sort();
    reasons.dedup();
    if status == "insufficient_input" && reasons.is_empty() {
        reasons.push("insufficient_input".to_string());
    }
    let entry_ids: Vec<String> = entry_ids.into_iter().collect();
    let join_body = json!({
        "assertion_id": assertion_id,
        "catalog_id": catalog.id,
        "status": status,
        "sorted_entry_ids": entry_ids,
    });
    let join_id = format!("j-{}", &p4_verify::value_digest(&join_body)[..16]);
    let mut evidence_refs: Vec<Value> = assertion
        .get("evidence_refs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    evidence_refs.sort_by(|a, b| a.as_str().unwrap_or("").cmp(b.as_str().unwrap_or("")));
    let row = json!({
        "join_id": join_id,
        "assertion_id": assertion_id,
        "occurrence_id": assertion.get("occurrence_id").cloned().unwrap_or(Value::Null),
        "status": status,
        "entry_ids": entry_ids,
        "reasons": reasons,
        "evidence_refs": evidence_refs,
        "verdict_digest": assertion.get("verdict_digest").cloned().unwrap_or(Value::Null),
        "evidence_span_refs": assertion.get("evidence_span_refs").cloned().unwrap_or_default(),
    });
    JoinRow { row, status }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_key_grammar_is_exact() {
        assert!(catalog_key_valid("procedure:read-inspect"));
        assert!(catalog_key_valid("a"));
        assert!(!catalog_key_valid(""));
        assert!(!catalog_key_valid("Upper"));
        assert!(!catalog_key_valid("-leading"));
        assert!(!catalog_key_valid(&"x".repeat(65)));
    }

    #[test]
    fn digest_fields_require_prefixed_lowercase_hex() {
        assert!(sha256_field_valid(Some("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")));
        assert!(!sha256_field_valid(Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")));
        assert!(!sha256_field_valid(Some("sha256:FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF")));
        assert!(!sha256_field_valid(None));
    }

    #[test]
    fn span_reference_compares_as_an_unordered_set() {
        let a = json!([{"evidence_id":"ev-1","file_id":3,"selector":"s","block_sha256":"sha256:a"}]);
        let b = json!([{"evidence_id":"ev-1","file_id":3,"selector":"s","block_sha256":"sha256:a"}]);
        assert!(same_spans(a.as_array().unwrap(), b.as_array().unwrap()));
        let c = json!([{"evidence_id":"ev-1","file_id":4,"selector":"s","block_sha256":"sha256:a"}]);
        assert!(!same_spans(a.as_array().unwrap(), c.as_array().unwrap()));
    }
}
