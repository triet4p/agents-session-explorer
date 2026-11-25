//! Hidden one-time key input. The prompt goes to stderr and the key is read
//! from stdin only: never from argv, never from a config file, never echoed to
//! stdout. On a Windows console echo is disabled for the read (and restored by
//! a guard plus a console control handler); a piped stdin is read without a
//! terminal echo by definition.

use ase_core::{err, Result};
use std::io::{self, Write};

use crate::secret::ApiKey;

pub const KEY_PROMPT: &str = "API key (input hidden): ";

pub fn read_secret_line(prompt: &str) -> Result<ApiKey> {
    let _ = write_prompt(prompt);
    #[cfg(windows)]
    let guard = crate::win::disable_echo();

    let mut line = String::new();
    let read = io::stdin().read_line(&mut line);
    #[cfg(windows)]
    let newline_needed = guard.is_some() && crate::win::stdin_is_console();
    #[cfg(windows)]
    drop(guard);

    match read {
        Ok(0) => {
            zero_string(&mut line);
            return Err(err("api_key_prompt_empty"));
        }
        Ok(_) => {}
        Err(error) => {
            zero_string(&mut line);
            return Err(err(format!("api_key_prompt_read_failed: {error}")));
        }
    }
    #[cfg(windows)]
    if newline_needed {
        eprintln!();
    }
    let value = line.trim_end_matches(['\r', '\n']).to_string();
    let key = ApiKey::parse(&value);
    let mut value = value;
    zero_string(&mut value);
    zero_string(&mut line);
    key
}

fn write_prompt(prompt: &str) -> io::Result<()> {
    let mut stderr = io::stderr();
    stderr.write_all(prompt.as_bytes())?;
    stderr.flush()
}

fn zero_string(value: &mut String) {
    unsafe {
        for byte in value.as_bytes_mut() {
            std::ptr::write_volatile(byte, 0);
        }
    }
}

#[cfg(windows)]
pub fn stdin_is_console() -> bool {
    crate::win::stdin_is_console()
}

#[cfg(not(windows))]
pub fn stdin_is_console() -> bool {
    false
}
