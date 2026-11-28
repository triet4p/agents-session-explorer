//! `ase llm` — local configuration, credential enrollment and status.
//!
//! This module owns the command router, strict option validation and the
//! config/key/status handlers. Those commands are local and open no socket.
//! Non-secret settings and the credential store belong to `ase-llm`; the
//! smoke command lives in `llm_smoke` because it is the only route that
//! opens a socket.
//!
//! The key is only ever read from hidden stdin. Every mutating/status command
//! validates its flags strictly and never echoes a value that could have been
//! a pasted secret; the status/JSON paths are built from explicit non-secret
//! fields.

use ase_core::{err, Result};
use crate::cli_option::{has, opt};
use crate::cli_output::outln;
use serde_json::json;

pub fn run(args: &[String]) -> Result<()> {
    let Some(sub) = args.first().map(String::as_str) else {
        llm_usage();
        return Ok(());
    };
    if matches!(sub, "--help" | "-h") {
        llm_usage();
        return Ok(());
    }
    let rest = &args[1..];
    match sub {
        "config" => cmd_llm_config(rest),
        "key" => cmd_llm_key(rest),
        "status" => cmd_llm_status(rest),
        "smoke" => crate::llm_smoke::run(rest),
        _ => Err(err("unknown llm command (see `ase llm --help`)")),
    }
}

pub(crate) fn llm_usage() {
    outln!(
        "ase llm config [--json] [--profile NAME]\n\
         ase llm config set --profile NAME --provider NAME\n\
         \x20           [--type chat-completions|responses|decision|gemini|gemini-decision] [--endpoint URL]\n\
         \x20           [--model MODEL] [--effort none|minimal|low|medium|high]\n\
         \x20           [--use] [--json]\n\
         ase llm config use --profile NAME [--json]\n\
         ase llm config enroll --profile NAME --provider NAME\n\
         \x20           [--type ...] [--endpoint URL] [--model MODEL] [--effort ...]\n\
         \x20           [--use] [--json]\n\
         ase llm key set [--profile NAME] [--json]   (hidden prompt; never argv)\n\
         ase llm key clear [--profile NAME] [--json]\n\
         ase llm status [--profile NAME] [--json]\n\
         ase llm smoke [--profile NAME] [--timeout-ms N] [--json]\n\
         \x20           (one fixed synthetic call; no session or journal data)\n\n\
         Non-secret settings resolve in this order: profile/config file, then\n\
         ASE_LLM_PROVIDER, OPENAI_API_TYPE, OPENAI_API_ENDPOINT,\n\
         OPENAI_API_MODEL and OPENAI_API_EFFORT. The credential is the typed\n\
         API family's process variable when nonempty (OPENAI_API_KEY or\n\
         GEMINI_API_KEY), otherwise the stored key for the exact\n\
         (provider, normalized origin) pair.\n\
         Only `ase llm smoke` sends a request. Every other `ase llm` command is\n\
         local and opens no socket."
    );
}

/// Strict option validation. Values are never printed back: an unexpected
/// token is rejected without echoing it, so `ase llm key set --key SECRET`
/// cannot place the secret in the error path.
pub(crate) fn llm_expect(args: &[String], value_flags: &[&str], bool_flags: &[&str]) -> Result<()> {
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let Some(flag) = arg.strip_prefix("--") else {
            return Err(err(
                "unexpected argument: the API key is only read from a hidden prompt, never from argv",
            ));
        };
        if flag.contains('=') {
            return Err(err(
                "llm options must be passed as `--flag VALUE`; `--flag=value` is not accepted",
            ));
        }
        let name = format!("--{flag}");
        if bool_flags.iter().any(|allowed| *allowed == name) {
            index += 1;
            continue;
        }
        if value_flags.iter().any(|allowed| *allowed == name) {
            let value = args.get(index + 1).ok_or_else(|| {
                err(format!("llm option {name} needs a value"))
            })?;
            if value.starts_with("--") {
                return Err(err(format!("llm option {name} needs a value")));
            }
            index += 2;
            continue;
        }
        return Err(err("unknown llm option (see `ase llm --help`)"));
    }
    Ok(())
}

