use crate::delta::{count_metrics, order_attempts, AttemptDelta, AttemptOrderKey, DeltaAttemptRef};
use super::*;
use crate::model::ManifestEntry;

const BRANCH: &str =
    include_str!("../../artifacts/sprint-4/ASE-S4-T1/fixtures/edge_001_branch_edit.jsonl");
const PARALLEL: &str =
    include_str!("../../artifacts/sprint-4/ASE-S4-T1/fixtures/edge_002_parallel_tools.jsonl");
const NON_USER: &str =
    include_str!("../../artifacts/sprint-4/ASE-S4-T1/fixtures/edge_004_non_user.jsonl");
const EMPTY: &str =
    include_str!("../../artifacts/sprint-4/ASE-S4-T1/fixtures/edge_009_empty_journal.jsonl");
const INCOMPLETE: &str =
    include_str!("../../artifacts/sprint-4/ASE-S4-T1/fixtures/edge_003_incomplete.jsonl");

fn project(name: &str, bytes: &[u8], explicit: Option<&str>) -> JournalProjection {
    let sha = sha256_hex(bytes);
    let id = source_id(name);
    let opts = ProjectOptions {
        file_id: 0,
        path: name,
        source_id: &id,
        generation: "gen-test",
        sha256: &sha,
        event_base: 0,
        explicit_leaf: explicit,
        window: None,
    };
    project_bytes(&opts, bytes).expect("synthetic journal projects")
}

#[test]
fn branch_edit_uses_latest_leaf_and_keeps_abandoned_turn_separate() {
    let p = project("p/a.jsonl", BRANCH.as_bytes(), None);
    assert_eq!(p.active_leaf.entry_id.as_deref(), Some("a2b"));
    assert_eq!(p.active_leaf.ordinal, Some(7));
    assert_eq!(p.counts.turns_active_leaf, 2);
    assert_eq!(p.counts.turns_all_branches, 3);
    assert_eq!(p.counts.abandoned_turns, 1);
    assert_eq!(p.counts.episodes_primary_count, 1);
    assert_eq!(p.counts.episodes_primary_turn_sum, 2);
    assert_eq!(p.turns.iter().map(|t| t.entry_id.as_str()).collect::<Vec<_>>(), vec!["a1", "a2b"]);
}

#[test]
fn explicit_older_leaf_projects_that_branch_and_marks_selection() {
    let p = project("p/a.jsonl", BRANCH.as_bytes(), Some("a2a"));
    assert_eq!(p.active_leaf.entry_id.as_deref(), Some("a2a"));
    assert_eq!(p.active_leaf.selection, "explicit");
    assert!(p.active_leaf.is_graph_leaf);
    assert_eq!(p.turns.iter().map(|t| t.entry_id.as_str()).collect::<Vec<_>>(), vec!["a1", "a2a"]);

    let sliced = project("p/a.jsonl", BRANCH.as_bytes(), Some("a1"));
    assert_eq!(sliced.active_leaf.entry_id.as_deref(), Some("a1"));
    assert!(!sliced.active_leaf.is_graph_leaf);
    assert_eq!(sliced.counts.turns_active_leaf, 1);
}

#[test]
fn parallel_tool_calls_stay_one_turn_per_assistant_cycle() {
    let p = project("p/b.jsonl", PARALLEL.as_bytes(), None);
    assert_eq!(p.active_leaf.entry_id.as_deref(), Some("a2"));
    assert_eq!(p.counts.turns_active_leaf, 2);
    assert_eq!(p.counts.toolcall_blocks, 2);
    assert_eq!(p.counts.toolresult_entries, 2);
    assert_eq!(p.counts.assistant_msgs_with_multiple_calls, 1);
    assert_eq!(p.counts.max_calls_per_assistant, 2);
    assert_eq!(p.turns[0].tool_calls, 2);
    assert_eq!(p.turns[0].outcome, "completed");
}

