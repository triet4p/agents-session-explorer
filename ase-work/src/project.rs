//! Leaf-aware Turn/Episode projection for one OMP journal.
//!
//! The unit and branch rule are the locked Sprint-4 T1 reference
//! (`s4t1-turn-v1`): a turn is an assistant `message` entry carrying a
//! `message.usage` object; the active leaf is the graph leaf with the greatest
//! `(entry timestamp, ordinal)`; the active path walks `parentId` links.
//! Episodes are runs of active-path turns between user boundaries.
//!
//! The projection never reads resolver or motif membership and never requires
//! a task key: unresolved and no-task sessions project identically.

use ase_core::{err, SpanRef};
use serde::Deserialize;
use serde_json::value::RawValue;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use crate::model::*;
use crate::util::{fnv64_hex, format_iso_ms, parse_ts_opt, sha256_hex};

#[derive(Clone, Copy, Debug)]
pub struct ProjectOptions<'a> {
    pub file_id: u32,
    pub path: &'a str,
    pub source_id: &'a str,
    pub generation: &'a str,
    pub sha256: &'a str,
    pub event_base: u32,
    pub explicit_leaf: Option<&'a str>,
    pub window: Option<Window>,
}

#[derive(Deserialize)]
struct RawRecord<'a> {
    #[serde(rename = "type")]
    ty: Option<Cow<'a, str>>,
    id: Option<Cow<'a, str>>,
    #[serde(rename = "parentId")]
    parent_id: Option<Cow<'a, str>>,
    #[serde(rename = "parentSession")]
    parent_session: Option<Cow<'a, str>>,
    timestamp: Option<Cow<'a, str>>,
    attribution: Option<Cow<'a, str>>,
    #[serde(borrow)]
    message: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct RawMessage<'a> {
    role: Option<&'a str>,
    content: Option<&'a RawValue>,
    usage: Option<&'a RawValue>,
    #[serde(rename = "stopReason")]
    stop_reason: Option<&'a str>,
    #[serde(rename = "toolCallId")]
    tool_call_id: Option<&'a str>,
    model: Option<&'a str>,
    #[serde(rename = "timestamp")]
    msg_ts: Option<serde_json::Number>,
    #[serde(rename = "completedAt")]
    completed_at: Option<serde_json::Number>,
    duration: Option<serde_json::Number>,
    ttft: Option<serde_json::Number>,
}

#[derive(Deserialize)]
struct RawBlock<'a> {
    #[serde(rename = "type")]
    ty: Option<&'a str>,
    id: Option<&'a str>,
    #[serde(rename = "toolCallId")]
    tool_call_id: Option<&'a str>,
}

struct Node {
    ordinal: u32,
    source_type: String,
    id: String,
    parent: Option<String>,
    role: Option<String>,
    usage: bool,
    usage_nonempty: bool,
    stop: Option<String>,
    attribution: Option<String>,
    ts_ms: Option<i64>,
    model: Option<String>,
    cost_total_usd: Option<f64>,
    msg_ts_ms: Option<i64>,
    completed_ms: Option<i64>,
    duration_ms: Option<f64>,
    ttft_ms: Option<f64>,
    tool_calls: Vec<String>,
    tool_result_id: Option<String>,
    event: u32,
    span: SpanRef,
    line_digest: String,
    index_checksum: String,
}

impl Node {
    fn is_turn(&self) -> bool {
        self.source_type == "message" && self.role.as_deref() == Some("assistant") && self.usage
    }

    fn is_primary_boundary(&self) -> bool {
        self.source_type == "message" && self.role.as_deref() == Some("user")
    }

    fn is_wide_boundary(&self) -> bool {
        self.is_primary_boundary()
            || (self.source_type == "custom_message" && self.attribution.as_deref() == Some("user"))
    }
}

