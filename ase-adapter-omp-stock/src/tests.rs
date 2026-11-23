use super::*;
use ase_core::{Encoding, EventKind, FileId, Provider, Role, SpanRef};
use serde_json::{json, Value};
use std::collections::HashMap;

struct Mock {
    generation: String,
    files: Vec<ViewFile>,
    events: HashMap<FileId, Vec<ViewEvent>>,
    bytes: HashMap<FileId, Vec<u8>>,
    sidecars: HashMap<String, Vec<u8>>,
}

impl Mock {
    fn new(generation: &str, entries: Vec<(&str, Vec<Value>)>) -> Mock {
        let mut files = Vec::new();
        let mut events = HashMap::new();
        let mut bytes = HashMap::new();
        let mut global = 0u32;
        for (id, (path, lines)) in entries.into_iter().enumerate() {
            let mut content = Vec::new();
            let mut evs = Vec::new();
            let mut off = 0u64;
            let mut ordinal = 0u32;
            for value in lines {
                let line = serde_json::to_vec(&value).unwrap();
                let hash = fnv64(&line);
                let kind = match value.get("type").and_then(Value::as_str) {
                    Some("session") => EventKind::SessionMeta,
                    Some("message") => match value
                        .get("message")
                        .and_then(|m| m.get("role"))
                        .and_then(Value::as_str)
                    {
                        Some("user") => EventKind::UserInput,
                        Some("assistant") => EventKind::AssistantText,
                        Some("toolResult") => EventKind::ToolResult,
                        _ => EventKind::Unknown,
                    },
                    _ => EventKind::Notice,
                };
                let role = match kind {
                    EventKind::SessionMeta => Role::System,
                    EventKind::UserInput => Role::User,
                    EventKind::AssistantText => Role::Assistant,
                    EventKind::ToolResult => Role::Tool,
                    _ => Role::Unknown,
                };
                let tag = value
                    .get("customType")
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| value.get("type").and_then(Value::as_str).unwrap_or(""));
                evs.push(ViewEvent {
                    event: global,
                    ordinal,
                    span: SpanRef { file: id as u32, off, len: line.len() as u32 },
                    hash,
                    kind,
                    role,
                    tag: tag.to_string(),
                });
                off += line.len() as u64 + 1;
                global += 1;
                ordinal += 1;
                content.extend_from_slice(&line);
                content.push(b'\n');
            }
            let digest = sha256_hex(&content);
            files.push(ViewFile {
                id: id as u32,
                path: path.to_string(),
                provider: Provider::Omp,
                encoding: Encoding::Jsonl,
                size: content.len() as u64,
                manifest_digest: Some(digest),
            });
            events.insert(id as u32, evs);
            bytes.insert(id as u32, content);
        }
        Mock {
            generation: generation.to_string(),
            files,
            events,
            bytes,
            sidecars: HashMap::new(),
        }
    }

    fn with_sidecar(mut self, path: &str, text: &str) -> Self {
        self.sidecars.insert(path.to_string(), text.as_bytes().to_vec());
        self
    }

    fn replace_bytes(&mut self, file: FileId, bytes: Vec<u8>) {
        self.bytes.insert(file, bytes);
    }
}

