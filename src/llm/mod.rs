//! LLM integration for the agent.
//!
//! Supports two API modes:
//! - **Responses API** (chat-api): Session-based auth, uses `/v1/responses` endpoint
//! - **Chat Completions API** (cloud-api): API key auth, uses `/v1/chat/completions` endpoint

mod nearai;
mod nearai_chat;
mod provider;
mod reasoning;
pub mod session;

pub use nearai::{ModelInfo, NearAiProvider};
pub use nearai_chat::NearAiChatProvider;
pub use provider::{
    ChatMessage, CompletionRequest, CompletionResponse, LlmProvider, Role, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse, ToolDefinition, ToolResult,
};
pub use reasoning::{ActionPlan, Reasoning, ReasoningContext, RespondResult, ToolSelection};
pub use session::{SessionConfig, SessionManager, create_session_manager};

use std::sync::Arc;

use crate::config::{LlmConfig, NearAiApiMode};
use crate::error::LlmError;

/// Create an LLM provider based on configuration.
///
/// - For `Responses` mode: Requires a session manager for authentication
/// - For `ChatCompletions` mode: Uses API key from config (session not needed)
pub fn create_llm_provider(
    config: &LlmConfig,
    session: Arc<SessionManager>,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    match config.nearai.api_mode {
        NearAiApiMode::Responses => {
            tracing::info!("Using Responses API (chat-api) with session auth");
            Ok(Arc::new(NearAiProvider::new(
                config.nearai.clone(),
                session,
            )))
        }
        NearAiApiMode::ChatCompletions => {
            tracing::info!("Using Chat Completions API (cloud-api) with API key auth");
            Ok(Arc::new(NearAiChatProvider::new(config.nearai.clone())?))
        }
    }
}

/// Create a cheap/fast LLM provider for lightweight tasks (heartbeat, routing, evaluation).
///
/// Uses `NEARAI_CHEAP_MODEL` if set, otherwise falls back to the main provider.
pub fn create_cheap_llm_provider(
    config: &LlmConfig,
    session: Arc<SessionManager>,
) -> Result<Option<Arc<dyn LlmProvider>>, LlmError> {
    let Some(ref cheap_model) = config.nearai.cheap_model else {
        return Ok(None);
    };

    let mut cheap_config = config.nearai.clone();
    cheap_config.model = cheap_model.clone();

    tracing::info!("Cheap LLM provider: {}", cheap_model);

    match cheap_config.api_mode {
        NearAiApiMode::Responses => Ok(Some(Arc::new(NearAiProvider::new(
            cheap_config,
            session,
        )))),
        NearAiApiMode::ChatCompletions => {
            Ok(Some(Arc::new(NearAiChatProvider::new(cheap_config)?)))
        }
    }
}
