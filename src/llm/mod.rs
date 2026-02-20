//! LLM integration for the agent.
//!
//! Supports multiple backends:
//! - **NEAR AI** (default): Session-based or API key auth via NEAR AI proxy
//! - **OpenAI**: Direct API access with your own key
//! - **Anthropic**: Direct API access with your own key
//! - **Ollama**: Local model inference
//! - **OpenAI-compatible**: Any endpoint that speaks the OpenAI API

pub mod circuit_breaker;
pub mod costs;
pub mod failover;
mod nearai;
mod nearai_chat;
mod provider;
mod reasoning;
pub mod response_cache;
pub mod retry;
mod rig_adapter;
pub mod session;

pub use circuit_breaker::{CircuitBreakerConfig, CircuitBreakerProvider};
pub use failover::{CooldownConfig, FailoverProvider};
pub use nearai::{ModelInfo, NearAiProvider};
pub use nearai_chat::NearAiChatProvider;
pub use provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ModelMetadata,
    Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse, ToolDefinition, ToolResult,
};
pub use reasoning::{
    ActionPlan, Reasoning, ReasoningContext, RespondOutput, RespondResult, TokenUsage,
    ToolSelection,
};
pub use response_cache::{CachedProvider, ResponseCacheConfig};
pub use retry::{RetryConfig, RetryProvider};
pub use rig_adapter::RigAdapter;
pub use session::{SessionConfig, SessionManager, create_session_manager};

use std::sync::Arc;

use rig::client::CompletionClient;
use secrecy::ExposeSecret;

use crate::config::{LlmBackend, LlmConfig, NearAiApiMode, NearAiConfig};
use crate::error::LlmError;

/// Create an LLM provider based on configuration.
///
/// - `NearAi` backend: Uses session manager for authentication (Responses API)
///   or API key (Chat Completions API)
/// - Other backends: Use rig-core adapter with provider-specific clients
pub fn create_llm_provider(
    config: &LlmConfig,
    session: Arc<SessionManager>,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    match config.backend {
        LlmBackend::NearAi => create_llm_provider_with_config(&config.nearai, session),
        LlmBackend::OpenAi => create_openai_provider(config),
        LlmBackend::Anthropic => create_anthropic_provider(config),
        LlmBackend::Ollama => create_ollama_provider(config),
        LlmBackend::OpenAiCompatible => create_openai_compatible_provider(config),
        LlmBackend::Tinfoil => create_tinfoil_provider(config),
    }
}

/// Create an LLM provider from a `NearAiConfig` directly.
///
/// This is useful when constructing additional providers for failover,
/// where only the model name differs from the primary config.
pub fn create_llm_provider_with_config(
    config: &NearAiConfig,
    session: Arc<SessionManager>,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    match config.api_mode {
        NearAiApiMode::Responses => {
            tracing::info!(
                model = %config.model,
                "Using Responses API (chat-api) with session auth"
            );
            Ok(Arc::new(NearAiProvider::new(config.clone(), session)?))
        }
        NearAiApiMode::ChatCompletions => {
            tracing::info!(
                model = %config.model,
                "Using Chat Completions API (cloud-api) with API key auth"
            );
            Ok(Arc::new(NearAiChatProvider::new(config.clone())?))
        }
    }
}

fn create_openai_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let oai = config.openai.as_ref().ok_or_else(|| LlmError::AuthFailed {
        provider: "openai".to_string(),
    })?;

    use rig::providers::openai;

    // Use CompletionsClient (Chat Completions API) instead of the default Client
    // (Responses API). The Responses API path in rig-core panics when tool results
    // are sent back because ironclaw doesn't thread `call_id` through its ToolCall
    // type. The Chat Completions API works correctly with the existing code.
    let client: openai::CompletionsClient = if let Some(ref base_url) = oai.base_url {
        tracing::info!(
            "Using OpenAI direct API (chat completions, model: {}, base_url: {})",
            oai.model,
            base_url,
        );
        openai::Client::builder()
            .base_url(base_url)
            .api_key(oai.api_key.expose_secret())
            .build()
    } else {
        tracing::info!(
            "Using OpenAI direct API (chat completions, model: {}, base_url: default)",
            oai.model,
        );
        openai::Client::new(oai.api_key.expose_secret())
    }
    .map_err(|e| LlmError::RequestFailed {
        provider: "openai".to_string(),
        reason: format!("Failed to create OpenAI client: {}", e),
    })?
    .completions_api();

    let model = client.completion_model(&oai.model);
    Ok(Arc::new(RigAdapter::new(model, &oai.model)))
}

fn create_anthropic_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let anth = config
        .anthropic
        .as_ref()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: "anthropic".to_string(),
        })?;

    use rig::providers::anthropic;

    let client: anthropic::Client = if let Some(ref base_url) = anth.base_url {
        anthropic::Client::builder()
            .api_key(anth.api_key.expose_secret())
            .base_url(base_url)
            .build()
    } else {
        anthropic::Client::new(anth.api_key.expose_secret())
    }
    .map_err(|e| LlmError::RequestFailed {
        provider: "anthropic".to_string(),
        reason: format!("Failed to create Anthropic client: {}", e),
    })?;

    let model = client.completion_model(&anth.model);
    tracing::info!(
        "Using Anthropic direct API (model: {}, base_url: {})",
        anth.model,
        anth.base_url.as_deref().unwrap_or("default"),
    );
    Ok(Arc::new(RigAdapter::new(model, &anth.model)))
}

