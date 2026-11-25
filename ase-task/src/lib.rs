//! Reconcile derived signals into an evidence-backed task decision.
//!
//! Inputs are already-derived, metadata-only signals:
//! - `file-name-grammar` / `naming-v1` parse results (derived),
//! - `parentSession` delegation edges (`parent-session-delegation` / `delegation-v1`),
//! - `task_id=` prose observations (`task-id-marker` / `marker-v1`),
//! - native OMP `task_dispatch_v1` / `task_dispatch_child_v1` records joined by
//!   `dispatch_id` (`task-dispatch-provenance` / `dispatch-v1`).
//!
//! A project scope comes from a parsed naming key or from one unique,
//! consistent dispatch join whose two records live in the containing parent
//! and child journals. A marker alone, a path match, a parent declaration, a
//! malformed/duplicate/mismatched dispatch record or a competing claim never
//! becomes a task. Conflicting signals return every candidate with
//! support/refute reasons; insufficient signals stay `unresolved`. Nothing
//! here reads raw bytes, prose, or a model.

use ase_markers::Usage;
use std::collections::{BTreeMap, BTreeSet};

/// Stable identifier of the reconciliation rule recorded on every decision.
pub const RULE_NAME: &str = "task-identity-reconciliation";
/// Bump when the decision policy, candidate rules or status meanings change.
pub const RULE_VERSION: &str = "task-resolve-v2";

/// Path matches are observed source metadata; they are never a derivation.
pub const PATH_RULE_NAME: &str = "source-path-token";
pub const PATH_RULE_VERSION: &str = "path-token-v1";

/// Native OMP parent/child dispatch provenance rule.
pub const DISPATCH_RULE_NAME: &str = "task-dispatch-provenance";
pub const DISPATCH_RULE_VERSION: &str = "dispatch-v1";

/// Re-exported so callers build signal inputs with the original status type.
pub use ase_naming::Status as NamingStatus;

mod dispatch;
mod model;

pub use model::*;

struct KeyAccum {
    project: String,
    task: String,
    naming: Vec<usize>,
    dispatch: Vec<usize>,
}

fn key_of(project: &str, task: &str) -> String {
    format!("{project}|{task}")
}

fn session_of(evidence: &[Evidence], id: usize) -> Option<String> {
    evidence[id].session_id().map(str::to_string)
}


