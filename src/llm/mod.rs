//! LLM integration for the agent.
//!
//! Supports multiple backends:
//! - **NEAR AI** (default): Session token or API key auth via Chat Completions API
//! - **OpenAI**: Direct API access with your own key
//! - **Anthropic**: Direct API access with your own key
//! - **Ollama**: Local model inference
//! - **OpenAI-compatible**: Any endpoint that speaks the OpenAI API
//! - **AWS Bedrock**: Native Converse API via aws-sdk-bedrockruntime

mod anthropic_oauth;
#[cfg(feature = "bedrock")]
mod bedrock;
pub mod circuit_breaker;
pub(crate) mod codex_auth;
mod codex_chatgpt;
pub mod config;
pub mod costs;
pub mod error;
pub mod failover;
pub mod gemini_oauth;
mod github_copilot;
pub(crate) mod github_copilot_auth;
mod nearai_chat;
pub mod oauth_helpers;
pub mod openai_codex_provider;
pub mod openai_codex_session;
mod provider;
mod reasoning;
pub mod recording;
pub mod registry;
pub mod response_cache;
pub mod retry;
mod rig_adapter;
pub mod session;
pub mod smart_routing;
mod token_refreshing;
pub mod transcription;

#[cfg(test)]
mod codex_test_helpers;

pub mod image_models;
pub mod models;
pub mod reasoning_models;
pub mod vision_models;

pub use circuit_breaker::{CircuitBreakerConfig, CircuitBreakerProvider};
pub use config::{
    BedrockConfig, CacheRetention, LlmConfig, NearAiConfig, OAUTH_PLACEHOLDER, OpenAiCodexConfig,
    RegistryProviderConfig,
};
pub use error::LlmError;
pub use failover::{CooldownConfig, FailoverProvider};
pub use gemini_oauth::GeminiOauthProvider;
pub use nearai_chat::{DEFAULT_MODEL, ModelInfo, NearAiChatProvider, default_models};
pub use openai_codex_provider::OpenAiCodexProvider;
pub use openai_codex_session::{OpenAiCodexSession, OpenAiCodexSessionManager};
pub use provider::{
    ChatMessage, CompletionRequest, CompletionResponse, ContentPart, FinishReason, ImageUrl,
    LlmProvider, ModelMetadata, Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse,
    ToolDefinition, ToolResult, generate_tool_call_id,
};
pub use reasoning::{
    ActionPlan, Reasoning, ReasoningContext, RespondOutput, RespondResult, ResponseAnomaly,
    ResponseMetadata, SILENT_REPLY_TOKEN, TOOL_INTENT_NUDGE, TRUNCATED_TOOL_CALL_NOTICE,
    TokenUsage, ToolSelection, is_silent_reply, llm_signals_tool_intent,
};
pub use recording::RecordingLlm;
pub use registry::{ProviderDefinition, ProviderProtocol, ProviderRegistry};
pub use response_cache::{CachedProvider, ResponseCacheConfig};
pub use retry::{RetryConfig, RetryProvider};
pub use rig_adapter::RigAdapter;
pub use session::{SessionConfig, SessionManager, create_session_manager};
pub use smart_routing::{SmartRoutingConfig, SmartRoutingProvider, TaskComplexity};
pub use token_refreshing::TokenRefreshingProvider;

use std::sync::Arc;

use rig::client::CompletionClient;
use secrecy::ExposeSecret;

// LlmConfig, NearAiConfig, RegistryProviderConfig, and LlmError are
// re-exported via `pub use` above from config and error submodules.

