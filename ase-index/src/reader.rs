use ase_core::{
    err, Encoding, EventKind, FileId, Provider, Result, Role, SpanRef, NONE,
};
use std::fs;
use std::path::Path;
use std::sync::Arc;

use crate::format::*;

pub struct Index {
    data: Arc<[u8]>,
    pub file_count: u32,
    pub event_count: u32,
    pub session_count: u32,
    pub string_len: u32,
    pub files_off: u32,
    pub events_off: u32,
    pub sessions_off: u32,
    pub strings_off: u32,
    pub type_count: u32,
    pub types_off: u32,
    /// Absolute start of the type-string region inside the string blob.
    type_base: u32,
    pub build_ts: i64,
}

impl Index {
    pub fn open(path: &Path) -> Result<Index> {
        let bytes = fs::read(path).map_err(|e| err(format!("read index {}: {e}", path.display())))?;
        Index::from_bytes(bytes.into_boxed_slice().into())
    }

    pub fn from_bytes(data: Arc<[u8]>) -> Result<Index> {
        if data.len() < HEADER_LEN || &data[0..8] != &MAGIC[..] {
            return Err(err("bad index magic or truncated header"));
        }
        let idx = Index {
            file_count: get_u32(&data, H_FILE_COUNT),
            event_count: get_u32(&data, H_EVENT_COUNT),
            session_count: get_u32(&data, H_SESSION_COUNT),
            string_len: get_u32(&data, H_STRING_LEN),
            files_off: get_u32(&data, H_FILES_OFF),
            events_off: get_u32(&data, H_EVENTS_OFF),
            sessions_off: get_u32(&data, H_SESSIONS_OFF),
            strings_off: get_u32(&data, H_STRINGS_OFF),
            type_count: get_u32(&data, H_TYPE_COUNT),
            types_off: get_u32(&data, H_TYPES_OFF),
            type_base: 0,
            build_ts: get_i64(&data, H_BUILD),
            data,
        };
        let need = (idx.files_off as usize)
            .checked_add(idx.file_count as usize * FILE_LEN)
            .and_then(|v| v.checked_add(idx.event_count as usize * EV_LEN))
            .and_then(|v| v.checked_add(idx.session_count as usize * SESSION_LEN))
            .ok_or_else(|| err("index size overflow"))?;
        if idx.data.len() < need
            || (idx.strings_off as usize + idx.string_len as usize) > idx.data.len()
            || (idx.types_off as usize + idx.type_count as usize * TYPE_LEN) > idx.data.len()
        {
            return Err(err("truncated address index"));
        }
        // Event type/tag offsets are relative to the type-string region, which
        // follows the path/session string region in one blob. The type table's
        // entries span exactly that region, so its length (end - minimum
        // offset) yields the boundary regardless of whether the writer stored
        // relative or blob-absolute table offsets.
        let mut min_type_off = u32::MAX;
        let mut max_type_end = 0u32;
        for i in 0..idx.type_count as usize {
            let rec = idx.types_off as usize + i * TYPE_LEN;
            let off = get_u32(&idx.data, rec);
            let len = get_u32(&idx.data, rec + 4);
            if off < min_type_off {
                min_type_off = off;
            }
            if off.saturating_add(len) > max_type_end {
                max_type_end = off.saturating_add(len);
            }
        }
        let types_len = if min_type_off == u32::MAX {
            0
        } else {
            max_type_end.saturating_sub(min_type_off)
        };
        let type_base = idx
            .strings_off
            .saturating_add(idx.string_len.saturating_sub(types_len));
        let idx = Index { type_base, ..idx };
        Ok(idx)
    }

    #[inline]
    pub fn string(&self, off: u32, len: u32) -> &str {
        if off == NONE {
            return "";
        }
        let base = self.strings_off as usize;
        let a = base.saturating_add(off as usize);
        let b = a.saturating_add(len as usize).min(self.data.len());
        if b > self.data.len() || a > b || a < base {
            return "";
        }
        std::str::from_utf8(&self.data[a..b]).unwrap_or("")
    }

