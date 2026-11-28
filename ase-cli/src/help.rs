//! Context-aware help for the CLI command tree.
//!
//! A help request is decided from argv alone. It is recognized only where a
//! flag is expected and only on a known command path, then prints that
//! command's usage to stdout. It never opens an index, reads configuration or
//! credentials, prompts, writes a file or opens a socket. Value positions are
//! skipped, so a value that resembles a help flag stays an ordinary value.

use crate::cli_output::outln;

const TOP_LEVEL: &[&str] = &[
    "index", "show", "resolve", "verify", "bench-show", "fixture", "fixture-address",
    "search", "open", "shingle", "bench-search", "audit", "fixture-naming",
    "fixture-delegation", "fixture-markers", "fixture-resolve", "fixture-dispatch",
    "fixture-delta", "motif", "export", "import", "join", "adapter", "work",
    "analytics", "judge", "llm", "fixture-analytics", "fixture-adapter",
    "fixture-motif",
];

const CHILDREN: &[(&[&str], &[&str])] = &[
    (&["audit"], &["naming", "delegation", "markers"]),
    (&["adapter"], &["omp-stock"]),
    (&["work"], &["project", "session", "reopen", "compare", "delta"]),
    (&["analytics"], &["project", "session", "reopen"]),
    (&["import"], &["verdicts"]),
    (&["join"], &["skill-gap"]),
    (
        &["judge"],
        &["run", "verify", "report", "show", "fixture", "contract-runner"],
    ),
    (&["llm"], &["config", "key", "status", "smoke"]),
    (&["llm", "config"], &["set", "use", "enroll"]),
    (&["llm", "key"], &["set", "clear"]),
];

/// Flags whose next token is a value. A help-shaped token in that position is
/// not a help request; it must reach the command's own validation.
const VALUE_FLAGS: &[&str] = &[
    "--layer", "--omp-root", "--codex-root", "--index", "--sample", "--n",
    "--seed", "--top-k", "--event", "--selector", "--generation", "--hash",
    "--source", "--probe", "--out", "--project", "--role", "--ordinal", "--tau",
    "--k-min", "--k-max", "--max-gap", "--band", "--max-substitutions",
    "--null-permutations", "--min-tasks", "--address", "--addresses",
    "--local-map", "--verdicts", "--bundle", "--store", "--catalog", "--limit",
    "--reopen", "--claim", "--start", "--end", "--leaf", "--before", "--after",
    "--task", "--reference", "--report", "--scope", "--only-ordinal",
    "--workspace", "--slm-runner", "--slm-arg", "--slm-model-id",
    "--slm-model-sha256", "--slm-weights", "--slm-profile",
    "--slm-api-timeout-ms", "--judge-runner", "--judge-model-id",
    "--judge-profile", "--judge-api-timeout-ms", "--profile", "--provider",
    "--type", "--endpoint", "--model", "--effort", "--timeout-ms", "--label",
];

fn children_of(path: &[&str]) -> Option<&'static [&'static str]> {
    CHILDREN
        .iter()
        .find(|(parent, _)| *parent == path)
        .map(|(_, children)| *children)
}

/// Consume only the command names at the head of argv. An unknown group child
/// marks the path invalid so `--help` cannot turn a bad command into success.
fn command_path(args: &[String]) -> (Vec<&str>, bool) {
    let mut path: Vec<&str> = Vec::new();
    let Some(first) = args.first() else {
        return (path, false);
    };
    if !TOP_LEVEL.contains(&first.as_str()) {
        return (path, true);
    }
    path.push(first.as_str());
    let mut at = 1;
    while let Some(children) = children_of(&path) {
        let Some(next) = args.get(at) else {
            break;
        };
        if next.starts_with('-') {
            break;
        }
        if !children.contains(&next.as_str()) {
            return (path, true);
        }
        path.push(next.as_str());
        at += 1;
    }
    (path, false)
}

