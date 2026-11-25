//! Runtime analytics over the locked Turn/Episode projection.
//!
//! This layer does not re-decide leaf membership, re-price a cost, or read
//! `stats.db`. It consumes the same indexed bytes and the same turn/leaf
//! selection, then reports nominal stored cost, duration, clock-interval
//! union and time-to-first-token with explicit coverage. A missing producer
//! field stays missing: it is counted in coverage and excluded from sums,
//! never converted into a zero amount or a guessed project.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::model::*;
use crate::util::format_iso_ms;

pub const ANALYTICS_RULE: &str = "runtime-analytics";
pub const ANALYTICS_VERSION: &str = "runtime-analytics-v1";
pub const UNKNOWN_PROJECT: &str = "<unknown-project>";
pub const UNKNOWN_MODEL: &str = "<unknown-model>";
pub const PERIOD_FIELD: &str = "message.timestamp";
pub const COST_FIELD: &str = "message.usage.cost.total";
pub const DURATION_FIELD: &str = "message.duration";
pub const LATENCY_FIELD: &str = "message.ttft";
pub const CLOCK_START_FIELD: &str = "message.timestamp";
pub const CLOCK_END_FIELD: &str = "message.completedAt";
pub const PROJECT_PROVENANCE: &str = "tree-layout:<project>/<name>.jsonl";
const AMOUNT_EPSILON: f64 = 1e-9;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowJson {
    pub start: String,
    pub end: String,
    pub bounds: String,
    pub timezone: String,
}