#[test]
fn episode_boundaries_distinguish_primary_from_user_attributed_custom_message() {
    let p = project("p/c.jsonl", NON_USER.as_bytes(), None);
    assert_eq!(p.active_leaf.entry_id.as_deref(), Some("a2"));
    assert_eq!(p.counts.turns_active_leaf, 2);
    assert_eq!(p.counts.user_messages_primary, 1);
    assert_eq!(p.counts.user_boundaries_wide, 2);
    assert_eq!(p.counts.episodes_primary_count, 1);
    assert_eq!(p.counts.episodes_wide_count, 2);
    assert_eq!(p.counts.episodes_primary_turn_sum, 2);
    assert_eq!(p.counts.episodes_wide_turn_sum, 2);
    assert_eq!(p.episodes_wide[1].boundary_kind, "wide");
    assert_eq!(p.episodes_wide[1].boundary_entry_id.as_deref(), Some("cm1"));
}

#[test]
fn empty_journal_has_no_leaf_and_no_turns() {
    let p = project("p/empty.jsonl", EMPTY.as_bytes(), None);
    assert_eq!(p.active_leaf.entry_id, None);
    assert_eq!(p.counts.turns_active_leaf, 0);
    assert_eq!(p.counts.leaf_count, 0);
}

#[test]
fn failed_and_unknown_stop_outcomes_are_reported_not_dropped() {
    let p = project("p/incomplete.jsonl", INCOMPLETE.as_bytes(), None);
    assert!(p.counts.turns_failed_active_leaf > 0 || p.counts.turns_unknown_stop_active_leaf > 0);
    assert_eq!(
        p.counts.turns_active_leaf,
        p.counts.turns_completed_active_leaf
            + p.counts.turns_failed_active_leaf
            + p.counts.turns_unknown_stop_active_leaf
    );
}

#[test]
fn projection_needs_no_task_key() {
    // The only inputs are journal bytes and addressing. There is deliberately
    // no task/resolver parameter, so unresolved and no-task sessions project.
    let p = project("p/unresolved.jsonl", BRANCH.as_bytes(), None);
    assert_eq!(p.counts.turns_active_leaf, 2);
    assert!(p.turns.iter().all(|t| !t.entry_id.is_empty()));
}

#[test]
fn reopen_verifies_and_reports_stale_after_tamper() {
    let p = project("p/a.jsonl", BRANCH.as_bytes(), None);
    let ok = reopen_bytes(&p, BRANCH.as_bytes(), "gen-test");
    assert_eq!(ok.status, "verified", "{:?}", ok.errors);
    assert!(ok.projection_recomputed);
    assert!(ok.evidence_checked > 0);
    assert_eq!(ok.evidence_checked, ok.evidence_verified);

    let mut tampered = BRANCH.as_bytes().to_vec();
    let needle = b"\"usage\":{\"totalTokens\":1}";
    let at = tampered
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("usage field present");
    tampered[at + 2] = b'x';
    let stale = reopen_bytes(&p, &tampered, "gen-test");
    assert_eq!(stale.status, "stale");
    assert!(stale.errors.iter().any(|e| e.contains("digest_mismatch")));
}

#[test]
fn reopen_refuses_another_generation_before_reading_bytes() {
    let p = project("p/a.jsonl", BRANCH.as_bytes(), None);
    let report = reopen_bytes(&p, BRANCH.as_bytes(), "gen-other");
    assert_eq!(report.status, "generation_mismatch");
    assert!(!report.projection_recomputed);
}

#[test]
fn manifest_entry_json_matches_the_t1_canonical_form() {
    let entry = ManifestEntry {
        relpath: "p/a.jsonl".to_string(),
        size: 12,
        sha256: "d".to_string(),
        head_sha256: "h".to_string(),
        tail_sha256: "t".to_string(),
        kind: "root".to_string(),
        tree_relpath: "p/a.jsonl".to_string(),
        session_id: Some("s".to_string()),
        header_timestamp: Some("2026-09-15T00:00:00.000Z".to_string()),
        parent_session: None,
        filename_timestamp: None,
        header_ok: true,
    };
    assert_eq!(
        canonical_manifest_entry(&entry),
        concat!(
            "{\"filename_timestamp\":null,\"head_sha256\":\"h\",\"header_ok\":true,",
            "\"header_timestamp\":\"2026-09-15T00:00:00.000Z\",\"kind\":\"root\",",
            "\"parent_session\":null,\"relpath\":\"p/a.jsonl\",\"session_id\":\"s\",",
            "\"sha256\":\"d\",\"size\":12,\"tail_sha256\":\"t\",",
            "\"tree_relpath\":\"p/a.jsonl\"}"
        )
    );
    assert_eq!(manifest_digest(&[entry]).len(), 64);
}