/// Look for a help flag at flag positions after `start`.
fn help_in_flags(args: &[String], start: usize) -> bool {
    let mut at = start;
    let mut positional_only = false;
    while at < args.len() {
        let token = args[at].as_str();
        if positional_only {
            at += 1;
            continue;
        }
        if token == "--" {
            positional_only = true;
            at += 1;
            continue;
        }
        if token == "--help" || token == "-h" {
            return true;
        }
        if VALUE_FLAGS.contains(&token) {
            at += 2;
            continue;
        }
        at += 1;
    }
    false
}

/// Grammar-aware replacement for a raw `--help` search inside a command.
pub(crate) fn has_help(args: &[String]) -> bool {
    help_in_flags(args, 0)
}

/// Print help for the known command path in argv, if argv asks for it.
pub(crate) fn try_print(args: &[String]) -> bool {
    let Some(first) = args.first() else {
        return false;
    };
    if first == "--help" || first == "-h" {
        crate::usage();
        return true;
    }
    let (path, invalid) = command_path(args);
    if invalid || !help_in_flags(args, path.len()) {
        return false;
    }
    print_usage(&path);
    true
}

const VERIFY_USAGE: &str = "ase verify [--all | --sample N] [--index FILE] [--json]\n\
     ase verify <BUNDLE-DIR> [--local-map FILE] [--verdicts FILE] [--json]\n\
     (all/sample re-read pinned spans; a positional directory reopens an evidence bundle)";
const SHOW_USAGE: &str = "ase show <span:F:L:LEN|session:ID> [--role ROLE] [--ordinal A:B]\n\
     [--explain] [--content] [--generation G] [--json]\n\
     (--generation G pins the active publication; a stale locator exits 4)";
const RESOLVE_USAGE: &str =
    "ase resolve <session:ID|span:F:L:LEN|task:TOKEN> [--explain] [--generation G] [--json]\n\
     (--generation G pins the active publication; a stale locator exits 4)";
const BENCH_SHOW_USAGE: &str = "ase bench-show [--n N] [--seed N] [--index FILE] [--json]";
const BENCH_SEARCH_USAGE: &str =
    "ase bench-search [--n N] [--seed N] [--top-k N] [--source all|jsonl|zstd] [--probe TERM] [--json]";
const SHINGLE_USAGE: &str = "ase shingle (--hash HASH | --event N) [--json]";
const FIXTURE_USAGE: &str = "ase fixture --out DIR";
const FIXTURE_ADDRESS_USAGE: &str = "ase fixture-address --out DIR";
const AUDIT_USAGE: &str = "ase audit naming [--index FILE] [--project PROJECT] [--explain] [--json]\n\
     ase audit delegation [--index FILE] [--explain] [--json]\n\
     ase audit markers [--index FILE] [--explain] [--json]";
const AUDIT_NAMING_USAGE: &str = "ase audit naming [--index FILE] [--project PROJECT] [--explain] [--json]";
const AUDIT_DELEGATION_USAGE: &str = "ase audit delegation [--index FILE] [--explain] [--json]";
const AUDIT_MARKERS_USAGE: &str = "ase audit markers [--index FILE] [--explain] [--json]";
const MOTIF_USAGE: &str = "ase motif [PROJECT|TASK|TOKEN]... [--tau F] [--k-min N] [--k-max N]\n\
     [--max-gap N] [--band N] [--max-substitutions N]\n\
     [--null-permutations N] [--min-tasks N] [--explain] [--json]\n\
     (PROJECT|TASK addresses one exact sampling key; a bare\n\
     TOKEN stays ambiguous when two projects share it)";
const EXPORT_USAGE: &str = "ase export motif:<id> --out DIR --local-map FILE\n\
     (--address PROJECT|TASK ... | --addresses FILE)\n\
     [--index FILE] [--omp-root DIR] [--codex-root DIR] [--json]\n\
     (writes only path-free bundle files; the private alias map stays outside)";
