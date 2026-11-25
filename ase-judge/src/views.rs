//! Read-only evaluation views: pinned-source verification, store reports and
//! assertion projection. These commands never construct a runner or contact a
//! model; they reopen pinned source from exact bytes and reject stale state.

use ase_core::{err, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

use crate::assertion::sha256_hex;
use crate::evaluate::VerifyOptions;
use crate::pin::{heritage_unit_id, read_json, PrivateRow};
use crate::rubric;
use crate::source::{CorpusSource, EvidenceVerifier};
use crate::store::Store;

pub const VERIFY_SCHEMA: &str = "ase-judge-verify/v1";
pub const REPORT_SCHEMA: &str = "ase-judge-store-report/v1";
pub const FIXTURE_SCHEMA: &str = "ase-judge-contract-fixture/v1";

#[derive(Serialize)]
pub struct VerifyReport {
    pub schema: &'static str,
    pub store: String,
    pub assertions: u64,
    pub verified: u64,
    pub failed: u64,
    pub missing_evidence: u64,
    pub failures: Vec<Value>,
    pub status: String,
}

pub fn verify(options: &VerifyOptions) -> Result<VerifyReport> {
    let private = read_json(&options.private, "private path map")?;
    let mut private_rows: BTreeMap<u32, PrivateRow> = BTreeMap::new();
    for row in private
        .get("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| err("private path map has no rows"))?
    {
        let ordinal = row
            .get("ordinal")
            .and_then(Value::as_u64)
            .ok_or_else(|| err("private row has no ordinal"))? as u32;
        private_rows.insert(
            ordinal,
            PrivateRow {
                rel_path: row
                    .get("rel_path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| err("private row has no rel_path"))?
                    .to_string(),
                session_id: row
                    .get("session_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| err("private row has no session_id"))?
                    .to_string(),
            },
        );
    }
    let units_text = std::fs::read_to_string(&options.units)
        .map_err(|e| err(format!("read units {}: {e}", options.units.display())))?;
    let mut unit_paths: BTreeMap<String, String> = BTreeMap::new();
    for line in units_text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let row: Value = serde_json::from_str(line)
            .map_err(|e| err(format!("parse units row: {e}")))?;
        let Some(ordinal) = row.get("ordinal").and_then(Value::as_u64) else {
            continue;
        };
        let Some(private_row) = private_rows.get(&(ordinal as u32)) else {
            continue;
        };
        unit_paths.insert(
            heritage_unit_id(&private_row.session_id),
            private_row.rel_path.clone(),
        );
    }
    let source = CorpusSource::new(&options.root);
    for (unit_id, rel_path) in &unit_paths {
        source.register(unit_id, rel_path);
    }
    let store = Store::open_read_only(&options.store)?;
    let rows = store.assertions(
        if options.include_verification {
            None
        } else {
            Some(false)
        },
        None,
        None,
    )?;
    let mut failures = Vec::new();
    let mut verified = 0u64;
    for stored in &rows {
        let assertion = &stored.assertion;
        let check = (|| -> Result<()> {
            assertion.validate_shape()?;
            source.verify_source(assertion)?;
            let recomputed_input = assertion.input_sha256();
            if recomputed_input != stored.input_sha256 {
                return Err(err("stored input digest does not recompute from source"));
            }
            let recomputed_key = assertion.cache_key();
            if recomputed_key != stored.cache_key {
                return Err(err("stored cache key does not recompute"));
            }
            Ok(())
        })();
        match check {
            Ok(()) => verified += 1,
            Err(e) => {
                if failures.len() < 50 {
                    failures.push(json!({
                        "unit_id": assertion.unit_id,
                        "decision_index": assertion.decision_index,
                        "generator": assertion.generator.as_str(),
                        "assertion_id": stored.assertion_id,
                        "reason": e.0,
                    }));
                }
            }
        }
    }
    let failed = rows.len() as u64 - verified;
    let missing_evidence = store.count_without_evidence()?;
    let status = if failed == 0 && missing_evidence == 0 {
        "verified"
    } else {
        "stale"
    };
    Ok(VerifyReport {
        schema: VERIFY_SCHEMA,
        store: options
            .store
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default(),
        assertions: rows.len() as u64,
        verified,
        failed,
        missing_evidence,
        failures,
        status: status.to_string(),
    })
}

pub fn report(store_path: &Path, include_verification: bool) -> Result<Value> {
    let store = Store::open_read_only(store_path)?;
    let filter = if include_verification {
        None
    } else {
        Some(false)
    };
    Ok(json!({
        "schema": REPORT_SCHEMA,
        "store": store_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default(),
        "assertions": store.count()?,
        "assertion_rows_in_scope": store.assertions(filter, None, None)?.len(),
        "missing_evidence": store.count_without_evidence()?,
        "assertions_sha256": {
            "evaluation": store.digest(false)?,
            "verification_only": store.digest(true)?,
        },
        "assertion_stats": store.assertion_stats()?,
        "call_stats": store.call_stats()?,
        "runs": store.runs()?,
    }))
}

pub fn show(
    store_path: &Path,
    unit_id: Option<&str>,
    decision_index: Option<u32>,
    include_verification: bool,
) -> Result<Value> {
    let store = Store::open_read_only(store_path)?;
    let filter = if include_verification {
        None
    } else {
        Some(false)
    };
    let rows = store.assertions(filter, unit_id, decision_index)?;
    let assertions: Vec<Value> = rows
        .iter()
        .map(|stored| {
            let assertion = &stored.assertion;
            json!({
                "unit_id": assertion.unit_id,
                "decision_index": assertion.decision_index,
                "generator": assertion.generator.as_str(),
                "generator_id": assertion.generator_id,
                "rule_version": assertion.rule_version,
                "label": assertion.label.as_str(),
                "confidence": assertion.confidence,
                "confidence_semantics": assertion
                    .confidence
                    .as_deref()
                    .map(|text| {
                        let (semantics, value) = rubric::decode_confidence(text);
                        json!({"semantics": semantics, "value": value})
                    }),
                "abstain_reason": assertion.abstain_reason,
                "context": assertion.context,
                "evidence": assertion.evidence,
                "input_sha256": stored.input_sha256,
                "cache_key": stored.cache_key,
                "assertion_id": stored.assertion_id,
                "verification_only": assertion.verification_only,
                "run_id": stored.run_id,
                "created_at": stored.created_at,
            })
        })
        .collect();
    Ok(json!({
        "schema": "ase-judge-assertions/v1",
        "assertions": assertions,
        "count": assertions.len(),
        "model_calls": 0,
    }))
}

/// Writes a tiny synthetic OMP journal plus its pin so the local-model
/// contract and cache mechanics can be verified without real prose and
/// without standing in for an evaluation.
pub fn write_fixture(out_dir: &Path) -> Result<Value> {
    let corpus = out_dir.join("corpus");
    let project = corpus.join("proj");
    std::fs::create_dir_all(&project)
        .map_err(|e| err(format!("create fixture dir: {e}")))?;
    let session_id = "11111111-1111-4111-8111-111111111111";
    let lines = vec![
        json!({
            "type": "session",
            "id": session_id,
            "timestamp": "2026-10-01T00:00:00.000Z",
        }),
        json!({
            "type": "message",
            "id": "aaaabbbb",
            "parentId": session_id,
            "timestamp": "2026-10-01T00:00:01.000Z",
            "message": {
                "role": "user",
                "attribution": "user",
                "content": [{"type": "text", "text": "synthetic earlier instruction"}],
            },
        }),
        json!({
            "type": "message",
            "id": "ccccdddd",
            "parentId": "aaaabbbb",
            "timestamp": "2026-10-01T00:00:02.000Z",
            "message": {
                "role": "user",
                "attribution": "user",
                "content": [{"type": "text", "text": "synthetic later follow-up"}],
            },
        }),
        json!({
            "type": "message",
            "id": "eeeeffff",
            "parentId": "ccccdddd",
            "timestamp": "2026-10-01T00:00:03.000Z",
            "message": {
                "role": "user",
                "attribution": "user",
                "content": [{"type": "text", "text": "synthetic final refinement"}],
            },
        }),
    ];
    let mut journal = String::new();
    for line in &lines {
        journal.push_str(&serde_json::to_string(line).map_err(|e| err(format!("fixture json: {e}")))?);
        journal.push('\n');
    }
    let rel_path = "proj/contract-fixture.jsonl";
    let journal_path = corpus.join("proj/contract-fixture.jsonl");
    std::fs::write(&journal_path, journal.as_bytes())
        .map_err(|e| err(format!("write fixture journal: {e}")))?;
    let journal = crate::source::Journal::load(&corpus, rel_path)?;
    let units = json!({
        "ordinal": 0,
        "unit_id": "0000000000000000",
        "primary_candidate": true,
        "unknown_stratum": false,
        "human_user_msgs_active": journal.human.len(),
        "user_msgs_active": journal.all_role_user,
        "leaf_entry_id": journal.leaf_entry_id,
        "kind": "root",
        "naming_status": "unparsed",
    });
    let units_path = out_dir.join("units.jsonl");
    std::fs::write(
        &units_path,
        format!("{}\n", serde_json::to_string(&units).map_err(|e| err(format!("fixture units: {e}")))?),
    )
    .map_err(|e| err(format!("write fixture units: {e}")))?;
    let private = json!({
        "schema": "ase-judge-private-manifest/v1",
        "rows": [{"ordinal": 0, "rel_path": rel_path, "session_id": journal.session_id}],
    });
    let private_path = out_dir.join("private.json");
    std::fs::write(
        &private_path,
        serde_json::to_string_pretty(&private)
            .map_err(|e| err(format!("encode fixture private: {e}")))?
            + "\n",
    )
    .map_err(|e| err(format!("write fixture private: {e}")))?;
    let manifest_body = json!({
        "schema": "ase-judge-contract-fixture-manifest/v1",
        "rows": [{"ordinal": 0, "size": journal.size, "sha256": journal.sha256}],
    });
    let manifest = json!({
        "schema": "ase-judge-contract-fixture-manifest/v1",
        "rows": [{"ordinal": 0, "size": journal.size, "sha256": journal.sha256}],
        "manifest_sha256": sha256_hex(
            &serde_json::to_vec(&manifest_body).map_err(|e| err(format!("fixture manifest: {e}")))?,
        ),
    });
    let manifest_path = out_dir.join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest)
            .map_err(|e| err(format!("encode fixture manifest: {e}")))?
            + "\n",
    )
    .map_err(|e| err(format!("write fixture manifest: {e}")))?;
    Ok(json!({
        "schema": FIXTURE_SCHEMA,
        "corpus": "corpus",
        "units": "units.jsonl",
        "private": "private.json",
        "manifest": "manifest.json",
        "session_id": journal.session_id,
        "journal_sha256": journal.sha256,
        "human_messages": journal.human.len(),
        "decisions": journal.decision_count(),
        "manifest_sha256": manifest["manifest_sha256"],
        "synthetic": true,
    }))
}
