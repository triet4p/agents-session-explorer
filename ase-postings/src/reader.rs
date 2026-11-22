//! Read-only query layer. Search returns metadata and an address event plus a
//! versioned selector; prose and snippets are never selected from storage.

use ase_core::{err, Encoding, Result, SpanRef};
use ase_index::Index;
use rusqlite::{params, Connection, OpenFlags};

use crate::publication::Publication;

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub rowid: i64,
    pub event: u32,
    pub selector: String,
    pub role: String,
    pub session_local: u32,
    pub parent_idx: u32,
    pub block_hash: String,
    pub score: f64,
    pub span: SpanRef,
    pub encoding: Encoding,
}

#[derive(Clone, Debug)]
pub struct BlockRecord {
    pub rowid: i64,
    pub event: u32,
    pub selector: String,
    pub kind: String,
    pub role: String,
    pub span: SpanRef,
    pub session_local: u32,
    pub parent_idx: u32,
    pub block_hash: String,
}

#[derive(Clone, Debug)]
pub struct ShingleRow {
    pub k: u32,
    pub token: String,
    pub event: u32,
    pub end_event: u32,
    pub file: u32,
    pub session_local: u32,
    pub parent_idx: u32,
    pub selector_from: String,
    pub selector_to: String,
    pub constituents: String,
}

fn db(published: &Publication) -> Result<Connection> {
    Connection::open_with_flags(&published.postings_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| err(format!("open postings: {e}")))
}

/// Build an FTS MATCH expression from user text. Each whitespace-delimited
/// term is quoted as a literal FTS phrase so FTS5 operator syntax cannot be
/// injected; the same unicode61 tokenizer is used at index and query time.
pub fn match_expression(query: &str, phrase: bool) -> Result<String> {
    let q = query.trim();
    if q.is_empty() {
        return Err(err("empty query"));
    }
    if phrase {
        return Ok(format!("\"{}\"", q.replace('"', "\"\"")));
    }
    let terms: Vec<String> = q
        .split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() {
        return Err(err("empty query"));
    }
    Ok(terms.join(" AND "))
}

pub fn search(
    published: &Publication,
    index: &Index,
    query: &str,
    phrase: bool,
    top_k: u32,
) -> Result<Vec<SearchHit>> {
    let expression = match_expression(query, phrase)?;
    let conn = db(published)?;
    let mut stmt = conn
        .prepare(
            "SELECT b.rowid, b.event_idx, b.selector, b.role, b.session_local,
                    b.parent_idx, b.block_hash, bm25(fts_prose) AS score,
                    b.file, b.span_off, b.span_len
             FROM fts_prose JOIN blocks b ON b.rowid = fts_prose.rowid
             WHERE fts_prose MATCH ?1 AND b.generation = ?2 AND b.kind = 'prose'
             ORDER BY score, b.rowid LIMIT ?3",
        )
        .map_err(|e| err(format!("search prepare: {e}")))?;
    let rows = stmt
        .query_map(
            params![expression, published.generation, top_k as i64],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, u32>(4)?,
                    r.get::<_, u32>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, f64>(7)?,
                    r.get::<_, u32>(8)?,
                    r.get::<_, i64>(9)? as u64,
                    r.get::<_, u32>(10)?,
                ))
            },
        )
        .map_err(|e| err(format!("search: {e}")))?;
    let mut out = Vec::new();
    for row in rows {
        let (rowid, event, selector, role, session_local, parent_idx, block_hash, score, file, off, len) =
            row.map_err(|e| err(format!("search row: {e}")))?;
        if event >= index.event_count {
            return Err(err("postings event index outside pinned address"));
        }
        let ev = index.event(event)?;
        let span = ev.span();
        if span.file != file || span.off != off || span.len != len {
            return Err(err("postings span differs from pinned address"));
        }
        out.push(SearchHit {
            rowid,
            event,
            selector,
            role,
            session_local,
            parent_idx,
            block_hash,
            score,
            span,
            encoding: index.file(span.file)?.encoding(),
        });
    }
    Ok(out)
}

