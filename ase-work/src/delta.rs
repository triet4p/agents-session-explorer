//! Explicit before/after delta between two attempts of one confirmed task.
//!
//! This module is pure: it receives two already-projected T2 journals and
//! their dispatch-derived attempt identities. It never reads resolver input,
//! adapter claims or motif membership, and it never writes a source, index or
//! membership store. Attempt chronology comes from the dispatch parent record
//! timestamp with deterministic tie-breaks; attempt labels stay labels.
//!
//! Rule `omp-attempt-delta` / `attempt-delta-v1` pins the reported ordering,
//! count differences and change set. The counted turn/episode unit is the T2
//! projection's `omp-turn-episode` / `turn-episode-v1`.

use crate::model::*;
use ase_core::{err, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const DELTA_RULE: &str = "omp-attempt-delta";
pub const DELTA_VERSION: &str = "attempt-delta-v1";
pub const MEMBERSHIP_RULE: &str = "task-dispatch-provenance";
pub const MEMBERSHIP_VERSION: &str = "dispatch-v1";
pub const ORDER_BASIS: &str = concat!(
    "dispatch parent-record timestamp, then spawn_index, then dispatch_id, ",
    "then child session id; attempt labels are labels, not chronology"
);

/// The executing child of one admitted `dispatch-v1` join.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeltaAttemptRef {
    pub session_id: String,
    pub dispatch_id: String,
    pub project: String,
    pub task: String,
    pub attempt: String,
    pub parent_session_id: String,
    pub parent_span: String,
    pub child_span: String,
}

/// One side of the delta, reduced from a T2 projection. Full evidence stays
/// on the turn and boundary changes below; the side keeps the pinned header
/// and counts so a difference is attributable to an exact source byte range.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptSide {
    pub membership: DeltaAttemptRef,
    pub source_id: String,
    pub file_id: u32,
    pub path: String,
    pub sha256: String,
    pub generation: String,
    pub active_leaf: LeafSummary,
    pub counts: Counts,
}

impl AttemptSide {
    pub fn new(membership: DeltaAttemptRef, projection: &JournalProjection) -> Self {
        Self {
            membership,
            source_id: projection.source_id.clone(),
            file_id: projection.file_id,
            path: projection.path.clone(),
            sha256: projection.sha256.clone(),
            generation: projection.generation.clone(),
            active_leaf: projection.active_leaf.clone(),
            counts: projection.counts.clone(),
        }
    }
}

