//! Contract tests for provider+origin credential scope, resolution precedence,
//! effort policy and secret redaction. They protect observable behavior: a
//! stored key is reused only for the same explicit provider and the same
//! normalized origin, an environment key wins, the Decision API sends no
//! effort, and no status/debug payload contains the key.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::config::{
    apply_edit, load_config, save_config, use_profile, ApiKind, ConfigFile, EffortEdit,
    ProfileEdit, CONFIG_VERSION,
};
use crate::credentials::{open_store, CredentialStore};
use crate::resolve::{resolve_with_store, status, KeySource};
use crate::secret::ApiKey;
use crate::url::validate_endpoint;

const SYNTHETIC: &str = "sk-synthetic-t4-do-not-store-in-plaintext";
const OTHER: &str = "sk-synthetic-t4-other-provider";

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::current_dir()
        .expect("cwd")
        .join(".tmp")
        .join("ase-llm-tests")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn config_env(dir: &Path) -> BTreeMap<String, String> {
    env(&[(
        crate::config::CONFIG_ENV,
        dir.join("llm-config.json").to_str().expect("utf8 path"),
    )])
}

fn profile<'a>(
    name: &'a str,
    provider: &'a str,
    endpoint: &'a str,
    api: ApiKind,
) -> ProfileEdit<'a> {
    ProfileEdit {
        profile: name,
        provider: Some(provider),
        api: Some(api),
        endpoint: Some(endpoint),
        model: Some("gpt-synthetic"),
        effort: EffortEdit::Keep,
        make_active: false,
    }
}

fn seed_config(config: &mut ConfigFile, name: &str, provider: &str, api: ApiKind) {
    apply_edit(
        config,
        &profile(
            name,
            provider,
            "https://api.openai.com/v1/chat/completions",
            api,
        ),
    )
    .unwrap();
}

#[derive(Default)]
struct MemoryStore {
    entries: RefCell<BTreeMap<String, String>>,
}

impl MemoryStore {
    fn seed(&self, provider: &str, origin: &str, key: &str) {
        self.entries
            .borrow_mut()
            .insert(format!("{provider}|{origin}"), key.to_string());
    }

    fn len(&self) -> usize {
        self.entries.borrow().len()
    }
}

impl CredentialStore for MemoryStore {
    fn backend(&self) -> &'static str {
        "memory-test"
    }

    fn get(&self, provider: &str, origin: &str) -> ase_core::Result<Option<ApiKey>> {
        match self.entries.borrow().get(&format!("{provider}|{origin}")) {
            Some(value) => Ok(Some(ApiKey::parse(value)?)),
            None => Ok(None),
        }
    }

    fn set(&self, provider: &str, origin: &str, key: &ApiKey) -> ase_core::Result<()> {
        self.entries.borrow_mut().insert(
            format!("{provider}|{origin}"),
            key.expose().to_string(),
        );
        Ok(())
    }

    fn delete(&self, provider: &str, origin: &str) -> ase_core::Result<bool> {
        Ok(self
            .entries
            .borrow_mut()
            .remove(&format!("{provider}|{origin}"))
            .is_some())
    }
}

#[test]
fn same_provider_and_origin_share_one_key_across_profiles() {
    let dir = scratch("shared-provider-origin");
    let env = config_env(&dir);
    let mut config = ConfigFile::empty();
    seed_config(&mut config, "chat", "openai", ApiKind::ChatCompletions);
    apply_edit(
        &mut config,
        &profile(
            "responses",
            "openai",
            "https://api.openai.com:443/v1/responses",
            ApiKind::Responses,
        ),
    )
    .unwrap();
    save_config(&env, &config).unwrap();

    let store = MemoryStore::default();
    store.seed("openai", "https://api.openai.com:443", SYNTHETIC);
    let first = resolve_with_store(&env, Some("chat"), &store).unwrap();
    assert_eq!(first.profile.provider, "openai");
    assert_eq!(first.key.expose(), SYNTHETIC);
    assert_eq!(first.key_source, KeySource::Store("memory-test"));
    assert_eq!(first.profile.effort.as_deref(), Some("medium"));

    let switched = resolve_with_store(&env, Some("responses"), &store).unwrap();
    assert_eq!(switched.key.expose(), SYNTHETIC);
    assert_eq!(switched.profile.provider, "openai");
    assert_eq!(switched.origin(), "https://api.openai.com:443");
    assert_eq!(switched.profile.api, ApiKind::Responses);
}

