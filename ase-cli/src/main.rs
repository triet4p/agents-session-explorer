//! P0a address CLI plus the P0b derived search layer. All decoding,
//! addressing and projection derivation lives in library crates; the CLI only
//! renders metadata. No model is on any query path. P0c-2 adds a metadata-only
//! file-name grammar audit (`audit naming`) and its synthetic fixture writer;
//! P0c-3 adds a metadata-only `parentSession` delegation audit
//! (`audit delegation`) and its synthetic fixture writer.
//! P0c-4 adds a metadata-only `task_id` marker observation audit
//! (`audit markers`) and its synthetic fixture writer.
//! P0c-5 reconciles naming, delegation and marker signals in `resolve task:`
//! with evidence-backed `assigned`, `ambiguous` or `unresolved` output; it also
//! adds `fixture-resolve` synthetic fixtures.
//! P0c-9 joins native OMP `task_dispatch_v1` / `task_dispatch_child_v1`
//! records into `resolve task:` evidence and adds `fixture-dispatch`.
//! The record loader itself lives in `ase-motif::membership` so the motif
//! projection reuses exactly the same validated membership.
//! P0d-2 adds `motif` (bounded alignment, task-deduplicated support, recorded
//! denominator/exclusions, fixed-seed null model and per-occurrence evidence)
//! and `fixture-motif` synthetic fixtures.

use ase_core::{err, EventIdx, ProjectionKind, RawStore, Result, Role, SpanRef, NONE};
use ase_index::{
    build, decode_corpus, sha256_hex, write_address, DecodedCorpus, FsRawStore, Index,
};
use ase_ingest::util::fnv64;
use ase_postings::{
    block_by_selector, publication, search, shingles_by_event, shingles_by_token, Publication,
    WriterInput,
};
use ase_naming::{SourceRef, Status};
use ase_postings::reader::BlockRecord;
use ase_task::{
    Basis, DelegationEvidence, MarkerEvidence, MarkerLayer, NamingEvidence, NamingSignal,
    PathEvidence, Resolution, Stance, Status as TaskStatus,
};
use ase_motif::membership::dispatch_input;
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
        "audit" => cmd_audit(rest),
        "fixture-naming" => cmd_fixture_naming(rest),
        "fixture-delegation" => cmd_fixture_delegation(rest),
        "fixture-markers" => cmd_fixture_markers(rest),
        "fixture-resolve" => cmd_fixture_resolve(rest),
        "fixture-dispatch" => cmd_fixture_dispatch(rest),
        "motif" => cmd_motif(rest),
        "fixture-motif" => cmd_fixture_motif(rest),
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
         ase audit  naming [--index FILE] [--project PROJECT] [--explain] [--json]\n\
         ase audit  delegation [--index FILE] [--explain] [--json]\n\
         ase audit  markers [--index FILE] [--explain] [--json]\n\
         ase fixture-naming --out DIR\n\
         ase fixture-delegation --out DIR\n\
         ase fixture-markers --out DIR\n\
         ase fixture-resolve --out DIR\n\
         ase fixture-dispatch --out DIR\n\
         ase motif  TOKEN... [--tau F] [--k-min N] [--k-max N] [--max-gap N]\n\
                     [--band N] [--max-substitutions N] [--null-permutations N]\n\
                     [--min-tasks N] [--explain] [--json]\n\
         ase fixture-motif --out DIR\n\
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

