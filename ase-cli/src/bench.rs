//! In-process benchmark views for address spans and derived search.
//!
//! Both views load their sources once, report counts and deterministic
//! evidence checks, and leave timing values as the only measured fields.

use crate::cli_option::{has, opt};
use crate::cli_output::emit;
use crate::index::{load, require_publication};
use crate::open::hash_bytes;
use ase_core::{err, RawStore, Result};
use ase_ingest::util::fnv64;
use ase_postings::search;
use serde_json::json;
use std::time::Instant;


fn percentile(sorted: &[u128], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let pos = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[pos] as f64 / 1000.0
}

pub fn run_show(args: &[String]) -> Result<()> {
    let (idx, store) = load(args)?;
    let n: usize = opt(args, "--n").and_then(|s| s.parse().ok()).unwrap_or(200);
    let mut seed: u64 = opt(args, "--seed").and_then(|s| s.parse().ok()).unwrap_or(1);
    let mut pool: Vec<u32> = Vec::new();
    for fid in 0..idx.file_count {
        if idx.file(fid)?.provider() == ase_core::Provider::Omp {
            let f = idx.file(fid)?;
            pool.extend(f.event_start()..f.event_end());
        }
    }
    if pool.is_empty() {
        return Err(err("no omp events to benchmark"));
    }
    let mut times: Vec<u128> = Vec::with_capacity(n);
    let mut bytes = 0u64;
    for _ in 0..n {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let pick = pool[(seed >> 33) as usize % pool.len()];
        let t = Instant::now();
        let e = idx.event(pick)?;
        let span = e.span();
        let raw = store.span(span)?;
        let _hash_ok = fnv64(&raw) == e.hash();
        times.push(t.elapsed().as_nanos());
        bytes += raw.len() as u64;
    }
    times.sort_unstable();
    let q = |p: f64| -> u128 { times[((times.len() as f64 - 1.0) * p).round() as usize] };
    let v = json!({
        "command": "bench-show",
        "source": "omp",
        "n": n,
        "pool_size": pool.len(),
        "indexed_events": idx.event_count,
        "seed": opt(args, "--seed").and_then(|s| s.parse::<u64>().ok()).unwrap_or(1),
        "p50_us": q(0.50) as f64 / 1000.0,
        "p90_us": q(0.90) as f64 / 1000.0,
        "p99_us": q(0.99) as f64 / 1000.0,
        "max_us": *times.last().unwrap_or(&0) as f64 / 1000.0,
        "bytes_read": bytes,
        "includes": "coordinate lookup + raw span read + checksum (in-process, no process startup)"
    });
    if has(args, "--json") {
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        println!(
            "bench-show omp n={} p50={:.1}us p90={:.1}us p99={:.1}us max={:.1}us",
            n, v["p50_us"], v["p90_us"], v["p99_us"], v["max_us"]
        );
    }
    Ok(())
}