#[test]
fn different_providers_on_the_same_origin_are_isolated() {
    let dir = scratch("provider-isolation");
    let env = config_env(&dir);
    let mut config = ConfigFile::empty();
    seed_config(&mut config, "chat", "openai", ApiKind::ChatCompletions);
    apply_edit(
        &mut config,
        &profile(
            "alt",
            "opencode-go",
            "https://api.openai.com/v1/chat/completions",
            ApiKind::ChatCompletions,
        ),
    )
    .unwrap();
    save_config(&env, &config).unwrap();

    let store = MemoryStore::default();
    store.seed("openai", "https://api.openai.com:443", SYNTHETIC);

    let missing = resolve_with_store(&env, Some("alt"), &store)
        .err()
        .expect("a different provider must not reuse the openai key");
    assert!(missing.to_string().contains("api_key_missing"));

    let mut with_key = env.clone();
    with_key.insert("OPENAI_API_KEY".to_string(), OTHER.to_string());
    let resolved = resolve_with_store(&with_key, Some("alt"), &store).unwrap();
    assert_eq!(resolved.profile.provider, "opencode-go");
    assert_eq!(resolved.key_source, KeySource::Environment("OPENAI_API_KEY"));
    assert_eq!(store.len(), 1, "status/resolve must never persist an env key");
}

#[test]
fn provider_switch_does_not_reuse_the_saved_key() {
    let dir = scratch("provider-switch");
    let env = config_env(&dir);
    let mut config = ConfigFile::empty();
    seed_config(&mut config, "chat", "openai", ApiKind::ChatCompletions);
    save_config(&env, &config).unwrap();

    let store = MemoryStore::default();
    store.seed("openai", "https://api.openai.com:443", SYNTHETIC);

    apply_edit(
        &mut config,
        &ProfileEdit {
            profile: "chat",
            provider: Some("gemini"),
            api: None,
            endpoint: None,
            model: None,
            effort: EffortEdit::Keep,
            make_active: false,
        },
    )
    .unwrap();
    save_config(&env, &config).unwrap();
    let missing = resolve_with_store(&env, None, &store)
        .err()
        .expect("provider switch must not reuse the old provider key");
    assert!(missing.to_string().contains("api_key_missing"));
}

#[test]
fn changed_origin_never_reuses_a_key_for_the_same_provider() {
    let dir = scratch("provider-origin-change");
    let env = config_env(&dir);
    let mut config = ConfigFile::empty();
    seed_config(&mut config, "chat", "openai", ApiKind::ChatCompletions);
    save_config(&env, &config).unwrap();

    let store = MemoryStore::default();
    store.seed("openai", "https://api.openai.com:443", SYNTHETIC);
    let mut overridden = env.clone();
    overridden.insert(
        "OPENAI_API_ENDPOINT".to_string(),
        "https://api.example.com/v1/chat/completions".to_string(),
    );
    let missing = resolve_with_store(&overridden, None, &store)
        .err()
        .expect("changed origin must not resolve");
    assert!(missing.to_string().contains("api_key_missing"));

    overridden.insert(
        "OPENAI_API_KEY".to_string(),
        "sk-env-override-only".to_string(),
    );
    let resolved = resolve_with_store(&overridden, None, &store).unwrap();
    assert_eq!(resolved.origin(), "https://api.example.com:443");
    assert_eq!(resolved.key_source, KeySource::Environment("OPENAI_API_KEY"));
}

#[test]
fn environment_key_is_ephemeral_and_overrides_the_store() {
    let dir = scratch("env-ephemeral");
    let env = config_env(&dir);
    let mut config = ConfigFile::empty();
    seed_config(&mut config, "chat", "openai", ApiKind::ChatCompletions);
    save_config(&env, &config).unwrap();

    let store = MemoryStore::default();
    store.seed("openai", "https://api.openai.com:443", SYNTHETIC);

    let mut with_env = env.clone();
    with_env.insert("OPENAI_API_KEY".to_string(), "sk-env-wins".to_string());
    let resolved = resolve_with_store(&with_env, None, &store).unwrap();
    assert_eq!(resolved.key_source, KeySource::Environment("OPENAI_API_KEY"));
    assert_eq!(resolved.key.expose(), "sk-env-wins");
    assert_eq!(store.len(), 1, "env override must not be written to the store");

    let mut empty_env = env.clone();
    empty_env.insert("OPENAI_API_KEY".to_string(), "   ".to_string());
    let resolved = resolve_with_store(&empty_env, None, &store).unwrap();
    assert_eq!(resolved.key_source, KeySource::Store("memory-test"));
}

