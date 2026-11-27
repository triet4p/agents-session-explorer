//! Selector-resolved block reopen against one pinned publication.
//!
//! A hit is accepted only when its generation, event, span and block digest
//! all agree with the address snapshot and the current raw bytes.

use crate::cli_option::{has, opt};
use crate::cli_output::emit;
use crate::index::require_publication;
use crate::session_nav::session_args;
use crate::show::{freshness_json, span_label};
use ase_postings::block_by_selector;
use ase_core::{err, RawStore, Result};
use ase_ingest::util::fnv64;
use serde_json::{json, Value};


const RAW_READ_TEXT_ERROR: &str = "cannot read the pinned raw source; check that the indexed source is still available, then re-index and search again for a new snapshot";

/// Flag combinations that make the metadata-only text report ambiguous.
fn conflicting_text_flag(args: &[String]) -> Option<&'static str> {
    if !has(args, "--text") {
        return None;
    }
    ["--json", "--content", "--session-args"]
        .into_iter()
        .find(|flag| has(args, flag))
}

fn open_usage_text() -> &'static str {
    "ase open --event N --selector S --generation G [--index FILE] [--text]\n\
     [--session-args] [--content] [--json]\n\
     --text prints the evidence status, block key, span, encoding and coordinate\n\
     system, both hash checks and the freshness hint. It excludes --json, --content\n\
     and --session-args. No prose, signature or journal path is printed."
}

fn open_usage() {
    println!("{}", open_usage_text());
}

/// Verified evidence requires both the pinned span checksum and the block digest.
fn evidence_status(span_ok: bool, block_ok: bool) -> &'static str {
    if span_ok && block_ok {
        "verified_against_index_snapshot"
    } else {
        "unverified"
    }
}

/// Human-readable counterpart of the JSON reopen report. It reads only metadata
/// fields: content, signatures and source paths are never rendered.
fn render_text(v: &Value) -> String {
    let status = v["status"].as_str().unwrap_or("unverified");
    let span_ok = v["evidence"]["raw_span_hash_match"]
        .as_bool()
        .unwrap_or(false);
    let block_ok = v["evidence"]["block_digest_match"]
        .as_bool()
        .unwrap_or(false);
    let mut out = format!(
        "open: status={status}\n\
         block: generation={} event={} selector={}\n\
         span: {} encoding={} coordinate={}\n\
         evidence: raw_span_hash_match={span_ok} block_digest_match={block_ok}\n\
         freshness: {} (metadata hint only; verification re-reads the current raw bytes)\n",
        v["generation"].as_str().unwrap_or(""),
        v["event"].as_u64().unwrap_or_default(),
        v["selector"].as_str().unwrap_or(""),
        v["span"].as_str().unwrap_or(""),
        v["encoding"].as_str().unwrap_or(""),
        v["coordinate"].as_str().unwrap_or(""),
        v["evidence"]["source_freshness"]
            .as_str()
            .unwrap_or("unknown"),
    );
    if status == "verified_against_index_snapshot" {
        out.push_str("note: both the span checksum and block digest match the pinned snapshot\n");
    } else {
        out.push_str("note: the raw source did not match the pinned snapshot; inspect the source; re-index and search again only when a new snapshot is intended\n");
    }
    if v["coordinate"].as_str() == Some("decompressed_stream_bytes") {
        out.push_str(
            "note: zstd offsets address the decompressed stream, not the compressed file bytes\n",
        );
    }
    out
}

fn render_generation_mismatch(v: &Value) -> String {
    format!(
        "open: status=generation_mismatch\n\
         requested_generation={}\n\
         active_generation={}\n\
         note: refused before reading raw; the hit belongs to another generation\n\
         note: search again on the intended --index; do not substitute the generation of an old hit\n",
        v["requested_generation"].as_str().unwrap_or(""),
        v["active_generation"].as_str().unwrap_or(""),
    )
}

pub(crate) fn hash_bytes(hex: &str) -> Option<[u8; 8]> {
    if hex.len() != 16 {
        return None;
    }
    let mut out = [0u8; 8];
    for (i, pair) in hex.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(pair).ok()?;
        out[i] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(out)
}

