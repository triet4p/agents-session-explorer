//! Versioned boundary-rule shingle views over one pinned publication.

use crate::cli_option::opt;
use crate::cli_output::emit;
use crate::index::require_publication;
use ase_core::{err, Result};
use ase_postings::{shingles_by_event, shingles_by_token};
use serde_json::{json, Value};


pub fn run(args: &[String]) -> Result<()> {
    let (_, _, published) = require_publication(args)?;
    let hash = opt(args, "--hash");
    let event = opt(args, "--event").and_then(|s| s.parse::<u32>().ok());
    let rows = match (hash, event) {
        (Some(h), None) => shingles_by_token(&published, &h)?,
        (None, Some(e)) => shingles_by_event(&published, e)?,
        _ => return Err(err("shingle needs exactly one of --hash HASH or --event N")),
    };
    let rendered: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "k": r.k,
                "token": r.token,
                "event": r.event,
                "end_event": r.end_event,
                "file": r.file,
                "session_local": r.session_local,
                "parent_idx": r.parent_idx,
                "selector_from": r.selector_from,
                "selector_to": r.selector_to,
                "constituents": serde_json::from_str::<Value>(&r.constituents).unwrap_or(json!([]))
            })
        })
        .collect();
    let v = json!({
        "command": "shingle",
        "status": "ok",
        "generation": published.generation,
        "count": rendered.len(),
        "boundary_rule": "k=3..5 windows over a same-session explicit parent chain; no cross-file, cross-session, fork-sibling or unresolved-parent join; ambiguous parents are abstained",
        "shingles": rendered
    });
    emit(&v, args);
    Ok(())
}
