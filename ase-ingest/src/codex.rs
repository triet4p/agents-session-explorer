use ase_core::{
    err, DecodeStats, DecodedEvent, Decoder, EventKind, FileDecode, Result, Role, SessionMeta,
    SourceFile, SpanRef, NONE,
};
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::source::base_name;
use crate::projection::project_codex_payload;
use crate::util::{assign_sessions, fnv64, link_events, parse_ts_ms};

#[derive(Deserialize)]
struct CodexLine<'a> {
    timestamp: Option<&'a str>,
    #[serde(rename = "ordinal")]
    ordinal: Option<u64>,
    #[serde(rename = "type")]
    ty: &'a str,
    payload: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct Payload<'a> {
    #[serde(rename = "type")]
    ty: Option<&'a str>,
    role: Option<&'a str>,
    id: Option<&'a str>,
    #[serde(rename = "call_id")]
    call_id: Option<&'a str>,
}

pub struct CodexDecoder;

impl Decoder for CodexDecoder {
    fn decode(&self, file: &SourceFile, bytes: &[u8]) -> Result<FileDecode> {
        decode_codex(file, bytes)
    }
}

pub fn decode_codex(file: &SourceFile, bytes: &[u8]) -> Result<FileDecode> {
    let mut events: Vec<DecodedEvent> = Vec::new();
    let mut projections = Vec::new();
    let mut stats = DecodeStats::default();
    let mut explicit_session: Option<String> = None;
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

        match serde_json::from_slice::<CodexLine>(line) {
            Ok(v) => {
                ev.source_type = v.ty.to_string();
                ev.tag = v.ty.to_string();
                ev.ts_ms = v.timestamp.map(parse_ts_ms).unwrap_or(0);
                if let Some(ord) = v.ordinal {
                    ev.source_id = Some(format!("ordinal:{ord}"));
                }
                let payload_raw = v.payload;
                let payload = payload_raw.and_then(|p| serde_json::from_str::<Payload>(p.get()).ok());
                match v.ty {
                    "session_meta" => {
                        ev.kind = EventKind::SessionMeta;
                        ev.role = Role::System;
                        if let Some(p) = &payload {
                            if let Some(id) = p.id {
                                explicit_session = Some(id.to_string());
                            }
                        }
                    }
                    "event_msg" => match payload.as_ref().and_then(|p| p.ty) {
                        Some("user_message") => {
                            ev.kind = EventKind::UserInput;
                            ev.role = Role::User;
                            ev.tag = "user_message".to_string();
                        }
                        Some("agent_message") => {
                            ev.kind = EventKind::AssistantText;
                            ev.role = Role::Assistant;
                            ev.tag = "agent_message".to_string();
                        }
                        Some(other) => {
                            stats.unknown_type += 1;
                            ev.tag = other.to_string();
                        }
                        None => stats.unknown_type += 1,
                    },
                    "response_item" => match payload.as_ref().and_then(|p| p.ty) {
                        Some("message") => match payload.as_ref().and_then(|p| p.role) {
                            Some("user") => {
                                ev.kind = EventKind::UserInput;
                                ev.role = Role::User;
                            }
                            Some("assistant") => {
                                ev.kind = EventKind::AssistantText;
                                ev.role = Role::Assistant;
                            }
                            Some(other) => {
                                ev.kind = EventKind::Unknown;
                                ev.role = Role::System;
                                ev.tag = format!("message:{other}");
                            }
                            None => stats.unknown_type += 1,
                        },
                        Some("function_call")
                        | Some("custom_tool_call")
                        | Some("web_search_call") => {
                            ev.kind = EventKind::ToolCall;
                            ev.role = Role::Assistant;
                            ev.call_id = payload.as_ref().and_then(|p| p.call_id).map(str::to_string);
                        }
                        Some("function_call_output") | Some("custom_tool_call_output") => {
                            ev.kind = EventKind::ToolResult;
                            ev.call_id = payload.as_ref().and_then(|p| p.call_id).map(str::to_string);
                        }
                        Some("reasoning") => {
                            ev.kind = EventKind::Reasoning;
                            ev.role = Role::Assistant;
                        }
                        Some(other) => {
                            stats.unknown_type += 1;
                            ev.tag = other.to_string();
                        }
                        None => stats.unknown_type += 1,
                    },
                    "token_count" | "token_usage_record" => {
                        ev.kind = EventKind::Usage;
                        ev.role = Role::System;
                    }
                    other => {
                        stats.unknown_type += 1;
                        ev.tag = other.to_string();
                    }
                }
                if let Some(p) = &payload {
                    if ev.tag == v.ty {
                        if let Some(pt) = p.ty {
                            ev.tag = pt.to_string();
                        }
                    }
                }
                if let Some(raw) = payload_raw {
                    projections.extend(project_codex_payload(
                        ev.ordinal, ev.span, v.ty, raw, ev.role,
                    ));
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

    let path_derived = explicit_session.is_none();
    let sessions = vec![SessionMeta {
        id: explicit_session.unwrap_or_else(|| base_name(&file.path)),
        file: file.id,
        parent_session: None,
        path_derived,
        local_index: 0,
    }];

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
