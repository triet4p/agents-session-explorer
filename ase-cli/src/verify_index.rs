//! `ase verify` on the binary address, with the bundle route handed off.
//!
//! Both forms stay on the same command: a positional bundle directory opens
//! the evidence-bundle verifier, and `--all` / `--sample N` re-read pinned
//! address spans.

use crate::cli_option::{has, opt, positional};
use crate::cli_output::outln;
use crate::index::load;
use crate::verify;
use crate::show::span_label;
use ase_core::{err, Result};
use ase_ingest::util::fnv64;
use serde_json::json;
use std::time::Instant;


pub fn run(args: &[String]) -> Result<()> {
    if let Some(bundle) = positional(args) {
        return verify::run_bundle(&bundle, args);
    }
    let all = has(args, "--all");
    let sample: Option<u64> = opt(args, "--sample").and_then(|s| s.parse().ok());
    if !all && sample.is_none() {
        return Err(err("verify needs --all or --sample N"));
    }
    let (idx, store) = load(args)?;
    let total = idx.event_count as u64;
    let targets: Vec<u64> = if all {
        (0..total).collect()
    } else {
        let n = sample.unwrap().min(total);
        (0..n).map(|k| k * total / n.max(1)).collect()
    };
    let started = Instant::now();
    let mut ok = 0u64;
    let mut failed = 0u64;
    let mut bytes_read = 0u64;
    let mut failures = Vec::new();
    for t in &targets {
        let i = *t as u32;
        let e = idx.event(i)?;
        let span = e.span();
        let bytes = store.span_cached(span)?;
        bytes_read += bytes.len() as u64;
        let hash_ok = fnv64(&bytes) == e.hash();
        let parses = serde_json::from_slice::<&serde_json::value::RawValue>(&bytes).is_ok();
        let good = hash_ok && parses == e.valid_json();
        if good {
            ok += 1;
        } else {
            failed += 1;
            if failures.len() < 20 {
                failures.push(json!({
                    "event": i,
                    "span": span_label(span),
                    "hash_ok": hash_ok,
                    "json_ok": parses,
                    "expected_json": e.valid_json()
                }));
            }
        }
    }
    let v = json!({
        "command": "verify",
        "mode": if all { "all" } else { "sample" },
        "requested": targets.len(),
        "ok": ok,
        "failed": failed,
        "bytes_re_read": bytes_read,
        "elapsed_ms": started.elapsed().as_millis(),
        "coverage": if all { 1.0 } else { targets.len() as f64 / total as f64 },
        "failures": failures
    });
    if has(args, "--json") {
        outln!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        outln!(
            "verify: {}/{} spans ok ({} failed), {} bytes re-read, {} ms, coverage {:.4}",
            ok, targets.len(), failed, bytes_read, started.elapsed().as_millis(),
            v["coverage"].as_f64().unwrap_or(0.0)
        );
    }
    if failed > 0 {
        std::process::exit(3);
    }
    Ok(())
}
