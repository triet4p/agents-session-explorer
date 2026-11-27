//! Derived prose search over one pinned publication.
//!
//! A missing or mismatched layer is reported as unavailable, never as zero
//! hits. No prose is stored in the derived layer: hits carry coordinates and
//! selectors that `open` re-derives from the raw address.

use crate::cli_option::{has, opt, positional};
use crate::cli_output::emit;
use crate::index::{index_path, load_with_publication};
use crate::show::{freshness_json, span_label};
use ase_core::{err, Result};
use ase_index::sha256_hex;
use ase_postings::search;
use serde_json::{json, Value};


/// Flag combinations that make the metadata-only text report ambiguous.
fn conflicting_text_flag(args: &[String]) -> Option<&'static str> {
    if !has(args, "--text") {
        return None;
    }
    ["--json", "--open-args", "--explain"]
        .into_iter()
        .find(|flag| has(args, flag))
}

fn search_usage() {
    println!(
        "ase search <QUERY> [--index FILE] [--phrase] [--top-k N] [--text]\n\
         [--explain] [--open-args] [--json]\n\
         --text prints the hit count, full generation and metadata-only hit lines; it excludes\n\
         --json, --open-args and --explain. No query, prose, snippet or source path is printed."
    );
}

/// Human-readable counterpart of the JSON hit list. Rank is display order only;
/// the reopen keys come from the pinned header and each hit line.
fn render_text(generation: &str, index: &str, hits: &[Value]) -> String {
    let mut out = format!(
        "search: {} hit(s), generation={}, index={}\n",
        hits.len(),
        generation,
        index
    );
    for (rank, hit) in hits.iter().enumerate() {
        out.push_str(&format!(
            "{}. role={} span={} freshness={} event={} selector={}\n",
            rank + 1,
            hit["role"].as_str().unwrap_or(""),
            hit["span"].as_str().unwrap_or(""),
            hit["freshness"].as_str().unwrap_or(""),
            hit["event"].as_u64().unwrap_or_default(),
            hit["selector"].as_str().unwrap_or("")
        ));
    }
    out.push_str("note: the leading number is display order only; reopen by event, selector, generation and index");
    out
}

/// Text-mode status for an opened address with no usable postings layer.
fn unavailable_text() -> [&'static str; 4] {
    [
        "search: layer_unavailable",
        "no search result was produced; this is not a zero-hit report",
        "check --index; if postings were not published, rebuild with --layer all and both --omp-root and --codex-root",
        "note: the address layer remains usable",
    ]
}

fn unavailable_value(command: &str, reason: &str) -> Value {
    json!({
        "command": command,
        "status": "layer_unavailable",
        "reason": reason,
        "address_usable": true,
        "hits": [],
        "explain": [
            "The address index was opened independently of the derived layer.",
            "A missing, tombstoned or mismatched sidecar is never reported as zero hits.",
            "show / resolve / verify remain available against the binary address."
        ]
    })
}

/// Ready-to-use `open` arguments for one hit. Only the four pinned coordinates
/// and the exact index path are emitted: no query, prose, or source path.
fn open_args(event: u32, selector: &str, generation: &str, index: &str) -> Value {
    json!({
        "event": event,
        "selector": selector,
        "generation": generation,
        "index": index,
        "flags": [
            "--event", event.to_string(),
            "--selector", selector,
            "--generation", generation,
            "--index", index
        ]
    })
}

