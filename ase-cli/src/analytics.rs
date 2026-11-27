//! `ase analytics` — runtime cost, duration and coverage reporting.
//!
//! Owns the command router, period selection and projection of journal turns
//! into analytics rows. Shared projection helpers live with the work command;
//! synthetic fixtures stay beside the rule they exercise.

use ase_core::{err, RawStore, Result};
use ase_index::Index;
use ase_work::{aggregate, reconciled, JournalAnalytics};
use rayon::prelude::*;
use serde_json::json;
use std::fs;
use crate::config;
use crate::journal_projection::{
    load_work, parse_work_window, project_work_file, read_jsonl, validate_work_projection,
    ProjectedWork, SelectedWork, WorkFile,
};
use crate::cli_option::{has, opt, positional};
use crate::cli_output::emit;
use crate::show::unique_session;

// -------------------------------------------------------- runtime analytics

fn analytics_usage() {
    println!(
        "ase analytics project [--index FILE] [--omp-root DIR] [--start ISO --end ISO]\n\
         [--out ROWS.jsonl] [--report REPORT.json] [--limit N] [--json]\n\
         ase analytics session (--session ID | --file N) [--leaf ENTRY_ID]\n\
         [--omp-root DIR] [--out ROW.json] [--json]\n\
         ase analytics reopen ROWS.jsonl [--index FILE] [--json]\n\
         ase fixture-analytics --out DIR"
    );
}

pub fn run(args: &[String]) -> Result<()> {
    let Some(sub) = args.first().map(String::as_str) else {
        analytics_usage();
        return Err(err("analytics needs project, session or reopen"));
    };
    let rest = &args[1..];
    if matches!(sub, "--help" | "-h") {
        analytics_usage();
        return Ok(());
    }
    match sub {
        "project" => cmd_analytics_project(rest),
        "session" => cmd_analytics_session(rest),
        "reopen" => cmd_analytics_reopen(rest),
        other => Err(err(format!("unknown analytics command: {other}"))),
    }
}


/// Project provenance is the first path segment of the locked tree layout,
/// never a model name, session title or adapter candidate. Absence stays an
/// explicit unknown group in the analytics layer.
fn analytics_project_of_tree(tree_relpath: &str) -> Option<String> {
    tree_relpath
        .split('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
}


/// Analytics window selection follows the contracted `message.timestamp`
/// period field, not the session-header event time. A root journal that began
/// before the window can carry in-window turns, so a scoped analytics run
/// projects every root (and its subagents) with complete branch context;
/// `aggregate` builds its `period` totals from in-window turns only.
/// `ase work project` keeps its separate header-time selection contract.
fn collect_analytics_files(
    idx: &Index,
    omp_root: &str,
    limit: Option<usize>,
) -> Result<SelectedWork> {
    let mut roots: std::collections::BTreeMap<String, (u32, ase_work::util::Layout)> =
        std::collections::BTreeMap::new();
    let mut subagents: Vec<WorkFile> = Vec::new();
    let mut scanned = 0u64;
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
            roots.insert(layout.tree_relpath.clone(), (file_id, layout));
        } else {
            subagents.push(WorkFile { file_id, rel, layout });
        }
    }
    let mut files: Vec<WorkFile> = roots
        .values()
        .map(|(file_id, layout)| WorkFile {
            file_id: *file_id,
            rel: layout.tree_relpath.clone(),
            layout: layout.clone(),
        })
        .collect();
    for sub in subagents {
        if roots.contains_key(&sub.layout.tree_relpath) {
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
        root_journals_selected: scanned,
        root_journals_outside_window: 0,
        root_journals_missing_header: 0,
    })
}


fn cmd_analytics_project(args: &[String]) -> Result<()> {
    let (idx, store, generation) = load_work(args)?;
    let window = parse_work_window(args)?;
    let omp_root = opt(args, "--omp-root").unwrap_or_else(|| config::omp_root());
    let limit = opt(args, "--limit").and_then(|s| s.parse::<usize>().ok());
    let selected = collect_analytics_files(&idx, &omp_root, limit)?;
    let projected: Vec<ProjectedWork> = selected
        .files
        .par_iter()
        .map(|file| project_work_file(&idx, &store, file, &generation, window))
        .collect::<Result<Vec<_>>>()?;
    let rows: Vec<JournalAnalytics> = projected
        .iter()
        .zip(selected.files.iter())
        .map(|(item, file)| {
            JournalAnalytics::new(
                item.projection.clone(),
                analytics_project_of_tree(&file.layout.tree_relpath),
            )
        })
        .collect();
    let report = aggregate(&rows);
    let ok = reconciled(&report);

    if let Some(out) = opt(args, "--out") {
        let mut content = String::new();
        for row in &rows {
            content.push_str(
                &serde_json::to_string(row)
                    .map_err(|e| err(format!("serialize analytics row: {e}")))?,
            );
            content.push('\n');
        }
        fs::write(&out, content).map_err(|e| err(format!("write analytics rows {out}: {e}")))?;
    }
    if let Some(out) = opt(args, "--report") {
        let content = serde_json::to_vec_pretty(&report)
            .map_err(|e| err(format!("serialize analytics report: {e}")))?;
        fs::write(&out, content).map_err(|e| err(format!("write analytics report {out}: {e}")))?;
    }

    let mut value = serde_json::to_value(&report)
        .map_err(|e| err(format!("render analytics report: {e}")))?;
    value["command"] = json!("analytics project");
    value["status"] = json!(if ok { "ok" } else { "reconciliation_failed" });
    value["limited"] = json!(limit.is_some());
    value["selection"] = json!({
        "scanned": selected.root_journals_scanned,
        "selected": selected.root_journals_selected,
        "outside_window": selected.root_journals_outside_window,
        "missing_header": selected.root_journals_missing_header,
        "root_selection": "all-root-journals; period totals filtered on message.timestamp"
    });
    if !has(args, "--json") {
        value["buckets"] = json!([]);
    }
    emit(&value, args);
    if !ok {
        std::process::exit(3);
    }
    Ok(())
}


