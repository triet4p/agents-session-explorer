//! Parent-child linkage claims derived from stock task calls.
//!
//! A link is confirmed only from one task call, one result naming exactly one
//! child and a matching child header; every other shape stays candidate,
//! missing or conflict. Sibling children are never merged.

use ase_core::FileId;
use serde_json::{json, Value};
use std::collections::HashSet;

use crate::journal::{
    CallRecord, LoadedFile, child_file_by_name, child_stem, header_parent_matches,
};
use crate::model::*;
use crate::provenance::make_evidence;

pub(crate) fn link_child_context(loaded: &[LoadedFile], parent: &LoadedFile, name: &str) -> Option<usize> {
    let child = child_file_by_name(loaded, &parent.file, name)?;
    loaded.iter().position(|f| f.file.id == child.file.id)
}

pub(crate) fn link_claims_for_call(
    loaded: &[LoadedFile],
    parent: &LoadedFile,
    call: &CallRecord,
    generation: &str,
    seen_children: &mut HashSet<FileId>,
) -> Vec<Claim> {
    let session_id = parent.header.as_ref().and_then(|h| h.session_id.clone());
    let mut claims = Vec::new();
    let assignment_evidence: Vec<EvidenceRef> = call
        .assignments
        .iter()
        .filter_map(|a| a.name_evidence.clone())
        .collect();
    let base_evidence: Vec<EvidenceRef> = call
        .start_evidence
        .iter()
        .chain(call.result_evidence.iter())
        .cloned()
        .collect();
    let event = call
        .args_event
        .as_ref()
        .map(|e| e.event)
        .or_else(|| call.start_event.as_ref().map(|e| e.event));

    if call.spawns.is_empty() {
        // A call with assignment names but no result may still have a child
        // journal. That is a candidate, never a confirmed link.
        let mut any = false;
        for a in &call.assignments {
            let Some(name) = &a.name else { continue };
            if let Some(child_idx) = link_child_context(loaded, parent, name) {
                any = true;
                let child = &loaded[child_idx];
                seen_children.insert(child.file.id);
                let mut evidence = base_evidence.clone();
                evidence.extend(a.name_evidence.clone());
                if let Some(hev) = child.header_event.as_ref() {
                    evidence.push(make_evidence(
                        &child.file,
                        generation,
                        hev,
                        &selector_session("parentSession"),
                        &child.header_line,
                        child
                            .header
                            .as_ref()
                            .and_then(|h| h.parent_session.clone())
                            .unwrap_or_default()
                            .as_bytes(),
                    ));
                }
                let mut subject = Subject::for_call(
                    &parent.file,
                    session_id.as_deref(),
                    event,
                    Some(&call.call_id),
                )
                .with_child(&child.file);
                subject.event = event;
                claims.push(
                    Claim::new(
                        LAYER_LINK,
                        STATE_CANDIDATE,
                        "delegated_from",
                        json!({
                            "parent_session_id": session_id,
                            "parent_tool_call_id": call.call_id,
                            "child_session_id": child.header.as_ref().and_then(|h| h.session_id.clone()),
                            "child_label": name,
                            "child_journal_present": true,
                            "result_named_child": false
                        }),
                        subject,
                        "mixed",
                        LINK_RULE,
                        LINK_VERSION,
                        evidence,
                    )
                    .reason("assignment name and child journal exist, but no result names the child")
                    .context("a task call without a result does not create a confirmed link"),
                );
            }
        }
        if !any {
            let failed = call
                .spawns
                .iter()
                .find(|(s, _)| s.shape == "failed")
                .map(|(s, ev)| (s, ev));
            let mut evidence = base_evidence.clone();
            if let Some((_, ev)) = failed {
                evidence.push(ev.clone());
            }
            claims.push(
                Claim::new(
                    LAYER_LINK,
                    STATE_MISSING,
                    "delegated_from",
                    json!({
                        "parent_session_id": session_id,
                        "parent_tool_call_id": call.call_id,
                        "child_session_id": Value::Null
                    }),
                    Subject::for_call(&parent.file, session_id.as_deref(), event, Some(&call.call_id)),
                    "observed",
                    LINK_RULE,
                    LINK_VERSION,
                    evidence,
                )
                .reason(if failed.is_some() {
                    "task result reports a preflight failure and no child journal"
                } else {
                    "no result and no child journal for this task call"
                }),
            );
        }
        return claims;
    }

    // One result can name several sibling children. Keep every name separate;
    // do not merge siblings and do not promote a shared call to confirmed.
    let multi = call
        .spawns
        .iter()
        .filter(|(s, _)| s.shape != "failed")
        .count()
        > 1;
    for (spawn, spawn_evidence) in &call.spawns {
        let child_idx = link_child_context(loaded, parent, &spawn.name);
        let mut evidence = assignment_evidence.clone();
        evidence.push(spawn_evidence.clone());
        let mut reasons = Vec::new();
        let mut refutes = Vec::new();
        let (state, child) = match child_idx {
            Some(ci) => {
                let child = &loaded[ci];
                seen_children.insert(child.file.id);
                if let Some(hev) = child.header_event.as_ref() {
                    let parent_ref = child
                        .header
                        .as_ref()
                        .and_then(|h| h.parent_session.clone())
                        .unwrap_or_default();
                    evidence.push(make_evidence(
                        &child.file,
                        generation,
                        hev,
                        &selector_session("parentSession"),
                        &child.header_line,
                        parent_ref.as_bytes(),
                    ));
                }
                match header_parent_matches(child, &parent.file, session_id.as_deref()) {
                    Some(true) => {
                        if spawn.shape == "failed" {
                            reasons.push("result reports failure for an existing child journal".to_string());
                            (STATE_CONFLICT, Some(child))
                        } else if multi {
                            reasons.push(
                                "one task call names multiple children; each link stays candidate"
                                    .to_string(),
                            );
                            (STATE_CANDIDATE, Some(child))
                        } else {
                            reasons.push(
                                "task call, result child name and child parentSession form one structure"
                                    .to_string(),
                            );
                            (STATE_CONFIRMED, Some(child))
                        }
                    }
                    Some(false) => {
                        refutes.push(
                            "child header parentSession names a different journal".to_string(),
                        );
                        (STATE_CONFLICT, Some(child))
                    }
                    None => {
                        reasons.push("child journal has no parentSession header".to_string());
                        (STATE_CANDIDATE, Some(child))
                    }
                }
            }
            None => {
                if spawn.shape == "failed" {
                    (STATE_MISSING, None)
                } else {
                    (STATE_MISSING, None)
                }
            }
        };
        let mut subject = Subject::for_call(
            &parent.file,
            session_id.as_deref(),
            event,
            Some(&call.call_id),
        );
        if let Some(c) = child {
            subject = subject.with_child(&c.file);
        }
        let mut claim = Claim::new(
            LAYER_LINK,
            state,
            "delegated_from",
            json!({
                "parent_session_id": session_id,
                "parent_tool_call_id": call.call_id,
                "child_session_id": child.and_then(|c| c.header.as_ref().and_then(|h| h.session_id.clone())),
                "child_label": spawn.name,
                "child_journal_present": child.is_some(),
                "result_shape": spawn.shape,
                "result_status": spawn.status,
                "multi_child_call": multi
            }),
            subject,
            if child.is_some() { "mixed" } else { "observed" },
            LINK_RULE,
            LINK_VERSION,
            evidence,
        )
        .context(format!("spawn name offset {}", spawn.offset));
        if let Some(r) = reasons.pop() {
            claim = claim.reason(r);
        }
        for r in reasons {
            claim = claim.reason(r);
        }
        if child.is_none() {
            if spawn.shape == "failed" {
                claim = claim.reason(
                    "task result reports a preflight failure and no child journal",
                );
            } else {
                claim = claim.reason("no child journal matches the result child name");
            }
        }
        for r in refutes {
            claim = claim.refute(r);
        }
        claims.push(claim);
    }

    // A child journal whose header names this parent but that no result named
    // is still a candidate: parentSession alone is not confirmed.
    for child in loaded.iter().filter(|f| f.parsed_parent == Some(parent.file.id)) {
        let name = child_stem(&child.file.path);
        if call.spawns.iter().any(|(s, _)| s.name == name) {
            continue;
        }
        if !child_stem_matches_any_assignment(call, &name) {
            continue;
        }
        seen_children.insert(child.file.id);
        let mut evidence: Vec<EvidenceRef> = base_evidence.clone();
        if let Some(hev) = child.header_event.as_ref() {
            let parent_ref = child
                .header
                .as_ref()
                .and_then(|h| h.parent_session.clone())
                .unwrap_or_default();
            evidence.push(make_evidence(
                &child.file,
                generation,
                hev,
                &selector_session("parentSession"),
                &child.header_line,
                parent_ref.as_bytes(),
            ));
        }
        claims.push(
            Claim::new(
                LAYER_LINK,
                STATE_CANDIDATE,
                "delegated_from",
                json!({
                    "parent_session_id": session_id,
                    "parent_tool_call_id": call.call_id,
                    "child_session_id": child.header.as_ref().and_then(|h| h.session_id.clone()),
                    "child_label": name,
                    "child_journal_present": true,
                    "result_named_child": false
                }),
                Subject::for_call(&parent.file, session_id.as_deref(), event, Some(&call.call_id))
                    .with_child(&child.file),
                "mixed",
                LINK_RULE,
                LINK_VERSION,
                evidence,
            )
            .reason("child header names this parent but no task result names the child"),
        );
    }

    claims
}

fn child_stem_matches_any_assignment(call: &CallRecord, stem: &str) -> bool {
    call.assignments.iter().any(|a| {
        a.name
            .as_deref()
            .map(|n| n == stem || n.eq_ignore_ascii_case(stem))
            .unwrap_or(false)
    })
}