/// Ordering input for one admitted attempt. `dispatch_ts_ms` is the timestamp
/// of the parent record that spawned it; labels are carried only for display
/// and for the reported label/chronology agreement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptOrderKey {
    pub session_id: String,
    pub dispatch_id: String,
    pub attempt: String,
    pub dispatch_timestamp: Option<String>,
    pub dispatch_ts_ms: i64,
    pub spawn_index: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderedAttempt {
    pub rank: u32,
    pub session_id: String,
    pub dispatch_id: String,
    pub attempt: String,
    pub dispatch_timestamp: Option<String>,
    pub spawn_index: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptOrder {
    pub basis: String,
    pub attempts: Vec<OrderedAttempt>,
    pub before_rank: u32,
    pub after_rank: u32,
    pub label_order_agrees: Option<bool>,
}

/// Sorts admitted attempts by time, never by attempt label, and locates the
/// two explicitly selected sides. A missing side is an error, not a default.
pub fn order_attempts(
    keys: &[AttemptOrderKey],
    before_session: &str,
    after_session: &str,
) -> Result<AttemptOrder> {
    let mut sorted: Vec<&AttemptOrderKey> = keys.iter().collect();
    sorted.sort_by(|left, right| {
        (
            left.dispatch_ts_ms,
            left.spawn_index,
            &left.dispatch_id,
            &left.session_id,
        )
        .cmp(&(
            right.dispatch_ts_ms,
            right.spawn_index,
            &right.dispatch_id,
            &right.session_id,
        ))
    });
    let rank_of = |session: &str| -> Option<u32> {
        sorted
            .iter()
            .position(|key| key.session_id == session)
            .map(|index| index as u32 + 1)
    };
    let before_rank = rank_of(before_session).ok_or_else(|| {
        err(format!("before session is not in the admitted task membership: {before_session}"))
    })?;
    let after_rank = rank_of(after_session).ok_or_else(|| {
        err(format!("after session is not in the admitted task membership: {after_session}"))
    })?;
    let before_key = sorted[(before_rank - 1) as usize];
    let after_key = sorted[(after_rank - 1) as usize];
    let label_order_agrees = match (
        before_key.attempt.parse::<u64>(),
        after_key.attempt.parse::<u64>(),
    ) {
        (Ok(left), Ok(right)) => Some((left < right) == (before_rank < after_rank)),
        _ => None,
    };
    Ok(AttemptOrder {
        basis: ORDER_BASIS.to_string(),
        attempts: sorted
            .iter()
            .enumerate()
            .map(|(index, key)| OrderedAttempt {
                rank: index as u32 + 1,
                session_id: key.session_id.clone(),
                dispatch_id: key.dispatch_id.clone(),
                attempt: key.attempt.clone(),
                dispatch_timestamp: key.dispatch_timestamp.clone(),
                spawn_index: key.spawn_index,
            })
            .collect(),
        before_rank,
        after_rank,
        label_order_agrees,
    })
}

/// Named projection metrics exposed in a delta. The list is explicit so a
/// changed projection rule cannot silently redefine an existing difference.
pub fn count_metrics(counts: &Counts) -> BTreeMap<String, u64> {
    [
        ("records_total", counts.records_total),
        ("parse_errors", counts.parse_errors),
        ("conversation_nodes", counts.conversation_nodes),
        ("leaf_count", counts.leaf_count),
        ("branch_parent_count", counts.branch_parent_count),
        ("turns_all_branches", counts.turns_all_branches),
        ("turns_active_leaf", counts.turns_active_leaf),
        ("abandoned_turns", counts.abandoned_turns),
        ("turns_completed_active_leaf", counts.turns_completed_active_leaf),
        ("turns_failed_active_leaf", counts.turns_failed_active_leaf),
        ("turns_unknown_stop_active_leaf", counts.turns_unknown_stop_active_leaf),
        ("user_messages_primary", counts.user_messages_primary),
        ("user_boundaries_wide", counts.user_boundaries_wide),
        ("episodes_primary_count", counts.episodes_primary_count),
        ("episodes_primary_turn_sum", counts.episodes_primary_turn_sum),
        ("episodes_wide_count", counts.episodes_wide_count),
        ("episodes_wide_turn_sum", counts.episodes_wide_turn_sum),
        ("toolcall_blocks", counts.toolcall_blocks),
        ("toolresult_entries", counts.toolresult_entries),
        ("assistant_msgs_with_multiple_calls", counts.assistant_msgs_with_multiple_calls),
        ("max_calls_per_assistant", counts.max_calls_per_assistant),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_string(), value as u64))
    .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountDeltaEntry {
    pub before: u64,
    pub after: u64,
    pub difference: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
}

impl ChangeKind {
    fn rank(self) -> u8 {
        match self {
            ChangeKind::Added => 0,
            ChangeKind::Removed => 1,
            ChangeKind::Modified => 2,
        }
    }
}

/// A turn present on one or both sides. `entry_id` keys the comparison inside
/// one journal; the embedded evidence is the T2 selector/span/digest tuple.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnChange {
    pub change: ChangeKind,
    pub entry_id: String,
    pub ordinal: u32,
    pub before: Option<TurnProjection>,
    pub after: Option<TurnProjection>,
}

/// A primary or wide episode boundary change. Boundaries are keyed by kind,
/// boundary entry id and ordinal; a moved boundary appears as remove + add.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundaryChange {
    pub change: ChangeKind,
    pub kind: String,
    pub entry_id: Option<String>,
    pub ordinal: Option<u32>,
    pub before: Option<EvidenceRef>,
    pub after: Option<EvidenceRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptDelta {
    pub rule: String,
    pub rule_version: String,
    pub projection_rule: String,
    pub projection_rule_version: String,
    pub membership_rule: String,
    pub membership_rule_version: String,
    pub task_key: String,
    pub generation: String,
    pub order: AttemptOrder,
    pub before: AttemptSide,
    pub after: AttemptSide,
    pub counts: BTreeMap<String, CountDeltaEntry>,
    pub turns: Vec<TurnChange>,
    pub boundaries: Vec<BoundaryChange>,
    pub summary: String,
}

