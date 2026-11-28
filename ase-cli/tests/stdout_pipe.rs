//! Subprocess checks for a stdout consumer that stops reading.
//!
//! Each case starts the real CLI with a pipe, either closes the read end
//! before the first write or reads a small prefix and then closes it. A closed
//! pipe must end output quietly: no panic on stderr and the command's normal
//! exit code. The same commands are first read in full to show the normal
//! bytes and exit codes are unchanged.

use serde_json::{json, Value};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_ase");
const TOKEN: &str = "PIPE TOKEN";
const SESSION_ID: &str = "pipe-session";
const MESSAGES: usize = 700;

fn scratch_root(label: &str) -> PathBuf {
    let base = match option_env!("CARGO_TARGET_TMPDIR") {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target"),
    };
    let root = base
        .join("ase-stdout-pipe")
        .join(format!("{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    root
}

/// Synthetic index with one session and enough prose output to fill a pipe.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Fixture {
        let root = scratch_root(label);
        let omp = root.join("omp");
        let codex = root.join("codex-empty");
        fs::create_dir_all(&omp).expect("create omp fixture root");
        fs::create_dir_all(&codex).expect("create empty codex root");

        let mut journal = String::new();
        journal.push_str(
            &json!({
                "type": "session",
                "version": 3,
                "id": SESSION_ID,
                "timestamp": "2026-01-02T03:04:05.000Z",
                "cwd": "C:/SYNTHETIC/pipe",
                "title": "synthetic"
            })
            .to_string(),
        );
        journal.push('\n');
        for ordinal in 0..MESSAGES {
            journal.push_str(
                &json!({
                    "type": "message",
                    "id": format!("pipe-e{ordinal}"),
                    "parentId": null,
                    "timestamp": "2026-01-02T03:04:06.000Z",
                    "message": {
                        "role": "user",
                        "content": [{"type": "text", "text": format!("{TOKEN} synthetic message {ordinal}")}]
                    }
                })
                .to_string(),
            );
            journal.push('\n');
        }
        fs::write(omp.join("pipe.jsonl"), journal).expect("write synthetic journal");
        fs::create_dir_all(root.join("home")).expect("create isolated home");

        let fixture = Fixture { root };
        fixture.build_index();
        fixture
    }

    fn index_path(&self) -> PathBuf {
        self.root.join("ase.index")
    }

    fn command(&self, args: &[&str]) -> Command {
        let home = self.root.join("home");
        let mut command = Command::new(BIN);
        command
            .args(args)
            .current_dir(&self.root)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("APPDATA", &home)
            .env("LOCALAPPDATA", &home)
            .env("XDG_CONFIG_HOME", &home)
            .env("ASE_OMP_ROOT", self.root.join("no-omp-root"))
            .env("ASE_CODEX_ROOT", self.root.join("no-codex-root"))
            .env_remove("OPENAI_API_KEY")
            .env_remove("GEMINI_API_KEY")
            .stdin(Stdio::null());
        command
    }

    fn build_index(&self) {
        let omp = self.root.join("omp");
        let codex = self.root.join("codex-empty");
        let index = self.root.join("ase.index");
        let output = self
            .command(&[
                "index",
                "--layer",
                "all",
                "--omp-root",
                omp.to_str().expect("utf-8 fixture root"),
                "--codex-root",
                codex.to_str().expect("utf-8 fixture root"),
                "--index",
                index.to_str().expect("utf-8 fixture index"),
                "--json",
            ])
            .output()
            .expect("index subprocess must start");
        assert!(
            output.status.success(),
            "index fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).expect("index JSON");
        assert_eq!(value["files"].as_u64(), Some(1), "unexpected fixture file count: {value}");
        assert_eq!(
            value["sessions"].as_u64(),
            Some(1),
            "unexpected fixture session count: {value}"
        );
        assert_eq!(value["publication"]["status"], "ready");
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("ase subprocess must start")
    }

    fn spawn(&self, args: &[&str]) -> Child {
        self.command(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("ase subprocess must start")
    }

    /// Commands whose stdout is large enough to keep writing after a consumer
    /// closes early, with the exit code each one owns.
    fn cases(&self) -> Vec<(&'static str, Vec<String>, i32)> {
        let index_path = self.index_path();
        let index = index_path.to_str().expect("utf-8 index path");
        vec![
            ("help", vec!["--help".to_string()], 0),
            (
                "search json",
                vec![
                    "search".to_string(),
                    TOKEN.to_string(),
                    "--index".to_string(),
                    index.to_string(),
                    "--json".to_string(),
                    "--top-k".to_string(),
                    "1000".to_string(),
                ],
                0,
            ),
            (
                "search text",
                vec![
                    "search".to_string(),
                    TOKEN.to_string(),
                    "--index".to_string(),
                    index.to_string(),
                    "--text".to_string(),
                    "--top-k".to_string(),
                    "1000".to_string(),
                ],
                0,
            ),
            (
                "show text",
                vec![
                    "show".to_string(),
                    format!("session:{SESSION_ID}"),
                    "--index".to_string(),
                    index.to_string(),
                    "--limit".to_string(),
                    "1000".to_string(),
                ],
                0,
            ),
            (
                "open generation mismatch",
                vec![
                    "open".to_string(),
                    "--event".to_string(),
                    "0".to_string(),
                    "--selector".to_string(),
                    "text".to_string(),
                    "--generation".to_string(),
                    "stale-generation".to_string(),
                    "--index".to_string(),
                    index.to_string(),
                ],
                4,
            ),
        ]
    }
}

fn assert_clean(output: &Output, expected_code: i32, label: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.is_empty(),
        "{label}: unexpected stderr (exit {:?}): {stderr}",
        output.status.code()
    );
    assert_eq!(
        output.status.code(),
        Some(expected_code),
        "{label}: unexpected exit status with closed stdout, stderr: {stderr}"
    );
}

#[test]
fn test_full_stdout_read_keeps_output_and_exit_codes() {
    let fixture = Fixture::new("full");
    let index_path = fixture.index_path();
    let index = index_path.to_str().expect("utf-8 index path");

    let help = fixture.run(&["--help"]);
    assert_clean(&help, 0, "help");
    assert!(help.stdout.starts_with(b"ase (address index / derived search)"));
    assert!(help.stdout.len() > 1000, "help output unexpectedly short");

    let search_json = fixture.run(&[
        "search", TOKEN, "--index", index, "--json", "--top-k", "1000",
    ]);
    assert_clean(&search_json, 0, "search json");
    let value: Value = serde_json::from_slice(&search_json.stdout).expect("search JSON");
    assert_eq!(value["count"].as_u64(), Some(MESSAGES as u64));
    assert!(search_json.stdout.len() > 64 * 1024);

    let search_text = fixture.run(&[
        "search", TOKEN, "--index", index, "--text", "--top-k", "1000",
    ]);
    assert_clean(&search_text, 0, "search text");
    let text = String::from_utf8_lossy(&search_text.stdout);
    let header = text.lines().next().unwrap_or_default();
    assert!(
        header.starts_with(&format!("search: {MESSAGES} hit(s), generation=")),
        "text header: {header}"
    );
    assert!(search_text.stdout.len() > 64 * 1024);

    let session = format!("session:{SESSION_ID}");
    let show = fixture.run(&[
        "show",
        &session,
        "--index",
        index,
        "--limit",
        "1000",
    ]);
    assert_clean(&show, 0, "show text");
    let text = String::from_utf8_lossy(&show.stdout);
    let tail = text.lines().last().unwrap_or_default();
    assert_eq!(
        tail,
        format!("show: {} event(s), truncated=false", MESSAGES + 1)
    );

    let mismatch = fixture.run(&[
        "open",
        "--event",
        "0",
        "--selector",
        "text",
        "--generation",
        "stale-generation",
        "--index",
        index,
    ]);
    assert_clean(&mismatch, 4, "open generation mismatch");
    let value: Value = serde_json::from_slice(&mismatch.stdout).expect("open JSON");
    assert_eq!(value["status"], "generation_mismatch");
}

#[test]
fn test_closed_reader_before_write_keeps_exit_clean() {
    let fixture = Fixture::new("closed");
    for (label, args, expected_code) in fixture.cases() {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut child = fixture.spawn(&args);
        drop(child.stdout.take().expect("piped stdout"));
        let output = child.wait_with_output().expect("ase subprocess must finish");
        assert_clean(&output, expected_code, label);
    }
}

#[test]
fn test_partial_reader_close_keeps_exit_clean() {
    let fixture = Fixture::new("partial");
    for (label, args, expected_code) in fixture.cases() {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut child = fixture.spawn(&args);
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut prefix = vec![0u8; 1024];
        let read = stdout.read(&mut prefix).expect("read stdout prefix");
        assert!(read > 0, "{label}: no stdout before the consumer closed");
        drop(stdout);
        let output = child.wait_with_output().expect("ase subprocess must finish");
        assert_clean(&output, expected_code, label);
    }
}
