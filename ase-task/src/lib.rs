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

/// Whether a signal is read directly from the source or derived by a rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Basis {
    Observed,
    Derived,
}

impl Basis {
    pub fn label(self) -> &'static str {
        match self {
            Basis::Observed => "observed",
            Basis::Derived => "derived",
        }
    }
}

/// Decision status for one `resolve task:TOKEN` query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Exactly one project-scoped naming key, with no layer conflict.
    Assigned,
    /// Competing or conflicting candidates; every one is retained.
    Ambiguous,
    /// No project-scoped key and no conflict; signals remain candidates.
    Unresolved,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Assigned => "assigned",
            Status::Ambiguous => "ambiguous",
            Status::Unresolved => "unresolved",
        }
    }
}

/// How one evidence item relates to one candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stance {
    Support,
    Refute,
    Context,
    Candidate,
}

impl Stance {
    pub fn label(self) -> &'static str {
        match self {
            Stance::Support => "support",
            Stance::Refute => "refute",
            Stance::Context => "context",
            Stance::Candidate => "candidate",
        }
    }
}

/// One grammar-derived naming candidate for a single indexed file.
#[derive(Clone, Debug)]
pub struct NamingEvidence {
    pub project: String,
    pub task: String,
    pub role: String,
    pub attempt: String,
    pub split_rule: String,
    pub file_id: u32,
    pub path: String,
    pub provider: String,
    pub session_id: String,
    pub span_anchor: String,
}

/// One `task_id=` occurrence, metadata only (no prose text).
#[derive(Clone, Debug)]
pub struct MarkerEvidence {
    pub value: String,
    pub usage: Usage,
    pub reason: String,
    pub assignment_cue: Option<String>,
    pub mention_cue: Option<String>,
    pub occurrence_in_block: u32,
    pub char_start: u32,
    pub char_end: u32,
    pub generation: String,
    pub event: u32,
    pub span: String,
    pub selector: String,
    pub block_hash: String,
    pub role: String,
    pub file_id: u32,
    pub session_id: String,
}

/// One `delegated_from` edge from a `parentSession` declaration.
#[derive(Clone, Debug)]
pub struct DelegationEvidence {
    pub from_session_id: String,
    pub from_span: String,
    pub declared_id: String,
    pub observation: String,
    pub field: String,
    pub link_status: String,
    pub target_session_id: Option<String>,
    pub target_span: Option<String>,
    pub reason: String,
}

/// One literal source-path token match. Observed metadata, not identity.
#[derive(Clone, Debug)]
pub struct PathEvidence {
    pub session_id: String,
    pub path: String,
    pub provider: String,
    pub span_anchor: String,
}

/// One native OMP `task_dispatch_v1` parent record reopened from its span.
///
/// `containing_session_id` is the indexed journal that holds the record; the
/// structured `parent_session_id` must equal it before the record can join.
#[derive(Clone, Debug)]
pub struct DispatchIntent {
    pub dispatch_id: String,
    pub parent_session_id: String,
    pub parent_tool_call_id: Option<String>,
    pub spawn_index: u32,
    pub project: String,
    pub task: String,
    pub attempt: String,
    pub event: u32,
    pub span: String,
    pub file_id: u32,
    pub containing_session_id: String,
    pub span_verified: bool,
}

/// One native OMP `task_dispatch_child_v1` link reopened from its span.
///
/// `session_id` is the structured runtime value; `containing_session_id` is
/// the journal that holds the link. They must agree for a unique join.
#[derive(Clone, Debug)]
pub struct DispatchChildLink {
    pub dispatch_id: String,
    pub session_id: String,
    pub event: u32,
    pub span: String,
    pub file_id: u32,
    pub containing_session_id: String,
    pub span_verified: bool,
}

/// One dispatch record that could not be admitted as a join candidate.
#[derive(Clone, Debug)]
pub struct DispatchIssue {
    pub record: String,
    pub kind: String,
    pub dispatch_id: Option<String>,
    pub task: Option<String>,
    pub event: u32,
    pub span: String,
    pub detail: String,
}

