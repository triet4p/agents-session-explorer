//! Command-line surface for the address index, derived search layer, task
//! resolution, work projection, motif, judge and evidence-bundle layers.
//!
//! All decoding, addressing and projection derivation lives in library crates;
//! the CLI only renders metadata and composes those layers. No model runs on a
//! query path. Metadata-only audits cover file-name grammar, `parentSession`
//! delegation and `task_id` markers; `resolve task:` reconciles those signals
//! with native OMP `task_dispatch_v1` / `task_dispatch_child_v1` records. The
//! record loader itself lives in `ase-motif::membership` so the motif
//! projection reuses exactly the same validated membership. Bundle verification
//! reopens evidence against a private local source map; `verify --all` remains
//! the address span check.

use ase_core::{err, EventIdx, ProjectionKind, RawStore, Result, Role, SpanRef, NONE};
use ase_work::{
    aggregate, reconciled, JournalAnalytics, JournalProjection, JournalRow, ManifestEntry,
    ReferenceRow, Window,
};
use rayon::prelude::*;
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
    Basis, DelegationEvidence, Evidence as TaskEvidence, MarkerEvidence, MarkerLayer,
    NamingEvidence, NamingSignal, PathEvidence, Resolution, Stance, Status as TaskStatus,
};
use ase_motif::membership::dispatch_input;
use ase_adapter_omp_stock::{
    path_eq, reopen, scan, ClaimsFile, Options as AdapterOptions, StockView, ViewEvent, ViewFile,
};
use ase_judge::Scope;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::fs;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
mod verify;
mod export;
mod import;
mod join;
mod config;

const DEFAULT_INDEX: &str = "ase.index";

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
        "fixture-address" => cmd_fixture_address(rest),
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
        "fixture-delta" => cmd_fixture_delta(rest),
        "motif" => cmd_motif(rest),
        "export" => export::run(rest),
        "import" => import::run(rest),
        "join" => join::run(rest),
        "adapter" => cmd_adapter(rest),
        "work" => cmd_work(rest),
        "analytics" => cmd_analytics(rest),
        "judge" => cmd_judge(rest),
        "llm" => cmd_llm(rest),
        "fixture-analytics" => cmd_fixture_analytics(rest),
        "fixture-adapter" => cmd_fixture_adapter(rest),
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
        "ase (address index / derived search)\n\n\
         ase index   [--layer address|postings|all] [--omp-root DIR] [--codex-root DIR]\n\
                     [--index FILE] [--fail-before-publish] [--json]\n\
         ase show    <span:F:L:LEN|session:ID> [--role ROLE] [--ordinal A:B] [--explain] [--content] [--json]\n\
         ase resolve <session:ID|span:F:L:LEN|task:TOKEN> [--explain] [--json]\n\
         ase verify  [--all | --sample N] [--index FILE] [--json]\n\
         ase verify  <BUNDLE-DIR> [--local-map FILE] [--verdicts FILE] [--json]\n\
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
         ase fixture-delta --out DIR\n\
         ase motif  [PROJECT|TASK|TOKEN]... [--tau F] [--k-min N] [--k-max N]\n\
                     [--max-gap N] [--band N] [--max-substitutions N]\n\
                     [--null-permutations N] [--min-tasks N] [--explain] [--json]\n\
                     (PROJECT|TASK addresses one exact sampling key; a bare\n\
                     TOKEN stays ambiguous when two projects share it)\n\
         ase export motif:<id> --out DIR --local-map FILE\n\
                     (--address PROJECT|TASK ... | --addresses FILE)\n\
                     [--index FILE] [--omp-root DIR] [--codex-root DIR] [--json]\n\
                     (writes only path-free bundle files; the private alias map stays outside)\n\
         ase import verdicts FILE --bundle DIR --local-map FILE --store FILE [--json]\
                     (reopens the bundle first; accepted assertions stay in the local store)\
         ase join skill-gap --store FILE --catalog FILE --bundle DIR --local-map FILE [--verdicts FILE] [--json]\
                     (read-only exact-catalog join of verified assertions; no store write)\
         ase adapter omp-stock (--session ID | --file N | --all) [--limit N]\n\
                     [--no-sidecars] [--out CLAIMS.json] [--explain] [--json]\n\
         ase adapter omp-stock --reopen CLAIMS.json [--claim ID] [--json]\n\
         ase work project [--index FILE] [--omp-root DIR] [--start ISO --end ISO]\n\
                     [--out ROWS.jsonl] [--limit N] [--json]\n\
         ase work session (--session ID | --file N) [--leaf ENTRY_ID]\n\
                     [--omp-root DIR] [--out PROJECTION.json] [--json]\n\
         ase work reopen PROJECTION.json [--index FILE] [--json]\n\
         ase work compare --projection ROWS.jsonl --reference SOURCES.jsonl\n\
                     [--out REPORT.json] [--json]\n\
                     (leaf-aware Turn/Episode projection; task-agnostic;\n\
                     resolver and motif membership are never read or written)\n\
         ase work delta --task TOKEN [--project PROJECT] --before SESSION --after SESSION\n\
                     [--omp-root DIR] [--out DELTA.json] [--before-out F] [--after-out F]\n\
                     (explicit attempts of one dispatch-confirmed project/task;\n\
                     task order by dispatch time, not by attempt label)\n\
         ase analytics project [--index FILE] [--omp-root DIR] [--start ISO --end ISO]\n\
                     [--out ROWS.jsonl] [--report REPORT.json] [--limit N] [--json]\n\
         ase analytics session (--session ID | --file N) [--leaf ENTRY_ID]\n\
                     [--omp-root DIR] [--out ROW.json] [--json]\n\
         ase analytics reopen ROWS.jsonl [--index FILE] [--json]\n\
         ase fixture-analytics --out DIR\n\
                     (nominal stored cost, duration vs clock union, coverage;\n\
                     no repricing and no stats.db dependency)\n\
         ase fixture-adapter --out DIR\n\
                     (advisory claims only; resolver and motif membership unchanged)\n\
         ase fixture-motif --out DIR\n\
         ase judge run --root DIR --manifest FILE --units FILE --private FILE\n\
                     --store FILE [--scope primary|unknown|all] [--only-ordinal N]...\n\
                     [--limit N] [--workspace DIR] [--verification-only]\n\
                     [--slm-runner PROG --slm-arg ARG... --slm-model-id ID\n\
                      (--slm-model-sha256 HEX | --slm-weights FILE)] [...]\n\
                     [--slm-profile NAME [--slm-api-timeout-ms N]]\n\
                     [--judge-runner ... --judge-model-id ID ...]\n\
                     [--judge-profile NAME [--judge-api-timeout-ms N]]\n\
                     [--allow-remote-session-data] [--out REPORT.json]\n\
                     (API profiles come from `ase llm`; remote egress needs opt-in)\n\
         ase judge verify --root DIR --units FILE --private FILE --store FILE\n\
                     [--include-verification]   (reopen every stored evidence span)\n\
         ase judge report --store FILE [--include-verification]\n\
         ase judge show --store FILE [--unit ID] [--decision N] [--include-verification]\n\
         ase judge fixture --out DIR   (synthetic contract fixture; no real prose)\n\
         ase judge contract-runner [--label split|continue|unknown] [--fail]\n\
                     [--model-id ID] [--model-sha256 HEX]\n\
                     (local-model protocol double for cache/contract verification)\n\
         ase llm config [--json] [--profile NAME]\n\
                     (redacted local status; no network)\n\
         ase llm config set --profile NAME --provider NAME\n\
                     [--type chat-completions|responses|decision|gemini|gemini-decision] [--endpoint URL]\n\
                     [--model MODEL] [--effort minimal|low|medium|high]\n\
                     [--use] [--json]\n\
         ase llm config use --profile NAME [--json]\n\
         ase llm config enroll --profile NAME --provider NAME\n\
                     [--type ...] [--endpoint URL] [--model MODEL] [--effort ...]\n\
                     [--use] [--json]\n\
                     (non-secret settings plus one-time hidden key input)\n\
         ase llm key set [--profile NAME] [--json]   (hidden prompt; never argv)\n\
         ase llm key clear [--profile NAME] [--json]\n\
         ase llm status [--profile NAME] [--json]\n\
         ase fixture-address --out DIR"
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
    let omp = PathBuf::from(opt(args, "--omp-root").unwrap_or_else(|| config::omp_root()));
    let codex = PathBuf::from(opt(args, "--codex-root").unwrap_or_else(|| config::codex_root()));
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
            "task: resolution is candidate-only at this layer: a literal path token hit is NOT task identity and NOT an assignment. Naming grammar, prose task_id, continuation and dispatch reconciliation are reported separately; none are guessed here.",
            "Every relation carries at least one raw byte span; stale files are reported, not silently served.",
            "No postings, motif, turn or judge data is used by this relation view."
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

// ------------------------------------------------ task identity resolution

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
        project: None,
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

pub(crate) fn require_publication(args: &[String]) -> Result<(Arc<Index>, FsRawStore, Publication)> {
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

const ADDRESS_OMP: &str = r#"{"type":"session","version":3,"id":"addr-session-omp","timestamp":"2026-02-03T04:05:06.000Z","cwd":"C:\\SYNTHETIC\\addr","title":"synthetic"}
{"type":"message","id":"addr-e1","parentId":null,"timestamp":"2026-02-03T04:05:07.000Z","message":{"role":"user","content":[{"type":"text","text":"ADDRESS PROSE ALPHA \u00e9\u2603"},{"type":"text","text":"ADDRESS PROSE BETA"}]}}
{"type":"message","id":"addr-e2","parentId":"addr-e1","timestamp":"2026-02-03T04:05:08.000Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"ADDRESS EXCLUDED THINKING"},{"type":"text","text":"ADDRESS ASSISTANT GAMMA"},{"type":"toolCall","id":"addr-call-a","name":"alphaTool","args":{"path":"synthetic/a.txt","flag":true}},{"type":"toolCall","id":"addr-call-b","name":"betaTool","args":{"path":"synthetic/b.txt","count":2}},{"type":"toolCall","id":"addr-call-c","name":"gammaTool","args":{}}]}}
{"type":"message","id":"addr-e3","parentId":"addr-e1","timestamp":"2026-02-03T04:05:09.000Z","message":{"role":"toolResult","toolCallId":"addr-call-a","content":[{"type":"text","text":"ADDRESS EXCLUDED TOOL OUTPUT"}]}}
{"type":"message","id":"addr-e5","parentId":"addr-e2","timestamp":"2026-02-03T04:05:10.000Z","message":{"role":"assistant","content":[{"type":"text","text":"ADDRESS LINEAR DELTA"},{"type":"toolCall","id":"addr-call-e","name":"epsilonTool","args":{}}]}}
{"type":"message","id":"addr-e6","parentId":"addr-e5","timestamp":"2026-02-03T04:05:11.000Z","message":{"role":"assistant","content":[{"type":"text","text":"ADDRESS LINEAR ETA"},{"type":"toolCall","id":"addr-call-f","name":"zetaTool","args":{}}]}}
{"type":"message","id":"addr-fork-a","parentId":"addr-e2","timestamp":"2026-02-03T04:05:12.000Z","message":{"role":"assistant","content":[{"type":"toolCall","id":"addr-call-ga","name":"forkATool","args":{}}]}}
{"type":"message","id":"addr-fork-b","parentId":"addr-e2","timestamp":"2026-02-03T04:05:13.000Z","message":{"role":"assistant","content":[{"type":"toolCall","id":"addr-call-gb","name":"forkBTool","args":{}}]}}
{this line is deliberately not json}
"#;

const ADDRESS_CODEX: &str = r#"{"timestamp":"2026-02-03T04:05:06.000Z","ordinal":0,"type":"session_meta","payload":{"id":"addr-session-codex","cwd":"C:\\SYNTHETIC\\addr"}}
{"timestamp":"2026-02-03T04:05:07.000Z","ordinal":1,"type":"event_msg","payload":{"type":"user_message","message":"ADDRESS CODEX EPSILON \u00e9"}}
{"timestamp":"2026-02-03T04:05:08.000Z","ordinal":2,"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ADDRESS CODEX ZETA"},{"type":"output_text","text":"ADDRESS CODEX ETA"}]}}
{"timestamp":"2026-02-03T04:05:09.000Z","ordinal":3,"type":"response_item","payload":{"type":"function_call","call_id":"addr-call-x","name":"alphaTool","arguments":"{\"path\":\"synthetic/x.txt\",\"flag\":true}"}}
{"timestamp":"2026-02-03T04:05:10.000Z","ordinal":4,"type":"response_item","payload":{"type":"function_call_output","call_id":"addr-call-x","output":"ADDRESS EXCLUDED TOOL OUTPUT"}}
{"timestamp":"2026-02-03T04:05:11.000Z","ordinal":5,"type":"response_item","payload":{"type":"reasoning","summary":[{"type":"summary_text","text":"ADDRESS EXCLUDED THINKING"}]}}
"#;

const ADDRESS_CODEX_SECOND: &str = r#"{"timestamp":"2026-02-03T05:05:06.000Z","ordinal":0,"type":"session_meta","payload":{"id":"addr-session-codex-2","cwd":"C:\\SYNTHETIC\\addr"}}
{"timestamp":"2026-02-03T05:05:07.000Z","ordinal":1,"type":"response_item","payload":{"type":"function_call","call_id":"addr-call-x","name":"alphaTool","arguments":"{\"path\":\"synthetic/y.txt\",\"flag\":true}"}}
{"timestamp":"2026-02-03T05:05:08.000Z","ordinal":2,"type":"event_msg","payload":{"type":"user_message","message":"ADDRESS OTHER SESSION THETA"}}
"#;

fn cmd_fixture_address(args: &[String]) -> Result<()> {
    let out = PathBuf::from(opt(args, "--out").ok_or_else(|| err("fixture-address needs --out DIR"))?);
    let omp = out.join("omp");
    let codex = out.join("codex");
    std::fs::create_dir_all(&omp).map_err(|e| err(format!("create {}: {e}", omp.display())))?;
    std::fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;
    std::fs::write(omp.join("synthetic-addr.jsonl"), ADDRESS_OMP)
        .map_err(|e| err(format!("write fixture: {e}")))?;
    let codex_bytes = ADDRESS_CODEX.as_bytes();
    std::fs::write(codex.join("synthetic-addr.jsonl"), codex_bytes)
        .map_err(|e| err(format!("write fixture: {e}")))?;
    let compressed = zstd::stream::encode_all(std::io::Cursor::new(codex_bytes), 3)
        .map_err(|e| err(format!("zstd fixture: {e}")))?;
    std::fs::write(codex.join("synthetic-addr.jsonl.zst"), compressed)
        .map_err(|e| err(format!("write fixture: {e}")))?;
    std::fs::write(codex.join("synthetic-addr-second.jsonl"), ADDRESS_CODEX_SECOND)
        .map_err(|e| err(format!("write fixture: {e}")))?;
    println!("wrote synthetic address fixtures to {}", out.display());
    Ok(())
}

// ------------------------------------------------------------ naming audit

// Only synthetic names are used for rule work. Each tuple is
// (parent directory, file basenames); project comes from the name itself.
const NAMING_FIXTURE: &[(&str, &[&str])] = &[
    ("multiple-splits", &["Proj18Task70BS01", "Proj18Task70SR02", "Proj18Task70DEP3"]),
    ("marker-t", &["Proj14T3ProviderA5", "Proj14T3ReviewB7"]),
    ("shared-prefix-tie", &["Proj813BoundaryGold23", "Proj813BoundarySilver24"]),
    ("missing-suffix", &["Proj14T3ProviderA", "Proj14T3OtherB9"]),
    ("collision-a", &["Proj18Task70BS01"]),
    ("collision-b", &["Proj18Task70BS01"]),
    ("cross-project", &["Proj18Task70BS01", "Proj19Task70BS01"]),
];

fn naming_fixture_content(sid: &str) -> String {
    format!(
        "{{\"type\":\"session\",\"version\":3,\"id\":\"{sid}\",\"timestamp\":\"2026-03-04T05:06:07.000Z\",\"cwd\":\"C:\\\\SYNTHETIC\\\\naming\",\"title\":\"synthetic naming fixture\",\"parentSession\":\"naming-parent\"}}\n{{\"type\":\"message\",\"id\":\"{sid}-e1\",\"parentId\":null,\"timestamp\":\"2026-03-04T05:06:08.000Z\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"SYNTHETIC NAMING TOKEN\"}}]}}}}\n"
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
            let sid = format!("naming-{parent}-{name}");
            let path = dir.join(format!("{name}.jsonl"));
            fs::write(&path, naming_fixture_content(&sid))
                .map_err(|e| err(format!("write {}: {e}", path.display())))?;
            written += 1;
        }
    }
    println!("wrote {written} synthetic naming fixtures to {}", out.display());
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

// ----------------------------------------------- parentSession delegation

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
        "{{\"type\":\"session\",\"version\":3,\"id\":\"{id}\",\"timestamp\":\"{DELEGATION_TS}\",\"cwd\":\"C:\\\\SYNTHETIC\\\\delegation\",\"title\":\"synthetic delegation fixture\"{parent_field}}}\n"
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
        "wrote {written} synthetic delegation fixtures to {}",
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

// ------------------------------------------------------------ marker audit

