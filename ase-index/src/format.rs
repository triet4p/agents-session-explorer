//! Compact, content-free address index format.
//!
//! All integers are little-endian and read by offset, so no unaligned casts.
//! No decoded text, postings, shingle or model output is stored.

use ase_core::{EventKind, Role};

/// Legacy on-disk magic written by released indexes. The bytes are a stable
/// file-format contract and must not change; the spelling is retained only
/// inside this constant and is not a pattern for new identifiers.
pub const MAGIC: &[u8; 8] = b"ASEP0A01";
pub const HEADER_LEN: usize = 64;
pub const EV_LEN: usize = 72;
pub const FILE_LEN: usize = 48;
pub const SESSION_LEN: usize = 40;
pub const TYPE_LEN: usize = 8;

pub const H_BUILD: usize = 8;
pub const H_FILE_COUNT: usize = 16;
pub const H_EVENT_COUNT: usize = 20;
pub const H_SESSION_COUNT: usize = 24;
pub const H_STRING_LEN: usize = 28;
pub const H_FILES_OFF: usize = 32;
pub const H_EVENTS_OFF: usize = 36;
pub const H_SESSIONS_OFF: usize = 40;
pub const H_STRINGS_OFF: usize = 44;
pub const H_TYPE_COUNT: usize = 48;
pub const H_TYPES_OFF: usize = 52;
pub const E_OFF: usize = 0;
pub const E_HASH: usize = 8;
pub const E_TS: usize = 16;
pub const E_FILE: usize = 24;
pub const E_ORDINAL: usize = 28;
pub const E_LEN: usize = 32;
pub const E_SESSION: usize = 36;
pub const E_PARENT: usize = 40;
pub const E_LINK: usize = 44;
pub const E_EXT: usize = 48;
pub const E_TYPE: usize = 52;
pub const E_TAG: usize = 56;
pub const E_TYPE_LEN: usize = 60;
pub const E_TAG_LEN: usize = 64;
pub const E_KIND: usize = 68;
pub const E_ROLE: usize = 69;
pub const E_FLAGS: usize = 70;

pub const F_SIZE: usize = 0;
pub const F_MTIME_S: usize = 8;
pub const F_MTIME_N: usize = 16;
pub const F_PATH_OFF: usize = 20;
pub const F_PATH_LEN: usize = 24;
pub const F_EVT_START: usize = 28;
pub const F_EVT_END: usize = 32;
pub const F_SES_START: usize = 36;
pub const F_SES_END: usize = 40;
pub const F_PROVIDER: usize = 44;
pub const F_ENCODING: usize = 45;
pub const F_FLAGS: usize = 46;

pub const S_ID_OFF: usize = 0;
pub const S_ID_LEN: usize = 4;
pub const S_FILE: usize = 8;
pub const S_FIRST: usize = 12;
pub const S_LAST: usize = 16;
pub const S_PARENT_OFF: usize = 20;
pub const S_PARENT_LEN: usize = 24;
pub const S_PARENT_IDX: usize = 28;
pub const S_HEADER: usize = 32;
pub const S_FLAGS: usize = 36;

#[inline]
pub fn get_u32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().expect("u32 slice"))
}

#[inline]
pub fn get_i64(b: &[u8], o: usize) -> i64 {
    i64::from_le_bytes(b[o..o + 8].try_into().expect("i64 slice"))
}

#[inline]
pub fn get_u64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().expect("u64 slice"))
}

#[inline]
pub fn put_u32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}

#[inline]
pub fn put_i64(b: &mut Vec<u8>, v: i64) {
    b.extend_from_slice(&v.to_le_bytes());
}

#[inline]
pub fn put_u64(b: &mut Vec<u8>, v: u64) {
    b.extend_from_slice(&v.to_le_bytes());
}

pub fn pad8(b: &mut Vec<u8>) {
    while b.len() % 8 != 0 {
        b.push(0);
    }
}

pub fn kind_code(k: EventKind) -> u8 {
    match k {
        EventKind::SessionMeta => 0,
        EventKind::UserInput => 1,
        EventKind::AssistantText => 2,
        EventKind::Reasoning => 3,
        EventKind::ToolCall => 4,
        EventKind::ToolResult => 5,
        EventKind::Notice => 6,
        EventKind::Title => 7,
        EventKind::ModelChange => 8,
        EventKind::Usage => 9,
        EventKind::Unknown => 10,
    }
}

pub fn kind_from(code: u8) -> EventKind {
    match code {
        0 => EventKind::SessionMeta,
        1 => EventKind::UserInput,
        2 => EventKind::AssistantText,
        3 => EventKind::Reasoning,
        4 => EventKind::ToolCall,
        5 => EventKind::ToolResult,
        6 => EventKind::Notice,
        7 => EventKind::Title,
        8 => EventKind::ModelChange,
        9 => EventKind::Usage,
        _ => EventKind::Unknown,
    }
}

pub fn role_code(r: Role) -> u8 {
    match r {
        Role::Unknown => 0,
        Role::User => 1,
        Role::Assistant => 2,
        Role::Tool => 3,
        Role::System => 4,
    }
}

pub fn role_from(code: u8) -> Role {
    match code {
        1 => Role::User,
        2 => Role::Assistant,
        3 => Role::Tool,
        4 => Role::System,
        _ => Role::Unknown,
    }
}
