//! P0a Source + Decoder implementations for omp JSONL and codex JSONL/zstd.

pub mod codex;
pub mod omp;
pub mod source;
pub mod projection;
pub mod util;

pub use codex::CodexDecoder;
pub use omp::OmpDecoder;
pub use source::{discover_all, CodexSource, OmpSource};
pub use ase_core::{BlockProjection, ProjectionKind};
pub use projection::extract_block;
use ase_core::{err, Encoding, Result};
use std::fs;
pub use codex::decode_codex;
pub use omp::decode_omp;

/// Read raw bytes in *file* coordinates. zstd bytes are decompressed here so
/// that every downstream decoder sees a plain JSONL stream.
pub fn read_source_bytes(path: &str, encoding: Encoding) -> Result<Vec<u8>> {
    match encoding {
        Encoding::Jsonl => fs::read(path).map_err(|e| err(format!("read {path}: {e}"))),
        Encoding::JsonlZstd => {
            let file = fs::File::open(path).map_err(|e| err(format!("open {path}: {e}")))?;
            zstd::stream::decode_all(file).map_err(|e| err(format!("zstd decode {path}: {e}")))
        }
    }
}
