//! P4 bundle verification per `artifacts/sprint-7/ASE-S7-T1/contract-a1.md` §6.
//!
//! The bundle is read as data only. Every evidence row is reopened from a
//! private local source map: source bytes are read again, the exact byte span
//! named by the selector is sliced, the versioned block selector is re-run, the
//! strong block digest is recomputed, the paired tool result is resolved again,
//! and the redaction policy is re-applied before the published excerpt is
//! accepted. Nothing here imports a verdict, joins a catalog, calls a model or
//! claims a P4 gate result. `verify --all` lives in `cmd_verify` and is untouched.

use ase_core::{err, Result};
use ase_index::sha256_hex;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

pub(crate) const BUNDLE_VERSION: &str = "ase-p4-bundle-v1";
pub(crate) const MOTIF_SCHEMA: &str = "ase-p4-motif-v1";
pub(crate) const REDACTION_POLICY: &str = "ase-p4-redact-v1";
pub(crate) const EVIDENCE_RENDER: &str = "ase-p4-evidence-md-v1";
pub(crate) const QUESTIONS_RENDER: &str = "ase-p4-questions-md-v1";
pub(crate) const REPRESENTATIVE_POLICY: &str = "ase-p4-representatives-v1";
pub(crate) const SELECTOR_VERSION: &str = "block-v1";
const LOCAL_MAP_SCHEMA: &str = "ase-p4-local-sources-v1";

#[derive(Clone)]
struct Failure {
    code: &'static str,
    detail: String,
}

fn fail(code: &'static str, detail: impl Into<String>) -> Failure {
    Failure {
        code,
        detail: detail.into(),
    }
}

type Check<T> = std::result::Result<T, Failure>;

/// Codes where the source could not be examined at all (as opposed to a
/// source-backed observation that disagreed with the bundle).
fn unverifiable(code: &str) -> bool {
    matches!(code, "alias_unknown" | "source_missing" | "source_unreadable")
}

pub(crate) fn canonical(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}

pub(crate) fn value_digest(value: &Value) -> String {
    sha256_hex(&canonical(value))
}

fn hex64(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn sha_body(value: &str) -> Option<&str> {
    let body = value.strip_prefix("sha256:")?;
    if hex64(body) {
        Some(body)
    } else {
        None
    }
}

fn read_json(path: &Path) -> Check<Value> {
    let bytes = fs::read(path).map_err(|_| fail("source_unreadable", "file not readable"))?;
    serde_json::from_slice(&bytes).map_err(|_| fail("source_unreadable", "file not JSON"))
}

fn str_field<'a>(value: &'a Value, key: &str) -> Check<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| fail("bundle_tampered", format!("missing string field {key}")))
}

fn push_failure(report: &mut Value, ev: Option<&str>, occ: Option<&str>, code: &str, detail: &str) {
    report["failures"].as_array_mut().expect("failures array").push(json!({
        "evidence_id": ev,
        "occurrence_id": occ,
        "code": code,
        "detail": detail,
    }));
}

fn bump(report: &mut Value, key: &str, by: u64) {
    let current = report[key].as_u64().unwrap_or(0);
    report[key] = json!(current + by);
}

fn base_report() -> Value {
    json!({
        "command": "verify",
        "mode": "bundle",
        "scope": "P4 bundle evidence reopen; no gate measurement",
        "status": "unverified",
        "schema_checked": BUNDLE_VERSION,
        "evidence_total": 0,
        "evidence_verified": 0,
        "evidence_failed": 0,
        "evidence_unverified": 0,
        "motifs_total": 0,
        "motifs_verified": 0,
        "assertions_total": 0,
        "assertions_verified": 0,
        "assertions_unverified": 0,
        "assertion_source": null,
        "views_checked": [],
        "watermarks": [],
        "notes": [],
        "failures": []
    })
}

fn count_evidence(bundle_dir: &Path, motifs: &[Value]) -> (u64, u64) {
    let mut evidence = 0u64;
    let mut occurrences = 0u64;
    for row in motifs {
        let Some(rel) = row.get("motif_path").and_then(Value::as_str) else {
            continue;
        };
        if let Ok(motif) = read_json(&bundle_dir.join(rel)) {
            if let Some(reps) = motif.get("representatives").and_then(Value::as_array) {
                occurrences += reps.len() as u64;
                for rep in reps {
                    evidence += rep
                        .get("evidence")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len) as u64;
                }
            }
        }
    }
    (evidence, occurrences)
}

fn top_failure(
    report: &mut Value,
    code: &str,
    detail: &str,
    evidence: u64,
    motifs: u64,
    occurrences: u64,
) -> Value {
    report["evidence_total"] = json!(evidence);
    report["evidence_failed"] = json!(evidence);
    report["motifs_total"] = json!(motifs);
    report["occurrences_total"] = json!(occurrences);
    push_failure(report, None, None, code, detail);
    report["status"] = json!("unverified");
    report.clone()
}

/// CLI entry point. A positional argument selects the P4 bundle form; the
/// legacy `--all` / `--sample` form never reaches this module.
pub fn run_bundle(bundle: &str, args: &[String]) -> Result<()> {
    let local_map = opt_value(args, "--local-map").map(PathBuf::from);
    let verdicts = opt_value(args, "--verdicts").map(PathBuf::from);
    let report = verify(
        Path::new(bundle),
        local_map.as_deref(),
        verdicts.as_deref(),
    );
    let rendered = serde_json::to_string_pretty(&report).map_err(|e| err(format!("verify report: {e}")))?;
    if args.iter().any(|a| a == "--json") {
        println!("{rendered}");
    } else {
        println!(
            "p4 verify: {}; evidence {}/{} ({} failed, {} unverified), motifs {}/{}, assertions {}/{}",
            report["status"].as_str().unwrap_or("unverified"),
            report["evidence_verified"].as_u64().unwrap_or(0),
            report["evidence_total"].as_u64().unwrap_or(0),
            report["evidence_failed"].as_u64().unwrap_or(0),
            report["evidence_unverified"].as_u64().unwrap_or(0),
            report["motifs_verified"].as_u64().unwrap_or(0),
            report["motifs_total"].as_u64().unwrap_or(0),
            report["assertions_verified"].as_u64().unwrap_or(0),
            report["assertions_total"].as_u64().unwrap_or(0),
        );
    }
    if report["status"].as_str() != Some("verified") {
        std::process::exit(3);
    }
    Ok(())
}

