//! Full LLM config resolution for composition roots.
//!
//! This is the shared path for callers that select providers from
//! `providers.json` but still want the normal `LlmConfig`/provider-chain
//! behavior, including dedicated providers such as NEAR AI, OpenAI Codex,
//! Gemini OAuth, and Bedrock.

use std::path::{Path, PathBuf};

use secrecy::SecretString;

use crate::auth::{self, CredentialSource};
use crate::config::{
    BedrockConfig, CacheRetention, GeminiOauthConfig, LlmConfig, NearAiConfig, OAUTH_PLACEHOLDER,
    OpenAiCodexConfig, RegistryProviderConfig,
};
use crate::error::{LlmConfigError, LlmError};
use crate::registry::{ProviderDefinition, ProviderProtocol, ProviderRegistry};
use crate::session::SessionConfig;

/// Already-resolved provider input from a catalog selection.
#[derive(Debug, Clone)]
pub struct ResolvedProviderConfig {
    pub protocol: ProviderProtocol,
    pub provider_id: String,
    pub api_key: Option<SecretString>,
    pub base_url: String,
    pub model: String,
    pub extra_headers: Vec<(String, String)>,
    pub oauth_token: Option<SecretString>,
    pub is_codex_chatgpt: bool,
    pub refresh_token: Option<SecretString>,
    pub auth_path: Option<PathBuf>,
    pub cache_retention: Option<CacheRetention>,
    pub unsupported_params: Vec<String>,
}

/// Provider selection overrides supplied by a composition root.
#[derive(Debug, Clone)]
pub struct ProviderSelection {
    pub provider_id: String,
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
}

/// Resolve a full [`LlmConfig`] from generic LLM environment variables.
pub fn resolve_llm_config_from_env(
    user_providers_path: Option<&Path>,
) -> Result<Option<LlmConfig>, LlmError> {
    resolve_provider_config_from_env(user_providers_path)?
        .map(build_llm_config_from_resolved_provider)
        .transpose()
}

/// Resolve a provider selection from generic LLM environment variables.
pub fn resolve_provider_config_from_env(
    user_providers_path: Option<&Path>,
) -> Result<Option<ResolvedProviderConfig>, LlmError> {
    if let Some(backend) = nonempty_env("LLM_BACKEND") {
        let registry = try_load_provider_registry(user_providers_path)?;
        let provider = registry
            .find(&backend)
            .ok_or_else(|| LlmError::AuthFailed {
                provider: backend.clone(),
            })?;
        if codex_auth_enabled_from_env() && provider.protocol == ProviderProtocol::OpenAiCodex {
            return resolve_codex_cli_auth_provider().map(Some);
        }
        return resolve_provider_definition_from_env(provider).map(Some);
    }

    if codex_auth_enabled_from_env() {
        return resolve_codex_cli_auth_provider().map(Some);
    }

    let registry = ProviderRegistry::load_from_path(user_providers_path);
    let Some(provider) = registry
        .all()
        .iter()
        .find(|provider| provider_env_present(provider))
    else {
        return Ok(None);
    };
    resolve_provider_definition_from_env(provider).map(Some)
}

/// Resolve a catalog selection against an already-loaded provider registry.
pub fn resolve_provider_config_from_selection(
    selection: ProviderSelection,
    registry: &ProviderRegistry,
) -> Result<ResolvedProviderConfig, LlmError> {
    let provider = registry
        .find(&selection.provider_id)
        .ok_or_else(|| LlmError::AuthFailed {
            provider: selection.provider_id.clone(),
        })?;
    resolve_provider_definition(
        provider,
        selection.api_key_env.as_deref(),
        selection.base_url,
        selection.model,
        false,
    )
}

/// Resolve a full [`LlmConfig`] from a catalog selection.
pub fn resolve_llm_config_from_selection(
    selection: ProviderSelection,
    registry: &ProviderRegistry,
) -> Result<LlmConfig, LlmError> {
    let resolved = resolve_provider_config_from_selection(selection, registry)?;
    build_llm_config_from_resolved_provider(resolved)
}

