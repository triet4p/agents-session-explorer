//! OMP stock claim derivation.
//!
//! The adapter reads source journals through `StockView`. It emits claims only;
//! nothing in this module writes to an index, resolver input or motif
//! membership.

use ase_core::{FileId, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::claims::{add_repo_claims, task_claims_for_call};
use crate::journal::{LoadedFile, load_headers, parse_parent_file, resolve_parent_indexes};
use crate::link::link_claims_for_call;
use crate::model::*;

#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Bound the number of parent journals selected after path sort. This is
    /// not a claim cap: one parent can emit several claims.
    pub max_parent_files: Option<usize>,
    /// Read a child handoff `.md` for independent task-token corroboration.
    pub read_sidecars: bool,
}

pub fn scan(view: &dyn StockView, options: &Options) -> Result<Report> {
    let generation = view.generation().to_string();
    let (mut loaded, mut errors) = load_headers(view);
    resolve_parent_indexes(&mut loaded);

    let mut parents: Vec<usize> = loaded
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.is_child)
        .map(|(i, _)| i)
        .collect();
    parents.sort_by(|a, b| loaded[*a].file.path.cmp(&loaded[*b].file.path));
    if let Some(limit) = options.max_parent_files {
        parents.truncate(limit);
    }

    let mut claims: Vec<Claim> = Vec::new();
    let mut link = layer_summary(LINK_RULE, LINK_VERSION, "delegated_from");
    let mut repo = layer_summary(REPO_RULE, REPO_VERSION, "workspace");
    let mut task = layer_summary(TASK_RULE, TASK_VERSION, "task_claim");
    let mut scan_counts = ScanCounts {
        files: loaded.len(),
        parent_files: parents.len(),
        child_files: loaded.iter().filter(|f| f.is_child).count(),
        sessions: loaded.iter().filter(|f| f.header.is_some()).count(),
        ..ScanCounts::default()
    };

    let mut seen_children: HashSet<FileId> = HashSet::new();

    for idx in &parents {
        let parent = &loaded[*idx];
        let parsed = match parse_parent_file(view, &parent.file, &generation) {
            Ok(p) => p,
            Err(e) => {
                errors.push(ScanError {
                    code: "source_missing".to_string(),
                    path: Some(parent.file.path.clone()),
                    detail: e.to_string(),
                });
                continue;
            }
        };
        scan_counts.events_read += view.events(parent.file.id).map(|e| e.len()).unwrap_or(0);
        scan_counts.task_calls += parsed.calls.len();
        let parent_session = parent
            .header
            .as_ref()
            .and_then(|h| h.session_id.clone());

        // ---------------------------------------------------------- repo layer
        add_repo_claims(
            &mut claims,
            &mut repo,
            &parent.file,
            parent.header.as_ref(),
            parent.header_event.as_ref(),
            &parent.header_line,
            &generation,
            &parsed.reminder_cwds,
            &parsed.path_clues,
        );

        // ------------------------------------------------------ task/link layer
        for call in &parsed.calls {
            let mut link_claims = link_claims_for_call(
                &loaded,
                parent,
                call,
                &generation,
                &mut seen_children,
            );
            let mut task_claims = task_claims_for_call(
                view,
                &loaded,
                parent,
                call,
                &generation,
                options,
                &parsed,
                &mut scan_counts,
                &mut errors,
            );
            claims.append(&mut link_claims);
            claims.append(&mut task_claims);
        }

        // Parent-session-only task mentions (prose that is not tied to a call)
        // remain candidates and never become an assignment.
        for clue in &parsed.prose_tokens {
            let state = STATE_CANDIDATE;
            claims.push(
                Claim::new(
                    LAYER_TASK,
                    state,
                    "task_claim",
                    json!({ "kind": "task_token", "token": clue.token, "source": clue.source_field }),
                    Subject::for_file(&parent.file, parent_session.as_deref()),
                    "derived",
                    TASK_RULE,
                    TASK_VERSION,
                    vec![clue.evidence.clone()],
                )
                .reason("prose mention without a task-call scope; mention-only never supports assignment")
                .context("parent-session prose token"),
            );
        }
    }

    // Repo claims for child files that were not already covered by a parent
    // scan (their workspace is still a useful independent observation).
    for f in &loaded {
        if f.is_child {
            add_repo_claims(
                &mut claims,
                &mut repo,
                &f.file,
                f.header.as_ref(),
                f.header_event.as_ref(),
                &f.header_line,
                &generation,
                &[],
                &[],
            );
        }
    }

    // Remove duplicate claims produced by overlapping scopes. Claim ids are
    // deterministic, so deduping is stable and order-independent.
    let mut seen_ids = HashSet::new();
    claims.retain(|c| seen_ids.insert(c.claim_id.clone()));

    add_counts(&mut link, &claims.iter().filter(|c| c.layer == LAYER_LINK).cloned().collect::<Vec<_>>());
    add_counts(&mut repo, &claims.iter().filter(|c| c.layer == LAYER_REPO).cloned().collect::<Vec<_>>());
    add_counts(&mut task, &claims.iter().filter(|c| c.layer == LAYER_TASK).cloned().collect::<Vec<_>>());
    link.scopes_with_signal = distinct_scopes(&claims, LAYER_LINK);
    repo.scopes_with_signal = distinct_scopes(&claims, LAYER_REPO);
    task.scopes_with_signal = distinct_scopes(&claims, LAYER_TASK);

    finalize_layer_sessions(&mut task, &claims, &loaded);
    link.notes.push("confirmed requires one task call record, one result naming exactly one child, and a child header parentSession matching the parent journal; multi-child spawn summaries stay candidates and are never merged.".to_string());
    repo.notes.push("workspace is confirmed from one unique explicit cwd after case/separator normalization; repo_key is derived and stays candidate.".to_string());
    task.notes.push("child_label confirmed requires the structured parent task name plus the independent child filename; prose task tokens are assignment only inside the task-call prompt and become confirmed only when the child handoff agrees.".to_string());
    link.scopes_missing = distinct_state_scopes(&claims, LAYER_LINK, STATE_MISSING);
    repo.scopes_missing = distinct_state_scopes(&claims, LAYER_REPO, STATE_MISSING);
    task.scopes_missing = distinct_state_scopes(&claims, LAYER_TASK, STATE_MISSING);

    let mut layers = BTreeMap::new();
    layers.insert(LAYER_LINK.to_string(), link);
    layers.insert(LAYER_REPO.to_string(), repo);
    layers.insert(LAYER_TASK.to_string(), task);

    Ok(Report {
        adapter: ADAPTER_NAME.to_string(),
        generation,
        evidence_selector_version: SELECTOR_VERSION.to_string(),
        claim_policy: "adapter claims only; no resolver assignment or motif membership is written".to_string(),
        scan: scan_counts,
        layers,
        claims,
        errors,
        notes: vec![
            "parent_child, repo and task are independent layers; one layer never promotes another.".to_string(),
            "missing means the searched source had no signal; unavailable means the needed layer/generation was absent or unreadable.".to_string(),
            "candidate and conflict claims are kept and are not counted as confirmed.".to_string(),
        ],
    })
}

