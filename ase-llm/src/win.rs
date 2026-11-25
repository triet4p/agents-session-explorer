//! Minimal Win32 bindings for the user credential store, DPAPI and hidden
//! console input. Declared by hand so the crate adds no dependency beyond the
//! workspace; every binding is used on a path that the CLI reaches.

#![cfg(windows)]

use ase_core::{err, Result};
use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};

use crate::secret::ApiKey;

const CRED_TYPE_GENERIC: u32 = 1;
const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;
const ERROR_NOT_FOUND: u32 = 1168;
const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;
const STD_INPUT_HANDLE: u32 = 0xFFFF_FFF6; // (DWORD)-10
const ENABLE_ECHO_INPUT: u32 = 0x0004;
const CREDENTIAL_TARGET_PREFIX: &str = "ase.llm.credential.";
const DPAPI_ENTROPY: &[u8] = b"ase-llm-credential-v1";

#[repr(C)]
struct FileTime {
    low: u32,
    high: u32,
}

#[repr(C)]
struct CredentialW {
    flags: u32,
    type_: u32,
    target_name: *mut u16,
    comment: *mut u16,
    last_written: FileTime,
    credential_blob_size: u32,
    credential_blob: *mut u8,
    persist: u32,
    attribute_count: u32,
    attributes: *mut c_void,
    target_alias: *mut u16,
    user_name: *mut u16,
}

#[repr(C)]
struct DataBlob {
    size: u32,
    data: *mut u8,
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn CredWriteW(credential: *const CredentialW, flags: u32) -> i32;
    fn CredReadW(
        target: *const u16,
        credential_type: u32,
        flags: u32,
        credential: *mut *mut CredentialW,
    ) -> i32;
    fn CredDeleteW(target: *const u16, credential_type: u32, flags: u32) -> i32;
    fn CredFree(buffer: *mut c_void);
}

#[link(name = "crypt32")]
unsafe extern "system" {
    fn CryptProtectData(
        data_in: *const DataBlob,
        description: *const u16,
        entropy: *const DataBlob,
        reserved: *mut c_void,
        prompt: *mut c_void,
        flags: u32,
        data_out: *mut DataBlob,
    ) -> i32;
    fn CryptUnprotectData(
        data_in: *const DataBlob,
        description: *mut *mut u16,
        entropy: *const DataBlob,
        reserved: *mut c_void,
        prompt: *mut c_void,
        flags: u32,
        data_out: *mut DataBlob,
    ) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
    fn GetStdHandle(std_handle: u32) -> isize;
    fn GetConsoleMode(console: isize, mode: *mut u32) -> i32;
    fn SetConsoleMode(console: isize, mode: u32) -> i32;
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn read_wide(pointer: *const u16) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    let mut length = 0usize;
    while *pointer.add(length) != 0 {
        length += 1;
    }
    Some(String::from_utf16_lossy(std::slice::from_raw_parts(
        pointer, length,
    )))
}

fn target_for(provider: &str, origin: &str) -> String {
    let identity = crate::credentials::scope_identity(provider, origin);
    let digest = crate::credentials::sha256_hex(identity.as_bytes());
    format!("{CREDENTIAL_TARGET_PREFIX}{}", &digest[..32])
}

fn scope_user_name(provider: &str, origin: &str) -> String {
    crate::credentials::scope_identity(provider, origin)
}

fn os_error(code: u32) -> String {
    format!("os_error_{code}")
}