#[test]
fn environment_provider_override_is_explicit_and_never_reuses_another_key() {
    let dir = scratch("env-provider");
    let env = config_env(&dir);
    let mut config = ConfigFile::empty();
    seed_config(&mut config, "chat", "openai", ApiKind::ChatCompletions);
    save_config(&env, &config).unwrap();

    let store = MemoryStore::default();
    store.seed("openai", "https://api.openai.com:443", SYNTHETIC);
    let mut with_provider = env.clone();
    with_provider.insert("ASE_LLM_PROVIDER".to_string(), "gemini".to_string());
    let missing = resolve_with_store(&with_provider, None, &store)
        .err()
        .expect("provider override must not reuse the openai key");
    assert!(missing.to_string().contains("api_key_missing"));

    let mut with_key = with_provider.clone();
    with_key.insert("OPENAI_API_KEY".to_string(), OTHER.to_string());
    let resolved = resolve_with_store(&with_key, None, &store).unwrap();
    assert_eq!(resolved.profile.provider, "gemini");
    assert!(resolved
        .profile
        .env_overrides
        .contains(&"ASE_LLM_PROVIDER"));
    assert_eq!(store.len(), 1);
}

#[test]
fn native_gemini_kinds_use_only_the_gemini_key_family() {
    let dir = scratch("gemini-key-family");
    let env = config_env(&dir);
    let mut config = ConfigFile::empty();
    apply_edit(
        &mut config,
        &profile(
            "gemini",
            "gemini-native",
            "https://gemini.example.com/v1/generate",
            ApiKind::Gemini,
        ),
    )
    .unwrap();
    apply_edit(
        &mut config,
        &profile(
            "gemini-decision",
            "gemini-native",
            "https://gemini.example.com/v1/decide",
            ApiKind::GeminiDecision,
        ),
    )
    .unwrap();
    save_config(&env, &config).unwrap();

    let store = MemoryStore::default();
    store.seed("gemini-native", "https://gemini.example.com:443", SYNTHETIC);

    // GEMINI_API_KEY is the only env override for native kinds.
    let mut gemini_env = env.clone();
    gemini_env.insert("GEMINI_API_KEY".to_string(), OTHER.to_string());
    let resolved = resolve_with_store(&gemini_env, Some("gemini"), &store).unwrap();
    assert_eq!(resolved.key_source, KeySource::Environment("GEMINI_API_KEY"));
    assert_eq!(resolved.key.expose(), OTHER);
    assert_eq!(resolved.profile.effort, None);
    let decision = resolve_with_store(&gemini_env, Some("gemini-decision"), &store).unwrap();
    assert_eq!(
        decision.key_source,
        KeySource::Environment("GEMINI_API_KEY")
    );
    assert_eq!(decision.profile.effort, None);

    // OPENAI_API_KEY must never fall back into a native kind. A provider with
    // no stored key isolates the env-family rule from the stored-key path.
    apply_edit(
        &mut config,
        &profile(
            "gemini-nostore",
            "gemini-unstored",
            "https://gemini.example.com/v1/generate",
            ApiKind::Gemini,
        ),
    )
    .unwrap();
    save_config(&env, &config).unwrap();
    let mut openai_env = env.clone();
    openai_env.insert("OPENAI_API_KEY".to_string(), "sk-openai-only".to_string());
    let missing = resolve_with_store(&openai_env, Some("gemini-nostore"), &store)
        .err()
        .expect("openai env key must not feed a native kind");
    assert!(missing.to_string().contains("api_key_missing"));
    assert!(missing.to_string().contains("GEMINI_API_KEY"));
}

#[test]
fn openai_kinds_never_borrow_the_gemini_key_family() {
    let dir = scratch("openai-key-family");
    let env = config_env(&dir);
    let mut config = ConfigFile::empty();
    seed_config(&mut config, "chat", "openai", ApiKind::ChatCompletions);
    save_config(&env, &config).unwrap();

    let store = MemoryStore::default();
    store.seed("openai", "https://api.openai.com:443", SYNTHETIC);
    let mut gemini_env = env.clone();
    gemini_env.insert("GEMINI_API_KEY".to_string(), OTHER.to_string());
    let resolved = resolve_with_store(&gemini_env, None, &store).unwrap();
    assert_eq!(resolved.key_source, KeySource::Store("memory-test"));
    assert_eq!(resolved.key.expose(), SYNTHETIC);

    let mut both = gemini_env.clone();
    both.insert("OPENAI_API_KEY".to_string(), "sk-openai-wins".to_string());
    let resolved = resolve_with_store(&both, None, &store).unwrap();
    assert_eq!(resolved.key_source, KeySource::Environment("OPENAI_API_KEY"));
    assert_eq!(resolved.key.expose(), "sk-openai-wins");
}

