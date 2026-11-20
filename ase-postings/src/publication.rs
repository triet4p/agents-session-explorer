//! Generation publication: immutable artifacts plus one validated pointer.
//!
//! Neither layer is ever read as "usable" just because a file exists. The
//! pointer is the single enable switch for search; a missing, tombstoned or
//! mismatched pointer yields `layer_unavailable`, never an empty hit set.

use ase_core::{
    err, ALLOWLIST_VERSION, CALLSIG_VERSION, PROJECTION_VERSION, Result, SHINGLE_VERSION,
    TOKENIZER_VERSION,
};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

pub const POINTER_MAGIC: &str = "ASEPUB01";
pub const MANIFEST_MAGIC: &str = "ASEMAN01";
pub const SCHEMA: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactRef {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pointer {
    pub magic: String,
    pub schema: u32,
    pub status: String,
    pub reason: String,
    pub generation: String,
    pub address: ArtifactRef,
    pub postings: ArtifactRef,
    pub manifest: ArtifactRef,
    pub full_footprint_bytes: u64,
    pub versions: BTreeMap<String, String>,
    pub counts: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileWatermark {
    pub id: u32,
    pub provider: String,
    pub encoding: String,
    pub size: u64,
    pub mtime_secs: i64,
    pub mtime_nanos: u32,
    pub digest: String,
    pub events: u32,
    pub event_start: u32,
    pub event_end: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub magic: String,
    pub schema: u32,
    pub generation: String,
    pub core_digest: String,
    pub versions: BTreeMap<String, String>,
    pub counts: BTreeMap<String, u64>,
    pub files: Vec<FileWatermark>,
    pub address_bytes: u64,
    pub address_sha256: String,
    pub postings_bytes: u64,
    pub postings_sha256: String,
}

#[derive(Clone, Debug)]
pub struct Publication {
    pub generation: String,
    pub address_path: PathBuf,
    pub postings_path: PathBuf,
    pub manifest_path: PathBuf,
    pub manifest: Manifest,
    pub pointer: Pointer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unavailable {
    SidecarMissing,
    Tombstone(String),
    BadPointer(String),
    UnsupportedSchema,
    BadPath(String),
    AddressDigestMismatch,
    AddressShapeMismatch,
    PostingsDigestMismatch,
    ManifestDigestMismatch,
    GenerationMismatch,
    MetaMismatch(String),
    VersionMismatch,
    Io(String),
}

impl Unavailable {
    pub fn code(&self) -> &'static str {
        match self {
            Unavailable::SidecarMissing => "sidecar_missing",
            Unavailable::Tombstone(_) => "pointer_tombstoned",
            Unavailable::BadPointer(_) => "pointer_invalid",
            Unavailable::UnsupportedSchema => "schema_unsupported",
            Unavailable::BadPath(_) => "pointer_path_invalid",
            Unavailable::AddressDigestMismatch => "address_digest_mismatch",
            Unavailable::AddressShapeMismatch => "address_shape_mismatch",
            Unavailable::PostingsDigestMismatch => "postings_digest_mismatch",
            Unavailable::ManifestDigestMismatch => "manifest_digest_mismatch",
            Unavailable::GenerationMismatch => "generation_mismatch",
            Unavailable::MetaMismatch(_) => "meta_mismatch",
            Unavailable::VersionMismatch => "version_mismatch",
            Unavailable::Io(_) => "io_error",
        }
    }
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())?;
        match self {
            Unavailable::Tombstone(s)
            | Unavailable::BadPointer(s)
            | Unavailable::BadPath(s)
            | Unavailable::MetaMismatch(s)
            | Unavailable::Io(s) => write!(f, "({s})"),
            _ => Ok(()),
        }
    }
}

impl std::error::Error for Unavailable {}

pub fn sidecar_path(index: &Path) -> PathBuf {
    let mut name = index.as_os_str().to_os_string();
    name.push(OsString::from(".pub.json"));
    PathBuf::from(name)
}

