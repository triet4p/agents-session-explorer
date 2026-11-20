use ase_core::{
    err, DecodeStats, DecodedEvent, Decoder, EventKind, FileDecode, Result, Role, SessionMeta,
    SourceFile, SpanRef, NONE,
};
use memchr::memmem;
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::source::base_name;
use crate::projection::project_omp_content;
use crate::util::{assign_sessions, fnv64, link_events, parse_ts_ms};

#[derive(Deserialize)]
struct OmpLine<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    #[serde(rename = "customType")]
    custom_type: Option<&'a str>,
    id: Option<&'a str>,
    #[serde(rename = "parentId")]
    parent_id: Option<&'a str>,
    #[serde(rename = "parentSession")]
    parent_session: Option<&'a str>,
    timestamp: Option<&'a str>,
    message: Option<OmpMessage<'a>>,
}

#[derive(Deserialize)]
struct OmpMessage<'a> {
    role: Option<&'a str>,
    content: Option<&'a RawValue>,
    #[serde(rename = "toolCallId")]
    tool_call_id: Option<&'a str>,
}
#[derive(Deserialize)]
struct Block<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    id: Option<&'a str>,
}

pub struct OmpDecoder;

impl Decoder for OmpDecoder {
    fn decode(&self, file: &SourceFile, bytes: &[u8]) -> Result<FileDecode> {
        decode_omp(file, bytes)
    }
}

pub fn decode_omp(file: &SourceFile, bytes: &[u8]) -> Result<FileDecode> {
    let mut events: Vec<DecodedEvent> = Vec::new();
    let mut projections = Vec::new();
    let mut stats = DecodeStats::default();
    let mut explicit_session: Option<(String, Option<String>)> = None;
    let mut off = 0usize;

    for seg in bytes.split(|b| *b == b'\n') {
        let raw_len = seg.len();
        let mut len = raw_len;
        if len > 0 && seg[len - 1] == b'\r' {
            len -= 1;
        }
        let line = &seg[..len];
        let span_off = off as u64;
        off += raw_len + 1;
        if line.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }

        let hash = fnv64(line);
        let mut ev = DecodedEvent {
            ordinal: events.len() as u32,
            span: SpanRef { file: file.id, off: span_off, len: len as u32 },
            kind: EventKind::Unknown,
            role: Role::Unknown,
            ts_ms: 0,
            source_type: String::new(),
            tag: String::new(),
            source_id: None,
            parent_id: None,
            parent_idx: NONE,
            call_id: None,
            link_idx: NONE,
            ext_id: None,
            session_local: NONE,
            hash,
            valid_json: true,
        };

        match serde_json::from_slice::<OmpLine>(line) {
            Ok(v) => {
                ev.source_type = v.ty.to_string();
                ev.source_id = v.id.map(str::to_string);
                ev.parent_id = v.parent_id.map(str::to_string);
                ev.ts_ms = v.timestamp.map(parse_ts_ms).unwrap_or(0);
                match v.ty {
                    "session" => {
                        ev.kind = EventKind::SessionMeta;
                        ev.role = Role::System;
                        ev.tag = "session".to_string();
                        if let Some(id) = ev.source_id.clone() {
                            explicit_session = Some((id, v.parent_session.map(str::to_string)));
                        }
                    }
                    "title" => {
                        ev.kind = EventKind::Title;
                        ev.role = Role::System;
                        ev.tag = "title".to_string();
                    }
                    "model_change" => {
                        ev.kind = EventKind::ModelChange;
                        ev.role = Role::System;
                        ev.tag = "model_change".to_string();
                    }
                    "custom" => {
                        ev.kind = EventKind::Notice;
                        ev.role = Role::System;
                        ev.tag = v.custom_type.unwrap_or("custom").to_string();
                    }
                    "message" => match v.message {
                        Some(m) => {
                            let content = m.content;
                            match m.role.unwrap_or("") {
                            "user" => {
                                ev.kind = EventKind::UserInput;
                                ev.role = Role::User;
                                ev.tag = "user".to_string();
                            }
                            "assistant" => {
                                ev.kind = EventKind::AssistantText;
                                ev.role = Role::Assistant;
                                ev.tag = "assistant".to_string();
                                    if let Some(content) = content {
                                    let raw = content.get().as_bytes();
                                    if memmem::find(raw, b"toolCall").is_some() {
                                        if let Ok(blocks) =
                                            serde_json::from_str::<Vec<Block>>(content.get())
                                        {
                                            let mut call_ids = blocks.iter().filter_map(|b| {
                                                if b.ty == "toolCall" { b.id.map(str::to_string) } else { None }
                                            });
                                            if let Some(first) = call_ids.next() {
                                                ev.call_id = Some(first);
                                                if call_ids.next().is_some() {
                                                    stats.multi_call_entries += 1;
                                                }
                                            }
                                        }
                                    }
                                    if memmem::find(raw, b"thinking").is_some() {
                                        ev.tag = "assistant_thinking".to_string();
                                    }
                                }
                            }
                            "toolResult" => {
                                ev.kind = EventKind::ToolResult;
                                ev.role = Role::Tool;
                                ev.tag = "toolResult".to_string();
                                ev.call_id = m.tool_call_id.map(str::to_string);
                            }
                            other => {
                                stats.unknown_type += 1;
                                ev.tag = other.to_string();
                            }
                            }
                            if let Some(raw) = content {
                                projections.extend(project_omp_content(
                                    ev.ordinal, ev.span, raw, ev.role,
                                ));
                            }
                        }
                        None => {
                            stats.unknown_type += 1;
                            ev.tag = "message_no_payload".to_string();
                        }
                    },
                    other => {
                        stats.unknown_type += 1;
                        ev.kind = EventKind::Unknown;
                        ev.tag = other.to_string();
                    }
                }
            }
            Err(_) => {
                let json_ok = serde_json::from_slice::<&RawValue>(line).is_ok();
                if json_ok { stats.unknown_type += 1; } else { stats.malformed += 1; }
                ev.source_type = "<unparsed>".to_string();
                ev.tag = "<unparsed>".to_string();
                ev.valid_json = json_ok;
            }
        }
        stats.events += 1;
        events.push(ev);
    }

    let sessions = match explicit_session {
        Some((id, parent)) => vec![SessionMeta {
            id,
            file: file.id,
            parent_session: parent,
            path_derived: false,
            local_index: 0,
        }],
        None => {
            let mut parent = None;
            if let Some(p) = std::path::Path::new(&file.path).parent() {
                if let Some(name) = p.file_name().and_then(|x| x.to_str()) {
                    // `<project>/<timestamp>_<uuid>/<Agent>.jsonl` sidecars.
                    if name.contains('_') && name.len() > 30 {
                        parent = Some(name.to_string());
                    }
                }
            }
            vec![SessionMeta {
                id: base_name(&file.path),
                file: file.id,
                parent_session: parent,
                path_derived: true,
                local_index: 0,
            }]
        }
    };

    assign_sessions(&mut events, &sessions);
    link_events(&mut events, &mut stats);
    for p in &mut projections {
        if let Some(e) = events.get(p.local_event as usize) {
            p.session_local = e.session_local;
            p.parent_idx = e.parent_idx;
        }
    }
    stats.sessions = sessions.len();
    if events.is_empty() {
        return Err(err("empty source file"));
    }
    Ok(FileDecode { events, sessions, stats, projections })
}
