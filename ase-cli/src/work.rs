//! `ase work` — leaf-aware journal projection, reopen, comparison and delta.
//!
//! Owns the command router and the explicit attempt delta contract. Project
//! selection uses header time; delta membership comes only from dispatch
//! joins. Projection and evidence checks are shared with analytics.

use ase_core::{err, NONE, RawStore, Result};
use ase_index::{FsRawStore, Index};
use ase_motif::membership::dispatch_input;
use ase_task::{Evidence as TaskEvidence, Status as TaskStatus};
use ase_work::{JournalProjection, JournalRow, ManifestEntry, ReferenceRow, Window};
use rayon::prelude::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use crate::config;
use crate::journal_projection::{
    load_work, parse_work_window, project_work_file, read_jsonl, validate_work_projection,
    ProjectedWork, SelectedWork, WorkFile,
};
use crate::cli_option::{has, opt, positional};
use crate::cli_output::emit;
use crate::index::index_path;
use crate::show::unique_session;

fn work_usage() {
    println!(
        "ase work project [--index FILE] [--omp-root DIR] [--start ISO --end ISO]\n\
         [--out ROWS.jsonl] [--limit N] [--json]\n\
         ase work session (--session ID | --file N) [--leaf ENTRY_ID] [--omp-root DIR]\n\
         [--out PROJECTION.json] [--json]\n\
         ase work reopen PROJECTION.json [--index FILE] [--json]\n\
         ase work delta --task TOKEN [--project PROJECT] --before CHILD_SESSION_ID\n\
         \n         --after CHILD_SESSION_ID [--omp-root DIR] [--out DELTA.json]\n\
         \n         [--before-out FILE.json] [--after-out FILE.json] [--json]\n\
         ase work compare --projection ROWS.jsonl --reference SOURCES.jsonl\n\
         [--out REPORT.json] [--json]"
    );
}

pub fn run(args: &[String]) -> Result<()> {
    let Some(sub) = args.first().map(String::as_str) else {
        work_usage();
        return Err(err("work needs project, session, reopen, compare or delta"));
    };
    let rest = &args[1..];
    if matches!(sub, "--help" | "-h") {
        work_usage();
        return Ok(());
    }
    match sub {
        "project" => cmd_work_project(rest),
        "session" => cmd_work_session(rest),
        "reopen" => cmd_work_reopen(rest),
        "compare" => cmd_work_compare(rest),
        "delta" => cmd_work_delta(rest),
        other => Err(err(format!("unknown work command: {other}"))),
    }
}


fn header_events(idx: &Index) -> Result<std::collections::BTreeMap<u32, u32>> {
    let mut out = std::collections::BTreeMap::new();
    for session in 0..idx.session_count {
        let s = idx.session(session)?;
        if s.header() != NONE {
            out.insert(s.file(), s.header());
        }
    }
    Ok(out)
}


fn collect_work_files(
    idx: &Index,
    omp_root: &str,
    window: Option<Window>,
    limit: Option<usize>,
) -> Result<SelectedWork> {
    let headers = header_events(idx)?;
    let mut roots: std::collections::BTreeMap<String, (u32, ase_work::util::Layout, bool)> =
        std::collections::BTreeMap::new();
    let mut subagents: Vec<WorkFile> = Vec::new();
    let mut scanned = 0u64;
    let mut outside = 0u64;
    let mut missing = 0u64;
    for file_id in 0..idx.file_count {
        let file = idx.file(file_id)?;
        if file.provider() != ase_core::Provider::Omp {
            continue;
        }
        let Some(rel) = ase_work::relpath(file.path(), omp_root) else {
            continue;
        };
        let Some(layout) = ase_work::layout(&rel) else {
            continue;
        };
        if layout.kind == "root" {
            scanned += 1;
            let Some(header) = headers.get(&file_id) else {
                missing += 1;
                continue;
            };
            let ts = idx.event(*header)?.ts_ms();
            if ts == 0 {
                missing += 1;
                continue;
            }
            if window.is_some_and(|w| !w.contains(ts)) {
                outside += 1;
                continue;
            }
            roots.insert(layout.tree_relpath.clone(), (file_id, layout, true));
        } else {
            subagents.push(WorkFile { file_id, rel, layout });
        }
    }
    let selected_roots = roots.values().filter(|(_, _, selected)| *selected).count() as u64;
    let mut files: Vec<WorkFile> = Vec::new();
    for (file_id, layout, selected) in roots.values() {
        if *selected {
            files.push(WorkFile {
                file_id: *file_id,
                rel: layout.tree_relpath.clone(),
                layout: layout.clone(),
            });
        }
    }
    for sub in subagents {
        let Some((_, _, selected)) = roots.get(&sub.layout.tree_relpath) else {
            continue;
        };
        if *selected {
            files.push(sub);
        }
    }
    files.sort_by(|a, b| a.rel.as_bytes().cmp(b.rel.as_bytes()));
    if let Some(limit) = limit {
        files.truncate(limit);
    }
    Ok(SelectedWork {
        files,
        root_journals_scanned: scanned,
        root_journals_selected: selected_roots,
        root_journals_outside_window: outside,
        root_journals_missing_header: missing,
    })
}


