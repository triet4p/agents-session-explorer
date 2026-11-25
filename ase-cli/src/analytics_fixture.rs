//! Synthetic producer-shaped OMP journals for runtime analytics rules.
//!
//! Each file isolates one rule: parallel clocks, two projects sharing a model,
//! active-leaf choice, half-open period and missing producer fields. No corpus
//! path, prose or private identifier is written.

use ase_core::{err, Result};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use crate::opt;

// Synthetic producer-shaped OMP journals for a deterministic runtime check. No
// corpus path, prose or private identifier is written; each file isolates one
// rule: parallel clocks, two projects sharing a model, active-leaf choice,
// half-open period and missing producer fields.
fn fixture_analytics_session(id: &str, ts: &str) -> String {
    format!(
        "{}\n",
        json!({
            "type": "session", "version": 3, "id": id, "timestamp": ts,
            "cwd": "C:\\SYNTHETIC\\analytics", "title": "synthetic runtime-analytics fixture"
        })
    )
}

fn fixture_analytics_user(id: &str, parent: Option<&str>, ts: &str) -> String {
    let mut record = json!({
        "type": "message", "id": id, "timestamp": ts,
        "message": {"role": "user", "content": [{"type": "text", "text": "synthetic"}]}
    });
    if let Some(parent) = parent {
        record["parentId"] = json!(parent);
    }
    format!("{record}\n")
}