fn distinct_state_scopes(claims: &[Claim], layer: &str, state: &str) -> usize {
    let mut set = HashSet::new();
    for c in claims
        .iter()
        .filter(|c| c.layer == layer && c.state == state)
    {
        set.insert((
            c.subject.file_id,
            if layer == LAYER_REPO {
                None
            } else {
                c.subject.call_id.clone()
            },
        ));
    }
    set.len()
}

fn distinct_scopes(claims: &[Claim], layer: &str) -> usize {
    let mut set = HashSet::new();
    for c in claims.iter().filter(|c| c.layer == layer) {
        set.insert((
            c.subject.file_id,
            if layer == LAYER_REPO {
                None
            } else {
                c.subject.call_id.clone()
            },
        ));
    }
    set.len()
}

fn add_counts(summary: &mut LayerSummary, claims: &[Claim]) {
    for c in claims {
        summary.counts.add(&c.state);
    }
}

fn layer_summary(rule: &str, version: &str, predicate: &str) -> LayerSummary {
    LayerSummary {
        rule: rule.to_string(),
        rule_version: version.to_string(),
        predicate: predicate.to_string(),
        counts: StateCounts::default(),
        scopes_with_signal: 0,
        scopes_missing: 0,
        session_states: BTreeMap::new(),
        notes: Vec::new(),
    }
}

fn finalize_layer_sessions(summary: &mut LayerSummary, claims: &[Claim], loaded: &[LoadedFile]) {
    let mut by_session: BTreeMap<Option<String>, Vec<&Claim>> = BTreeMap::new();
    for c in claims.iter().filter(|c| c.layer == LAYER_TASK) {
        by_session.entry(c.subject.session_id.clone()).or_default().push(c);
    }
    for (session, list) in by_session {
        if session.is_none() {
            continue;
        }
        let conflict = list.iter().any(|c| c.state == STATE_CONFLICT);
        let confirmed_labels: BTreeSet<(String, Option<String>)> = list
            .iter()
            .filter(|c| {
                c.state == STATE_CONFIRMED
                    && c.value.get("kind").and_then(Value::as_str) == Some("child_label")
            })
            .filter_map(|c| {
                let label = c.value.get("label").and_then(Value::as_str)?;
                Some((label.to_string(), c.subject.call_id.clone()))
            })
            .collect();
        let distinct_calls: BTreeSet<String> = confirmed_labels
            .iter()
            .filter_map(|(_, call)| call.clone())
            .collect();
        let state = if conflict {
            STATE_CONFLICT
        } else if distinct_calls.len() > 1 {
            STATE_MULTI_TASK
        } else if !confirmed_labels.is_empty() {
            STATE_CONFIRMED
        } else if list.iter().any(|c| c.state == STATE_CANDIDATE) {
            STATE_CANDIDATE
        } else {
            STATE_MISSING
        };
        *summary.session_states.entry(state.to_string()).or_default() += 1;
    }
    // Report how many selected files actually carried a parsed session header.
    summary.notes.push(format!(
        "session headers parsed: {}",
        loaded.iter().filter(|f| f.header.is_some()).count()
    ));
}
