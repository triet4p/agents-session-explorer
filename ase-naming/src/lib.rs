//! Versioned file-name grammar and naming audit.
//!
//! This crate reads names and source coordinates only. It never opens a
//! transcript, never decodes prose, and never calls a model. Ambiguous splits
//! are kept as candidates with an explicit abstention; they are never forced
//! into a task identity.

/// Stable identifier of the derivation rule recorded on every assertion.
pub const RULE_NAME: &str = "file-name-grammar";
/// Bump when the meaning of any field or split changes.
pub const RULE_VERSION: &str = "naming-v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRef {
    pub file_id: u32,
    pub path: String,
    pub provider: String,
    pub session_id: String,
    /// Pointer to the session header event, metadata only (no prose is read).
    pub span_anchor: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub project: String,
    pub task: String,
    pub role: String,
    pub attempt: String,
    /// `marker:Task`, `marker:T` or `digit-boundary` with the exact slot.
    pub split_rule: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Parsed,
    Ambiguous,
    Unparsed,
}

#[derive(Clone, Debug)]
pub struct Signal {
    pub source: SourceRef,
    pub basename: String,
    pub parent: String,
    pub status: Status,
    pub reason: String,
    pub candidates: Vec<Candidate>,
    /// Longest character prefix shared with any sibling in the same parent.
    pub shared_prefix: String,
    pub sibling_count: usize,
}

#[derive(Clone, Debug)]
pub struct NameInCluster {
    pub basename: String,
    pub candidate: Candidate,
    pub source: SourceRef,
}

#[derive(Clone, Debug)]
pub struct Cluster {
    pub project: String,
    pub parent: String,
    pub common_prefix: String,
    pub names: Vec<NameInCluster>,
}

#[derive(Clone, Debug)]
pub struct Tie {
    pub parent: String,
    pub basename: String,
    pub candidates: Vec<Candidate>,
    pub decision: &'static str,
    pub reason: String,
    pub source: SourceRef,
}

#[derive(Clone, Debug)]
pub struct Unparsed {
    pub parent: String,
    pub basename: String,
    pub reason: String,
    pub source: SourceRef,
}

#[derive(Clone, Debug)]
pub struct Collision {
    pub kind: &'static str,
    pub left: String,
    pub right: String,
    pub note: String,
}

#[derive(Clone, Debug)]
pub struct RuleInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub attempt_semantics: String,
    pub abstention: String,
}