#[allow(clippy::too_many_arguments)]
fn fixture_analytics_turn(
    id: &str,
    parent: &str,
    ts: &str,
    model: Option<&str>,
    msg_ts: Option<i64>,
    completed: Option<i64>,
    duration: Option<f64>,
    ttft: Option<f64>,
    cost: Option<f64>,
    usage: bool,
) -> String {
    let mut message = json!({
        "role": "assistant",
        "content": [{"type": "text", "text": "synthetic"}],
        "stopReason": "stop"
    });
    if let Some(model) = model {
        message["model"] = json!(model);
    }
    if let Some(ts) = msg_ts {
        message["timestamp"] = json!(ts);
    }
    if let Some(ts) = completed {
        message["completedAt"] = json!(ts);
    }
    if let Some(value) = duration {
        message["duration"] = json!(value);
    }
    if let Some(value) = ttft {
        message["ttft"] = json!(value);
    }
    if usage {
        let mut cost_obj = json!({"input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0});
        if let Some(value) = cost {
            cost_obj["total"] = json!(value);
        }
        message["usage"] = json!({
            "input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0,
            "totalTokens": 2, "cost": cost_obj
        });
    }
    format!(
        "{}\n",
        json!({
            "type": "message", "id": id, "parentId": parent, "timestamp": ts,
            "message": message
        })
    )
}

pub(crate) fn run(args: &[String]) -> Result<()> {
    let out = PathBuf::from(
        opt(args, "--out").ok_or_else(|| err("fixture-analytics needs --out DIR"))?,
    );
    let omp = out.join("omp");
    let codex = out.join("codex");
    fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;
    let write = |rel: &str, content: String| -> Result<()> {
        let path = omp.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| err(format!("create {}: {e}", parent.display())))?;
        }
        fs::write(&path, content).map_err(|e| err(format!("write {}: {e}", path.display())))
    };

    let mut parallel = fixture_analytics_session("analytics-parallel", "2026-10-05T00:00:00.000Z");
    parallel.push_str(&fixture_analytics_user("u1", None, "2026-10-05T00:00:01.000Z"));
    parallel.push_str(&fixture_analytics_turn("a1", "u1", "2026-10-05T00:00:02.000Z",
        Some("fixture-parallel"), Some(1_791_158_402_000), Some(1_791_158_404_000),
        Some(2000.0), Some(500.0), Some(0.01), true));
    parallel.push_str(&fixture_analytics_turn("a2", "a1", "2026-10-05T00:00:03.000Z",
        Some("fixture-parallel"), Some(1_791_158_402_500), Some(1_791_158_404_500),
        Some(2000.0), Some(700.0), Some(0.02), true));
    write("p-parallel/parallel.jsonl", parallel)?;

    let mut second = fixture_analytics_session("analytics-second", "2026-10-05T01:00:00.000Z");
    second.push_str(&fixture_analytics_user("u1", None, "2026-10-05T01:00:01.000Z"));
    second.push_str(&fixture_analytics_turn("b1", "u1", "2026-10-05T01:00:02.000Z",
        Some("fixture-parallel"), Some(1_791_162_002_000), Some(1_791_162_003_000),
        Some(1000.0), Some(200.0), Some(0.03), true));
    write("q-parallel/second.jsonl", second)?;

    let mut branch = fixture_analytics_session("analytics-branch", "2026-10-02T00:00:00.000Z");
    branch.push_str(&fixture_analytics_user("u1", None, "2026-10-02T00:00:01.000Z"));
    branch.push_str(&fixture_analytics_turn("a1", "u1", "2026-10-02T00:00:02.000Z",
        Some("fixture-branch"), Some(1_790_899_202_000), Some(1_790_899_203_000),
        Some(1000.0), Some(100.0), Some(0.01), true));
    branch.push_str(&fixture_analytics_turn("a2a", "a1", "2026-10-02T00:00:03.000Z",
        Some("fixture-branch"), Some(1_790_899_203_000), Some(1_790_899_204_000),
        Some(1000.0), Some(100.0), Some(0.10), true));
    branch.push_str(&fixture_analytics_turn("a2b", "a1", "2026-10-02T00:00:04.000Z",
        Some("fixture-branch"), Some(1_790_899_204_000), Some(1_790_899_205_000),
        Some(1000.0), Some(100.0), Some(0.20), true));
    write("p-branch/branch.jsonl", branch)?;

    let mut window = fixture_analytics_session("analytics-window", "2026-10-01T00:00:00.000Z");
    window.push_str(&fixture_analytics_user("u1", None, "2026-10-01T00:00:01.000Z"));
    window.push_str(&fixture_analytics_turn("before", "u1", "2026-09-30T23:59:59.000Z",
        Some("fixture-window"), Some(1_790_812_799_000), Some(1_790_812_799_500),
        Some(500.0), Some(50.0), Some(0.01), true));
    window.push_str(&fixture_analytics_turn("inside", "before", "2026-10-01T00:00:00.000Z",
        Some("fixture-window"), Some(1_790_812_800_000), Some(1_790_812_800_500),
        Some(500.0), Some(50.0), Some(0.02), true));
    window.push_str(&fixture_analytics_turn("at-end", "inside", "2026-11-01T00:00:00.000Z",
        Some("fixture-window"), Some(1_793_491_200_000), Some(1_793_491_200_500),
        Some(500.0), Some(50.0), Some(0.04), true));
    write("p-window/window.jsonl", window)?;

    let mut gaps = fixture_analytics_session("analytics-gaps", "2026-10-03T00:00:00.000Z");
    gaps.push_str(&fixture_analytics_user("u1", None, "2026-10-03T00:00:01.000Z"));
    gaps.push_str(&format!("{}\n", json!({
        "type": "message", "id": "n1", "parentId": "u1",
        "timestamp": "2026-10-03T00:00:02.000Z",
        "message": {"role": "assistant", "content": [{"type": "text", "text": "synthetic"}]}
    })));
    gaps.push_str(&fixture_analytics_turn("c1", "n1", "2026-10-03T00:00:03.000Z",
        Some("fixture-gaps"), Some(1_790_983_203_000), None, None, None, None, true));
    gaps.push_str(&fixture_analytics_turn("c2", "c1", "2026-10-03T00:00:04.000Z",
        None, Some(1_790_983_204_000), Some(1_790_983_205_000),
        Some(1000.0), Some(300.0), Some(0.05), true));
    write("p-gaps/gaps.jsonl", gaps)?;

    println!("wrote 5 synthetic runtime-analytics OMP fixtures to {}", out.display());
    Ok(())
}