pub fn block_by_selector(
    published: &Publication,
    event: u32,
    selector: &str,
) -> Result<Option<BlockRecord>> {
    let conn = db(published)?;
    let mut stmt = conn
        .prepare(
            "SELECT rowid, event_idx, selector, kind, role, file, span_off, span_len,
                    session_local, parent_idx, block_hash
             FROM blocks WHERE generation=?1 AND event_idx=?2 AND selector=?3",
        )
        .map_err(|e| err(format!("block prepare: {e}")))?;
    let mut rows = stmt
        .query_map(
            params![published.generation, event as i64, selector],
            |r| {
                Ok(BlockRecord {
                    rowid: r.get(0)?,
                    event: r.get(1)?,
                    selector: r.get(2)?,
                    kind: r.get(3)?,
                    role: r.get(4)?,
                    span: SpanRef {
                        file: r.get(5)?,
                        off: r.get::<_, i64>(6)? as u64,
                        len: r.get(7)?,
                    },
                    session_local: r.get(8)?,
                    parent_idx: r.get(9)?,
                    block_hash: r.get(10)?,
                })
            },
        )
        .map_err(|e| err(format!("block query: {e}")))?;
    match rows.next() {
        Some(row) => row.map(Some).map_err(|e| err(format!("block row: {e}"))),
        None => Ok(None),
    }
}

/// All prose blocks pinned to one generation, in a stable address order.
/// Only metadata is returned; prose must be re-derived from raw bytes by the
/// caller through the versioned selector.
pub fn all_prose_blocks(published: &Publication) -> Result<Vec<BlockRecord>> {
    let conn = db(published)?;
    let mut stmt = conn
        .prepare(
            "SELECT rowid, event_idx, selector, kind, role, file, span_off, span_len,
                    session_local, parent_idx, block_hash
             FROM blocks WHERE generation=?1 AND kind='prose'
             ORDER BY event_idx, selector",
        )
        .map_err(|e| err(format!("prose blocks prepare: {e}")))?;
    let rows = stmt
        .query_map(params![published.generation], |r| {
            Ok(BlockRecord {
                rowid: r.get(0)?,
                event: r.get(1)?,
                selector: r.get(2)?,
                kind: r.get(3)?,
                role: r.get(4)?,
                span: SpanRef {
                    file: r.get(5)?,
                    off: r.get::<_, i64>(6)? as u64,
                    len: r.get(7)?,
                },
                session_local: r.get(8)?,
                parent_idx: r.get(9)?,
                block_hash: r.get(10)?,
            })
        })
        .map_err(|e| err(format!("prose blocks query: {e}")))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| err(format!("prose block row: {e}")))?);
    }
    Ok(out)
}

pub fn shingles_by_token(published: &Publication, token: &str) -> Result<Vec<ShingleRow>> {
    let conn = db(published)?;
    let mut stmt = conn
        .prepare(
            "SELECT k, token, event_idx, end_event, file, session_local, parent_idx,
                    selector_from, selector_to, constituents
             FROM shingles WHERE generation=?1 AND token=?2
             ORDER BY event_idx, end_event, k",
        )
        .map_err(|e| err(format!("shingle prepare: {e}")))?;
    let rows = stmt
        .query_map(params![published.generation, token], |r| {
            Ok(ShingleRow {
                k: r.get(0)?,
                token: r.get(1)?,
                event: r.get(2)?,
                end_event: r.get(3)?,
                file: r.get(4)?,
                session_local: r.get(5)?,
                parent_idx: r.get(6)?,
                selector_from: r.get(7)?,
                selector_to: r.get(8)?,
                constituents: r.get(9)?,
            })
        })
        .map_err(|e| err(format!("shingle query: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| err(format!("shingle row: {e}")))
}

pub fn shingles_by_event(published: &Publication, event: u32) -> Result<Vec<ShingleRow>> {
    let conn = db(published)?;
    let mut stmt = conn
        .prepare(
            "SELECT k, token, event_idx, end_event, file, session_local, parent_idx,
                    selector_from, selector_to, constituents
             FROM shingles WHERE generation=?1 AND event_idx<=?2 AND end_event>=?3
             ORDER BY k, event_idx",
        )
        .map_err(|e| err(format!("shingle prepare: {e}")))?;
    let rows = stmt
        .query_map(
            params![published.generation, event as i64, event as i64],
            |r| {
                Ok(ShingleRow {
                    k: r.get(0)?,
                    token: r.get(1)?,
                    event: r.get(2)?,
                    end_event: r.get(3)?,
                    file: r.get(4)?,
                    session_local: r.get(5)?,
                    parent_idx: r.get(6)?,
                    selector_from: r.get(7)?,
                    selector_to: r.get(8)?,
                    constituents: r.get(9)?,
                })
            },
        )
        .map_err(|e| err(format!("shingle query: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| err(format!("shingle row: {e}")))
}
