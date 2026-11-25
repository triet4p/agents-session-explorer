//! Index-backed journal projection shared by the work and analytics commands.
//!
//! Both groups resolve the address index, open the pinned raw store and check
//! projected evidence against the same generation. Selection, rendering and
//! command-specific output stay with the owning command module.

use ase_core::{err, RawStore, Result, SpanRef};
use ase_index::{FsRawStore, Index};
use ase_postings::publication;
use ase_work::{JournalProjection, ManifestEntry, Window};
use std::fs;
use std::sync::Arc;
use crate::{index_path, opt};

// ---------------------------------------------------------- work projection

pub(crate) struct WorkFile {
    pub(crate) file_id: u32,
    pub(crate) rel: String,
    pub(crate) layout: ase_work::util::Layout,
}

pub(crate) struct SelectedWork {
    pub(crate) files: Vec<WorkFile>,
    pub(crate) root_journals_scanned: u64,
    pub(crate) root_journals_selected: u64,
    pub(crate) root_journals_outside_window: u64,
    pub(crate) root_journals_missing_header: u64,
}

pub(crate) struct ProjectedWork {
    pub(crate) projection: JournalProjection,
    pub(crate) entry: ManifestEntry,
}


pub(crate) fn load_work(args: &[String]) -> Result<(Arc<Index>, FsRawStore, String)> {
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


pub(crate) fn parse_work_window(args: &[String]) -> Result<Option<Window>> {
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


pub(crate) fn validate_work_projection(idx: &Index, projection: &JournalProjection) -> Result<()> {
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


pub(crate) fn project_work_file(
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


pub(crate) fn read_jsonl<T: serde::de::DeserializeOwned>(path: &str) -> Result<Vec<T>> {
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
