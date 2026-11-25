//! Model stage coordination: process/API preparation, call recording and
//! label accounting.
//!
//! The SLM and Judge are separate generators with independent prompt
//! versions, runner configurations, identities and cache namespaces. A model
//! stage that cannot run is reported unavailable; it never falls back to the
//! baseline, to fixture output or to worker inference.

use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::assertion::{Decision, LOCAL_MODEL_PROTOCOL};
use crate::runner::{self, ApiStage, CallOutcome, RunnerConfig, StageModel};
use crate::store::{CallRecord, Store};

#[derive(Clone, Debug, Default, Serialize)]
pub struct StageStats {
    pub status: String,
    pub reason: Option<String>,
    pub backend: Option<String>,
    pub api: Option<String>,
    pub provider: Option<String>,
    pub endpoint_origin: Option<String>,
    pub prompt_version: Option<String>,
    pub generator_id: Option<String>,
    pub model_tag: Option<String>,
    pub model_digest: Option<String>,
    pub inference_config_sha256: Option<String>,
    pub decisions: u64,
    pub assertions: u64,
    pub inserted: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub calls_attempted: u64,
    pub calls_ok: u64,
    pub calls_failed: u64,
    pub calls_rejected: u64,
    pub calls_unavailable: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// How many labels carried a verbal self-report vs a decision probability.
    pub confidence_semantics: BTreeMap<String, u64>,
    pub labels: BTreeMap<String, u64>,
}

pub(crate) fn stage_config_identity(model: &StageModel) -> Value {
    match model {
        StageModel::Process(config) => json!({
            "backend": "process",
            "model_id": config.model_id,
            "model_sha256": config.model_sha256,
            "program": config.program,
            "args": config.args,
            "timeout_ms": config.timeout_ms,
        }),
        StageModel::Api(stage) => {
            let profile = &stage.resolution.profile;
            let identity = ase_llm::identity(profile);
            json!({
                "backend": "api",
                "api": profile.api.as_str(),
                "provider": profile.provider,
                "endpoint": profile.endpoint.url,
                "origin": profile.endpoint.origin,
                "model": profile.model,
                "effort": profile.effort,
                "wire_version": ase_llm::WIRE_VERSION,
                "parse_version": ase_llm::PARSE_VERSION,
                "timeout_ms": stage.timeout_ms,
                "identity_sha256": identity.digest,
            })
        }
    }
}

pub(crate) fn runner_request(
    stage: &str,
    generator_id: &str,
    prompt_version: &str,
    input_sha256: &str,
    prompt: String,
) -> Value {
    json!({
        "protocol": LOCAL_MODEL_PROTOCOL,
        "stage": stage,
        "generator_id": generator_id,
        "prompt_version": prompt_version,
        "input_sha256": input_sha256,
        "prompt": prompt,
    })
}

pub(crate) enum StageState<'a> {
    Disabled,
    Process {
        program: PathBuf,
        config: RunnerConfig,
        generator_id: String,
    },
    Api {
        stage: &'a ApiStage,
        generator_id: String,
    },
}

impl StageState<'_> {
    pub(crate) fn generator_id(&self) -> Option<&str> {
        match self {
            StageState::Disabled => None,
            StageState::Process { generator_id, .. }
            | StageState::Api { generator_id, .. } => Some(generator_id),
        }
    }

    pub(crate) fn is_runnable(&self) -> bool {
        !matches!(self, StageState::Disabled)
    }
}

fn unavailable_stage(
    label: &str,
    reason: &str,
    generator_id: Option<&str>,
    store: &mut Store,
    run_id: &str,
    verification_only: bool,
    at: &str,
    stats: &mut StageStats,
) -> StageState<'static> {
    stats.status = "unavailable".to_string();
    stats.reason = Some(reason.to_string());
    let _ = store.record_call(&CallRecord {
        run_id,
        stage: label,
        generator_id,
        input_sha256: "",
        outcome: "unavailable",
        detail: Some(reason),
        verification_only,
        at,
    });
    stats.calls_unavailable += 1;
    StageState::Disabled
}

