//! CLI argument access shared by command modules.
//!
//! All command groups read flags and positionals through these helpers so
//! precedence and defaults stay identical.

use ase_core::{err, Result};

pub(crate) fn parse_opt<T: std::str::FromStr>(args: &[String], name: &str) -> Result<Option<T>> {
    match opt(args, name) {
        Some(value) => value
            .parse()
            .map(Some)
            .map_err(|_| err(format!("{name} needs a number, got {value}"))),
        None => Ok(None),
    }
}

pub(crate) fn positional(args: &[String]) -> Option<String> {
    let mut expecting = false;
    for a in args {
        if a.starts_with("--") {
            expecting = true;
            continue;
        }
        if !expecting {
            return Some(a.clone());
        }
        expecting = false;
    }
    None
}

pub(crate) fn positionals(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut expecting = false;
    for a in args {
        if a.starts_with("--") {
            expecting = true;
            continue;
        }
        if expecting {
            expecting = false;
            continue;
        }
        out.push(a.clone());
    }
    out
}

pub(crate) fn opt(args: &[String], name: &str) -> Option<String> {
    for (i, a) in args.iter().enumerate() {
        if a == name {
            return args.get(i + 1).cloned();
        }
    }
    None
}

pub(crate) fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}
