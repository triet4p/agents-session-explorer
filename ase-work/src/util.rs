//! Small deterministic helpers shared by projection, comparison and manifests.

use sha2::{Digest, Sha256};
use std::fmt::Write as _;

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        let _ = write!(out, "{b:02x}");
    }
    out
}

pub fn fnv64_hex(bytes: &[u8]) -> String {
    format!("{:016x}", ase_ingest::util::fnv64(bytes))
}

/// Opaque, path-free journal id. Mirrors the T1 `source_id` derivation
/// (SHA-256 of the normalized relative path, first 16 hex characters) so two
/// independently produced cohorts can be aligned without exposing paths.
pub fn source_id(relpath: &str) -> String {
    sha256_hex(relpath.as_bytes())[..16].to_string()
}

/// Parses a timestamp the same way the T1 reference does: RFC3339 with an
/// optional fractional part and offset, naive values treated as UTC. `None`
/// means absent or unparseable.
pub fn parse_ts_opt(text: Option<&str>) -> Option<i64> {
    let text = text?;
    let b = text.as_bytes();
    if b.len() < 19
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    if !(b[..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..10].iter().all(u8::is_ascii_digit))
    {
        return None;
    }
    let month: u32 = text[5..7].parse().ok()?;
    let day: u32 = text[8..10].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(ase_ingest::util::parse_ts_ms(text))
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp as i64 + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + if m <= 2 { 1 } else { 0 }, m as u32, d as u32)
}

/// Formats UTC milliseconds as `YYYY-MM-DDTHH:MM:SS.mmmZ`, matching the T1
/// reference's `iso()` normalization.
pub fn format_iso_ms(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    let h = rem / 3_600_000;
    let mi = (rem / 60_000) % 60;
    let s = (rem / 1000) % 60;
    let milli = rem % 1000;
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{milli:03}Z")
}

/// Parses the T1 filename shape `YYYY-MM-DDTHH-MM-SS-mmmZ_...`.
pub fn filename_timestamp(path: &str) -> Option<i64> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let b = name.as_bytes();
    if b.len() < 24
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b'-'
        || b[16] != b'-'
        || b[19] != b'-'
        || b[23] != b'Z'
        || b.get(24) != Some(&b'_')
    {
        return None;
    }
    let y: i64 = name[0..4].parse().ok()?;
    let mo: u32 = name[5..7].parse().ok()?;
    let d: u32 = name[8..10].parse().ok()?;
    let h: i64 = name[11..13].parse().ok()?;
    let mi: i64 = name[14..16].parse().ok()?;
    let s: i64 = name[17..19].parse().ok()?;
    let milli: i64 = name[20..23].parse().ok()?;
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    Some((days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s) * 1000 + milli)
}

/// Normalizes a native path to forward slashes for stable relative addressing.
pub fn slash_path(path: &str) -> String {
    path.replace('\\', "/")
}

/// Returns the forward-slash relative path under `root`, or `None` when the
/// file is outside it. Windows drive-letter and separator case are folded.
pub fn relpath(path: &str, root: &str) -> Option<String> {
    let p = slash_path(path);
    let r = slash_path(root).trim_end_matches('/').to_string();
    if p.len() < r.len() {
        return None;
    }
    let head = &p[..r.len()];
    if !head.eq_ignore_ascii_case(&r) {
        return None;
    }
    let rest = &p[r.len()..];
    if !rest.is_empty() && !rest.starts_with('/') {
        return None;
    }
    let rest = rest.trim_start_matches('/');
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub kind: &'static str,
    pub tree_relpath: String,
    pub tree_stem: String,
}

/// Classifies a forward-slash relative journal path using the locked T1 tree
/// layout: `<project>/<name>.jsonl` is a root; `<project>/<name>/...jsonl`
/// belongs to that root's tree.
pub fn layout(relpath: &str) -> Option<Layout> {
    let parts: Vec<&str> = relpath.split('/').collect();
    if parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    if parts.len() == 2 && parts[1].ends_with(".jsonl") {
        let stem = format!("{}/{}", parts[0], &parts[1][..parts[1].len() - 6]);
        return Some(Layout {
            kind: "root",
            tree_relpath: relpath.to_string(),
            tree_stem: stem,
        });
    }
    if parts.len() > 2 && parts.last().is_some_and(|name| name.ends_with(".jsonl")) {
        let stem = format!("{}/{}", parts[0], parts[1]);
        return Some(Layout {
            kind: "subagent",
            tree_relpath: format!("{}.jsonl", stem),
            tree_stem: stem,
        });
    }
    None
}

fn python_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c if (c as u32) < 0x7f => out.push(c),
            c => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
        }
    }
    out.push('"');
    out
}

fn python_json_opt(s: &Option<String>) -> String {
    match s {
        Some(v) => python_json_string(v),
        None => "null".to_string(),
    }
}

/// Canonical JSON for one manifest entry, byte-compatible with the T1
/// reference's `json.dumps(entry, ensure_ascii=True, sort_keys=True,
/// separators=(",", ":"))`.
pub fn canonical_manifest_entry(e: &crate::model::ManifestEntry) -> String {
    format!(
        concat!(
            "{{\"filename_timestamp\":{},\"head_sha256\":{},\"header_ok\":{},",
            "\"header_timestamp\":{},\"kind\":{},\"parent_session\":{},",
            "\"relpath\":{},\"session_id\":{},\"sha256\":{},\"size\":{},",
            "\"tail_sha256\":{},\"tree_relpath\":{}}}"
        ),
        python_json_opt(&e.filename_timestamp),
        python_json_string(&e.head_sha256),
        if e.header_ok { "true" } else { "false" },
        python_json_opt(&e.header_timestamp),
        python_json_string(&e.kind),
        python_json_opt(&e.parent_session),
        python_json_string(&e.relpath),
        python_json_opt(&e.session_id),
        python_json_string(&e.sha256),
        e.size,
        python_json_string(&e.tail_sha256),
        python_json_string(&e.tree_relpath),
    )
}

pub fn manifest_digest(entries: &[crate::model::ManifestEntry]) -> String {
    let mut digest = Sha256::new();
    for entry in entries {
        digest.update(canonical_manifest_entry(entry).as_bytes());
        digest.update(b"\n");
    }
    let mut out = String::with_capacity(64);
    for b in digest.finalize() {
        let _ = write!(out, "{b:02x}");
    }
    out
}
