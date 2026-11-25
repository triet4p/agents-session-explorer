//! SQLite assertion store and L2b cache.
//!
//! Assertions are inserted only after structural validation and a source
//! re-open of both evidence spans. The store keeps addresses, version ids and
//! judgements; it never keeps message prose. Cache lookups are reads: no model
//! is reachable from this module.

use ase_core::{err, Result};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::assertion::{sha256_hex, Assertion, Decision, Evidence, Generator};
use crate::source::EvidenceVerifier;

const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS assertions (
  cache_key TEXT PRIMARY KEY,
  assertion_id TEXT NOT NULL,
  unit_id TEXT NOT NULL,
  decision_index INTEGER NOT NULL,
  generator TEXT NOT NULL,
  rule_version TEXT NOT NULL,
  generator_id TEXT NOT NULL,
  input_sha256 TEXT NOT NULL,
  label TEXT NOT NULL,
  confidence TEXT,
  abstain_reason TEXT,
  evidence_json TEXT NOT NULL,
  context_json TEXT,
  render_sha256 TEXT NOT NULL,
  verification_only INTEGER NOT NULL,
  run_id TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS assertions_unit ON assertions(unit_id, decision_index, generator);
CREATE INDEX IF NOT EXISTS assertions_generator ON assertions(generator, verification_only);
CREATE TABLE IF NOT EXISTS runs (
  run_id TEXT PRIMARY KEY,
  config_sha256 TEXT NOT NULL,
  started_at TEXT NOT NULL,
  finished_at TEXT NOT NULL,
  scope TEXT NOT NULL,
  units INTEGER NOT NULL,
  decisions INTEGER NOT NULL,
  model_calls INTEGER NOT NULL,
  verification_only INTEGER NOT NULL,
  report_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS calls (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id TEXT NOT NULL,
  stage TEXT NOT NULL,
  generator_id TEXT,
  input_sha256 TEXT NOT NULL,
  outcome TEXT NOT NULL,
  detail TEXT,
  verification_only INTEGER NOT NULL,
  at TEXT NOT NULL
);
";

const ASSERTION_COLUMNS: &str = "cache_key, assertion_id, unit_id, decision_index, generator, rule_version, generator_id, input_sha256, label, confidence, abstain_reason, evidence_json, context_json, render_sha256, verification_only, run_id, created_at";

#[derive(Clone, Debug)]
pub struct StoredAssertion {
    pub cache_key: String,
    pub assertion_id: String,
    pub input_sha256: String,
    pub assertion: Assertion,
    pub run_id: String,
    pub created_at: String,
}

pub enum InsertOutcome {
    Inserted,
    Hit(StoredAssertion),
}

pub struct CallRecord<'a> {
    pub run_id: &'a str,
    pub stage: &'a str,
    pub generator_id: Option<&'a str>,
    pub input_sha256: &'a str,
    pub outcome: &'a str,
    pub detail: Option<&'a str>,
    pub verification_only: bool,
    pub at: &'a str,
}

pub struct RunRecord<'a> {
    pub run_id: &'a str,
    pub config_sha256: &'a str,
    pub started_at: &'a str,
    pub finished_at: &'a str,
    pub scope: &'a str,
    pub units: u64,
    pub decisions: u64,
    pub model_calls: u64,
    pub verification_only: bool,
    pub report_json: &'a str,
}

pub struct Store {
    conn: Connection,
    path: PathBuf,
}

fn sqlite_error(context: &str, error: rusqlite::Error) -> ase_core::Error {
    err(format!("{context}: {error}"))
}