/// Native dispatch records for one index snapshot. No prose is present.
#[derive(Clone, Debug, Default)]
pub struct DispatchInput {
    pub parents: Vec<DispatchIntent>,
    pub children: Vec<DispatchChildLink>,
    pub issues: Vec<DispatchIssue>,
}

/// A structurally unique, consistent `dispatch_id` join.
#[derive(Clone, Debug)]
pub struct DispatchJoin {
    pub dispatch_id: String,
    pub parent_session_id: String,
    pub child_session_id: String,
    pub parent_tool_call_id: Option<String>,
    pub spawn_index: u32,
    pub project: String,
    pub task: String,
    pub attempt: String,
    pub parent_event: u32,
    pub parent_span: String,
    pub parent_file_id: u32,
    pub child_event: u32,
    pub child_span: String,
    pub child_file_id: u32,
    pub parent_span_verified: bool,
    pub child_span_verified: bool,
}

/// Aggregate dispatch layer accounting for one query.
#[derive(Clone, Debug, Default)]
pub struct DispatchOutcome {
    pub parents: usize,
    pub children: usize,
    pub joined: usize,
    pub rejected_groups: usize,
    pub issues_total: usize,
    pub claims_for_token: usize,
    pub issues_for_token: Vec<DispatchIssue>,
}

#[derive(Clone, Debug)]
pub enum Evidence {
    Naming(NamingEvidence),
    Marker(MarkerEvidence),
    Delegation(DelegationEvidence),
    Path(PathEvidence),
    Dispatch(DispatchJoin),
}

impl Evidence {
    pub fn kind(&self) -> &'static str {
        match self {
            Evidence::Naming(_) => "naming_parse",
            Evidence::Marker(_) => "task_id_marker",
            Evidence::Delegation(_) => "parent_session",
            Evidence::Path(_) => "path_match",
            Evidence::Dispatch(_) => "dispatch_join",
        }
    }

    pub fn basis(&self) -> Basis {
        match self {
            // The path is observed; the project/task/role/attempt split is derived.
            Evidence::Naming(_) => Basis::Derived,
            // The occurrence is observed; its usage class is a marker-v1 derivation.
            Evidence::Marker(_) => Basis::Observed,
            // The declaration is observed; the linked edge is a delegation-v1 derivation.
            Evidence::Delegation(_) => Basis::Observed,
            Evidence::Path(_) => Basis::Observed,
            // Both records are observed; the join and identity are dispatch-v1 derivations.
            Evidence::Dispatch(_) => Basis::Observed,
        }
    }

    pub fn rule_name(&self) -> &'static str {
        match self {
            Evidence::Naming(_) => ase_naming::RULE_NAME,
            Evidence::Marker(_) => ase_markers::RULE_NAME,
            Evidence::Delegation(_) => ase_delegation::RULE_NAME,
            Evidence::Path(_) => PATH_RULE_NAME,
            Evidence::Dispatch(_) => DISPATCH_RULE_NAME,
        }
    }

    pub fn rule_version(&self) -> &'static str {
        match self {
            Evidence::Naming(_) => ase_naming::RULE_VERSION,
            Evidence::Marker(_) => ase_markers::RULE_VERSION,
            Evidence::Delegation(_) => ase_delegation::RULE_VERSION,
            Evidence::Path(_) => PATH_RULE_VERSION,
            Evidence::Dispatch(_) => DISPATCH_RULE_VERSION,
        }
    }

    fn session_id(&self) -> Option<&str> {
        match self {
            Evidence::Naming(n) => Some(&n.session_id),
            Evidence::Marker(m) => Some(&m.session_id),
            Evidence::Delegation(_) => None,
            Evidence::Path(p) => Some(&p.session_id),
            Evidence::Dispatch(d) => Some(&d.child_session_id),
        }
    }
}

/// Naming audit output for one indexed file, kept per-file for attribution.
#[derive(Clone, Debug)]
pub struct NamingSignal {
    pub status: NamingStatus,
    pub reason: String,
    pub candidates: Vec<NamingEvidence>,
}

