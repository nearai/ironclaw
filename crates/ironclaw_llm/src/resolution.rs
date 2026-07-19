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

/// Already-resolved provider input from env or a catalog selection.
#[derive(Debug, Clone)]
pub enum ResolvedProviderConfig {
    Registry(RegistryProviderConfig),
    Dedicated(ResolvedDedicatedProviderConfig),
}

/// Resolved input for providers represented by dedicated `LlmConfig` slots.
#[derive(Debug, Clone)]
pub struct ResolvedDedicatedProviderConfig {
    pub protocol: ProviderProtocol,
    pub provider_id: String,
    pub api_key: Option<SecretString>,
    pub base_url: String,
    pub model: String,
}

impl ResolvedProviderConfig {
    pub fn provider_id(&self) -> &str {
        match self {
            Self::Registry(config) => &config.provider_id,
            Self::Dedicated(config) => &config.provider_id,
        }
    }

    pub fn base_url(&self) -> &str {
        match self {
            Self::Registry(config) => &config.base_url,
            Self::Dedicated(config) => &config.base_url,
        }
    }

    pub fn model(&self) -> &str {
        match self {
            Self::Registry(config) => &config.model,
            Self::Dedicated(config) => &config.model,
        }
    }

    /// The resolved API key, if the provider carries one — the same value
    /// (from env) that would otherwise only ever reach a live provider
    /// client, exposed so a caller (onboard's env-detect step) can persist
    /// it into the encrypted secret store rather than leaving it only in
    /// the process's shell env.
    pub fn api_key(&self) -> Option<&SecretString> {
        match self {
            Self::Registry(config) => config.api_key.as_ref(),
            Self::Dedicated(config) => config.api_key.as_ref(),
        }
    }
}