fn evidence_for(opts: &ProjectOptions<'_>, node: &Node, selector: &str) -> EvidenceRef {
    EvidenceRef {
        provider: "omp".to_string(),
        generation: opts.generation.to_string(),
        coordinate: "raw_file_bytes".to_string(),
        file_id: opts.file_id,
        path: opts.path.to_string(),
        event: node.event,
        ordinal: node.ordinal,
        selector: format!("{EVIDENCE_VERSION}|{selector}"),
        selector_version: EVIDENCE_VERSION.to_string(),
        span: node.span.into(),
        index_checksum: node.index_checksum.clone(),
        line_digest: node.line_digest.clone(),
    }
}

fn stop_bucket(stop: Option<&str>) -> (&'static str, &'static str) {
    match stop {
        Some("stop") | Some("toolUse") => ("completed", "turns_completed_active_leaf"),
        Some("error") | Some("aborted") | Some("length") => {
            ("failed", "turns_failed_active_leaf")
        }
        _ => ("unknown", "turns_unknown_stop_active_leaf"),
    }
}

fn parse_message(raw: Option<&RawValue>) -> RawMessage<'_> {
    raw.and_then(|value| serde_json::from_str::<RawMessage<'_>>(value.get()).ok())
        .unwrap_or(RawMessage { role: None, content: None, usage: None, stop_reason: None, tool_call_id: None, model: None, msg_ts: None, completed_at: None, duration: None, ttft: None })
}

fn tool_calls(content: Option<&RawValue>) -> Vec<String> {
    let Some(content) = content else { return Vec::new() };
    let Ok(blocks) = serde_json::from_str::<Vec<RawBlock<'_>>>(content.get()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for block in blocks {
        if block.ty != Some("toolCall") {
            continue;
        }
        let id = block.id.filter(|s| !s.is_empty()).or(block.tool_call_id);
        if let Some(id) = id.filter(|s| !s.is_empty()) {
            out.push(id.to_string());
        }
    }
    out
}

