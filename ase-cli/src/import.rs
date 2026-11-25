//! Verdict import for the local assertion layer.
//!
//! Verdicts are an imported judgment layer, never source evidence. The whole
//! bundle is reopened by `verify::verify` before anything is written, every
//! row is checked in the documented order, and accepted assertions are stored
//! in a separate local assertion file. No model, network, catalog or gate
//! measurement is part of this module.

use crate::verify::{self, value_digest, BUNDLE_VERSION};
use ase_core::{err, Result};
use ase_index::sha256_hex;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const VERDICTS_SCHEMA: &str = "ase-verdicts-v1";
const ASSERTIONS_SCHEMA: &str = "ase-assertions-v1";
const ASSERTIONS_VERSION: &str = "1";

#[derive(Clone)]
struct Outcome {
    assertion_id: Option<String>,
    occurrence_id: Option<String>,
    question_id: Option<String>,
    code: String,
    detail: String,
    stored: bool,
}

impl Outcome {
    fn value(&self) -> Value {
        json!({
            "assertion_id": self.assertion_id,
            "occurrence_id": self.occurrence_id,
            "question_id": self.question_id,
            "code": self.code,
            "detail": self.detail,
            "stored": self.stored,
        })
    }
}

struct Summary {
    status: &'static str,
    verdicts_total: usize,
    accepted: usize,
    duplicate_ignored: usize,
    rejected: usize,
    assertions_total: usize,
    failure_code: Option<String>,
    verify_status: String,
    evidence_total: u64,
    evidence_verified: u64,
    outcomes: Vec<Outcome>,
    notes: Vec<String>,
}

struct BundleView {
    bundle: Value,
    occurrences: HashMap<String, Value>,
    evidence: HashSet<String>,
    span_refs: HashMap<String, Vec<Value>>,
}

struct Store {
    value: Value,
    assertions: BTreeMap<String, Value>,
    registry: BTreeMap<String, String>,
    sources: Vec<Value>,
}

// ---------------------------------------------------------------- CLI surface

