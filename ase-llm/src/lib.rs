//! User-scoped LLM API configuration, credential resolution, typed protocol
//! framing and HTTP transport.
//!
//! This crate is the configuration/secret/transport boundary between the CLI
//! and the evaluation adapter. It keeps five typed APIs distinct:
//! `chat-completions`, `responses`, `decision` (Console System One), `gemini`
//! (native generation) and `gemini-decision` (native Decisions).
//!
//! Boundaries kept here:
//!
//! - the API key is entered once from hidden stdin, stored in a user-scoped
//!   credential store and bound to the normalized endpoint *origin*; it is
//!   never accepted from argv, written to the config file or printed;
//! - nonsecret settings live in a JSON user config and may be overridden, in a
//!   fixed order, by `ASE_LLM_PROVIDER`, `OPENAI_API_TYPE`,
//!   `OPENAI_API_ENDPOINT`, `OPENAI_API_MODEL`, `OPENAI_API_EFFORT` and the
//!   typed API's key-family environment variable;
//! - the full request endpoint is validated and normalized; a resolved
//!   credential is looked up for the resolved origin, so a changed host can
//!   never reuse a stored key;
//! - only the two OpenAI-compatible text protocols carry an effort value;
//!   native Gemini and both decision kinds never do, and a configured effort
//!   is rejected rather than silently dropped or converted;
//! - `wire` builds one typed request and parses one typed response without a
//!   credential or a socket; `transport` is the only network path, refuses
//!   redirects and never forwards credentials;
//! - `api::call` performs exactly one request with no retry, never includes a
//!   prompt, response body or key in an error, and derives the cache identity
//!   from the typed API, normalized endpoint, provider, model, effort and
//!   versioned wire semantics;
//! - status is local and redacted: it reports presence and backend, never the
//!   secret, and opens no socket; only the explicit synthetic `smoke` path
//!   sends a request, and it sends no session data.

pub mod api;
pub mod config;
pub mod credentials;
pub mod prompt;
pub mod resolve;
pub mod secret;
pub mod transport;
pub mod url;
pub mod wire;

#[cfg(windows)]
mod win;

#[cfg(test)]
mod tests;

pub use api::{
    call, identity, session_id, usage_json, ApiError, ApiIdentity, CallResult,
    DEFAULT_TIMEOUT_MS, MAX_RESPONSE_BYTES,
};
pub use config::{
    apply_edit, capture_env, config_path, load_config, save_config, use_profile, ApiKind,
    ConfigFile, ConfigSource, EffortEdit, Env, Profile, ProfileEdit, CONFIG_ENV, CONFIG_VERSION,
    DEFAULT_PROFILE, DEFAULT_TEXT_EFFORT, EFFORT_VALUES,
};
pub use credentials::{
    open_store, selected_backend, CredentialStore, DPAPI_BACKEND, NATIVE_BACKEND, STORE_ENV,
    UNSUPPORTED_BACKEND,
};
pub use prompt::{read_secret_line, stdin_is_console, KEY_PROMPT};
pub use resolve::{
    env_key_present, resolve, resolve_profile, resolve_with_store, status, Check, KeySource,
    Resolution,
    ResolvedProfile, StatusReport, EFFORT_ENV, ENDPOINT_ENV, GEMINI_KEY_ENV, MODEL_ENV,
    OPENAI_KEY_ENV, PROVIDER_ENV, TYPE_ENV,
};
pub use secret::ApiKey;
pub use url::{validate_endpoint, Endpoint, MAX_ENDPOINT_LEN};
pub use wire::{
    AuthStyle, CallSpec, Usage, WireAnswer, WireRequest, WireResponse, PARSE_VERSION, SESSION_HEADER,
    WIRE_VERSION,
};
