//! Aligned comparison between the product projection and the independent T1
//! reference counts.
//!
//! Journals are aligned by exact file SHA-256, not by path, order or count.
//! Per-journal differences are reported before aggregation so opposite-sign
//! errors cannot hide inside a matching total.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize)]
pub struct ReferenceRow {
    pub source_id: String,
    pub sha256: String,
    pub session_id: Option<String>,
    pub kind: String,
    pub tree_id: String,
    pub header_timestamp: Option<String>,
    pub leaf_entry_id: Option<String>,
    pub leaf_ordinal: Option<u32>,
    pub leaf_timestamp: Option<String>,
    pub leaf_type: Option<String>,
    pub leaf_count: u32,
    pub turns_all_branches: u32,
    pub turns_active_leaf: u32,
    pub abandoned_turns: u32,
    pub episodes_primary_count: u32,
    pub episodes_wide_count: u32,
    #[serde(default)]
    pub parse_errors: u32,
    #[serde(default)]
    pub missing_parent_refs: u32,
    #[serde(default)]
    pub cycles: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct DiffRow {
    pub source_id: String,
    pub reference_source_id: String,
    pub sha256_prefix: String,
    pub turns_active_leaf: i64,
    pub turns_all_branches: i64,
    pub episodes_primary_count: i64,
    pub episodes_wide_count: i64,
    pub leaf_entry_match: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Comparison {
    pub status: String,
    pub rule: String,
    pub reference_rule: String,
    pub alignment: String,
    pub product_files: u64,
    pub reference_files: u64,
    pub matched_files: u64,
    pub product_total_active_leaf: u64,
    pub reference_total_active_leaf: u64,
    pub matched_product_total_active_leaf: u64,
    pub matched_reference_total_active_leaf: u64,
    pub net_diff_active_leaf: i64,
    pub gross_abs_diff_active_leaf: u64,
    pub cancelled_turns: u64,
    pub max_abs_diff_active_leaf: u64,
    pub journals_differing_active_leaf: u64,
    pub relative_error: f64,
    pub strict_relative_error_below_1_percent: bool,
    pub threshold: f64,
    pub cohort_match: bool,
    pub product_only: Vec<String>,
    pub reference_only: Vec<String>,
    pub diffs: Vec<DiffRow>,
    pub product_totals: Totals,
    pub reference_totals: Totals,
    pub excluded: Exclusions,
    pub disclosure: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Totals {
    pub files: u64,
    pub turns_active_leaf: u64,
    pub turns_all_branches: u64,
    pub abandoned_turns: u64,
    pub episodes_primary_count: u64,
    pub episodes_wide_count: u64,
    pub leaf_null: u64,
    pub parse_errors: u64,
    pub missing_parent_refs: u64,
    pub cycles: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Exclusions {
    pub product_only_files: u64,
    pub reference_only_files: u64,
    pub duplicate_product_digests: u64,
    pub duplicate_reference_digests: u64,
    pub product_leaf_null_matched: u64,
    pub reference_leaf_null_matched: u64,
}

fn index_rows<T, F>(rows: &[T], key: F) -> Result<BTreeMap<String, usize>, String>
where
    F: Fn(&T) -> String,
{
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    let mut duplicates = 0u64;
    for (position, row) in rows.iter().enumerate() {
        if index.insert(key(row), position).is_some() {
            duplicates += 1;
        }
    }
    if duplicates > 0 {
        return Err(format!("duplicate file digests: {duplicates}"));
    }
    Ok(index)
}

fn totals_from_rows(rows: &[&ReferenceRow]) -> Totals {
    let mut t = Totals::default();
    for row in rows {
        t.files += 1;
        t.turns_active_leaf += row.turns_active_leaf as u64;
        t.turns_all_branches += row.turns_all_branches as u64;
        t.abandoned_turns += row.abandoned_turns as u64;
        t.episodes_primary_count += row.episodes_primary_count as u64;
        t.episodes_wide_count += row.episodes_wide_count as u64;
        if row.leaf_entry_id.is_none() {
            t.leaf_null += 1;
        }
        t.parse_errors += row.parse_errors as u64;
        t.missing_parent_refs += row.missing_parent_refs as u64;
        t.cycles += row.cycles as u64;
    }
    t
}

fn totals_from_product(rows: &[&crate::model::JournalRow]) -> Totals {
    let mut t = Totals::default();
    for row in rows {
        t.files += 1;
        t.turns_active_leaf += row.turns_active_leaf as u64;
        t.turns_all_branches += row.turns_all_branches as u64;
        t.abandoned_turns += row.abandoned_turns as u64;
        t.episodes_primary_count += row.episodes_primary_count as u64;
        t.episodes_wide_count += row.episodes_wide_count as u64;
        if row.leaf_entry_id.is_none() {
            t.leaf_null += 1;
        }
        t.parse_errors += row.parse_errors as u64;
        t.missing_parent_refs += row.missing_parent_refs as u64;
        t.cycles += row.cycles as u64;
    }
    t
}

/// Compares product rows to reference rows. Alignment is by exact SHA-256;
/// excluded rows are listed by opaque source id and never folded into the
/// matched denominator.
pub fn compare(
    product: &[crate::model::JournalRow],
    reference: &[ReferenceRow],
) -> Result<Comparison, String> {
    let product_index = index_rows(product, |r| r.sha256.clone())?;
    let reference_index = index_rows(reference, |r| r.sha256.clone())?;

    let mut matched: Vec<(&crate::model::JournalRow, &ReferenceRow)> = Vec::new();
    let mut product_only: Vec<String> = Vec::new();
    let mut reference_only: Vec<String> = Vec::new();
    for (sha, &position) in &product_index {
        match reference_index.get(sha) {
            Some(&other) => matched.push((&product[position], &reference[other])),
            None => product_only.push(product[position].source_id.clone()),
        }
    }
    for (sha, &position) in &reference_index {
        if !product_index.contains_key(sha) {
            reference_only.push(reference[position].source_id.clone());
        }
    }
    product_only.sort();
    reference_only.sort();
    matched.sort_by(|a, b| a.0.source_id.cmp(&b.0.source_id));

    let mut net: i64 = 0;
    let mut gross: u64 = 0;
    let mut max_abs: u64 = 0;
    let mut differing: u64 = 0;
    let mut matched_product_total: u64 = 0;
    let mut matched_reference_total: u64 = 0;
    let mut product_leaf_null = 0u64;
    let mut reference_leaf_null = 0u64;
    let mut diffs: Vec<DiffRow> = Vec::new();
    let mut product_refs: Vec<&crate::model::JournalRow> = Vec::new();
    let mut reference_refs: Vec<&ReferenceRow> = Vec::new();
    for (p, r) in &matched {
        product_refs.push(p);
        reference_refs.push(r);
        matched_product_total += p.turns_active_leaf as u64;
        matched_reference_total += r.turns_active_leaf as u64;
        if p.leaf_entry_id.is_none() {
            product_leaf_null += 1;
        }
        if r.leaf_entry_id.is_none() {
            reference_leaf_null += 1;
        }
        let diff = p.turns_active_leaf as i64 - r.turns_active_leaf as i64;
        let abs = diff.unsigned_abs();
        net += diff;
        gross += abs;
        max_abs = max_abs.max(abs);
        if diff != 0 {
            differing += 1;
            diffs.push(DiffRow {
                source_id: p.source_id.clone(),
                reference_source_id: r.source_id.clone(),
                sha256_prefix: p.sha256.chars().take(12).collect(),
                turns_active_leaf: diff,
                turns_all_branches: p.turns_all_branches as i64 - r.turns_all_branches as i64,
                episodes_primary_count: p.episodes_primary_count as i64
                    - r.episodes_primary_count as i64,
                episodes_wide_count: p.episodes_wide_count as i64
                    - r.episodes_wide_count as i64,
                leaf_entry_match: p.leaf_entry_id == r.leaf_entry_id,
            });
        }
    }

    let relative = if matched_reference_total == 0 {
        if net == 0 { 0.0 } else { f64::INFINITY }
    } else {
        net.unsigned_abs() as f64 / matched_reference_total as f64
    };
    let threshold = 0.01_f64;
    let strict = relative < threshold;
    let cancelled = gross.saturating_sub(net.unsigned_abs());
    let product_only_len = product_only.len() as u64;
    let reference_only_len = reference_only.len() as u64;

    let mut disclosure = vec![
        "Alignment is by exact file SHA-256, never by path, order or count.".to_string(),
        "Matched totals exclude product-only and reference-only files; both are listed.".to_string(),
        "Gross absolute per-journal differences and cancelled turns are reported so opposite-sign errors cannot hide in the aggregate.".to_string(),
        "Turns before the first user boundary are counted in turns_active_leaf but not in the primary episode sum.".to_string(),
        "Wide episode boundaries include `custom_message` entries with attribution user; primary boundaries are user messages only.".to_string(),
        "The projection is task-agnostic; unresolved and no-task sessions are included identically.".to_string(),
    ];
    if product_only_len > 0 || reference_only_len > 0 {
        disclosure.push(format!(
            "Cohort sets differ: {product_only_len} product-only and {reference_only_len} reference-only files are excluded from the aligned gate."
        ));
    }

    Ok(Comparison {
        status: if strict { "pass".to_string() } else { "fail".to_string() },
        rule: crate::model::WORK_RULE.to_string(),
        reference_rule: crate::model::COMPATIBLE_REFERENCE_RULE.to_string(),
        alignment: "sha256".to_string(),
        product_files: product.len() as u64,
        reference_files: reference.len() as u64,
        matched_files: matched.len() as u64,
        product_total_active_leaf: product.iter().map(|r| r.turns_active_leaf as u64).sum(),
        reference_total_active_leaf: reference.iter().map(|r| r.turns_active_leaf as u64).sum(),
        matched_product_total_active_leaf: matched_product_total,
        matched_reference_total_active_leaf: matched_reference_total,
        net_diff_active_leaf: net,
        gross_abs_diff_active_leaf: gross,
        cancelled_turns: cancelled,
        max_abs_diff_active_leaf: max_abs,
        journals_differing_active_leaf: differing,
        relative_error: relative,
        strict_relative_error_below_1_percent: strict,
        threshold,
        cohort_match: product_only_len == 0 && reference_only_len == 0,
        product_only,
        reference_only,
        diffs,
        product_totals: totals_from_product(&product_refs),
        reference_totals: totals_from_rows(&reference_refs),
        excluded: Exclusions {
            product_only_files: product_only_len,
            reference_only_files: reference_only_len,
            duplicate_product_digests: 0,
            duplicate_reference_digests: 0,
            product_leaf_null_matched: product_leaf_null,
            reference_leaf_null_matched: reference_leaf_null,
        },
        disclosure,
    })
}
