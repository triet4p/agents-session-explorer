//! Prepublication leak guard for exported bundle files.
//!
//! The guard scans every written file for private tracking identities and
//! every published excerpt for an unredacted path, URL, token or secret. It
//! runs before the staging bundle is reopened, so a leak refuses the whole
//! export instead of publishing a partially clean tree.

use ase_core::{err, Result};
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeSet;

fn leak_patterns() -> Vec<Regex> {
    let raw = [
        r#"[A-Za-z][A-Za-z0-9+.-]*://[^\s"'<>\x60]+"#,
        r#"[A-Za-z]:[\\/][^\s"'<>\x60|]+"#,
        r#"\\\\[^\s"'<>\x60|]+"#,
        r#"/(?:home|Users|root|tmp|var|etc|opt|mnt|workspace|data)/[^\s"'<>\x60|]+"#,
        r#"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"#,
        r#"(?:sk-|ghp_|github_pat_|AKIA|ASIA|xox[abp]-)[A-Za-z0-9_\-]{8,}"#,
        r#"[A-Za-z0-9+/=_-]{48,}"#,
    ];
    raw.iter()
        .map(|source| Regex::new(source).expect("fixed leak pattern"))
        .collect()
}

pub(crate) fn scan_no_leaks(
    files: &[(String, Vec<u8>)],
    motif: &Value,
    private_strings: &BTreeSet<String>,
) -> Result<()> {
    for (rel, bytes) in files {
        let text = String::from_utf8_lossy(bytes);
        for private in private_strings {
            if private.len() >= 8 && text.contains(private.as_str()) {
                return Err(err(format!("private tracking identity would leak in {rel}")));
            }
        }
    }
    let patterns = leak_patterns();
    let excerpts: Vec<&str> = motif["representatives"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|row| row["evidence"].as_array().into_iter().flatten())
        .filter_map(|evidence| evidence["excerpt"].as_str())
        .collect();
    for excerpt in excerpts {
        for pattern in &patterns {
            if pattern.is_match(excerpt) {
                return Err(err("unredacted result text would leak in an excerpt"));
            }
        }
    }
    Ok(())
}
