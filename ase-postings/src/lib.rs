//! P0b derived layer: contentless SQLite FTS5 prose postings and CallSig
//! shingles pinned to one published address generation.
//!
//! This crate never imports a model, network or renderer. It reads metadata
//! from the pinned binary address and reopens evidence only through raw bytes.

pub mod publication;
pub mod reader;
pub mod writer;

pub use publication::{open, Publication, Unavailable};
pub use reader::{block_by_selector, search, shingles_by_event, shingles_by_token, SearchHit};
pub use writer::{write_postings, WriterInput, WriterStats};
