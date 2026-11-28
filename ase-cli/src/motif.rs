//! `ase motif` — dispatch-confirmed task sampling and motif reporting.
//!
//! Membership comes only from the native dispatch validator in `ase-motif`.
//! Address parsing is shared with the bundle exporter; ranking, ordering and
//! reopened evidence references are rendered exactly as the detector returns
//! them.

use crate::cli_option::parse_opt;
use crate::cli_option::{has, positionals};
use crate::cli_output::outln;
use crate::index::require_publication;
use crate::show::span_label;
use ase_core::{err, NONE, Result};
use serde_json::{json, Value};

pub(crate) struct MotifTarget {
    pub(crate) address: String,
    pub(crate) project: Option<String>,
    pub(crate) token: String,
}

/// Parse one motif address. `PROJECT|TASK` is the locked `(project, task)`
/// sampling key; a bare token keeps the unscoped contract and stays ambiguous
/// when two projects share it. No fallback rewrites one form into the other.
pub(crate) fn motif_target(address: &str) -> Result<MotifTarget> {
    match address.split_once('|') {
        Some((project, task)) => {
            if project.is_empty() || task.is_empty() || task.contains('|') {
                return Err(err(format!(
                    "motif address {address} must be PROJECT|TASK with non-empty parts"
                )));
            }
            Ok(MotifTarget {
                address: address.to_string(),
                project: Some(project.to_string()),
                token: task.to_string(),
            })
        }
        None => Ok(MotifTarget {
            address: address.to_string(),
            project: None,
            token: address.to_string(),
        }),
    }
}

