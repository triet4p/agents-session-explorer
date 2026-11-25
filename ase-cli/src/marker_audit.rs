//! Marker audit and its verified prose seam.
//!
//! Only allowlisted prose blocks of one published generation are re-derived
//! from raw bytes; marker observations keep generation, span, selector and
//! block digest provenance without emitting prose text.

use crate::hash_bytes;
use crate::{emit, has, index_path, require_publication, span_label};
use ase_core::{err, ProjectionKind, RawStore, Result, NONE};
use ase_ingest::util::fnv64;
use ase_index::{FsRawStore, Index};
use ase_postings::reader::BlockRecord;
use ase_postings::Publication;
use ase_task::MarkerEvidence;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use crate::opt;
use std::sync::Arc;

/// Allowlisted prose blocks of one published generation that verify by span
/// hash, selector re-derivation and block digest. Prose text never leaves here.
pub(crate) struct VerifiedProse {
    pub(crate) blocks_total: usize,
    pub(crate) excluded_role_blocks: usize,
    pub(crate) unverified_blocks: usize,
    pub(crate) texts: Vec<ase_markers::Block>,
    pub(crate) kept: Vec<BlockRecord>,
}

pub(crate) fn verified_prose(
    idx: &Arc<Index>,
    store: &FsRawStore,
    published: &Publication,
) -> Result<VerifiedProse> {
    let blocks = ase_postings::all_prose_blocks(published)?;
    let mut texts: Vec<ase_markers::Block> = Vec::with_capacity(blocks.len());
    let mut kept = Vec::with_capacity(blocks.len());
    let mut excluded_role_blocks = 0usize;
    let mut unverified_blocks = 0usize;
    for block in &blocks {
        if block.role != "user" && block.role != "assistant" {
            excluded_role_blocks += 1;
            continue;
        }
        if block.event >= idx.event_count {
            unverified_blocks += 1;
            continue;
        }
        let ev = idx.event(block.event)?;
        if ev.span() != block.span {
            unverified_blocks += 1;
            continue;
        }
        let raw = store.span(block.span)?;
        if fnv64(&raw) != ev.hash() {
            unverified_blocks += 1;
            continue;
        }
        let provider = idx.file(block.span.file)?.provider();
        let extracted = match ase_ingest::extract_block(provider, &raw, &block.selector) {
            Ok(value) => value,
            Err(_) => {
                unverified_blocks += 1;
                continue;
            }
        };
        if extracted.kind != ProjectionKind::Prose
            || hash_bytes(&block.block_hash) != Some(extracted.block_hash)
        {
            unverified_blocks += 1;
            continue;
        }
        let text = extracted.prose.unwrap_or_default();
        if text.trim().is_empty() {
            unverified_blocks += 1;
            continue;
        }
        kept.push(block.clone());
        texts.push(ase_markers::Block { text });
    }
    Ok(VerifiedProse {
        blocks_total: blocks.len(),
        excluded_role_blocks,
        unverified_blocks,
        texts,
        kept,
    })
}

/// Convert verified marker occurrences into metadata-only resolver evidence.
pub(crate) fn marker_evidence(
    idx: &Index,
    kept: &[BlockRecord],
    observations: &[ase_markers::Observation],
    published: &Publication,
) -> Result<Vec<MarkerEvidence>> {
    let mut out = Vec::with_capacity(observations.len());
    for observation in observations {
        let block = &kept[observation.block_index];
        let sidx = idx.event(block.event)?.session();
        let session_id = if sidx != NONE {
            idx.session(sidx)?.id().to_string()
        } else {
            String::new()
        };
        out.push(MarkerEvidence {
            value: observation.value.clone(),
            usage: observation.usage,
            reason: observation.reason.to_string(),
            assignment_cue: observation.assignment_cue.clone(),
            mention_cue: observation.mention_cue.clone(),
            occurrence_in_block: observation.occurrence_in_block,
            char_start: observation.char_start,
            char_end: observation.char_end,
            generation: published.generation.clone(),
            event: block.event,
            span: span_label(block.span),
            selector: block.selector.clone(),
            block_hash: block.block_hash.clone(),
            role: block.role.clone(),
            file_id: block.span.file,
            session_id,
        });
    }
    Ok(out)
}
// ------------------------------------------------------------ marker audit