pub fn sha256_file(path: &Path) -> Result<(u64, String)> {
    let mut file = File::open(path).map_err(|e| err(format!("open {}: {e}", path.display())))?;
    let mut buf = [0u8; 65_536];
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buf).map_err(|e| err(format!("read {}: {e}", path.display())))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    let bytes = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for b in bytes {
        hex.push_str(&format!("{b:02x}"));
    }
    Ok((total, hex))
}

fn checked_path(base: &Path, stored: &str) -> Result<PathBuf> {
    let rel = Path::new(stored);
    if rel.is_absolute() {
        return Err(err("pointer path is absolute"));
    }
    for c in rel.components() {
        if !matches!(c, Component::Normal(_)) {
            return Err(err("pointer path escapes publication directory"));
        }
    }
    Ok(base.join(rel))
}

/// Replace one publication object. Windows `MoveFileExW` with
/// MOVEFILE_REPLACE_EXISTING gives an atomic directory-entry swap; POSIX
/// `rename` has the same property.
#[cfg(windows)]
pub fn atomic_replace(from: &Path, to: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::iter::once;
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| err(format!("create {}: {e}", parent.display())))?;
    }
    let a: Vec<u16> = from.as_os_str().encode_wide().chain(once(0)).collect();
    let b: Vec<u16> = to.as_os_str().encode_wide().chain(once(0)).collect();
    // MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH
    let ok = unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), 0x1 | 0x8) };
    if ok == 0 {
        return Err(err(format!("atomic replace failed for {}", to.display())));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn atomic_replace(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| err(format!("create {}: {e}", parent.display())))?;
    }
    fs::rename(from, to).map_err(|e| err(format!("atomic replace failed: {e}")))
}

pub fn write_json_atomic(value: &serde_json::Value, target: &Path) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| err(format!("serialize: {e}")))?;
    let tmp = sidecar_path(target);
    let mut f = File::create(&tmp).map_err(|e| err(format!("create {}: {e}", tmp.display())))?;
    f.write_all(&bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| err(format!("write {}: {e}", tmp.display())))?;
    drop(f);
    atomic_replace(&tmp, target)
}

pub fn write_pointer(index: &Path, pointer: &Pointer) -> Result<()> {
    let value = serde_json::to_value(pointer).map_err(|e| err(format!("serialize pointer: {e}")))?;
    write_json_atomic(&value, &sidecar_path(index))
}

pub fn tombstone(index: &Path, reason: &str) -> Result<()> {
    let empty = ArtifactRef { path: "-".to_string(), bytes: 0, sha256: "-".to_string() };
    let pointer = Pointer {
        magic: POINTER_MAGIC.to_string(),
        schema: SCHEMA,
        status: "unavailable".to_string(),
        reason: reason.to_string(),
        generation: "-".to_string(),
        address: empty.clone(),
        postings: empty.clone(),
        manifest: empty,
        full_footprint_bytes: 0,
        versions: BTreeMap::new(),
        counts: BTreeMap::new(),
    };
    write_pointer(index, &pointer)
}

fn expected_versions() -> BTreeMap<String, String> {
    let mut v = BTreeMap::new();
    v.insert("projection".to_string(), PROJECTION_VERSION.to_string());
    v.insert("callsig".to_string(), CALLSIG_VERSION.to_string());
    v.insert("allowlist".to_string(), ALLOWLIST_VERSION.to_string());
    v.insert("tokenizer".to_string(), TOKENIZER_VERSION.to_string());
    v.insert("shingle".to_string(), SHINGLE_VERSION.to_string());
    v
}

pub fn validate_versions(versions: &BTreeMap<String, String>) -> Result<()> {
    let expected = expected_versions();
    if versions.len() != expected.len() {
        return Err(err("version set mismatch"));
    }
    for (k, v) in &expected {
        if versions.get(k) != Some(v) {
            return Err(err(format!("version mismatch: {k}")));
        }
    }
    Ok(())
}

fn check_artifact(reference: &ArtifactRef, base: &Path) -> Result<PathBuf, Unavailable> {
    let path = checked_path(base, &reference.path).map_err(|_| Unavailable::BadPath(reference.path.clone()))?;
    let (size, digest) = sha256_file(&path).map_err(|e| Unavailable::Io(e.0))?;
    if size != reference.bytes || digest != reference.sha256 {
        return Err(Unavailable::Io(format!("artifact mismatch: {}", reference.path)));
    }
    Ok(path)
}

