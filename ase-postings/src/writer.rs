//! Build a contentless FTS5 table and a CallSig shingle table in one staged
//! SQLite file. No prose is stored in any column: FTS keeps only tokens and
//! posting lists, and block records keep only metadata plus a short digest.

use ase_core::{
    err, ProjectionKind, Result, ALLOWLIST_VERSION, CALLSIG_VERSION, NONE,
    PROJECTION_VERSION, SHINGLE_VERSION, TOKENIZER_VERSION,
};
use ase_index::DecodedCorpus;
use rusqlite::{params, Connection};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Clone)]
pub struct WriterInput<'a> {
    pub corpus: &'a DecodedCorpus,
    pub generation: &'a str,
    pub core_digest: &'a str,
    pub address_bytes: u64,
    pub address_sha256: &'a str,
}

#[derive(Clone, Debug, Default)]
pub struct WriterStats {
    pub db_bytes: u64,
    pub blocks: u64,
    pub prose_blocks: u64,
    pub call_blocks: u64,
    pub fts_rows: u64,
    pub shingles: u64,
    pub abstentions: u64,
}

fn hash_hex(hash: &[u8; 8]) -> String {
    let mut s = String::with_capacity(16);
    for b in hash {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn schema(db: &Connection) -> Result<()> {
    db.execute_batch(
        "PRAGMA journal_mode=OFF;
         PRAGMA synchronous=OFF;
         PRAGMA temp_store=MEMORY;
         PRAGMA page_size=4096;
         BEGIN;
         CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE blocks (
             rowid INTEGER PRIMARY KEY,
             generation TEXT NOT NULL,
             event_idx INTEGER NOT NULL,
             file INTEGER NOT NULL,
             span_off INTEGER NOT NULL,
             span_len INTEGER NOT NULL,
             selector TEXT NOT NULL,
             kind TEXT NOT NULL,
             role TEXT NOT NULL,
             session_local INTEGER NOT NULL,
             parent_idx INTEGER NOT NULL,
             block_hash TEXT NOT NULL
         );
         CREATE INDEX blocks_generation_event_selector
             ON blocks(generation, event_idx, selector);
         CREATE VIRTUAL TABLE fts_prose USING fts5(
             prose, content='', tokenize='unicode61 remove_diacritics 0'
         );
         CREATE TABLE shingles (
             k INTEGER NOT NULL,
             token TEXT NOT NULL,
             generation TEXT NOT NULL,
             event_idx INTEGER NOT NULL,
             end_event INTEGER NOT NULL,
             file INTEGER NOT NULL,
             session_local INTEGER NOT NULL,
             parent_idx INTEGER NOT NULL,
             selector_from TEXT NOT NULL,
             selector_to TEXT NOT NULL,
             constituents TEXT NOT NULL
         );
         CREATE INDEX shingles_token ON shingles(generation, token);
         CREATE INDEX shingles_event ON shingles(generation, event_idx, end_event);
         COMMIT;",
    )
    .map_err(|e| err(format!("postings schema: {e}")))
}

#[derive(Clone, Debug)]
struct CallItem {
    event: u32,
    hash: String,
    selector: String,
}

/// Emit k=3..5 windows from one linear call sequence. Every row records the
/// constituent event/selector pairs so each item can be reopened independently.
fn flush_sequence(
    db: &Connection,
    generation: &str,
    calls: &[CallItem],
    file: u32,
    session_local: u32,
    parent_idx: u32,
    stats: &mut WriterStats,
) -> Result<()> {
    if calls.len() < 3 {
        return Ok(());
    }
    let mut stmt = db
        .prepare(
            "INSERT INTO shingles(k,token,generation,event_idx,end_event,file,session_local,parent_idx,selector_from,selector_to,constituents)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        )
        .map_err(|e| err(format!("shingle prepare: {e}")))?;
    for k in 3..=5 {
        for window in calls.windows(k) {
            let token = window.iter().map(|c| c.hash.as_str()).collect::<Vec<_>>().join("-");
            let constituents: Vec<serde_json::Value> = window
                .iter()
                .map(|c| serde_json::json!({"event": c.event, "selector": c.selector}))
                .collect();
            stmt.execute(params![
                k as i64,
                token,
                generation,
                window.first().map(|c| c.event).unwrap_or(0) as i64,
                window.last().map(|c| c.event).unwrap_or(0) as i64,
                file as i64,
                session_local as i64,
                parent_idx as i64,
                window.first().map(|c| c.selector.as_str()).unwrap_or(""),
                window.last().map(|c| c.selector.as_str()).unwrap_or(""),
                serde_json::to_string(&constituents)
                    .map_err(|e| err(format!("constituents: {e}")))?,
            ])
            .map_err(|e| err(format!("shingle insert: {e}")))?;
            stats.shingles += 1;
        }
    }
    Ok(())
}

pub fn write_postings(input: WriterInput<'_>, out: &Path) -> Result<WriterStats> {
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|e| err(format!("create {}: {e}", parent.display())))?;
    }
    let db = Connection::open(out).map_err(|e| err(format!("create db: {e}")))?;
    schema(&db)?;
    let mut stats = WriterStats::default();
    let tx = db.unchecked_transaction().map_err(|e| err(format!("transaction: {e}")))?;
    let mut insert_block = tx
        .prepare(
            "INSERT INTO blocks(rowid,generation,event_idx,file,span_off,span_len,selector,kind,role,session_local,parent_idx,block_hash)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        )
        .map_err(|e| err(format!("block prepare: {e}")))?;
    let mut insert_fts = tx
        .prepare("INSERT INTO fts_prose(rowid, prose) VALUES (?1, ?2)")
        .map_err(|e| err(format!("fts prepare: {e}")))?;

    let mut rowid = 0i64;
    for (file_idx, decoded) in input.corpus.decoded.iter().enumerate() {
        let base = input.corpus.bases[file_idx];
        let mut calls_by_event: Vec<Vec<CallItem>> = vec![Vec::new(); decoded.events.len()];
        for projection in &decoded.projections {
            let local = projection.local_event as usize;
            let event = decoded
                .events
                .get(local)
                .ok_or_else(|| err("projection has no matching address event"))?;
            if event.span != projection.span {
                return Err(err("projection span differs from address span"));
            }
            let event_idx = base + projection.local_event;
            rowid += 1;
            let parent = if projection.parent_idx == NONE {
                NONE
            } else {
                base + projection.parent_idx
            };
            insert_block
                .execute(params![
                    rowid,
                    input.generation,
                    event_idx as i64,
                    event.span.file as i64,
                    event.span.off as i64,
                    event.span.len as i64,
                    projection.selector,
                    projection.kind.label(),
                    projection.role.label(),
                    projection.session_local as i64,
                    parent as i64,
                    hash_hex(&projection.block_hash),
                ])
                .map_err(|e| err(format!("block insert: {e}")))?;
            stats.blocks += 1;
            match projection.kind {
                ProjectionKind::Prose => {
                    insert_fts
                        .execute(params![rowid, projection.prose])
                        .map_err(|e| err(format!("fts insert: {e}")))?;
                    stats.prose_blocks += 1;
                    stats.fts_rows += 1;
                }
                ProjectionKind::Call => {
                    calls_by_event[local].push(CallItem {
                        event: event_idx,
                        hash: hash_hex(&projection.block_hash),
                        selector: projection.selector.clone(),
                    });
                    stats.call_blocks += 1;
                }
            }
        }

        // Continue a sequence only along an explicit same-session parent
        // chain. Siblings of a fork, roots and unresolved parents start a new
        // sequence; no window can cross them.
        let mut sequence: Vec<CallItem> = Vec::new();
        let mut last_event = NONE;
        let mut sequence_session = NONE;
        let mut sequence_file = input.corpus.files[file_idx].id;
        let mut sequence_parent = NONE;
        let file_end = base + decoded.events.len() as u32;
        for (local, event) in decoded.events.iter().enumerate() {
            let calls = &calls_by_event[local];
            if calls.is_empty() {
                continue;
            }
            let event_idx = base + local as u32;
            let parent = if event.parent_idx == NONE {
                NONE
            } else {
                base + event.parent_idx
            };
            if !sequence.is_empty() {
                let continues = parent != NONE
                    && parent == last_event
                    && event.session_local == sequence_session
                    && event.span.file == sequence_file;
                if !continues {
                    flush_sequence(
                        &tx,
                        input.generation,
                        &sequence,
                        sequence_file,
                        sequence_session,
                        sequence_parent,
                        &mut stats,
                    )?;
                    sequence.clear();
                    if parent == NONE
                        || parent < base
                        || parent >= file_end
                        || event.session_local != sequence_session
                    {
                        stats.abstentions += 1;
                    }
                }
            }
            if sequence.is_empty() {
                sequence_file = event.span.file;
                sequence_session = event.session_local;
                sequence_parent = parent;
            }
            sequence.extend(calls.iter().cloned());
            last_event = event_idx;
        }
        if !sequence.is_empty() {
            flush_sequence(
                &tx,
                input.generation,
                &sequence,
                sequence_file,
                sequence_session,
                sequence_parent,
                &mut stats,
            )?;
        }
    }
    drop(insert_block);
    drop(insert_fts);

    let mut versions: BTreeMap<String, String> = BTreeMap::new();
    versions.insert("projection".to_string(), PROJECTION_VERSION.to_string());
    versions.insert("callsig".to_string(), CALLSIG_VERSION.to_string());
    versions.insert("allowlist".to_string(), ALLOWLIST_VERSION.to_string());
    versions.insert("tokenizer".to_string(), TOKENIZER_VERSION.to_string());
    versions.insert("shingle".to_string(), SHINGLE_VERSION.to_string());
    let mut meta = tx
        .prepare("INSERT INTO meta(key,value) VALUES (?1,?2)")
        .map_err(|e| err(format!("meta prepare: {e}")))?;
    meta.execute(params!["generation", input.generation])
        .map_err(|e| err(format!("meta insert: {e}")))?;
    meta.execute(params!["manifest_core_digest", input.core_digest])
        .map_err(|e| err(format!("meta insert: {e}")))?;
    meta.execute(params!["address_sha256", input.address_sha256])
        .map_err(|e| err(format!("meta insert: {e}")))?;
    meta.execute(params!["address_bytes", input.address_bytes.to_string()])
        .map_err(|e| err(format!("meta insert: {e}")))?;
    meta.execute(params!["shingle_abstentions", stats.abstentions.to_string()])
        .map_err(|e| err(format!("meta insert: {e}")))?;
    for (k, v) in &versions {
        meta.execute(params![format!("version_{k}"), v])
            .map_err(|e| err(format!("meta insert: {e}")))?;
    }
    drop(meta);
    tx.commit().map_err(|e| err(format!("commit: {e}")))?;
    stats.db_bytes = fs::metadata(out).map_err(|e| err(format!("stat db: {e}")))?.len();
    Ok(stats)
}