// Synthetic markers only. One OMP session carries allowlisted user/assistant
// text plus deliberately marker-bearing thinking, tool output and call
// arguments that the allowlist must never project as prose.
const MARKER_FIXTURE_LINES: &[&str] = &[
    r#"{"type":"session","version":3,"id":"marker-fixture-session","timestamp":"2026-03-06T07:08:09.000Z","cwd":"C:\\SYNTHETIC\\markers","title":"synthetic marker fixture"}"#,
    r#"{"type":"message","id":"marker-e1","parentId":null,"timestamp":"2026-03-06T07:08:10.000Z","message":{"role":"user","content":[{"type":"text","text":"Please implement task_id=SYN-A1 and task_id=SYN-A2 in the parser."}]}}"#,
    r#"{"type":"message","id":"marker-e2","parentId":null,"timestamp":"2026-03-06T07:08:11.000Z","message":{"role":"assistant","content":[{"type":"text","text":"I will work on task_id=SYN-A1. Please implement task_id=SYN-A1 again."}]}}"#,
    r#"{"type":"message","id":"marker-e3","parentId":null,"timestamp":"2026-03-06T07:08:12.000Z","message":{"role":"user","content":[{"type":"text","text":"Earlier we mentioned task_id=SYN-M1 for context only."}]}}"#,
    r#"{"type":"message","id":"marker-e4","parentId":null,"timestamp":"2026-03-06T07:08:13.000Z","message":{"role":"assistant","content":[{"type":"text","text":"Implement task_id=SYN-X1 but note task_id=SYN-X1 for reference."}]}}"#,
    r#"{"type":"message","id":"marker-e5","parentId":null,"timestamp":"2026-03-06T07:08:14.000Z","message":{"role":"user","content":[{"type":"text","text":"task_id=SYN-U1"}]}}"#,
    r#"{"type":"message","id":"marker-e6","parentId":null,"timestamp":"2026-03-06T07:08:15.000Z","message":{"role":"assistant","content":[{"type":"thinking","text":"task_id=SYN-THINK assigned here"},{"type":"text","text":"Please implement task_id=SYN-A3."}]}}"#,
    r#"{"type":"message","id":"marker-e7","parentId":null,"timestamp":"2026-03-06T07:08:16.000Z","message":{"role":"assistant","content":[{"type":"toolCall","name":"synthetic_tool","args":"{\"task_id\":\"SYN-CALL\",\"mode\":1}"}]}}"#,
    r#"{"type":"message","id":"marker-e8","parentId":null,"timestamp":"2026-03-06T07:08:17.000Z","message":{"role":"toolResult","toolCallId":"marker-call","content":[{"type":"text","text":"tool output only: task_id=SYN-TOOL assigned here"}]}}"#,
];
pub fn run_fixture(args: &[String]) -> Result<()> {
    let out = PathBuf::from(
        opt(args, "--out").ok_or_else(|| err("fixture-markers needs --out DIR"))?,
    );
    let omp = out.join("omp");
    let codex = out.join("codex");
    fs::create_dir_all(&omp).map_err(|e| err(format!("create {}: {e}", omp.display())))?;
    fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;
    let mut content = String::new();
    for line in MARKER_FIXTURE_LINES {
        content.push_str(line);
        content.push('\n');
    }
    let path = omp.join("synthetic-markers-markers.jsonl");
    fs::write(&path, content).map_err(|e| err(format!("write {}: {e}", path.display())))?;
    println!(
        "wrote {} synthetic marker fixture lines to {}",
        MARKER_FIXTURE_LINES.len(),
        out.display()
    );
    Ok(())
}