    /// Resolve an event `source_type`/`tag` offset, which is relative to the
    /// type-string region rather than to the path/session strings.
    #[inline]
    fn type_string(&self, off: u32, len: u32) -> &str {
        if off == NONE {
            return "";
        }
        let base = self.type_base as usize;
        let a = base.saturating_add(off as usize);
        let b = a.saturating_add(len as usize).min(self.data.len());
        if b > self.data.len() || a > b || a < base {
            return "";
        }
        std::str::from_utf8(&self.data[a..b]).unwrap_or("")
    }

    #[inline]
    fn file_rec(&self, id: FileId) -> Result<&[u8]> {
        if id >= self.file_count {
            return Err(err(format!("unknown file id {id}")));
        }
        let a = self.files_off as usize + id as usize * FILE_LEN;
        Ok(&self.data[a..a + FILE_LEN])
    }

    #[inline]
    fn event_rec(&self, idx: u32) -> Result<&[u8]> {
        if idx >= self.event_count {
            return Err(err(format!("unknown event idx {idx}")));
        }
        let a = self.events_off as usize + idx as usize * EV_LEN;
        Ok(&self.data[a..a + EV_LEN])
    }

    #[inline]
    fn session_rec(&self, idx: u32) -> Result<&[u8]> {
        if idx >= self.session_count {
            return Err(err(format!("unknown session idx {idx}")));
        }
        let a = self.sessions_off as usize + idx as usize * SESSION_LEN;
        Ok(&self.data[a..a + SESSION_LEN])
    }

    pub fn file(&self, id: FileId) -> Result<FileView<'_>> {
        Ok(FileView { id, rec: self.file_rec(id)?, index: self })
    }

    pub fn event(&self, idx: u32) -> Result<EventView<'_>> {
        Ok(EventView { idx, rec: self.event_rec(idx)?, index: self })
    }

    pub fn session(&self, idx: u32) -> Result<SessionView<'_>> {
        Ok(SessionView { idx, rec: self.session_rec(idx)?, index: self })
    }

    /// Exact lookup by source coordinate. Spans are unique per line in P0a.
    pub fn event_at(&self, span: SpanRef) -> Result<Option<u32>> {
        let f = self.file(span.file)?;
        let mut lo = f.event_start() as usize;
        let mut hi = f.event_end() as usize;
        while lo < hi {
            let mid = (lo + hi) / 2;
            let off = get_u64(self.event_rec(mid as u32)?, E_OFF);
            if off < span.off {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let end = f.event_end() as usize;
        let mut pos = lo;
        while pos < end {
            let rec = self.event_rec(pos as u32)?;
            let off = get_u64(rec, E_OFF);
            if off != span.off {
                break;
            }
            if get_u32(rec, E_LEN) == span.len {
                return Ok(Some(pos as u32));
            }
            pos += 1;
        }
        Ok(None)
    }

    pub fn session_by_id(&self, id: &str) -> Option<u32> {
        let mut lo = 0usize;
        let mut hi = self.session_count as usize;
        while lo < hi {
            let mid = (lo + hi) / 2;
            let rec = self.session_rec(mid as u32).ok()?;
            let sid = self.string(get_u32(rec, S_ID_OFF), get_u32(rec, S_ID_LEN));
            match sid.cmp(id) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(mid as u32),
            }
        }
        None
    }

    pub fn sessions_by_parent(&self, parent: u32) -> Vec<u32> {
        let mut out = Vec::new();
        for i in 0..self.session_count {
            if let Ok(rec) = self.session_rec(i) {
                if get_u32(rec, S_PARENT_IDX) == parent {
                    out.push(i);
                }
            }
        }
        out
    }

    /// Events in a session, filtered by recorded session index. P0a files hold
    /// one session each, but the filter keeps this correct if that changes.
    pub fn session_events(&self, idx: u32) -> Result<Vec<u32>> {
        let s = self.session(idx)?;
        let mut out = Vec::new();
        for i in s.first()..s.last() {
            let e = self.event(i)?;
            if e.session() == idx {
                out.push(i);
            }
        }
        Ok(out)
    }
}

