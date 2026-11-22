//! Versioned per-block projections emitted next to an address event.
//!
//! A source line keeps one address but may project to zero or more blocks.
//! Only allowlisted user/assistant prose and tool-call signatures are emitted.
//! Thinking, tool output, system and unknown blocks are never projected.
//! This module knows nothing about SQLite, ranking, models or rendering.

use ase_core::{
    err, BlockProjection, ProjectionKind, Provider, Role, SpanRef, PROJECTION_VERSION, NONE,
};
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub use ase_core::{
    ALLOWLIST_VERSION, CALLSIG_VERSION, SHINGLE_VERSION, TOKENIZER_VERSION,
};

fn new_projection(
    local_event: u32,
    span: SpanRef,
    selector: String,
    kind: ProjectionKind,
    role: Role,
) -> BlockProjection {
    BlockProjection {
        local_event,
        span,
        selector,
        kind,
        role,
        prose: String::new(),
        call_sig: String::new(),
        block_hash: [0; 8],
        session_local: NONE,
        parent_idx: NONE,
    }
}

#[derive(Deserialize)]
struct OmpBlock<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    text: Option<String>,
    name: Option<String>,
    args: Option<Value>,
    arguments: Option<Value>,
}

#[derive(Deserialize)]
struct CodexPayload<'a> {
    #[serde(rename = "type")]
    ty: Option<&'a str>,
    message: Option<&'a RawValue>,
    content: Option<&'a RawValue>,
    name: Option<String>,
    arguments: Option<&'a RawValue>,
    parameters: Option<&'a RawValue>,
}


/// Short digest used only to re-derive a projected value from raw bytes at
/// evidence-reopen time. It is not an authenticity signature.
pub fn digest8(value: &str) -> [u8; 8] {
    let d = Sha256::digest(value.as_bytes());
    let mut out = [0u8; 8];
    out.copy_from_slice(&d[..8]);
    out
}

fn prose_allowed(role: Role) -> bool {
    matches!(role, Role::User | Role::Assistant)
}

fn normalized_tool(name: Option<&str>) -> String {
    let raw = name.unwrap_or("unknown").trim().to_lowercase();
    let kept: String = raw
        .chars()
        .take(64)
        .map(|c| if c.is_alphanumeric() || matches!(c, '_' | '-' | '.') { c } else { '_' })
        .collect();
    if kept.is_empty() { "unknown".to_string() } else { kept }
}

fn path_like(s: &str) -> bool {
    if s.is_empty() || s.len() > 512 {
        return false;
    }
    let compact = !s.chars().any(char::is_whitespace);
    compact
        && (s.contains('/') || s.contains('\\') || (s.contains('.') && s.chars().any(char::is_alphanumeric)))
}

fn value_class(v: &Value) -> char {
    match v {
        Value::Null => 'Z',
        Value::Bool(_) => 'B',
        Value::Number(_) => 'N',
        Value::String(s) => {
            if path_like(s) { 'P' } else { 'S' }
        }
        Value::Array(_) => 'A',
        Value::Object(_) => 'O',
    }
}

/// `callsig-v1`: lowercase tool name plus sorted argument keys and coarse
/// value classes. No argument value, path or text is retained.
pub fn normalize_call_sig(name: Option<&str>, args: Option<&Value>) -> String {
    let tool = normalized_tool(name);
    let mut shape = String::new();
    if let Some(Value::Object(map)) = args {
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        for k in keys {
            let kk: String = k
                .chars()
                .take(48)
                .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
                .collect();
            shape.push_str(&format!("{}={},", kk, value_class(&map[k])));
        }
        shape.pop();
    } else if let Some(v) = args {
        shape.push_str(&format!("arg={}", value_class(v)));
    }
    format!("{tool}|{shape}")
}

fn args_value(raw: Option<&RawValue>) -> Option<Value> {
    raw.and_then(|r| serde_json::from_str(r.get()).ok())
}