const IMPORT_USAGE: &str = "ase import verdicts FILE --bundle DIR --local-map FILE --store FILE [--json]\n\
     (reopens the bundle first; accepted assertions stay in the local store)";
const JOIN_USAGE: &str = "ase join skill-gap --store FILE --catalog FILE --bundle DIR --local-map FILE\n\
     [--verdicts FILE] [--json]\n\
     (read-only exact-catalog join of verified assertions; no store write)";
const ADAPTER_USAGE: &str = "ase adapter omp-stock (--session ID | --file N | --all) [--limit N]\n\
     [--no-sidecars] [--out CLAIMS.json] [--explain] [--json]\n\
     ase adapter omp-stock --reopen CLAIMS.json [--claim ID] [--json]\n\
     (scan selects parent files; reopen reads saved advisory claims)";
const WORK_PROJECT_USAGE: &str =
    "ase work project [--index FILE] [--omp-root DIR] [--start ISO --end ISO]\n\
     [--out ROWS.jsonl] [--limit N] [--json]";
const WORK_SESSION_USAGE: &str =
    "ase work session (--session ID | --file N) [--leaf ENTRY_ID]\n\
     [--generation G] [--omp-root DIR] [--out PROJECTION.json] [--json]\n\
     (--generation G pins the active publication; a stale locator exits 4)";
const WORK_REOPEN_USAGE: &str = "ase work reopen PROJECTION.json [--index FILE] [--json]";
const WORK_COMPARE_USAGE: &str =
    "ase work compare --projection ROWS.jsonl --reference SOURCES.jsonl\n\
     [--out REPORT.json] [--json]";
const WORK_DELTA_USAGE: &str = "ase work delta --task TOKEN [--project PROJECT] --before SESSION --after SESSION\n\
     [--omp-root DIR] [--out DELTA.json] [--before-out F] [--after-out F]\n\
     (explicit attempts of one dispatch-confirmed project/task; task order by dispatch time)";
const ANALYTICS_PROJECT_USAGE: &str =
    "ase analytics project [--index FILE] [--omp-root DIR] [--start ISO --end ISO]\n\
     [--out ROWS.jsonl] [--report REPORT.json] [--limit N] [--json]";
const ANALYTICS_SESSION_USAGE: &str =
    "ase analytics session (--session ID | --file N) [--leaf ENTRY_ID]\n\
     [--omp-root DIR] [--out ROW.json] [--json]";
const ANALYTICS_REOPEN_USAGE: &str = "ase analytics reopen ROWS.jsonl [--index FILE] [--json]";
const FIXTURE_ANALYTICS_USAGE: &str =
    "ase fixture-analytics --out DIR\n\
     (nominal stored cost, duration vs clock union, coverage; no repricing and no stats.db dependency)";
const FIXTURE_ADAPTER_USAGE: &str =
    "ase fixture-adapter --out DIR\n\
     (advisory claims only; resolver and motif membership unchanged)";
const FIXTURE_MOTIF_USAGE: &str = "ase fixture-motif --out DIR";
const JUDGE_RUN_USAGE: &str = "ase judge run --root DIR --manifest FILE --units FILE --private FILE\n\
     --store FILE [--scope primary|unknown|all] [--only-ordinal N]... [--limit N]\n\
     [--workspace DIR] [--verification-only]\n\
     [--slm-runner PROG --slm-arg ARG... --slm-model-id ID\n\
      (--slm-model-sha256 HEX | --slm-weights FILE)] [...]\n\
     [--slm-profile NAME [--slm-api-timeout-ms N]]\n\
     [--judge-runner ... --judge-model-id ID ...]\n\
     [--judge-profile NAME [--judge-api-timeout-ms N]]\n\
     [--allow-remote-session-data] [--out REPORT.json]\n\
     (API profiles come from `ase llm`; remote egress needs opt-in)";
