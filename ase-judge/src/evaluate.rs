//! Offline evaluation orchestration: baseline, SLM boundary and Judge on a
//! pinned, reopenable cohort projection.
//!
//! The run reads a pin (manifest + units + private path map), re-opens every
//! selected journal from exact bytes, checks the pin, derives decision inputs,
//! consults the span/input cache, and inserts validated assertions. Model
//! stages are separate: the SLM and the Judge have independent prompt
//! versions, runner configurations, identities and cache namespaces. A model
//! stage that cannot run is reported as unavailable; it never falls back to
//! the baseline, to fixture output or to worker inference.
//!
//! Reads (`verify`, `report`, `show`) never construct a runner.

use ase_core::{err, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::assertion::{
    sha256_hex, Assertion, Decision, Generator, BASELINE_GENERATOR_ID,
    BASELINE_RULE_VERSION, JUDGE_PROMPT_VERSION, INPUT_RENDER_VERSION,
    SLM_PROMPT_VERSION, VIEW_VERSION,
};
use crate::baseline;
use crate::pin;
use crate::rubric::{self, ModelConfidence};
use crate::runner::{self, CallOutcome, StageModel};
use crate::source::CorpusSource;
use crate::stage::{self, StageState};
use crate::store::{InsertOutcome, RunRecord, Store};

pub use crate::stage::StageStats;
pub use crate::views::{
    report, show, verify, write_fixture, VerifyReport, FIXTURE_SCHEMA, REPORT_SCHEMA,
    VERIFY_SCHEMA,
};

pub const EVAL_SCHEMA: &str = "ase-judge-eval-run/v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Primary,
    Unknown,
    All,
}

impl Scope {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "primary" => Some(Scope::Primary),
            "unknown" => Some(Scope::Unknown),
            "all" => Some(Scope::All),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Primary => "primary",
            Scope::Unknown => "unknown",
            Scope::All => "all",
        }
    }
}

pub struct EvalOptions {
    pub root: PathBuf,
    pub manifest: PathBuf,
    pub units: PathBuf,
    pub private: PathBuf,
    pub store: PathBuf,
    pub scope: Scope,
    pub only_ordinals: BTreeSet<u32>,
    pub limit: Option<usize>,
    pub workspace: PathBuf,
    pub verification_only: bool,
    /// Explicit owner opt-in for a request that carries session text to a
    /// non-loopback endpoint. Without it such a stage is unavailable.
    pub allow_remote_session_data: bool,
    pub slm: Option<StageModel>,
    pub judge: Option<StageModel>,
}

pub struct VerifyOptions {
    pub root: PathBuf,
    pub units: PathBuf,
    pub private: PathBuf,
    pub store: PathBuf,
    pub include_verification: bool,
}

#[derive(Serialize)]
pub struct PinReport {
    pub source: &'static str,
    pub manifest_sha256: String,
    pub units_sha256: String,
    pub private_sha256: String,
    pub units: u64,
    pub decisions: u64,
}