pub(crate) fn prepare_stage<'a>(
    label: &str,
    workspace: &Path,
    model: Option<&'a StageModel>,
    store: &mut Store,
    run_id: &str,
    verification_only: bool,
    allow_remote_session_data: bool,
    at: &str,
) -> (StageState<'a>, StageStats) {
    let mut stats = StageStats {
        status: "unavailable".to_string(),
        ..StageStats::default()
    };
    let Some(model) = model else {
        let state = unavailable_stage(
            label,
            "model_backend_not_configured",
            None,
            store,
            run_id,
            verification_only,
            at,
            &mut stats,
        );
        return (state, stats);
    };
    match model {
        StageModel::Process(config) => {
            let generator = runner::generator_id(&config.model_id, &config.model_sha256);
            stats.backend = Some("process".to_string());
            stats.generator_id = Some(generator.clone());
            stats.model_tag = Some(config.model_id.clone());
            stats.model_digest = Some(config.model_sha256.clone());
            match runner::prepare(workspace, config) {
                Ok(program) => {
                    stats.status = "not_started".to_string();
                    (
                        StageState::Process {
                            program,
                            config: config.clone(),
                            generator_id: generator,
                        },
                        stats,
                    )
                }
                Err(reason) => {
                    let state = unavailable_stage(
                        label,
                        &reason,
                        Some(&generator),
                        store,
                        run_id,
                        verification_only,
                        at,
                        &mut stats,
                    );
                    (state, stats)
                }
            }
        }
        StageModel::Api(stage) => {
            let profile = &stage.resolution.profile;
            let identity = ase_llm::identity(profile);
            stats.backend = Some(format!("api:{}", profile.api.as_str()));
            stats.api = Some(profile.api.as_str().to_string());
            stats.provider = Some(profile.provider.clone());
            stats.endpoint_origin = Some(profile.endpoint.origin.clone());
            stats.model_tag = Some(profile.model.clone());
            stats.model_digest = Some(identity.digest.clone());
            stats.inference_config_sha256 = Some(identity.digest.clone());
            stats.generator_id = Some(identity.generator_id.clone());
            if !profile.endpoint.is_loopback() && !allow_remote_session_data {
                let reason = format!(
                    "session_data_egress_not_authorized: {} at {} (pass --allow-remote-session-data to opt in)",
                    profile.api.as_str(),
                    profile.endpoint.origin
                );
                let state = unavailable_stage(
                    label,
                    &reason,
                    Some(&identity.generator_id),
                    store,
                    run_id,
                    verification_only,
                    at,
                    &mut stats,
                );
                return (state, stats);
            }
            stats.status = "not_started".to_string();
            (
                StageState::Api {
                    stage,
                    generator_id: identity.generator_id,
                },
                stats,
            )
        }
    }
}

pub(crate) fn record_stage(
    stats: &mut StageStats,
    outcome: &CallOutcome,
    store: &mut Store,
    run_id: &str,
    stage: &str,
    generator_id: Option<&str>,
    input_sha256: &str,
    verification_only: bool,
    at: &str,
) {
    stats.calls_attempted += 1;
    match outcome {
        CallOutcome::Ok(response) => {
            stats.calls_ok += 1;
            stats.input_tokens += response.input_tokens.unwrap_or(0);
            stats.output_tokens += response.output_tokens.unwrap_or(0);
            if let Some(confidence) = &response.confidence {
                *stats
                    .confidence_semantics
                    .entry(confidence.semantics().to_string())
                    .or_insert(0) += 1;
            }
        }
        CallOutcome::Failed(_) => stats.calls_failed += 1,
        CallOutcome::Rejected(_) => stats.calls_rejected += 1,
        CallOutcome::Unavailable(_) => stats.calls_unavailable += 1,
    }
    let _ = store.record_call(&CallRecord {
        run_id,
        stage,
        generator_id,
        input_sha256,
        outcome: outcome.kind(),
        detail: outcome.detail(),
        verification_only,
        at,
    });
}

pub(crate) fn bump_label(stats: &mut StageStats, label: Decision) {
    *stats
        .labels
        .entry(label.as_str().to_string())
        .or_insert(0) += 1;
}