pub fn run(args: &[String]) -> Result<()> {
    if args.first().map(String::as_str) != Some("verdicts") {
        return Err(err("import needs a `verdicts <file>` target"));
    }
    let verdict_path = args
        .get(1)
        .filter(|value| !value.starts_with("--"))
        .ok_or_else(|| err("import verdicts needs a verdict file"))?
        .clone();
    let mut bundle: Option<String> = None;
    let mut local_map: Option<String> = None;
    let mut store: Option<String> = None;
    let mut as_json = false;
    let mut i = 2usize;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                as_json = true;
                i += 1;
            }
            flag @ ("--bundle" | "--local-map" | "--store") => {
                let value = args
                    .get(i + 1)
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| err(format!("{flag} needs a value")))?
                    .clone();
                let slot = match flag {
                    "--bundle" => &mut bundle,
                    "--local-map" => &mut local_map,
                    _ => &mut store,
                };
                if slot.is_some() {
                    return Err(err(format!("duplicate {flag} option")));
                }
                *slot = Some(value);
                i += 2;
            }
            other => return Err(err(format!("unknown import option: {other}"))),
        }
    }
    let bundle = PathBuf::from(bundle.ok_or_else(|| err("import verdicts needs --bundle DIR"))?);
    let map_path = PathBuf::from(local_map.ok_or_else(|| err("import verdicts needs --local-map FILE"))?);
    let store_path = PathBuf::from(store.ok_or_else(|| err("import verdicts needs --store FILE"))?);

    let verdict_bytes = fs::read(&verdict_path).map_err(|_| err("verdict file not readable"))?;
    let verdicts = serde_json::from_slice::<Value>(&verdict_bytes)
        .map_err(|_| err("verdict file not JSON"))?;
    let verdict_file_digest = format!("sha256:{}", sha256_hex(&verdict_bytes));
    let bundle_bytes = fs::read(bundle.join("bundle.json"))
        .map_err(|_| err("bundle.json not readable"))?;
    let bundle_json = serde_json::from_slice::<Value>(&bundle_bytes)
        .map_err(|_| err("bundle.json not JSON"))?;

    let rows = verdicts.get("verdicts").and_then(Value::as_array);
    if let Some(code) = envelope_code(&verdicts, &bundle_json, rows) {
        let outcomes = rows
            .map(|rows| {
                rows.iter()
                    .map(|row| Outcome {
                        assertion_id: row.get("assertion_id").and_then(Value::as_str).map(str::to_string),
                        occurrence_id: row.get("occurrence_id").and_then(Value::as_str).map(str::to_string),
                        question_id: row.get("question_id").and_then(Value::as_str).map(str::to_string),
                        code: code.to_string(),
                        detail: "verdict envelope rejected before any write".to_string(),
                        stored: false,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let summary = Summary {
            status: "rejected",
            failure_code: Some(code.to_string()),
            verdicts_total: rows.map_or(0, Vec::len),
            accepted: 0,
            duplicate_ignored: 0,
            rejected: rows.map_or(0, Vec::len),
            assertions_total: 0,
            verify_status: "not_run".to_string(),
            evidence_total: 0,
            evidence_verified: 0,
            outcomes,
            notes: vec!["no assertion store was read or written".to_string()],
        };
        return emit(&report(&summary, &verdicts, &bundle_json, &verdict_file_digest), as_json);
    }

    let view = load_bundle_view(&bundle)?;
    let (verify_report, evidence_state, evidence_codes) =
        evidence_reopen(&bundle, &map_path, &view);
    let mut store = load_store(&store_path)?;

    let checks = check_rows(&view, &verify_report, &evidence_state, &evidence_codes, &store, &verdicts);
    let rejected = checks.iter().filter(|o| o.code != "accepted" && o.code != "duplicate_ignored").count();
    let accepted = checks.iter().filter(|o| o.code == "accepted").count();
    let duplicate = checks.iter().filter(|o| o.code == "duplicate_ignored").count();
    let rows_array = verdicts
        .get("verdicts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut inserted: BTreeMap<String, Value> = BTreeMap::new();
    for row in &rows_array {
        let Some(id) = row.get("assertion_id").and_then(Value::as_str) else {
            continue;
        };
        if !checks.iter().any(|o| o.assertion_id.as_deref() == Some(id) && o.code == "accepted") {
            continue;
        }
        if store.assertions.contains_key(id) || inserted.contains_key(id) {
            continue;
        }
        inserted.insert(id.to_string(), assertion_record(row, &verdicts, &verdict_file_digest, &view, &bundle_json));
    }
    let wrote = !inserted.is_empty();
    if wrote {
        for (id, record) in inserted {
            store.assertions.insert(id, record);
        }
        let bundle_id = bundle_json.get("bundle_id").and_then(Value::as_str).unwrap_or("");
        for evidence_id in &view.evidence {
            store.registry.insert(evidence_id.clone(), bundle_id.to_string());
        }
        if !store.sources.iter().any(|entry| {
            entry.get("verdict_file_digest").and_then(Value::as_str) == Some(verdict_file_digest.as_str())
        }) {
            store.sources.push(json!({
                "verdict_file_digest": verdict_file_digest,
                "judge": verdicts.get("judge").cloned().unwrap_or(Value::Null),
                "generated_at": verdicts.get("generated_at").cloned().unwrap_or(Value::Null),
                "bundle_id": bundle_id,
                "bundle_digest": bundle_json.get("bundle_digest").cloned().unwrap_or(Value::Null),
                "verdicts_total": rows_array.len(),
                "accepted": accepted,
                "duplicate_ignored": duplicate,
            }));
        }
        store.value["assertions"] = json!(store.assertions.values().cloned().collect::<Vec<Value>>());
        store.value["bundle_registry"] = json!(store.registry);
        store.value["sources"] = json!(store.sources);
        store.value.as_object_mut().expect("store object").remove("store_digest");
        store.value["store_digest"] = json!(format!("sha256:{}", value_digest(&store.value)));
        write_store(&store_path, &store.value)?;
    }

    let status = if rejected > 0 {
        "rejected"
    } else if accepted > 0 {
        "accepted"
    } else {
        "duplicate"
    };
    let mut notes = Vec::new();
    if rejected > 0 {
        notes.push("rejected rows were never stored; only verified rows were inserted atomically".to_string());
    }
    notes.push(if wrote {
        "assertion layer is local-only; no bundle, model, catalog or gate result was written".to_string()
    } else {
        "no store write was needed for this replay or rejection".to_string()
    });
    let summary = Summary {
        status,
        failure_code: None,
        verdicts_total: checks.len(),
        accepted,
        duplicate_ignored: duplicate,
        rejected,
        assertions_total: store.assertions.len(),
        verify_status: verify_report["status"].as_str().unwrap_or("unverified").to_string(),
        evidence_total: verify_report["evidence_total"].as_u64().unwrap_or(0),
        evidence_verified: verify_report["evidence_verified"].as_u64().unwrap_or(0),
        outcomes: checks,
        notes,
    };
    emit(&report(&summary, &verdicts, &bundle_json, &verdict_file_digest), as_json)
}

// -------------------------------------------------------------------- helpers

fn emit(report: &Value, as_json: bool) -> Result<()> {
    if as_json {
        println!("{}", serde_json::to_string_pretty(report).map_err(|e| err(format!("report: {e}")))?);
    } else {
        println!(
            "import verdicts: {}; verdicts {} ({} new, {} duplicate, {} rejected), assertions {}",
            report["status"].as_str().unwrap_or("rejected"),
            report["verdicts_total"].as_u64().unwrap_or(0),
            report["accepted"].as_u64().unwrap_or(0),
            report["duplicate_ignored"].as_u64().unwrap_or(0),
            report["rejected"].as_u64().unwrap_or(0),
            report["assertions_total"].as_u64().unwrap_or(0),
        );
    }
    if report["status"].as_str() == Some("rejected") {
        std::process::exit(3);
    }
    Ok(())
}

fn report(summary: &Summary, verdicts: &Value, bundle: &Value, verdict_file_digest: &str) -> Value {
    json!({
        "command": "import",
        "action": "verdicts",
        "scope": "imported judgment layer; no model, catalog join or gate measurement",
        "status": summary.status,
        "failure_code": summary.failure_code,
        "verdict_schema": VERDICTS_SCHEMA,
        "assertion_schema": ASSERTIONS_SCHEMA,
        "verdict_file_digest": verdict_file_digest,
        "bundle_id": bundle.get("bundle_id").cloned().unwrap_or(Value::Null),
        "bundle_digest": bundle.get("bundle_digest").cloned().unwrap_or(Value::Null),
        "judge": verdicts.get("judge").cloned().unwrap_or(Value::Null),
        "generated_at": verdicts.get("generated_at").cloned().unwrap_or(Value::Null),
        "verify_status": summary.verify_status,
        "evidence_total": summary.evidence_total,
        "evidence_verified": summary.evidence_verified,
        "verdicts_total": summary.verdicts_total,
        "accepted": summary.accepted,
        "duplicate_ignored": summary.duplicate_ignored,
        "rejected": summary.rejected,
        "assertions_total": summary.assertions_total,
        "rows": summary.outcomes.iter().map(Outcome::value).collect::<Vec<Value>>(),
        "notes": summary.notes,
    })
}

/// Contract §4.3 checks 1–4, plus the explicit empty-set refusal after them.
fn envelope_code(verdicts: &Value, bundle: &Value, rows: Option<&Vec<Value>>) -> Option<&'static str> {
    if verdicts.get("schema").and_then(Value::as_str) != Some(VERDICTS_SCHEMA) {
        return Some("schema_version_unsupported");
    }
    if verdicts.get("bundle_id").and_then(Value::as_str)
        != bundle.get("bundle_id").and_then(Value::as_str)
    {
        return Some("bundle_id_mismatch");
    }
    if verdicts.get("bundle_digest").and_then(Value::as_str)
        != bundle.get("bundle_digest").and_then(Value::as_str)
        || verdicts.get("bundle_version").and_then(Value::as_str) != Some(BUNDLE_VERSION)
        || bundle.get("bundle_version").and_then(Value::as_str) != Some(BUNDLE_VERSION)
    {
        return Some("stale_bundle");
    }
    let judge = verdicts.get("judge");
    let kind = judge.and_then(|j| j.get("kind")).and_then(Value::as_str);
    let id = judge.and_then(|j| j.get("judge_id")).and_then(Value::as_str).unwrap_or("");
    if !matches!(kind, Some("user" | "external_llm")) || id.trim().is_empty() {
        return Some("judge_missing");
    }
    match rows {
        None => Some("verdict_rows_missing"),
        Some(rows) if rows.is_empty() => Some("empty_verdicts"),
        _ => None,
    }
}

fn load_bundle_view(dir: &Path) -> Result<BundleView> {
    let bytes = fs::read(dir.join("bundle.json")).map_err(|_| err("bundle.json not readable"))?;
    let bundle: Value = serde_json::from_slice(&bytes).map_err(|_| err("bundle.json not JSON"))?;
    let mut occurrences = HashMap::new();
    let mut evidence = HashSet::new();
    let mut span_refs = HashMap::new();
    let motif_rows = bundle.get("motifs").and_then(Value::as_array).cloned().unwrap_or_default();
    for motif_row in motif_rows {
        let rel = motif_row.get("motif_path").and_then(Value::as_str).unwrap_or("");
        let bytes = fs::read(dir.join(rel)).map_err(|_| err("motif.json not readable"))?;
        let motif: Value = serde_json::from_slice(&bytes).map_err(|_| err("motif.json not JSON"))?;
        let motif_id = motif.get("motif_id").cloned().unwrap_or(Value::Null);
        let reps = motif.get("representatives").and_then(Value::as_array).cloned().unwrap_or_default();
        for rep in reps {
            let occ_id = rep.get("occurrence_id").and_then(Value::as_str).unwrap_or("").to_string();
            let mut indexed = rep.clone();
            indexed["motif_id"] = motif_id.clone();
            indexed["questions"] = motif.get("questions").cloned().unwrap_or_else(|| json!([]));
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
    Ok(BundleView { bundle, occurrences, evidence, span_refs })
}

/// Reopen the whole bundle read-only. A bundle-level failure makes every
/// evidence row unverifiable, so no assertion can be stored from it.
fn evidence_reopen(
    dir: &Path,
    map_path: &Path,
    view: &BundleView,
) -> (Value, HashMap<String, &'static str>, HashMap<String, String>) {
    let report = verify::verify(dir, Some(map_path), None);
    let mut state: HashMap<String, &'static str> = view.evidence.iter().map(|id| (id.clone(), "unverified")).collect();
    let mut codes: HashMap<String, String> = HashMap::new();
    if report["status"].as_str() == Some("verified") {
        for id in &view.evidence {
            state.insert(id.clone(), "verified");
        }
        for watermark in report["watermarks"].as_array().cloned().unwrap_or_default() {
            if let Some(id) = watermark.get("evidence_id").and_then(Value::as_str) {
                state.insert(id.to_string(), "verified_span_after_append");
            }
        }
    } else {
        let failures = report["failures"].as_array().cloned().unwrap_or_default();
        for failure in &failures {
            let code = failure.get("code").and_then(Value::as_str).unwrap_or("evidence_unverified");
            match failure.get("evidence_id").and_then(Value::as_str) {
                Some(id) => {
                    let kind = if matches!(code, "alias_unknown" | "source_missing" | "source_unreadable") {
                        "unverified"
                    } else {
                        "failed"
                    };
                    state.insert(id.to_string(), kind);
                    codes.insert(id.to_string(), code.to_string());
                }
                None => {
                    for id in &view.evidence {
                        state.insert(id.clone(), "unverified");
                        codes.insert(id.clone(), code.to_string());
                    }
                }
            }
        }
    }
    (report, state, codes)
}

fn catalog_key_valid(key: &str) -> bool {
    let bytes = key.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes[1..].iter().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b':' | b'-')
        })
}

fn check_rows(
    view: &BundleView,
    verify_report: &Value,
    state: &HashMap<String, &'static str>,
    codes: &HashMap<String, String>,
    store: &Store,
    verdicts: &Value,
) -> Vec<Outcome> {
    let rows = verdicts.get("verdicts").and_then(Value::as_array).cloned().unwrap_or_default();
    let bundle_id = view.bundle.get("bundle_id").and_then(Value::as_str).unwrap_or("");
    let mut pending: BTreeMap<String, String> = BTreeMap::new();
    let mut outcomes = Vec::with_capacity(rows.len());
    for row in &rows {
        let assertion_id = row.get("assertion_id").and_then(Value::as_str).map(str::to_string);
        let occurrence_id = row.get("occurrence_id").and_then(Value::as_str).map(str::to_string);
        let question_id = row.get("question_id").and_then(Value::as_str).map(str::to_string);
        let reject = |code: &str, detail: &str| Outcome {
            assertion_id: assertion_id.clone(),
            occurrence_id: occurrence_id.clone(),
            question_id: question_id.clone(),
            code: code.to_string(),
            detail: detail.to_string(),
            stored: false,
        };
        let Some(occ_id) = occurrence_id.as_deref() else {
            outcomes.push(reject("occurrence_unknown", "occurrence id missing"));
            continue;
        };
        let Some(occurrence) = view.occurrences.get(occ_id) else {
            outcomes.push(reject("occurrence_unknown", "occurrence unknown in the current bundle"));
            continue;
        };
        let qid = question_id.as_deref().unwrap_or("");
        let question_ok = occurrence.get("questions").and_then(Value::as_array).is_some_and(|qs| {
            qs.iter().any(|q| q.get("question_id").and_then(Value::as_str) == Some(qid))
        });
        if !question_ok {
            outcomes.push(reject("question_unknown", "question unknown for that motif"));
            continue;
        }
        let refs: Vec<String> = row
            .get("evidence_refs")
            .and_then(Value::as_array)
            .map(|refs| {
                refs.iter()
                    .filter_map(|r| r.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if refs.is_empty() {
            outcomes.push(reject("missing_evidence", "verdict has no evidence refs"));
            continue;
        }
        let mut reference_failure: Option<Outcome> = None;
        for ev in &refs {
            if !view.evidence.contains(ev) {
                let code = if store.registry.get(ev).is_some_and(|other| other != bundle_id) {
                    "cross_bundle_reference"
                } else {
                    "missing_evidence"
                };
                reference_failure = Some(reject(code, "evidence ref is not in the current bundle"));
                break;
            }
            if !matches!(state.get(ev).copied(), Some("verified" | "verified_span_after_append")) {
                let detail = codes
                    .get(ev)
                    .map(|code| format!("evidence not accepted: {code}"))
                    .unwrap_or_else(|| "evidence not accepted by reopen".to_string());
                reference_failure = Some(reject("evidence_unverified", &detail));
                break;
            }
        }
        if let Some(failure) = reference_failure {
            outcomes.push(failure);
            continue;
        }
        if row.get("input_identity").and_then(Value::as_str)
            != occurrence.get("input_identity").and_then(Value::as_str)
        {
            outcomes.push(reject("input_identity_mismatch", "bundle content judged is stale"));
            continue;
        }
        // Mirrors the read-only cross-check in `verify`: contract §4.2.
        let expected = format!("as-{}", &value_digest(&verify::assertion_body(&view.bundle, row))[..16]);
        if assertion_id.as_deref() != Some(expected.as_str()) {
            outcomes.push(reject("assertion_id_conflict", "assertion id differs from content"));
            continue;
        }
        let verdict_digest = format!("sha256:{}", value_digest(row));
        if let Some(existing) = store.assertions.get(&expected) {
            if existing.get("verdict_digest").and_then(Value::as_str) == Some(verdict_digest.as_str()) {
                outcomes.push(Outcome {
                    code: "duplicate_ignored".to_string(),
                    detail: "identical assertion already stored".to_string(),
                    stored: false,
                    ..reject("", "")
                });
            } else {
                outcomes.push(reject("assertion_id_conflict", "assertion id already stored with another body"));
            }
            continue;
        }
        if let Some(seen) = pending.get(&expected) {
            if seen == &verdict_digest {
                outcomes.push(Outcome {
                    code: "duplicate_ignored".to_string(),
                    detail: "identical assertion already in this verdict file".to_string(),
                    stored: false,
                    ..reject("", "")
                });
            } else {
                outcomes.push(reject("assertion_id_conflict", "assertion id reused with another body"));
            }
            continue;
        }
        if let Some(kind) = row.get("catalog_kind").and_then(Value::as_str) {
            if kind != "skill" && kind != "template" {
                outcomes.push(reject("catalog_key_invalid", "catalog kind unsupported"));
                continue;
            }
        }
        if let Some(key) = row.get("catalog_key").and_then(Value::as_str) {
            if !catalog_key_valid(key) {
                outcomes.push(reject("catalog_key_invalid", "catalog key grammar violated"));
                continue;
            }
        }
        pending.insert(expected.clone(), verdict_digest);
        outcomes.push(Outcome {
            code: "accepted".to_string(),
            detail: format!("verified against bundle {}", verify_report["status"].as_str().unwrap_or("unverified")),
            stored: true,
            ..reject("", "")
        });
    }
    outcomes
}

fn assertion_record(
    row: &Value,
    verdicts: &Value,
    verdict_file_digest: &str,
    view: &BundleView,
    bundle: &Value,
) -> Value {
    let id = row.get("assertion_id").and_then(Value::as_str).unwrap_or("");
    let mut refs: Vec<String> = row
        .get("evidence_refs")
        .and_then(Value::as_array)
        .map(|refs| refs.iter().filter_map(|r| r.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    refs.sort();
    json!({
        "assertion_id": id,
        "bundle_id": bundle.get("bundle_id").cloned().unwrap_or(Value::Null),
        "bundle_digest": bundle.get("bundle_digest").cloned().unwrap_or(Value::Null),
        "bundle_version": bundle.get("bundle_version").cloned().unwrap_or(Value::Null),
        "motif_id": view.occurrences.get(row.get("occurrence_id").and_then(Value::as_str).unwrap_or(""))
            .and_then(|occ| occ.get("motif_id")).cloned().unwrap_or(Value::Null),
        "occurrence_id": row.get("occurrence_id").cloned().unwrap_or(Value::Null),
        "question_id": row.get("question_id").cloned().unwrap_or(Value::Null),
        "answer": row.get("answer").cloned().unwrap_or(Value::Null),
        "should_codify": row.get("should_codify").cloned().unwrap_or(Value::Null),
        "waste_score": row.get("waste_score").cloned().unwrap_or(Value::Null),
        "catalog_kind": row.get("catalog_kind").cloned().unwrap_or(Value::Null),
        "catalog_key": row.get("catalog_key").cloned().unwrap_or(Value::Null),
        "evidence_refs": refs,
        "input_identity": row.get("input_identity").cloned().unwrap_or(Value::Null),
        "evidence_span_refs": view.span_refs.get(row.get("occurrence_id").and_then(Value::as_str).unwrap_or(""))
            .cloned().unwrap_or_default(),
        "verdict_digest": format!("sha256:{}", value_digest(row)),
        "verdict_file_digest": verdict_file_digest,
        "judge": verdicts.get("judge").cloned().unwrap_or(Value::Null),
        "generated_at": verdicts.get("generated_at").cloned().unwrap_or(Value::Null),
    })
}

fn empty_store() -> Value {
    let value = json!({
        "schema": ASSERTIONS_SCHEMA,
        "store_version": ASSERTIONS_VERSION,
        "assertions": [],
        "bundle_registry": {},
        "sources": [],
    });
    let mut value = value;
    value["store_digest"] = json!(format!("sha256:{}", value_digest(&value)));
    value
}

fn load_store(path: &Path) -> Result<Store> {
    let value = if path.exists() {
        let bytes = fs::read(path).map_err(|_| err("assertion store not readable"))?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| err("assertion store not JSON"))?;
        if value.get("schema").and_then(Value::as_str) != Some(ASSERTIONS_SCHEMA) {
            return Err(err("assertion store schema unsupported"));
        }
        let mut body = value.clone();
        let recorded = body.get("store_digest").and_then(Value::as_str).unwrap_or("").to_string();
        body.as_object_mut().expect("store object").remove("store_digest");
        if recorded != format!("sha256:{}", value_digest(&body)) {
            return Err(err("assertion store digest mismatch"));
        }
        value
    } else {
        empty_store()
    };
    let mut assertions = BTreeMap::new();
    for row in value.get("assertions").and_then(Value::as_array).cloned().unwrap_or_default() {
        if let Some(id) = row.get("assertion_id").and_then(Value::as_str) {
            assertions.insert(id.to_string(), row);
        }
    }
    let mut registry = BTreeMap::new();
    if let Some(map) = value.get("bundle_registry").and_then(Value::as_object) {
        for (evidence_id, bundle_id) in map {
            if let Some(bundle_id) = bundle_id.as_str() {
                registry.insert(evidence_id.clone(), bundle_id.to_string());
            }
        }
    }
    let sources = value.get("sources").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(Store { value, assertions, registry, sources })
}

fn write_store(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|_| err("assertion store directory not creatable"))?;
        }
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| err(format!("assertion store: {e}")))?;
    let tmp = path.with_extension("ase-tmp");
    let backup = path.with_extension("ase-bak");
    fs::write(&tmp, &bytes).map_err(|_| err("assertion store temp write failed"))?;
    if path.exists() {
        let _ = fs::remove_file(&backup);
        fs::rename(path, &backup).map_err(|_| err("assertion store backup failed"))?;
        if let Err(_e) = fs::rename(&tmp, path) {
            let _ = fs::rename(&backup, path);
            let _ = fs::remove_file(&tmp);
            return Err(err("assertion store replace failed"));
        }
        let _ = fs::remove_file(&backup);
    } else {
        fs::rename(&tmp, path).map_err(|_| err("assertion store create failed"))?;
    }
    Ok(())
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
    fn empty_store_has_a_matching_digest() {
        let value = empty_store();
        let mut body = value.clone();
        body.as_object_mut().expect("object").remove("store_digest");
        assert_eq!(
            value["store_digest"].as_str(),
            Some(format!("sha256:{}", value_digest(&body)).as_str())
        );
    }
}
