//! Pinned cohort source: manifest, unit rows and private path map.
//!
//! A run reopens each selected journal and checks it against the pin before
//! any decision is derived. Verification reopens the same pinned source to
//! reject stored assertions whose evidence or digests no longer recompute.

use ase_core::{err, Result};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::assertion::sha256_hex;
use crate::evaluate::Scope;
use crate::source::Journal;

pub(crate) struct UnitPin {
    pub(crate) ordinal: u32,
    pub(crate) unit_id: String,
    pub(crate) primary: bool,
    pub(crate) unknown: bool,
    pub(crate) human: u32,
    pub(crate) all_role: u32,
    pub(crate) leaf: Option<String>,
}

pub(crate) struct PrivateRow {
    pub(crate) rel_path: String,
    pub(crate) session_id: String,
}

pub(crate) struct ManifestRow {
    pub(crate) size: u64,
    pub(crate) sha256: String,
}

pub(crate) struct Pin {
    pub(crate) manifest_sha256: String,
    pub(crate) units_sha256: String,
    pub(crate) private_sha256: String,
    pub(crate) manifest: BTreeMap<u32, ManifestRow>,
    pub(crate) units: Vec<UnitPin>,
    pub(crate) private: BTreeMap<u32, PrivateRow>,
}

pub(crate) fn read_json(path: &Path, what: &str) -> Result<Value> {
    let bytes = std::fs::read(path)
        .map_err(|e| err(format!("read {what} {}: {e}", path.display())))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| err(format!("parse {what} {}: {e}", path.display())))
}

fn read_file_sha256(path: &Path, what: &str) -> Result<String> {
    let bytes = std::fs::read(path)
        .map_err(|e| err(format!("read {what} {}: {e}", path.display())))?;
    Ok(sha256_hex(&bytes))
}

pub(crate) fn heritage_unit_id(session_id: &str) -> String {
    sha256_hex(format!("ase-judge-v1-unit\u{0}{session_id}").as_bytes())
}

