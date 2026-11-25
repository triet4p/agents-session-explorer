//! `ase judge` — offline evaluation routing and rendering.
//!
//! This module owns the command router, option parsing, runner/profile
//! selection and JSON output for the judge group. Evaluation, source pinning,
//! caching and verification live in `ase-judge`. Read commands never start a
//! model; only `run` configures the two model stages.

use ase_core::{err, Result};
use ase_judge::Scope;
use crate::{has, opt};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- CLI surface

pub fn run(args: &[String]) -> Result<()> {
    let Some(sub) = args.first().map(String::as_str) else {
        usage();
        return Err(err("judge needs run, verify, report, show, fixture or contract-runner"));
    };
    let rest = &args[1..];
    if matches!(sub, "--help" | "-h") {
        usage();
        return Ok(());
    }
    match sub {
        "run" => run_evaluation(rest),
        "verify" => verify_evaluation(rest),
        "report" => report_store(rest),
        "show" => show_store(rest),
        "fixture" => write_fixture(rest),
        "contract-runner" => contract_runner(rest),
        other => Err(err(format!("unknown judge command: {other}"))),
    }
}

fn usage() {
    println!(
        "ase judge run --root DIR --manifest FILE --units FILE --private FILE\n\
         \x20           --store FILE [--scope primary|unknown|all] [--only-ordinal N]...\n\
         \x20           [--limit N] [--workspace DIR] [--verification-only]\n\
         \x20           [--slm-runner PROG --slm-arg ARG... --slm-model-id ID\n\
         \x20            (--slm-model-sha256 HEX | --slm-weights FILE)] [...]\n\
         \x20           [--slm-profile NAME [--slm-api-timeout-ms N]]\n\
         \x20           [--judge-runner ... --judge-model-id ID ...]\n\
         \x20           [--judge-profile NAME [--judge-api-timeout-ms N]]\n\
         \x20           [--allow-remote-session-data] [--out REPORT.json]\n\
         ase judge verify --root DIR --units FILE --private FILE --store FILE\n\
         \x20           [--include-verification]\n\
         ase judge report --store FILE [--include-verification]\n\
         ase judge show --store FILE [--unit ID] [--decision N] [--include-verification]\n\
         ase judge fixture --out DIR\n\
         ase judge contract-runner [--label split|continue|unknown] [--fail]\n\
         \x20           [--model-id ID] [--model-sha256 HEX]\n\n\
         No judge command calls a model on a read path. Model stages only run\n\
         under `judge run`: a workspace-local process (stdin/stdout) or a typed\n\
         API profile resolved by `ase llm`. A non-loopback API endpoint that\n\
         would receive session text is refused unless --allow-remote-session-data\n\
         is passed; the destination is printed. Use `ase llm smoke` for a fixed\n\
         synthetic call with no session data."
    );
}

fn opts(args: &[String], name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if args[index] == name {
            if let Some(value) = args.get(index + 1) {
                out.push(value.clone());
                index += 1;
            }
        }
        index += 1;
    }
    out
}

fn workspace_file_digest(path: &Path, workspace: &Path) -> Result<String> {
    let real_workspace = fs::canonicalize(workspace)
        .map_err(|e| err(format!("resolve workspace {}: {e}", workspace.display())))?;
    let real = fs::canonicalize(path)
        .map_err(|e| err(format!("resolve model weights {}: {e}", path.display())))?;
    if !real.starts_with(&real_workspace) {
        return Err(err(format!(
            "model weights must stay inside the workspace: {}",
            path.display()
        )));
    }
    let bytes = fs::read(&real)
        .map_err(|e| err(format!("read model weights {}: {e}", real.display())))?;
    Ok(ase_judge::sha256_hex(&bytes))
}

fn parse_runner_config(
    args: &[String],
    prefix: &str,
    workspace: &Path,
) -> Result<Option<ase_judge::RunnerConfig>> {
    let program = opt(args, &format!("--{prefix}-runner"));
    let model_id = opt(args, &format!("--{prefix}-model-id"));
    let digest = opt(args, &format!("--{prefix}-model-sha256"));
    let weights = opt(args, &format!("--{prefix}-weights"));
    let runner_args = opts(args, &format!("--{prefix}-arg"));
    let timeout = match opt(args, &format!("--{prefix}-timeout-ms")) {
        Some(text) => Some(
            text.parse::<u64>()
                .map_err(|_| err(format!("invalid --{prefix}-timeout-ms: {text}")))?,
        ),
        None => None,
    };
    let any = program.is_some()
        || model_id.is_some()
        || digest.is_some()
        || weights.is_some()
        || !runner_args.is_empty();
    if !any {
        return Ok(None);
    }
    let program = program.ok_or_else(|| err(format!("--{prefix}-runner is required")))?;
    let model_id = model_id.ok_or_else(|| err(format!("--{prefix}-model-id is required")))?;
    let model_sha256 = match (digest, weights) {
        (Some(digest), None) => {
            if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(err(format!(
                    "--{prefix}-model-sha256 must be a 64-hex digest"
                )));
            }
            digest.to_ascii_lowercase()
        }
        (None, Some(weights)) => {
            workspace_file_digest(Path::new(&weights), workspace)?
        }
        (Some(_), Some(_)) => {
            return Err(err(format!(
                "use either --{prefix}-model-sha256 or --{prefix}-weights, not both"
            )))
        }
        (None, None) => {
            return Err(err(format!(
                "--{prefix}-model-sha256 or --{prefix}-weights is required"
            )))
        }
    };
    Ok(Some(ase_judge::RunnerConfig {
        program,
        args: runner_args,
        model_id,
        model_sha256,
        timeout_ms: timeout.unwrap_or(ase_judge::runner::DEFAULT_TIMEOUT_MS),
    }))
}