fn positionals(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut expecting = false;
    for a in args {
        if a.starts_with("--") {
            expecting = true;
            continue;
        }
        if expecting {
            expecting = false;
            continue;
        }
        out.push(a.clone());
    }
    out
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
    let (idx, store, published, publication_reason) = load_with_publication(args)?;
    let idx = &idx;
    let explain = has(args, "--explain");
    let mut relations = Vec::new();
    let mut steps = Vec::new();
    let status = "exact";
    let subject: String;
    let note = String::new();

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
        return cmd_resolve_task(
            args,
            idx,
            &store,
            published.as_ref(),
            publication_reason.as_deref(),
            token,
            explain,
        );
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

// ------------------------------------------- P0c-5 task identity resolution

fn resolve_naming_signals(idx: &Index) -> Result<Vec<NamingSignal>> {
    let audit = ase_naming::audit(&naming_sources(idx)?);
    Ok(audit
        .signals
        .iter()
        .map(|signal| NamingSignal {
            status: signal.status,
            reason: signal.reason.clone(),
            candidates: signal
                .candidates
                .iter()
                .map(|candidate| NamingEvidence {
                    project: candidate.project.clone(),
                    task: candidate.task.clone(),
                    role: candidate.role.clone(),
                    attempt: candidate.attempt.clone(),
                    split_rule: candidate.split_rule.clone(),
                    file_id: signal.source.file_id,
                    path: signal.source.path.clone(),
                    provider: signal.source.provider.clone(),
                    session_id: signal.source.session_id.clone(),
                    span_anchor: signal.source.span_anchor.clone(),
                })
                .collect(),
        })
        .collect())
}

fn resolve_delegation_signals(idx: &Index) -> Result<Vec<DelegationEvidence>> {
    let audit = ase_delegation::audit(&delegation_inputs(idx)?);
    Ok(audit
        .edges
        .iter()
        .map(|edge| DelegationEvidence {
            from_session_id: edge.from.id.clone(),
            from_span: edge.from.header_span.clone(),
            declared_id: edge.declaration.declared_id.clone(),
            observation: edge.declaration.kind.label().to_string(),
            field: edge.declaration.field.to_string(),
            link_status: edge.status.label().to_string(),
            target_session_id: edge.target.as_ref().map(|target| target.id.clone()),
            target_span: edge.target.as_ref().map(|target| target.header_span.clone()),
            reason: edge.reason.to_string(),
        })
        .collect())
}

/// Literal path-token matches are observed metadata candidates only.
fn resolve_path_matches(idx: &Index, token: &str) -> Result<Vec<PathEvidence>> {
    let needle = token.to_lowercase();
    let mut out = Vec::new();
    for sidx in 0..idx.session_count {
        let session = idx.session(sidx)?;
        let file = idx.file(session.file())?;
        if !file.path().to_lowercase().contains(&needle) {
            continue;
        }
        out.push(PathEvidence {
            session_id: session.id().to_string(),
            path: file.path().to_string(),
            provider: file.provider().label().to_string(),
            span_anchor: span_label(idx.event(session.header())?.span()),
        });
    }
    Ok(out)
}

/// Allowlisted prose blocks of one published generation that verify by span
/// hash, selector re-derivation and block digest. Prose text never leaves here.
struct VerifiedProse {
    blocks_total: usize,
    excluded_role_blocks: usize,
    unverified_blocks: usize,
    texts: Vec<ase_markers::Block>,
    kept: Vec<BlockRecord>,
}

fn verified_prose(
    idx: &Arc<Index>,
    store: &FsRawStore,
    published: &Publication,
) -> Result<VerifiedProse> {
    let blocks = ase_postings::all_prose_blocks(published)?;
    let mut texts: Vec<ase_markers::Block> = Vec::with_capacity(blocks.len());
    let mut kept = Vec::with_capacity(blocks.len());
    let mut excluded_role_blocks = 0usize;
    let mut unverified_blocks = 0usize;
    for block in &blocks {
        if block.role != "user" && block.role != "assistant" {
            excluded_role_blocks += 1;
            continue;
        }
        if block.event >= idx.event_count {
            unverified_blocks += 1;
            continue;
        }
        let ev = idx.event(block.event)?;
        if ev.span() != block.span {
            unverified_blocks += 1;
            continue;
        }
        let raw = store.span(block.span)?;
        if fnv64(&raw) != ev.hash() {
            unverified_blocks += 1;
            continue;
        }
        let provider = idx.file(block.span.file)?.provider();
        let extracted = match ase_ingest::extract_block(provider, &raw, &block.selector) {
            Ok(value) => value,
            Err(_) => {
                unverified_blocks += 1;
                continue;
            }
        };
        if extracted.kind != ProjectionKind::Prose
            || hash_bytes(&block.block_hash) != Some(extracted.block_hash)
        {
            unverified_blocks += 1;
            continue;
        }
        let text = extracted.prose.unwrap_or_default();
        if text.trim().is_empty() {
            unverified_blocks += 1;
            continue;
        }
        kept.push(block.clone());
        texts.push(ase_markers::Block { text });
    }
    Ok(VerifiedProse {
        blocks_total: blocks.len(),
        excluded_role_blocks,
        unverified_blocks,
        texts,
        kept,
    })
}

/// Convert verified marker occurrences into metadata-only resolver evidence.
fn marker_evidence(
    idx: &Index,
    kept: &[BlockRecord],
    observations: &[ase_markers::Observation],
    published: &Publication,
) -> Result<Vec<MarkerEvidence>> {
    let mut out = Vec::with_capacity(observations.len());
    for observation in observations {
        let block = &kept[observation.block_index];
        let sidx = idx.event(block.event)?.session();
        let session_id = if sidx != NONE {
            idx.session(sidx)?.id().to_string()
        } else {
            String::new()
        };
        out.push(MarkerEvidence {
            value: observation.value.clone(),
            usage: observation.usage,
            reason: observation.reason.to_string(),
            assignment_cue: observation.assignment_cue.clone(),
            mention_cue: observation.mention_cue.clone(),
            occurrence_in_block: observation.occurrence_in_block,
            char_start: observation.char_start,
            char_end: observation.char_end,
            generation: published.generation.clone(),
            event: block.event,
            span: span_label(block.span),
            selector: block.selector.clone(),
            block_hash: block.block_hash.clone(),
            role: block.role.clone(),
            file_id: block.span.file,
            session_id,
        });
    }
    Ok(out)
}

fn task_evidence_value(
    evidence: &ase_task::Evidence,
    stance: Stance,
    index_label: &str,
) -> Value {
    match evidence {
        ase_task::Evidence::Naming(naming) => json!({
            "kind": evidence.kind(),
            "basis": Basis::Derived.label(),
            "stance": stance.label(),
            "rule": { "name": ase_naming::RULE_NAME, "version": ase_naming::RULE_VERSION },
            "project": &naming.project,
            "task": &naming.task,
            "role": &naming.role,
            "attempt": &naming.attempt,
            "split_rule": &naming.split_rule,
            "source": {
                "file_id": naming.file_id,
                "path": &naming.path,
                "provider": &naming.provider,
                "session_id": &naming.session_id,
                "span_anchor": &naming.span_anchor
            },
            "note": "The path is observed metadata; project/task/role/attempt are derived from the basename inside its (project,parent) scope."
        }),
        ase_task::Evidence::Marker(marker) => json!({
            "kind": evidence.kind(),
            "basis": Basis::Observed.label(),
            "stance": stance.label(),
            "rule": { "name": ase_markers::RULE_NAME, "version": ase_markers::RULE_VERSION },
            "value": &marker.value,
            "char_start": marker.char_start,
            "char_end": marker.char_end,
            "observed": {
                "generation": &marker.generation,
                "event": marker.event,
                "span": &marker.span,
                "selector": &marker.selector,
                "block_hash": &marker.block_hash,
                "role": &marker.role,
                "file_id": marker.file_id,
                "session_id": &marker.session_id
            },
            "derived": {
                "usage": marker.usage.label(),
                "reason": &marker.reason,
                "assignment_cue": &marker.assignment_cue,
                "mention_cue": &marker.mention_cue,
                "occurrence_in_block": marker.occurrence_in_block,
                "rule": { "name": ase_markers::RULE_NAME, "version": ase_markers::RULE_VERSION }
            },
            "note": "The occurrence is observed; its assignment/mention class is derived. No prose text is emitted."
        }),
        ase_task::Evidence::Delegation(delegation) => json!({
            "kind": evidence.kind(),
            "basis": Basis::Observed.label(),
            "stance": stance.label(),
            "rule": { "name": ase_delegation::RULE_NAME, "version": ase_delegation::RULE_VERSION },
            "observed": {
                "from_session_id": &delegation.from_session_id,
                "from_span": &delegation.from_span,
                "declared_id": &delegation.declared_id,
                "field": &delegation.field,
                "observation": &delegation.observation
            },
            "derived": {
                "link_status": &delegation.link_status,
                "target_session_id": &delegation.target_session_id,
                "target_span": &delegation.target_span,
                "reason": &delegation.reason,
                "rule": { "name": ase_delegation::RULE_NAME, "version": ase_delegation::RULE_VERSION }
            },
            "task_boundary": "A parentSession declaration is a delegated_from edge, never a task assignment."
        }),
        ase_task::Evidence::Path(path) => json!({
            "kind": evidence.kind(),
            "basis": Basis::Observed.label(),
            "stance": stance.label(),
            "session_id": &path.session_id,
            "path": &path.path,
            "provider": &path.provider,
            "span_anchor": &path.span_anchor,
            "rule": { "name": ase_task::PATH_RULE_NAME, "version": ase_task::PATH_RULE_VERSION },
            "note": "A literal source-path substring is a candidate, not task identity."
        }),
        ase_task::Evidence::Dispatch(join) => json!({
            "kind": evidence.kind(),
            "basis": Basis::Observed.label(),
            "stance": stance.label(),
            "rule": { "name": ase_task::DISPATCH_RULE_NAME, "version": ase_task::DISPATCH_RULE_VERSION },
            "observed": {
                "dispatch_id": &join.dispatch_id,
                "parent_session_id": &join.parent_session_id,
                "child_session_id": &join.child_session_id,
                "parent_tool_call_id": &join.parent_tool_call_id,
                "spawn_index": join.spawn_index,
                "parent": {
                    "event": join.parent_event,
                    "file_id": join.parent_file_id,
                    "span": &join.parent_span,
                    "span_hash_verified": join.parent_span_verified,
                    "reopen": format!("ase show {} --index {} --json --explain", join.parent_span, index_label)
                },
                "child": {
                    "event": join.child_event,
                    "file_id": join.child_file_id,
                    "span": &join.child_span,
                    "span_hash_verified": join.child_span_verified,
                    "reopen": format!("ase show {} --index {} --json --explain", join.child_span, index_label)
                }
            },
            "derived": {
                "project": &join.project,
                "task": &join.task,
                "attempt": &join.attempt,
                "join": "exactly one task_dispatch_v1 parent record and one task_dispatch_child_v1 link for this dispatch_id; the structured parent_session_id and child session_id match their containing journals; parent and child journals differ; both spans verified",
                "rule": { "name": ase_task::DISPATCH_RULE_NAME, "version": ase_task::DISPATCH_RULE_VERSION }
            },
            "task_boundary": "The join assigns the executing child session named by the runtime records only; a suffix, path, worker name, prose echo or parent-only claim never does. A malformed, duplicated, mismatched or competing record assigns nothing."
        }),
    }
}

fn task_evidence_list(
    resolution: &Resolution,
    ids: &[usize],
    stance: Stance,
    index_label: &str,
) -> Vec<Value> {
    ids.iter()
        .filter_map(|id| resolution.evidence.get(*id))
        .map(|evidence| task_evidence_value(evidence, stance, index_label))
        .collect()
}

fn task_candidate_relation(
    resolution: &Resolution,
    candidate: &ase_task::Candidate,
    index_label: &str,
) -> Value {
    let kind = if resolution.status == TaskStatus::Assigned {
        "assigned_to"
    } else {
        "candidate_task"
    };
    json!({
        "kind": kind,
        "target": {
            "project": &candidate.project,
            "task": &candidate.task,
            "task_key": &candidate.task_key
        },
        "candidate_state": candidate.state.label(),
        "reason": &candidate.reason,
        "support": task_evidence_list(resolution, &candidate.supports, Stance::Support, index_label),
        "refute": task_evidence_list(resolution, &candidate.refutes, Stance::Refute, index_label),
        "context": task_evidence_list(resolution, &candidate.context, Stance::Context, index_label),
        "rule": { "name": resolution.rule.name, "version": resolution.rule.version }
    })
}

fn cmd_resolve_task(
    args: &[String],
    idx: &Arc<Index>,
    store: &FsRawStore,
    published: Option<&Publication>,
    publication_reason: Option<&str>,
    token: &str,
    explain: bool,
) -> Result<()> {
    if token.trim().is_empty() {
        return Err(err("resolve task: needs a non-empty token"));
    }
    let naming = resolve_naming_signals(idx)?;
    let delegations = resolve_delegation_signals(idx)?;
    let paths = resolve_path_matches(idx, token)?;
    let dispatch = dispatch_input(idx, store)?;
    let naming_count = naming.len();
    let delegation_count = delegations.len();
    let path_count = paths.len();
    let dispatch_parent_count = dispatch.parents.len();
    let dispatch_child_count = dispatch.children.len();

    let (marker_layer, markers) = match published {
        Some(published) => {
            let verified = verified_prose(idx, store, published)?;
            let audit = ase_markers::audit(&verified.texts);
            let markers = marker_evidence(idx, &verified.kept, &audit.observations, published)?;
            let layer = MarkerLayer {
                generation: Some(published.generation.clone()),
                unavailable: None,
                blocks_total: verified.blocks_total,
                blocks_scanned: verified.texts.len(),
                excluded_role_blocks: verified.excluded_role_blocks,
                unverified_blocks: verified.unverified_blocks,
            };
            (layer, markers)
        }
        None => (
            MarkerLayer {
                generation: None,
                unavailable: Some(
                    publication_reason
                        .unwrap_or("derived layer unavailable")
                        .to_string(),
                ),
                blocks_total: 0,
                blocks_scanned: 0,
                excluded_role_blocks: 0,
                unverified_blocks: 0,
            },
            Vec::new(),
        ),
    };

    let marker_count = markers.iter().filter(|marker| marker.value == token).count();
    let resolution = ase_task::resolve(&ase_task::Input {
        token: token.to_string(),
        naming,
        markers,
        delegations,
        paths,
        dispatch,
        marker_layer,
    });

    let index_label = index_path(args).to_string_lossy().to_string();
    let relations = resolution
        .candidates
        .iter()
        .map(|candidate| task_candidate_relation(&resolution, candidate, &index_label))
        .collect::<Vec<Value>>();
    let marker_layer = &resolution.marker_layer;
    let marker_layer_value = json!({
        "status": if marker_layer.generation.is_some() { "available" } else { "unavailable" },
        "reason": &marker_layer.unavailable,
        "generation": &marker_layer.generation,
        "blocks_total": marker_layer.blocks_total,
        "blocks_scanned": marker_layer.blocks_scanned,
        "excluded_role_blocks": marker_layer.excluded_role_blocks,
        "unverified_blocks": marker_layer.unverified_blocks
    });
    let dispatch_layer = &resolution.dispatch;
    let dispatch_layer_value = json!({
        "status": "available",
        "rule": { "name": ase_task::DISPATCH_RULE_NAME, "version": ase_task::DISPATCH_RULE_VERSION },
        "parents_indexed": dispatch_parent_count,
        "child_links_indexed": dispatch_child_count,
        "unique_joins": dispatch_layer.joined,
        "rejected_groups": dispatch_layer.rejected_groups,
        "record_issues": dispatch_layer.issues_total,
        "claims_for_token": dispatch_layer.claims_for_token,
        "rejected_claims_for_token": dispatch_layer
            .issues_for_token
            .iter()
            .map(|issue| json!({
                "record": &issue.record,
                "kind": &issue.kind,
                "dispatch_id": &issue.dispatch_id,
                "event": issue.event,
                "span": &issue.span,
                "detail": &issue.detail
            }))
            .collect::<Vec<Value>>()
    });
    let rule = &resolution.rule;
    let v = json!({
        "command": "resolve",
        "subject": format!("task:{token}"),
        "status": resolution.status.label(),
        "task_key": &resolution.task_key,
        "resolution": {
            "status": resolution.status.label(),
            "task_key": &resolution.task_key,
            "summary": &resolution.summary,
            "candidate_count": resolution.candidates.len(),
            "rule": { "name": &rule.name, "version": &rule.version }
        },
        "relations": relations,
        "signals": {
            "naming_signals_indexed": naming_count,
            "marker_occurrences_for_token": marker_count,
            "delegation_edges_indexed": delegation_count,
            "path_matches": path_count,
            "marker_layer": marker_layer_value,
            "dispatch_layer": dispatch_layer_value
        },
        "explain": if explain { resolution.explain.clone() } else { Vec::<String>::new() },
        "hash_verified": Value::Null,
        "guarantees": [
            "Assignment requires exactly one project-scoped key from a parsed name grammar or from one unique, consistent native dispatch join; assignment-context markers in the same file corroborate it but no marker alone assigns a task.",
            "A dispatch join requires exactly one task_dispatch_v1 parent record and one task_dispatch_child_v1 link for the same runtime dispatch_id, structured session ids that match their containing journals, distinct parent and child journals, and both raw spans re-verified against indexed bytes.",
            "Malformed, missing, duplicated, wrong-session, mismatched or competing dispatch records never assign; they are reported as rejected claims with their span and reason.",
            "A conflicting layer returns ambiguous with every candidate, its support/refute evidence and reasons; insufficient evidence stays unresolved.",
            "A mention, an ambiguous marker, a literal path match or a parentSession declaration is a candidate/context only, never an assignment.",
            "Project is part of the key: the same token in different projects is reported separately and never merged. The child session named by a dispatch join is the executing child, not a suffix or worker-name match.",
            "Observed and derived evidence are labelled with their rule name/version; marker evidence is pinned to one generation, span, selector and block digest, and dispatch evidence carries both parent and child record spans.",
            "Metadata-only: no prose text is emitted, no postings content is returned and no model is called."
        ]
    });
    if has(args, "--json") {
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        println!(
            "resolve task:{token} -> {} ({} candidate(s))",
            resolution.status.label(),
            resolution.candidates.len()
        );
        println!("  {}", resolution.summary);
        for r in &relations {
            let target = &r["target"];
            println!(
                "  {} -> project={} task={} state={} support={} refute={} context={}",
                r["kind"].as_str().unwrap_or(""),
                target["project"].as_str().unwrap_or("-"),
                target["task"].as_str().unwrap_or("-"),
                r["candidate_state"].as_str().unwrap_or(""),
                r["support"].as_array().map_or(0, Vec::len),
                r["refute"].as_array().map_or(0, Vec::len),
                r["context"].as_array().map_or(0, Vec::len)
            );
            println!("    reason: {}", r["reason"].as_str().unwrap_or(""));
        }
        if explain {
            let empty_arr: Vec<Value> = Vec::new();
            for step in v["explain"].as_array().unwrap_or(&empty_arr) {
                println!("  step: {}", step.as_str().unwrap_or(""));
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

// ------------------------------------------------------- P0c-2 naming audit

// Only synthetic names are used for rule work. Each tuple is
// (parent directory, file basenames); project comes from the name itself.
const NAMING_FIXTURE: &[(&str, &[&str])] = &[
    ("multiple-splits", &["S18Task70BS01", "S18Task70SR02", "S18Task70DEP3"]),
    ("marker-t", &["S14T3ProviderA5", "S14T3ReviewB7"]),
    ("shared-prefix-tie", &["S813BoundaryGold23", "S813BoundarySilver24"]),
    ("missing-suffix", &["S14T3ProviderA", "S14T3OtherB9"]),
    ("collision-a", &["S18Task70BS01"]),
    ("collision-b", &["S18Task70BS01"]),
    ("cross-project", &["S18Task70BS01", "S19Task70BS01"]),
];

fn naming_fixture_content(sid: &str) -> String {
    format!(
        "{{\"type\":\"session\",\"version\":3,\"id\":\"{sid}\",\"timestamp\":\"2026-03-04T05:06:07.000Z\",\"cwd\":\"C:\\\\SYNTHETIC\\\\p0c2\",\"title\":\"synthetic naming fixture\",\"parentSession\":\"p0c2-parent\"}}\n{{\"type\":\"message\",\"id\":\"{sid}-e1\",\"parentId\":null,\"timestamp\":\"2026-03-04T05:06:08.000Z\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"SYNTHETIC NAMING TOKEN\"}}]}}}}\n"
    )
}

fn cmd_fixture_naming(args: &[String]) -> Result<()> {
    let out = PathBuf::from(opt(args, "--out").ok_or_else(|| err("fixture-naming needs --out DIR"))?);
    let omp = out.join("omp");
    let codex = out.join("codex");
    fs::create_dir_all(&omp).map_err(|e| err(format!("create {}: {e}", omp.display())))?;
    fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;
    let mut written = 0usize;
    for (parent, names) in NAMING_FIXTURE {
        let dir = omp.join(parent);
        fs::create_dir_all(&dir).map_err(|e| err(format!("create {}: {e}", dir.display())))?;
        for name in *names {
            let sid = format!("p0c2-{parent}-{name}");
            let path = dir.join(format!("{name}.jsonl"));
            fs::write(&path, naming_fixture_content(&sid))
                .map_err(|e| err(format!("write {}: {e}", path.display())))?;
            written += 1;
        }
    }
    println!("wrote {written} synthetic P0c-2 naming fixtures to {}", out.display());
    Ok(())
}

/// Build grammar inputs from the address index only. No raw bytes are read.
fn naming_sources(idx: &Index) -> Result<Vec<SourceRef>> {
    let mut session_by_file: std::collections::HashMap<u32, (String, String)> =
        std::collections::HashMap::new();
    for s in 0..idx.session_count {
        let view = idx.session(s)?;
        let session_id = view.id().to_string();
        let span_anchor = idx
            .event(view.header())
            .map(|e| span_label(e.span()))
            .unwrap_or_default();
        session_by_file
            .entry(view.file())
            .or_insert((session_id, span_anchor));
    }
    let mut out = Vec::with_capacity(idx.file_count as usize);
    for fid in 0..idx.file_count {
        let f = idx.file(fid)?;
        let path = f.path().to_string();
        let (session_id, span_anchor) = session_by_file
            .get(&fid)
            .cloned()
            .unwrap_or_else(|| (String::new(), String::new()));
        out.push(SourceRef {
            file_id: fid,
            path,
            provider: f.provider().label().to_string(),
            session_id,
            span_anchor,
        });
    }
    Ok(out)
}

fn naming_evidence(s: &SourceRef, rule: &ase_naming::RuleInfo) -> Value {
    json!({
        "source": {
            "file_id": s.file_id,
            "path": s.path,
            "provider": s.provider,
            "session_id": s.session_id,
            "span_anchor": s.span_anchor
        },
        "locator": format!("file.path:{}", s.path),
        "rule": { "name": &rule.name, "version": &rule.version }
    })
}

fn candidate_value(c: &ase_naming::Candidate) -> Value {
    json!({
        "project": &c.project,
        "task": &c.task,
        "role": &c.role,
        "attempt": &c.attempt,
        "split_rule": &c.split_rule
    })
}

fn cmd_audit(args: &[String]) -> Result<()> {
    let subject = args.first().map(String::as_str).unwrap_or("");
    match subject {
        "naming" => cmd_audit_naming(args),
        "delegation" => cmd_audit_delegation(args),
        "markers" => cmd_audit_markers(args),
        _ => Err(err("audit needs a subject: naming|delegation|markers")),
    }
}

fn cmd_audit_naming(args: &[String]) -> Result<()> {
    // Deliberately open the address directly: this command never constructs a
    // RawStore, opens a span, reads prose, or consults postings.
    let idx = Index::open(&index_path(args))?;
    let mut result = ase_naming::audit(&naming_sources(&idx)?);
    if let Some(project) = opt(args, "--project") {
        result.signals.retain(|s| match s.status {
            Status::Parsed | Status::Ambiguous => {
                s.candidates.iter().any(|c| c.project == project)
            }
            Status::Unparsed => s.basename.starts_with(&project),
        });
        result
            .clusters
            .retain(|c| c.project == project);
        result.ties.retain(|t| t.candidates.iter().any(|c| c.project == project));
        result.unparsed.retain(|u| u.basename.starts_with(&project));
        result.collisions.retain(|c| {
            c.left.starts_with(&project) || c.right.starts_with(&project)
        });
        result.coverage.names = result.signals.len();
        result.coverage.parsed = result
            .signals
            .iter()
            .filter(|s| s.status == Status::Parsed)
            .count();
        result.coverage.ambiguous = result
            .signals
            .iter()
            .filter(|s| s.status == Status::Ambiguous)
            .count();
        result.coverage.unparsed = result.coverage.names
            - result.coverage.parsed
            - result.coverage.ambiguous;
    }

    let rule = &result.rule;
    let coverage = &result.coverage;
    let v = json!({
        "command": "audit",
        "subject": "naming",
        "status": "ok",
        "rule": {
            "name": &rule.name,
            "version": &rule.version,
            "description": &rule.description,
            "attempt_semantics": &rule.attempt_semantics,
            "abstention": &rule.abstention
        },
        "source": {
            "kind": "address_index_metadata",
            "index": index_path(args).to_string_lossy(),
            "fields": ["file.path", "file.id", "file.provider", "session.id", "session.header.span"],
            "transcript_read": false,
            "note": "Names come from indexed path metadata; this command does not open raw spans or prose."
        },
        "coverage": {
            "names": coverage.names,
            "parsed": coverage.parsed,
            "ambiguous": coverage.ambiguous,
            "unparsed": coverage.unparsed,
            "parsed_ratio": if coverage.names == 0 {
                0.0
            } else {
                coverage.parsed as f64 / coverage.names as f64
            },
            "denominator": "all indexed source file names in scope"
        },
        "clusters": result.clusters.iter().map(|c| json!({
            "project": &c.project,
            "parent": &c.parent,
            "common_prefix": &c.common_prefix,
            "names": c.names.iter().map(|n| json!({
                "basename": &n.basename,
                "task_key": format!("{}|{}|{}|{}", n.candidate.project, n.candidate.task, n.candidate.role, n.candidate.attempt),
                "candidate": candidate_value(&n.candidate),
                "evidence": naming_evidence(&n.source, rule)
            })).collect::<Vec<Value>>()
        })).collect::<Vec<Value>>(),
        "ties": result.ties.iter().map(|t| json!({
            "parent": &t.parent,
            "basename": &t.basename,
            "decision": t.decision,
            "reason": &t.reason,
            "candidates": t.candidates.iter().map(candidate_value).collect::<Vec<Value>>(),
            "evidence": naming_evidence(&t.source, rule)
        })).collect::<Vec<Value>>(),
        "unparsed": result.unparsed.iter().map(|u| json!({
            "parent": &u.parent,
            "basename": &u.basename,
            "reason": &u.reason,
            "evidence": naming_evidence(&u.source, rule)
        })).collect::<Vec<Value>>(),
        "signals": result.signals.iter().map(|s| json!({
            "basename": &s.basename,
            "parent": &s.parent,
            "status": format!("{:?}", s.status).to_lowercase(),
            "reason": &s.reason,
            "shared_prefix": &s.shared_prefix,
            "sibling_count": s.sibling_count,
            "candidates": s.candidates.iter().map(candidate_value).collect::<Vec<Value>>(),
            "evidence": naming_evidence(&s.source, rule)
        })).collect::<Vec<Value>>(),
        "collisions": result.collisions.iter().map(|c| json!({
            "kind": c.kind,
            "left": &c.left,
            "right": &c.right,
            "note": &c.note
        })).collect::<Vec<Value>>(),
        "explain": if has(args, "--explain") {
            vec![
                "Read file.path, file.id, provider, session.id and session.header.span from the address index; no transcript, postings or model is touched.".to_string(),
                "Grouped source names by parent directory and recorded the longest prefix shared with a sibling as cluster evidence.".to_string(),
                "Parsed trailing digits as the attempt label, trailing letters as the role, then a `Task`/`T` marker or an admissible project/task digit boundary.".to_string(),
                "An explicit marker is decisive; without one, every admissible boundary is retained and more than one candidate is reported as a tie with decision abstain.".to_string(),
                "Attempt numbers are labels only: no chronology, newest attempt or ordering is inferred from them.".to_string(),
                "Project and parent stay in every task key; cross-project and cross-parent collisions are reported rather than merged.".to_string(),
            ]
        } else {
            Vec::<String>::new()
        },
        "guarantees": [
            "Metadata-only audit: no raw span is opened and no prose is decoded.",
            "Every parsed assertion carries the source path, session pointer and the rule name/version.",
            "Ambiguous and unparsed names are retained with reasons and are never promoted to task keys.",
            "No cross-project or cross-parent merge; ties are abstentions, not assignments."
        ]
    });
    emit(&v, args);
    Ok(())
}

// --------------------------------------------- P0c-3 parentSession delegation

const DELEGATION_TS: &str = "2026-03-05T06:07:08.000Z";

// Only synthetic ids and paths are used. Tuples are
// (relative path under omp/, explicit session id or None, declared parent or None).
// A `None` id is a path-derived OMP session: the decoder reads the parent id
// from the sidecar directory name, recorded as a path observation.
const DELEGATION_FIXTURE: &[(&str, Option<&str>, Option<&str>)] = &[
    ("unique/del-parent-unique.jsonl", Some("del-parent-unique"), None),
    ("unique/del-child-unique.jsonl", Some("del-child-unique"), Some("del-parent-unique")),
    ("missing/del-child-missing.jsonl", Some("del-child-missing"), Some("del-parent-absent")),
    // Adjacent files share one id: the index's first-match parent_idx must not select one.
    ("ambiguous/del-parent-dup-a.jsonl", Some("del-parent-dup"), None),
    ("ambiguous/del-parent-dup-b.jsonl", Some("del-parent-dup"), None),
    ("ambiguous/del-child-ambiguous.jsonl", Some("del-child-ambiguous"), Some("del-parent-dup")),
    (
        "path/2026-01-01_abcdefghijklmnopqrstuvwxyz1234/del-path-child.jsonl",
        None,
        None,
    ),
    (
        "path/del-path-parent.jsonl",
        Some("2026-01-01_abcdefghijklmnopqrstuvwxyz1234"),
        None,
    ),
];

fn delegation_session_line(id: &str, parent: Option<&str>) -> String {
    let parent_field = parent
        .map(|p| format!(",\"parentSession\":\"{p}\""))
        .unwrap_or_default();
    format!(
        "{{\"type\":\"session\",\"version\":3,\"id\":\"{id}\",\"timestamp\":\"{DELEGATION_TS}\",\"cwd\":\"C:\\\\SYNTHETIC\\\\p0c3\",\"title\":\"synthetic delegation fixture\"{parent_field}}}\n"
    )
}

fn delegation_message_line(sid: &str) -> String {
    format!(
        "{{\"type\":\"message\",\"id\":\"{sid}-e1\",\"parentId\":null,\"timestamp\":\"{DELEGATION_TS}\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"SYNTHETIC DELEGATION TOKEN\"}}]}}}}\n"
    )
}

fn cmd_fixture_delegation(args: &[String]) -> Result<()> {
    let out = PathBuf::from(
        opt(args, "--out").ok_or_else(|| err("fixture-delegation needs --out DIR"))?,
    );
    let omp = out.join("omp");
    let codex = out.join("codex");
    fs::create_dir_all(&omp).map_err(|e| err(format!("create {}: {e}", omp.display())))?;
    fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;
    let mut written = 0usize;
    for &(rel, id, parent) in DELEGATION_FIXTURE {
        let path = omp.join(rel);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| err(format!("create {}: {e}", dir.display())))?;
        }
        let sid = id.unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("synthetic-path-derived")
        });
        let mut content = String::new();
        if let Some(id) = id {
            content.push_str(&delegation_session_line(id, parent));
        }
        content.push_str(&delegation_message_line(sid));
        fs::write(&path, content).map_err(|e| err(format!("write {}: {e}", path.display())))?;
        written += 1;
    }
    println!(
        "wrote {written} synthetic P0c-3 delegation fixtures to {}",
        out.display()
    );
    Ok(())
}

/// Build delegation inputs from indexed session metadata only. No raw bytes
/// are read; the header span is a locator, not decoded evidence.
fn delegation_inputs(idx: &Index) -> Result<Vec<ase_delegation::SessionInput>> {
    use ase_delegation::{Declaration, ObservationKind, SessionInput, SessionRef};

    let mut out = Vec::with_capacity(idx.session_count as usize);
    for sidx in 0..idx.session_count {
        let s = idx.session(sidx)?;
        let f = idx.file(s.file())?;
        let path = f.path().to_string();
        let provider = f.provider().label().to_string();
        let header_span = span_label(idx.event(s.header())?.span());
        let declared = s.parent_id();
        let declaration = if declared.is_empty() {
            None
        } else if s.path_derived() {
            let path_segment = Path::new(&path)
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .map(str::to_string);
            Some(Declaration {
                declared_id: declared.to_string(),
                kind: ObservationKind::PathDerived,
                field: "source.path.parent_directory",
                payload_span: None,
                path_segment,
            })
        } else {
            Some(Declaration {
                declared_id: declared.to_string(),
                kind: ObservationKind::PayloadField,
                field: if provider == "codex" {
                    "codex.session_meta.parent"
                } else {
                    "omp.parentSession"
                },
                payload_span: Some(header_span.clone()),
                path_segment: None,
            })
        };
        out.push(SessionInput {
            session: SessionRef {
                session_idx: sidx,
                id: s.id().to_string(),
                file_id: s.file(),
                path,
                provider,
                header_span,
            },
            declaration,
        });
    }
    Ok(out)
}

fn delegation_session_value(s: &ase_delegation::SessionRef) -> Value {
    json!({
        "session_idx": s.session_idx,
        "id": &s.id,
        "file_id": s.file_id,
        "path": &s.path,
        "provider": &s.provider,
        "header_span": &s.header_span
    })
}

fn cmd_audit_delegation(args: &[String]) -> Result<()> {
    // Same metadata-only seam as `audit naming`: no RawStore, no span read,
    // no postings and no model.
    let idx = Index::open(&index_path(args))?;
    let result = ase_delegation::audit(&delegation_inputs(&idx)?);
    let rule = &result.rule;
    let rule_value = json!({ "name": &rule.name, "version": &rule.version });
    let edges = result
        .edges
        .iter()
        .map(|e| {
            json!({
                "predicate": "delegated_from",
                "from": delegation_session_value(&e.from),
                "declaration": {
                    "declared_id": &e.declaration.declared_id,
                    "observation": e.declaration.kind.label(),
                    "field": e.declaration.field,
                    "payload_span": &e.declaration.payload_span,
                    "path_segment": &e.declaration.path_segment
                },
                "status": e.status.label(),
                "reason": e.reason,
                "target": e.target.as_ref().map(delegation_session_value),
                "candidates": e.candidates.iter().map(delegation_session_value).collect::<Vec<Value>>(),
                "evidence": {
                    "declaration_anchor": e.declaration.payload_span.clone().unwrap_or_else(|| e.from.header_span.clone()),
                    "target_span": e.target.as_ref().map(|t| t.header_span.clone()),
                    "note": "A header span locates the source file; a path-derived declaration has no payload span and no id text is claimed inside the payload."
                },
                "rule": rule_value.clone(),
                "task_key": Value::Null,
                "assigned_to": Value::Null
            })
        })
        .collect::<Vec<Value>>();

    let v = json!({
        "command": "audit",
        "subject": "delegation",
        "status": "ok",
        "rule": {
            "name": &rule.name,
            "version": &rule.version,
            "domain": &rule.domain,
            "abstention": &rule.abstention,
            "task_boundary": &rule.task_boundary
        },
        "source": {
            "kind": "address_index_metadata",
            "index": index_path(args).to_string_lossy(),
            "fields": ["session.id", "session.parent_id", "session.path_derived", "session.header.span", "file.path", "file.id", "file.provider"],
            "transcript_read": false,
            "note": "Declared parent ids come from indexed session metadata; no raw span is opened, no prose is decoded and no model is called."
        },
        "counts": {
            "sessions": result.sessions,
            "declarations": result.declarations,
            "linked": result.linked,
            "unlinked_absent": result.unlinked_absent,
            "unlinked_ambiguous": result.unlinked_ambiguous,
            "unlinked_self": result.unlinked_self,
            "without_parent": result.without_parent,
            "payload_declarations": result.payload_declarations,
            "path_declarations": result.path_declarations
        },
        "edges": edges,
        "sessions_without_parent": result.sessions_without_parent.iter().map(delegation_session_value).collect::<Vec<Value>>(),
        "task_assignment": "none",
        "task_key": Value::Null,
        "explain": if has(args, "--explain") {
            vec![
                "Read session.id, session.parent_id, session.path_derived, the session header span and file path/provider from the address index; no raw bytes, prose, postings or model are touched.".to_string(),
                "Payload-field and path-derived declarations are recorded separately: a header span locates the file but never proves that the declared id text is in a payload.".to_string(),
                "A target is accepted only when exactly one indexed session id equals the declared id byte-for-byte; file order, path proximity and the index's first-match parent_idx are not used to choose a target.".to_string(),
                "Zero matches -> unlinked_absent; more than one -> unlinked_ambiguous with every candidate retained; a self reference -> unlinked_self. Candidate order is enumeration order only and never selects a target.".to_string(),
                "Every edge carries the rule name/version, both endpoint locators and task_key/assigned_to set to null: a parent declaration alone is a delegation edge, not a task assignment.".to_string()
            ]
        } else {
            Vec::<String>::new()
        },
        "guarantees": [
            "Metadata-only: no raw span is opened, no prose is decoded and no model is called.",
            "Unique exact-id targets are linked with inspectable source and target locators plus rule name/version.",
            "Missing, multiple or self targets stay unlinked with all candidates retained; no file-order or path-proximity tie-break exists.",
            "Payload and path observations are recorded separately, with no payload span claimed for a path-derived id.",
            "A parent declaration never produces a TaskKey or an assigned_to edge."
        ]
    });
    emit(&v, args);
    Ok(())
}

// ------------------------------------------------------- P0c-4 marker audit

// Synthetic markers only. One OMP session carries allowlisted user/assistant
// text plus deliberately marker-bearing thinking, tool output and call
// arguments that the allowlist must never project as prose.
const MARKER_FIXTURE_LINES: &[&str] = &[
    r#"{"type":"session","version":3,"id":"marker-fixture-session","timestamp":"2026-03-06T07:08:09.000Z","cwd":"C:\\SYNTHETIC\\p0c4","title":"synthetic marker fixture"}"#,
    r#"{"type":"message","id":"marker-e1","parentId":null,"timestamp":"2026-03-06T07:08:10.000Z","message":{"role":"user","content":[{"type":"text","text":"Please implement task_id=SYN-A1 and task_id=SYN-A2 in the parser."}]}}"#,
    r#"{"type":"message","id":"marker-e2","parentId":null,"timestamp":"2026-03-06T07:08:11.000Z","message":{"role":"assistant","content":[{"type":"text","text":"I will work on task_id=SYN-A1. Please implement task_id=SYN-A1 again."}]}}"#,
    r#"{"type":"message","id":"marker-e3","parentId":null,"timestamp":"2026-03-06T07:08:12.000Z","message":{"role":"user","content":[{"type":"text","text":"Earlier we mentioned task_id=SYN-M1 for context only."}]}}"#,
    r#"{"type":"message","id":"marker-e4","parentId":null,"timestamp":"2026-03-06T07:08:13.000Z","message":{"role":"assistant","content":[{"type":"text","text":"Implement task_id=SYN-X1 but note task_id=SYN-X1 for reference."}]}}"#,
    r#"{"type":"message","id":"marker-e5","parentId":null,"timestamp":"2026-03-06T07:08:14.000Z","message":{"role":"user","content":[{"type":"text","text":"task_id=SYN-U1"}]}}"#,
    r#"{"type":"message","id":"marker-e6","parentId":null,"timestamp":"2026-03-06T07:08:15.000Z","message":{"role":"assistant","content":[{"type":"thinking","text":"task_id=SYN-THINK assigned here"},{"type":"text","text":"Please implement task_id=SYN-A3."}]}}"#,
    r#"{"type":"message","id":"marker-e7","parentId":null,"timestamp":"2026-03-06T07:08:16.000Z","message":{"role":"assistant","content":[{"type":"toolCall","name":"synthetic_tool","args":"{\"task_id\":\"SYN-CALL\",\"mode\":1}"}]}}"#,
    r#"{"type":"message","id":"marker-e8","parentId":null,"timestamp":"2026-03-06T07:08:17.000Z","message":{"role":"toolResult","toolCallId":"marker-call","content":[{"type":"text","text":"tool output only: task_id=SYN-TOOL assigned here"}]}}"#,
];

// ------------------------------------------------ P0c-5 resolve fixture

const RESOLVE_TS: &str = "2026-03-07T08:09:10.000Z";

struct ResolveFixtureFile {
    rel: &'static str,
    id: &'static str,
    parent: Option<&'static str>,
    messages: &'static [(&'static str, &'static str)],
}

// Synthetic only. Each case isolates one reconciliation outcome:
// consistent naming+marker, a cross-layer conflict, naming-only, a multi-ID
// session, the same token in two projects, marker-only, mention-only and a
// parentSession pair without a task grammar.
const RESOLVE_FIXTURE: &[ResolveFixtureFile] = &[
    ResolveFixtureFile {
        rel: "p0c5/S10Task101Alpha1.jsonl",
        id: "fx-101-alpha",
        parent: None,
        messages: &[("user", "Please implement task_id=101 in the parser.")],
    },
    ResolveFixtureFile {
        rel: "p0c5/S10Task102Beta1.jsonl",
        id: "fx-102-beta",
        parent: None,
        messages: &[(
            "assistant",
            "Please implement task_id=999 for this change.",
        )],
    },
    ResolveFixtureFile {
        rel: "p0c5/S10Task103Gamma1.jsonl",
        id: "fx-103-gamma-1",
        parent: None,
        messages: &[("assistant", "Working on the naming grammar only.")],
    },
    ResolveFixtureFile {
        rel: "p0c5/S10Task103Gamma3.jsonl",
        id: "fx-103-gamma-3",
        parent: None,
        messages: &[("user", "Same task, later attempt label.")],
    },
    ResolveFixtureFile {
        rel: "p0c5/S10Task108Eta1.jsonl",
        id: "fx-108-eta",
        parent: None,
        messages: &[(
            "user",
            "Please implement task_id=108 and task_id=777 as separate follow-ups.",
        )],
    },
    ResolveFixtureFile {
        rel: "p0c5/S20Task55Gold1.jsonl",
        id: "fx-55-s20",
        parent: None,
        messages: &[("user", "Implement task_id=55 for project S20.")],
    },
    ResolveFixtureFile {
        rel: "p0c5/S21Task55Gold2.jsonl",
        id: "fx-55-s21",
        parent: None,
        messages: &[("assistant", "Implement task_id=55 for project S21.")],
    },
    ResolveFixtureFile {
        rel: "p0c5/session-notes.jsonl",
        id: "fx-notes",
        parent: None,
        messages: &[(
            "user",
            "Please implement task_id=555, but the filename has no grammar.",
        )],
    },
    ResolveFixtureFile {
        rel: "p0c5/session-mention.jsonl",
        id: "fx-mention",
        parent: None,
        messages: &[(
            "user",
            "Earlier we mentioned task_id=666 for reference only.",
        )],
    },
    ResolveFixtureFile {
        rel: "p0c5/syn-parent-root.jsonl",
        id: "syn-parent-root",
        parent: None,
        messages: &[("user", "root synthetic session")],
    },
    ResolveFixtureFile {
        rel: "p0c5/syn-parent-child.jsonl",
        id: "syn-parent-child",
        parent: Some("syn-parent-root"),
        messages: &[("user", "delegated synthetic session")],
    },
];

fn resolve_session_line(id: &str, parent: Option<&str>) -> String {
    let parent_field = parent
        .map(|value| format!(",\"parentSession\":\"{value}\""))
        .unwrap_or_default();
    format!(
        "{{\"type\":\"session\",\"version\":3,\"id\":\"{id}\",\"timestamp\":\"{RESOLVE_TS}\",\"cwd\":\"C:\\\\SYNTHETIC\\\\p0c5\",\"title\":\"synthetic resolve fixture\"{parent_field}}}\n"
    )
}

fn resolve_message_line(sid: &str, ordinal: usize, role: &str, text: &str) -> String {
    format!(
        "{{\"type\":\"message\",\"id\":\"{sid}-e{ordinal}\",\"parentId\":null,\"timestamp\":\"{RESOLVE_TS}\",\"message\":{{\"role\":\"{role}\",\"content\":[{{\"type\":\"text\",\"text\":\"{text}\"}}]}}}}\n"
    )
}

fn cmd_fixture_resolve(args: &[String]) -> Result<()> {
    let out = PathBuf::from(
        opt(args, "--out").ok_or_else(|| err("fixture-resolve needs --out DIR"))?,
    );
    let omp = out.join("omp");
    let codex = out.join("codex");
    fs::create_dir_all(&omp).map_err(|e| err(format!("create {}: {e}", omp.display())))?;
    fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;
    for entry in RESOLVE_FIXTURE {
        let path = omp.join(entry.rel);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| err(format!("create {}: {e}", dir.display())))?;
        }
        let mut content = resolve_session_line(entry.id, entry.parent);
        for (ordinal, (role, text)) in entry.messages.iter().enumerate() {
            content.push_str(&resolve_message_line(entry.id, ordinal + 1, role, text));
        }
        fs::write(&path, content).map_err(|e| err(format!("write {}: {e}", path.display())))?;
    }
    println!(
        "wrote {} synthetic P0c-5 resolve fixtures to {}",
        RESOLVE_FIXTURE.len(),
        out.display()
    );
    Ok(())
}