/// Projects one journal from its exact raw bytes. `bytes` are in the source
/// coordinate system used by the address index (raw file for JSONL).
pub fn project_bytes(opts: &ProjectOptions<'_>, bytes: &[u8]) -> ase_core::Result<JournalProjection> {
    let mut counts = Counts::default();
    let mut nodes: Vec<Node> = Vec::new();
    let mut header_raw: Option<(Option<String>, Option<String>, Option<String>)> = None;
    let mut event_ordinal = 0u32;
    let mut off = 0usize;

    for seg in bytes.split(|b| *b == b'\n') {
        if seg.is_empty() && off >= bytes.len() {
            break;
        }
        let raw_len = seg.len();
        let mut len = raw_len;
        if len > 0 && seg[len - 1] == b'\r' {
            len -= 1;
        }
        let line = &seg[..len];
        let span_off = off as u64;
        off += raw_len + 1;
        if line.iter().all(u8::is_ascii_whitespace) {
            counts.blank_lines += 1;
            continue;
        }
        let event = opts.event_base + event_ordinal;
        event_ordinal += 1;

        let Ok(record) = serde_json::from_slice::<RawRecord<'_>>(line) else {
            counts.parse_errors += 1;
            continue;
        };
        counts.records_total += 1;
        let source_type = record.ty.as_deref().unwrap_or("");
        let ts_ms = parse_ts_opt(record.timestamp.as_deref());
        if source_type == "session" && header_raw.is_none() {
            header_raw = Some((
                record.id.as_deref().map(str::to_string),
                ts_ms.map(format_iso_ms),
                record.parent_session.as_deref().map(str::to_string),
            ));
        }

        let Some(id) = record.id.as_deref().filter(|s| !s.is_empty()) else {
            continue;
        };
        if source_type == "session" {
            continue;
        }
        let message = parse_message(record.message);
        let role = message.role.map(str::to_string);
        let usage = message.usage.and_then(|v| serde_json::from_str::<serde_json::Value>(v.get()).ok());
        let usage_present = usage.as_ref().is_some_and(|v| v.is_object());
        let usage_nonempty = usage
            .as_ref()
            .and_then(|v| v.as_object())
            .is_some_and(|obj| !obj.is_empty());
        let cost_total_usd = usage
            .as_ref()
            .and_then(|v| v.get("cost"))
            .and_then(|c| c.get("total"))
            .and_then(serde_json::Value::as_f64)
            .filter(|v| v.is_finite());
        let finite_nonneg = |n: &serde_json::Number| {
            n.as_f64().filter(|v| v.is_finite() && *v >= 0.0)
        };
        let msg_ts_ms = message.msg_ts.as_ref().and_then(|n| n.as_i64());
        let completed_ms = message.completed_at.as_ref().and_then(|n| n.as_i64());
        let duration_ms = message.duration.as_ref().and_then(finite_nonneg);
        let ttft_ms = message.ttft.as_ref().and_then(finite_nonneg);
        let calls = tool_calls(message.content);
        nodes.push(Node {
            ordinal: nodes.len() as u32,
            source_type: source_type.to_string(),
            id: id.to_string(),
            parent: record.parent_id.as_deref().map(str::to_string),
            role,
            usage: usage_present,
            usage_nonempty,
            stop: message.stop_reason.map(str::to_string),
            attribution: record.attribution.as_deref().map(str::to_string),
            ts_ms,
            model: message.model.map(str::to_string),
            cost_total_usd,
            msg_ts_ms,
            completed_ms,
            duration_ms,
            ttft_ms,
            tool_calls: calls,
            tool_result_id: message.tool_call_id.map(str::to_string),
            event,
            span: SpanRef { file: opts.file_id, off: span_off, len: len as u32 },
            line_digest: sha256_hex(line),
            index_checksum: fnv64_hex(line),
        });
    }
    counts.conversation_nodes = nodes.len() as u32;

    // Duplicate ids and parent links keep T1's first-match convention; they
    // are counted, never silently dropped.
    let mut idmap: BTreeMap<String, usize> = BTreeMap::new();
    for (index, node) in nodes.iter().enumerate() {
        if idmap.contains_key(&node.id) {
            counts.duplicate_id_occurrences += 1;
        } else {
            idmap.insert(node.id.clone(), index);
        }
    }
    counts.duplicate_ids = counts.duplicate_id_occurrences;
    let mut children: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, node) in nodes.iter().enumerate() {
        if let Some(parent) = &node.parent {
            if idmap.contains_key(parent) {
                children.entry(parent.clone()).or_default().push(index);
            } else {
                counts.missing_parent_refs += 1;
            }
        }
    }
    counts.branch_parent_count = children.values().filter(|v| v.len() > 1).count() as u32;
    let child_parent_ids: BTreeSet<&str> = children.keys().map(String::as_str).collect();

    let mut leaves: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| !child_parent_ids.contains(node.id.as_str()))
        .map(|(index, _)| index)
        .collect();
    counts.leaf_count = leaves.len() as u32;

    let mut active_leaf_index: Option<usize> = None;
    if !leaves.is_empty() {
        leaves.sort_by_key(|index| nodes[*index].ordinal);
        let ordinal_leaf = *leaves.last().unwrap();
        let mut ts_leaf = leaves[0];
        for &candidate in &leaves {
            let best_key = (
                nodes[ts_leaf].ts_ms.unwrap_or(i64::MIN),
                nodes[ts_leaf].ordinal,
            );
            let candidate_key = (
                nodes[candidate].ts_ms.unwrap_or(i64::MIN),
                nodes[candidate].ordinal,
            );
            if candidate_key > best_key {
                ts_leaf = candidate;
            }
        }
        if nodes[ordinal_leaf].id != nodes[ts_leaf].id {
            counts.leaf_selection_disagreement = 1;
        }
        active_leaf_index = Some(ts_leaf);
    }

    let explicit_index = match opts.explicit_leaf {
        Some(requested) => {
            let found = idmap.get(requested).copied().ok_or_else(|| {
                err(format!("requested leaf entry id not found in journal: {requested}"))
            })?;
            Some(found)
        }
        None => None,
    };
    let selected_index = explicit_index.or(active_leaf_index);

    let mut path_indices: Vec<usize> = Vec::new();
    let mut is_graph_leaf = false;
    if let Some(selected) = selected_index {
        is_graph_leaf = !child_parent_ids.contains(nodes[selected].id.as_str());
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut current = selected;
        loop {
            path_indices.push(current);
            let Some(parent) = nodes[current].parent.clone() else { break };
            let Some(&parent_index) = idmap.get(&parent) else { break };
            if seen.contains(&parent) {
                counts.cycles = 1;
                break;
            }
            seen.insert(parent);
            current = parent_index;
        }
        path_indices.sort_by_key(|index| nodes[*index].ordinal);
    }
    let is_turn = |index: usize| nodes[index].is_turn();
    let all_turns: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(index, _)| is_turn(*index))
        .map(|(index, _)| index)
        .collect();
    let leaf_turn_indices: Vec<usize> = path_indices
        .iter()
        .copied()
        .filter(|index| is_turn(*index))
        .collect();
    counts.turns_all_branches = all_turns.len() as u32;
    counts.turns_active_leaf = leaf_turn_indices.len() as u32;
    counts.abandoned_turns =
        counts.turns_all_branches.saturating_sub(counts.turns_active_leaf);

    let mut turn_seq: BTreeMap<usize, u32> = BTreeMap::new();
    for (seq, index) in leaf_turn_indices.iter().enumerate() {
        turn_seq.insert(*index, seq as u32);
    }

    // P2 runtime facts ride on the same parse and the same P1 turn evidence.
    // `on_active_leaf` is the locked leaf path, so all-branch vs leaf
    // reconciliation cannot drift from the P1 projection.
    let runtime: Vec<RuntimeTurn> = all_turns
        .iter()
        .map(|index| {
            let node = &nodes[*index];
            RuntimeTurn {
                ordinal: node.ordinal,
                entry_id: node.id.clone(),
                on_active_leaf: turn_seq.contains_key(index),
                model: node.model.clone(),
                cost_total_usd: node.cost_total_usd,
                msg_ts_ms: node.msg_ts_ms,
                completed_ms: node.completed_ms,
                duration_ms: node.duration_ms,
                ttft_ms: node.ttft_ms,
                evidence: evidence_for(opts, node, SEL_TURN),
            }
        })
        .collect();
    for index in &all_turns {
        let node = &nodes[*index];
        match node.ts_ms {
            None => counts.turn_ts_missing += 1,
            Some(ts) => {
                if opts.window.is_some_and(|w| w.contains(ts)) {
                    counts.turns_entry_ts_in_window_all += 1;
                }
            }
        }
    }
    for index in &leaf_turn_indices {
        let node = &nodes[*index];
        let (_, key) = stop_bucket(node.stop.as_deref());
        match key {
            "turns_completed_active_leaf" => counts.turns_completed_active_leaf += 1,
            "turns_failed_active_leaf" => counts.turns_failed_active_leaf += 1,
            _ => counts.turns_unknown_stop_active_leaf += 1,
        }
        if node.ts_ms.is_none() {
            counts.leaf_turns_with_missing_timestamp += 1;
        } else if opts.window.is_some_and(|w| w.contains(node.ts_ms.unwrap())) {
            counts.turns_entry_ts_in_window_active_leaf += 1;
        }
    }

    counts.assistant_no_usage = nodes
        .iter()
        .filter(|n| n.source_type == "message" && n.role.as_deref() == Some("assistant") && !n.usage)
        .count() as u32;
    counts.assistant_usage_empty = nodes
        .iter()
        .filter(|n| {
            n.source_type == "message"
                && n.role.as_deref() == Some("assistant")
                && n.usage
                && !n.usage_nonempty
        })
        .count() as u32;
    counts.user_messages_primary = nodes.iter().filter(|n| n.is_primary_boundary()).count() as u32;
    counts.user_boundaries_wide = nodes.iter().filter(|n| n.is_wide_boundary()).count() as u32;
    counts.aux_model_usage_records =
        nodes.iter().filter(|n| n.source_type == "model_usage").count() as u32;
    counts.developer_messages = nodes
        .iter()
        .filter(|n| n.source_type == "message" && n.role.as_deref() == Some("developer"))
        .count() as u32;
    counts.file_mention_messages = nodes
        .iter()
        .filter(|n| n.source_type == "message" && n.role.as_deref() == Some("fileMention"))
        .count() as u32;
    counts.custom_role_messages = nodes
        .iter()
        .filter(|n| n.source_type == "message" && n.role.as_deref() == Some("custom"))
        .count() as u32;

    let mut call_ids: BTreeSet<&str> = BTreeSet::new();
    let mut result_ids: BTreeSet<&str> = BTreeSet::new();
    for node in &nodes {
        call_ids.extend(node.tool_calls.iter().map(String::as_str));
        if node.source_type == "message" && node.role.as_deref() == Some("toolResult") {
            if let Some(id) = &node.tool_result_id {
                result_ids.insert(id.as_str());
            }
        }
    }
    counts.toolcall_blocks = nodes.iter().map(|n| n.tool_calls.len() as u32).sum();
    counts.toolresult_entries = nodes
        .iter()
        .filter(|n| n.source_type == "message" && n.role.as_deref() == Some("toolResult"))
        .count() as u32;
    counts.toolresults_unmatched = result_ids.difference(&call_ids).count() as u32;
    counts.toolcalls_without_result = call_ids.difference(&result_ids).count() as u32;
    counts.assistant_msgs_with_multiple_calls = nodes
        .iter()
        .filter(|n| {
            n.source_type == "message"
                && n.role.as_deref() == Some("assistant")
                && n.tool_calls.len() > 1
        })
        .count() as u32;
    counts.max_calls_per_assistant = nodes
        .iter()
        .filter(|n| n.source_type == "message" && n.role.as_deref() == Some("assistant"))
        .map(|n| n.tool_calls.len() as u32)
        .max()
        .unwrap_or(0);

    // Episode segmentation follows T1 exactly: pre-user turns are counted but
    // not assigned; empty episodes are reported, not dropped.
    let primary = build_episodes(
        opts,
        &nodes,
        &path_indices,
        &turn_seq,
        |n| n.is_primary_boundary(),
        "primary",
    );
    let wide = build_episodes(
        opts,
        &nodes,
        &path_indices,
        &turn_seq,
        |n| n.is_wide_boundary(),
        "wide",
    );

    counts.episodes_primary_count = primary.count;
    counts.episodes_primary_empty = primary.empty;
    counts.episodes_primary_preuser_turns = primary.pre_user;
    counts.episodes_primary_turn_sum = primary.turn_sum;
    counts.episodes_wide_count = wide.count;
    counts.episodes_wide_empty = wide.empty;
    counts.episodes_wide_preuser_turns = wide.pre_user;
    counts.episodes_wide_turn_sum = wide.turn_sum;

    let primary_of = primary.turn_episode;
    let wide_of = wide.turn_episode;
    let episodes_primary = primary.episodes;
    let episodes_wide = wide.episodes;
    let turns: Vec<TurnProjection> = leaf_turn_indices
        .iter()
        .enumerate()
        .map(|(seq, index)| {
            let node = &nodes[*index];
            let (outcome, _) = stop_bucket(node.stop.as_deref());
            TurnProjection {
                seq: seq as u32,
                entry_id: node.id.clone(),
                ordinal: node.ordinal,
                timestamp: node.ts_ms.map(format_iso_ms),
                stop_reason: node.stop.clone(),
                outcome: outcome.to_string(),
                tool_calls: node.tool_calls.len() as u32,
                episode_primary: primary_of.get(index).copied(),
                episode_wide: wide_of.get(index).copied(),
                evidence: evidence_for(opts, node, SEL_TURN),
            }
        })
        .collect();

    let leaf = match selected_index {
        Some(index) => {
            let node = &nodes[index];
            LeafSummary {
                entry_id: Some(node.id.clone()),
                ordinal: Some(node.ordinal),
                timestamp: node.ts_ms.map(format_iso_ms),
                source_type: Some(node.source_type.clone()),
                selection: if explicit_index.is_some() { "explicit" } else { "active" }.to_string(),
                is_graph_leaf,
                graph_leaf_count: counts.leaf_count,
                path_node_count: path_indices.len() as u32,
                evidence: Some(evidence_for(opts, node, SEL_LEAF)),
            }
        }
        None => LeafSummary {
            entry_id: None,
            ordinal: None,
            timestamp: None,
            source_type: None,
            selection: if explicit_index.is_some() { "explicit" } else { "active" }.to_string(),
            is_graph_leaf,
            graph_leaf_count: counts.leaf_count,
            path_node_count: 0,
            evidence: None,
        },
    };

    let header = match header_raw {
        Some((id, timestamp, parent)) => HeaderInfo {
            session_id: id,
            timestamp: timestamp.clone(),
            parent_session: parent,
            header_ok: timestamp.is_some(),
            missing_session_header: false,
        },
        None => HeaderInfo {
            session_id: None,
            timestamp: None,
            parent_session: None,
            header_ok: false,
            missing_session_header: true,
        },
    };

    Ok(JournalProjection {
        adapter: ADAPTER_NAME.to_string(),
        rule: WORK_RULE.to_string(),
        rule_version: WORK_VERSION.to_string(),
        compatible_reference_rule: COMPATIBLE_REFERENCE_RULE.to_string(),
        generation: opts.generation.to_string(),
        source_id: opts.source_id.to_string(),
        file_id: opts.file_id,
        path: opts.path.to_string(),
        sha256: opts.sha256.to_string(),
        event_base: opts.event_base,
        window: opts.window,
        session_id: header.session_id.clone(),
        header,
        active_leaf: leaf,
        requested_leaf: opts.explicit_leaf.map(str::to_string),
        path_nodes: path_indices
            .iter()
            .map(|index| PathNode {
                ordinal: nodes[*index].ordinal,
                entry_id: nodes[*index].id.clone(),
                source_type: nodes[*index].source_type.clone(),
            })
            .collect(),
        turns,
        episodes_primary,
        episodes_wide,
        counts,
        runtime,
    })
}