/// OMP call argument convention shared by projection and evidence reopen:
/// legacy `args` wins over `arguments`, and the selected field's parsed JSON
/// value is used unchanged (objects stay objects, strings stay strings).
fn omp_call_args<'a>(args: Option<&'a Value>, arguments: Option<&'a Value>) -> Option<&'a Value> {
    args.or(arguments)
}

fn strings_from_raw(raw: Option<&RawValue>) -> Vec<(String, String)> {
    let Some(raw) = raw else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<Value>(raw.get()) else { return Vec::new() };
    match v {
        Value::String(s) => vec!(("text".to_string(), s)),
        Value::Array(items) => {
            let mut out = Vec::new();
            for (i, item) in items.iter().enumerate() {
                let label = item
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("text")
                    .to_string();
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    out.push((format!("{label}[{i}]"), text.to_string()));
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

/// Project OMP `message.content` blocks from the exact bytes already decoded
/// for the address writer. Each emitted block gets a distinct versioned
/// selector even when several share one span.
pub fn project_omp_content(
    local_event: u32,
    span: SpanRef,
    content: &RawValue,
    role: Role,
) -> Vec<BlockProjection> {
    let mut out = Vec::new();
    let Ok(blocks) = serde_json::from_str::<Vec<OmpBlock<'_>>>(content.get()) else {
        return out;
    };
    for (i, b) in blocks.iter().enumerate() {
        let selector = format!("{PROJECTION_VERSION}|message.content[{i}]|{}", b.ty);
        match b.ty {
            "text" if prose_allowed(role) => {
                if let Some(text) = b.text.as_deref().filter(|t| !t.trim().is_empty()) {
                    let mut p = new_projection(
                        local_event, span, selector, ProjectionKind::Prose, role,
                    );
                    p.block_hash = digest8(text);
                    p.prose = text.to_string();
                    out.push(p);
                }
            }
            "toolCall" if role == Role::Assistant => {
                let null = Value::Null;
                let args = omp_call_args(b.args.as_ref(), b.arguments.as_ref()).unwrap_or(&null);
                let mut p = new_projection(
                    local_event, span, selector, ProjectionKind::Call, role,
                );
                p.call_sig = normalize_call_sig(b.name.as_deref(), Some(args));
                p.block_hash = digest8(&p.call_sig);
                out.push(p);
            }
            // thinking, toolResult, image and unknown types are excluded.
            _ => {}
        }
    }
    out
}

/// Project allowlisted Codex prose and call signatures from one payload.
pub fn project_codex_payload(
    local_event: u32,
    span: SpanRef,
    top_type: &str,
    payload: &RawValue,
    role: Role,
) -> Vec<BlockProjection> {
    let mut out = Vec::new();
    let Ok(p) = serde_json::from_str::<CodexPayload<'_>>(payload.get()) else {
        return out;
    };
    let inner = p.ty.unwrap_or("");
    let prose_role = if top_type == "event_msg" && matches!(inner, "user_message" | "agent_message") {
        if inner == "user_message" { Role::User } else { Role::Assistant }
    } else if top_type == "response_item" && inner == "message" && prose_allowed(role) {
        role
    } else {
        Role::Unknown
    };
    if prose_allowed(prose_role) {
        let src = if top_type == "event_msg" { p.message } else { p.content };
        let path = if top_type == "event_msg" { "payload.message" } else { "payload.content" };
        for (label, text) in strings_from_raw(src) {
            if text.trim().is_empty() {
                continue;
            }
            let selector = format!("{PROJECTION_VERSION}|{path}|{label}|text");
            let mut proj = new_projection(
                local_event, span, selector, ProjectionKind::Prose, prose_role,
            );
            proj.block_hash = digest8(&text);
            proj.prose = text;
            out.push(proj);
        }
    }
    if top_type == "response_item" && matches!(inner, "function_call" | "custom_tool_call" | "web_search_call") {
        let args = args_value(p.arguments.or(p.parameters));
        let mut proj = new_projection(
            local_event,
            span,
            format!("{PROJECTION_VERSION}|payload|call"),
            ProjectionKind::Call,
            Role::Assistant,
        );
        proj.call_sig = normalize_call_sig(p.name.as_deref(), args.as_ref());
        proj.block_hash = digest8(&proj.call_sig);
        out.push(proj);
    }
    out
}

#[derive(Clone, Debug)]
pub struct ExtractedBlock {
    pub kind: ProjectionKind,
    pub prose: Option<String>,
    pub call_sig: Option<String>,
    pub block_hash: [u8; 8],
}

/// Re-derive one projected block from raw line bytes by its versioned
/// selector. Used only when reopening evidence; no stored prose is consulted.
pub fn extract_block(provider: Provider, line: &[u8], selector: &str) -> ase_core::Result<ExtractedBlock> {
    let v: Value = serde_json::from_slice(line)
        .map_err(|_| err("raw span is not valid JSON for this selector"))?;
    let parts: Vec<&str> = selector.split('|').collect();
    if parts.first() != Some(&PROJECTION_VERSION) {
        return Err(err("unsupported selector version"));
    }
    let bracket_idx = |s: &str| -> Option<usize> {
        let a = s.find('[')? + 1;
        let b = a + s[a..].find(']')?;
        s[a..b].parse().ok()
    };
    let is_call = parts.get(2) == Some(&"call")
        || parts.get(2) == Some(&"toolCall")
        || parts.last() == Some(&"call");
    let value = match parts.get(1).copied() {
        Some(p) if p.starts_with("message.content[") => {
            if provider != Provider::Omp {
                return Err(err("selector/provider mismatch"));
            }
            let idx = bracket_idx(p).ok_or_else(|| err("bad content selector"))?;
            v.get("message")
                .and_then(|m| m.get("content"))
                .and_then(Value::as_array)
                .and_then(|a| a.get(idx))
                .ok_or_else(|| err("selector block not present in raw span"))?
        }
        Some("payload.message") => v
            .get("payload")
            .and_then(|p| p.get("message"))
            .ok_or_else(|| err("selector block not present in raw span"))?,
        Some("payload.content") => {
            let idx = parts
                .get(2)
                .and_then(|s| bracket_idx(s))
                .ok_or_else(|| err("bad content selector"))?;
            v.get("payload")
                .and_then(|p| p.get("content"))
                .and_then(Value::as_array)
                .and_then(|a| a.get(idx))
                .ok_or_else(|| err("selector block not present in raw span"))?
        }
        Some("payload") => v
            .get("payload")
            .ok_or_else(|| err("selector block not present in raw span"))?,
        _ => return Err(err("unsupported selector path")),
    };
    if is_call {
        let name = value.get("name").and_then(Value::as_str);
        let sig = if provider == Provider::Omp {
            // An OMP tool call must re-derive exactly what project_omp_content
            // wrote, including object-valued `arguments` and the legacy
            // `args`-before-`arguments` field precedence.
            let null = Value::Null;
            let args = omp_call_args(value.get("args"), value.get("arguments")).unwrap_or(&null);
            normalize_call_sig(name, Some(args))
        } else {
            // Codex and older call shapes keep their string, `args` and
            // `parameters` variants.
            let args = value
                .get("arguments")
                .and_then(Value::as_str)
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .or_else(|| value.get("args").cloned())
                .or_else(|| value.get("parameters").cloned());
            normalize_call_sig(name, args.as_ref())
        };
        let block_hash = digest8(&sig);
        return Ok(ExtractedBlock {
            kind: ProjectionKind::Call,
            prose: None,
            call_sig: Some(sig),
            block_hash,
        });
    }
    let text = match value {
        Value::String(s) => Some(s.clone()),
        _ => value.get("text").and_then(Value::as_str).map(str::to_string),
    }
    .ok_or_else(|| err("selector does not address text"))?;
    Ok(ExtractedBlock {
        kind: ProjectionKind::Prose,
        prose: Some(text.clone()),
        call_sig: None,
        block_hash: digest8(&text),
    })
}
