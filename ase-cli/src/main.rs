//! Command-line surface for the address index, derived search layer, task
//! resolution, work projection, judge and evidence-bundle layers.
//!
//! Decoding, addressing and projection derivation live in library crates; the
//! CLI only renders metadata and composes those layers. Command groups own
//! their parsing and output; this entry point reads argv, prints global help,
//! dispatches and reports process-level errors.

use ase_core::err;

mod adapter;
mod adapter_fixture;
mod analytics;
mod analytics_fixture;
mod audit;
mod audit_fixture;
mod bench;
mod bundle_digest;
mod bundle_evidence;
mod bundle_input;
mod bundle_redaction;
mod bundle_report;
mod bundle_schema;
mod bundle_selector;
mod bundle_verdicts;
mod bundle_views;
mod cli_option;
mod cli_output;
mod config;
mod export;
mod export_draft;
mod export_evidence;
mod export_guard;
mod export_policy;
mod export_query;
mod export_selection;
mod fixture;
mod fixture_dispatch;
mod import;
mod index;
mod join;
mod journal_projection;
mod judge;
mod llm;
mod llm_smoke;
mod marker_audit;
mod motif;
mod motif_fixture;
mod open;
mod resolve;
mod resolve_fixture;
mod search;
mod session_nav;
mod shingle;
mod show;
mod verify;
mod verify_index;
mod work;
mod work_fixture;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        usage();
        return;
    }
    let rest = &args[1..];
    let result = match args[0].as_str() {
        "index" => index::run(rest),
        "show" => show::run(rest),
        "resolve" => resolve::run(rest),
        "verify" => verify_index::run(rest),
        "bench-show" => bench::run_show(rest),
        "fixture" => fixture::run(rest),
        "fixture-address" => fixture::run_address(rest),
        "search" => search::run(rest),
        "open" => open::run(rest),
        "shingle" => shingle::run(rest),
        "bench-search" => bench::run_search(rest),
        "audit" => audit::run(rest),
        "fixture-naming" => audit_fixture::run_naming(rest),
        "fixture-delegation" => audit_fixture::run_delegation(rest),
        "fixture-markers" => marker_audit::run_fixture(rest),
        "fixture-resolve" => resolve_fixture::run(rest),
        "fixture-dispatch" => fixture_dispatch::run(rest),
        "fixture-delta" => work_fixture::run(rest),
        "motif" => motif::run(rest),
        "export" => export::run(rest),
        "import" => import::run(rest),
        "join" => join::run(rest),
        "adapter" => adapter::run(rest),
        "work" => work::run(rest),
        "analytics" => analytics::run(rest),
        "judge" => judge::run(rest),
        "llm" => llm::run(rest),
        "fixture-analytics" => analytics_fixture::run(rest),
        "fixture-adapter" => adapter_fixture::run(rest),
        "fixture-motif" => motif_fixture::run(rest),
        other => Err(err(format!("unknown command: {other}"))),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(2);
    }
}