impl Default for RuleInfo {
    fn default() -> Self {
        Self {
            name: RULE_NAME.to_string(),
            version: RULE_VERSION.to_string(),
            description: "Within a parent directory, split a name into project + task + role + attempt. An explicit `Task`/`T` marker between project and task is decisive; without a marker every digit boundary is kept and more than one candidate is an abstention.".to_string(),
            attempt_semantics: "Trailing number is a label, not an ordering. No attempt chronology or newest-attempt claim is derived here.".to_string(),
            abstention: "Conflicting or multiple plausible splits stay ambiguous/unparsed with all evidence; they are never promoted to a task key.".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Coverage {
    pub names: usize,
    pub parsed: usize,
    pub ambiguous: usize,
    pub unparsed: usize,
}

#[derive(Clone, Debug)]
pub struct Audit {
    pub rule: RuleInfo,
    pub coverage: Coverage,
    pub signals: Vec<Signal>,
    pub clusters: Vec<Cluster>,
    pub ties: Vec<Tie>,
    pub unparsed: Vec<Unparsed>,
    pub collisions: Vec<Collision>,
}

enum ParseOutcome {
    One(Candidate),
    Many(Vec<Candidate>),
    None(String),
}

/// Remove only the two source suffixes understood by the indexer.
pub fn strip_extension(name: &str) -> &str {
    if let Some(s) = name.strip_suffix(".jsonl.zst") {
        s
    } else if let Some(s) = name.strip_suffix(".jsonl") {
        s
    } else {
        name
    }
}

fn lcp(a: &str, b: &str) -> String {
    let mut end = 0usize;
    for ((i, ca), cb) in a.char_indices().zip(b.chars()) {
        if ca != cb {
            break;
        }
        end = i + ca.len_utf8();
    }
    a[..end].to_string()
}

fn is_project(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().all(|b| b.is_ascii_alphanumeric())
        && s.bytes().any(|b| b.is_ascii_alphabetic())
        && s.bytes().any(|b| b.is_ascii_digit())
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Split one basename. No corpus statistics and no hard-coded project list:
/// evidence for a split comes only from the name and its sibling names.
fn parse_name(name: &str) -> ParseOutcome {
    let bytes = name.as_bytes();
    let mut attempt_start = name.len();
    while attempt_start > 0 && bytes[attempt_start - 1].is_ascii_digit() {
        attempt_start -= 1;
    }
    if attempt_start == name.len() {
        return ParseOutcome::None("missing_attempt_number".to_string());
    }
    let attempt = &name[attempt_start..];
    let head = &name[..attempt_start];

    let mut role_start = head.len();
    let hb = head.as_bytes();
    while role_start > 0 && hb[role_start - 1].is_ascii_alphabetic() {
        role_start -= 1;
    }
    if role_start == head.len() {
        return ParseOutcome::None("missing_role".to_string());
    }
    let role = &head[role_start..];
    let body = &head[..role_start];
    if body.is_empty() {
        return ParseOutcome::None("missing_task".to_string());
    }

    // A marker before the task digits removes the digit-boundary ambiguity.
    let mut marker_candidates = Vec::new();
    for marker in ["Task", "T"] {
        if let Some((prefix, rest)) = body.rsplit_once(marker) {
            if is_project(prefix) && all_digits(rest) {
                marker_candidates.push(Candidate {
                    project: prefix.to_string(),
                    task: rest.to_string(),
                    role: role.to_string(),
                    attempt: attempt.to_string(),
                    split_rule: format!("marker:{marker}"),
                });
            }
        }
    }
    match marker_candidates.len() {
        1 => return ParseOutcome::One(marker_candidates.remove(0)),
        n if n > 1 => return ParseOutcome::Many(marker_candidates),
        _ => {}
    }

    // No marker: keep every admissible boundary. More than one means a tie.
    let mut boundary_candidates = Vec::new();
    for (i, ch) in body.char_indices() {
        if !ch.is_ascii_digit() {
            continue;
        }
        let prefix = &body[..i];
        let task = &body[i..];
        if is_project(prefix) && all_digits(task) {
            boundary_candidates.push(Candidate {
                project: prefix.to_string(),
                task: task.to_string(),
                role: role.to_string(),
                attempt: attempt.to_string(),
                split_rule: format!("digit-boundary@{i}"),
            });
        }
    }
    match boundary_candidates.len() {
        1 => ParseOutcome::One(boundary_candidates.remove(0)),
        n if n > 1 => ParseOutcome::Many(boundary_candidates),
        _ => ParseOutcome::None("project_or_task_missing".to_string()),
    }
}

fn parent_of(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    match normalized.rfind('/') {
        Some(i) => normalized[..i].to_string(),
        None => String::new(),
    }
}

fn file_name_of(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    normalized
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_string()
}

struct Entry {
    source: SourceRef,
    basename: String,
    parent: String,
    shared_prefix: String,
    sibling_count: usize,
    status: Status,
    reason: String,
    candidates: Vec<Candidate>,
}

/// Task/role/attempt label only: project and parent are scope, not label.
fn label_key(c: &Candidate) -> String {
    format!("{}|{}|{}", c.task, c.role, c.attempt)
}

/// Derive signals and coverage from indexed source names and coordinates.
pub fn audit(sources: &[SourceRef]) -> Audit {
    let mut entries: Vec<Entry> = sources
        .iter()
        .map(|s| {
            let basename = file_name_of(&s.path);
            let parent = parent_of(&s.path);
            Entry {
                source: s.clone(),
                basename,
                parent,
                shared_prefix: String::new(),
                sibling_count: 0,
                status: Status::Unparsed,
                reason: String::new(),
                candidates: Vec::new(),
            }
        })
        .collect();
    entries.sort_by(|a, b| {
        a.parent
            .cmp(&b.parent)
            .then_with(|| a.basename.cmp(&b.basename))
            .then_with(|| a.source.file_id.cmp(&b.source.file_id))
    });

    for i in 0..entries.len() {
        let mut best = String::new();
        let mut count = 0usize;
        for j in 0..entries.len() {
            if i == j || entries[i].parent != entries[j].parent {
                continue;
            }
            let candidate = lcp(&entries[i].basename, &entries[j].basename);
            if candidate.len() > best.len() {
                best = candidate;
                count = 1;
            } else if !best.is_empty() && candidate == best {
                count += 1;
            }
        }
        entries[i].shared_prefix = best;
        entries[i].sibling_count = count;

        let stem = strip_extension(&entries[i].basename);
        match parse_name(stem) {
            ParseOutcome::One(c) => {
                entries[i].status = Status::Parsed;
                entries[i].reason = c.split_rule.clone();
                entries[i].candidates.push(c);
            }
            ParseOutcome::Many(c) => {
                entries[i].status = Status::Ambiguous;
                entries[i].reason = "multiple_plausible_splits".to_string();
                entries[i].candidates = c;
            }
            ParseOutcome::None(reason) => {
                entries[i].status = Status::Unparsed;
                entries[i].reason = reason;
            }
        }
    }

    let mut clusters: Vec<Cluster> = Vec::new();
    let mut ties = Vec::new();
    let mut unparsed = Vec::new();
    for e in &entries {
        match e.status {
            Status::Parsed => {
                let c = &e.candidates[0];
                if let Some(cluster) = clusters.iter_mut().find(|cl| {
                    cl.project == c.project && cl.parent == e.parent
                }) {
                    cluster.names.push(NameInCluster {
                        basename: e.basename.clone(),
                        candidate: c.clone(),
                        source: e.source.clone(),
                    });
                } else {
                    clusters.push(Cluster {
                        project: c.project.clone(),
                        parent: e.parent.clone(),
                        common_prefix: e.shared_prefix.clone(),
                        names: vec![NameInCluster {
                            basename: e.basename.clone(),
                            candidate: c.clone(),
                            source: e.source.clone(),
                        }],
                    });
                }
            }
            Status::Ambiguous => ties.push(Tie {
                parent: e.parent.clone(),
                basename: e.basename.clone(),
                candidates: e.candidates.clone(),
                decision: "abstain",
                reason: e.reason.clone(),
                source: e.source.clone(),
            }),
            Status::Unparsed => unparsed.push(Unparsed {
                parent: e.parent.clone(),
                basename: e.basename.clone(),
                reason: e.reason.clone(),
                source: e.source.clone(),
            }),
        }
    }
    for cluster in &mut clusters {
        let mut prefix = cluster.names[0].basename.clone();
        for name in &cluster.names[1..] {
            prefix = lcp(&prefix, &name.basename);
        }
        cluster.common_prefix = prefix;
    }

    // Collisions are reported, never merged: project and parent are part of
    // the task scope even when the remaining task label is identical.
    let mut collisions = Vec::new();
    let parsed: Vec<&Entry> = entries
        .iter()
        .filter(|e| e.status == Status::Parsed)
        .collect();
    for (i, a) in parsed.iter().enumerate() {
        for b in parsed.iter().skip(i + 1) {
            let ca = &a.candidates[0];
            let cb = &b.candidates[0];
            if a.parent == b.parent && ca.project != cb.project && label_key(ca) == label_key(cb) {
                collisions.push(Collision {
                    kind: "cross_project_same_task",
                    left: format!("{}@{}", a.basename, a.parent),
                    right: format!("{}@{}", b.basename, b.parent),
                    note: format!("{} vs {} kept separate", ca.project, cb.project),
                });
            }
            if a.parent != b.parent && ca.project == cb.project && label_key(ca) == label_key(cb) {
                collisions.push(Collision {
                    kind: "same_name_other_parent",
                    left: format!("{}@{}", a.basename, a.parent),
                    right: format!("{}@{}", b.basename, b.parent),
                    note: "parent is part of the scope; no cross-parent merge".to_string(),
                });
            }
        }
    }

    let signals: Vec<Signal> = entries
        .into_iter()
        .map(|e| Signal {
            source: e.source,
            basename: e.basename,
            parent: e.parent,
            status: e.status,
            reason: e.reason,
            candidates: e.candidates,
            shared_prefix: e.shared_prefix,
            sibling_count: e.sibling_count,
        })
        .collect();

    let parsed_count = signals
        .iter()
        .filter(|s| s.status == Status::Parsed)
        .count();
    let ambiguous = signals
        .iter()
        .filter(|s| s.status == Status::Ambiguous)
        .count();
    let unparsed_count = signals.len() - parsed_count - ambiguous;

    Audit {
        rule: RuleInfo::default(),
        coverage: Coverage {
            names: signals.len(),
            parsed: parsed_count,
            ambiguous,
            unparsed: unparsed_count,
        },
        signals,
        clusters,
        ties,
        unparsed,
        collisions,
    }
}