#[derive(Serialize)]
pub struct RunReport {
    pub schema: &'static str,
    pub run_id: String,
    pub config_sha256: String,
    pub started_at: String,
    pub finished_at: String,
    pub elapsed_ms: u64,
    pub verification_only: bool,
    pub scope: String,
    pub pin: PinReport,
    pub versions: Value,
    pub units: u64,
    pub decisions: u64,
    pub baseline: StageStats,
    pub slm: StageStats,
    pub judge: StageStats,
    /// Actual spawn attempts across both model stages (ok + failed + rejected).
    pub model_calls: u64,
    pub model_calls_unavailable: u64,
    pub assertions_sha256: String,
    pub assertion_rows: u64,
    pub missing_evidence: u64,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn now_iso() -> String {
    ase_work::format_iso_ms(now_ms())
}

pub fn run(options: &EvalOptions) -> Result<RunReport> {
    for (stage, model) in [
        ("slm", options.slm.as_ref()),
        ("judge", options.judge.as_ref()),
    ] {
        if let Some(StageModel::Process(config)) = model {
            if config.model_id == "ase-contract-runner" && !options.verification_only {
                return Err(err(format!(
                    "{stage} is configured with the contract runner; pass --verification-only so its output is never read as evaluation"
                )));
            }
        }
    }
    let started = Instant::now();
    let started_at = now_iso();
    let run_id = format!(
        "run-{:016x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    );
    let at = started_at.clone();
    let pin = pin::Pin::load(&options.manifest, &options.units, &options.private)?;
    let selected = pin.select(options.scope, &options.only_ordinals, options.limit);
    if selected.is_empty() {
        return Err(err("no units selected for the requested scope"));
    }
    let source = CorpusSource::new(&options.root);
    for (_, unit_id, rel_path) in &selected {
        source.register(unit_id, rel_path);
    }
    let mut store = Store::open(&options.store)?;
    let config_sha256 = sha256_hex(
        &serde_json::to_vec(&json!({
            "schema": "ase-judge-run-config/v1",
            "manifest_sha256": pin.manifest_sha256,
            "units_sha256": pin.units_sha256,
            "private_sha256": pin.private_sha256,
            "scope": options.scope.as_str(),
            "only_ordinals": options.only_ordinals.iter().collect::<Vec<_>>(),
            "limit": options.limit,
            "verification_only": options.verification_only,
            "baseline_rule": BASELINE_RULE_VERSION,
            "view_version": VIEW_VERSION,
            "slm_prompt": SLM_PROMPT_VERSION,
            "judge_prompt": JUDGE_PROMPT_VERSION,
            "rubric_version": rubric::RUBRIC_VERSION,
            "wire_version": ase_llm::WIRE_VERSION,
            "parse_version": ase_llm::PARSE_VERSION,
            "allow_remote_session_data": options.allow_remote_session_data,
            "slm_model": options.slm.as_ref().map(stage::stage_config_identity),
            "judge_model": options.judge.as_ref().map(stage::stage_config_identity),
        }))
        .unwrap_or_default(),
    );

    let (mut slm_state, mut slm_stats) = stage::prepare_stage(
        "slm",
        &options.workspace,
        options.slm.as_ref(),
        &mut store,
        &run_id,
        options.verification_only,
        options.allow_remote_session_data,
        &at,
    );
    let (mut judge_state, mut judge_stats) = stage::prepare_stage(
        "judge",
        &options.workspace,
        options.judge.as_ref(),
        &mut store,
        &run_id,
        options.verification_only,
        options.allow_remote_session_data,
        &at,
    );
    slm_stats.prompt_version = Some(SLM_PROMPT_VERSION.to_string());
    judge_stats.prompt_version = Some(JUDGE_PROMPT_VERSION.to_string());
    let mut baseline_stats = StageStats {
        status: "ok".to_string(),
        prompt_version: Some(BASELINE_RULE_VERSION.to_string()),
        generator_id: Some(BASELINE_GENERATOR_ID.to_string()),
        ..StageStats::default()
    };

    let mut units = 0u64;
    let mut decisions = 0u64;
    let mut model_calls = 0u64;

    for (ordinal, unit_id, _rel_path) in &selected {
        let journal = source.journal(unit_id)?;
        pin.verify_unit(*ordinal, &journal)?;
        units += 1;
        for index in 0..journal.decision_count() as u32 {
            decisions += 1;
            let (evidence, rendered) = journal.decision(index)?;
            let render_sha256 = sha256_hex(rendered.as_bytes());
            let baseline_assertion = Assertion {
                unit_id: unit_id.clone(),
                decision_index: index,
                generator: Generator::Baseline,
                rule_version: BASELINE_RULE_VERSION.to_string(),
                generator_id: BASELINE_GENERATOR_ID.to_string(),
                evidence: evidence.clone(),
                render_sha256: render_sha256.clone(),
                label: Decision::Unknown,
                confidence: None,
                abstain_reason: Some("placeholder".to_string()),
                context: None,
                verification_only: options.verification_only,
            };
            let baseline_key = {
                let mut probe = baseline_assertion.clone();
                probe.label = Decision::Unknown;
                probe.cache_key()
            };
            let baseline_label = if let Some(existing) = store.find(&baseline_key)? {
                baseline_stats.cache_hits += 1;
                existing.assertion.label
            } else {
                let outcome = baseline::classify(
                    &source.human_text(unit_id, index)?,
                    &source.human_text(unit_id, index + 1)?,
                );
                let assertion = Assertion {
                    label: outcome.label,
                    confidence: Some(outcome.confidence.to_string()),
                    abstain_reason: outcome.abstain_reason.map(str::to_string),
                    ..baseline_assertion
                };
                match store.insert(&assertion, &source, &run_id, &at)? {
                    InsertOutcome::Inserted => {
                        baseline_stats.cache_misses += 1;
                        baseline_stats.inserted += 1;
                    }
                    InsertOutcome::Hit(_) => baseline_stats.cache_hits += 1,
                }
                outcome.label
            };
            baseline_stats.decisions += 1;
            baseline_stats.assertions += 1;
            stage::bump_label(&mut baseline_stats, baseline_label);

            // Separate SLM boundary stage.
            let mut slm_label: Option<Decision> = None;
            slm_stats.decisions += 1;
            let slm_key = {
                let probe = Assertion {
                    unit_id: unit_id.clone(),
                    decision_index: index,
                    generator: Generator::Slm,
                    rule_version: SLM_PROMPT_VERSION.to_string(),
                    generator_id: slm_state
                        .generator_id()
                        .unwrap_or("unavailable")
                        .to_string(),
                    evidence: evidence.clone(),
                    render_sha256: render_sha256.clone(),
                    label: Decision::Unknown,
                    confidence: None,
                    abstain_reason: Some("placeholder".to_string()),
                    context: None,
                    verification_only: options.verification_only,
                };
                if slm_state.is_runnable() {
                    Some(probe.cache_key())
                } else {
                    None
                }
            };
            if let Some(key) = slm_key {
                if let Some(existing) = store.find(&key)? {
                    slm_stats.cache_hits += 1;
                    slm_label = Some(existing.assertion.label);
                    slm_stats.assertions += 1;
                    stage::bump_label(&mut slm_stats, existing.assertion.label);
                } else {
                    slm_stats.cache_misses += 1;
                    let generator_id = slm_state.generator_id().unwrap_or_default().to_string();
                    let mut probe = Assertion {
                        unit_id: unit_id.clone(),
                        decision_index: index,
                        generator: Generator::Slm,
                        rule_version: SLM_PROMPT_VERSION.to_string(),
                        generator_id: generator_id.clone(),
                        evidence: evidence.clone(),
                        render_sha256: render_sha256.clone(),
                        label: Decision::Unknown,
                        confidence: None,
                        abstain_reason: Some("placeholder".to_string()),
                        context: None,
                        verification_only: options.verification_only,
                    };
                    let input_sha = probe.base_input_sha256();
                    let outcome = match &slm_state {
                        StageState::Process {
                            program, config, ..
                        } => {
                            let prompt = rubric::slm_prompt(&rendered);
                            let request = stage::runner_request(
                                "slm",
                                &generator_id,
                                SLM_PROMPT_VERSION,
                                &input_sha,
                                prompt,
                            );
                            runner::invoke(program, config, &options.workspace, &request)
                        }
                        StageState::Api { stage, .. } => {
                            runner::invoke_api(stage, rubric::slm_instruction(), &rendered)
                        }
                        StageState::Disabled => unreachable!(),
                    };
                    stage::record_stage(
                        &mut slm_stats,
                        &outcome,
                        &mut store,
                        &run_id,
                        "slm",
                        Some(&generator_id),
                        &input_sha,
                        options.verification_only,
                        &at,
                    );
                    match outcome {
                        CallOutcome::Ok(response) => {
                            model_calls += 1;
                            probe.label = response.label;
                            probe.confidence = response.confidence.as_ref().map(ModelConfidence::encode);
                            probe.abstain_reason = response.abstain_reason;
                            match store.insert(&probe, &source, &run_id, &at)? {
                                InsertOutcome::Inserted => {
                                    slm_stats.inserted += 1;
                                    slm_stats.assertions += 1;
                                }
                                InsertOutcome::Hit(_) => slm_stats.cache_hits += 1,
                            }
                            slm_label = Some(probe.label);
                            stage::bump_label(&mut slm_stats, probe.label);
                        }
                        other => {
                            if !matches!(other, CallOutcome::Unavailable(_)) {
                                model_calls += 1;
                            }
                            slm_stats.reason =
                                Some(other.detail().unwrap_or("stage_disabled").to_string());
                            if matches!(
                                other,
                                CallOutcome::Failed(_) | CallOutcome::Unavailable(_)
                            ) {
                                slm_state = StageState::Disabled;
                                slm_stats.status = other.kind().to_string();
                            }
                        }
                    }
                }
            }
            if slm_state.is_runnable() {
                slm_stats.status = if slm_stats.assertions == slm_stats.decisions {
                    "ok".to_string()
                } else {
                    "partial".to_string()
                };
            }

            // Separate Judge stage; its input includes the candidate codes.
            judge_stats.decisions += 1;
            let context = json!({
                "baseline": baseline_label.as_str(),
                "slm": slm_label.map(Decision::as_str).unwrap_or("unavailable"),
            });
            let judge_key = {
                let probe = Assertion {
                    unit_id: unit_id.clone(),
                    decision_index: index,
                    generator: Generator::Judge,
                    rule_version: JUDGE_PROMPT_VERSION.to_string(),
                    generator_id: judge_state
                        .generator_id()
                        .unwrap_or("unavailable")
                        .to_string(),
                    evidence: evidence.clone(),
                    render_sha256: render_sha256.clone(),
                    label: Decision::Unknown,
                    confidence: None,
                    abstain_reason: Some("placeholder".to_string()),
                    context: Some(context.clone()),
                    verification_only: options.verification_only,
                };
                if judge_state.is_runnable() {
                    Some(probe.cache_key())
                } else {
                    None
                }
            };
            if let Some(key) = judge_key {
                if let Some(existing) = store.find(&key)? {
                    judge_stats.cache_hits += 1;
                    judge_stats.assertions += 1;
                    stage::bump_label(&mut judge_stats, existing.assertion.label);
                } else {
                    judge_stats.cache_misses += 1;
                    let generator_id =
                        judge_state.generator_id().unwrap_or_default().to_string();
                    let mut probe = Assertion {
                        unit_id: unit_id.clone(),
                        decision_index: index,
                        generator: Generator::Judge,
                        rule_version: JUDGE_PROMPT_VERSION.to_string(),
                        generator_id: generator_id.clone(),
                        evidence: evidence.clone(),
                        render_sha256: render_sha256.clone(),
                        label: Decision::Unknown,
                        confidence: None,
                        abstain_reason: Some("placeholder".to_string()),
                        context: Some(context.clone()),
                        verification_only: options.verification_only,
                    };
                    let input_sha = probe.input_sha256();
                    let outcome = match &judge_state {
                        StageState::Process {
                            program, config, ..
                        } => {
                            let prompt =
                                rubric::judge_prompt(&rendered, baseline_label, slm_label);
                            let request = stage::runner_request(
                                "judge",
                                &generator_id,
                                JUDGE_PROMPT_VERSION,
                                &input_sha,
                                prompt,
                            );
                            runner::invoke(program, config, &options.workspace, &request)
                        }
                        StageState::Api { stage, .. } => runner::invoke_api(
                            stage,
                            &rubric::judge_instruction(baseline_label, slm_label),
                            &rendered,
                        ),
                        StageState::Disabled => unreachable!(),
                    };
                    stage::record_stage(
                        &mut judge_stats,
                        &outcome,
                        &mut store,
                        &run_id,
                        "judge",
                        Some(&generator_id),
                        &input_sha,
                        options.verification_only,
                        &at,
                    );
                    match outcome {
                        CallOutcome::Ok(response) => {
                            model_calls += 1;
                            probe.label = response.label;
                            probe.confidence = response.confidence.as_ref().map(ModelConfidence::encode);
                            probe.abstain_reason = response.abstain_reason;
                            match store.insert(&probe, &source, &run_id, &at)? {
                                InsertOutcome::Inserted => {
                                    judge_stats.inserted += 1;
                                    judge_stats.assertions += 1;
                                }
                                InsertOutcome::Hit(_) => judge_stats.cache_hits += 1,
                            }
                            stage::bump_label(&mut judge_stats, probe.label);
                        }
                        other => {
                            if !matches!(other, CallOutcome::Unavailable(_)) {
                                model_calls += 1;
                            }
                            judge_stats.reason =
                                Some(other.detail().unwrap_or("stage_disabled").to_string());
                            if matches!(
                                other,
                                CallOutcome::Failed(_) | CallOutcome::Unavailable(_)
                            ) {
                                judge_state = StageState::Disabled;
                                judge_stats.status = other.kind().to_string();
                            }
                        }
                    }
                }
            }
            if judge_state.is_runnable() {
                judge_stats.status = if judge_stats.assertions == judge_stats.decisions {
                    "ok".to_string()
                } else {
                    "partial".to_string()
                };
            }
        }
    }
    let model_calls_unavailable = slm_stats.calls_unavailable + judge_stats.calls_unavailable;

    let finished_at = now_iso();
    let assertions_sha256 = store.digest(options.verification_only)?;
    let assertion_rows = store.count_namespace(options.verification_only)?;
    let missing_evidence = store.count_without_evidence()?;
    let report = RunReport {
        schema: EVAL_SCHEMA,
        run_id: run_id.clone(),
        config_sha256: config_sha256.clone(),
        started_at,
        finished_at: finished_at.clone(),
        elapsed_ms: started.elapsed().as_millis() as u64,
        verification_only: options.verification_only,
        scope: options.scope.as_str().to_string(),
        pin: PinReport {
            source: "pinned cohort",
            manifest_sha256: pin.manifest_sha256,
            units_sha256: pin.units_sha256,
            private_sha256: pin.private_sha256,
            units,
            decisions,
        },
        versions: json!({
            "view": VIEW_VERSION,
            "baseline_rule": BASELINE_RULE_VERSION,
            "slm_prompt": SLM_PROMPT_VERSION,
            "judge_prompt": JUDGE_PROMPT_VERSION,
            "render": INPUT_RENDER_VERSION,
            "rubric": rubric::RUBRIC_VERSION,
            "wire": ase_llm::WIRE_VERSION,
            "parse": ase_llm::PARSE_VERSION,
        }),
        units,
        decisions,
        baseline: baseline_stats,
        slm: slm_stats,
        judge: judge_stats,
        model_calls,
        model_calls_unavailable,
        assertions_sha256,
        assertion_rows,
        missing_evidence,
    };
    let report_json = serde_json::to_string(&report)
        .map_err(|e| err(format!("encode run report: {e}")))?;
    store.record_run(&RunRecord {
        run_id: &run_id,
        config_sha256: &config_sha256,
        started_at: &report.started_at,
        finished_at: &finished_at,
        scope: options.scope.as_str(),
        units,
        decisions,
        model_calls,
        verification_only: options.verification_only,
        report_json: &report_json,
    })?;
    Ok(report)
}
