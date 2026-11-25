//! Core types and seams.
//!
//! This crate deliberately contains no parsing, I/O, SQLite, model or network
//! dependency. It is the shared vocabulary between the source/decoder, the
//! address index and the CLI render layer (INITIALIZATION.md §7.3, §7.4).

use std::fmt;
use std::sync::Arc;

pub type FileId = u32;
pub type EventIdx = u32;
pub const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Omp,
    Codex,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Provider::Omp => "omp",
            Provider::Codex => "codex",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Jsonl,
    JsonlZstd,
}

impl Encoding {
    pub fn label(self) -> &'static str {
        match self {
            Encoding::Jsonl => "jsonl",
            Encoding::JsonlZstd => "jsonl.zst",
        }
    }

    /// Source coordinate system for spans of this encoding.
    /// zstd offsets are offsets in the decompressed stream, never in the
    /// compressed file (INITIALIZATION.md §6.1, §14).
    pub fn coordinate(self) -> &'static str {
        match self {
            Encoding::Jsonl => "raw_file_bytes",
            Encoding::JsonlZstd => "decompressed_stream_bytes",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    SessionMeta,
    UserInput,
    AssistantText,
    Reasoning,
    ToolCall,
    ToolResult,
    Notice,
    Title,
    ModelChange,
    Usage,
    Unknown,
}

impl EventKind {
    pub fn label(self) -> &'static str {
        match self {
            EventKind::SessionMeta => "SessionMeta",
            EventKind::UserInput => "UserInput",
            EventKind::AssistantText => "AssistantText",
            EventKind::Reasoning => "Reasoning",
            EventKind::ToolCall => "ToolCall",
            EventKind::ToolResult => "ToolResult",
            EventKind::Notice => "Notice",
            EventKind::Title => "Title",
            EventKind::ModelChange => "ModelChange",
            EventKind::Usage => "Usage",
            EventKind::Unknown => "Unknown",
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        Some(match s {
            "SessionMeta" => EventKind::SessionMeta,
            "UserInput" => EventKind::UserInput,
            "AssistantText" => EventKind::AssistantText,
            "Reasoning" => EventKind::Reasoning,
            "ToolCall" => EventKind::ToolCall,
            "ToolResult" => EventKind::ToolResult,
            "Notice" => EventKind::Notice,
            "Title" => EventKind::Title,
            "ModelChange" => EventKind::ModelChange,
            "Usage" => EventKind::Usage,
            "Unknown" => EventKind::Unknown,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Unknown,
    User,
    Assistant,
    Tool,
    System,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::Unknown => "unknown",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "toolResult",
            Role::System => "system",
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        Some(match s {
            "unknown" => Role::Unknown,
            "user" => Role::User,
            "assistant" => Role::Assistant,
            "toolResult" | "tool" => Role::Tool,
            "system" | "developer" => Role::System,
            _ => return None,
        })
    }
}

/// The single currency of the address layer. `off`/`len` are in the coordinate system of the
/// owning file (see `Encoding::coordinate`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpanRef {
    pub file: FileId,
    pub off: u64,
    pub len: u32,
}

#[derive(Clone, Debug)]
pub struct SourceFile {
    pub id: FileId,
    pub path: String,
    pub provider: Provider,
    pub encoding: Encoding,
    pub size: u64,
    pub mtime_secs: i64,
    pub mtime_nanos: u32,
}

#[derive(Clone, Debug)]
pub struct DecodedEvent {
    pub ordinal: u32,
    pub span: SpanRef,
    pub kind: EventKind,
    pub role: Role,
    pub ts_ms: i64,
    /// Verbatim top-level `type` (or `<malformed>`).
    pub source_type: String,
    /// Most specific source label: customType / message role / top-level type.
    pub tag: String,
    pub source_id: Option<String>,
    pub parent_id: Option<String>,
    /// Global event index after assembly; `NONE` when unresolved/absent.
    pub parent_idx: u32,
    pub call_id: Option<String>,
    /// Global event index after assembly; `NONE` when unresolved/absent.
    pub link_idx: u32,
    /// Interned id for an unresolved external reference (source metadata).
    pub ext_id: Option<String>,
    pub session_local: u32,
    /// Non-cryptographic FNV-1a 64-bit digest of the exact span bytes.
    pub hash: u64,
    pub valid_json: bool,
}

#[derive(Clone, Debug)]
pub struct SessionMeta {
    pub id: String,
    pub file: FileId,
    pub parent_session: Option<String>,
    pub path_derived: bool,
    pub local_index: u32,
}

#[derive(Clone, Debug, Default)]
pub struct DecodeStats {
    pub events: usize,
    pub malformed: usize,
    pub unknown_type: usize,
    pub unresolved_parent: usize,
    pub unresolved_link: usize,
    pub multi_call_entries: usize,
    pub sessions: usize,
}

/// Version namespace for per-block selectors. Changing the meaning of an
/// existing selector requires a new version, never an in-place reinterpretation.
pub const PROJECTION_VERSION: &str = "block-v1";
pub const CALLSIG_VERSION: &str = "callsig-v1";
pub const ALLOWLIST_VERSION: &str = "prose-allowlist-v1";
pub const TOKENIZER_VERSION: &str = "unicode61-remove-diacritics-0";
pub const SHINGLE_VERSION: &str = "shingle-k3k5-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionKind {
    Prose,
    Call,
}

impl ProjectionKind {
    pub fn label(self) -> &'static str {
        match self {
            ProjectionKind::Prose => "prose",
            ProjectionKind::Call => "call",
        }
    }
}

#[derive(Clone, Debug)]
pub struct BlockProjection {
    /// Event ordinal inside its source file; writers add the file base.
    pub local_event: u32,
    pub span: SpanRef,
    pub selector: String,
    pub kind: ProjectionKind,
    pub role: Role,
    /// Present only for prose; consumed by the writer and then dropped.
    pub prose: String,
    /// Present only for calls; normalized, with no argument values.
    pub call_sig: String,
    /// First 8 bytes of SHA-256 over the canonical projected value.
    pub block_hash: [u8; 8],
    pub session_local: u32,
    pub parent_idx: u32,
}

#[derive(Clone, Debug)]
pub struct FileDecode {
    pub events: Vec<DecodedEvent>,
    pub sessions: Vec<SessionMeta>,
    pub stats: DecodeStats,
    /// Projections derived from the same bytes that produced `events`.
    pub projections: Vec<BlockProjection>,
}

#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub fn err<S: Into<String>>(s: S) -> Error {
    Error(s.into())
}

/// Enumerates raw files. No index, no content decoding.
pub trait Source {
    fn discover(&self) -> Result<Vec<SourceFile>>;
}

/// Turns raw bytes into addressable events. Never stores decoded text.
pub trait Decoder {
    fn decode(&self, file: &SourceFile, bytes: &[u8]) -> Result<FileDecode>;
}

/// Reads raw bytes in source coordinates. jsonl: direct file bytes.
/// zstd: bytes of the decompressed stream.
pub trait RawStore {
    fn bytes(&self, file: FileId) -> Result<Arc<[u8]>>;

    fn span(&self, span: SpanRef) -> Result<Vec<u8>> {
        let bytes = self.bytes(span.file)?;
        let start = span.off as usize;
        let end = start
            .checked_add(span.len as usize)
            .ok_or_else(|| err("span length overflow"))?;
        if end > bytes.len() {
            return Err(err(format!(
                "span out of range: off={} len={} stream_len={}",
                span.off,
                span.len,
                bytes.len()
            )));
        }
        Ok(bytes[start..end].to_vec())
    }
}
