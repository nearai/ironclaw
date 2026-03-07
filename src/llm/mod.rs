//! LLM integration for the agent.
//!
//! Supports multiple backends:
//! - **NEAR AI** (default): Session token or API key auth via Chat Completions API
//! - **OpenAI**: Direct API access with your own key
//! - **Anthropic**: Direct API access with your own key
//! - **Ollama**: Local model inference
//! - **OpenAI-compatible**: Any endpoint that speaks the OpenAI API

mod anthropic_oauth;
pub mod circuit_breaker;
pub mod codex_chatgpt;
pub mod costs;
pub mod failover;
mod nearai_chat;
mod provider;
mod reasoning;
pub mod recording;
pub mod registry;
pub mod response_cache;
pub mod retry;
mod rig_adapter;
pub mod session;
pub mod smart_routing;

pub use circuit_breaker::{CircuitBreakerConfig, CircuitBreakerProvider};
pub use failover::{CooldownConfig, FailoverProvider};
pub use nearai_chat::{ModelInfo, NearAiChatProvider};
pub use provider::{
    ChatMessage, CompletionRequest, CompletionResponse, ContentPart, FinishReason, ImageUrl,
    LlmProvider, ModelMetadata, Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse,
    ToolDefinition, ToolResult,
};
pub use reasoning::{
    ActionPlan, Reasoning, ReasoningContext, RespondOutput, RespondResult, SILENT_REPLY_TOKEN,
    TOOL_INTENT_NUDGE, TokenUsage, ToolSelection, is_silent_reply, llm_signals_tool_intent,
};
pub use recording::RecordingLlm;
pub use registry::{ProviderDefinition, ProviderProtocol, ProviderRegistry};
pub use response_cache::{CachedProvider, ResponseCacheConfig};
pub use retry::{RetryConfig, RetryProvider};
pub use rig_adapter::RigAdapter;
pub use session::{SessionConfig, SessionManager, create_session_manager};
pub use smart_routing::{SmartRoutingConfig, SmartRoutingProvider, TaskComplexity};

use std::sync::Arc;

use rig::client::CompletionClient;
use secrecy::ExposeSecret;

use crate::config::{LlmConfig, NearAiConfig, RegistryProviderConfig};
use crate::error::LlmError;

/// Create an LLM provider based on configuration.
///
/// - NearAI backend: Uses session manager for authentication
/// - Registry providers: Looked up by protocol and constructed generically
pub fn create_llm_provider(
    config: &LlmConfig,
    session: Arc<SessionManager>,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    if config.backend == "nearai" || config.backend == "near_ai" || config.backend == "near" {
        return create_llm_provider_with_config(&config.nearai, session);
    }

    let reg_config = config
        .provider
        .as_ref()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: config.backend.clone(),
        })?;

    create_registry_provider(reg_config)
}

/// Create an LLM provider from a `NearAiConfig` directly.
///
/// This is useful when constructing additional providers for failover,
/// where only the model name differs from the primary config.
pub fn create_llm_provider_with_config(
    config: &NearAiConfig,
    session: Arc<SessionManager>,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let auth_mode = if config.api_key.is_some() {
        "API key"
    } else {
        "session token"
    };
    tracing::info!(
        model = %config.model,
        base_url = %config.base_url,
        auth = auth_mode,
        "Using NEAR AI (Chat Completions API)"
    );
    Ok(Arc::new(NearAiChatProvider::new(config.clone(), session)?))
}

/// Create a provider from a registry-resolved config.
///
/// Dispatches on `RegistryProviderConfig::protocol` to build the appropriate
/// rig-core client. This single function replaces what used to be 5 separate
/// `create_*_provider` functions.
fn create_registry_provider(
    config: &RegistryProviderConfig,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    // Codex ChatGPT mode: use the Responses API provider
    if config.is_codex_chatgpt {
        return create_codex_chatgpt_from_registry(config);
    }

    match config.protocol {
        ProviderProtocol::OpenAiCompletions => create_openai_compat_from_registry(config),
        ProviderProtocol::Anthropic => create_anthropic_from_registry(config),
        ProviderProtocol::Ollama => create_ollama_from_registry(config),
    }
}

fn create_codex_chatgpt_from_registry(
    config: &RegistryProviderConfig,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let api_key = config
        .api_key
        .as_ref()
        .map(|k| k.expose_secret().to_string())
        .ok_or_else(|| LlmError::AuthFailed {
            provider: "codex_chatgpt".to_string(),
        })?;

    tracing::info!(
        configured_model = %config.model,
        base_url = %config.base_url,
        "Using Codex ChatGPT provider (Responses API) — auto-detecting model"
    );

    // Run the async model auto-detection synchronously.
    // This is safe because build_provider_chain runs on a multi-threaded tokio runtime.
    let provider = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(
            codex_chatgpt::CodexChatGptProvider::with_auto_model(
                &config.base_url,
                &api_key,
                &config.model,
            ),
        )
    });

    Ok(Arc::new(provider))
}

