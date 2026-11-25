//! Native dispatch provenance analysis.
//!
//! Reopened parent and child records are reconciled into unique, consistent
//! joins; rejected groups, token claims and cross-layer contradictions stay
//! inspectable in the outcome.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{
    DispatchChildLink, DispatchIntent, DispatchIssue, DispatchJoin, DispatchOutcome,
    Evidence, NamingEvidence,
};
use crate::{key_of, Input, KeyAccum};

pub(crate) struct DispatchAnalysis {
    pub(crate) outcome: DispatchOutcome,
    pub(crate) admitted: Vec<DispatchJoin>,
    pub(crate) token_conflicts: Vec<usize>,
    pub(crate) naming_conflicts: BTreeMap<u32, usize>,
}

pub(crate) fn analyze(
    input: &Input,
    parsed_by_file: &BTreeMap<u32, NamingEvidence>,
    evidence: &mut Vec<Evidence>,
    keys: &mut BTreeMap<String, KeyAccum>,
) -> DispatchAnalysis {
    let scope = input.project.as_deref();
    let in_scope = |project: &str| scope.is_none_or(|scope| project == scope);

fn dispatch_issue(
    record: &str,
    kind: &str,
    dispatch_id: Option<String>,
    task: Option<String>,
    event: u32,
    span: String,
    detail: String,
) -> DispatchIssue {
    DispatchIssue {
        record: record.to_string(),
        kind: kind.to_string(),
        dispatch_id,
        task,
        event,
        span,
        detail,
    }
}

    // ---- native dispatch provenance -------------------------------------
    // The record payloads were reopened and verified by the caller. This
    // block decides whether one dispatch_id yields exactly one structurally
    // consistent parent -> child join; record-level extraction issues arrive
    // in `input.dispatch.issues` and can never assign.
    let mut dispatch_issues = input.dispatch.issues.clone();
    let mut groups: BTreeMap<String, (Vec<&DispatchIntent>, Vec<&DispatchChildLink>)> =
        BTreeMap::new();
    for parent in &input.dispatch.parents {
        groups
            .entry(parent.dispatch_id.clone())
            .or_default()
            .0
            .push(parent);
    }
    for child in &input.dispatch.children {
        groups
            .entry(child.dispatch_id.clone())
            .or_default()
            .1
            .push(child);
    }

    let mut joins: Vec<DispatchJoin> = Vec::new();
    for (dispatch_id, (parents, children)) in &groups {
        if parents.len() != 1 || children.len() != 1 {
            let kind = match (parents.len(), children.len()) {
                (0, 1) => "missing_parent",
                (1, 0) => "missing_child",
                (0, _) => "missing_parent",
                (_, 0) => "duplicate_parent",
                (_, 1) => "duplicate_parent",
                (1, _) => "duplicate_child",
                _ => "duplicate_records",
            };
            let task = parents
                .first()
                .map(|parent| parent.task.clone())
                .filter(|task| !task.is_empty());
            let (event, span) = parents
                .first()
                .map(|parent| (parent.event, parent.span.clone()))
                .or_else(|| {
                    children
                        .first()
                        .map(|child| (child.event, child.span.clone()))
                })
                .unwrap_or((0, String::new()));
            dispatch_issues.push(dispatch_issue(
                "join",
                kind,
                Some(dispatch_id.clone()),
                task,
                event,
                span,
                format!(
                    "expected exactly one parent record and one child link for this dispatch id; observed {} parent record(s) and {} child link(s), so no assignment can be derived",
                    parents.len(),
                    children.len()
                ),
            ));
            continue;
        }
        let parent = parents[0];
        let child = children[0];
        let reject = if !parent.span_verified || !child.span_verified {
            Some((
                "unverified_span",
                "at least one record span no longer matches its indexed checksum".to_string(),
            ))
        } else if parent.project.is_empty()
            || parent.task.is_empty()
            || parent.attempt.is_empty()
        {
            Some((
                "missing_identity",
                "the parent record lacks a non-empty project/task/attempt identity".to_string(),
            ))
        } else if parent.containing_session_id.is_empty() || child.containing_session_id.is_empty()
        {
            Some((
                "unresolved_session",
                "a record is not assigned to an indexed journal session".to_string(),
            ))
        } else if parent.parent_session_id != parent.containing_session_id {
            Some((
                "parent_session_mismatch",
                "the structured parent_session_id does not match the journal that contains the parent record".to_string(),
            ))
        } else if child.session_id != child.containing_session_id {
            Some((
                "child_session_mismatch",
                "the structured child session_id does not match the journal that contains the child link".to_string(),
            ))
        } else if parent.containing_session_id == child.containing_session_id {
            Some((
                "self_target",
                "the parent record and the child link occupy the same journal session".to_string(),
            ))
        } else {
            None
        };
        if let Some((kind, detail)) = reject {
            dispatch_issues.push(dispatch_issue(
                "join",
                kind,
                Some(dispatch_id.clone()),
                Some(parent.task.clone()),
                parent.event,
                parent.span.clone(),
                detail,
            ));
            continue;
        }
        joins.push(DispatchJoin {
            dispatch_id: dispatch_id.clone(),
            parent_session_id: parent.parent_session_id.clone(),
            child_session_id: child.session_id.clone(),
            parent_tool_call_id: parent.parent_tool_call_id.clone(),
            spawn_index: parent.spawn_index,
            project: parent.project.clone(),
            task: parent.task.clone(),
            attempt: parent.attempt.clone(),
            parent_event: parent.event,
            parent_span: parent.span.clone(),
            parent_file_id: parent.file_id,
            child_event: child.event,
            child_span: child.span.clone(),
            child_file_id: child.file_id,
            parent_span_verified: parent.span_verified,
            child_span_verified: child.span_verified,
        });
    }

    // A child journal is one executing session; two dispatch ids claiming it
    // are competing claims and neither may assign.
    let mut claims_by_child: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, join) in joins.iter().enumerate() {
        claims_by_child
            .entry(join.child_session_id.clone())
            .or_default()
            .push(index);
    }
    let mut rejected_joins: BTreeSet<usize> = BTreeSet::new();
    for (child_session, claims) in &claims_by_child {
        if claims.len() < 2 {
            continue;
        }
        for index in claims {
            rejected_joins.insert(*index);
            let join = &joins[*index];
            dispatch_issues.push(dispatch_issue(
                "join",
                "competing_claims",
                Some(join.dispatch_id.clone()),
                Some(join.task.clone()),
                join.child_event,
                join.child_span.clone(),
                format!(
                    "child session {child_session} is claimed by {} dispatch ids; no unique executing child can be selected",
                    claims.len()
                ),
            ));
        }
    }
    if !rejected_joins.is_empty() {
        joins = joins
            .into_iter()
            .enumerate()
            .filter(|(index, _)| !rejected_joins.contains(index))
            .map(|(_, join)| join)
            .collect();
    }

    let joined_ids: BTreeSet<String> = joins
        .iter()
        .map(|join| join.dispatch_id.clone())
        .collect();

    // A structurally valid join whose child journal name parses to a
    // different key is a cross-layer contradiction. The dispatch claim never
    // assigns, and the naming key on the other side is contested as well.
    let mut contradicted: Vec<(usize, u32)> = Vec::new();
    for (index, join) in joins.iter().enumerate() {
        if let Some(naming) = parsed_by_file.get(&join.child_file_id) {
            let naming_key = key_of(&naming.project, &naming.task);
            if naming_key != key_of(&join.project, &join.task) {
                contradicted.push((index, join.child_file_id));
            }
        }
    }
    let mut naming_conflict_evidence: BTreeMap<u32, usize> = BTreeMap::new();
    for (join_index, file_id) in &contradicted {
        let join = &joins[*join_index];
        let id = evidence.len();
        evidence.push(Evidence::Dispatch(join.clone()));
        naming_conflict_evidence.insert(*file_id, id);
    }

    let mut dispatch_outcome = DispatchOutcome {
        parents: input.dispatch.parents.len(),
        children: input.dispatch.children.len(),
        joined: joins.len(),
        rejected_groups: groups
            .keys()
            .filter(|dispatch_id| !joined_ids.contains(*dispatch_id))
            .count(),
        issues_total: dispatch_issues.len() + contradicted.len(),
        ..Default::default()
    };

    let token_join_claims = joins
        .iter()
        .filter(|join| join.task == input.token && in_scope(&join.project))
        .count();
    let mut dispatch_admitted: Vec<DispatchJoin> = Vec::new();
    let mut token_dispatch_conflicts: Vec<usize> = Vec::new();
    for join in &joins {
        if join.task != input.token || !in_scope(&join.project) {
            continue;
        }
        if let Some(evidence_id) = naming_conflict_evidence.get(&join.child_file_id) {
            token_dispatch_conflicts.push(*evidence_id);
            if let Some(naming) = parsed_by_file.get(&join.child_file_id) {
                dispatch_outcome.issues_for_token.push(dispatch_issue(
                    "join",
                    "naming_conflict",
                    Some(join.dispatch_id.clone()),
                    Some(join.task.clone()),
                    join.child_event,
                    join.child_span.clone(),
                    format!(
                        "the child file name derives key {}/{} while the runtime record states {}/{}; the layers disagree, so the dispatch claim does not assign",
                        naming.project, naming.task, join.project, join.task
                    ),
                ));
            }
            continue;
        }
        let id = evidence.len();
        evidence.push(Evidence::Dispatch(join.clone()));
        let key = key_of(&join.project, &join.task);
        keys.entry(key)
            .or_insert_with(|| KeyAccum {
                project: join.project.clone(),
                task: join.task.clone(),
                naming: Vec::new(),
                dispatch: Vec::new(),
            })
            .dispatch
            .push(id);
        dispatch_admitted.push(join.clone());
    }
    let extra_issues_for_token = dispatch_issues
        .iter()
        .filter(|issue| issue.task.as_deref() == Some(input.token.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    dispatch_outcome.claims_for_token = token_join_claims + extra_issues_for_token.len();
    dispatch_outcome
        .issues_for_token
        .extend(extra_issues_for_token);
    DispatchAnalysis {
        outcome: dispatch_outcome,
        admitted: dispatch_admitted,
        token_conflicts: token_dispatch_conflicts,
        naming_conflicts: naming_conflict_evidence,
    }
}
