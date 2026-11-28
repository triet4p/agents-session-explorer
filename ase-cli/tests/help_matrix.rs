//! Subprocess matrix for read-only help across the whole CLI command tree.
//!
//! Every dispatched command path is asked for `--help` and `-h` from an empty
//! working directory, both bare and with canary paths that must stay untouched.
//! Unknown commands and help-shaped values must keep their normal errors.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_ase");

type Case = (&'static [&'static str], &'static str);

fn cases() -> Vec<Case> {
    vec![
        (&[], "ase (address index / derived search)"),
        (&["index"], "ase index [--layer address|postings|all]"),
        (&["show"], "ase show <span:F:L:LEN|session:ID>"),
        (&["resolve"], "ase resolve <session:ID|span:F:L:LEN|task:TOKEN>"),
        (&["verify"], "ase verify [--all | --sample N]"),
        (&["bench-show"], "ase bench-show [--n N]"),
        (&["fixture"], "ase fixture --out DIR"),
        (&["fixture-address"], "ase fixture-address --out DIR"),
        (&["search"], "ase search <QUERY>"),
        (&["open"], "ase open --event N"),
        (&["shingle"], "ase shingle (--hash HASH | --event N)"),
        (&["bench-search"], "ase bench-search [--n N]"),
        (&["audit"], "ase audit naming"),
        (&["audit", "naming"], "ase audit naming [--index FILE]"),
        (&["audit", "delegation"], "ase audit delegation [--index FILE]"),
        (&["audit", "markers"], "ase audit markers [--index FILE]"),
        (&["fixture-naming"], "ase fixture-naming --out DIR"),
        (&["fixture-delegation"], "ase fixture-delegation --out DIR"),
        (&["fixture-markers"], "ase fixture-markers --out DIR"),
        (&["fixture-resolve"], "ase fixture-resolve --out DIR"),
        (&["fixture-dispatch"], "ase fixture-dispatch --out DIR"),
        (&["fixture-delta"], "ase fixture-delta --out DIR"),
        (&["motif"], "ase motif [PROJECT|TASK|TOKEN]"),
        (&["export"], "ase export motif:<id>"),
        (&["import"], "ase import verdicts FILE"),
        (&["import", "verdicts"], "ase import verdicts FILE"),
        (&["join"], "ase join skill-gap"),
        (&["join", "skill-gap"], "ase join skill-gap"),
        (&["adapter"], "ase adapter omp-stock"),
        (&["adapter", "omp-stock"], "ase adapter omp-stock"),
        (&["work"], "ase work project"),
        (&["work", "project"], "ase work project [--index FILE]"),
        (&["work", "session"], "ase work session (--session ID | --file N)"),
        (&["work", "reopen"], "ase work reopen PROJECTION.json"),
        (&["work", "compare"], "ase work compare --projection ROWS.jsonl"),
        (&["work", "delta"], "ase work delta --task TOKEN"),
        (&["analytics"], "ase analytics project"),
        (&["analytics", "project"], "ase analytics project [--index FILE]"),
        (
            &["analytics", "session"],
            "ase analytics session (--session ID | --file N)",
        ),
        (&["analytics", "reopen"], "ase analytics reopen ROWS.jsonl"),
        (&["judge"], "ase judge run --root DIR"),
        (&["judge", "run"], "ase judge run --root DIR"),
        (&["judge", "verify"], "ase judge verify --root DIR"),
        (&["judge", "report"], "ase judge report --store FILE"),
        (&["judge", "show"], "ase judge show --store FILE"),
        (&["judge", "fixture"], "ase judge fixture --out DIR"),
        (&["judge", "contract-runner"], "ase judge contract-runner"),
        (&["llm"], "ase llm config"),
        (&["llm", "config"], "ase llm config [--json]"),
        (&["llm", "config", "set"], "ase llm config set --profile NAME"),
        (&["llm", "config", "use"], "ase llm config use --profile NAME"),
        (
            &["llm", "config", "enroll"],
            "ase llm config enroll --profile NAME",
        ),
        (&["llm", "key"], "ase llm key set"),
        (&["llm", "key", "set"], "ase llm key set [--profile NAME]"),
        (&["llm", "key", "clear"], "ase llm key clear [--profile NAME]"),
        (&["llm", "status"], "ase llm status [--profile NAME]"),
        (&["llm", "smoke"], "ase llm smoke [--profile NAME]"),
        (&["fixture-analytics"], "ase fixture-analytics --out DIR"),
        (&["fixture-adapter"], "ase fixture-adapter --out DIR"),
        (&["fixture-motif"], "ase fixture-motif --out DIR"),
    ]
}

fn scratch_root() -> PathBuf {
    match option_env!("CARGO_TARGET_TMPDIR") {
        Some(dir) => PathBuf::from(dir).join("ase-help-matrix"),
        None => Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target")
            .join("ase-help-matrix"),
    }
    .join(format!("p{}", std::process::id()))
}

