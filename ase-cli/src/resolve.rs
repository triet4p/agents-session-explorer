//! Task, session and span resolution against one published index snapshot.
//!
//! Relations are verbatim source metadata or reconciled candidate evidence:
//! assignment, ambiguity and abstention are reported without inferring edges
//! from prose, paths or dispatch claims.
use serde_json::{json, Value};
use std::sync::Arc;

use crate::audit::{delegation_inputs, naming_sources};
use crate::marker_audit::{marker_evidence, verified_prose};
use crate::{has, index_path, load_with_publication, parse_span, positional, span_label};
use ase_core::{err, EventIdx, RawStore, Result, NONE};
use ase_ingest::util::fnv64;
use ase_index::{FsRawStore, Index};
use ase_motif::membership::dispatch_input;
use ase_postings::Publication;
use ase_task::{
    Basis, DelegationEvidence, MarkerLayer, NamingEvidence, NamingSignal,
    PathEvidence, Resolution, Stance, Status as TaskStatus,
};

pub fn run(args: &[String]) -> Result<()> {
    let target = positional(args).ok_or_else(|| err("resolve needs a target"))?;
    let (idx, store, published, publication_reason) = load_with_publication(args)?;
    let idx = &idx;
    let explain = has(args, "--explain");
    let mut relations = Vec::new();
    let mut steps = Vec::new();
    let status = "exact";
    let subject: String;
    let note = String::new();

    if let Some(id) = target.strip_prefix("session:") {
        let sidx = idx
            .session_by_id(id)
            .ok_or_else(|| err(format!("session not indexed: {id}")))?;
        let s = idx.session(sidx)?;
        subject = format!("session:{id}");
        let header = s.header();
        steps.push(format!(
            "looked up session id by binary search over {} sorted session ids -> idx {}",
            idx.session_count, sidx
        ));
        steps.push(format!("session header event is {}", span_label(idx.event(header)?.span())));

        if s.parent_idx() != NONE {
            let p = idx.session(s.parent_idx())?;
            relations.push(relation(
                "parent_session",
                json!(p.id()),
                if s.path_derived() { "source path (sidecar directory)" } else { "omp.parentSession / codex session_meta" },
                vec![evidence_for(idx, header), evidence_for(idx, p.header())],
            ));
        } else if !s.parent_id().is_empty() {
            relations.push(relation(
                "unresolved_parent",
                json!(s.parent_id()),
                "verbatim source parent id (no indexed session)",
                vec![evidence_for(idx, header)],
            ));
        }
        for child in idx.sessions_by_parent(sidx) {
            let c = idx.session(child)?;
            relations.push(relation(
                "child_session",
                json!(c.id()),
                "reverse of source parent reference",
                vec![evidence_for(idx, header), evidence_for(idx, c.header())],
            ));
        }
        let events = idx.session_events(sidx)?;
        let mut branch_count = 0usize;
        let mut call_count = 0usize;
        for e in &events {
            let ev = idx.event(*e)?;
            if ev.parent() != NONE {
                branch_count += 1;
                relations.push(relation(
                    "branch_parent",
                    json!(span_label(idx.event(ev.parent())?.span())),
                    "omp.parentId (verbatim source DAG edge)",
                    vec![evidence_for(idx, *e), evidence_for(idx, ev.parent())],
                ));
            }
            if ev.link() != NONE {
                call_count += 1;
                let other = idx.event(ev.link())?;
                let kind = if ev.kind() == ase_core::EventKind::ToolResult {
                    "call_link_result"
                } else {
                    "call_link_call"
                };
                relations.push(relation(
                    kind,
                    json!(span_label(other.span())),
                    "toolCallId / codex call_id (verbatim source link)",
                    vec![evidence_for(idx, *e), evidence_for(idx, ev.link())],
                ));
            }
        }
        if explain {
            steps.push(format!(
                "scanned {} indexed events of this session: {} branch edges, {} call links",
                events.len(), branch_count, call_count
            ));
        }
    } else if let Some(span) = parse_span(&target) {
        let e = idx
            .event_at(span)?
            .ok_or_else(|| err("no indexed event at that coordinate"))?;
        let ev = idx.event(e)?;
        subject = span_label(span);
        steps.push(format!(
            "exact coordinate lookup -> event {}: {} / tag {}",
            e,
            ev.kind().label(),
            ev.tag()
        ));
        if ev.parent() != NONE {
            relations.push(relation(
                "branch_parent",
                json!(span_label(idx.event(ev.parent())?.span())),
                "omp.parentId (verbatim source DAG edge)",
                vec![evidence_for(idx, e), evidence_for(idx, ev.parent())],
            ));
        }
        if ev.session() != NONE {
            for i in idx.session_events(ev.session())? {
                let other = idx.event(i)?;
                if other.parent() == e {
                    relations.push(relation(
                        "branch_child",
                        json!(span_label(other.span())),
                        "reverse of omp.parentId",
                        vec![evidence_for(idx, e), evidence_for(idx, i)],
                    ));
                }
            }
        }
        if ev.link() != NONE {
            relations.push(relation(
                "call_link",
                json!(span_label(idx.event(ev.link())?.span())),
                "toolCallId / codex call_id (verbatim source link)",
                vec![evidence_for(idx, e), evidence_for(idx, ev.link())],
            ));
        }
    } else if let Some(token) = target.strip_prefix("task:") {
        return cmd_resolve_task(
            args,
            idx,
            &store,
            published.as_ref(),
            publication_reason.as_deref(),
            token,
            explain,
        );
    } else {
        return Err(err("target must be session:ID, span:F:OFF:LEN or task:TOKEN"));
    }

    // Read one evidence span so provenance is checked even for metadata-only output.
    let mut hash_verified = None;
    if let Some(r) = relations.first() {
        if let Some(ev) = r["evidence"][0]["event"].as_u64() {
            let e = idx.event(ev as u32)?;
            let bytes = store.span(e.span())?;
            hash_verified = Some(fnv64(&bytes) == e.hash());
        }
    }

    let v = json!({
        "command": "resolve",
        "subject": subject,
        "status": status,
        "relations": relations,
        "explain": if explain { steps } else { Vec::<String>::new() },
        "note": note,
        "hash_verified": hash_verified,
        "guarantees": [
            "All relations are verbatim source metadata captured at index time; no inferred or model-generated edges.",
            "session: and span: resolution is exhaustive over the indexed snapshot and reports unresolved references explicitly.",
            "task: resolution is candidate-only at this layer: a literal path token hit is NOT task identity and NOT an assignment. Naming grammar, prose task_id, continuation and dispatch reconciliation are reported separately; none are guessed here.",
            "Every relation carries at least one raw byte span; stale files are reported, not silently served.",
            "No postings, motif, turn or judge data is used by this relation view."
        ]
    });
    if has(args, "--json") {
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        println!("resolve {} -> {} ({} relation(s))", subject, status, relations.len());
        for r in &relations {
            println!(
                "  {} -> {} [{}] evidence={}",
                r["kind"].as_str().unwrap_or(""),
                r["target"].as_str().unwrap_or(""),
                r["source_field"].as_str().unwrap_or(""),
                r["evidence"].as_array().map(|a| a.len()).unwrap_or(0)
            );
        }
        if !note.is_empty() {
            println!("  note: {note}");
        }
            let empty_arr: Vec<Value> = Vec::new();
        if explain {
            for s in v["explain"].as_array().unwrap_or(&empty_arr) {
                println!("  step: {}", s.as_str().unwrap_or(""));
            }
        }
    }
    Ok(())
}