// -------------------------------------------- P0c-9 dispatch fixture

const DISPATCH_TS: &str = "2026-03-08T09:10:11.000Z";
const DISPATCH_PARENT_SESSION: &str = "dispatch-fixture-parent";

const D_410: &str = "11111111-1111-4111-8111-111111111111";
const D_10410: &str = "22222222-2222-4222-8222-222222222222";
const D_ONLY: &str = "33333333-3333-4333-8333-333333333333";
const D_CONFLICT: &str = "44444444-4444-4444-8444-444444444444";
const D_MISSING: &str = "55555555-5555-4555-8555-555555555555";
const D_DUP_PARENT: &str = "66666666-6666-4666-8666-666666666666";
const D_DUP_CHILD: &str = "77777777-7777-4777-8777-777777777777";
const D_WRONG: &str = "88888888-8888-4888-8888-888888888888";
const D_MISMATCH: &str = "99999999-9999-4999-8999-999999999999";
const D_ORPHAN: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const D_COMP_A: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const D_COMP_B: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
const D_INVALID: &str = "not-a-runtime-uuid";

struct DispatchFixtureParent {
    dispatch_id: &'static str,
    parent_session_id: &'static str,
    parent_tool_call_id: Option<&'static str>,
    spawn_index: u32,
    project: &'static str,
    task: &'static str,
    attempt: &'static str,
}