/// Create an LLM provider based on configuration.
///
/// - NearAI backend: Uses session manager for authentication
/// - Registry providers: Looked up by protocol and constructed generically
pub async fn create_llm_provider(
    config: &LlmConfig,
    session: Arc<SessionManager>,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let timeout = config.request_timeout_secs;

    if config.backend == "nearai" || config.backend == "near_ai" || config.backend == "near" {
        return create_llm_provider_with_config(&config.nearai, session, timeout);
    }

    if config.backend == "gemini_oauth" || config.backend == "gemini-oauth" {
        return create_gemini_oauth_provider(config);
    }

    // Bedrock uses a native AWS SDK, not the rig-core registry
    if config.backend == "bedrock" {
        #[cfg(feature = "bedrock")]
        {
            return create_bedrock_provider(config).await;
        }
        #[cfg(not(feature = "bedrock"))]
        {
            return Err(LlmError::RequestFailed {
                provider: "bedrock".to_string(),
                reason: "Bedrock support not compiled. Rebuild with --features bedrock".to_string(),
            });
        }
    }

    if config.backend == "openai_codex" {
        return Err(LlmError::RequestFailed {
            provider: "openai_codex".to_string(),
            reason:
                "OpenAI Codex uses a dedicated factory path. Use build_provider_chain() instead of create_llm_provider()."
                    .to_string(),
        });
    }

    let reg_config = config
        .provider
        .as_ref()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: config.backend.clone(),
        })?;

    create_registry_provider(reg_config, timeout)
}

/// Create an LLM provider from a `NearAiConfig` directly.
///
/// This is useful when constructing additional providers for failover,
/// where only the model name differs from the primary config.
pub fn create_llm_provider_with_config(
    config: &NearAiConfig,
    session: Arc<SessionManager>,
    request_timeout_secs: u64,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let auth_mode = if config.api_key.is_some() {
        "API key"
    } else {
        "session token"
    };
    tracing::debug!(
        model = %config.model,
        base_url = %config.base_url,
        auth = auth_mode,
        timeout_secs = request_timeout_secs,
        "Using NEAR AI (Chat Completions API)"
    );
    Ok(Arc::new(NearAiChatProvider::new_with_timeout(
        config.clone(),
        session,
        request_timeout_secs,
    )?))
}

/// Create a provider from a registry-resolved config.
///
/// Dispatches on `RegistryProviderConfig::protocol` to build the appropriate
/// rig-core client. This single function replaces what used to be 5 separate
/// `create_*_provider` functions.
fn create_registry_provider(
    config: &RegistryProviderConfig,
    request_timeout_secs: u64,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    // Codex ChatGPT mode: use the Responses API provider
    if config.is_codex_chatgpt {
        return create_codex_chatgpt_from_registry(config, request_timeout_secs);
    }

    match config.protocol {
        ProviderProtocol::OpenAiCompletions => create_openai_compat_from_registry(config),
        ProviderProtocol::Anthropic => create_anthropic_from_registry(config),
        ProviderProtocol::Ollama => create_ollama_from_registry(config),
        ProviderProtocol::GithubCopilot => {
            let provider =
                github_copilot::GithubCopilotProvider::new(config, request_timeout_secs)?;
            tracing::debug!(
                provider = %config.provider_id,
                model = %config.model,
                base_url = %config.base_url,
                "Using GitHub Copilot provider (token exchange)"
            );
            Ok(Arc::new(provider))
        }
    }
}

fn create_codex_chatgpt_from_registry(
    config: &RegistryProviderConfig,
    request_timeout_secs: u64,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let api_key = config
        .api_key
        .as_ref()
        .cloned()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: "codex_chatgpt".to_string(),
        })?;

    tracing::info!(
        configured_model = %config.model,
        base_url = %config.base_url,
        "Using Codex ChatGPT provider (Responses API) — model detection deferred to first call"
    );

    let provider = codex_chatgpt::CodexChatGptProvider::with_lazy_model(
        &config.base_url,
        api_key,
        &config.model,
        config.refresh_token.clone(),
        config.auth_path.clone(),
        request_timeout_secs,
    );

    Ok(Arc::new(provider))
}