pub fn native_get(provider: &str, origin: &str) -> Result<Option<ApiKey>> {
    let expected_user = scope_user_name(provider, origin);
    let target = wide(&target_for(provider, origin));
    let mut pointer: *mut CredentialW = std::ptr::null_mut();
    let ok = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut pointer) };
    if ok == 0 {
        let code = unsafe { GetLastError() };
        if code == ERROR_NOT_FOUND {
            return Ok(None);
        }
        return Err(err(format!("credential_read_failed: {}", os_error(code))));
    }
    if pointer.is_null() {
        return Err(err("credential_read_returned_null"));
    }
    let credential = unsafe { &*pointer };
    let user_name = unsafe { read_wide(credential.user_name) };
    let blob_size = credential.credential_blob_size as usize;
    let mut blob = if blob_size > 0 && !credential.credential_blob.is_null() {
        unsafe {
            std::slice::from_raw_parts(credential.credential_blob, blob_size).to_vec()
        }
    } else {
        Vec::new()
    };
    unsafe {
        if !credential.credential_blob.is_null() {
            std::ptr::write_bytes(credential.credential_blob, 0, blob_size);
        }
        CredFree(pointer as *mut c_void);
    }
    if user_name.as_deref() != Some(expected_user.as_str()) {
        zero_bytes(&mut blob);
        return Err(err("credential_scope_mismatch"));
    }
    if blob.is_empty() {
        return Err(err("credential_blob_empty"));
    }
    let text = String::from_utf8(blob).map_err(|_| err("credential_blob_invalid"))?;
    ApiKey::from_owned(text).map(Some)
}

pub fn native_set(provider: &str, origin: &str, key: &ApiKey) -> Result<()> {
    let mut target = wide(&target_for(provider, origin));
    let mut user_name = wide(&scope_user_name(provider, origin));
    let mut comment = wide(&format!(
        "ASE LLM API key for provider {provider} at {origin}"
    ));
    let mut blob = key.expose().as_bytes().to_vec();
    if blob.len() > u32::MAX as usize {
        return Err(err("credential_blob_too_large"));
    }
    let credential = CredentialW {
        flags: 0,
        type_: CRED_TYPE_GENERIC,
        target_name: target.as_mut_ptr(),
        comment: comment.as_mut_ptr(),
        last_written: FileTime { low: 0, high: 0 },
        credential_blob_size: blob.len() as u32,
        credential_blob: blob.as_mut_ptr(),
        persist: CRED_PERSIST_LOCAL_MACHINE,
        attribute_count: 0,
        attributes: std::ptr::null_mut(),
        target_alias: std::ptr::null_mut(),
        user_name: user_name.as_mut_ptr(),
    };
    let ok = unsafe { CredWriteW(&credential, 0) };
    let code = unsafe { GetLastError() };
    zero_bytes(&mut blob);
    if ok == 0 {
        return Err(err(format!("credential_write_failed: {}", os_error(code))));
    }
    Ok(())
}

pub fn native_delete(provider: &str, origin: &str) -> Result<bool> {
    let target = wide(&target_for(provider, origin));
    let ok = unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
    if ok == 0 {
        let code = unsafe { GetLastError() };
        if code == ERROR_NOT_FOUND {
            return Ok(false);
        }
        return Err(err(format!(
            "credential_delete_failed: {}",
            os_error(code)
        )));
    }
    Ok(true)
}