struct DispatchFixtureChild {
    dispatch_id: &'static str,
    session_id: &'static str,
}

struct DispatchFixtureFile {
    rel: &'static str,
    session_id: &'static str,
    messages: &'static [(&'static str, &'static str)],
    parents: &'static [DispatchFixtureParent],
    children: &'static [DispatchFixtureChild],
    raw_lines: &'static [&'static str],
}

// Synthetic only, shaped exactly like the native OMP v19.0.0 P0c-8
// records. The parent journal carries positive, rejected and competing
// claims; each child journal is the only place its link is valid.
const DISPATCH_PARENTS: &[DispatchFixtureParent] = &[
    DispatchFixtureParent {
        dispatch_id: D_410,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-410"),
        spawn_index: 0,
        project: "S30",
        task: "410",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_10410,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-10410"),
        spawn_index: 1,
        project: "S30",
        task: "10410",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_ONLY,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-only"),
        spawn_index: 2,
        project: "SYN-PROJECT",
        task: "SYN-DISPATCH-ONLY",
        attempt: "attempt-3",
    },
    DispatchFixtureParent {
        dispatch_id: D_CONFLICT,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-conflict"),
        spawn_index: 3,
        project: "SYN-PROJECT",
        task: "SYN-CONFLICT",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_MISSING,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-missing"),
        spawn_index: 4,
        project: "SYN-PROJECT",
        task: "SYN-MISSING",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_DUP_PARENT,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-dup-parent-a"),
        spawn_index: 5,
        project: "SYN-PROJECT",
        task: "SYN-DUP-PARENT",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_DUP_PARENT,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-dup-parent-b"),
        spawn_index: 6,
        project: "SYN-PROJECT",
        task: "SYN-DUP-PARENT",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_DUP_CHILD,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-dup-child"),
        spawn_index: 7,
        project: "SYN-PROJECT",
        task: "SYN-DUP-CHILD",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_WRONG,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-wrong"),
        spawn_index: 8,
        project: "SYN-PROJECT",
        task: "SYN-WRONG-CHILD",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_MISMATCH,
        parent_session_id: "ghost-parent-session",
        parent_tool_call_id: Some("call-mismatch"),
        spawn_index: 9,
        project: "SYN-PROJECT",
        task: "SYN-MISMATCH-PARENT",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_COMP_A,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-comp-a"),
        spawn_index: 10,
        project: "SYN-PROJECT-A",
        task: "SYN-COMPETING",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_COMP_B,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-comp-b"),
        spawn_index: 11,
        project: "SYN-PROJECT-B",
        task: "SYN-COMPETING",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_INVALID,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-malformed"),
        spawn_index: 12,
        project: "SYN-PROJECT",
        task: "SYN-MALFORMED",
        attempt: "1",
    },
];