impl Store {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| err(format!("create store dir {}: {e}", parent.display())))?;
            }
        }
        let conn = Connection::open(&path)
            .map_err(|e| err(format!("open assertion store {}: {e}", path.display())))?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| err(format!("initialize assertion store: {e}")))?;
        Ok(Self { conn, path })
    }

    pub fn open_read_only(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if !path.exists() {
            return Err(err(format!(
                "assertion store does not exist: {}",
                path.display()
            )));
        }
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| err(format!("open assertion store {}: {e}", path.display())))?;
        Ok(Self { conn, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn row_to_stored(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredAssertion> {
        let generator_text: String = row.get(4)?;
        let label_text: String = row.get(8)?;
        let evidence_text: String = row.get(11)?;
        let context_text: Option<String> = row.get(12)?;
        let verification_only: i64 = row.get(14)?;
        let convert = |message: String| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, message)),
            )
        };
        let generator = Generator::parse(&generator_text)
            .ok_or_else(|| convert(format!("unknown generator {generator_text}")))?;
        let label = Decision::parse(&label_text)
            .ok_or_else(|| convert(format!("unknown label {label_text}")))?;
        let evidence: Evidence = serde_json::from_str(&evidence_text)
            .map_err(|e| convert(format!("bad evidence json: {e}")))?;
        let context: Option<Value> = context_text
            .map(|text| serde_json::from_str(&text))
            .transpose()
            .map_err(|e| convert(format!("bad context json: {e}")))?;
        let assertion = Assertion {
            unit_id: row.get(2)?,
            decision_index: row.get(3)?,
            generator,
            rule_version: row.get(5)?,
            generator_id: row.get(6)?,
            evidence,
            render_sha256: row.get(13)?,
            label,
            confidence: row.get(9)?,
            abstain_reason: row.get(10)?,
            context,
            verification_only: verification_only != 0,
        };
        Ok(StoredAssertion {
            cache_key: row.get(0)?,
            assertion_id: row.get(1)?,
            input_sha256: row.get(7)?,
            assertion,
            run_id: row.get(15)?,
            created_at: row.get(16)?,
        })
    }

    pub fn find(&self, cache_key: &str) -> Result<Option<StoredAssertion>> {
        let sql = format!("SELECT {ASSERTION_COLUMNS} FROM assertions WHERE cache_key = ?1");
        self.conn
            .query_row(&sql, params![cache_key], Self::row_to_stored)
            .optional()
            .map_err(|e| sqlite_error("read assertion", e))
    }

    /// Validates and inserts one assertion. A missing/malformed evidence pair
    /// or a source mismatch fails here, before any row exists.
    pub fn insert(
        &mut self,
        assertion: &Assertion,
        verifier: &dyn EvidenceVerifier,
        run_id: &str,
        at: &str,
    ) -> Result<InsertOutcome> {
        assertion.validate_shape()?;
        verifier.verify_source(assertion)?;
        let cache_key = assertion.cache_key();
        let input_sha256 = assertion.input_sha256();
        let assertion_id = assertion.assertion_id();
        if let Some(existing) = self.find(&cache_key)? {
            if existing.assertion_id != assertion_id {
                return Err(err(format!(
                    "cache conflict for key {cache_key}: stored assertion differs from the new judgement"
                )));
            }
            if existing.input_sha256 != input_sha256 {
                return Err(err(format!(
                    "cache conflict for key {cache_key}: stored input digest differs"
                )));
            }
            return Ok(InsertOutcome::Hit(existing));
        }
        let evidence_json = serde_json::to_string(&assertion.evidence)
            .map_err(|e| err(format!("encode evidence: {e}")))?;
        let context_json = assertion
            .context
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| err(format!("encode context: {e}")))?;
        self.conn
            .execute(
                "INSERT INTO assertions (cache_key, assertion_id, unit_id, decision_index, generator, rule_version, generator_id, input_sha256, label, confidence, abstain_reason, evidence_json, context_json, render_sha256, verification_only, run_id, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
                params![
                    cache_key,
                    assertion_id,
                    assertion.unit_id,
                    assertion.decision_index,
                    assertion.generator.as_str(),
                    assertion.rule_version,
                    assertion.generator_id,
                    input_sha256,
                    assertion.label.as_str(),
                    assertion.confidence,
                    assertion.abstain_reason,
                    evidence_json,
                    context_json,
                    assertion.render_sha256,
                    if assertion.verification_only { 1i64 } else { 0i64 },
                    run_id,
                    at,
                ],
            )
            .map_err(|e| sqlite_error("insert assertion", e))?;
        Ok(InsertOutcome::Inserted)
    }

    pub fn assertions(
        &self,
        verification_only: Option<bool>,
        unit_id: Option<&str>,
        decision_index: Option<u32>,
    ) -> Result<Vec<StoredAssertion>> {
        let mut sql = format!("SELECT {ASSERTION_COLUMNS} FROM assertions WHERE 1=1");
        let mut values: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(verification) = verification_only {
            sql.push_str(" AND verification_only = ?");
            values.push(rusqlite::types::Value::Integer(if verification {
                1
            } else {
                0
            }));
        }
        if let Some(unit_id) = unit_id {
            sql.push_str(" AND unit_id = ?");
            values.push(rusqlite::types::Value::Text(unit_id.to_string()));
        }
        if let Some(decision_index) = decision_index {
            sql.push_str(" AND decision_index = ?");
            values.push(rusqlite::types::Value::Integer(i64::from(decision_index)));
        }
        sql.push_str(" ORDER BY unit_id, decision_index, generator");
        let mut statement = self
            .conn
            .prepare(&sql)
            .map_err(|e| sqlite_error("prepare assertion query", e))?;
        let rows = statement
            .query_map(rusqlite::params_from_iter(values), Self::row_to_stored)
            .map_err(|e| sqlite_error("query assertions", e))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| sqlite_error("decode assertion", e))?);
        }
        Ok(out)
    }

    /// Deterministic digest over the stored assertion ids in one namespace.
    pub fn digest(&self, verification_only: bool) -> Result<String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT assertion_id FROM assertions WHERE verification_only = ?1 ORDER BY cache_key",
            )
            .map_err(|e| sqlite_error("prepare digest", e))?;
        let rows = statement
            .query_map(params![if verification_only { 1i64 } else { 0i64 }], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| sqlite_error("query digest", e))?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.map_err(|e| sqlite_error("decode digest", e))?);
        }
        Ok(sha256_hex(ids.join("\n").as_bytes()))
    }

    pub fn count(&self) -> Result<u64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM assertions", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|value| value as u64)
            .map_err(|e| sqlite_error("count assertions", e))
    }

    pub fn count_namespace(&self, verification_only: bool) -> Result<u64> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM assertions WHERE verification_only = ?1",
                params![if verification_only { 1i64 } else { 0i64 }],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value as u64)
            .map_err(|e| sqlite_error("count assertions in namespace", e))
    }

    /// Rows without a non-empty evidence object. Must always be zero: the
    /// insert path refuses them.
    pub fn count_without_evidence(&self) -> Result<u64> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM assertions WHERE evidence_json IS NULL OR evidence_json = '' OR evidence_json = 'null'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value as u64)
            .map_err(|e| sqlite_error("count evidence-less assertions", e))
    }

    pub fn record_call(&self, call: &CallRecord<'_>) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO calls (run_id, stage, generator_id, input_sha256, outcome, detail, verification_only, at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    call.run_id,
                    call.stage,
                    call.generator_id,
                    call.input_sha256,
                    call.outcome,
                    call.detail,
                    if call.verification_only { 1i64 } else { 0i64 },
                    call.at,
                ],
            )
            .map_err(|e| sqlite_error("record model call", e))?;
        Ok(())
    }

    pub fn record_run(&self, run: &RunRecord<'_>) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO runs (run_id, config_sha256, started_at, finished_at, scope, units, decisions, model_calls, verification_only, report_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    run.run_id,
                    run.config_sha256,
                    run.started_at,
                    run.finished_at,
                    run.scope,
                    run.units as i64,
                    run.decisions as i64,
                    run.model_calls as i64,
                    if run.verification_only { 1i64 } else { 0i64 },
                    run.report_json,
                ],
            )
            .map_err(|e| sqlite_error("record run", e))?;
        Ok(())
    }

    pub fn runs(&self) -> Result<Vec<Value>> {
        let mut statement = self
            .conn
            .prepare("SELECT run_id, config_sha256, started_at, finished_at, scope, units, decisions, model_calls, verification_only, report_json FROM runs ORDER BY started_at, run_id")
            .map_err(|e| sqlite_error("prepare runs", e))?;
        let rows = statement
            .query_map([], |row| {
                let report: String = row.get(9)?;
                Ok(serde_json::json!({
                    "run_id": row.get::<_, String>(0)?,
                    "config_sha256": row.get::<_, String>(1)?,
                    "started_at": row.get::<_, String>(2)?,
                    "finished_at": row.get::<_, String>(3)?,
                    "scope": row.get::<_, String>(4)?,
                    "units": row.get::<_, i64>(5)?,
                    "decisions": row.get::<_, i64>(6)?,
                    "model_calls": row.get::<_, i64>(7)?,
                    "verification_only": row.get::<_, i64>(8)? != 0,
                    "report": serde_json::from_str::<Value>(&report).unwrap_or(Value::Null),
                }))
            })
            .map_err(|e| sqlite_error("query runs", e))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| sqlite_error("decode run", e))?);
        }
        Ok(out)
    }

    pub fn assertion_stats(&self) -> Result<Value> {
        let mut statement = self
            .conn
            .prepare("SELECT generator, label, verification_only, COUNT(*) FROM assertions GROUP BY generator, label, verification_only ORDER BY generator, label, verification_only")
            .map_err(|e| sqlite_error("prepare assertion stats", e))?;
        let rows = statement
            .query_map([], |row| {
                Ok(serde_json::json!({
                    "generator": row.get::<_, String>(0)?,
                    "label": row.get::<_, String>(1)?,
                    "verification_only": row.get::<_, i64>(2)? != 0,
                    "count": row.get::<_, i64>(3)?,
                }))
            })
            .map_err(|e| sqlite_error("query assertion stats", e))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| sqlite_error("decode assertion stats", e))?);
        }
        Ok(Value::Array(out))
    }

    pub fn call_stats(&self) -> Result<Value> {
        let mut statement = self
            .conn
            .prepare("SELECT stage, outcome, verification_only, COUNT(*) FROM calls GROUP BY stage, outcome, verification_only ORDER BY stage, outcome, verification_only")
            .map_err(|e| sqlite_error("prepare call stats", e))?;
        let rows = statement
            .query_map([], |row| {
                Ok(serde_json::json!({
                    "stage": row.get::<_, String>(0)?,
                    "outcome": row.get::<_, String>(1)?,
                    "verification_only": row.get::<_, i64>(2)? != 0,
                    "count": row.get::<_, i64>(3)?,
                }))
            })
            .map_err(|e| sqlite_error("query call stats", e))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| sqlite_error("decode call stats", e))?);
        }
        Ok(Value::Array(out))
    }

    pub fn calls(&self, limit: u32) -> Result<Vec<Value>> {
        let mut statement = self
            .conn
            .prepare("SELECT run_id, stage, generator_id, input_sha256, outcome, detail, verification_only, at FROM calls ORDER BY id LIMIT ?1")
            .map_err(|e| sqlite_error("prepare calls", e))?;
        let rows = statement
            .query_map(params![i64::from(limit)], |row| {
                Ok(serde_json::json!({
                    "run_id": row.get::<_, String>(0)?,
                    "stage": row.get::<_, String>(1)?,
                    "generator_id": row.get::<_, Option<String>>(2)?,
                    "input_sha256": row.get::<_, String>(3)?,
                    "outcome": row.get::<_, String>(4)?,
                    "detail": row.get::<_, Option<String>>(5)?,
                    "verification_only": row.get::<_, i64>(6)? != 0,
                    "at": row.get::<_, String>(7)?,
                }))
            })
            .map_err(|e| sqlite_error("query calls", e))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| sqlite_error("decode call", e))?);
        }
        Ok(out)
    }
}
