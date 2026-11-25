//! Nonsecret LLM profile configuration. The user config file holds the
//! request endpoint, typed API, model and optional effort only; the API key is
//! never a field here. Files are written atomically and strictly parsed
//! (`deny_unknown_fields`) so a manually added `key`/`token` field is a load
//! error rather than a silent plaintext secret.

use ase_core::{err, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::url::validate_endpoint;

/// Config schema version. Version 2 adds the mandatory explicit `provider`
/// scope for credentials; a version-1 file is rejected instead of being
/// reinterpreted with an implicit provider.
pub const CONFIG_VERSION: u32 = 2;
pub const CONFIG_ENV: &str = "ASE_LLM_CONFIG";
pub const DEFAULT_PROFILE: &str = "default";
pub const MAX_PROFILE_NAME_LEN: usize = 64;
pub const MAX_PROVIDER_NAME_LEN: usize = 64;
pub const MAX_MODEL_LEN: usize = 200;
pub const EFFORT_VALUES: [&str; 5] = ["none", "minimal", "low", "medium", "high"];
pub const DEFAULT_TEXT_EFFORT: &str = "medium";

pub type Env = BTreeMap<String, String>;

/// Snapshot of the process environment. Tests pass an explicit map instead.
pub fn capture_env() -> Env {
    std::env::vars().collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApiKind {
    /// OpenAI-compatible Chat Completions generation.
    ChatCompletions,
    /// OpenAI-compatible Responses generation.
    Responses,
    /// Console System One Decision.
    Decision,
    /// Native Gemini generation (distinct endpoint and auth family).
    Gemini,
    /// Native Gemini Decisions (distinct endpoint and auth family).
    GeminiDecision,
}

impl ApiKind {
    pub const ALL: [ApiKind; 5] = [
        ApiKind::ChatCompletions,
        ApiKind::Responses,
        ApiKind::Decision,
        ApiKind::Gemini,
        ApiKind::GeminiDecision,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ApiKind::ChatCompletions => "chat-completions",
            ApiKind::Responses => "responses",
            ApiKind::Decision => "decision",
            ApiKind::Gemini => "gemini",
            ApiKind::GeminiDecision => "gemini-decision",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "chat-completions" => Some(ApiKind::ChatCompletions),
            "responses" => Some(ApiKind::Responses),
            "decision" => Some(ApiKind::Decision),
            "gemini" => Some(ApiKind::Gemini),
            "gemini-decision" => Some(ApiKind::GeminiDecision),
            _ => None,
        }
    }

    /// Both decision kinds share the "produces a decision, no generation
    /// effort parameter" property; they remain distinct typed APIs.
    pub fn is_decision(self) -> bool {
        matches!(self, ApiKind::Decision | ApiKind::GeminiDecision)
    }

    /// Native Gemini kinds use the `GEMINI_API_KEY` process override; every
    /// other kind uses `OPENAI_API_KEY`. The key families never fall back to
    /// each other.
    pub fn key_env(self) -> &'static str {
        match self {
            ApiKind::Gemini | ApiKind::GeminiDecision => "GEMINI_API_KEY",
            _ => "OPENAI_API_KEY",
        }
    }

    /// Only the two OpenAI-compatible generation protocols carry the OpenAI
    /// effort parameter. Native Gemini generation and both decision kinds
    /// never resolve or send it.
    pub fn supports_openai_effort(self) -> bool {
        matches!(self, ApiKind::ChatCompletions | ApiKind::Responses)
    }

    /// `medium` for the two OpenAI-compatible generation protocols; every
    /// other typed API has no effort.
    pub fn default_effort(self) -> Option<&'static str> {
        if self.supports_openai_effort() {
            Some(DEFAULT_TEXT_EFFORT)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Explicit user-defined provider scope. Credentials are stored per
    /// `(provider, normalized origin)`: profiles of the same provider on the
    /// same origin share one key, while two providers on one origin stay
    /// isolated. There is no inference from the origin, URL or profile name.
    pub provider: String,
    pub api: ApiKind,
    /// Normalized full request endpoint URL.
    pub endpoint: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

impl ConfigFile {
    pub fn empty() -> Self {
        ConfigFile {
            version: CONFIG_VERSION,
            active: None,
            profiles: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LoadedConfig {
    pub path: PathBuf,
    /// `None` when the config file does not exist yet.
    pub config: Option<ConfigFile>,
    pub source: ConfigSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigSource {
    File,
    Missing,
}

impl ConfigSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ConfigSource::File => "file",
            ConfigSource::Missing => "none",
        }
    }
}

/// Resolves the user config path: `ASE_LLM_CONFIG` wins, then the platform
/// configuration home. No default path is written unless the owner runs a
/// mutating command.
pub fn config_path(env: &Env) -> Result<PathBuf> {
    if let Some(value) = env_value(env, CONFIG_ENV) {
        return Ok(PathBuf::from(value));
    }
    #[cfg(windows)]
    {
        let appdata = env_value(env, "APPDATA")
            .ok_or_else(|| err("llm_config_home_unavailable: APPDATA is not set"))?;
        return Ok(PathBuf::from(appdata).join("ase").join("llm-config.json"));
    }
    #[cfg(not(windows))]
    {
        if let Some(value) = env_value(env, "XDG_CONFIG_HOME") {
            return Ok(PathBuf::from(value).join("ase").join("llm-config.json"));
        }
        let home = env_value(env, "HOME")
            .ok_or_else(|| err("llm_config_home_unavailable: HOME is not set"))?;
        Ok(PathBuf::from(home)
            .join(".config")
            .join("ase")
            .join("llm-config.json"))
    }
}

pub fn load_config(env: &Env) -> Result<LoadedConfig> {
    let path = config_path(env)?;
    match std::fs::read(&path) {
        Ok(bytes) => {
            let config: ConfigFile = serde_json::from_slice(&bytes).map_err(|e| {
                err(format!("llm_config_invalid_json: {}", e))
            })?;
            validate_config(&config)?;
            Ok(LoadedConfig {
                path,
                config: Some(config),
                source: ConfigSource::File,
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(LoadedConfig {
            path,
            config: None,
            source: ConfigSource::Missing,
        }),
        Err(error) => Err(err(format!("llm_config_read_failed: {error}"))),
    }
}

pub fn save_config(env: &Env, config: &ConfigFile) -> Result<PathBuf> {
    validate_config(config)?;
    let path = config_path(env)?;
    let parent = path
        .parent()
        .ok_or_else(|| err("llm_config_path_has_no_parent"))?;
    std::fs::create_dir_all(parent)
        .map_err(|e| err(format!("llm_config_dir_create_failed: {e}")))?;
    let bytes = serde_json::to_vec_pretty(config)
        .map_err(|e| err(format!("llm_config_serialize_failed: {e}")))?;
    let temp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&temp, &bytes)
        .map_err(|e| err(format!("llm_config_write_failed: {e}")))?;
    std::fs::rename(&temp, &path)
        .map_err(|e| err(format!("llm_config_publish_failed: {e}")))?;
    Ok(path)
}

fn validate_config(config: &ConfigFile) -> Result<()> {
    if config.version != CONFIG_VERSION {
        return Err(err(format!(
            "llm_config_version_unsupported: {}",
            config.version
        )));
    }
    for (index, (name, profile)) in config.profiles.iter().enumerate() {
        validate_profile_name(name)?;
        validate_provider_name(&profile.provider)
            .map_err(|e| err(format!("profile #{index}: {e}")))?;
        // Position, not the configured name: a name is nonsecret by contract,
        // but an accidental paste must not be echoed by an error payload.
        validate_endpoint(&profile.endpoint)
            .map_err(|e| err(format!("profile #{index}: {e}")))?;
        validate_model(&profile.model)
            .map_err(|e| err(format!("profile #{index}: {e}")))?;
        validate_effort(profile.api, profile.effort.as_deref())
            .map_err(|e| err(format!("profile #{index}: {e}")))?;
    }
    if let Some(active) = &config.active {
        if !config.profiles.contains_key(active) {
            return Err(err("llm_active_profile_missing"));
        }
    }
    Ok(())
}

pub fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > MAX_PROFILE_NAME_LEN {
        return Err(err("llm_profile_name_invalid"));
    }
    if name == "." || name == ".." {
        return Err(err("llm_profile_name_invalid"));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        return Err(err("llm_profile_name_invalid"));
    }
    Ok(())
}

pub fn validate_provider_name(provider: &str) -> Result<()> {
    if provider.is_empty() || provider.len() > MAX_PROVIDER_NAME_LEN {
        return Err(err("llm_provider_name_invalid"));
    }
    if provider == "." || provider == ".." {
        return Err(err("llm_provider_name_invalid"));
    }
    // `|` separates provider and origin in the credential identity, so the
    // charset deliberately excludes it.
    if !provider
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        return Err(err("llm_provider_name_invalid"));
    }
    Ok(())
}

pub fn validate_model(model: &str) -> Result<()> {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return Err(err("llm_model_missing"));
    }
    if trimmed.len() > MAX_MODEL_LEN {
        return Err(err("llm_model_too_long"));
    }
    if trimmed
        .bytes()
        .any(|b| b <= 0x20 || b == 0x7f || b > 0x7e)
    {
        return Err(err("llm_model_invalid"));
    }
    Ok(())
}

pub fn validate_effort(api: ApiKind, effort: Option<&str>) -> Result<Option<String>> {
    let Some(value) = effort else {
        return Ok(None);
    };
    if !api.supports_openai_effort() {
        return Err(err("api_kind_has_no_effort"));
    }
    let normalized = value.trim().to_ascii_lowercase();
    if !EFFORT_VALUES.contains(&normalized.as_str()) {
        return Err(err("llm_effort_invalid"));
    }
    Ok(Some(normalized))
}

#[derive(Clone, Debug)]
pub enum EffortEdit {
    Keep,
    Set(String),
    /// Explicitly absent. Only valid for the Decision API, which never sends
    /// an effort value.
    Clear,
}

#[derive(Clone, Debug)]
pub struct ProfileEdit<'a> {
    pub profile: &'a str,
    pub provider: Option<&'a str>,
    pub api: Option<ApiKind>,
    pub endpoint: Option<&'a str>,
    pub model: Option<&'a str>,
    pub effort: EffortEdit,
    pub make_active: bool,
}

/// Applies a create-or-update edit. New profiles need an API type, endpoint and
/// model; existing profiles accept partial updates. The endpoint is validated
/// and stored normalized before anything is written to disk.
pub fn apply_edit(config: &mut ConfigFile, edit: &ProfileEdit<'_>) -> Result<()> {
    validate_profile_name(edit.profile)?;
    if config.profiles.len() >= 64 && !config.profiles.contains_key(edit.profile) {
        return Err(err("llm_profile_limit_reached"));
    }
    let existing = config.profiles.get(edit.profile).cloned();
    let provider = match (edit.provider, existing.as_ref().map(|p| p.provider.as_str())) {
        (Some(provider), _) => {
            validate_provider_name(provider)?;
            provider.to_string()
        }
        (None, Some(existing)) => existing.to_string(),
        (None, None) => return Err(err("llm_provider_required_for_new_profile")),
    };
    let api = match (edit.api, existing.as_ref().map(|p| p.api)) {
        (Some(api), _) => api,
        (None, Some(api)) => api,
        (None, None) => return Err(err("llm_api_type_required_for_new_profile")),
    };
    let endpoint = match (edit.endpoint, existing.as_ref().map(|p| p.endpoint.as_str())) {
        (Some(raw), _) => validate_endpoint(raw)?.url,
        (None, Some(existing)) => existing.to_string(),
        (None, None) => return Err(err("llm_endpoint_required_for_new_profile")),
    };
    let model = match (edit.model, existing.as_ref().map(|p| p.model.as_str())) {
        (Some(raw), _) => {
            validate_model(raw)?;
            raw.trim().to_string()
        }
        (None, Some(existing)) => existing.to_string(),
        (None, None) => return Err(err("llm_model_required_for_new_profile")),
    };
    let existing_effort = existing.as_ref().and_then(|p| p.effort.clone());
    let effort = match &edit.effort {
        EffortEdit::Keep => {
            if api.supports_openai_effort() {
                validate_effort(api, existing_effort.as_deref())?
            } else {
                None
            }
        }
        EffortEdit::Set(value) => validate_effort(api, Some(value))?,
        EffortEdit::Clear => {
            if api.supports_openai_effort() {
                return Err(err("llm_effort_none_only_valid_for_text_api"));
            }
            None
        }
    };
    config.profiles.insert(
        edit.profile.to_string(),
        Profile {
            provider,
            api,
            endpoint,
            model,
            effort,
        },
    );
    if edit.make_active || config.active.is_none() {
        config.active = Some(edit.profile.to_string());
    }
    validate_config(config)
}

pub fn use_profile(config: &mut ConfigFile, name: &str) -> Result<()> {
    if !config.profiles.contains_key(name) {
        return Err(err("llm_profile_not_found"));
    }
    config.active = Some(name.to_string());
    Ok(())
}

pub fn env_value(env: &Env, name: &str) -> Option<String> {
    env.get(name)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The raw key override is handled separately: trimming only trailing CR/LF
/// keeps the value intact while still tolerating a piped environment.
pub fn env_raw(env: &Env, name: &str) -> Option<String> {
    env.get(name)
        .map(|value| value.trim_end_matches(['\r', '\n']).to_string())
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_roundtrip_and_decision_effort_policy() {
        let mut config = ConfigFile::empty();
        apply_edit(
            &mut config,
            &ProfileEdit {
                profile: "chat",
                provider: Some("openai"),
                api: Some(ApiKind::ChatCompletions),
                endpoint: Some("https://API.OpenAI.com:443/v1/chat/completions"),
                model: Some("gpt-test"),
                effort: EffortEdit::Keep,
                make_active: false,
            },
        )
        .unwrap();
        let profile = &config.profiles["chat"];
        assert_eq!(profile.endpoint, "https://api.openai.com/v1/chat/completions");
        assert_eq!(config.active.as_deref(), Some("chat"));
        assert_eq!(profile.effort, None);

        apply_edit(
            &mut config,
            &ProfileEdit {
                profile: "decision",
                provider: Some("openai"),
                api: Some(ApiKind::Decision),
                endpoint: Some("https://api.openai.com/v1/responses"),
                model: Some("gpt-test"),
                effort: EffortEdit::Keep,
                make_active: false,
            },
        )
        .unwrap();
        assert_eq!(config.profiles["decision"].effort, None);
        assert!(apply_edit(
            &mut config,
            &ProfileEdit {
                profile: "decision",
                provider: Some("openai"),
                api: Some(ApiKind::Decision),
                endpoint: None,
                model: None,
                effort: EffortEdit::Set("high".into()),
                make_active: false,
            },
        )
        .is_err());
    }

    #[test]
    fn config_rejects_unknown_fields_and_invalid_names() {
        let raw = r#"{"version":2,"active":"a","profiles":{"a":{"provider":"p","api":"responses","endpoint":"https://api.openai.com/v1/responses","model":"m","key":"secret"}}}"#;
        assert!(serde_json::from_str::<ConfigFile>(raw).is_err());
        let mut config = ConfigFile::empty();
        assert!(apply_edit(
            &mut config,
            &ProfileEdit {
                profile: "../escape",
                provider: Some("p"),
                api: Some(ApiKind::Responses),
                endpoint: Some("https://api.openai.com/v1/responses"),
                model: Some("m"),
                effort: EffortEdit::Keep,
                make_active: false,
            },
        )
        .is_err());
    }

    #[test]
    fn effort_values_and_defaults() {
        assert_eq!(ApiKind::ChatCompletions.default_effort(), Some("medium"));
        assert_eq!(ApiKind::Responses.default_effort(), Some("medium"));
        for kind in [
            ApiKind::Decision,
            ApiKind::Gemini,
            ApiKind::GeminiDecision,
        ] {
            assert_eq!(kind.default_effort(), None);
            assert!(validate_effort(kind, Some("high")).is_err());
        }
        assert_eq!(
            validate_effort(ApiKind::Responses, Some("HIGH")).unwrap(),
            Some("high".to_string())
        );
        assert!(validate_effort(ApiKind::Responses, Some("extreme")).is_err());
        assert_eq!(
            validate_effort(ApiKind::ChatCompletions, Some("none")).unwrap(),
            Some("none".to_string())
        );
    }

    #[test]
    fn all_typed_apis_parse_and_keep_their_key_family() {
        for kind in ApiKind::ALL {
            assert_eq!(ApiKind::parse(kind.as_str()), Some(kind));
            let expected = match kind {
                ApiKind::Gemini | ApiKind::GeminiDecision => "GEMINI_API_KEY",
                _ => "OPENAI_API_KEY",
            };
            assert_eq!(kind.key_env(), expected);
        }
        assert_eq!(ApiKind::parse("gemini-decision"), Some(ApiKind::GeminiDecision));
        assert!(ApiKind::parse("unknown").is_none());
    }
}
