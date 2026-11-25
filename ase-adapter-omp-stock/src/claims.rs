//! Repository and task claim construction for OMP stock scans.
//!
//! Claim states, values, reasons, subjects and ordering are decided here from
//! the parsed journal and parent-child linkage; provenance bytes come from
//! `provenance`.

use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use crate::extract::{SpawnHit, extract_task_id_markers, extract_task_tokens};
use crate::journal::{
    Assignment, CallRecord, LoadedFile, ParsedParent, TokenClue, child_file_by_name,
    child_stem, read_handoff_tokens,
};
use crate::model::*;
use crate::provenance::{assignment_token_selector, make_evidence, path_evidence, retarget};
use crate::scan::Options;

pub(crate) fn add_repo_claims(
    claims: &mut Vec<Claim>,
    summary: &mut LayerSummary,
    file: &ViewFile,
    header: Option<&Header>,
    header_event: Option<&ViewEvent>,
    header_line: &[u8],
    generation: &str,
    reminders: &[(String, EvidenceRef)],
    path_clues: &[String],
) {
    let session_id = header.and_then(|h| h.session_id.clone());
    let mut observations: Vec<(String, Vec<EvidenceRef>)> = Vec::new();
    if let Some(cwd) = header.and_then(|h| h.cwd.clone()) {
        let ev = match header_event {
            Some(hev) => make_evidence(
                file,
                generation,
                hev,
                &selector_session("cwd"),
                header_line,
                cwd.as_bytes(),
            ),
            None => path_evidence(file, generation, "cwd"),
        };
        observations.push((cwd, vec![ev]));
    }
    for (cwd, ev) in reminders {
        if let Some((_, evidence)) = observations.iter_mut().find(|(v, _)| path_eq(v, cwd)) {
            evidence.push(ev.clone());
        } else {
            observations.push((cwd.clone(), vec![ev.clone()]));
        }
    }

    if observations.is_empty() {
        let claim = Claim::new(
            LAYER_REPO,
            STATE_MISSING,
            "workspace",
            Value::Null,
            Subject::for_file(file, session_id.as_deref()),
            "observed",
            REPO_RULE,
            REPO_VERSION,
            Vec::new(),
        )
        .reason("no explicit cwd in the stock session header or reminders");
        claims.push(claim);
        summary.scopes_with_signal += 1;
        return;
    }

    if observations.len() == 1 {
        let (cwd, evidence) = &observations[0];
        let repo_key = repo_key_from_cwd(cwd);
        let mut under_paths: Vec<String> = Vec::new();
        for clue in path_clues {
            if path_under(clue, cwd) && !path_eq(clue, cwd) {
                let normalized = normalize_path(clue);
                if !under_paths.iter().any(|p| path_eq(p, &normalized)) {
                    under_paths.push(normalized);
                }
            }
        }
        let under = under_paths.len();
        let shown: Vec<String> = under_paths.iter().take(8).cloned().collect();
        claims.push(
            Claim::new(
                LAYER_REPO,
                STATE_CONFIRMED,
                "workspace",
                json!({
                    "cwd": normalize_path(cwd),
                    "repo_key": repo_key,
                    "path_clue_count": under,
                    "path_clues": shown
                }),
                Subject::for_file(file, session_id.as_deref()),
                "observed",
                REPO_RULE,
                REPO_VERSION,
                evidence.clone(),
            )
            .reason("one explicit, unique cwd after case/separator normalization")
            .context(format!("independent path mentions under the workspace: {under}")),
        );
        if let Some(key) = repo_key {
            let mut ev = evidence.clone();
            ev.push(path_evidence(file, generation, "workspace"));
            claims.push(
                Claim::new(
                    LAYER_REPO,
                    STATE_CANDIDATE,
                    "repo_key",
                    json!({ "repo_key": key, "cwd": normalize_path(cwd) }),
                    Subject::for_file(file, session_id.as_deref()),
                    "derived",
                    REPO_RULE,
                    REPO_VERSION,
                    ev,
                )
                .reason("repo key is derived from the cwd basename; a path mention is not an independent repo marker")
                .context("repo_key never confirms the workspace by itself"),
            );
        }
        summary.scopes_with_signal += 1;
    } else {
        let values: Vec<String> = observations.iter().map(|(v, _)| normalize_path(v)).collect();
        for (cwd, evidence) in &observations {
            let idx = observations
                .iter()
                .position(|(v, _)| path_eq(v, cwd))
                .unwrap_or(0);
            claims.push(
                Claim::new(
                    LAYER_REPO,
                    STATE_CONFLICT,
                    "workspace",
                    json!({ "cwd": normalize_path(cwd) }),
                    Subject::for_file(file, session_id.as_deref()),
                    "observed",
                    REPO_RULE,
                    REPO_VERSION,
                    evidence.clone(),
                )
                .reason("more than one distinct cwd observation in this file")
                .refute(format!("other observed cwd values: {values:?}"))
                .context(format!("observation {idx} kept; no ordering or mtime tie-break")),
            );
        }
        summary.scopes_with_signal += 1;
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn task_claims_for_call(
    view: &dyn StockView,
    loaded: &[LoadedFile],
    parent: &LoadedFile,
    call: &CallRecord,
    generation: &str,
    options: &Options,
    _parsed: &ParsedParent,
    scan_counts: &mut ScanCounts,
    errors: &mut Vec<ScanError>,
) -> Vec<Claim> {
    let session_id = parent.header.as_ref().and_then(|h| h.session_id.clone());
    let event = call
        .args_event
        .as_ref()
        .map(|e| e.event)
        .or_else(|| call.start_event.as_ref().map(|e| e.event));
    let base_subject = Subject::for_call(
        &parent.file,
        session_id.as_deref(),
        event,
        Some(&call.call_id),
    );
    let mut claims = Vec::new();

    // One task call may dispatch several sibling children. Each child is its
    // own scope; siblings are never merged and never treated as a conflict.
    let spawn_children: Vec<(&SpawnHit, Option<&LoadedFile>)> = call
        .spawns
        .iter()
        .filter(|(s, _)| s.shape != "failed")
        .map(|(s, _)| (s, child_file_by_name(loaded, &parent.file, &s.name)))
        .collect();
    let single_child = if spawn_children.len() == 1 {
        spawn_children[0].1
    } else {
        None
    };

    let mut clues: Vec<TokenClue> = Vec::new();
    for assignment in &call.assignments {
        if let Some(prompt) = &assignment.prompt {
            let block_ev = assignment.prompt_evidence.clone();
            for hit in extract_task_tokens(prompt) {
                if let Some(ev) = &block_ev {
                    clues.push(TokenClue {
                        token: hit.token.clone(),
                        source_field: format!(
                            "task_argument.task[{}/{}]",
                            call.call_id, assignment.index
                        ),
                        source_kind: "assignment".to_string(),
                        strong: hit.offset < 400,
                        evidence: retarget(
                            ev,
                            assignment_token_selector(assignment.source, &hit.token),
                            hit.token.as_bytes(),
                        ),
                    });
                }
            }
            for hit in extract_task_id_markers(prompt) {
                if let Some(ev) = &block_ev {
                    clues.push(TokenClue {
                        token: hit.token.clone(),
                        source_field: "task_argument.task_id_marker".to_string(),
                        source_kind: "assignment".to_string(),
                        strong: true,
                        evidence: retarget(
                            ev,
                            assignment_token_selector(assignment.source, &hit.token),
                            hit.token.as_bytes(),
                        ),
                    });
                }
            }
        }
        if let Some(solution) = &assignment.solution_space {
            let ev = assignment.solution_evidence.clone();
            for hit in extract_task_tokens(solution) {
                if let Some(ev) = &ev {
                    clues.push(TokenClue {
                        token: hit.token.clone(),
                        source_field: "task_argument.solutionSpace".to_string(),
                        source_kind: "mention".to_string(),
                        strong: false,
                        evidence: retarget(
                            ev,
                            assignment_token_selector(assignment.source, &hit.token),
                            hit.token.as_bytes(),
                        ),
                    });
                }
            }
        }
    }
    for clue in &call.context_tokens {
        clues.push(clue.clone());
    }
    if let Some(intent) = &call.intent {
        if let Some(ev) = &call.intent_evidence {
            for hit in extract_task_tokens(intent) {
                clues.push(TokenClue {
                    token: hit.token,
                    source_field: "task_call.intent".to_string(),
                    source_kind: "mention".to_string(),
                    strong: false,
                    evidence: ev.clone(),
                });
            }
        }
    }

    // Read one handoff only when the call has one child. With siblings, a
    // token from one handoff cannot be attributed to the call scope safely.
    if let Some(child) = single_child {
        if options.read_sidecars {
            let (tokens, error) = read_handoff_tokens(view, &child.file, generation);
            if let Some(e) = error {
                errors.push(e);
            }
            if !tokens.is_empty() {
                scan_counts.sidecars_read += 1;
                clues.extend(tokens);
            }
        }
    }

    // ------------------------------------------------ child-label claims
    let mut label_claims = 0usize;
    let mut matched_parent_names: HashSet<String> = HashSet::new();
    for (spawn, child_opt) in &spawn_children {
        let name = &spawn.name;
        let matching: Vec<&Assignment> = call
            .assignments
            .iter()
            .filter(|a| a.name.as_deref() == Some(name.as_str()))
            .collect();
        match child_opt {
            Some(child) => {
                let stem = child_stem(&child.file.path);
                let filename_ev = path_evidence(&child.file, generation, "filename");
                let header_ev = child.header_event.as_ref().map(|hev| {
                    make_evidence(
                        &child.file,
                        generation,
                        hev,
                        &selector_session("id"),
                        &child.header_line,
                        child
                            .header
                            .as_ref()
                            .and_then(|h| h.session_id.clone())
                            .unwrap_or_default()
                            .as_bytes(),
                    )
                });
                if let Some(assignment) = matching.first() {
                    matched_parent_names.insert(name.clone());
                    let mut evidence = Vec::new();
                    if let Some(ev) = &assignment.name_evidence {
                        evidence.push(ev.clone());
                    }
                    evidence.push(filename_ev);
                    if let Some(ev) = header_ev {
                        evidence.push(ev);
                    }
                    claims.push(
                        Claim::new(
                            LAYER_TASK,
                            STATE_CONFIRMED,
                            "task_claim",
                            json!({
                                "kind": "child_label",
                                "label": stem,
                                "agent": assignment.agent,
                                "attempt": trailing_digits(&stem),
                                "project": Value::Null,
                                "task": Value::Null
                            }),
                            base_subject.clone().with_child(&child.file),
                            "mixed",
                            TASK_RULE,
                            TASK_VERSION,
                            evidence,
                        )
                        .reason(
                            "structured parent task name and child filename agree; two independent sources, one an observed field",
                        ),
                    );
                } else {
                    // A single child with a differing parent name is emitted
                    // below as an explicit conflict pair instead of a bare
                    // filename candidate.
                    let will_conflict = spawn_children.len() == 1
                        && call.assignments.iter().any(|a| {
                            a.name.as_deref().map(|n| n != stem).unwrap_or(false)
                        });
                    if !will_conflict {
                        claims.push(
                            Claim::new(
                                LAYER_TASK,
                                STATE_CANDIDATE,
                                "task_claim",
                                json!({
                                    "kind": "child_label",
                                    "label": stem,
                                    "agent": Value::Null,
                                    "attempt": trailing_digits(&stem),
                                    "project": Value::Null,
                                    "task": Value::Null
                                }),
                                base_subject.clone().with_child(&child.file),
                                "derived",
                                TASK_RULE,
                                TASK_VERSION,
                                {
                                    let mut ev = vec![filename_ev];
                                    if let Some(h) = header_ev {
                                        ev.push(h);
                                    }
                                    ev
                                },
                            )
                            .reason("child filename is name-only evidence; no parent assignment agrees"),
                        );
                        label_claims += 1;
                    }
                }
            }
            None => {
                for assignment in matching {
                    matched_parent_names.insert(name.clone());
                    let mut claim = Claim::new(
                        LAYER_TASK,
                        STATE_CANDIDATE,
                        "task_claim",
                        json!({
                            "kind": "child_label",
                            "label": name,
                            "agent": assignment.agent,
                            "attempt": trailing_digits(name),
                            "project": Value::Null,
                            "task": Value::Null
                        }),
                        base_subject.clone(),
                        "observed",
                        TASK_RULE,
                        TASK_VERSION,
                        assignment.name_evidence.iter().cloned().collect(),
                    )
                    .reason("parent task name has no matching child journal");
                    claim = claim.reason("name-only candidate; never confirmed without a child scope");
                    claims.push(claim);
                    label_claims += 1;
                }
            }
        }
    }

    // A single child whose filename disagrees with every structured parent
    // name is a task conflict: keep both values and do not tie-break.
    if let Some(child) = single_child {
        let stem = child_stem(&child.file.path);
        let mut conflict_parent = false;
        for assignment in &call.assignments {
            let Some(name) = &assignment.name else { continue };
            if name == &stem || matched_parent_names.contains(name) {
                continue;
            }
            conflict_parent = true;
            let filename_ev = path_evidence(&child.file, generation, "filename");
            let mut evidence: Vec<EvidenceRef> = assignment
                .name_evidence
                .iter()
                .cloned()
                .collect();
            evidence.push(filename_ev.clone());
            claims.push(
                Claim::new(
                    LAYER_TASK,
                    STATE_CONFLICT,
                    "task_claim",
                    json!({
                        "kind": "child_label",
                        "label": name,
                        "agent": assignment.agent,
                        "attempt": trailing_digits(name),
                        "project": Value::Null,
                        "task": Value::Null
                    }),
                    base_subject.clone().with_child(&child.file),
                    "mixed",
                    TASK_RULE,
                    TASK_VERSION,
                    evidence,
                )
                .reason("structured parent task name differs from the only child filename")
                .refute(format!("child filename value kept separately: {stem}")),
            );
            label_claims += 1;
        }
        if conflict_parent {
            claims.push(
                Claim::new(
                    LAYER_TASK,
                    STATE_CONFLICT,
                    "task_claim",
                    json!({
                        "kind": "child_label",
                        "label": stem,
                        "agent": Value::Null,
                        "attempt": trailing_digits(&stem),
                        "project": Value::Null,
                        "task": Value::Null
                    }),
                    base_subject.clone().with_child(&child.file),
                    "derived",
                    TASK_RULE,
                    TASK_VERSION,
                    vec![path_evidence(&child.file, generation, "filename")],
                )
                .reason("child filename differs from the structured parent task name")
                .refute("no line order, mtime, path length or suffix tie-break is applied"),
            );
            label_claims += 1;
        }
    }

    // Parent names that never had a child or a spawn stay candidates.
    for assignment in &call.assignments {
        let Some(name) = &assignment.name else { continue };
        if matched_parent_names.contains(name) {
            continue;
        }
        if single_child.is_some() {
            // Already handled above as a conflict pair.
            continue;
        }
        claims.push(
            Claim::new(
                LAYER_TASK,
                STATE_CANDIDATE,
                "task_claim",
                json!({
                    "kind": "child_label",
                    "label": name,
                    "agent": assignment.agent,
                    "attempt": trailing_digits(name),
                    "project": Value::Null,
                    "task": Value::Null
                }),
                base_subject.clone(),
                "observed",
                TASK_RULE,
                TASK_VERSION,
                assignment.name_evidence.iter().cloned().collect(),
            )
            .reason("parent task name has no child scope in this call"),
        );
        label_claims += 1;
    }

    // ------------------------------------------------------- token claims
    let mut by_token: BTreeMap<String, Vec<TokenClue>> = BTreeMap::new();
    for clue in &clues {
        if clue.source_field == "task_argument.name" || clue.source_field == "child_filename" {
            continue;
        }
        by_token.entry(clue.token.clone()).or_default().push(clue.clone());
    }
    for (token, sources) in by_token {
        let assignment = sources.iter().any(|s| s.source_kind == "assignment");
        let assignment_strong = sources
            .iter()
            .any(|s| s.source_kind == "assignment" && s.strong);
        let handoff = sources
            .iter()
            .any(|s| s.source_field.starts_with("child_handoff"));
        let handoff_strong = sources
            .iter()
            .any(|s| s.source_field.starts_with("child_handoff") && s.strong);
        let mention = sources.iter().any(|s| s.source_kind == "mention");
        let multi_scope = spawn_children.len() > 1;
        let state = if !multi_scope
            && assignment_strong
            && handoff_strong
            && single_child.is_some()
        {
            STATE_CONFIRMED
        } else if assignment || handoff || mention {
            STATE_CANDIDATE
        } else {
            continue;
        };
        let evidence: Vec<EvidenceRef> = sources.iter().map(|s| s.evidence.clone()).collect();
        let subject = match single_child {
            Some(child) => base_subject.clone().with_child(&child.file),
            None => base_subject.clone(),
        };
        let mut claim = Claim::new(
            LAYER_TASK,
            state,
            "task_claim",
            json!({
                "kind": "task_token",
                "token": token,
                "token_kind": crate::extract::classify_token(&token),
                "assignment": assignment,
                "assignment_strong": assignment_strong,
                "handoff": handoff,
                "handoff_strong": handoff_strong,
                "mention": mention,
                "multi_child_scope": multi_scope
            }),
            subject,
            if assignment && handoff { "mixed" } else { "derived" },
            TASK_RULE,
            TASK_VERSION,
            evidence,
        );
        if state == STATE_CONFIRMED {
            claim = claim.reason(
                "parent task-call prompt and independent child handoff contain the same token",
            );
        } else if multi_scope {
            claim = claim.reason(
                "one task call names multiple children; token scope is ambiguous and stays candidate",
            );
        } else if assignment_strong {
            claim = claim.reason(
                "assignment-heading token without a corroborating child-handoff token",
            );
        } else if assignment {
            claim = claim.reason("assignment-context token without independent corroboration");
        } else if handoff {
            claim = claim.reason("child handoff token without a parent assignment token");
        } else {
            claim = claim.reason("mention-only token; never promoted to confirmed");
        }
        claims.push(claim);
    }

    if label_claims == 0 && claims.is_empty() {
        claims.push(
            Claim::new(
                LAYER_TASK,
                STATE_MISSING,
                "task_claim",
                Value::Null,
                base_subject,
                "observed",
                TASK_RULE,
                TASK_VERSION,
                Vec::new(),
            )
            .reason("no task label or assignment token in this call scope"),
        );
    }

    claims
}

fn trailing_digits(value: &str) -> Option<String> {
    let start = value
        .as_bytes()
        .iter()
        .rposition(|b| !b.is_ascii_digit())
        .map(|i| i + 1)
        .unwrap_or(0);
    if start == value.len() {
        None
    } else {
        Some(value[start..].to_string())
    }
}