#[test]
fn no_effort_env_is_applied_to_native_gemini_or_either_decision_kind() {
    let dir = scratch("no-effort-kinds");
    let env = config_env(&dir);
    let mut config = ConfigFile::empty();
    seed_config(&mut config, "chat", "openai", ApiKind::ChatCompletions);
    seed_config(&mut config, "decision", "openai", ApiKind::Decision);
    apply_edit(
        &mut config,
        &profile(
            "gemini",
            "gemini-native",
            "https://gemini.example.com/v1/generate",
            ApiKind::Gemini,
        ),
    )
    .unwrap();
    apply_edit(
        &mut config,
        &profile(
            "gemini-decision",
            "gemini-native",
            "https://gemini.example.com/v1/decide",
            ApiKind::GeminiDecision,
        ),
    )
    .unwrap();
    save_config(&env, &config).unwrap();

    let store = MemoryStore::default();
    store.seed("openai", "https://api.openai.com:443", SYNTHETIC);
    store.seed("gemini-native", "https://gemini.example.com:443", OTHER);
    let mut with_effort = env.clone();
    with_effort.insert("OPENAI_API_EFFORT".to_string(), "high".to_string());

    let chat = resolve_with_store(&with_effort, Some("chat"), &store).unwrap();
    assert_eq!(chat.profile.effort.as_deref(), Some("high"));
    for name in ["decision", "gemini", "gemini-decision"] {
        let resolved = resolve_with_store(&with_effort, Some(name), &store).unwrap();
        assert_eq!(resolved.profile.effort, None, "{name} must not send effort");
        assert!(
            resolved
                .profile
                .notes
                .iter()
                .any(|note| note.contains("OPENAI_API_EFFORT is ignored")),
            "{name} must explain the ignored effort override"
        );
    }

    let mut config = config.clone();
    assert!(apply_edit(
        &mut config,
        &ProfileEdit {
            profile: "gemini",
            provider: None,
            api: Some(ApiKind::Gemini),
            endpoint: None,
            model: None,
            effort: EffortEdit::Set("high".into()),
            make_active: false,
        },
    )
    .is_err());
}

#[test]
fn gemini_status_reports_its_own_key_source_and_no_effort() {
    let dir = scratch("gemini-status");
    let env = config_env(&dir);
    let mut config = ConfigFile::empty();
    apply_edit(
        &mut config,
        &profile(
            "gemini",
            "gemini-native",
            "https://gemini.example.com/v1/generate",
            ApiKind::Gemini,
        ),
    )
    .unwrap();
    save_config(&env, &config).unwrap();
    let mut with_key = env.clone();
    with_key.insert("GEMINI_API_KEY".to_string(), OTHER.to_string());
    let report = status(&with_key, None).unwrap();
    assert!(report.ready);
    assert_eq!(report.provider.as_deref(), Some("gemini-native"));
    assert_eq!(report.api.as_deref(), Some("gemini"));
    assert_eq!(report.effort, None);
    assert_eq!(report.credential_source, "env:GEMINI_API_KEY");
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains(OTHER));
}

#[test]
fn decision_api_never_resolves_or_sends_an_effort() {
    let dir = scratch("decision-effort");
    let env = config_env(&dir);
    let mut config = ConfigFile::empty();
    seed_config(&mut config, "decision", "openai", ApiKind::Decision);
    save_config(&env, &config).unwrap();

    let store = MemoryStore::default();
    store.seed("openai", "https://api.openai.com:443", SYNTHETIC);
    let base = resolve_with_store(&env, None, &store).unwrap();
    assert_eq!(base.profile.effort, None);
    assert_eq!(base.profile.api, ApiKind::Decision);

    let mut with_effort = env.clone();
    with_effort.insert("OPENAI_API_EFFORT".to_string(), "high".to_string());
    let resolved = resolve_with_store(&with_effort, None, &store).unwrap();
    assert_eq!(resolved.profile.effort, None);
    assert!(resolved
        .profile
        .notes
        .iter()
        .any(|note| note.contains("OPENAI_API_EFFORT is ignored")));
}

