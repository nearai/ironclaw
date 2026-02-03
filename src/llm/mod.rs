//! LLM integration for the agent.
//!
//! Provides a unified interface to different LLM providers (OpenAI, Anthropic)
//! and implements reasoning capabilities for planning, tool selection, and evaluation.

mod anthropic;
mod openai;
mod provider;
mod reasoning;

pub use anthropic::AnthropicProvider;
pub use openai::OpenAiProvider;
pub use provider::{
    ChatMessage, CompletionRequest, CompletionResponse, LlmProvider, Role, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse, ToolDefinition, ToolResult,
};
pub use reasoning::{ActionPlan, Reasoning, ReasoningContext, ToolSelection};

use std::sync::Arc;

use crate::config::{LlmConfig, LlmProvider as LlmProviderType};
use crate::error::LlmError;

/// Create an LLM provider based on configuration.
pub fn create_llm_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    match config.provider {
        LlmProviderType::OpenAi => {
            let openai_config = config.openai.as_ref().ok_or_else(|| LlmError::AuthFailed {
                provider: "openai".to_string(),
            })?;
            Ok(Arc::new(OpenAiProvider::new(openai_config.clone())))
        }
        LlmProviderType::Anthropic => {
            let anthropic_config =
                config
                    .anthropic
                    .as_ref()
                    .ok_or_else(|| LlmError::AuthFailed {
                        provider: "anthropic".to_string(),
                    })?;
            Ok(Arc::new(AnthropicProvider::new(anthropic_config.clone())))
        }
    }
}