const DISPATCH_RAW_LINES: &[&str] = &[
    r#"{"type":"custom","timestamp":"2026-03-08T09:10:11.000Z","customType":"task_dispatch_v1","data":{"dispatch_id":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","parent_session_id":"dispatch-fixture-parent","spawn_index":13,"project_id":"SYN-PROJECT","attempt_id":"1"}}"#,
];

const DISPATCH_FIXTURE: &[DispatchFixtureFile] = &[
    DispatchFixtureFile {
        rel: "p0c9/parent-root.jsonl",
        session_id: DISPATCH_PARENT_SESSION,
        messages: &[("user", "dispatch fixture root session")],
        parents: DISPATCH_PARENTS,
        children: &[],
        raw_lines: DISPATCH_RAW_LINES,
    },
    DispatchFixtureFile {
        rel: "p0c9/S30Task410Alpha1.jsonl",
        session_id: "dispatch-child-410",
        messages: &[("assistant", "Tracked child for the first shared-suffix task.")],
        parents: &[],
        children: &[DispatchFixtureChild {
            dispatch_id: D_410,
            session_id: "dispatch-child-410",
        }],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "p0c9/S30Task10410Beta1.jsonl",
        session_id: "dispatch-child-10410",
        messages: &[("assistant", "Tracked child for the second shared-suffix task.")],
        parents: &[],
        children: &[DispatchFixtureChild {
            dispatch_id: D_10410,
            session_id: "dispatch-child-10410",
        }],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "p0c9/TrackedOnlyGamma2.jsonl",
        session_id: "dispatch-child-only",
        messages: &[("assistant", "Tracked child without a name-grammar key.")],
        parents: &[],
        children: &[DispatchFixtureChild {
            dispatch_id: D_ONLY,
            session_id: "dispatch-child-only",
        }],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "p0c9/S99Task999Delta3.jsonl",
        session_id: "dispatch-child-conflict",
        messages: &[("assistant", "Child whose file name disagrees with its dispatch identity.")],
        parents: &[],
        children: &[DispatchFixtureChild {
            dispatch_id: D_CONFLICT,
            session_id: "dispatch-child-conflict",
        }],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "p0c9/DupParentZeta1.jsonl",
        session_id: "dispatch-child-dup-parent",
        messages: &[("assistant", "Child for a duplicated parent claim.")],
        parents: &[],
        children: &[DispatchFixtureChild {
            dispatch_id: D_DUP_PARENT,
            session_id: "dispatch-child-dup-parent",
        }],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "p0c9/DupChildEta1.jsonl",
        session_id: "dispatch-child-dup-child",
        messages: &[("assistant", "Child carrying a duplicated link.")],
        parents: &[],
        children: &[
            DispatchFixtureChild {
                dispatch_id: D_DUP_CHILD,
                session_id: "dispatch-child-dup-child",
            },
            DispatchFixtureChild {
                dispatch_id: D_DUP_CHILD,
                session_id: "dispatch-child-dup-child",
            },
        ],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "p0c9/WrongChildTheta1.jsonl",
        session_id: "dispatch-child-wrong",
        messages: &[("assistant", "Child whose link names another session.")],
        parents: &[],
        children: &[DispatchFixtureChild {
            dispatch_id: D_WRONG,
            session_id: "ghost-session",
        }],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "p0c9/MismatchParentIota1.jsonl",
        session_id: "dispatch-child-mismatch",
        messages: &[("assistant", "Child whose parent record names another session.")],
        parents: &[],
        children: &[DispatchFixtureChild {
            dispatch_id: D_MISMATCH,
            session_id: "dispatch-child-mismatch",
        }],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "p0c9/OrphanChildKappa1.jsonl",
        session_id: "dispatch-child-orphan",
        messages: &[("assistant", "Child carrying a link with no parent record.")],
        parents: &[],
        children: &[DispatchFixtureChild {
            dispatch_id: D_ORPHAN,
            session_id: "dispatch-child-orphan",
        }],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "p0c9/CompetingChildLambda1.jsonl",
        session_id: "dispatch-child-competing",
        messages: &[("assistant", "Child claimed by two dispatch identities.")],
        parents: &[],
        children: &[
            DispatchFixtureChild {
                dispatch_id: D_COMP_A,
                session_id: "dispatch-child-competing",
            },
            DispatchFixtureChild {
                dispatch_id: D_COMP_B,
                session_id: "dispatch-child-competing",
            },
        ],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "p0c9/S40Task888Legacy1.jsonl",
        session_id: "dispatch-legacy-child",
        messages: &[(
            "user",
            "Please implement task_id=888 in the parser without dispatch tracking.",
        )],
        parents: &[],
        children: &[],
        raw_lines: &[],
    },
];

