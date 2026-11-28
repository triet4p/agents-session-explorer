//! Synthetic native dispatch records for resolver and join regressions.
//!
//! The parent journal carries positive, rejected and competing claims; each
//! child journal is the only place its link is valid. All identifiers are
//! synthetic and every file is written below the caller's output path.

use crate::cli_option::opt;
use crate::cli_output::outln;
use ase_core::{err, Result};
use serde_json::json;
use std::fs;
use std::path::PathBuf;


// ------------------------------------------------------ dispatch fixture

const DISPATCH_TS: &str = "2026-03-08T09:10:11.000Z";
const DISPATCH_PARENT_SESSION: &str = "dispatch-fixture-parent";

const D_410: &str = "11111111-1111-4111-8111-111111111111";
const D_10410: &str = "22222222-2222-4222-8222-222222222222";
const D_ONLY: &str = "33333333-3333-4333-8333-333333333333";
const D_CONFLICT: &str = "44444444-4444-4444-8444-444444444444";
const D_MISSING: &str = "55555555-5555-4555-8555-555555555555";
const D_DUP_PARENT: &str = "66666666-6666-4666-8666-666666666666";
const D_DUP_CHILD: &str = "77777777-7777-4777-8777-777777777777";
const D_WRONG: &str = "88888888-8888-4888-8888-888888888888";
const D_MISMATCH: &str = "99999999-9999-4999-8999-999999999999";
const D_ORPHAN: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const D_COMP_A: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const D_COMP_B: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
const D_INVALID: &str = "not-a-runtime-uuid";

struct DispatchFixtureParent {
    dispatch_id: &'static str,
    parent_session_id: &'static str,
    parent_tool_call_id: Option<&'static str>,
    spawn_index: u32,
    project: &'static str,
    task: &'static str,
    attempt: &'static str,
}

struct DispatchFixtureChild {
    dispatch_id: &'static str,
    session_id: &'static str,
}

struct DispatchFixtureFile {
    rel: &'static str,
    session_id: &'static str,
    messages: &'static [(&'static str, &'static str)],
    parents: &'static [DispatchFixtureParent],
    children: &'static [DispatchFixtureChild],
    raw_lines: &'static [&'static str],
}

// Synthetic only, shaped exactly like native OMP dispatch
// records. The parent journal carries positive, rejected and competing
// claims; each child journal is the only place its link is valid.
const DISPATCH_PARENTS: &[DispatchFixtureParent] = &[
    DispatchFixtureParent {
        dispatch_id: D_410,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-410"),
        spawn_index: 0,
        project: "Proj30",
        task: "410",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_10410,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-10410"),
        spawn_index: 1,
        project: "Proj30",
        task: "10410",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_ONLY,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-only"),
        spawn_index: 2,
        project: "SYN-PROJECT",
        task: "SYN-DISPATCH-ONLY",
        attempt: "attempt-3",
    },
    DispatchFixtureParent {
        dispatch_id: D_CONFLICT,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-conflict"),
        spawn_index: 3,
        project: "SYN-PROJECT",
        task: "SYN-CONFLICT",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_MISSING,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-missing"),
        spawn_index: 4,
        project: "SYN-PROJECT",
        task: "SYN-MISSING",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_DUP_PARENT,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-dup-parent-a"),
        spawn_index: 5,
        project: "SYN-PROJECT",
        task: "SYN-DUP-PARENT",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_DUP_PARENT,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-dup-parent-b"),
        spawn_index: 6,
        project: "SYN-PROJECT",
        task: "SYN-DUP-PARENT",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_DUP_CHILD,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-dup-child"),
        spawn_index: 7,
        project: "SYN-PROJECT",
        task: "SYN-DUP-CHILD",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_WRONG,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-wrong"),
        spawn_index: 8,
        project: "SYN-PROJECT",
        task: "SYN-WRONG-CHILD",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_MISMATCH,
        parent_session_id: "ghost-parent-session",
        parent_tool_call_id: Some("call-mismatch"),
        spawn_index: 9,
        project: "SYN-PROJECT",
        task: "SYN-MISMATCH-PARENT",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_COMP_A,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-comp-a"),
        spawn_index: 10,
        project: "SYN-PROJECT-A",
        task: "SYN-COMPETING",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_COMP_B,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-comp-b"),
        spawn_index: 11,
        project: "SYN-PROJECT-B",
        task: "SYN-COMPETING",
        attempt: "1",
    },
    DispatchFixtureParent {
        dispatch_id: D_INVALID,
        parent_session_id: DISPATCH_PARENT_SESSION,
        parent_tool_call_id: Some("call-malformed"),
        spawn_index: 12,
        project: "SYN-PROJECT",
        task: "SYN-MALFORMED",
        attempt: "1",
    },
];