fn turn_equivalent(left: &TurnProjection, right: &TurnProjection) -> bool {
    left.timestamp == right.timestamp
        && left.stop_reason == right.stop_reason
        && left.outcome == right.outcome
        && left.tool_calls == right.tool_calls
        && left.episode_primary == right.episode_primary
        && left.episode_wide == right.episode_wide
        && left.evidence.line_digest == right.evidence.line_digest
        && left.evidence.index_checksum == right.evidence.index_checksum
        && left.evidence.event == right.evidence.event
        && left.evidence.ordinal == right.evidence.ordinal
        && left.evidence.span == right.evidence.span
}

fn boundary_key(kind: &str, episode: &EpisodeProjection) -> (String, Option<String>, Option<u32>) {
    (kind.to_string(), episode.boundary_entry_id.clone(), episode.boundary_ordinal)
}

fn boundary_evidence(left: &EpisodeProjection, right: &EpisodeProjection) -> bool {
    match (&left.evidence, &right.evidence) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.line_digest == b.line_digest
                && a.event == b.event
                && a.span == b.span
                && a.selector == b.selector
        }
        _ => false,
    }
}

fn turn_changes(
    before: &JournalProjection,
    after: &JournalProjection,
) -> Vec<TurnChange> {
    let left: BTreeMap<&str, &TurnProjection> = before
        .turns
        .iter()
        .map(|turn| (turn.entry_id.as_str(), turn))
        .collect();
    let right: BTreeMap<&str, &TurnProjection> = after
        .turns
        .iter()
        .map(|turn| (turn.entry_id.as_str(), turn))
        .collect();
    let mut keys: BTreeSet<&str> = BTreeSet::new();
    keys.extend(left.keys().copied());
    keys.extend(right.keys().copied());
    let mut out: Vec<TurnChange> = keys
        .into_iter()
        .filter_map(|entry_id| match (left.get(entry_id), right.get(entry_id)) {
            (Some(a), Some(b)) if turn_equivalent(a, b) => None,
            (Some(a), Some(b)) => Some(TurnChange {
                change: ChangeKind::Modified,
                entry_id: entry_id.to_string(),
                ordinal: b.ordinal,
                before: Some((*a).clone()),
                after: Some((*b).clone()),
            }),
            (Some(a), None) => Some(TurnChange {
                change: ChangeKind::Removed,
                entry_id: entry_id.to_string(),
                ordinal: a.ordinal,
                before: Some((*a).clone()),
                after: None,
            }),
            (None, Some(b)) => Some(TurnChange {
                change: ChangeKind::Added,
                entry_id: entry_id.to_string(),
                ordinal: b.ordinal,
                before: None,
                after: Some((*b).clone()),
            }),
            (None, None) => unreachable!("key came from one of the two maps"),
        })
        .collect();
    out.sort_by(|a, b| {
        (a.change.rank(), a.ordinal, &a.entry_id).cmp(&(b.change.rank(), b.ordinal, &b.entry_id))
    });
    out
}

