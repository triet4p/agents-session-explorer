//! Address-only index: file/ordinal/offset/len plus verbatim source
//! metadata. No postings, no text, no model output (INITIALIZATION.md §7.2).

pub mod builder;
pub mod format;
pub mod raw;
pub mod reader;

pub use builder::{build, decode_corpus, write_address, BuildStats, DecodedCorpus};
pub use builder::sha256_hex;
pub use raw::FsRawStore;
pub use reader::{EventView, FileView, Index, SessionView};