fn dispatch_session_line(id: &str) -> String {
    format!(
        "{{\"type\":\"session\",\"version\":3,\"id\":\"{id}\",\"timestamp\":\"{DISPATCH_TS}\",\"cwd\":\"C:\\\\SYNTHETIC\\\\p0c9\",\"title\":\"synthetic dispatch fixture\"}}\n"
    )
}

fn dispatch_parent_line(row: &DispatchFixtureParent) -> String {
    let mut data = json!({
        "dispatch_id": row.dispatch_id,
        "parent_session_id": row.parent_session_id,
        "spawn_index": row.spawn_index,
        "project_id": row.project,
        "task_id": row.task,
        "attempt_id": row.attempt,
    });
    if let Some(tool_call_id) = row.parent_tool_call_id {
        data["parent_tool_call_id"] = json!(tool_call_id);
    }
    format!(
        "{}\n",
        json!({
            "type": "custom",
            "timestamp": DISPATCH_TS,
            "customType": "task_dispatch_v1",
            "data": data
        })
    )
}

fn dispatch_child_line(row: &DispatchFixtureChild) -> String {
    format!(
        "{}\n",
        json!({
            "type": "custom",
            "timestamp": DISPATCH_TS,
            "customType": "task_dispatch_child_v1",
            "data": {
                "dispatch_id": row.dispatch_id,
                "session_id": row.session_id
            }
        })
    )
}

fn cmd_fixture_dispatch(args: &[String]) -> Result<()> {
    let out = PathBuf::from(
        opt(args, "--out").ok_or_else(|| err("fixture-dispatch needs --out DIR"))?,
    );
    let omp = out.join("omp");
    let codex = out.join("codex");
    fs::create_dir_all(&omp).map_err(|e| err(format!("create {}: {e}", omp.display())))?;
    fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;
    for entry in DISPATCH_FIXTURE {
        let path = omp.join(entry.rel);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| err(format!("create {}: {e}", dir.display())))?;
        }
        let mut content = dispatch_session_line(entry.session_id);
        for parent in entry.parents {
            content.push_str(&dispatch_parent_line(parent));
        }
        for child in entry.children {
            content.push_str(&dispatch_child_line(child));
        }
        for raw in entry.raw_lines {
            content.push_str(raw);
            content.push('\n');
        }
        for (ordinal, (role, text)) in entry.messages.iter().enumerate() {
            content.push_str(&resolve_message_line(entry.session_id, ordinal + 1, role, text));
        }
        fs::write(&path, content).map_err(|e| err(format!("write {}: {e}", path.display())))?;
    }
    println!(
        "wrote {} synthetic P0c-9 dispatch fixtures to {}",
        DISPATCH_FIXTURE.len(),
        out.display()
    );
    Ok(())
}

fn parse_opt<T: std::str::FromStr>(args: &[String], name: &str) -> Result<Option<T>> {
    match opt(args, name) {
        Some(value) => value
            .parse()
            .map(Some)
            .map_err(|_| err(format!("{name} needs a number, got {value}"))),
        None => Ok(None),
    }
}

