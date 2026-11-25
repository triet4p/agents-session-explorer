//! Journal loading and parsing for OMP stock adapter scans.
//!
//! This module owns reads of the pinned `StockView`: session headers, raw
//! events, child handoff sidecars and the derived per-file navigation data.
//! It never writes claims or resolver state.

use ase_core::{FileId, Result, SpanRef, err};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use crate::extract::{
    SpawnHit, extract_path_mentions, extract_spawns, extract_task_id_markers,
    extract_task_tokens,
};
use crate::model::*;
use crate::provenance::{
    make_evidence, sidecar_evidence, token_clues_from_value, values_in_json_strings,
};

pub(crate) struct LoadedFile {
    pub(crate) file: ViewFile,
    pub(crate) header: Option<Header>,
    pub(crate) header_event: Option<ViewEvent>,
    pub(crate) header_line: Vec<u8>,
    pub(crate) parsed_parent: Option<FileId>,
    pub(crate) is_child: bool,
}

#[derive(Clone)]
pub(crate) struct TokenClue {
    pub(crate) token: String,
    pub(crate) source_field: String,
    pub(crate) source_kind: String,
    /// A token near an assignment heading/marker is strong enough to
    /// corroborate a task identity. Later prose tokens stay mentions.
    pub(crate) strong: bool,
    pub(crate) evidence: EvidenceRef,
}

#[derive(Clone, Copy)]
pub(crate) enum AssignSource {
    Content(usize),
    DataArgs,
}

#[derive(Clone)]
pub(crate) struct Assignment {
    pub(crate) index: usize,
    pub(crate) source: AssignSource,
    pub(crate) name: Option<String>,
    pub(crate) agent: Option<String>,
    pub(crate) name_evidence: Option<EvidenceRef>,
    pub(crate) prompt_evidence: Option<EvidenceRef>,
    pub(crate) prompt: Option<String>,
    pub(crate) solution_evidence: Option<EvidenceRef>,
    pub(crate) solution_space: Option<String>,
}

pub(crate) struct CallRecord {
    pub(crate) call_id: String,
    pub(crate) args_event: Option<ViewEvent>,
    pub(crate) start_event: Option<ViewEvent>,
    pub(crate) assignments: Vec<Assignment>,
    pub(crate) intent: Option<String>,
    pub(crate) intent_evidence: Option<EvidenceRef>,
    pub(crate) start_evidence: Vec<EvidenceRef>,
    pub(crate) result_evidence: Vec<EvidenceRef>,
    pub(crate) spawns: Vec<(SpawnHit, EvidenceRef)>,
    pub(crate) result_texts: Vec<String>,
    pub(crate) context_tokens: Vec<TokenClue>,
}

pub(crate) struct ParsedParent {
    pub(crate) calls: Vec<CallRecord>,
    pub(crate) reminder_cwds: Vec<(String, EvidenceRef)>,
    pub(crate) prose_tokens: Vec<TokenClue>,
    pub(crate) path_clues: Vec<String>,
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

pub(crate) fn load_headers(view: &dyn StockView) -> (Vec<LoadedFile>, Vec<ScanError>) {
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

pub(crate) fn resolve_parent_indexes(loaded: &mut [LoadedFile]) {
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

pub(crate) fn child_stem(path: &str) -> String {
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

pub(crate) fn child_file_by_name<'a>(loaded: &'a [LoadedFile], parent: &ViewFile, name: &str) -> Option<&'a LoadedFile> {
    let dir = parent_sidecar_dir(parent);
    loaded.iter().find(|f| {
        let p = normalize_path(&f.file.path);
        p.starts_with(&dir)
            && child_stem(&p) == name
            && (p.ends_with(".jsonl") || p.ends_with(".jsonl.zst"))
    })
}

pub(crate) fn header_parent_matches(child: &LoadedFile, parent: &ViewFile, parent_session: Option<&str>) -> Option<bool> {
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

pub(crate) fn parse_parent_file(
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

pub(crate) fn read_handoff_tokens(
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
