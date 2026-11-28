//! Metadata-only naming grammar and parentSession delegation audits.
//!
//! Both subjects resolve the logical `--index` path through the shared reader
//! seam and read indexed metadata only: no raw span, prose, postings or model.
//! Marker auditing lives with the verified prose seam.

use crate::cli_option::{has, opt};
use crate::cli_output::emit;
use crate::index::{index_path, load_address};
use crate::show::span_label;
use ase_core::{err, Result};
use ase_index::Index;
use ase_naming::{SourceRef, Status};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) fn naming_sources(idx: &Index) -> Result<Vec<SourceRef>> {
    let mut session_by_file: std::collections::HashMap<u32, (String, String)> =
        std::collections::HashMap::new();
    for s in 0..idx.session_count {
        let view = idx.session(s)?;
        let session_id = view.id().to_string();
        let span_anchor = idx
            .event(view.header())
            .map(|e| span_label(e.span()))
            .unwrap_or_default();
        session_by_file
            .entry(view.file())
            .or_insert((session_id, span_anchor));
    }
    let mut out = Vec::with_capacity(idx.file_count as usize);
    for fid in 0..idx.file_count {
        let f = idx.file(fid)?;
        let path = f.path().to_string();
        let (session_id, span_anchor) = session_by_file
            .get(&fid)
            .cloned()
            .unwrap_or_else(|| (String::new(), String::new()));
        out.push(SourceRef {
            file_id: fid,
            path,
            provider: f.provider().label().to_string(),
            session_id,
            span_anchor,
        });
    }
    Ok(out)
}

fn naming_evidence(s: &SourceRef, rule: &ase_naming::RuleInfo) -> Value {
    json!({
        "source": {
            "file_id": s.file_id,
            "path": s.path,
            "provider": s.provider,
            "session_id": s.session_id,
            "span_anchor": s.span_anchor
        },
        "locator": format!("file.path:{}", s.path),
        "rule": { "name": &rule.name, "version": &rule.version }
    })
}

fn candidate_value(c: &ase_naming::Candidate) -> Value {
    json!({
        "project": &c.project,
        "task": &c.task,
        "role": &c.role,
        "attempt": &c.attempt,
        "split_rule": &c.split_rule
    })
}

pub fn run(args: &[String]) -> Result<()> {
    let subject = args.first().map(String::as_str).unwrap_or("");
    match subject {
        "naming" => cmd_audit_naming(args),
        "delegation" => cmd_audit_delegation(args),
        "markers" => crate::marker_audit::run(args),
        _ => Err(err("audit needs a subject: naming|delegation|markers")),
    }
}