fn parse_stage_model(
    args: &[String],
    prefix: &str,
    workspace: &Path,
) -> Result<Option<ase_judge::runner::StageModel>> {
    let profile_name = opt(args, &format!("--{prefix}-profile"));
    let process_any = opt(args, &format!("--{prefix}-runner")).is_some()
        || opt(args, &format!("--{prefix}-model-id")).is_some()
        || opt(args, &format!("--{prefix}-model-sha256")).is_some()
        || opt(args, &format!("--{prefix}-weights")).is_some()
        || !opts(args, &format!("--{prefix}-arg")).is_empty();
    if profile_name.is_some() && process_any {
        return Err(err(format!(
            "use either --{prefix}-profile or the process runner flags, not both"
        )));
    }
    if let Some(name) = profile_name {
        let timeout_ms = match opt(args, &format!("--{prefix}-api-timeout-ms")) {
            Some(text) => text.parse::<u64>().map_err(|_| {
                err(format!("invalid --{prefix}-api-timeout-ms: {text}"))
            })?,
            None => ase_llm::DEFAULT_TIMEOUT_MS,
        };
        let env = ase_llm::capture_env();
        let resolution = ase_llm::resolve(&env, Some(&name))
            .map_err(|e| err(format!("{prefix} API profile: {e}")))?;
        return Ok(Some(ase_judge::runner::StageModel::Api(
            ase_judge::runner::ApiStage {
                resolution,
                timeout_ms,
            },
        )));
    }
    if opt(args, &format!("--{prefix}-api-timeout-ms")).is_some() {
        return Err(err(format!(
            "--{prefix}-api-timeout-ms needs --{prefix}-profile"
        )));
    }
    Ok(parse_runner_config(args, prefix, workspace)?
        .map(ase_judge::runner::StageModel::Process))
}

fn run_evaluation(args: &[String]) -> Result<()> {
    let root = PathBuf::from(opt(args, "--root").ok_or_else(|| err("--root is required"))?);
    let manifest =
        PathBuf::from(opt(args, "--manifest").ok_or_else(|| err("--manifest is required"))?);
    let units = PathBuf::from(opt(args, "--units").ok_or_else(|| err("--units is required"))?);
    let private =
        PathBuf::from(opt(args, "--private").ok_or_else(|| err("--private is required"))?);
    let store = PathBuf::from(opt(args, "--store").ok_or_else(|| err("--store is required"))?);
    let workspace = match opt(args, "--workspace") {
        Some(path) => PathBuf::from(path),
        None => std::env::current_dir()
            .map_err(|e| err(format!("resolve current directory: {e}")))?,
    };
    let scope = Scope::parse(&opt(args, "--scope").unwrap_or_else(|| "primary".to_string()))
        .ok_or_else(|| err("--scope must be primary, unknown or all"))?;
    let mut only_ordinals = std::collections::BTreeSet::new();
    for value in opts(args, "--only-ordinal") {
        only_ordinals.insert(
            value
                .parse::<u32>()
                .map_err(|_| err(format!("invalid --only-ordinal: {value}")))?,
        );
    }
    let limit = match opt(args, "--limit") {
        Some(value) => Some(
            value
                .parse::<usize>()
                .map_err(|_| err(format!("invalid --limit: {value}")))?,
        ),
        None => None,
    };
    let slm = parse_stage_model(args, "slm", &workspace)?;
    let judge = parse_stage_model(args, "judge", &workspace)?;
    let allow_remote = has(args, "--allow-remote-session-data");
    for (stage, model) in [("slm", &slm), ("judge", &judge)] {
        if let Some(ase_judge::runner::StageModel::Api(api)) = model {
            let profile = &api.resolution.profile;
            if !profile.endpoint.is_loopback() {
                if !allow_remote {
                    return Err(err(format!(
                        "session data egress is not authorized for {stage}: {} at {} (provider {}); pass --allow-remote-session-data to opt in",
                        profile.api.as_str(),
                        profile.endpoint.origin,
                        profile.provider
                    )));
                }
                eprintln!(
                    "session data egress: {stage} -> provider {} api {} at {} (synthetic or pinned evaluation input)",
                    profile.provider,
                    profile.api.as_str(),
                    profile.endpoint.origin
                );
            }
        }
    }
    let options = ase_judge::EvalOptions {
        root,
        manifest,
        units,
        private,
        store,
        scope,
        only_ordinals,
        limit,
        workspace,
        verification_only: has(args, "--verification-only"),
        allow_remote_session_data: allow_remote,
        slm,
        judge,
    };
    let report = ase_judge::run(&options)?;
    let value = serde_json::to_value(&report)
        .map_err(|e| err(format!("serialize judge run report: {e}")))?;
    if let Some(out) = opt(args, "--out") {
        fs::write(&out, serde_json::to_string_pretty(&value).unwrap() + "\n")
            .map_err(|e| err(format!("write judge report {out}: {e}")))?;
    }
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
    Ok(())
}