pub fn run_search(args: &[String]) -> Result<()> {
    let (idx, store, published) = require_publication(args)?;
    let n: usize = opt(args, "--n").and_then(|s| s.parse().ok()).unwrap_or(2000);
    let seed: u64 = opt(args, "--seed").and_then(|s| s.parse().ok()).unwrap_or(1);
    let top_k: u32 = opt(args, "--top-k").and_then(|s| s.parse().ok()).unwrap_or(10).clamp(1, 1000);
    let source = opt(args, "--source").unwrap_or_else(|| "all".to_string());
    if !matches!(source.as_str(), "all" | "jsonl" | "zstd") {
        return Err(err("--source must be all, jsonl or zstd"));
    }
    let favour = opt(args, "--favour");
    let common = favour.unwrap_or_else(|| "the".to_string());
    let phrase = format!("{common} function");
    let rare = "quartzy".to_string();
    let absent = "asebenchabsent7b2f9c".to_string();
    let probes: Vec<(String, bool, String)> = vec![
        (common.clone(), false, "common".to_string()),
        (phrase, true, "phrase".to_string()),
        (rare.clone(), false, "rare".to_string()),
        (absent.clone(), false, "absent".to_string()),
        (common, false, "common".to_string()),
        (rare, false, "rare".to_string()),
    ];
    let mut lcg = seed;
    let mut lookup = Vec::with_capacity(n);
    let mut reopen = Vec::with_capacity(n / 10 + 1);
    let mut hits_total = 0u64;
    let mut by_class: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    let mut query_counts: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    let mut reopen_span_ok = 0u64;
    let mut reopen_block_ok = 0u64;
    let mut reopen_both_ok = 0u64;
    let mut reopen_zstd = 0u64;
    let mut reopen_jsonl = 0u64;
    for i in 0..n {
        lcg = lcg.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let pick = (lcg >> 33) as usize % probes.len();
        let (query, is_phrase, class) = &probes[pick];
        *query_counts.entry(class.clone()).or_default() += 1;
        let t0 = Instant::now();
        let hits = search(&published, &idx, query, *is_phrase, top_k)?;
        lookup.push(t0.elapsed().as_nanos());
        hits_total += hits.len() as u64;
        *by_class.entry(class.clone()).or_default() += hits.len() as u64;
        if i % 10 == 0 {
            if let Some(h) = hits.iter().find(|h| match source.as_str() {
                "jsonl" => h.encoding == ase_core::Encoding::Jsonl,
                "zstd" => h.encoding == ase_core::Encoding::JsonlZstd,
                _ => true,
            }) {
                let t1 = Instant::now();
                let raw = store.span(h.span)?;
                let ev = idx.event(h.event)?;
                let span_ok = fnv64(&raw) == ev.hash();
                let provider = idx.file(h.span.file)?.provider();
                let block_ok = ase_ingest::extract_block(provider, &raw, &h.selector)
                    .map(|e| hash_bytes(&h.block_hash) == Some(e.block_hash))
                    .unwrap_or(false);
                reopen.push(t1.elapsed().as_nanos());
                if span_ok {
                    reopen_span_ok += 1;
                }
                if block_ok {
                    reopen_block_ok += 1;
                }
                if span_ok && block_ok {
                    reopen_both_ok += 1;
                }
                if h.encoding == ase_core::Encoding::JsonlZstd {
                    reopen_zstd += 1;
                } else {
                    reopen_jsonl += 1;
                }
            }
        }
    }
    lookup.sort_unstable();
    reopen.sort_unstable();
    let v = json!({
        "command": "bench-search",
        "generation": published.generation,
        "n": n,
        "seed": seed,
        "top_k": top_k,
        "source_filter": source,
        "lookup_scope": "all sources, unrestricted",
        "source_filter_scope": "evidence_reopen_hit_selection_only",
        "query_classes": ["common", "phrase", "rare", "absent"],
        "query_counts": query_counts,
        "query_text_retained": false,
        "hits_aggregate": hits_total,
        "hits_by_class": by_class,
        "lookup": {
            "samples": lookup.len(),
            "p50_us": percentile(&lookup, 0.50),
            "p99_us": percentile(&lookup, 0.99),
            "max_us": percentile(&lookup, 1.0),
            "includes": "in-process FTS5 MATCH + rowid-to-block join + pinned-generation filter; no process startup"
        },
        "evidence_reopen": {
            "samples": reopen.len(),
            "jsonl_samples": reopen_jsonl,
            "zstd_samples": reopen_zstd,
            "span_verified": reopen_span_ok,
            "block_verified": reopen_block_ok,
            "both_verified": reopen_both_ok,
            "p50_us": percentile(&reopen, 0.50),
            "p99_us": percentile(&reopen, 0.99),
            "max_us": percentile(&reopen, 1.0),
            "includes": "raw span read + span checksum + selector re-derivation + block digest compare",
            "sampling": "every tenth query whose first eligible hit matches --source"
        },
        "cache_conditions": "publication digests and address are loaded once before the loop; FsRawStore caches at most four decoded zstd files",
        "published_footprint_bytes": published.pointer.full_footprint_bytes
    });
    emit(&v, args);
    Ok(())
}
