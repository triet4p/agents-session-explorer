//! Address index build and pinned publication access.
//!
//! The `index` command builds the binary address or a full published layer
//! set. Derived commands load the same pinned publication from the helpers
//! here; no model, network or prose is consulted.

use crate::cli_option::{has, opt};
use crate::cli_output::emit;
use crate::config;
use ase_core::{err, Result};
use ase_index::{build, decode_corpus, sha256_hex, write_address, DecodedCorpus, FsRawStore, Index};
use ase_postings::{publication, Publication, WriterInput};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};


const DEFAULT_INDEX: &str = "ase.index";

pub(crate) fn index_path(args: &[String]) -> PathBuf {
    PathBuf::from(opt(args, "--index").unwrap_or_else(|| DEFAULT_INDEX.to_string()))
}

/// Resolve the logical `--index` path to the address a reader must open.
///
/// A usable publication always wins and pins its own immutable address. When
/// the sidecar is missing or unusable, the path the caller named is returned
/// unchanged together with the reason; this never scans a directory, picks a
/// newer generation or substitutes the default index name.
pub(crate) fn resolve_source(args: &[String]) -> (PathBuf, Option<Publication>, Option<String>) {
    let logical = index_path(args);
    match publication::open(&logical) {
        Ok(Some(published)) => (published.address_path.clone(), Some(published), None),
        Ok(None) => (logical, None, Some("sidecar_missing".to_string())),
        Err(e) => (logical, None, Some(e.to_string())),
    }
}

/// Guard an explicit `--generation` pin before a reader touches raw bytes.
/// Saved flag arrays without the pin keep their address-only behavior.
pub(crate) fn check_generation_pin(args: &[String], command: &str) -> Result<()> {
    let Some(pinned) = opt(args, "--generation") else {
        return Ok(());
    };
    let logical = index_path(args);
    let published = match publication::open(&logical) {
        Ok(Some(published)) => published,
        Ok(None) => {
            return Err(err(format!(
                "{command}: a pinned --generation needs a usable publication at {}",
                logical.display()
            )))
        }
        Err(e) => return Err(err(format!("{command}: publication unavailable ({e})"))),
    };
    if pinned != published.generation {
        let v = json!({
            "command": command,
            "status": "generation_mismatch",
            "requested_generation": &pinned,
            "active_generation": &published.generation,
            "address_usable": true,
            "note": "refused before reading raw: the locator belongs to another generation; search again on the intended --index"
        });
        emit(&v, args);
        std::process::exit(4);
    }
    Ok(())
}

/// Open one resolved address. A failed address read also names why the
/// publication could not be used, so a missing or mismatched layer is never
/// mistaken for an empty result.
pub(crate) fn open_address(address: &Path, reason: Option<&str>) -> Result<Index> {
    match Index::open(address) {
        Ok(idx) => Ok(idx),
        Err(e) => match reason {
            Some(reason) => Err(err(format!("{e}; publication unavailable ({reason})"))),
            None => Err(e),
        },
    }
}

/// Address-only load for readers that never touch raw spans, postings or prose.
pub(crate) fn load_address(args: &[String]) -> Result<Arc<Index>> {
    let (address, _, reason) = resolve_source(args);
    Ok(Arc::new(open_address(&address, reason.as_deref())?))
}

pub(crate) fn load_with_publication(
    args: &[String],
) -> Result<(Arc<Index>, FsRawStore, Option<Publication>, Option<String>)> {
    let (address, published, reason) = resolve_source(args);
    let idx = Arc::new(open_address(&address, reason.as_deref())?);
    let store = FsRawStore::new(idx.clone());
    Ok((idx, store, published, reason))
}

pub(crate) fn load(args: &[String]) -> Result<(Arc<Index>, FsRawStore)> {
    let (idx, store, _, _) = load_with_publication(args)?;
    Ok((idx, store))
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

pub fn run(args: &[String]) -> Result<()> {
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

fn index_usage() {
    println!(
        "ase index [--layer address|postings|all] [--omp-root DIR] [--codex-root DIR]\n\
         [--index FILE] [--fail-before-publish] [--json]"
    );
}

#[cfg(test)]
mod tests {
    use super::{index_path, open_address, resolve_source};

    #[test]
    fn source_selection_keeps_the_named_path_without_a_publication() {
        let args = vec![
            "source-selection-canary".to_string(),
            "--index".to_string(),
            "missing logical path/ase.index".to_string(),
        ];
        let (address, published, reason) = resolve_source(&args);
        assert_eq!(address, index_path(&args));
        assert!(published.is_none());
        assert_eq!(reason.as_deref(), Some("sidecar_missing"));
        let error = match open_address(&address, reason.as_deref()) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("a missing address cannot open"),
        };
        assert!(
            error.contains("publication unavailable (sidecar_missing)"),
            "{error}"
        );
    }
}