fn create_openai_compat_from_registry(
    config: &RegistryProviderConfig,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    use rig::providers::openai;

    let mut extra_headers = reqwest::header::HeaderMap::new();
    for (key, value) in &config.extra_headers {
        let name = match reqwest::header::HeaderName::from_bytes(key.as_bytes()) {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(header = %key, error = %e, "Skipping extra header: invalid name");
                continue;
            }
        };
        let val = match reqwest::header::HeaderValue::from_str(value) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(header = %key, error = %e, "Skipping extra header: invalid value");
                continue;
            }
        };
        extra_headers.insert(name, val);
    }

    let api_key = config
        .api_key
        .as_ref()
        .map(|k| k.expose_secret().to_string())
        .unwrap_or_else(|| {
            tracing::warn!(
                provider = %config.provider_id,
                "No API key configured for {}. Requests will likely fail with 401. \
                 Check your .env or secrets store.",
                config.provider_id,
            );
            "no-key".to_string()
        });

    let mut builder = openai::Client::builder().api_key(&api_key);
    if !config.base_url.is_empty() {
        builder = builder.base_url(&config.base_url);
    }
    if !extra_headers.is_empty() {
        builder = builder.http_headers(extra_headers);
    }

    let client: openai::Client = builder.build().map_err(|e| LlmError::RequestFailed {
        provider: config.provider_id.clone(),
        reason: format!("Failed to create OpenAI-compatible client: {e}"),
    })?;

    // Use CompletionsClient (Chat Completions API) instead of the default
    // Client (Responses API). The Responses API path in rig-core handles
    // tool results differently, which breaks IronClaw's tool call flow.
    let client = client.completions_api();
    let model = client.completion_model(&config.model);

    tracing::info!(
        provider = %config.provider_id,
        model = %config.model,
        base_url = %config.base_url,
        "Using OpenAI-compatible provider"
    );

    Ok(Arc::new(RigAdapter::new(model, &config.model)))
}

fn create_anthropic_from_registry(
    config: &RegistryProviderConfig,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    // Route to OAuth provider when an OAuth token is present and no real API
    // key was provided. When both are set, the API key takes priority (standard
    // x-api-key auth via rig-core).
    let api_key_is_placeholder = config
        .api_key
        .as_ref()
        .is_some_and(|k| k.expose_secret() == crate::config::llm::OAUTH_PLACEHOLDER);
    if config.oauth_token.is_some() && (config.api_key.is_none() || api_key_is_placeholder) {
        tracing::info!(
            provider = %config.provider_id,
            model = %config.model,
            base_url = if config.base_url.is_empty() { "default" } else { &config.base_url },
            "Using Anthropic OAuth API"
        );
        let provider = anthropic_oauth::AnthropicOAuthProvider::new(config)?;
        return Ok(Arc::new(provider));
    }

    use crate::config::CacheRetention;
    use crate::config::helpers::optional_env;
    use rig::providers::anthropic;

    let api_key = config
        .api_key
        .as_ref()
        .map(|k| k.expose_secret().to_string())
        .ok_or_else(|| LlmError::AuthFailed {
            provider: config.provider_id.clone(),
        })?;

    let client: anthropic::Client = if config.base_url.is_empty() {
        anthropic::Client::new(&api_key)
    } else {
        anthropic::Client::builder()
            .api_key(&api_key)
            .base_url(&config.base_url)
            .build()
    }
    .map_err(|e| LlmError::RequestFailed {
        provider: config.provider_id.clone(),
        reason: format!("Failed to create Anthropic client: {e}"),
    })?;

    // Resolve prompt cache retention from env (default: Short).
    // Injects top-level cache_control via additional_params for Anthropic
    // automatic caching (the API auto-places the breakpoint at the last
    // cacheable block).
    let cache_retention: CacheRetention = optional_env("ANTHROPIC_CACHE_RETENTION")
        .ok()
        .flatten()
        .and_then(|val| match val.parse::<CacheRetention>() {
            Ok(r) => Some(r),
            Err(e) => {
                tracing::warn!("Invalid ANTHROPIC_CACHE_RETENTION: {e}; defaulting to short");
                None
            }
        })
        .unwrap_or_default();

    let model = client.completion_model(&config.model);

    if cache_retention != CacheRetention::None {
        tracing::info!(
            model = %config.model,
            retention = %cache_retention,
            "Anthropic automatic prompt caching enabled"
        );
    }

    tracing::info!(
        provider = %config.provider_id,
        model = %config.model,
        base_url = if config.base_url.is_empty() { "default" } else { &config.base_url },
        "Using Anthropic provider"
    );

    Ok(Arc::new(
        RigAdapter::new(model, &config.model).with_cache_retention(cache_retention),
    ))
}