fn cmd_analytics_session(args: &[String]) -> Result<()> {
    let (idx, store, generation) = load_work(args)?;
    let omp_root = opt(args, "--omp-root").unwrap_or_else(|| config::omp_root());
    let file_id = if let Some(session) = opt(args, "--session") {
        let session_idx = unique_session(&idx, &session)?;
        idx.session(session_idx)?.file()
    } else if let Some(file) = opt(args, "--file").and_then(|s| s.parse::<u32>().ok()) {
        file
    } else {
        return Err(err("analytics session needs --session ID or --file N"));
    };
    let view = idx.file(file_id)?;
    let rel_opt = ase_work::relpath(view.path(), &omp_root);
    let rel = rel_opt
        .clone()
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
    let project = rel_opt.as_deref().and_then(analytics_project_of_tree);
    let row = JournalAnalytics::new(projection, project);
    if let Some(out) = opt(args, "--out") {
        let content = serde_json::to_vec_pretty(&row)
            .map_err(|e| err(format!("serialize analytics row: {e}")))?;
        fs::write(&out, content).map_err(|e| err(format!("write analytics row {out}: {e}")))?;
    }
    let report = aggregate(&[row.clone()]);
    let mut value = serde_json::to_value(&row)
        .map_err(|e| err(format!("render analytics row: {e}")))?;
    value["command"] = json!("analytics session");
    value["reconciliation"] = serde_json::to_value(&report.reconciliation)
        .map_err(|e| err(format!("render reconciliation: {e}")))?;
    emit(&value, args);
    Ok(())
}


fn cmd_analytics_reopen(args: &[String]) -> Result<()> {
    let path = positional(args)
        .or_else(|| opt(args, "--projection"))
        .ok_or_else(|| err("analytics reopen needs ROWS.jsonl"))?;
    let rows: Vec<JournalAnalytics> = read_jsonl(&path)?;
    let (idx, store, generation) = load_work(args)?;
    let mut checked = 0u64;
    let mut verified = 0u64;
    let mut stale = 0u64;
    let mut evidence_checked = 0u64;
    let mut evidence_verified = 0u64;
    let mut runtime_checked = 0u64;
    let mut runtime_verified = 0u64;
    let mut failures: Vec<String> = Vec::new();
    for row in &rows {
        checked += 1;
        let projection = &row.projection;
        let view = idx.file(projection.file_id)?;
        let path_ok = view.path() == projection.path;
        let raw = store.bytes(projection.file_id)?;
        let report = if path_ok {
            ase_work::reopen_bytes(projection, &raw, &generation)
        } else {
            let mut report = ase_work::ReopenReport {
                status: "stale".to_string(),
                generation: generation.clone(),
                file_id: projection.file_id,
                source_id: projection.source_id.clone(),
                ..ase_work::ReopenReport::default()
            };
            report
                .errors
                .push("file_mapping_mismatch:index maps this file id to another path".to_string());
            report
        };
        evidence_checked += report.evidence_checked as u64;
        evidence_verified += report.evidence_verified as u64;
        runtime_checked += report.runtime_checked as u64;
        runtime_verified += report.runtime_verified as u64;
        if report.status == "verified" {
            verified += 1;
        } else {
            stale += 1;
            failures.push(format!(
                "{}:{}:{}",
                row.source_id,
                report.status,
                report.errors.join(";")
            ));
        }
    }
    let value = json!({
        "command": "analytics reopen",
        "status": if stale == 0 { "verified" } else { "stale" },
        "generation": generation,
        "files_checked": checked,
        "files_verified": verified,
        "files_stale": stale,
        "evidence_checked": evidence_checked,
        "evidence_verified": evidence_verified,
        "runtime_checked": runtime_checked,
        "runtime_verified": runtime_verified,
        "failures_shown": failures.len().min(20),
        "failures": &failures[..failures.len().min(20)]
    });
    emit(&value, args);
    if stale > 0 {
        std::process::exit(3);
    }
    Ok(())
}