impl StockView for Mock {
    fn generation(&self) -> &str {
        &self.generation
    }
    fn files(&self) -> &[ViewFile] {
        &self.files
    }
    fn events(&self, file: FileId) -> ase_core::Result<Vec<ViewEvent>> {
        Ok(self.events.get(&file).cloned().unwrap_or_default())
    }
    fn bytes(&self, file: FileId) -> ase_core::Result<std::sync::Arc<[u8]>> {
        Ok(self
            .bytes
            .get(&file)
            .cloned()
            .map(|b| std::sync::Arc::from(b.into_boxed_slice()))
            .unwrap_or_default())
    }
    fn span_bytes(&self, span: SpanRef) -> ase_core::Result<Vec<u8>> {
        let bytes = self.bytes.get(&span.file).cloned().unwrap_or_default();
        let start = span.off as usize;
        let end = start + span.len as usize;
        Ok(bytes[start..end].to_vec())
    }
    fn event_at(&self, span: SpanRef) -> ase_core::Result<Option<ViewEvent>> {
        Ok(self
            .events
            .get(&span.file)
            .and_then(|evs| evs.iter().find(|e| e.span == span).cloned()))
    }
    fn session_header(&self, file: FileId) -> ase_core::Result<Option<ViewEvent>> {
        Ok(self
            .events
            .get(&file)
            .and_then(|evs| evs.iter().find(|e| e.kind == EventKind::SessionMeta).cloned()))
    }
    fn sidecar_bytes(&self, path: &str) -> ase_core::Result<Option<Vec<u8>>> {
        Ok(self.sidecars.get(path).cloned())
    }
}

const PARENT: &str = "C:/SYN/root/2026-01-01T00-00-00-000Z_p.jsonl";
const CHILD_DIR: &str = "C:/SYN/root/2026-01-01T00-00-00-000Z_p";

fn session(id: &str, cwd: &str, parent: Option<&str>) -> Value {
    let mut v = json!({
        "type": "session",
        "version": 3,
        "id": id,
        "timestamp": "2026-01-01T00:00:00.000Z",
        "cwd": cwd
    });
    if let Some(p) = parent {
        v["parentSession"] = json!(p);
    }
    v
}

fn task_call(call_id: &str, names: &[&str], prompt: &str) -> Value {
    let tasks: Vec<Value> = names
        .iter()
        .map(|n| json!({"name": n, "agent": "gold-task", "task": prompt}))
        .collect();
    json!({
        "type": "message",
        "id": "a1",
        "parentId": null,
        "message": {
            "role": "assistant",
            "content": [{
                "type": "toolCall",
                "id": call_id,
                "name": "task",
                "arguments": {"i": "dispatch", "tasks": tasks}
            }]
        }
    })
}

fn result(call_id: &str, text: &str) -> Value {
    json!({
        "type": "message",
        "id": "r1",
        "parentId": null,
        "message": {
            "role": "toolResult",
            "toolCallId": call_id,
            "toolName": "task",
            "content": [{"type": "text", "text": text}]
        }
    })
}

fn claims_of<'a>(report: &'a Report, layer: &str, state: &str) -> Vec<&'a Claim> {
    report
        .claims
        .iter()
        .filter(|c| c.layer == layer && c.state == state)
        .collect()
}

#[test]
fn matching_case_confirms_link_and_label_and_repo_workspace() {
    let parent = PARENT.to_string();
    let child = format!("{CHILD_DIR}/S3T1A1.jsonl");
    let mock = Mock::new(
        "gen-1",
        vec![
            (
                PARENT,
                vec![
                    session("p", "C:\\SYN\\demo", None),
                    task_call("call-7", &["S3T1A1"], "Target: S3-T1 attempt 1"),
                    result("call-7", "Spawned agent `S3T1A1` (job `S3T1A1`)."),
                ],
            ),
            (
                &child,
                vec![
                    session("p", "C:\\SYN\\demo", Some(&parent)),
                    json!({"type":"message","message":{"role":"user","content":[{"type":"text","text":"Target: S3-T1 attempt 1"}]}}),
                ],
            ),
        ],
    )
    .with_sidecar(
        "C:/SYN/root/2026-01-01T00-00-00-000Z_p/S3T1A1.md",
        "Handoff\n- task_id: S3-T1\n- attempt_id: A1\n",
    );

    let report = scan(&mock, &Options { read_sidecars: true, ..Options::default() }).unwrap();
    assert_eq!(claims_of(&report, LAYER_LINK, STATE_CONFIRMED).len(), 1);
    assert_eq!(claims_of(&report, LAYER_TASK, STATE_CONFIRMED).len(), 2); // label + token
    assert_eq!(claims_of(&report, LAYER_REPO, STATE_CONFIRMED).len(), 2); // parent + child workspace
    assert_eq!(claims_of(&report, LAYER_REPO, STATE_CANDIDATE).len(), 2); // repo keys

    let reopened = reopen(
        &mock,
        &ClaimsFile {
            adapter: ADAPTER_NAME.to_string(),
            generation: "gen-1".to_string(),
            evidence_selector_version: SELECTOR_VERSION.to_string(),
            claims: report.claims.clone(),
        },
        None,
    )
    .unwrap();
    assert_eq!(reopened.status, "verified");
    assert!(reopened.claims.iter().all(|c| c.assessed_state != "stale"));
}