pub fn run(args: &[String]) -> Result<()> {
    if has(args, "--help") || has(args, "-h") {
        search_usage();
        return Ok(());
    }
    if let Some(flag) = conflicting_text_flag(args) {
        return Err(err(format!(
            "--text cannot be combined with {flag}; see `ase search --help`"
        )));
    }
    let query = positional(args).ok_or_else(|| err("search needs a query"))?;
    let (idx, _, published, reason) = load_with_publication(args)?;
    let Some(published) = published else {
        if has(args, "--text") {
            for line in unavailable_text() {
                println!("{line}");
            }
        } else {
            let v = unavailable_value("search", &reason.unwrap_or_else(|| "unknown".to_string()));
            emit(&v, args);
        }
        std::process::exit(4);
    };
    let top_k: u32 = opt(args, "--top-k")
        .and_then(|s| s.parse().ok())
        .unwrap_or(10)
        .clamp(1, 1000);
    let phrase = has(args, "--phrase");
    let with_open_args = has(args, "--open-args");
    let index = index_path(args).to_string_lossy().into_owned();
    let hits = search(&published, &idx, &query, phrase, top_k)?;
    let rendered: Vec<Value> = hits
        .iter()
        .map(|h| {
            let fresh = freshness_json(&idx, h.span.file);
            let mut hit_value = json!({
                "event": h.event,
                "span": span_label(h.span),
                "selector": h.selector,
                "role": h.role,
                "session_local": h.session_local,
                "parent_idx": h.parent_idx,
                "block_hash": h.block_hash,
                "score": h.score,
                "encoding": h.encoding.label(),
                "coordinate": h.encoding.coordinate(),
                "freshness": fresh["status"]
            });
            if with_open_args {
                hit_value["open_args"] =
                    open_args(h.event, &h.selector, &published.generation, &index);
            }
            hit_value
        })
        .collect();
    if has(args, "--text") {
        println!("{}", render_text(&published.generation, &index, &rendered));
        return Ok(());
    }
    let query_hash = sha256_hex(query.as_bytes());
    let mut explain = vec![
        format!("pinned generation {}", published.generation),
        "FTS5 contentless prose index; role allowlist user/assistant; thinking and tool output excluded".to_string(),
        "ranking: FTS5 bm25 ascending, tie-break by posting rowid; top-k fixed".to_string(),
        "no prose or snippet is stored or returned; open the selector from raw bytes".to_string(),
        format!("query digest {}", &query_hash[..12]),
    ];
    if phrase {
        explain.push("query treated as one phrase".to_string());
    } else {
        explain.push("query terms combined with AND after unicode61 tokenization".to_string());
    }
    let v = json!({
        "command": "search",
        "status": "ok",
        "generation": published.generation,
        "top_k": top_k,
        "count": rendered.len(),
        "hits": rendered,
        "explain": if has(args, "--explain") { explain } else { Vec::<String>::new() }
    });
    emit(&v, args);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{conflicting_text_flag, open_args, render_text, unavailable_text};
    use serde_json::json;

    #[test]
    fn test_open_args_pin_one_hit_without_query_or_source_text() {
        let v = open_args(7, "block-v1|test", "gen-1", "work/ase.index");
        assert_eq!(v["event"], json!(7));
        assert_eq!(v["selector"], json!("block-v1|test"));
        assert_eq!(v["generation"], json!("gen-1"));
        assert_eq!(v["index"], json!("work/ase.index"));
        assert_eq!(
            v["flags"],
            json!([
                "--event", "7",
                "--selector", "block-v1|test",
                "--generation", "gen-1",
                "--index", "work/ase.index"
            ])
        );
        let encoded = v.to_string();
        assert!(!encoded.contains("QUERY_CANARY"));
        assert!(!encoded.contains("PROSE_CANARY"));
        assert!(!encoded.contains("synthetic.jsonl"));
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn test_text_conflicts_are_reported_before_index_access() {
        for flag in ["--json", "--open-args", "--explain"] {
            assert_eq!(
                conflicting_text_flag(&args(&["TERM", "--text", flag])),
                Some(flag)
            );
        }
        assert_eq!(
            conflicting_text_flag(&args(&[
                "TERM", "--text", "--phrase", "--top-k", "3", "--index",
                "dir with space/ase.index"
            ])),
            None
        );
        assert_eq!(
            conflicting_text_flag(&args(&["TERM", "--json", "--open-args"])),
            None
        );
    }

    #[test]
    fn test_text_render_prints_ordered_metadata_without_query_prose_or_source_path() {
        let hits = vec![
            json!({
                "event": 7,
                "span": "span:0:12:34",
                "selector": "block-v1|first",
                "role": "user",
                "freshness": "fresh",
                "query": "QUERY_CANARY",
                "content": "PROSE_CANARY",
                "path": "journal-canary.jsonl"
            }),
            json!({
                "event": 9,
                "span": "span:1:56:78",
                "selector": "block-v1|second",
                "role": "assistant",
                "freshness": "stale"
            }),
        ];
        let text = render_text("gen-0123456789abcdef", "dir with space/ase.index", &hits);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            "search: 2 hit(s), generation=gen-0123456789abcdef, index=dir with space/ase.index"
        );
        assert_eq!(
            lines[1],
            "1. role=user span=span:0:12:34 freshness=fresh event=7 selector=block-v1|first"
        );
        assert_eq!(
            lines[2],
            "2. role=assistant span=span:1:56:78 freshness=stale event=9 selector=block-v1|second"
        );
        assert!(text.contains("display order only"));
        assert!(!text.contains("QUERY_CANARY"));
        assert!(!text.contains("PROSE_CANARY"));
        assert!(!text.contains("journal-canary.jsonl"));
        assert!(!text.contains("& $ase"));
        assert!(!text.contains("ConvertFrom-Json"));
    }

    #[test]
    fn test_unavailable_text_never_claims_zero_hits() {
        let lines = unavailable_text();
        assert_eq!(lines[0], "search: layer_unavailable");
        assert!(lines.iter().any(|line| line.contains("--index")));
        assert!(lines.iter().any(|line| line.contains("--layer all")));
        assert!(lines
            .iter()
            .any(|line| line.contains("--omp-root") && line.contains("--codex-root")));
        assert!(lines.iter().all(|line| !line.contains("0 hit")));
    }
}
