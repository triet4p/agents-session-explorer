//! Default session-root resolution.
//!
//! Roots are explicit arguments first, then environment overrides, then the
//! current user's home directory. No account or machine path is compiled in.

use std::path::PathBuf;

const OMP_ROOT_ENV: &str = "ASE_OMP_ROOT";
const CODEX_ROOT_ENV: &str = "ASE_CODEX_ROOT";
const OMP_HOME_SUFFIX: &str = ".omp/agent/sessions";
const CODEX_HOME_SUFFIX: &str = ".codex/sessions";

pub fn omp_root() -> String {
    root(OMP_ROOT_ENV, OMP_HOME_SUFFIX)
}

pub fn codex_root() -> String {
    root(CODEX_ROOT_ENV, CODEX_HOME_SUFFIX)
}

fn root(env_key: &str, home_suffix: &str) -> String {
    if let Some(value) = std::env::var_os(env_key) {
        if !value.is_empty() {
            return PathBuf::from(value).to_string_lossy().into_owned();
        }
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    match home {
        Some(home) => PathBuf::from(home)
            .join(home_suffix)
            .to_string_lossy()
            .into_owned(),
        None => home_suffix.to_string(),
    }
}
