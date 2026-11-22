//! Deterministic shingle families over projected CallSig sequences.
//!
//! Each sequence becomes a set of `shingle-k3k5-v1` tokens. Two sequences form
//! an edge when their Jaccard overlap reaches `tau`; families are the connected
//! components of that graph (single linkage). Input order is the only order:
//! pairs are visited by ascending index and components are sorted by their
//! lowest member, so the same sequences always produce the same families and
//! family ids.

use ase_core::{err, Result};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

use crate::projection::Sequence;

/// Identifier of the grouping rule recorded on a report.
pub const RULE_NAME: &str = "motif-callsig-family";
pub const RULE_VERSION: &str = "callsig-family-v1";

#[derive(Clone, Copy, Debug)]
pub struct FamilyConfig {
    pub k_min: u32,
    pub k_max: u32,
    /// Minimum Jaccard overlap; a caller parameter, not a measured gate.
    pub tau: f64,
}

impl Default for FamilyConfig {
    fn default() -> Self {
        Self {
            k_min: 3,
            k_max: 5,
            tau: 0.5,
        }
    }
}

impl FamilyConfig {
    pub fn validate(&self) -> Result<()> {
        if self.k_min == 0 || self.k_max < self.k_min || self.k_max > 64 {
            return Err(err("family config needs 1 <= k_min <= k_max <= 64"));
        }
        if !self.tau.is_finite() || !(0.0..=1.0).contains(&self.tau) {
            return Err(err("family config tau must be a finite value in [0, 1]"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Family {
    pub id: String,
    pub members: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct FamilyReport {
    pub rule_name: &'static str,
    pub rule_version: &'static str,
    pub config: FamilyConfig,
    pub sequences: usize,
    pub eligible_sequences: usize,
    /// Sequence indices whose configured `k` range yields no shingle.
    pub empty_sequences: Vec<usize>,
    /// How many eligible pairs were compared; reported, never hidden.
    pub pairs_compared: usize,
    pub families: Vec<Family>,
}

impl FamilyReport {
    pub fn family_of(&self, sequence: usize) -> Option<&Family> {
        self.families
            .iter()
            .find(|family| family.members.binary_search(&sequence).is_ok())
    }

    pub fn of_size(&self, size: usize) -> Vec<&Family> {
        self.families
            .iter()
            .filter(|family| family.members.len() == size)
            .collect()
    }

    /// Sequence indices that connected to nothing.
    pub fn singletons(&self) -> Vec<usize> {
        self.families
            .iter()
            .filter(|family| family.members.len() == 1)
            .map(|family| family.members[0])
            .collect()
    }
}

fn tokens_of(sequence: &Sequence, config: &FamilyConfig) -> BTreeSet<String> {
    sequence
        .shingle_tokens(config.k_min, config.k_max)
        .into_iter()
        .collect()
}

/// Jaccard overlap of two token sets. Two empty sets are never similar.
pub fn jaccard(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(right).count();
    let union = left.union(right).count();
    intersection as f64 / union as f64
}

fn find(parent: &mut [usize], mut node: usize) -> usize {
    while parent[node] != node {
        parent[node] = parent[parent[node]];
        node = parent[node];
    }
    node
}

fn family_id(tokens: &BTreeSet<String>) -> String {
    let mut hasher = Sha256::new();
    for token in tokens {
        hasher.update(token.as_bytes());
        hasher.update(b"\n");
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(16);
    for byte in &digest[..8] {
        hex.push_str(&format!("{byte:02x}"));
    }
    format!("fam-{hex}")
}

/// Group sequences by Jaccard overlap over their shingle tokens.
pub fn families(sequences: &[&Sequence], config: &FamilyConfig) -> Result<FamilyReport> {
    config.validate()?;
    let token_sets: Vec<BTreeSet<String>> = sequences
        .iter()
        .map(|sequence| tokens_of(sequence, config))
        .collect();
    let mut empty_sequences = Vec::new();
    let mut eligible = Vec::new();
    for (index, tokens) in token_sets.iter().enumerate() {
        if tokens.is_empty() {
            empty_sequences.push(index);
        } else {
            eligible.push(index);
        }
    }

    let mut parent: Vec<usize> = (0..sequences.len()).collect();
    let mut pairs_compared = 0usize;
    for (position, left) in eligible.iter().enumerate() {
        for right in &eligible[position + 1..] {
            pairs_compared += 1;
            let overlap = jaccard(&token_sets[*left], &token_sets[*right]);
            if overlap >= config.tau {
                let left_root = find(&mut parent, *left);
                let right_root = find(&mut parent, *right);
                if left_root != right_root {
                    // Keep the lower index as the component root.
                    if left_root < right_root {
                        parent[right_root] = left_root;
                    } else {
                        parent[left_root] = right_root;
                    }
                }
            }
        }
    }

    let mut components: Vec<Vec<usize>> = Vec::new();
    let mut slot: Vec<Option<usize>> = vec![None; sequences.len()];
    for index in eligible {
        let root = find(&mut parent, index);
        match slot[root] {
            Some(position) => components[position].push(index),
            None => {
                slot[root] = Some(components.len());
                components.push(vec![index]);
            }
        }
    }
    let mut families: Vec<Family> = components
        .into_iter()
        .map(|members| {
            let mut members = members;
            members.sort_unstable();
            let canonical = members[0];
            Family {
                id: family_id(&token_sets[canonical]),
                members,
            }
        })
        .collect();
    families.sort_by_key(|family| family.members[0]);

    Ok(FamilyReport {
        rule_name: RULE_NAME,
        rule_version: RULE_VERSION,
        config: *config,
        sequences: sequences.len(),
        eligible_sequences: eligible_count(&token_sets),
        empty_sequences,
        pairs_compared,
        families,
    })
}

fn eligible_count(token_sets: &[BTreeSet<String>]) -> usize {
    token_sets.iter().filter(|tokens| !tokens.is_empty()).count()
}