#[test]
fn status_is_redacted_ready_and_reports_the_provider() {
    let dir = scratch("status-redaction");
    let env = config_env(&dir);
    let mut config = ConfigFile::empty();
    seed_config(&mut config, "chat", "openai", ApiKind::ChatCompletions);
    save_config(&env, &config).unwrap();

    let mut with_key = env.clone();
    with_key.insert("OPENAI_API_KEY".to_string(), SYNTHETIC.to_string());
    let report = status(&with_key, None).unwrap();
    assert!(report.ready);
    assert_eq!(report.provider.as_deref(), Some("openai"));
    assert_eq!(report.credential_source, "env:OPENAI_API_KEY");
    assert_eq!(report.effort.as_deref(), Some("medium"));
    assert_eq!(report.network, "not_used");
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains(SYNTHETIC));
    assert!(!json.contains("synthetic-t4"));
    assert!(json.contains("\"provider\":\"openai\""));

    let missing = status(&env, None).unwrap();
    assert!(!missing.ready);
    assert!(missing.provider.as_deref() == Some("openai"));
}

#[test]
fn status_reports_absent_config_and_provider_origin_binding() {
    let dir = scratch("status-unconfigured");
    let env = config_env(&dir);
    let report = status(&env, None).unwrap();
    assert!(!report.configured);
    assert!(!report.ready);
    assert_eq!(report.config_source, "none");

    let mut config = ConfigFile::empty();
    seed_config(&mut config, "chat", "openai", ApiKind::ChatCompletions);
    save_config(&env, &config).unwrap();
    let mut with_key = env.clone();
    with_key.insert("OPENAI_API_KEY".to_string(), SYNTHETIC.to_string());
    let report = status(&with_key, None).unwrap();
    assert_eq!(report.origin.as_deref(), Some("https://api.openai.com:443"));
    assert!(report.checks.iter().any(|check| check.name == "origin_binding"
        && check.ok
        && check.detail.contains("provider openai")));
}

#[test]
fn environment_only_configuration_requires_an_explicit_provider() {
    let dir = scratch("env-only");
    let env = config_env(&dir);
    let store = MemoryStore::default();
    store.seed("openai", "https://api.openai.com:443", SYNTHETIC);
    let mut partial = env.clone();
    partial.insert(
        "OPENAI_API_ENDPOINT".to_string(),
        "https://api.openai.com/v1/chat/completions".to_string(),
    );
    let missing = resolve_with_store(&partial, None, &store)
        .err()
        .expect("provider is required");
    assert!(missing
        .to_string()
        .contains("openai_api_provider_missing"));

    partial.insert("ASE_LLM_PROVIDER".to_string(), "openai".to_string());
    let missing = resolve_with_store(&partial, None, &store)
        .err()
        .expect("type is required");
    assert!(missing.to_string().contains("openai_api_type_missing"));

    partial.insert(
        "OPENAI_API_TYPE".to_string(),
        "chat-completions".to_string(),
    );
    partial.insert("OPENAI_API_MODEL".to_string(), "gpt-synthetic".to_string());
    let resolved = resolve_with_store(&partial, None, &store).unwrap();
    assert_eq!(resolved.profile.profile, "env");
    assert_eq!(resolved.profile.provider, "openai");
    assert_eq!(resolved.profile.effort.as_deref(), Some("medium"));
}

#[test]
fn store_selection_errors_are_explicit_and_never_plaintext_fallbacks() {
    let dir = scratch("store-selection");
    let mut env = config_env(&dir);
    env.insert(
        crate::credentials::STORE_ENV.to_string(),
        "unknown-backend".to_string(),
    );
    let error = open_store(&env).err().expect("bogus store must fail");
    assert!(error
        .to_string()
        .contains("credential_store_setting_unrecognized"));
    assert!(!format!("{error:?}").contains(SYNTHETIC));
}

#[test]
fn config_version_2_requires_an_explicit_provider_and_rejects_unknown_fields() {
    let mut v1 = ConfigFile::empty();
    v1.version = 1;
    assert!(save_config(
        &env(&[(
            crate::config::CONFIG_ENV,
            scratch("v1-rejected").join("llm.json").to_str().unwrap(),
        )]),
        &v1,
    )
    .is_err());

    let raw_missing_provider = r#"{"version":2,"active":"a","profiles":{"a":{"api":"responses","endpoint":"https://api.openai.com/v1/responses","model":"m"}}}"#;
    assert!(serde_json::from_str::<ConfigFile>(raw_missing_provider).is_err());

    let raw_secret = r#"{"version":2,"active":"a","profiles":{"a":{"provider":"p","api":"responses","endpoint":"https://api.openai.com/v1/responses","model":"m","key":"secret"}}}"#;
    assert!(serde_json::from_str::<ConfigFile>(raw_secret).is_err());

    let mut config = ConfigFile::empty();
    assert!(apply_edit(
        &mut config,
        &ProfileEdit {
            profile: "a",
            provider: None,
            api: Some(ApiKind::Responses),
            endpoint: Some("https://api.openai.com/v1/responses"),
            model: Some("m"),
            effort: EffortEdit::Keep,
            make_active: false,
        },
    )
    .is_err());
    assert!(apply_edit(
        &mut config,
        &ProfileEdit {
            profile: "a",
            provider: Some("bad|name"),
            api: Some(ApiKind::Responses),
            endpoint: Some("https://api.openai.com/v1/responses"),
            model: Some("m"),
            effort: EffortEdit::Keep,
            make_active: false,
        },
    )
    .is_err());
    assert_eq!(config.version, CONFIG_VERSION);
}

