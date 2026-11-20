use ase_core::{DecodeStats, DecodedEvent, EventKind, SessionMeta, NONE};
use std::collections::HashMap;

/// FNV-1a 64. Deliberately non-cryptographic: this is a consistency checksum
/// for span re-verification, not a content identity or version key.
pub fn fnv64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Minimal RFC3339 parser (no chrono dependency). Returns 0 when unparseable;
/// a zero timestamp is never used for ordering in P0a.
pub fn parse_ts_ms(s: &str) -> i64 {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return 0;
    }
    let num = |a: usize, z: usize| -> Option<i64> {
        s.get(a..z)?.parse::<i64>().ok()
    };
    let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(se)) =
        (num(0, 4), num(5, 7), num(8, 10), num(11, 13), num(14, 16), num(17, 19))
    else {
        return 0;
    };
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return 0;
    }
    let mut millis = 0i64;
    let mut pos = 19usize;
    if b.get(pos) == Some(&b'.') {
        pos += 1;
        let start = pos;
        while pos < b.len() && b[pos].is_ascii_digit() && pos - start < 3 {
            millis = millis * 10 + (b[pos] - b'0') as i64;
            pos += 1;
        }
        while pos < b.len() && b[pos].is_ascii_digit() {
            pos += 1;
        }
        for _ in 0..(3 - (pos - start).min(3)) {
            millis *= 10;
        }
    }
    let mut offset = 0i64;
    match b.get(pos) {
        Some(&b'Z') | Some(&b'z') => {}
        Some(&sign @ (b'+' | b'-')) => {
            if pos + 6 > b.len() || b[pos + 3] != b':' {
                return 0;
            }
            let oh = num(pos + 1, pos + 3).unwrap_or(0);
            let om = num(pos + 4, pos + 6).unwrap_or(0);
            offset = (oh * 3600 + om * 60) * if sign == b'+' { 1 } else { -1 };
        }
        _ => return 0,
    }
    let days = days_from_civil(y, mo, d);
    (days * 86_400 + h * 3600 + mi * 60 + se - offset) * 1000 + millis
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Resolve source `parentId` and tool-call links inside one file.
/// Cross-file references stay unresolved and are recorded as external ids.
pub fn link_events(events: &mut [DecodedEvent], stats: &mut DecodeStats) {
    let mut by_id: HashMap<String, u32> = HashMap::with_capacity(events.len());
    for (i, e) in events.iter().enumerate() {
        if let Some(id) = &e.source_id {
            by_id.insert(id.clone(), i as u32);
        }
    }
    for e in events.iter_mut() {
        if let Some(pid) = e.parent_id.clone() {
            if let Some(&idx) = by_id.get(&pid) {
                e.parent_idx = idx;
                e.parent_id = None;
            } else {
                stats.unresolved_parent += 1;
                if e.ext_id.is_none() {
                    e.ext_id = Some(pid);
                }
            }
        }
        e.source_id = None;
    }

    let mut by_call: HashMap<String, u32> = HashMap::new();
    for (i, e) in events.iter().enumerate() {
        if matches!(e.kind, EventKind::AssistantText | EventKind::ToolCall) {
            if let Some(cid) = &e.call_id {
                by_call.entry(cid.clone()).or_insert(i as u32);
            }
        }
    }
    let mut pairs = Vec::new();
    for (i, e) in events.iter().enumerate() {
        if e.kind == EventKind::ToolResult {
            if let Some(cid) = &e.call_id {
                if let Some(&call) = by_call.get(cid) {
                    pairs.push((call, i as u32));
                } else {
                    stats.unresolved_link += 1;
                }
            }
        }
    }
    for (call, result) in pairs {
        events[call as usize].link_idx = result;
        events[result as usize].link_idx = call;
    }
    for e in events.iter_mut() {
        if e.link_idx == NONE && e.ext_id.is_none() {
            if let Some(cid) = &e.call_id {
                if e.kind == EventKind::ToolResult {
                    e.ext_id = Some(cid.clone());
                }
            }
        }
    }
}

/// P0a has one session per source file; attach every event in the file to it.
pub fn assign_sessions(events: &mut [DecodedEvent], sessions: &[SessionMeta]) {
    let local = if sessions.is_empty() { NONE } else { 0 };
    for e in events.iter_mut() {
        e.session_local = local;
    }
}
