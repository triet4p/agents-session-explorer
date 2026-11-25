//! Offline P3 evaluation orchestration: baseline, SLM boundary and Judge on a
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
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::assertion::{
    sha256_hex, Assertion, Decision, Generator, BASELINE_GENERATOR_ID,
    BASELINE_RULE_VERSION, JUDGE_PROMPT_VERSION, INPUT_RENDER_VERSION, LOCAL_MODEL_PROTOCOL,
    SLM_PROMPT_VERSION, VIEW_VERSION,
};
use crate::baseline;
use crate::rubric::{self, ModelConfidence};
use crate::runner::{self, ApiStage, CallOutcome, RunnerConfig, StageModel};
use crate::source::{CorpusSource, EvidenceVerifier};
use crate::store::{CallRecord, InsertOutcome, RunRecord, Store};

pub const EVAL_SCHEMA: &str = "ase-s6-t2-eval-run/v1";
pub const VERIFY_SCHEMA: &str = "ase-s6-t2-verify/v1";
pub const REPORT_SCHEMA: &str = "ase-s6-t2-store-report/v1";
pub const FIXTURE_SCHEMA: &str = "ase-s6-t2-contract-fixture/v1";

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

struct UnitPin {
    ordinal: u32,
    unit_id: String,
    primary: bool,
    unknown: bool,
    human: u32,
    all_role: u32,
    leaf: Option<String>,
}

struct PrivateRow {
    rel_path: String,
    session_id: String,
}

struct ManifestRow {
    size: u64,
    sha256: String,
}

struct Pin {
    manifest_sha256: String,
    units_sha256: String,
    private_sha256: String,
    manifest: BTreeMap<u32, ManifestRow>,
    units: Vec<UnitPin>,
    private: BTreeMap<u32, PrivateRow>,
}

fn read_json(path: &Path, what: &str) -> Result<Value> {
    let bytes = std::fs::read(path)
        .map_err(|e| err(format!("read {what} {}: {e}", path.display())))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| err(format!("parse {what} {}: {e}", path.display())))
}

fn read_file_sha256(path: &Path, what: &str) -> Result<String> {
    let bytes = std::fs::read(path)
        .map_err(|e| err(format!("read {what} {}: {e}", path.display())))?;
    Ok(sha256_hex(&bytes))
}

fn heritage_unit_id(session_id: &str) -> String {
    sha256_hex(format!("ase-s6-v1-unit\u{0}{session_id}").as_bytes())
}