const JUDGE_VERIFY_USAGE: &str =
    "ase judge verify --root DIR --units FILE --private FILE --store FILE\n\
     [--include-verification]   (reopen every stored evidence span)";
const JUDGE_REPORT_USAGE: &str = "ase judge report --store FILE [--include-verification]";
const JUDGE_SHOW_USAGE: &str =
    "ase judge show --store FILE [--unit ID] [--decision N] [--include-verification]";
const JUDGE_FIXTURE_USAGE: &str = "ase judge fixture --out DIR   (synthetic contract fixture; no real prose)";
const JUDGE_CONTRACT_RUNNER_USAGE: &str =
    "ase judge contract-runner [--label split|continue|unknown] [--fail]\n\
     [--model-id ID] [--model-sha256 HEX]\n\
     (local-model protocol double for cache/contract verification)";
const LLM_CONFIG_SET_USAGE: &str = "ase llm config set --profile NAME --provider NAME\n\
     [--type chat-completions|responses|decision|gemini|gemini-decision] [--endpoint URL]\n\
     [--model MODEL] [--effort none|minimal|low|medium|high] [--use] [--json]";
const LLM_CONFIG_USE_USAGE: &str = "ase llm config use --profile NAME [--json]";
const LLM_CONFIG_ENROLL_USAGE: &str = "ase llm config enroll --profile NAME --provider NAME\n\
     [--type chat-completions|responses|decision|gemini|gemini-decision] [--endpoint URL]\n\
     [--model MODEL] [--effort none|minimal|low|medium|high] [--use] [--json]\n\
     (non-secret settings plus one-time hidden key input)";
const LLM_KEY_USAGE: &str = "ase llm key set [--profile NAME] [--json]   (hidden prompt; never argv)\n\
     ase llm key clear [--profile NAME] [--json]";
const LLM_KEY_SET_USAGE: &str = "ase llm key set [--profile NAME] [--json]   (hidden prompt; never argv)";
const LLM_KEY_CLEAR_USAGE: &str = "ase llm key clear [--profile NAME] [--json]";
const LLM_STATUS_USAGE: &str = "ase llm status [--profile NAME] [--json]";
const LLM_SMOKE_USAGE: &str =
    "ase llm smoke [--profile NAME] [--timeout-ms N] [--json]\n\
     (one fixed synthetic call; no session or journal data)";

