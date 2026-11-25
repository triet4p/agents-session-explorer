//! `ase adapter omp-stock` — advisory claim scanning and claim reopening.
//!
//! The view adapter exposes the published index and raw store to the stock
//! library. Selection, sidecar roots, claim serialization and the reopen exit
//! contract stay in this CLI module; the library owns claim policy.

use crate::cli_option::{has, opt, parse_opt};
use crate::cli_output::emit;
use crate::index::load_with_publication;
use ase_adapter_omp_stock::{
    path_eq, reopen, scan, ClaimsFile, Options as AdapterOptions, StockView, ViewEvent, ViewFile,
};
use ase_core::{err, RawStore, Result, SpanRef};
use ase_index::{FsRawStore, Index};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

fn adapter_view(args: &[String], reopen_mode: bool) -> Result<AdapterView> {
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
    Ok(AdapterView {
        idx,
        store,
        files,
        generation,
        roots,
    })
}

pub fn run(args: &[String]) -> Result<()> {
    if args.first().map(String::as_str) != Some("omp-stock") {
        return Err(err("adapter needs a provider: omp-stock"));
    }
    let rest = &args[1..];
    if let Some(path) = opt(rest, "--reopen") {
        let bytes = fs::read(&path).map_err(|e| err(format!("read claims {}: {e}", path)))?;
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
