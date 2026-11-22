//! P0c-3: evidence-backed `parentSession` delegation.
//!
//! This crate consumes indexed session metadata only. It never opens raw
//! bytes, decodes prose, consults file order or path proximity, or emits a
//! task key. A declared parent is recorded as a `delegated_from` edge with its
//! own rule version and evidence; it is never an `assigned_to` edge.

/// Stable identifier of the derivation rule recorded on every edge.
pub const RULE_NAME: &str = "parent-session-delegation";
/// Bump when target selection, evidence fields or status meanings change.
pub const RULE_VERSION: &str = "delegation-v1";

/// Where the declared parent id was observed. The two origins are kept apart:
/// a header span locates the file but does not prove the id text lives in a
/// payload field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationKind {
    /// `omp.parentSession` or an equivalent session-meta payload field.
    PayloadField,
    /// Sidecar directory name recorded by the decoder (`path_derived`).
    PathDerived,
}

impl ObservationKind {
    pub fn label(self) -> &'static str {
        match self {
            ObservationKind::PayloadField => "payload_field",
            ObservationKind::PathDerived => "path_derived",
        }
    }
}

/// Minimal, index-backed description of a session. `header_span` is a locator
/// only; it is not evidence that a name text lives in the payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRef {
    pub session_idx: u32,
    pub id: String,
    pub file_id: u32,
    pub path: String,
    pub provider: String,
    pub header_span: String,
}

/// One declared parent, with the exact observation surface preserved.
#[derive(Clone, Debug)]
pub struct Declaration {
    /// Verbatim parent id from indexed session metadata.
    pub declared_id: String,
    pub kind: ObservationKind,
    /// Indexed field that produced the declaration.
    pub field: &'static str,
    /// Span of the event carrying a payload field; `None` for a path observation.
    pub payload_span: Option<String>,
    /// Directory segment carrying the id; `None` for a payload observation.
    pub path_segment: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SessionInput {
    pub session: SessionRef,
    pub declaration: Option<Declaration>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Exactly one distinct indexed session has the declared id.
    Linked,
    /// No indexed session has the declared id.
    UnlinkedAbsent,
    /// More than one indexed session has the declared id.
    UnlinkedAmbiguous,
    /// The declared id is the source session itself.
    UnlinkedSelf,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Linked => "linked",
            Status::UnlinkedAbsent => "unlinked_absent",
            Status::UnlinkedAmbiguous => "unlinked_ambiguous",
            Status::UnlinkedSelf => "unlinked_self",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Edge {
    pub from: SessionRef,
    pub declaration: Declaration,
    pub status: Status,
    /// Set only for `Linked`: a unique exact-id match.
    pub target: Option<SessionRef>,
    /// Every exact-id match, retained verbatim. Never ranked or selected.
    pub candidates: Vec<SessionRef>,
    pub reason: &'static str,
}

#[derive(Clone, Debug)]
pub struct RuleInfo {
    pub name: String,
    pub version: String,
    pub domain: String,
    pub abstention: String,
    pub task_boundary: String,
}

impl Default for RuleInfo {
    fn default() -> Self {
        Self {
            name: RULE_NAME.to_string(),
            version: RULE_VERSION.to_string(),
            domain: "A target is accepted only by exact session-id equality inside the indexed snapshot. File order, path adjacency and the index's first-match parent_idx are never used to choose a target.".to_string(),
            abstention: "Zero matches stay unlinked_absent; more than one stays unlinked_ambiguous with every candidate retained; a self reference stays unlinked_self. No tie-break is applied.".to_string(),
            task_boundary: "A parent declaration is a delegated_from edge only. It never produces a TaskKey, an assigned_to edge, a task/role/attempt label or a chronology.".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Audit {
    pub rule: RuleInfo,
    pub sessions: usize,
    pub declarations: usize,
    pub linked: usize,
    pub unlinked_absent: usize,
    pub unlinked_ambiguous: usize,
    pub unlinked_self: usize,
    pub without_parent: usize,
    pub payload_declarations: usize,
    pub path_declarations: usize,
    pub edges: Vec<Edge>,
    /// Sessions without a declared parent: no edge is emitted for them.
    pub sessions_without_parent: Vec<SessionRef>,
}

/// Derive delegation edges from indexed session metadata only.
pub fn audit(input: &[SessionInput]) -> Audit {
    let mut out = Audit {
        rule: RuleInfo::default(),
        sessions: input.len(),
        declarations: 0,
        linked: 0,
        unlinked_absent: 0,
        unlinked_ambiguous: 0,
        unlinked_self: 0,
        without_parent: 0,
        payload_declarations: 0,
        path_declarations: 0,
        edges: Vec::new(),
        sessions_without_parent: Vec::new(),
    };
    for item in input {
        let Some(decl) = &item.declaration else {
            out.without_parent += 1;
            out.sessions_without_parent.push(item.session.clone());
            continue;
        };
        out.declarations += 1;
        match decl.kind {
            ObservationKind::PayloadField => out.payload_declarations += 1,
            ObservationKind::PathDerived => out.path_declarations += 1,
        }

        // Exact id equality over every indexed session. The index's own
        // `parent_idx` is a first-match shortcut and is deliberately ignored.
        let candidates: Vec<SessionRef> = input
            .iter()
            .filter(|other| other.session.id == decl.declared_id)
            .map(|other| other.session.clone())
            .collect();

        let (status, target, reason) = match candidates.len() {
            0 => (
                Status::UnlinkedAbsent,
                None,
                "no indexed session has the declared parent id",
            ),
            1 if candidates[0].session_idx == item.session.session_idx => (
                Status::UnlinkedSelf,
                None,
                "the declared parent id is the source session itself",
            ),
            1 => (
                Status::Linked,
                Some(candidates[0].clone()),
                "exactly one indexed session has the declared parent id",
            ),
            _ => (
                Status::UnlinkedAmbiguous,
                None,
                "multiple indexed sessions share the declared parent id; no file-order or path-proximity tie-break is applied",
            ),
        };
        match status {
            Status::Linked => out.linked += 1,
            Status::UnlinkedAbsent => out.unlinked_absent += 1,
            Status::UnlinkedAmbiguous => out.unlinked_ambiguous += 1,
            Status::UnlinkedSelf => out.unlinked_self += 1,
        }
        out.edges.push(Edge {
            from: item.session.clone(),
            declaration: decl.clone(),
            status,
            target,
            candidates,
            reason,
        });
    }
    out
}