impl Pin {
    fn load(manifest_path: &Path, units_path: &Path, private_path: &Path) -> Result<Self> {
        let manifest = read_json(manifest_path, "cohort manifest")?;
        let manifest_sha256 = manifest
            .get("manifest_sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| err("cohort manifest has no manifest_sha256"))?
            .to_string();
        let mut rows = BTreeMap::new();
        let manifest_rows = manifest
            .get("rows")
            .and_then(Value::as_array)
            .ok_or_else(|| err("cohort manifest has no rows"))?;
        for row in manifest_rows {
            let ordinal = row
                .get("ordinal")
                .and_then(Value::as_u64)
                .ok_or_else(|| err("cohort manifest row has no ordinal"))? as u32;
            let size = row
                .get("size")
                .and_then(Value::as_u64)
                .ok_or_else(|| err("cohort manifest row has no size"))?;
            let sha256 = row
                .get("sha256")
                .and_then(Value::as_str)
                .ok_or_else(|| err("cohort manifest row has no sha256"))?
                .to_string();
            rows.insert(ordinal, ManifestRow { size, sha256 });
        }
        let private = read_json(private_path, "private path map")?;
        let mut private_rows = BTreeMap::new();
        let private_array = private
            .get("rows")
            .and_then(Value::as_array)
            .ok_or_else(|| err("private path map has no rows"))?;
        for row in private_array {
            let ordinal = row
                .get("ordinal")
                .and_then(Value::as_u64)
                .ok_or_else(|| err("private row has no ordinal"))? as u32;
            let rel_path = row
                .get("rel_path")
                .and_then(Value::as_str)
                .ok_or_else(|| err("private row has no rel_path"))?
                .to_string();
            let session_id = row
                .get("session_id")
                .and_then(Value::as_str)
                .ok_or_else(|| err("private row has no session_id"))?
                .to_string();
            private_rows.insert(ordinal, PrivateRow { rel_path, session_id });
        }
        let units_text = std::fs::read_to_string(units_path)
            .map_err(|e| err(format!("read units {}: {e}", units_path.display())))?;
        let mut units = Vec::new();
        for line in units_text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let row: Value = serde_json::from_str(line)
                .map_err(|e| err(format!("parse units row: {e}")))?;
            let ordinal = row
                .get("ordinal")
                .and_then(Value::as_u64)
                .ok_or_else(|| err("units row has no ordinal"))? as u32;
            let private_row = private_rows.get(&ordinal).ok_or_else(|| {
                err(format!("unit ordinal {ordinal} has no private path row"))
            })?;
            units.push(UnitPin {
                ordinal,
                unit_id: heritage_unit_id(&private_row.session_id),
                primary: row
                    .get("primary_candidate")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                unknown: row
                    .get("unknown_stratum")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                human: row
                    .get("human_user_msgs_active")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32,
                all_role: row
                    .get("user_msgs_active")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32,
                leaf: row
                    .get("leaf_entry_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
        units.sort_by_key(|unit| unit.ordinal);
        Ok(Pin {
            manifest_sha256,
            units_sha256: read_file_sha256(units_path, "units")?,
            private_sha256: read_file_sha256(private_path, "private path map")?,
            manifest: rows,
            units,
            private: private_rows,
        })
    }

    fn select(&self, scope: Scope, only: &BTreeSet<u32>, limit: Option<usize>) -> Vec<(u32, String, String)> {
        let mut selected = Vec::new();
        for unit in &self.units {
            if !only.is_empty() && !only.contains(&unit.ordinal) {
                continue;
            }
            let in_scope = match scope {
                Scope::Primary => unit.primary,
                Scope::Unknown => unit.unknown,
                Scope::All => unit.primary || unit.unknown,
            };
            if !in_scope {
                continue;
            }
            let Some(private) = self.private.get(&unit.ordinal) else {
                continue;
            };
            selected.push((
                unit.ordinal,
                unit.unit_id.clone(),
                private.rel_path.clone(),
            ));
            if let Some(limit) = limit {
                if selected.len() >= limit {
                    break;
                }
            }
        }
        selected
    }
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

fn stage_config_identity(model: &StageModel) -> Value {
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

fn runner_request(
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

enum StageState<'a> {
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
    fn generator_id(&self) -> Option<&str> {
        match self {
            StageState::Disabled => None,
            StageState::Process { generator_id, .. }
            | StageState::Api { generator_id, .. } => Some(generator_id),
        }
    }

    fn is_runnable(&self) -> bool {
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

fn prepare_stage<'a>(
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

fn record_stage(
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

fn bump_label(stats: &mut StageStats, label: Decision) {
    *stats
        .labels
        .entry(label.as_str().to_string())
        .or_insert(0) += 1;
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
    let pin = Pin::load(&options.manifest, &options.units, &options.private)?;
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
            "schema": "ase-s6-t2-run-config/v1",
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
            "slm_model": options.slm.as_ref().map(stage_config_identity),
            "judge_model": options.judge.as_ref().map(stage_config_identity),
        }))
        .unwrap_or_default(),
    );

    let (mut slm_state, mut slm_stats) = prepare_stage(
        "slm",
        &options.workspace,
        options.slm.as_ref(),
        &mut store,
        &run_id,
        options.verification_only,
        options.allow_remote_session_data,
        &at,
    );
    let (mut judge_state, mut judge_stats) = prepare_stage(
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
        let manifest_row = pin
            .manifest
            .get(ordinal)
            .ok_or_else(|| err(format!("manifest has no row for ordinal {ordinal}")))?;
        if journal.size != manifest_row.size || journal.sha256 != manifest_row.sha256 {
            return Err(err(format!(
                "journal for ordinal {ordinal} does not match the pinned size/SHA-256"
            )));
        }
        let private_row = pin
            .private
            .get(ordinal)
            .ok_or_else(|| err(format!("private map has no row for ordinal {ordinal}")))?;
        if journal.session_id != private_row.session_id {
            return Err(err(format!(
                "journal for ordinal {ordinal} does not match the pinned session id"
            )));
        }
        let unit = pin
            .units
            .iter()
            .find(|unit| unit.ordinal == *ordinal)
            .ok_or_else(|| err(format!("units file has no row for ordinal {ordinal}")))?;
        if journal.human.len() as u32 != unit.human {
            return Err(err(format!(
                "ordinal {ordinal}: projected human messages {} != pinned {}",
                journal.human.len(),
                unit.human
            )));
        }
        if journal.all_role_user as u32 != unit.all_role {
            return Err(err(format!(
                "ordinal {ordinal}: projected user messages {} != pinned {}",
                journal.all_role_user, unit.all_role
            )));
        }
        if (journal.all_role_user as u32) > journal.projection_user_all_branches {
            return Err(err(format!(
                "ordinal {ordinal}: decision projection has {} active user messages but the product projection counts only {} across all branches",
                journal.all_role_user, journal.projection_user_all_branches
            )));
        }
        if let Some(leaf) = &unit.leaf {
            if journal.leaf_entry_id.as_deref() != Some(leaf.as_str()) {
                return Err(err(format!(
                    "ordinal {ordinal}: active leaf {:?} != pinned leaf {leaf}",
                    journal.leaf_entry_id
                )));
            }
        }
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
            bump_label(&mut baseline_stats, baseline_label);

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
                    bump_label(&mut slm_stats, existing.assertion.label);
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
                            let request = runner_request(
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
                    record_stage(
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
                            bump_label(&mut slm_stats, probe.label);
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
                    bump_label(&mut judge_stats, existing.assertion.label);
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
                            let request = runner_request(
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
                    record_stage(
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
                            bump_label(&mut judge_stats, probe.label);
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
            source: "ASE-S6-T1 pinned cohort",
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

#[derive(Serialize)]
pub struct VerifyReport {
    pub schema: &'static str,
    pub store: String,
    pub assertions: u64,
    pub verified: u64,
    pub failed: u64,
    pub missing_evidence: u64,
    pub failures: Vec<Value>,
    pub status: String,
}

pub fn verify(options: &VerifyOptions) -> Result<VerifyReport> {
    let private = read_json(&options.private, "private path map")?;
    let mut private_rows: BTreeMap<u32, PrivateRow> = BTreeMap::new();
    for row in private
        .get("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| err("private path map has no rows"))?
    {
        let ordinal = row
            .get("ordinal")
            .and_then(Value::as_u64)
            .ok_or_else(|| err("private row has no ordinal"))? as u32;
        private_rows.insert(
            ordinal,
            PrivateRow {
                rel_path: row
                    .get("rel_path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| err("private row has no rel_path"))?
                    .to_string(),
                session_id: row
                    .get("session_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| err("private row has no session_id"))?
                    .to_string(),
            },
        );
    }
    let units_text = std::fs::read_to_string(&options.units)
        .map_err(|e| err(format!("read units {}: {e}", options.units.display())))?;
    let mut unit_paths: BTreeMap<String, String> = BTreeMap::new();
    for line in units_text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let row: Value = serde_json::from_str(line)
            .map_err(|e| err(format!("parse units row: {e}")))?;
        let Some(ordinal) = row.get("ordinal").and_then(Value::as_u64) else {
            continue;
        };
        let Some(private_row) = private_rows.get(&(ordinal as u32)) else {
            continue;
        };
        unit_paths.insert(
            heritage_unit_id(&private_row.session_id),
            private_row.rel_path.clone(),
        );
    }
    let source = CorpusSource::new(&options.root);
    for (unit_id, rel_path) in &unit_paths {
        source.register(unit_id, rel_path);
    }
    let store = Store::open_read_only(&options.store)?;
    let rows = store.assertions(
        if options.include_verification {
            None
        } else {
            Some(false)
        },
        None,
        None,
    )?;
    let mut failures = Vec::new();
    let mut verified = 0u64;
    for stored in &rows {
        let assertion = &stored.assertion;
        let check = (|| -> Result<()> {
            assertion.validate_shape()?;
            source.verify_source(assertion)?;
            let recomputed_input = assertion.input_sha256();
            if recomputed_input != stored.input_sha256 {
                return Err(err("stored input digest does not recompute from source"));
            }
            let recomputed_key = assertion.cache_key();
            if recomputed_key != stored.cache_key {
                return Err(err("stored cache key does not recompute"));
            }
            Ok(())
        })();
        match check {
            Ok(()) => verified += 1,
            Err(e) => {
                if failures.len() < 50 {
                    failures.push(json!({
                        "unit_id": assertion.unit_id,
                        "decision_index": assertion.decision_index,
                        "generator": assertion.generator.as_str(),
                        "assertion_id": stored.assertion_id,
                        "reason": e.0,
                    }));
                }
            }
        }
    }
    let failed = rows.len() as u64 - verified;
    let missing_evidence = store.count_without_evidence()?;
    let status = if failed == 0 && missing_evidence == 0 {
        "verified"
    } else {
        "stale"
    };
    Ok(VerifyReport {
        schema: VERIFY_SCHEMA,
        store: options
            .store
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default(),
        assertions: rows.len() as u64,
        verified,
        failed,
        missing_evidence,
        failures,
        status: status.to_string(),
    })
}

pub fn report(store_path: &Path, include_verification: bool) -> Result<Value> {
    let store = Store::open_read_only(store_path)?;
    let filter = if include_verification {
        None
    } else {
        Some(false)
    };
    Ok(json!({
        "schema": REPORT_SCHEMA,
        "store": store_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default(),
        "assertions": store.count()?,
        "assertion_rows_in_scope": store.assertions(filter, None, None)?.len(),
        "missing_evidence": store.count_without_evidence()?,
        "assertions_sha256": {
            "evaluation": store.digest(false)?,
            "verification_only": store.digest(true)?,
        },
        "assertion_stats": store.assertion_stats()?,
        "call_stats": store.call_stats()?,
        "runs": store.runs()?,
    }))
}

pub fn show(
    store_path: &Path,
    unit_id: Option<&str>,
    decision_index: Option<u32>,
    include_verification: bool,
) -> Result<Value> {
    let store = Store::open_read_only(store_path)?;
    let filter = if include_verification {
        None
    } else {
        Some(false)
    };
    let rows = store.assertions(filter, unit_id, decision_index)?;
    let assertions: Vec<Value> = rows
        .iter()
        .map(|stored| {
            let assertion = &stored.assertion;
            json!({
                "unit_id": assertion.unit_id,
                "decision_index": assertion.decision_index,
                "generator": assertion.generator.as_str(),
                "generator_id": assertion.generator_id,
                "rule_version": assertion.rule_version,
                "label": assertion.label.as_str(),
                "confidence": assertion.confidence,
                "confidence_semantics": assertion
                    .confidence
                    .as_deref()
                    .map(|text| {
                        let (semantics, value) = rubric::decode_confidence(text);
                        json!({"semantics": semantics, "value": value})
                    }),
                "abstain_reason": assertion.abstain_reason,
                "context": assertion.context,
                "evidence": assertion.evidence,
                "input_sha256": stored.input_sha256,
                "cache_key": stored.cache_key,
                "assertion_id": stored.assertion_id,
                "verification_only": assertion.verification_only,
                "run_id": stored.run_id,
                "created_at": stored.created_at,
            })
        })
        .collect();
    Ok(json!({
        "schema": "ase-s6-t2-assertions/v1",
        "assertions": assertions,
        "count": assertions.len(),
        "model_calls": 0,
    }))
}

/// Writes a tiny synthetic OMP journal plus its pin so the local-model
/// contract and cache mechanics can be verified without real prose and
/// without standing in for an evaluation.
pub fn write_fixture(out_dir: &Path) -> Result<Value> {
    let corpus = out_dir.join("corpus");
    let project = corpus.join("proj");
    std::fs::create_dir_all(&project)
        .map_err(|e| err(format!("create fixture dir: {e}")))?;
    let session_id = "11111111-1111-4111-8111-111111111111";
    let lines = vec![
        json!({
            "type": "session",
            "id": session_id,
            "timestamp": "2026-10-01T00:00:00.000Z",
        }),
        json!({
            "type": "message",
            "id": "aaaabbbb",
            "parentId": session_id,
            "timestamp": "2026-10-01T00:00:01.000Z",
            "message": {
                "role": "user",
                "attribution": "user",
                "content": [{"type": "text", "text": "synthetic earlier instruction"}],
            },
        }),
        json!({
            "type": "message",
            "id": "ccccdddd",
            "parentId": "aaaabbbb",
            "timestamp": "2026-10-01T00:00:02.000Z",
            "message": {
                "role": "user",
                "attribution": "user",
                "content": [{"type": "text", "text": "synthetic later follow-up"}],
            },
        }),
        json!({
            "type": "message",
            "id": "eeeeffff",
            "parentId": "ccccdddd",
            "timestamp": "2026-10-01T00:00:03.000Z",
            "message": {
                "role": "user",
                "attribution": "user",
                "content": [{"type": "text", "text": "synthetic final refinement"}],
            },
        }),
    ];
    let mut journal = String::new();
    for line in &lines {
        journal.push_str(&serde_json::to_string(line).map_err(|e| err(format!("fixture json: {e}")))?);
        journal.push('\n');
    }
    let rel_path = "proj/contract-fixture.jsonl";
    let journal_path = corpus.join("proj/contract-fixture.jsonl");
    std::fs::write(&journal_path, journal.as_bytes())
        .map_err(|e| err(format!("write fixture journal: {e}")))?;
    let journal = crate::source::Journal::load(&corpus, rel_path)?;
    let units = json!({
        "ordinal": 0,
        "unit_id": "0000000000000000",
        "primary_candidate": true,
        "unknown_stratum": false,
        "human_user_msgs_active": journal.human.len(),
        "user_msgs_active": journal.all_role_user,
        "leaf_entry_id": journal.leaf_entry_id,
        "kind": "root",
        "naming_status": "unparsed",
    });
    let units_path = out_dir.join("units.jsonl");
    std::fs::write(
        &units_path,
        format!("{}\n", serde_json::to_string(&units).map_err(|e| err(format!("fixture units: {e}")))?),
    )
    .map_err(|e| err(format!("write fixture units: {e}")))?;
    let private = json!({
        "schema": "ase-s6-t1-private-manifest/v1",
        "rows": [{"ordinal": 0, "rel_path": rel_path, "session_id": journal.session_id}],
    });
    let private_path = out_dir.join("private.json");
    std::fs::write(
        &private_path,
        serde_json::to_string_pretty(&private)
            .map_err(|e| err(format!("encode fixture private: {e}")))?
            + "\n",
    )
    .map_err(|e| err(format!("write fixture private: {e}")))?;
    let manifest_body = json!({
        "schema": "ase-s6-t2-contract-fixture-manifest/v1",
        "rows": [{"ordinal": 0, "size": journal.size, "sha256": journal.sha256}],
    });
    let manifest = json!({
        "schema": "ase-s6-t2-contract-fixture-manifest/v1",
        "rows": [{"ordinal": 0, "size": journal.size, "sha256": journal.sha256}],
        "manifest_sha256": sha256_hex(
            &serde_json::to_vec(&manifest_body).map_err(|e| err(format!("fixture manifest: {e}")))?,
        ),
    });
    let manifest_path = out_dir.join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest)
            .map_err(|e| err(format!("encode fixture manifest: {e}")))?
            + "\n",
    )
    .map_err(|e| err(format!("write fixture manifest: {e}")))?;
    Ok(json!({
        "schema": FIXTURE_SCHEMA,
        "corpus": "corpus",
        "units": "units.jsonl",
        "private": "private.json",
        "manifest": "manifest.json",
        "session_id": journal.session_id,
        "journal_sha256": journal.sha256,
        "human_messages": journal.human.len(),
        "decisions": journal.decision_count(),
        "manifest_sha256": manifest["manifest_sha256"],
        "synthetic": true,
    }))
}
