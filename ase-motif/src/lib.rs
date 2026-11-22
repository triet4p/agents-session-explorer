//! Motif evidence: CallSig sequences, deterministic families, bounded
//! alignment, task-level support and a permutation null model.
//!
//! Each step refuses to invent evidence:
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
//! 4. [`align::align`] aligns a sequence to a motif skeleton while keeping
//!    insertions and substitutions as separate observations.
//! 5. [`motif::detect`] dedupes duplicate sequence text within one task key
//!    (`motif-sequence-text` / `seq-text-v1`), counts support per task across
//!    attempts, records the denominator and exclusions, and reports a
//!    fixed-seed permutation null (lift and z) over each motif's reopenable
//!    occurrences.
//!
//! There is no model on any path.

pub mod align;
pub mod family;
pub mod membership;
pub mod motif;
pub mod projection;

pub use align::{align, AlignConfig, AlignOp, Alignment};
pub use family::{families, Family, FamilyConfig, FamilyReport, RULE_NAME, RULE_VERSION};
pub use membership::{
    membership, membership_scoped, AttemptMembership, TaskMembership,
};
pub use motif::{
    detect, DeduplicatedSequence, Motif, MotifConfig, MotifReport, NullModel, Occurrence,
    OccurrenceCall, SequenceRef, TaskExclusion, TaskExclusionReason, MOTIF_RULE_NAME,
    MOTIF_RULE_VERSION, NULL_SEED, TEXT_HASH_RULE_NAME, TEXT_HASH_RULE_VERSION,
};
pub use projection::{
    project, reopen_call, AttemptSequences, CallEvidence, Sequence, TaskSequences,
};