fn cmd_llm_config(args: &[String]) -> Result<()> {
    let Some(sub) = args.first().map(String::as_str) else {
        return print_llm_status(None, false, false);
    };
    if matches!(sub, "--help" | "-h") {
        llm_usage();
        return Ok(());
    }
    if sub.starts_with("--") {
        llm_expect(args, &["--profile"], &["--json"])?;
        return print_llm_status(opt(args, "--profile"), has(args, "--json"), false);
    }
    let rest = &args[1..];
    match sub {
        "set" => {
            llm_expect(
                rest,
                &[
                    "--profile", "--provider", "--type", "--endpoint", "--model",
                    "--effort",
                ],
                &["--json", "--use"],
            )?;
            let env = ase_llm::capture_env();
            let mut config = ase_llm::load_config(&env)?
                .config
                .unwrap_or_else(ase_llm::ConfigFile::empty);
            let name = llm_profile_name(rest);
            let (provider, api, endpoint, model, effort) = llm_settings(rest)?;
            ase_llm::apply_edit(
                &mut config,
                &ase_llm::ProfileEdit {
                    profile: &name,
                    provider: provider.as_deref(),
                    api,
                    endpoint: endpoint.as_deref(),
                    model: model.as_deref(),
                    effort,
                    make_active: has(rest, "--use"),
                },
            )?;
            let path = ase_llm::save_config(&env, &config)?;
            if has(rest, "--json") {
                let value = json!({
                    "saved": true,
                    "profile": name,
                    "provider": config.profiles.get(&name).map(|p| p.provider.clone()),
                    "active": config.active,
                    "config_path": path.display().to_string(),
                });
                outln!("{}", serde_json::to_string_pretty(&value).unwrap());
            } else {
                outln!("saved profile {name} in {}", path.display());
                outln!(
                    "next: run `ase llm key set --profile {name}` in a terminal (hidden input)"
                );
            }
            Ok(())
        }
        "use" => {
            llm_expect(rest, &["--profile"], &["--json"])?;
            let name = opt(rest, "--profile").ok_or_else(|| err("--profile is required"))?;
            let env = ase_llm::capture_env();
            let mut config = ase_llm::load_config(&env)?
                .config
                .ok_or_else(|| err("llm_config_missing: run `ase llm config set` first"))?;
            ase_llm::use_profile(&mut config, &name)?;
            let path = ase_llm::save_config(&env, &config)?;
            if has(rest, "--json") {
                let value = json!({
                    "active": name,
                    "config_path": path.display().to_string(),
                });
                outln!("{}", serde_json::to_string_pretty(&value).unwrap());
            } else {
                outln!("active profile: {name}");
                outln!("credential is resolved for the endpoint origin, so a profile on the same origin reuses the stored key");
            }
            Ok(())
        }
        "enroll" => {
            llm_expect(
                rest,
                &[
                    "--profile", "--provider", "--type", "--endpoint", "--model",
                    "--effort",
                ],
                &["--json", "--use"],
            )?;
            let env = ase_llm::capture_env();
            let mut config = ase_llm::load_config(&env)?
                .config
                .unwrap_or_else(ase_llm::ConfigFile::empty);
            let name = llm_profile_name(rest);
            let (provider, api, endpoint, model, effort) = llm_settings(rest)?;
            ase_llm::apply_edit(
                &mut config,
                &ase_llm::ProfileEdit {
                    profile: &name,
                    provider: provider.as_deref(),
                    api,
                    endpoint: endpoint.as_deref(),
                    model: model.as_deref(),
                    effort,
                    make_active: has(rest, "--use"),
                },
            )?;
            // Bind enrollment to the profile being saved, not to the old
            // on-disk configuration or to runtime environment overrides.
            let profile = &config.profiles[&name];
            let endpoint = ase_llm::validate_endpoint(&profile.endpoint)?;
            let store = ase_llm::open_store(&env)?;
            store.ensure_writable()?;
            ase_llm::save_config(&env, &config)?;
            eprintln!(
                "storing the key for provider {} / profile {} ({})",
                profile.provider, name, endpoint.origin
            );
            let key = ase_llm::read_secret_line(ase_llm::KEY_PROMPT)?;
            store.set(&profile.provider, &endpoint.origin, &key)?;
            if has(rest, "--json") {
                let value = json!({
                    "stored": true,
                    "profile": name,
                    "provider": profile.provider,
                    "origin": endpoint.origin,
                    "credential_store": store.backend(),
                });
                outln!("{}", serde_json::to_string_pretty(&value).unwrap());
            } else {
                outln!(
                    "stored key for provider {} at {} in {}",
                    profile.provider,
                    endpoint.origin,
                    store.backend()
                );
            }
            Ok(())
        }
        _ => Err(err("unknown llm config command (see `ase llm --help`)")),
    }
}

fn llm_profile_name(args: &[String]) -> String {
    opt(args, "--profile").unwrap_or_else(|| ase_llm::DEFAULT_PROFILE.to_string())
}

fn llm_settings(
    args: &[String],
) -> Result<(
    Option<String>,
    Option<ase_llm::ApiKind>,
    Option<String>,
    Option<String>,
    ase_llm::EffortEdit,
)> {
    let provider = opt(args, "--provider");
    let api = match opt(args, "--type") {
        Some(value) => Some(
            ase_llm::ApiKind::parse(&value)
                .ok_or_else(|| {
                    err("--type must be chat-completions, responses, decision, gemini or gemini-decision")
                })?,
        ),
        None => None,
    };
    let endpoint = opt(args, "--endpoint");
    let model = opt(args, "--model");
    let effort = match opt(args, "--effort") {
        Some(value) if value.eq_ignore_ascii_case("none") => ase_llm::EffortEdit::Clear,
        Some(value) => ase_llm::EffortEdit::Set(value),
        None => ase_llm::EffortEdit::Keep,
    };
    Ok((provider, api, endpoint, model, effort))
}

