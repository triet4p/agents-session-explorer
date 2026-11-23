//! OMP stock claim derivation.
//!
//! The adapter reads source journals through `StockView`. It emits claims only;
//! nothing in this module writes to an index, resolver input or motif
//! membership.

use ase_core::{FileId, Result, SpanRef, err};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::extract::{
    extract_path_mentions, extract_spawns, extract_task_id_markers, extract_task_tokens, SpawnHit,
};
use crate::model::*;

#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Bound the number of root journals read. `None` scans all selected roots.
    pub max_parent_files: Option<usize>,
    /// Read a child handoff `.md` for independent task-token corroboration.
    pub read_sidecars: bool,
}

struct LoadedFile {
    file: ViewFile,
    header: Option<Header>,
    header_event: Option<ViewEvent>,
    header_line: Vec<u8>,
    parsed_parent: Option<FileId>,
    is_child: bool,
}

#[derive(Clone)]
struct TokenClue {
    token: String,
    source_field: String,
    source_kind: String,
    /// A token near an assignment heading/marker is strong enough to
    /// corroborate a task identity. Later prose tokens stay mentions.
    strong: bool,
    evidence: EvidenceRef,
}

#[derive(Clone, Copy)]
enum AssignSource {
    Content(usize),
    DataArgs,
}

#[derive(Clone)]
struct Assignment {
    index: usize,
    source: AssignSource,
    name: Option<String>,
    agent: Option<String>,
    name_evidence: Option<EvidenceRef>,
    prompt_evidence: Option<EvidenceRef>,
    prompt: Option<String>,
    solution_evidence: Option<EvidenceRef>,
    solution_space: Option<String>,
}

struct CallRecord {
    call_id: String,
    args_event: Option<ViewEvent>,
    start_event: Option<ViewEvent>,
    assignments: Vec<Assignment>,
    intent: Option<String>,
    intent_evidence: Option<EvidenceRef>,
    start_evidence: Vec<EvidenceRef>,
    result_evidence: Vec<EvidenceRef>,
    spawns: Vec<(SpawnHit, EvidenceRef)>,
    result_texts: Vec<String>,
    context_tokens: Vec<TokenClue>,
}

struct ParsedParent {
    calls: Vec<CallRecord>,
    reminder_cwds: Vec<(String, EvidenceRef)>,
    prose_tokens: Vec<TokenClue>,
    path_clues: Vec<String>,
}

fn slice_span(bytes: &[u8], span: SpanRef) -> Result<&[u8]> {
    let start = span.off as usize;
    let end = start
        .checked_add(span.len as usize)
        .ok_or_else(|| err("span length overflow"))?;
    if end > bytes.len() {
        return Err(err(format!(
            "span out of range: file={} off={} len={} source_len={}",
            span.file, span.off, span.len, bytes.len()
        )));
    }
    Ok(&bytes[start..end])
}

fn make_evidence(
    file: &ViewFile,
    generation: &str,
    event: &ViewEvent,
    selector: &str,
    line: &[u8],
    block: &[u8],
) -> EvidenceRef {
    EvidenceRef {
        provider: file.provider.label().to_string(),
        generation_id: generation.to_string(),
        coordinate: file.encoding.coordinate().to_string(),
        file_id: Some(file.id),
        path: file.path.clone(),
        event: Some(event.event),
        selector: selector.to_string(),
        selector_version: SELECTOR_VERSION.to_string(),
        span: event.span.into(),
        span_checksum: Some(format!("{:016x}", event.hash)),
        line_digest: sha256_hex(line),
        block_digest: Some(sha256_hex(block)),
        manifest_digest: file.manifest_digest.clone(),
    }
}

pub(crate) fn path_evidence(file: &ViewFile, generation: &str, field: &str) -> EvidenceRef {
    let selector = format!("{SELECTOR_VERSION}|path.{field}");
    let normalized = path_key(&file.path);
    EvidenceRef {
        provider: file.provider.label().to_string(),
        generation_id: generation.to_string(),
        coordinate: file.encoding.coordinate().to_string(),
        file_id: Some(file.id),
        path: file.path.clone(),
        event: None,
        selector,
        selector_version: SELECTOR_VERSION.to_string(),
        span: SpanJson { file: file.id, off: 0, len: 0 },
        span_checksum: None,
        line_digest: sha256_hex(b""),
        block_digest: Some(sha256_hex(normalized.as_bytes())),
        manifest_digest: file.manifest_digest.clone(),
    }
}

fn sidecar_evidence(
    generation: &str,
    path: &str,
    token: &str,
    bytes: &[u8],
    offset: usize,
) -> EvidenceRef {
    // The sidecar is not indexed. Keep a bounded context window so reopen can
    // re-run the same context-sensitive token extractor, never a guessed line.
    let text = String::from_utf8_lossy(bytes);
    let mut start = offset.saturating_sub(160);
    while start > 0 && !text.is_char_boundary(start.min(text.len())) {
        start -= 1;
    }
    let mut end = (offset + token.len() + 160).min(text.len());
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }
    let window = &bytes[start..end];
    EvidenceRef {
        provider: "omp".to_string(),
        generation_id: generation.to_string(),
        coordinate: "raw_file_bytes".to_string(),
        file_id: None,
        path: path.to_string(),
        event: None,
        selector: selector_sidecar_token(token),
        selector_version: SELECTOR_VERSION.to_string(),
        span: SpanJson { file: 0, off: start as u64, len: (end - start) as u32 },
        span_checksum: None,
        line_digest: sha256_hex(window),
        block_digest: Some(sha256_hex(token.as_bytes())),
        manifest_digest: Some(sha256_hex(bytes)),
    }
}

fn assignment_token_selector(source: AssignSource, token: &str) -> String {
    match source {
        AssignSource::Content(index) => selector_arguments_token(index, token),
        AssignSource::DataArgs => selector_data_args_token(token),
    }
}

fn retarget(ev: &EvidenceRef, selector: String, block: &[u8]) -> EvidenceRef {
    let mut out = ev.clone();
    out.selector = selector;
    out.block_digest = Some(sha256_hex(block));
    out
}

fn values_in_json_strings(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => out.push(s.clone()),
            Value::Array(items) => {
                for item in items {
                    walk(item, out);
                }
            }
            Value::Object(map) => {
                for item in map.values() {
                    walk(item, out);
                }
            }
            _ => {}
        }
    }
    walk(value, &mut out);
    out
}

