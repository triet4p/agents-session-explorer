//! Canonical JSON bytes and SHA-256 identities for bundle artifacts.
//!
//! Bundle ids, evidence ids and occurrence input identities are hashes of
//! canonical JSON, so every writer and reader must serialize a value the same
//! way before hashing it.

use ase_index::sha256_hex;
use serde_json::Value;

pub(crate) fn canonical(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}

pub(crate) fn value_digest(value: &Value) -> String {
    sha256_hex(&canonical(value))
}

fn hex64(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

pub(crate) fn sha_body(value: &str) -> Option<&str> {
    let body = value.strip_prefix("sha256:")?;
    if hex64(body) {
        Some(body)
    } else {
        None
    }
}
