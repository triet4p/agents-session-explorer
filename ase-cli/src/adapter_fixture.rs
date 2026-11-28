//! Synthetic OMP stock journal fixture for advisory adapter claims.
//!
//! The generated parent and child journals cover matching, multi-child,
//! missing-result and conflicting-parent claim shapes plus sidecar handoffs.
//! No resolver or motif membership is read or written here.

use crate::cli_option::{opt, parse_opt};
use crate::cli_output::outln;
use ase_core::{err, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

fn write_jsonl(path: &Path, lines: &[Value]) -> Result<()> {
    let mut content = String::new();
    for line in lines {
        content.push_str(
            &serde_json::to_string(line).map_err(|e| err(format!("serialize fixture: {e}")))?,
        );
        content.push('\n');
    }
    fs::write(path, content).map_err(|e| err(format!("write {}: {e}", path.display())))
}

fn fixture_task_call(call_id: &str, names: &[&str], prompt: &str) -> Value {
    let tasks: Vec<Value> = names
        .iter()
        .map(|n| json!({"name": n, "agent": "gold-task", "task": prompt}))
        .collect();
    json!({
        "type": "message",
        "id": format!("call-{call_id}"),
        "parentId": null,
        "timestamp": "2026-01-01T00:00:01.000Z",
        "message": {
            "role": "assistant",
            "content": [{"type": "toolCall", "id": call_id, "name": "task",
                         "arguments": {"i": "synthetic dispatch", "tasks": tasks}}]
        }
    })
}

fn fixture_task_result(call_id: &str, text: &str) -> Value {
    json!({
        "type": "message",
        "id": format!("result-{call_id}"),
        "parentId": null,
        "timestamp": "2026-01-01T00:00:02.000Z",
        "message": {"role": "toolResult", "toolCallId": call_id, "toolName": "task",
                    "content": [{"type": "text", "text": text}]}
    })
}

fn fixture_session(id: &str, cwd: &str, parent: Option<&str>) -> Value {
    let mut v = json!({
        "type": "session",
        "version": 3,
        "id": id,
        "timestamp": "2026-01-01T00:00:00.000Z",
        "cwd": cwd
    });
    if let Some(p) = parent {
        v["parentSession"] = json!(p);
    }
    v
}

pub fn run(args: &[String]) -> Result<()> {
    let out = PathBuf::from(
        opt(args, "--out").ok_or_else(|| err("fixture-adapter needs --out DIR"))?,
    );
    let parents = parse_opt::<usize>(args, "--parents")?.unwrap_or(1);
    if parents == 0 {
        return Err(err("--parents needs at least 1 parent journal"));
    }
    let project = out.join("omp").join("synthetic");
    let side = project.join("2026-01-01T00-00-00-000Z_syn");
    fs::create_dir_all(&side).map_err(|e| err(format!("create {}: {e}", side.display())))?;
    let parent = project.join("2026-01-01T00-00-00-000Z_syn.jsonl");
    let parent_abs = std::path::absolute(&parent)
        .unwrap_or_else(|_| parent.clone())
        .to_string_lossy()
        .into_owned();
    let other_parent = std::path::absolute(project.join("2026-01-01T00-00-00-000Z_other.jsonl"))
        .unwrap_or_else(|_| parent.clone())
        .to_string_lossy()
        .into_owned();

    let child = |name: &str, parent_ref: &str| -> Vec<Value> {
        vec![
            fixture_session("syn-root", "C:\\SYN\\demo", Some(parent_ref)),
            json!({"type":"message","id":format!("{name}-u"),"parentId":null,
                   "message":{"role":"user","content":[{"type":"text","text":"SYNTHETIC assignment"}]}}),
        ]
    };

    let parent_lines = vec![
        fixture_session("syn-root", "C:\\SYN\\demo", None),
        fixture_task_call("call-a", &["T1A1"], "Target: W1a attempt A1"),
        fixture_task_result("call-a", "Spawned agent `T1A1` (job `T1A1`)."),
        fixture_task_call("call-b", &["T2A1", "T2B1"], "Target: W2a attempt A1"),
        fixture_task_result(
            "call-b",
            "Spawned 2 background agents using gold-task.\n- `T2A1` (job `T2A1`)\n- `T2B1` (job `T2B1`)",
        ),
        fixture_task_call("call-c", &["T9A1"], "Target: W9a attempt A1"),
        fixture_task_result("call-c", "Task T9A1 failed preflight: synthetic error"),
        fixture_task_call("call-d", &["T1A1"], "Target: W1a attempt A2"),
        fixture_task_result("call-d", "Spawned agent `T4A1` (job `T4A1`)."),
        fixture_task_call("call-e", &["T5A1"], "Target: W5a attempt A1"),
        fixture_task_result("call-e", "Spawned agent `T5A1` (job `T5A1`)."),
    ];
    write_jsonl(&parent, &parent_lines)?;

    write_jsonl(&side.join("T1A1.jsonl"), &child("T1A1", &parent_abs))?;
    write_jsonl(&side.join("T2A1.jsonl"), &child("T2A1", &parent_abs))?;
    write_jsonl(&side.join("T2B1.jsonl"), &child("T2B1", &parent_abs))?;
    write_jsonl(&side.join("T4A1.jsonl"), &child("T4A1", &parent_abs))?;
    // Link conflict: the result names this child but its header names another
    // parent journal. Keep both facts; do not tie-break.
    write_jsonl(&side.join("T5A1.jsonl"), &child("T5A1", &other_parent))?;
    fs::write(
        side.join("T1A1.md"),
        "Synthetic handoff\n- task_id: W1a\n- attempt_id: A1\n",
    )
    .map_err(|e| err(format!("write synthetic handoff: {e}")))?;
    fs::write(
        side.join("T2A1.md"),
        "Synthetic handoff\n- task_id: W2a\n- attempt_id: A1\n",
    )
    .map_err(|e| err(format!("write synthetic handoff: {e}")))?;
    for n in 1..parents {
        let label = format!("p{n:02}");
        let extra = project.join(format!("2026-01-01T00-00-00-000Z_{label}.jsonl"));
        let extra_abs = std::path::absolute(&extra)
            .unwrap_or_else(|_| extra.clone())
            .to_string_lossy()
            .into_owned();
        let child_name = format!("T{label}");
        let lines = vec![
            fixture_session(
                &format!("syn-root-{label}"),
                &format!("C:\\SYN\\{label}"),
                None,
            ),
            fixture_task_call(
                &label,
                &[child_name.as_str()],
                &format!("Target: W{label} attempt A1"),
            ),
            fixture_task_result(
                &label,
                &format!("Spawned agent `{child_name}` (job `{child_name}`)."),
            ),
        ];
        write_jsonl(&extra, &lines)?;
        write_jsonl(
            &side.join(format!("{child_name}.jsonl")),
            &vec![
                fixture_session(
                    &format!("syn-child-{label}"),
                    &format!("C:\\SYN\\{label}"),
                    Some(&extra_abs),
                ),
                json!({"type":"message","id":format!("{child_name}-u"),"parentId":null,
                       "message":{"role":"user","content":[{"type":"text","text":"SYNTHETIC assignment"}]}}),
            ],
        )?;
        fs::write(
            side.join(format!("{child_name}.md")),
            format!("Synthetic handoff\n- task_id: W{label}\n- attempt_id: A1\n"),
        )
        .map_err(|e| err(format!("write synthetic handoff: {e}")))?;
    }
    outln!("wrote synthetic stock adapter fixture to {}", out.display());
    Ok(())
}
