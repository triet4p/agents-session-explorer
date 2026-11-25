//! Model access boundary for the P3 evaluation layer.
//!
//! Two backends remain:
//!
//! - a workspace-local process that reads one JSON request on stdin and writes
//!   one JSON response on stdout; it has no network path;
//! - one of the five typed API profiles resolved by `ase-llm`. The profile
//!   supplies the endpoint, typed protocol, model, effort and credential; this
//!   module builds the rubric instruction and maps the typed answer, while
//!   `ase-llm` owns framing and transport.
//!
//! Every attempt is classified as `ok`, `unavailable`, `failed` or `rejected`,
//! and a failed stage never falls back to the baseline, fixture output or a
//! different API/effort. Preparing a stage never opens a socket, so a warm
//! cache replay performs zero model calls.

use ase_core::{err, Result};
use serde::Deserialize;
use serde_json::Value;
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use ase_llm::{ApiError, CallSpec, Resolution};

use crate::assertion::{Decision, LOCAL_MODEL_PROTOCOL};
use crate::rubric::{self, ModelConfidence};

pub const DEFAULT_TIMEOUT_MS: u64 = 120_000;

#[derive(Clone, Debug)]
pub struct RunnerConfig {
    pub program: String,
    pub args: Vec<String>,
    pub model_id: String,
    pub model_sha256: String,
    pub timeout_ms: u64,
}

/// The resolved API backend for one stage. The credential lives inside
/// `resolution` and is only reachable through `ApiKey::expose()`; this struct
/// deliberately has a redacted `Debug`.
pub struct ApiStage {
    pub resolution: Resolution,
    pub timeout_ms: u64,
}

impl fmt::Debug for ApiStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let profile = &self.resolution.profile;
        formatter
            .debug_struct("ApiStage")
            .field("profile", &profile.profile)
            .field("provider", &profile.provider)
            .field("api", &profile.api.as_str())
            .field("origin", &profile.endpoint.origin)
            .field("model", &profile.model)
            .field("effort", &profile.effort)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunnerResponse {
    pub label: Decision,
    /// Typed model confidence: a verbal self-report from a generation model or
    /// a calibrated probability from a decision API.
    pub confidence: Option<ModelConfidence>,
    pub abstain_reason: Option<String>,
    pub model_id: String,
    pub model_sha256: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

pub enum CallOutcome {
    Ok(RunnerResponse),
    Unavailable(String),
    Failed(String),
    Rejected(String),
}

impl CallOutcome {
    pub fn kind(&self) -> &'static str {
        match self {
            CallOutcome::Ok(_) => "ok",
            CallOutcome::Unavailable(_) => "unavailable",
            CallOutcome::Failed(_) => "failed",
            CallOutcome::Rejected(_) => "rejected",
        }
    }

    pub fn detail(&self) -> Option<&str> {
        match self {
            CallOutcome::Ok(_) => None,
            CallOutcome::Unavailable(reason)
            | CallOutcome::Failed(reason)
            | CallOutcome::Rejected(reason) => Some(reason.as_str()),
        }
    }
}

fn api_outcome(error: ApiError) -> CallOutcome {
    match error {
        ApiError::Unavailable(reason) => CallOutcome::Unavailable(reason),
        ApiError::Failed(reason) => CallOutcome::Failed(reason),
        ApiError::Rejected(reason) => CallOutcome::Rejected(reason),
    }
}

fn canonical_inside(workspace: &Path, program: &str) -> Result<PathBuf> {
    let program_path = Path::new(program);
    if !program_path.is_absolute() && program_path.components().count() <= 1 {
        return Err(err("runner_expects_path"));
    }
    let real_workspace = std::fs::canonicalize(workspace)
        .map_err(|e| err(format!("runner_workspace_unresolved:{e}")))?;
    let program_path = if program_path.is_absolute() {
        program_path.to_path_buf()
    } else {
        workspace.join(program_path)
    };
    let real_program = std::fs::canonicalize(&program_path)
        .map_err(|e| err(format!("runner_missing:{e}")))?;
    if !real_program.starts_with(&real_workspace) {
        return Err(err("runner_outside_workspace"));
    }
    Ok(real_program)
}

/// Resolves the configured program against the workspace boundary. A program
/// outside the workspace is treated as unavailable and is never spawned.
pub fn prepare(workspace: &Path, config: &RunnerConfig) -> std::result::Result<PathBuf, String> {
    match canonical_inside(workspace, &config.program) {
        Ok(path) => Ok(path),
        Err(e) => Err(e.0),
    }
}

#[derive(Deserialize)]
struct RawResponse {
    protocol: Option<String>,
    label: Option<String>,
    confidence: Option<String>,
    abstain_reason: Option<String>,
    model_id: Option<String>,
    model_sha256: Option<String>,
}

