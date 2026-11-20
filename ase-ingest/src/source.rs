use ase_core::{err, Encoding, Provider, Result, Source, SourceFile};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub struct OmpSource {
    pub root: PathBuf,
}

impl Source for OmpSource {
    fn discover(&self) -> Result<Vec<SourceFile>> {
        discover_typed(&self.root, Provider::Omp)
    }
}

pub struct CodexSource {
    pub root: PathBuf,
}

impl Source for CodexSource {
    fn discover(&self) -> Result<Vec<SourceFile>> {
        discover_typed(&self.root, Provider::Codex)
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries =
        fs::read_dir(dir).map_err(|e| err(format!("read_dir {}: {e}", dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| err(format!("dir entry {}: {e}", dir.display())))?;
        let ty = entry
            .file_type()
            .map_err(|e| err(format!("file_type {}: {e}", entry.path().display())))?;
        if ty.is_dir() {
            walk(&entry.path(), out)?;
        } else if ty.is_file() {
            out.push(entry.path());
        }
    }
    Ok(())
}

fn discover_typed(root: &Path, provider: Provider) -> Result<Vec<SourceFile>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    if !root.is_dir() {
        return Err(err(format!("{} is not a directory", root.display())));
    }
    let mut paths = Vec::new();
    walk(root, &mut paths)?;
    let mut files = Vec::new();
    for path in paths {
        let name = path.file_name().and_then(|x| x.to_str()).unwrap_or("");
        let encoding = if name.ends_with(".jsonl.zst") && provider == Provider::Codex {
            Some(Encoding::JsonlZstd)
        } else if name.ends_with(".jsonl") {
            Some(Encoding::Jsonl)
        } else {
            None
        };
        let Some(encoding) = encoding else { continue };
        let meta = fs::metadata(&path)
            .map_err(|e| err(format!("metadata {}: {e}", path.display())))?;
        let modified = meta
            .modified()
            .map_err(|e| err(format!("mtime {}: {e}", path.display())))?;
        let dur = modified
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| std::time::Duration::ZERO);
        files.push(SourceFile {
            id: 0,
            path: path.to_string_lossy().into_owned(),
            provider,
            encoding,
            size: meta.len(),
            mtime_secs: dur.as_secs() as i64,
            mtime_nanos: dur.subsec_nanos(),
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    for (i, f) in files.iter_mut().enumerate() {
        f.id = i as u32;
    }
    Ok(files)
}

/// Discover both providers into one deterministically ordered file list.
/// File ids are assigned after the merged sort, so spans are stable for a
/// given corpus snapshot.
pub fn discover_all(omp_root: &Path, codex_root: &Path) -> Result<Vec<SourceFile>> {
    let mut files = Vec::new();
    files.extend(OmpSource { root: omp_root.to_path_buf() }.discover()?);
    let codex_base = files.len() as u32;
    let mut codex = CodexSource { root: codex_root.to_path_buf() }.discover()?;
    for f in codex.iter_mut() {
        f.id += codex_base;
    }
    files.extend(codex);
    files.sort_by(|a, b| a.path.cmp(&b.path));
    for (i, f) in files.iter_mut().enumerate() {
        f.id = i as u32;
    }
    Ok(files)
}

/// Strip a `.jsonl.zst` or `.jsonl` suffix from a path for path-derived ids.
pub fn base_name(path: &str) -> String {
    let mut s = path.to_string();
    if s.ends_with(".jsonl.zst") {
        s.truncate(s.len() - ".jsonl.zst".len());
    } else if s.ends_with(".jsonl") {
        s.truncate(s.len() - ".jsonl".len());
    }
    Path::new(&s)
        .file_name()
        .and_then(|x| x.to_str())
        .unwrap_or(&s)
        .to_string()
}