#[cfg(feature = "bedrock")]
async fn create_bedrock_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let br = config
        .bedrock
        .as_ref()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: "bedrock".to_string(),
        })?;

    let provider = bedrock::BedrockProvider::new(br).await?;
    tracing::debug!(
        "Using AWS Bedrock (Converse API, region: {}, model: {})",
        br.region,
        provider.active_model_name(),
    );

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

    tracing::debug!(
        provider = %config.provider_id,
        model = %config.model,
        base_url = %config.base_url,
        "Using OpenAI-compatible provider"
    );

    let adapter = RigAdapter::new(model, &config.model)
        .with_unsupported_params(config.unsupported_params.clone());
    Ok(Arc::new(adapter))
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
        .is_some_and(|k| k.expose_secret() == crate::llm::config::OAUTH_PLACEHOLDER);
    if config.oauth_token.is_some() && (config.api_key.is_none() || api_key_is_placeholder) {
        tracing::debug!(
            provider = %config.provider_id,
            model = %config.model,
            base_url = if config.base_url.is_empty() { "default" } else { &config.base_url },
            "Using Anthropic OAuth API"
        );
        let provider = anthropic_oauth::AnthropicOAuthProvider::new(config)?;
        return Ok(Arc::new(provider));
    }

    use crate::llm::config::CacheRetention;
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

    let cache_retention = config.cache_retention;

    let model = client.completion_model(&config.model);

    if cache_retention != CacheRetention::None {
        tracing::debug!(
            model = %config.model,
            retention = %cache_retention,
            "Anthropic automatic prompt caching enabled"
        );
    }

    tracing::debug!(
        provider = %config.provider_id,
        model = %config.model,
        base_url = if config.base_url.is_empty() { "default" } else { &config.base_url },
        "Using Anthropic provider"
    );

    Ok(Arc::new(
        RigAdapter::new(model, &config.model)
            .with_cache_retention(cache_retention)
            .with_unsupported_params(config.unsupported_params.clone()),
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

    tracing::debug!(
        provider = %config.provider_id,
        model = %config.model,
        base_url = %config.base_url,
        "Using Ollama provider"
    );

    let adapter = RigAdapter::new(model, &config.model)
        .with_unsupported_params(config.unsupported_params.clone());
    Ok(Arc::new(adapter))
}

/// Create an OpenAI Codex provider with OAuth authentication.
///
/// This is async because it needs to ensure authentication before
/// creating the provider (which requires a valid Bearer token).
///
/// Uses the Responses API (`chatgpt.com/backend-api/codex/responses`)
/// instead of the Chat Completions API, matching OpenClaw's approach.
async fn create_openai_codex_provider(
    config: &LlmConfig,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let codex = config
        .openai_codex
        .as_ref()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: "openai_codex".to_string(),
        })?;

    let session_mgr = Arc::new(OpenAiCodexSessionManager::new(codex.clone())?);
    session_mgr.ensure_authenticated().await?;

    let token = session_mgr.get_access_token().await?;

    let provider = Arc::new(OpenAiCodexProvider::new(
        &codex.model,
        &codex.api_base_url,
        token.expose_secret(),
        config.request_timeout_secs,
    )?);

    tracing::info!(
        "Using OpenAI Codex (Responses API, model: {}, base: {})",
        codex.model,
        codex.api_base_url,
    );

    Ok(Arc::new(TokenRefreshingProvider::new(
        provider,
        session_mgr,
    )))
}

/// Create a cheap/fast LLM provider for lightweight tasks (heartbeat, routing, evaluation).
///
/// Resolution order:
/// 1. `LLM_CHEAP_MODEL` (generic, works with any backend)
/// 2. `NEARAI_CHEAP_MODEL` (NearAI-only, backward compatibility)
///
/// Returns `None` if no cheap model is configured.
pub fn create_cheap_llm_provider(
    config: &LlmConfig,
    session: Arc<SessionManager>,
) -> Result<Option<Arc<dyn LlmProvider>>, LlmError> {
    let Some(cheap_model) = config.cheap_model_name() else {
        return Ok(None);
    };

    create_cheap_provider_for_backend(config, session, cheap_model)
}

