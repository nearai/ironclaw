//! LLM backend trait.
//!
//! The engine's abstraction over language model providers. Deliberately
//! simpler than the main crate's `LlmProvider` — the engine only needs
//! to make completion calls. Cost tracking, caching, retry, and circuit
//! breaking are host concerns handled by the bridge adapter.

use std::collections::HashMap;

use crate::types::capability::ActionDef;
use crate::types::error::EngineError;
use crate::types::message::ThreadMessage;
use crate::types::step::{LlmResponse, TokenUsage};

/// Configuration for a single LLM call.
#[derive(Debug, Clone, Default)]
pub struct LlmCallConfig {
    /// Maximum tokens to generate.
    pub max_tokens: Option<u32>,
    /// Sampling temperature.
    pub temperature: Option<f32>,
    /// When true, the LLM should not return action calls.
    pub force_text: bool,
    /// Depth in the recursive call tree (0 = root, 1+ = sub-call).
    /// Implementations can use this to route to cheaper models for sub-calls.
    pub depth: u32,
    /// Opaque metadata forwarded to the LLM provider.
    pub metadata: HashMap<String, String>,
}

/// Output from a single LLM call.
#[derive(Debug, Clone)]
pub struct LlmOutput {
    pub response: LlmResponse,
    pub usage: TokenUsage,
}

/// Abstraction over language model providers.
///
/// The main crate implements this by wrapping its `LlmProvider` trait,
/// converting between `ThreadMessage` and `ChatMessage`.
#[async_trait::async_trait]
pub trait LlmBackend: Send + Sync {
    /// Call the LLM with conversation messages and available action definitions.
    ///
    /// Returns either a text response or a set of action calls.
    async fn complete(
        &self,
        messages: &[ThreadMessage],
        actions: &[ActionDef],
        config: &LlmCallConfig,
    ) -> Result<LlmOutput, EngineError>;

    /// The model identifier (e.g. "gpt-4", "claude-opus-4-20250514").
    fn model_name(&self) -> &str;
}
