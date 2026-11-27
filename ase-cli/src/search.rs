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
    let query = positional(args).ok_or_else(|| err("search needs a query"))?;
    let (idx, _, published, reason) = load_with_publication(args)?;
    let Some(published) = published else {
        let v = unavailable_value("search", &reason.unwrap_or_else(|| "unknown".to_string()));
        emit(&v, args);
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
    let query_hash = sha256_hex(query.as_bytes());
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
    use super::open_args;
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
}