pub fn dpapi_protect(plain: &[u8]) -> Result<Vec<u8>> {
    if plain.len() > u32::MAX as usize {
        return Err(err("credential_blob_too_large"));
    }
    let input = DataBlob {
        size: plain.len() as u32,
        data: plain.as_ptr() as *mut u8,
    };
    let entropy = DataBlob {
        size: DPAPI_ENTROPY.len() as u32,
        data: DPAPI_ENTROPY.as_ptr() as *mut u8,
    };
    let mut output = DataBlob {
        size: 0,
        data: std::ptr::null_mut(),
    };
    let ok = unsafe {
        CryptProtectData(
            &input,
            std::ptr::null(),
            &entropy,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        let code = unsafe { GetLastError() };
        return Err(err(format!("dpapi_protect_failed: {}", os_error(code))));
    }
    let bytes = unsafe {
        std::slice::from_raw_parts(output.data, output.size as usize).to_vec()
    };
    unsafe {
        std::ptr::write_bytes(output.data, 0, output.size as usize);
        LocalFree(output.data as *mut c_void);
    }
    Ok(bytes)
}

pub fn dpapi_unprotect(cipher: &[u8]) -> Result<Vec<u8>> {
    if cipher.is_empty() {
        return Err(err("credential_store_corrupt"));
    }
    if cipher.len() > u32::MAX as usize {
        return Err(err("credential_store_corrupt"));
    }
    let input = DataBlob {
        size: cipher.len() as u32,
        data: cipher.as_ptr() as *mut u8,
    };
    let entropy = DataBlob {
        size: DPAPI_ENTROPY.len() as u32,
        data: DPAPI_ENTROPY.as_ptr() as *mut u8,
    };
    let mut output = DataBlob {
        size: 0,
        data: std::ptr::null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            &entropy,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(err("credential_store_unprotect_failed"));
    }
    let mut bytes = unsafe {
        std::slice::from_raw_parts(output.data, output.size as usize).to_vec()
    };
    unsafe {
        std::ptr::write_bytes(output.data, 0, output.size as usize);
        LocalFree(output.data as *mut c_void);
    }
    if bytes.is_empty() {
        zero_bytes(&mut bytes);
        return Err(err("credential_store_corrupt"));
    }
    Ok(bytes)
}

fn zero_bytes(bytes: &mut [u8]) {
    for byte in bytes {
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
}

static ORIGINAL_HANDLE: AtomicIsize = AtomicIsize::new(0);
static ORIGINAL_MODE: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn restore_echo_on_console_event(_event: u32) -> i32 {
    let handle = ORIGINAL_HANDLE.load(Ordering::SeqCst);
    if handle != 0 {
        SetConsoleMode(handle, ORIGINAL_MODE.load(Ordering::SeqCst));
    }
    0 // pass the event to the next handler (the default terminates)
}

/// Disables console echo for the lifetime of the guard. Returns `None` when
/// stdin is not a console (a piped secret is never echoed by a terminal
/// anyway), which keeps the same code path usable from scripts.
pub struct EchoGuard {
    handle: isize,
    mode: u32,
    registered: bool,
}

impl Drop for EchoGuard {
    fn drop(&mut self) {
        unsafe {
            SetConsoleMode(self.handle, self.mode);
            if self.registered {
                SetConsoleCtrlHandler(Some(restore_echo_on_console_event), 0);
            }
        }
        ORIGINAL_HANDLE.store(0, Ordering::SeqCst);
        ORIGINAL_MODE.store(0, Ordering::SeqCst);
    }
}

pub fn disable_echo() -> Option<EchoGuard> {
    unsafe {
        let handle = GetStdHandle(STD_INPUT_HANDLE);
        if handle == 0 || handle == -1isize {
            return None;
        }
        let mut mode = 0u32;
        if GetConsoleMode(handle, &mut mode) == 0 {
            return None;
        }
        if mode & ENABLE_ECHO_INPUT == 0 {
            return Some(EchoGuard {
                handle,
                mode,
                registered: false,
            });
        }
        if SetConsoleMode(handle, mode & !ENABLE_ECHO_INPUT) == 0 {
            return None;
        }
        ORIGINAL_HANDLE.store(handle, Ordering::SeqCst);
        ORIGINAL_MODE.store(mode, Ordering::SeqCst);
        let registered = SetConsoleCtrlHandler(Some(restore_echo_on_console_event), 1) != 0;
        Some(EchoGuard {
            handle,
            mode,
            registered,
        })
    }
}

pub fn stdin_is_console() -> bool {
    unsafe {
        let handle = GetStdHandle(STD_INPUT_HANDLE);
        if handle == 0 || handle == -1isize {
            return false;
        }
        let mut mode = 0u32;
        GetConsoleMode(handle, &mut mode) != 0
    }
}
