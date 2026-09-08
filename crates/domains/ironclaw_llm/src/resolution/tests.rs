//! Tests for provider catalog and environment resolution.
//!
//! Split from `resolution.rs` to keep the production resolver within the
//! architecture file-size budget.

use super::*;

const CHAIN_ENV_VARS: &[&str] = &[
    "LLM_REQUEST_TIMEOUT_SECS",
    "LLM_CHEAP_MODEL",
    "LLM_BACKEND",
    "SMART_ROUTING_CASCADE",
    "CODEX_AUTH_PATH",
    "LLM_USE_CODEX_AUTH",
    "LLM_MAX_RETRIES",
    "NEARAI_MAX_RETRIES",
    "LLM_CIRCUIT_BREAKER_THRESHOLD",
    "CIRCUIT_BREAKER_THRESHOLD",
    "LLM_CIRCUIT_BREAKER_RECOVERY_SECS",
    "CIRCUIT_BREAKER_RECOVERY_SECS",
    "LLM_RESPONSE_CACHE_ENABLED",
    "RESPONSE_CACHE_ENABLED",
    "LLM_RESPONSE_CACHE_TTL_SECS",
    "RESPONSE_CACHE_TTL_SECS",
    "LLM_RESPONSE_CACHE_MAX_ENTRIES",
    "RESPONSE_CACHE_MAX_ENTRIES",
    "NEARAI_API_KEY",
    "NEARAI_BASE_URL",
    "NEARAI_MODEL",
    "NEARAI_CHEAP_MODEL",
    "NEARAI_FALLBACK_MODEL",
];