/// Provider selection overrides supplied by a composition root.
#[derive(Debug, Clone)]
pub struct ProviderSelection {
    pub provider_id: String,
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderResolutionError {
    #[error("provider `{provider}` is not in the provider registry")]
    UnknownProvider { provider: String },
    #[error("provider `{provider}` requires an API key")]
    MissingApiKey { provider: String },
    #[error("provider `{provider}` requires a base URL")]
    MissingBaseUrl { provider: String },
    #[error(transparent)]
    Llm(#[from] LlmError),
}

impl ProviderResolutionError {
    pub fn into_llm_error(self) -> LlmError {
        match self {
            Self::UnknownProvider { provider } | Self::MissingApiKey { provider } => {
                LlmError::AuthFailed { provider }
            }
            Self::MissingBaseUrl { provider } => LlmError::RequestFailed {
                provider,
                reason: "base URL is required but no base URL environment variable is set"
                    .to_string(),
            },
            Self::Llm(source) => source,
        }
    }
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
        if provider.protocol == ProviderProtocol::OpenAiCodex
            && (codex_auth_enabled_from_env() || std::env::var_os("CODEX_AUTH_PATH").is_some())
        {
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
) -> Result<ResolvedProviderConfig, ProviderResolutionError> {
    let provider = registry.find(&selection.provider_id).ok_or_else(|| {
        ProviderResolutionError::UnknownProvider {
            provider: selection.provider_id.clone(),
        }
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
    let resolved = resolve_provider_config_from_selection(selection, registry)
        .map_err(ProviderResolutionError::into_llm_error)?;
    build_llm_config_from_resolved_provider(resolved)
}

/// Build a full [`LlmConfig`] from a catalog entry whose basic fields
/// have already been resolved and validated by the caller.
pub fn build_llm_config_from_resolved_provider(
    resolved: ResolvedProviderConfig,
) -> Result<LlmConfig, LlmError> {
    let chain = ChainSettings::from_env()?;
    let session = nearai_session_config();

    let backend = resolved.provider_id().to_string();
    let mut nearai = nearai_config_from_env(&chain)?;
    let mut provider = None;
    let mut bedrock = None;
    let mut gemini_oauth = None;
    let mut openai_codex = None;

    match resolved {
        ResolvedProviderConfig::Registry(registry_config) => {
            provider = Some(registry_config);
        }
        ResolvedProviderConfig::Dedicated(dedicated) => match dedicated.protocol {
            ProviderProtocol::NearAi => {
                nearai = nearai_config_from_dedicated(&dedicated, &chain)?;
            }
            ProviderProtocol::Bedrock => {
                bedrock = Some(
                    BedrockConfig::build(
                        nonempty_env("BEDROCK_REGION"),
                        Some(dedicated.model.clone()),
                        nonempty_env("BEDROCK_CROSS_REGION"),
                        nonempty_env("AWS_PROFILE"),
                    )
                    .map_err(config_error_to_llm_error("bedrock"))?,
                );
            }
            ProviderProtocol::GeminiOauth => {
                gemini_oauth = Some(GeminiOauthConfig::build(
                    Some(dedicated.model.clone()),
                    nonempty_env("GEMINI_CREDENTIALS_PATH").map(PathBuf::from),
                ));
            }
            ProviderProtocol::OpenAiCodex => {
                openai_codex = Some(OpenAiCodexConfig::build(
                    Some(dedicated.model.clone()),
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
                return Err(LlmError::RequestFailed {
                    provider: dedicated.provider_id,
                    reason: "registry provider protocol resolved as dedicated config".to_string(),
                });
            }
        },
    }

    Ok(LlmConfig {
        backend,
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
    match resolved {
        ResolvedProviderConfig::Registry(config) => Ok(config),
        ResolvedProviderConfig::Dedicated(config) => Err(LlmError::RequestFailed {
            provider: config.provider_id,
            reason: "dedicated provider protocols require full LlmConfig resolution".to_string(),
        }),
    }
}

fn resolve_provider_definition_from_env(
    provider: &ProviderDefinition,
) -> Result<ResolvedProviderConfig, LlmError> {
    resolve_provider_definition(provider, None, None, None, true)
        .map_err(ProviderResolutionError::into_llm_error)
}

fn resolve_provider_definition(
    provider: &ProviderDefinition,
    api_key_env_override: Option<&str>,
    base_url_override: Option<String>,
    model_override: Option<String>,
    allow_llm_model_fallback: bool,
) -> Result<ResolvedProviderConfig, ProviderResolutionError> {
    let api_key_env = api_key_env_override.or(provider.api_key_env.as_deref());
    let api_key = match api_key_env.and_then(nonempty_env) {
        Some(value) => Some(SecretString::from(value)),
        None if provider.api_key_required => {
            return Err(ProviderResolutionError::MissingApiKey {
                provider: provider.id.clone(),
            });
        }
        None => None,
    };
    // Precedence: an explicit override (the operator's WebUI/config.toml
    // selection) wins over the provider's ambient env var, which wins over
    // the catalog default. The env-fallback path (`resolve_provider_*_from_env`)
    // passes `None` overrides, so pure-env deployments keep env-first behavior.
    // Putting the env var first here would let a startup `NEARAI_BASE_URL` /
    // `NEARAI_MODEL` silently override what a user just picked in onboarding.
    let base_url = base_url_override
        .or_else(|| provider.base_url_env.as_deref().and_then(nonempty_env))
        .or_else(|| provider.default_base_url.clone())
        .unwrap_or_default();
    if provider.base_url_required && base_url.is_empty() {
        return Err(ProviderResolutionError::MissingBaseUrl {
            provider: provider.id.clone(),
        });
    }
    let model = model_override
        .or_else(|| nonempty_env(&provider.model_env))
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
        .transpose()
        .map_err(ProviderResolutionError::Llm)?
        .unwrap_or_default();
    let extra_headers = if provider.protocol == ProviderProtocol::GithubCopilot {
        merge_extra_headers(
            auth::default_headers(auth::AuthBackend::GithubCopilot),
            extra_headers,
        )
    } else {
        extra_headers
    };

    let resolved = ResolvedDedicatedProviderConfig {
        protocol: provider.protocol,
        provider_id: provider.id.clone(),
        api_key,
        base_url,
        model,
    };

    if is_registry_protocol(provider.protocol) {
        let mut config = RegistryProviderConfig::generic(
            resolved.protocol,
            resolved.provider_id,
            resolved.api_key,
            resolved.base_url,
            resolved.model,
        )
        .with_extra_headers(extra_headers)
        .with_unsupported_params(provider.unsupported_params.clone());
        apply_registry_provider_env(&mut config).map_err(ProviderResolutionError::Llm)?;
        Ok(ResolvedProviderConfig::Registry(config))
    } else {
        Ok(ResolvedProviderConfig::Dedicated(resolved))
    }
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
                // Subscription (ChatGPT-account) Codex rejects codex-only slugs
                // like `gpt-5.3-codex` with HTTP 400; use a model the account
                // is entitled to.
                "gpt-5.5".to_string()
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

    Ok(ResolvedProviderConfig::Registry(registry_config))
}

fn apply_registry_provider_env(config: &mut RegistryProviderConfig) -> Result<(), LlmError> {
    if config.protocol == ProviderProtocol::Anthropic {
        config.cache_retention = nonempty_env("ANTHROPIC_CACHE_RETENTION")
            .map(|value| {
                value
                    .parse::<CacheRetention>()
                    .map_err(|reason| LlmError::RequestFailed {
                        provider: config.provider_id.clone(),
                        reason: format!("invalid ANTHROPIC_CACHE_RETENTION: {reason}"),
                    })
            })
            .transpose()?
            .unwrap_or_default();

        if let Some(token) = nonempty_env("ANTHROPIC_OAUTH_TOKEN") {
            config.oauth_token = Some(SecretString::from(token));
            if config.api_key.is_none() {
                config.api_key = Some(SecretString::from(OAUTH_PLACEHOLDER.to_string()));
            }
        }
    }
    Ok(())
}

fn nearai_config_from_env(chain: &ChainSettings) -> Result<NearAiConfig, LlmError> {
    let api_key = nonempty_env("NEARAI_API_KEY").map(SecretString::from);
    let base_url = default_nearai_base_url(nonempty_env("NEARAI_BASE_URL"));
    Ok(build_nearai_config(
        NearAiRuntimeFields {
            model: nonempty_env("NEARAI_MODEL").unwrap_or_else(|| crate::DEFAULT_MODEL.to_string()),
            api_key,
            base_url,
            failover_cooldown_secs: 300,
            failover_cooldown_threshold: 3,
        },
        chain,
    ))
}

fn nearai_config_from_dedicated(
    resolved: &ResolvedDedicatedProviderConfig,
    chain: &ChainSettings,
) -> Result<NearAiConfig, LlmError> {
    let api_key = resolved.api_key.clone();
    let configured_base_url = if resolved.base_url.is_empty() {
        nonempty_env("NEARAI_BASE_URL")
    } else {
        Some(resolved.base_url.clone())
    };
    let base_url = default_nearai_base_url(configured_base_url);

    Ok(build_nearai_config(
        NearAiRuntimeFields {
            model: resolved.model.clone(),
            api_key,
            base_url,
            failover_cooldown_secs: parse_optional_u64("LLM_FAILOVER_COOLDOWN_SECS", "nearai")?
                .unwrap_or(300),
            failover_cooldown_threshold: parse_optional_u32("LLM_FAILOVER_THRESHOLD", "nearai")?
                .unwrap_or(3),
        },
        chain,
    ))
}

struct NearAiRuntimeFields {
    model: String,
    api_key: Option<SecretString>,
    base_url: String,
    failover_cooldown_secs: u64,
    failover_cooldown_threshold: u32,
}

fn build_nearai_config(fields: NearAiRuntimeFields, chain: &ChainSettings) -> NearAiConfig {
    NearAiConfig {
        model: fields.model,
        cheap_model: nonempty_env("NEARAI_CHEAP_MODEL"),
        base_url: fields.base_url,
        api_key: fields.api_key,
        fallback_model: nonempty_env("NEARAI_FALLBACK_MODEL"),
        max_retries: chain.max_retries,
        circuit_breaker_threshold: chain.circuit_breaker_threshold,
        circuit_breaker_recovery_secs: chain.circuit_breaker_recovery_secs,
        response_cache_enabled: chain.response_cache_enabled,
        response_cache_ttl_secs: chain.response_cache_ttl_secs,
        response_cache_max_entries: chain.response_cache_max_entries,
        failover_cooldown_secs: fields.failover_cooldown_secs,
        failover_cooldown_threshold: fields.failover_cooldown_threshold,
        smart_routing_cascade: chain.smart_routing_cascade,
    }
}

pub const NEARAI_CLOUD_DEFAULT_BASE_URL: &str = "https://cloud-api.near.ai";
/// No longer used by [`default_nearai_base_url`] (nearai always defaults to
/// cloud regardless of key presence — see that function's doc comment).
/// Kept only because `session.rs`'s OAuth/session-token auth URL default and
/// v1 `src/` still reference it independently of provider-config base-URL
/// resolution.
pub const NEARAI_PRIVATE_DEFAULT_BASE_URL: &str = "https://private.near.ai";

/// Resolve the nearai provider's base URL: an explicit override always wins,
/// otherwise the cloud endpoint.
///
/// Used to key on API-key presence (cloud when a key was present, private
/// otherwise), to match the session-token-only private backend's implicit
/// no-key affordance. That coupling made resolution order load-bearing:
/// whichever step attached the key had to run *before* this was called, and
/// an operator-stored key (attached after resolution, from the secret store)
/// always missed the window and landed on the keyless private default. There
/// is now exactly one nearai default — cloud — so resolution order no longer
/// matters and every caller (probe, resolution, snapshot display) agrees
/// unconditionally.
pub fn default_nearai_base_url(configured_base_url: Option<String>) -> String {
    configured_base_url.unwrap_or_else(|| NEARAI_CLOUD_DEFAULT_BASE_URL.to_string())
}

fn is_registry_protocol(protocol: ProviderProtocol) -> bool {
    matches!(
        protocol,
        ProviderProtocol::OpenAiCompletions
            | ProviderProtocol::Anthropic
            | ProviderProtocol::Ollama
            | ProviderProtocol::GithubCopilot
            | ProviderProtocol::DeepSeek
            | ProviderProtocol::Gemini
            | ProviderProtocol::OpenRouter
    )
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
            request_timeout_secs: crate::config::DEFAULT_REQUEST_TIMEOUT_SECS,
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
            request_timeout_secs: parse_option_env::<u64>(
                "LLM_REQUEST_TIMEOUT_SECS",
                "llm_config",
            )?
            .unwrap_or(defaults.request_timeout_secs),
            cheap_model: nonempty_env("LLM_CHEAP_MODEL"),
            smart_routing_cascade: parse_option_env::<bool>("SMART_ROUTING_CASCADE", "llm_config")?
                .unwrap_or(defaults.smart_routing_cascade),
            max_retries: parse_option_env_with_fallback::<u32>(
                "LLM_MAX_RETRIES",
                "NEARAI_MAX_RETRIES",
                "llm_config",
            )?
            .unwrap_or(defaults.max_retries),
            circuit_breaker_threshold: parse_option_env_with_fallback::<u32>(
                "LLM_CIRCUIT_BREAKER_THRESHOLD",
                "CIRCUIT_BREAKER_THRESHOLD",
                "llm_config",
            )?,
            circuit_breaker_recovery_secs: parse_option_env_with_fallback::<u64>(
                "LLM_CIRCUIT_BREAKER_RECOVERY_SECS",
                "CIRCUIT_BREAKER_RECOVERY_SECS",
                "llm_config",
            )?
            .unwrap_or(defaults.circuit_breaker_recovery_secs),
            response_cache_enabled: parse_option_env_with_fallback::<bool>(
                "LLM_RESPONSE_CACHE_ENABLED",
                "RESPONSE_CACHE_ENABLED",
                "llm_config",
            )?
            .unwrap_or(defaults.response_cache_enabled),
            response_cache_ttl_secs: parse_option_env_with_fallback::<u64>(
                "LLM_RESPONSE_CACHE_TTL_SECS",
                "RESPONSE_CACHE_TTL_SECS",
                "llm_config",
            )?
            .unwrap_or(defaults.response_cache_ttl_secs),
            response_cache_max_entries: parse_option_env_with_fallback::<usize>(
                "LLM_RESPONSE_CACHE_MAX_ENTRIES",
                "RESPONSE_CACHE_MAX_ENTRIES",
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

fn parse_optional_u32(name: &str, provider: &str) -> Result<Option<u32>, LlmError> {
    parse_option_env(name, provider)
}

fn parse_optional_u64(name: &str, provider: &str) -> Result<Option<u64>, LlmError> {
    parse_option_env(name, provider)
}

trait EnvParse: Sized {
    fn parse_env(value: &str) -> Result<Self, String>;
}

macro_rules! impl_env_parse_from_str {
    ($($ty:ty),* $(,)?) => {
        $(
            impl EnvParse for $ty {
                fn parse_env(value: &str) -> Result<Self, String> {
                    value.parse::<Self>().map_err(|source| source.to_string())
                }
            }
        )*
    };
}

impl_env_parse_from_str!(u32, u64, usize);

impl EnvParse for bool {
    fn parse_env(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            _ => Err("expected a boolean value".to_string()),
        }
    }
}

fn parse_option_env<T: EnvParse>(name: &str, provider: &str) -> Result<Option<T>, LlmError> {
    nonempty_env(name)
        .map(|value| T::parse_env(&value).map_err(|source| invalid_env(provider, name, source)))
        .transpose()
}

fn parse_option_env_with_fallback<T: EnvParse>(
    primary: &str,
    fallback: &str,
    provider: &str,
) -> Result<Option<T>, LlmError> {
    match parse_option_env(primary, provider)? {
        Some(value) => Ok(Some(value)),
        None => parse_option_env(fallback, provider),
    }
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

#[cfg(test)]
mod tests {
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
}
