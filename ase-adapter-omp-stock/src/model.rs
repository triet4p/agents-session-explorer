//! Claim vocabulary for the OMP stock adapter.
//!
//! This crate is deliberately separate from `ase-task` and `ase-motif`: it only
//! produces advisory claims with reopenable evidence. It never writes resolver
//! membership, motif membership, the address index or a source journal.

use ase_core::{Encoding, EventIdx, FileId, Provider, Role, SpanRef};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const LINK_RULE: &str = "omp-stock-link";
pub const LINK_VERSION: &str = "stock-link-v1";
pub const REPO_RULE: &str = "omp-stock-repo";
pub const REPO_VERSION: &str = "stock-repo-v1";
pub const TASK_RULE: &str = "omp-stock-task";
pub const TASK_VERSION: &str = "stock-task-v1";
/// Version of the evidence selector/decoder used by this adapter. Changing the
/// extracted value requires a new version, never an in-place reinterpretation.
pub const SELECTOR_VERSION: &str = "stock-sel-v1";
pub const ADAPTER_NAME: &str = "omp-stock";

pub const LAYER_LINK: &str = "parent_child";
pub const LAYER_REPO: &str = "repo";
pub const LAYER_TASK: &str = "task";

pub const STATE_CONFIRMED: &str = "confirmed";
pub const STATE_CANDIDATE: &str = "candidate";
pub const STATE_MISSING: &str = "missing";
pub const STATE_UNAVAILABLE: &str = "unavailable";
pub const STATE_CONFLICT: &str = "conflict";
pub const STATE_MULTI_TASK: &str = "multi_task";

/// Facts about one indexed source file, supplied by the composition root.
#[derive(Clone, Debug)]
pub struct ViewFile {
    pub id: FileId,
    pub path: String,
    pub provider: Provider,
    pub encoding: Encoding,
    pub size: u64,
    /// SHA-256 of the whole source file from the pinned publication manifest.
    pub manifest_digest: Option<String>,
}

/// Facts about one indexed event. The adapter reads bytes through the pinned
/// view; it never assumes that a `FileId` survives a rebuild.
#[derive(Clone, Debug)]
pub struct ViewEvent {
    pub event: EventIdx,
    pub ordinal: u32,
    pub span: SpanRef,
    pub hash: u64,
    pub kind: ase_core::EventKind,
    pub role: Role,
    pub tag: String,
}