/// Build a full [`LlmConfig`] from a catalog entry whose basic fields
/// have already been resolved and validated by the caller.
pub fn build_llm_config_from_resolved_provider(
    resolved: ResolvedProviderConfig,
) -> Result<LlmConfig, LlmError> {
    let chain = ChainSettings::from_env()?;
    let session = nearai_session_config();
    let nearai = nearai_config_from_resolved(&resolved, &chain)?;

    let mut provider = None;
    let mut bedrock = None;
    let mut gemini_oauth = None;
    let mut openai_codex = None;

    match resolved.protocol {
        ProviderProtocol::NearAi => {}
        ProviderProtocol::Bedrock => {
            bedrock = Some(
                BedrockConfig::build(
                    nonempty_env("BEDROCK_REGION"),
                    Some(resolved.model.clone()),
                    nonempty_env("BEDROCK_CROSS_REGION"),
                    nonempty_env("AWS_PROFILE"),
                )
                .map_err(config_error_to_llm_error("bedrock"))?,
            );
        }
        ProviderProtocol::GeminiOauth => {
            gemini_oauth = Some(GeminiOauthConfig::build(
                Some(resolved.model.clone()),
                nonempty_env("GEMINI_CREDENTIALS_PATH").map(PathBuf::from),
            ));
        }
        ProviderProtocol::OpenAiCodex => {
            openai_codex = Some(OpenAiCodexConfig::build(
                Some(resolved.model.clone()),
                nonempty_env("OPENAI_CODEX_AUTH_URL"),
                nonempty_env("OPENAI_CODEX_API_URL"),
                nonempty_env("OPENAI_CODEX_CLIENT_ID"),
                nonempty_env("OPENAI_CODEX_SESSION_PATH").map(PathBuf::from),
                parse_optional_u64("OPENAI_CODEX_REFRESH_MARGIN_SECS", "openai_codex")?,
            ));
        }
        ProviderProtocol::OpenAiCompletions
        | ProviderProtocol::Anthropic
        | ProviderProtocol::Ollama
        | ProviderProtocol::GithubCopilot
        | ProviderProtocol::DeepSeek
        | ProviderProtocol::Gemini
        | ProviderProtocol::OpenRouter => {
            provider = Some(build_registry_provider_config_from_resolved_provider(
                resolved.clone(),
            )?);
        }
    }

    Ok(LlmConfig {
        backend: resolved.provider_id,
        session,
        nearai,
        provider,
        bedrock,
        gemini_oauth,
        openai_codex,
        request_timeout_secs: chain.request_timeout_secs,
        cheap_model: chain.cheap_model,
        smart_routing_cascade: chain.smart_routing_cascade,
        max_retries: chain.max_retries,
        circuit_breaker_threshold: chain.circuit_breaker_threshold,
        circuit_breaker_recovery_secs: chain.circuit_breaker_recovery_secs,
        response_cache_enabled: chain.response_cache_enabled,
        response_cache_ttl_secs: chain.response_cache_ttl_secs,
        response_cache_max_entries: chain.response_cache_max_entries,
    })
}

/// Build a registry provider config from an already-resolved provider.
pub fn build_registry_provider_config_from_resolved_provider(
    resolved: ResolvedProviderConfig,
) -> Result<RegistryProviderConfig, LlmError> {
    if matches!(
        resolved.protocol,
        ProviderProtocol::NearAi
            | ProviderProtocol::Bedrock
            | ProviderProtocol::GeminiOauth
            | ProviderProtocol::OpenAiCodex
    ) {
        return Err(LlmError::RequestFailed {
            provider: resolved.provider_id,
            reason: "dedicated provider protocols require full LlmConfig resolution".to_string(),
        });
    }

    registry_config_from_resolved(resolved)
}

fn resolve_provider_definition_from_env(
    provider: &ProviderDefinition,
) -> Result<ResolvedProviderConfig, LlmError> {
    resolve_provider_definition(provider, None, None, None, true)
}

