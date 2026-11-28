//! Synthetic dispatch and turn fixture for the motif command.
//!
//! The generated journals exercise exact, gapped, substituted, deduplicated
//! and cross-project sampling cases. All events are synthetic; no resolver or
//! detection policy lives here.

use crate::cli_option::opt;
use crate::cli_output::outln;
use ase_core::{err, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

const MOTIF_TS: &str = "2026-11-02T00:00:00.000Z";
const MOTIF_PARENT_SESSION: &str = "motif-fixture-parent";

const MESSAGE_TS: &str = "2026-03-07T08:09:10.000Z";

const M_A1: &str = "a1a1a1a1-a1a1-4a1a-8a1a-a1a1a1a1a1a1";
const M_A2: &str = "a2a2a2a2-a2a2-4a2a-8a2a-a2a2a2a2a2a2";
const M_A3: &str = "a3a3a3a3-a3a3-4a3a-8a3a-a3a3a3a3a3a3";
const M_B1: &str = "b1b1b1b1-b1b1-4b1b-8b1b-b1b1b1b1b1b1";
const M_C1: &str = "c1c1c1c1-c1c1-4c1c-8c1c-c1c1c1c1c1c1";
const M_E1: &str = "e1e1e1e1-e1e1-4e1e-8e1e-e1e1e1e1e1e1";
const M_G1: &str = "71717171-7171-4171-8171-717171717171";
const M_G2: &str = "72727272-7272-4272-8272-727272727272";

struct MotifFixtureTask {
    project: &'static str,
    dispatch_id: &'static str,
    task: &'static str,
    attempt: &'static str,
    session_id: &'static str,
    tools: &'static [&'static str],
}

// Synthetic only. Task 1 repeats across three attempts: a1 is exact, a2 adds
// one call, and a3 repeats a1's text (deduped for counting, retained as
// evidence). Task 2 is gapped-only, task 3 is a near miss with one
// substitution, and task 9 is a valid join whose child has no call at all.
// Task 7 exists in two projects with the same call text so the fixture proves
// that a colliding bare token stays ambiguous while `PROJECT|TASK` yields two
// independent samples.
const MOTIF_FIXTURE_TASKS: &[MotifFixtureTask] = &[
    MotifFixtureTask {
        project: "MotifAlpha",
        dispatch_id: M_A1,
        task: "1",
        attempt: "a1",
        session_id: "motif-child-1-a1",
        tools: &["t1", "t2", "t3", "t4"],
    },
    MotifFixtureTask {
        project: "MotifAlpha",
        dispatch_id: M_A2,
        task: "1",
        attempt: "a2",
        session_id: "motif-child-1-a2",
        tools: &["t1", "t2", "x1", "t3", "t4"],
    },
    MotifFixtureTask {
        project: "MotifAlpha",
        dispatch_id: M_A3,
        task: "1",
        attempt: "a3",
        session_id: "motif-child-1-a3",
        tools: &["t1", "t2", "t3", "t4"],
    },
    MotifFixtureTask {
        project: "MotifAlpha",
        dispatch_id: M_B1,
        task: "2",
        attempt: "b1",
        session_id: "motif-child-2-b1",
        tools: &["t1", "t2", "y1", "t3", "t4"],
    },
    MotifFixtureTask {
        project: "MotifAlpha",
        dispatch_id: M_C1,
        task: "3",
        attempt: "c1",
        session_id: "motif-child-3-c1",
        tools: &["t1", "t2", "z1", "t4"],
    },
    MotifFixtureTask {
        project: "MotifAlpha",
        dispatch_id: M_E1,
        task: "9",
        attempt: "a1",
        session_id: "motif-child-9-a1",
        tools: &[],
    },
    MotifFixtureTask {
        project: "MotifAlpha",
        dispatch_id: M_G1,
        task: "7",
        attempt: "g1",
        session_id: "motif-child-7-g1",
        tools: &["s1", "s2", "s3", "s4"],
    },
    MotifFixtureTask {
        project: "MotifBeta",
        dispatch_id: M_G2,
        task: "7",
        attempt: "g2",
        session_id: "motif-alt-child-7-g2",
        tools: &["s1", "s2", "s3", "s4"],
    },
];

fn motif_session_line(id: &str) -> String {
    format!(
        "{}\n",
        json!({
            "type": "session",
            "version": 3,
            "id": id,
            "timestamp": MOTIF_TS,
            "cwd": "C:\\SYNTHETIC\\motif",
            "title": "synthetic motif fixture"
        })
    )
}