struct EpisodeBuild {
    episodes: Vec<EpisodeProjection>,
    turn_episode: BTreeMap<usize, u32>,
    count: u32,
    empty: u32,
    pre_user: u32,
    turn_sum: u32,
}

fn build_episodes(
    opts: &ProjectOptions<'_>,
    nodes: &[Node],
    path_indices: &[usize],
    turn_seq: &BTreeMap<usize, u32>,
    boundary: impl Fn(&Node) -> bool,
    kind: &str,
) -> EpisodeBuild {
    let mut episodes: Vec<EpisodeProjection> = Vec::new();
    let mut turn_episode: BTreeMap<usize, u32> = BTreeMap::new();
    let mut empty = 0u32;
    let mut pre_user = 0u32;
    let mut turn_sum = 0u32;
    let mut current_turns = 0u32;
    for index in path_indices {
        let node = &nodes[*index];
        if boundary(node) {
            if let Some(last) = episodes.last_mut() {
                last.turn_count = current_turns;
            }
            if !episodes.is_empty() && current_turns == 0 {
                empty += 1;
            }
            episodes.push(EpisodeProjection {
                index: episodes.len() as u32,
                boundary_kind: kind.to_string(),
                boundary_entry_id: Some(node.id.clone()),
                boundary_ordinal: Some(node.ordinal),
                turn_start: None,
                turn_count: 0,
                evidence: Some(evidence_for(
                    opts,
                    node,
                    if node.source_type == "message" { SEL_USER } else { SEL_CUSTOM_USER },
                )),
            });
            current_turns = 0;
        } else if node.is_turn() {
            if episodes.is_empty() {
                pre_user += 1;
            } else {
                current_turns += 1;
                turn_sum += 1;
                let episode_index = episodes.len() as u32 - 1;
                turn_episode.insert(*index, episode_index);
                if let Some(seq) = turn_seq.get(index) {
                    episodes.last_mut().unwrap().turn_start.get_or_insert(*seq);
                }
            }
        }
    }
    if let Some(last) = episodes.last_mut() {
        last.turn_count = current_turns;
    }
    if !episodes.is_empty() && current_turns == 0 {
        empty += 1;
    }
    EpisodeBuild {
        count: episodes.len() as u32,
        episodes,
        turn_episode,
        empty,
        pre_user,
        turn_sum,
    }
}