pub fn run(args: &[String]) -> Result<()> {
    let positionals = positionals(args);
    if positionals.is_empty() {
        return Err(err("motif needs at least one task token"));
    }
    let targets = positionals
        .iter()
        .map(|address| motif_target(address))
        .collect::<Result<Vec<MotifTarget>>>()?;
    let (idx, store, published) = require_publication(args)?;

    let mut tasks = Vec::with_capacity(targets.len());
    let mut task_rows = Vec::with_capacity(targets.len());
    for target in &targets {
        let member =
            ase_motif::membership_scoped(&idx, &store, &target.token, target.project.as_deref())?;
        let projected = ase_motif::project(&published, &idx, &member)?;
        task_rows.push(json!({
            "address": &target.address,
            "addressing": if target.project.is_some() { "project_task" } else { "token" },
            "token": &target.token,
            "project": &member.project,
            "status": member.status.label(),
            "task_key": &member.task_key,
            "attempts": member.attempts.iter().map(|attempt| json!({
                "attempt": &attempt.attempt,
                "dispatch_id": &attempt.dispatch_id,
                "child_session_id": &attempt.child_session_id,
                "child_span": &attempt.child_span,
                "parent_span": &attempt.parent_span
            })).collect::<Vec<Value>>(),
            "rejected_claims": member.dispatch.issues_for_token.iter().map(|issue| json!({
                "record": &issue.record,
                "kind": &issue.kind,
                "dispatch_id": &issue.dispatch_id,
                "event": issue.event,
                "span": &issue.span,
                "detail": &issue.detail
            })).collect::<Vec<Value>>()
        }));
        tasks.push(projected);
    }

    let mut config = ase_motif::MotifConfig::default();
    if let Some(value) = parse_opt::<f64>(args, "--tau")? {
        config.family.tau = value;
    }
    if let Some(value) = parse_opt::<u32>(args, "--k-min")? {
        config.family.k_min = value;
    }
    if let Some(value) = parse_opt::<u32>(args, "--k-max")? {
        config.family.k_max = value;
    }
    if let Some(value) = parse_opt::<u32>(args, "--max-gap")? {
        config.max_gap = value;
    }
    if let Some(value) = parse_opt::<u32>(args, "--band")? {
        config.band = value;
    }
    if let Some(value) = parse_opt::<u32>(args, "--max-substitutions")? {
        config.max_substitutions = value;
    }
    if let Some(value) = parse_opt::<u32>(args, "--null-permutations")? {
        config.null_permutations = value;
    }
    if let Some(value) = parse_opt::<u32>(args, "--min-tasks")? {
        config.min_tasks = value;
    }

    let report = ase_motif::detect(&published, &idx, &store, &tasks, &config)?;

    let motifs: Vec<Value> = report
        .motifs
        .iter()
        .map(|motif| {
            let null_model = &motif.null;
            json!({
                "id": &motif.id,
                "family_id": &motif.family_id,
                "rule": { "name": report.rule_name, "version": report.rule_version },
                "reference": {
                    "task": &motif.reference_task,
                    "attempt": &motif.reference_attempt,
                    "sequence": motif.reference_sequence
                },
                "signatures": &motif.signatures,
                "columns": motif.columns.iter().map(|column| json!({
                    "index": column.index,
                    "signature": &column.signature,
                    "matches": column.matches,
                    "substitutions": &column.substitutions,
                    "missing": column.missing
                })).collect::<Vec<Value>>(),
                "support": {
                    "exact": motif.support_exact,
                    "relaxed": &motif.support_relaxed,
                    "gap_max": config.max_gap,
                    "rule": "distinct task keys; a task counts once no matter how many attempts or occurrences contain the motif; duplicate sequence text inside one task is counted once"
                },
                "null": {
                    "permutations": null_model.permutations,
                    "seed": null_model.seed.to_string(),
                    "observed": null_model.observed,
                    "expected": null_model.expected,
                    "stddev": null_model.stddev,
                    "z": null_model.z,
                    "lift": null_model.lift,
                    "rule": "fixed-seed SplitMix64 permutation of each distinct sequence unit's call order; duplicate sequence text is not permuted twice; exact support recounted per task"
                },
                "denominator": {
                    "tasks_total": motif.denominator.tasks_total,
                    "tasks_in_family": motif.denominator.tasks_in_family,
                    "tasks_with_occurrence": motif.denominator.tasks_with_occurrence,
                    "sequences_in_family": motif.denominator.sequences_in_family,
                    "sequences_with_occurrence": motif.denominator.sequences_with_occurrence,
                    "sequences_deduplicated": motif.denominator.sequences_deduplicated,
                    "excluded": motif.denominator.excluded.iter().map(|exclusion| json!({
                        "task": &exclusion.task,
                        "reason": &exclusion.reason
                    })).collect::<Vec<Value>>()
                },
                "occurrences": motif.occurrences.iter().map(|occurrence| json!({
                    "task": &occurrence.task,
                    "attempt": &occurrence.attempt,
                    "dispatch_id": &occurrence.dispatch_id,
                    "sequence": occurrence.sequence,
                    "text_hash": &occurrence.text_hash,
                    "text_duplicate": occurrence.duplicate_of.is_some(),
                    "duplicate_of": occurrence.duplicate_of.as_ref().map(|reference| json!({
                        "task": &reference.task,
                        "attempt": &reference.attempt,
                        "dispatch_id": &reference.dispatch_id,
                        "sequence": reference.sequence
                    })),
                    "gap": occurrence.gap,
                    "calls": occurrence.calls.iter().map(|call| json!({
                        "position": call.position,
                        "signature": &call.signature,
                        "generation": &call.evidence.generation,
                        "event": call.evidence.event,
                        "file": call.evidence.file,
                        "span": span_label(call.evidence.span),
                        "selector": &call.evidence.selector,
                        "block_hash": &call.evidence.block_hash,
                        "session_local": call.evidence.session_local,
                        "reopened": true
                    })).collect::<Vec<Value>>()
                })).collect::<Vec<Value>>(),
                "occurrences_truncated": motif.occurrences_truncated,
                "variants": motif.variants.iter().map(|variant| json!({
                    "task": &variant.task,
                    "attempt": &variant.attempt,
                    "sequence": variant.sequence,
                    "matches": variant.matches,
                    "substitutions": variant.substitutions,
                    "insertions": variant.insertions,
                    "deletions": variant.deletions,
                    "substituted_columns": &variant.substituted_columns,
                    "near_miss": variant.near_miss
                })).collect::<Vec<Value>>(),
                "sequence_exclusions": motif.sequence_exclusions.iter().map(|exclusion| json!({
                    "task": &exclusion.task,
                    "attempt": &exclusion.attempt,
                    "sequence": exclusion.sequence,
                    "reason": &exclusion.reason
                })).collect::<Vec<Value>>()
            })
        })
        .collect();

    let explain = vec![
        "Membership is one native dispatch join per attempt; naming, path and parentSession candidates are never inputs. `PROJECT|TASK` filters the same dispatch validator before reconciliation, so one token in two projects stays two independent samples and a bare token stays ambiguous.".to_string(),
        "A `resolve task:TOKEN` query also reads naming, marker, delegation and path signals; its `assigned` status alone never creates motif membership or a motif sample.".to_string(),
        "Support counts distinct task keys: repeated attempts and repeated occurrences of one task are one observation.".to_string(),
        "Duplicate sequence text is the SHA-256 of a sequence's reopened CallSig strings (rule motif-sequence-text / seq-text-v1). Dedupe is scoped to one task key, so distinct tasks keep their own samples; duplicate whole sequences are excluded from family/null/denominator counting but every occurrence stays reopenable evidence, and calls inside a sequence are never removed or reordered.".to_string(),
        "support_relaxed[g] allows at most g skipped calls between motif columns, so raising g never lowers support.".to_string(),
        "An occurrence may carry a gap above --max-gap; it stays evidence with its gap value, while support_relaxed only counts the configured ceiling.".to_string(),
        "The null model permutes each sequence's call order with the recorded fixed seed and recounts exact per-task support; lift and z compare the observation against that expectation.".to_string(),
        "Insertion and substitution stay separate: an added call is an insertion, a different call at the same column is a substitution.".to_string(),
        "Every occurrence call was reopened from raw bytes at its pinned generation, span and selector before entering the motif; `ase open --event --selector --generation` re-verifies a single call.".to_string(),
    ];

    let v = json!({
        "command": "motif",
        "status": "ok",
        "generation": published.generation,
        "tasks": task_rows,
        "config": {
            "rule": { "name": report.rule_name, "version": report.rule_version },
            "family": { "k_min": config.family.k_min, "k_max": config.family.k_max, "tau": config.family.tau },
            "dedupe": {
                "rule": { "name": ase_motif::TEXT_HASH_RULE_NAME, "version": ase_motif::TEXT_HASH_RULE_VERSION },
                "unit": "one projected CallSig sequence; text_hash = SHA-256 of its reopened signatures in order",
                "scope": "within one resolved task key (project|task); distinct tasks are never merged and no sequence crosses a branch or session boundary",
                "effect": "duplicate copies are excluded from family, null and sequence-denominator counting; support stays per task and every occurrence remains reopenable evidence"
            },
            "max_gap": config.max_gap,
            "band": config.band,
            "max_substitutions": config.max_substitutions,
            "null_permutations": config.null_permutations,
            "null_seed": config.null_seed.to_string(),
            "max_occurrences": config.max_occurrences,
            "min_tasks": config.min_tasks
        },
        "summary": {
            "tasks_total": report.tasks,
            "sequences": report.sequences,
            "sequences_raw": report.sequences_raw,
            "sequences_deduplicated": report.deduplicated_sequences.len(),
            "families": report.families,
            "families_below_min_tasks": report.families_below_min_tasks,
            "motifs": motifs.len()
        },
        "excluded_tasks": report.excluded_tasks.iter().map(|exclusion| json!({
            "task": &exclusion.task,
            "reason": exclusion.reason.label()
        })).collect::<Vec<Value>>(),
        "deduplicated_sequences": report.deduplicated_sequences.iter().map(|duplicate| json!({
            "task": &duplicate.task,
            "attempt": &duplicate.attempt,
            "dispatch_id": &duplicate.dispatch_id,
            "sequence": duplicate.sequence,
            "text_hash": &duplicate.text_hash,
            "duplicate_of": {
                "task": &duplicate.duplicate_of.task,
                "attempt": &duplicate.duplicate_of.attempt,
                "dispatch_id": &duplicate.duplicate_of.dispatch_id,
                "sequence": duplicate.duplicate_of.sequence
            },
            "effect": "not counted; the occurrence remains reopenable evidence"
        })).collect::<Vec<Value>>(),
        "motifs": motifs,
        "explain": if has(args, "--explain") { explain } else { Vec::<String>::new() }
    });

    if has(args, "--json") {
        outln!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        outln!(
            "motif: {} task(s), {} distinct sequence(s) ({} raw, {} duplicate text), {} family(ies), {} motif(s)",
            report.tasks,
            report.sequences,
            report.sequences_raw,
            report.deduplicated_sequences.len(),
            report.families,
            report.motifs.len()
        );
        for exclusion in &report.excluded_tasks {
            outln!(
                "  excluded task {} ({})",
                exclusion.task,
                exclusion.reason.label()
            );
        }
        for motif in &report.motifs {
            let z = motif
                .null
                .z
                .map(|value| format!("{value:.2}"))
                .unwrap_or_else(|| "-".to_string());
            let lift = motif
                .null
                .lift
                .map(|value| format!("{value:.2}"))
                .unwrap_or_else(|| "-".to_string());
            outln!(
                "  {} tasks={}/{} exact={} relaxed={:?} z={} lift={} occurrences={}",
                motif.id,
                motif.denominator.tasks_in_family,
                motif.denominator.tasks_total,
                motif.support_exact,
                motif.support_relaxed,
                z,
                lift,
                motif.occurrences.len()
            );
            outln!(
                "    signatures: {}",
                motif.signatures.join(" -> ")
            );
            for occurrence in &motif.occurrences {
                outln!(
                    "    occurrence task={} attempt={} gap={} text_duplicate={} first_event={}",
                    occurrence.task,
                    occurrence.attempt,
                    occurrence.gap,
                    occurrence.duplicate_of.is_some(),
                    occurrence
                        .calls
                        .first()
                        .map(|call| call.evidence.event)
                        .unwrap_or(NONE)
                );
            }
        }
        if has(args, "--explain") {
            for step in v["explain"].as_array().into_iter().flatten() {
                outln!("  step: {}", step.as_str().unwrap_or(""));
            }
        }
    }
    Ok(())
}