// ------------------------------------------------ task identity resolution

fn resolve_naming_signals(idx: &Index) -> Result<Vec<NamingSignal>> {
    let audit = ase_naming::audit(&naming_sources(idx)?);
    Ok(audit
        .signals
        .iter()
        .map(|signal| NamingSignal {
            status: signal.status,
            reason: signal.reason.clone(),
            candidates: signal
                .candidates
                .iter()
                .map(|candidate| NamingEvidence {
                    project: candidate.project.clone(),
                    task: candidate.task.clone(),
                    role: candidate.role.clone(),
                    attempt: candidate.attempt.clone(),
                    split_rule: candidate.split_rule.clone(),
                    file_id: signal.source.file_id,
                    path: signal.source.path.clone(),
                    provider: signal.source.provider.clone(),
                    session_id: signal.source.session_id.clone(),
                    span_anchor: signal.source.span_anchor.clone(),
                })
                .collect(),
        })
        .collect())
}

fn resolve_delegation_signals(idx: &Index) -> Result<Vec<DelegationEvidence>> {
    let audit = ase_delegation::audit(&delegation_inputs(idx)?);
    Ok(audit
        .edges
        .iter()
        .map(|edge| DelegationEvidence {
            from_session_id: edge.from.id.clone(),
            from_span: edge.from.header_span.clone(),
            declared_id: edge.declaration.declared_id.clone(),
            observation: edge.declaration.kind.label().to_string(),
            field: edge.declaration.field.to_string(),
            link_status: edge.status.label().to_string(),
            target_session_id: edge.target.as_ref().map(|target| target.id.clone()),
            target_span: edge.target.as_ref().map(|target| target.header_span.clone()),
            reason: edge.reason.to_string(),
        })
        .collect())
}

