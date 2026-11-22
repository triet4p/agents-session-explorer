//! P0d-1 motif evidence: CallSig sequences and deterministic families.
//!
//! Three steps, each refusing to invent evidence:
//!
//! 1. [`membership::membership`] resolves a task token from native
//!    `task_dispatch_v1` / `task_dispatch_child_v1` records only
//!    (`dispatch-v1`). Naming candidates, path matches and `parentSession`
//!    declarations are never inputs, so they cannot widen membership.
//! 2. [`projection::project`] rebuilds sequences from the call blocks of one
//!    pinned generation for the admitted child sessions, keeps per-call
//!    reopenable evidence, and never continues a sequence across a fork or a
//!    session boundary.
//! 3. [`family::families`] groups sequences by Jaccard overlap over
//!    `shingle-k3k5-v1` tokens in a deterministic order.
//!
//! There is no CLI, no alignment and no model in this crate.

pub mod family;
pub mod membership;
pub mod projection;

pub use family::{families, Family, FamilyConfig, FamilyReport, RULE_NAME, RULE_VERSION};
pub use membership::{membership, AttemptMembership, TaskMembership};
pub use projection::{
    project, reopen_call, AttemptSequences, CallEvidence, Sequence, TaskSequences,
};
