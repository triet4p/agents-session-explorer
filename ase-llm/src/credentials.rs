//! Credential storage. Keys are user-scoped and persistent, never plaintext:
//!
//! - the default backend is a Windows Credential Manager generic credential
//!   (user-scoped, `CRED_PERSIST_LOCAL_MACHINE`) keyed by normalized origin;
//! - an isolated backend selected explicitly with
//!   `ASE_LLM_CREDENTIAL_STORE=dpapi-file:<dir>` stores DPAPI-encrypted blobs
//!   (same Windows user, current machine) under that directory. It exists for
//!   verification and workspace-local isolation, not as a plaintext fallback;
//! - on a platform without a supported native store the backend reports
//!   `unsupported` and every write/read fails explicitly. `OPENAI_API_KEY`
//!   remains usable by the resolver as a process override.

use ase_core::{err, Result};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

use crate::config::Env;
use crate::secret::ApiKey;

pub const STORE_ENV: &str = "ASE_LLM_CREDENTIAL_STORE";
pub const NATIVE_BACKEND: &str = "windows-credential-manager";
pub const DPAPI_BACKEND: &str = "dpapi-file";
pub const UNSUPPORTED_BACKEND: &str = "unsupported";
pub const UNSUPPORTED_PLATFORM: &str =
    "credential_platform_unsupported: no native user credential store is available on this platform";

/// One credential per `(provider, normalized origin)`. Switching to another
/// profile of the same provider on the same origin never asks for the key
/// again; a different provider on the same origin is isolated; and a changed
/// host never reuses a key.
pub trait CredentialStore {
    fn backend(&self) -> &'static str;
    fn get(&self, provider: &str, origin: &str) -> Result<Option<ApiKey>>;
    fn set(&self, provider: &str, origin: &str, key: &ApiKey) -> Result<()>;
    /// Returns whether an entry existed.
    fn delete(&self, provider: &str, origin: &str) -> Result<bool>;

    /// Capability check called before any hidden prompt, so an unsupported
    /// platform fails without asking the user for a key first.
    fn ensure_writable(&self) -> Result<()> {
        Ok(())
    }
}

/// The configured backend name, without opening it. Used by status even when
/// the store cannot be used.
pub fn selected_backend(env: &Env) -> Result<&'static str> {
    match selection(env)? {
        Selection::Native => Ok(NATIVE_BACKEND),
        Selection::DpapiFile(_) => Ok(DPAPI_BACKEND),
    }
}

enum Selection {
    Native,
    DpapiFile(PathBuf),
}

fn selection(env: &Env) -> Result<Selection> {
    let Some(value) = env.get(STORE_ENV) else {
        return Ok(Selection::Native);
    };
    let value = value.trim();
    if value.is_empty() || value == "native" {
        return Ok(Selection::Native);
    }
    if let Some(dir) = value.strip_prefix("dpapi-file:") {
        if dir.trim().is_empty() {
            return Err(err("credential_store_dir_missing"));
        }
        return Ok(Selection::DpapiFile(PathBuf::from(dir)));
    }
    Err(err(
        "credential_store_setting_unrecognized: expected native or dpapi-file:<dir>",
    ))
}

pub fn open_store(env: &Env) -> Result<Box<dyn CredentialStore>> {
    match selection(env)? {
        Selection::Native => Ok(native_store()),
        Selection::DpapiFile(dir) => dpapi_store(dir),
    }
}

#[cfg(windows)]
fn native_store() -> Box<dyn CredentialStore> {
    Box::new(NativeStore)
}

#[cfg(not(windows))]
fn native_store() -> Box<dyn CredentialStore> {
    Box::new(UnsupportedStore)
}

#[cfg(windows)]
fn dpapi_store(dir: PathBuf) -> Result<Box<dyn CredentialStore>> {
    Ok(Box::new(DpapiFileStore { dir }))
}

#[cfg(not(windows))]
fn dpapi_store(_dir: PathBuf) -> Result<Box<dyn CredentialStore>> {
    Ok(Box::new(UnsupportedStore))
}

#[cfg(not(windows))]
struct UnsupportedStore;

