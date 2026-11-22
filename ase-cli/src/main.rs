//! P0a address CLI plus the P0b derived search layer. All decoding,
//! addressing and projection derivation lives in library crates; the CLI only
//! renders metadata. No model is on any query path.

use ase_core::{err, EventIdx, RawStore, Result, Role, SpanRef, NONE};
use ase_index::{
    build, decode_corpus, sha256_hex, write_address, DecodedCorpus, FsRawStore, Index,
};
use ase_ingest::util::fnv64;
use ase_postings::{
    block_by_selector, publication, search, shingles_by_event, shingles_by_token, Publication,
    WriterInput,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::fs;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_INDEX: &str = "ase.index";
const DEFAULT_OMP: &str = "C:/Users/admin/.omp/agent/sessions";
const DEFAULT_CODEX: &str = "C:/Users/admin/.codex/sessions";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        usage();
        return;
    }
    let rest = &args[1..];
    let result = match args[0].as_str() {
        "index" => cmd_index(rest),
        "show" => cmd_show(rest),
        "resolve" => cmd_resolve(rest),
        "verify" => cmd_verify(rest),
        "bench-show" => cmd_bench_show(rest),
        "fixture" => cmd_fixture(rest),
        "fixture-p0b" => cmd_fixture_p0b(rest),
        "search" => cmd_search(rest),
        "open" => cmd_open(rest),
        "shingle" => cmd_shingle(rest),
        "bench-search" => cmd_bench_search(rest),
        other => Err(err(format!("unknown command: {other}"))),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(2);
    }
}

fn usage() {
    println!(
        "ase (P0a address / P0b derived search)\n\n\
         ase index   [--layer address|postings|all] [--omp-root DIR] [--codex-root DIR]\n\
                     [--index FILE] [--fail-before-publish] [--json]\n\
         ase show    <span:F:L:LEN|session:ID> [--role ROLE] [--ordinal A:B] [--explain] [--content] [--json]\n\
         ase resolve <session:ID|span:F:L:LEN|task:TOKEN> [--explain] [--json]\n\
         ase verify  [--all | --sample N] [--index FILE] [--json]\n\
         ase bench-show [--n N] [--seed N] [--index FILE] [--json]\n\
         ase search  <QUERY> [--phrase] [--top-k N] [--explain] [--json]\n\
         ase open    --event N --selector S --generation G [--content] [--json]\n\
         ase shingle (--hash HASH | --event N) [--json]\n\
         ase bench-search [--n N] [--seed N] [--top-k N] [--source all|jsonl|zstd] [--probe TERM] [--json]\n\
         ase fixture --out DIR\n\
         ase fixture-p0b --out DIR"
    );
}

fn index_usage() {
    println!(
        "ase index [--layer address|postings|all] [--omp-root DIR] [--codex-root DIR]\n\
         [--index FILE] [--fail-before-publish] [--json]"
    );
}

fn positional(args: &[String]) -> Option<String> {
    let mut expecting = false;
    for a in args {
        if a.starts_with("--") {
            expecting = true;
            continue;
        }
        if !expecting {
            return Some(a.clone());
        }
        expecting = false;
    }
    None
}

fn opt(args: &[String], name: &str) -> Option<String> {
    for (i, a) in args.iter().enumerate() {
        if a == name {
            return args.get(i + 1).cloned();
        }
    }
    None
}

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn index_path(args: &[String]) -> PathBuf {
    PathBuf::from(opt(args, "--index").unwrap_or_else(|| DEFAULT_INDEX.to_string()))
}

fn load_with_publication(
    args: &[String],
) -> Result<(Arc<Index>, FsRawStore, Option<Publication>, Option<String>)> {
    let p = index_path(args);
    let (published, reason) = match publication::open(&p) {
        Ok(Some(published)) => (Some(published), None),
        Ok(None) => (None, Some("sidecar_missing".to_string())),
        Err(e) => (None, Some(e.to_string())),
    };
    let address = published
        .as_ref()
        .map(|v| v.address_path.clone())
        .unwrap_or(p);
    let idx = Arc::new(Index::open(&address)?);
    let store = FsRawStore::new(idx.clone());
    Ok((idx, store, published, reason))
}

fn load(args: &[String]) -> Result<(Arc<Index>, FsRawStore)> {
    let (idx, store, _, _) = load_with_publication(args)?;
    Ok((idx, store))
}

fn opt_idx(v: u32) -> Option<u32> {
    if v == NONE { None } else { Some(v) }
}

fn span_label(s: SpanRef) -> String {
    format!("span:{}:{}:{}", s.file, s.off, s.len)
}