/// Print the usage belonging to one command path. Shared modules expose their
/// own text so the global and context-specific help cannot drift apart.
pub(crate) fn print_usage(path: &[&str]) {
    match path {
        ["index"] => crate::index::index_usage(),
        ["search"] => crate::search::search_usage(),
        ["open"] => crate::open::open_usage(),
        ["work"] => crate::work::work_usage(),
        ["analytics"] => crate::analytics::analytics_usage(),
        ["judge"] => crate::judge::usage(),
        ["llm"] | ["llm", "config"] => crate::llm::llm_usage(),
        ["show"] => outln!("{SHOW_USAGE}"),
        ["resolve"] => outln!("{RESOLVE_USAGE}"),
        ["verify"] => outln!("{VERIFY_USAGE}"),
        ["bench-show"] => outln!("{BENCH_SHOW_USAGE}"),
        ["fixture"] => outln!("{FIXTURE_USAGE}"),
        ["fixture-address"] => outln!("{FIXTURE_ADDRESS_USAGE}"),
        ["shingle"] => outln!("{SHINGLE_USAGE}"),
        ["bench-search"] => outln!("{BENCH_SEARCH_USAGE}"),
        ["audit"] => outln!("{AUDIT_USAGE}"),
        ["audit", "naming"] => outln!("{AUDIT_NAMING_USAGE}"),
        ["audit", "delegation"] => outln!("{AUDIT_DELEGATION_USAGE}"),
        ["audit", "markers"] => outln!("{AUDIT_MARKERS_USAGE}"),
        [
            "fixture-naming" | "fixture-delegation" | "fixture-markers" | "fixture-resolve"
            | "fixture-dispatch" | "fixture-delta",
        ] => outln!("ase {} --out DIR", path[0]),
        ["motif"] => outln!("{MOTIF_USAGE}"),
        ["export"] => outln!("{EXPORT_USAGE}"),
        ["import"] | ["import", "verdicts"] => outln!("{IMPORT_USAGE}"),
        ["join"] | ["join", "skill-gap"] => outln!("{JOIN_USAGE}"),
        ["adapter"] | ["adapter", "omp-stock"] => outln!("{ADAPTER_USAGE}"),
        ["work", "project"] => outln!("{WORK_PROJECT_USAGE}"),
        ["work", "session"] => outln!("{WORK_SESSION_USAGE}"),
        ["work", "reopen"] => outln!("{WORK_REOPEN_USAGE}"),
        ["work", "compare"] => outln!("{WORK_COMPARE_USAGE}"),
        ["work", "delta"] => outln!("{WORK_DELTA_USAGE}"),
        ["analytics", "project"] => outln!("{ANALYTICS_PROJECT_USAGE}"),
        ["analytics", "session"] => outln!("{ANALYTICS_SESSION_USAGE}"),
        ["analytics", "reopen"] => outln!("{ANALYTICS_REOPEN_USAGE}"),
        ["fixture-analytics"] => outln!("{FIXTURE_ANALYTICS_USAGE}"),
        ["fixture-adapter"] => outln!("{FIXTURE_ADAPTER_USAGE}"),
        ["fixture-motif"] => outln!("{FIXTURE_MOTIF_USAGE}"),
        ["judge", "run"] => outln!("{JUDGE_RUN_USAGE}"),
        ["judge", "verify"] => outln!("{JUDGE_VERIFY_USAGE}"),
        ["judge", "report"] => outln!("{JUDGE_REPORT_USAGE}"),
        ["judge", "show"] => outln!("{JUDGE_SHOW_USAGE}"),
        ["judge", "fixture"] => outln!("{JUDGE_FIXTURE_USAGE}"),
        ["judge", "contract-runner"] => outln!("{JUDGE_CONTRACT_RUNNER_USAGE}"),
        ["llm", "config", "set"] => outln!("{LLM_CONFIG_SET_USAGE}"),
        ["llm", "config", "use"] => outln!("{LLM_CONFIG_USE_USAGE}"),
        ["llm", "config", "enroll"] => outln!("{LLM_CONFIG_ENROLL_USAGE}"),
        ["llm", "key"] => outln!("{LLM_KEY_USAGE}"),
        ["llm", "key", "set"] => outln!("{LLM_KEY_SET_USAGE}"),
        ["llm", "key", "clear"] => outln!("{LLM_KEY_CLEAR_USAGE}"),
        ["llm", "status"] => outln!("{LLM_STATUS_USAGE}"),
        ["llm", "smoke"] => outln!("{LLM_SMOKE_USAGE}"),
        _ => crate::usage(),
    }
}

#[cfg(test)]
mod tests {
    use super::{command_path, has_help};

    fn args(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|token| token.to_string()).collect()
    }

    #[test]
    fn test_help_is_ignored_in_a_value_position() {
        assert!(!has_help(&args(&["--event", "--help", "--selector", "S"])));
        assert!(!has_help(&args(&["--index", "--help"])));
        assert!(has_help(&args(&["--index", "missing", "--help"])));
        assert!(!has_help(&args(&["--", "--help"])));
    }

    #[test]
    fn test_command_path_stops_on_unknown_group_child() {
        let bad = args(&["work", "not-a-leaf", "--help"]);
        let (path, invalid) = command_path(&bad);
        assert_eq!(path, vec!["work"]);
        assert!(invalid);
        let good = args(&["llm", "config", "set", "--help"]);
        let (path, invalid) = command_path(&good);
        assert_eq!(path, vec!["llm", "config", "set"]);
        assert!(!invalid);
    }
}