fn motif_parent_line(spawn: u32, task: &MotifFixtureTask) -> String {
    format!(
        "{}\n",
        json!({
            "type": "custom",
            "timestamp": MOTIF_TS,
            "customType": "task_dispatch_v1",
            "data": {
                "dispatch_id": task.dispatch_id,
                "parent_session_id": MOTIF_PARENT_SESSION,
                "parent_tool_call_id": format!("call-motif-{spawn}"),
                "spawn_index": spawn,
                "project_id": task.project,
                "task_id": task.task,
                "attempt_id": task.attempt
            }
        })
    )
}

fn motif_child_line(dispatch_id: &str, session_id: &str) -> String {
    format!(
        "{}\n",
        json!({
            "type": "custom",
            "timestamp": MOTIF_TS,
            "customType": "task_dispatch_child_v1",
            "data": { "dispatch_id": dispatch_id, "session_id": session_id }
        })
    )
}

// The no-call child reuses the resolve fixture's message shape so its bytes
// stay identical to the pinned fixture; only this fixture owns the copy.
fn message_line(sid: &str, ordinal: usize, role: &str, text: &str) -> String {
    format!(
        "{{\"type\":\"message\",\"id\":\"{sid}-e{ordinal}\",\"parentId\":null,\"timestamp\":\"{MESSAGE_TS}\",\"message\":{{\"role\":\"{role}\",\"content\":[{{\"type\":\"text\",\"text\":\"{text}\"}}]}}}}\n"
    )
}

fn motif_turn_line(
    session_id: &str,
    ordinal: usize,
    parent: Option<&str>,
    tools: &[&str],
) -> String {
    let id = format!("{session_id}-e{ordinal}");
    let mut content = vec![json!({
        "type": "text",
        "text": format!("synthetic motif turn {ordinal}")
    })];
    for tool in tools {
        content.push(json!({
            "type": "toolCall",
            "id": format!("{id}-{tool}"),
            "name": tool,
            "args": { "path": format!("synthetic/{tool}.txt"), "flag": true }
        }));
    }
    let mut value = json!({
        "type": "message",
        "id": id,
        "parentId": Value::Null,
        "timestamp": MOTIF_TS,
        "message": { "role": "assistant", "content": content }
    });
    if let Some(parent) = parent {
        value["parentId"] = json!(parent);
    }
    format!("{value}\n")
}

pub fn run(args: &[String]) -> Result<()> {
    let out = PathBuf::from(
        opt(args, "--out").ok_or_else(|| err("fixture-motif needs --out DIR"))?,
    );
    let omp = out.join("omp");
    let codex = out.join("codex");
    fs::create_dir_all(&omp).map_err(|e| err(format!("create {}: {e}", omp.display())))?;
    fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;

    let mut parent = motif_session_line(MOTIF_PARENT_SESSION);
    for (spawn, task) in MOTIF_FIXTURE_TASKS.iter().enumerate() {
        parent.push_str(&motif_parent_line(spawn as u32, task));
    }
    let parent_path = omp.join("motif").join("parent.jsonl");
    if let Some(dir) = parent_path.parent() {
        fs::create_dir_all(dir).map_err(|e| err(format!("create {}: {e}", dir.display())))?;
    }
    fs::write(&parent_path, parent)
        .map_err(|e| err(format!("write {}: {e}", parent_path.display())))?;

    for task in MOTIF_FIXTURE_TASKS {
        let mut content = motif_session_line(task.session_id);
        content.push_str(&motif_child_line(task.dispatch_id, task.session_id));
        if task.tools.is_empty() {
            content.push_str(&message_line(
                task.session_id,
                1,
                "assistant",
                "synthetic motif child without tool calls",
            ));
        } else {
            let mut parent_id: Option<String> = None;
            for (ordinal, tool) in task.tools.iter().enumerate() {
                content.push_str(&motif_turn_line(
                    task.session_id,
                    ordinal + 1,
                    parent_id.as_deref(),
                    &[*tool],
                ));
                parent_id = Some(format!("{}-e{}", task.session_id, ordinal + 1));
            }
        }
        let path = omp
            .join("motif")
            .join(format!(
                "child-{}-{}-{}.jsonl",
                task.project, task.task, task.attempt
            ));
        fs::write(&path, content).map_err(|e| err(format!("write {}: {e}", path.display())))?;
    }

    outln!(
        "wrote {} synthetic motif fixture tasks to {}",
        MOTIF_FIXTURE_TASKS.len(),
        out.display()
    );
    Ok(())
}