#[cfg(not(windows))]
#[test]
fn unsupported_platform_fails_explicitly_and_before_any_prompt() {
    let dir = scratch("unsupported-platform");
    let env = config_env(&dir);
    let store = open_store(&env).unwrap();
    assert_eq!(store.backend(), crate::credentials::UNSUPPORTED_BACKEND);
    assert!(store
        .get("openai", "https://api.openai.com:443")
        .unwrap_err()
        .to_string()
        .contains("credential_platform_unsupported"));
    assert!(store
        .ensure_writable()
        .unwrap_err()
        .to_string()
        .contains("credential_platform_unsupported"));
}

#[cfg(windows)]
#[test]
fn dpapi_file_store_is_ciphertext_provider_and_origin_bound() {
    let dir = scratch("dpapi-store");
    let store_dir = dir.join("store");
    let mut env = config_env(&dir);
    env.insert(
        crate::credentials::STORE_ENV.to_string(),
        format!("dpapi-file:{}", store_dir.display()),
    );
    let store = open_store(&env).unwrap();
    assert_eq!(store.backend(), crate::credentials::DPAPI_BACKEND);

    let provider = "openai";
    let origin = "https://api.openai.com:443";
    let key = ApiKey::parse(SYNTHETIC).unwrap();
    store.set(provider, origin, &key).unwrap();
    let path = store_dir.join(format!(
        "{}.cred",
        crate::credentials::sha256_hex(format!("{provider}|{origin}").as_bytes())
    ));
    let cipher = std::fs::read(&path).unwrap();
    assert!(!cipher.is_empty());
    assert!(!cipher
        .windows(SYNTHETIC.len())
        .any(|window| window == SYNTHETIC.as_bytes()));
    assert!(!String::from_utf8_lossy(&cipher).contains("synthetic"));

    let read = store.get(provider, origin).unwrap().expect("stored key");
    assert_eq!(read.expose(), SYNTHETIC);
    assert!(store
        .get("opencode-go", origin)
        .unwrap()
        .is_none());

    let other_path = store_dir.join(format!(
        "{}.cred",
        crate::credentials::sha256_hex(format!("gemini|{origin}").as_bytes())
    ));
    std::fs::copy(&path, &other_path).unwrap();
    let mismatch = store.get("gemini", origin).unwrap_err();
    assert!(mismatch
        .to_string()
        .contains("credential_provider_mismatch"));

    assert!(store.delete(provider, origin).unwrap());
    assert!(!store.delete(provider, origin).unwrap());
    assert!(store.get(provider, origin).unwrap().is_none());
}

#[cfg(windows)]
#[test]
fn native_credential_store_reports_a_missing_entry_without_writing() {
    let dir = scratch("native-probe");
    let env = config_env(&dir);
    let store = open_store(&env).unwrap();
    assert_eq!(store.backend(), crate::credentials::NATIVE_BACKEND);
    let probe = store
        .get("ase-probe", "https://ase-llm-probe.invalid:443")
        .expect("read-only probe");
    assert!(probe.is_none());
}

