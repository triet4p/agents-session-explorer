//! Address span and session views over the binary index.
//!
//! Span parsing, freshness checks and event rendering are shared with the
//! metadata audits; `show` renders one session or span target.

use crate::cli_option::{has, opt, positional};
use crate::index::load;
use ase_core::{err, EventIdx, RawStore, Result, Role, SpanRef, NONE};
use ase_ingest::util::fnv64;
use ase_index::{FsRawStore, Index};
use serde_json::{json, Value};


fn opt_idx(v: u32) -> Option<u32> {
    if v == NONE { None } else { Some(v) }
}

pub(crate) fn span_label(s: SpanRef) -> String {
    format!("span:{}:{}:{}", s.file, s.off, s.len)
}

pub(crate) fn parse_span(s: &str) -> Option<SpanRef> {
    let body = s.strip_prefix("span:").unwrap_or(s);
    let parts: Vec<&str> = body.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    Some(SpanRef {
        file: parts[0].parse().ok()?,
        off: parts[1].parse().ok()?,
        len: parts[2].parse().ok()?,
    })
}

fn parse_range(s: Option<String>) -> Option<(u32, u32)> {
    let s = s?;
    let mut p = s.split(':');
    Some((p.next()?.parse().ok()?, p.next()?.parse().ok()?))
}

pub(crate) fn freshness_json(idx: &Index, file: u32) -> Value {
    let f = match idx.file(file) {
        Ok(f) => f,
        Err(_) => return json!({"status": "unknown"}),
    };
    let indexed_size = f.size() as i64;
    let (indexed_mtime, indexed_ns) = f.mtime();
    let meta = std::fs::metadata(f.path());
    let (status, current_size, current_mtime) = match meta {
        Ok(m) => {
            let cs = m.len() as i64;
            let cm = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let fresh = cs == indexed_size && cm == indexed_mtime;
            (if fresh { "fresh" } else { "stale" }, cs, cm)
        }
        Err(_) => ("unavailable", -1, -1),
    };
    json!({
        "status": status,
        "indexed_size": indexed_size,
        "current_size": current_size,
        "indexed_mtime_secs": indexed_mtime,
        "indexed_mtime_ns": indexed_ns,
        "current_mtime_secs": current_mtime
    })
}

fn event_json(
    idx: &Index,
    store: &FsRawStore,
    ev_idx: EventIdx,
    explain: bool,
    content: bool,
) -> Result<Value> {
    let e = idx.event(ev_idx)?;
    let span = e.span();
    let f = idx.file(span.file)?;
    let mut v = json!({
        "event": ev_idx,
        "span": span_label(span),
        "file": span.file,
        "path": f.path(),
        "provider": f.provider().label(),
        "encoding": f.encoding().label(),
        "coordinate": f.encoding().coordinate(),
        "off": span.off,
        "len": span.len,
        "ordinal": e.ordinal(),
        "kind": e.kind().label(),
        "role": e.role().label(),
        "source_type": e.source_type(),
        "tag": e.tag(),
        "ts_ms": e.ts_ms(),
        "valid_json": e.valid_json(),
        "hash": format!("{:016x}", e.hash()),
        "parent": opt_idx(e.parent()),
        "link": opt_idx(e.link()),
        "session": opt_idx(e.session()),
        "freshness": freshness_json(idx, span.file)
    });
    if explain || content {
        let bytes = store.span(span)?;
        let hash_ok = fnv64(&bytes) == e.hash();
        v["hash_verified"] = json!(hash_ok);
        if explain {
            v["explain"] = json!([
                format!("binary search over file {} event range [{}, {}) using source offset {}",
                    span.file, f.event_start(), f.event_end(), span.off),
                format!("span bytes addressed in {}", f.encoding().coordinate()),
                format!("re-read {} bytes from raw source; FNV-1a checksum match={}", bytes.len(), hash_ok),
                "relations are verbatim source metadata; no inference or model call"
            ]);
        }
        if content {
            v["content"] = json!(String::from_utf8_lossy(&bytes).to_string());
        }
    }
    Ok(v)
}

pub fn run(args: &[String]) -> Result<()> {
    let target = positional(args).ok_or_else(|| err("show needs a target"))?;
    let (idx, store) = load(args)?;
    let explain = has(args, "--explain");
    let content = has(args, "--content");
    let role_filter = opt(args, "--role").and_then(|s| Role::from_label(&s));
    let ordinal = parse_range(opt(args, "--ordinal"));
    let limit: usize = opt(args, "--limit").and_then(|s| s.parse().ok()).unwrap_or(50);

    let mut found: Vec<EventIdx> = Vec::new();
    if let Some(id) = target.strip_prefix("session:") {
        let sidx = idx.session_by_id(id).ok_or_else(|| err(format!("session not indexed: {id}")))?;
        for e in idx.session_events(sidx)? {
            let ev = idx.event(e)?;
            if let Some(r) = role_filter {
                if ev.role() != r {
                    continue;
                }
            }
            if let Some((a, b)) = ordinal {
                let o = ev.ordinal();
                if o < a || o > b {
                    continue;
                }
            }
            found.push(e);
        }
    } else if let Some(span) = parse_span(&target) {
        if let Some(e) = idx.event_at(span)? {
            found.push(e);
        }
    } else {
        return Err(err("target must be span:F:OFF:LEN or session:ID"));
    }
    let truncated = found.len() > limit;
    found.truncate(limit);

    let mut events = Vec::new();
    for e in &found {
        events.push(event_json(&idx, &store, *e, explain, content)?);
    }
    let v = json!({
        "command": "show",
        "target": target,
        "count": found.len(),
        "truncated": truncated,
        "events": events
    });
    if has(args, "--json") {
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        let empty_arr: Vec<Value> = Vec::new();
        for e in &events {
            let f = e["span"].as_str().unwrap_or("");
            println!(
                "{} file={} {}/{} kind={} role={} tag={} ordinal={} freshness={}",
                f,
                e["file"],
                e["provider"].as_str().unwrap_or(""),
                e["encoding"].as_str().unwrap_or(""),
                e["kind"].as_str().unwrap_or(""),
                e["role"].as_str().unwrap_or(""),
                e["tag"].as_str().unwrap_or(""),
                e["ordinal"],
                e["freshness"]["status"].as_str().unwrap_or("")
            );
            if explain {
                for step in e["explain"].as_array().unwrap_or(&empty_arr) {
                    println!("    - {}", step.as_str().unwrap_or(""));
                }
            }
            if content {
                println!("    content: {}", e["content"].as_str().unwrap_or(""));
            }
        }
        println!("show: {} event(s), truncated={}", found.len(), truncated);
    }
    Ok(())
}