/// Reconcile one token from already-derived signals. Pure and deterministic.
pub fn resolve(input: &Input) -> Resolution {
    let mut evidence: Vec<Evidence> = Vec::new();
    let mut explain: Vec<String> = Vec::new();
    let scope = input.project.as_deref();
    let in_scope = |project: &str| scope.is_none_or(|scope| project == scope);
    if let Some(project) = scope {
        explain.push(format!(
            "Explicit project scope {project}: only naming and dispatch signals of this project are reconciled; other projects are not fallbacks."
        ));
    }

    // ---- naming: parsed keys and ambiguous candidates -------------------
    let mut parsed_by_file: BTreeMap<u32, NamingEvidence> = BTreeMap::new();
    for signal in &input.naming {
        if signal.status == NamingStatus::Parsed {
            if let Some(candidate) = signal
                .candidates
                .first()
                .filter(|candidate| in_scope(&candidate.project))
            {
                parsed_by_file
                    .entry(candidate.file_id)
                    .or_insert_with(|| candidate.clone());
            }
        }
    }

    let mut keys: BTreeMap<String, KeyAccum> = BTreeMap::new();
    let mut ambiguous_naming: Vec<usize> = Vec::new();
    for signal in &input.naming {
        for candidate in &signal.candidates {
            if candidate.task != input.token || !in_scope(&candidate.project) {
                continue;
            }
            match signal.status {
                NamingStatus::Parsed => {
                    let id = evidence.len();
                    evidence.push(Evidence::Naming(candidate.clone()));
                    let key = key_of(&candidate.project, &candidate.task);
                    let entry = keys.entry(key).or_insert_with(|| KeyAccum {
                        project: candidate.project.clone(),
                        task: candidate.task.clone(),
                        naming: Vec::new(),
                        dispatch: Vec::new(),
                    });
                    entry.naming.push(id);
                }
                NamingStatus::Ambiguous => {
                    let id = evidence.len();
                    evidence.push(Evidence::Naming(candidate.clone()));
                    ambiguous_naming.push(id);
                }
                NamingStatus::Unparsed => {}
            }
        }
    }

    // ---- task_id markers ------------------------------------------------
    let mut key_marker: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut key_other: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut other_values: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut unattributed: Vec<usize> = Vec::new();
    let mut mentions: Vec<usize> = Vec::new();
    let mut ambiguous_markers: Vec<usize> = Vec::new();

    for marker in &input.markers {
        let file_naming = parsed_by_file.get(&marker.file_id);
        if marker.value == input.token {
            let id = evidence.len();
            evidence.push(Evidence::Marker(marker.clone()));
            match marker.usage {
                Usage::AssignmentContext => match file_naming {
                    Some(naming) if naming.task == input.token => key_marker
                        .entry(key_of(&naming.project, &naming.task))
                        .or_default()
                        .push(id),
                    _ => unattributed.push(id),
                },
                Usage::Mention => mentions.push(id),
                Usage::Ambiguous => ambiguous_markers.push(id),
            }
        } else if marker.usage == Usage::AssignmentContext {
            if let Some(naming) = file_naming {
                if naming.task == input.token {
                    let id = evidence.len();
                    evidence.push(Evidence::Marker(marker.clone()));
                    key_other
                        .entry(key_of(&naming.project, &naming.task))
                        .or_default()
                        .push(id);
                    other_values
                        .entry(marker.value.clone())
                        .or_default()
                        .push(id);
                }
            }
        }
    }

    // ---- path matches ----------------------------------------------------
    let mut path_ids: Vec<usize> = Vec::new();
    for path in &input.paths {
        let id = evidence.len();
        evidence.push(Evidence::Path(path.clone()));
        path_ids.push(id);
    }
    // ---- native dispatch provenance -------------------------------------
    let dispatch::DispatchAnalysis {
        outcome: dispatch_outcome,
        admitted: dispatch_admitted,
        token_conflicts: token_dispatch_conflicts,
        naming_conflicts: naming_conflict_evidence,
    } = dispatch::analyze(&input, &parsed_by_file, &mut evidence, &mut keys);

    // ---- delegation context ---------------------------------------------
    let mut relevant_sessions: BTreeSet<String> = BTreeSet::new();
    for signal in &input.naming {
        for candidate in &signal.candidates {
            if candidate.task == input.token {
                relevant_sessions.insert(candidate.session_id.clone());
            }
        }
    }
    for marker in &input.markers {
        if marker.value == input.token {
            relevant_sessions.insert(marker.session_id.clone());
        }
    }
    for path in &input.paths {
        relevant_sessions.insert(path.session_id.clone());
    }
    for join in &dispatch_admitted {
        relevant_sessions.insert(join.child_session_id.clone());
    }
    let mut delegation_ids: Vec<usize> = Vec::new();
    for delegation in &input.delegations {
        let touches = relevant_sessions.contains(&delegation.from_session_id)
            || delegation
                .target_session_id
                .as_ref()
                .is_some_and(|target| relevant_sessions.contains(target));
        if touches {
            let id = evidence.len();
            evidence.push(Evidence::Delegation(delegation.clone()));
            delegation_ids.push(id);
        }
    }

    // Sessions that belong to each project-scoped key, for context attachment.
    let mut key_sessions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (key, accum) in &keys {
        for id in &accum.naming {
            if let Some(session) = session_of(&evidence, *id) {
                key_sessions.entry(key.clone()).or_default().insert(session);
            }
        }
        for id in key_marker.get(key).into_iter().flatten() {
            if let Some(session) = session_of(&evidence, *id) {
                key_sessions.entry(key.clone()).or_default().insert(session);
            }
        }
        for id in &accum.dispatch {
            if let Some(session) = session_of(&evidence, *id) {
                key_sessions.entry(key.clone()).or_default().insert(session);
            }
        }
    }

    let mut candidates: Vec<Candidate> = Vec::new();
    let mut candidate_for_key: BTreeMap<String, usize> = BTreeMap::new();
    let status: Status;
    let mut task_key: Option<String> = None;
    if keys.len() == 1 {
        let (key, accum) = keys.iter().next().expect("one key");
        let supports_marker = key_marker.get(key).cloned().unwrap_or_default();
        let supports_dispatch = accum.dispatch.clone();
        let other_assignments = key_other.get(key).cloned().unwrap_or_default();
        let dispatch_refutes = accum
            .naming
            .iter()
            .filter_map(|id| match &evidence[*id] {
                Evidence::Naming(naming) => naming_conflict_evidence.get(&naming.file_id).copied(),
                _ => None,
            })
            .collect::<BTreeSet<usize>>()
            .into_iter()
            .collect::<Vec<usize>>();
        let conflict = (supports_marker.is_empty() && !other_assignments.is_empty())
            || !dispatch_refutes.is_empty();

        if conflict {
            status = Status::Ambiguous;
            let mut supports = accum.naming.clone();
            supports.extend(supports_dispatch.iter().copied());
            let mut refutes = other_assignments.clone();
            refutes.extend(dispatch_refutes.iter().copied());
            let reason = match (other_assignments.is_empty(), dispatch_refutes.is_empty()) {
                (false, true) => if supports_dispatch.is_empty() {
                    "the name grammar yields this key, but prose in its own file assigns only different task_id values and no assignment marker corroborates this key".to_string()
                } else {
                    format!(
                        "the name grammar and {} dispatch join(s) yield this key, but prose in its own file assigns only different task_id values and no assignment marker corroborates this key",
                        supports_dispatch.len()
                    )
                },
                (true, false) => format!(
                    "the name grammar yields this key, but {} native dispatch join(s) identify a different project/task for the child journal; the dispatch claim does not assign and both sides are retained",
                    dispatch_refutes.len()
                ),
                (false, false) => format!(
                    "the name grammar yields this key, but prose assigns different task_id values and {} native dispatch join(s) identify a different project/task; both sides are retained",
                    dispatch_refutes.len()
                ),
                (true, true) => "the name grammar yields this key without corroboration; the signal is retained as contested".to_string(),
            };
            candidate_for_key.insert(key.clone(), candidates.len());
            candidates.push(Candidate {
                project: Some(accum.project.clone()),
                task: Some(accum.task.clone()),
                task_key: Some(key.clone()),
                state: CandidateState::Contested,
                reason,
                supports,
                refutes,
                context: Vec::new(),
            });
            for (value, ids) in &other_values {
                candidates.push(Candidate {
                    project: None,
                    task: Some(value.clone()),
                    task_key: None,
                    state: CandidateState::Candidate,
                    reason: "assignment-context marker for a different value in a file whose name yields the queried token; no naming-derived project scope".to_string(),
                    supports: ids.clone(),
                    refutes: accum.naming.clone(),
                    context: Vec::new(),
                });
            }
            for evidence_id in &dispatch_refutes {
                if let Evidence::Dispatch(join) = &evidence[*evidence_id] {
                    let dispatch_key = key_of(&join.project, &join.task);
                    candidates.push(Candidate {
                        project: Some(join.project.clone()),
                        task: Some(join.task.clone()),
                        task_key: Some(dispatch_key.clone()),
                        state: CandidateState::Candidate,
                        reason: format!(
                            "native dispatch join {} names {dispatch_key} for the executing child, while its file name yields {key}; the layers disagree, so the dispatch claim is not an assignment",
                            join.dispatch_id
                        ),
                        supports: vec![*evidence_id],
                        refutes: accum.naming.clone(),
                        context: Vec::new(),
                    });
                }
            }
            if !other_assignments.is_empty() {
                explain.push(format!(
                    "Conflict: the only parsed key {key} has no assignment marker for its own value, while {} prose occurrence(s) assign {} different value(s). Both sides are retained.",
                    other_assignments.len(),
                    other_values.len()
                ));
            }
            if !dispatch_refutes.is_empty() {
                explain.push(format!(
                    "Conflict: the only parsed key {key} is contradicted by {} native dispatch join(s) whose child journal name parses to a different project/task. Both sides are retained and neither is assigned.",
                    dispatch_refutes.len()
                ));
            }
        } else {
            status = Status::Assigned;
            task_key = Some(key.clone());
            let mut supports = accum.naming.clone();
            supports.extend(supports_marker.iter().copied());
            supports.extend(supports_dispatch.iter().copied());
            let context = other_assignments.clone();
            let from_naming = !accum.naming.is_empty();
            let reason = match (from_naming, supports_dispatch.is_empty(), supports_marker.is_empty()) {
                (true, false, false) => format!(
                    "single project-scoped key from the name grammar and {} unique dispatch join(s), corroborated by assignment-context markers in the same file",
                    supports_dispatch.len()
                ),
                (true, false, true) => format!(
                    "single project-scoped key supported by the name grammar and {} unique dispatch join(s); no assignment-context marker was observed",
                    supports_dispatch.len()
                ),
                (true, true, false) => "single project-scoped naming key corroborated by assignment-context markers in the same file".to_string(),
                (true, true, true) => "single project-scoped naming key; no assignment-context marker was observed, so the grammar alone supplies the key".to_string(),
                (false, false, false) => format!(
                    "single project-scoped key from {} unique dispatch join(s) to the executing child, corroborated by assignment-context markers",
                    supports_dispatch.len()
                ),
                (false, false, true) => format!(
                    "single project-scoped key from {} unique dispatch join(s) to the executing child; no name-grammar key or assignment marker was found",
                    supports_dispatch.len()
                ),
                (false, true, _) => "a project-scoped key requires a naming parse or a unique dispatch join; none is present".to_string(),
            };
            candidate_for_key.insert(key.clone(), candidates.len());
            candidates.push(Candidate {
                project: Some(accum.project.clone()),
                task: Some(accum.task.clone()),
                task_key: Some(key.clone()),
                state: CandidateState::Supported,
                reason,
                supports,
                refutes: Vec::new(),
                context,
            });
            explain.push(format!(
                "Assigned: exactly one project-scoped key {key}, with {} naming evidence item(s), {} dispatch join(s) and {} corroborating marker(s).",
                accum.naming.len(),
                supports_dispatch.len(),
                supports_marker.len()
            ));
        }
    } else if keys.len() > 1 {
        status = Status::Ambiguous;
        let key_count = keys.len();
        for (key, accum) in &keys {
            let supports_marker = key_marker.get(key).cloned().unwrap_or_default();
            let mut supports = accum.naming.clone();
            supports.extend(supports_marker.iter().copied());
            supports.extend(accum.dispatch.iter().copied());
            let other_assignments = key_other.get(key).cloned().unwrap_or_default();
            let dispatch_refutes = accum
                .naming
                .iter()
                .filter_map(|id| match &evidence[*id] {
                    Evidence::Naming(naming) => {
                        naming_conflict_evidence.get(&naming.file_id).copied()
                    }
                    _ => None,
                })
                .collect::<BTreeSet<usize>>()
                .into_iter()
                .collect::<Vec<usize>>();
            let (mut refutes, context) = if supports_marker.is_empty() && !other_assignments.is_empty() {
                (other_assignments, Vec::new())
            } else {
                (Vec::new(), other_assignments)
            };
            refutes.extend(dispatch_refutes.iter().copied());
            candidate_for_key.insert(key.clone(), candidates.len());
            candidates.push(Candidate {
                project: Some(accum.project.clone()),
                task: Some(accum.task.clone()),
                task_key: Some(key.clone()),
                state: CandidateState::Contested,
                reason: format!(
                    "the same task label occurs in {key_count} project-scoped keys from the name grammar and/or native dispatch joins; project is part of the key and the candidates are not merged"
                ),
                supports,
                refutes,
                context,
            });
        }
        explain.push(format!(
            "Ambiguous: {key_count} distinct project-scoped keys carry the token; every key is retained with its own project and evidence."
        ));
    } else {
        status = Status::Unresolved;
        explain.push(
            "Unresolved: no parsed file-name grammar candidate or unique dispatch join yields a project scope for this token."
                .to_string(),
        );
    }

    if keys.is_empty() && !token_dispatch_conflicts.is_empty() {
        for evidence_id in &token_dispatch_conflicts {
            if let Evidence::Dispatch(join) = &evidence[*evidence_id] {
                let dispatch_key = key_of(&join.project, &join.task);
                candidates.push(Candidate {
                    project: Some(join.project.clone()),
                    task: Some(join.task.clone()),
                    task_key: Some(dispatch_key.clone()),
                    state: CandidateState::Contested,
                    reason: format!(
                        "native dispatch join {} names {dispatch_key} for this token, but the child journal's file name derives a different project/task; the layers disagree, so the claim is not an assignment",
                        join.dispatch_id
                    ),
                    supports: vec![*evidence_id],
                    refutes: Vec::new(),
                    context: Vec::new(),
                });
            }
        }
        explain.push(
            "Conflict: the dispatch claim for this token is retained as contested because the child's file name yields a different project/task; it is not an assignment."
                .to_string(),
        );
    }

    // Attach token-level context (paths, delegation, ambiguous grammar,
    // unattributed markers) to the owning key when its session is known.
    let mut leftover: Vec<usize> = Vec::new();
    let mut contextual: Vec<usize> = Vec::new();
    contextual.extend(unattributed.iter().copied());
    contextual.extend(mentions.iter().copied());
    contextual.extend(ambiguous_markers.iter().copied());
    contextual.extend(ambiguous_naming.iter().copied());
    contextual.extend(path_ids.iter().copied());
    contextual.extend(delegation_ids.iter().copied());

    for id in contextual {
        let mut owners: Vec<String> = Vec::new();
        match &evidence[id] {
            Evidence::Delegation(delegation) => {
                for (key, sessions) in &key_sessions {
                    if sessions.contains(&delegation.from_session_id)
                        || delegation
                            .target_session_id
                            .as_ref()
                            .is_some_and(|target| sessions.contains(target))
                    {
                        owners.push(key.clone());
                    }
                }
            }
            other => {
                if let Some(session) = other.session_id() {
                    for (key, sessions) in &key_sessions {
                        if sessions.contains(session) {
                            owners.push(key.clone());
                        }
                    }
                }
            }
        }
        if owners.len() == 1 {
            if let Some(index) = candidate_for_key.get(&owners[0]) {
                candidates[*index].context.push(id);
                continue;
            }
        }
        leftover.push(id);
    }

    if keys.len() == 1 && !leftover.is_empty() {
        if let Some(index) = candidate_for_key.values().next() {
            candidates[*index].context.extend(leftover.drain(..));
        }
    }

    if keys.len() != 1 && !leftover.is_empty() {
        let mut supports = Vec::new();
        let mut context = Vec::new();
        for id in leftover {
            match &evidence[id] {
                Evidence::Marker(marker) if marker.value == input.token => {
                    if marker.usage == Usage::AssignmentContext {
                        supports.push(id);
                    } else {
                        context.push(id);
                    }
                }
                _ => context.push(id),
            }
        }
        let reason = if supports.is_empty() {
            "token-level evidence without a project-scoped naming key or dispatch join: mentions, path hits, ambiguous grammar or parent declarations are candidates, not assignments"
        } else {
            "assignment-context marker for this token without a naming-derived or dispatch-derived project scope: a marker alone is not a TaskKey"
        };
        candidates.push(Candidate {
            project: None,
            task: Some(input.token.clone()),
            task_key: None,
            state: CandidateState::Candidate,
            reason: reason.to_string(),
            supports,
            refutes: Vec::new(),
            context,
        });
        explain.push(
            "A token-level candidate was kept: it has no project scope, so it cannot become a TaskKey."
                .to_string(),
        );
    }

    // ---- layer accounting and explain -----------------------------------
    let marker_assignment = input
        .markers
        .iter()
        .filter(|m| m.value == input.token && m.usage == Usage::AssignmentContext)
        .count();
    explain.insert(
        0,
        format!(
            "Rule {RULE_NAME} / {RULE_VERSION}: a project-scoped key comes from a parsed file-name grammar candidate or a unique, consistent native dispatch join; task_id markers corroborate, path hits and parentSession are context only."
        ),
    );
    explain.push(format!(
        "Layers: {} naming signal(s), {} evidence item(s), {} marker occurrence(s) for the token ({} assignment-context), {} path match(es), {} relevant parentSession edge(s), {} admitted dispatch join(s) for the token.",
        input.naming.len(),
        evidence.len(),
        input.markers.iter().filter(|m| m.value == input.token).count(),
        marker_assignment,
        input.paths.len(),
        delegation_ids.len(),
        dispatch_admitted.len()
    ));
    explain.push(format!(
        "Dispatch layer {DISPATCH_RULE_NAME} / {DISPATCH_RULE_VERSION}: {} native parent record(s), {} child link(s), {} structurally unique join(s), {} rejected dispatch id group(s), {} record issue(s); a record never assigns without exactly one consistent parent-child session match.",
        dispatch_outcome.parents,
        dispatch_outcome.children,
        dispatch_outcome.joined,
        dispatch_outcome.rejected_groups,
        dispatch_outcome.issues_total
    ));
    for join in &dispatch_admitted {
        explain.push(format!(
            "Dispatch join {}: parent session {} at {} -> child session {} at {}; runtime project={}, task={}, attempt={}; both record spans were verified against indexed bytes.",
            join.dispatch_id,
            join.parent_session_id,
            join.parent_span,
            join.child_session_id,
            join.child_span,
            join.project,
            join.task,
            join.attempt
        ));
    }
    for issue in &dispatch_outcome.issues_for_token {
        explain.push(format!(
            "Dispatch claim rejected for this token: {} {} record at {} (dispatch id {}); {}.",
            issue.kind,
            issue.record,
            issue.span,
            issue.dispatch_id.as_deref().unwrap_or("-"),
            issue.detail
        ));
    }
    match &input.marker_layer {
        MarkerLayer {
            unavailable: Some(reason),
            ..
        } => explain.push(format!(
            "Marker layer unavailable: {reason}; no task_id signal was consulted and no absence of markers is claimed."
        )),
        MarkerLayer {
            generation: Some(generation),
            blocks_total,
            blocks_scanned,
            excluded_role_blocks,
            unverified_blocks,
            ..
        } => explain.push(format!(
            "Marker layer: generation {generation}; {blocks_scanned}/{blocks_total} prose block(s) scanned, {excluded_role_blocks} non-prose-role block(s) excluded, {unverified_blocks} unverified block(s) dropped."
        )),
        MarkerLayer { .. } => {}
    }
    explain.push(format!(
        "Evidence basis: naming is derived by {} / {}; marker occurrences and parent declarations are observed with their classification/edge derived by {} / {} and {} / {}; native dispatch records are observed and the join/identity is derived by {} / {}; path hits are observed metadata.",
        ase_naming::RULE_NAME,
        ase_naming::RULE_VERSION,
        ase_markers::RULE_NAME,
        ase_markers::RULE_VERSION,
        ase_delegation::RULE_NAME,
        ase_delegation::RULE_VERSION,
        DISPATCH_RULE_NAME,
        DISPATCH_RULE_VERSION
    ));

    let summary = match status {
        Status::Assigned => format!(
            "assigned to {} from exactly one project-scoped key with inspectable evidence",
            task_key.as_deref().unwrap_or("")
        ),
        Status::Ambiguous => format!(
            "{} competing candidate(s) retained with support/refute reasons; the bare token does not identify one task",
            candidates.len()
        ),
        Status::Unresolved => {
            if candidates.is_empty() {
                if dispatch_outcome.claims_for_token > 0 {
                    format!(
                        "{} dispatch claim(s) name this token, but none forms a unique consistent parent-child join; the query stays unresolved",
                        dispatch_outcome.claims_for_token
                    )
                } else {
                    "no observed or derived signal matches this token; the query stays unresolved".to_string()
                }
            } else {
                format!(
                    "no project-scoped naming key or dispatch join; {} candidate signal set(s) retained without an assignment",
                    candidates.len()
                )
            }
        }
    };

    Resolution {
        token: input.token.clone(),
        status,
        summary,
        task_key,
        candidates,
        evidence,
        dispatch: dispatch_outcome,
        marker_layer: input.marker_layer.clone(),
        explain,
        rule: RuleInfo::default(),
    }
}
