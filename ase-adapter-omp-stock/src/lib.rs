//! OMP stock adapter: advisory parent-child, workspace and task claims over
//! unmodified OMP journals. It reads through a pinned `StockView` and never
//! writes resolver or motif membership.

mod claims;
mod journal;
mod link;
mod provenance;
pub mod extract;
pub mod model;
pub mod reopen;
pub mod scan;

pub use extract::{
    extract_path_mentions, extract_spawns, extract_task_id_markers, extract_task_tokens,
};
pub use model::*;
pub use reopen::reopen;
pub use scan::{scan, Options};

#[cfg(test)]
mod tests;