impl Pin {
    pub(crate) fn load(manifest_path: &Path, units_path: &Path, private_path: &Path) -> Result<Self> {
        let manifest = read_json(manifest_path, "cohort manifest")?;
        let manifest_sha256 = manifest
            .get("manifest_sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| err("cohort manifest has no manifest_sha256"))?
            .to_string();
        let mut rows = BTreeMap::new();
        let manifest_rows = manifest
            .get("rows")
            .and_then(Value::as_array)
            .ok_or_else(|| err("cohort manifest has no rows"))?;
        for row in manifest_rows {
            let ordinal = row
                .get("ordinal")
                .and_then(Value::as_u64)
                .ok_or_else(|| err("cohort manifest row has no ordinal"))? as u32;
            let size = row
                .get("size")
                .and_then(Value::as_u64)
                .ok_or_else(|| err("cohort manifest row has no size"))?;
            let sha256 = row
                .get("sha256")
                .and_then(Value::as_str)
                .ok_or_else(|| err("cohort manifest row has no sha256"))?
                .to_string();
            rows.insert(ordinal, ManifestRow { size, sha256 });
        }
        let private = read_json(private_path, "private path map")?;
        let mut private_rows = BTreeMap::new();
        let private_array = private
            .get("rows")
            .and_then(Value::as_array)
            .ok_or_else(|| err("private path map has no rows"))?;
        for row in private_array {
            let ordinal = row
                .get("ordinal")
                .and_then(Value::as_u64)
                .ok_or_else(|| err("private row has no ordinal"))? as u32;
            let rel_path = row
                .get("rel_path")
                .and_then(Value::as_str)
                .ok_or_else(|| err("private row has no rel_path"))?
                .to_string();
            let session_id = row
                .get("session_id")
                .and_then(Value::as_str)
                .ok_or_else(|| err("private row has no session_id"))?
                .to_string();
            private_rows.insert(ordinal, PrivateRow { rel_path, session_id });
        }
        let units_text = std::fs::read_to_string(units_path)
            .map_err(|e| err(format!("read units {}: {e}", units_path.display())))?;
        let mut units = Vec::new();
        for line in units_text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let row: Value = serde_json::from_str(line)
                .map_err(|e| err(format!("parse units row: {e}")))?;
            let ordinal = row
                .get("ordinal")
                .and_then(Value::as_u64)
                .ok_or_else(|| err("units row has no ordinal"))? as u32;
            let private_row = private_rows.get(&ordinal).ok_or_else(|| {
                err(format!("unit ordinal {ordinal} has no private path row"))
            })?;
            units.push(UnitPin {
                ordinal,
                unit_id: heritage_unit_id(&private_row.session_id),
                primary: row
                    .get("primary_candidate")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                unknown: row
                    .get("unknown_stratum")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                human: row
                    .get("human_user_msgs_active")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32,
                all_role: row
                    .get("user_msgs_active")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32,
                leaf: row
                    .get("leaf_entry_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
        units.sort_by_key(|unit| unit.ordinal);
        Ok(Pin {
            manifest_sha256,
            units_sha256: read_file_sha256(units_path, "units")?,
            private_sha256: read_file_sha256(private_path, "private path map")?,
            manifest: rows,
            units,
            private: private_rows,
        })
    }

    pub(crate) fn select(
        &self,
        scope: Scope,
        only: &BTreeSet<u32>,
        limit: Option<usize>,
    ) -> Vec<(u32, String, String)> {
        let mut selected = Vec::new();
        for unit in &self.units {
            if !only.is_empty() && !only.contains(&unit.ordinal) {
                continue;
            }
            let in_scope = match scope {
                Scope::Primary => unit.primary,
                Scope::Unknown => unit.unknown,
                Scope::All => unit.primary || unit.unknown,
            };
            if !in_scope {
                continue;
            }
            let Some(private) = self.private.get(&unit.ordinal) else {
                continue;
            };
            selected.push((
                unit.ordinal,
                unit.unit_id.clone(),
                private.rel_path.clone(),
            ));
            if let Some(limit) = limit {
                if selected.len() >= limit {
                    break;
                }
            }
        }
        selected
    }

    /// Reopens a journal against the manifest, private path map and unit row
    /// that pin it. Any mismatch is a hard error before decisions are derived.
    pub(crate) fn verify_unit(&self, ordinal: u32, journal: &Journal) -> Result<()> {
        let manifest_row = self
            .manifest
            .get(&ordinal)
            .ok_or_else(|| err(format!("manifest has no row for ordinal {ordinal}")))?;
        if journal.size != manifest_row.size || journal.sha256 != manifest_row.sha256 {
            return Err(err(format!(
                "journal for ordinal {ordinal} does not match the pinned size/SHA-256"
            )));
        }
        let private_row = self
            .private
            .get(&ordinal)
            .ok_or_else(|| err(format!("private map has no row for ordinal {ordinal}")))?;
        if journal.session_id != private_row.session_id {
            return Err(err(format!(
                "journal for ordinal {ordinal} does not match the pinned session id"
            )));
        }
        let unit = self
            .units
            .iter()
            .find(|unit| unit.ordinal == ordinal)
            .ok_or_else(|| err(format!("units file has no row for ordinal {ordinal}")))?;
        if journal.human.len() as u32 != unit.human {
            return Err(err(format!(
                "ordinal {ordinal}: projected human messages {} != pinned {}",
                journal.human.len(),
                unit.human
            )));
        }
        if journal.all_role_user as u32 != unit.all_role {
            return Err(err(format!(
                "ordinal {ordinal}: projected user messages {} != pinned {}",
                journal.all_role_user, unit.all_role
            )));
        }
        if (journal.all_role_user as u32) > journal.projection_user_all_branches {
            return Err(err(format!(
                "ordinal {ordinal}: decision projection has {} active user messages but the product projection counts only {} across all branches",
                journal.all_role_user, journal.projection_user_all_branches
            )));
        }
        if let Some(leaf) = &unit.leaf {
            if journal.leaf_entry_id.as_deref() != Some(leaf.as_str()) {
                return Err(err(format!(
                    "ordinal {ordinal}: active leaf {:?} != pinned leaf {leaf}",
                    journal.leaf_entry_id
                )));
            }
        }
        Ok(())
    }
}