/// Read allowlisted prose blocks from one published generation, re-derive each
/// block from raw bytes at its pinned span, and emit marker observations only
/// after the span hash and block digest both verify. No prose is emitted.
pub fn run(args: &[String]) -> Result<()> {
    let (idx, store, published) = require_publication(args)?;
    let VerifiedProse {
        blocks_total,
        excluded_role_blocks,
        unverified_blocks,
        texts,
        kept,
    } = verified_prose(&idx, &store, &published)?;

    let result = ase_markers::audit(&texts);
    let rule = &result.rule;
    let rule_value = json!({
        "name": &rule.name, "version": &rule.version, "matcher": &rule.matcher
    });
    let observations = result
        .observations
        .iter()
        .map(|o| {
            let b = &kept[o.block_index];
            let reopen = format!(
                "ase open --event {} --selector {} --generation {} --index {}",
                b.event,
                b.selector,
                published.generation,
                index_path(args).display()
            );
            json!({
                "value": &o.value,
                "usage": o.usage.label(),
                "reason": o.reason,
                "assignment_cue": &o.assignment_cue,
                "mention_cue": &o.mention_cue,
                "occurrence_in_block": o.occurrence_in_block,
                "char_start": o.char_start,
                "char_end": o.char_end,
                "evidence": {
                    "generation": &published.generation,
                    "event": b.event,
                    "span": span_label(b.span),
                    "selector": &b.selector,
                    "block_hash": &b.block_hash,
                    "role": &b.role,
                    "session_local": b.session_local,
                    "parent_idx": b.parent_idx,
                    "span_hash_match": true,
                    "block_digest_match": true,
                    "reopen": reopen,
                    "note": "Prose is re-derived from raw bytes at the pinned span by the versioned selector; it is never stored or emitted by this audit."
                },
                "rule": rule_value.clone(),
                "task_key": Value::Null,
                "assigned_to": Value::Null,
                "session_assignment": Value::Null
            })
        })
        .collect::<Vec<Value>>();
    let distinct = result
        .observations
        .iter()
        .map(|o| o.value.clone())
        .collect::<std::collections::BTreeSet<String>>()
        .len();

    let v = json!({
        "command": "audit",
        "subject": "markers",
        "status": "ok",
        "rule": {
            "name": &rule.name,
            "version": &rule.version,
            "matcher": &rule.matcher,
            "domain": &rule.domain,
            "abstention": &rule.abstention,
            "task_boundary": &rule.task_boundary
        },
        "source": {
            "kind": "published_prose_blocks",
            "generation": &published.generation,
            "index": index_path(args).to_string_lossy(),
            "fields": ["blocks.generation", "blocks.event_idx", "blocks.selector", "blocks.span", "blocks.block_hash", "blocks.role", "raw.span bytes", "PROJECTION_VERSION"],
            "raw_span_read": true,
            "prose_stored": false,
            "prose_emitted": false,
            "note": "Only allowlisted user/assistant prose blocks of one published generation are scanned. Raw spans are re-read to verify and re-derive the block; prose text is never stored or printed."
        },
        "counts": {
            "blocks_total": blocks_total,
            "blocks_scanned": result.blocks_scanned,
            "excluded_role_blocks": excluded_role_blocks,
            "unverified_blocks": unverified_blocks,
            "occurrences": result.occurrences,
            "assignment_context": result.assignment_context,
            "mention": result.mention,
            "ambiguous": result.ambiguous,
            "distinct_values": distinct
        },
        "observations": observations,
        "task_assignment": "none",
        "task_key": Value::Null,
        "explain": if has(args, "--explain") {
            vec![
                "Read every prose block row pinned to the active generation from the published postings layer; thinking, tool output, system, unknown and call-signature blocks are never prose rows.".to_string(),
                "Re-read the pinned raw span, check the FNV span hash against the address event, re-derive the block with its versioned selector and require the block digest to match before scanning any marker.".to_string(),
                "Scan only the exact task_id= key with an ASCII marker value; every occurrence is retained with its in-block character offsets, never deduplicated into a session-level claim.".to_string(),
                "Classify each occurrence from assignment and mention cues in its local window, bounded at the next marker so one marker's cue cannot justify another; no cue or both cue classes means ambiguous/abstain.".to_string(),
                "Every observation carries generation, event, span, selector and block digest for reopening with `ase open`, plus rule name/version and null task_key/assigned_to/session_assignment: this rule observes markers only.".to_string()
            ]
        } else {
            Vec::<String>::new()
        },
        "guarantees": [
            "Metadata-only by default: no prose text, snippet or session-level summary is emitted; the exact block is reopened from raw bytes with the same generation.",
            "Only user/assistant prose blocks that verify by span hash and block digest are scanned; tool output and thinking markers cannot appear.",
            "Multiple markers, repeated references and every occurrence are retained separately; no first-token or single-session promotion exists.",
            "Assignment-context, mention and ambiguous statuses remain candidate signals with their cues and reasons; ambiguity is never resolved by this rule.",
            "No TaskKey, assigned_to edge or session-level inference is produced; reconciling signals happens in task resolution."
        ]
    });
    emit(&v, args);
    Ok(())
}