fn resolve_provider_definition(
    provider: &ProviderDefinition,
    api_key_env_override: Option<&str>,
    base_url_override: Option<String>,
    model_override: Option<String>,
    allow_llm_model_fallback: bool,
) -> Result<ResolvedProviderConfig, LlmError> {
    let api_key_env = api_key_env_override.or(provider.api_key_env.as_deref());
    let api_key = match api_key_env.and_then(nonempty_env) {
        Some(value) => Some(SecretString::from(value)),
        None if provider.api_key_required => {
            return Err(LlmError::AuthFailed {
                provider: provider.id.clone(),
            });
        }
        None => None,
    };
    let base_url = provider
        .base_url_env
        .as_deref()
        .and_then(nonempty_env)
        .or(base_url_override)
        .or_else(|| provider.default_base_url.clone())
        .unwrap_or_default();
    if provider.base_url_required && base_url.is_empty() {
        return Err(LlmError::RequestFailed {
            provider: provider.id.clone(),
            reason: "base URL is required but no base URL environment variable is set".to_string(),
        });
    }
    let model = nonempty_env(&provider.model_env)
        .or(model_override)
        .or_else(|| {
            allow_llm_model_fallback
                .then(|| nonempty_env("LLM_MODEL"))
                .flatten()
        })
        .unwrap_or_else(|| provider.default_model.clone());
    let extra_headers = provider
        .extra_headers_env
        .as_deref()
        .and_then(nonempty_env)
        .map(|value| parse_extra_headers(&provider.id, &value))
        .transpose()?
        .unwrap_or_default();
    let extra_headers = if provider.protocol == ProviderProtocol::GithubCopilot {
        merge_extra_headers(
            auth::default_headers(auth::AuthBackend::GithubCopilot),
            extra_headers,
        )
    } else {
        extra_headers
    };

    Ok(ResolvedProviderConfig {
        protocol: provider.protocol,
        provider_id: provider.id.clone(),
        api_key,
        base_url,
        model,
        extra_headers,
        oauth_token: None,
        is_codex_chatgpt: false,
        refresh_token: None,
        auth_path: None,
        cache_retention: None,
        unsupported_params: provider.unsupported_params.clone(),
    })
}

fn resolve_codex_cli_auth_provider() -> Result<ResolvedProviderConfig, LlmError> {
    let auth_path = std::env::var_os("CODEX_AUTH_PATH").map(PathBuf::from);
    let credentials =
        auth::load_persisted_credentials(CredentialSource::CodexCli, auth_path.as_deref())
            .ok_or_else(|| LlmError::AuthFailed {
                provider: "openai_codex".to_string(),
            })?;
    let model = nonempty_env("OPENAI_CODEX_MODEL")
        .or_else(|| nonempty_env("OPENAI_MODEL"))
        .or_else(|| nonempty_env("LLM_MODEL"))
        .unwrap_or_else(|| {
            if credentials.is_subscription {
                "gpt-5.3-codex".to_string()
            } else {
                "gpt-4o-mini".to_string()
            }
        });
    let provider_id = if credentials.is_subscription {
        "codex_chatgpt"
    } else {
        "openai"
    };

    let mut registry_config = RegistryProviderConfig::generic(
        ProviderProtocol::OpenAiCompletions,
        provider_id,
        Some(credentials.token),
        credentials.base_url,
        model,
    );
    registry_config.is_codex_chatgpt = credentials.is_subscription;
    registry_config.refresh_token = credentials.refresh_token;
    registry_config.auth_path = credentials.source_path;

    Ok(ResolvedProviderConfig {
        protocol: registry_config.protocol,
        provider_id: registry_config.provider_id,
        api_key: registry_config.api_key,
        base_url: registry_config.base_url,
        model: registry_config.model,
        extra_headers: registry_config.extra_headers,
        oauth_token: registry_config.oauth_token,
        is_codex_chatgpt: registry_config.is_codex_chatgpt,
        refresh_token: registry_config.refresh_token,
        auth_path: registry_config.auth_path,
        cache_retention: Some(registry_config.cache_retention),
        unsupported_params: registry_config.unsupported_params,
    })
}

fn registry_config_from_resolved(
    resolved: ResolvedProviderConfig,
) -> Result<RegistryProviderConfig, LlmError> {
    let mut config = RegistryProviderConfig::generic(
        resolved.protocol,
        resolved.provider_id.clone(),
        resolved.api_key,
        resolved.base_url,
        resolved.model,
    )
    .with_extra_headers(resolved.extra_headers)
    .with_unsupported_params(resolved.unsupported_params);
    config.oauth_token = resolved.oauth_token;
    config.is_codex_chatgpt = resolved.is_codex_chatgpt;
    config.refresh_token = resolved.refresh_token;
    config.auth_path = resolved.auth_path;

    if resolved.protocol == ProviderProtocol::Anthropic {
        config.cache_retention = match resolved.cache_retention {
            Some(cache_retention) => cache_retention,
            None => nonempty_env("ANTHROPIC_CACHE_RETENTION")
                .map(|value| {
                    value
                        .parse::<CacheRetention>()
                        .map_err(|reason| LlmError::RequestFailed {
                            provider: resolved.provider_id.clone(),
                            reason: format!("invalid ANTHROPIC_CACHE_RETENTION: {reason}"),
                        })
                })
                .transpose()?
                .unwrap_or_default(),
        };

        if config.oauth_token.is_none() {
            if let Some(token) = nonempty_env("ANTHROPIC_OAUTH_TOKEN") {
                config.oauth_token = Some(SecretString::from(token));
                if config.api_key.is_none() {
                    config.api_key = Some(SecretString::from(OAUTH_PLACEHOLDER.to_string()));
                }
            }
        } else if config.api_key.is_none() {
            config.api_key = Some(SecretString::from(OAUTH_PLACEHOLDER.to_string()));
        }
    } else if let Some(cache_retention) = resolved.cache_retention {
        config.cache_retention = cache_retention;
    }

    Ok(config)
}

