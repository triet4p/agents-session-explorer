//! Dispatch-backed task membership.
//!
//! Membership is defined only by the native OMP `task_dispatch_v1` /
//! `task_dispatch_child_v1` records joined by the `dispatch-v1` rule. The
//! loader below is the same one the `resolve task:` path uses; it reopened
//! every record span from raw bytes before trusting it. Naming candidates,
//! path matches and `parentSession` declarations are never passed into the
//! reconciliation, so they cannot widen membership.

use ase_core::{err, FileId, Provider, RawStore, Result, SpanRef};
use ase_index::Index;
use ase_task::{
    DispatchChildLink, DispatchInput, DispatchIntent, DispatchIssue, DispatchOutcome, Evidence,
    MarkerLayer, Status,
};
use serde_json::Value;

/// Native OMP dispatch provenance records, addressed by their custom-type tag.
const DISPATCH_PARENT_TAG: &str = "task_dispatch_v1";
const DISPATCH_CHILD_TAG: &str = "task_dispatch_child_v1";

fn span_label(span: SpanRef) -> String {
    format!("span:{}:{}:{}", span.file, span.off, span.len)
}

/// Producer identity grammar shared by project/task/attempt:
/// `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$`.
fn valid_dispatch_identity(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 128 || !bytes[0].is_ascii_alphanumeric() {
        return false;
    }
    bytes
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

/// `crypto.randomUUID()` canonical form, without pinning a UUID version.
fn canonical_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    bytes.iter().enumerate().all(|(index, byte)| match index {
        8 | 13 | 18 | 23 => *byte == b'-',
        _ => byte.is_ascii_hexdigit(),
    })
}

fn push_dispatch_issue(
    input: &mut DispatchInput,
    record: &str,
    kind: &str,
    dispatch_id: Option<String>,
    task: Option<String>,
    event: u32,
    span: String,
    detail: String,
) {
    input.issues.push(DispatchIssue {
        record: record.to_string(),
        kind: kind.to_string(),
        dispatch_id,
        task,
        event,
        span,
        detail,
    });
}