fn cmd_work_project(args: &[String]) -> Result<()> {
    let (idx, store, generation) = load_work(args)?;
    let window = parse_work_window(args)?;
    let omp_root = opt(args, "--omp-root").unwrap_or_else(|| config::omp_root());
    let limit = opt(args, "--limit").and_then(|s| s.parse::<usize>().ok());
    let selected = collect_work_files(&idx, &omp_root, window, limit)?;
    let projected: Vec<ProjectedWork> = selected
        .files
        .par_iter()
        .map(|file| project_work_file(&idx, &store, file, &generation, window))
        .collect::<Result<Vec<_>>>()?;

    let mut rows: Vec<JournalRow> = Vec::with_capacity(projected.len());
    let mut counts = ase_work::AggregateCounts::default();
    let mut entries: Vec<ManifestEntry> = Vec::with_capacity(projected.len());
    let mut filename_header_mismatches = 0u64;
    let mut subagent_headers_missing = 0u64;
    for item in &projected {
        let tree_id = ase_work::source_id(&item.entry.tree_relpath);
        let row = JournalRow::from_projection(&item.projection, &item.entry.kind, &tree_id);
        counts.add(&row, item.entry.size);
        if let (Some(filename), Some(header)) =
            (&item.entry.filename_timestamp, &item.entry.header_timestamp)
        {
            if filename != header {
                filename_header_mismatches += 1;
            }
        }
        if item.entry.kind == "subagent" && !item.entry.header_ok {
            subagent_headers_missing += 1;
        }
        rows.push(row);
        entries.push(item.entry.clone());
    }
    entries.sort_by(|a, b| a.relpath.as_bytes().cmp(b.relpath.as_bytes()));
    let manifest = ase_work::manifest_digest(&entries);
    let snapshot = ase_work::CohortSnapshot {
        manifest_sha256: manifest.clone(),
        files: counts.files,
        root_files: counts.root_files,
        subagent_files: counts.subagent_files,
        bytes: counts.bytes,
        root_journals_scanned: selected.root_journals_scanned,
        root_journals_selected: selected.root_journals_selected,
        root_journals_outside_window: selected.root_journals_outside_window,
        root_journals_missing_header: selected.root_journals_missing_header,
        filename_header_mismatches,
        subagent_headers_missing,
    };

    if let Some(out) = opt(args, "--out") {
        let mut content = String::new();
        for row in &rows {
            content.push_str(
                &serde_json::to_string(row)
                    .map_err(|e| err(format!("serialize work row: {e}")))?,
            );
            content.push('\n');
        }
        fs::write(&out, content).map_err(|e| err(format!("write work rows {out}: {e}")))?;
    }

    let window_value = window.map(|w| {
        json!({
            "start": ase_work::format_iso_ms(w.start_ms),
            "end": ase_work::format_iso_ms(w.end_ms),
            "end_exclusive": true
        })
    });
    let value = json!({
        "command": "work project",
        "status": "ok",
        "adapter": "omp-stock",
        "rule": ase_work::WORK_RULE,
        "rule_version": ase_work::WORK_VERSION,
        "compatible_reference_rule": ase_work::COMPATIBLE_REFERENCE_RULE,
        "generation": generation,
        "window": window_value,
        "selection_rule": "source tree = root journal <project>/<name>.jsonl plus every *.jsonl under <project>/<name>/; selected when root session timestamp is in [start, end) UTC; all files of a selected tree are read",
        "task_dependency": "none",
        "resolver_membership": "not_read",
        "limited": limit.is_some(),
        "snapshot": snapshot,
        "counts": counts,
        "journals_written": rows.len(),
        "journals": if has(args, "--json") {
            Value::Array(rows.iter().map(|r| serde_json::to_value(r).unwrap()).collect())
        } else {
            Value::Array(Vec::new())
        }
    });
    emit(&value, args);
    Ok(())
}


