//! Synthetic task reconciliation fixtures covering assigned, ambiguous and
//! unresolved signals, including cross-project tokens and delegation edges.

use crate::cli_option::opt;
use crate::cli_output::outln;
use ase_core::{err, Result};
use std::fs;
use std::path::PathBuf;

// ------------------------------------------------------- resolve fixture

const RESOLVE_TS: &str = "2026-03-07T08:09:10.000Z";

struct ResolveFixtureFile {
    rel: &'static str,
    id: &'static str,
    parent: Option<&'static str>,
    messages: &'static [(&'static str, &'static str)],
}

// Synthetic only. Each case isolates one reconciliation outcome:
// consistent naming+marker, a cross-layer conflict, naming-only, a multi-ID
// session, the same token in two projects, marker-only, mention-only and a
// parentSession pair without a task grammar, plus a two-project token and a
// naming-only assignment with no dispatch join.
const RESOLVE_FIXTURE: &[ResolveFixtureFile] = &[
    ResolveFixtureFile {
        rel: "resolve/Proj10Task101Alpha1.jsonl",
        id: "fx-101-alpha",
        parent: None,
        messages: &[("user", "Please implement task_id=101 in the parser.")],
    },
    ResolveFixtureFile {
        rel: "resolve/Proj10Task102Beta1.jsonl",
        id: "fx-102-beta",
        parent: None,
        messages: &[(
            "assistant",
            "Please implement task_id=999 for this change.",
        )],
    },
    ResolveFixtureFile {
        rel: "resolve/Proj10Task103Gamma1.jsonl",
        id: "fx-103-gamma-1",
        parent: None,
        messages: &[("assistant", "Working on the naming grammar only.")],
    },
    ResolveFixtureFile {
        rel: "resolve/Proj10Task103Gamma3.jsonl",
        id: "fx-103-gamma-3",
        parent: None,
        messages: &[("user", "Same task, later attempt label.")],
    },
    ResolveFixtureFile {
        rel: "resolve/Proj10Task108Eta1.jsonl",
        id: "fx-108-eta",
        parent: None,
        messages: &[(
            "user",
            "Please implement task_id=108 and task_id=777 as separate follow-ups.",
        )],
    },
    ResolveFixtureFile {
        rel: "resolve/Proj20Task55Gold1.jsonl",
        id: "fx-55-alpha",
        parent: None,
        messages: &[("user", "Implement task_id=55 for project Proj20.")],
    },
    ResolveFixtureFile {
        rel: "resolve/Proj21Task55Gold2.jsonl",
        id: "fx-55-beta",
        parent: None,
        messages: &[("assistant", "Implement task_id=55 for project Proj21.")],
    },
    ResolveFixtureFile {
        rel: "resolve/session-notes.jsonl",
        id: "fx-notes",
        parent: None,
        messages: &[(
            "user",
            "Please implement task_id=555, but the filename has no grammar.",
        )],
    },
    ResolveFixtureFile {
        rel: "resolve/session-mention.jsonl",
        id: "fx-mention",
        parent: None,
        messages: &[(
            "user",
            "Earlier we mentioned task_id=666 for reference only.",
        )],
    },
    ResolveFixtureFile {
        rel: "resolve/syn-parent-root.jsonl",
        id: "syn-parent-root",
        parent: None,
        messages: &[("user", "root synthetic session")],
    },
    ResolveFixtureFile {
        rel: "resolve/syn-parent-child.jsonl",
        id: "syn-parent-child",
        parent: Some("syn-parent-root"),
        messages: &[("user", "delegated synthetic session")],
    },
    ResolveFixtureFile {
        rel: "resolve/Four4AlphaTask4Role1.jsonl",
        id: "fx-four-alpha-4",
        parent: None,
        messages: &[("user", "Implement task_id=4 for Four4Alpha.")],
    },
    ResolveFixtureFile {
        rel: "resolve/Four4BetaTask4Role1.jsonl",
        id: "fx-four-beta-4",
        parent: None,
        messages: &[("user", "Implement task_id=4 for Four4Beta.")],
    },
    ResolveFixtureFile {
        rel: "resolve/Solo4NameTask411Role1.jsonl",
        id: "fx-solo-411",
        parent: None,
        messages: &[("user", "Implement task_id=411 in the parser.")],
    },
];

fn resolve_session_line(id: &str, parent: Option<&str>) -> String {
    let parent_field = parent
        .map(|value| format!(",\"parentSession\":\"{value}\""))
        .unwrap_or_default();
    format!(
        "{{\"type\":\"session\",\"version\":3,\"id\":\"{id}\",\"timestamp\":\"{RESOLVE_TS}\",\"cwd\":\"C:\\\\SYNTHETIC\\\\resolve\",\"title\":\"synthetic resolve fixture\"{parent_field}}}\n"
    )
}

pub(crate) fn resolve_message_line(sid: &str, ordinal: usize, role: &str, text: &str) -> String {
    format!(
        "{{\"type\":\"message\",\"id\":\"{sid}-e{ordinal}\",\"parentId\":null,\"timestamp\":\"{RESOLVE_TS}\",\"message\":{{\"role\":\"{role}\",\"content\":[{{\"type\":\"text\",\"text\":\"{text}\"}}]}}}}\n"
    )
}

pub fn run(args: &[String]) -> Result<()> {
    let out = PathBuf::from(
        opt(args, "--out").ok_or_else(|| err("fixture-resolve needs --out DIR"))?,
    );
    let omp = out.join("omp");
    let codex = out.join("codex");
    fs::create_dir_all(&omp).map_err(|e| err(format!("create {}: {e}", omp.display())))?;
    fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;
    for entry in RESOLVE_FIXTURE {
        let path = omp.join(entry.rel);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| err(format!("create {}: {e}", dir.display())))?;
        }
        let mut content = resolve_session_line(entry.id, entry.parent);
        for (ordinal, (role, text)) in entry.messages.iter().enumerate() {
            content.push_str(&resolve_message_line(entry.id, ordinal + 1, role, text));
        }
        fs::write(&path, content).map_err(|e| err(format!("write {}: {e}", path.display())))?;
    }
    outln!(
        "wrote {} synthetic resolve fixtures to {}",
        RESOLVE_FIXTURE.len(),
        out.display()
    );
    Ok(())
}
