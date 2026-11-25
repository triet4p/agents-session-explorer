//! Typed API calls over the protocol and transport layers.
//!
//! `call` is the only function in this crate that opens a socket. It builds
//! the protocol request, adds the credential at the last moment, refuses
//! redirects, and converts one response envelope into a typed answer. Nothing
//! here retries: a capability or auth failure is reported once and never
//! re-sent with a different effort, model or endpoint.
//!
//! Diagnostics are stable codes. Error payloads, prompts and response bodies
//! are never placed in an error string, so a key cannot leak through a
//! transport message and a rejected response cannot echo session text.

use std::fmt;
use std::time::Duration;

use serde_json::json;

use crate::credentials::sha256_hex;
use crate::resolve::{ResolvedProfile, Resolution};
use crate::wire::{
    self, CallSpec, Usage, WireAnswer, WIRE_VERSION, PARSE_VERSION,
};

pub const DEFAULT_TIMEOUT_MS: u64 = 120_000;
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// Failure classes map one-to-one onto the evaluation runner's
/// `unavailable`/`failed`/`rejected` outcomes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiError {
    /// Configuration or platform precondition; no request was sent.
    Unavailable(String),
    /// Transport failure or non-2xx response. No body text is retained.
    Failed(String),
    /// A 3xx redirect or a response that does not match the typed protocol.
    Rejected(String),
}

impl ApiError {
    pub fn kind(&self) -> &'static str {
        match self {
            ApiError::Unavailable(_) => "unavailable",
            ApiError::Failed(_) => "failed",
            ApiError::Rejected(_) => "rejected",
        }
    }

    pub fn detail(&self) -> &str {
        match self {
            ApiError::Unavailable(reason)
            | ApiError::Failed(reason)
            | ApiError::Rejected(reason) => reason,
        }
    }

    fn status(status: u16) -> Self {
        match status {
            400 => ApiError::Failed("bad_request".into()),
            401 | 403 => ApiError::Failed("auth_rejected".into()),
            404 => ApiError::Failed("endpoint_not_found".into()),
            408 => ApiError::Failed("request_timeout".into()),
            413 => ApiError::Failed("request_too_large".into()),
            429 => ApiError::Failed("rate_limited".into()),
            500..=599 => ApiError::Failed("server_error".into()),
            other => ApiError::Failed(format!("http_status:{other}")),
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.kind(), self.detail())
    }
}

impl std::error::Error for ApiError {}

/// One completed protocol call, with nonsecret request metadata needed by the
/// smoke report and the evaluation statistics.
pub struct CallResult {
    pub answer: WireAnswer,
    pub usage: Usage,
    pub model: Option<String>,
    pub auth_header: &'static str,
    pub auth_scheme: &'static str,
    pub body_keys: Vec<String>,
    pub effort_sent: bool,
    pub session_sent: bool,
    pub status: u16,
}

/// Deterministic session identifier for the Console gateway only. It is
/// derived from nonsecret profile fields and is never a credential.
pub fn session_id(profile: &ResolvedProfile) -> String {
    let material = format!(
        "ase-llm-session/v1\u{0}{}\u{0}{}\u{0}{}",
        profile.provider, profile.endpoint.url, profile.model
    );
    let digest = sha256_hex(material.as_bytes());
    format!("ses_{}", &digest[..24])
}

/// The versioned cache identity of one API stage. It covers the typed API,
/// normalized endpoint, provider, model, resolved effort and the wire/parse
/// semantics; it never contains a credential. The plaintext generator id is
/// informational, while `digest` is the complete identity hash.
pub struct ApiIdentity {
    pub generator_id: String,
    pub digest: String,
}

pub fn identity(profile: &ResolvedProfile) -> ApiIdentity {
    let material = json!({
        "schema": "ase-llm-api-identity/v1",
        "api": profile.api.as_str(),
        "provider": profile.provider,
        "endpoint": profile.endpoint.url,
        "origin": profile.endpoint.origin,
        "model": profile.model,
        "effort": profile.effort,
        "wire_version": WIRE_VERSION,
        "parse_version": PARSE_VERSION,
    });
    let bytes = serde_json::to_vec(&material).unwrap_or_default();
    let digest = sha256_hex(&bytes);
    let generator_id = format!(
        "{}:{}:{}#{}",
        profile.provider,
        profile.api.as_str(),
        profile.model,
        &digest[..12]
    );
    ApiIdentity {
        generator_id,
        digest,
    }
}

fn classify_status(status: u16) -> Result<(), ApiError> {
    if (200..300).contains(&status) {
        return Ok(());
    }
    if (300..400).contains(&status) {
        return Err(ApiError::Rejected("redirect_refused".into()));
    }
    Err(ApiError::status(status))
}

/// Executes one typed call. No retry, no redirect, no credential forwarding.
pub fn call(
    resolution: &Resolution,
    spec: &CallSpec<'_>,
    timeout: Duration,
) -> Result<CallResult, ApiError> {
    let profile = &resolution.profile;
    if spec.api != profile.api {
        return Err(ApiError::Unavailable("api_kind_mismatch".into()));
    }
    let session = if wire::is_console_gateway(&profile.endpoint) {
        Some(session_id(profile))
    } else {
        None
    };
    let request = wire::build_request(spec, &profile.endpoint, session.as_deref())?;
    let outcome = crate::transport::send(&request, &resolution.key, timeout)?;
    classify_status(outcome.status)?;
    let parsed = wire::parse_response(spec.api, spec.question_id, &outcome.body)?;
    Ok(CallResult {
        answer: parsed.answer,
        usage: parsed.usage,
        model: parsed.model,
        auth_header: request.auth.header_name(),
        auth_scheme: request.auth.scheme(),
        body_keys: request.body_keys,
        effort_sent: request.effort_sent,
        session_sent: request.session_sent,
        status: outcome.status,
    })
}

/// Convenience for report builders: usage as JSON without any body text.
pub fn usage_json(usage: &Usage) -> serde_json::Value {
    json!({
        "input_tokens": usage.input_tokens,
        "output_tokens": usage.output_tokens,
    })
}