const DISPATCH_RAW_LINES: &[&str] = &[
    r#"{"type":"custom","timestamp":"2026-03-08T09:10:11.000Z","customType":"task_dispatch_v1","data":{"dispatch_id":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","parent_session_id":"dispatch-fixture-parent","spawn_index":13,"project_id":"SYN-PROJECT","attempt_id":"1"}}"#,
];

const DISPATCH_FIXTURE: &[DispatchFixtureFile] = &[
    DispatchFixtureFile {
        rel: "dispatch/parent-root.jsonl",
        session_id: DISPATCH_PARENT_SESSION,
        messages: &[("user", "dispatch fixture root session")],
        parents: DISPATCH_PARENTS,
        children: &[],
        raw_lines: DISPATCH_RAW_LINES,
    },
    DispatchFixtureFile {
        rel: "dispatch/Proj30Task410Alpha1.jsonl",
        session_id: "dispatch-child-410",
        messages: &[("assistant", "Tracked child for the first shared-suffix task.")],
        parents: &[],
        children: &[DispatchFixtureChild {
            dispatch_id: D_410,
            session_id: "dispatch-child-410",
        }],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "dispatch/Proj30Task10410Beta1.jsonl",
        session_id: "dispatch-child-10410",
        messages: &[("assistant", "Tracked child for the second shared-suffix task.")],
        parents: &[],
        children: &[DispatchFixtureChild {
            dispatch_id: D_10410,
            session_id: "dispatch-child-10410",
        }],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "dispatch/TrackedOnlyGamma2.jsonl",
        session_id: "dispatch-child-only",
        messages: &[("assistant", "Tracked child without a name-grammar key.")],
        parents: &[],
        children: &[DispatchFixtureChild {
            dispatch_id: D_ONLY,
            session_id: "dispatch-child-only",
        }],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "dispatch/Proj99Task999Delta3.jsonl",
        session_id: "dispatch-child-conflict",
        messages: &[("assistant", "Child whose file name disagrees with its dispatch identity.")],
        parents: &[],
        children: &[DispatchFixtureChild {
            dispatch_id: D_CONFLICT,
            session_id: "dispatch-child-conflict",
        }],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "dispatch/DupParentZeta1.jsonl",
        session_id: "dispatch-child-dup-parent",
        messages: &[("assistant", "Child for a duplicated parent claim.")],
        parents: &[],
        children: &[DispatchFixtureChild {
            dispatch_id: D_DUP_PARENT,
            session_id: "dispatch-child-dup-parent",
        }],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "dispatch/DupChildEta1.jsonl",
        session_id: "dispatch-child-dup-child",
        messages: &[("assistant", "Child carrying a duplicated link.")],
        parents: &[],
        children: &[
            DispatchFixtureChild {
                dispatch_id: D_DUP_CHILD,
                session_id: "dispatch-child-dup-child",
            },
            DispatchFixtureChild {
                dispatch_id: D_DUP_CHILD,
                session_id: "dispatch-child-dup-child",
            },
        ],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "dispatch/WrongChildTheta1.jsonl",
        session_id: "dispatch-child-wrong",
        messages: &[("assistant", "Child whose link names another session.")],
        parents: &[],
        children: &[DispatchFixtureChild {
            dispatch_id: D_WRONG,
            session_id: "ghost-session",
        }],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "dispatch/MismatchParentIota1.jsonl",
        session_id: "dispatch-child-mismatch",
        messages: &[("assistant", "Child whose parent record names another session.")],
        parents: &[],
        children: &[DispatchFixtureChild {
            dispatch_id: D_MISMATCH,
            session_id: "dispatch-child-mismatch",
        }],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "dispatch/OrphanChildKappa1.jsonl",
        session_id: "dispatch-child-orphan",
        messages: &[("assistant", "Child carrying a link with no parent record.")],
        parents: &[],
        children: &[DispatchFixtureChild {
            dispatch_id: D_ORPHAN,
            session_id: "dispatch-child-orphan",
        }],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "dispatch/CompetingChildLambda1.jsonl",
        session_id: "dispatch-child-competing",
        messages: &[("assistant", "Child claimed by two dispatch identities.")],
        parents: &[],
        children: &[
            DispatchFixtureChild {
                dispatch_id: D_COMP_A,
                session_id: "dispatch-child-competing",
            },
            DispatchFixtureChild {
                dispatch_id: D_COMP_B,
                session_id: "dispatch-child-competing",
            },
        ],
        raw_lines: &[],
    },
    DispatchFixtureFile {
        rel: "dispatch/Proj40Task888Legacy1.jsonl",
        session_id: "dispatch-legacy-child",
        messages: &[(
            "user",
            "Please implement task_id=888 in the parser without dispatch tracking.",
        )],
        parents: &[],
        children: &[],
        raw_lines: &[],
    },
];

