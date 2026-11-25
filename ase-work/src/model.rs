//! Turn/Episode projection vocabulary for OMP journals.
//!
//! The projection is task-agnostic: a session with no resolved task key is
//! projected exactly like an assigned one. It never writes resolver or motif
//! membership and never calls a model.
//!
//! Rule `omp-turn-episode` / `turn-episode-v1` pins the counting unit and
//! branch rule described by the released reference identifier in
//! `COMPATIBLE_REFERENCE_RULE`. Evidence selectors are versioned separately so
//! a changed extraction requires a new version.

use ase_core::SpanRef;
use serde::{Deserialize, Serialize};

pub const WORK_RULE: &str = "omp-turn-episode";
pub const WORK_VERSION: &str = "turn-episode-v1";
/// Selector/decoder namespace for turn, boundary and leaf evidence.
pub const EVIDENCE_VERSION: &str = "work-evidence-v1";
/// Legacy rule identifier published by `v0.4.0`. It is quoted verbatim only so
/// consumers can compare the released compatibility field; do not propagate
/// this spelling into new names, docs or contracts.
pub const COMPATIBLE_REFERENCE_RULE: &str = "s4t1-turn-v1";
pub const ADAPTER_NAME: &str = "omp-stock";

pub const SEL_TURN: &str = "assistant-usage-turn";
pub const SEL_USER: &str = "user-message-boundary";
pub const SEL_CUSTOM_USER: &str = "custom-user-boundary";
pub const SEL_LEAF: &str = "active-leaf";
/// Stored runtime facts for one assistant-usage turn. Extracted from the same
/// indexed line that backs the turn evidence, so leaf membership and spans
/// stay identical. Missing producer fields remain `None`: analytics reports
/// coverage instead of substituting zero or an inferred price.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeTurn {
    pub ordinal: u32,
    pub entry_id: String,
    pub on_active_leaf: bool,
    pub model: Option<String>,
    /// Stored `message.usage.cost.total` (nominal producer-catalog USD).
    pub cost_total_usd: Option<f64>,
    /// `message.timestamp` epoch-ms: the pinned T1 period field.
    pub msg_ts_ms: Option<i64>,
    /// `message.completedAt` epoch-ms.
    pub completed_ms: Option<i64>,
    /// `message.duration` in milliseconds (a float in producer data).
    pub duration_ms: Option<f64>,
    /// `message.ttft` in milliseconds.
    pub ttft_ms: Option<f64>,
    pub evidence: EvidenceRef,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    pub start_ms: i64,
    pub end_ms: i64,
}

