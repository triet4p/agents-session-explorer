//! Work reconstruction: leaf-aware OMP Turn/Episode projection with
//! reopenable evidence.
//!
//! The projection is derived from exact source bytes addressed by the ASE
//! address index. It is task-agnostic and never writes resolver or motif
//! membership. `project` produces the selected branch and episodes;
//! `reopen` re-reads every evidence span and recomputes the projection;
//! `compare` aligns product rows with the independent reference by exact
//! file digest and reports per-journal differences before aggregation.
//! `analytics` aggregates the stored runtime facts from that same projection
//! into nominal-cost, duration, clock-coverage and latency reports without
//! repricing, resolver membership or a `stats.db` dependency.

pub mod compare;
pub mod analytics;
pub mod delta;
pub mod model;
pub mod project;
pub mod reopen;
pub mod util;

#[cfg(test)]
mod tests;

pub use compare::{compare, Comparison, ReferenceRow};
pub use analytics::{
    aggregate, reconciled, AnalyticsBucket, AnalyticsContract, AnalyticsReport, BucketStats,
    Coverage, JournalAnalytics, Reconciliation, WindowJson, ANALYTICS_RULE, ANALYTICS_VERSION,
    CLOCK_END_FIELD, CLOCK_START_FIELD, COST_FIELD, DURATION_FIELD, LATENCY_FIELD,
    PERIOD_FIELD, PROJECT_PROVENANCE, UNKNOWN_MODEL, UNKNOWN_PROJECT,
};
pub use delta::{
    attempt_delta, order_attempts, AttemptDelta, AttemptOrder, AttemptOrderKey, AttemptSide,
    BoundaryChange, ChangeKind, CountDeltaEntry, DeltaAttemptRef, OrderedAttempt, TurnChange,
    DELTA_RULE, DELTA_VERSION, MEMBERSHIP_RULE, MEMBERSHIP_VERSION, ORDER_BASIS,
};
pub use model::*;
pub use project::{project_bytes, ProjectOptions};
pub use reopen::{reopen_bytes, ReopenReport};
pub use util::{
    canonical_manifest_entry, filename_timestamp, format_iso_ms, layout, manifest_digest,
    parse_ts_opt, relpath, sha256_hex, source_id,
};