fn parse_span(s: &str) -> Option<SpanRef> {
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

// ---------------------------------------------------------------- commands

fn versions_map() -> std::collections::BTreeMap<String, String> {
    let mut v = std::collections::BTreeMap::new();
    v.insert("projection".to_string(), ase_core::PROJECTION_VERSION.to_string());
    v.insert("callsig".to_string(), ase_core::CALLSIG_VERSION.to_string());
    v.insert("allowlist".to_string(), ase_core::ALLOWLIST_VERSION.to_string());
    v.insert("tokenizer".to_string(), ase_core::TOKENIZER_VERSION.to_string());
    v.insert("shingle".to_string(), ase_core::SHINGLE_VERSION.to_string());
    v
}

fn relative_path(base: &Path, target: &Path) -> Result<String> {
    let rel = target
        .strip_prefix(base)
        .map_err(|_| err("artifact is outside the publication directory"))?;
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

fn validate_address_corpus(idx: &Index, corpus: &DecodedCorpus) -> Result<()> {
    if idx.file_count as usize != corpus.files.len()
        || idx.event_count as usize != corpus.total_events
    {
        return Err(err("address and decoded corpus counts differ"));
    }
    for (id, file) in corpus.files.iter().enumerate() {
        let view = idx.file(id as u32)?;
        if view.size() != file.size || view.mtime() != (file.mtime_secs, file.mtime_nanos) {
            return Err(err("address file watermark differs from decoded source"));
        }
    }
    for (file_idx, decoded) in corpus.decoded.iter().enumerate() {
        let base = corpus.bases[file_idx];
        for (ordinal, event) in decoded.events.iter().enumerate() {
            let e = idx.event(base + ordinal as u32)?;
            if e.span() != event.span || e.hash() != event.hash {
                return Err(err("address event bytes differ from decoded source"));
            }
        }
    }
    Ok(())
}

fn cmd_index(args: &[String]) -> Result<()> {
    if has(args, "--help") || has(args, "-h") {
        index_usage();
        return Ok(());
    }
    let out = index_path(args);
    let omp = PathBuf::from(opt(args, "--omp-root").unwrap_or_else(|| DEFAULT_OMP.to_string()));
    let codex = PathBuf::from(opt(args, "--codex-root").unwrap_or_else(|| DEFAULT_CODEX.to_string()));
    let layer = opt(args, "--layer").unwrap_or_else(|| "address".to_string());
    if !matches!(layer.as_str(), "address" | "postings" | "all") {
        return Err(err("--layer must be address, postings or all"));
    }
    if layer == "address" {
        let stats = build(&omp, &codex, &out)?;
        publication::tombstone(&out, "address_layer_only")?;
        let v = json!({
            "command": "index", "layer": "address", "publication": null,
            "index": out.to_string_lossy(),
            "files": stats.files, "omp_files": stats.omp_files,
            "codex_files": stats.codex_files, "zstd_files": stats.zstd_files,
            "events": stats.events, "sessions": stats.sessions,
            "index_bytes": stats.index_bytes,
            "malformed_lines_preserved": stats.malformed,
            "unknown_type_events": stats.unknown_type,
            "unresolved_parent_refs": stats.unresolved_parent,
            "unresolved_call_links": stats.unresolved_link,
            "multi_toolcall_entries": stats.multi_call_entries,
            "duration_ms": stats.duration_ms
        });
        print_index_json(&v, stats.files, stats.omp_files, stats.codex_files,
            stats.zstd_files, stats.events, stats.sessions, stats.index_bytes,
            stats.duration_ms, stats.malformed, stats.unknown_type,
            stats.unresolved_parent, stats.unresolved_link, stats.multi_call_entries,
            has(args, "--json"));
        return Ok(());
    }

    let started = Instant::now();
    let corpus = decode_corpus(&omp, &codex)?;
    let parent = out.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|e| err(format!("create {}: {e}", parent.display())))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        ^ (std::process::id() as u128);

    let (address_bytes, address_sha, staged_address) = if layer == "postings" {
        let idx = Index::open(&out)?;
        validate_address_corpus(&idx, &corpus)?;
        let (bytes, sha) = publication::sha256_file(&out)?;
        (bytes, sha, out.clone())
    } else {
        let stage = parent.join(format!(".stage-{nonce}-address.bin"));
        write_address(&corpus, &stage)?;
        let (bytes, sha) = publication::sha256_file(&stage)?;
        (bytes, sha, stage)
    };

    let versions = versions_map();
    let files: Vec<publication::FileWatermark> = corpus
        .files
        .iter()
        .enumerate()
        .map(|(id, f)| publication::FileWatermark {
            id: id as u32,
            provider: f.provider.label().to_string(),
            encoding: f.encoding.label().to_string(),
            size: f.size,
            mtime_secs: f.mtime_secs,
            mtime_nanos: f.mtime_nanos,
            digest: corpus.file_digests[id].clone(),
            events: corpus.decoded[id].events.len() as u32,
            event_start: corpus.bases[id],
            event_end: corpus.bases[id] + corpus.decoded[id].events.len() as u32,
        })
        .collect();
    let counts: std::collections::BTreeMap<String, u64> = [
        ("files", corpus.files.len() as u64),
        ("events", corpus.total_events as u64),
        ("sessions", corpus.stats.sessions as u64),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let core = json!({
        "schema": publication::SCHEMA,
        "versions": versions,
        "counts": counts,
        "files": files,
    });
    let core_digest = sha256_hex(core.to_string().as_bytes());
    let generation = sha256_hex(format!("{core_digest}:{nonce}").as_bytes());

    let staged_db = parent.join(format!(".stage-{nonce}-postings.sqlite"));
    let writer_stats = ase_postings::write_postings(
        WriterInput {
            corpus: &corpus,
            generation: &generation,
            core_digest: &core_digest,
            address_bytes,
            address_sha256: &address_sha,
        },
        &staged_db,
    )?;
    let (postings_bytes, postings_sha) = publication::sha256_file(&staged_db)?;
    let staged_manifest = parent.join(format!(".stage-{nonce}-manifest.json"));
    let manifest = publication::Manifest {
        magic: publication::MANIFEST_MAGIC.to_string(),
        schema: publication::SCHEMA,
        generation: generation.clone(),
        core_digest: core_digest.clone(),
        versions: versions.clone(),
        counts: [
            ("files", corpus.files.len() as u64),
            ("events", corpus.total_events as u64),
            ("sessions", corpus.stats.sessions as u64),
            ("blocks", writer_stats.blocks),
            ("prose_blocks", writer_stats.prose_blocks),
            ("call_blocks", writer_stats.call_blocks),
            ("fts_rows", writer_stats.fts_rows),
            ("shingles", writer_stats.shingles),
            ("shingle_abstentions", writer_stats.abstentions),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect(),
        files,
        address_bytes,
        address_sha256: address_sha.clone(),
        postings_bytes,
        postings_sha256: postings_sha.clone(),
    };
    let manifest_value = serde_json::to_value(&manifest).map_err(|e| err(format!("manifest: {e}")))?;
    fs::write(&staged_manifest, serde_json::to_vec_pretty(&manifest_value).unwrap())
        .map_err(|e| err(format!("write manifest: {e}")))?;
    let (manifest_bytes, manifest_sha) = publication::sha256_file(&staged_manifest)?;

    let gen_dir = parent.join("generations").join(&generation);
    fs::create_dir_all(&gen_dir).map_err(|e| err(format!("create {}: {e}", gen_dir.display())))?;
    let final_postings = gen_dir.join("postings.sqlite");
    let final_manifest = gen_dir.join("manifest.json");
    fs::copy(&staged_db, &final_postings)
        .map_err(|e| err(format!("stage postings: {e}")))?;
    fs::copy(&staged_manifest, &final_manifest)
        .map_err(|e| err(format!("stage manifest: {e}")))?;
    // Both layers pin an immutable address copy. A later address-only rebuild
    // of the legacy --index input must never change this generation.
    let final_address = gen_dir.join("address.bin");
    let address_source = if layer == "all" { &staged_address } else { &out };
    fs::copy(address_source, &final_address)
        .map_err(|e| err(format!("stage address: {e}")))?;
    let address_rel = relative_path(parent, &final_address)?;
    let address_target = final_address;
    let postings_rel = relative_path(parent, &final_postings)?;
    let manifest_rel = relative_path(parent, &final_manifest)?;
    let artifact = |path: String, bytes: u64, sha: String| publication::ArtifactRef {
        path,
        bytes,
        sha256: sha,
    };
    let pointer = publication::Pointer {
        magic: publication::POINTER_MAGIC.to_string(),
        schema: publication::SCHEMA,
        status: "ready".to_string(),
        reason: String::new(),
        generation: generation.clone(),
        address: artifact(address_rel, address_bytes, address_sha),
        postings: artifact(postings_rel, postings_bytes, postings_sha),
        manifest: artifact(manifest_rel, manifest_bytes, manifest_sha.clone()),
        full_footprint_bytes: address_bytes + postings_bytes + manifest_bytes,
        versions,
        counts: manifest.counts.clone(),
    };
    let checked = Index::open(&address_target)?;
    if checked.event_count as u64 != *pointer.counts.get("events").unwrap_or(&u64::MAX) {
        return Err(err("staged address count differs from manifest"));
    }
    if has(args, "--fail-before-publish") {
        let _ = fs::remove_file(&staged_db);
        let _ = fs::remove_file(&staged_manifest);
        if layer == "all" {
            let _ = fs::remove_file(&staged_address);
        }
        let _ = fs::remove_dir_all(&gen_dir);
        return Err(err("fault injected before publication; pointer unchanged and owned staging removed"));
    }
    publication::write_pointer(&out, &pointer)?;
    let _ = fs::remove_file(&staged_db);
    if layer == "all" {
        let _ = fs::remove_file(&staged_address);
    }
    let _ = fs::remove_file(&staged_manifest);
    let sidecar_bytes = fs::metadata(publication::sidecar_path(&out)).map(|m| m.len()).unwrap_or(0);
    let v = json!({
        "command": "index",
        "layer": layer,
        "index": out.to_string_lossy(),
        "files": corpus.files.len(),
        "omp_files": corpus.files.iter().filter(|f| f.provider == ase_core::Provider::Omp).count(),
        "codex_files": corpus.files.iter().filter(|f| f.provider == ase_core::Provider::Codex).count(),
        "zstd_files": corpus.files.iter().filter(|f| f.encoding == ase_core::Encoding::JsonlZstd).count(),
        "events": corpus.total_events,
        "sessions": corpus.stats.sessions,
        "index_bytes": address_bytes,
        "malformed_lines_preserved": corpus.stats.malformed,
        "unknown_type_events": corpus.stats.unknown_type,
        "unresolved_parent_refs": corpus.stats.unresolved_parent,
        "unresolved_call_links": corpus.stats.unresolved_link,
        "multi_toolcall_entries": corpus.stats.multi_call_entries,
        "duration_ms": started.elapsed().as_millis(),
        "publication": {
            "status": "ready",
            "generation": generation,
            "address_bytes": address_bytes,
            "postings_bytes": postings_bytes,
            "manifest_bytes": manifest_bytes,
            "sidecar_bytes": sidecar_bytes,
            "full_footprint_bytes": pointer.full_footprint_bytes,
            "blocks": writer_stats.blocks,
            "prose_blocks": writer_stats.prose_blocks,
            "call_blocks": writer_stats.call_blocks,
            "fts_rows": writer_stats.fts_rows,
            "shingles": writer_stats.shingles,
            "shingle_abstentions": writer_stats.abstentions
        }
    });
    if has(args, "--json") {
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        println!(
            "indexed layer {} with {} files / {} events; published generation {}",
            layer, corpus.files.len(), corpus.total_events, v["publication"]["generation"]
        );
        println!(
            "address={} postings={} manifest={} sidecar={} blocks={} shingles={}",
            address_bytes, postings_bytes, manifest_bytes, sidecar_bytes,
            writer_stats.blocks, writer_stats.shingles
        );
    }
    Ok(())
}

fn print_index_json(
    v: &Value,
    files: usize,
    omp: usize,
    codex: usize,
    zstd: usize,
    events: usize,
    sessions: usize,
    bytes: u64,
    duration_ms: u128,
    malformed: usize,
    unknown: usize,
    parent: usize,
    link: usize,
    multi: usize,
    json: bool,
) {
    if json {
        println!("{}", serde_json::to_string_pretty(v).unwrap());
    } else {
        println!(
            "indexed {} files ({} omp, {} codex, {} zstd), {} events, {} sessions, {} bytes in {} ms",
            files, omp, codex, zstd, events, sessions, bytes, duration_ms
        );
        println!(
            "preserved: malformed={} unknown_type={} unresolved_parent={} unresolved_call={} multi_call={}",
            malformed, unknown, parent, link, multi
        );
    }
}

fn freshness_json(idx: &Index, file: u32) -> Value {
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

fn cmd_show(args: &[String]) -> Result<()> {
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

fn relation(kind: &str, target: Value, source_field: &str, evidence: Vec<Value>) -> Value {
    json!({
        "kind": kind,
        "target": target,
        "source_field": source_field,
        "evidence": evidence
    })
}

fn evidence_for(idx: &Arc<Index>, e: EventIdx) -> Value {
    match idx.event(e) {
        Ok(ev) => json!({"span": span_label(ev.span()), "event": e}),
        Err(_) => json!({"span": null, "event": e}),
    }
}

fn cmd_resolve(args: &[String]) -> Result<()> {
    let target = positional(args).ok_or_else(|| err("resolve needs a target"))?;
    let (idx, store) = load(args)?;
    let idx = &idx;
    let explain = has(args, "--explain");
    let mut relations = Vec::new();
    let mut steps = Vec::new();
    let mut status = "exact";
    let subject: String;
    let mut note = String::new();

    if let Some(id) = target.strip_prefix("session:") {
        let sidx = idx
            .session_by_id(id)
            .ok_or_else(|| err(format!("session not indexed: {id}")))?;
        let s = idx.session(sidx)?;
        subject = format!("session:{id}");
        let header = s.header();
        steps.push(format!(
            "looked up session id by binary search over {} sorted session ids -> idx {}",
            idx.session_count, sidx
        ));
        steps.push(format!("session header event is {}", span_label(idx.event(header)?.span())));

        if s.parent_idx() != NONE {
            let p = idx.session(s.parent_idx())?;
            relations.push(relation(
                "parent_session",
                json!(p.id()),
                if s.path_derived() { "source path (sidecar directory)" } else { "omp.parentSession / codex session_meta" },
                vec![evidence_for(idx, header), evidence_for(idx, p.header())],
            ));
        } else if !s.parent_id().is_empty() {
            relations.push(relation(
                "unresolved_parent",
                json!(s.parent_id()),
                "verbatim source parent id (no indexed session)",
                vec![evidence_for(idx, header)],
            ));
        }
        for child in idx.sessions_by_parent(sidx) {
            let c = idx.session(child)?;
            relations.push(relation(
                "child_session",
                json!(c.id()),
                "reverse of source parent reference",
                vec![evidence_for(idx, header), evidence_for(idx, c.header())],
            ));
        }
        let events = idx.session_events(sidx)?;
        let mut branch_count = 0usize;
        let mut call_count = 0usize;
        for e in &events {
            let ev = idx.event(*e)?;
            if ev.parent() != NONE {
                branch_count += 1;
                relations.push(relation(
                    "branch_parent",
                    json!(span_label(idx.event(ev.parent())?.span())),
                    "omp.parentId (verbatim source DAG edge)",
                    vec![evidence_for(idx, *e), evidence_for(idx, ev.parent())],
                ));
            }
            if ev.link() != NONE {
                call_count += 1;
                let other = idx.event(ev.link())?;
                let kind = if ev.kind() == ase_core::EventKind::ToolResult {
                    "call_link_result"
                } else {
                    "call_link_call"
                };
                relations.push(relation(
                    kind,
                    json!(span_label(other.span())),
                    "toolCallId / codex call_id (verbatim source link)",
                    vec![evidence_for(idx, *e), evidence_for(idx, ev.link())],
                ));
            }
        }
        if explain {
            steps.push(format!(
                "scanned {} indexed events of this session: {} branch edges, {} call links",
                events.len(), branch_count, call_count
            ));
        }
    } else if let Some(span) = parse_span(&target) {
        let e = idx
            .event_at(span)?
            .ok_or_else(|| err("no indexed event at that coordinate"))?;
        let ev = idx.event(e)?;
        subject = span_label(span);
        steps.push(format!(
            "exact coordinate lookup -> event {}: {} / tag {}",
            e,
            ev.kind().label(),
            ev.tag()
        ));
        if ev.parent() != NONE {
            relations.push(relation(
                "branch_parent",
                json!(span_label(idx.event(ev.parent())?.span())),
                "omp.parentId (verbatim source DAG edge)",
                vec![evidence_for(idx, e), evidence_for(idx, ev.parent())],
            ));
        }
        if ev.session() != NONE {
            for i in idx.session_events(ev.session())? {
                let other = idx.event(i)?;
                if other.parent() == e {
                    relations.push(relation(
                        "branch_child",
                        json!(span_label(other.span())),
                        "reverse of omp.parentId",
                        vec![evidence_for(idx, e), evidence_for(idx, i)],
                    ));
                }
            }
        }
        if ev.link() != NONE {
            relations.push(relation(
                "call_link",
                json!(span_label(idx.event(ev.link())?.span())),
                "toolCallId / codex call_id (verbatim source link)",
                vec![evidence_for(idx, e), evidence_for(idx, ev.link())],
            ));
        }
    } else if let Some(token) = target.strip_prefix("task:") {
        status = "partial";
        subject = format!("task:{token}");
        note = "Literal source-path token matches are CANDIDATES only; they do not establish task identity. P0c owns task assignment (naming grammar, parentSession, task_id prose, structural/semantic boundaries), none of which P0a performs.".to_string();
        let needle = token.to_lowercase();
        steps.push(format!("literal case-insensitive substring search over {} indexed source paths; output is candidate-only, not a task identity", idx.file_count));
        for fid in 0..idx.file_count {
            let f = idx.file(fid)?;
            if !f.path().to_lowercase().contains(&needle) {
                continue;
            }
            for sidx in 0..idx.session_count {
                let s = idx.session(sidx)?;
                if s.file() != fid {
                    continue;
                }
                relations.push(relation(
                    "candidate_path_match",
                    json!(s.id()),
                    "source file path (verbatim metadata; candidate, not identity)",
                    vec![evidence_for(idx, s.header())],
                ));
            }
        }
    } else {
        return Err(err("target must be session:ID, span:F:OFF:LEN or task:TOKEN"));
    }

    // Read one evidence span so provenance is checked even for metadata-only output.
    let mut hash_verified = None;
    if let Some(r) = relations.first() {
        if let Some(ev) = r["evidence"][0]["event"].as_u64() {
            let e = idx.event(ev as u32)?;
            let bytes = store.span(e.span())?;
            hash_verified = Some(fnv64(&bytes) == e.hash());
        }
    }

    let v = json!({
        "command": "resolve",
        "subject": subject,
        "status": status,
        "relations": relations,
        "explain": if explain { steps } else { Vec::<String>::new() },
        "note": note,
        "hash_verified": hash_verified,
        "guarantees": [
            "All relations are verbatim source metadata captured at index time; no inferred or model-generated edges.",
            "session: and span: resolution is exhaustive over the indexed snapshot and reports unresolved references explicitly.",
            "task: resolution is candidate-only by design in P0a: a literal path token hit is NOT task identity and NOT an assignment. P0c owns grammar, prose task_id, continuation and semantic boundaries; none are performed here.",
            "Every relation carries at least one raw byte span; stale files are reported, not silently served.",
            "No postings, motif, turn or judge data exists in P0a."
        ]
    });
    if has(args, "--json") {
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        println!("resolve {} -> {} ({} relation(s))", subject, status, relations.len());
        for r in &relations {
            println!(
                "  {} -> {} [{}] evidence={}",
                r["kind"].as_str().unwrap_or(""),
                r["target"].as_str().unwrap_or(""),
                r["source_field"].as_str().unwrap_or(""),
                r["evidence"].as_array().map(|a| a.len()).unwrap_or(0)
            );
        }
        if !note.is_empty() {
            println!("  note: {note}");
        }
            let empty_arr: Vec<Value> = Vec::new();
        if explain {
            for s in v["explain"].as_array().unwrap_or(&empty_arr) {
                println!("  step: {}", s.as_str().unwrap_or(""));
            }
        }
    }
    Ok(())
}

fn cmd_verify(args: &[String]) -> Result<()> {
    let (idx, store) = load(args)?;
    let all = has(args, "--all");
    let sample: Option<u64> = opt(args, "--sample").and_then(|s| s.parse().ok());
    if !all && sample.is_none() {
        return Err(err("verify needs --all or --sample N"));
    }
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
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        println!(
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

fn cmd_bench_show(args: &[String]) -> Result<()> {
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

fn emit(v: &Value, _args: &[String]) {
    println!("{}", serde_json::to_string_pretty(v).unwrap());
}

fn require_publication(args: &[String]) -> Result<(Arc<Index>, FsRawStore, Publication)> {
    let (idx, store, published, reason) = load_with_publication(args)?;
    match published {
        Some(p) => Ok((idx, store, p)),
        None => Err(err(format!(
            "derived layer unavailable: {}; address remains usable",
            reason.unwrap_or_else(|| "unknown".to_string())
        ))),
    }
}

fn cmd_search(args: &[String]) -> Result<()> {
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
    let hits = search(&published, &idx, &query, phrase, top_k)?;
    let query_hash = sha256_hex(query.as_bytes());
    let rendered: Vec<Value> = hits
        .iter()
        .map(|h| {
            let fresh = freshness_json(&idx, h.span.file);
            json!({
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
            })
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

fn hash_bytes(hex: &str) -> Option<[u8; 8]> {
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

fn cmd_open(args: &[String]) -> Result<()> {
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
        emit(&v, args);
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
    let raw = store.span(span)?;
    let span_ok = fnv64(&raw) == ev.hash();
    let extracted = ase_ingest::extract_block(idx.file(span.file)?.provider(), &raw, &selector);
    let block_ok = extracted
        .as_ref()
        .map(|e| hash_bytes(&record.block_hash) == Some(e.block_hash))
        .unwrap_or(false);
    let fresh = freshness_json(&idx, span.file);
    let status = if span_ok && block_ok {
        "verified_against_index_snapshot"
    } else {
        "unverified"
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
    if has(args, "--content") {
        let content = match extracted {
            Ok(e) => e.prose.or(e.call_sig).unwrap_or_default(),
            Err(_) => String::new(),
        };
        v["content"] = json!(content);
    }
    emit(&v, args);
    Ok(())
}

fn cmd_shingle(args: &[String]) -> Result<()> {
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

fn percentile(sorted: &[u128], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let pos = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[pos] as f64 / 1000.0
}

fn cmd_bench_search(args: &[String]) -> Result<()> {
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

const SYNTHETIC_OMP: &str = r#"{"type":"session","version":3,"id":"syn-session-omp-1","timestamp":"2026-01-02T03:04:05.000Z","cwd":"C:\\SYNTHETIC\\proj","title":"synthetic","parentSession":"syn-session-omp-parent"}
{"type":"message","id":"syn-e1","parentId":null,"timestamp":"2026-01-02T03:04:06.000Z","message":{"role":"user","content":[{"type":"text","text":"SYNTHETIC USER TOKEN"}]}}
{"type":"message","id":"syn-e2","parentId":"syn-e1","timestamp":"2026-01-02T03:04:07.000Z","message":{"role":"assistant","content":[{"type":"text","text":"SYNTHETIC ASSISTANT TOKEN"},{"type":"toolCall","id":"syn-call-1","name":"read","args":{"path":"C:\\SYNTHETIC\\file.txt"}}]}}
{"type":"custom","customType":"tool_execution_start","data":{"toolCallId":"syn-call-1","toolName":"read","args":{"path":"C:\\SYNTHETIC\\file.txt"}},"id":"syn-e3","parentId":"syn-e2","timestamp":"2026-01-02T03:04:08.000Z"}
{"type":"message","id":"syn-e4","parentId":"syn-e2","timestamp":"2026-01-02T03:04:09.000Z","message":{"role":"toolResult","toolCallId":"syn-call-1","toolName":"read","content":[{"type":"text","text":"SYNTHETIC TOOL OUTPUT"}]}}
{"type":"model_change","id":"syn-e5","parentId":"syn-e4","model":"synthetic/model","timestamp":"2026-01-02T03:04:10.000Z"}
{"type":"future_entry_type","id":"syn-e6","parentId":"syn-e5","timestamp":"2026-01-02T03:04:11.000Z","note":"SYNTHETIC"}
{this line is deliberately not json}
"#;

const SYNTHETIC_CODEX: &str = r#"{"timestamp":"2026-01-02T03:04:05.000Z","ordinal":0,"type":"session_meta","payload":{"id":"syn-session-codex-1","cwd":"C:\\SYNTHETIC\\proj"}}
{"timestamp":"2026-01-02T03:04:06.000Z","ordinal":1,"type":"event_msg","payload":{"type":"user_message","message":"SYNTHETIC USER TOKEN"}}
{"timestamp":"2026-01-02T03:04:07.000Z","ordinal":2,"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"text","text":"SYNTHETIC ASSISTANT TOKEN"}]}}
{"timestamp":"2026-01-02T03:04:08.000Z","ordinal":3,"type":"response_item","payload":{"type":"function_call","call_id":"syn-call-c1","name":"read","arguments":"{}"}}
{"timestamp":"2026-01-02T03:04:09.000Z","ordinal":4,"type":"response_item","payload":{"type":"function_call_output","call_id":"syn-call-c1","output":"SYNTHETIC OUTPUT"}}
{"timestamp":"2026-01-02T03:04:10.000Z","ordinal":5,"type":"token_count","payload":{"total_token_usage":{"input_tokens":1}}}
{"timestamp":"2026-01-02T03:04:11.000Z","ordinal":6,"type":"future_type","payload":{"note":"SYNTHETIC"}}
"#;

fn cmd_fixture(args: &[String]) -> Result<()> {
    let out = PathBuf::from(opt(args, "--out").ok_or_else(|| err("fixture needs --out DIR"))?);
    let omp = out.join("omp");
    let codex = out.join("codex");
    std::fs::create_dir_all(&omp).map_err(|e| err(format!("create {}: {e}", omp.display())))?;
    std::fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;
    std::fs::write(omp.join("synthetic.jsonl"), SYNTHETIC_OMP)
        .map_err(|e| err(format!("write fixture: {e}")))?;
    let cb = SYNTHETIC_CODEX.as_bytes();
    std::fs::write(codex.join("synthetic.jsonl"), cb)
        .map_err(|e| err(format!("write fixture: {e}")))?;
    let compressed = zstd::stream::encode_all(std::io::Cursor::new(cb), 3)
        .map_err(|e| err(format!("zstd fixture: {e}")))?;
    std::fs::write(codex.join("synthetic.jsonl.zst"), compressed)
        .map_err(|e| err(format!("write fixture: {e}")))?;
    println!("wrote synthetic fixtures to {}", out.display());
    Ok(())
}

const P0B_OMP: &str = r#"{"type":"session","version":3,"id":"p0b-session-omp","timestamp":"2026-02-03T04:05:06.000Z","cwd":"C:\\SYNTHETIC\\p0b","title":"synthetic"}
{"type":"message","id":"p0b-e1","parentId":null,"timestamp":"2026-02-03T04:05:07.000Z","message":{"role":"user","content":[{"type":"text","text":"P0B PROSE ALPHA \u00e9\u2603"},{"type":"text","text":"P0B PROSE BETA"}]}}
{"type":"message","id":"p0b-e2","parentId":"p0b-e1","timestamp":"2026-02-03T04:05:08.000Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"P0B EXCLUDED THINKING"},{"type":"text","text":"P0B ASSISTANT GAMMA"},{"type":"toolCall","id":"p0b-call-a","name":"alphaTool","args":{"path":"synthetic/a.txt","flag":true}},{"type":"toolCall","id":"p0b-call-b","name":"betaTool","args":{"path":"synthetic/b.txt","count":2}},{"type":"toolCall","id":"p0b-call-c","name":"gammaTool","args":{}}]}}
{"type":"message","id":"p0b-e3","parentId":"p0b-e1","timestamp":"2026-02-03T04:05:09.000Z","message":{"role":"toolResult","toolCallId":"p0b-call-a","content":[{"type":"text","text":"P0B EXCLUDED TOOL OUTPUT"}]}}
{"type":"message","id":"p0b-e5","parentId":"p0b-e2","timestamp":"2026-02-03T04:05:10.000Z","message":{"role":"assistant","content":[{"type":"text","text":"P0B LINEAR DELTA"},{"type":"toolCall","id":"p0b-call-e","name":"epsilonTool","args":{}}]}}
{"type":"message","id":"p0b-e6","parentId":"p0b-e5","timestamp":"2026-02-03T04:05:11.000Z","message":{"role":"assistant","content":[{"type":"text","text":"P0B LINEAR ETA"},{"type":"toolCall","id":"p0b-call-f","name":"zetaTool","args":{}}]}}
{"type":"message","id":"p0b-fork-a","parentId":"p0b-e2","timestamp":"2026-02-03T04:05:12.000Z","message":{"role":"assistant","content":[{"type":"toolCall","id":"p0b-call-ga","name":"forkATool","args":{}}]}}
{"type":"message","id":"p0b-fork-b","parentId":"p0b-e2","timestamp":"2026-02-03T04:05:13.000Z","message":{"role":"assistant","content":[{"type":"toolCall","id":"p0b-call-gb","name":"forkBTool","args":{}}]}}
{this line is deliberately not json}
"#;

const P0B_CODEX: &str = r#"{"timestamp":"2026-02-03T04:05:06.000Z","ordinal":0,"type":"session_meta","payload":{"id":"p0b-session-codex","cwd":"C:\\SYNTHETIC\\p0b"}}
{"timestamp":"2026-02-03T04:05:07.000Z","ordinal":1,"type":"event_msg","payload":{"type":"user_message","message":"P0B CODEX EPSILON \u00e9"}}
{"timestamp":"2026-02-03T04:05:08.000Z","ordinal":2,"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"P0B CODEX ZETA"},{"type":"output_text","text":"P0B CODEX ETA"}]}}
{"timestamp":"2026-02-03T04:05:09.000Z","ordinal":3,"type":"response_item","payload":{"type":"function_call","call_id":"p0b-call-x","name":"alphaTool","arguments":"{\"path\":\"synthetic/x.txt\",\"flag\":true}"}}
{"timestamp":"2026-02-03T04:05:10.000Z","ordinal":4,"type":"response_item","payload":{"type":"function_call_output","call_id":"p0b-call-x","output":"P0B EXCLUDED TOOL OUTPUT"}}
{"timestamp":"2026-02-03T04:05:11.000Z","ordinal":5,"type":"response_item","payload":{"type":"reasoning","summary":[{"type":"summary_text","text":"P0B EXCLUDED THINKING"}]}}
"#;

const P0B_CODEX_SECOND: &str = r#"{"timestamp":"2026-02-03T05:05:06.000Z","ordinal":0,"type":"session_meta","payload":{"id":"p0b-session-codex-2","cwd":"C:\\SYNTHETIC\\p0b"}}
{"timestamp":"2026-02-03T05:05:07.000Z","ordinal":1,"type":"response_item","payload":{"type":"function_call","call_id":"p0b-call-x","name":"alphaTool","arguments":"{\"path\":\"synthetic/y.txt\",\"flag\":true}"}}
{"timestamp":"2026-02-03T05:05:08.000Z","ordinal":2,"type":"event_msg","payload":{"type":"user_message","message":"P0B OTHER SESSION THETA"}}
"#;

fn cmd_fixture_p0b(args: &[String]) -> Result<()> {
    let out = PathBuf::from(opt(args, "--out").ok_or_else(|| err("fixture-p0b needs --out DIR"))?);
    let omp = out.join("omp");
    let codex = out.join("codex");
    std::fs::create_dir_all(&omp).map_err(|e| err(format!("create {}: {e}", omp.display())))?;
    std::fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;
    std::fs::write(omp.join("synthetic-p0b.jsonl"), P0B_OMP)
        .map_err(|e| err(format!("write fixture: {e}")))?;
    let codex_bytes = P0B_CODEX.as_bytes();
    std::fs::write(codex.join("synthetic-p0b.jsonl"), codex_bytes)
        .map_err(|e| err(format!("write fixture: {e}")))?;
    let compressed = zstd::stream::encode_all(std::io::Cursor::new(codex_bytes), 3)
        .map_err(|e| err(format!("zstd fixture: {e}")))?;
    std::fs::write(codex.join("synthetic-p0b.jsonl.zst"), compressed)
        .map_err(|e| err(format!("write fixture: {e}")))?;
    std::fs::write(codex.join("synthetic-p0b-second.jsonl"), P0B_CODEX_SECOND)
        .map_err(|e| err(format!("write fixture: {e}")))?;
    println!("wrote synthetic P0b fixtures to {}", out.display());
    Ok(())
}