/// Create a cheap provider for a specific backend.
///
/// Handles backend-specific provider construction:
/// - `nearai` — clones NearAiConfig, swaps model, uses `create_llm_provider_with_config`
/// - `bedrock` — returns error (smart routing not yet supported)
/// - All others — clones `RegistryProviderConfig`, swaps model, uses `create_registry_provider`
fn create_cheap_provider_for_backend(
    config: &LlmConfig,
    session: Arc<SessionManager>,
    cheap_model: &str,
) -> Result<Option<Arc<dyn LlmProvider>>, LlmError> {
    if config.backend == "nearai" {
        let mut cheap_config = config.nearai.clone();
        cheap_config.model = cheap_model.to_string();
        let provider =
            create_llm_provider_with_config(&cheap_config, session, config.request_timeout_secs)?;
        return Ok(Some(provider));
    }

    if config.backend == "bedrock" {
        return Err(LlmError::RequestFailed {
            provider: "bedrock".to_string(),
            reason: "Smart routing with cheap model is not supported for Bedrock yet".to_string(),
        });
    }

    if config.backend == "gemini_oauth" {
        let Some(ref gemini_config) = config.gemini_oauth else {
            return Err(LlmError::RequestFailed {
                provider: "gemini_oauth".to_string(),
                reason: "Gemini OAuth config not available for cheap model".to_string(),
            });
        };
        let mut cheap_gemini_config = gemini_config.clone();
        cheap_gemini_config.model = cheap_model.to_string();
        let provider = GeminiOauthProvider::new(cheap_gemini_config)?;
        return Ok(Some(Arc::new(provider)));
    }

    // Registry-based provider: clone config and swap model
    let reg_config = config.provider.as_ref().ok_or_else(|| LlmError::RequestFailed {
        provider: config.backend.clone(),
        reason: format!(
            "Cannot create cheap provider for backend '{}': no registry provider config available",
            config.backend
        ),
    })?;

    let mut cheap_reg_config = reg_config.clone();
    cheap_reg_config.model = cheap_model.to_string();
    let provider = create_registry_provider(&cheap_reg_config, config.request_timeout_secs)?;
    Ok(Some(provider))
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
pub async fn build_provider_chain(
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
    let llm: Arc<dyn LlmProvider> = if config.backend == "openai_codex" {
        create_openai_codex_provider(config).await?
    } else {
        create_llm_provider(config, session.clone()).await?
    };
    tracing::debug!("LLM provider initialized: {}", llm.model_name());

    // 1. Retry
    let retry_config = RetryConfig {
        max_retries: config.nearai.max_retries,
    };
    let llm: Arc<dyn LlmProvider> = if retry_config.max_retries > 0 {
        tracing::debug!(
            max_retries = retry_config.max_retries,
            "LLM retry wrapper enabled"
        );
        Arc::new(RetryProvider::new(llm, retry_config.clone()))
    } else {
        llm
    };

    // 2. Smart routing (cheap/primary split)
    let llm: Arc<dyn LlmProvider> = if let Some(cheap_model) = config.cheap_model_name() {
        let cheap = create_cheap_provider_for_backend(config, session.clone(), cheap_model)?
            .ok_or_else(|| LlmError::RequestFailed {
                provider: config.backend.clone(),
                reason: format!(
                    "Failed to create cheap provider for model '{cheap_model}' on backend '{}'",
                    config.backend
                ),
            })?;
        let cheap: Arc<dyn LlmProvider> = if retry_config.max_retries > 0 {
            Arc::new(RetryProvider::new(cheap, retry_config.clone()))
        } else {
            cheap
        };
        tracing::debug!(
            primary = %llm.model_name(),
            cheap = %cheap.model_name(),
            "Smart routing enabled"
        );
        Arc::new(SmartRoutingProvider::new(
            llm,
            cheap,
            SmartRoutingConfig {
                cascade_enabled: config.smart_routing_cascade,
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
        let fallback = create_llm_provider_with_config(
            &fallback_config,
            session.clone(),
            config.request_timeout_secs,
        )?;
        tracing::debug!(
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
        tracing::debug!(
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
        tracing::debug!(
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
        tracing::debug!("Cheap LLM provider initialized: {}", cheap.model_name());
    }

    Ok((llm, cheap_llm, recording_handle))
}

pub fn create_gemini_oauth_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let gemini_config = config
        .gemini_oauth
        .clone()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: "gemini_oauth".to_string(),
        })?;
    let provider = gemini_oauth::GeminiOauthProvider::new(gemini_config)?;
    Ok(Arc::new(provider))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::config::NearAiConfig;

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
            bedrock: None,
            gemini_oauth: None,
            request_timeout_secs: 120,
            cheap_model: None,
            smart_routing_cascade: true,
            openai_codex: None,
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
    fn test_create_cheap_llm_provider_creates_provider_with_nearai_cheap_model() {
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
    fn test_create_cheap_llm_provider_generic_overrides_nearai() {
        let mut config = test_llm_config();
        config.nearai.cheap_model = Some("nearai-cheap".to_string());
        config.cheap_model = Some("generic-cheap".to_string());

        let session = Arc::new(SessionManager::new(SessionConfig::default()));
        let result = create_cheap_llm_provider(&config, session);

        assert!(result.is_ok());
        let provider = result.unwrap();
        assert!(provider.is_some());
        assert_eq!(
            provider.unwrap().model_name(),
            "generic-cheap",
            "LLM_CHEAP_MODEL should take priority over NEARAI_CHEAP_MODEL"
        );
    }

    #[test]
    fn test_create_cheap_llm_provider_nearai_cheap_ignored_for_non_nearai_backend() {
        let mut config = test_llm_config();
        config.backend = "openai".to_string();
        config.nearai.cheap_model = Some("cheap-test-model".to_string());

        let session = Arc::new(SessionManager::new(SessionConfig::default()));
        let result = create_cheap_llm_provider(&config, session);

        assert!(result.is_ok());
        assert!(
            result.unwrap().is_none(),
            "NEARAI_CHEAP_MODEL should be ignored when backend is not nearai"
        );
    }

    #[test]
    fn test_create_cheap_llm_provider_bedrock_returns_error() {
        let mut config = test_llm_config();
        config.backend = "bedrock".to_string();
        config.cheap_model = Some("cheap-model".to_string());

        let session = Arc::new(SessionManager::new(SessionConfig::default()));
        let result = create_cheap_llm_provider(&config, session);

        assert!(
            result.is_err(),
            "Bedrock should return an error for cheap model"
        );
    }

    #[test]
    fn test_create_cheap_llm_provider_gemini_oauth_creates_provider() {
        let mut config = test_llm_config();
        config.backend = "gemini_oauth".to_string();
        config.cheap_model = Some("gemini-2.5-flash-lite".to_string());
        config.gemini_oauth = Some(crate::config::GeminiOauthConfig {
            model: "gemini-2.5-pro".to_string(),
            credentials_path: std::path::PathBuf::from("/tmp/nonexistent-creds.json"),
        });

        let session = Arc::new(SessionManager::new(SessionConfig::default()));
        let result = create_cheap_llm_provider(&config, session);

        // Should succeed and return a provider (credentials validation is deferred
        // until the first LLM call, not at construction time).
        let provider = result.expect("gemini_oauth cheap provider should succeed");
        assert!(provider.is_some(), "Should return Some(provider)");
        assert_eq!(
            provider.unwrap().model_name(),
            "gemini-2.5-flash-lite",
            "Cheap provider should use the overridden model name"
        );
    }

    #[test]
    fn test_cheap_model_name_resolution() {
        // Generic takes priority
        let mut config = test_llm_config();
        config.cheap_model = Some("generic".to_string());
        config.nearai.cheap_model = Some("nearai".to_string());
        assert_eq!(config.cheap_model_name(), Some("generic"));

        // NearAI fallback when backend is nearai
        let mut config = test_llm_config();
        config.nearai.cheap_model = Some("nearai".to_string());
        assert_eq!(config.cheap_model_name(), Some("nearai"));

        // NearAI ignored for non-nearai backend
        let mut config = test_llm_config();
        config.backend = "openai".to_string();
        config.nearai.cheap_model = Some("nearai".to_string());
        assert_eq!(config.cheap_model_name(), None);

        // None when nothing configured
        let config = test_llm_config();
        assert_eq!(config.cheap_model_name(), None);
    }
}