fn create_ollama_from_registry(
    config: &RegistryProviderConfig,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    use rig::client::Nothing;
    use rig::providers::ollama;

    let client: ollama::Client = ollama::Client::builder()
        .base_url(&config.base_url)
        .api_key(Nothing)
        .build()
        .map_err(|e| LlmError::RequestFailed {
            provider: config.provider_id.clone(),
            reason: format!("Failed to create Ollama client: {e}"),
        })?;

    let model = client.completion_model(&config.model);

    tracing::info!(
        provider = %config.provider_id,
        model = %config.model,
        base_url = %config.base_url,
        "Using Ollama provider"
    );

    Ok(Arc::new(RigAdapter::new(model, &config.model)))
}

/// Create a cheap/fast LLM provider for lightweight tasks (heartbeat, routing, evaluation).
///
/// Uses `NEARAI_CHEAP_MODEL` if set, otherwise falls back to the main provider.
/// Currently only supports NEAR AI backend.
pub fn create_cheap_llm_provider(
    config: &LlmConfig,
    session: Arc<SessionManager>,
) -> Result<Option<Arc<dyn LlmProvider>>, LlmError> {
    let Some(ref cheap_model) = config.nearai.cheap_model else {
        return Ok(None);
    };

    if config.backend != "nearai" {
        tracing::warn!(
            "NEARAI_CHEAP_MODEL is set but LLM_BACKEND is '{}', not nearai. \
             Cheap model setting will be ignored.",
            config.backend
        );
        return Ok(None);
    }

    let mut cheap_config = config.nearai.clone();
    cheap_config.model = cheap_model.clone();

    Ok(Some(Arc::new(NearAiChatProvider::new(
        cheap_config,
        session,
    )?)))
}

/// Build the full LLM provider chain with all configured wrappers.
///
/// Applies decorators in this order:
/// 1. Raw provider (from config)
/// 2. RetryProvider (per-provider retry with exponential backoff)
/// 3. SmartRoutingProvider (cheap/primary split when cheap model is configured)
/// 4. FailoverProvider (fallback model when primary fails)
/// 5. CircuitBreakerProvider (fast-fail when backend is degraded)
/// 6. CachedProvider (in-memory response cache)
///
/// Also returns a separate cheap LLM provider for heartbeat/evaluation (not
/// part of the chain — it's a standalone provider for explicitly cheap tasks).
///
/// This is the single source of truth for provider chain construction,
/// called by both `main.rs` and `app.rs`.
#[allow(clippy::type_complexity)]
pub fn build_provider_chain(
    config: &LlmConfig,
    session: Arc<SessionManager>,
) -> Result<
    (
        Arc<dyn LlmProvider>,
        Option<Arc<dyn LlmProvider>>,
        Option<Arc<RecordingLlm>>,
    ),
    LlmError,
