//! LLM integration for the agent.
//!
//! Supports multiple backends:
//! - **NEAR AI** (default): Session-based or API key auth via NEAR AI proxy
//! - **OpenRouter**: Broad model access via OpenAI-compatible API
//! - **OpenAI**: Direct API access with your own key
//! - **Anthropic**: Direct API access with your own key
//! - **Ollama**: Local model inference
//! - **OpenAI-compatible**: Any endpoint that speaks the OpenAI API

mod costs;
mod nearai;
mod nearai_chat;
mod provider;
mod reasoning;
mod rig_adapter;
pub mod session;

pub use nearai::{ModelInfo, NearAiProvider};
pub use nearai_chat::NearAiChatProvider;
pub use provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ModelMetadata,
    Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse, ToolDefinition, ToolResult,
};
pub use reasoning::{ActionPlan, Reasoning, ReasoningContext, RespondResult, ToolSelection};
pub use rig_adapter::RigAdapter;
pub use session::{SessionConfig, SessionManager, create_session_manager};

use std::sync::Arc;

use rig::client::CompletionClient;
use secrecy::ExposeSecret;

use crate::config::{LlmBackend, LlmConfig, NearAiApiMode};
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
        LlmBackend::OpenRouter => create_openrouter_provider(config),
        LlmBackend::NearAi => create_nearai_provider(config, session),
        LlmBackend::OpenAi => create_openai_provider(config),
        LlmBackend::Anthropic => create_anthropic_provider(config),
        LlmBackend::Ollama => create_ollama_provider(config),
        LlmBackend::OpenAiCompatible => create_openai_compatible_provider(config),
    }
}

fn create_openrouter_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let or = config
        .openrouter
        .as_ref()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: "openrouter".to_string(),
        })?;

    use rig::providers::openai;

    let client: openai::Client = openai::Client::builder()
        .base_url("https://openrouter.ai/api/v1")
        .api_key(or.api_key.expose_secret())
        .build()
        .map_err(|e| LlmError::RequestFailed {
            provider: "openrouter".to_string(),
            reason: format!("Failed to create OpenRouter client: {}", e),
        })?;

    let model = client.completion_model(&or.model);
    tracing::info!("Using OpenRouter API (model: {})", or.model);
    Ok(Arc::new(RigAdapter::new(model, &or.model)))
}

fn create_nearai_provider(
    config: &LlmConfig,
    session: Arc<SessionManager>,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    match config.nearai.api_mode {
        NearAiApiMode::Responses => {
            tracing::info!("Using NEAR AI Responses API (chat-api) with session auth");
            Ok(Arc::new(NearAiProvider::new(
                config.nearai.clone(),
                session,
            )))
        }
        NearAiApiMode::ChatCompletions => {
            tracing::info!("Using NEAR AI Chat Completions API (cloud-api) with API key auth");
            Ok(Arc::new(NearAiChatProvider::new(config.nearai.clone())?))
        }
    }
}

fn create_openai_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let oai = config.openai.as_ref().ok_or_else(|| LlmError::AuthFailed {
        provider: "openai".to_string(),
    })?;

    use rig::providers::openai;

    let client: openai::Client =
        openai::Client::new(oai.api_key.expose_secret()).map_err(|e| LlmError::RequestFailed {
            provider: "openai".to_string(),
            reason: format!("Failed to create OpenAI client: {}", e),
        })?;

    let model = client.completion_model(&oai.model);
    tracing::info!("Using OpenAI direct API (model: {})", oai.model);
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

    let client: anthropic::Client =
        anthropic::Client::new(anth.api_key.expose_secret()).map_err(|e| {
            LlmError::RequestFailed {
                provider: "anthropic".to_string(),
                reason: format!("Failed to create Anthropic client: {}", e),
            }
        })?;

    let model = client.completion_model(&anth.model);
    tracing::info!("Using Anthropic direct API (model: {})", anth.model);
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

fn create_openai_compatible_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let compat = config
        .openai_compatible
        .as_ref()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: "openai_compatible".to_string(),
        })?;

    use rig::providers::openai;

    let api_key = compat
        .api_key
        .as_ref()
        .map(|k| k.expose_secret().to_string())
        .unwrap_or_else(|| "no-key".to_string());

    let client: openai::Client = openai::Client::builder()
        .base_url(&compat.base_url)
        .api_key(api_key)
        .build()
        .map_err(|e| LlmError::RequestFailed {
            provider: "openai_compatible".to_string(),
            reason: format!("Failed to create OpenAI-compatible client: {}", e),
        })?;

    let model = client.completion_model(&compat.model);
    tracing::info!(
        "Using OpenAI-compatible endpoint (base_url: {}, model: {})",
        compat.base_url,
        compat.model
    );
    Ok(Arc::new(RigAdapter::new(model, &compat.model)))
}