/// Re-open every indexed dispatch record, verify its span checksum and parse
/// only the native producer schema. No prose is read or emitted.
pub fn dispatch_input(idx: &Index, store: &dyn RawStore) -> Result<DispatchInput> {
    let mut input = DispatchInput::default();
    for event in 0..idx.event_count {
        let view = idx.event(event)?;
        let tag = view.tag();
        let is_parent = tag == DISPATCH_PARENT_TAG;
        if !is_parent && tag != DISPATCH_CHILD_TAG {
            continue;
        }
        let file = idx.file(view.span().file)?;
        if file.provider() != Provider::Omp {
            continue;
        }
        let span = span_label(view.span());
        let raw = match store.span(view.span()) {
            Ok(raw) => raw,
            Err(error) => {
                push_dispatch_issue(
                    &mut input,
                    if is_parent { "parent" } else { "child" },
                    "unreadable_span",
                    None,
                    None,
                    event,
                    span,
                    format!("raw span could not be re-read: {error}"),
                );
                continue;
            }
        };
        if ase_ingest::util::fnv64(&raw) != view.hash() {
            push_dispatch_issue(
                &mut input,
                if is_parent { "parent" } else { "child" },
                "span_hash_mismatch",
                None,
                None,
                event,
                span,
                "the re-read span checksum does not match the indexed checksum".to_string(),
            );
            continue;
        }
        let session_index = view.session();
        if session_index == ase_core::NONE {
            push_dispatch_issue(
                &mut input,
                if is_parent { "parent" } else { "child" },
                "unresolved_session",
                None,
                None,
                event,
                span,
                "the record is not assigned to an indexed journal session".to_string(),
            );
            continue;
        }
        let session = idx.session(session_index)?;
        if session.path_derived() {
            push_dispatch_issue(
                &mut input,
                if is_parent { "parent" } else { "child" },
                "path_derived_session",
                None,
                None,
                event,
                span,
                "the record is not in a journal with an explicit native session header".to_string(),
            );
            continue;
        }
        let containing_session_id = session.id().to_string();
        if containing_session_id.is_empty() {
            push_dispatch_issue(
                &mut input,
                if is_parent { "parent" } else { "child" },
                "unresolved_session",
                None,
                None,
                event,
                span,
                "the containing journal session id is empty".to_string(),
            );
            continue;
        }
        let parsed: Value = match serde_json::from_slice(&raw) {
            Ok(value) => value,
            Err(_) => {
                push_dispatch_issue(
                    &mut input,
                    if is_parent { "parent" } else { "child" },
                    "malformed_record",
                    None,
                    None,
                    event,
                    span,
                    "the record line is not valid JSON".to_string(),
                );
                continue;
            }
        };
        let shape_ok = parsed.get("type").and_then(Value::as_str) == Some("custom")
            && parsed.get("customType").and_then(Value::as_str) == Some(tag);
        let data = parsed.get("data");
        if !shape_ok || !data.is_some_and(Value::is_object) {
            push_dispatch_issue(
                &mut input,
                if is_parent { "parent" } else { "child" },
                "record_shape",
                None,
                None,
                event,
                span,
                "the record is not a custom entry with an object data payload".to_string(),
            );
            continue;
        }
        let data = data.expect("object data checked");
        let task_hint = data
            .get("task_id")
            .and_then(Value::as_str)
            .filter(|value| valid_dispatch_identity(value))
            .map(str::to_string);
        let dispatch_id = match data.get("dispatch_id").and_then(Value::as_str) {
            Some(value) if canonical_uuid(value) => value.to_string(),
            _ => {
                push_dispatch_issue(
                    &mut input,
                    if is_parent { "parent" } else { "child" },
                    "invalid_dispatch_id",
                    None,
                    task_hint,
                    event,
                    span,
                    "dispatch_id is missing or not a canonical runtime UUID".to_string(),
                );
                continue;
            }
        };
        if is_parent {
            let parent_session_id = match data.get("parent_session_id").and_then(Value::as_str) {
                Some(value) if !value.is_empty() => value.to_string(),
                _ => {
                    push_dispatch_issue(
                        &mut input,
                        "parent",
                        "invalid_parent_record",
                        Some(dispatch_id),
                        task_hint,
                        event,
                        span,
                        "parent_session_id is missing or empty".to_string(),
                    );
                    continue;
                }
            };
            let project = match data.get("project_id").and_then(Value::as_str) {
                Some(value) if valid_dispatch_identity(value) => value.to_string(),
                _ => {
                    push_dispatch_issue(
                        &mut input,
                        "parent",
                        "invalid_parent_identity",
                        Some(dispatch_id),
                        task_hint,
                        event,
                        span,
                        "project_id is missing or violates the producer identity grammar".to_string(),
                    );
                    continue;
                }
            };
            let task = match data.get("task_id").and_then(Value::as_str) {
                Some(value) if valid_dispatch_identity(value) => value.to_string(),
                _ => {
                    push_dispatch_issue(
                        &mut input,
                        "parent",
                        "invalid_parent_identity",
                        Some(dispatch_id),
                        None,
                        event,
                        span,
                        "task_id is missing or violates the producer identity grammar".to_string(),
                    );
                    continue;
                }
            };
            let attempt = match data.get("attempt_id").and_then(Value::as_str) {
                Some(value) if valid_dispatch_identity(value) => value.to_string(),
                _ => {
                    push_dispatch_issue(
                        &mut input,
                        "parent",
                        "invalid_parent_identity",
                        Some(dispatch_id),
                        Some(task),
                        event,
                        span,
                        "attempt_id is missing or violates the producer identity grammar".to_string(),
                    );
                    continue;
                }
            };
            let spawn_index = match data.get("spawn_index").and_then(Value::as_u64) {
                Some(value) if value <= u32::MAX as u64 => value as u32,
                _ => {
                    push_dispatch_issue(
                        &mut input,
                        "parent",
                        "invalid_spawn_index",
                        Some(dispatch_id),
                        Some(task),
                        event,
                        span,
                        "spawn_index is missing or not an unsigned 32-bit integer".to_string(),
                    );
                    continue;
                }
            };
            let parent_tool_call_id = match data.get("parent_tool_call_id") {
                None | Some(Value::Null) => None,
                Some(Value::String(value)) => Some(value.clone()),
                Some(_) => {
                    push_dispatch_issue(
                        &mut input,
                        "parent",
                        "invalid_parent_record",
                        Some(dispatch_id),
                        Some(task),
                        event,
                        span,
                        "parent_tool_call_id is present but not a string".to_string(),
                    );
                    continue;
                }
            };
            input.parents.push(DispatchIntent {
                dispatch_id,
                parent_session_id,
                parent_tool_call_id,
                spawn_index,
                project,
                task,
                attempt,
                event,
                span,
                file_id: view.span().file,
                containing_session_id,
                span_verified: true,
            });
        } else {
            let session_id = match data.get("session_id").and_then(Value::as_str) {
                Some(value) if !value.is_empty() => value.to_string(),
                _ => {
                    push_dispatch_issue(
                        &mut input,
                        "child",
                        "invalid_child_record",
                        Some(dispatch_id),
                        None,
                        event,
                        span,
                        "session_id is missing or empty".to_string(),
                    );
                    continue;
                }
            };
            input.children.push(DispatchChildLink {
                dispatch_id,
                session_id,
                event,
                span,
                file_id: view.span().file,
                containing_session_id,
                span_verified: true,
            });
        }
    }
    Ok(input)
}

