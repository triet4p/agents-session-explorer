//! Reconciliation state and evidence model.
//!
//! Data types for one query and their provenance, kept separate from the
//! reconciliation algorithm that consumes them.

use ase_markers::Usage;

use crate::{
    DISPATCH_RULE_NAME, DISPATCH_RULE_VERSION, NamingStatus, PATH_RULE_NAME,
    PATH_RULE_VERSION, RULE_NAME, RULE_VERSION,
};
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

    pub(crate) fn session_id(&self) -> Option<&str> {
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