fn create_ollama_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let oll = config.ollama.as_ref().ok_or_else(|| LlmError::AuthFailed {
        provider: "ollama".to_string(),
    })?;

    use rig::client::Nothing;
    use rig::providers::ollama;

    let client: ollama::Client = ollama::Client::builder()
        .base_url(&oll.base_url)
        .api_key(Nothing)
        .build()
        .map_err(|e| LlmError::RequestFailed {
            provider: "ollama".to_string(),
            reason: format!("Failed to create Ollama client: {}", e),
        })?;

    let model = client.completion_model(&oll.model);
    tracing::info!(
        "Using Ollama (base_url: {}, model: {})",
        oll.base_url,
        oll.model
    );
    Ok(Arc::new(RigAdapter::new(model, &oll.model)))
}

const TINFOIL_BASE_URL: &str = "https://inference.tinfoil.sh/v1";

fn create_tinfoil_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let tf = config
        .tinfoil
        .as_ref()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: "tinfoil".to_string(),
        })?;

    use rig::providers::openai;

    let client: openai::Client = openai::Client::builder()
        .base_url(TINFOIL_BASE_URL)
        .api_key(tf.api_key.expose_secret())
        .build()
        .map_err(|e| LlmError::RequestFailed {
            provider: "tinfoil".to_string(),
            reason: format!("Failed to create Tinfoil client: {}", e),
        })?;

    // Tinfoil currently only supports the Chat Completions API and not the newer Responses API,
    // so we must explicitly select the completions API here (unlike other OpenAI-compatible providers).
    let client = client.completions_api();
    let model = client.completion_model(&tf.model);
    tracing::info!("Using Tinfoil private inference (model: {})", tf.model);
    Ok(Arc::new(RigAdapter::new(model, &tf.model)))
}

fn create_openai_compatible_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let compat = config
        .openai_compatible
        .as_ref()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: "openai_compatible".to_string(),
        })?;

    use rig::providers::openai;

    let client: openai::CompletionsClient = openai::Client::builder()
        .base_url(&compat.base_url)
        .api_key(
            compat
                .api_key
                .as_ref()
                .map(|k| k.expose_secret().to_string())
                .unwrap_or_else(|| "no-key".to_string()),
        )
        .build()
        .map_err(|e| LlmError::RequestFailed {
            provider: "openai_compatible".to_string(),
            reason: format!("Failed to create OpenAI-compatible client: {}", e),
        })?
        .completions_api();

    let model = client.completion_model(&compat.model);
    tracing::info!(
        "Using OpenAI-compatible endpoint (chat completions, base_url: {}, model: {})",
        compat.base_url,
        compat.model
    );
    Ok(Arc::new(RigAdapter::new(model, &compat.model)))
}

/// Create a cheap/fast LLM provider for lightweight tasks (heartbeat, routing, evaluation).
///
/// Uses `NEARAI_CHEAP_MODEL` if set, otherwise falls back to the main provider.
/// Currently only supports NEAR AI backends (Responses and ChatCompletions modes).
pub fn create_cheap_llm_provider(
    config: &LlmConfig,
    session: Arc<SessionManager>,
) -> Result<Option<Arc<dyn LlmProvider>>, LlmError> {
    let Some(ref cheap_model) = config.nearai.cheap_model else {
        return Ok(None);
    };

    if config.backend != LlmBackend::NearAi {
        tracing::warn!(
            "NEARAI_CHEAP_MODEL is set but LLM_BACKEND is {:?}, not NearAi. \
             Cheap model setting will be ignored.",
            config.backend
        );
        return Ok(None);
    }

    let mut cheap_config = config.nearai.clone();
    cheap_config.model = cheap_model.clone();

    tracing::info!("Cheap LLM provider: {}", cheap_model);

    match cheap_config.api_mode {
        NearAiApiMode::Responses => Ok(Some(Arc::new(NearAiProvider::new(cheap_config, session)?))),
        NearAiApiMode::ChatCompletions => {
            Ok(Some(Arc::new(NearAiChatProvider::new(cheap_config)?)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LlmBackend, NearAiApiMode, NearAiConfig};
    use std::path::PathBuf;

    fn test_nearai_config() -> NearAiConfig {
        NearAiConfig {
            model: "test-model".to_string(),
            cheap_model: None,
            base_url: "https://api.near.ai".to_string(),
            auth_base_url: "https://private.near.ai".to_string(),
            session_path: PathBuf::from("/tmp/test-session.json"),
            api_mode: NearAiApiMode::Responses,
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
        }
    }

    fn test_llm_config() -> LlmConfig {
        LlmConfig {
            backend: LlmBackend::NearAi,
            nearai: test_nearai_config(),
            openai: None,
            anthropic: None,
            ollama: None,
            openai_compatible: None,
            tinfoil: None,
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
        config.backend = LlmBackend::OpenAi;
        config.nearai.cheap_model = Some("cheap-test-model".to_string());

        let session = Arc::new(SessionManager::new(SessionConfig::default()));
        let result = create_cheap_llm_provider(&config, session);

        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }
}