struct EnvGuard {
    saved: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn clear(names: &[&'static str]) -> Self {
        let saved = names
            .iter()
            .map(|name| (*name, std::env::var(name).ok()))
            .collect();
        for name in names {
            unsafe {
                std::env::remove_var(name);
            }
        }
        Self { saved }
    }

    fn set(&self, name: &str, value: &str) {
        unsafe {
            std::env::set_var(name, value);
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (name, value) in self.saved.drain(..) {
            unsafe {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

fn registry_resolved_provider() -> ResolvedProviderConfig {
    ResolvedProviderConfig::Registry(RegistryProviderConfig::generic(
        ProviderProtocol::OpenAiCompletions,
        "openai",
        None,
        "https://api.openai.com/v1",
        "gpt-test",
    ))
}

#[test]
fn full_config_resolution_uses_legacy_chain_env_fallbacks() {
    let _env_lock = ironclaw_common::env_helpers::lock_env();
    let env = EnvGuard::clear(CHAIN_ENV_VARS);
    env.set("NEARAI_MAX_RETRIES", "7");
    env.set("CIRCUIT_BREAKER_THRESHOLD", "11");
    env.set("CIRCUIT_BREAKER_RECOVERY_SECS", "19");
    env.set("RESPONSE_CACHE_ENABLED", "true");
    env.set("RESPONSE_CACHE_TTL_SECS", "23");
    env.set("RESPONSE_CACHE_MAX_ENTRIES", "29");

    let config = build_llm_config_from_resolved_provider(registry_resolved_provider())
        .expect("legacy chain environment fallbacks should resolve");

    assert_eq!(config.max_retries, 7);
    assert_eq!(config.circuit_breaker_threshold, Some(11));
    assert_eq!(config.circuit_breaker_recovery_secs, 19);
    assert!(config.response_cache_enabled);
    assert_eq!(config.response_cache_ttl_secs, 23);
    assert_eq!(config.response_cache_max_entries, 29);
}

#[test]
fn full_config_resolution_accepts_common_boolean_env_values() {
    let _env_lock = ironclaw_common::env_helpers::lock_env();
    let env = EnvGuard::clear(CHAIN_ENV_VARS);
    env.set("SMART_ROUTING_CASCADE", "off");
    env.set("LLM_RESPONSE_CACHE_ENABLED", "yes");

    let config = build_llm_config_from_resolved_provider(registry_resolved_provider())
        .expect("common boolean environment values should resolve");

    assert!(!config.smart_routing_cascade);
    assert!(config.response_cache_enabled);
}

#[test]
fn openai_codex_backend_with_missing_codex_auth_path_fails_fast() {
    let _env_lock = ironclaw_common::env_helpers::lock_env();
    let env = EnvGuard::clear(CHAIN_ENV_VARS);
    env.set("LLM_BACKEND", "openai_codex");
    env.set("CODEX_AUTH_PATH", "/tmp/ironclaw-missing-codex-auth.json");

    let error = resolve_provider_config_from_env(None)
        .expect_err("missing explicit Codex auth path should fail before provider startup");

    assert!(matches!(
        error,
        LlmError::AuthFailed { provider } if provider == "openai_codex"
    ));
}

#[test]
fn codex_auth_env_route_preserves_catalog_unsupported_params() {
    let _env_lock = ironclaw_common::env_helpers::lock_env();
    let env = EnvGuard::clear(CHAIN_ENV_VARS);
    let directory = tempfile::tempdir().expect("temporary catalog directory");
    let providers_path = directory.path().join("providers.json");
    let auth_path = directory.path().join("auth.json");
    std::fs::write(
        &providers_path,
        r#"[{
                "id": "openai_codex",
                "aliases": ["openai-codex", "codex"],
                "protocol": "openai_codex",
                "api_key_required": false,
                "model_env": "OPENAI_CODEX_MODEL",
                "default_model": "gpt-5.5",
                "description": "OpenAI Codex test override",
                "unsupported_params": ["prompt_cache_key"]
            }]"#,
    )
    .expect("write provider overlay");
    std::fs::write(
            &auth_path,
            r#"{"auth_mode":"chatgpt","tokens":{"id_token":{},"access_token":"test-token","refresh_token":"test-refresh"}}"#,
        )
        .expect("write Codex auth fixture");
    env.set("LLM_BACKEND", "openai_codex");
    env.set(
        "CODEX_AUTH_PATH",
        auth_path.to_str().expect("UTF-8 auth path"),
    );

    let resolved = resolve_provider_config_from_env(Some(&providers_path))
        .expect("Codex auth route should resolve")
        .expect("Codex provider config");
    let ResolvedProviderConfig::Registry(config) = resolved else {
        panic!("Codex auth resolves to a registry-backed Responses provider");
    };

    assert_eq!(
        config.unsupported_params,
        vec!["prompt_cache_key".to_string()],
        "the Codex auth shortcut must retain the selected catalog kill switch",
    );
}

#[test]
fn implicit_codex_auth_env_route_preserves_catalog_unsupported_params() {
    let _env_lock = ironclaw_common::env_helpers::lock_env();
    let env = EnvGuard::clear(CHAIN_ENV_VARS);
    let directory = tempfile::tempdir().expect("temporary catalog directory");
    let providers_path = directory.path().join("providers.json");
    let auth_path = directory.path().join("auth.json");
    std::fs::write(
        &providers_path,
        r#"[{
                "id": "openai_codex",
                "aliases": ["openai-codex", "codex"],
                "protocol": "openai_codex",
                "api_key_required": false,
                "model_env": "OPENAI_CODEX_MODEL",
                "default_model": "gpt-5.5",
                "description": "OpenAI Codex test override",
                "unsupported_params": ["prompt_cache_key"]
            }]"#,
    )
    .expect("write provider overlay");
    std::fs::write(
            &auth_path,
            r#"{"auth_mode":"chatgpt","tokens":{"id_token":{},"access_token":"test-token","refresh_token":"test-refresh"}}"#,
        )
        .expect("write Codex auth fixture");
    env.set("LLM_USE_CODEX_AUTH", "true");
    env.set(
        "CODEX_AUTH_PATH",
        auth_path.to_str().expect("UTF-8 auth path"),
    );

    let resolved = resolve_provider_config_from_env(Some(&providers_path))
        .expect("implicit Codex auth route should resolve")
        .expect("Codex provider config");
    let ResolvedProviderConfig::Registry(config) = resolved else {
        panic!("Codex auth resolves to a registry-backed Responses provider");
    };

    assert_eq!(
        config.unsupported_params,
        vec!["prompt_cache_key".to_string()],
        "the implicit Codex auth shortcut must retain the catalog kill switch",
    );
}

/// Regression for the Reborn onboarding bug (#4079 introduced the
/// precedence, #4481's WebUI onboarding made it user-visible): an explicit
/// model/base_url the operator picked in the UI must win over the ambient
/// startup env vars (`NEARAI_MODEL` / `NEARAI_BASE_URL`), which a user
/// inherits verbatim from `.env.example`. Before the fix, a user who
/// selected DeepSeek + the cloud endpoint still got Qwen on the
/// session-token endpoint.
#[test]
fn explicit_selection_overrides_env_for_model_and_base_url() {
    let _env_lock = ironclaw_common::env_helpers::lock_env();
    let env = EnvGuard::clear(CHAIN_ENV_VARS);
    env.set("NEARAI_MODEL", "Qwen/Qwen3.5-122B-A10B");
    env.set("NEARAI_BASE_URL", "https://private.near.ai");

    let registry =
        ProviderRegistry::try_load_from_path(None).expect("builtin registry should load");

    let resolved = resolve_provider_config_from_selection(
        ProviderSelection {
            provider_id: "nearai".to_string(),
            api_key_env: None,
            base_url: Some("https://cloud-api.near.ai".to_string()),
            model: Some("deepseek-ai/DeepSeek-V4-Flash".to_string()),
        },
        &registry,
    )
    .expect("nearai selection should resolve");

    let ResolvedProviderConfig::Dedicated(dedicated) = resolved else {
        panic!("nearai must resolve as a dedicated provider config");
    };
    assert_eq!(dedicated.model, "deepseek-ai/DeepSeek-V4-Flash");
    assert_eq!(dedicated.base_url, "https://cloud-api.near.ai");
}

/// A NEAR AI catalog override may need to disable an OpenAI-compatible
/// request extension that its deployment rejects. The resolved dedicated
/// config must preserve that operator control all the way into the live
/// `NearAiConfig`; dropping it makes the advertised kill switch inert.
#[test]
fn nearai_selection_preserves_catalog_unsupported_params() {
    let _env_lock = ironclaw_common::env_helpers::lock_env();
    let _env = EnvGuard::clear(CHAIN_ENV_VARS);

    let builtins =
        ProviderRegistry::try_load_from_path(None).expect("builtin registry should load");
    let mut nearai = builtins.find("nearai").expect("nearai definition").clone();
    nearai.unsupported_params = vec!["prompt_cache_key".to_string()];
    let registry = ProviderRegistry::new(vec![nearai]);

    let config = resolve_llm_config_from_selection(
        ProviderSelection {
            provider_id: "nearai".to_string(),
            api_key_env: None,
            base_url: None,
            model: None,
        },
        &registry,
    )
    .expect("nearai selection should resolve");

    assert_eq!(
        config.nearai.unsupported_params,
        vec!["prompt_cache_key".to_string()],
        "the dedicated NEAR AI path must retain the catalog kill switch",
    );
}

#[test]
fn openai_codex_resolution_preserves_catalog_unsupported_params() {
    let _env_lock = ironclaw_common::env_helpers::lock_env();
    let _env = EnvGuard::clear(CHAIN_ENV_VARS);
    let config = build_llm_config_from_resolved_provider(ResolvedProviderConfig::Dedicated(
        ResolvedDedicatedProviderConfig {
            protocol: ProviderProtocol::OpenAiCodex,
            provider_id: "openai_codex".to_string(),
            api_key: None,
            base_url: "https://chatgpt.com/backend-api/codex".to_string(),
            model: "gpt-5.3-codex".to_string(),
            unsupported_params: vec!["prompt_cache_key".to_string()],
        },
    ))
    .expect("OpenAI Codex config should resolve");

    assert_eq!(
        config
            .openai_codex
            .expect("dedicated OpenAI Codex config")
            .unsupported_params,
        vec!["prompt_cache_key".to_string()],
        "the dedicated Responses path must retain the catalog kill switch",
    );
}

/// The pure-env path (no explicit selection override) must keep its
/// env-first behavior so hosted/headless deployments that configure
/// everything through env vars are unaffected by the precedence fix.
#[test]
fn env_still_wins_when_no_explicit_selection_override() {
    let _env_lock = ironclaw_common::env_helpers::lock_env();
    let env = EnvGuard::clear(CHAIN_ENV_VARS);
    env.set("LLM_BACKEND", "nearai");
    env.set("NEARAI_MODEL", "Qwen/Qwen3.5-122B-A10B");
    env.set("NEARAI_BASE_URL", "https://private.near.ai");

    let resolved = resolve_provider_config_from_env(None)
        .expect("env resolution should succeed")
        .expect("nearai backend should resolve from env");

    let ResolvedProviderConfig::Dedicated(dedicated) = resolved else {
        panic!("nearai must resolve as a dedicated provider config");
    };
    assert_eq!(dedicated.model, "Qwen/Qwen3.5-122B-A10B");
    assert_eq!(dedicated.base_url, "https://private.near.ai");
}