/// One admitted dispatch join, reduced to the executing attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptMembership {
    pub dispatch_id: String,
    pub project: String,
    pub task: String,
    pub attempt: String,
    pub parent_session_id: String,
    pub child_session_id: String,
    pub parent_file_id: FileId,
    pub parent_event: u32,
    pub parent_span: String,
    pub child_file_id: FileId,
    pub child_event: u32,
    pub child_span: String,
}

impl AttemptMembership {
    /// Stable sort key: attempt label, then the runtime dispatch id.
    pub fn order_key(&self) -> (&str, &str) {
        (&self.attempt, &self.dispatch_id)
    }
}

/// Dispatch-only membership for one task token.
#[derive(Clone, Debug)]
pub struct TaskMembership {
    pub token: String,
    pub status: Status,
    pub task_key: Option<String>,
    pub attempts: Vec<AttemptMembership>,
    pub dispatch: DispatchOutcome,
    pub explain: Vec<String>,
}

/// Reconcile one token from native dispatch records only.
///
/// The reconciliation receives no naming, marker, path or delegation input, so
/// no other layer can contribute a project scope. An `ambiguous` or
/// `unresolved` outcome yields no attempts, and the record-level issues stay
/// attached to the result instead of being silently dropped.
pub fn membership(idx: &Index, store: &dyn RawStore, token: &str) -> Result<TaskMembership> {
    if token.trim().is_empty() {
        return Err(err("membership needs a non-empty task token"));
    }
    let dispatch = dispatch_input(idx, store)?;
    let resolution = ase_task::resolve(&ase_task::Input {
        token: token.to_string(),
        naming: Vec::new(),
        markers: Vec::new(),
        delegations: Vec::new(),
        paths: Vec::new(),
        dispatch,
        marker_layer: MarkerLayer {
            generation: None,
            unavailable: Some(
                "motif membership is derived from native dispatch records only".to_string(),
            ),
            blocks_total: 0,
            blocks_scanned: 0,
            excluded_role_blocks: 0,
            unverified_blocks: 0,
        },
    });

    let mut attempts = Vec::new();
    if resolution.status == Status::Assigned {
        for evidence in &resolution.evidence {
            let Evidence::Dispatch(join) = evidence else {
                continue;
            };
            if join.task != token {
                continue;
            }
            attempts.push(AttemptMembership {
                dispatch_id: join.dispatch_id.clone(),
                project: join.project.clone(),
                task: join.task.clone(),
                attempt: join.attempt.clone(),
                parent_session_id: join.parent_session_id.clone(),
                child_session_id: join.child_session_id.clone(),
                parent_file_id: join.parent_file_id,
                parent_event: join.parent_event,
                parent_span: join.parent_span.clone(),
                child_file_id: join.child_file_id,
                child_event: join.child_event,
                child_span: join.child_span.clone(),
            });
        }
        attempts.sort_by(|left, right| left.order_key().cmp(&right.order_key()));
        attempts.dedup_by(|left, right| {
            left.dispatch_id == right.dispatch_id && left.child_session_id == right.child_session_id
        });
    }

    Ok(TaskMembership {
        token: token.to_string(),
        status: resolution.status,
        task_key: resolution.task_key,
        attempts,
        dispatch: resolution.dispatch,
        explain: resolution.explain,
    })
}