fn verify_evaluation(args: &[String]) -> Result<()> {
    let options = ase_judge::VerifyOptions {
        root: PathBuf::from(opt(args, "--root").ok_or_else(|| err("--root is required"))?),
        units: PathBuf::from(opt(args, "--units").ok_or_else(|| err("--units is required"))?),
        private: PathBuf::from(
            opt(args, "--private").ok_or_else(|| err("--private is required"))?,
        ),
        store: PathBuf::from(opt(args, "--store").ok_or_else(|| err("--store is required"))?),
        include_verification: has(args, "--include-verification"),
    };
    let report = ase_judge::verify(&options)?;
    let value = serde_json::to_value(&report)
        .map_err(|e| err(format!("serialize judge verify report: {e}")))?;
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
    if report.status != "verified" {
        std::process::exit(3);
    }
    Ok(())
}

fn report_store(args: &[String]) -> Result<()> {
    let store = PathBuf::from(opt(args, "--store").ok_or_else(|| err("--store is required"))?);
    let value = ase_judge::report(&store, has(args, "--include-verification"))?;
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
    Ok(())
}

fn show_store(args: &[String]) -> Result<()> {
    let store = PathBuf::from(opt(args, "--store").ok_or_else(|| err("--store is required"))?);
    let unit = opt(args, "--unit");
    let decision = match opt(args, "--decision") {
        Some(value) => Some(
            value
                .parse::<u32>()
                .map_err(|_| err(format!("invalid --decision: {value}")))?,
        ),
        None => None,
    };
    let value = ase_judge::show(
        &store,
        unit.as_deref(),
        decision,
        has(args, "--include-verification"),
    )?;
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
    Ok(())
}

fn write_fixture(args: &[String]) -> Result<()> {
    let out = PathBuf::from(opt(args, "--out").ok_or_else(|| err("--out is required"))?);
    let value = ase_judge::write_fixture(&out)?;
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
    Ok(())
}

fn contract_runner(args: &[String]) -> Result<()> {
    use std::io::Read;
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| err(format!("read contract request: {e}")))?;
    if has(args, "--fail") {
        return Err(err("contract runner configured to fail"));
    }
    let request: Value = serde_json::from_str(&input)
        .map_err(|e| err(format!("parse contract request: {e}")))?;
    if request.get("protocol").and_then(Value::as_str) != Some(ase_judge::LOCAL_MODEL_PROTOCOL) {
        return Err(err("contract runner received a foreign protocol"));
    }
    let stage = request
        .get("stage")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if stage != "slm" && stage != "judge" {
        return Err(err("contract runner received an unknown stage"));
    }
    let label = opt(args, "--label").unwrap_or_else(|| "continue".to_string());
    let decision = ase_judge::Decision::parse(&label)
        .ok_or_else(|| err(format!("invalid --label: {label}")))?;
    let model_id =
        opt(args, "--model-id").unwrap_or_else(|| "ase-contract-runner".to_string());
    let model_sha256 = opt(args, "--model-sha256").unwrap_or_else(|| {
        ase_judge::sha256_hex(b"ase-contract-runner/v1")
    });
    let response = json!({
        "protocol": ase_judge::LOCAL_MODEL_PROTOCOL,
        "label": decision.as_str(),
        "confidence": "medium",
        "abstain_reason": if decision == ase_judge::Decision::Unknown {
            Some("contract_runner_abstain")
        } else {
            None
        },
        "model_id": model_id,
        "model_sha256": model_sha256,
        "stage": stage,
        "received_input_sha256": request.get("input_sha256"),
    });
    println!("{}", serde_json::to_string(&response).unwrap());
    Ok(())
}