fn token_clues_from_value(
    value: &Value,
    source_field: &str,
    source_kind: &str,
    file: &ViewFile,
    generation: &str,
    event: &ViewEvent,
    line: &[u8],
    selector: &str,
    block: &[u8],
) -> Vec<TokenClue> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for text in values_in_json_strings(value) {
        for hit in extract_task_tokens(&text) {
            if !seen.insert(hit.token.clone()) {
                continue;
            }
            out.push(TokenClue {
                token: hit.token,
                source_field: source_field.to_string(),
                source_kind: source_kind.to_string(),
                strong: false,
                evidence: make_evidence(file, generation, event, selector, line, block),
            });
        }
    }
    out
}

fn load_headers(view: &dyn StockView) -> (Vec<LoadedFile>, Vec<ScanError>) {
    let mut errors = Vec::new();
    let mut loaded = Vec::with_capacity(view.files().len());
    for file in view.files() {
        let header_event = match view.session_header(file.id) {
            Ok(v) => v,
            Err(e) => {
                errors.push(ScanError {
                    code: "layer_unavailable".to_string(),
                    path: Some(file.path.clone()),
                    detail: e.to_string(),
                });
                None
            }
        };
        let header_line = header_event
            .as_ref()
            .and_then(|ev| view.span_bytes(ev.span).ok())
            .unwrap_or_default();
        let header = header_event
            .as_ref()
            .and_then(|_| serde_json::from_slice::<Value>(&header_line).ok())
            .map(|v| parse_header(&v));
        loaded.push(LoadedFile {
            file: file.clone(),
            header,
            header_event,
            header_line,
            parsed_parent: None,
            is_child: false,
        });
    }
    (loaded, errors)
}

fn resolve_parent_indexes(loaded: &mut [LoadedFile]) {
    let mut by_path: HashMap<String, FileId> = HashMap::new();
    let mut by_session: HashMap<String, Vec<FileId>> = HashMap::new();
    for f in loaded.iter() {
        by_path.insert(path_key(&f.file.path), f.file.id);
        if let Some(id) = f.header.as_ref().and_then(|h| h.session_id.clone()) {
            by_session.entry(id).or_default().push(f.file.id);
        }
    }
    for f in loaded.iter_mut() {
        let Some(parent_ref) = f.header.as_ref().and_then(|h| h.parent_session.clone()) else {
            continue;
        };
        let target = by_path
            .get(&path_key(&parent_ref))
            .copied()
            .or_else(|| {
                by_session
                    .get(&parent_ref)
                    .filter(|ids| ids.len() == 1)
                    .map(|ids| ids[0])
            });
        f.parsed_parent = target;
        f.is_child = target.is_some();
    }
}

fn child_stem(path: &str) -> String {
    let name = normalize_path(path)
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_string();
    name.strip_suffix(".jsonl.zst")
        .or_else(|| name.strip_suffix(".jsonl"))
        .unwrap_or(&name)
        .to_string()
}

fn parent_sidecar_dir(parent: &ViewFile) -> String {
    let p = normalize_path(&parent.path);
    let stem = p
        .strip_suffix(".jsonl.zst")
        .or_else(|| p.strip_suffix(".jsonl"))
        .unwrap_or(&p);
    format!("{stem}/")
}

fn child_file_by_name<'a>(loaded: &'a [LoadedFile], parent: &ViewFile, name: &str) -> Option<&'a LoadedFile> {
    let dir = parent_sidecar_dir(parent);
    loaded.iter().find(|f| {
        let p = normalize_path(&f.file.path);
        p.starts_with(&dir)
            && child_stem(&p) == name
            && (p.ends_with(".jsonl") || p.ends_with(".jsonl.zst"))
    })
}

fn header_parent_matches(child: &LoadedFile, parent: &ViewFile, parent_session: Option<&str>) -> Option<bool> {
    let Some(parent_ref) = child.header.as_ref().and_then(|h| h.parent_session.clone()) else {
        return None;
    };
    if path_eq(&parent_ref, &parent.path) {
        return Some(true);
    }
    if let Some(sid) = parent_session {
        if parent_ref == sid {
            return Some(true);
        }
    }
    Some(false)
}