// Synthetic markers only. One OMP session carries allowlisted user/assistant
// text plus deliberately marker-bearing thinking, tool output and call
// arguments that the allowlist must never project as prose.
const MARKER_FIXTURE_LINES: &[&str] = &[
    r#"{"type":"session","version":3,"id":"marker-fixture-session","timestamp":"2026-03-06T07:08:09.000Z","cwd":"C:\\SYNTHETIC\\markers","title":"synthetic marker fixture"}"#,
    r#"{"type":"message","id":"marker-e1","parentId":null,"timestamp":"2026-03-06T07:08:10.000Z","message":{"role":"user","content":[{"type":"text","text":"Please implement task_id=SYN-A1 and task_id=SYN-A2 in the parser."}]}}"#,
    r#"{"type":"message","id":"marker-e2","parentId":null,"timestamp":"2026-03-06T07:08:11.000Z","message":{"role":"assistant","content":[{"type":"text","text":"I will work on task_id=SYN-A1. Please implement task_id=SYN-A1 again."}]}}"#,
    r#"{"type":"message","id":"marker-e3","parentId":null,"timestamp":"2026-03-06T07:08:12.000Z","message":{"role":"user","content":[{"type":"text","text":"Earlier we mentioned task_id=SYN-M1 for context only."}]}}"#,
    r#"{"type":"message","id":"marker-e4","parentId":null,"timestamp":"2026-03-06T07:08:13.000Z","message":{"role":"assistant","content":[{"type":"text","text":"Implement task_id=SYN-X1 but note task_id=SYN-X1 for reference."}]}}"#,
    r#"{"type":"message","id":"marker-e5","parentId":null,"timestamp":"2026-03-06T07:08:14.000Z","message":{"role":"user","content":[{"type":"text","text":"task_id=SYN-U1"}]}}"#,
    r#"{"type":"message","id":"marker-e6","parentId":null,"timestamp":"2026-03-06T07:08:15.000Z","message":{"role":"assistant","content":[{"type":"thinking","text":"task_id=SYN-THINK assigned here"},{"type":"text","text":"Please implement task_id=SYN-A3."}]}}"#,
    r#"{"type":"message","id":"marker-e7","parentId":null,"timestamp":"2026-03-06T07:08:16.000Z","message":{"role":"assistant","content":[{"type":"toolCall","name":"synthetic_tool","args":"{\"task_id\":\"SYN-CALL\",\"mode\":1}"}]}}"#,
    r#"{"type":"message","id":"marker-e8","parentId":null,"timestamp":"2026-03-06T07:08:17.000Z","message":{"role":"toolResult","toolCallId":"marker-call","content":[{"type":"text","text":"tool output only: task_id=SYN-TOOL assigned here"}]}}"#,
];

// ------------------------------------------------------- resolve fixture

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
        rel: "resolve/Proj10Task101Alpha1.jsonl",
        id: "fx-101-alpha",
        parent: None,
        messages: &[("user", "Please implement task_id=101 in the parser.")],
    },
    ResolveFixtureFile {
        rel: "resolve/Proj10Task102Beta1.jsonl",
        id: "fx-102-beta",
        parent: None,
        messages: &[(
            "assistant",
            "Please implement task_id=999 for this change.",
        )],
    },
    ResolveFixtureFile {
        rel: "resolve/Proj10Task103Gamma1.jsonl",
        id: "fx-103-gamma-1",
        parent: None,
        messages: &[("assistant", "Working on the naming grammar only.")],
    },
    ResolveFixtureFile {
        rel: "resolve/Proj10Task103Gamma3.jsonl",
        id: "fx-103-gamma-3",
        parent: None,
        messages: &[("user", "Same task, later attempt label.")],
    },
    ResolveFixtureFile {
        rel: "resolve/Proj10Task108Eta1.jsonl",
        id: "fx-108-eta",
        parent: None,
        messages: &[(
            "user",
            "Please implement task_id=108 and task_id=777 as separate follow-ups.",
        )],
    },
    ResolveFixtureFile {
        rel: "resolve/Proj20Task55Gold1.jsonl",
        id: "fx-55-alpha",
        parent: None,
        messages: &[("user", "Implement task_id=55 for project Proj20.")],
    },
    ResolveFixtureFile {
        rel: "resolve/Proj21Task55Gold2.jsonl",
        id: "fx-55-beta",
        parent: None,
        messages: &[("assistant", "Implement task_id=55 for project Proj21.")],
    },
    ResolveFixtureFile {
        rel: "resolve/session-notes.jsonl",
        id: "fx-notes",
        parent: None,
        messages: &[(
            "user",
            "Please implement task_id=555, but the filename has no grammar.",
        )],
    },
    ResolveFixtureFile {
        rel: "resolve/session-mention.jsonl",
        id: "fx-mention",
        parent: None,
        messages: &[(
            "user",
            "Earlier we mentioned task_id=666 for reference only.",
        )],
    },
    ResolveFixtureFile {
        rel: "resolve/syn-parent-root.jsonl",
        id: "syn-parent-root",
        parent: None,
        messages: &[("user", "root synthetic session")],
    },
    ResolveFixtureFile {
        rel: "resolve/syn-parent-child.jsonl",
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
        "{{\"type\":\"session\",\"version\":3,\"id\":\"{id}\",\"timestamp\":\"{RESOLVE_TS}\",\"cwd\":\"C:\\\\SYNTHETIC\\\\resolve\",\"title\":\"synthetic resolve fixture\"{parent_field}}}\n"
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
        "wrote {} synthetic resolve fixtures to {}",
        RESOLVE_FIXTURE.len(),
        out.display()
    );
    Ok(())
}

// ------------------------------------------------------ dispatch fixture

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