impl WindowJson {
    pub fn from_window(window: Option<Window>) -> Option<Self> {
        window.map(|w| Self {
            start: format_iso_ms(w.start_ms),
            end: format_iso_ms(w.end_ms),
            bounds: "half-open".to_string(),
            timezone: "UTC".to_string(),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    pub turns: u64,
    pub model_present: u64,
    pub cost_present: u64,
    pub duration_present: u64,
    pub ttft_present: u64,
    pub clock_present: u64,
    pub clock_invalid: u64,
    pub period_in: u64,
    pub period_out: u64,
    pub period_missing: u64,
    pub period_unscoped: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BucketStats {
    pub turns: u64,
    /// Sum of stored nominal amounts over turns where the amount is present.
    pub cost_usd: f64,
    /// Sum of stored call durations over turns where duration is present.
    pub duration_ms: f64,
    /// Length of the union of `[timestamp, completedAt)` intervals.
    pub clock_union_ms: u64,
    /// Sum of stored time-to-first-token over turns where ttft is present.
    pub ttft_ms: f64,
    pub coverage: Coverage,
}

/// Half-open, `message.timestamp`-scoped sums. `BucketStats` above stays the
/// complete-branch view used by the reconciliation; this view excludes
/// turns outside the period window so a scoped report is a period figure.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PeriodBucket {
    pub turns: u64,
    pub cost_usd: f64,
    pub duration_ms: f64,
    pub ttft_ms: f64,
    pub clock_union_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PeriodTotals {
    pub all: PeriodBucket,
    pub leaf: PeriodBucket,
    pub abandoned: PeriodBucket,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnalyticsBucket {
    pub model: String,
    pub project: String,
    pub leaf: BucketStats,
    pub all: BucketStats,
    pub abandoned: BucketStats,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Reconciliation {
    pub all_turns: u64,
    pub leaf_turns: u64,
    pub abandoned_turns: u64,
    pub p1_all_turns: u64,
    pub p1_leaf_turns: u64,
    pub turns_match_p1: bool,
    pub all_cost_usd: f64,
    pub leaf_cost_usd: f64,
    pub abandoned_cost_usd: f64,
    pub cost_residual_usd: f64,
    pub cost_reconciled: bool,
    pub all_duration_ms: f64,
    pub leaf_duration_ms: f64,
    pub abandoned_duration_ms: f64,
    pub duration_residual_ms: f64,
    pub duration_reconciled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnalyticsContract {
    pub consumer: String,
    pub unit: String,
    pub stored_amount_only: bool,
    pub repricing: String,
    pub stats_db_dependency: String,
    pub branch_rule: String,
    pub duration_vs_clock: String,
    pub missing_value_policy: String,
    pub reopen: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnalyticsReport {
    pub rule: String,
    pub rule_version: String,
    pub generation: String,
    pub window: Option<WindowJson>,
    pub period_field: String,
    pub cost_field: String,
    pub duration_field: String,
    pub latency_field: String,
    pub clock_fields: [String; 2],
    pub project_provenance: String,
    pub unknown_project_group: String,
    pub unknown_model_group: String,
    pub files: u64,
    pub buckets: Vec<AnalyticsBucket>,
    pub totals: BucketStats,
    pub period: Option<PeriodTotals>,
    pub reconciliation: Reconciliation,
    pub contract: AnalyticsContract,
}

/// One reopenable analytics row: the Turn/Episode projection plus explicit project
/// provenance. Runtime facts live in `projection.runtime`, so there is one
/// evidence set and one source of leaf truth.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JournalAnalytics {
    pub rule: String,
    pub rule_version: String,
    pub generation: String,
    pub source_id: String,
    pub project: Option<String>,
    pub project_source: String,
    pub projection: JournalProjection,
}

impl JournalAnalytics {
    pub fn new(projection: JournalProjection, project: Option<String>) -> Self {
        Self {
            rule: ANALYTICS_RULE.to_string(),
            rule_version: ANALYTICS_VERSION.to_string(),
            generation: projection.generation.clone(),
            source_id: projection.source_id.clone(),
            project_source: if project.is_some() {
                PROJECT_PROVENANCE.to_string()
            } else {
                "unavailable".to_string()
            },
            project,
            projection,
        }
    }

    pub fn project_label(&self) -> &str {
        self.project.as_deref().filter(|s| !s.is_empty()).unwrap_or(UNKNOWN_PROJECT)
    }
}

/// Store a millisecond duration as exact fixed-point nanoseconds. A fixed
/// absolute float epsilon cannot reconcile a sum of ~2e9 ms; integer
/// accumulation keeps all = leaf + abandoned exact at corpus scale.
fn duration_ns(ms: f64) -> i128 {
    (ms * 1_000_000.0).round() as i128
}

fn ns_to_ms(ns: i128) -> f64 {
    ns as f64 / 1_000_000.0
}


#[derive(Default)]
struct StatBuild {
    stats: BucketStats,
    intervals: Vec<(i64, i64)>,
    duration_ns: i128,
}

impl StatBuild {
    fn add(&mut self, turn: &RuntimeTurn, window: Option<Window>) {
        self.stats.turns += 1;
        let cov = &mut self.stats.coverage;
        cov.turns += 1;
        if turn.model.as_ref().is_some_and(|m| !m.is_empty()) {
            cov.model_present += 1;
        }
        if let Some(cost) = turn.cost_total_usd {
            if cost.is_finite() {
                self.stats.cost_usd += cost;
                cov.cost_present += 1;
            }
        }
        if let Some(duration) = turn.duration_ms {
            if duration.is_finite() && duration >= 0.0 {
                self.duration_ns += duration_ns(duration);
                cov.duration_present += 1;
            }
        }
        if let Some(ttft) = turn.ttft_ms {
            if ttft.is_finite() && ttft >= 0.0 {
                self.stats.ttft_ms += ttft;
                cov.ttft_present += 1;
            }
        }
        match (turn.msg_ts_ms, turn.completed_ms) {
            (Some(start), Some(end)) if end >= start => {
                self.intervals.push((start, end));
                cov.clock_present += 1;
            }
            (Some(_), Some(_)) => cov.clock_invalid += 1,
            _ => {}
        }
        match (window, turn.msg_ts_ms) {
            (Some(w), Some(ts)) if w.contains(ts) => cov.period_in += 1,
            (Some(_), Some(_)) => cov.period_out += 1,
            (Some(_), None) => cov.period_missing += 1,
            (None, Some(_)) => cov.period_unscoped += 1,
            (None, None) => {
                cov.period_missing += 1;
                cov.period_unscoped += 1;
            }
        }
    }

    fn finish(mut self) -> BucketStats {
        self.stats.clock_union_ms = clock_union(&mut self.intervals);
        self.stats.duration_ms = ns_to_ms(self.duration_ns);
        self.stats
    }
}

/// Merge half-open clock intervals so overlapping parallel calls are counted
/// once. Missing endpoints are excluded upstream and reported in coverage.
fn clock_union(intervals: &mut Vec<(i64, i64)>) -> u64 {
    intervals.sort_unstable_by_key(|interval| interval.0);
    let mut total: i64 = 0;
    let mut current: Option<(i64, i64)> = None;
    for &(start, end) in intervals.iter() {
        match current {
            None => current = Some((start, end)),
            Some((cur_start, cur_end)) if start <= cur_end => {
                current = Some((cur_start, cur_end.max(end)));
            }
            Some((cur_start, cur_end)) => {
                total += cur_end - cur_start;
                current = Some((start, end));
            }
        }
    }
    if let Some((start, end)) = current {
        total += end - start;
    }
    total.max(0) as u64
}

#[derive(Default)]
struct GroupBuild {
    leaf: StatBuild,
    all: StatBuild,
    abandoned: StatBuild,
}

/// Aggregate reopenable rows into the report consumed by T3.
pub fn aggregate(rows: &[JournalAnalytics]) -> AnalyticsReport {
    let generation = rows
        .first()
        .map(|row| row.generation.clone())
        .unwrap_or_default();
    let window = rows.first().and_then(|row| row.projection.window);
    let mut groups: BTreeMap<(String, String), GroupBuild> = BTreeMap::new();
    let mut all_total = StatBuild::default();
    let mut leaf_total = StatBuild::default();
    let mut abandoned_total = StatBuild::default();
    let mut recon = Reconciliation::default();
    let mut period_all = StatBuild::default();
    let mut period_leaf = StatBuild::default();
    let mut period_abandoned = StatBuild::default();
    let mut all_duration_ns = 0i128;
    let mut leaf_duration_ns = 0i128;
    let mut abandoned_duration_ns = 0i128;

    for row in rows {
        let project = row.project_label().to_string();
        recon.p1_all_turns += row.projection.counts.turns_all_branches as u64;
        recon.p1_leaf_turns += row.projection.counts.turns_active_leaf as u64;
        for turn in &row.projection.runtime {
            let model = turn
                .model
                .as_deref()
                .filter(|m| !m.is_empty())
                .unwrap_or(UNKNOWN_MODEL)
                .to_string();
            let build = groups.entry((model, project.clone())).or_default();
            build.all.add(turn, window);
            all_total.add(turn, window);
            let in_period =
                window.is_some_and(|w| turn.msg_ts_ms.is_some_and(|ts| w.contains(ts)));
            if in_period {
                period_all.add(turn, window);
            }
            if turn.on_active_leaf {
                build.leaf.add(turn, window);
                leaf_total.add(turn, window);
                if in_period {
                    period_leaf.add(turn, window);
                }
            } else {
                build.abandoned.add(turn, window);
                abandoned_total.add(turn, window);
                if in_period {
                    period_abandoned.add(turn, window);
                }
            }
            recon.all_turns += 1;
            if let Some(cost) = turn.cost_total_usd.filter(|v| v.is_finite()) {
                recon.all_cost_usd += cost;
                if turn.on_active_leaf {
                    recon.leaf_cost_usd += cost;
                } else {
                    recon.abandoned_cost_usd += cost;
                }
            }
            if let Some(duration) = turn.duration_ms.filter(|v| v.is_finite() && *v >= 0.0) {
                let ns = duration_ns(duration);
                all_duration_ns += ns;
                if turn.on_active_leaf {
                    leaf_duration_ns += ns;
                } else {
                    abandoned_duration_ns += ns;
                }
            }
        }
    }

    let buckets: Vec<AnalyticsBucket> = groups
        .into_iter()
        .map(|((model, project), build)| AnalyticsBucket {
            model,
            project,
            leaf: build.leaf.finish(),
            all: build.all.finish(),
            abandoned: build.abandoned.finish(),
        })
        .collect();

    recon.leaf_turns = leaf_total.stats.coverage.turns;
    recon.abandoned_turns = abandoned_total.stats.coverage.turns;
    recon.turns_match_p1 = recon.all_turns == recon.p1_all_turns
        && recon.leaf_turns == recon.p1_leaf_turns
        && recon.all_turns == recon.leaf_turns + recon.abandoned_turns;
    recon.cost_residual_usd = recon.all_cost_usd - recon.leaf_cost_usd - recon.abandoned_cost_usd;
    recon.all_duration_ms = ns_to_ms(all_duration_ns);
    recon.leaf_duration_ms = ns_to_ms(leaf_duration_ns);
    recon.abandoned_duration_ms = ns_to_ms(abandoned_duration_ns);
    let duration_residual_ns = all_duration_ns - leaf_duration_ns - abandoned_duration_ns;
    recon.duration_residual_ms = ns_to_ms(duration_residual_ns);
    recon.cost_reconciled = recon.cost_residual_usd.abs() <= AMOUNT_EPSILON;
    // Exact fixed-point residual: zero means every duration-bearing turn is
    // accounted for in exactly one branch; a real imbalance stays visible.
    recon.duration_reconciled = duration_residual_ns == 0;

    let all_stats = all_total.finish();
    let totals = BucketStats {
        turns: all_stats.turns,
        cost_usd: all_stats.cost_usd,
        duration_ms: all_stats.duration_ms,
        clock_union_ms: all_stats.clock_union_ms,
        ttft_ms: all_stats.ttft_ms,
        coverage: all_stats.coverage,
    };
    let period = window.map(|_| {
        let to_bucket = |build: StatBuild| {
            let stats = build.finish();
            PeriodBucket {
                turns: stats.turns,
                cost_usd: stats.cost_usd,
                duration_ms: stats.duration_ms,
                ttft_ms: stats.ttft_ms,
                clock_union_ms: stats.clock_union_ms,
            }
        };
        PeriodTotals {
            all: to_bucket(period_all),
            leaf: to_bucket(period_leaf),
            abandoned: to_bucket(period_abandoned),
        }
    });

    AnalyticsReport {
        rule: ANALYTICS_RULE.to_string(),
        rule_version: ANALYTICS_VERSION.to_string(),
        generation,
        window: WindowJson::from_window(window),
        period_field: PERIOD_FIELD.to_string(),
        cost_field: COST_FIELD.to_string(),
        duration_field: DURATION_FIELD.to_string(),
        latency_field: LATENCY_FIELD.to_string(),
        clock_fields: [CLOCK_START_FIELD.to_string(), CLOCK_END_FIELD.to_string()],
        project_provenance: PROJECT_PROVENANCE.to_string(),
        unknown_project_group: UNKNOWN_PROJECT.to_string(),
        unknown_model_group: UNKNOWN_MODEL.to_string(),
        files: rows.len() as u64,
        buckets,
        totals,
        period,
        reconciliation: recon,
        contract: AnalyticsContract {
            consumer: "ase-analytics".to_string(),
            unit: "one assistant message carrying usage = one API call".to_string(),
            stored_amount_only: true,
            repricing: "none; nominal producer-catalog amount read from usage.cost.total".to_string(),
            stats_db_dependency: "none".to_string(),
            branch_rule: "active leaf per the released compatibility rule; all and abandoned reported beside leaf".to_string(),
            duration_vs_clock:
                "duration = exact fixed-point sum of message.duration in ns; clock = union of [message.timestamp, message.completedAt)".to_string(),
            missing_value_policy:
                "missing cost/duration/ttft/clock/model are counted in coverage and excluded from sums; never zero-filled".to_string(),
            reopen: "runtime facts verify against the same generation-pinned turn evidence spans".to_string(),
        },
    }
}

/// True when every branch reconciliation invariant holds.
pub fn reconciled(report: &AnalyticsReport) -> bool {
    let r = &report.reconciliation;
    r.turns_match_p1 && r.cost_reconciled && r.duration_reconciled
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{project_bytes, reopen_bytes, sha256_hex, source_id, ProjectOptions, Window};


    fn project(name: &str, bytes: &[u8], window: Option<Window>) -> JournalProjection {
        let sha = sha256_hex(bytes);
        let id = source_id(name);
        let opts = ProjectOptions {
            file_id: 0,
            path: name,
            source_id: &id,
            generation: "gen-analytics",
            sha256: &sha,
            event_base: 0,
            explicit_leaf: None,
            window,
        };
        project_bytes(&opts, bytes).expect("synthetic journal projects")
    }

    fn row(name: &str, bytes: &[u8], project_name: Option<&str>, window: Option<Window>) -> JournalAnalytics {
        JournalAnalytics::new(project(name, bytes, window), project_name.map(str::to_string))
    }

    fn head(id: &str, ts: &str) -> String {
        format!(
            "{{\"type\":\"session\",\"version\":3,\"id\":\"{id}\",\"timestamp\":\"{ts}\",\"cwd\":\"C:\\\\SYNTHETIC\\\\analytics\",\"title\":\"synthetic runtime analytics fixture\"}}\n"
        )
    }

    fn user(id: &str, parent: Option<&str>, ts: &str) -> String {
        let parent = parent
            .map(|p| format!(",\"parentId\":\"{p}\""))
            .unwrap_or_default();
        format!("{{\"type\":\"message\",\"id\":\"{id}\"{parent},\"timestamp\":\"{ts}\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"synthetic\"}}]}}}}\n")
    }

    /// One assistant turn. `usage: None` omits the object entirely so it is
    /// not a turn; other `None` values omit only that producer field.
    #[allow(clippy::too_many_arguments)]
    fn turn(
        id: &str,
        parent: &str,
        entry_ts: &str,
        model: Option<&str>,
        msg_ts: Option<i64>,
        completed: Option<i64>,
        duration: Option<f64>,
        ttft: Option<f64>,
        cost: Option<f64>,
        usage: bool,
    ) -> String {
        let mut message = serde_json::json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "synthetic"}],
            "stopReason": "stop"
        });
        if let Some(model) = model {
            message["model"] = serde_json::json!(model);
        }
        if let Some(ts) = msg_ts {
            message["timestamp"] = serde_json::json!(ts);
        }
        if let Some(ts) = completed {
            message["completedAt"] = serde_json::json!(ts);
        }
        if let Some(value) = duration {
            message["duration"] = serde_json::json!(value);
        }
        if let Some(value) = ttft {
            message["ttft"] = serde_json::json!(value);
        }
        if usage {
            let mut cost_obj = serde_json::json!({
                "input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0
            });
            if let Some(value) = cost {
                cost_obj["total"] = serde_json::json!(value);
            }
            message["usage"] = serde_json::json!({
                "input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0,
                "totalTokens": 2, "cost": cost_obj
            });
        }
        format!("{{\"type\":\"message\",\"id\":\"{id}\",\"parentId\":\"{parent}\",\"timestamp\":\"{entry_ts}\",\"message\":{message}}}\n")
    }

    const OCT: &str = "2026-10-01T00:00:00.000Z";
    const NOV: &str = "2026-11-01T00:00:00.000Z";
    fn oct_window() -> Window {
        Window { start_ms: 1_790_812_800_000, end_ms: 1_793_491_200_000 }
    }

    #[test]
    fn parallel_calls_count_clock_once_but_duration_twice() {
        let mut journal = head("parallel", "2026-10-05T00:00:00.000Z");
        journal.push_str(&user("u1", None, "2026-10-05T00:00:01.000Z"));
        // [2000, 4000) and [2500, 4500) overlap for 2500 ms of clock.
        journal.push_str(&turn("a1", "u1", "2026-10-05T00:00:02.000Z", Some("model-a"),
            Some(1_791_158_402_000), Some(1_791_158_404_000), Some(2000.0), Some(500.0), Some(0.01), true));
        journal.push_str(&turn("a2", "a1", "2026-10-05T00:00:03.000Z", Some("model-a"),
            Some(1_791_158_402_500), Some(1_791_158_404_500), Some(2000.0), Some(700.0), Some(0.02), true));
        let bytes = journal.as_bytes();
        let report = aggregate(&[row("p/parallel.jsonl", bytes, Some("p"), Some(oct_window()))]);
        assert_eq!(report.totals.turns, 2);
        assert_eq!(report.totals.duration_ms, 4000.0);
        assert_eq!(report.totals.clock_union_ms, 2500);
        assert_eq!(report.totals.ttft_ms, 1200.0);
        assert!((report.totals.cost_usd - 0.03).abs() < AMOUNT_EPSILON);
        assert_eq!(report.totals.coverage.clock_present, 2);
        assert_eq!(report.totals.coverage.cost_present, 2);
        assert!(reconciled(&report));
    }

    #[test]
    fn active_leaf_and_all_branch_are_reported_separately() {
        let mut journal = head("branch", "2026-10-02T00:00:00.000Z");
        journal.push_str(&user("u1", None, "2026-10-02T00:00:01.000Z"));
        journal.push_str(&turn("a1", "u1", "2026-10-02T00:00:02.000Z", Some("model-b"),
            Some(1_790_899_202_000), Some(1_790_899_203_000), Some(1000.0), Some(100.0), Some(0.01), true));
        // Two leaves: the later entry timestamp wins the active-leaf rule.
        journal.push_str(&turn("a2a", "a1", "2026-10-02T00:00:03.000Z", Some("model-b"),
            Some(1_790_899_203_000), Some(1_790_899_204_000), Some(1000.0), Some(100.0), Some(0.10), true));
        journal.push_str(&turn("a2b", "a1", "2026-10-02T00:00:04.000Z", Some("model-b"),
            Some(1_790_899_204_000), Some(1_790_899_205_000), Some(1000.0), Some(100.0), Some(0.20), true));
        let bytes = journal.as_bytes();
        let projection = project("p/branch.jsonl", bytes, Some(oct_window()));
        assert_eq!(projection.active_leaf.entry_id.as_deref(), Some("a2b"));
        let report = aggregate(&[JournalAnalytics::new(projection, Some("p".to_string()))]);
        let r = &report.reconciliation;
        assert_eq!(r.all_turns, 3);
        assert_eq!(r.leaf_turns, 2);
        assert_eq!(r.abandoned_turns, 1);
        assert!(r.turns_match_p1);
        assert!((r.all_cost_usd - 0.31).abs() < AMOUNT_EPSILON);
        assert!((r.leaf_cost_usd - 0.21).abs() < AMOUNT_EPSILON);
        assert!((r.abandoned_cost_usd - 0.10).abs() < AMOUNT_EPSILON);
        assert!(r.cost_reconciled && r.duration_reconciled);
        assert_eq!(report.buckets.len(), 1);
        assert_eq!(report.buckets[0].all.coverage.turns, 3);
        assert_eq!(report.buckets[0].leaf.coverage.turns, 2);
        assert_eq!(report.buckets[0].abandoned.coverage.turns, 1);
    }

    #[test]
    fn period_is_half_open_on_message_timestamp() {
        let mut journal = head("window", "2026-10-01T00:00:00.000Z");
        journal.push_str(&user("u1", None, OCT));
        journal.push_str(&turn("before", "u1", "2026-09-30T23:59:59.000Z", Some("model-c"),
            Some(1_790_812_799_000), Some(1_790_812_799_500), Some(500.0), Some(50.0), Some(0.01), true));
        journal.push_str(&turn("inside", "before", OCT, Some("model-c"),
            Some(1_790_812_800_000), Some(1_790_812_800_500), Some(500.0), Some(50.0), Some(0.02), true));
        journal.push_str(&turn("at-end", "inside", NOV, Some("model-c"),
            Some(1_793_491_200_000), Some(1_793_491_200_500), Some(500.0), Some(50.0), Some(0.04), true));
        let bytes = journal.as_bytes();
        let report = aggregate(&[row("p/window.jsonl", bytes, Some("p"), Some(oct_window()))]);
        let cov = &report.totals.coverage;
        assert_eq!(cov.period_in, 1);
        assert_eq!(cov.period_out, 2);
        assert_eq!(cov.period_missing, 0);
        assert_eq!(report.window.unwrap().bounds, "half-open");
        // Complete-branch totals stay unscoped; the period view excludes the
        // two out-of-window turns on the contracted message.timestamp field.
        assert_eq!(report.totals.turns, 3);
        let period = report.period.as_ref().expect("scoped report carries period totals");
        assert_eq!(period.all.turns, 1);
        assert!((period.all.cost_usd - 0.02).abs() < AMOUNT_EPSILON);
        assert_eq!(period.leaf.turns, 1);
        assert_eq!(period.abandoned.turns, 0);
    }

    #[test]
    fn duration_reconciles_exactly_at_corpus_scale() {
        let mut journal = head("scale", "2026-10-02T00:00:00.000Z");
        journal.push_str(&user("u1", None, "2026-10-02T00:00:01.000Z"));
        journal.push_str(&turn("a1", "u1", "2026-10-02T00:00:02.000Z", Some("model-s"),
            Some(1_790_899_202_000), Some(1_790_899_203_000), Some(1000.0), Some(100.0), Some(0.01), true));
        // Abandoned branch with a fractional duration, plus a leaf branch with
        // a corpus-scale fractional duration. A fixed 1e-9 float epsilon
        // falsely fails this balance; fixed-point accumulation is exact.
        journal.push_str(&turn("a2a", "a1", "2026-10-02T00:00:03.000Z", Some("model-s"),
            Some(1_790_899_203_000), Some(1_790_899_204_000), Some(7326.1387), Some(100.0), Some(0.10), true));
        journal.push_str(&turn("a2b", "a1", "2026-10-02T00:00:04.000Z", Some("model-s"),
            Some(1_790_899_204_000), Some(1_790_899_205_000), Some(2_254_356_632.351203), Some(100.0), Some(0.20), true));
        let bytes = journal.as_bytes();
        let report = aggregate(&[row("p/scale.jsonl", bytes, Some("p"), Some(oct_window()))]);
        let r = &report.reconciliation;
        assert!(r.turns_match_p1);
        assert_eq!(r.abandoned_turns, 1);
        assert_eq!(r.duration_residual_ms, 0.0);
        assert!(r.duration_reconciled);
        assert!(r.cost_reconciled);
        assert!((r.abandoned_duration_ms - 7326.1387).abs() < 1e-6);
    }

    #[test]
    fn missing_fields_are_coverage_not_zero_and_unknown_project_is_explicit() {
        let mut journal = head("gaps", "2026-10-03T00:00:00.000Z");
        journal.push_str(&user("u1", None, "2026-10-03T00:00:01.000Z"));
        // Assistant with no usage is not a turn; it lowers coverage only via
        // the projection's assistant_no_usage count, never by pretending a zero cost.
        journal.push_str(&format!("{{\"type\":\"message\",\"id\":\"n1\",\"parentId\":\"u1\",\"timestamp\":\"2026-10-03T00:00:02.000Z\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"synthetic\"}}]}}}}\n"));
        journal.push_str(&turn("c1", "n1", "2026-10-03T00:00:03.000Z", Some("model-d"),
            Some(1_790_983_203_000), None, None, None, None, true));
        journal.push_str(&turn("c2", "c1", "2026-10-03T00:00:04.000Z", None,
            Some(1_790_983_204_000), Some(1_790_983_205_000), Some(1000.0), Some(300.0), Some(0.05), true));
        let bytes = journal.as_bytes();
        let projection = project("gaps.jsonl", bytes, Some(oct_window()));
        assert_eq!(projection.counts.assistant_no_usage, 1);
        let report = aggregate(&[JournalAnalytics::new(projection, None)]);
        assert_eq!(report.totals.turns, 2);
        assert_eq!(report.totals.coverage.cost_present, 1);
        assert_eq!(report.totals.coverage.clock_present, 1);
        assert_eq!(report.totals.coverage.duration_present, 1);
        assert_eq!(report.totals.coverage.ttft_present, 1);
        assert_eq!(report.totals.coverage.model_present, 1);
        assert!((report.totals.cost_usd - 0.05).abs() < AMOUNT_EPSILON);
        assert_eq!(report.buckets.len(), 2);
        assert!(report.buckets.iter().all(|b| b.project == UNKNOWN_PROJECT));
        assert!(report.buckets.iter().any(|b| b.model == UNKNOWN_MODEL));
    }

    #[test]
    fn reopen_verifies_runtime_and_rejects_tampered_stored_amount() {
        let mut journal = head("reopen", "2026-10-04T00:00:00.000Z");
        journal.push_str(&user("u1", None, "2026-10-04T00:00:01.000Z"));
        journal.push_str(&turn("a1", "u1", "2026-10-04T00:00:02.000Z", Some("model-e"),
            Some(1_791_074_402_000), Some(1_791_074_403_000), Some(1000.0), Some(200.0), Some(0.07), true));
        let bytes = journal.as_bytes();
        let projection = project("p/reopen.jsonl", bytes, Some(oct_window()));
        let ok = reopen_bytes(&projection, bytes, "gen-analytics");
        assert_eq!(ok.status, "verified", "{:?}", ok.errors);
        assert_eq!(ok.runtime_checked, 1);
        assert_eq!(ok.runtime_verified, 1);

        let mut tampered = projection.clone();
        tampered.runtime[0].cost_total_usd = Some(9.99);
        let bad = reopen_bytes(&tampered, bytes, "gen-analytics");
        assert_eq!(bad.status, "stale");
        assert!(bad.errors.iter().any(|e| e.starts_with("runtime_mismatch")));
    }

}