fn opt_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

// ---------------------------------------------------------------------------
// Local-only source map

fn load_local_map(path: Option<&Path>) -> Check<(Option<Value>, PathBuf)> {
    let Some(path) = path else {
        return Ok((None, PathBuf::new()));
    };
    let value = read_json(path).map_err(|_| fail("alias_unknown", "local source map not readable"))?;
    if value.get("schema").and_then(Value::as_str) != Some(LOCAL_MAP_SCHEMA) {
        return Err(fail("alias_unknown", "local source map schema unsupported"));
    }
    let base = path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
    Ok((Some(value), base))
}

fn alias_source<'a>(
    map: Option<&'a Value>,
    base: &Path,
    alias: &str,
    file_id: u64,
    file_sha: &str,
) -> Check<PathBuf> {
    let map = map.ok_or_else(|| fail("alias_unknown", "no local alias map supplied"))?;
    let aliases = map
        .get("aliases")
        .and_then(Value::as_object)
        .ok_or_else(|| fail("alias_unknown", "alias map missing"))?;
    let entry = aliases
        .get(alias)
        .ok_or_else(|| fail("alias_unknown", "alias not in local map"))?;
    if entry.get("file_id").and_then(Value::as_u64) != Some(file_id) {
        return Err(fail("alias_unknown", "alias file id differs"));
    }
    let entry_sha = entry.get("sha256").and_then(Value::as_str).unwrap_or("");
    if sha_body(entry_sha).unwrap_or(entry_sha) != file_sha {
        return Err(fail("alias_unknown", "alias file digest differs"));
    }
    let rel = entry
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| fail("alias_unknown", "alias has no local path"))?;
    let rel = Path::new(rel);
    Ok(if rel.is_absolute() {
        rel.to_path_buf()
    } else {
        base.join(rel)
    })
}

fn snapshot_files(bundle: &Value) -> HashMap<u64, (String, u64)> {
    let mut out = HashMap::new();
    let Some(files) = bundle
        .get("snapshot")
        .and_then(|s| s.get("files"))
        .and_then(Value::as_array)
    else {
        return out;
    };
    for file in files {
        let Some(id) = file.get("file_id").and_then(Value::as_u64) else {
            continue;
        };
        let raw = file.get("sha256").and_then(Value::as_str).unwrap_or_default();
        let sha = sha_body(raw).unwrap_or(raw).to_string();
        let size = file.get("size").and_then(Value::as_u64).unwrap_or(0);
        out.insert(id, (sha, size));
    }
    out
}

fn check_snapshot(bundle: &Value, map: Option<&Value>) -> Check<()> {
    let Some(map) = map else {
        return Ok(());
    };
    let Some(local) = map.get("snapshot") else {
        return Err(fail("snapshot_mismatch", "local snapshot pin absent"));
    };
    let snap = bundle.get("snapshot").unwrap_or(&Value::Null);
    let bundle_id = snap.get("snapshot_id").and_then(Value::as_str);
    let local_id = local.get("snapshot_id").and_then(Value::as_str);
    if bundle_id.is_none() || bundle_id != local_id {
        return Err(fail("snapshot_mismatch", "snapshot id differs from local pin"));
    }
    let bundle_core = snap.get("core_digest").and_then(Value::as_str);
    let local_core = local.get("core_digest").and_then(Value::as_str);
    if bundle_core.is_none() || bundle_core != local_core {
        return Err(fail("snapshot_mismatch", "snapshot core digest differs from local pin"));
    }
    Ok(())
}

fn read_source(path: &Path) -> Check<Vec<u8>> {
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(fail("source_missing", "mapped source file absent"))
        }
        Err(_) => Err(fail("source_unreadable", "mapped source file not readable")),
    }
}

// ---------------------------------------------------------------------------
// Deterministic selectors and source reopening

pub(crate) fn row_spans(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = 0usize;
    for (i, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            let mut end = i;
            if end > start && bytes[end - 1] == b'\r' {
                end -= 1;
            }
            spans.push((start, end - start));
            start = i + 1;
        }
    }
    if start < bytes.len() {
        let mut end = bytes.len();
        if end > start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
        spans.push((start, end - start));
    }
    spans
}

fn parsed_rows(bytes: &[u8]) -> Vec<Option<Value>> {
    row_spans(bytes)
        .iter()
        .map(|(off, len)| serde_json::from_slice(&bytes[*off..*off + *len]).ok())
        .collect()
}

pub(crate) fn row_index_of(off: usize, bytes: &[u8]) -> usize {
    bytes[..off.min(bytes.len())]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
}

pub(crate) fn selector_of(bytes: &[u8], row: usize, content: usize, kind: &str) -> String {
    let (off, len) = row_spans(bytes)
        .get(row)
        .copied()
        .unwrap_or((0, 0));
    format!("{SELECTOR_VERSION}|span:{off}:{len}|message.content[{content}]|{kind}")
}

fn parse_selector(selector: &str) -> Check<(usize, usize, usize, String)> {
    let parts: Vec<&str> = selector.split('|').collect();
    if parts.len() != 4 || parts[0] != SELECTOR_VERSION {
        return Err(fail("selector_mismatch", "selector version/shape unsupported"));
    }
    let span = parts[1]
        .strip_prefix("span:")
        .and_then(|s| s.split_once(':'))
        .ok_or_else(|| fail("selector_mismatch", "selector span malformed"))?;
    let off = span.0.parse::<usize>().map_err(|_| fail("selector_mismatch", "selector offset malformed"))?;
    let len = span.1.parse::<usize>().map_err(|_| fail("selector_mismatch", "selector length malformed"))?;
    let inner = parts[2]
        .strip_prefix("message.content[")
        .and_then(|s| s.strip_suffix(']'))
        .and_then(|s| s.parse::<usize>().ok())
        .ok_or_else(|| fail("selector_mismatch", "selector content index malformed"))?;
    Ok((off, len, inner, parts[3].to_string()))
}

