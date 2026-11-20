use ase_core::{
    err, DecodeStats, Encoding, FileDecode, Provider, Result, SessionMeta, SourceFile, NONE,
};
use ase_ingest::{decode_codex, decode_omp, read_source_bytes, source::discover_all};
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::format::*;

#[derive(Clone, Debug)]
pub struct BuildStats {
    pub files: usize,
    pub events: usize,
    pub sessions: usize,
    pub index_bytes: u64,
    pub malformed: usize,
    pub unknown_type: usize,
    pub unresolved_parent: usize,
    pub unresolved_link: usize,
    pub multi_call_entries: usize,
    pub omp_files: usize,
    pub codex_files: usize,
    pub zstd_files: usize,
    pub duration_ms: u128,
}

#[derive(Default)]
struct Interner {
    buf: Vec<u8>,
    map: HashMap<String, (u32, u32)>,
}

impl Interner {
    fn intern(&mut self, s: &str) -> (u32, u32) {
        if let Some(&v) = self.map.get(s) {
            return v;
        }
        let off = self.buf.len() as u32;
        self.buf.extend_from_slice(s.as_bytes());
        let v = (off, s.len() as u32);
        self.map.insert(s.to_string(), v);
        v
    }
}

#[derive(Clone)]
struct AssembledSession {
    meta: SessionMeta,
    file_base: u32,
    file_end: u32,
    header_event: u32,
}