fn nearai_config_from_resolved(
    resolved: &ResolvedProviderConfig,
    chain: &ChainSettings,
) -> Result<NearAiConfig, LlmError> {
    let api_key = if resolved.protocol == ProviderProtocol::NearAi {
        resolved.api_key.clone()
    } else {
        nonempty_env("NEARAI_API_KEY").map(SecretString::from)
    };
    let base_url = if resolved.protocol == ProviderProtocol::NearAi && !resolved.base_url.is_empty()
    {
        resolved.base_url.clone()
    } else if let Some(base_url) = nonempty_env("NEARAI_BASE_URL") {
        base_url
    } else if api_key.is_some() {
        "https://cloud-api.near.ai".to_string()
    } else {
        "https://private.near.ai".to_string()
    };
    let model = if resolved.protocol == ProviderProtocol::NearAi {
        resolved.model.clone()
    } else {
        nonempty_env("NEARAI_MODEL").unwrap_or_else(|| crate::DEFAULT_MODEL.to_string())
    };

    let failover_cooldown_secs = if resolved.protocol == ProviderProtocol::NearAi {
        parse_optional_u64("LLM_FAILOVER_COOLDOWN_SECS", "nearai")?.unwrap_or(300)
    } else {
        300
    };
    let failover_cooldown_threshold = if resolved.protocol == ProviderProtocol::NearAi {
        parse_optional_u32("LLM_FAILOVER_THRESHOLD", "nearai")?.unwrap_or(3)
    } else {
        3
    };

    Ok(NearAiConfig {
        model,
        cheap_model: nonempty_env("NEARAI_CHEAP_MODEL"),
        base_url,
        api_key,
        fallback_model: nonempty_env("NEARAI_FALLBACK_MODEL"),
        max_retries: chain.max_retries,
        circuit_breaker_threshold: chain.circuit_breaker_threshold,
        circuit_breaker_recovery_secs: chain.circuit_breaker_recovery_secs,
        response_cache_enabled: chain.response_cache_enabled,
        response_cache_ttl_secs: chain.response_cache_ttl_secs,
        response_cache_max_entries: chain.response_cache_max_entries,
        failover_cooldown_secs,
        failover_cooldown_threshold,
        smart_routing_cascade: chain.smart_routing_cascade,
    })
}