/// Read-only port implemented by the CLI composition root over `ase-index` and
/// the pinned raw store. No write method is exposed to the adapter.
pub trait StockView {
    fn generation(&self) -> &str;
    fn files(&self) -> &[ViewFile];
    fn events(&self, file: FileId) -> ase_core::Result<Vec<ViewEvent>>;
    /// Whole source bytes in the file's coordinate system.
    fn bytes(&self, file: FileId) -> ase_core::Result<std::sync::Arc<[u8]>>;
    fn span_bytes(&self, span: SpanRef) -> ase_core::Result<Vec<u8>>;
    fn event_at(&self, span: SpanRef) -> ase_core::Result<Option<ViewEvent>>;
    fn session_header(&self, file: FileId) -> ase_core::Result<Option<ViewEvent>>;
    /// Reads an unindexed sidecar file (a child handoff `.md`) only. `None`
    /// means the file does not exist or exceeds the adapter's read cap.
    fn sidecar_bytes(&self, path: &str) -> ase_core::Result<Option<Vec<u8>>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpanJson {
    pub file: u32,
    pub off: u64,
    pub len: u32,
}

impl From<SpanRef> for SpanJson {
    fn from(s: SpanRef) -> Self {
        Self { file: s.file, off: s.off, len: s.len }
    }
}

impl SpanJson {
    pub fn to_span(self) -> SpanRef {
        SpanRef { file: self.file, off: self.off, len: self.len }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub provider: String,
    pub generation_id: String,
    pub coordinate: String,
    pub file_id: Option<u32>,
    pub path: String,
    pub event: Option<u32>,
    pub selector: String,
    pub selector_version: String,
    pub span: SpanJson,
    /// FNV-1a index checksum for indexed journal spans. Sidecar files are not
    /// indexed, so they carry no index checksum.
    pub span_checksum: Option<String>,
    /// SHA-256 over the exact source bytes of the span.
    pub line_digest: String,
    /// SHA-256 over the value re-derived by `selector`.
    pub block_digest: Option<String>,
    pub manifest_digest: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subject {
    pub session_id: Option<String>,
    pub file_id: Option<u32>,
    pub path: Option<String>,
    pub event: Option<u32>,
    pub call_id: Option<String>,
    pub child_file_id: Option<u32>,
    pub child_path: Option<String>,
    pub segment: Option<[u32; 2]>,
}

impl Subject {
    pub fn for_file(file: &ViewFile, session_id: Option<&str>) -> Self {
        Self {
            session_id: session_id.map(str::to_string),
            file_id: Some(file.id),
            path: Some(file.path.clone()),
            event: None,
            call_id: None,
            child_file_id: None,
            child_path: None,
            segment: None,
        }
    }

    pub fn for_call(
        file: &ViewFile,
        session_id: Option<&str>,
        event: Option<u32>,
        call_id: Option<&str>,
    ) -> Self {
        Self {
            session_id: session_id.map(str::to_string),
            file_id: Some(file.id),
            path: Some(file.path.clone()),
            event,
            call_id: call_id.map(str::to_string),
            child_file_id: None,
            child_path: None,
            segment: None,
        }
    }

    pub fn with_child(mut self, child: &ViewFile) -> Self {
        self.child_file_id = Some(child.id);
        self.child_path = Some(child.path.clone());
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Claim {
    pub claim_id: String,
    pub layer: String,
    pub state: String,
    pub predicate: String,
    pub value: Value,
    pub subject: Subject,
    pub basis: String,
    pub rule: String,
    pub rule_version: String,
    pub evidence: Vec<EvidenceRef>,
    pub refutes: Vec<String>,
    pub context: Vec<String>,
    pub reasons: Vec<String>,
}

impl Claim {
    pub fn new(
        layer: &str,
        state: &str,
        predicate: &str,
        value: Value,
        subject: Subject,
        basis: &str,
        rule: &str,
        rule_version: &str,
        evidence: Vec<EvidenceRef>,
    ) -> Self {
        let mut claim = Self {
            claim_id: String::new(),
            layer: layer.to_string(),
            state: state.to_string(),
            predicate: predicate.to_string(),
            value,
            subject,
            basis: basis.to_string(),
            rule: rule.to_string(),
            rule_version: rule_version.to_string(),
            evidence,
            refutes: Vec::new(),
            context: Vec::new(),
            reasons: Vec::new(),
        };
        claim.claim_id = claim.deterministic_id();
        claim
    }

    pub fn reason(mut self, reason: impl Into<String>) -> Self {
        self.reasons.push(reason.into());
        self
    }

    pub fn refute(mut self, reason: impl Into<String>) -> Self {
        self.refutes.push(reason.into());
        self
    }

    pub fn context(mut self, note: impl Into<String>) -> Self {
        self.context.push(note.into());
        self
    }

    /// Deterministic identity from the claim's semantic coordinates and its
    /// evidence spans. Paths are not identity; file ids are only used together
    /// with the generation pinned in every evidence item.
    fn deterministic_id(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.layer.as_bytes());
        h.update(b"\0");
        h.update(self.rule.as_bytes());
        h.update(b"\0");
        h.update(self.rule_version.as_bytes());
        h.update(b"\0");
        h.update(self.predicate.as_bytes());
        h.update(b"\0");
        h.update(serde_json::to_vec(&self.value).unwrap_or_default());
        h.update(b"\0");
        h.update(serde_json::to_vec(&self.subject).unwrap_or_default());
        for e in &self.evidence {
            h.update(b"\0");
            h.update(e.generation_id.as_bytes());
            h.update(b"\0");
            h.update(e.path.as_bytes());
            h.update(b"\0");
            h.update(e.span.file.to_le_bytes());
            h.update(e.span.off.to_le_bytes());
            h.update(e.span.len.to_le_bytes());
            h.update(b"\0");
            h.update(e.selector.as_bytes());
        }
        let digest = h.finalize();
        let mut out = String::with_capacity(16);
        for b in &digest[..8] {
            out.push_str(&format!("{b:02x}"));
        }
        out
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StateCounts {
    pub claims: usize,
    pub confirmed: usize,
    pub candidate: usize,
    pub missing: usize,
    pub unavailable: usize,
    pub conflict: usize,
    pub multi_task: usize,
}

impl StateCounts {
    pub fn add(&mut self, state: &str) {
        self.claims += 1;
        match state {
            STATE_CONFIRMED => self.confirmed += 1,
            STATE_CANDIDATE => self.candidate += 1,
            STATE_MISSING => self.missing += 1,
            STATE_UNAVAILABLE => self.unavailable += 1,
            STATE_CONFLICT => self.conflict += 1,
            STATE_MULTI_TASK => self.multi_task += 1,
            _ => {}
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayerSummary {
    pub rule: String,
    pub rule_version: String,
    pub predicate: String,
    pub counts: StateCounts,
    /// Number of scanned scopes that had at least one signal.
    pub scopes_with_signal: usize,
    /// Number of scanned scopes with no signal (reported as missing, never
    /// silently dropped or counted as "wrong").
    pub scopes_missing: usize,
    /// Session-level states, e.g. `multi_task: 3`.
    pub session_states: BTreeMap<String, usize>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ScanCounts {
    pub files: usize,
    pub parent_files: usize,
    pub child_files: usize,
    pub sessions: usize,
    pub events_read: usize,
    pub task_calls: usize,
    pub sidecars_read: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanError {
    pub code: String,
    pub path: Option<String>,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub adapter: String,
    pub generation: String,
    pub evidence_selector_version: String,
    pub claim_policy: String,
    pub scan: ScanCounts,
    pub layers: BTreeMap<String, LayerSummary>,
    pub claims: Vec<Claim>,
    pub errors: Vec<ScanError>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClaimsFile {
    pub adapter: String,
    pub generation: String,
    pub evidence_selector_version: String,
    pub claims: Vec<Claim>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvidenceCheck {
    pub index: usize,
    pub path: String,
    pub selector: String,
    pub status: String,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReopenedClaim {
    pub claim_id: String,
    pub layer: String,
    pub original_state: String,
    pub assessed_state: String,
    pub evidence: Vec<EvidenceCheck>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReopenReport {
    pub adapter: String,
    pub generation: String,
    pub active_generation: String,
    pub status: String,
    pub claims: Vec<ReopenedClaim>,
    pub counts: BTreeMap<String, usize>,
    pub errors: Vec<ScanError>,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// FNV-1a 64-bit, the checksum already stored by the address index. Only
/// unindexed sidecar evidence needs a local computation.
pub fn fnv64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Selector construction and re-derivation. Every value used by a claim is
/// named by exactly one versioned selector so `reopen` can re-extract it from
/// raw bytes instead of trusting stored text.
pub fn selector_session(field: &str) -> String {
    format!("{SELECTOR_VERSION}|session.{field}")
}

pub fn selector_content(index: usize, field: &str) -> String {
    format!("{SELECTOR_VERSION}|message.content[{index}].{field}")
}

/// Nested field inside an assistant `toolCall` argument object. `pointer` is a
/// JSON-pointer-like `/tasks/0/name`, resolved on the `arguments` value.
pub fn selector_content_json(index: usize, field: &str, pointer: &str) -> String {
    format!("{SELECTOR_VERSION}|message.content[{index}].{field}#{pointer}")
}

pub fn selector_data(field: &str) -> String {
    format!("{SELECTOR_VERSION}|data.{field}")
}

/// Nested field inside `data.args` of a `tool_execution_start` record.
pub fn selector_data_json(pointer: &str) -> String {
    format!("{SELECTOR_VERSION}|data.args#{pointer}")
}

pub fn selector_token(scope: &str) -> String {
    format!("{SELECTOR_VERSION}|extract:{scope}")
}

/// Token occurrence inside an assistant `toolCall` argument object (usually the
/// assigned `task` prompt).
pub fn selector_arguments_token(index: usize, token: &str) -> String {
    format!("{SELECTOR_VERSION}|extract:task_token.arguments[{index}]:{token}")
}

/// Token occurrence inside a `tool_execution_start` `data.args` object.
pub fn selector_data_args_token(token: &str) -> String {
    format!("{SELECTOR_VERSION}|extract:task_token.data_args:{token}")
}

pub fn selector_message_token(index: usize, token: &str) -> String {
    format!("{SELECTOR_VERSION}|extract:task_token.message[{index}]:{token}")
}

pub fn selector_reminder_cwd(cwd: &str) -> String {
    format!("{SELECTOR_VERSION}|extract:cwd.reminder:{cwd}")
}

pub fn selector_sidecar_token(token: &str) -> String {
    format!("{SELECTOR_VERSION}|sidecar.task_token:{token}")
}

#[derive(Debug)]
pub struct SelectorError(pub String);

impl std::fmt::Display for SelectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Re-derives the value named by `selector` from one decoded event. Returns the
/// canonical block bytes whose SHA-256 is the claim's `block_digest`.
pub fn derive_block(value: &Value, selector: &str) -> Result<Vec<u8>, SelectorError> {
    let rest = selector
        .strip_prefix(SELECTOR_VERSION)
        .and_then(|s| s.strip_prefix('|'))
        .ok_or_else(|| SelectorError("selector_version_unknown".to_string()))?;
    if let Some(field) = rest.strip_prefix("session.") {
        return field_value(value.get(field), selector);
    }
    if let Some(field) = rest.strip_prefix("data.") {
        let (field, pointer) = split_pointer(field);
        let base = value.get("data").and_then(|v| v.get(field));
        return resolve_pointer(base, pointer, selector);
    }
    if let Some(body) = rest.strip_prefix("message.content[") {
        let (idx, field) = body
            .split_once("].")
            .ok_or_else(|| SelectorError("malformed content selector".to_string()))?;
        let idx: usize = idx
            .parse()
            .map_err(|_| SelectorError("malformed content index".to_string()))?;
        let block = value
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.get(idx))
            .ok_or_else(|| SelectorError("content block missing".to_string()))?;
        let (field, pointer) = split_pointer(field);
        return resolve_pointer(block.get(field), pointer, selector);
    }
    if let Some(scope) = rest.strip_prefix("extract:") {
        if let Some(body) = scope.strip_prefix("task_token.message[") {
            let (idx, token) = body
                .split_once("]:")
                .ok_or_else(|| SelectorError("malformed message token selector".to_string()))?;
            let idx: usize = idx
                .parse()
                .map_err(|_| SelectorError("malformed content index".to_string()))?;
            let text = value
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.get(idx))
                .and_then(|b| b.get("text"))
                .and_then(Value::as_str)
                .ok_or_else(|| SelectorError("message text missing".to_string()))?;
            if !token_present(text, token) {
                return Err(SelectorError(format!("token not present: {token}")));
            }
            return Ok(token.as_bytes().to_vec());
        }
        if let Some(body) = scope.strip_prefix("task_token.arguments[") {
            let (idx, token) = body
                .split_once("]:")
                .ok_or_else(|| SelectorError("malformed arguments token selector".to_string()))?;
            let idx: usize = idx
                .parse()
                .map_err(|_| SelectorError("malformed content index".to_string()))?;
            let args = value
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.get(idx))
                .and_then(|b| b.get("arguments"))
                .ok_or_else(|| SelectorError("task arguments missing".to_string()))?;
            for text in strings_in_value(args) {
                if token_present(&text, token) {
                    return Ok(token.as_bytes().to_vec());
                }
            }
            return Err(SelectorError(format!("token not present: {token}")));
        }
        if let Some(cwd) = scope.strip_prefix("cwd.reminder:") {
            let reminder = value
                .get("data")
                .and_then(|d| d.get("reminder"))
                .and_then(Value::as_str)
                .ok_or_else(|| SelectorError("reminder text missing".to_string()))?;
            let observed = reminder_cwd(reminder)
                .ok_or_else(|| SelectorError("reminder cwd missing".to_string()))?;
            if !path_eq(&observed, cwd) {
                return Err(SelectorError("reminder cwd differs".to_string()));
            }
            return Ok(observed.into_bytes());
        }
        if let Some(token) = scope.strip_prefix("task_token.data_args:") {
            let args = value
                .get("data")
                .and_then(|d| d.get("args"))
                .ok_or_else(|| SelectorError("task args missing".to_string()))?;
            for text in strings_in_value(args) {
                if token_present(&text, token) {
                    return Ok(token.as_bytes().to_vec());
                }
            }
            return Err(SelectorError(format!("token not present: {token}")));
        }
        if let Some(scope) = scope.strip_prefix("task_token.task_argument:") {
            let tasks = value
                .get("data")
                .and_then(|d| d.get("args"))
                .and_then(|a| a.get("tasks"))
                .and_then(Value::as_array)
                .ok_or_else(|| SelectorError("task args missing".to_string()))?;
            for task in tasks {
                if let Some(text) = task.get("task").and_then(Value::as_str) {
                    if crate::extract::extract_task_tokens(text)
                        .iter()
                        .any(|t| t.token == scope)
                    {
                        return Ok(scope.as_bytes().to_vec());
                    }
                }
            }
            return Err(SelectorError(format!("token not present: {scope}")));
        }
        return Err(SelectorError(format!("selector scope unknown: {scope}")));
    }
    Err(SelectorError(format!("selector unknown: {selector}")))
}

fn token_present(text: &str, token: &str) -> bool {
    crate::extract::extract_task_tokens(text)
        .iter()
        .any(|t| t.token == token)
        || crate::extract::extract_task_id_markers(text)
            .iter()
            .any(|t| t.token == token)
}

fn strings_in_value(value: &Value) -> Vec<String> {
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

fn split_pointer(field: &str) -> (&str, Option<&str>) {
    match field.split_once('#') {
        Some((field, pointer)) => (field, Some(pointer)),
        None => (field, None),
    }
}

fn field_value(v: Option<&Value>, selector: &str) -> Result<Vec<u8>, SelectorError> {
    v.map(canonical)
        .ok_or_else(|| SelectorError(format!("selector field missing: {selector}")))
}

fn resolve_pointer(
    base: Option<&Value>,
    pointer: Option<&str>,
    selector: &str,
) -> Result<Vec<u8>, SelectorError> {
    let Some(base) = base else {
        return Err(SelectorError(format!("selector base missing: {selector}")));
    };
    let Some(pointer) = pointer else {
        return Ok(canonical(base));
    };
    let mut cur = base;
    for part in pointer.trim_start_matches('/').split('/').filter(|p| !p.is_empty()) {
        cur = match cur {
            Value::Object(map) => map
                .get(part)
                .ok_or_else(|| SelectorError(format!("selector pointer missing: {selector}")))?,
            Value::Array(items) => {
                let idx: usize = part
                    .parse()
                    .map_err(|_| SelectorError("selector pointer index".to_string()))?;
                items
                    .get(idx)
                    .ok_or_else(|| SelectorError(format!("selector pointer missing: {selector}")))?
            }
            _ => return Err(SelectorError(format!("selector pointer not traversable: {selector}"))),
        };
    }
    Ok(canonical(cur))
}

pub fn reminder_cwd(reminder: &str) -> Option<String> {
    let lower = reminder.to_ascii_lowercase();
    let marker = "current working directory:";
    let at = lower.find(marker)? + marker.len();
    let rest = reminder[at..].trim_start();
    let rest = rest.strip_prefix('\'')?;
    let end = rest.find('\'')?;
    Some(rest[..end].to_string())
}

fn canonical(v: &Value) -> Vec<u8> {
    match v {
        Value::String(s) => s.as_bytes().to_vec(),
        other => serde_json::to_vec(other).unwrap_or_default(),
    }
}

/// Normalize the case and separator differences the stock journals show for
/// the same workspace path. Comparison is always on the normalized form; the
/// original string remains separate evidence.
pub fn normalize_path(path: &str) -> String {
    let mut s = path.trim().replace('\\', "/");
    while s.contains("//") {
        s = s.replace("//", "/");
    }
    if s.len() > 2 && s.ends_with('/') {
        s.pop();
    }
    s
}

pub fn path_key(path: &str) -> String {
    normalize_path(path).to_lowercase()
}

pub fn path_eq(a: &str, b: &str) -> bool {
    path_key(a) == path_key(b)
}

pub fn path_under(path: &str, root: &str) -> bool {
    let p = path_key(path);
    let r = path_key(root);
    p == r || p.starts_with(&(r + "/"))
}

pub fn repo_key_from_cwd(cwd: &str) -> Option<String> {
    normalize_path(cwd)
        .rsplit('/')
        .find(|s| !s.is_empty())
        .map(str::to_string)
}

/// Session header fields observed directly in the stock `session` record.
#[derive(Clone, Debug, Default)]
pub struct Header {
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub parent_session: Option<String>,
    pub title: Option<String>,
}

pub fn parse_header(value: &Value) -> Header {
    Header {
        session_id: value.get("id").and_then(Value::as_str).map(str::to_string),
        cwd: value.get("cwd").and_then(Value::as_str).map(str::to_string),
        parent_session: value
            .get("parentSession")
            .and_then(Value::as_str)
            .map(str::to_string),
        title: value.get("title").and_then(Value::as_str).map(str::to_string),
    }
}