/// One decode pass over the exact source bytes, shared by the address writer
/// and the postings writer so the two layers cannot diverge.
#[derive(Clone)]
pub struct DecodedCorpus {
    pub files: Vec<SourceFile>,
    pub decoded: Vec<FileDecode>,
    pub bases: Vec<u32>,
    pub total_events: usize,
    pub stats: DecodeStats,
    pub file_digests: Vec<String>,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub fn decode_corpus(omp_root: &Path, codex_root: &Path) -> Result<DecodedCorpus> {
    let files: Vec<SourceFile> = discover_all(omp_root, codex_root)?;
    let pairs: Vec<(FileDecode, String)> = files
        .par_iter()
        .map(|f| {
            let bytes = read_source_bytes(&f.path, f.encoding)?;
            let digest = sha256_hex(&bytes);
            let decoded = match f.provider {
                Provider::Omp => decode_omp(f, &bytes),
                Provider::Codex => decode_codex(f, &bytes),
            }?;
            Ok((decoded, digest))
        })
        .collect::<Result<Vec<_>>>()?;
    let total_events: usize = pairs.iter().map(|(d, _)| d.events.len()).sum();
    let mut stats = DecodeStats::default();
    let mut decoded = Vec::with_capacity(pairs.len());
    let mut file_digests = Vec::with_capacity(pairs.len());
    for (d, digest) in pairs {
        stats.events += d.stats.events;
        stats.malformed += d.stats.malformed;
        stats.unknown_type += d.stats.unknown_type;
        stats.unresolved_parent += d.stats.unresolved_parent;
        stats.unresolved_link += d.stats.unresolved_link;
        stats.multi_call_entries += d.stats.multi_call_entries;
        stats.sessions += d.stats.sessions;
        decoded.push(d);
        file_digests.push(digest);
    }
    let mut bases: Vec<u32> = Vec::with_capacity(decoded.len());
    let mut running = 0u32;
    for d in &decoded {
        bases.push(running);
        running += d.events.len() as u32;
    }
    Ok(DecodedCorpus { files, decoded, bases, total_events, stats, file_digests })
}

pub fn build(omp_root: &Path, codex_root: &Path, out: &Path) -> Result<BuildStats> {
    let corpus = decode_corpus(omp_root, codex_root)?;
    write_address(&corpus, out)
}

pub fn write_address(corpus: &DecodedCorpus, out: &Path) -> Result<BuildStats> {
    let start = Instant::now();
    let files = &corpus.files;
    let decoded = &corpus.decoded;
    let total_events = corpus.total_events;
    let agg = &corpus.stats;
    let bases = &corpus.bases;

    let mut types = Interner::default();
    let mut strings = Interner::default();
    // Sessions are sorted by id so `session_by_id` is a binary search.
    let mut sessions: Vec<AssembledSession> = Vec::with_capacity(agg.sessions);
    for (idx, d) in decoded.iter().enumerate() {
        let base = bases[idx];
        let end = base + d.events.len() as u32;
        let header_local = d
            .events
            .iter()
            .position(|e| e.source_type == "session" || e.source_type == "session_meta")
            .unwrap_or(0) as u32;
        for meta in &d.sessions {
            sessions.push(AssembledSession {
                meta: meta.clone(),
                file_base: base,
                file_end: end,
                header_event: base + header_local,
            });
        }
    }
    sessions.sort_by(|a, b| a.meta.id.cmp(&b.meta.id));
    let mut session_by_id: HashMap<String, u32> = HashMap::with_capacity(sessions.len());
    for (i, s) in sessions.iter().enumerate() {
        session_by_id.entry(s.meta.id.clone()).or_insert(i as u32);
    }
    let mut session_by_file_local: HashMap<(u32, u32), u32> = HashMap::new();
    for (i, s) in sessions.iter().enumerate() {
        session_by_file_local.insert((s.meta.file, s.meta.local_index), i as u32);
    }

    let mut buf = Vec::with_capacity(HEADER_LEN + total_events * EV_LEN);
    buf.resize(HEADER_LEN, 0);

    // ---- file records ----
    let files_off = buf.len() as u32;
    for (idx, f) in files.iter().enumerate() {
        let (po, pl) = strings.intern(&f.path);
        let mut rec = vec![0u8; FILE_LEN];
        rec[F_SIZE..F_SIZE + 8].copy_from_slice(&f.size.to_le_bytes());
        rec[F_MTIME_S..F_MTIME_S + 8].copy_from_slice(&f.mtime_secs.to_le_bytes());
        rec[F_MTIME_N..F_MTIME_N + 4].copy_from_slice(&f.mtime_nanos.to_le_bytes());
        rec[F_PATH_OFF..F_PATH_OFF + 4].copy_from_slice(&po.to_le_bytes());
        rec[F_PATH_LEN..F_PATH_LEN + 4].copy_from_slice(&pl.to_le_bytes());
        rec[F_EVT_START..F_EVT_START + 4].copy_from_slice(&bases[idx].to_le_bytes());
        rec[F_EVT_END..F_EVT_END + 4]
            .copy_from_slice(&(bases[idx] + decoded[idx].events.len() as u32).to_le_bytes());
        rec[F_SES_START..F_SES_START + 4]
            .copy_from_slice(&(decoded[idx].sessions.len() as u32).to_le_bytes());
        rec[F_PROVIDER] = match f.provider { Provider::Omp => 0, Provider::Codex => 1 };
        rec[F_ENCODING] = match f.encoding {
            Encoding::Jsonl => 0,
            Encoding::JsonlZstd => 1,
        };
        buf.extend_from_slice(&rec);
    }
    // ---- event records ----
    let events_off = buf.len() as u32;
    for (idx, d) in decoded.iter().enumerate() {
        let base = bases[idx];
        for e in &d.events {
            let (to, tl) = types.intern(&e.source_type);
            let (go, gl) = types.intern(&e.tag);
            let (xo, _xl) = match &e.ext_id {
                Some(s) => strings.intern(s),
                None => (NONE, 0),
            };
            let parent = if e.parent_idx == NONE { NONE } else { base + e.parent_idx };
            let link = if e.link_idx == NONE { NONE } else { base + e.link_idx };
            let session = session_by_file_local
                .get(&(e.span.file, e.session_local))
                .copied()
                .unwrap_or(NONE);
            let mut rec = vec![0u8; EV_LEN];
            rec[E_OFF..E_OFF + 8].copy_from_slice(&e.span.off.to_le_bytes());
            rec[E_TYPE_LEN..E_TYPE_LEN + 4].copy_from_slice(&tl.to_le_bytes());
            rec[E_TAG_LEN..E_TAG_LEN + 4].copy_from_slice(&gl.to_le_bytes());
            rec[E_HASH..E_HASH + 8].copy_from_slice(&e.hash.to_le_bytes());
            rec[E_TS..E_TS + 8].copy_from_slice(&e.ts_ms.to_le_bytes());
            rec[E_FILE..E_FILE + 4].copy_from_slice(&e.span.file.to_le_bytes());
            rec[E_ORDINAL..E_ORDINAL + 4].copy_from_slice(&e.ordinal.to_le_bytes());
            rec[E_LEN..E_LEN + 4].copy_from_slice(&e.span.len.to_le_bytes());
            rec[E_SESSION..E_SESSION + 4].copy_from_slice(&session.to_le_bytes());
            rec[E_PARENT..E_PARENT + 4].copy_from_slice(&parent.to_le_bytes());
            rec[E_LINK..E_LINK + 4].copy_from_slice(&link.to_le_bytes());
            rec[E_EXT..E_EXT + 4].copy_from_slice(&xo.to_le_bytes());
            rec[E_TYPE..E_TYPE + 4].copy_from_slice(&to.to_le_bytes());
            rec[E_TAG..E_TAG + 4].copy_from_slice(&go.to_le_bytes());
            rec[E_KIND] = kind_code(e.kind);
            rec[E_ROLE] = role_code(e.role);
            rec[E_FLAGS] = if e.valid_json { 1 } else { 0 };
            buf.extend_from_slice(&rec);
        }
    }
    pad8(&mut buf);

    // ---- session records ----
    let sessions_off = buf.len() as u32;
    for s in &sessions {
        let (io, il) = strings.intern(&s.meta.id);
        let (po, pl) = match &s.meta.parent_session {
            Some(p) => strings.intern(p),
            None => (NONE, 0),
        };
        let parent_idx = s
            .meta
            .parent_session
            .as_ref()
            .and_then(|p| session_by_id.get(p).copied())
            .unwrap_or(NONE);
        let mut rec = vec![0u8; SESSION_LEN];
        rec[S_ID_OFF..S_ID_OFF + 4].copy_from_slice(&io.to_le_bytes());
        rec[S_ID_LEN..S_ID_LEN + 4].copy_from_slice(&il.to_le_bytes());
        rec[S_FILE..S_FILE + 4].copy_from_slice(&s.meta.file.to_le_bytes());
        rec[S_FIRST..S_FIRST + 4].copy_from_slice(&s.file_base.to_le_bytes());
        rec[S_LAST..S_LAST + 4].copy_from_slice(&s.file_end.to_le_bytes());
        rec[S_PARENT_OFF..S_PARENT_OFF + 4].copy_from_slice(&po.to_le_bytes());
        rec[S_PARENT_LEN..S_PARENT_LEN + 4].copy_from_slice(&pl.to_le_bytes());
        rec[S_PARENT_IDX..S_PARENT_IDX + 4].copy_from_slice(&parent_idx.to_le_bytes());
        rec[S_HEADER..S_HEADER + 4].copy_from_slice(&s.header_event.to_le_bytes());
        rec[S_FLAGS..S_FLAGS + 4]
            .copy_from_slice(&(if s.meta.path_derived { 1u32 } else { 0u32 }).to_le_bytes());
        buf.extend_from_slice(&rec);
    }
    pad8(&mut buf);

    // ---- string blob (paths / session ids / external ids), then type strings ----
    let strings_off = buf.len() as u32;
    buf.extend_from_slice(&strings.buf);
    let type_strings_base = strings.buf.len() as u32;
    buf.extend_from_slice(&types.buf);
    let strings_len = (type_strings_base as usize + types.buf.len()) as u32;

    // ---- type table (offsets are relative to the whole string blob) ----
    let types_off = buf.len() as u32;
    let mut entries: Vec<(String, (u32, u32))> =
        types.map.iter().map(|(k, v)| (k.clone(), *v)).collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, (o, l)) in &entries {
        buf.extend_from_slice(&(type_strings_base + *o).to_le_bytes());
        buf.extend_from_slice(&l.to_le_bytes());
    }