fn parse_parent_file(
    view: &dyn StockView,
    file: &ViewFile,
    generation: &str,
) -> Result<ParsedParent> {
    let bytes = view.bytes(file.id)?;
    let events = view.events(file.id)?;
    let mut calls: Vec<CallRecord> = Vec::new();
    let mut call_pos: HashMap<String, usize> = HashMap::new();
    let mut reminder_cwds = Vec::new();
    let mut prose_tokens = Vec::new();
    let mut path_clues = Vec::new();

    for ev in events {
        let line = slice_span(&bytes, ev.span)?;
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let ty = value.get("type").and_then(Value::as_str).unwrap_or("");
        let custom = value.get("customType").and_then(Value::as_str).unwrap_or("");

        if ty == "custom" && custom == "date-cwd-reminder" {
            if let Some(reminder) = value
                .get("data")
                .and_then(|d| d.get("reminder"))
                .and_then(Value::as_str)
            {
                if let Some(cwd) = reminder_cwd(reminder) {
                    let selector = selector_reminder_cwd(&cwd);
                    let evidence = make_evidence(
                        file,
                        generation,
                        &ev,
                        &selector,
                        line,
                        cwd.as_bytes(),
                    );
                    reminder_cwds.push((cwd, evidence));
                }
            }
        }

        if ty == "message" {
            let role = value
                .get("message")
                .and_then(|m| m.get("role"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if role == "assistant" {
                if let Some(blocks) = value
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_array)
                {
                    for (i, block) in blocks.iter().enumerate() {
                        if block.get("type").and_then(Value::as_str) != Some("toolCall") {
                            continue;
                        }
                        if block.get("name").and_then(Value::as_str) != Some("task") {
                            continue;
                        }
                        let Some(call_id) = block.get("id").and_then(Value::as_str) else {
                            continue;
                        };
                        let Some(args) = block.get("arguments") else {
                            continue;
                        };
                        let block_bytes = serde_json::to_vec(args).unwrap_or_default();
                        let mut assignments = Vec::new();
                        if let Some(tasks) = args.get("tasks").and_then(Value::as_array) {
                            for (ti, task) in tasks.iter().enumerate() {
                                let name = task
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .map(str::to_string);
                                let agent = task
                                    .get("agent")
                                    .and_then(Value::as_str)
                                    .map(str::to_string);
                                let prompt = task
                                    .get("task")
                                    .and_then(Value::as_str)
                                    .map(str::to_string);
                                let solution_space = task
                                    .get("solutionSpace")
                                    .and_then(Value::as_str)
                                    .map(str::to_string);
                                let name_evidence = name.as_ref().map(|_| {
                                    make_evidence(
                                        file,
                                        generation,
                                        &ev,
                                        &selector_content_json(i, "arguments", &format!("/tasks/{ti}/name")),
                                        line,
                                        name.as_ref().unwrap().as_bytes(),
                                    )
                                });
                                let prompt_evidence = prompt.as_ref().map(|_| {
                                    make_evidence(
                                        file,
                                        generation,
                                        &ev,
                                        &selector_content_json(i, "arguments", &format!("/tasks/{ti}/task")),
                                        line,
                                        prompt.as_ref().unwrap().as_bytes(),
                                    )
                                });
                                let solution_evidence = solution_space.as_ref().map(|_| {
                                    make_evidence(
                                        file,
                                        generation,
                                        &ev,
                                        &selector_content_json(i, "arguments", &format!("/tasks/{ti}/solutionSpace")),
                                        line,
                                        solution_space.as_ref().unwrap().as_bytes(),
                                    )
                                });
                                assignments.push(Assignment {
                                    index: ti,
                                    source: AssignSource::Content(i),
                                    name,
                                    agent,
                                    name_evidence,
                                    prompt_evidence,
                                    prompt,
                                    solution_evidence,
                                    solution_space,
                                });
                            }
                        }
                        // Top-level context and intent are mentions, not
                        // assignment arguments.
                        let mut context_tokens = Vec::new();
                        if let Some(context) = args.get("context") {
                            let selector_ctx = selector_content(i, "arguments");
                            context_tokens.extend(token_clues_from_value(
                                context,
                                "task_argument.context",
                                "mention",
                                file,
                                generation,
                                &ev,
                                line,
                                &selector_ctx,
                                &block_bytes,
                            ));
                        }
                        for text in values_in_json_strings(args) {
                            for hit in extract_path_mentions(&text) {
                                path_clues.push(hit.token);
                            }
                        }
                        let record = CallRecord {
                            call_id: call_id.to_string(),
                            args_event: Some(ev.clone()),
                            start_event: None,
                            assignments,
                            intent: None,
                            intent_evidence: None,
                            start_evidence: Vec::new(),
                            result_evidence: Vec::new(),
                            spawns: Vec::new(),
                            result_texts: Vec::new(),
                            context_tokens,
                        };
                        match call_pos.get(call_id).copied() {
                            Some(pos) => {
                                if calls[pos].args_event.is_none() {
                                    calls[pos].args_event = record.args_event;
                                    calls[pos].assignments = record.assignments;
                                }
                            }
                            None => {
                                call_pos.insert(call_id.to_string(), calls.len());
                                calls.push(record);
                            }
                        }
                    }
                }
            } else if role == "toolResult" {
                let Some(call_id) = value
                    .get("message")
                    .and_then(|m| m.get("toolCallId"))
                    .and_then(Value::as_str)
                else {
                    continue;
                };
                let tool_name = value
                    .get("message")
                    .and_then(|m| m.get("toolName"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let mut texts = Vec::new();
                let mut evidences = Vec::new();
                if let Some(blocks) = value
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_array)
                {
                    for (i, block) in blocks.iter().enumerate() {
                        if let Some(text) = block.get("text").and_then(Value::as_str) {
                            evidences.push(make_evidence(
                                file,
                                generation,
                                &ev,
                                &selector_content(i, "text"),
                                line,
                                text.as_bytes(),
                            ));
                            texts.push(text.to_string());
                        }
                    }
                }
                // Only a task call can create a parent-child link. A `wait`
                // or `hub` result may contain `<task-result>` for an already
                // existing child; that is not the spawning call.
                let existing_task_call = call_pos.get(call_id).copied().map(|pos| {
                    calls[pos].args_event.is_some() || calls[pos].start_event.is_some()
                }).unwrap_or(false);
                if tool_name != "task" && !existing_task_call {
                    continue;
                }
                let spawns = texts
                    .iter()
                    .flat_map(|t| extract_spawns(t))
                    .collect::<Vec<_>>();
                let pos = match call_pos.get(call_id).copied() {
                    Some(pos) => pos,
                    None => {
                        call_pos.insert(call_id.to_string(), calls.len());
                        calls.push(CallRecord {
                            call_id: call_id.to_string(),
                            args_event: None,
                            start_event: None,
                            assignments: Vec::new(),
                            intent: None,
                            intent_evidence: None,
                            start_evidence: Vec::new(),
                            result_evidence: Vec::new(),
                            spawns: Vec::new(),
                            result_texts: Vec::new(),
                            context_tokens: Vec::new(),
                        });
                        calls.len() - 1
                    }
                };
                let evidence_for_spawn = evidences.first().cloned();
                calls[pos].result_evidence.extend(evidences);
                calls[pos].result_texts.extend(texts);
                if let Some(ev0) = evidence_for_spawn {
                    for spawn in spawns {
                        calls[pos].spawns.push((spawn, ev0.clone()));
                    }
                }
            }
        }

        if ty == "custom" && custom == "tool_execution_start" {
            let data = value.get("data").unwrap_or(&Value::Null);
            if data.get("toolName").and_then(Value::as_str) != Some("task") {
                continue;
            }
            let Some(call_id) = data.get("toolCallId").and_then(Value::as_str) else {
                continue;
            };
            let mut evidence = Vec::new();
            if let Some(field) = data.get("toolCallId") {
                evidence.push(make_evidence(
                    file,
                    generation,
                    &ev,
                    &selector_data("toolCallId"),
                    line,
                    field.as_str().unwrap_or("").as_bytes(),
                ));
            }
            let intent = data.get("intent").and_then(Value::as_str).map(str::to_string);
            let intent_evidence = intent.as_ref().map(|intent| {
                make_evidence(
                    file,
                    generation,
                    &ev,
                    &selector_data("intent"),
                    line,
                    intent.as_bytes(),
                )
            });
            let mut assignments = Vec::new();
            if let Some(tasks) = data
                .get("args")
                .and_then(|a| a.get("tasks"))
                .and_then(Value::as_array)
            {
                for (ti, task) in tasks.iter().enumerate() {
                    let name = task.get("name").and_then(Value::as_str).map(str::to_string);
                    let agent = task.get("agent").and_then(Value::as_str).map(str::to_string);
                    let prompt = task.get("task").and_then(Value::as_str).map(str::to_string);
                    let solution_space = task
                        .get("solutionSpace")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let name_evidence = name.as_ref().map(|_| {
                        make_evidence(
                            file,
                            generation,
                            &ev,
                            &selector_data_json(&format!("/tasks/{ti}/name")),
                            line,
                            name.as_ref().unwrap().as_bytes(),
                        )
                    });
                    let prompt_evidence = prompt.as_ref().map(|_| {
                        make_evidence(
                            file,
                            generation,
                            &ev,
                            &selector_data_json(&format!("/tasks/{ti}/task")),
                            line,
                            prompt.as_ref().unwrap().as_bytes(),
                        )
                    });
                    let solution_evidence = solution_space.as_ref().map(|_| {
                        make_evidence(
                            file,
                            generation,
                            &ev,
                            &selector_data_json(&format!("/tasks/{ti}/solutionSpace")),
                            line,
                            solution_space.as_ref().unwrap().as_bytes(),
                        )
                    });
                    assignments.push(Assignment {
                        index: ti,
                        source: AssignSource::DataArgs,
                        name,
                        agent,
                        name_evidence,
                        prompt_evidence,
                        prompt,
                        solution_evidence,
                        solution_space,
                    });
                }
            }
            let pos = match call_pos.get(call_id).copied() {
                Some(pos) => pos,
                None => {
                    call_pos.insert(call_id.to_string(), calls.len());
                    calls.push(CallRecord {
                        call_id: call_id.to_string(),
                        args_event: None,
                        start_event: None,
                        assignments: Vec::new(),
                        intent: None,
                        intent_evidence: None,
                        start_evidence: Vec::new(),
                        result_evidence: Vec::new(),
                        spawns: Vec::new(),
                        result_texts: Vec::new(),
                        context_tokens: Vec::new(),
                    });
                    calls.len() - 1
                }
            };
            calls[pos].start_event = Some(ev.clone());
            calls[pos].start_evidence = evidence;
            if calls[pos].intent.is_none() {
                calls[pos].intent = intent;
                calls[pos].intent_evidence = intent_evidence;
            }
            if calls[pos].assignments.is_empty() {
                calls[pos].assignments = assignments;
            }
        }

        // Assignment and mention prose in user/assistant messages.
        if ty == "message" {
            let role = value
                .get("message")
                .and_then(|m| m.get("role"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if matches!(role, "user" | "assistant") {
                if let Some(blocks) = value
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_array)
                {
                    for (i, block) in blocks.iter().enumerate() {
                        let Some(text) = block.get("text").and_then(Value::as_str) else {
                            continue;
                        };
                        for hit in extract_task_tokens(text) {
                            prose_tokens.push(TokenClue {
                                token: hit.token.clone(),
                                source_field: format!("{role}.text"),
                                source_kind: "mention".to_string(),
                                strong: false,
                                evidence: make_evidence(
                                    file,
                                    generation,
                                    &ev,
                                    &selector_message_token(i, &hit.token),
                                    line,
                                    hit.token.as_bytes(),
                                ),
                            });
                        }
                        for hit in extract_task_id_markers(text) {
                            prose_tokens.push(TokenClue {
                                token: hit.token.clone(),
                                source_field: "task_id_marker".to_string(),
                                source_kind: "mention".to_string(),
                                strong: true,
                                evidence: make_evidence(
                                    file,
                                    generation,
                                    &ev,
                                    &selector_message_token(i, &hit.token),
                                    line,
                                    hit.token.as_bytes(),
                                ),
                            });
                        }
                        for hit in extract_path_mentions(text) {
                            path_clues.push(hit.token);
                        }
                    }
                }
            }
        }
    }

    Ok(ParsedParent { calls, reminder_cwds, prose_tokens, path_clues })
}

fn handoff_path(child: &ViewFile) -> String {
    let p = normalize_path(&child.path);
    let stem = p
        .strip_suffix(".jsonl.zst")
        .or_else(|| p.strip_suffix(".jsonl"))
        .unwrap_or(&p);
    format!("{stem}.md")
}

fn read_handoff_tokens(
    view: &dyn StockView,
    child: &ViewFile,
    generation: &str,
) -> (Vec<TokenClue>, Option<ScanError>) {
    let path = handoff_path(child);
    let bytes = match view.sidecar_bytes(&path) {
        Ok(Some(b)) => b,
        Ok(None) => return (Vec::new(), None),
        Err(e) => {
            return (
                Vec::new(),
                Some(ScanError {
                    code: "source_missing".to_string(),
                    path: Some(path),
                    detail: e.to_string(),
                }),
            )
        }
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return (Vec::new(), None);
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for hit in extract_task_tokens(text) {
        if !seen.insert(hit.token.clone()) {
            continue;
        }
        out.push(TokenClue {
            token: hit.token.clone(),
            source_field: "child_handoff".to_string(),
            source_kind: "mention".to_string(),
            strong: hit.offset < 400,
            evidence: sidecar_evidence(generation, &path, &hit.token, &bytes, hit.offset),
        });
    }
    for hit in extract_task_id_markers(text) {
        if !seen.insert(hit.token.clone()) {
            continue;
        }
        out.push(TokenClue {
            token: hit.token.clone(),
            source_field: "child_handoff.task_id_marker".to_string(),
            source_kind: "mention".to_string(),
            strong: true,
            evidence: sidecar_evidence(generation, &path, &hit.token, &bytes, hit.offset),
        });
    }
    (out, None)
}

fn distinct_state_scopes(claims: &[Claim], layer: &str, state: &str) -> usize {
    let mut set = HashSet::new();
    for c in claims
        .iter()
        .filter(|c| c.layer == layer && c.state == state)
    {
        set.insert((
            c.subject.file_id,
            if layer == LAYER_REPO {
                None
            } else {
                c.subject.call_id.clone()
            },
        ));
    }
    set.len()
}

fn distinct_scopes(claims: &[Claim], layer: &str) -> usize {
    let mut set = HashSet::new();
    for c in claims.iter().filter(|c| c.layer == layer) {
        set.insert((
            c.subject.file_id,
            if layer == LAYER_REPO {
                None
            } else {
                c.subject.call_id.clone()
            },
        ));
    }
    set.len()
}

fn add_counts(summary: &mut LayerSummary, claims: &[Claim]) {
    for c in claims {
        summary.counts.add(&c.state);
    }
}

fn layer_summary(rule: &str, version: &str, predicate: &str) -> LayerSummary {
    LayerSummary {
        rule: rule.to_string(),
        rule_version: version.to_string(),
        predicate: predicate.to_string(),
        counts: StateCounts::default(),
        scopes_with_signal: 0,
        scopes_missing: 0,
        session_states: BTreeMap::new(),
        notes: Vec::new(),
    }
}

pub fn scan(view: &dyn StockView, options: &Options) -> Result<Report> {
    let generation = view.generation().to_string();
    let (mut loaded, mut errors) = load_headers(view);
    resolve_parent_indexes(&mut loaded);

    let mut parents: Vec<usize> = loaded
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.is_child)
        .map(|(i, _)| i)
        .collect();
    parents.sort_by(|a, b| loaded[*a].file.path.cmp(&loaded[*b].file.path));
    if let Some(limit) = options.max_parent_files {
        parents.truncate(limit);
    }

    let mut claims: Vec<Claim> = Vec::new();
    let mut link = layer_summary(LINK_RULE, LINK_VERSION, "delegated_from");
    let mut repo = layer_summary(REPO_RULE, REPO_VERSION, "workspace");
    let mut task = layer_summary(TASK_RULE, TASK_VERSION, "task_claim");
    let mut scan_counts = ScanCounts {
        files: loaded.len(),
        parent_files: parents.len(),
        child_files: loaded.iter().filter(|f| f.is_child).count(),
        sessions: loaded.iter().filter(|f| f.header.is_some()).count(),
        ..ScanCounts::default()
    };

    let mut seen_children: HashSet<FileId> = HashSet::new();

    for idx in &parents {
        let parent = &loaded[*idx];
        let parsed = match parse_parent_file(view, &parent.file, &generation) {
            Ok(p) => p,
            Err(e) => {
                errors.push(ScanError {
                    code: "source_missing".to_string(),
                    path: Some(parent.file.path.clone()),
                    detail: e.to_string(),
                });
                continue;
            }
        };
        scan_counts.events_read += view.events(parent.file.id).map(|e| e.len()).unwrap_or(0);
        scan_counts.task_calls += parsed.calls.len();
        let parent_session = parent
            .header
            .as_ref()
            .and_then(|h| h.session_id.clone());

        // ---------------------------------------------------------- repo layer
        add_repo_claims(
            &mut claims,
            &mut repo,
            &parent.file,
            parent.header.as_ref(),
            parent.header_event.as_ref(),
            &parent.header_line,
            &generation,
            &parsed.reminder_cwds,
            &parsed.path_clues,
        );

        // ------------------------------------------------------ task/link layer
        for call in &parsed.calls {
            let mut link_claims = link_claims_for_call(
                &loaded,
                parent,
                call,
                &generation,
                &mut seen_children,
            );
            let mut task_claims = task_claims_for_call(
                view,
                &loaded,
                parent,
                call,
                &generation,
                options,
                &parsed,
                &mut scan_counts,
                &mut errors,
            );
            claims.append(&mut link_claims);
            claims.append(&mut task_claims);
        }

        // Parent-session-only task mentions (prose that is not tied to a call)
        // remain candidates and never become an assignment.
        for clue in &parsed.prose_tokens {
            let state = STATE_CANDIDATE;
            claims.push(
                Claim::new(
                    LAYER_TASK,
                    state,
                    "task_claim",
                    json!({ "kind": "task_token", "token": clue.token, "source": clue.source_field }),
                    Subject::for_file(&parent.file, parent_session.as_deref()),
                    "derived",
                    TASK_RULE,
                    TASK_VERSION,
                    vec![clue.evidence.clone()],
                )
                .reason("prose mention without a task-call scope; mention-only never supports assignment")
                .context("parent-session prose token"),
            );
        }
    }

    // Repo claims for child files that were not already covered by a parent
    // scan (their workspace is still a useful independent observation).
    for f in &loaded {
        if f.is_child {
            add_repo_claims(
                &mut claims,
                &mut repo,
                &f.file,
                f.header.as_ref(),
                f.header_event.as_ref(),
                &f.header_line,
                &generation,
                &[],
                &[],
            );
        }
    }

    // Remove duplicate claims produced by overlapping scopes. Claim ids are
    // deterministic, so deduping is stable and order-independent.
    let mut seen_ids = HashSet::new();
    claims.retain(|c| seen_ids.insert(c.claim_id.clone()));

    add_counts(&mut link, &claims.iter().filter(|c| c.layer == LAYER_LINK).cloned().collect::<Vec<_>>());
    add_counts(&mut repo, &claims.iter().filter(|c| c.layer == LAYER_REPO).cloned().collect::<Vec<_>>());
    add_counts(&mut task, &claims.iter().filter(|c| c.layer == LAYER_TASK).cloned().collect::<Vec<_>>());
    link.scopes_with_signal = distinct_scopes(&claims, LAYER_LINK);
    repo.scopes_with_signal = distinct_scopes(&claims, LAYER_REPO);
    task.scopes_with_signal = distinct_scopes(&claims, LAYER_TASK);

    finalize_layer_sessions(&mut task, &claims, &loaded);
    link.notes.push("confirmed requires one task call record, one result naming exactly one child, and a child header parentSession matching the parent journal; multi-child spawn summaries stay candidates and are never merged.".to_string());
    repo.notes.push("workspace is confirmed from one unique explicit cwd after case/separator normalization; repo_key is derived and stays candidate.".to_string());
    task.notes.push("child_label confirmed requires the structured parent task name plus the independent child filename; prose task tokens are assignment only inside the task-call prompt and become confirmed only when the child handoff agrees.".to_string());
    link.scopes_missing = distinct_state_scopes(&claims, LAYER_LINK, STATE_MISSING);
    repo.scopes_missing = distinct_state_scopes(&claims, LAYER_REPO, STATE_MISSING);
    task.scopes_missing = distinct_state_scopes(&claims, LAYER_TASK, STATE_MISSING);

    let mut layers = BTreeMap::new();
    layers.insert(LAYER_LINK.to_string(), link);
    layers.insert(LAYER_REPO.to_string(), repo);
    layers.insert(LAYER_TASK.to_string(), task);

    Ok(Report {
        adapter: ADAPTER_NAME.to_string(),
        generation,
        evidence_selector_version: SELECTOR_VERSION.to_string(),
        claim_policy: "adapter claims only; no resolver assignment or motif membership is written".to_string(),
        scan: scan_counts,
        layers,
        claims,
        errors,
        notes: vec![
            "parent_child, repo and task are independent layers; one layer never promotes another.".to_string(),
            "missing means the searched source had no signal; unavailable means the needed layer/generation was absent or unreadable.".to_string(),
            "candidate and conflict claims are kept and are not counted as confirmed.".to_string(),
        ],
    })
}

fn add_repo_claims(
    claims: &mut Vec<Claim>,
    summary: &mut LayerSummary,
    file: &ViewFile,
    header: Option<&Header>,
    header_event: Option<&ViewEvent>,
    header_line: &[u8],
    generation: &str,
    reminders: &[(String, EvidenceRef)],
    path_clues: &[String],
) {
    let session_id = header.and_then(|h| h.session_id.clone());
    let mut observations: Vec<(String, Vec<EvidenceRef>)> = Vec::new();
    if let Some(cwd) = header.and_then(|h| h.cwd.clone()) {
        let ev = match header_event {
            Some(hev) => make_evidence(
                file,
                generation,
                hev,
                &selector_session("cwd"),
                header_line,
                cwd.as_bytes(),
            ),
            None => path_evidence(file, generation, "cwd"),
        };
        observations.push((cwd, vec![ev]));
    }
    for (cwd, ev) in reminders {
        if let Some((_, evidence)) = observations.iter_mut().find(|(v, _)| path_eq(v, cwd)) {
            evidence.push(ev.clone());
        } else {
            observations.push((cwd.clone(), vec![ev.clone()]));
        }
    }

    if observations.is_empty() {
        let claim = Claim::new(
            LAYER_REPO,
            STATE_MISSING,
            "workspace",
            Value::Null,
            Subject::for_file(file, session_id.as_deref()),
            "observed",
            REPO_RULE,
            REPO_VERSION,
            Vec::new(),
        )
        .reason("no explicit cwd in the stock session header or reminders");
        claims.push(claim);
        summary.scopes_with_signal += 1;
        return;
    }

    if observations.len() == 1 {
        let (cwd, evidence) = &observations[0];
        let repo_key = repo_key_from_cwd(cwd);
        let mut under_paths: Vec<String> = Vec::new();
        for clue in path_clues {
            if path_under(clue, cwd) && !path_eq(clue, cwd) {
                let normalized = normalize_path(clue);
                if !under_paths.iter().any(|p| path_eq(p, &normalized)) {
                    under_paths.push(normalized);
                }
            }
        }
        let under = under_paths.len();
        let shown: Vec<String> = under_paths.iter().take(8).cloned().collect();
        claims.push(
            Claim::new(
                LAYER_REPO,
                STATE_CONFIRMED,
                "workspace",
                json!({
                    "cwd": normalize_path(cwd),
                    "repo_key": repo_key,
                    "path_clue_count": under,
                    "path_clues": shown
                }),
                Subject::for_file(file, session_id.as_deref()),
                "observed",
                REPO_RULE,
                REPO_VERSION,
                evidence.clone(),
            )
            .reason("one explicit, unique cwd after case/separator normalization")
            .context(format!("independent path mentions under the workspace: {under}")),
        );
        if let Some(key) = repo_key {
            let mut ev = evidence.clone();
            ev.push(path_evidence(file, generation, "workspace"));
            claims.push(
                Claim::new(
                    LAYER_REPO,
                    STATE_CANDIDATE,
                    "repo_key",
                    json!({ "repo_key": key, "cwd": normalize_path(cwd) }),
                    Subject::for_file(file, session_id.as_deref()),
                    "derived",
                    REPO_RULE,
                    REPO_VERSION,
                    ev,
                )
                .reason("repo key is derived from the cwd basename; a path mention is not an independent repo marker")
                .context("repo_key never confirms the workspace by itself"),
            );
        }
        summary.scopes_with_signal += 1;
    } else {
        let values: Vec<String> = observations.iter().map(|(v, _)| normalize_path(v)).collect();
        for (cwd, evidence) in &observations {
            let idx = observations
                .iter()
                .position(|(v, _)| path_eq(v, cwd))
                .unwrap_or(0);
            claims.push(
                Claim::new(
                    LAYER_REPO,
                    STATE_CONFLICT,
                    "workspace",
                    json!({ "cwd": normalize_path(cwd) }),
                    Subject::for_file(file, session_id.as_deref()),
                    "observed",
                    REPO_RULE,
                    REPO_VERSION,
                    evidence.clone(),
                )
                .reason("more than one distinct cwd observation in this file")
                .refute(format!("other observed cwd values: {values:?}"))
                .context(format!("observation {idx} kept; no ordering or mtime tie-break")),
            );
        }
        summary.scopes_with_signal += 1;
    }
}

fn link_child_context(loaded: &[LoadedFile], parent: &LoadedFile, name: &str) -> Option<usize> {
    let child = child_file_by_name(loaded, &parent.file, name)?;
    loaded.iter().position(|f| f.file.id == child.file.id)
}

fn link_claims_for_call(
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

#[allow(clippy::too_many_arguments)]
fn task_claims_for_call(
    view: &dyn StockView,
    loaded: &[LoadedFile],
    parent: &LoadedFile,
    call: &CallRecord,
    generation: &str,
    options: &Options,
    _parsed: &ParsedParent,
    scan_counts: &mut ScanCounts,
    errors: &mut Vec<ScanError>,
) -> Vec<Claim> {
    let session_id = parent.header.as_ref().and_then(|h| h.session_id.clone());
    let event = call
        .args_event
        .as_ref()
        .map(|e| e.event)
        .or_else(|| call.start_event.as_ref().map(|e| e.event));
    let base_subject = Subject::for_call(
        &parent.file,
        session_id.as_deref(),
        event,
        Some(&call.call_id),
    );
    let mut claims = Vec::new();

    // One task call may dispatch several sibling children. Each child is its
    // own scope; siblings are never merged and never treated as a conflict.
    let spawn_children: Vec<(&SpawnHit, Option<&LoadedFile>)> = call
        .spawns
        .iter()
        .filter(|(s, _)| s.shape != "failed")
        .map(|(s, _)| (s, child_file_by_name(loaded, &parent.file, &s.name)))
        .collect();
    let single_child = if spawn_children.len() == 1 {
        spawn_children[0].1
    } else {
        None
    };

    let mut clues: Vec<TokenClue> = Vec::new();
    for assignment in &call.assignments {
        if let Some(prompt) = &assignment.prompt {
            let block_ev = assignment.prompt_evidence.clone();
            for hit in extract_task_tokens(prompt) {
                if let Some(ev) = &block_ev {
                    clues.push(TokenClue {
                        token: hit.token.clone(),
                        source_field: format!(
                            "task_argument.task[{}/{}]",
                            call.call_id, assignment.index
                        ),
                        source_kind: "assignment".to_string(),
                        strong: hit.offset < 400,
                        evidence: retarget(
                            ev,
                            assignment_token_selector(assignment.source, &hit.token),
                            hit.token.as_bytes(),
                        ),
                    });
                }
            }
            for hit in extract_task_id_markers(prompt) {
                if let Some(ev) = &block_ev {
                    clues.push(TokenClue {
                        token: hit.token.clone(),
                        source_field: "task_argument.task_id_marker".to_string(),
                        source_kind: "assignment".to_string(),
                        strong: true,
                        evidence: retarget(
                            ev,
                            assignment_token_selector(assignment.source, &hit.token),
                            hit.token.as_bytes(),
                        ),
                    });
                }
            }
        }
        if let Some(solution) = &assignment.solution_space {
            let ev = assignment.solution_evidence.clone();
            for hit in extract_task_tokens(solution) {
                if let Some(ev) = &ev {
                    clues.push(TokenClue {
                        token: hit.token.clone(),
                        source_field: "task_argument.solutionSpace".to_string(),
                        source_kind: "mention".to_string(),
                        strong: false,
                        evidence: retarget(
                            ev,
                            assignment_token_selector(assignment.source, &hit.token),
                            hit.token.as_bytes(),
                        ),
                    });
                }
            }
        }
    }
    for clue in &call.context_tokens {
        clues.push(clue.clone());
    }
    if let Some(intent) = &call.intent {
        if let Some(ev) = &call.intent_evidence {
            for hit in extract_task_tokens(intent) {
                clues.push(TokenClue {
                    token: hit.token,
                    source_field: "task_call.intent".to_string(),
                    source_kind: "mention".to_string(),
                    strong: false,
                    evidence: ev.clone(),
                });
            }
        }
    }

    // Read one handoff only when the call has one child. With siblings, a
    // token from one handoff cannot be attributed to the call scope safely.
    if let Some(child) = single_child {
        if options.read_sidecars {
            let (tokens, error) = read_handoff_tokens(view, &child.file, generation);
            if let Some(e) = error {
                errors.push(e);
            }
            if !tokens.is_empty() {
                scan_counts.sidecars_read += 1;
                clues.extend(tokens);
            }
        }
    }

    // ------------------------------------------------ child-label claims
    let mut label_claims = 0usize;
    let mut matched_parent_names: HashSet<String> = HashSet::new();
    for (spawn, child_opt) in &spawn_children {
        let name = &spawn.name;
        let matching: Vec<&Assignment> = call
            .assignments
            .iter()
            .filter(|a| a.name.as_deref() == Some(name.as_str()))
            .collect();
        match child_opt {
            Some(child) => {
                let stem = child_stem(&child.file.path);
                let filename_ev = path_evidence(&child.file, generation, "filename");
                let header_ev = child.header_event.as_ref().map(|hev| {
                    make_evidence(
                        &child.file,
                        generation,
                        hev,
                        &selector_session("id"),
                        &child.header_line,
                        child
                            .header
                            .as_ref()
                            .and_then(|h| h.session_id.clone())
                            .unwrap_or_default()
                            .as_bytes(),
                    )
                });
                if let Some(assignment) = matching.first() {
                    matched_parent_names.insert(name.clone());
                    let mut evidence = Vec::new();
                    if let Some(ev) = &assignment.name_evidence {
                        evidence.push(ev.clone());
                    }
                    evidence.push(filename_ev);
                    if let Some(ev) = header_ev {
                        evidence.push(ev);
                    }
                    claims.push(
                        Claim::new(
                            LAYER_TASK,
                            STATE_CONFIRMED,
                            "task_claim",
                            json!({
                                "kind": "child_label",
                                "label": stem,
                                "agent": assignment.agent,
                                "attempt": trailing_digits(&stem),
                                "project": Value::Null,
                                "task": Value::Null
                            }),
                            base_subject.clone().with_child(&child.file),
                            "mixed",
                            TASK_RULE,
                            TASK_VERSION,
                            evidence,
                        )
                        .reason(
                            "structured parent task name and child filename agree; two independent sources, one an observed field",
                        ),
                    );
                } else {
                    // A single child with a differing parent name is emitted
                    // below as an explicit conflict pair instead of a bare
                    // filename candidate.
                    let will_conflict = spawn_children.len() == 1
                        && call.assignments.iter().any(|a| {
                            a.name.as_deref().map(|n| n != stem).unwrap_or(false)
                        });
                    if !will_conflict {
                        claims.push(
                            Claim::new(
                                LAYER_TASK,
                                STATE_CANDIDATE,
                                "task_claim",
                                json!({
                                    "kind": "child_label",
                                    "label": stem,
                                    "agent": Value::Null,
                                    "attempt": trailing_digits(&stem),
                                    "project": Value::Null,
                                    "task": Value::Null
                                }),
                                base_subject.clone().with_child(&child.file),
                                "derived",
                                TASK_RULE,
                                TASK_VERSION,
                                {
                                    let mut ev = vec![filename_ev];
                                    if let Some(h) = header_ev {
                                        ev.push(h);
                                    }
                                    ev
                                },
                            )
                            .reason("child filename is name-only evidence; no parent assignment agrees"),
                        );
                        label_claims += 1;
                    }
                }
            }
            None => {
                for assignment in matching {
                    matched_parent_names.insert(name.clone());
                    let mut claim = Claim::new(
                        LAYER_TASK,
                        STATE_CANDIDATE,
                        "task_claim",
                        json!({
                            "kind": "child_label",
                            "label": name,
                            "agent": assignment.agent,
                            "attempt": trailing_digits(name),
                            "project": Value::Null,
                            "task": Value::Null
                        }),
                        base_subject.clone(),
                        "observed",
                        TASK_RULE,
                        TASK_VERSION,
                        assignment.name_evidence.iter().cloned().collect(),
                    )
                    .reason("parent task name has no matching child journal");
                    claim = claim.reason("name-only candidate; never confirmed without a child scope");
                    claims.push(claim);
                    label_claims += 1;
                }
            }
        }
    }

    // A single child whose filename disagrees with every structured parent
    // name is a task conflict: keep both values and do not tie-break.
    if let Some(child) = single_child {
        let stem = child_stem(&child.file.path);
        let mut conflict_parent = false;
        for assignment in &call.assignments {
            let Some(name) = &assignment.name else { continue };
            if name == &stem || matched_parent_names.contains(name) {
                continue;
            }
            conflict_parent = true;
            let filename_ev = path_evidence(&child.file, generation, "filename");
            let mut evidence: Vec<EvidenceRef> = assignment
                .name_evidence
                .iter()
                .cloned()
                .collect();
            evidence.push(filename_ev.clone());
            claims.push(
                Claim::new(
                    LAYER_TASK,
                    STATE_CONFLICT,
                    "task_claim",
                    json!({
                        "kind": "child_label",
                        "label": name,
                        "agent": assignment.agent,
                        "attempt": trailing_digits(name),
                        "project": Value::Null,
                        "task": Value::Null
                    }),
                    base_subject.clone().with_child(&child.file),
                    "mixed",
                    TASK_RULE,
                    TASK_VERSION,
                    evidence,
                )
                .reason("structured parent task name differs from the only child filename")
                .refute(format!("child filename value kept separately: {stem}")),
            );
            label_claims += 1;
        }
        if conflict_parent {
            claims.push(
                Claim::new(
                    LAYER_TASK,
                    STATE_CONFLICT,
                    "task_claim",
                    json!({
                        "kind": "child_label",
                        "label": stem,
                        "agent": Value::Null,
                        "attempt": trailing_digits(&stem),
                        "project": Value::Null,
                        "task": Value::Null
                    }),
                    base_subject.clone().with_child(&child.file),
                    "derived",
                    TASK_RULE,
                    TASK_VERSION,
                    vec![path_evidence(&child.file, generation, "filename")],
                )
                .reason("child filename differs from the structured parent task name")
                .refute("no line order, mtime, path length or suffix tie-break is applied"),
            );
            label_claims += 1;
        }
    }

    // Parent names that never had a child or a spawn stay candidates.
    for assignment in &call.assignments {
        let Some(name) = &assignment.name else { continue };
        if matched_parent_names.contains(name) {
            continue;
        }
        if single_child.is_some() {
            // Already handled above as a conflict pair.
            continue;
        }
        claims.push(
            Claim::new(
                LAYER_TASK,
                STATE_CANDIDATE,
                "task_claim",
                json!({
                    "kind": "child_label",
                    "label": name,
                    "agent": assignment.agent,
                    "attempt": trailing_digits(name),
                    "project": Value::Null,
                    "task": Value::Null
                }),
                base_subject.clone(),
                "observed",
                TASK_RULE,
                TASK_VERSION,
                assignment.name_evidence.iter().cloned().collect(),
            )
            .reason("parent task name has no child scope in this call"),
        );
        label_claims += 1;
    }

    // ------------------------------------------------------- token claims
    let mut by_token: BTreeMap<String, Vec<TokenClue>> = BTreeMap::new();
    for clue in &clues {
        if clue.source_field == "task_argument.name" || clue.source_field == "child_filename" {
            continue;
        }
        by_token.entry(clue.token.clone()).or_default().push(clue.clone());
    }
    for (token, sources) in by_token {
        let assignment = sources.iter().any(|s| s.source_kind == "assignment");
        let assignment_strong = sources
            .iter()
            .any(|s| s.source_kind == "assignment" && s.strong);
        let handoff = sources
            .iter()
            .any(|s| s.source_field.starts_with("child_handoff"));
        let handoff_strong = sources
            .iter()
            .any(|s| s.source_field.starts_with("child_handoff") && s.strong);
        let mention = sources.iter().any(|s| s.source_kind == "mention");
        let multi_scope = spawn_children.len() > 1;
        let state = if !multi_scope
            && assignment_strong
            && handoff_strong
            && single_child.is_some()
        {
            STATE_CONFIRMED
        } else if assignment || handoff || mention {
            STATE_CANDIDATE
        } else {
            continue;
        };
        let evidence: Vec<EvidenceRef> = sources.iter().map(|s| s.evidence.clone()).collect();
        let subject = match single_child {
            Some(child) => base_subject.clone().with_child(&child.file),
            None => base_subject.clone(),
        };
        let mut claim = Claim::new(
            LAYER_TASK,
            state,
            "task_claim",
            json!({
                "kind": "task_token",
                "token": token,
                "token_kind": crate::extract::classify_token(&token),
                "assignment": assignment,
                "assignment_strong": assignment_strong,
                "handoff": handoff,
                "handoff_strong": handoff_strong,
                "mention": mention,
                "multi_child_scope": multi_scope
            }),
            subject,
            if assignment && handoff { "mixed" } else { "derived" },
            TASK_RULE,
            TASK_VERSION,
            evidence,
        );
        if state == STATE_CONFIRMED {
            claim = claim.reason(
                "parent task-call prompt and independent child handoff contain the same token",
            );
        } else if multi_scope {
            claim = claim.reason(
                "one task call names multiple children; token scope is ambiguous and stays candidate",
            );
        } else if assignment_strong {
            claim = claim.reason(
                "assignment-heading token without a corroborating child-handoff token",
            );
        } else if assignment {
            claim = claim.reason("assignment-context token without independent corroboration");
        } else if handoff {
            claim = claim.reason("child handoff token without a parent assignment token");
        } else {
            claim = claim.reason("mention-only token; never promoted to confirmed");
        }
        claims.push(claim);
    }

    if label_claims == 0 && claims.is_empty() {
        claims.push(
            Claim::new(
                LAYER_TASK,
                STATE_MISSING,
                "task_claim",
                Value::Null,
                base_subject,
                "observed",
                TASK_RULE,
                TASK_VERSION,
                Vec::new(),
            )
            .reason("no task label or assignment token in this call scope"),
        );
    }

    claims
}

fn trailing_digits(value: &str) -> Option<String> {
    let start = value
        .as_bytes()
        .iter()
        .rposition(|b| !b.is_ascii_digit())
        .map(|i| i + 1)
        .unwrap_or(0);
    if start == value.len() {
        None
    } else {
        Some(value[start..].to_string())
    }
}

fn finalize_layer_sessions(summary: &mut LayerSummary, claims: &[Claim], loaded: &[LoadedFile]) {
    let mut by_session: BTreeMap<Option<String>, Vec<&Claim>> = BTreeMap::new();
    for c in claims.iter().filter(|c| c.layer == LAYER_TASK) {
        by_session.entry(c.subject.session_id.clone()).or_default().push(c);
    }
    for (session, list) in by_session {
        if session.is_none() {
            continue;
        }
        let conflict = list.iter().any(|c| c.state == STATE_CONFLICT);
        let confirmed_labels: BTreeSet<(String, Option<String>)> = list
            .iter()
            .filter(|c| {
                c.state == STATE_CONFIRMED
                    && c.value.get("kind").and_then(Value::as_str) == Some("child_label")
            })
            .filter_map(|c| {
                let label = c.value.get("label").and_then(Value::as_str)?;
                Some((label.to_string(), c.subject.call_id.clone()))
            })
            .collect();
        let distinct_calls: BTreeSet<String> = confirmed_labels
            .iter()
            .filter_map(|(_, call)| call.clone())
            .collect();
        let state = if conflict {
            STATE_CONFLICT
        } else if distinct_calls.len() > 1 {
            STATE_MULTI_TASK
        } else if !confirmed_labels.is_empty() {
            STATE_CONFIRMED
        } else if list.iter().any(|c| c.state == STATE_CANDIDATE) {
            STATE_CANDIDATE
        } else {
            STATE_MISSING
        };
        *summary.session_states.entry(state.to_string()).or_default() += 1;
    }
    // Report how many selected files actually carried a parsed session header.
    summary.notes.push(format!(
        "session headers parsed: {}",
        loaded.iter().filter(|f| f.header.is_some()).count()
    ));
}
