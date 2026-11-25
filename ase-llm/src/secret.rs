//! The API key value type. It is only reachable through an explicit
//! `expose()` accessor and is never `Display`, `Serialize` or usefully
//! `Debug`-printed, so a key cannot reach argv handling, JSON status, logs or
//! an error payload by accident.

use ase_core::{err, Result};
use std::fmt;

/// A nonempty, printable-ASCII secret with whitespace and control characters
/// rejected (those always mean a pasted newline or a corrupt source).
pub struct ApiKey(String);

impl ApiKey {
    /// Validates and takes ownership of a freshly read secret.
    pub fn parse(raw: &str) -> Result<Self> {
        Self::from_owned(raw.to_string())
    }

    pub(crate) fn from_owned(raw: String) -> Result<Self> {
        if raw.is_empty() {
            return Err(err("api_key_empty"));
        }
        if raw.len() > 8192 {
            return Err(err("api_key_too_long"));
        }
        if raw
            .bytes()
            .any(|byte| !(0x21..=0x7e).contains(&byte))
        {
            return Err(err(
                "api_key_must_be_printable_ascii_without_whitespace",
            ));
        }
        Ok(ApiKey(raw))
    }

    /// The only way to read the secret. Callers must not log or persist the
    /// returned value in plaintext.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ApiKey(<redacted>)")
    }
}

impl Drop for ApiKey {
    fn drop(&mut self) {
        // Best-effort in-process scrub. The OS credential store and DPAPI
        // blob remain the authoritative at-rest copies.
        unsafe {
            for byte in self.0.as_bytes_mut() {
                std::ptr::write_volatile(byte, 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_validation_and_redaction() {
        assert!(ApiKey::parse("").is_err());
        assert!(ApiKey::parse(" sk-secret").is_err());
        assert!(ApiKey::parse("sk\nsecret").is_err());
        let key = ApiKey::parse("sk-synthetic-0123456789").unwrap();
        assert_eq!(key.expose(), "sk-synthetic-0123456789");
        assert_eq!(format!("{key:?}"), "ApiKey(<redacted>)");
        assert!(!format!("{key:?}").contains("synthetic"));
    }
}