fn boundary_changes(
    before: &JournalProjection,
    after: &JournalProjection,
) -> Vec<BoundaryChange> {
    fn collect<'a>(
        projection: &'a JournalProjection,
        kind: &'static str,
    ) -> BTreeMap<(String, Option<String>, Option<u32>), &'a EpisodeProjection> {
        projection
            .episodes_primary
            .iter()
            .filter(|_| kind == "primary")
            .chain(projection.episodes_wide.iter().filter(|_| kind == "wide"))
            .map(|episode| (boundary_key(kind, episode), episode))
            .collect::<BTreeMap<_, _>>()
    }
    let left = collect(before, "primary")
        .into_iter()
        .chain(collect(before, "wide"))
        .collect::<BTreeMap<_, _>>();
    let right = collect(after, "primary")
        .into_iter()
        .chain(collect(after, "wide"))
        .collect::<BTreeMap<_, _>>();
    let mut keys: BTreeSet<_> = BTreeSet::new();
    keys.extend(left.keys().cloned());
    keys.extend(right.keys().cloned());
    let mut out: Vec<BoundaryChange> = keys
        .into_iter()
        .filter_map(|key| {
            let (kind, entry_id, ordinal) = key.clone();
            match (left.get(&key), right.get(&key)) {
                (Some(a), Some(b)) if boundary_evidence(a, b) => None,
                (Some(a), Some(b)) => Some(BoundaryChange {
                    change: ChangeKind::Modified,
                    kind,
                    entry_id,
                    ordinal,
                    before: a.evidence.clone(),
                    after: b.evidence.clone(),
                }),
                (Some(a), None) => Some(BoundaryChange {
                    change: ChangeKind::Removed,
                    kind,
                    entry_id,
                    ordinal,
                    before: a.evidence.clone(),
                    after: None,
                }),
                (None, Some(b)) => Some(BoundaryChange {
                    change: ChangeKind::Added,
                    kind,
                    entry_id,
                    ordinal,
                    before: None,
                    after: b.evidence.clone(),
                }),
                (None, None) => unreachable!("key came from one of the two maps"),
            }
        })
        .collect();
    out.sort_by(|a, b| {
        (a.change.rank(), &a.kind, &a.ordinal, &a.entry_id).cmp(&(
            b.change.rank(),
            &b.kind,
            &b.ordinal,
            &b.entry_id,
        ))
    });
    out
}

/// Builds the user-visible delta. `before` and `after` must be projections of
/// two distinct attempts of the same admitted `(project, task)` key; callers
/// enforce that membership before reaching this function.
pub fn attempt_delta(
    before_projection: &JournalProjection,
    after_projection: &JournalProjection,
    before_membership: DeltaAttemptRef,
    after_membership: DeltaAttemptRef,
    order: AttemptOrder,
) -> AttemptDelta {
    let before = AttemptSide::new(before_membership, before_projection);
    let after = AttemptSide::new(after_membership, after_projection);
    let left_counts = count_metrics(&before.counts);
    let right_counts = count_metrics(&after.counts);
    let mut counts: BTreeMap<String, CountDeltaEntry> = BTreeMap::new();
    for name in left_counts.keys().chain(right_counts.keys()) {
        let a = left_counts.get(name).copied().unwrap_or(0);
        let b = right_counts.get(name).copied().unwrap_or(0);
        counts.insert(
            name.clone(),
            CountDeltaEntry { before: a, after: b, difference: b as i64 - a as i64 },
        );
    }
    let turns = turn_changes(before_projection, after_projection);
    let boundaries = boundary_changes(before_projection, after_projection);
    let added = turns.iter().filter(|t| t.change == ChangeKind::Added).count();
    let removed = turns.iter().filter(|t| t.change == ChangeKind::Removed).count();
    let modified = turns.iter().filter(|t| t.change == ChangeKind::Modified).count();
    let turns_diff = counts["turns_active_leaf"].difference;
    let primary_diff = counts["episodes_primary_count"].difference;
    let wide_diff = counts["episodes_wide_count"].difference;
    let summary = format!(
        "order {} (#{}) -> {} (#{}); turns_active_leaf {} -> {} ({:+}); episodes_primary {} -> {} ({:+}); episodes_wide {:+} turns added/removed/modified {}/{}/{}",
        order.attempts[(order.before_rank - 1) as usize].session_id,
        order.before_rank,
        order.attempts[(order.after_rank - 1) as usize].session_id,
        order.after_rank,
        counts["turns_active_leaf"].before,
        counts["turns_active_leaf"].after,
        turns_diff,
        counts["episodes_primary_count"].before,
        counts["episodes_primary_count"].after,
        primary_diff,
        wide_diff,
        added,
        removed,
        modified,
    );
    AttemptDelta {
        rule: DELTA_RULE.to_string(),
        rule_version: DELTA_VERSION.to_string(),
        projection_rule: WORK_RULE.to_string(),
        projection_rule_version: WORK_VERSION.to_string(),
        membership_rule: MEMBERSHIP_RULE.to_string(),
        membership_rule_version: MEMBERSHIP_VERSION.to_string(),
        task_key: format!("{}|{}", before.membership.project, before.membership.task),
        generation: before.generation.clone(),
        order,
        before,
        after,
        counts,
        turns,
        boundaries,
        summary,
    }
}