fn cmd_llm_key(args: &[String]) -> Result<()> {
    let Some(sub) = args.first().map(String::as_str) else {
        return print_llm_status(None, false, false);
    };
    if matches!(sub, "--help" | "-h") {
        llm_usage();
        return Ok(());
    }
    let rest = &args[1..];
    match sub {
        "set" => {
            llm_expect(rest, &["--profile"], &["--json"])?;
            let env = ase_llm::capture_env();
            let profile = ase_llm::resolve_profile(&env, opt(rest, "--profile").as_deref())?;
            let store = ase_llm::open_store(&env)?;
            store.ensure_writable()?;
            eprintln!(
                "storing the key for provider {} / profile {} ({})",
                profile.provider, profile.profile, profile.endpoint.origin
            );
            let key = ase_llm::read_secret_line(ase_llm::KEY_PROMPT)?;
            store.set(&profile.provider, &profile.endpoint.origin, &key)?;
            if has(rest, "--json") {
                let value = json!({
                    "stored": true,
                    "profile": profile.profile,
                    "provider": profile.provider,
                    "origin": profile.endpoint.origin,
                    "credential_store": store.backend(),
                });
                outln!("{}", serde_json::to_string_pretty(&value).unwrap());
            } else {
                outln!(
                    "stored key for provider {} at {} in {}",
                    profile.provider,
                    profile.endpoint.origin,
                    store.backend()
                );
            }
            Ok(())
        }
        "clear" => {
            llm_expect(rest, &["--profile"], &["--json"])?;
            let env = ase_llm::capture_env();
            let profile = ase_llm::resolve_profile(&env, opt(rest, "--profile").as_deref())?;
            let store = ase_llm::open_store(&env)?;
            let deleted = store.delete(&profile.provider, &profile.endpoint.origin)?;
            let env_override = ase_llm::env_key_present(&env, profile.api);
            if has(rest, "--json") {
                let value = json!({
                    "deleted": deleted,
                    "profile": profile.profile,
                    "provider": profile.provider,
                    "origin": profile.endpoint.origin,
                    "credential_store": store.backend(),
                    "env_override_active": env_override,
                });
                outln!("{}", serde_json::to_string_pretty(&value).unwrap());
            } else {
                outln!(
                    "{} credential for provider {} at {}",
                    if deleted { "deleted" } else { "no stored" },
                    profile.provider,
                    profile.endpoint.origin
                );
                if env_override {
                    outln!(
                        "note: a nonempty {} still overrides the store",
                        profile.api.key_env()
                    );
                }
            }
            Ok(())
        }
        "status" => {
            llm_expect(rest, &["--profile"], &["--json"])?;
            print_llm_status(opt(rest, "--profile"), has(rest, "--json"), true)
        }
        _ => Err(err("unknown llm key command (see `ase llm --help`)")),
    }
}

fn cmd_llm_status(args: &[String]) -> Result<()> {
    llm_expect(args, &["--profile"], &["--json"])?;
    print_llm_status(opt(args, "--profile"), has(args, "--json"), true)
}

fn print_llm_status(
    profile: Option<String>,
    as_json: bool,
    exit_on_not_ready: bool,
) -> Result<()> {
    let env = ase_llm::capture_env();
    let report = ase_llm::status(&env, profile.as_deref())?;
    if as_json {
        outln!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|e| err(format!("serialize llm status: {e}")))?
        );
    } else {
        outln!("config: {}", report.config_path);
        outln!("config source: {}", report.config_source);
        if let Some(name) = &report.profile {
            outln!("profile: {name}");
        }
        if let Some(value) = &report.provider {
            outln!("provider: {value}");
        }
        if let Some(value) = &report.api {
            outln!("api: {value}");
        }
        if let Some(value) = &report.endpoint {
            outln!("endpoint: {value}");
        }
        if let Some(value) = &report.origin {
            outln!("origin: {value}");
        }
        if let Some(value) = &report.model {
            outln!("model: {value}");
        }
        outln!(
            "effort: {}",
            report.effort.as_deref().unwrap_or("none")
        );
        outln!(
            "credential: {} ({})",
            report.credential_source, report.credential_store
        );
        outln!(
            "env overrides: {}",
            if report.env_overrides.is_empty() {
                "none".to_string()
            } else {
                report.env_overrides.join(", ")
            }
        );
        for note in &report.notes {
            outln!("note: {note}");
        }
        for check in &report.checks {
            outln!(
                "check {}: {} ({})",
                check.name,
                if check.ok { "ok" } else { "failed" },
                check.detail
            );
        }
        outln!("ready: {}", report.ready);
        outln!("network: not used");
        if !report.configured {
            outln!();
            outln!(
                "one-time enrollment: ase llm config enroll --profile default \\\n  --type chat-completions \\\n  --endpoint https://api.openai.com/v1/chat/completions \\\n  --model <MODEL>"
            );
        }
    }
    if exit_on_not_ready && report.configured && !report.ready {
        std::process::exit(3);
    }
    Ok(())
}