fn reopen_block(bytes: &[u8], selector: &str) -> Check<(usize, Value)> {
    let (off, len, content, kind) = parse_selector(selector)?;
    if off > bytes.len() || len > bytes.len() - off {
        return Err(fail("span_out_of_range", "selector span outside source"));
    }
    let slice = &bytes[off..off + len];
    let row: Value = serde_json::from_slice(slice)
        .map_err(|_| fail("span_out_of_range", "spanned bytes are not a JSON row"))?;
    let blocks = row
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
        .ok_or_else(|| fail("span_out_of_range", "selector message content absent"))?;
    let Some(block) = blocks.get(content) else {
        return Err(fail("span_out_of_range", "selector content index absent"));
    };
    if block.get("type").and_then(Value::as_str) != Some(kind.as_str()) {
        return Err(fail("selector_mismatch", "selector block type differs"));
    }
    Ok((row_index_of(off, bytes), block.clone()))
}

pub(crate) fn derive_tool(block: &Value) -> String {
    block
        .get("toolName")
        .or_else(|| block.get("tool"))
        .or_else(|| block.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn value_class(value: &Value) -> &'static str {
    match value {
        Value::String(s) if looks_like_path(s) => "P",
        Value::String(_) => "S",
        Value::Number(_) => "N",
        Value::Bool(_) => "B",
        Value::Null => "Z",
        Value::Array(_) => "A",
        Value::Object(_) => "O",
    }
}

fn looks_like_path(text: &str) -> bool {
    text.starts_with('/')
        || text.starts_with('~')
        || text.contains('\\')
        || (text.len() > 2 && text.as_bytes()[1] == b':')
}

pub(crate) fn derive_signature(block: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(args) = block.get("arguments").and_then(Value::as_object) {
        for (key, value) in args {
            parts.push(format!("{}={}", key, value_class(value)));
        }
    }
    parts.sort();
    format!("{}|{}", derive_tool(block), parts.join(","))
}

// ---------------------------------------------------------------------------
// Redaction policy `ase-p4-redact-v1` (contract §3.7)

fn patterns() -> &'static Vec<(Regex, &'static str)> {
    static PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
        let raw: [(&str, &str); 8] = [
            (r#"[A-Za-z][A-Za-z0-9+.-]*://[^\s"'<>\x60]+"#, "«url»"),
            (r#"[A-Za-z]:[\\/][^\s"'<>\x60|]+"#, "«path»"),
            (r#"\\\\[^\s"'<>\x60|]+"#, "«path»"),
            (r#"/(?:home|Users|root|tmp|var|etc|opt|mnt|workspace|data)/[^\s"'<>\x60|]+"#, "«path»"),
            (r#"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"#, "«email»"),
            (r#"(?:sk-|ghp_|github_pat_|AKIA|ASIA|xox[abp]-)[A-Za-z0-9_\-]{8,}"#, "«secret»"),
            (r#"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}"#, "«secret»"),
            (r#"[A-Za-z0-9+/=_-]{48,}"#, "«token»"),
        ];
        raw.iter()
            .map(|(source, marker)| {
                (
                    Regex::new(source).expect("fixed redaction pattern"),
                    *marker,
                )
            })
            .collect()
    });
    &PATTERNS
}

fn replace_filtered<F>(text: &str, re: &Regex, marker: &str, allowed: F) -> String
where
    F: Fn(&str, usize) -> bool,
{
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    for m in re.find_iter(text) {
        if !allowed(text, m.start()) {
            continue;
        }
        out.push_str(&text[last..m.start()]);
        out.push_str(marker);
        last = m.end();
    }
    out.push_str(&text[last..]);
    out
}

fn word_boundary(text: &str, at: usize) -> bool {
    text[..at]
        .chars()
        .next_back()
        .is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_')
}

fn strict_boundary(text: &str, at: usize) -> bool {
    text[..at].chars().next_back().is_none_or(|c| {
        !c.is_ascii_alphanumeric() && c != '_' && c != '.' && c != '-'
    })
}

fn redact(text: &str) -> String {
    let p = patterns();
    let mut out = p[0].0.replace_all(text, p[0].1).into_owned();
    // The table is applied in order, one left-to-right pass per pattern.
    out = replace_filtered(&out, &p[1].0, p[1].1, word_boundary);
    out = p[2].0.replace_all(&out, p[2].1).into_owned();
    out = replace_filtered(&out, &p[3].0, p[3].1, strict_boundary);
    for (re, marker) in &p[4..] {
        out = re.replace_all(&out, *marker).into_owned();
    }
    out
}


fn truncate_line(line: &str) -> String {
    if line.len() <= 160 {
        return line.to_string();
    }
    let mut cut = 0usize;
    for (i, ch) in line.char_indices() {
        if i + ch.len_utf8() > 157 {
            break;
        }
        cut = i + ch.len_utf8();
    }
    let mut out = line[..cut].to_string();
    out.push('…');
    out
}

pub(crate) fn cap_signature(signature: &str) -> String {
    truncate_line(signature)
}

pub(crate) fn redaction_lines(text: &str) -> Vec<String> {
    let redacted = redact(text);
    if redacted.is_empty() {
        return Vec::new();
    }
    redacted
        .split('\n')
        .take(3)
        .map(|line| truncate_line(line.strip_suffix('\r').unwrap_or(line)))
        .collect()
}

pub(crate) fn render_excerpt(
    ordinal: u64,
    tool: &str,
    signature: &str,
    status: &str,
    lines: &[String],
) -> String {
    let mut out = format!("call[{ordinal}] tool={tool} sig={signature}\n");
    match status {
        "absent" => out.push_str("result: «absent»"),
        "nontext" => out.push_str("result: «nontext»"),
        _ if lines.is_empty() => out.push_str("result: «empty»"),
        _ => {
            for (i, line) in lines.iter().enumerate() {
                if i == 0 {
                    out.push_str("result: ");
                } else {
                    out.push_str("        ");
                }
                out.push_str(line);
                if i + 1 < lines.len() {
                    out.push('\n');
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Evidence checks


pub(crate) fn result_status_of(block: &Value) -> &'static str {
    match block.get("status").and_then(Value::as_str) {
        Some("absent") => "absent",
        Some("nontext") => "nontext",
        _ if block.get("text").and_then(Value::as_str).is_some() => "present",
        _ => "nontext",
    }
}
pub(crate) fn result_for_call(rows: &[Option<Value>], call_row: usize, call_id: &str) -> Option<(usize, usize, Value)> {
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by_key(|&i| {
        let row = rows[i].as_ref();
        (
            row.and_then(|r| r.get("timestamp")).and_then(Value::as_str).unwrap_or("").to_string(),
            row.and_then(|r| r.get("ordinal")).and_then(Value::as_u64).unwrap_or(0),
        )
    });
    for i in order {
        if i <= call_row {
            continue;
        }
        let Some(row) = rows[i].as_ref() else { continue };
        let message = row.get("message").unwrap_or(&Value::Null);
        let role = message.get("role").and_then(Value::as_str);
        if role != Some("tool") && role != Some("toolResult") {
            continue;
        }
        let Some(content) = message.get("content").and_then(Value::as_array) else {
            continue;
        };
        // Fixture and older shapes carry the id on a toolResult content block.
        for (ci, block) in content.iter().enumerate() {
            if block.get("type").and_then(Value::as_str) == Some("toolResult")
                && block.get("toolCallId").and_then(Value::as_str) == Some(call_id)
            {
                return Some((i, ci, block.clone()));
            }
        }
        // Native OMP puts the id on the message and the text in a content block.
        if message.get("toolCallId").and_then(Value::as_str) == Some(call_id) {
            for (ci, block) in content.iter().enumerate() {
                if block.get("text").and_then(Value::as_str).is_some() {
                    return Some((i, ci, block.clone()));
                }
            }
        }
    }
    None
}

fn check_evidence(
    row: &Value,
    occurrence: &Value,
    motif: &Value,
    bundle: &Value,
    map: Option<&Value>,
    map_base: &Path,
    files: &HashMap<u64, (String, u64)>,
) -> Check<bool> {
    let call = row.get("call").unwrap_or(&Value::Null);
    let file_id = call.get("file_id").and_then(Value::as_u64).ok_or_else(|| fail("alias_unknown", "call file id missing"))?;
    let (file_sha, _) = files.get(&file_id).ok_or_else(|| fail("alias_unknown", "call file id absent from snapshot"))?;
    let alias = str_field(occurrence, "source_alias")?;
    let source = alias_source(map, map_base, alias, file_id, file_sha)?;
    let bytes = read_source(&source)?;

    // A changed whole-file digest is only a watermark: the decision is made
    // from the re-opened spans below, never from the whole-file digest alone.
    let append = sha256_hex(&bytes) != *file_sha;

    let selector = str_field(call, "selector")?;
    let (call_row, call_block) = reopen_block(&bytes, selector)?;
    let recorded_block_sha = sha_body(str_field(call, "block_sha256")?)
        .ok_or_else(|| fail("digest_mismatch", "call block digest malformed"))?;
    if sha256_hex(&canonical(&call_block)) != recorded_block_sha {
        return Err(fail("digest_mismatch", "reopened call block digest differs"));
    }

    let expected_id = value_digest(&json!({
        "occurrence_id": str_field(occurrence, "occurrence_id")?,
        "call_ordinal": row.get("call_ordinal").and_then(Value::as_u64).unwrap_or(0),
        "selector": selector,
        "block_sha256": str_field(call, "block_sha256")?,
    }));
    if row.get("evidence_id").and_then(Value::as_str)
        != Some(format!("ev-{}", &expected_id[..16]).as_str())
    {
        return Err(fail("bundle_tampered", "evidence id does not match its coordinates"));
    }

    let tool = derive_tool(&call_block);
    let signature = cap_signature(&derive_signature(&call_block));
    if call.get("tool").and_then(Value::as_str) != Some(tool.as_str())
        || call.get("signature").and_then(Value::as_str) != Some(signature.as_str())
    {
        return Err(fail("selector_mismatch", "reopened tool/signature differs from bundle"));
    }

    let recorded_result = row.get("result").unwrap_or(&Value::Null);
    let status = recorded_result
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !matches!(status, "present" | "absent" | "nontext") {
        return Err(fail("result_status_mismatch", "published result status unsupported"));
    }
    let call_id = call_block
        .get("toolCallId")
        .or_else(|| call_block.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| fail("selector_mismatch", "reopened call has no tool call id"))?;
    let rows = parsed_rows(&bytes);
    let found = result_for_call(&rows, call_row, call_id);
    let (result_text, result_status) = match (found, status) {
        (None, "absent") => {
            if recorded_result.get("selector").is_some() || recorded_result.get("block_sha256").is_some() {
                return Err(fail("result_status_mismatch", "absent result carries a digest/selector"));
            }
            (String::new(), "absent")
        }
        (None, _) => return Err(fail("result_status_mismatch", "paired tool result absent")),
        (Some(_), "absent") => return Err(fail("result_status_mismatch", "paired tool result is present")),
        (Some((ri, ci, result_block)), expected) => {
            let actual_status = result_status_of(&result_block);
            if actual_status != expected {
                return Err(fail("result_status_mismatch", "result status differs from source"));
            }
            let kind = result_block.get("type").and_then(Value::as_str).unwrap_or("toolResult");
            let expected_selector = selector_of(&bytes, ri, ci, kind);
            if recorded_result.get("selector").and_then(Value::as_str) != Some(expected_selector.as_str()) {
                return Err(fail("selector_mismatch", "result selector differs from source"));
            }
            let recorded_sha = sha_body(
                recorded_result
                    .get("block_sha256")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            )
            .ok_or_else(|| fail("digest_mismatch", "result block digest malformed"))?;
            if sha256_hex(&canonical(&result_block)) != recorded_sha {
                return Err(fail("digest_mismatch", "reopened result block digest differs"));
            }
            let text = result_block
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            (text, expected)
        }
    };

    let ordinal = row.get("call_ordinal").and_then(Value::as_u64).unwrap_or(0);
    let mut lines = redaction_lines(&result_text);
    if let Some(kept) = row.get("result_lines").and_then(Value::as_u64) {
        if kept > 3 {
            return Err(fail("excerpt_mismatch", "result_lines exceeds the three-line limit"));
        }
        lines.truncate(kept as usize);
    }
    let excerpt = render_excerpt(ordinal, &tool, &signature, result_status, &lines);
    let published = row.get("excerpt").and_then(Value::as_str).unwrap_or("");
    let recorded_excerpt_sha = sha_body(
        row.get("excerpt_sha256").and_then(Value::as_str).unwrap_or(""),
    )
    .ok_or_else(|| fail("excerpt_digest_mismatch", "excerpt digest malformed"))?;
    if sha256_hex(published.as_bytes()) != recorded_excerpt_sha {
        return Err(fail("excerpt_digest_mismatch", "published excerpt digest differs"));
    }
    if published != excerpt {
        return Err(fail("excerpt_mismatch", "published excerpt differs from policy output"));
    }

    let expected_identity = occurrence_identity(bundle, motif, occurrence)?;
    if occurrence.get("input_identity").and_then(Value::as_str) != Some(expected_identity.as_str()) {
        return Err(fail("input_identity_mismatch", "occurrence input identity differs"));
    }
    Ok(append)
}

fn occurrence_identity(bundle: &Value, motif: &Value, occurrence: &Value) -> Check<String> {
    let mut evidence = Vec::new();
    let rows = occurrence
        .get("evidence")
        .and_then(Value::as_array)
        .ok_or_else(|| fail("bundle_tampered", "occurrence evidence missing"))?;
    for row in rows {
        evidence.push(json!({
            "evidence_id": row.get("evidence_id").and_then(Value::as_str).unwrap_or(""),
            "call_block_sha256": row.get("call").and_then(|c| c.get("block_sha256")).and_then(Value::as_str).unwrap_or(""),
            "excerpt_sha256": row.get("excerpt_sha256").and_then(Value::as_str).unwrap_or(""),
        }));
    }
    let questions = motif
        .get("questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let question_ids: Vec<String> = questions
        .iter()
        .filter_map(|q| q.get("question_id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let engine = bundle.get("engine").unwrap_or(&Value::Null);
    Ok(value_digest(&json!({
        "bundle_version": meta_version(bundle, "bundle_version", BUNDLE_VERSION)?,
        "motif_id": motif.get("motif_id").and_then(Value::as_str).unwrap_or(""),
        "occurrence_id": str_field(occurrence, "occurrence_id")?,
        "question_ids": question_ids,
        "rule_versions": engine.get("rule_versions").cloned().unwrap_or(Value::Null),
        "redaction_policy": engine.get("redaction_policy").and_then(Value::as_str).unwrap_or(""),
        "evidence": evidence,
    })))
}

pub(crate) fn occurrence_identity_hex(bundle: &Value, motif: &Value, occurrence: &Value) -> Option<String> {
    occurrence_identity(bundle, motif, occurrence).ok()
}

fn meta_version(bundle: &Value, key: &str, expected: &str) -> Check<String> {
    let actual = str_field(bundle, key)?;
    if actual != expected {
        return Err(fail("redaction_version_unsupported", format!("{key} unsupported")));
    }
    Ok(actual.to_string())
}

// ---------------------------------------------------------------------------
// Views

fn synthetic_line(value: &Value) -> &'static str {
    if value.get("synthetic").and_then(Value::as_bool) == Some(true) {
        "synthetic: true\n\n"
    } else {
        ""
    }
}

pub(crate) fn render_evidence_view(motif: &Value) -> String {
    let mut out = format!("# Evidence — {}\n\n", motif.get("motif_id").and_then(Value::as_str).unwrap_or(""));
    out.push_str(synthetic_line(motif));
    let reps = motif.get("representatives").and_then(Value::as_array).cloned().unwrap_or_default();
    for rep in reps {
        out.push_str(&format!(
            "## {} — selection={}\n",
            rep.get("occurrence_id").and_then(Value::as_str).unwrap_or(""),
            rep.get("selection").and_then(Value::as_str).unwrap_or(""),
        ));
        out.push_str(&format!("source_alias: {}\n", rep.get("source_alias").and_then(Value::as_str).unwrap_or("")));
        out.push_str(&format!("input_identity: sha256:{}\n\n", rep.get("input_identity").and_then(Value::as_str).unwrap_or("")));
        let rows = rep.get("evidence").and_then(Value::as_array).cloned().unwrap_or_default();
        for row in rows {
            let result = row.get("result").unwrap_or(&Value::Null);
            out.push_str(&format!("### call {}\n", row.get("call_ordinal").and_then(Value::as_u64).unwrap_or(0)));
            out.push_str(&format!("selector: {}\n", row.get("call").and_then(|c| c.get("selector")).and_then(Value::as_str).unwrap_or("")));
            out.push_str(&format!("block_sha256: {}\n", row.get("call").and_then(|c| c.get("block_sha256")).and_then(Value::as_str).unwrap_or("")));
            out.push_str(&format!("tool: {}\n", row.get("call").and_then(|c| c.get("tool")).and_then(Value::as_str).unwrap_or("")));
            out.push_str(&format!("signature: {}\n", row.get("call").and_then(|c| c.get("signature")).and_then(Value::as_str).unwrap_or("")));
            out.push_str(&format!("result_status: {}\n", result.get("status").and_then(Value::as_str).unwrap_or("")));
            out.push_str(&format!("result_selector: {}\n", result.get("selector").and_then(Value::as_str).unwrap_or("")));
            out.push_str(&format!("result_block_sha256: {}\n", result.get("block_sha256").and_then(Value::as_str).unwrap_or("")));
            out.push_str("excerpt:\n");
            for line in row.get("excerpt").and_then(Value::as_str).unwrap_or("").split('\n') {
                out.push_str("    ");
                out.push_str(line);
                out.push('\n');
            }
            out.push('\n');
        }
    }
    out
}

pub(crate) fn render_questions_view(motif: &Value) -> String {
    let mut out = format!("# Questions — {}\n\n", motif.get("motif_id").and_then(Value::as_str).unwrap_or(""));
    out.push_str(synthetic_line(motif));
    if let Some(questions) = motif.get("questions").and_then(Value::as_array) {
        for q in questions {
            out.push_str(&format!(
                "{}. {}\n",
                q.get("question_id").and_then(Value::as_str).unwrap_or(""),
                q.get("text").and_then(Value::as_str).unwrap_or(""),
            ));
        }
    }
    out
}

fn check_file_view(bundle_dir: &Path, rel: &str, recorded: &str, rendered: Option<String>) -> Check<()> {
    let bytes = fs::read(bundle_dir.join(rel))
        .map_err(|_| fail("view_digest_mismatch", "view file not readable"))?;
    let recorded = sha_body(recorded)
        .ok_or_else(|| fail("view_digest_mismatch", "view digest malformed"))?;
    if sha256_hex(&bytes) != recorded {
        return Err(fail("view_digest_mismatch", "view bytes differ from recorded digest"));
    }
    if let Some(rendered) = rendered {
        let rendered = rendered.into_bytes();
        if bytes != rendered {
            let at = bytes
                .iter()
                .zip(rendered.iter())
                .position(|(a, b)| a != b)
                .unwrap_or(bytes.len().min(rendered.len()));
            return Err(fail(
                "view_digest_mismatch",
                format!("view is not the deterministic render at byte {at} ({} vs {} bytes)", bytes.len(), rendered.len()),
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Optional verdict cross-check (read-only; no import, no store write)

pub(crate) fn assertion_body(bundle: &Value, verdict: &Value) -> Value {
    let mut refs = verdict
        .get("evidence_refs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    refs.sort_by(|a, b| a.as_str().unwrap_or("").cmp(b.as_str().unwrap_or("")));
    json!({
        "bundle_id": bundle.get("bundle_id").cloned().unwrap_or(Value::Null),
        "occurrence_id": verdict.get("occurrence_id").cloned().unwrap_or(Value::Null),
        "question_id": verdict.get("question_id").cloned().unwrap_or(Value::Null),
        "answer": verdict.get("answer").cloned().unwrap_or(Value::Null),
        "should_codify": verdict.get("should_codify").cloned().unwrap_or(Value::Null),
        "waste_score": verdict.get("waste_score").cloned().unwrap_or(Value::Null),
        "catalog_kind": verdict.get("catalog_kind").cloned().unwrap_or(Value::Null),
        "catalog_key": verdict.get("catalog_key").cloned().unwrap_or(Value::Null),
        "evidence_refs": refs,
    })
}

fn check_verdicts(
    bundle: &Value,
    verdicts: &Value,
    occurrences: &HashMap<String, Value>,
    evidence_state: &HashMap<String, &'static str>,
    report: &mut Value,
) {
    report["assertion_source"] = json!("local verdict file (read-only cross-check)");
    let Some(rows) = verdicts.get("verdicts").and_then(Value::as_array) else {
        push_failure(report, None, None, "stale_bundle", "verdict rows missing");
        return;
    };
    report["assertions_total"] = json!(rows.len());
    let envelope_failure = if verdicts.get("schema").and_then(Value::as_str)
        != Some("ase-p4-verdicts-v1")
    {
        Some(fail("schema_version_unsupported", "verdict file schema unsupported"))
    } else if verdicts.get("bundle_id").and_then(Value::as_str)
        != bundle.get("bundle_id").and_then(Value::as_str)
    {
        Some(fail("bundle_id_mismatch", "verdict bundle id differs"))
    } else if verdicts.get("bundle_digest").and_then(Value::as_str)
        != bundle.get("bundle_digest").and_then(Value::as_str)
        || verdicts.get("bundle_version").and_then(Value::as_str)
            != bundle.get("bundle_version").and_then(Value::as_str)
    {
        Some(fail("stale_bundle", "verdict bundle identity stale"))
    } else {
        None
    };
    if let Some(f) = envelope_failure {
        for verdict in rows {
            push_failure(
                report,
                verdict.get("assertion_id").and_then(Value::as_str),
                verdict.get("occurrence_id").and_then(Value::as_str),
                f.code,
                &f.detail,
            );
        }
        report["assertions_unverified"] = json!(rows.len());
        return;
    }
    for verdict in rows {
        let occ_id = verdict.get("occurrence_id").and_then(Value::as_str).unwrap_or("");
        let Some(occ) = occurrences.get(occ_id).cloned() else {
            push_failure(report, None, Some(occ_id), "occurrence_unknown", "occurrence unknown in bundle");
            bump(report, "assertions_unverified", 1);
            continue;
        };
        let refs = verdict.get("evidence_refs").and_then(Value::as_array).cloned().unwrap_or_default();
        let question_ok = occ.get("questions").and_then(Value::as_array).is_some_and(|qs| {
            qs.iter().any(|q| {
                q.get("question_id").and_then(Value::as_str)
                    == verdict.get("question_id").and_then(Value::as_str)
            })
        });
        let failure = if !question_ok {
            Some(fail("question_unknown", "question unknown for occurrence"))
        } else if refs.is_empty() {
            Some(fail("missing_evidence", "verdict has no evidence refs"))
        } else if refs.iter().any(|r| !evidence_state.contains_key(r.as_str().unwrap_or(""))) {
            Some(fail("missing_evidence", "evidence ref unknown in bundle"))
        } else if refs.iter().any(|r| {
            evidence_state.get(r.as_str().unwrap_or("")) != Some(&"verified")
                && evidence_state.get(r.as_str().unwrap_or("")) != Some(&"verified_span_after_append")
        }) {
            Some(fail("evidence_unverified", "referenced evidence not verified"))
        } else if verdict.get("input_identity").and_then(Value::as_str)
            != occ.get("input_identity").and_then(Value::as_str)
        {
            Some(fail("input_identity_mismatch", "verdict input identity stale"))
        } else {
            let expected = value_digest(&assertion_body(bundle, verdict));
            if verdict.get("assertion_id").and_then(Value::as_str)
                != Some(format!("as-{}", &expected[..16]).as_str())
            {
                Some(fail("assertion_id_conflict", "assertion id differs from content"))
            } else {
                None
            }
        };
        match failure {
            Some(f) => {
                push_failure(report, None, Some(occ_id), f.code, &f.detail);
                bump(report, "assertions_unverified", 1);
            }
            None => bump(report, "assertions_verified", 1),
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level orchestration

pub(crate) fn verify(bundle_dir: &Path, map_path: Option<&Path>, verdicts_path: Option<&Path>) -> Value {
    let mut report = base_report();
    let bytes = match fs::read(bundle_dir.join("bundle.json")) {
        Ok(bytes) => bytes,
        Err(_) => return top_failure(&mut report, "bundle_tampered", "bundle.json not readable", 0, 0, 0),
    };
    let bundle: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return top_failure(&mut report, "bundle_tampered", "bundle.json not JSON", 0, 0, 0),
    };
    let motifs: Vec<Value> = bundle
        .get("motifs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let (total_evidence, total_occurrences) = count_evidence(bundle_dir, &motifs);
    report["occurrences_total"] = json!(total_occurrences);

    let schema_ok = bundle.get("schema").and_then(Value::as_str) == Some(BUNDLE_VERSION)
        && bundle.get("bundle_version").and_then(Value::as_str) == Some(BUNDLE_VERSION);
    if !schema_ok {
        return top_failure(&mut report, "schema_version_unsupported", "bundle schema/version unsupported", total_evidence, motifs.len() as u64, total_occurrences);
    }
    let no_digest = {
        let mut value = bundle.clone();
        value.as_object_mut().expect("bundle object").remove("bundle_digest");
        value
    };
    if bundle.get("bundle_digest").and_then(Value::as_str)
        != Some(format!("sha256:{}", value_digest(&no_digest)).as_str())
    {
        return top_failure(&mut report, "bundle_digest_mismatch", "bundle digest does not match canonical body", total_evidence, motifs.len() as u64, total_occurrences);
    }
    let no_id = {
        let mut value = bundle.clone();
        value.as_object_mut().expect("bundle object").remove("bundle_id");
        value.as_object_mut().expect("bundle object").remove("bundle_digest");
        value
    };
    if bundle.get("bundle_id").and_then(Value::as_str)
        != Some(format!("bun-{}", &value_digest(&no_id)[..16]).as_str())
    {
        return top_failure(&mut report, "bundle_tampered", "bundle id does not match canonical body", total_evidence, motifs.len() as u64, total_occurrences);
    }

    let engine = bundle.get("engine").unwrap_or(&Value::Null);
    let version_check: [(&str, &str); 5] = [
        ("redaction_policy", REDACTION_POLICY),
        ("evidence_render", EVIDENCE_RENDER),
        ("questions_render", QUESTIONS_RENDER),
        ("representative_policy", REPRESENTATIVE_POLICY),
        ("name", "ase"),
    ];
    for (key, expected) in version_check {
        if bundle.get(key).and_then(Value::as_str) == Some(expected)
            || engine.get(key).and_then(Value::as_str) == Some(expected)
        {
            continue;
        }
        let code = match key {
            "redaction_policy" => "redaction_version_unsupported",
            "evidence_render" | "questions_render" => "view_digest_mismatch",
            "representative_policy" => "engine_version_unsupported",
            _ => "engine_version_unsupported",
        };
        return top_failure(&mut report, code, "bundle version unsupported", total_evidence, motifs.len() as u64, total_occurrences);
    }
    let rules = engine.get("rule_versions").and_then(Value::as_object);
    let required_rules: [(&str, &str); 4] = [
        ("callsig", "callsig-v1"),
        ("motif_alignment", "motif-align-v1"),
        ("motif_dedupe", "seq-text-v1"),
        ("projection", SELECTOR_VERSION),
    ];
    for (key, expected) in required_rules {
        if rules.and_then(|r| r.get(key)).and_then(Value::as_str) != Some(expected) {
            return top_failure(&mut report, "engine_version_unsupported", "rule version unsupported", total_evidence, motifs.len() as u64, total_occurrences);
        }
    }
    if bundle.get("redaction").and_then(|r| r.get("policy")).and_then(Value::as_str)
        != Some(REDACTION_POLICY)
    {
        return top_failure(&mut report, "redaction_version_unsupported", "redaction policy block unsupported", total_evidence, motifs.len() as u64, total_occurrences);
    }

    let (map, map_base) = match load_local_map(map_path) {
        Ok(value) => value,
        Err(f) => return top_failure(&mut report, f.code, &f.detail, total_evidence, motifs.len() as u64, total_occurrences),
    };
    if let Err(f) = check_snapshot(&bundle, map.as_ref()) {
        return top_failure(&mut report, f.code, &f.detail, total_evidence, motifs.len() as u64, total_occurrences);
    }
    let files = snapshot_files(&bundle);

    let mut occurrences: HashMap<String, Value> = HashMap::new();
    let mut evidence_state: HashMap<String, &'static str> = HashMap::new();
    let mut all_motifs_ok = true;
    for motif_row in &motifs {
        bump(&mut report, "motifs_total", 1);
        let motif_id = motif_row.get("motif_id").and_then(Value::as_str).unwrap_or("");
        let motif_rel = motif_row.get("motif_path").and_then(Value::as_str).unwrap_or("");
        let evidence_rel = motif_row.get("evidence_path").and_then(Value::as_str).unwrap_or("");
        let questions_rel = motif_row.get("questions_path").and_then(Value::as_str).unwrap_or("");
        let motif: Value = match read_json(&bundle_dir.join(motif_rel)) {
            Ok(v) => v,
            Err(_) => {
                push_failure(&mut report, None, None, "motif_digest_mismatch", "motif.json unreadable");
                all_motifs_ok = false;
                continue;
            }
        };
        if motif.get("schema").and_then(Value::as_str) != Some(MOTIF_SCHEMA)
            || motif.get("motif_id").and_then(Value::as_str) != Some(motif_id)
        {
            push_failure(&mut report, None, None, "motif_digest_mismatch", "motif schema/id mismatch");
            all_motifs_ok = false;
            continue;
        }
        let mut motif_ok = true;
        let recorded_motif_sha = motif_row.get("motif_sha256").and_then(Value::as_str).unwrap_or("");
        match fs::read(bundle_dir.join(motif_rel)) {
            Ok(bytes) if sha_body(recorded_motif_sha) == Some(sha256_hex(&bytes).as_str()) => {}
            _ => {
                push_failure(&mut report, None, None, "motif_digest_mismatch", "motif bytes differ from recorded digest");
                motif_ok = false;
            }
        }
        for (rel, recorded, rendered) in [
            (evidence_rel, motif_row.get("evidence_sha256").and_then(Value::as_str).unwrap_or(""), Some(render_evidence_view(&motif))),
            (questions_rel, motif_row.get("questions_sha256").and_then(Value::as_str).unwrap_or(""), Some(render_questions_view(&motif))),
        ] {
            if let Err(f) = check_file_view(bundle_dir, rel, recorded, rendered) {
                push_failure(&mut report, None, None, f.code, &f.detail);
                motif_ok = false;
            }
        }
        report["views_checked"] = json!(["evidence.md", "questions.md"]);

        let reps = motif.get("representatives").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut motif_all_accepted = motif_ok;
        for rep in reps {
            let occ_id = rep.get("occurrence_id").and_then(Value::as_str).unwrap_or("").to_string();
            let mut indexed = rep.clone();
            indexed["questions"] = motif.get("questions").cloned().unwrap_or_else(|| json!([]));
            occurrences.insert(occ_id.clone(), indexed);
            let rows = rep.get("evidence").and_then(Value::as_array).cloned().unwrap_or_default();
            for _row in &rows {
                bump(&mut report, "evidence_total", 1);
            }
            if !motif_ok {
                for row in &rows {
                    bump(&mut report, "evidence_failed", 1);
                    if let Some(id) = row.get("evidence_id").and_then(Value::as_str) {
                        evidence_state.insert(id.to_string(), "failed");
                    }
                }
                continue;
            }
            let mut outcomes: Vec<Check<bool>> = Vec::new();
            for row in &rows {
                outcomes.push(check_evidence(
                    row,
                    &rep,
                    &motif,
                    &bundle,
                    map.as_ref(),
                    &map_base,
                    &files,
                ));
            }
            for (row, outcome) in rows.iter().zip(outcomes.into_iter()) {
                let ev_id = row.get("evidence_id").and_then(Value::as_str).unwrap_or("");
                match outcome {
                    Ok(append) => {
                        if append {
                            bump(&mut report, "evidence_verified", 1);
                            evidence_state.insert(ev_id.to_string(), "verified_span_after_append");
                            report["watermarks"].as_array_mut().expect("watermarks array").push(json!({
                                "evidence_id": ev_id,
                                "occurrence_id": occ_id,
                                "code": "verified_span_after_append",
                                "note": "whole-file digest differs from the frozen pin; only reopened spans were rechecked, no current-file completeness claimed",
                            }));
                        } else {
                            bump(&mut report, "evidence_verified", 1);
                            evidence_state.insert(ev_id.to_string(), "verified");
                        }
                    }
                    Err(f) => {
                        motif_all_accepted = false;
                        bump(&mut report, if unverifiable(f.code) { "evidence_unverified" } else { "evidence_failed" }, 1);
                        evidence_state.insert(ev_id.to_string(), "failed");
                        push_failure(&mut report, Some(ev_id), Some(&occ_id), f.code, &f.detail);
                    }
                }
            }
        }
        if motif_ok && motif_all_accepted {
            bump(&mut report, "motifs_verified", 1);
        } else {
            all_motifs_ok = false;
        }
    }

    if report["evidence_total"].as_u64().unwrap_or(0) == 0 {
        report["notes"].as_array_mut().expect("notes").push(json!("empty bundle is never verified"));
    }
    if let Some(path) = verdicts_path {
        match read_json(path) {
            Ok(verdicts) => check_verdicts(&bundle, &verdicts, &occurrences, &evidence_state, &mut report),
            Err(f) => push_failure(&mut report, None, None, f.code, &f.detail),
        }
    }

    let total_ev = report["evidence_total"].as_u64().unwrap_or(0);
    let verified = report["evidence_verified"].as_u64().unwrap_or(0);
    let failed = report["evidence_failed"].as_u64().unwrap_or(0);
    let unverified = report["evidence_unverified"].as_u64().unwrap_or(0);
    let assertions_ok = report["assertions_total"].as_u64().unwrap_or(0)
        == report["assertions_verified"].as_u64().unwrap_or(0);
    let ok = all_motifs_ok
        && total_ev > 0
        && verified == total_ev
        && failed == 0
        && unverified == 0
        && assertions_ok;
    report["status"] = json!(if ok { "verified" } else { "unverified" });
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_applies_normative_markers_in_order() {
        let text = "go https://x.invalid/a C:\\tmp\\a /workspace/b a@b.invalid sk-abcdefghij\n";
        assert_eq!(
            redact(text),
            "go «url» «path» «path» «email» «secret»\n"
        );
    }

    #[test]
    fn truncation_keeps_utf8_boundary_at_or_below_157_bytes() {
        let line = format!("{}é", "x".repeat(160));
        let out = truncate_line(&line);
        assert_eq!(out.len(), 160);
        assert!(out.ends_with('…'));
    }
}