/// Open the publication pointer without ever treating a missing sidecar as a
/// successful empty search. Returns `Ok(None)` only when no sidecar exists.
pub fn open(index: &Path) -> Result<Option<Publication>, Unavailable> {
    let sidecar = sidecar_path(index);
    if !sidecar.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&sidecar).map_err(|e| Unavailable::Io(e.to_string()))?;
    let pointer: Pointer = serde_json::from_slice(&bytes)
        .map_err(|e| Unavailable::BadPointer(e.to_string()))?;
    if pointer.magic != POINTER_MAGIC {
        return Err(Unavailable::BadPointer("magic".to_string()));
    }
    if pointer.schema != SCHEMA {
        return Err(Unavailable::UnsupportedSchema);
    }
    if pointer.status != "ready" {
        return Err(Unavailable::Tombstone(pointer.reason.clone()));
    }
    validate_versions(&pointer.versions).map_err(|_| Unavailable::VersionMismatch)?;
    let base = sidecar.parent().unwrap_or_else(|| Path::new("."));
    let address_path = check_artifact(&pointer.address, base)?;
    let postings_path = check_artifact(&pointer.postings, base)?;
    let manifest_path = check_artifact(&pointer.manifest, base)?;
    let (_, manifest_digest) = sha256_file(&manifest_path).map_err(|e| Unavailable::Io(e.0))?;
    if manifest_digest != pointer.manifest.sha256 {
        return Err(Unavailable::ManifestDigestMismatch);
    }
    let raw = fs::read(&manifest_path).map_err(|e| Unavailable::Io(e.to_string()))?;
    let manifest: Manifest =
        serde_json::from_slice(&raw).map_err(|e| Unavailable::BadPointer(e.to_string()))?;
    if manifest.magic != MANIFEST_MAGIC || manifest.schema != SCHEMA {
        return Err(Unavailable::BadPointer("manifest header".to_string()));
    }
    if manifest.generation != pointer.generation {
        return Err(Unavailable::GenerationMismatch);
    }
    if manifest.address_sha256 != pointer.address.sha256
        || manifest.postings_sha256 != pointer.postings.sha256
        || manifest.address_bytes != pointer.address.bytes
        || manifest.postings_bytes != pointer.postings.bytes
    {
        return Err(Unavailable::ManifestDigestMismatch);
    }
    validate_versions(&manifest.versions).map_err(|_| Unavailable::VersionMismatch)?;
    if manifest.counts != pointer.counts {
        return Err(Unavailable::MetaMismatch("counts".to_string()));
    }
    let address = ase_index::Index::open(&address_path).map_err(|_| Unavailable::AddressShapeMismatch)?;
    if address.event_count as u64 != *pointer.counts.get("events").unwrap_or(&u64::MAX)
        || address.file_count as u64 != *pointer.counts.get("files").unwrap_or(&u64::MAX)
    {
        return Err(Unavailable::AddressShapeMismatch);
    }
    let db = Connection::open_with_flags(&postings_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| Unavailable::Io(e.to_string()))?;
    let generation: String = db
        .query_row("SELECT value FROM meta WHERE key='generation'", [], |r| r.get(0))
        .map_err(|e| Unavailable::MetaMismatch(e.to_string()))?;
    let core: String = db
        .query_row("SELECT value FROM meta WHERE key='manifest_core_digest'", [], |r| r.get(0))
        .map_err(|e| Unavailable::MetaMismatch(e.to_string()))?;
    if generation != pointer.generation || core != manifest.core_digest {
        return Err(Unavailable::GenerationMismatch);
    }
    for (k, v) in &manifest.versions {
        let stored: String = db
            .query_row("SELECT value FROM meta WHERE key=?1", [format!("version_{k}")], |r| r.get(0))
            .map_err(|e| Unavailable::MetaMismatch(e.to_string()))?;
        if &stored != v {
            return Err(Unavailable::VersionMismatch);
        }
    }
    Ok(Some(Publication {
        generation: pointer.generation.clone(),
        address_path,
        postings_path,
        manifest_path,
        manifest,
        pointer: pointer.clone(),
    }))
}