impl Window {
    pub fn contains(self, ts_ms: i64) -> bool {
        ts_ms >= self.start_ms && ts_ms < self.end_ms
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpanJson {
    pub file: u32,
    pub off: u64,
    pub len: u32,
}

impl From<SpanRef> for SpanJson {
    fn from(s: SpanRef) -> Self {
        Self { file: s.file, off: s.off, len: s.len }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub provider: String,
    pub generation: String,
    pub coordinate: String,
    pub file_id: u32,
    pub path: String,
    pub event: u32,
    pub ordinal: u32,
    pub selector: String,
    pub selector_version: String,
    pub span: SpanJson,
    /// FNV-1a checksum of the exact source line, as stored by the address index.
    pub index_checksum: String,
    /// SHA-256 of the exact source line bytes.
    pub line_digest: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub records_total: u32,
    pub blank_lines: u32,
    pub parse_errors: u32,
    pub conversation_nodes: u32,
    pub duplicate_ids: u32,
    pub duplicate_id_occurrences: u32,
    pub missing_parent_refs: u32,
    pub cycles: u32,
    pub leaf_count: u32,
    pub branch_parent_count: u32,
    pub leaf_selection_disagreement: u32,
    pub turns_all_branches: u32,
    pub turns_active_leaf: u32,
    pub abandoned_turns: u32,
    pub turns_completed_active_leaf: u32,
    pub turns_failed_active_leaf: u32,
    pub turns_unknown_stop_active_leaf: u32,
    pub turns_entry_ts_in_window_all: u32,
    pub turns_entry_ts_in_window_active_leaf: u32,
    pub turn_ts_missing: u32,
    pub assistant_no_usage: u32,
    pub assistant_usage_empty: u32,
    pub user_messages_primary: u32,
    pub user_boundaries_wide: u32,
    pub aux_model_usage_records: u32,
    pub developer_messages: u32,
    pub file_mention_messages: u32,
    pub custom_role_messages: u32,
    pub toolcall_blocks: u32,
    pub toolresult_entries: u32,
    pub toolresults_unmatched: u32,
    pub toolcalls_without_result: u32,
    pub assistant_msgs_with_multiple_calls: u32,
    pub max_calls_per_assistant: u32,
    pub episodes_primary_count: u32,
    pub episodes_primary_empty: u32,
    pub episodes_primary_preuser_turns: u32,
    pub episodes_primary_turn_sum: u32,
    pub episodes_wide_count: u32,
    pub episodes_wide_empty: u32,
    pub episodes_wide_preuser_turns: u32,
    pub episodes_wide_turn_sum: u32,
    pub leaf_turns_with_missing_timestamp: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderInfo {
    pub session_id: Option<String>,
    pub timestamp: Option<String>,
    pub parent_session: Option<String>,
    pub header_ok: bool,
    pub missing_session_header: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeafSummary {
    pub entry_id: Option<String>,
    pub ordinal: Option<u32>,
    pub timestamp: Option<String>,
    pub source_type: Option<String>,
    pub selection: String,
    pub is_graph_leaf: bool,
    pub graph_leaf_count: u32,
    pub path_node_count: u32,
    pub evidence: Option<EvidenceRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathNode {
    pub ordinal: u32,
    pub entry_id: String,
    pub source_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnProjection {
    pub seq: u32,
    pub entry_id: String,
    pub ordinal: u32,
    pub timestamp: Option<String>,
    pub stop_reason: Option<String>,
    pub outcome: String,
    pub tool_calls: u32,
    pub episode_primary: Option<u32>,
    pub episode_wide: Option<u32>,
    pub evidence: EvidenceRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpisodeProjection {
    pub index: u32,
    pub boundary_kind: String,
    pub boundary_entry_id: Option<String>,
    pub boundary_ordinal: Option<u32>,
    pub turn_start: Option<u32>,
    pub turn_count: u32,
    pub evidence: Option<EvidenceRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JournalProjection {
    pub adapter: String,
    pub rule: String,
    pub rule_version: String,
    pub compatible_reference_rule: String,
    pub generation: String,
    pub source_id: String,
    pub file_id: u32,
    pub path: String,
    pub sha256: String,
    #[serde(default)]
    pub event_base: u32,
    #[serde(default)]
    pub window: Option<Window>,
    pub session_id: Option<String>,
    pub header: HeaderInfo,
    pub active_leaf: LeafSummary,
    pub requested_leaf: Option<String>,
    pub path_nodes: Vec<PathNode>,
    pub turns: Vec<TurnProjection>,
    pub episodes_primary: Vec<EpisodeProjection>,
    pub episodes_wide: Vec<EpisodeProjection>,
    pub counts: Counts,

    /// Additive runtime facts for every assistant-usage turn (both
    /// branches). Older projections decode with an empty vector.
    #[serde(default)]
    pub runtime: Vec<RuntimeTurn>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalRow {
    pub source_id: String,
    pub sha256: String,
    pub session_id: Option<String>,
    pub kind: String,
    pub tree_id: String,
    pub header_timestamp: Option<String>,
    pub header_ok: bool,
    pub leaf_entry_id: Option<String>,
    pub leaf_ordinal: Option<u32>,
    pub leaf_timestamp: Option<String>,
    pub leaf_type: Option<String>,
    pub leaf_count: u32,
    pub turns_all_branches: u32,
    pub turns_active_leaf: u32,
    pub abandoned_turns: u32,
    pub episodes_primary_count: u32,
    pub episodes_wide_count: u32,
    pub parse_errors: u32,
    pub missing_parent_refs: u32,
    pub cycles: u32,
}

impl JournalRow {
    pub fn from_projection(p: &JournalProjection, kind: &str, tree_id: &str) -> Self {
        Self {
            source_id: p.source_id.clone(),
            sha256: p.sha256.clone(),
            session_id: p.session_id.clone(),
            kind: kind.to_string(),
            tree_id: tree_id.to_string(),
            header_timestamp: p.header.timestamp.clone(),
            header_ok: p.header.header_ok,
            leaf_entry_id: p.active_leaf.entry_id.clone(),
            leaf_ordinal: p.active_leaf.ordinal,
            leaf_timestamp: p.active_leaf.timestamp.clone(),
            leaf_type: p.active_leaf.source_type.clone(),
            leaf_count: p.counts.leaf_count,
            turns_all_branches: p.counts.turns_all_branches,
            turns_active_leaf: p.counts.turns_active_leaf,
            abandoned_turns: p.counts.abandoned_turns,
            episodes_primary_count: p.counts.episodes_primary_count,
            episodes_wide_count: p.counts.episodes_wide_count,
            parse_errors: p.counts.parse_errors,
            missing_parent_refs: p.counts.missing_parent_refs,
            cycles: p.counts.cycles,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AggregateCounts {
    pub files: u64,
    pub bytes: u64,
    pub root_files: u64,
    pub subagent_files: u64,
    pub turns_all_branches: u64,
    pub turns_active_leaf: u64,
    pub abandoned_turns: u64,
    pub episodes_primary_count: u64,
    pub episodes_wide_count: u64,
    pub parse_errors: u64,
    pub missing_parent_refs: u64,
    pub cycles: u64,
    pub leaf_null: u64,
}

impl AggregateCounts {
    pub fn add(&mut self, row: &JournalRow, bytes: u64) {
        self.files += 1;
        self.bytes += bytes;
        if row.kind == "root" {
            self.root_files += 1;
        } else {
            self.subagent_files += 1;
        }
        self.turns_all_branches += row.turns_all_branches as u64;
        self.turns_active_leaf += row.turns_active_leaf as u64;
        self.abandoned_turns += row.abandoned_turns as u64;
        self.episodes_primary_count += row.episodes_primary_count as u64;
        self.episodes_wide_count += row.episodes_wide_count as u64;
        self.parse_errors += row.parse_errors as u64;
        self.missing_parent_refs += row.missing_parent_refs as u64;
        self.cycles += row.cycles as u64;
        if row.leaf_entry_id.is_none() {
            self.leaf_null += 1;
        }
    }
}

/// One pinned journal entry, shaped exactly like the T1 manifest entry so the
/// product can independently reproduce the locked manifest digest.
#[derive(Clone, Debug)]
pub struct ManifestEntry {
    pub relpath: String,
    pub size: u64,
    pub sha256: String,
    pub head_sha256: String,
    pub tail_sha256: String,
    pub kind: String,
    pub tree_relpath: String,
    pub session_id: Option<String>,
    pub header_timestamp: Option<String>,
    pub parent_session: Option<String>,
    pub filename_timestamp: Option<String>,
    pub header_ok: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CohortSnapshot {
    pub manifest_sha256: String,
    pub files: u64,
    pub root_files: u64,
    pub subagent_files: u64,
    pub bytes: u64,
    pub root_journals_scanned: u64,
    pub root_journals_selected: u64,
    pub root_journals_outside_window: u64,
    pub root_journals_missing_header: u64,
    pub filename_header_mismatches: u64,
    pub subagent_headers_missing: u64,
}