pub fn run(args: &[String]) -> Result<()> {
    if has(args, "--help") || has(args, "-h") {
        open_usage();
        return Ok(());
    }
    if let Some(flag) = conflicting_text_flag(args) {
        return Err(err(format!(
            "--text cannot be combined with {flag}; see `ase open --help`"
        )));
    }
    let text = has(args, "--text");
    let event: u32 = opt(args, "--event")
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| err("open needs --event N"))?;
    let selector = opt(args, "--selector").ok_or_else(|| err("open needs --selector S"))?;
    let requested_generation = opt(args, "--generation")
        .ok_or_else(|| err("open needs --generation from the search hit"))?;
    let (idx, store, published) = require_publication(args)?;
    if requested_generation != published.generation {
        let v = json!({
            "command": "open",
            "status": "generation_mismatch",
            "requested_generation": requested_generation,
            "active_generation": published.generation,
            "address_usable": true,
            "note": "refused before reading raw: the hit belongs to another generation"
        });
        if text {
            println!("{}", render_generation_mismatch(&v));
        } else {
            emit(&v, args);
        }
        std::process::exit(4);
    }
    let record = block_by_selector(&published, event, &selector)?
        .ok_or_else(|| err("no block mapping for that event and selector in this generation"))?;
    if event >= idx.event_count {
        return Err(err("mapped event outside pinned address"));
    }
    let ev = idx.event(event)?;
    let span = ev.span();
    if span != record.span {
        return Err(err("block span differs from pinned address"));
    }
    let raw = match store.span(span) {
        Ok(raw) => raw,
        Err(e) => {
            return Err(if text { err(RAW_READ_TEXT_ERROR) } else { e });
        }
    };
    let span_ok = fnv64(&raw) == ev.hash();
    let extracted = ase_ingest::extract_block(idx.file(span.file)?.provider(), &raw, &selector);
    let block_ok = extracted
        .as_ref()
        .map(|e| hash_bytes(&record.block_hash) == Some(e.block_hash))
        .unwrap_or(false);
    let fresh = freshness_json(&idx, span.file);
    let status = evidence_status(span_ok, block_ok);
    let verified = status == "verified_against_index_snapshot";
    let navigation = if has(args, "--session-args") {
        Some(session_args(&idx, &store, &published, event, verified)?)
    } else {
        None
    };
    let mut v = json!({
        "command": "open",
        "status": status,
        "generation": published.generation,
        "event": event,
        "selector": selector,
        "span": span_label(span),
        "kind": record.kind,
        "role": record.role,
        "session_local": record.session_local,
        "parent_idx": record.parent_idx,
        "encoding": idx.file(span.file)?.encoding().label(),
        "coordinate": idx.file(span.file)?.encoding().coordinate(),
        "evidence": {
            "raw_span_hash_match": span_ok,
            "block_digest_match": block_ok,
            "source_freshness": fresh["status"],
            "note": "verification re-derives from current raw bytes; edited raw is never verified evidence"
        },
        "explain": [
            "block row resolved by (generation, event, selector), never by rowid alone",
            "raw bytes re-read at the pinned address coordinate",
            "block re-derived from raw JSON with the versioned selector; no stored prose is consulted"
        ]
    });
    if let Some(nav) = navigation {
        v["session_args"] = nav;
    }
    if has(args, "--content") {
        let content = match extracted {
            Ok(e) => e.prose.or(e.call_sig).unwrap_or_default(),
            Err(_) => String::new(),
        };
        v["content"] = json!(content);
    }
    if text {
        println!("{}", render_text(&v));
        return Ok(());
    }
    emit(&v, args);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        conflicting_text_flag, evidence_status, open_usage_text, render_generation_mismatch,
        render_text, run, RAW_READ_TEXT_ERROR,
    };
    use serde_json::json;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn test_help_returns_before_reading_a_missing_index() {
        assert!(run(&args(&["--help", "--index", "no-such-dir/ase.index"])).is_ok());
    }

    #[test]
    fn test_text_conflict_errors_before_reading_a_missing_index() {
        let error = run(&args(&[
            "--event",
            "1",
            "--selector",
            "block-v1|x",
            "--generation",
            "gen-1",
            "--index",
            "no-such-dir/ase.index",
            "--text",
            "--json",
        ]))
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("--text cannot be combined with --json"));
    }

    #[test]
    fn test_verified_requires_both_span_and_block_checks() {
        assert_eq!(
            evidence_status(true, true),
            "verified_against_index_snapshot"
        );
        for (span_ok, block_ok) in [(true, false), (false, true), (false, false)] {
            assert_eq!(evidence_status(span_ok, block_ok), "unverified");
        }
    }

    #[test]
    fn test_text_conflicts_are_reported_before_index_access() {
        for flag in ["--json", "--content", "--session-args"] {
            assert_eq!(
                conflicting_text_flag(&args(&["--event", "7", "--text", flag])),
                Some(flag)
            );
        }
        assert_eq!(
            conflicting_text_flag(&args(&[
                "--event",
                "7",
                "--selector",
                "block-v1|x",
                "--generation",
                "gen-1",
                "--index",
                "dir with space/ase.index",
                "--text"
            ])),
            None
        );
        assert_eq!(
            conflicting_text_flag(&args(&["--json", "--content", "--session-args"])),
            None
        );
    }

    #[test]
    fn test_help_names_index_text_and_excluded_flags() {
        let help = open_usage_text();
        assert!(help.contains("--index FILE"));
        assert!(help.contains("--text"));
        assert!(help.contains("excludes --json, --content"));
        assert!(help.contains("--session-args"));
        assert!(help.contains("No prose, signature or journal path"));
    }

    #[test]
    fn test_text_render_reports_both_hashes_and_freshness_without_source_material() {
        let v = json!({
            "status": "unverified",
            "generation": "gen-0123456789abcdef",
            "event": 7,
            "selector": "block-v1|abc",
            "span": "span:0:12:34",
            "encoding": "jsonl.zst",
            "coordinate": "decompressed_stream_bytes",
            "evidence": {
                "raw_span_hash_match": true,
                "block_digest_match": false,
                "source_freshness": "stale"
            },
            "content": "PROSE_CANARY",
            "signature": "SIGNATURE_CANARY",
            "path": "journal-canary.jsonl"
        });
        let text = render_text(&v);
        assert!(text.contains("open: status=unverified"));
        assert!(text.contains("generation=gen-0123456789abcdef"));
        assert!(text.contains("event=7"));
        assert!(text.contains("selector=block-v1|abc"));
        assert!(text.contains("span:0:12:34"));
        assert!(text.contains("encoding=jsonl.zst"));
        assert!(text.contains("coordinate=decompressed_stream_bytes"));
        assert!(text.contains("raw_span_hash_match=true"));
        assert!(text.contains("block_digest_match=false"));
        assert!(text.contains("freshness: stale"));
        assert!(text.contains("decompressed stream"));
        assert!(text.contains("re-index and search again"));
        assert!(!text.contains("PROSE_CANARY"));
        assert!(!text.contains("SIGNATURE_CANARY"));
        assert!(!text.contains("journal-canary.jsonl"));
        assert!(!text.contains("--content"));
    }

    #[test]
    fn test_verified_text_claims_snapshot_match() {
        let v = json!({
            "status": "verified_against_index_snapshot",
            "generation": "gen-1",
            "event": 2,
            "selector": "block-v1|ok",
            "span": "span:0:0:9",
            "encoding": "jsonl",
            "coordinate": "raw_file_bytes",
            "evidence": {
                "raw_span_hash_match": true,
                "block_digest_match": true,
                "source_freshness": "fresh"
            }
        });
        let text = render_text(&v);
        assert!(text.contains("open: status=verified_against_index_snapshot"));
        assert!(text.contains("freshness: fresh"));
        assert!(text.contains("both the span checksum and block digest match"));
        assert!(!text.contains("decompressed stream"));
    }

    #[test]
    fn test_generation_mismatch_text_refuses_before_raw_and_points_at_search() {
        let v = json!({
            "status": "generation_mismatch",
            "requested_generation": "gen-old",
            "active_generation": "gen-new",
            "index": "index-canary/ase.index",
            "content": "PROSE_CANARY",
            "path": "journal-canary.jsonl"
        });
        let text = render_generation_mismatch(&v);
        assert!(text.contains("open: status=generation_mismatch"));
        assert!(text.contains("requested_generation=gen-old"));
        assert!(text.contains("active_generation=gen-new"));
        assert!(text.contains("refused before reading raw"));
        assert!(text.contains("search again on the intended --index"));
        assert!(!text.contains("index-canary"));
        assert!(!text.contains("PROSE_CANARY"));
        assert!(!text.contains("journal-canary.jsonl"));
    }

    #[test]
    fn test_raw_read_text_error_omits_source_path() {
        assert!(RAW_READ_TEXT_ERROR.contains("cannot read the pinned raw source"));
        assert!(RAW_READ_TEXT_ERROR.contains("re-index and search again"));
        assert!(!RAW_READ_TEXT_ERROR.contains("journal-canary.jsonl"));
        assert!(!RAW_READ_TEXT_ERROR.contains('\\'));
    }
}