/// Availability and verification counts of the published marker layer.
#[derive(Clone, Debug)]
pub struct MarkerLayer {
    pub generation: Option<String>,
    pub unavailable: Option<String>,
    pub blocks_total: usize,
    pub blocks_scanned: usize,
    pub excluded_role_blocks: usize,
    pub unverified_blocks: usize,
}

#[derive(Clone, Debug)]
pub struct Input {
    pub token: String,
    /// Explicit project scope for `(project, task)` addressing. `None` keeps
    /// the unscoped token contract, where the same token in two projects is
    /// ambiguous. `Some(project)` filters the same naming and dispatch signals
    /// by project before reconciliation; it never selects among candidates
    /// after the fact.
    pub project: Option<String>,
    pub naming: Vec<NamingSignal>,
    pub markers: Vec<MarkerEvidence>,
    pub delegations: Vec<DelegationEvidence>,
    pub paths: Vec<PathEvidence>,
    pub dispatch: DispatchInput,
    pub marker_layer: MarkerLayer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateState {
    Supported,
    Contested,
    Candidate,
}

impl CandidateState {
    pub fn label(self) -> &'static str {
        match self {
            CandidateState::Supported => "supported",
            CandidateState::Contested => "contested",
            CandidateState::Candidate => "candidate",
        }
    }
}

/// One competing interpretation. Evidence indices point into `Resolution::evidence`.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub project: Option<String>,
    pub task: Option<String>,
    pub task_key: Option<String>,
    pub state: CandidateState,
    pub reason: String,
    pub supports: Vec<usize>,
    pub refutes: Vec<usize>,
    pub context: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct RuleInfo {
    pub name: String,
    pub version: String,
    pub assignment_rule: String,
    pub abstention: String,
    pub observed_derived: String,
    pub task_boundary: String,
}

impl Default for RuleInfo {
    fn default() -> Self {
        Self {
            name: RULE_NAME.to_string(),
            version: RULE_VERSION.to_string(),
            assignment_rule: "A project-scoped key comes from a parsed file-name grammar candidate or from one unique, consistent native dispatch join (one task_dispatch_v1 parent record, one task_dispatch_child_v1 link, matching containing sessions, verified spans, distinct parent/child journals). An assignment-context task_id marker in the same file corroborates the key but is not required; a marker or dispatch claim in a file with no usable project scope cannot create one.".to_string(),
            abstention: "More than one project-scoped key for the token, or a file whose own prose assigns only different task_id values, returns ambiguous with every candidate and support/refute reasons. Without a project-scoped key the result stays unresolved; mentions, path hits and parent declarations remain candidates. Dispatch records that are malformed, missing a counterpart, duplicated, mismatched to the containing session, contradicted by the child's own parsed name, or competing for one child session never assign.".to_string(),
            observed_derived: "Naming evidence is derived (file-name-grammar/naming-v1) over an observed path. Marker occurrences and parentSession declarations are observed; their classification and edge resolution are derived by marker-v1 and delegation-v1. Native dispatch parent/child records are observed source records, and the dispatch-id join and project/task/attempt selection are derived by dispatch-v1. Path matches are observed metadata. No external judgment is included.".to_string(),
            task_boundary: "A parentSession edge is delegation, never a task. A mention, an ambiguous marker, a path substring or a parent declaration never assigns a task. A dispatch record assigns only the executing child session it uniquely joins; a suffix, path, name, prose echo or parent-only claim never does. Project is part of the key; identical tokens in different projects are never merged.".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Resolution {
    pub token: String,
    pub status: Status,
    pub summary: String,
    pub task_key: Option<String>,
    pub candidates: Vec<Candidate>,
    pub evidence: Vec<Evidence>,
    pub dispatch: DispatchOutcome,
    pub marker_layer: MarkerLayer,
    pub explain: Vec<String>,
    pub rule: RuleInfo,
}

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