fn cmd_audit_naming(args: &[String]) -> Result<()> {
    // Use the shared logical-path selection; this command never constructs a
    // RawStore, opens a span, reads prose, or consults postings.
    let idx = load_address(args)?;
    let mut result = ase_naming::audit(&naming_sources(&idx)?);
    if let Some(project) = opt(args, "--project") {
        result.signals.retain(|s| match s.status {
            Status::Parsed | Status::Ambiguous => {
                s.candidates.iter().any(|c| c.project == project)
            }
            Status::Unparsed => s.basename.starts_with(&project),
        });
        result
            .clusters
            .retain(|c| c.project == project);
        result.ties.retain(|t| t.candidates.iter().any(|c| c.project == project));
        result.unparsed.retain(|u| u.basename.starts_with(&project));
        result.collisions.retain(|c| {
            c.left.starts_with(&project) || c.right.starts_with(&project)
        });
        result.coverage.names = result.signals.len();
        result.coverage.parsed = result
            .signals
            .iter()
            .filter(|s| s.status == Status::Parsed)
            .count();
        result.coverage.ambiguous = result
            .signals
            .iter()
            .filter(|s| s.status == Status::Ambiguous)
            .count();
        result.coverage.unparsed = result.coverage.names
            - result.coverage.parsed
            - result.coverage.ambiguous;
    }

    let rule = &result.rule;
    let coverage = &result.coverage;
    let v = json!({
        "command": "audit",
        "subject": "naming",
        "status": "ok",
        "rule": {
            "name": &rule.name,
            "version": &rule.version,
            "description": &rule.description,
            "attempt_semantics": &rule.attempt_semantics,
            "abstention": &rule.abstention
        },
        "source": {
            "kind": "address_index_metadata",
            "index": index_path(args).to_string_lossy(),
            "fields": ["file.path", "file.id", "file.provider", "session.id", "session.header.span"],
            "transcript_read": false,
            "note": "Names come from indexed path metadata; this command does not open raw spans or prose."
        },
        "coverage": {
            "names": coverage.names,
            "parsed": coverage.parsed,
            "ambiguous": coverage.ambiguous,
            "unparsed": coverage.unparsed,
            "parsed_ratio": if coverage.names == 0 {
                0.0
            } else {
                coverage.parsed as f64 / coverage.names as f64
            },
            "denominator": "all indexed source file names in scope"
        },
        "clusters": result.clusters.iter().map(|c| json!({
            "project": &c.project,
            "parent": &c.parent,
            "common_prefix": &c.common_prefix,
            "names": c.names.iter().map(|n| json!({
                "basename": &n.basename,
                "task_key": format!("{}|{}|{}|{}", n.candidate.project, n.candidate.task, n.candidate.role, n.candidate.attempt),
                "candidate": candidate_value(&n.candidate),
                "evidence": naming_evidence(&n.source, rule)
            })).collect::<Vec<Value>>()
        })).collect::<Vec<Value>>(),
        "ties": result.ties.iter().map(|t| json!({
            "parent": &t.parent,
            "basename": &t.basename,
            "decision": t.decision,
            "reason": &t.reason,
            "candidates": t.candidates.iter().map(candidate_value).collect::<Vec<Value>>(),
            "evidence": naming_evidence(&t.source, rule)
        })).collect::<Vec<Value>>(),
        "unparsed": result.unparsed.iter().map(|u| json!({
            "parent": &u.parent,
            "basename": &u.basename,
            "reason": &u.reason,
            "evidence": naming_evidence(&u.source, rule)
        })).collect::<Vec<Value>>(),
        "signals": result.signals.iter().map(|s| json!({
            "basename": &s.basename,
            "parent": &s.parent,
            "status": format!("{:?}", s.status).to_lowercase(),
            "reason": &s.reason,
            "shared_prefix": &s.shared_prefix,
            "sibling_count": s.sibling_count,
            "candidates": s.candidates.iter().map(candidate_value).collect::<Vec<Value>>(),
            "evidence": naming_evidence(&s.source, rule)
        })).collect::<Vec<Value>>(),
        "collisions": result.collisions.iter().map(|c| json!({
            "kind": c.kind,
            "left": &c.left,
            "right": &c.right,
            "note": &c.note
        })).collect::<Vec<Value>>(),
        "explain": if has(args, "--explain") {
            vec![
                "Read file.path, file.id, provider, session.id and session.header.span from the address index; no transcript, postings or model is touched.".to_string(),
                "Grouped source names by parent directory and recorded the longest prefix shared with a sibling as cluster evidence.".to_string(),
                "Parsed trailing digits as the attempt label, trailing letters as the role, then a `Task`/`T` marker or an admissible project/task digit boundary.".to_string(),
                "An explicit marker is decisive; without one, every admissible boundary is retained and more than one candidate is reported as a tie with decision abstain.".to_string(),
                "Attempt numbers are labels only: no chronology, newest attempt or ordering is inferred from them.".to_string(),
                "Project and parent stay in every task key; cross-project and cross-parent collisions are reported rather than merged.".to_string(),
            ]
        } else {
            Vec::<String>::new()
        },
        "guarantees": [
            "Metadata-only audit: no raw span is opened and no prose is decoded.",
            "Every parsed assertion carries the source path, session pointer and the rule name/version.",
            "Ambiguous and unparsed names are retained with reasons and are never promoted to task keys.",
            "No cross-project or cross-parent merge; ties are abstentions, not assignments."
        ]
    });
    emit(&v, args);
    Ok(())
}
/// Build delegation inputs from indexed session metadata only. No raw bytes
/// are read; the header span is a locator, not decoded evidence.
pub(crate) fn delegation_inputs(idx: &Index) -> Result<Vec<ase_delegation::SessionInput>> {
    use ase_delegation::{Declaration, ObservationKind, SessionInput, SessionRef};

    let mut out = Vec::with_capacity(idx.session_count as usize);
    for sidx in 0..idx.session_count {
        let s = idx.session(sidx)?;
        let f = idx.file(s.file())?;
        let path = f.path().to_string();
        let provider = f.provider().label().to_string();
        let header_span = span_label(idx.event(s.header())?.span());
        let declared = s.parent_id();
        let declaration = if declared.is_empty() {
            None
        } else if s.path_derived() {
            let path_segment = Path::new(&path)
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .map(str::to_string);
            Some(Declaration {
                declared_id: declared.to_string(),
                kind: ObservationKind::PathDerived,
                field: "source.path.parent_directory",
                payload_span: None,
                path_segment,
            })
        } else {
            Some(Declaration {
                declared_id: declared.to_string(),
                kind: ObservationKind::PayloadField,
                field: if provider == "codex" {
                    "codex.session_meta.parent"
                } else {
                    "omp.parentSession"
                },
                payload_span: Some(header_span.clone()),
                path_segment: None,
            })
        };
        out.push(SessionInput {
            session: SessionRef {
                session_idx: sidx,
                id: s.id().to_string(),
                file_id: s.file(),
                path,
                provider,
                header_span,
            },
            declaration,
        });
    }
    Ok(out)
}