fn cmd_motif(args: &[String]) -> Result<()> {
    let tokens = positionals(args);
    if tokens.is_empty() {
        return Err(err("motif needs at least one task token"));
    }
    let (idx, store, published) = require_publication(args)?;

    let mut tasks = Vec::with_capacity(tokens.len());
    let mut task_rows = Vec::with_capacity(tokens.len());
    for token in &tokens {
        let member = ase_motif::membership(&idx, &store, token)?;
        let projected = ase_motif::project(&published, &idx, &member)?;
        task_rows.push(json!({
            "token": token,
            "status": member.status.label(),
            "task_key": &member.task_key,
            "attempts": member.attempts.iter().map(|attempt| json!({
                "attempt": &attempt.attempt,
                "dispatch_id": &attempt.dispatch_id,
                "child_session_id": &attempt.child_session_id,
                "child_span": &attempt.child_span,
                "parent_span": &attempt.parent_span
            })).collect::<Vec<Value>>(),
            "rejected_claims": member.dispatch.issues_for_token.iter().map(|issue| json!({
                "record": &issue.record,
                "kind": &issue.kind,
                "dispatch_id": &issue.dispatch_id,
                "event": issue.event,
                "span": &issue.span,
                "detail": &issue.detail
            })).collect::<Vec<Value>>()
        }));
        tasks.push(projected);
    }

    let mut config = ase_motif::MotifConfig::default();
    if let Some(value) = parse_opt::<f64>(args, "--tau")? {
        config.family.tau = value;
    }
    if let Some(value) = parse_opt::<u32>(args, "--k-min")? {
        config.family.k_min = value;
    }
    if let Some(value) = parse_opt::<u32>(args, "--k-max")? {
        config.family.k_max = value;
    }
    if let Some(value) = parse_opt::<u32>(args, "--max-gap")? {
        config.max_gap = value;
    }
    if let Some(value) = parse_opt::<u32>(args, "--band")? {
        config.band = value;
    }
    if let Some(value) = parse_opt::<u32>(args, "--max-substitutions")? {
        config.max_substitutions = value;
    }
    if let Some(value) = parse_opt::<u32>(args, "--null-permutations")? {
        config.null_permutations = value;
    }
    if let Some(value) = parse_opt::<u32>(args, "--min-tasks")? {
        config.min_tasks = value;
    }

    let report = ase_motif::detect(&published, &idx, &store, &tasks, &config)?;

    let motifs: Vec<Value> = report
        .motifs
        .iter()
        .map(|motif| {
            let null_model = &motif.null;
            json!({
                "id": &motif.id,
                "family_id": &motif.family_id,
                "rule": { "name": report.rule_name, "version": report.rule_version },
                "reference": {
                    "task": &motif.reference_task,
                    "attempt": &motif.reference_attempt,
                    "sequence": motif.reference_sequence
                },
                "signatures": &motif.signatures,
                "columns": motif.columns.iter().map(|column| json!({
                    "index": column.index,
                    "signature": &column.signature,
                    "matches": column.matches,
                    "substitutions": &column.substitutions,
                    "missing": column.missing
                })).collect::<Vec<Value>>(),
                "support": {
                    "exact": motif.support_exact,
                    "relaxed": &motif.support_relaxed,
                    "gap_max": config.max_gap,
                    "rule": "distinct task keys; a task counts once no matter how many attempts or occurrences contain the motif"
                },
                "null": {
                    "permutations": null_model.permutations,
                    "seed": null_model.seed.to_string(),
                    "observed": null_model.observed,
                    "expected": null_model.expected,
                    "stddev": null_model.stddev,
                    "z": null_model.z,
                    "lift": null_model.lift,
                    "rule": "fixed-seed SplitMix64 permutation of each sequence's call order; exact support recounted per task"
                },
                "denominator": {
                    "tasks_total": motif.denominator.tasks_total,
                    "tasks_in_family": motif.denominator.tasks_in_family,
                    "tasks_with_occurrence": motif.denominator.tasks_with_occurrence,
                    "sequences_in_family": motif.denominator.sequences_in_family,
                    "sequences_with_occurrence": motif.denominator.sequences_with_occurrence,
                    "excluded": motif.denominator.excluded.iter().map(|exclusion| json!({
                        "task": &exclusion.task,
                        "reason": &exclusion.reason
                    })).collect::<Vec<Value>>()
                },
                "occurrences": motif.occurrences.iter().map(|occurrence| json!({
                    "task": &occurrence.task,
                    "attempt": &occurrence.attempt,
                    "dispatch_id": &occurrence.dispatch_id,
                    "sequence": occurrence.sequence,
                    "gap": occurrence.gap,
                    "calls": occurrence.calls.iter().map(|call| json!({
                        "position": call.position,
                        "signature": &call.signature,
                        "generation": &call.evidence.generation,
                        "event": call.evidence.event,
                        "file": call.evidence.file,
                        "span": span_label(call.evidence.span),
                        "selector": &call.evidence.selector,
                        "block_hash": &call.evidence.block_hash,
                        "session_local": call.evidence.session_local,
                        "reopened": true
                    })).collect::<Vec<Value>>()
                })).collect::<Vec<Value>>(),
                "occurrences_truncated": motif.occurrences_truncated,
                "variants": motif.variants.iter().map(|variant| json!({
                    "task": &variant.task,
                    "attempt": &variant.attempt,
                    "sequence": variant.sequence,
                    "matches": variant.matches,
                    "substitutions": variant.substitutions,
                    "insertions": variant.insertions,
                    "deletions": variant.deletions,
                    "substituted_columns": &variant.substituted_columns,
                    "near_miss": variant.near_miss
                })).collect::<Vec<Value>>(),
                "sequence_exclusions": motif.sequence_exclusions.iter().map(|exclusion| json!({
                    "task": &exclusion.task,
                    "attempt": &exclusion.attempt,
                    "sequence": exclusion.sequence,
                    "reason": &exclusion.reason
                })).collect::<Vec<Value>>()
            })
        })
        .collect();

    let explain = vec![
        "Membership is one native dispatch join per attempt; naming, path and parentSession candidates are never inputs.".to_string(),
        "Support counts distinct task keys: repeated attempts and repeated occurrences of one task are one observation.".to_string(),
        "support_relaxed[g] allows at most g skipped calls between motif columns, so raising g never lowers support.".to_string(),
        "An occurrence may carry a gap above --max-gap; it stays evidence with its gap value, while support_relaxed only counts the configured ceiling.".to_string(),
        "The null model permutes each sequence's call order with the recorded fixed seed and recounts exact per-task support; lift and z compare the observation against that expectation.".to_string(),
        "Insertion and substitution stay separate: an added call is an insertion, a different call at the same column is a substitution.".to_string(),
        "Every occurrence call was reopened from raw bytes at its pinned generation, span and selector before entering the motif; `ase open --event --selector --generation` re-verifies a single call.".to_string(),
    ];

    let v = json!({
        "command": "motif",
        "status": "ok",
        "generation": published.generation,
        "tasks": task_rows,
        "config": {
            "rule": { "name": report.rule_name, "version": report.rule_version },
            "family": { "k_min": config.family.k_min, "k_max": config.family.k_max, "tau": config.family.tau },
            "max_gap": config.max_gap,
            "band": config.band,
            "max_substitutions": config.max_substitutions,
            "null_permutations": config.null_permutations,
            "null_seed": config.null_seed.to_string(),
            "max_occurrences": config.max_occurrences,
            "min_tasks": config.min_tasks
        },
        "summary": {
            "tasks_total": report.tasks,
            "sequences": report.sequences,
            "families": report.families,
            "families_below_min_tasks": report.families_below_min_tasks,
            "motifs": motifs.len()
        },
        "excluded_tasks": report.excluded_tasks.iter().map(|exclusion| json!({
            "task": &exclusion.task,
            "reason": exclusion.reason.label()
        })).collect::<Vec<Value>>(),
        "motifs": motifs,
        "explain": if has(args, "--explain") { explain } else { Vec::<String>::new() }
    });

    if has(args, "--json") {
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        println!(
            "motif: {} task(s), {} sequence(s), {} family(ies), {} motif(s)",
            report.tasks,
            report.sequences,
            report.families,
            report.motifs.len()
        );
        for exclusion in &report.excluded_tasks {
            println!(
                "  excluded task {} ({})",
                exclusion.task,
                exclusion.reason.label()
            );
        }
        for motif in &report.motifs {
            let z = motif
                .null
                .z
                .map(|value| format!("{value:.2}"))
                .unwrap_or_else(|| "-".to_string());
            let lift = motif
                .null
                .lift
                .map(|value| format!("{value:.2}"))
                .unwrap_or_else(|| "-".to_string());
            println!(
                "  {} tasks={}/{} exact={} relaxed={:?} z={} lift={} occurrences={}",
                motif.id,
                motif.denominator.tasks_in_family,
                motif.denominator.tasks_total,
                motif.support_exact,
                motif.support_relaxed,
                z,
                lift,
                motif.occurrences.len()
            );
            println!(
                "    signatures: {}",
                motif.signatures.join(" -> ")
            );
            for occurrence in &motif.occurrences {
                println!(
                    "    occurrence task={} attempt={} gap={} first_event={}",
                    occurrence.task,
                    occurrence.attempt,
                    occurrence.gap,
                    occurrence
                        .calls
                        .first()
                        .map(|call| call.evidence.event)
                        .unwrap_or(NONE)
                );
            }
        }
        if has(args, "--explain") {
            for step in v["explain"].as_array().into_iter().flatten() {
                println!("  step: {}", step.as_str().unwrap_or(""));
            }
        }
    }
    Ok(())
}

const MOTIF_TS: &str = "2026-11-02T00:00:00.000Z";
const MOTIF_PARENT_SESSION: &str = "p0d2-fixture-parent";

const M_A1: &str = "a1a1a1a1-a1a1-4a1a-8a1a-a1a1a1a1a1a1";
const M_A2: &str = "a2a2a2a2-a2a2-4a2a-8a2a-a2a2a2a2a2a2";
const M_B1: &str = "b1b1b1b1-b1b1-4b1b-8b1b-b1b1b1b1b1b1";
const M_C1: &str = "c1c1c1c1-c1c1-4c1c-8c1c-c1c1c1c1c1c1";
const M_E1: &str = "e1e1e1e1-e1e1-4e1e-8e1e-e1e1e1e1e1e1";

struct MotifFixtureTask {
    dispatch_id: &'static str,
    task: &'static str,
    attempt: &'static str,
    session_id: &'static str,
    tools: &'static [&'static str],
}

// Synthetic only. Task 1 repeats across two attempts (one exact, one with an
// inserted call), task 2 is gapped-only, task 3 is a near miss with one
// substitution, and task 9 is a valid join whose child has no call at all.
const MOTIF_FIXTURE_TASKS: &[MotifFixtureTask] = &[
    MotifFixtureTask {
        dispatch_id: M_A1,
        task: "1",
        attempt: "a1",
        session_id: "p0d2-child-1-a1",
        tools: &["t1", "t2", "t3", "t4"],
    },
    MotifFixtureTask {
        dispatch_id: M_A2,
        task: "1",
        attempt: "a2",
        session_id: "p0d2-child-1-a2",
        tools: &["t1", "t2", "x1", "t3", "t4"],
    },
    MotifFixtureTask {
        dispatch_id: M_B1,
        task: "2",
        attempt: "b1",
        session_id: "p0d2-child-2-b1",
        tools: &["t1", "t2", "y1", "t3", "t4"],
    },
    MotifFixtureTask {
        dispatch_id: M_C1,
        task: "3",
        attempt: "c1",
        session_id: "p0d2-child-3-c1",
        tools: &["t1", "t2", "z1", "t4"],
    },
    MotifFixtureTask {
        dispatch_id: M_E1,
        task: "9",
        attempt: "a1",
        session_id: "p0d2-child-9-a1",
        tools: &[],
    },
];

