//! Synthetic producer-shaped OMP journals for the explicit attempt delta.
//!
//! All identifiers and prose are synthetic; every file is written below the
//! caller's output path.

use ase_core::{err, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use crate::cli_option::opt;

// ----------------------------------------------- explicit attempt delta

const DELTA_PARENT_SESSION: &str = "delta-parent";
const D_DELTA_BEFORE: &str = "ab000001-0000-4000-8000-000000000001";
const D_DELTA_AFTER: &str = "ab000002-0000-4000-8000-000000000002";
const D_DELTA_LATEST: &str = "ab000003-0000-4000-8000-000000000003";
const D_DELTA_CROSS: &str = "ab000004-0000-4000-8000-000000000004";
const D_DELTA_OTHER: &str = "ab000005-0000-4000-8000-000000000005";

fn delta_ts(day: u32, second: u32) -> String {
    format!("2026-10-{day:02}T00:00:{second:02}.000Z")
}

fn delta_session_line(id: &str, ts: &str) -> String {
    format!(
        "{}\n",
        json!({
            "type": "session", "version": 3, "id": id, "timestamp": ts,
            "cwd": "C:\\SYNTHETIC\\delta", "title": "synthetic attempt-delta fixture"
        })
    )
}

fn delta_parent_line(
    dispatch_id: &str,
    project: &str,
    task: &str,
    attempt: &str,
    spawn_index: u32,
    ts: &str,
) -> String {
    format!(
        "{}\n",
        json!({
            "type": "custom", "timestamp": ts, "customType": "task_dispatch_v1",
            "data": {
                "dispatch_id": dispatch_id,
                "parent_session_id": DELTA_PARENT_SESSION,
                "parent_tool_call_id": format!("call-{spawn_index}"),
                "spawn_index": spawn_index,
                "project_id": project,
                "task_id": task,
                "attempt_id": attempt
            }
        })
    )
}

fn delta_child_link_line(dispatch_id: &str, session_id: &str, ts: &str) -> String {
    format!(
        "{}\n",
        json!({
            "type": "custom", "timestamp": ts, "customType": "task_dispatch_child_v1",
            "data": { "dispatch_id": dispatch_id, "session_id": session_id }
        })
    )
}

fn delta_message_line(
    id: &str,
    parent: Option<&str>,
    ts: &str,
    role: &str,
    usage: bool,
    tool_calls: u32,
) -> String {
    let content: Vec<Value> = if tool_calls > 0 {
        (1..=tool_calls)
            .map(|index| {
                json!({
                    "type": "toolCall", "id": format!("c{index}"),
                    "name": "read", "arguments": {}
                })
            })
            .collect()
    } else {
        vec![json!({"type": "text", "text": "synthetic delta fixture entry"})]
    };
    let mut message = json!({"role": role, "content": content});
    if usage {
        message["usage"] = json!({"totalTokens": 1});
        message["stopReason"] = json!("stop");
    }
    let mut record = json!({"type": "message", "id": id, "timestamp": ts, "message": message});
    if let Some(parent) = parent {
        record["parentId"] = json!(parent);
    }
    format!("{record}\n")
}

fn delta_tool_result_line(id: &str, parent: &str, ts: &str, call_id: &str) -> String {
    format!(
        "{}\n",
        json!({
            "type": "message", "id": id, "parentId": parent, "timestamp": ts,
            "message": {
                "role": "toolResult", "toolCallId": call_id, "toolName": "read",
                "content": [{"type": "text", "text": "synthetic delta fixture result"}]
            }
        })
    )
}

fn delta_custom_user_line(id: &str, parent: &str, ts: &str) -> String {
    format!(
        "{}\n",
        json!({
            "type": "custom_message", "customType": "synthetic-user-note",
            "attribution": "user", "id": id, "parentId": parent, "timestamp": ts,
            "content": "synthetic wide boundary"
        })
    )
}

fn delta_prose_candidate_line(id: &str, parent: Option<&str>, ts: &str) -> String {
    let mut record = json!({
        "type": "message", "id": id, "timestamp": ts,
        "message": {
            "role": "user",
            "content": [{"type": "text", "text": "synthetic mention task_id=T701 without dispatch"}]
        }
    });
    if let Some(parent) = parent {
        record["parentId"] = json!(parent);
    }
    format!("{record}\n")
}

/// Synthetic, producer-shaped OMP journals for a deterministic delta check.
/// Two same-task attempts have visibly different turns and episode
/// boundaries; a later attempt carries a smaller label to prove labels are
/// not chronology. A cross-project same-token attempt, another-task attempt
/// and an unconfirmed prose-only session exercise the rejections.
pub(crate) fn run(args: &[String]) -> Result<()> {
    let out = PathBuf::from(
        opt(args, "--out").ok_or_else(|| err("fixture-delta needs --out DIR"))?,
    );
    let omp = out.join("omp");
    let codex = out.join("codex");
    let dir = omp.join("delta");
    fs::create_dir_all(&dir).map_err(|e| err(format!("create {}: {e}", dir.display())))?;
    fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;
    let write = |name: &str, content: String| -> Result<()> {
        let path = dir.join(name);
        fs::write(&path, content).map_err(|e| err(format!("write {}: {e}", path.display())))
    };

    let mut parent = delta_session_line(DELTA_PARENT_SESSION, &delta_ts(1, 0));
    parent.push_str(&delta_parent_line(D_DELTA_BEFORE, "Proj31", "T701", "1", 0, &delta_ts(1, 0)));
    parent.push_str(&delta_parent_line(D_DELTA_AFTER, "Proj31", "T701", "2", 1, &delta_ts(2, 0)));
    parent.push_str(&delta_parent_line(D_DELTA_LATEST, "Proj31", "T701", "0", 2, &delta_ts(3, 0)));
    parent.push_str(&delta_parent_line(D_DELTA_CROSS, "Proj32", "T701", "1", 3, &delta_ts(1, 0)));
    parent.push_str(&delta_parent_line(D_DELTA_OTHER, "Proj31", "T702", "1", 4, &delta_ts(1, 0)));
    parent.push_str(&delta_message_line("p-u1", None, &delta_ts(1, 1), "user", false, 0));
    write("parent.jsonl", parent)?;

    let mut before = delta_session_line("delta-before", &delta_ts(1, 0));
    before.push_str(&delta_child_link_line(D_DELTA_BEFORE, "delta-before", &delta_ts(1, 1)));
    before.push_str(&delta_message_line("u1", None, &delta_ts(1, 2), "user", false, 0));
    before.push_str(&delta_message_line("a1", Some("u1"), &delta_ts(1, 3), "assistant", true, 0));
    write("child-before.jsonl", before)?;

    let mut after = delta_session_line("delta-after", &delta_ts(2, 0));
    after.push_str(&delta_child_link_line(D_DELTA_AFTER, "delta-after", &delta_ts(2, 1)));
    after.push_str(&delta_message_line("u1", None, &delta_ts(2, 2), "user", false, 0));
    after.push_str(&delta_message_line("a1", Some("u1"), &delta_ts(2, 3), "assistant", true, 2));
    after.push_str(&format!(
        "{}\n",
        json!({
            "type": "custom", "customType": "tool_execution_start", "id": "x1",
            "parentId": "a1", "timestamp": delta_ts(2, 4), "data": {}
        })
    ));
    after.push_str(&delta_tool_result_line("r1", "x1", &delta_ts(2, 5), "c1"));
    after.push_str(&delta_message_line("u2", Some("r1"), &delta_ts(2, 6), "user", false, 0));
    after.push_str(&delta_custom_user_line("cm1", "u2", &delta_ts(2, 7)));
    after.push_str(&delta_message_line("a2", Some("cm1"), &delta_ts(2, 8), "assistant", true, 0));
    write("child-after.jsonl", after)?;

    let mut latest = delta_session_line("delta-latest", &delta_ts(3, 0));
    latest.push_str(&delta_child_link_line(D_DELTA_LATEST, "delta-latest", &delta_ts(3, 1)));
    latest.push_str(&delta_message_line("u1", None, &delta_ts(3, 2), "user", false, 0));
    latest.push_str(&delta_message_line("b1", Some("u1"), &delta_ts(3, 3), "assistant", true, 0));
    latest.push_str(&delta_message_line("u2", Some("b1"), &delta_ts(3, 4), "user", false, 0));
    latest.push_str(&delta_message_line("b2", Some("u2"), &delta_ts(3, 5), "assistant", true, 0));
    write("child-latest.jsonl", latest)?;

    let mut cross = delta_session_line("delta-cross", &delta_ts(1, 0));
    cross.push_str(&delta_child_link_line(D_DELTA_CROSS, "delta-cross", &delta_ts(1, 1)));
    cross.push_str(&delta_message_line("u1", None, &delta_ts(1, 2), "user", false, 0));
    cross.push_str(&delta_message_line("a1", Some("u1"), &delta_ts(1, 3), "assistant", true, 0));
    write("child-cross.jsonl", cross)?;

    let mut other = delta_session_line("delta-other-task", &delta_ts(1, 0));
    other.push_str(&delta_child_link_line(D_DELTA_OTHER, "delta-other-task", &delta_ts(1, 1)));
    other.push_str(&delta_message_line("u1", None, &delta_ts(1, 2), "user", false, 0));
    other.push_str(&delta_message_line("a1", Some("u1"), &delta_ts(1, 3), "assistant", true, 0));
    write("child-other-task.jsonl", other)?;

    let mut unconfirmed = delta_session_line("delta-unconfirmed", &delta_ts(1, 0));
    unconfirmed.push_str(&delta_prose_candidate_line("u1", None, &delta_ts(1, 2)));
    unconfirmed.push_str(&delta_message_line("a1", Some("u1"), &delta_ts(1, 3), "assistant", true, 0));
    write("child-unconfirmed.jsonl", unconfirmed)?;

    println!(
        "wrote 7 synthetic attempt-delta OMP fixtures to {}",
        out.display()
    );
    Ok(())
}