#[test]
fn config_file_roundtrip_contains_provider_and_no_secret_fields() {
    let dir = scratch("config-roundtrip");
    let env = config_env(&dir);
    let mut config = ConfigFile::empty();
    apply_edit(
        &mut config,
        &ProfileEdit {
            profile: "chat",
            provider: Some("opencode-go"),
            api: Some(ApiKind::ChatCompletions),
            endpoint: Some("https://api.openai.com/v1/chat/completions"),
            model: Some("gpt-synthetic"),
            effort: EffortEdit::Set("high".into()),
            make_active: false,
        },
    )
    .unwrap();
    use_profile(&mut config, "chat").unwrap();
    save_config(&env, &config).unwrap();
    let loaded = load_config(&env).unwrap();
    let profile = &loaded.config.unwrap().profiles["chat"];
    assert_eq!(profile.provider, "opencode-go");
    assert_eq!(profile.effort.as_deref(), Some("high"));
    let raw = std::fs::read_to_string(dir.join("llm-config.json")).unwrap();
    assert!(!raw.contains("api_key"));
    assert!(!raw.contains("OPENAI_API_KEY"));
    assert_eq!(
        validate_endpoint(&profile.endpoint).unwrap().origin,
        "https://api.openai.com:443"
    );
}

fn endpoint(url: &str) -> crate::url::Endpoint {
    validate_endpoint(url).expect("valid endpoint")
}

fn spec<'a>(
    api: ApiKind,
    effort: Option<&'a str>,
    choices: &'a [(&'a str, &'a str)],
) -> crate::wire::CallSpec<'a> {
    crate::wire::CallSpec {
        api,
        model: "synthetic-model",
        effort,
        instruction: "Choose one synthetic option.",
        input: "synthetic earlier pair",
        question_id: "task_boundary",
        choices,
    }
}

#[test]
fn wire_bodies_and_auth_locations_are_typed() {
    let choices = [
        ("split", "new task"),
        ("continue", "same task"),
        ("unknown", "undecidable"),
    ];
    let chat = crate::wire::build_request(
        &spec(ApiKind::ChatCompletions, Some("none"), &choices),
        &endpoint("https://api.openai.com/v1/chat/completions"),
        None,
    )
    .expect("chat request");
    let body: serde_json::Value = serde_json::from_slice(&chat.body).unwrap();
    assert_eq!(body["reasoning_effort"], "none");
    assert_eq!(body["messages"][1]["content"], "synthetic earlier pair");
    assert_eq!(chat.auth.header_name(), "authorization");
    assert!(chat.effort_sent);

    let responses = crate::wire::build_request(
        &spec(ApiKind::Responses, Some("medium"), &choices),
        &endpoint("https://api.openai.com/v1/responses"),
        None,
    )
    .expect("responses request");
    let body: serde_json::Value = serde_json::from_slice(&responses.body).unwrap();
    assert_eq!(body["reasoning"]["effort"], "medium");

    let gemini = crate::wire::build_request(
        &spec(ApiKind::Gemini, None, &choices),
        &endpoint("https://generativelanguage.googleapis.com/v1beta/models/gemini:generateContent"),
        None,
    )
    .expect("gemini request");
    let body: serde_json::Value = serde_json::from_slice(&gemini.body).unwrap();
    assert_eq!(body["contents"][0]["parts"][0]["text"], "synthetic earlier pair");
    assert!(body.get("reasoning_effort").is_none());
    assert!(body.get("reasoning").is_none());
    assert_eq!(gemini.auth.header_name(), "x-goog-api-key");
    assert!(!gemini.effort_sent);
    assert!(crate::wire::build_request(
        &spec(ApiKind::Gemini, Some("high"), &choices),
        &endpoint("https://generativelanguage.googleapis.com/v1beta/models/gemini:generateContent"),
        None,
    )
    .is_err());

    let decision = crate::wire::build_request(
        &spec(ApiKind::Decision, None, &choices),
        &endpoint("https://opencode.ai/inference/systemone/v1/systemone"),
        Some("ses_0123456789abcdef01234567"),
    )
    .expect("decision request");
    let body: serde_json::Value = serde_json::from_slice(&decision.body).unwrap();
    assert_eq!(body["state"], "synthetic earlier pair");
    assert!(body.get("input").is_none());
    assert_eq!(body["questions"]["task_boundary"]["type"], "choice");
    assert_eq!(
        body["questions"]["task_boundary"]["criteria"]["split"],
        "new task"
    );
    assert!(decision.session_sent);
    assert!(decision
        .headers
        .iter()
        .any(|(name, _)| *name == crate::wire::SESSION_HEADER));

    let gemini_decision = crate::wire::build_request(
        &spec(ApiKind::GeminiDecision, None, &choices),
        &endpoint("https://generativelanguage.googleapis.com/v1beta/decisions"),
        Some("ses_ignored_for_native"),
    )
    .expect("gemini decision request");
    let body: serde_json::Value = serde_json::from_slice(&gemini_decision.body).unwrap();
    assert_eq!(body["input"], "synthetic earlier pair");
    assert!(body.get("state").is_none());
    assert!(!gemini_decision.session_sent);
    assert_eq!(gemini_decision.auth.header_name(), "x-goog-api-key");
}