fn nearai_session_config() -> SessionConfig {
    SessionConfig {
        auth_base_url: nonempty_env("NEARAI_AUTH_URL")
            .unwrap_or_else(|| "https://private.near.ai".to_string()),
        session_path: nonempty_env("NEARAI_SESSION_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| ironclaw_common::paths::ironclaw_base_dir().join("session.json")),
    }
}

#[derive(Debug, Clone)]
struct ChainSettings {
    request_timeout_secs: u64,
    cheap_model: Option<String>,
    smart_routing_cascade: bool,
    max_retries: u32,
    circuit_breaker_threshold: Option<u32>,
    circuit_breaker_recovery_secs: u64,
    response_cache_enabled: bool,
    response_cache_ttl_secs: u64,
    response_cache_max_entries: usize,
}

impl Default for ChainSettings {
    fn default() -> Self {
        Self {
            request_timeout_secs: 120,
            cheap_model: None,
            smart_routing_cascade: true,
            max_retries: 3,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
        }
    }
}

impl ChainSettings {
    fn from_env() -> Result<Self, LlmError> {
        let defaults = Self::default();
        Ok(Self {
            request_timeout_secs: parse_optional_u64("LLM_REQUEST_TIMEOUT_SECS", "llm_config")?
                .unwrap_or(defaults.request_timeout_secs),
            cheap_model: nonempty_env("LLM_CHEAP_MODEL"),
            smart_routing_cascade: parse_optional_bool("SMART_ROUTING_CASCADE", "llm_config")?
                .unwrap_or(defaults.smart_routing_cascade),
            max_retries: parse_optional_u32("LLM_MAX_RETRIES", "llm_config")?
                .unwrap_or(defaults.max_retries),
            circuit_breaker_threshold: parse_optional_u32(
                "LLM_CIRCUIT_BREAKER_THRESHOLD",
                "llm_config",
            )?,
            circuit_breaker_recovery_secs: parse_optional_u64(
                "LLM_CIRCUIT_BREAKER_RECOVERY_SECS",
                "llm_config",
            )?
            .unwrap_or(defaults.circuit_breaker_recovery_secs),
            response_cache_enabled: parse_optional_bool(
                "LLM_RESPONSE_CACHE_ENABLED",
                "llm_config",
            )?
            .unwrap_or(defaults.response_cache_enabled),
            response_cache_ttl_secs: parse_optional_u64(
                "LLM_RESPONSE_CACHE_TTL_SECS",
                "llm_config",
            )?
            .unwrap_or(defaults.response_cache_ttl_secs),
            response_cache_max_entries: parse_optional_usize(
                "LLM_RESPONSE_CACHE_MAX_ENTRIES",
                "llm_config",
            )?
            .unwrap_or(defaults.response_cache_max_entries),
        })
    }
}

fn try_load_provider_registry(
    user_providers_path: Option<&Path>,
) -> Result<ProviderRegistry, LlmError> {
    ProviderRegistry::try_load_from_path(user_providers_path).map_err(|source| {
        LlmError::RequestFailed {
            provider: "provider_registry".to_string(),
            reason: source.to_string(),
        }
    })
}

fn provider_env_present(provider: &ProviderDefinition) -> bool {
    provider
        .api_key_env
        .as_deref()
        .and_then(nonempty_env)
        .is_some()
        || provider
            .base_url_env
            .as_deref()
            .and_then(nonempty_env)
            .is_some()
        || nonempty_env(&provider.model_env).is_some()
}

fn parse_extra_headers(provider: &str, value: &str) -> Result<Vec<(String, String)>, LlmError> {
    let mut headers = Vec::new();
    for part in value.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((key, header_value)) = part.split_once(':') else {
            return Err(LlmError::RequestFailed {
                provider: provider.to_string(),
                reason: "extra header must use `Name:Value` format".to_string(),
            });
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(LlmError::RequestFailed {
                provider: provider.to_string(),
                reason: "extra header name must not be empty".to_string(),
            });
        }
        headers.push((key.to_string(), header_value.trim().to_string()));
    }
    Ok(headers)
}

fn merge_extra_headers(
    defaults: Vec<(String, String)>,
    overrides: Vec<(String, String)>,
) -> Vec<(String, String)> {
    let mut merged = defaults;
    for (key, value) in overrides {
        if let Some((_, existing_value)) = merged
            .iter_mut()
            .find(|(existing_key, _)| existing_key.eq_ignore_ascii_case(&key))
        {
            *existing_value = value;
        } else {
            merged.push((key, value));
        }
    }
    merged
}

fn codex_auth_enabled_from_env() -> bool {
    std::env::var("LLM_USE_CODEX_AUTH")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn parse_optional_bool(name: &str, provider: &str) -> Result<Option<bool>, LlmError> {
    nonempty_env(name)
        .map(|value| {
            value
                .parse::<bool>()
                .map_err(|source| invalid_env(provider, name, source))
        })
        .transpose()
}

fn parse_optional_u32(name: &str, provider: &str) -> Result<Option<u32>, LlmError> {
    nonempty_env(name)
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|source| invalid_env(provider, name, source))
        })
        .transpose()
}

fn parse_optional_u64(name: &str, provider: &str) -> Result<Option<u64>, LlmError> {
    nonempty_env(name)
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|source| invalid_env(provider, name, source))
        })
        .transpose()
}

fn parse_optional_usize(name: &str, provider: &str) -> Result<Option<usize>, LlmError> {
    nonempty_env(name)
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|source| invalid_env(provider, name, source))
        })
        .transpose()
}

fn invalid_env(provider: &str, name: &str, source: impl std::fmt::Display) -> LlmError {
    LlmError::RequestFailed {
        provider: provider.to_string(),
        reason: format!("{name} is invalid: {source}"),
    }
}

fn config_error_to_llm_error(provider: &'static str) -> impl FnOnce(LlmConfigError) -> LlmError {
    move |source| LlmError::RequestFailed {
        provider: provider.to_string(),
        reason: source.to_string(),
    }
}