/// Literal path-token matches are observed metadata candidates only.
fn resolve_path_matches(idx: &Index, token: &str) -> Result<Vec<PathEvidence>> {
    let needle = token.to_lowercase();
    let mut out = Vec::new();
    for sidx in 0..idx.session_count {
        let session = idx.session(sidx)?;
        let file = idx.file(session.file())?;
        if !file.path().to_lowercase().contains(&needle) {
            continue;
        }
        out.push(PathEvidence {
            session_id: session.id().to_string(),
            path: file.path().to_string(),
            provider: file.provider().label().to_string(),
            span_anchor: span_label(idx.event(session.header())?.span()),
        });
    }
    Ok(out)
}
fn relation(kind: &str, target: Value, source_field: &str, evidence: Vec<Value>) -> Value {
    json!({
        "kind": kind,
        "target": target,
        "source_field": source_field,
        "evidence": evidence
    })
}

fn evidence_for(idx: &Arc<Index>, e: EventIdx) -> Value {
    match idx.event(e) {
        Ok(ev) => json!({"span": span_label(ev.span()), "event": e}),
        Err(_) => json!({"span": null, "event": e}),
    }
}
fn task_evidence_value(
    evidence: &ase_task::Evidence,
    stance: Stance,
    index_label: &str,
) -> Value {
    match evidence {
        ase_task::Evidence::Naming(naming) => json!({
            "kind": evidence.kind(),
            "basis": Basis::Derived.label(),
            "stance": stance.label(),
            "rule": { "name": ase_naming::RULE_NAME, "version": ase_naming::RULE_VERSION },
            "project": &naming.project,
            "task": &naming.task,
            "role": &naming.role,
            "attempt": &naming.attempt,
            "split_rule": &naming.split_rule,
            "source": {
                "file_id": naming.file_id,
                "path": &naming.path,
                "provider": &naming.provider,
                "session_id": &naming.session_id,
                "span_anchor": &naming.span_anchor
            },
            "note": "The path is observed metadata; project/task/role/attempt are derived from the basename inside its (project,parent) scope."
        }),
        ase_task::Evidence::Marker(marker) => json!({
            "kind": evidence.kind(),
            "basis": Basis::Observed.label(),
            "stance": stance.label(),
            "rule": { "name": ase_markers::RULE_NAME, "version": ase_markers::RULE_VERSION },
            "value": &marker.value,
            "char_start": marker.char_start,
            "char_end": marker.char_end,
            "observed": {
                "generation": &marker.generation,
                "event": marker.event,
                "span": &marker.span,
                "selector": &marker.selector,
                "block_hash": &marker.block_hash,
                "role": &marker.role,
                "file_id": marker.file_id,
                "session_id": &marker.session_id
            },
            "derived": {
                "usage": marker.usage.label(),
                "reason": &marker.reason,
                "assignment_cue": &marker.assignment_cue,
                "mention_cue": &marker.mention_cue,
                "occurrence_in_block": marker.occurrence_in_block,
                "rule": { "name": ase_markers::RULE_NAME, "version": ase_markers::RULE_VERSION }
            },
            "note": "The occurrence is observed; its assignment/mention class is derived. No prose text is emitted."
        }),
        ase_task::Evidence::Delegation(delegation) => json!({
            "kind": evidence.kind(),
            "basis": Basis::Observed.label(),
            "stance": stance.label(),
            "rule": { "name": ase_delegation::RULE_NAME, "version": ase_delegation::RULE_VERSION },
            "observed": {
                "from_session_id": &delegation.from_session_id,
                "from_span": &delegation.from_span,
                "declared_id": &delegation.declared_id,
                "field": &delegation.field,
                "observation": &delegation.observation
            },
            "derived": {
                "link_status": &delegation.link_status,
                "target_session_id": &delegation.target_session_id,
                "target_span": &delegation.target_span,
                "reason": &delegation.reason,
                "rule": { "name": ase_delegation::RULE_NAME, "version": ase_delegation::RULE_VERSION }
            },
            "task_boundary": "A parentSession declaration is a delegated_from edge, never a task assignment."
        }),
        ase_task::Evidence::Path(path) => json!({
            "kind": evidence.kind(),
            "basis": Basis::Observed.label(),
            "stance": stance.label(),
            "session_id": &path.session_id,
            "path": &path.path,
            "provider": &path.provider,
            "span_anchor": &path.span_anchor,
            "rule": { "name": ase_task::PATH_RULE_NAME, "version": ase_task::PATH_RULE_VERSION },
            "note": "A literal source-path substring is a candidate, not task identity."
        }),
        ase_task::Evidence::Dispatch(join) => json!({
            "kind": evidence.kind(),
            "basis": Basis::Observed.label(),
            "stance": stance.label(),
            "rule": { "name": ase_task::DISPATCH_RULE_NAME, "version": ase_task::DISPATCH_RULE_VERSION },
            "observed": {
                "dispatch_id": &join.dispatch_id,
                "parent_session_id": &join.parent_session_id,
                "child_session_id": &join.child_session_id,
                "parent_tool_call_id": &join.parent_tool_call_id,
                "spawn_index": join.spawn_index,
                "parent": {
                    "event": join.parent_event,
                    "file_id": join.parent_file_id,
                    "span": &join.parent_span,
                    "span_hash_verified": join.parent_span_verified,
                    "reopen": format!("ase show {} --index {} --json --explain", join.parent_span, index_label)
                },
                "child": {
                    "event": join.child_event,
                    "file_id": join.child_file_id,
                    "span": &join.child_span,
                    "span_hash_verified": join.child_span_verified,
                    "reopen": format!("ase show {} --index {} --json --explain", join.child_span, index_label)
                }
            },
            "derived": {
                "project": &join.project,
                "task": &join.task,
                "attempt": &join.attempt,
                "join": "exactly one task_dispatch_v1 parent record and one task_dispatch_child_v1 link for this dispatch_id; the structured parent_session_id and child session_id match their containing journals; parent and child journals differ; both spans verified",
                "rule": { "name": ase_task::DISPATCH_RULE_NAME, "version": ase_task::DISPATCH_RULE_VERSION }
            },
            "task_boundary": "The join assigns the executing child session named by the runtime records only; a suffix, path, worker name, prose echo or parent-only claim never does. A malformed, duplicated, mismatched or competing record assigns nothing."
        }),
    }
}