#[test]
fn two_calls_with_two_children_are_multi_task_not_conflict() {
    let parent = PARENT.to_string();
    let c1 = format!("{CHILD_DIR}/S3T1A1.jsonl");
    let c2 = format!("{CHILD_DIR}/S3T2A1.jsonl");
    let mock = Mock::new(
        "gen-1",
        vec![
            (
                PARENT,
                vec![
                    session("p", "C:\\SYN\\demo", None),
                    task_call("call-7", &["S3T1A1"], "Target: S3-T1 attempt 1"),
                    result("call-7", "Spawned agent `S3T1A1` (job `S3T1A1`)."),
                    task_call("call-9", &["S3T2A1"], "Target: S3-T2 attempt 1"),
                    result("call-9", "Spawned agent `S3T2A1` (job `S3T2A1`)."),
                ],
            ),
            (c1.as_str(), vec![session("p", "C:\\SYN\\demo", Some(&parent))]),
            (c2.as_str(), vec![session("p", "C:\\SYN\\demo", Some(&parent))]),
        ],
    );
    let report = scan(&mock, &Options::default()).unwrap();
    assert_eq!(report.layers[LAYER_TASK].session_states.get("multi_task"), Some(&1));
    assert_eq!(claims_of(&report, LAYER_LINK, STATE_CONFIRMED).len(), 2);
    assert!(claims_of(&report, LAYER_LINK, STATE_CONFLICT).is_empty());
}

#[test]
fn missing_child_is_missing_not_candidate_or_error() {
    let mock = Mock::new(
        "gen-1",
        vec![(
            PARENT,
            vec![
                session("p", "C:\\SYN\\demo", None),
                task_call("call-7", &["NoChild"], "Target: S3-T9 attempt 1"),
            ],
        )],
    );
    let report = scan(&mock, &Options::default()).unwrap();
    assert_eq!(claims_of(&report, LAYER_LINK, STATE_MISSING).len(), 1);
    assert_eq!(claims_of(&report, LAYER_TASK, STATE_CANDIDATE).len(), 2); // label + token
    assert_eq!(claims_of(&report, LAYER_REPO, STATE_CONFIRMED).len(), 1);
}

#[test]
fn parent_result_and_child_header_disagreement_keeps_both_as_conflict() {
    let child = format!("{CHILD_DIR}/S3T1A1.jsonl");
    let mock = Mock::new(
        "gen-1",
        vec![
            (
                PARENT,
                vec![
                    session("p", "C:\\SYN\\demo", None),
                    task_call("call-7", &["S3T1A1"], "Target: S3-T1 attempt 1"),
                    result("call-7", "Spawned agent `S3T1A1` (job `S3T1A1`)."),
                ],
            ),
            (
                &child,
                vec![session(
                    "p",
                    "C:\\SYN\\demo",
                    Some("C:/SYN/other/2026-01-01T00-00-00-000Z_other.jsonl"),
                )],
            ),
        ],
    );
    let report = scan(&mock, &Options::default()).unwrap();
    let conflict = claims_of(&report, LAYER_LINK, STATE_CONFLICT);
    assert_eq!(conflict.len(), 1);
    assert!(!conflict[0].refutes.is_empty());
    assert!(claims_of(&report, LAYER_LINK, STATE_CONFIRMED).is_empty());
}