fn usage() {
    println!(
        "ase (address index / derived search)\n\n\
         ase index   [--layer address|postings|all] [--omp-root DIR] [--codex-root DIR]\n\
                     [--index FILE] [--fail-before-publish] [--json]\n\
         ase show    <span:F:L:LEN|session:ID> [--role ROLE] [--ordinal A:B] [--explain] [--content] [--json]\n\
         ase resolve <session:ID|span:F:L:LEN|task:TOKEN> [--explain] [--json]\n\
         ase verify  [--all | --sample N] [--index FILE] [--json]\n\
         ase verify  <BUNDLE-DIR> [--local-map FILE] [--verdicts FILE] [--json]\n\
         ase bench-show [--n N] [--seed N] [--index FILE] [--json]\n\
         ase search  <QUERY> [--phrase] [--top-k N] [--explain] [--open-args] [--json]\n\
         ase open    --event N --selector S --generation G [--session-args] [--content] [--json]\n\
         ase shingle (--hash HASH | --event N) [--json]\n\
         ase bench-search [--n N] [--seed N] [--top-k N] [--source all|jsonl|zstd] [--probe TERM] [--json]\n\
         ase fixture --out DIR\n\
         ase audit  naming [--index FILE] [--project PROJECT] [--explain] [--json]\n\
         ase audit  delegation [--index FILE] [--explain] [--json]\n\
         ase audit  markers [--index FILE] [--explain] [--json]\n\
         ase fixture-naming --out DIR\n\
         ase fixture-delegation --out DIR\n\
         ase fixture-markers --out DIR\n\
         ase fixture-resolve --out DIR\n\
         ase fixture-dispatch --out DIR\n\
         ase fixture-delta --out DIR\n\
         ase motif  [PROJECT|TASK|TOKEN]... [--tau F] [--k-min N] [--k-max N]\n\
                     [--max-gap N] [--band N] [--max-substitutions N]\n\
                     [--null-permutations N] [--min-tasks N] [--explain] [--json]\n\
                     (PROJECT|TASK addresses one exact sampling key; a bare\n\
                     TOKEN stays ambiguous when two projects share it)\n\
         ase export motif:<id> --out DIR --local-map FILE\n\
                     (--address PROJECT|TASK ... | --addresses FILE)\n\
                     [--index FILE] [--omp-root DIR] [--codex-root DIR] [--json]\n\
                     (writes only path-free bundle files; the private alias map stays outside)\n\
         ase import verdicts FILE --bundle DIR --local-map FILE --store FILE [--json]\
                     (reopens the bundle first; accepted assertions stay in the local store)\
         ase join skill-gap --store FILE --catalog FILE --bundle DIR --local-map FILE [--verdicts FILE] [--json]\
                     (read-only exact-catalog join of verified assertions; no store write)\
         ase adapter omp-stock (--session ID | --file N | --all) [--limit N]\n\
                     [--no-sidecars] [--out CLAIMS.json] [--explain] [--json]\n\
         ase adapter omp-stock --reopen CLAIMS.json [--claim ID] [--json]\n\
         ase work project [--index FILE] [--omp-root DIR] [--start ISO --end ISO]\n\
                     [--out ROWS.jsonl] [--limit N] [--json]\n\
         ase work session (--session ID | --file N) [--leaf ENTRY_ID]\n\
                     [--omp-root DIR] [--out PROJECTION.json] [--json]\n\
         ase work reopen PROJECTION.json [--index FILE] [--json]\n\
         ase work compare --projection ROWS.jsonl --reference SOURCES.jsonl\n\
                     [--out REPORT.json] [--json]\n\
                     (leaf-aware Turn/Episode projection; task-agnostic;\n\
                     resolver and motif membership are never read or written)\n\
         ase work delta --task TOKEN [--project PROJECT] --before SESSION --after SESSION\n\
                     [--omp-root DIR] [--out DELTA.json] [--before-out F] [--after-out F]\n\
                     (explicit attempts of one dispatch-confirmed project/task;\n\
                     task order by dispatch time, not by attempt label)\n\
         ase analytics project [--index FILE] [--omp-root DIR] [--start ISO --end ISO]\n\
                     [--out ROWS.jsonl] [--report REPORT.json] [--limit N] [--json]\n\
         ase analytics session (--session ID | --file N) [--leaf ENTRY_ID]\n\
                     [--omp-root DIR] [--out ROW.json] [--json]\n\
         ase analytics reopen ROWS.jsonl [--index FILE] [--json]\n\
         ase fixture-analytics --out DIR\n\
                     (nominal stored cost, duration vs clock union, coverage;\n\
                     no repricing and no stats.db dependency)\n\
         ase fixture-adapter --out DIR\n\
                     (advisory claims only; resolver and motif membership unchanged)\n\
         ase fixture-motif --out DIR\n\
         ase judge run --root DIR --manifest FILE --units FILE --private FILE\n\
                     --store FILE [--scope primary|unknown|all] [--only-ordinal N]...\n\
                     [--limit N] [--workspace DIR] [--verification-only]\n\
                     [--slm-runner PROG --slm-arg ARG... --slm-model-id ID\n\
                      (--slm-model-sha256 HEX | --slm-weights FILE)] [...]\n\
                     [--slm-profile NAME [--slm-api-timeout-ms N]]\n\
                     [--judge-runner ... --judge-model-id ID ...]\n\
                     [--judge-profile NAME [--judge-api-timeout-ms N]]\n\
                     [--allow-remote-session-data] [--out REPORT.json]\n\
                     (API profiles come from `ase llm`; remote egress needs opt-in)\n\
         ase judge verify --root DIR --units FILE --private FILE --store FILE\n\
                     [--include-verification]   (reopen every stored evidence span)\n\
         ase judge report --store FILE [--include-verification]\n\
         ase judge show --store FILE [--unit ID] [--decision N] [--include-verification]\n\
         ase judge fixture --out DIR   (synthetic contract fixture; no real prose)\n\
         ase judge contract-runner [--label split|continue|unknown] [--fail]\n\
                     [--model-id ID] [--model-sha256 HEX]\n\
                     (local-model protocol double for cache/contract verification)\n\
         ase llm config [--json] [--profile NAME]\n\
                     (redacted local status; no network)\n\
         ase llm config set --profile NAME --provider NAME\n\
                     [--type chat-completions|responses|decision|gemini|gemini-decision] [--endpoint URL]\n\
                     [--model MODEL] [--effort minimal|low|medium|high]\n\
                     [--use] [--json]\n\
         ase llm config use --profile NAME [--json]\n\
         ase llm config enroll --profile NAME --provider NAME\n\
                     [--type ...] [--endpoint URL] [--model MODEL] [--effort ...]\n\
                     [--use] [--json]\n\
                     (non-secret settings plus one-time hidden key input)\n\
         ase llm key set [--profile NAME] [--json]   (hidden prompt; never argv)\n\
         ase llm key clear [--profile NAME] [--json]\n\
         ase llm status [--profile NAME] [--json]\n\
         ase fixture-address --out DIR"
    );
}