#[cfg(not(windows))]
impl CredentialStore for UnsupportedStore {
    fn backend(&self) -> &'static str {
        UNSUPPORTED_BACKEND
    }

    fn get(&self, _provider: &str, _origin: &str) -> Result<Option<ApiKey>> {
        Err(err(UNSUPPORTED_PLATFORM))
    }

    fn set(&self, _provider: &str, _origin: &str, _key: &ApiKey) -> Result<()> {
        Err(err(UNSUPPORTED_PLATFORM))
    }

    fn delete(&self, _provider: &str, _origin: &str) -> Result<bool> {
        Err(err(UNSUPPORTED_PLATFORM))
    }

    fn ensure_writable(&self) -> Result<()> {
        Err(err(UNSUPPORTED_PLATFORM))
    }
}

#[cfg(windows)]
struct NativeStore;

#[cfg(windows)]
impl CredentialStore for NativeStore {
    fn backend(&self) -> &'static str {
        NATIVE_BACKEND
    }

    fn get(&self, provider: &str, origin: &str) -> Result<Option<ApiKey>> {
        crate::win::native_get(provider, origin)
    }

    fn set(&self, provider: &str, origin: &str, key: &ApiKey) -> Result<()> {
        crate::win::native_set(provider, origin, key)
    }

    fn delete(&self, provider: &str, origin: &str) -> Result<bool> {
        crate::win::native_delete(provider, origin)
    }
}

#[cfg(windows)]
struct DpapiFileStore {
    dir: PathBuf,
}

#[cfg(windows)]
impl DpapiFileStore {
    fn path_for(&self, provider: &str, origin: &str) -> PathBuf {
        self.dir.join(format!(
            "{}.cred",
            sha256_hex(scope_identity(provider, origin).as_bytes())
        ))
    }
}

/// Unambiguous credential identity. Provider names cannot contain `|`, and
/// endpoint origins are ASCII printable, so the pair cannot alias.
pub(crate) fn scope_identity(provider: &str, origin: &str) -> String {
    format!("{provider}|{origin}")
}

#[cfg(windows)]
impl CredentialStore for DpapiFileStore {
    fn backend(&self) -> &'static str {
        DPAPI_BACKEND
    }

    fn get(&self, provider: &str, origin: &str) -> Result<Option<ApiKey>> {
        let path = self.path_for(provider, origin);
        let cipher = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None)
            }
            Err(error) => {
                return Err(err(format!("credential_store_read_failed: {error}")))
            }
        };
        let mut plain = crate::win::dpapi_unprotect(&cipher)?;
        let parsed: StoredCredential = serde_json::from_slice(&plain)
            .map_err(|_| err("credential_store_corrupt"))?;
        zero(&mut plain);
        if parsed.version != 2 {
            return Err(err("credential_store_corrupt"));
        }
        if parsed.provider != provider {
            return Err(err("credential_provider_mismatch"));
        }
        if parsed.origin != origin {
            return Err(err("credential_origin_mismatch"));
        }
        let mut parsed = parsed;
        let key = std::mem::take(&mut parsed.key);
        ApiKey::from_owned(key).map(Some)
    }

    fn set(&self, provider: &str, origin: &str, key: &ApiKey) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| err(format!("credential_store_dir_create_failed: {e}")))?;
        let mut payload = format!(
            "{{\"version\":2,\"provider\":{},\"origin\":{},\"key\":{}}}",
            json_string(provider),
            json_string(origin),
            json_string(key.expose())
        );
        let cipher = crate::win::dpapi_protect(payload.as_bytes())?;
        zero_string(&mut payload);
        let path = self.path_for(provider, origin);
        let temp = path.with_extension(format!("tmp{}", std::process::id()));
        std::fs::write(&temp, &cipher)
            .map_err(|e| err(format!("credential_store_write_failed: {e}")))?;
        std::fs::rename(&temp, &path)
            .map_err(|e| err(format!("credential_store_publish_failed: {e}")))?;
        Ok(())
    }

    fn delete(&self, provider: &str, origin: &str) -> Result<bool> {
        match std::fs::remove_file(self.path_for(provider, origin)) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(err(format!(
                "credential_store_delete_failed: {error}"
            ))),
        }
    }
}

#[cfg(windows)]
#[derive(serde::Deserialize)]
struct StoredCredential {
    version: u32,
    provider: String,
    origin: String,
    key: String,
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(windows)]
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[cfg(windows)]
fn zero(bytes: &mut [u8]) {
    for byte in bytes {
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
}

#[cfg(windows)]
fn zero_string(value: &mut String) {
    unsafe {
        for byte in value.as_bytes_mut() {
            std::ptr::write_volatile(byte, 0);
        }
    }
}