#[test]
fn opposing_parent_and_child_labels_conflict_without_tie_break() {
    let child = format!("{CHILD_DIR}/S3T2A1.jsonl");
    let mock = Mock::new(
        "gen-1",
        vec![
            (
                PARENT,
                vec![
                    session("p", "C:\\SYN\\demo", None),
                    task_call("call-7", &["S3T1A1"], "Target: S3-T1 attempt 1"),
                    result("call-7", "Spawned agent `S3T2A1` (job `S3T2A1`)."),
                ],
            ),
            (child.as_str(), vec![session("p", "C:\\SYN\\demo", Some(PARENT))]),
        ],
    );
    let report = scan(&mock, &Options::default()).unwrap();
    let labels: Vec<&Claim> = report
        .claims
        .iter()
        .filter(|c| {
            c.layer == LAYER_TASK
                && c.value.get("kind").and_then(Value::as_str) == Some("child_label")
        })
        .collect();
    assert_eq!(labels.len(), 2);
    assert!(labels.iter().all(|c| c.state == STATE_CONFLICT));
}

#[test]
fn edited_raw_marks_reopened_claim_stale() {
    let child = format!("{CHILD_DIR}/S3T1A1.jsonl");
    let mut mock = Mock::new(
        "gen-1",
        vec![
            (
                PARENT,
                vec![
                    session("p", "C:\\SYN\\demo", None),
                    task_call("call-7", &["S3T1A1"], "Target: S3-T1 attempt 1"),
                    result("call-7", "Spawned agent `S3T1A1` (job `S3T1A1`)."),
                ],
            ),
            (child.as_str(), vec![session("p", "C:\\SYN\\demo", Some(PARENT))]),
        ],
    );
    let report = scan(&mock, &Options::default()).unwrap();
    let claims = ClaimsFile {
        adapter: ADAPTER_NAME.to_string(),
        generation: "gen-1".to_string(),
        evidence_selector_version: SELECTOR_VERSION.to_string(),
        claims: report.claims.clone(),
    };
    let parent_file = mock
        .files()
        .iter()
        .find(|f| f.path == PARENT)
        .unwrap()
        .id;
    let raw = mock.bytes(parent_file).unwrap();
    let mut edited = raw.to_vec();
    let pos = edited.windows(4).position(|w| w == b"S3-T").unwrap();
    edited[pos] = b'X';
    mock.replace_bytes(parent_file, edited);
    let reopened = reopen(&mock, &claims, None).unwrap();
    assert_eq!(reopened.status, "stale");
    assert!(reopened.claims.iter().any(|c| c.assessed_state == "stale"));
}

#[test]
fn generation_mismatch_never_reads_other_generation() {
    let mock = Mock::new(
        "gen-2",
        vec![(
            PARENT,
            vec![
                session("p", "C:\\SYN\\demo", None),
                task_call("call-7", &["S3T1A1"], "Target: S3-T1 attempt 1"),
            ],
        )],
    );
    let claims = ClaimsFile {
        adapter: ADAPTER_NAME.to_string(),
        generation: "gen-1".to_string(),
        evidence_selector_version: SELECTOR_VERSION.to_string(),
        claims: vec![],
    };
    let report = reopen(&mock, &claims, None).unwrap();
    assert_eq!(report.status, "generation_mismatch");
}

#[test]
fn path_evidence_and_selector_version_round_trip() {
    let file = ViewFile {
        id: 3,
        path: "C:/SYN/demo/file.jsonl".to_string(),
        provider: Provider::Omp,
        encoding: Encoding::Jsonl,
        size: 1,
        manifest_digest: None,
    };
    let ev = crate::scan::path_evidence(&file, "gen", "filename");
    assert_eq!(ev.selector, "stock-sel-v1|path.filename");
    assert!(ev.block_digest.is_some());
}
