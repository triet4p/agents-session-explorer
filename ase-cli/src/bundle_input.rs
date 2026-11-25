//! Inputs for bundle verification: adjacent JSON, the private local map and
//! mapped source bytes.
//!
//! The bundle is read as data. A private `ase-local-sources-v1` map decides
//! which local file backs an alias; the map records the file id and frozen
//! digest, and a mismatch is an unreadable alias rather than a tampered bundle.

use crate::bundle_digest::sha_body;
use crate::bundle_report::{fail, Check};
use crate::bundle_schema::LOCAL_MAP_SCHEMA;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn read_json(path: &Path) -> Check<Value> {
    let bytes = fs::read(path).map_err(|_| fail("source_unreadable", "file not readable"))?;
    serde_json::from_slice(&bytes).map_err(|_| fail("source_unreadable", "file not JSON"))
}

pub(crate) fn load_local_map(path: Option<&Path>) -> Check<(Option<Value>, PathBuf)> {
    let Some(path) = path else {
        return Ok((None, PathBuf::new()));
    };
    let value = read_json(path).map_err(|_| fail("alias_unknown", "local source map not readable"))?;
    if value.get("schema").and_then(Value::as_str) != Some(LOCAL_MAP_SCHEMA) {
        return Err(fail("alias_unknown", "local source map schema unsupported"));
    }
    let base = path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
    Ok((Some(value), base))
}

pub(crate) fn alias_source<'a>(
    map: Option<&'a Value>,
    base: &Path,
    alias: &str,
    file_id: u64,
    file_sha: &str,
) -> Check<PathBuf> {
    let map = map.ok_or_else(|| fail("alias_unknown", "no local alias map supplied"))?;
    let aliases = map
        .get("aliases")
        .and_then(Value::as_object)
        .ok_or_else(|| fail("alias_unknown", "alias map missing"))?;
    let entry = aliases
        .get(alias)
        .ok_or_else(|| fail("alias_unknown", "alias not in local map"))?;
    if entry.get("file_id").and_then(Value::as_u64) != Some(file_id) {
        return Err(fail("alias_unknown", "alias file id differs"));
    }
    let entry_sha = entry.get("sha256").and_then(Value::as_str).unwrap_or("");
    if sha_body(entry_sha).unwrap_or(entry_sha) != file_sha {
        return Err(fail("alias_unknown", "alias file digest differs"));
    }
    let rel = entry
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| fail("alias_unknown", "alias has no local path"))?;
    let rel = Path::new(rel);
    Ok(if rel.is_absolute() {
        rel.to_path_buf()
    } else {
        base.join(rel)
    })
}

pub(crate) fn snapshot_files(bundle: &Value) -> HashMap<u64, (String, u64)> {
    let mut out = HashMap::new();
    let Some(files) = bundle
        .get("snapshot")
        .and_then(|s| s.get("files"))
        .and_then(Value::as_array)
    else {
        return out;
    };
    for file in files {
        let Some(id) = file.get("file_id").and_then(Value::as_u64) else {
            continue;
        };
        let raw = file.get("sha256").and_then(Value::as_str).unwrap_or_default();
        let sha = sha_body(raw).unwrap_or(raw).to_string();
        let size = file.get("size").and_then(Value::as_u64).unwrap_or(0);
        out.insert(id, (sha, size));
    }
    out
}

pub(crate) fn check_snapshot(bundle: &Value, map: Option<&Value>) -> Check<()> {
    let Some(map) = map else {
        return Ok(());
    };
    let Some(local) = map.get("snapshot") else {
        return Err(fail("snapshot_mismatch", "local snapshot pin absent"));
    };
    let snap = bundle.get("snapshot").unwrap_or(&Value::Null);
    let bundle_id = snap.get("snapshot_id").and_then(Value::as_str);
    let local_id = local.get("snapshot_id").and_then(Value::as_str);
    if bundle_id.is_none() || bundle_id != local_id {
        return Err(fail("snapshot_mismatch", "snapshot id differs from local pin"));
    }
    let bundle_core = snap.get("core_digest").and_then(Value::as_str);
    let local_core = local.get("core_digest").and_then(Value::as_str);
    if bundle_core.is_none() || bundle_core != local_core {
        return Err(fail("snapshot_mismatch", "snapshot core digest differs from local pin"));
    }
    Ok(())
}

pub(crate) fn read_source(path: &Path) -> Check<Vec<u8>> {
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(fail("source_missing", "mapped source file absent"))
        }
        Err(_) => Err(fail("source_unreadable", "mapped source file not readable")),
    }
}