#[test]
fn wire_parses_each_typed_answer_without_echoing_the_body() {
    let chat = br#"{"model":"m","choices":[{"message":{"content":"{\"label\":\"split\"}"}}],"usage":{"prompt_tokens":7,"completion_tokens":3}}"#;
    match crate::wire::parse_response(ApiKind::ChatCompletions, "q", chat).unwrap().answer {
        crate::wire::WireAnswer::Text(text) => assert!(text.contains("split")),
        _ => panic!("chat must be text"),
    }

    let responses = br#"{"output":[{"type":"message","content":[{"type":"output_text","text":"hello"}]}],"usage":{"input_tokens":9,"output_tokens":4}}"#;
    match crate::wire::parse_response(ApiKind::Responses, "q", responses).unwrap().answer {
        crate::wire::WireAnswer::Text(text) => assert_eq!(text, "hello"),
        _ => panic!("responses must be text"),
    }

    let gemini = br#"{"candidates":[{"content":{"parts":[{"text":"a"},{"text":"b"}]}}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":2}}"#;
    let parsed = crate::wire::parse_response(ApiKind::Gemini, "q", gemini).unwrap();
    match parsed.answer {
        crate::wire::WireAnswer::Text(text) => assert_eq!(text, "ab"),
        _ => panic!("gemini must be text"),
    }
    assert_eq!(parsed.usage.input_tokens, Some(5));

    let decision = br#"{"answers":{"task_boundary":{"type":"choice","choice":"continue","confidence":0.93,"probabilities":{"continue":0.93,"split":0.07}}},"usage":{"input_tokens":12,"output_tokens":1}}"#;
    let parsed = crate::wire::parse_response(ApiKind::Decision, "task_boundary", decision).unwrap();
    match parsed.answer {
        crate::wire::WireAnswer::Choice {
            choice, confidence, ..
        } => {
            assert_eq!(choice, "continue");
            assert!((confidence - 0.93).abs() < 1e-9);
        }
        _ => panic!("decision must be a typed choice"),
    }

    let native = br#"{"answers":{"task_boundary":{"type":"choice","choice":"unknown","confidence":0.6}},"usage":{"total_input_tokens":21,"total_output_tokens":0}}"#;
    let parsed =
        crate::wire::parse_response(ApiKind::GeminiDecision, "task_boundary", native).unwrap();
    match parsed.answer {
        crate::wire::WireAnswer::Choice { choice, .. } => assert_eq!(choice, "unknown"),
        _ => panic!("native decision must be a typed choice"),
    }
    assert_eq!(parsed.usage.input_tokens, Some(21));

    let marker = "MARKER-RAW-BODY-SHOULD-NOT-BE-ECHOED";
    let malformed = format!(r#"{{"error":"{marker}"}}"#);
    let error = crate::wire::parse_response(ApiKind::Gemini, "q", malformed.as_bytes())
        .err()
        .expect("missing candidates must be rejected");
    assert!(!error.detail().contains(marker));
    assert!(!format!("{error}").contains(marker));
}

#[test]
fn api_identity_and_session_are_versioned_and_credential_free() {
    let profile = crate::resolve::ResolvedProfile {
        profile: "chat".to_string(),
        provider: "provider".to_string(),
        api: ApiKind::ChatCompletions,
        endpoint: endpoint("https://api.openai.com/v1/chat/completions"),
        model: "synthetic-model".to_string(),
        effort: Some("medium".to_string()),
        env_overrides: Vec::new(),
        notes: Vec::new(),
    };
    let first = crate::api::identity(&profile);
    let second = crate::api::identity(&profile);
    assert_eq!(first.digest, second.digest);
    assert_eq!(first.generator_id, second.generator_id);
    assert!(!first.generator_id.contains(SYNTHETIC));

    let mut other = crate::resolve::ResolvedProfile {
        effort: Some("high".to_string()),
        ..profile
    };
    let high = crate::api::identity(&other);
    assert_ne!(first.digest, high.digest, "effort is part of the identity");
    other.effort = None;
    other.api = ApiKind::Responses;
    let responses = crate::api::identity(&other);
    assert_ne!(first.digest, responses.digest, "typed API is part of the identity");
    assert_ne!(first.generator_id, responses.generator_id);

    let session = crate::api::session_id(&other);
    assert!(session.starts_with("ses_"));
    assert_eq!(session.len(), 4 + 24);
    assert!(!session.contains(SYNTHETIC));
}