/// Runs one request against the workspace-local process protocol. `stage` is
/// `slm` or `judge`; the response cannot change which stage it is.
pub fn invoke(
    program: &Path,
    config: &RunnerConfig,
    workspace: &Path,
    request: &Value,
) -> CallOutcome {
    let payload = match serde_json::to_vec(request) {
        Ok(payload) => payload,
        Err(e) => return CallOutcome::Failed(format!("request_encode:{e}")),
    };
    let mut command = Command::new(program);
    command
        .args(&config.args)
        .current_dir(workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => return CallOutcome::Failed(format!("spawn:{e}")),
    };
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(&payload) {
            let _ = child.kill();
            let _ = child.wait();
            return CallOutcome::Failed(format!("stdin:{e}"));
        }
    }
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_thread = thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(mut handle) = stdout {
            let _ = handle.read_to_end(&mut buffer);
        }
        buffer
    });
    let stderr_thread = thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(mut handle) = stderr {
            let _ = handle.read_to_end(&mut buffer);
        }
        buffer
    });
    let timeout = Duration::from_millis(config.timeout_ms.max(1));
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break None;
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return CallOutcome::Failed(format!("wait:{e}"));
            }
        }
    };
    let stdout_bytes = stdout_thread.join().unwrap_or_default();
    let stderr_bytes = stderr_thread.join().unwrap_or_default();
    if timed_out {
        return CallOutcome::Failed("timeout".to_string());
    }
    let status = match status {
        Some(status) => status,
        None => return CallOutcome::Failed("no_exit_status".to_string()),
    };
    if !status.success() {
        return CallOutcome::Failed(format!(
            "exit_status:{:?};stderr_bytes:{}",
            status.code(),
            stderr_bytes.len()
        ));
    }
    let raw: RawResponse = match serde_json::from_slice(&stdout_bytes) {
        Ok(raw) => raw,
        Err(e) => return CallOutcome::Rejected(format!("response_json:{e}")),
    };
    if raw.protocol.as_deref() != Some(LOCAL_MODEL_PROTOCOL) {
        return CallOutcome::Rejected("protocol_mismatch".to_string());
    }
    let Some(label) = raw.label.as_deref().and_then(Decision::parse) else {
        return CallOutcome::Rejected("invalid_label".to_string());
    };
    let (Some(model_id), Some(model_sha256)) = (raw.model_id, raw.model_sha256) else {
        return CallOutcome::Rejected("missing_model_identity".to_string());
    };
    if model_id != config.model_id {
        return CallOutcome::Rejected("model_id_mismatch".to_string());
    }
    if model_sha256.to_ascii_lowercase() != config.model_sha256.to_ascii_lowercase() {
        return CallOutcome::Rejected("model_digest_mismatch".to_string());
    }
    let confidence = match raw.confidence {
        Some(level) => {
            let normalized = level.trim().to_ascii_lowercase();
            if !rubric::VERBAL_VALUES.contains(&normalized.as_str()) {
                return CallOutcome::Rejected("invalid_confidence".to_string());
            }
            Some(ModelConfidence::Verbal(normalized))
        }
        None => None,
    };
    let abstain_reason = if label == Decision::Unknown {
        Some(
            raw.abstain_reason
                .filter(|reason| !reason.trim().is_empty())
                .unwrap_or_else(|| "model_chose_unknown".to_string()),
        )
    } else {
        None
    };
    CallOutcome::Ok(RunnerResponse {
        label,
        confidence,
        abstain_reason,
        model_id,
        model_sha256,
        input_tokens: None,
        output_tokens: None,
    })
}

/// One typed API call for a stage. The instruction contains no session text;
/// the rendered pair is passed as data. A response that does not match the
/// shared rubric is rejected, never guessed.
pub fn invoke_api(stage: &ApiStage, instruction: &str, input: &str) -> CallOutcome {
    let profile = &stage.resolution.profile;
    let identity = ase_llm::identity(profile);
    let spec = CallSpec {
        api: profile.api,
        model: &profile.model,
        effort: profile.effort.as_deref(),
        instruction,
        input,
        question_id: rubric::QUESTION_ID,
        choices: &rubric::CHOICES,
    };
    let timeout = Duration::from_millis(stage.timeout_ms.max(1));
    let result = match ase_llm::call(&stage.resolution, &spec, timeout) {
        Ok(result) => result,
        Err(error) => return api_outcome(error),
    };
    let (label, confidence, abstain_reason) =
        match rubric::interpret(profile.api, result.answer) {
            Ok(mapped) => mapped,
            Err(code) => return CallOutcome::Rejected(code.to_string()),
        };
    CallOutcome::Ok(RunnerResponse {
        label,
        confidence,
        abstain_reason,
        model_id: profile.model.clone(),
        model_sha256: identity.digest,
        input_tokens: result.usage.input_tokens,
        output_tokens: result.usage.output_tokens,
    })
}

/// Declared identity string used as `generator_id` for process-model
/// assertions. API stages use the full versioned identity from `ase-llm`.
pub fn generator_id(model_id: &str, model_sha256: &str) -> String {
    let digest = model_sha256.to_ascii_lowercase();
    format!("{model_id}#{}", &digest[..12.min(digest.len())])
}

/// The configured model backend for one stage.
pub enum StageModel {
    /// Workspace-local program speaking the stdin/stdout protocol.
    Process(RunnerConfig),
    /// One resolved typed API profile.
    Api(ApiStage),
}
