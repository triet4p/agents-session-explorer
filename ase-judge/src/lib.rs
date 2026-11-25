//! Offline evaluation layer: deterministic baseline, workspace-local model
//! boundary and a separate Judge over a reopenable, pinned projection.
//!
//! Boundaries this crate keeps:
//! - every assertion carries two exact source spans and a rendered-input
//!   digest; insertion re-opens the spans and rejects missing evidence;
//! - cache identity is span/input + version based, never a whole-journal hash,
//!   and a configured API backend contributes its typed protocol, normalized
//!   endpoint, provider, model, effort and wire semantics to that identity;
//! - the SLM and the Judge are separate generators with independent prompt
//!   versions, runner configurations and cache namespaces;
//! - model access is either a workspace-local process protocol (no network) or
//!   one explicitly resolved typed API profile; no read/query command reaches
//!   a model or a network;
//! - a request that would carry session text to a non-loopback endpoint needs
//!   an explicit run opt-in; a synthetic smoke path sends no session data;
//! - a model stage that cannot run is reported unavailable and never replaced
//!   by baseline or fixture output.

pub mod assertion;
pub mod baseline;
pub mod evaluate;
pub mod rubric;
pub mod runner;
pub mod source;
pub mod store;

#[cfg(test)]
mod tests;

pub use assertion::{
    sha256_hex, Anchor, Assertion, Decision, Evidence, Generator, BASELINE_GENERATOR_ID,
    BASELINE_RULE_VERSION, INPUT_RENDER_VERSION, JUDGE_PROMPT_VERSION, LOCAL_MODEL_PROTOCOL,
    SLM_PROMPT_VERSION, VIEW_VERSION,
};
pub use evaluate::{
    report, run, show, verify, write_fixture, EvalOptions, PinReport, RunReport, Scope,
    StageStats, VerifyOptions, VerifyReport,
};
pub use rubric::{decode_confidence, interpret, ModelConfidence};
pub use runner::{ApiStage, CallOutcome, RunnerConfig, RunnerResponse, StageModel};
pub use source::{CorpusSource, EvidenceVerifier, Journal};
pub use store::{InsertOutcome, Store, StoredAssertion};
