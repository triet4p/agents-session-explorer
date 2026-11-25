//! Synthetic naming and delegation fixtures for metadata-only audits.

use crate::opt;
use ase_core::{err, Result};
use std::fs;
use std::path::PathBuf;

// ------------------------------------------------------------ naming audit

// Only synthetic names are used for rule work. Each tuple is
// (parent directory, file basenames); project comes from the name itself.
const NAMING_FIXTURE: &[(&str, &[&str])] = &[
    ("multiple-splits", &["Proj18Task70BS01", "Proj18Task70SR02", "Proj18Task70DEP3"]),
    ("marker-t", &["Proj14T3ProviderA5", "Proj14T3ReviewB7"]),
    ("shared-prefix-tie", &["Proj813BoundaryGold23", "Proj813BoundarySilver24"]),
    ("missing-suffix", &["Proj14T3ProviderA", "Proj14T3OtherB9"]),
    ("collision-a", &["Proj18Task70BS01"]),
    ("collision-b", &["Proj18Task70BS01"]),
    ("cross-project", &["Proj18Task70BS01", "Proj19Task70BS01"]),
];
fn naming_fixture_content(sid: &str) -> String {
    format!(
        "{{\"type\":\"session\",\"version\":3,\"id\":\"{sid}\",\"timestamp\":\"2026-03-04T05:06:07.000Z\",\"cwd\":\"C:\\\\SYNTHETIC\\\\naming\",\"title\":\"synthetic naming fixture\",\"parentSession\":\"naming-parent\"}}\n{{\"type\":\"message\",\"id\":\"{sid}-e1\",\"parentId\":null,\"timestamp\":\"2026-03-04T05:06:08.000Z\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"SYNTHETIC NAMING TOKEN\"}}]}}}}\n"
    )
}

pub fn run_naming(args: &[String]) -> Result<()> {
    let out = PathBuf::from(opt(args, "--out").ok_or_else(|| err("fixture-naming needs --out DIR"))?);
    let omp = out.join("omp");
    let codex = out.join("codex");
    fs::create_dir_all(&omp).map_err(|e| err(format!("create {}: {e}", omp.display())))?;
    fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;
    let mut written = 0usize;
    for (parent, names) in NAMING_FIXTURE {
        let dir = omp.join(parent);
        fs::create_dir_all(&dir).map_err(|e| err(format!("create {}: {e}", dir.display())))?;
        for name in *names {
            let sid = format!("naming-{parent}-{name}");
            let path = dir.join(format!("{name}.jsonl"));
            fs::write(&path, naming_fixture_content(&sid))
                .map_err(|e| err(format!("write {}: {e}", path.display())))?;
            written += 1;
        }
    }
    println!("wrote {written} synthetic naming fixtures to {}", out.display());
    Ok(())
}


const DELEGATION_TS: &str = "2026-03-05T06:07:08.000Z";

// Only synthetic ids and paths are used. Tuples are
// (relative path under omp/, explicit session id or None, declared parent or None).
// A `None` id is a path-derived OMP session: the decoder reads the parent id
// from the sidecar directory name, recorded as a path observation.
const DELEGATION_FIXTURE: &[(&str, Option<&str>, Option<&str>)] = &[
    ("unique/del-parent-unique.jsonl", Some("del-parent-unique"), None),
    ("unique/del-child-unique.jsonl", Some("del-child-unique"), Some("del-parent-unique")),
    ("missing/del-child-missing.jsonl", Some("del-child-missing"), Some("del-parent-absent")),
    // Adjacent files share one id: the index's first-match parent_idx must not select one.
    ("ambiguous/del-parent-dup-a.jsonl", Some("del-parent-dup"), None),
    ("ambiguous/del-parent-dup-b.jsonl", Some("del-parent-dup"), None),
    ("ambiguous/del-child-ambiguous.jsonl", Some("del-child-ambiguous"), Some("del-parent-dup")),
    (
        "path/2026-01-01_abcdefghijklmnopqrstuvwxyz1234/del-path-child.jsonl",
        None,
        None,
    ),
    (
        "path/del-path-parent.jsonl",
        Some("2026-01-01_abcdefghijklmnopqrstuvwxyz1234"),
        None,
    ),
];

fn delegation_session_line(id: &str, parent: Option<&str>) -> String {
    let parent_field = parent
        .map(|p| format!(",\"parentSession\":\"{p}\""))
        .unwrap_or_default();
    format!(
        "{{\"type\":\"session\",\"version\":3,\"id\":\"{id}\",\"timestamp\":\"{DELEGATION_TS}\",\"cwd\":\"C:\\\\SYNTHETIC\\\\delegation\",\"title\":\"synthetic delegation fixture\"{parent_field}}}\n"
    )
}

fn delegation_message_line(sid: &str) -> String {
    format!(
        "{{\"type\":\"message\",\"id\":\"{sid}-e1\",\"parentId\":null,\"timestamp\":\"{DELEGATION_TS}\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"SYNTHETIC DELEGATION TOKEN\"}}]}}}}\n"
    )
}
pub fn run_delegation(args: &[String]) -> Result<()> {
    let out = PathBuf::from(
        opt(args, "--out").ok_or_else(|| err("fixture-delegation needs --out DIR"))?,
    );
    let omp = out.join("omp");
    let codex = out.join("codex");
    fs::create_dir_all(&omp).map_err(|e| err(format!("create {}: {e}", omp.display())))?;
    fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;
    let mut written = 0usize;
    for &(rel, id, parent) in DELEGATION_FIXTURE {
        let path = omp.join(rel);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| err(format!("create {}: {e}", dir.display())))?;
        }
        let sid = id.unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("synthetic-path-derived")
        });
        let mut content = String::new();
        if let Some(id) = id {
            content.push_str(&delegation_session_line(id, parent));
        }
        content.push_str(&delegation_message_line(sid));
        fs::write(&path, content).map_err(|e| err(format!("write {}: {e}", path.display())))?;
        written += 1;
    }
    println!(
        "wrote {written} synthetic delegation fixtures to {}",
        out.display()
    );
    Ok(())
}