> {
    let llm = create_llm_provider(config, session.clone())?;
    tracing::info!("LLM provider initialized: {}", llm.model_name());

    // 1. Retry
    let retry_config = RetryConfig {
        max_retries: config.nearai.max_retries,
    };
    let llm: Arc<dyn LlmProvider> = if retry_config.max_retries > 0 {
        tracing::info!(
            max_retries = retry_config.max_retries,
            "LLM retry wrapper enabled"
        );
        Arc::new(RetryProvider::new(llm, retry_config.clone()))
    } else {
        llm
    };

    // 2. Smart routing (cheap/primary split)
    let llm: Arc<dyn LlmProvider> = if let Some(ref cheap_model) = config.nearai.cheap_model {
        let mut cheap_config = config.nearai.clone();
        cheap_config.model = cheap_model.clone();
        let cheap = create_llm_provider_with_config(&cheap_config, session.clone())?;
        let cheap: Arc<dyn LlmProvider> = if retry_config.max_retries > 0 {
            Arc::new(RetryProvider::new(cheap, retry_config.clone()))
        } else {
            cheap
        };
        tracing::info!(
            primary = %llm.model_name(),
            cheap = %cheap.model_name(),
            "Smart routing enabled"
        );
        Arc::new(SmartRoutingProvider::new(
            llm,
            cheap,
            SmartRoutingConfig {
                cascade_enabled: config.nearai.smart_routing_cascade,
                ..SmartRoutingConfig::default()
            },
        ))
    } else {
        llm
    };

    // 3. Failover
    let llm: Arc<dyn LlmProvider> = if let Some(ref fallback_model) = config.nearai.fallback_model {
        if fallback_model == &config.nearai.model {
            tracing::warn!(
                "fallback_model is the same as primary model, failover may not be effective"
            );
        }
        let mut fallback_config = config.nearai.clone();
        fallback_config.model = fallback_model.clone();
        let fallback = create_llm_provider_with_config(&fallback_config, session.clone())?;
        tracing::info!(
            primary = %llm.model_name(),
            fallback = %fallback.model_name(),
            "LLM failover enabled"
        );
        let fallback: Arc<dyn LlmProvider> = if retry_config.max_retries > 0 {
            Arc::new(RetryProvider::new(fallback, retry_config.clone()))
        } else {
            fallback
        };
        let cooldown_config = CooldownConfig {
            cooldown_duration: std::time::Duration::from_secs(config.nearai.failover_cooldown_secs),
            failure_threshold: config.nearai.failover_cooldown_threshold,
        };
        Arc::new(FailoverProvider::with_cooldown(
            vec![llm, fallback],
            cooldown_config,
        )?)
    } else {
        llm
    };

    // 4. Circuit breaker
    let llm: Arc<dyn LlmProvider> = if let Some(threshold) = config.nearai.circuit_breaker_threshold
    {
        let cb_config = CircuitBreakerConfig {
            failure_threshold: threshold,
            recovery_timeout: std::time::Duration::from_secs(
                config.nearai.circuit_breaker_recovery_secs,
            ),
            ..CircuitBreakerConfig::default()
        };
        tracing::info!(
            threshold,
            recovery_secs = config.nearai.circuit_breaker_recovery_secs,
            "LLM circuit breaker enabled"
        );
        Arc::new(CircuitBreakerProvider::new(llm, cb_config))
    } else {
        llm
    };

    // 5. Response cache
    let llm: Arc<dyn LlmProvider> = if config.nearai.response_cache_enabled {
        let rc_config = ResponseCacheConfig {
            ttl: std::time::Duration::from_secs(config.nearai.response_cache_ttl_secs),
            max_entries: config.nearai.response_cache_max_entries,
        };
        tracing::info!(
            ttl_secs = config.nearai.response_cache_ttl_secs,
            max_entries = config.nearai.response_cache_max_entries,
            "LLM response cache enabled"
        );
        Arc::new(CachedProvider::new(llm, rc_config))
    } else {
        llm
    };

    // 6. Recording (trace capture for replay testing)
    let recording_handle = RecordingLlm::from_env(llm.clone());
    let llm: Arc<dyn LlmProvider> = if let Some(ref recorder) = recording_handle {
        Arc::clone(recorder) as Arc<dyn LlmProvider>
    } else {
        llm
    };

    // Standalone cheap LLM for heartbeat/evaluation (not part of the chain)
    let cheap_llm = create_cheap_llm_provider(config, session)?;
    if let Some(ref cheap) = cheap_llm {
        tracing::info!("Cheap LLM provider initialized: {}", cheap.model_name());
    }

    Ok((llm, cheap_llm, recording_handle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NearAiConfig;

    fn test_nearai_config() -> NearAiConfig {
        NearAiConfig {
            model: "test-model".to_string(),
            cheap_model: None,
            base_url: "https://api.near.ai".to_string(),
            api_key: None,
            fallback_model: None,
            max_retries: 3,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
            failover_cooldown_secs: 300,
            failover_cooldown_threshold: 3,
            smart_routing_cascade: true,
        }
    }

    fn test_llm_config() -> LlmConfig {
        LlmConfig {
            backend: "nearai".to_string(),
            session: SessionConfig::default(),
            nearai: test_nearai_config(),
            provider: None,
        }
    }

    #[test]
    fn test_create_cheap_llm_provider_returns_none_when_not_configured() {
        let config = test_llm_config();
        let session = Arc::new(SessionManager::new(SessionConfig::default()));

        let result = create_cheap_llm_provider(&config, session);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_create_cheap_llm_provider_creates_provider_when_configured() {
        let mut config = test_llm_config();
        config.nearai.cheap_model = Some("cheap-test-model".to_string());

        let session = Arc::new(SessionManager::new(SessionConfig::default()));
        let result = create_cheap_llm_provider(&config, session);

        assert!(result.is_ok());
        let provider = result.unwrap();
        assert!(provider.is_some());
        assert_eq!(provider.unwrap().model_name(), "cheap-test-model");
    }

    #[test]
    fn test_create_cheap_llm_provider_ignored_for_non_nearai_backend() {
        let mut config = test_llm_config();
        config.backend = "openai".to_string();
        config.nearai.cheap_model = Some("cheap-test-model".to_string());

        let session = Arc::new(SessionManager::new(SessionConfig::default()));
        let result = create_cheap_llm_provider(&config, session);

        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }
}
