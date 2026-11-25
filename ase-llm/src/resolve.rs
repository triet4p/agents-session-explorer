//! Nonsecret resolution and the redacted local status contract consumed by the
//! future API transport (T5).
//!
//! Precedence is explicit: a nonempty `OPENAI_API_KEY` in the process
//! environment overrides the stored credential; otherwise the credential is
//! looked up by the *resolved* endpoint origin. `OPENAI_API_TYPE`,
//! `OPENAI_API_ENDPOINT`, `OPENAI_API_MODEL` and `OPENAI_API_EFFORT` override
//! the selected profile field by field. No function here opens a socket.

use ase_core::{err, Result};
use serde::Serialize;

use crate::config::{
    env_raw, env_value, load_config, validate_effort, validate_model, validate_profile_name,
    validate_provider_name, ApiKind, Env,
};
use crate::credentials::{open_store, CredentialStore};
use crate::secret::ApiKey;
use crate::url::{validate_endpoint, Endpoint};

pub const OPENAI_KEY_ENV: &str = "OPENAI_API_KEY";
pub const GEMINI_KEY_ENV: &str = "GEMINI_API_KEY";
/// Explicit provider scope. Required for environment-only configuration and
/// available as an override so switching provider is never implicit.
pub const PROVIDER_ENV: &str = "ASE_LLM_PROVIDER";
pub const TYPE_ENV: &str = "OPENAI_API_TYPE";
pub const ENDPOINT_ENV: &str = "OPENAI_API_ENDPOINT";
pub const MODEL_ENV: &str = "OPENAI_API_MODEL";
pub const EFFORT_ENV: &str = "OPENAI_API_EFFORT";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeySource {
    /// Which process-only key family supplied the override. The two families
    /// never fall back to each other.
    Environment(&'static str),
    Store(&'static str),
}

impl KeySource {
    pub fn as_str(self) -> String {
        match self {
            KeySource::Environment(name) => format!("env:{name}"),
            KeySource::Store(backend) => format!("store:{backend}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedProfile {
    pub profile: String,
    /// Explicit provider scope; credentials are looked up for
    /// `(provider, endpoint.origin)`.
    pub provider: String,
    pub api: ApiKind,
    pub endpoint: Endpoint,
    pub model: String,
    /// `None` for the Decision API; `Some("medium")` by default for the two
    /// text-generating APIs.
    pub effort: Option<String>,
    pub env_overrides: Vec<&'static str>,
    pub notes: Vec<String>,
}

/// The complete input a transport needs. The key is only reachable through
/// `key.expose()`; everything else is nonsecret.
pub struct Resolution {
    pub profile: ResolvedProfile,
    pub key: ApiKey,
    pub key_source: KeySource,
}

impl Resolution {
    pub fn endpoint_url(&self) -> &str {
        &self.profile.endpoint.url
    }

    pub fn origin(&self) -> &str {
        &self.profile.endpoint.origin
    }
}

pub fn resolve_profile(env: &Env, requested: Option<&str>) -> Result<ResolvedProfile> {
    if let Some(name) = requested {
        validate_profile_name(name)?;
    }
    let loaded = load_config(env)?;
    let (profile_name, mut provider, mut api, mut endpoint, mut model, effort) = match loaded.config {
        Some(config) => {
            let name = match requested {
                Some(name) => {
                    if !config.profiles.contains_key(name) {
                        return Err(err("llm_profile_not_found"));
                    }
                    name.to_string()
                }
                None => config.active.clone().ok_or_else(|| {
                    err("llm_profile_not_selected: run `ase llm config use --profile <name>`")
                })?,
            };
            let profile = &config.profiles[&name];
            (
                name,
                Some(profile.provider.clone()),
                Some(profile.api),
                Some(validate_endpoint(&profile.endpoint)?),
                Some(profile.model.clone()),
                profile.effort.clone(),
            )
        }
        None => {
            let name = requested.unwrap_or("env").to_string();
            (name, None, None, None, None, None)
        }
    };

    let mut env_overrides = Vec::new();
    let mut notes = Vec::new();

    if let Some(value) = env_value(env, PROVIDER_ENV) {
        validate_provider_name(&value)?;
        provider = Some(value);
        env_overrides.push(PROVIDER_ENV);
    }
    if let Some(value) = env_value(env, TYPE_ENV) {
        api = Some(
            ApiKind::parse(&value)
                .ok_or_else(|| err("openai_api_type_invalid: expected chat-completions, responses, decision, gemini or gemini-decision"))?,
        );
        env_overrides.push(TYPE_ENV);
    }
    if let Some(value) = env_value(env, ENDPOINT_ENV) {
        endpoint = Some(validate_endpoint(&value)?);
        env_overrides.push(ENDPOINT_ENV);
    }
    if let Some(value) = env_value(env, MODEL_ENV) {
        validate_model(&value)?;
        model = Some(value.trim().to_string());
        env_overrides.push(MODEL_ENV);
    }

    let provider = provider.ok_or_else(|| {
        err("openai_api_provider_missing: set ASE_LLM_PROVIDER or select a profile with an explicit provider")
    })?;
    let api = api.ok_or_else(|| {
        err("openai_api_type_missing: set OPENAI_API_TYPE or create a profile with `ase llm config set`")
    })?;
    let endpoint = endpoint.ok_or_else(|| {
        err("openai_api_endpoint_missing: set OPENAI_API_ENDPOINT or create a profile")
    })?;
    let model = model.ok_or_else(|| {
        err("openai_api_model_missing: set OPENAI_API_MODEL or create a profile")
    })?;

    let env_effort = env_value(env, EFFORT_ENV);
    let effort = if !api.supports_openai_effort() {
        if env_effort.is_some() {
            notes.push(format!(
                "OPENAI_API_EFFORT is ignored: the {} API does not use the OpenAI effort parameter",
                api.as_str()
            ));
        }
        None
    } else if let Some(value) = env_effort {
        env_overrides.push(EFFORT_ENV);
        validate_effort(api, Some(&value))?
    } else {
        validate_effort(api, effort.as_deref())?
    };
    let effort = if api.supports_openai_effort() {
        Some(
            effort
                .or_else(|| api.default_effort().map(str::to_string))
                .expect("OpenAI-compatible generation APIs always resolve an effort"),
        )
    } else {
        None
    };

    Ok(ResolvedProfile {
        profile: profile_name,
        provider,
        api,
        endpoint,
        model,
        effort,
        env_overrides,
        notes,
    })
}

pub fn resolve_with_store(
    env: &Env,
    requested: Option<&str>,
    store: &dyn CredentialStore,
) -> Result<Resolution> {
    let profile = resolve_profile(env, requested)?;
    let key_env = profile.api.key_env();
    let (key, key_source) = match env_raw(env, key_env) {
        Some(value) => (ApiKey::parse(&value)?, KeySource::Environment(key_env)),
        None => match store.get(&profile.provider, &profile.endpoint.origin)? {
            Some(key) => (key, KeySource::Store(store.backend())),
            None => {
                return Err(err(format!(
                    "api_key_missing: set {key_env}, or run `ase llm key set` for provider {} at this origin",
                    profile.provider
                )))
            }
        },
    };
    Ok(Resolution {
        profile,
        key,
        key_source,
    })
}

pub fn resolve(env: &Env, requested: Option<&str>) -> Result<Resolution> {
    let store = open_store(env)?;
    resolve_with_store(env, requested, store.as_ref())
}

/// Whether the nonempty process key override for this typed API is present.
/// The value itself is never returned, and the OpenAI/Gemini families never
/// borrow each other's override.
pub fn env_key_present(env: &Env, api: ApiKind) -> bool {
    env_raw(env, api.key_env()).is_some()
}

#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct StatusReport {
    pub version: u32,
    pub config_path: String,
    pub config_source: String,
    pub configured: bool,
    pub ready: bool,
    pub active_profile: Option<String>,
    pub profile: Option<String>,
    pub provider: Option<String>,
    pub api: Option<String>,
    pub endpoint: Option<String>,
    pub origin: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub credential_source: String,
    pub credential_store: String,
    pub env_overrides: Vec<String>,
    pub notes: Vec<String>,
    pub checks: Vec<Check>,
    pub network: &'static str,
}

/// Local, redacted readiness report. It never resolves or prints the key; it
/// only records presence and backend. No socket is opened.
pub fn status(env: &Env, requested: Option<&str>) -> Result<StatusReport> {
    let loaded = load_config(env)?;
    let configured = loaded.config.is_some() || env_value(env, ENDPOINT_ENV).is_some();
    let mut checks = vec![Check {
        name: "config_file",
        ok: true,
        detail: match loaded.source.as_str() {
            "file" => "present".to_string(),
            _ => "absent (environment-only configuration)".to_string(),
        },
    }];
    let mut report = StatusReport {
        version: 1,
        config_path: loaded.path.display().to_string(),
        config_source: loaded.source.as_str().to_string(),
        configured,
        ready: false,
        active_profile: loaded
            .config
            .as_ref()
            .and_then(|config| config.active.clone()),
        profile: None,
        provider: None,
        api: None,
        endpoint: None,
        origin: None,
        model: None,
        effort: None,
        credential_source: "missing".to_string(),
        credential_store: "unknown".to_string(),
        env_overrides: Vec::new(),
        notes: Vec::new(),
        checks: Vec::new(),
        network: "not_used",
    };
    if !configured {
        checks.push(Check {
            name: "endpoint",
            ok: false,
            detail: "no profile configured (run `ase llm config enroll`)".to_string(),
        });
        checks.push(Check {
            name: "credential",
            ok: false,
            detail: "no profile configured".to_string(),
        });
        report.checks = checks;
        return Ok(report);
    }

    let profile = match resolve_profile(env, requested) {
        Ok(profile) => profile,
        Err(error) => {
            checks.push(Check {
                name: "profile",
                ok: false,
                detail: error.to_string(),
            });
            report.checks = checks;
            return Ok(report);
        }
    };
    let env_key_present = env_key_present(env, profile.api);
    checks.push(Check {
        name: "provider",
        ok: true,
        detail: profile.provider.clone(),
    });
    checks.push(Check {
        name: "api_type",
        ok: true,
        detail: profile.api.as_str().to_string(),
    });
    checks.push(Check {
        name: "endpoint",
        ok: true,
        detail: profile.endpoint.url.clone(),
    });
    checks.push(Check {
        name: "model",
        ok: true,
        detail: profile.model.clone(),
    });
    checks.push(Check {
        name: "effort_policy",
        ok: true,
        detail: match &profile.effort {
            Some(effort) => format!("{effort} ({} default)", profile.api.as_str()),
            None => format!(
                "none ({} never sends an effort parameter)",
                profile.api.as_str()
            ),
        },
    });

    report.profile = Some(profile.profile.clone());
    report.provider = Some(profile.provider.clone());
    report.api = Some(profile.api.as_str().to_string());
    report.endpoint = Some(profile.endpoint.url.clone());
    report.origin = Some(profile.endpoint.origin.clone());
    report.model = Some(profile.model.clone());
    report.effort = profile.effort.clone();
    report.env_overrides = profile
        .env_overrides
        .iter()
        .map(|name| name.to_string())
        .collect();
    report.notes = profile.notes.clone();
    report.credential_source = "missing".to_string();

    match open_store(env) {
        Err(error) => {
            report.credential_store = "unrecognized".to_string();
            checks.push(Check {
                name: "credential_store",
                ok: false,
                detail: error.to_string(),
            });
            checks.push(Check {
                name: "credential",
                ok: false,
                detail: "credential store could not be opened".to_string(),
            });
        }
        Ok(store) => {
            let backend = store.backend();
            report.credential_store = backend.to_string();
            if env_key_present {
                report.credential_source = KeySource::Environment(profile.api.key_env()).as_str();
                checks.push(Check {
                    name: "credential_store",
                    ok: true,
                    detail: match backend {
                        crate::credentials::UNSUPPORTED_BACKEND => {
                            format!(
                            "not required: {} process override",
                            profile.api.key_env()
                        )
                        }
                        other => other.to_string(),
                    },
                });
                checks.push(Check {
                    name: "origin_binding",
                    ok: true,
                    detail: format!(
                        "credential lookup keyed by provider {} at {}",
                        profile.provider, profile.endpoint.origin
                    ),
                });
                checks.push(Check {
                    name: "credential",
                    ok: true,
                    detail: format!(
                        "present via {} (value not read or printed)",
                        profile.api.key_env()
                    ),
                });
            } else {
                let store_check_ok = backend != crate::credentials::UNSUPPORTED_BACKEND;
                checks.push(Check {
                    name: "credential_store",
                    ok: store_check_ok,
                    detail: if store_check_ok {
                        backend.to_string()
                    } else {
                        "no native user credential store on this platform".to_string()
                    },
                });
                checks.push(Check {
                    name: "origin_binding",
                    ok: true,
                    detail: format!(
                        "credential lookup keyed by provider {} at {}",
                        profile.provider, profile.endpoint.origin
                    ),
                });
                match store.get(&profile.provider, &profile.endpoint.origin) {
                    Ok(Some(_key)) => {
                        report.credential_source = KeySource::Store(backend).as_str();
                        checks.push(Check {
                            name: "credential",
                            ok: true,
                            detail: format!("present in {backend} for this origin"),
                        });
                    }
                    Ok(None) => {
                        checks.push(Check {
                            name: "credential",
                            ok: false,
                            detail: format!(
                                "missing for provider {} at {}: run `ase llm key set` or set {}",
                                profile.provider,
                                profile.endpoint.origin,
                                profile.api.key_env()
                            ),
                        });
                    }
                    Err(error) => {
                        report.credential_source = "error".to_string();
                        checks.push(Check {
                            name: "credential",
                            ok: false,
                            detail: error.to_string(),
                        });
                    }
                }
            }
        }
    }

    report.ready = checks.iter().all(|check| check.ok);
    report.checks = checks;
    Ok(report)
}