fn cmd_work_session(args: &[String]) -> Result<()> {
    let (idx, store, generation) = load_work(args)?;
    let omp_root = opt(args, "--omp-root").unwrap_or_else(|| config::omp_root());
    let file_id = if let Some(session) = opt(args, "--session") {
        let session_idx = unique_session(&idx, &session)?;
        idx.session(session_idx)?.file()
    } else if let Some(file) = opt(args, "--file").and_then(|s| s.parse::<u32>().ok()) {
        file
    } else {
        return Err(err("work session needs --session ID or --file N"));
    };
    let view = idx.file(file_id)?;
    let rel = ase_work::relpath(view.path(), &omp_root)
        .unwrap_or_else(|| ase_work::util::slash_path(view.path()));
    let bytes = store.bytes(file_id)?;
    let sha = ase_work::sha256_hex(&bytes);
    let sid = ase_work::source_id(&rel);
    let leaf = opt(args, "--leaf");
    let options = ase_work::ProjectOptions {
        file_id,
        path: view.path(),
        source_id: &sid,
        generation: &generation,
        sha256: &sha,
        event_base: view.event_start(),
        explicit_leaf: leaf.as_deref(),
        window: None,
    };
    let projection = ase_work::project_bytes(&options, &bytes)?;
    validate_work_projection(&idx, &projection)?;
    if let Some(out) = opt(args, "--out") {
        let content = serde_json::to_vec_pretty(&projection)
            .map_err(|e| err(format!("serialize projection: {e}")))?;
        fs::write(&out, content).map_err(|e| err(format!("write projection {out}: {e}")))?;
    }
    let value = json!({
        "command": "work session",
        "status": "ok",
        "generation": generation,
        "task_dependency": "none",
        "resolver_membership": "not_read",
        "projection": projection
    });
    emit(&value, args);
    Ok(())
}


fn cmd_work_reopen(args: &[String]) -> Result<()> {
    let path = opt(args, "--projection")
        .or_else(|| positional(args))
        .ok_or_else(|| err("work reopen needs PROJECTION.json"))?;
    let bytes = fs::read(&path).map_err(|e| err(format!("read projection {path}: {e}")))?;
    let projection: JournalProjection = serde_json::from_slice(&bytes)
        .map_err(|e| err(format!("parse projection {path}: {e}")))?;
    let (idx, store, generation) = load_work(args)?;
    let view = idx.file(projection.file_id)?;
    let mut report = if view.path() == projection.path {
        let raw = store.bytes(projection.file_id)?;
        ase_work::reopen_bytes(&projection, &raw, &generation)
    } else {
        let mut report = ase_work::ReopenReport {
            status: "stale".to_string(),
            generation,
            file_id: projection.file_id,
            source_id: projection.source_id.clone(),
            ..ase_work::ReopenReport::default()
        };
        report
            .errors
            .push("file_mapping_mismatch:index maps this file id to another path".to_string());
        report
    };
    if report.status == "verified" {
        report.errors.clear();
    }
    let value = serde_json::to_value(&report)
        .map_err(|e| err(format!("render reopen report: {e}")))?;
    emit(&value, args);
    match report.status.as_str() {
        "verified" => Ok(()),
        "generation_mismatch" => std::process::exit(4),
        _ => std::process::exit(3),
    }
}


fn cmd_work_compare(args: &[String]) -> Result<()> {
    let projection = opt(args, "--projection")
        .ok_or_else(|| err("work compare needs --projection ROWS.jsonl"))?;
    let reference = opt(args, "--reference")
        .ok_or_else(|| err("work compare needs --reference SOURCES.jsonl"))?;
    let product: Vec<JournalRow> = read_jsonl(&projection)?;
    let expected: Vec<ReferenceRow> = read_jsonl(&reference)?;
    let report = ase_work::compare(&product, &expected)
        .map_err(|e| err(format!("compare: {e}")))?;
    if let Some(out) = opt(args, "--out") {
        let content = serde_json::to_vec_pretty(&report)
            .map_err(|e| err(format!("serialize comparison: {e}")))?;
        fs::write(&out, content).map_err(|e| err(format!("write comparison {out}: {e}")))?;
    }
    let value = serde_json::to_value(&report)
        .map_err(|e| err(format!("render comparison: {e}")))?;
    emit(&value, args);
    if !report.strict_relative_error_below_1_percent || !report.cohort_match {
        std::process::exit(3);
    }
    Ok(())
}