fn task_evidence_list(
    resolution: &Resolution,
    ids: &[usize],
    stance: Stance,
    index_label: &str,
) -> Vec<Value> {
    ids.iter()
        .filter_map(|id| resolution.evidence.get(*id))
        .map(|evidence| task_evidence_value(evidence, stance, index_label))
        .collect()
}

fn task_candidate_relation(
    resolution: &Resolution,
    candidate: &ase_task::Candidate,
    index_label: &str,
) -> Value {
    let kind = if resolution.status == TaskStatus::Assigned {
        "assigned_to"
    } else {
        "candidate_task"
    };
    json!({
        "kind": kind,
        "target": {
            "project": &candidate.project,
            "task": &candidate.task,
            "task_key": &candidate.task_key
        },
        "candidate_state": candidate.state.label(),
        "reason": &candidate.reason,
        "support": task_evidence_list(resolution, &candidate.supports, Stance::Support, index_label),
        "refute": task_evidence_list(resolution, &candidate.refutes, Stance::Refute, index_label),
        "context": task_evidence_list(resolution, &candidate.context, Stance::Context, index_label),
        "rule": { "name": resolution.rule.name, "version": resolution.rule.version }
    })
}

fn cmd_resolve_task(
    args: &[String],
    idx: &Arc<Index>,
    store: &FsRawStore,
    published: Option<&Publication>,
    publication_reason: Option<&str>,
    token: &str,
    explain: bool,
) -> Result<()> {
    if token.trim().is_empty() {
        return Err(err("resolve task: needs a non-empty token"));
    }
    let naming = resolve_naming_signals(idx)?;
    let delegations = resolve_delegation_signals(idx)?;
    let paths = resolve_path_matches(idx, token)?;
    let dispatch = dispatch_input(idx, store)?;
    let naming_count = naming.len();
    let delegation_count = delegations.len();
    let path_count = paths.len();
    let dispatch_parent_count = dispatch.parents.len();
    let dispatch_child_count = dispatch.children.len();

    let (marker_layer, markers) = match published {
        Some(published) => {
            let verified = verified_prose(idx, store, published)?;
            let audit = ase_markers::audit(&verified.texts);
            let markers = marker_evidence(idx, &verified.kept, &audit.observations, published)?;
            let layer = MarkerLayer {
                generation: Some(published.generation.clone()),
                unavailable: None,
                blocks_total: verified.blocks_total,
                blocks_scanned: verified.texts.len(),
                excluded_role_blocks: verified.excluded_role_blocks,
                unverified_blocks: verified.unverified_blocks,
            };
            (layer, markers)
        }
        None => (
            MarkerLayer {
                generation: None,
                unavailable: Some(
                    publication_reason
                        .unwrap_or("derived layer unavailable")
                        .to_string(),
                ),
                blocks_total: 0,
                blocks_scanned: 0,
                excluded_role_blocks: 0,
                unverified_blocks: 0,
            },
            Vec::new(),
        ),
    };

    let marker_count = markers.iter().filter(|marker| marker.value == token).count();
    let resolution = ase_task::resolve(&ase_task::Input {
        token: token.to_string(),
        project: None,
        naming,
        markers,
        delegations,
        paths,
        dispatch,
        marker_layer,
    });

    let index_label = index_path(args).to_string_lossy().to_string();
    let relations = resolution
        .candidates
        .iter()
        .map(|candidate| task_candidate_relation(&resolution, candidate, &index_label))
        .collect::<Vec<Value>>();
    let marker_layer = &resolution.marker_layer;
    let marker_layer_value = json!({
        "status": if marker_layer.generation.is_some() { "available" } else { "unavailable" },
        "reason": &marker_layer.unavailable,
        "generation": &marker_layer.generation,
        "blocks_total": marker_layer.blocks_total,
        "blocks_scanned": marker_layer.blocks_scanned,
        "excluded_role_blocks": marker_layer.excluded_role_blocks,
        "unverified_blocks": marker_layer.unverified_blocks
    });
    let dispatch_layer = &resolution.dispatch;
    let dispatch_layer_value = json!({
        "status": "available",
        "rule": { "name": ase_task::DISPATCH_RULE_NAME, "version": ase_task::DISPATCH_RULE_VERSION },
        "parents_indexed": dispatch_parent_count,
        "child_links_indexed": dispatch_child_count,
        "unique_joins": dispatch_layer.joined,
        "rejected_groups": dispatch_layer.rejected_groups,
        "record_issues": dispatch_layer.issues_total,
        "claims_for_token": dispatch_layer.claims_for_token,
        "rejected_claims_for_token": dispatch_layer
            .issues_for_token
            .iter()
            .map(|issue| json!({
                "record": &issue.record,
                "kind": &issue.kind,
                "dispatch_id": &issue.dispatch_id,
                "event": issue.event,
                "span": &issue.span,
                "detail": &issue.detail
            }))
            .collect::<Vec<Value>>()
    });
    let rule = &resolution.rule;
    let v = json!({
        "command": "resolve",
        "subject": format!("task:{token}"),
        "status": resolution.status.label(),
        "task_key": &resolution.task_key,
        "resolution": {
            "status": resolution.status.label(),
            "task_key": &resolution.task_key,
            "summary": &resolution.summary,
            "candidate_count": resolution.candidates.len(),
            "rule": { "name": &rule.name, "version": &rule.version }
        },
        "relations": relations,
        "signals": {
            "naming_signals_indexed": naming_count,
            "marker_occurrences_for_token": marker_count,
            "delegation_edges_indexed": delegation_count,
            "path_matches": path_count,
            "marker_layer": marker_layer_value,
            "dispatch_layer": dispatch_layer_value
        },
        "explain": if explain { resolution.explain.clone() } else { Vec::<String>::new() },
        "hash_verified": Value::Null,
        "guarantees": [
            "Assignment requires exactly one project-scoped key from a parsed name grammar or from one unique, consistent native dispatch join; assignment-context markers in the same file corroborate it but no marker alone assigns a task.",
            "A dispatch join requires exactly one task_dispatch_v1 parent record and one task_dispatch_child_v1 link for the same runtime dispatch_id, structured session ids that match their containing journals, distinct parent and child journals, and both raw spans re-verified against indexed bytes.",
            "Malformed, missing, duplicated, wrong-session, mismatched or competing dispatch records never assign; they are reported as rejected claims with their span and reason.",
            "A conflicting layer returns ambiguous with every candidate, its support/refute evidence and reasons; insufficient evidence stays unresolved.",
            "A mention, an ambiguous marker, a literal path match or a parentSession declaration is a candidate/context only, never an assignment.",
            "Project is part of the key: the same token in different projects is reported separately and never merged. The child session named by a dispatch join is the executing child, not a suffix or worker-name match.",
            "Observed and derived evidence are labelled with their rule name/version; marker evidence is pinned to one generation, span, selector and block digest, and dispatch evidence carries both parent and child record spans.",
            "Metadata-only: no prose text is emitted, no postings content is returned and no model is called."
        ]
    });
    if has(args, "--json") {
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        println!(
            "resolve task:{token} -> {} ({} candidate(s))",
            resolution.status.label(),
            resolution.candidates.len()
        );
        println!("  {}", resolution.summary);
        for r in &relations {
            let target = &r["target"];
            println!(
                "  {} -> project={} task={} state={} support={} refute={} context={}",
                r["kind"].as_str().unwrap_or(""),
                target["project"].as_str().unwrap_or("-"),
                target["task"].as_str().unwrap_or("-"),
                r["candidate_state"].as_str().unwrap_or(""),
                r["support"].as_array().map_or(0, Vec::len),
                r["refute"].as_array().map_or(0, Vec::len),
                r["context"].as_array().map_or(0, Vec::len)
            );
            println!("    reason: {}", r["reason"].as_str().unwrap_or(""));
        }
        if explain {
            let empty_arr: Vec<Value> = Vec::new();
            for step in v["explain"].as_array().unwrap_or(&empty_arr) {
                println!("  step: {}", step.as_str().unwrap_or(""));
            }
        }
    }
    Ok(())
}