    let build_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    buf[0..8].copy_from_slice(MAGIC);
    buf[H_BUILD..H_BUILD + 8].copy_from_slice(&build_ts.to_le_bytes());
    buf[H_FILE_COUNT..H_FILE_COUNT + 4].copy_from_slice(&(files.len() as u32).to_le_bytes());
    buf[H_EVENT_COUNT..H_EVENT_COUNT + 4].copy_from_slice(&(total_events as u32).to_le_bytes());
    buf[H_SESSION_COUNT..H_SESSION_COUNT + 4].copy_from_slice(&(sessions.len() as u32).to_le_bytes());
    buf[H_STRING_LEN..H_STRING_LEN + 4].copy_from_slice(&strings_len.to_le_bytes());
    buf[H_FILES_OFF..H_FILES_OFF + 4].copy_from_slice(&files_off.to_le_bytes());
    buf[H_EVENTS_OFF..H_EVENTS_OFF + 4].copy_from_slice(&events_off.to_le_bytes());
    buf[H_SESSIONS_OFF..H_SESSIONS_OFF + 4].copy_from_slice(&sessions_off.to_le_bytes());
    buf[H_STRINGS_OFF..H_STRINGS_OFF + 4].copy_from_slice(&strings_off.to_le_bytes());
    buf[H_TYPE_COUNT..H_TYPE_COUNT + 4].copy_from_slice(&(entries.len() as u32).to_le_bytes());
    buf[H_TYPES_OFF..H_TYPES_OFF + 4].copy_from_slice(&types_off.to_le_bytes());

    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|e| err(format!("create {}: {e}", parent.display())))?;
    }
    fs::write(out, &buf).map_err(|e| err(format!("write {}: {e}", out.display())))?;

    let omp_files = files.iter().filter(|f| f.provider == Provider::Omp).count();
    let codex_files = files.len() - omp_files;
    let zstd_files = files
        .iter()
        .filter(|f| f.encoding == Encoding::JsonlZstd)
        .count();
    Ok(BuildStats {
        files: files.len(),
        events: total_events,
        sessions: sessions.len(),
        index_bytes: buf.len() as u64,
        malformed: agg.malformed,
        unknown_type: agg.unknown_type,
        unresolved_parent: agg.unresolved_parent,
        unresolved_link: agg.unresolved_link,
        multi_call_entries: agg.multi_call_entries,
        omp_files,
        codex_files,
        zstd_files,
        duration_ms: start.elapsed().as_millis(),
    })
}