#[test]
fn layout_matches_the_locked_tree_rule() {
    let root = layout("proj/2026-09-01T00-00-00-000Z_abc.jsonl").unwrap();
    assert_eq!(root.kind, "root");
    assert_eq!(root.tree_relpath, "proj/2026-09-01T00-00-00-000Z_abc.jsonl");
    let child = layout("proj/2026-09-01T00-00-00-000Z_abc/Worker.jsonl").unwrap();
    assert_eq!(child.kind, "subagent");
    assert_eq!(child.tree_relpath, "proj/2026-09-01T00-00-00-000Z_abc.jsonl");
    assert!(layout("proj/flat-extra/deeper/file.jsonl").is_some());
    assert!(layout("proj/flat-extra/deeper/file.jsonl.gz").is_none());
}

#[test]
fn escaped_parent_session_in_header_is_parsed_like_the_decoder() {
    let line = r#"{"type":"session","version":3,"id":"child-id","timestamp":"2026-09-16T06:00:04.825Z","cwd":"F:\\ai-ml\\demo","parentSession":"C:\\Users\\admin\\.omp\\agent\\sessions\\proj\\root.jsonl"}"#;
    let p = project("proj/root/Worker.jsonl", line.as_bytes(), None);
    assert!(p.header.header_ok);
    assert_eq!(p.header.session_id.as_deref(), Some("child-id"));
    assert_eq!(
        p.header.parent_session.as_deref(),
        Some(r"C:\Users\admin\.omp\agent\sessions\proj\root.jsonl")
    );
}

fn row(source: &str, sha: &str, turns: u32, all: u32, primary: u32, wide: u32) -> JournalRow {
    JournalRow {
        source_id: source.to_string(),
        sha256: sha.to_string(),
        session_id: Some(format!("s-{source}")),
        kind: "root".to_string(),
        tree_id: source.to_string(),
        header_timestamp: None,
        header_ok: true,
        leaf_entry_id: Some(format!("leaf-{source}")),
        leaf_ordinal: Some(0),
        leaf_timestamp: None,
        leaf_type: Some("message".to_string()),
        leaf_count: 1,
        turns_all_branches: all,
        turns_active_leaf: turns,
        abandoned_turns: all.saturating_sub(turns),
        episodes_primary_count: primary,
        episodes_wide_count: wide,
        parse_errors: 0,
        missing_parent_refs: 0,
        cycles: 0,
    }
}

fn reference_from(product: &JournalRow, turns: u32) -> ReferenceRow {
    ReferenceRow {
        source_id: product.source_id.clone(),
        sha256: product.sha256.clone(),
        session_id: product.session_id.clone(),
        kind: product.kind.clone(),
        tree_id: product.tree_id.clone(),
        header_timestamp: product.header_timestamp.clone(),
        leaf_entry_id: product.leaf_entry_id.clone(),
        leaf_ordinal: product.leaf_ordinal,
        leaf_timestamp: product.leaf_timestamp.clone(),
        leaf_type: product.leaf_type.clone(),
        leaf_count: product.leaf_count,
        turns_all_branches: product.turns_all_branches,
        turns_active_leaf: turns,
        abandoned_turns: product.turns_all_branches.saturating_sub(turns),
        episodes_primary_count: product.episodes_primary_count,
        episodes_wide_count: product.episodes_wide_count,
        parse_errors: 0,
        missing_parent_refs: 0,
        cycles: 0,
    }
}

#[test]
fn compare_reports_opposite_sign_differences_before_aggregation() {
    let product = vec![
        row("a", "sha-a", 11, 11, 1, 1),
        row("b", "sha-b", 9, 9, 1, 1),
    ];
    let reference = vec![
        reference_from(&product[0], 10),
        reference_from(&product[1], 10),
    ];
    let report = compare(&product, &reference).unwrap();
    assert_eq!(report.matched_files, 2);
    assert_eq!(report.net_diff_active_leaf, 0);
    assert_eq!(report.gross_abs_diff_active_leaf, 2);
    assert_eq!(report.cancelled_turns, 2);
    assert_eq!(report.journals_differing_active_leaf, 2);
    assert_eq!(report.diffs.len(), 2);
    assert!(report.strict_relative_error_below_1_percent);
    assert_eq!(report.status, "pass");
}

