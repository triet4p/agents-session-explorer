//! Pinned local motif query and the digest of its result.
//!
//! The exporter re-runs one fixed motif configuration over the selected
//! addresses and records that configuration in the bundle. Occurrence ids are
//! hashed here so the query digest and the exported evidence name occurrences
//! the same way.

use crate::bundle_digest::value_digest;
use crate::export_evidence::{occurrence_id, occurrence_value};
use ase_core::Result;
use ase_motif::{MotifConfig, MotifReport};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(crate) fn pinned_config() -> MotifConfig {
    let mut config = MotifConfig::default();
    config.family.k_min = 3;
    config.family.k_max = 5;
    config.family.tau = 0.5;
    config.max_gap = 2;
    config.band = 16;
    config.max_substitutions = 2;
    config.null_permutations = 64;
    config.null_seed = 11_400_714_819_323_198_485;
    config
}

pub(crate) fn query_config(config: &MotifConfig) -> Value {
    json!({
        "family": {
            "k_min": config.family.k_min,
            "k_max": config.family.k_max,
            "tau": config.family.tau
        },
        "max_gap": config.max_gap,
        "band": config.band,
        "max_substitutions": config.max_substitutions,
        "min_tasks": config.min_tasks,
        "null_permutations": config.null_permutations,
        "null_seed": config.null_seed.to_string()
    })
}

pub(crate) fn build_query_digest(report: &MotifReport, config: &MotifConfig) -> Result<String> {
    let mut motifs = Vec::with_capacity(report.motifs.len());
    for motif in &report.motifs {
        let mut occurrences = Vec::new();
        for occurrence in &motif.occurrences {
            if occurrence.duplicate_of.is_none() {
                occurrences.push(occurrence_id(&occurrence_value(occurrence))?);
            }
        }
        occurrences.sort();
        let mut relaxed = BTreeMap::new();
        for (gap, count) in motif.support_relaxed.iter().enumerate() {
            relaxed.insert(gap.to_string(), *count);
        }
        motifs.push(json!({
            "family_id": motif.family_id,
            "motif_id": motif.id,
            "occurrences": occurrences,
            "signatures": motif.signatures,
            "support": { "exact": motif.support_exact, "relaxed": relaxed },
        }));
    }
    motifs.sort_by(|a, b| a["motif_id"].as_str().cmp(&b["motif_id"].as_str()));
    Ok(value_digest(&json!({
        "config": query_config(config),
        "dedupe_rule": { "name": ase_motif::TEXT_HASH_RULE_NAME, "version": ase_motif::TEXT_HASH_RULE_VERSION },
        "motifs": motifs,
        "rule": { "name": report.rule_name, "version": report.rule_version },
    })))
}