fn delegation_session_value(s: &ase_delegation::SessionRef) -> Value {
    json!({
        "session_idx": s.session_idx,
        "id": &s.id,
        "file_id": s.file_id,
        "path": &s.path,
        "provider": &s.provider,
        "header_span": &s.header_span
    })
}

fn cmd_audit_delegation(args: &[String]) -> Result<()> {
    // Same shared source selection as `audit naming`: no RawStore, no span
    // read, no postings and no model.
    let idx = load_address(args)?;
    let result = ase_delegation::audit(&delegation_inputs(&idx)?);
    let rule = &result.rule;
    let rule_value = json!({ "name": &rule.name, "version": &rule.version });
    let edges = result
        .edges
        .iter()
        .map(|e| {
            json!({
                "predicate": "delegated_from",
                "from": delegation_session_value(&e.from),
                "declaration": {
                    "declared_id": &e.declaration.declared_id,
                    "observation": e.declaration.kind.label(),
                    "field": e.declaration.field,
                    "payload_span": &e.declaration.payload_span,
                    "path_segment": &e.declaration.path_segment
                },
                "status": e.status.label(),
                "reason": e.reason,
                "target": e.target.as_ref().map(delegation_session_value),
                "candidates": e.candidates.iter().map(delegation_session_value).collect::<Vec<Value>>(),
                "evidence": {
                    "declaration_anchor": e.declaration.payload_span.clone().unwrap_or_else(|| e.from.header_span.clone()),
                    "target_span": e.target.as_ref().map(|t| t.header_span.clone()),
                    "note": "A header span locates the source file; a path-derived declaration has no payload span and no id text is claimed inside the payload."
                },
                "rule": rule_value.clone(),
                "task_key": Value::Null,
                "assigned_to": Value::Null
            })
        })
        .collect::<Vec<Value>>();

    let v = json!({
        "command": "audit",
        "subject": "delegation",
        "status": "ok",
        "rule": {
            "name": &rule.name,
            "version": &rule.version,
            "domain": &rule.domain,
            "abstention": &rule.abstention,
            "task_boundary": &rule.task_boundary
        },
        "source": {
            "kind": "address_index_metadata",
            "index": index_path(args).to_string_lossy(),
            "fields": ["session.id", "session.parent_id", "session.path_derived", "session.header.span", "file.path", "file.id", "file.provider"],
            "transcript_read": false,
            "note": "Declared parent ids come from indexed session metadata; no raw span is opened, no prose is decoded and no model is called."
        },
        "counts": {
            "sessions": result.sessions,
            "declarations": result.declarations,
            "linked": result.linked,
            "unlinked_absent": result.unlinked_absent,
            "unlinked_ambiguous": result.unlinked_ambiguous,
            "unlinked_self": result.unlinked_self,
            "without_parent": result.without_parent,
            "payload_declarations": result.payload_declarations,
            "path_declarations": result.path_declarations
        },
        "edges": edges,
        "sessions_without_parent": result.sessions_without_parent.iter().map(delegation_session_value).collect::<Vec<Value>>(),
        "task_assignment": "none",
        "task_key": Value::Null,
        "explain": if has(args, "--explain") {
            vec![
                "Read session.id, session.parent_id, session.path_derived, the session header span and file path/provider from the address index; no raw bytes, prose, postings or model are touched.".to_string(),
                "Payload-field and path-derived declarations are recorded separately: a header span locates the file but never proves that the declared id text is in a payload.".to_string(),
                "A target is accepted only when exactly one indexed session id equals the declared id byte-for-byte; file order, path proximity and the index's first-match parent_idx are not used to choose a target.".to_string(),
                "Zero matches -> unlinked_absent; more than one -> unlinked_ambiguous with every candidate retained; a self reference -> unlinked_self. Candidate order is enumeration order only and never selects a target.".to_string(),
                "Every edge carries the rule name/version, both endpoint locators and task_key/assigned_to set to null: a parent declaration alone is a delegation edge, not a task assignment.".to_string()
            ]
        } else {
            Vec::<String>::new()
        },
        "guarantees": [
            "Metadata-only: no raw span is opened, no prose is decoded and no model is called.",
            "Unique exact-id targets are linked with inspectable source and target locators plus rule name/version.",
            "Missing, multiple or self targets stay unlinked with all candidates retained; no file-order or path-proximity tie-break exists.",
            "Payload and path observations are recorded separately, with no payload span claimed for a path-derived id.",
            "A parent declaration never produces a TaskKey or an assigned_to edge."
        ]
    });
    emit(&v, args);
    Ok(())
}