#[test]
fn compare_fails_the_strict_threshold_and_discloses_cohort_exclusions() {
    let product = vec![
        row("a", "sha-a", 98, 98, 1, 1),
        row("only-product", "sha-p", 5, 5, 1, 1),
    ];
    let reference = vec![
        reference_from(&product[0], 100),
        reference_from(&row("only-reference", "sha-r", 7, 7, 1, 1), 7),
    ];
    let report = compare(&product, &reference).unwrap();
    assert_eq!(report.matched_files, 1);
    assert!(!report.strict_relative_error_below_1_percent);
    assert_eq!(report.status, "fail");
    assert_eq!(report.product_only, vec!["only-product"]);
    assert_eq!(report.reference_only, vec!["only-reference"]);
    assert_eq!(report.excluded.product_only_files, 1);
    assert_eq!(report.excluded.reference_only_files, 1);
    assert!(!report.cohort_match);
    assert_eq!(report.net_diff_active_leaf, -2);
    assert_eq!(report.gross_abs_diff_active_leaf, 2);
}

#[test]
fn compare_refuses_duplicate_journal_digests() {
    let product = vec![row("a", "same", 1, 1, 1, 1), row("b", "same", 1, 1, 1, 1)];
    let reference = vec![reference_from(&product[0], 1)];
    assert!(compare(&product, &reference).is_err());
}

#[test]
fn attempt_order_uses_dispatch_time_not_attempt_label() {
    let key = |session: &str, label: &str, ts_ms: i64, spawn: u32| AttemptOrderKey {
        session_id: session.to_string(),
        dispatch_id: format!("dispatch-{session}"),
        attempt: label.to_string(),
        dispatch_timestamp: Some(format_iso_ms(ts_ms)),
        dispatch_ts_ms: ts_ms,
        spawn_index: spawn,
    };
    let keys = vec![
        key("early", "9", 1_000, 2),
        key("middle", "2", 2_000, 0),
        key("late", "0", 3_000, 1),
    ];
    let order = order_attempts(&keys, "early", "late").expect("order");
    assert_eq!(order.before_rank, 1);
    assert_eq!(order.after_rank, 3);
    assert_eq!(
        order.attempts.iter().map(|a| a.session_id.as_str()).collect::<Vec<_>>(),
        vec!["early", "middle", "late"]
    );
    // Label "9" precedes label "0" numerically but was spawned earlier, so
    // the reported label/chronology agreement must be false.
    assert_eq!(order.label_order_agrees, Some(false));
    assert!(order_attempts(&keys, "missing", "late").is_err());
}

const DELTA_BEFORE: &str = concat!(
    "{\"type\":\"session\",\"version\":3,\"id\":\"s1\",\"timestamp\":\"2026-10-01T00:00:00.000Z\"}\n",
    "{\"type\":\"message\",\"id\":\"u1\",\"parentId\":null,\"timestamp\":\"2026-10-01T00:00:02.000Z\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"synthetic request\"}]}}\n",
    "{\"type\":\"message\",\"id\":\"a1\",\"parentId\":\"u1\",\"timestamp\":\"2026-10-01T00:00:03.000Z\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"synthetic answer\"}],\"usage\":{\"totalTokens\":1},\"stopReason\":\"stop\"}}\n",
);