/// Raw dispatch records for one submitted session, always labelled as claims
/// rather than membership. Used only for rejection diagnostics.
fn delta_observed_claims(input: &ase_task::DispatchInput, session_id: &str) -> Vec<Value> {
    input
        .children
        .iter()
        .filter(|link| link.session_id == session_id && link.containing_session_id == session_id)
        .map(|link| {
            let parent = input.parents.iter().find(|claim| {
                claim.dispatch_id == link.dispatch_id
                    && claim.containing_session_id == claim.parent_session_id
            });
            json!({
                "dispatch_id": &link.dispatch_id,
                "child_session_id": &link.session_id,
                "child_span": &link.span,
                "child_event": link.event,
                "parent_session_id": parent.map(|p| p.parent_session_id.clone()),
                "project": parent.map(|p| p.project.clone()),
                "task": parent.map(|p| p.task.clone()),
                "attempt": parent.map(|p| p.attempt.clone()),
                "parent_span": parent.map(|p| p.span.clone()),
                "label": "observed_dispatch_claim_not_membership"
            })
        })
        .collect()
}


fn work_delta_reject(
    args: &[String],
    task: &str,
    project: Option<&str>,
    before: &str,
    after: &str,
    reason: &str,
    detail: &str,
    resolution: Option<&ase_task::Resolution>,
    claims: Value,
) -> ! {
    let admitted: Vec<Value> = resolution
        .map(|found| {
            found
                .evidence
                .iter()
                .filter_map(|evidence| match evidence {
                    TaskEvidence::Dispatch(join) => Some(json!({
                        "dispatch_id": join.dispatch_id,
                        "project": join.project,
                        "task": join.task,
                        "attempt": join.attempt,
                        "child_session_id": join.child_session_id,
                        "parent_span": join.parent_span,
                        "child_span": join.child_span
                    })),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let candidates: Vec<Value> = resolution
        .map(|found| {
            found
                .candidates
                .iter()
                .map(|candidate| {
                    json!({
                        "project": candidate.project,
                        "task": candidate.task,
                        "task_key": candidate.task_key,
                        "state": candidate.state.label(),
                        "reason": candidate.reason
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let value = json!({
        "command": "work delta",
        "status": "rejected",
        "reason": reason,
        "detail": detail,
        "task": task,
        "project": project,
        "before": before,
        "after": after,
        "rule": ase_work::DELTA_RULE,
        "rule_version": ase_work::DELTA_VERSION,
        "membership_rule": ase_task::DISPATCH_RULE_NAME,
        "membership_rule_version": ase_task::DISPATCH_RULE_VERSION,
        "dispatch": resolution.map(|found| json!({
            "status": found.status.label(),
            "task_key": found.task_key,
            "summary": found.summary,
            "parents_indexed": found.dispatch.parents,
            "child_links_indexed": found.dispatch.children,
            "unique_joins": found.dispatch.joined,
            "rejected_groups": found.dispatch.rejected_groups,
            "record_issues": found.dispatch.issues_total
        })).unwrap_or(Value::Null),
        "admitted_attempts": admitted,
        "candidates": candidates,
        "observed_claims": claims,
        "guarantees": [
            "Both sides must be executing child sessions of admitted dispatch-v1 joins for the same project/task key; no naming, marker, adapter or motif candidate can confirm membership.",
            "A bare token with two project scopes is ambiguous and rejected; a session whose raw claim names another task/project is rejected as a task mismatch.",
            "Attempt order uses dispatch parent-record time with deterministic tie-breaks; attempt labels are displayed but never used as chronology.",
            "No delta is produced for a rejected pair, and no source, index, resolver or membership store is written."
        ]
    });
    emit(&value, args);
    std::process::exit(3);
}


fn project_delta_attempt(
    idx: &Index,
    store: &FsRawStore,
    generation: &str,
    join: &ase_task::DispatchJoin,
    omp_root: &str,
    out_path: Option<&str>,
) -> Result<(JournalProjection, ase_work::DeltaAttemptRef, ase_work::ReopenReport)> {
    let file_id = join.child_file_id;
    let view = idx.file(file_id)?;
    let bytes = store.bytes(file_id)?;
    let sha = ase_work::sha256_hex(&bytes);
    let rel = ase_work::relpath(view.path(), omp_root)
        .unwrap_or_else(|| ase_work::util::slash_path(view.path()));
    let sid = ase_work::source_id(&rel);
    let options = ase_work::ProjectOptions {
        file_id,
        path: view.path(),
        source_id: &sid,
        generation,
        sha256: &sha,
        event_base: view.event_start(),
        explicit_leaf: None,
        window: None,
    };
    let projection = ase_work::project_bytes(&options, &bytes)?;
    validate_work_projection(idx, &projection)?;
    let reopen = ase_work::reopen_bytes(&projection, &bytes, generation);
    let membership = ase_work::DeltaAttemptRef {
        session_id: join.child_session_id.clone(),
        dispatch_id: join.dispatch_id.clone(),
        project: join.project.clone(),
        task: join.task.clone(),
        attempt: join.attempt.clone(),
        parent_session_id: join.parent_session_id.clone(),
        parent_span: join.parent_span.clone(),
        child_span: join.child_span.clone(),
    };
    if let Some(path) = out_path {
        let content = serde_json::to_vec_pretty(&projection)
            .map_err(|e| err(format!("serialize attempt projection: {e}")))?;
        fs::write(path, content).map_err(|e| err(format!("write projection {path}: {e}")))?;
    }
    Ok((projection, membership, reopen))
}


fn cmd_work_delta(args: &[String]) -> Result<()> {
    let task = opt(args, "--task")
        .ok_or_else(|| err("work delta needs --task TOKEN"))?;
    let project = opt(args, "--project");
    let before_session = opt(args, "--before")
        .ok_or_else(|| err("work delta needs --before CHILD_SESSION_ID"))?;
    let after_session = opt(args, "--after")
        .ok_or_else(|| err("work delta needs --after CHILD_SESSION_ID"))?;
    if before_session == after_session {
        work_delta_reject(
            args, &task, project.as_deref(), &before_session, &after_session,
            "same_attempt", "before and after name the same executing child session",
            None, json!({}),
        );
    }
    let (idx, store, generation) = load_work(args)?;
    let dispatch = dispatch_input(&idx, &store)?;
    let resolution = ase_task::resolve(&ase_task::Input {
        token: task.clone(),
        project: project.clone(),
        naming: Vec::new(),
        markers: Vec::new(),
        delegations: Vec::new(),
        paths: Vec::new(),
        dispatch: dispatch.clone(),
        marker_layer: ase_task::MarkerLayer {
            generation: None,
            unavailable: Some(
                "attempt delta membership is derived from native dispatch records only".to_string(),
            ),
            blocks_total: 0,
            blocks_scanned: 0,
            excluded_role_blocks: 0,
            unverified_blocks: 0,
        },
    });
    if resolution.status != TaskStatus::Assigned {
        let reason = if resolution.status == TaskStatus::Ambiguous {
            "ambiguous_membership"
        } else {
            "unconfirmed_membership"
        };
        let detail = if reason == "ambiguous_membership" {
            format!("task token {task} has more than one project-scoped dispatch key; pass --project to select one")
        } else {
            format!("task token {task} has no unique admitted dispatch membership")
        };
        work_delta_reject(
            args, &task, project.as_deref(), &before_session, &after_session,
            reason, &detail, Some(&resolution), json!({}),
        );
    }

    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let joins: Vec<&ase_task::DispatchJoin> = resolution
        .evidence
        .iter()
        .filter_map(|evidence| match evidence {
            TaskEvidence::Dispatch(join)
                if join.task == task
                    && project.as_deref().is_none_or(|scope| scope == join.project) =>
            {
                let key = (join.dispatch_id.clone(), join.child_session_id.clone());
                if seen.insert(key) { Some(join) } else { None }
            }
            _ => None,
        })
        .collect();
    let before_join = joins.iter().find(|join| join.child_session_id == before_session).copied();
    let after_join = joins.iter().find(|join| join.child_session_id == after_session).copied();
    if before_join.is_none() || after_join.is_none() {
        let empty: Vec<Value> = Vec::new();
        let claims = json!({
            "before": delta_observed_claims(&dispatch, &before_session),
            "after": delta_observed_claims(&dispatch, &after_session)
        });
        let mismatch = claims
            .get("before")
            .and_then(Value::as_array)
            .unwrap_or(&empty)
            .iter()
            .chain(
                claims
                    .get("after")
                    .and_then(Value::as_array)
                    .unwrap_or(&empty)
                    .iter(),
            )
            .any(|claim| {
                claim["task"].as_str() != Some(task.as_str())
                    || project
                        .as_deref()
                        .is_some_and(|scope| claim["project"].as_str() != Some(scope))
            });
        let (reason, detail) = if mismatch {
            (
                "task_mismatch",
                "a submitted session has an observed dispatch claim for another task or project; it is not membership for this task key",
            )
        } else {
            (
                "unconfirmed_membership",
                "a submitted session has no admitted dispatch join for this project/task key; candidates and prose mentions are not membership",
            )
        };
        work_delta_reject(
            args, &task, project.as_deref(), &before_session, &after_session,
            reason, detail, Some(&resolution), claims,
        );
    }
    let before_join = before_join.expect("checked above");
    let after_join = after_join.expect("checked above");

    let mut keys = Vec::with_capacity(joins.len());
    for join in &joins {
        let ts_ms = idx.event(join.parent_event)?.ts_ms();
        keys.push(ase_work::AttemptOrderKey {
            session_id: join.child_session_id.clone(),
            dispatch_id: join.dispatch_id.clone(),
            attempt: join.attempt.clone(),
            dispatch_timestamp: Some(ase_work::format_iso_ms(ts_ms)),
            dispatch_ts_ms: ts_ms,
            spawn_index: join.spawn_index,
        });
    }
    let order = ase_work::order_attempts(&keys, &before_session, &after_session)?;
    let omp_root = opt(args, "--omp-root").unwrap_or_else(|| config::omp_root());
    let before_out = opt(args, "--before-out");
    let after_out = opt(args, "--after-out");
    let (before_projection, before_membership, before_reopen) = project_delta_attempt(
        &idx, &store, &generation, before_join, &omp_root, before_out.as_deref(),
    )?;
    let (after_projection, after_membership, after_reopen) = project_delta_attempt(
        &idx, &store, &generation, after_join, &omp_root, after_out.as_deref(),
    )?;
    let claims = json!({
        "before": delta_observed_claims(&dispatch, &before_session),
        "after": delta_observed_claims(&dispatch, &after_session)
    });
    if before_reopen.status != "verified" || after_reopen.status != "verified" {
        work_delta_reject(
            args, &task, project.as_deref(), &before_session, &after_session,
            "evidence_unverified", "an attempt projection did not reopen against the pinned index and raw bytes",
            Some(&resolution), claims,
        );
    }
    let delta = ase_work::attempt_delta(
        &before_projection,
        &after_projection,
        before_membership,
        after_membership,
        order,
    );
    let index_label = index_path(args).to_string_lossy().to_string();
    let mut value = serde_json::to_value(&delta)
        .map_err(|e| err(format!("render attempt delta: {e}")))?;
    let report = value
        .as_object_mut()
        .ok_or_else(|| err("attempt delta did not render as an object"))?;
    report.insert("command".to_string(), json!("work delta"));
    report.insert("status".to_string(), json!("ok"));
    report.insert("omp_root".to_string(), json!(omp_root));
    report.insert("observed_claims".to_string(), claims);
    report.insert(
        "evidence_reopen".to_string(),
        json!({
            "before": before_reopen,
            "after": after_reopen,
            "projection_files": { "before": before_out, "after": after_out }
        }),
    );
    report.insert(
        "reopen_commands".to_string(),
        json!({
            "before": before_out.as_ref().map(|p| format!("ase work reopen {p} --index {index_label}")),
            "after": after_out.as_ref().map(|p| format!("ase work reopen {p} --index {index_label}"))
        }),
    );
    report.insert(
        "guarantees".to_string(),
        json!([
            "Membership comes only from admitted task-dispatch-provenance / dispatch-v1 joins; adapter claims, motif membership, naming, markers, path hits and parentSession edges are never membership inputs.",
            "Both sides are executing child sessions of one confirmed project/task key; a cross-project same-token attempt, an unrelated task and an unconfirmed candidate are rejected before any delta is computed.",
            "Ordering is dispatch parent-record time with spawn_index/dispatch_id/session tie-breaks; attempt labels are labels, not chronology, and label agreement is reported explicitly.",
            "Counts are after-minus-before; turns and boundaries are keyed by entry id plus pinned evidence and sorted by change kind, ordinal and id.",
            "Both projections reopen against the same index generation and raw bytes; any mismatch rejects the delta rather than silently repairing it."
        ]),
    );
    if let Some(out) = opt(args, "--out") {
        let content = serde_json::to_vec_pretty(&value)
            .map_err(|e| err(format!("serialize attempt delta: {e}")))?;
        fs::write(&out, content).map_err(|e| err(format!("write attempt delta {out}: {e}")))?;
    }
    emit(&value, args);
    Ok(())
}
