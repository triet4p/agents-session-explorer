//! Numeric option parsing shared by command modules.

use ase_core::{err, Result};

pub(crate) fn parse_opt<T: std::str::FromStr>(args: &[String], name: &str) -> Result<Option<T>> {
    match crate::opt(args, name) {
        Some(value) => value
            .parse()
            .map(Some)
            .map_err(|_| err(format!("{name} needs a number, got {value}"))),
        None => Ok(None),
    }
}