const DELTA_AFTER: &str = concat!(
    "{\"type\":\"session\",\"version\":3,\"id\":\"s1\",\"timestamp\":\"2026-10-02T00:00:00.000Z\"}\n",
    "{\"type\":\"message\",\"id\":\"u1\",\"parentId\":null,\"timestamp\":\"2026-10-02T00:00:02.000Z\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"synthetic request\"}]}}\n",
    "{\"type\":\"message\",\"id\":\"a1\",\"parentId\":\"u1\",\"timestamp\":\"2026-10-02T00:00:03.000Z\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"toolCall\",\"id\":\"c1\",\"name\":\"read\",\"arguments\":{}}],\"usage\":{\"totalTokens\":2},\"stopReason\":\"toolUse\"}}\n",
    "{\"type\":\"custom\",\"customType\":\"tool_execution_start\",\"id\":\"x1\",\"parentId\":\"a1\",\"timestamp\":\"2026-10-02T00:00:04.000Z\",\"data\":{}}\n",
    "{\"type\":\"message\",\"id\":\"r1\",\"parentId\":\"x1\",\"timestamp\":\"2026-10-02T00:00:05.000Z\",\"message\":{\"role\":\"toolResult\",\"toolCallId\":\"c1\",\"toolName\":\"read\",\"content\":[{\"type\":\"text\",\"text\":\"synthetic result\"}]}}\n",
    "{\"type\":\"message\",\"id\":\"u2\",\"parentId\":\"r1\",\"timestamp\":\"2026-10-02T00:00:06.000Z\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"synthetic follow-up\"}]}}\n",
    "{\"type\":\"message\",\"id\":\"a2\",\"parentId\":\"u2\",\"timestamp\":\"2026-10-02T00:00:07.000Z\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"synthetic final\"}],\"usage\":{\"totalTokens\":1},\"stopReason\":\"stop\"}}\n",
);

fn membership(session: &str, attempt: &str) -> DeltaAttemptRef {
    DeltaAttemptRef {
        session_id: session.to_string(),
        dispatch_id: format!("dispatch-{session}"),
        project: "S31".to_string(),
        task: "T701".to_string(),
        attempt: attempt.to_string(),
        parent_session_id: "parent".to_string(),
        parent_span: "span:0:0:1".to_string(),
        child_span: "span:1:0:1".to_string(),
    }
}

#[test]
fn delta_describes_signed_counts_and_turn_and_boundary_changes() {
    let before = project("p/before.jsonl", DELTA_BEFORE.as_bytes(), None);
    let after = project("p/after.jsonl", DELTA_AFTER.as_bytes(), None);
    let keys = vec![
        AttemptOrderKey {
            session_id: "before".to_string(),
            dispatch_id: "dispatch-before".to_string(),
            attempt: "1".to_string(),
            dispatch_timestamp: Some("2026-10-01T00:00:00.000Z".to_string()),
            dispatch_ts_ms: 1_000,
            spawn_index: 0,
        },
        AttemptOrderKey {
            session_id: "after".to_string(),
            dispatch_id: "dispatch-after".to_string(),
            attempt: "2".to_string(),
            dispatch_timestamp: Some("2026-10-02T00:00:00.000Z".to_string()),
            dispatch_ts_ms: 2_000,
            spawn_index: 1,
        },
    ];
    let order = order_attempts(&keys, "before", "after").expect("order");
    let delta: AttemptDelta = attempt_delta(
        &before,
        &after,
        membership("before", "1"),
        membership("after", "2"),
        order,
    );
    assert_eq!(delta.task_key, "S31|T701");
    assert_eq!(delta.counts["turns_active_leaf"].before, 1);
    assert_eq!(delta.counts["turns_active_leaf"].after, 2);
    assert_eq!(delta.counts["turns_active_leaf"].difference, 1);
    assert_eq!(delta.counts["toolcall_blocks"].difference, 1);
    assert_eq!(delta.counts["episodes_primary_count"].difference, 1);
    let added: Vec<_> = delta.turns.iter().filter(|t| t.change == ChangeKind::Added).collect();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].entry_id, "a2");
    assert_eq!(added[0].after.as_ref().unwrap().evidence.line_digest.len(), 64);
    let modified: Vec<_> =
        delta.turns.iter().filter(|t| t.change == ChangeKind::Modified).collect();
    assert_eq!(modified.len(), 1);
    assert_eq!(modified[0].entry_id, "a1");
    assert!(delta.boundaries.iter().any(|b| b.change == ChangeKind::Added && b.kind == "primary"));
    assert!(delta.summary.contains("turns_active_leaf 1 -> 2 (+1)"));
    assert_eq!(delta.before.sha256, sha256_hex(DELTA_BEFORE.as_bytes()));
    assert_eq!(count_metrics(&before.counts)["turns_active_leaf"], 1);
}