pub struct FileView<'a> {
    pub id: FileId,
    rec: &'a [u8],
    index: &'a Index,
}

impl FileView<'_> {
    pub fn path(&self) -> &str {
        self.index.string(get_u32(self.rec, F_PATH_OFF), get_u32(self.rec, F_PATH_LEN))
    }
    pub fn size(&self) -> u64 {
        get_u64(self.rec, F_SIZE)
    }
    pub fn mtime(&self) -> (i64, u32) {
        (get_i64(self.rec, F_MTIME_S), get_u32(self.rec, F_MTIME_N))
    }
    pub fn event_start(&self) -> u32 {
        get_u32(self.rec, F_EVT_START)
    }
    pub fn event_end(&self) -> u32 {
        get_u32(self.rec, F_EVT_END)
    }
    pub fn provider(&self) -> Provider {
        match self.rec[F_PROVIDER] {
            1 => Provider::Codex,
            _ => Provider::Omp,
        }
    }
    pub fn encoding(&self) -> Encoding {
        match self.rec[F_ENCODING] {
            1 => Encoding::JsonlZstd,
            _ => Encoding::Jsonl,
        }
    }
}

pub struct EventView<'a> {
    pub idx: u32,
    rec: &'a [u8],
    index: &'a Index,
}

impl EventView<'_> {
    pub fn span(&self) -> SpanRef {
        SpanRef {
            file: get_u32(self.rec, E_FILE),
            off: get_u64(self.rec, E_OFF),
            len: get_u32(self.rec, E_LEN),
        }
    }
    pub fn hash(&self) -> u64 {
        get_u64(self.rec, E_HASH)
    }
    pub fn ts_ms(&self) -> i64 {
        get_i64(self.rec, E_TS)
    }
    pub fn ordinal(&self) -> u32 {
        get_u32(self.rec, E_ORDINAL)
    }
    pub fn parent(&self) -> u32 {
        get_u32(self.rec, E_PARENT)
    }
    pub fn link(&self) -> u32 {
        get_u32(self.rec, E_LINK)
    }
    pub fn ext(&self) -> u32 {
        get_u32(self.rec, E_EXT)
    }
    pub fn session(&self) -> u32 {
        get_u32(self.rec, E_SESSION)
    }
    pub fn kind(&self) -> EventKind {
        kind_from(self.rec[E_KIND])
    }
    pub fn role(&self) -> Role {
        role_from(self.rec[E_ROLE])
    }
    pub fn source_type(&self) -> &str {
        self.index
            .type_string(get_u32(self.rec, E_TYPE), get_u32(self.rec, E_TYPE_LEN))
    }
    pub fn tag(&self) -> &str {
        self.index
            .type_string(get_u32(self.rec, E_TAG), get_u32(self.rec, E_TAG_LEN))
    }
    pub fn valid_json(&self) -> bool {
        self.rec[E_FLAGS] & 1 == 1
    }
}

pub struct SessionView<'a> {
    pub idx: u32,
    rec: &'a [u8],
    index: &'a Index,
}

impl SessionView<'_> {
    pub fn id(&self) -> &str {
        self.index.string(get_u32(self.rec, S_ID_OFF), get_u32(self.rec, S_ID_LEN))
    }
    pub fn file(&self) -> FileId {
        get_u32(self.rec, S_FILE)
    }
    pub fn first(&self) -> u32 {
        get_u32(self.rec, S_FIRST)
    }
    pub fn last(&self) -> u32 {
        get_u32(self.rec, S_LAST)
    }
    pub fn parent_id(&self) -> &str {
        self.index.string(get_u32(self.rec, S_PARENT_OFF), get_u32(self.rec, S_PARENT_LEN))
    }
    pub fn parent_idx(&self) -> u32 {
        get_u32(self.rec, S_PARENT_IDX)
    }
    pub fn header(&self) -> u32 {
        get_u32(self.rec, S_HEADER)
    }
    pub fn path_derived(&self) -> bool {
        get_u32(self.rec, S_FLAGS) & 1 == 1
    }
}