fn invoke(args: &[&str], cwd: &Path, home: &Path) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("APPDATA", home)
        .env("LOCALAPPDATA", home)
        .env("XDG_CONFIG_HOME", home)
        .env("ASE_OMP_ROOT", cwd.join("no-omp-root"))
        .env("ASE_CODEX_ROOT", cwd.join("no-codex-root"))
        .env_remove("OPENAI_API_KEY")
        .env_remove("GEMINI_API_KEY")
        .output()
        .expect("ase subprocess must start")
}

fn assert_no_files(dir: &Path, label: &str) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot inspect {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(entries.is_empty(), "{label} left files in {}: {entries:?}", dir.display());
}

fn assert_usage(output: &Output, expected: &str, label: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{label} exit: {:?}; stderr: {stderr}", output.status.code());
    assert!(stderr.is_empty(), "{label} wrote stderr: {stderr}");
    assert!(stdout.contains(expected), "{label} missed {expected:?}: {stdout}");
    assert!(!stdout.contains("error:"), "{label} reported an error: {stdout}");
}

#[test]
fn test_help_is_read_only_at_every_command_level() {
    let scratch = scratch_root();
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).expect("scratch root");
    let home = scratch.join("home");
    fs::create_dir_all(&home).expect("isolated home");

    let mut checked = 0usize;
    for (path, expected) in cases() {
        for flag in ["--help", "-h"] {
            for canary in [false, true] {
                let cwd = scratch.join(format!("case-{checked:03}"));
                fs::create_dir_all(&cwd).expect("empty case cwd");
                let missing_index = scratch.join("canary-does-not-exist/ase.index");
                let missing_dir = scratch.join("canary-empty-root");
                let mut args = path.to_vec();
                args.push(flag);
                if canary {
                    args.extend([
                        "--index",
                        missing_index.to_str().expect("utf-8 scratch path"),
                        missing_dir.to_str().expect("utf-8 scratch path"),
                    ]);
                }
                let label = format!("ase {} {flag} canary={canary}", path.join(" "));
                let output = invoke(&args, &cwd, &home);
                assert_usage(&output, expected, &label);
                if canary {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    assert!(
                        !stdout.contains("canary-does-not-exist")
                            && !stdout.contains("canary-empty-root"),
                        "{label} echoed a canary path: {stdout}"
                    );
                    assert!(!missing_index.exists(), "{label} created the canary index");
                    assert!(!missing_dir.exists(), "{label} created the canary root");
                }
                assert_no_files(&cwd, &label);
                assert_no_files(&home, &label);
                fs::remove_dir_all(&cwd).expect("remove case cwd");
                checked += 1;
            }
        }
    }

    // Both verify modes and both adapter modes stay in their context help.
    let verify = invoke(&["verify", "--help"], &home, &home);
    let verify_out = String::from_utf8_lossy(&verify.stdout);
    assert!(verify_out.contains("ase verify <BUNDLE-DIR>"), "{verify_out}");
    let adapter = invoke(&["adapter", "omp-stock", "-h"], &home, &home);
    let adapter_out = String::from_utf8_lossy(&adapter.stdout);
    assert!(adapter_out.contains("--reopen"), "{adapter_out}");
    assert!(adapter_out.contains("--all"), "{adapter_out}");

    let _ = fs::remove_dir_all(&scratch);
}

#[test]
fn test_unknown_commands_and_help_shaped_values_keep_their_errors() {
    let scratch = scratch_root().join("negative");
    let home = scratch_root().join("negative-home");
    let _ = fs::remove_dir_all(&scratch);
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&scratch).expect("negative scratch");
    fs::create_dir_all(&home).expect("isolated home");

    let wrong_commands: [(&[&str], &str); 4] = [
        (&["not-a-command", "--help"], "unknown command"),
        (&["work", "not-a-leaf", "--help"], "unknown work command"),
        (
            &["llm", "config", "not-a-leaf", "--help"],
            "unknown llm config command",
        ),
        (&["judge", "not-a-leaf", "-h"], "unknown judge command"),
    ];
    for (args, message) in wrong_commands {
        let output = invoke(args, &scratch, &home);
        assert!(!output.status.success(), "{args:?} unexpectedly succeeded");
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stderr.contains(message), "{args:?} stderr: {stderr}");
        assert!(!stdout.contains("Usage") && !stdout.contains("ase --help"), "{args:?} stdout: {stdout}");
    }

    // `--help` after a value-taking flag is a value, not a help request.
    let value_cases: [&[&str]; 3] = [
        &["open", "--event", "--help", "--selector", "S", "--generation", "G"],
        &["search", "--index", "--help", "query"],
        &["index", "--layer", "bogus", "--index", "--help"],
    ];
    for args in value_cases {
        let output = invoke(args, &scratch, &home);
        assert!(!output.status.success(), "{args:?} unexpectedly succeeded");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stdout.contains("ase index [--layer"), "{args:?} printed usage: {stdout}");
        assert!(!stdout.contains("ase open --event N"), "{args:?} printed usage: {stdout}");
        assert!(!stderr.is_empty(), "{args:?} had no error: {stderr}");
    }
    assert_no_files(&scratch, "negative cases");
    assert_no_files(&home, "negative cases");

    let _ = fs::remove_dir_all(&home);
    let _ = fs::remove_dir_all(&scratch);
}