fn dispatch_session_line(id: &str) -> String {
    format!(
        "{{\"type\":\"session\",\"version\":3,\"id\":\"{id}\",\"timestamp\":\"{DISPATCH_TS}\",\"cwd\":\"C:\\\\SYNTHETIC\\\\dispatch\",\"title\":\"synthetic dispatch fixture\"}}\n"
    )
}

fn dispatch_parent_line(row: &DispatchFixtureParent) -> String {
    let mut data = json!({
        "dispatch_id": row.dispatch_id,
        "parent_session_id": row.parent_session_id,
        "spawn_index": row.spawn_index,
        "project_id": row.project,
        "task_id": row.task,
        "attempt_id": row.attempt,
    });
    if let Some(tool_call_id) = row.parent_tool_call_id {
        data["parent_tool_call_id"] = json!(tool_call_id);
    }
    format!(
        "{}\n",
        json!({
            "type": "custom",
            "timestamp": DISPATCH_TS,
            "customType": "task_dispatch_v1",
            "data": data
        })
    )
}

fn dispatch_child_line(row: &DispatchFixtureChild) -> String {
    format!(
        "{}\n",
        json!({
            "type": "custom",
            "timestamp": DISPATCH_TS,
            "customType": "task_dispatch_child_v1",
            "data": {
                "dispatch_id": row.dispatch_id,
                "session_id": row.session_id
            }
        })
    )
}

pub fn run(args: &[String]) -> Result<()> {
    let out = PathBuf::from(
        opt(args, "--out").ok_or_else(|| err("fixture-dispatch needs --out DIR"))?,
    );
    let omp = out.join("omp");
    let codex = out.join("codex");
    fs::create_dir_all(&omp).map_err(|e| err(format!("create {}: {e}", omp.display())))?;
    fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;
    for entry in DISPATCH_FIXTURE {
        let path = omp.join(entry.rel);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| err(format!("create {}: {e}", dir.display())))?;
        }
        let mut content = dispatch_session_line(entry.session_id);
        for parent in entry.parents {
            content.push_str(&dispatch_parent_line(parent));
        }
        for child in entry.children {
            content.push_str(&dispatch_child_line(child));
        }
        for raw in entry.raw_lines {
            content.push_str(raw);
            content.push('\n');
        }
        for (ordinal, (role, text)) in entry.messages.iter().enumerate() {
            content.push_str(&crate::resolve_fixture::resolve_message_line(entry.session_id, ordinal + 1, role, text));
        }
        fs::write(&path, content).map_err(|e| err(format!("write {}: {e}", path.display())))?;
    }
    outln!(
        "wrote {} synthetic dispatch fixtures to {}",
        DISPATCH_FIXTURE.len(),
        out.display()
    );
    Ok(())
}
