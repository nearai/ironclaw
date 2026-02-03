//! LLM integration for the agent.
//!
//! Uses the NEAR AI chat-api as the unified LLM provider.

mod nearai;
mod provider;
mod reasoning;
pub mod session;

pub use nearai::{ModelInfo, NearAiProvider};
pub use provider::{
    ChatMessage, CompletionRequest, CompletionResponse, LlmProvider, Role, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse, ToolDefinition, ToolResult,
};
pub use reasoning::{ActionPlan, Reasoning, ReasoningContext, RespondResult, ToolSelection};
pub use session::{SessionConfig, SessionManager, create_session_manager};

use std::sync::Arc;

use crate::config::LlmConfig;
use crate::error::LlmError;

/// Create an LLM provider based on configuration.
///
/// Requires a session manager for authentication. Use `create_session_manager`
/// to create one from the config.
pub fn create_llm_provider(
    config: &LlmConfig,
    session: Arc<SessionManager>,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    Ok(Arc::new(NearAiProvider::new(
        config.nearai.clone(),
        session,
    )))
}