fn motif_session_line(id: &str) -> String {
    format!(
        "{}\n",
        json!({
            "type": "session",
            "version": 3,
            "id": id,
            "timestamp": MOTIF_TS,
            "cwd": "C:\\SYNTHETIC\\p0d2",
            "title": "synthetic P0d-2 motif fixture"
        })
    )
}

fn motif_parent_line(spawn: u32, task: &MotifFixtureTask) -> String {
    format!(
        "{}\n",
        json!({
            "type": "custom",
            "timestamp": MOTIF_TS,
            "customType": "task_dispatch_v1",
            "data": {
                "dispatch_id": task.dispatch_id,
                "parent_session_id": MOTIF_PARENT_SESSION,
                "parent_tool_call_id": format!("call-motif-{spawn}"),
                "spawn_index": spawn,
                "project_id": "P0D2",
                "task_id": task.task,
                "attempt_id": task.attempt
            }
        })
    )
}

fn motif_child_line(dispatch_id: &str, session_id: &str) -> String {
    format!(
        "{}\n",
        json!({
            "type": "custom",
            "timestamp": MOTIF_TS,
            "customType": "task_dispatch_child_v1",
            "data": { "dispatch_id": dispatch_id, "session_id": session_id }
        })
    )
}

fn motif_turn_line(
    session_id: &str,
    ordinal: usize,
    parent: Option<&str>,
    tools: &[&str],
) -> String {
    let id = format!("{session_id}-e{ordinal}");
    let mut content = vec![json!({
        "type": "text",
        "text": format!("synthetic motif turn {ordinal}")
    })];
    for tool in tools {
        content.push(json!({
            "type": "toolCall",
            "id": format!("{id}-{tool}"),
            "name": tool,
            "args": { "path": format!("synthetic/{tool}.txt"), "flag": true }
        }));
    }
    let mut value = json!({
        "type": "message",
        "id": id,
        "parentId": Value::Null,
        "timestamp": MOTIF_TS,
        "message": { "role": "assistant", "content": content }
    });
    if let Some(parent) = parent {
        value["parentId"] = json!(parent);
    }
    format!("{value}\n")
}

fn cmd_fixture_motif(args: &[String]) -> Result<()> {
    let out = PathBuf::from(
        opt(args, "--out").ok_or_else(|| err("fixture-motif needs --out DIR"))?,
    );
    let omp = out.join("omp");
    let codex = out.join("codex");
    fs::create_dir_all(&omp).map_err(|e| err(format!("create {}: {e}", omp.display())))?;
    fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;

    let mut parent = motif_session_line(MOTIF_PARENT_SESSION);
    for (spawn, task) in MOTIF_FIXTURE_TASKS.iter().enumerate() {
        parent.push_str(&motif_parent_line(spawn as u32, task));
    }
    let parent_path = omp.join("p0d2").join("parent.jsonl");
    if let Some(dir) = parent_path.parent() {
        fs::create_dir_all(dir).map_err(|e| err(format!("create {}: {e}", dir.display())))?;
    }
    fs::write(&parent_path, parent)
        .map_err(|e| err(format!("write {}: {e}", parent_path.display())))?;

    for task in MOTIF_FIXTURE_TASKS {
        let mut content = motif_session_line(task.session_id);
        content.push_str(&motif_child_line(task.dispatch_id, task.session_id));
        if task.tools.is_empty() {
            content.push_str(&resolve_message_line(
                task.session_id,
                1,
                "assistant",
                "synthetic motif child without tool calls",
            ));
        } else {
            let mut parent_id: Option<String> = None;
            for (ordinal, tool) in task.tools.iter().enumerate() {
                content.push_str(&motif_turn_line(
                    task.session_id,
                    ordinal + 1,
                    parent_id.as_deref(),
                    &[*tool],
                ));
                parent_id = Some(format!("{}-e{}", task.session_id, ordinal + 1));
            }
        }
        let path = omp
            .join("p0d2")
            .join(format!("child-{}-{}.jsonl", task.task, task.attempt));
        fs::write(&path, content).map_err(|e| err(format!("write {}: {e}", path.display())))?;
    }

    println!(
        "wrote {} synthetic P0d-2 motif fixture tasks to {}",
        MOTIF_FIXTURE_TASKS.len(),
        out.display()
    );
    Ok(())
}

fn cmd_fixture_markers(args: &[String]) -> Result<()> {
    let out = PathBuf::from(
        opt(args, "--out").ok_or_else(|| err("fixture-markers needs --out DIR"))?,
    );
    let omp = out.join("omp");
    let codex = out.join("codex");
    fs::create_dir_all(&omp).map_err(|e| err(format!("create {}: {e}", omp.display())))?;
    fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;
    let mut content = String::new();
    for line in MARKER_FIXTURE_LINES {
        content.push_str(line);
        content.push('\n');
    }
    let path = omp.join("synthetic-p0c4-markers.jsonl");
    fs::write(&path, content).map_err(|e| err(format!("write {}: {e}", path.display())))?;
    println!(
        "wrote {} synthetic P0c-4 marker fixture lines to {}",
        MARKER_FIXTURE_LINES.len(),
        out.display()
    );
    Ok(())
}

/// Read allowlisted prose blocks from one published generation, re-derive each
/// block from raw bytes at its pinned span, and emit marker observations only
/// after the span hash and block digest both verify. No prose is emitted.
fn cmd_audit_markers(args: &[String]) -> Result<()> {
    let (idx, store, published) = require_publication(args)?;
    let VerifiedProse {
        blocks_total,
        excluded_role_blocks,
        unverified_blocks,
        texts,
        kept,
    } = verified_prose(&idx, &store, &published)?;

    let result = ase_markers::audit(&texts);
    let rule = &result.rule;
    let rule_value = json!({
        "name": &rule.name, "version": &rule.version, "matcher": &rule.matcher
    });
    let observations = result
        .observations
        .iter()
        .map(|o| {
            let b = &kept[o.block_index];
            let reopen = format!(
                "ase open --event {} --selector {} --generation {} --index {}",
                b.event,
                b.selector,
                published.generation,
                index_path(args).display()
            );
            json!({
                "value": &o.value,
                "usage": o.usage.label(),
                "reason": o.reason,
                "assignment_cue": &o.assignment_cue,
                "mention_cue": &o.mention_cue,
                "occurrence_in_block": o.occurrence_in_block,
                "char_start": o.char_start,
                "char_end": o.char_end,
                "evidence": {
                    "generation": &published.generation,
                    "event": b.event,
                    "span": span_label(b.span),
                    "selector": &b.selector,
                    "block_hash": &b.block_hash,
                    "role": &b.role,
                    "session_local": b.session_local,
                    "parent_idx": b.parent_idx,
                    "span_hash_match": true,
                    "block_digest_match": true,
                    "reopen": reopen,
                    "note": "Prose is re-derived from raw bytes at the pinned span by the versioned selector; it is never stored or emitted by this audit."
                },
                "rule": rule_value.clone(),
                "task_key": Value::Null,
                "assigned_to": Value::Null,
                "session_assignment": Value::Null
            })
        })
        .collect::<Vec<Value>>();
    let distinct = result
        .observations
        .iter()
        .map(|o| o.value.clone())
        .collect::<std::collections::BTreeSet<String>>()
        .len();

    let v = json!({
        "command": "audit",
        "subject": "markers",
        "status": "ok",
        "rule": {
            "name": &rule.name,
            "version": &rule.version,
            "matcher": &rule.matcher,
            "domain": &rule.domain,
            "abstention": &rule.abstention,
            "task_boundary": &rule.task_boundary
        },
        "source": {
            "kind": "published_prose_blocks",
            "generation": &published.generation,
            "index": index_path(args).to_string_lossy(),
            "fields": ["blocks.generation", "blocks.event_idx", "blocks.selector", "blocks.span", "blocks.block_hash", "blocks.role", "raw.span bytes", "PROJECTION_VERSION"],
            "raw_span_read": true,
            "prose_stored": false,
            "prose_emitted": false,
            "note": "Only allowlisted user/assistant prose blocks of one published generation are scanned. Raw spans are re-read to verify and re-derive the block; prose text is never stored or printed."
        },
        "counts": {
            "blocks_total": blocks_total,
            "blocks_scanned": result.blocks_scanned,
            "excluded_role_blocks": excluded_role_blocks,
            "unverified_blocks": unverified_blocks,
            "occurrences": result.occurrences,
            "assignment_context": result.assignment_context,
            "mention": result.mention,
            "ambiguous": result.ambiguous,
            "distinct_values": distinct
        },
        "observations": observations,
        "task_assignment": "none",
        "task_key": Value::Null,
        "explain": if has(args, "--explain") {
            vec![
                "Read every prose block row pinned to the active generation from the published postings layer; thinking, tool output, system, unknown and call-signature blocks are never prose rows.".to_string(),
                "Re-read the pinned raw span, check the FNV span hash against the address event, re-derive the block with its versioned selector and require the block digest to match before scanning any marker.".to_string(),
                "Scan only the exact task_id= key with an ASCII marker value; every occurrence is retained with its in-block character offsets, never deduplicated into a session-level claim.".to_string(),
                "Classify each occurrence from assignment and mention cues in its local window, bounded at the next marker so one marker's cue cannot justify another; no cue or both cue classes means ambiguous/abstain.".to_string(),
                "Every observation carries generation, event, span, selector and block digest for reopening with `ase open`, plus rule name/version and null task_key/assigned_to/session_assignment: this rule observes markers only.".to_string()
            ]
        } else {
            Vec::<String>::new()
        },
        "guarantees": [
            "Metadata-only by default: no prose text, snippet or session-level summary is emitted; the exact block is reopened from raw bytes with the same generation.",
            "Only user/assistant prose blocks that verify by span hash and block digest are scanned; tool output and thinking markers cannot appear.",
            "Multiple markers, repeated references and every occurrence are retained separately; no first-token or single-session promotion exists.",
            "Assignment-context, mention and ambiguous statuses remain candidate signals with their cues and reasons; ambiguity is never resolved by this rule.",
            "No TaskKey, assigned_to edge or session-level inference is produced; reconciling signals is P0c-5."
        ]
    });
    emit(&v, args);
    Ok(())
}