// Synthetic only, shaped exactly like native OMP dispatch
// records. The parent journal carries positive, rejected and competing
// claims; each child journal is the only place its link is valid.
const DISPATCH_PARENTS: &[DispatchFixtureParent] = &[
    DispatchFixtureParent {
        dispatch_id: D_410,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-410"),
        spawn_index: 0,
        project: "Proj30",
        task: "410",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_10410,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-10410"),
        spawn_index: 1,
        project: "Proj30",
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
        rel: "dispatch/parent-root.jsonl",
        session_id: DISPATCH_PARENT_SESSION,
        messages: &[("user", "dispatch fixture root session")],
        parents: DISPATCH_PARENTS,
        children: &[],
        raw_lines: DISPATCH_RAW_LINES,
    },
    DispatchFixtureFile {
        rel: "dispatch/Proj30Task410Alpha1.jsonl",
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
        rel: "dispatch/Proj30Task10410Beta1.jsonl",
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
        rel: "dispatch/TrackedOnlyGamma2.jsonl",
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
        rel: "dispatch/Proj99Task999Delta3.jsonl",
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
        rel: "dispatch/DupParentZeta1.jsonl",
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
        rel: "dispatch/DupChildEta1.jsonl",
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
        rel: "dispatch/WrongChildTheta1.jsonl",
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
        rel: "dispatch/MismatchParentIota1.jsonl",
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
        rel: "dispatch/OrphanChildKappa1.jsonl",
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
        rel: "dispatch/CompetingChildLambda1.jsonl",
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
        rel: "dispatch/Proj40Task888Legacy1.jsonl",
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
        "{{\"type\":\"session\",\"version\":3,\"id\":\"{id}\",\"timestamp\":\"{DISPATCH_TS}\",\"cwd\":\"C:\\\\SYNTHETIC\\\\dispatch\",\"title\":\"synthetic dispatch fixture\"}}\n"
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
        "wrote {} synthetic dispatch fixtures to {}",
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

pub(crate) struct MotifTarget {
    pub(crate) address: String,
    pub(crate) project: Option<String>,
    pub(crate) token: String,
}

/// Parse one motif address. `PROJECT|TASK` is the locked `(project, task)`
/// sampling key; a bare token keeps the unscoped contract and stays ambiguous
/// when two projects share it. No fallback rewrites one form into the other.
pub(crate) fn motif_target(address: &str) -> Result<MotifTarget> {
    match address.split_once('|') {
        Some((project, task)) => {
            if project.is_empty() || task.is_empty() || task.contains('|') {
                return Err(err(format!(
                    "motif address {address} must be PROJECT|TASK with non-empty parts"
                )));
            }
            Ok(MotifTarget {
                address: address.to_string(),
                project: Some(project.to_string()),
                token: task.to_string(),
            })
        }
        None => Ok(MotifTarget {
            address: address.to_string(),
            project: None,
            token: address.to_string(),
        }),
    }
}

fn cmd_motif(args: &[String]) -> Result<()> {
    let positionals = positionals(args);
    if positionals.is_empty() {
        return Err(err("motif needs at least one task token"));
    }
    let targets = positionals
        .iter()
        .map(|address| motif_target(address))
        .collect::<Result<Vec<MotifTarget>>>()?;
    let (idx, store, published) = require_publication(args)?;

    let mut tasks = Vec::with_capacity(targets.len());
    let mut task_rows = Vec::with_capacity(targets.len());
    for target in &targets {
        let member =
            ase_motif::membership_scoped(&idx, &store, &target.token, target.project.as_deref())?;
        let projected = ase_motif::project(&published, &idx, &member)?;
        task_rows.push(json!({
            "address": &target.address,
            "addressing": if target.project.is_some() { "project_task" } else { "token" },
            "token": &target.token,
            "project": &member.project,
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
                    "rule": "distinct task keys; a task counts once no matter how many attempts or occurrences contain the motif; duplicate sequence text inside one task is counted once"
                },
                "null": {
                    "permutations": null_model.permutations,
                    "seed": null_model.seed.to_string(),
                    "observed": null_model.observed,
                    "expected": null_model.expected,
                    "stddev": null_model.stddev,
                    "z": null_model.z,
                    "lift": null_model.lift,
                    "rule": "fixed-seed SplitMix64 permutation of each distinct sequence unit's call order; duplicate sequence text is not permuted twice; exact support recounted per task"
                },
                "denominator": {
                    "tasks_total": motif.denominator.tasks_total,
                    "tasks_in_family": motif.denominator.tasks_in_family,
                    "tasks_with_occurrence": motif.denominator.tasks_with_occurrence,
                    "sequences_in_family": motif.denominator.sequences_in_family,
                    "sequences_with_occurrence": motif.denominator.sequences_with_occurrence,
                    "sequences_deduplicated": motif.denominator.sequences_deduplicated,
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
                    "text_hash": &occurrence.text_hash,
                    "text_duplicate": occurrence.duplicate_of.is_some(),
                    "duplicate_of": occurrence.duplicate_of.as_ref().map(|reference| json!({
                        "task": &reference.task,
                        "attempt": &reference.attempt,
                        "dispatch_id": &reference.dispatch_id,
                        "sequence": reference.sequence
                    })),
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
        "Membership is one native dispatch join per attempt; naming, path and parentSession candidates are never inputs. `PROJECT|TASK` filters the same dispatch validator before reconciliation, so one token in two projects stays two independent samples and a bare token stays ambiguous.".to_string(),
        "Support counts distinct task keys: repeated attempts and repeated occurrences of one task are one observation.".to_string(),
        "Duplicate sequence text is the SHA-256 of a sequence's reopened CallSig strings (rule motif-sequence-text / seq-text-v1). Dedupe is scoped to one task key, so distinct tasks keep their own samples; duplicate whole sequences are excluded from family/null/denominator counting but every occurrence stays reopenable evidence, and calls inside a sequence are never removed or reordered.".to_string(),
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
            "dedupe": {
                "rule": { "name": ase_motif::TEXT_HASH_RULE_NAME, "version": ase_motif::TEXT_HASH_RULE_VERSION },
                "unit": "one projected CallSig sequence; text_hash = SHA-256 of its reopened signatures in order",
                "scope": "within one resolved task key (project|task); distinct tasks are never merged and no sequence crosses a branch or session boundary",
                "effect": "duplicate copies are excluded from family, null and sequence-denominator counting; support stays per task and every occurrence remains reopenable evidence"
            },
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
            "sequences_raw": report.sequences_raw,
            "sequences_deduplicated": report.deduplicated_sequences.len(),
            "families": report.families,
            "families_below_min_tasks": report.families_below_min_tasks,
            "motifs": motifs.len()
        },
        "excluded_tasks": report.excluded_tasks.iter().map(|exclusion| json!({
            "task": &exclusion.task,
            "reason": exclusion.reason.label()
        })).collect::<Vec<Value>>(),
        "deduplicated_sequences": report.deduplicated_sequences.iter().map(|duplicate| json!({
            "task": &duplicate.task,
            "attempt": &duplicate.attempt,
            "dispatch_id": &duplicate.dispatch_id,
            "sequence": duplicate.sequence,
            "text_hash": &duplicate.text_hash,
            "duplicate_of": {
                "task": &duplicate.duplicate_of.task,
                "attempt": &duplicate.duplicate_of.attempt,
                "dispatch_id": &duplicate.duplicate_of.dispatch_id,
                "sequence": duplicate.duplicate_of.sequence
            },
            "effect": "not counted; the occurrence remains reopenable evidence"
        })).collect::<Vec<Value>>(),
        "motifs": motifs,
        "explain": if has(args, "--explain") { explain } else { Vec::<String>::new() }
    });

    if has(args, "--json") {
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        println!(
            "motif: {} task(s), {} distinct sequence(s) ({} raw, {} duplicate text), {} family(ies), {} motif(s)",
            report.tasks,
            report.sequences,
            report.sequences_raw,
            report.deduplicated_sequences.len(),
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
                    "    occurrence task={} attempt={} gap={} text_duplicate={} first_event={}",
                    occurrence.task,
                    occurrence.attempt,
                    occurrence.gap,
                    occurrence.duplicate_of.is_some(),
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
const MOTIF_PARENT_SESSION: &str = "motif-fixture-parent";

const M_A1: &str = "a1a1a1a1-a1a1-4a1a-8a1a-a1a1a1a1a1a1";
const M_A2: &str = "a2a2a2a2-a2a2-4a2a-8a2a-a2a2a2a2a2a2";
const M_A3: &str = "a3a3a3a3-a3a3-4a3a-8a3a-a3a3a3a3a3a3";
const M_B1: &str = "b1b1b1b1-b1b1-4b1b-8b1b-b1b1b1b1b1b1";
const M_C1: &str = "c1c1c1c1-c1c1-4c1c-8c1c-c1c1c1c1c1c1";
const M_E1: &str = "e1e1e1e1-e1e1-4e1e-8e1e-e1e1e1e1e1e1";
const M_G1: &str = "71717171-7171-4171-8171-717171717171";
const M_G2: &str = "72727272-7272-4272-8272-727272727272";

struct MotifFixtureTask {
    project: &'static str,
    dispatch_id: &'static str,
    task: &'static str,
    attempt: &'static str,
    session_id: &'static str,
    tools: &'static [&'static str],
}

// Synthetic only. Task 1 repeats across three attempts: a1 is exact, a2 adds
// one call, and a3 repeats a1's text (deduped for counting, retained as
// evidence). Task 2 is gapped-only, task 3 is a near miss with one
// substitution, and task 9 is a valid join whose child has no call at all.
// Task 7 exists in two projects with the same call text so the fixture proves
// that a colliding bare token stays ambiguous while `PROJECT|TASK` yields two
// independent samples.
const MOTIF_FIXTURE_TASKS: &[MotifFixtureTask] = &[
    MotifFixtureTask {
        project: "MotifAlpha",
        dispatch_id: M_A1,
        task: "1",
        attempt: "a1",
        session_id: "motif-child-1-a1",
        tools: &["t1", "t2", "t3", "t4"],
    },
    MotifFixtureTask {
        project: "MotifAlpha",
        dispatch_id: M_A2,
        task: "1",
        attempt: "a2",
        session_id: "motif-child-1-a2",
        tools: &["t1", "t2", "x1", "t3", "t4"],
    },
    MotifFixtureTask {
        project: "MotifAlpha",
        dispatch_id: M_A3,
        task: "1",
        attempt: "a3",
        session_id: "motif-child-1-a3",
        tools: &["t1", "t2", "t3", "t4"],
    },
    MotifFixtureTask {
        project: "MotifAlpha",
        dispatch_id: M_B1,
        task: "2",
        attempt: "b1",
        session_id: "motif-child-2-b1",
        tools: &["t1", "t2", "y1", "t3", "t4"],
    },
    MotifFixtureTask {
        project: "MotifAlpha",
        dispatch_id: M_C1,
        task: "3",
        attempt: "c1",
        session_id: "motif-child-3-c1",
        tools: &["t1", "t2", "z1", "t4"],
    },
    MotifFixtureTask {
        project: "MotifAlpha",
        dispatch_id: M_E1,
        task: "9",
        attempt: "a1",
        session_id: "motif-child-9-a1",
        tools: &[],
    },
    MotifFixtureTask {
        project: "MotifAlpha",
        dispatch_id: M_G1,
        task: "7",
        attempt: "g1",
        session_id: "motif-child-7-g1",
        tools: &["s1", "s2", "s3", "s4"],
    },
    MotifFixtureTask {
        project: "MotifBeta",
        dispatch_id: M_G2,
        task: "7",
        attempt: "g2",
        session_id: "motif-alt-child-7-g2",
        tools: &["s1", "s2", "s3", "s4"],
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
            "cwd": "C:\\SYNTHETIC\\motif",
            "title": "synthetic motif fixture"
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
                "project_id": task.project,
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
    let parent_path = omp.join("motif").join("parent.jsonl");
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
            .join("motif")
            .join(format!(
                "child-{}-{}-{}.jsonl",
                task.project, task.task, task.attempt
            ));
        fs::write(&path, content).map_err(|e| err(format!("write {}: {e}", path.display())))?;
    }

    println!(
        "wrote {} synthetic motif fixture tasks to {}",
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
    let path = omp.join("synthetic-markers-markers.jsonl");
    fs::write(&path, content).map_err(|e| err(format!("write {}: {e}", path.display())))?;
    println!(
        "wrote {} synthetic marker fixture lines to {}",
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
            "No TaskKey, assigned_to edge or session-level inference is produced; reconciling signals happens in task resolution."
        ]
    });
    emit(&v, args);
    Ok(())
}

// ---------------------------------------------------- OMP stock adapter

struct AdapterView {
    idx: Arc<Index>,
    store: FsRawStore,
    files: Vec<ViewFile>,
    generation: String,
    roots: Vec<String>,
}

fn adapter_abs_path(path: &str) -> String {
    std::path::absolute(Path::new(path))
        .unwrap_or_else(|_| PathBuf::from(path))
        .to_string_lossy()
        .into_owned()
}

fn adapter_event(idx: &Index, event: u32) -> Result<ViewEvent> {
    let e = idx.event(event)?;
    Ok(ViewEvent {
        event,
        ordinal: e.ordinal(),
        span: e.span(),
        hash: e.hash(),
        kind: e.kind(),
        role: e.role(),
        tag: e.tag().to_string(),
    })
}

fn adapter_event_at(idx: &Index, span: SpanRef) -> Result<Option<ViewEvent>> {
    match idx.event_at(span)? {
        Some(event) => Ok(Some(adapter_event(idx, event)?)),
        None => Ok(None),
    }
}

impl StockView for AdapterView {
    fn generation(&self) -> &str {
        &self.generation
    }

    fn files(&self) -> &[ViewFile] {
        &self.files
    }

    fn events(&self, file: u32) -> Result<Vec<ViewEvent>> {
        let f = self.idx.file(file)?;
        let mut out = Vec::with_capacity((f.event_end() - f.event_start()) as usize);
        for event in f.event_start()..f.event_end() {
            out.push(adapter_event(&self.idx, event)?);
        }
        Ok(out)
    }

    fn bytes(&self, file: u32) -> Result<Arc<[u8]>> {
        self.store.bytes(file)
    }

    fn span_bytes(&self, span: SpanRef) -> Result<Vec<u8>> {
        self.store.span(span)
    }

    fn event_at(&self, span: SpanRef) -> Result<Option<ViewEvent>> {
        adapter_event_at(&self.idx, span)
    }

    fn session_header(&self, file: u32) -> Result<Option<ViewEvent>> {
        let f = self.idx.file(file)?;
        for event in f.event_start()..f.event_end() {
            let e = adapter_event(&self.idx, event)?;
            if e.kind == ase_core::EventKind::SessionMeta {
                return Ok(Some(e));
            }
        }
        Ok(None)
    }

    fn sidecar_bytes(&self, path: &str) -> Result<Option<Vec<u8>>> {
        if !path.to_ascii_lowercase().ends_with(".md") {
            return Ok(None);
        }
        let normalized = ase_adapter_omp_stock::normalize_path(path);
        let allowed = self
            .roots
            .iter()
            .any(|r| normalized.to_ascii_lowercase().starts_with(&r.to_ascii_lowercase()));
        if !allowed {
            return Ok(None);
        }
        let meta = match std::fs::metadata(&normalized) {
            Ok(m) => m,
            Err(_) => return Ok(None),
        };
        if meta.len() > 4 * 1024 * 1024 {
            return Ok(None);
        }
        match std::fs::read(&normalized) {
            Ok(b) => Ok(Some(b)),
            Err(_) => Ok(None),
        }
    }
}

fn adapter_view(
    args: &[String],
    reopen_mode: bool,
) -> Result<AdapterView> {
    let (idx, store, published, _) = load_with_publication(args)?;
    let generation = match &published {
        Some(p) => p.generation.clone(),
        None => format!(
            "address-{}-{}-{}",
            idx.build_ts, idx.file_count, idx.event_count
        ),
    };
    let mut digests: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    if let Some(p) = &published {
        for f in &p.manifest.files {
            digests.insert(f.id, f.digest.clone());
        }
    }
    let mut all: Vec<ViewFile> = Vec::new();
    for id in 0..idx.file_count {
        let f = idx.file(id)?;
        if f.provider().label() != "omp" {
            continue;
        }
        all.push(ViewFile {
            id,
            path: adapter_abs_path(f.path()),
            provider: f.provider(),
            encoding: f.encoding(),
            size: f.size(),
            manifest_digest: digests.get(&id).cloned(),
        });
    }

    let mut selected: std::collections::HashSet<u32> = std::collections::HashSet::new();
    if reopen_mode {
        selected.extend(all.iter().map(|f| f.id));
    } else if let Some(session) = opt(args, "--session") {
        let sid = idx
            .session_by_id(&session)
            .ok_or_else(|| err(format!("unknown session: {session}")))?;
        let s = idx.session(sid)?;
        let parent_file = s.file();
        let parent_path = all
            .iter()
            .find(|f| f.id == parent_file)
            .map(|f| f.path.clone())
            .ok_or_else(|| err("session file is not an OMP journal"))?;
        selected.insert(parent_file);
        let stem = parent_path
            .strip_suffix(".jsonl.zst")
            .or_else(|| parent_path.strip_suffix(".jsonl"))
            .unwrap_or(&parent_path)
            .replace('\\', "/");
        let prefix = format!("{stem}/");
        for f in &all {
            if ase_adapter_omp_stock::normalize_path(&f.path).starts_with(&prefix) {
                selected.insert(f.id);
            }
        }
        for i in 0..idx.session_count {
            let other = idx.session(i)?;
            if other.file() != parent_file && path_eq(other.parent_id(), &parent_path) {
                selected.insert(other.file());
            }
        }
    } else if let Some(file) = opt(args, "--file") {
        let file: u32 = file
            .parse()
            .map_err(|_| err("--file needs an integer file id"))?;
        let parent_path = all
            .iter()
            .find(|f| f.id == file)
            .map(|f| f.path.clone())
            .ok_or_else(|| err("file is not an OMP journal"))?;
        selected.insert(file);
        let stem = parent_path
            .strip_suffix(".jsonl.zst")
            .or_else(|| parent_path.strip_suffix(".jsonl"))
            .unwrap_or(&parent_path)
            .replace('\\', "/");
        let prefix = format!("{stem}/");
        for f in &all {
            if ase_adapter_omp_stock::normalize_path(&f.path).starts_with(&prefix) {
                selected.insert(f.id);
            }
        }
        for i in 0..idx.session_count {
            let other = idx.session(i)?;
            if other.file() != file && path_eq(other.parent_id(), &parent_path) {
                selected.insert(other.file());
            }
        }
    } else if has(args, "--all") {
        selected.extend(all.iter().map(|f| f.id));
    } else {
        return Err(err(
            "adapter omp-stock needs --session ID, --file N or --all",
        ));
    }

    let files: Vec<ViewFile> = all
        .into_iter()
        .filter(|f| selected.contains(&f.id))
        .collect();
    let mut roots: Vec<String> = Vec::new();
    for f in &files {
        let normalized = ase_adapter_omp_stock::normalize_path(&f.path);
        if let Some(at) = normalized.rfind('/') {
            let root = normalized[..at].to_string();
            if !roots.iter().any(|r| r.eq_ignore_ascii_case(&root)) {
                roots.push(root);
            }
        }
    }
    Ok(AdapterView { idx, store, files, generation, roots })
}

fn cmd_adapter(args: &[String]) -> Result<()> {
    if args.first().map(String::as_str) != Some("omp-stock") {
        return Err(err("adapter needs a provider: omp-stock"));
    }
    let rest = &args[1..];
    if let Some(path) = opt(rest, "--reopen") {
        let bytes = fs::read(&path)
            .map_err(|e| err(format!("read claims {}: {e}", path)))?;
        let claims: ClaimsFile = serde_json::from_slice(&bytes)
            .map_err(|e| err(format!("parse claims {}: {e}", path)))?;
        let view = adapter_view(rest, true)?;
        let report = reopen(&view, &claims, opt(rest, "--claim").as_deref())?;
        let v = serde_json::to_value(&report)
            .map_err(|e| err(format!("render reopen report: {e}")))?;
        emit(&v, rest);
        if report.status != "verified" {
            std::process::exit(4);
        }
        return Ok(());
    }

    let options = AdapterOptions {
        max_parent_files: parse_opt::<usize>(rest, "--limit")?,
        read_sidecars: !has(rest, "--no-sidecars"),
    };
    let view = adapter_view(rest, false)?;
    let report = scan(&view, &options)?;
    if let Some(out) = opt(rest, "--out") {
        let file = ClaimsFile {
            adapter: report.adapter.clone(),
            generation: report.generation.clone(),
            evidence_selector_version: report.evidence_selector_version.clone(),
            claims: report.claims.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&file)
            .map_err(|e| err(format!("serialize claims: {e}")))?;
        fs::write(&out, bytes).map_err(|e| err(format!("write claims {out}: {e}")))?;
    }
    let v = serde_json::to_value(&report)
        .map_err(|e| err(format!("render adapter report: {e}")))?;
    emit(&v, rest);
    Ok(())
}


// ---------------------------------------------------------- work projection

struct WorkFile {
    file_id: u32,
    rel: String,
    layout: ase_work::util::Layout,
}

struct SelectedWork {
    files: Vec<WorkFile>,
    root_journals_scanned: u64,
    root_journals_selected: u64,
    root_journals_outside_window: u64,
    root_journals_missing_header: u64,
}

struct ProjectedWork {
    projection: JournalProjection,
    entry: ManifestEntry,
}

fn work_usage() {
    println!(
        "ase work project [--index FILE] [--omp-root DIR] [--start ISO --end ISO]\n\
         [--out ROWS.jsonl] [--limit N] [--json]\n\
         ase work session (--session ID | --file N) [--leaf ENTRY_ID] [--omp-root DIR]\n\
         [--out PROJECTION.json] [--json]\n\
         ase work reopen PROJECTION.json [--index FILE] [--json]\n\
         ase work delta --task TOKEN [--project PROJECT] --before CHILD_SESSION_ID\n\
         \n         --after CHILD_SESSION_ID [--omp-root DIR] [--out DELTA.json]\n\
         \n         [--before-out FILE.json] [--after-out FILE.json] [--json]\n\
         ase work compare --projection ROWS.jsonl --reference SOURCES.jsonl\n\
         [--out REPORT.json] [--json]"
    );
}

fn cmd_work(args: &[String]) -> Result<()> {
    let Some(sub) = args.first().map(String::as_str) else {
        work_usage();
        return Err(err("work needs project, session, reopen, compare or delta"));
    };
    let rest = &args[1..];
    if matches!(sub, "--help" | "-h") {
        work_usage();
        return Ok(());
    }
    match sub {
        "project" => cmd_work_project(rest),
        "session" => cmd_work_session(rest),
        "reopen" => cmd_work_reopen(rest),
        "compare" => cmd_work_compare(rest),
        "delta" => cmd_work_delta(rest),
        other => Err(err(format!("unknown work command: {other}"))),
    }
}

fn load_work(args: &[String]) -> Result<(Arc<Index>, FsRawStore, String)> {
    let pointer = index_path(args);
    let address = match publication::open(&pointer) {
        Ok(Some(published)) => published.address_path,
        // A missing, tombstoned or mismatched derived layer never disables the
        // address-only turn projection.
        _ => pointer,
    };
    let idx = Arc::new(Index::open(&address)?);
    let (_, digest) = publication::sha256_file(&address)?;
    let generation = format!("index-sha256:{digest}");
    let store = FsRawStore::new(idx.clone());
    Ok((idx, store, generation))
}

fn parse_work_window(args: &[String]) -> Result<Option<Window>> {
    let start = opt(args, "--start");
    let end = opt(args, "--end");
    match (start, end) {
        (None, None) => Ok(None),
        (Some(start), Some(end)) => {
            let start_ms = ase_work::parse_ts_opt(Some(&start))
                .ok_or_else(|| err(format!("invalid --start timestamp: {start}")))?;
            let end_ms = ase_work::parse_ts_opt(Some(&end))
                .ok_or_else(|| err(format!("invalid --end timestamp: {end}")))?;
            if end_ms <= start_ms {
                return Err(err("--end must be after --start"));
            }
            Ok(Some(Window { start_ms, end_ms }))
        }
        _ => Err(err("work project needs both --start and --end, or neither")),
    }
}

fn header_events(idx: &Index) -> Result<std::collections::BTreeMap<u32, u32>> {
    let mut out = std::collections::BTreeMap::new();
    for session in 0..idx.session_count {
        let s = idx.session(session)?;
        if s.header() != NONE {
            out.insert(s.file(), s.header());
        }
    }
    Ok(out)
}

fn collect_work_files(
    idx: &Index,
    omp_root: &str,
    window: Option<Window>,
    limit: Option<usize>,
) -> Result<SelectedWork> {
    let headers = header_events(idx)?;
    let mut roots: std::collections::BTreeMap<String, (u32, ase_work::util::Layout, bool)> =
        std::collections::BTreeMap::new();
    let mut subagents: Vec<WorkFile> = Vec::new();
    let mut scanned = 0u64;
    let mut outside = 0u64;
    let mut missing = 0u64;
    for file_id in 0..idx.file_count {
        let file = idx.file(file_id)?;
        if file.provider() != ase_core::Provider::Omp {
            continue;
        }
        let Some(rel) = ase_work::relpath(file.path(), omp_root) else {
            continue;
        };
        let Some(layout) = ase_work::layout(&rel) else {
            continue;
        };
        if layout.kind == "root" {
            scanned += 1;
            let Some(header) = headers.get(&file_id) else {
                missing += 1;
                continue;
            };
            let ts = idx.event(*header)?.ts_ms();
            if ts == 0 {
                missing += 1;
                continue;
            }
            if window.is_some_and(|w| !w.contains(ts)) {
                outside += 1;
                continue;
            }
            roots.insert(layout.tree_relpath.clone(), (file_id, layout, true));
        } else {
            subagents.push(WorkFile { file_id, rel, layout });
        }
    }
    let selected_roots = roots.values().filter(|(_, _, selected)| *selected).count() as u64;
    let mut files: Vec<WorkFile> = Vec::new();
    for (file_id, layout, selected) in roots.values() {
        if *selected {
            files.push(WorkFile {
                file_id: *file_id,
                rel: layout.tree_relpath.clone(),
                layout: layout.clone(),
            });
        }
    }
    for sub in subagents {
        let Some((_, _, selected)) = roots.get(&sub.layout.tree_relpath) else {
            continue;
        };
        if *selected {
            files.push(sub);
        }
    }
    files.sort_by(|a, b| a.rel.as_bytes().cmp(b.rel.as_bytes()));
    if let Some(limit) = limit {
        files.truncate(limit);
    }
    Ok(SelectedWork {
        files,
        root_journals_scanned: scanned,
        root_journals_selected: selected_roots,
        root_journals_outside_window: outside,
        root_journals_missing_header: missing,
    })
}

/// Analytics window selection follows the contracted `message.timestamp`
/// period field, not the session-header event time. A root journal that began
/// before the window can carry in-window turns, so a scoped analytics run
/// projects every root (and its subagents) with complete branch context;
/// `aggregate` builds its `period` totals from in-window turns only.
/// `ase work project` keeps its separate header-time selection contract.
fn collect_analytics_files(
    idx: &Index,
    omp_root: &str,
    limit: Option<usize>,
) -> Result<SelectedWork> {
    let mut roots: std::collections::BTreeMap<String, (u32, ase_work::util::Layout)> =
        std::collections::BTreeMap::new();
    let mut subagents: Vec<WorkFile> = Vec::new();
    let mut scanned = 0u64;
    for file_id in 0..idx.file_count {
        let file = idx.file(file_id)?;
        if file.provider() != ase_core::Provider::Omp {
            continue;
        }
        let Some(rel) = ase_work::relpath(file.path(), omp_root) else {
            continue;
        };
        let Some(layout) = ase_work::layout(&rel) else {
            continue;
        };
        if layout.kind == "root" {
            scanned += 1;
            roots.insert(layout.tree_relpath.clone(), (file_id, layout));
        } else {
            subagents.push(WorkFile { file_id, rel, layout });
        }
    }
    let mut files: Vec<WorkFile> = roots
        .values()
        .map(|(file_id, layout)| WorkFile {
            file_id: *file_id,
            rel: layout.tree_relpath.clone(),
            layout: layout.clone(),
        })
        .collect();
    for sub in subagents {
        if roots.contains_key(&sub.layout.tree_relpath) {
            files.push(sub);
        }
    }
    files.sort_by(|a, b| a.rel.as_bytes().cmp(b.rel.as_bytes()));
    if let Some(limit) = limit {
        files.truncate(limit);
    }
    Ok(SelectedWork {
        files,
        root_journals_scanned: scanned,
        root_journals_selected: scanned,
        root_journals_outside_window: 0,
        root_journals_missing_header: 0,
    })
}

fn validate_work_projection(idx: &Index, projection: &JournalProjection) -> Result<()> {
    let check = |ev: &ase_work::EvidenceRef| -> Result<()> {
        let event = idx.event(ev.event)?;
        let expected = SpanRef { file: ev.span.file, off: ev.span.off, len: ev.span.len };
        if event.span() != expected {
            return Err(err(format!(
                "projection evidence span differs from index at event {}",
                ev.event
            )));
        }
        if format!("{:016x}", event.hash()) != ev.index_checksum {
            return Err(err(format!(
                "projection evidence checksum differs from index at event {}",
                ev.event
            )));
        }
        Ok(())
    };
    if let Some(ev) = &projection.active_leaf.evidence {
        check(ev)?;
    }
    for turn in &projection.turns {
        check(&turn.evidence)?;
    }
    for episode in projection.episodes_primary.iter().chain(projection.episodes_wide.iter()) {
        if let Some(ev) = &episode.evidence {
            check(ev)?;
        }
    }
    Ok(())
}

fn project_work_file(
    idx: &Index,
    store: &FsRawStore,
    file: &WorkFile,
    generation: &str,
    window: Option<Window>,
) -> Result<ProjectedWork> {
    let view = idx.file(file.file_id)?;
    let bytes = store.bytes(file.file_id)?;
    let sha = ase_work::sha256_hex(&bytes);
    let sid = ase_work::source_id(&file.rel);
    let options = ase_work::ProjectOptions {
        file_id: file.file_id,
        path: view.path(),
        source_id: &sid,
        generation,
        sha256: &sha,
        event_base: view.event_start(),
        explicit_leaf: None,
        window,
    };
    let projection = ase_work::project_bytes(&options, &bytes)?;
    validate_work_projection(idx, &projection)?;
    let head_len = bytes.len().min(4096);
    let tail_start = bytes.len().saturating_sub(4096);
    let entry = ManifestEntry {
        relpath: file.rel.clone(),
        size: bytes.len() as u64,
        sha256: sha.clone(),
        head_sha256: ase_work::sha256_hex(&bytes[..head_len]),
        tail_sha256: ase_work::sha256_hex(&bytes[tail_start..]),
        kind: file.layout.kind.to_string(),
        tree_relpath: file.layout.tree_relpath.clone(),
        session_id: projection.header.session_id.clone(),
        header_timestamp: projection.header.timestamp.clone(),
        parent_session: projection.header.parent_session.clone(),
        filename_timestamp: ase_work::filename_timestamp(&file.rel)
            .map(ase_work::format_iso_ms),
        header_ok: projection.header.header_ok,
    };
    Ok(ProjectedWork { projection, entry })
}

fn cmd_work_project(args: &[String]) -> Result<()> {
    let (idx, store, generation) = load_work(args)?;
    let window = parse_work_window(args)?;
    let omp_root = opt(args, "--omp-root").unwrap_or_else(|| config::omp_root());
    let limit = opt(args, "--limit").and_then(|s| s.parse::<usize>().ok());
    let selected = collect_work_files(&idx, &omp_root, window, limit)?;
    let projected: Vec<ProjectedWork> = selected
        .files
        .par_iter()
        .map(|file| project_work_file(&idx, &store, file, &generation, window))
        .collect::<Result<Vec<_>>>()?;

    let mut rows: Vec<JournalRow> = Vec::with_capacity(projected.len());
    let mut counts = ase_work::AggregateCounts::default();
    let mut entries: Vec<ManifestEntry> = Vec::with_capacity(projected.len());
    let mut filename_header_mismatches = 0u64;
    let mut subagent_headers_missing = 0u64;
    for item in &projected {
        let tree_id = ase_work::source_id(&item.entry.tree_relpath);
        let row = JournalRow::from_projection(&item.projection, &item.entry.kind, &tree_id);
        counts.add(&row, item.entry.size);
        if let (Some(filename), Some(header)) =
            (&item.entry.filename_timestamp, &item.entry.header_timestamp)
        {
            if filename != header {
                filename_header_mismatches += 1;
            }
        }
        if item.entry.kind == "subagent" && !item.entry.header_ok {
            subagent_headers_missing += 1;
        }
        rows.push(row);
        entries.push(item.entry.clone());
    }
    entries.sort_by(|a, b| a.relpath.as_bytes().cmp(b.relpath.as_bytes()));
    let manifest = ase_work::manifest_digest(&entries);
    let snapshot = ase_work::CohortSnapshot {
        manifest_sha256: manifest.clone(),
        files: counts.files,
        root_files: counts.root_files,
        subagent_files: counts.subagent_files,
        bytes: counts.bytes,
        root_journals_scanned: selected.root_journals_scanned,
        root_journals_selected: selected.root_journals_selected,
        root_journals_outside_window: selected.root_journals_outside_window,
        root_journals_missing_header: selected.root_journals_missing_header,
        filename_header_mismatches,
        subagent_headers_missing,
    };

    if let Some(out) = opt(args, "--out") {
        let mut content = String::new();
        for row in &rows {
            content.push_str(
                &serde_json::to_string(row)
                    .map_err(|e| err(format!("serialize work row: {e}")))?,
            );
            content.push('\n');
        }
        fs::write(&out, content).map_err(|e| err(format!("write work rows {out}: {e}")))?;
    }

    let window_value = window.map(|w| {
        json!({
            "start": ase_work::format_iso_ms(w.start_ms),
            "end": ase_work::format_iso_ms(w.end_ms),
            "end_exclusive": true
        })
    });
    let value = json!({
        "command": "work project",
        "status": "ok",
        "adapter": "omp-stock",
        "rule": ase_work::WORK_RULE,
        "rule_version": ase_work::WORK_VERSION,
        "compatible_reference_rule": ase_work::COMPATIBLE_REFERENCE_RULE,
        "generation": generation,
        "window": window_value,
        "selection_rule": "source tree = root journal <project>/<name>.jsonl plus every *.jsonl under <project>/<name>/; selected when root session timestamp is in [start, end) UTC; all files of a selected tree are read",
        "task_dependency": "none",
        "resolver_membership": "not_read",
        "limited": limit.is_some(),
        "snapshot": snapshot,
        "counts": counts,
        "journals_written": rows.len(),
        "journals": if has(args, "--json") {
            Value::Array(rows.iter().map(|r| serde_json::to_value(r).unwrap()).collect())
        } else {
            Value::Array(Vec::new())
        }
    });
    emit(&value, args);
    Ok(())
}

fn cmd_work_session(args: &[String]) -> Result<()> {
    let (idx, store, generation) = load_work(args)?;
    let omp_root = opt(args, "--omp-root").unwrap_or_else(|| config::omp_root());
    let file_id = if let Some(session) = opt(args, "--session") {
        let session_idx = idx
            .session_by_id(&session)
            .ok_or_else(|| err(format!("session not found in address index: {session}")))?;
        idx.session(session_idx)?.file()
    } else if let Some(file) = opt(args, "--file").and_then(|s| s.parse::<u32>().ok()) {
        file
    } else {
        return Err(err("work session needs --session ID or --file N"));
    };
    let view = idx.file(file_id)?;
    let rel = ase_work::relpath(view.path(), &omp_root)
        .unwrap_or_else(|| ase_work::util::slash_path(view.path()));
    let bytes = store.bytes(file_id)?;
    let sha = ase_work::sha256_hex(&bytes);
    let sid = ase_work::source_id(&rel);
    let leaf = opt(args, "--leaf");
    let options = ase_work::ProjectOptions {
        file_id,
        path: view.path(),
        source_id: &sid,
        generation: &generation,
        sha256: &sha,
        event_base: view.event_start(),
        explicit_leaf: leaf.as_deref(),
        window: None,
    };
    let projection = ase_work::project_bytes(&options, &bytes)?;
    validate_work_projection(&idx, &projection)?;
    if let Some(out) = opt(args, "--out") {
        let content = serde_json::to_vec_pretty(&projection)
            .map_err(|e| err(format!("serialize projection: {e}")))?;
        fs::write(&out, content).map_err(|e| err(format!("write projection {out}: {e}")))?;
    }
    let value = json!({
        "command": "work session",
        "status": "ok",
        "generation": generation,
        "task_dependency": "none",
        "resolver_membership": "not_read",
        "projection": projection
    });
    emit(&value, args);
    Ok(())
}

fn cmd_work_reopen(args: &[String]) -> Result<()> {
    let path = opt(args, "--projection")
        .or_else(|| positional(args))
        .ok_or_else(|| err("work reopen needs PROJECTION.json"))?;
    let bytes = fs::read(&path).map_err(|e| err(format!("read projection {path}: {e}")))?;
    let projection: JournalProjection = serde_json::from_slice(&bytes)
        .map_err(|e| err(format!("parse projection {path}: {e}")))?;
    let (idx, store, generation) = load_work(args)?;
    let view = idx.file(projection.file_id)?;
    let mut report = if view.path() == projection.path {
        let raw = store.bytes(projection.file_id)?;
        ase_work::reopen_bytes(&projection, &raw, &generation)
    } else {
        let mut report = ase_work::ReopenReport {
            status: "stale".to_string(),
            generation,
            file_id: projection.file_id,
            source_id: projection.source_id.clone(),
            ..ase_work::ReopenReport::default()
        };
        report
            .errors
            .push("file_mapping_mismatch:index maps this file id to another path".to_string());
        report
    };
    if report.status == "verified" {
        report.errors.clear();
    }
    let value = serde_json::to_value(&report)
        .map_err(|e| err(format!("render reopen report: {e}")))?;
    emit(&value, args);
    match report.status.as_str() {
        "verified" => Ok(()),
        "generation_mismatch" => std::process::exit(4),
        _ => std::process::exit(3),
    }
}

fn read_jsonl<T: serde::de::DeserializeOwned>(path: &str) -> Result<Vec<T>> {
    let bytes = fs::read(path).map_err(|e| err(format!("read {path}: {e}")))?;
    let text = String::from_utf8(bytes).map_err(|e| err(format!("read {path}: {e}")))?;
    let mut out = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        out.push(
            serde_json::from_str::<T>(line)
                .map_err(|e| err(format!("parse {path}:{}: {e}", line_no + 1)))?,
        );
    }
    Ok(out)
}

fn cmd_work_compare(args: &[String]) -> Result<()> {
    let projection = opt(args, "--projection")
        .ok_or_else(|| err("work compare needs --projection ROWS.jsonl"))?;
    let reference = opt(args, "--reference")
        .ok_or_else(|| err("work compare needs --reference SOURCES.jsonl"))?;
    let product: Vec<JournalRow> = read_jsonl(&projection)?;
    let expected: Vec<ReferenceRow> = read_jsonl(&reference)?;
    let report = ase_work::compare(&product, &expected)
        .map_err(|e| err(format!("compare: {e}")))?;
    if let Some(out) = opt(args, "--out") {
        let content = serde_json::to_vec_pretty(&report)
            .map_err(|e| err(format!("serialize comparison: {e}")))?;
        fs::write(&out, content).map_err(|e| err(format!("write comparison {out}: {e}")))?;
    }
    let value = serde_json::to_value(&report)
        .map_err(|e| err(format!("render comparison: {e}")))?;
    emit(&value, args);
    if !report.strict_relative_error_below_1_percent || !report.cohort_match {
        std::process::exit(3);
    }
    Ok(())
}

// ----------------------------------------------- explicit attempt delta

const DELTA_PARENT_SESSION: &str = "delta-parent";
const D_DELTA_BEFORE: &str = "ab000001-0000-4000-8000-000000000001";
const D_DELTA_AFTER: &str = "ab000002-0000-4000-8000-000000000002";
const D_DELTA_LATEST: &str = "ab000003-0000-4000-8000-000000000003";
const D_DELTA_CROSS: &str = "ab000004-0000-4000-8000-000000000004";
const D_DELTA_OTHER: &str = "ab000005-0000-4000-8000-000000000005";

fn delta_ts(day: u32, second: u32) -> String {
    format!("2026-10-{day:02}T00:00:{second:02}.000Z")
}

fn delta_session_line(id: &str, ts: &str) -> String {
    format!(
        "{}\n",
        json!({
            "type": "session", "version": 3, "id": id, "timestamp": ts,
            "cwd": "C:\\SYNTHETIC\\delta", "title": "synthetic attempt-delta fixture"
        })
    )
}

fn delta_parent_line(
    dispatch_id: &str,
    project: &str,
    task: &str,
    attempt: &str,
    spawn_index: u32,
    ts: &str,
) -> String {
    format!(
        "{}\n",
        json!({
            "type": "custom", "timestamp": ts, "customType": "task_dispatch_v1",
            "data": {
                "dispatch_id": dispatch_id,
                "parent_session_id": DELTA_PARENT_SESSION,
                "parent_tool_call_id": format!("call-{spawn_index}"),
                "spawn_index": spawn_index,
                "project_id": project,
                "task_id": task,
                "attempt_id": attempt
            }
        })
    )
}

fn delta_child_link_line(dispatch_id: &str, session_id: &str, ts: &str) -> String {
    format!(
        "{}\n",
        json!({
            "type": "custom", "timestamp": ts, "customType": "task_dispatch_child_v1",
            "data": { "dispatch_id": dispatch_id, "session_id": session_id }
        })
    )
}

fn delta_message_line(
    id: &str,
    parent: Option<&str>,
    ts: &str,
    role: &str,
    usage: bool,
    tool_calls: u32,
) -> String {
    let content: Vec<Value> = if tool_calls > 0 {
        (1..=tool_calls)
            .map(|index| {
                json!({
                    "type": "toolCall", "id": format!("c{index}"),
                    "name": "read", "arguments": {}
                })
            })
            .collect()
    } else {
        vec![json!({"type": "text", "text": "synthetic delta fixture entry"})]
    };
    let mut message = json!({"role": role, "content": content});
    if usage {
        message["usage"] = json!({"totalTokens": 1});
        message["stopReason"] = json!("stop");
    }
    let mut record = json!({"type": "message", "id": id, "timestamp": ts, "message": message});
    if let Some(parent) = parent {
        record["parentId"] = json!(parent);
    }
    format!("{record}\n")
}

fn delta_tool_result_line(id: &str, parent: &str, ts: &str, call_id: &str) -> String {
    format!(
        "{}\n",
        json!({
            "type": "message", "id": id, "parentId": parent, "timestamp": ts,
            "message": {
                "role": "toolResult", "toolCallId": call_id, "toolName": "read",
                "content": [{"type": "text", "text": "synthetic delta fixture result"}]
            }
        })
    )
}

fn delta_custom_user_line(id: &str, parent: &str, ts: &str) -> String {
    format!(
        "{}\n",
        json!({
            "type": "custom_message", "customType": "synthetic-user-note",
            "attribution": "user", "id": id, "parentId": parent, "timestamp": ts,
            "content": "synthetic wide boundary"
        })
    )
}

fn delta_prose_candidate_line(id: &str, parent: Option<&str>, ts: &str) -> String {
    let mut record = json!({
        "type": "message", "id": id, "timestamp": ts,
        "message": {
            "role": "user",
            "content": [{"type": "text", "text": "synthetic mention task_id=T701 without dispatch"}]
        }
    });
    if let Some(parent) = parent {
        record["parentId"] = json!(parent);
    }
    format!("{record}\n")
}

/// Synthetic, producer-shaped OMP journals for a deterministic delta check.
/// Two same-task attempts have visibly different turns and episode
/// boundaries; a later attempt carries a smaller label to prove labels are
/// not chronology. A cross-project same-token attempt, another-task attempt
/// and an unconfirmed prose-only session exercise the rejections.
fn cmd_fixture_delta(args: &[String]) -> Result<()> {
    let out = PathBuf::from(
        opt(args, "--out").ok_or_else(|| err("fixture-delta needs --out DIR"))?,
    );
    let omp = out.join("omp");
    let codex = out.join("codex");
    let dir = omp.join("delta");
    fs::create_dir_all(&dir).map_err(|e| err(format!("create {}: {e}", dir.display())))?;
    fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;
    let write = |name: &str, content: String| -> Result<()> {
        let path = dir.join(name);
        fs::write(&path, content).map_err(|e| err(format!("write {}: {e}", path.display())))
    };

    let mut parent = delta_session_line(DELTA_PARENT_SESSION, &delta_ts(1, 0));
    parent.push_str(&delta_parent_line(D_DELTA_BEFORE, "Proj31", "T701", "1", 0, &delta_ts(1, 0)));
    parent.push_str(&delta_parent_line(D_DELTA_AFTER, "Proj31", "T701", "2", 1, &delta_ts(2, 0)));
    parent.push_str(&delta_parent_line(D_DELTA_LATEST, "Proj31", "T701", "0", 2, &delta_ts(3, 0)));
    parent.push_str(&delta_parent_line(D_DELTA_CROSS, "Proj32", "T701", "1", 3, &delta_ts(1, 0)));
    parent.push_str(&delta_parent_line(D_DELTA_OTHER, "Proj31", "T702", "1", 4, &delta_ts(1, 0)));
    parent.push_str(&delta_message_line("p-u1", None, &delta_ts(1, 1), "user", false, 0));
    write("parent.jsonl", parent)?;

    let mut before = delta_session_line("delta-before", &delta_ts(1, 0));
    before.push_str(&delta_child_link_line(D_DELTA_BEFORE, "delta-before", &delta_ts(1, 1)));
    before.push_str(&delta_message_line("u1", None, &delta_ts(1, 2), "user", false, 0));
    before.push_str(&delta_message_line("a1", Some("u1"), &delta_ts(1, 3), "assistant", true, 0));
    write("child-before.jsonl", before)?;

    let mut after = delta_session_line("delta-after", &delta_ts(2, 0));
    after.push_str(&delta_child_link_line(D_DELTA_AFTER, "delta-after", &delta_ts(2, 1)));
    after.push_str(&delta_message_line("u1", None, &delta_ts(2, 2), "user", false, 0));
    after.push_str(&delta_message_line("a1", Some("u1"), &delta_ts(2, 3), "assistant", true, 2));
    after.push_str(&format!(
        "{}\n",
        json!({
            "type": "custom", "customType": "tool_execution_start", "id": "x1",
            "parentId": "a1", "timestamp": delta_ts(2, 4), "data": {}
        })
    ));
    after.push_str(&delta_tool_result_line("r1", "x1", &delta_ts(2, 5), "c1"));
    after.push_str(&delta_message_line("u2", Some("r1"), &delta_ts(2, 6), "user", false, 0));
    after.push_str(&delta_custom_user_line("cm1", "u2", &delta_ts(2, 7)));
    after.push_str(&delta_message_line("a2", Some("cm1"), &delta_ts(2, 8), "assistant", true, 0));
    write("child-after.jsonl", after)?;

    let mut latest = delta_session_line("delta-latest", &delta_ts(3, 0));
    latest.push_str(&delta_child_link_line(D_DELTA_LATEST, "delta-latest", &delta_ts(3, 1)));
    latest.push_str(&delta_message_line("u1", None, &delta_ts(3, 2), "user", false, 0));
    latest.push_str(&delta_message_line("b1", Some("u1"), &delta_ts(3, 3), "assistant", true, 0));
    latest.push_str(&delta_message_line("u2", Some("b1"), &delta_ts(3, 4), "user", false, 0));
    latest.push_str(&delta_message_line("b2", Some("u2"), &delta_ts(3, 5), "assistant", true, 0));
    write("child-latest.jsonl", latest)?;

    let mut cross = delta_session_line("delta-cross", &delta_ts(1, 0));
    cross.push_str(&delta_child_link_line(D_DELTA_CROSS, "delta-cross", &delta_ts(1, 1)));
    cross.push_str(&delta_message_line("u1", None, &delta_ts(1, 2), "user", false, 0));
    cross.push_str(&delta_message_line("a1", Some("u1"), &delta_ts(1, 3), "assistant", true, 0));
    write("child-cross.jsonl", cross)?;

    let mut other = delta_session_line("delta-other-task", &delta_ts(1, 0));
    other.push_str(&delta_child_link_line(D_DELTA_OTHER, "delta-other-task", &delta_ts(1, 1)));
    other.push_str(&delta_message_line("u1", None, &delta_ts(1, 2), "user", false, 0));
    other.push_str(&delta_message_line("a1", Some("u1"), &delta_ts(1, 3), "assistant", true, 0));
    write("child-other-task.jsonl", other)?;

    let mut unconfirmed = delta_session_line("delta-unconfirmed", &delta_ts(1, 0));
    unconfirmed.push_str(&delta_prose_candidate_line("u1", None, &delta_ts(1, 2)));
    unconfirmed.push_str(&delta_message_line("a1", Some("u1"), &delta_ts(1, 3), "assistant", true, 0));
    write("child-unconfirmed.jsonl", unconfirmed)?;

    println!(
        "wrote 7 synthetic attempt-delta OMP fixtures to {}",
        out.display()
    );
    Ok(())
}

/// Raw dispatch records for one submitted session, always labelled as claims
/// rather than membership. Used only for rejection diagnostics.
fn delta_observed_claims(input: &ase_task::DispatchInput, session_id: &str) -> Vec<Value> {
    input
        .children
        .iter()
        .filter(|link| link.session_id == session_id && link.containing_session_id == session_id)
        .map(|link| {
            let parent = input.parents.iter().find(|claim| {
                claim.dispatch_id == link.dispatch_id
                    && claim.containing_session_id == claim.parent_session_id
            });
            json!({
                "dispatch_id": &link.dispatch_id,
                "child_session_id": &link.session_id,
                "child_span": &link.span,
                "child_event": link.event,
                "parent_session_id": parent.map(|p| p.parent_session_id.clone()),
                "project": parent.map(|p| p.project.clone()),
                "task": parent.map(|p| p.task.clone()),
                "attempt": parent.map(|p| p.attempt.clone()),
                "parent_span": parent.map(|p| p.span.clone()),
                "label": "observed_dispatch_claim_not_membership"
            })
        })
        .collect()
}

fn work_delta_reject(
    args: &[String],
    task: &str,
    project: Option<&str>,
    before: &str,
    after: &str,
    reason: &str,
    detail: &str,
    resolution: Option<&ase_task::Resolution>,
    claims: Value,
) -> ! {
    let admitted: Vec<Value> = resolution
        .map(|found| {
            found
                .evidence
                .iter()
                .filter_map(|evidence| match evidence {
                    TaskEvidence::Dispatch(join) => Some(json!({
                        "dispatch_id": join.dispatch_id,
                        "project": join.project,
                        "task": join.task,
                        "attempt": join.attempt,
                        "child_session_id": join.child_session_id,
                        "parent_span": join.parent_span,
                        "child_span": join.child_span
                    })),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let candidates: Vec<Value> = resolution
        .map(|found| {
            found
                .candidates
                .iter()
                .map(|candidate| {
                    json!({
                        "project": candidate.project,
                        "task": candidate.task,
                        "task_key": candidate.task_key,
                        "state": candidate.state.label(),
                        "reason": candidate.reason
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let value = json!({
        "command": "work delta",
        "status": "rejected",
        "reason": reason,
        "detail": detail,
        "task": task,
        "project": project,
        "before": before,
        "after": after,
        "rule": ase_work::DELTA_RULE,
        "rule_version": ase_work::DELTA_VERSION,
        "membership_rule": ase_task::DISPATCH_RULE_NAME,
        "membership_rule_version": ase_task::DISPATCH_RULE_VERSION,
        "dispatch": resolution.map(|found| json!({
            "status": found.status.label(),
            "task_key": found.task_key,
            "summary": found.summary,
            "parents_indexed": found.dispatch.parents,
            "child_links_indexed": found.dispatch.children,
            "unique_joins": found.dispatch.joined,
            "rejected_groups": found.dispatch.rejected_groups,
            "record_issues": found.dispatch.issues_total
        })).unwrap_or(Value::Null),
        "admitted_attempts": admitted,
        "candidates": candidates,
        "observed_claims": claims,
        "guarantees": [
            "Both sides must be executing child sessions of admitted dispatch-v1 joins for the same project/task key; no naming, marker, adapter or motif candidate can confirm membership.",
            "A bare token with two project scopes is ambiguous and rejected; a session whose raw claim names another task/project is rejected as a task mismatch.",
            "Attempt order uses dispatch parent-record time with deterministic tie-breaks; attempt labels are displayed but never used as chronology.",
            "No delta is produced for a rejected pair, and no source, index, resolver or membership store is written."
        ]
    });
    emit(&value, args);
    std::process::exit(3);
}

fn project_delta_attempt(
    idx: &Index,
    store: &FsRawStore,
    generation: &str,
    join: &ase_task::DispatchJoin,
    omp_root: &str,
    out_path: Option<&str>,
) -> Result<(JournalProjection, ase_work::DeltaAttemptRef, ase_work::ReopenReport)> {
    let file_id = join.child_file_id;
    let view = idx.file(file_id)?;
    let bytes = store.bytes(file_id)?;
    let sha = ase_work::sha256_hex(&bytes);
    let rel = ase_work::relpath(view.path(), omp_root)
        .unwrap_or_else(|| ase_work::util::slash_path(view.path()));
    let sid = ase_work::source_id(&rel);
    let options = ase_work::ProjectOptions {
        file_id,
        path: view.path(),
        source_id: &sid,
        generation,
        sha256: &sha,
        event_base: view.event_start(),
        explicit_leaf: None,
        window: None,
    };
    let projection = ase_work::project_bytes(&options, &bytes)?;
    validate_work_projection(idx, &projection)?;
    let reopen = ase_work::reopen_bytes(&projection, &bytes, generation);
    let membership = ase_work::DeltaAttemptRef {
        session_id: join.child_session_id.clone(),
        dispatch_id: join.dispatch_id.clone(),
        project: join.project.clone(),
        task: join.task.clone(),
        attempt: join.attempt.clone(),
        parent_session_id: join.parent_session_id.clone(),
        parent_span: join.parent_span.clone(),
        child_span: join.child_span.clone(),
    };
    if let Some(path) = out_path {
        let content = serde_json::to_vec_pretty(&projection)
            .map_err(|e| err(format!("serialize attempt projection: {e}")))?;
        fs::write(path, content).map_err(|e| err(format!("write projection {path}: {e}")))?;
    }
    Ok((projection, membership, reopen))
}

fn cmd_work_delta(args: &[String]) -> Result<()> {
    let task = opt(args, "--task")
        .ok_or_else(|| err("work delta needs --task TOKEN"))?;
    let project = opt(args, "--project");
    let before_session = opt(args, "--before")
        .ok_or_else(|| err("work delta needs --before CHILD_SESSION_ID"))?;
    let after_session = opt(args, "--after")
        .ok_or_else(|| err("work delta needs --after CHILD_SESSION_ID"))?;
    if before_session == after_session {
        work_delta_reject(
            args, &task, project.as_deref(), &before_session, &after_session,
            "same_attempt", "before and after name the same executing child session",
            None, json!({}),
        );
    }
    let (idx, store, generation) = load_work(args)?;
    let dispatch = dispatch_input(&idx, &store)?;
    let resolution = ase_task::resolve(&ase_task::Input {
        token: task.clone(),
        project: project.clone(),
        naming: Vec::new(),
        markers: Vec::new(),
        delegations: Vec::new(),
        paths: Vec::new(),
        dispatch: dispatch.clone(),
        marker_layer: ase_task::MarkerLayer {
            generation: None,
            unavailable: Some(
                "attempt delta membership is derived from native dispatch records only".to_string(),
            ),
            blocks_total: 0,
            blocks_scanned: 0,
            excluded_role_blocks: 0,
            unverified_blocks: 0,
        },
    });
    if resolution.status != TaskStatus::Assigned {
        let reason = if resolution.status == TaskStatus::Ambiguous {
            "ambiguous_membership"
        } else {
            "unconfirmed_membership"
        };
        let detail = if reason == "ambiguous_membership" {
            format!("task token {task} has more than one project-scoped dispatch key; pass --project to select one")
        } else {
            format!("task token {task} has no unique admitted dispatch membership")
        };
        work_delta_reject(
            args, &task, project.as_deref(), &before_session, &after_session,
            reason, &detail, Some(&resolution), json!({}),
        );
    }

    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let joins: Vec<&ase_task::DispatchJoin> = resolution
        .evidence
        .iter()
        .filter_map(|evidence| match evidence {
            TaskEvidence::Dispatch(join)
                if join.task == task
                    && project.as_deref().is_none_or(|scope| scope == join.project) =>
            {
                let key = (join.dispatch_id.clone(), join.child_session_id.clone());
                if seen.insert(key) { Some(join) } else { None }
            }
            _ => None,
        })
        .collect();
    let before_join = joins.iter().find(|join| join.child_session_id == before_session).copied();
    let after_join = joins.iter().find(|join| join.child_session_id == after_session).copied();
    if before_join.is_none() || after_join.is_none() {
        let empty: Vec<Value> = Vec::new();
        let claims = json!({
            "before": delta_observed_claims(&dispatch, &before_session),
            "after": delta_observed_claims(&dispatch, &after_session)
        });
        let mismatch = claims
            .get("before")
            .and_then(Value::as_array)
            .unwrap_or(&empty)
            .iter()
            .chain(
                claims
                    .get("after")
                    .and_then(Value::as_array)
                    .unwrap_or(&empty)
                    .iter(),
            )
            .any(|claim| {
                claim["task"].as_str() != Some(task.as_str())
                    || project
                        .as_deref()
                        .is_some_and(|scope| claim["project"].as_str() != Some(scope))
            });
        let (reason, detail) = if mismatch {
            (
                "task_mismatch",
                "a submitted session has an observed dispatch claim for another task or project; it is not membership for this task key",
            )
        } else {
            (
                "unconfirmed_membership",
                "a submitted session has no admitted dispatch join for this project/task key; candidates and prose mentions are not membership",
            )
        };
        work_delta_reject(
            args, &task, project.as_deref(), &before_session, &after_session,
            reason, detail, Some(&resolution), claims,
        );
    }
    let before_join = before_join.expect("checked above");
    let after_join = after_join.expect("checked above");

    let mut keys = Vec::with_capacity(joins.len());
    for join in &joins {
        let ts_ms = idx.event(join.parent_event)?.ts_ms();
        keys.push(ase_work::AttemptOrderKey {
            session_id: join.child_session_id.clone(),
            dispatch_id: join.dispatch_id.clone(),
            attempt: join.attempt.clone(),
            dispatch_timestamp: Some(ase_work::format_iso_ms(ts_ms)),
            dispatch_ts_ms: ts_ms,
            spawn_index: join.spawn_index,
        });
    }
    let order = ase_work::order_attempts(&keys, &before_session, &after_session)?;
    let omp_root = opt(args, "--omp-root").unwrap_or_else(|| config::omp_root());
    let before_out = opt(args, "--before-out");
    let after_out = opt(args, "--after-out");
    let (before_projection, before_membership, before_reopen) = project_delta_attempt(
        &idx, &store, &generation, before_join, &omp_root, before_out.as_deref(),
    )?;
    let (after_projection, after_membership, after_reopen) = project_delta_attempt(
        &idx, &store, &generation, after_join, &omp_root, after_out.as_deref(),
    )?;
    let claims = json!({
        "before": delta_observed_claims(&dispatch, &before_session),
        "after": delta_observed_claims(&dispatch, &after_session)
    });
    if before_reopen.status != "verified" || after_reopen.status != "verified" {
        work_delta_reject(
            args, &task, project.as_deref(), &before_session, &after_session,
            "evidence_unverified", "an attempt projection did not reopen against the pinned index and raw bytes",
            Some(&resolution), claims,
        );
    }
    let delta = ase_work::attempt_delta(
        &before_projection,
        &after_projection,
        before_membership,
        after_membership,
        order,
    );
    let index_label = index_path(args).to_string_lossy().to_string();
    let mut value = serde_json::to_value(&delta)
        .map_err(|e| err(format!("render attempt delta: {e}")))?;
    let report = value
        .as_object_mut()
        .ok_or_else(|| err("attempt delta did not render as an object"))?;
    report.insert("command".to_string(), json!("work delta"));
    report.insert("status".to_string(), json!("ok"));
    report.insert("omp_root".to_string(), json!(omp_root));
    report.insert("observed_claims".to_string(), claims);
    report.insert(
        "evidence_reopen".to_string(),
        json!({
            "before": before_reopen,
            "after": after_reopen,
            "projection_files": { "before": before_out, "after": after_out }
        }),
    );
    report.insert(
        "reopen_commands".to_string(),
        json!({
            "before": before_out.as_ref().map(|p| format!("ase work reopen {p} --index {index_label}")),
            "after": after_out.as_ref().map(|p| format!("ase work reopen {p} --index {index_label}"))
        }),
    );
    report.insert(
        "guarantees".to_string(),
        json!([
            "Membership comes only from admitted task-dispatch-provenance / dispatch-v1 joins; adapter claims, motif membership, naming, markers, path hits and parentSession edges are never membership inputs.",
            "Both sides are executing child sessions of one confirmed project/task key; a cross-project same-token attempt, an unrelated task and an unconfirmed candidate are rejected before any delta is computed.",
            "Ordering is dispatch parent-record time with spawn_index/dispatch_id/session tie-breaks; attempt labels are labels, not chronology, and label agreement is reported explicitly.",
            "Counts are after-minus-before; turns and boundaries are keyed by entry id plus pinned evidence and sorted by change kind, ordinal and id.",
            "Both projections reopen against the same index generation and raw bytes; any mismatch rejects the delta rather than silently repairing it."
        ]),
    );
    if let Some(out) = opt(args, "--out") {
        let content = serde_json::to_vec_pretty(&value)
            .map_err(|e| err(format!("serialize attempt delta: {e}")))?;
        fs::write(&out, content).map_err(|e| err(format!("write attempt delta {out}: {e}")))?;
    }
    emit(&value, args);
    Ok(())
}

// -------------------------------------------------- synthetic stock fixture

fn write_jsonl(path: &Path, lines: &[Value]) -> Result<()> {
    let mut content = String::new();
    for line in lines {
        content.push_str(&serde_json::to_string(line).map_err(|e| err(format!("serialize fixture: {e}")))?);
        content.push('\n');
    }
    fs::write(path, content).map_err(|e| err(format!("write {}: {e}", path.display())))
}

fn fixture_task_call(call_id: &str, names: &[&str], prompt: &str) -> Value {
    let tasks: Vec<Value> = names
        .iter()
        .map(|n| json!({"name": n, "agent": "gold-task", "task": prompt}))
        .collect();
    json!({
        "type": "message",
        "id": format!("call-{call_id}"),
        "parentId": null,
        "timestamp": "2026-01-01T00:00:01.000Z",
        "message": {
            "role": "assistant",
            "content": [{"type": "toolCall", "id": call_id, "name": "task",
                         "arguments": {"i": "synthetic dispatch", "tasks": tasks}}]
        }
    })
}

fn fixture_task_result(call_id: &str, text: &str) -> Value {
    json!({
        "type": "message",
        "id": format!("result-{call_id}"),
        "parentId": null,
        "timestamp": "2026-01-01T00:00:02.000Z",
        "message": {"role": "toolResult", "toolCallId": call_id, "toolName": "task",
                    "content": [{"type": "text", "text": text}]}
    })
}

fn fixture_session(id: &str, cwd: &str, parent: Option<&str>) -> Value {
    let mut v = json!({
        "type": "session",
        "version": 3,
        "id": id,
        "timestamp": "2026-01-01T00:00:00.000Z",
        "cwd": cwd
    });
    if let Some(p) = parent {
        v["parentSession"] = json!(p);
    }
    v
}

fn cmd_fixture_adapter(args: &[String]) -> Result<()> {
    let out = PathBuf::from(
        opt(args, "--out").ok_or_else(|| err("fixture-adapter needs --out DIR"))?,
    );
    let project = out.join("omp").join("synthetic");
    let side = project.join("2026-01-01T00-00-00-000Z_syn");
    fs::create_dir_all(&side).map_err(|e| err(format!("create {}: {e}", side.display())))?;
    let parent = project.join("2026-01-01T00-00-00-000Z_syn.jsonl");
    let parent_abs = std::path::absolute(&parent)
        .unwrap_or_else(|_| parent.clone())
        .to_string_lossy()
        .into_owned();
    let other_parent = std::path::absolute(project.join("2026-01-01T00-00-00-000Z_other.jsonl"))
        .unwrap_or_else(|_| parent.clone())
        .to_string_lossy()
        .into_owned();

    let child = |name: &str, parent_ref: &str| -> Vec<Value> {
        vec![
            fixture_session("syn-root", "C:\\SYN\\demo", Some(parent_ref)),
            json!({"type":"message","id":format!("{name}-u"),"parentId":null,
                   "message":{"role":"user","content":[{"type":"text","text":"SYNTHETIC assignment"}]}}),
        ]
    };

    let parent_lines = vec![
        fixture_session("syn-root", "C:\\SYN\\demo", None),
        fixture_task_call("call-a", &["T1A1"], "Target: W1a attempt A1"),
        fixture_task_result("call-a", "Spawned agent `T1A1` (job `T1A1`)."),
        fixture_task_call("call-b", &["T2A1", "T2B1"], "Target: W2a attempt A1"),
        fixture_task_result(
            "call-b",
            "Spawned 2 background agents using gold-task.\n- `T2A1` (job `T2A1`)\n- `T2B1` (job `T2B1`)",
        ),
        fixture_task_call("call-c", &["T9A1"], "Target: W9a attempt A1"),
        fixture_task_result("call-c", "Task T9A1 failed preflight: synthetic error"),
        fixture_task_call("call-d", &["T1A1"], "Target: W1a attempt A2"),
        fixture_task_result("call-d", "Spawned agent `T4A1` (job `T4A1`)."),
        fixture_task_call("call-e", &["T5A1"], "Target: W5a attempt A1"),
        fixture_task_result("call-e", "Spawned agent `T5A1` (job `T5A1`)."),
    ];
    write_jsonl(&parent, &parent_lines)?;

    write_jsonl(&side.join("T1A1.jsonl"), &child("T1A1", &parent_abs))?;
    write_jsonl(&side.join("T2A1.jsonl"), &child("T2A1", &parent_abs))?;
    write_jsonl(&side.join("T2B1.jsonl"), &child("T2B1", &parent_abs))?;
    write_jsonl(&side.join("T4A1.jsonl"), &child("T4A1", &parent_abs))?;
    // Link conflict: the result names this child but its header names another
    // parent journal. Keep both facts; do not tie-break.
    write_jsonl(&side.join("T5A1.jsonl"), &child("T5A1", &other_parent))?;
    fs::write(
        side.join("T1A1.md"),
        "Synthetic handoff\n- task_id: W1a\n- attempt_id: A1\n",
    )
    .map_err(|e| err(format!("write synthetic handoff: {e}")))?;
    fs::write(
        side.join("T2A1.md"),
        "Synthetic handoff\n- task_id: W2a\n- attempt_id: A1\n",
    )
    .map_err(|e| err(format!("write synthetic handoff: {e}")))?;
    println!("wrote synthetic stock adapter fixture to {}", out.display());
    Ok(())
}
// -------------------------------------------------------- runtime analytics

fn analytics_usage() {
    println!(
        "ase analytics project [--index FILE] [--omp-root DIR] [--start ISO --end ISO]\n\
         [--out ROWS.jsonl] [--report REPORT.json] [--limit N] [--json]\n\
         ase analytics session (--session ID | --file N) [--leaf ENTRY_ID]\n\
         [--omp-root DIR] [--out ROW.json] [--json]\n\
         ase analytics reopen ROWS.jsonl [--index FILE] [--json]\n\
         ase fixture-analytics --out DIR"
    );
}

fn cmd_analytics(args: &[String]) -> Result<()> {
    let Some(sub) = args.first().map(String::as_str) else {
        analytics_usage();
        return Err(err("analytics needs project, session or reopen"));
    };
    let rest = &args[1..];
    if matches!(sub, "--help" | "-h") {
        analytics_usage();
        return Ok(());
    }
    match sub {
        "project" => cmd_analytics_project(rest),
        "session" => cmd_analytics_session(rest),
        "reopen" => cmd_analytics_reopen(rest),
        other => Err(err(format!("unknown analytics command: {other}"))),
    }
}

/// Project provenance is the first path segment of the locked tree layout,
/// never a model name, session title or adapter candidate. Absence stays an
/// explicit unknown group in the analytics layer.
fn analytics_project_of_tree(tree_relpath: &str) -> Option<String> {
    tree_relpath
        .split('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
}

fn cmd_analytics_project(args: &[String]) -> Result<()> {
    let (idx, store, generation) = load_work(args)?;
    let window = parse_work_window(args)?;
    let omp_root = opt(args, "--omp-root").unwrap_or_else(|| config::omp_root());
    let limit = opt(args, "--limit").and_then(|s| s.parse::<usize>().ok());
    let selected = collect_analytics_files(&idx, &omp_root, limit)?;
    let projected: Vec<ProjectedWork> = selected
        .files
        .par_iter()
        .map(|file| project_work_file(&idx, &store, file, &generation, window))
        .collect::<Result<Vec<_>>>()?;
    let rows: Vec<JournalAnalytics> = projected
        .iter()
        .zip(selected.files.iter())
        .map(|(item, file)| {
            JournalAnalytics::new(
                item.projection.clone(),
                analytics_project_of_tree(&file.layout.tree_relpath),
            )
        })
        .collect();
    let report = aggregate(&rows);
    let ok = reconciled(&report);

    if let Some(out) = opt(args, "--out") {
        let mut content = String::new();
        for row in &rows {
            content.push_str(
                &serde_json::to_string(row)
                    .map_err(|e| err(format!("serialize analytics row: {e}")))?,
            );
            content.push('\n');
        }
        fs::write(&out, content).map_err(|e| err(format!("write analytics rows {out}: {e}")))?;
    }
    if let Some(out) = opt(args, "--report") {
        let content = serde_json::to_vec_pretty(&report)
            .map_err(|e| err(format!("serialize analytics report: {e}")))?;
        fs::write(&out, content).map_err(|e| err(format!("write analytics report {out}: {e}")))?;
    }

    let mut value = serde_json::to_value(&report)
        .map_err(|e| err(format!("render analytics report: {e}")))?;
    value["command"] = json!("analytics project");
    value["status"] = json!(if ok { "ok" } else { "reconciliation_failed" });
    value["limited"] = json!(limit.is_some());
    value["selection"] = json!({
        "scanned": selected.root_journals_scanned,
        "selected": selected.root_journals_selected,
        "outside_window": selected.root_journals_outside_window,
        "missing_header": selected.root_journals_missing_header,
        "root_selection": "all-root-journals; period totals filtered on message.timestamp"
    });
    if !has(args, "--json") {
        value["buckets"] = json!([]);
    }
    emit(&value, args);
    if !ok {
        std::process::exit(3);
    }
    Ok(())
}

fn cmd_analytics_session(args: &[String]) -> Result<()> {
    let (idx, store, generation) = load_work(args)?;
    let omp_root = opt(args, "--omp-root").unwrap_or_else(|| config::omp_root());
    let file_id = if let Some(session) = opt(args, "--session") {
        let session_idx = idx
            .session_by_id(&session)
            .ok_or_else(|| err(format!("session not found in address index: {session}")))?;
        idx.session(session_idx)?.file()
    } else if let Some(file) = opt(args, "--file").and_then(|s| s.parse::<u32>().ok()) {
        file
    } else {
        return Err(err("analytics session needs --session ID or --file N"));
    };
    let view = idx.file(file_id)?;
    let rel_opt = ase_work::relpath(view.path(), &omp_root);
    let rel = rel_opt
        .clone()
        .unwrap_or_else(|| ase_work::util::slash_path(view.path()));
    let bytes = store.bytes(file_id)?;
    let sha = ase_work::sha256_hex(&bytes);
    let sid = ase_work::source_id(&rel);
    let leaf = opt(args, "--leaf");
    let options = ase_work::ProjectOptions {
        file_id,
        path: view.path(),
        source_id: &sid,
        generation: &generation,
        sha256: &sha,
        event_base: view.event_start(),
        explicit_leaf: leaf.as_deref(),
        window: None,
    };
    let projection = ase_work::project_bytes(&options, &bytes)?;
    validate_work_projection(&idx, &projection)?;
    let project = rel_opt.as_deref().and_then(analytics_project_of_tree);
    let row = JournalAnalytics::new(projection, project);
    if let Some(out) = opt(args, "--out") {
        let content = serde_json::to_vec_pretty(&row)
            .map_err(|e| err(format!("serialize analytics row: {e}")))?;
        fs::write(&out, content).map_err(|e| err(format!("write analytics row {out}: {e}")))?;
    }
    let report = aggregate(&[row.clone()]);
    let mut value = serde_json::to_value(&row)
        .map_err(|e| err(format!("render analytics row: {e}")))?;
    value["command"] = json!("analytics session");
    value["reconciliation"] = serde_json::to_value(&report.reconciliation)
        .map_err(|e| err(format!("render reconciliation: {e}")))?;
    emit(&value, args);
    Ok(())
}

fn cmd_analytics_reopen(args: &[String]) -> Result<()> {
    let path = positional(args)
        .or_else(|| opt(args, "--projection"))
        .ok_or_else(|| err("analytics reopen needs ROWS.jsonl"))?;
    let rows: Vec<JournalAnalytics> = read_jsonl(&path)?;
    let (idx, store, generation) = load_work(args)?;
    let mut checked = 0u64;
    let mut verified = 0u64;
    let mut stale = 0u64;
    let mut evidence_checked = 0u64;
    let mut evidence_verified = 0u64;
    let mut runtime_checked = 0u64;
    let mut runtime_verified = 0u64;
    let mut failures: Vec<String> = Vec::new();
    for row in &rows {
        checked += 1;
        let projection = &row.projection;
        let view = idx.file(projection.file_id)?;
        let path_ok = view.path() == projection.path;
        let raw = store.bytes(projection.file_id)?;
        let report = if path_ok {
            ase_work::reopen_bytes(projection, &raw, &generation)
        } else {
            let mut report = ase_work::ReopenReport {
                status: "stale".to_string(),
                generation: generation.clone(),
                file_id: projection.file_id,
                source_id: projection.source_id.clone(),
                ..ase_work::ReopenReport::default()
            };
            report
                .errors
                .push("file_mapping_mismatch:index maps this file id to another path".to_string());
            report
        };
        evidence_checked += report.evidence_checked as u64;
        evidence_verified += report.evidence_verified as u64;
        runtime_checked += report.runtime_checked as u64;
        runtime_verified += report.runtime_verified as u64;
        if report.status == "verified" {
            verified += 1;
        } else {
            stale += 1;
            failures.push(format!(
                "{}:{}:{}",
                row.source_id,
                report.status,
                report.errors.join(";")
            ));
        }
    }
    let value = json!({
        "command": "analytics reopen",
        "status": if stale == 0 { "verified" } else { "stale" },
        "generation": generation,
        "files_checked": checked,
        "files_verified": verified,
        "files_stale": stale,
        "evidence_checked": evidence_checked,
        "evidence_verified": evidence_verified,
        "runtime_checked": runtime_checked,
        "runtime_verified": runtime_verified,
        "failures_shown": failures.len().min(20),
        "failures": &failures[..failures.len().min(20)]
    });
    emit(&value, args);
    if stale > 0 {
        std::process::exit(3);
    }
    Ok(())
}

// Synthetic producer-shaped OMP journals for a deterministic runtime check. No
// corpus path, prose or private identifier is written; each file isolates one
// rule: parallel clocks, two projects sharing a model, active-leaf choice,
// half-open period and missing producer fields.
fn fixture_analytics_session(id: &str, ts: &str) -> String {
    format!(
        "{}\n",
        json!({
            "type": "session", "version": 3, "id": id, "timestamp": ts,
            "cwd": "C:\\SYNTHETIC\\analytics", "title": "synthetic runtime-analytics fixture"
        })
    )
}

fn fixture_analytics_user(id: &str, parent: Option<&str>, ts: &str) -> String {
    let mut record = json!({
        "type": "message", "id": id, "timestamp": ts,
        "message": {"role": "user", "content": [{"type": "text", "text": "synthetic"}]}
    });
    if let Some(parent) = parent {
        record["parentId"] = json!(parent);
    }
    format!("{record}\n")
}

#[allow(clippy::too_many_arguments)]
fn fixture_analytics_turn(
    id: &str,
    parent: &str,
    ts: &str,
    model: Option<&str>,
    msg_ts: Option<i64>,
    completed: Option<i64>,
    duration: Option<f64>,
    ttft: Option<f64>,
    cost: Option<f64>,
    usage: bool,
) -> String {
    let mut message = json!({
        "role": "assistant",
        "content": [{"type": "text", "text": "synthetic"}],
        "stopReason": "stop"
    });
    if let Some(model) = model {
        message["model"] = json!(model);
    }
    if let Some(ts) = msg_ts {
        message["timestamp"] = json!(ts);
    }
    if let Some(ts) = completed {
        message["completedAt"] = json!(ts);
    }
    if let Some(value) = duration {
        message["duration"] = json!(value);
    }
    if let Some(value) = ttft {
        message["ttft"] = json!(value);
    }
    if usage {
        let mut cost_obj = json!({"input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0});
        if let Some(value) = cost {
            cost_obj["total"] = json!(value);
        }
        message["usage"] = json!({
            "input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0,
            "totalTokens": 2, "cost": cost_obj
        });
    }
    format!(
        "{}\n",
        json!({
            "type": "message", "id": id, "parentId": parent, "timestamp": ts,
            "message": message
        })
    )
}

fn cmd_fixture_analytics(args: &[String]) -> Result<()> {
    let out = PathBuf::from(
        opt(args, "--out").ok_or_else(|| err("fixture-analytics needs --out DIR"))?,
    );
    let omp = out.join("omp");
    let codex = out.join("codex");
    fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;
    let write = |rel: &str, content: String| -> Result<()> {
        let path = omp.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| err(format!("create {}: {e}", parent.display())))?;
        }
        fs::write(&path, content).map_err(|e| err(format!("write {}: {e}", path.display())))
    };

    let mut parallel = fixture_analytics_session("analytics-parallel", "2026-10-05T00:00:00.000Z");
    parallel.push_str(&fixture_analytics_user("u1", None, "2026-10-05T00:00:01.000Z"));
    parallel.push_str(&fixture_analytics_turn("a1", "u1", "2026-10-05T00:00:02.000Z",
        Some("fixture-parallel"), Some(1_791_158_402_000), Some(1_791_158_404_000),
        Some(2000.0), Some(500.0), Some(0.01), true));
    parallel.push_str(&fixture_analytics_turn("a2", "a1", "2026-10-05T00:00:03.000Z",
        Some("fixture-parallel"), Some(1_791_158_402_500), Some(1_791_158_404_500),
        Some(2000.0), Some(700.0), Some(0.02), true));
    write("p-parallel/parallel.jsonl", parallel)?;

    let mut second = fixture_analytics_session("analytics-second", "2026-10-05T01:00:00.000Z");
    second.push_str(&fixture_analytics_user("u1", None, "2026-10-05T01:00:01.000Z"));
    second.push_str(&fixture_analytics_turn("b1", "u1", "2026-10-05T01:00:02.000Z",
        Some("fixture-parallel"), Some(1_791_162_002_000), Some(1_791_162_003_000),
        Some(1000.0), Some(200.0), Some(0.03), true));
    write("q-parallel/second.jsonl", second)?;

    let mut branch = fixture_analytics_session("analytics-branch", "2026-10-02T00:00:00.000Z");
    branch.push_str(&fixture_analytics_user("u1", None, "2026-10-02T00:00:01.000Z"));
    branch.push_str(&fixture_analytics_turn("a1", "u1", "2026-10-02T00:00:02.000Z",
        Some("fixture-branch"), Some(1_790_899_202_000), Some(1_790_899_203_000),
        Some(1000.0), Some(100.0), Some(0.01), true));
    branch.push_str(&fixture_analytics_turn("a2a", "a1", "2026-10-02T00:00:03.000Z",
        Some("fixture-branch"), Some(1_790_899_203_000), Some(1_790_899_204_000),
        Some(1000.0), Some(100.0), Some(0.10), true));
    branch.push_str(&fixture_analytics_turn("a2b", "a1", "2026-10-02T00:00:04.000Z",
        Some("fixture-branch"), Some(1_790_899_204_000), Some(1_790_899_205_000),
        Some(1000.0), Some(100.0), Some(0.20), true));
    write("p-branch/branch.jsonl", branch)?;

    let mut window = fixture_analytics_session("analytics-window", "2026-10-01T00:00:00.000Z");
    window.push_str(&fixture_analytics_user("u1", None, "2026-10-01T00:00:01.000Z"));
    window.push_str(&fixture_analytics_turn("before", "u1", "2026-09-30T23:59:59.000Z",
        Some("fixture-window"), Some(1_790_812_799_000), Some(1_790_812_799_500),
        Some(500.0), Some(50.0), Some(0.01), true));
    window.push_str(&fixture_analytics_turn("inside", "before", "2026-10-01T00:00:00.000Z",
        Some("fixture-window"), Some(1_790_812_800_000), Some(1_790_812_800_500),
        Some(500.0), Some(50.0), Some(0.02), true));
    window.push_str(&fixture_analytics_turn("at-end", "inside", "2026-11-01T00:00:00.000Z",
        Some("fixture-window"), Some(1_793_491_200_000), Some(1_793_491_200_500),
        Some(500.0), Some(50.0), Some(0.04), true));
    write("p-window/window.jsonl", window)?;

    let mut gaps = fixture_analytics_session("analytics-gaps", "2026-10-03T00:00:00.000Z");
    gaps.push_str(&fixture_analytics_user("u1", None, "2026-10-03T00:00:01.000Z"));
    gaps.push_str(&format!("{}\n", json!({
        "type": "message", "id": "n1", "parentId": "u1",
        "timestamp": "2026-10-03T00:00:02.000Z",
        "message": {"role": "assistant", "content": [{"type": "text", "text": "synthetic"}]}
    })));
    gaps.push_str(&fixture_analytics_turn("c1", "n1", "2026-10-03T00:00:03.000Z",
        Some("fixture-gaps"), Some(1_790_983_203_000), None, None, None, None, true));
    gaps.push_str(&fixture_analytics_turn("c2", "c1", "2026-10-03T00:00:04.000Z",
        None, Some(1_790_983_204_000), Some(1_790_983_205_000),
        Some(1000.0), Some(300.0), Some(0.05), true));
    write("p-gaps/gaps.jsonl", gaps)?;

    println!("wrote 5 synthetic runtime-analytics OMP fixtures to {}", out.display());
    Ok(())
}

fn judge_usage() {
    println!(
        "ase judge run --root DIR --manifest FILE --units FILE --private FILE\n\
         \x20           --store FILE [--scope primary|unknown|all] [--only-ordinal N]...\n\
         \x20           [--limit N] [--workspace DIR] [--verification-only]\n\
         \x20           [--slm-runner PROG --slm-arg ARG... --slm-model-id ID\n\
         \x20            (--slm-model-sha256 HEX | --slm-weights FILE)] [...]\n\
         \x20           [--slm-profile NAME [--slm-api-timeout-ms N]]\n\
         \x20           [--judge-runner ... --judge-model-id ID ...]\n\
         \x20           [--judge-profile NAME [--judge-api-timeout-ms N]]\n\
         \x20           [--allow-remote-session-data] [--out REPORT.json]\n\
         ase judge verify --root DIR --units FILE --private FILE --store FILE\n\
         \x20           [--include-verification]\n\
         ase judge report --store FILE [--include-verification]\n\
         ase judge show --store FILE [--unit ID] [--decision N] [--include-verification]\n\
         ase judge fixture --out DIR\n\
         ase judge contract-runner [--label split|continue|unknown] [--fail]\n\
         \x20           [--model-id ID] [--model-sha256 HEX]\n\n\
         No judge command calls a model on a read path. Model stages only run\n\
         under `judge run`: a workspace-local process (stdin/stdout) or a typed\n\
         API profile resolved by `ase llm`. A non-loopback API endpoint that\n\
         would receive session text is refused unless --allow-remote-session-data\n\
         is passed; the destination is printed. Use `ase llm smoke` for a fixed\n\
         synthetic call with no session data."
    );
}

fn opts(args: &[String], name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if args[index] == name {
            if let Some(value) = args.get(index + 1) {
                out.push(value.clone());
                index += 1;
            }
        }
        index += 1;
    }
    out
}

fn workspace_file_digest(path: &Path, workspace: &Path) -> Result<String> {
    let real_workspace = fs::canonicalize(workspace)
        .map_err(|e| err(format!("resolve workspace {}: {e}", workspace.display())))?;
    let real = fs::canonicalize(path)
        .map_err(|e| err(format!("resolve model weights {}: {e}", path.display())))?;
    if !real.starts_with(&real_workspace) {
        return Err(err(format!(
            "model weights must stay inside the workspace: {}",
            path.display()
        )));
    }
    let bytes = fs::read(&real)
        .map_err(|e| err(format!("read model weights {}: {e}", real.display())))?;
    Ok(ase_judge::sha256_hex(&bytes))
}

fn parse_runner_config(
    args: &[String],
    prefix: &str,
    workspace: &Path,
) -> Result<Option<ase_judge::RunnerConfig>> {
    let program = opt(args, &format!("--{prefix}-runner"));
    let model_id = opt(args, &format!("--{prefix}-model-id"));
    let digest = opt(args, &format!("--{prefix}-model-sha256"));
    let weights = opt(args, &format!("--{prefix}-weights"));
    let runner_args = opts(args, &format!("--{prefix}-arg"));
    let timeout = match opt(args, &format!("--{prefix}-timeout-ms")) {
        Some(text) => Some(
            text.parse::<u64>()
                .map_err(|_| err(format!("invalid --{prefix}-timeout-ms: {text}")))?,
        ),
        None => None,
    };
    let any = program.is_some()
        || model_id.is_some()
        || digest.is_some()
        || weights.is_some()
        || !runner_args.is_empty();
    if !any {
        return Ok(None);
    }
    let program = program.ok_or_else(|| err(format!("--{prefix}-runner is required")))?;
    let model_id = model_id.ok_or_else(|| err(format!("--{prefix}-model-id is required")))?;
    let model_sha256 = match (digest, weights) {
        (Some(digest), None) => {
            if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(err(format!(
                    "--{prefix}-model-sha256 must be a 64-hex digest"
                )));
            }
            digest.to_ascii_lowercase()
        }
        (None, Some(weights)) => {
            workspace_file_digest(Path::new(&weights), workspace)?
        }
        (Some(_), Some(_)) => {
            return Err(err(format!(
                "use either --{prefix}-model-sha256 or --{prefix}-weights, not both"
            )))
        }
        (None, None) => {
            return Err(err(format!(
                "--{prefix}-model-sha256 or --{prefix}-weights is required"
            )))
        }
    };
    Ok(Some(ase_judge::RunnerConfig {
        program,
        args: runner_args,
        model_id,
        model_sha256,
        timeout_ms: timeout.unwrap_or(ase_judge::runner::DEFAULT_TIMEOUT_MS),
    }))
}

fn parse_stage_model(
    args: &[String],
    prefix: &str,
    workspace: &Path,
) -> Result<Option<ase_judge::runner::StageModel>> {
    let profile_name = opt(args, &format!("--{prefix}-profile"));
    let process_any = opt(args, &format!("--{prefix}-runner")).is_some()
        || opt(args, &format!("--{prefix}-model-id")).is_some()
        || opt(args, &format!("--{prefix}-model-sha256")).is_some()
        || opt(args, &format!("--{prefix}-weights")).is_some()
        || !opts(args, &format!("--{prefix}-arg")).is_empty();
    if profile_name.is_some() && process_any {
        return Err(err(format!(
            "use either --{prefix}-profile or the process runner flags, not both"
        )));
    }
    if let Some(name) = profile_name {
        let timeout_ms = match opt(args, &format!("--{prefix}-api-timeout-ms")) {
            Some(text) => text.parse::<u64>().map_err(|_| {
                err(format!("invalid --{prefix}-api-timeout-ms: {text}"))
            })?,
            None => ase_llm::DEFAULT_TIMEOUT_MS,
        };
        let env = ase_llm::capture_env();
        let resolution = ase_llm::resolve(&env, Some(&name))
            .map_err(|e| err(format!("{prefix} API profile: {e}")))?;
        return Ok(Some(ase_judge::runner::StageModel::Api(
            ase_judge::runner::ApiStage {
                resolution,
                timeout_ms,
            },
        )));
    }
    if opt(args, &format!("--{prefix}-api-timeout-ms")).is_some() {
        return Err(err(format!(
            "--{prefix}-api-timeout-ms needs --{prefix}-profile"
        )));
    }
    Ok(parse_runner_config(args, prefix, workspace)?
        .map(ase_judge::runner::StageModel::Process))
}

fn cmd_judge(args: &[String]) -> Result<()> {
    let Some(sub) = args.first().map(String::as_str) else {
        judge_usage();
        return Err(err("judge needs run, verify, report, show, fixture or contract-runner"));
    };
    let rest = &args[1..];
    if matches!(sub, "--help" | "-h") {
        judge_usage();
        return Ok(());
    }
    match sub {
        "run" => cmd_judge_run(rest),
        "verify" => cmd_judge_verify(rest),
        "report" => cmd_judge_report(rest),
        "show" => cmd_judge_show(rest),
        "fixture" => cmd_judge_fixture(rest),
        "contract-runner" => cmd_judge_contract_runner(rest),
        other => Err(err(format!("unknown judge command: {other}"))),
    }
}

fn cmd_judge_run(args: &[String]) -> Result<()> {
    let root = PathBuf::from(opt(args, "--root").ok_or_else(|| err("--root is required"))?);
    let manifest =
        PathBuf::from(opt(args, "--manifest").ok_or_else(|| err("--manifest is required"))?);
    let units = PathBuf::from(opt(args, "--units").ok_or_else(|| err("--units is required"))?);
    let private =
        PathBuf::from(opt(args, "--private").ok_or_else(|| err("--private is required"))?);
    let store = PathBuf::from(opt(args, "--store").ok_or_else(|| err("--store is required"))?);
    let workspace = match opt(args, "--workspace") {
        Some(path) => PathBuf::from(path),
        None => std::env::current_dir()
            .map_err(|e| err(format!("resolve current directory: {e}")))?,
    };
    let scope = Scope::parse(&opt(args, "--scope").unwrap_or_else(|| "primary".to_string()))
        .ok_or_else(|| err("--scope must be primary, unknown or all"))?;
    let mut only_ordinals = std::collections::BTreeSet::new();
    for value in opts(args, "--only-ordinal") {
        only_ordinals.insert(
            value
                .parse::<u32>()
                .map_err(|_| err(format!("invalid --only-ordinal: {value}")))?,
        );
    }
    let limit = match opt(args, "--limit") {
        Some(value) => Some(
            value
                .parse::<usize>()
                .map_err(|_| err(format!("invalid --limit: {value}")))?,
        ),
        None => None,
    };
    let slm = parse_stage_model(args, "slm", &workspace)?;
    let judge = parse_stage_model(args, "judge", &workspace)?;
    let allow_remote = has(args, "--allow-remote-session-data");
    for (stage, model) in [("slm", &slm), ("judge", &judge)] {
        if let Some(ase_judge::runner::StageModel::Api(api)) = model {
            let profile = &api.resolution.profile;
            if !profile.endpoint.is_loopback() {
                if !allow_remote {
                    return Err(err(format!(
                        "session data egress is not authorized for {stage}: {} at {} (provider {}); pass --allow-remote-session-data to opt in",
                        profile.api.as_str(),
                        profile.endpoint.origin,
                        profile.provider
                    )));
                }
                eprintln!(
                    "session data egress: {stage} -> provider {} api {} at {} (synthetic or pinned evaluation input)",
                    profile.provider,
                    profile.api.as_str(),
                    profile.endpoint.origin
                );
            }
        }
    }
    let options = ase_judge::EvalOptions {
        root,
        manifest,
        units,
        private,
        store,
        scope,
        only_ordinals,
        limit,
        workspace,
        verification_only: has(args, "--verification-only"),
        allow_remote_session_data: allow_remote,
        slm,
        judge,
    };
    let report = ase_judge::run(&options)?;
    let value = serde_json::to_value(&report)
        .map_err(|e| err(format!("serialize judge run report: {e}")))?;
    if let Some(out) = opt(args, "--out") {
        fs::write(&out, serde_json::to_string_pretty(&value).unwrap() + "\n")
            .map_err(|e| err(format!("write judge report {out}: {e}")))?;
    }
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
    Ok(())
}

fn cmd_judge_verify(args: &[String]) -> Result<()> {
    let options = ase_judge::VerifyOptions {
        root: PathBuf::from(opt(args, "--root").ok_or_else(|| err("--root is required"))?),
        units: PathBuf::from(opt(args, "--units").ok_or_else(|| err("--units is required"))?),
        private: PathBuf::from(
            opt(args, "--private").ok_or_else(|| err("--private is required"))?,
        ),
        store: PathBuf::from(opt(args, "--store").ok_or_else(|| err("--store is required"))?),
        include_verification: has(args, "--include-verification"),
    };
    let report = ase_judge::verify(&options)?;
    let value = serde_json::to_value(&report)
        .map_err(|e| err(format!("serialize judge verify report: {e}")))?;
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
    if report.status != "verified" {
        std::process::exit(3);
    }
    Ok(())
}

fn cmd_judge_report(args: &[String]) -> Result<()> {
    let store = PathBuf::from(opt(args, "--store").ok_or_else(|| err("--store is required"))?);
    let value = ase_judge::report(&store, has(args, "--include-verification"))?;
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
    Ok(())
}

fn cmd_judge_show(args: &[String]) -> Result<()> {
    let store = PathBuf::from(opt(args, "--store").ok_or_else(|| err("--store is required"))?);
    let unit = opt(args, "--unit");
    let decision = match opt(args, "--decision") {
        Some(value) => Some(
            value
                .parse::<u32>()
                .map_err(|_| err(format!("invalid --decision: {value}")))?,
        ),
        None => None,
    };
    let value = ase_judge::show(
        &store,
        unit.as_deref(),
        decision,
        has(args, "--include-verification"),
    )?;
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
    Ok(())
}

fn cmd_judge_fixture(args: &[String]) -> Result<()> {
    let out = PathBuf::from(opt(args, "--out").ok_or_else(|| err("--out is required"))?);
    let value = ase_judge::write_fixture(&out)?;
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
    Ok(())
}

fn cmd_judge_contract_runner(args: &[String]) -> Result<()> {
    use std::io::Read;
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| err(format!("read contract request: {e}")))?;
    if has(args, "--fail") {
        return Err(err("contract runner configured to fail"));
    }
    let request: Value = serde_json::from_str(&input)
        .map_err(|e| err(format!("parse contract request: {e}")))?;
    if request.get("protocol").and_then(Value::as_str) != Some(ase_judge::LOCAL_MODEL_PROTOCOL) {
        return Err(err("contract runner received a foreign protocol"));
    }
    let stage = request
        .get("stage")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if stage != "slm" && stage != "judge" {
        return Err(err("contract runner received an unknown stage"));
    }
    let label = opt(args, "--label").unwrap_or_else(|| "continue".to_string());
    let decision = ase_judge::Decision::parse(&label)
        .ok_or_else(|| err(format!("invalid --label: {label}")))?;
    let model_id =
        opt(args, "--model-id").unwrap_or_else(|| "ase-contract-runner".to_string());
    let model_sha256 = opt(args, "--model-sha256").unwrap_or_else(|| {
        ase_judge::sha256_hex(b"ase-contract-runner/v1")
    });
    let response = json!({
        "protocol": ase_judge::LOCAL_MODEL_PROTOCOL,
        "label": decision.as_str(),
        "confidence": "medium",
        "abstain_reason": if decision == ase_judge::Decision::Unknown {
            Some("contract_runner_abstain")
        } else {
            None
        },
        "model_id": model_id,
        "model_sha256": model_sha256,
        "stage": stage,
        "received_input_sha256": request.get("input_sha256"),
    });
    println!("{}", serde_json::to_string(&response).unwrap());
    Ok(())
}

// ---------------------------------------------------------------------------
// `ase llm` — user-scoped API configuration and secret resolution.
//
// The key is only ever read from hidden stdin. Every mutating/status command
// validates its flags strictly and never echoes a value that could have been a
// pasted secret; the status/JSON paths are built from explicit non-secret
// fields.
// ---------------------------------------------------------------------------

fn cmd_llm(args: &[String]) -> Result<()> {
    let Some(sub) = args.first().map(String::as_str) else {
        llm_usage();
        return Ok(());
    };
    if matches!(sub, "--help" | "-h") {
        llm_usage();
        return Ok(());
    }
    let rest = &args[1..];
    match sub {
        "config" => cmd_llm_config(rest),
        "key" => cmd_llm_key(rest),
        "status" => cmd_llm_status(rest),
        "smoke" => cmd_llm_smoke(rest),
        _ => Err(err("unknown llm command (see `ase llm --help`)")),
    }
}

fn llm_usage() {
    println!(
        "ase llm config [--json] [--profile NAME]\n\
         ase llm config set --profile NAME --provider NAME\n\
         \x20           [--type chat-completions|responses|decision|gemini|gemini-decision] [--endpoint URL]\n\
         \x20           [--model MODEL] [--effort none|minimal|low|medium|high]\n\
         \x20           [--use] [--json]\n\
         ase llm config use --profile NAME [--json]\n\
         ase llm config enroll --profile NAME --provider NAME\n\
         \x20           [--type ...] [--endpoint URL] [--model MODEL] [--effort ...]\n\
         \x20           [--use] [--json]\n\
         ase llm key set [--profile NAME] [--json]   (hidden prompt; never argv)\n\
         ase llm key clear [--profile NAME] [--json]\n\
         ase llm status [--profile NAME] [--json]\n\
         ase llm smoke [--profile NAME] [--timeout-ms N] [--json]\n\
         \x20           (one fixed synthetic call; no session or journal data)\n\n\
         Non-secret settings resolve in this order: profile/config file, then\n\
         ASE_LLM_PROVIDER, OPENAI_API_TYPE, OPENAI_API_ENDPOINT,\n\
         OPENAI_API_MODEL and OPENAI_API_EFFORT. The credential is the typed\n\
         API family's process variable when nonempty (OPENAI_API_KEY or\n\
         GEMINI_API_KEY), otherwise the stored key for the exact\n\
         (provider, normalized origin) pair.\n\
         Only `ase llm smoke` sends a request. Every other `ase llm` command is\n\
         local and opens no socket."
    );
}

/// Strict option validation. Values are never printed back: an unexpected
/// token is rejected without echoing it, so `ase llm key set --key SECRET`
/// cannot place the secret in the error path.
fn llm_expect(args: &[String], value_flags: &[&str], bool_flags: &[&str]) -> Result<()> {
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let Some(flag) = arg.strip_prefix("--") else {
            return Err(err(
                "unexpected argument: the API key is only read from a hidden prompt, never from argv",
            ));
        };
        if flag.contains('=') {
            return Err(err(
                "llm options must be passed as `--flag VALUE`; `--flag=value` is not accepted",
            ));
        }
        let name = format!("--{flag}");
        if bool_flags.iter().any(|allowed| *allowed == name) {
            index += 1;
            continue;
        }
        if value_flags.iter().any(|allowed| *allowed == name) {
            let value = args.get(index + 1).ok_or_else(|| {
                err(format!("llm option {name} needs a value"))
            })?;
            if value.starts_with("--") {
                return Err(err(format!("llm option {name} needs a value")));
            }
            index += 2;
            continue;
        }
        return Err(err("unknown llm option (see `ase llm --help`)"));
    }
    Ok(())
}

fn cmd_llm_config(args: &[String]) -> Result<()> {
    let Some(sub) = args.first().map(String::as_str) else {
        return print_llm_status(None, false, false);
    };
    if matches!(sub, "--help" | "-h") {
        llm_usage();
        return Ok(());
    }
    if sub.starts_with("--") {
        llm_expect(args, &["--profile"], &["--json"])?;
        return print_llm_status(opt(args, "--profile"), has(args, "--json"), false);
    }
    let rest = &args[1..];
    match sub {
        "set" => {
            llm_expect(
                rest,
                &[
                    "--profile", "--provider", "--type", "--endpoint", "--model",
                    "--effort",
                ],
                &["--json", "--use"],
            )?;
            let env = ase_llm::capture_env();
            let mut config = ase_llm::load_config(&env)?
                .config
                .unwrap_or_else(ase_llm::ConfigFile::empty);
            let name = llm_profile_name(rest);
            let (provider, api, endpoint, model, effort) = llm_settings(rest)?;
            ase_llm::apply_edit(
                &mut config,
                &ase_llm::ProfileEdit {
                    profile: &name,
                    provider: provider.as_deref(),
                    api,
                    endpoint: endpoint.as_deref(),
                    model: model.as_deref(),
                    effort,
                    make_active: has(rest, "--use"),
                },
            )?;
            let path = ase_llm::save_config(&env, &config)?;
            if has(rest, "--json") {
                let value = json!({
                    "saved": true,
                    "profile": name,
                    "provider": config.profiles.get(&name).map(|p| p.provider.clone()),
                    "active": config.active,
                    "config_path": path.display().to_string(),
                });
                println!("{}", serde_json::to_string_pretty(&value).unwrap());
            } else {
                println!("saved profile {name} in {}", path.display());
                println!(
                    "next: run `ase llm key set --profile {name}` in a terminal (hidden input)"
                );
            }
            Ok(())
        }
        "use" => {
            llm_expect(rest, &["--profile"], &["--json"])?;
            let name = opt(rest, "--profile").ok_or_else(|| err("--profile is required"))?;
            let env = ase_llm::capture_env();
            let mut config = ase_llm::load_config(&env)?
                .config
                .ok_or_else(|| err("llm_config_missing: run `ase llm config set` first"))?;
            ase_llm::use_profile(&mut config, &name)?;
            let path = ase_llm::save_config(&env, &config)?;
            if has(rest, "--json") {
                let value = json!({
                    "active": name,
                    "config_path": path.display().to_string(),
                });
                println!("{}", serde_json::to_string_pretty(&value).unwrap());
            } else {
                println!("active profile: {name}");
                println!("credential is resolved for the endpoint origin, so a profile on the same origin reuses the stored key");
            }
            Ok(())
        }
        "enroll" => {
            llm_expect(
                rest,
                &[
                    "--profile", "--provider", "--type", "--endpoint", "--model",
                    "--effort",
                ],
                &["--json", "--use"],
            )?;
            let env = ase_llm::capture_env();
            let mut config = ase_llm::load_config(&env)?
                .config
                .unwrap_or_else(ase_llm::ConfigFile::empty);
            let name = llm_profile_name(rest);
            let (provider, api, endpoint, model, effort) = llm_settings(rest)?;
            ase_llm::apply_edit(
                &mut config,
                &ase_llm::ProfileEdit {
                    profile: &name,
                    provider: provider.as_deref(),
                    api,
                    endpoint: endpoint.as_deref(),
                    model: model.as_deref(),
                    effort,
                    make_active: has(rest, "--use"),
                },
            )?;
            let profile = ase_llm::resolve_profile(&env, Some(&name))?;
            let store = ase_llm::open_store(&env)?;
            store.ensure_writable()?;
            ase_llm::save_config(&env, &config)?;
            eprintln!(
                "storing the key for provider {} / profile {} ({})",
                profile.provider, profile.profile, profile.endpoint.origin
            );
            let key = ase_llm::read_secret_line(ase_llm::KEY_PROMPT)?;
            store.set(&profile.provider, &profile.endpoint.origin, &key)?;
            if has(rest, "--json") {
                let value = json!({
                    "stored": true,
                    "profile": name,
                    "provider": profile.provider,
                    "origin": profile.endpoint.origin,
                    "credential_store": store.backend(),
                });
                println!("{}", serde_json::to_string_pretty(&value).unwrap());
            } else {
                println!(
                    "stored key for provider {} at {} in {}",
                    profile.provider,
                    profile.endpoint.origin,
                    store.backend()
                );
            }
            Ok(())
        }
        _ => Err(err("unknown llm config command (see `ase llm --help`)")),
    }
}

fn llm_profile_name(args: &[String]) -> String {
    opt(args, "--profile").unwrap_or_else(|| ase_llm::DEFAULT_PROFILE.to_string())
}

fn llm_settings(
    args: &[String],
) -> Result<(
    Option<String>,
    Option<ase_llm::ApiKind>,
    Option<String>,
    Option<String>,
    ase_llm::EffortEdit,
)> {
    let provider = opt(args, "--provider");
    let api = match opt(args, "--type") {
        Some(value) => Some(
            ase_llm::ApiKind::parse(&value)
                .ok_or_else(|| {
                    err("--type must be chat-completions, responses, decision, gemini or gemini-decision")
                })?,
        ),
        None => None,
    };
    let endpoint = opt(args, "--endpoint");
    let model = opt(args, "--model");
    let effort = match opt(args, "--effort") {
        Some(value) if value.eq_ignore_ascii_case("none") => ase_llm::EffortEdit::Clear,
        Some(value) => ase_llm::EffortEdit::Set(value),
        None => ase_llm::EffortEdit::Keep,
    };
    Ok((provider, api, endpoint, model, effort))
}

fn cmd_llm_key(args: &[String]) -> Result<()> {
    let Some(sub) = args.first().map(String::as_str) else {
        return print_llm_status(None, false, false);
    };
    if matches!(sub, "--help" | "-h") {
        llm_usage();
        return Ok(());
    }
    let rest = &args[1..];
    match sub {
        "set" => {
            llm_expect(rest, &["--profile"], &["--json"])?;
            let env = ase_llm::capture_env();
            let profile = ase_llm::resolve_profile(&env, opt(rest, "--profile").as_deref())?;
            let store = ase_llm::open_store(&env)?;
            store.ensure_writable()?;
            eprintln!(
                "storing the key for provider {} / profile {} ({})",
                profile.provider, profile.profile, profile.endpoint.origin
            );
            let key = ase_llm::read_secret_line(ase_llm::KEY_PROMPT)?;
            store.set(&profile.provider, &profile.endpoint.origin, &key)?;
            if has(rest, "--json") {
                let value = json!({
                    "stored": true,
                    "profile": profile.profile,
                    "provider": profile.provider,
                    "origin": profile.endpoint.origin,
                    "credential_store": store.backend(),
                });
                println!("{}", serde_json::to_string_pretty(&value).unwrap());
            } else {
                println!(
                    "stored key for provider {} at {} in {}",
                    profile.provider,
                    profile.endpoint.origin,
                    store.backend()
                );
            }
            Ok(())
        }
        "clear" => {
            llm_expect(rest, &["--profile"], &["--json"])?;
            let env = ase_llm::capture_env();
            let profile = ase_llm::resolve_profile(&env, opt(rest, "--profile").as_deref())?;
            let store = ase_llm::open_store(&env)?;
            let deleted = store.delete(&profile.provider, &profile.endpoint.origin)?;
            let env_override = ase_llm::env_key_present(&env, profile.api);
            if has(rest, "--json") {
                let value = json!({
                    "deleted": deleted,
                    "profile": profile.profile,
                    "provider": profile.provider,
                    "origin": profile.endpoint.origin,
                    "credential_store": store.backend(),
                    "env_override_active": env_override,
                });
                println!("{}", serde_json::to_string_pretty(&value).unwrap());
            } else {
                println!(
                    "{} credential for provider {} at {}",
                    if deleted { "deleted" } else { "no stored" },
                    profile.provider,
                    profile.endpoint.origin
                );
                if env_override {
                    println!(
                        "note: a nonempty {} still overrides the store",
                        profile.api.key_env()
                    );
                }
            }
            Ok(())
        }
        "status" => {
            llm_expect(rest, &["--profile"], &["--json"])?;
            print_llm_status(opt(rest, "--profile"), has(rest, "--json"), true)
        }
        _ => Err(err("unknown llm key command (see `ase llm --help`)")),
    }
}

const SMOKE_INSTRUCTION: &str =
    "Synthetic protocol check. Choose the option that matches the two synthetic messages. \
     No real session, journal or user content is present.";
const SMOKE_INPUT: &str = "<|ase:prev|>\nsynthetic earlier task instruction\n<|ase:curr|>\nsynthetic later refinement of the same task\n";
const SMOKE_CHOICES: [(&str, &str); 3] = [
    ("split", "A distinct synthetic task starts."),
    ("continue", "The same synthetic task continues."),
    ("unknown", "The synthetic pair does not decide."),
];

/// One fixed synthetic call through the configured protocol. This is the only
/// `ase llm` command that opens a socket; it never sends journal, database,
/// recap or excerpt data, and its report contains no credential or raw body.
fn cmd_llm_smoke(args: &[String]) -> Result<()> {
    llm_expect(
        args,
        &["--profile", "--timeout-ms"],
        &["--json"],
    )?;
    let timeout_ms = match opt(args, "--timeout-ms") {
        Some(text) => text
            .parse::<u64>()
            .map_err(|_| err("invalid --timeout-ms"))?,
        None => ase_llm::DEFAULT_TIMEOUT_MS,
    };
    let env = ase_llm::capture_env();
    let resolution = ase_llm::resolve(&env, opt(args, "--profile").as_deref())?;
    let profile = &resolution.profile;
    let spec = ase_llm::CallSpec {
        api: profile.api,
        model: &profile.model,
        effort: profile.effort.as_deref(),
        instruction: SMOKE_INSTRUCTION,
        input: SMOKE_INPUT,
        question_id: ase_judge::rubric::QUESTION_ID,
        choices: &SMOKE_CHOICES,
    };
    let result = ase_llm::call(
        &resolution,
        &spec,
        std::time::Duration::from_millis(timeout_ms.max(1)),
    )
    .map_err(|e| err(e.to_string()))?;
    let answer_kind = match &result.answer {
        ase_llm::WireAnswer::Text(_) => "text",
        ase_llm::WireAnswer::Choice { .. } => "choice",
    };
    let probabilities = match &result.answer {
        ase_llm::WireAnswer::Choice { probabilities, .. } => Some(probabilities.clone()),
        ase_llm::WireAnswer::Text(_) => None,
    };
    let (label, confidence, abstain_reason) =
        ase_judge::rubric::interpret(profile.api, result.answer)
            .map_err(|code| err(format!("typed rubric mapping failed: {code}")))?;
    let value = json!({
        "schema": "ase-llm-smoke/v1",
        "synthetic": true,
        "network": "used",
        "profile": profile.profile,
        "provider": profile.provider,
        "api": profile.api.as_str(),
        "endpoint": profile.endpoint.url,
        "origin": profile.endpoint.origin,
        "model": profile.model,
        "effort": {
            "value": profile.effort,
            "sent": result.effort_sent,
        },
        "auth": {
            "header": result.auth_header,
            "scheme": result.auth_scheme,
            "value_present": true,
            "value_printed": false,
        },
        "request": {
            "body_keys": result.body_keys,
            "session_header_sent": result.session_sent,
        },
        "answer": {
            "kind": answer_kind,
            "label": label.as_str(),
            "confidence": confidence.as_ref().map(|value| json!({
                "semantics": value.semantics(),
                "value": value.value_json(),
            })),
            "abstain_reason": abstain_reason,
            "probabilities": probabilities,
            "model": result.model,
            "usage": ase_llm::usage_json(&result.usage),
        },
        "status": result.status,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value)
            .map_err(|e| err(format!("serialize llm smoke report: {e}")))?
    );
    Ok(())
}

fn cmd_llm_status(args: &[String]) -> Result<()> {
    llm_expect(args, &["--profile"], &["--json"])?;
    print_llm_status(opt(args, "--profile"), has(args, "--json"), true)
}

fn print_llm_status(
    profile: Option<String>,
    as_json: bool,
    exit_on_not_ready: bool,
) -> Result<()> {
    let env = ase_llm::capture_env();
    let report = ase_llm::status(&env, profile.as_deref())?;
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|e| err(format!("serialize llm status: {e}")))?
        );
    } else {
        println!("config: {}", report.config_path);
        println!("config source: {}", report.config_source);
        if let Some(name) = &report.profile {
            println!("profile: {name}");
        }
        if let Some(value) = &report.provider {
            println!("provider: {value}");
        }
        if let Some(value) = &report.api {
            println!("api: {value}");
        }
        if let Some(value) = &report.endpoint {
            println!("endpoint: {value}");
        }
        if let Some(value) = &report.origin {
            println!("origin: {value}");
        }
        if let Some(value) = &report.model {
            println!("model: {value}");
        }
        println!(
            "effort: {}",
            report.effort.as_deref().unwrap_or("none")
        );
        println!(
            "credential: {} ({})",
            report.credential_source, report.credential_store
        );
        println!(
            "env overrides: {}",
            if report.env_overrides.is_empty() {
                "none".to_string()
            } else {
                report.env_overrides.join(", ")
            }
        );
        for note in &report.notes {
            println!("note: {note}");
        }
        for check in &report.checks {
            println!(
                "check {}: {} ({})",
                check.name,
                if check.ok { "ok" } else { "failed" },
                check.detail
            );
        }
        println!("ready: {}", report.ready);
        println!("network: not used");
        if !report.configured {
            println!();
            println!(
                "one-time enrollment: ase llm config enroll --profile default \\\n  --type chat-completions \\\n  --endpoint https://api.openai.com/v1/chat/completions \\\n  --model <MODEL>"
            );
        }
    }
    if exit_on_not_ready && report.configured && !report.ready {
        std::process::exit(3);
    }
    Ok(())
}
