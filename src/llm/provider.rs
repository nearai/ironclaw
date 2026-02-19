//! LLM provider trait and types.

use async_trait::async_trait;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::error::LlmError;

/// Role in a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// Tool call ID if this is a tool result message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Name of the tool for tool results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Tool calls made by the assistant (OpenAI protocol requires these
    /// to appear on the assistant message preceding tool result messages).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

impl ChatMessage {
    /// Create a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_call_id: None,
            name: None,
            tool_calls: None,
        }
    }

    /// Create a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_call_id: None,
            name: None,
            tool_calls: None,
        }
    }

    /// Create an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_call_id: None,
            name: None,
            tool_calls: None,
        }
    }

    /// Create an assistant message that includes tool calls.
    ///
    /// Per the OpenAI protocol, an assistant message with tool_calls must
    /// precede the corresponding tool result messages in the conversation.
    pub fn assistant_with_tool_calls(content: Option<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.unwrap_or_default(),
            tool_call_id: None,
            name: None,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
        }
    }

    /// Create a tool result message.
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_call_id: Some(tool_call_id.into()),
            name: Some(name.into()),
            tool_calls: None,
        }
    }
}

/// Request for a chat completion.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub messages: Vec<ChatMessage>,
    /// Optional per-request model override.
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stop_sequences: Option<Vec<String>>,
    /// Opaque metadata passed through to the provider (e.g. thread_id for chaining).
    pub metadata: std::collections::HashMap<String, String>,
}

impl CompletionRequest {
    /// Create a new completion request.
    pub fn new(messages: Vec<ChatMessage>) -> Self {
        Self {
            messages,
            model: None,
            max_tokens: None,
            temperature: None,
            stop_sequences: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Set model override.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set max tokens.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Set temperature.
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }
}

/// Response from a chat completion.
#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub content: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub finish_reason: FinishReason,
    /// Provider-specific response ID (e.g. for NEAR AI response chaining).
    pub response_id: Option<String>,
}

/// Why the completion finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    Length,
    ToolUse,
    ContentFilter,
    Unknown,
}

/// Definition of a tool for the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// A tool call requested by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Result of a tool execution to send back to the LLM.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub name: String,
    pub content: String,
    pub is_error: bool,
}

/// Request for a completion with tool use.
#[derive(Debug, Clone)]
pub struct ToolCompletionRequest {
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDefinition>,
    /// Optional per-request model override.
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    /// How to handle tool use: "auto", "required", or "none".
    pub tool_choice: Option<String>,
    /// Opaque metadata passed through to the provider (e.g. thread_id for chaining).
    pub metadata: std::collections::HashMap<String, String>,
}

impl ToolCompletionRequest {
    /// Create a new tool completion request.
    pub fn new(messages: Vec<ChatMessage>, tools: Vec<ToolDefinition>) -> Self {
        Self {
            messages,
            tools,
            model: None,
            max_tokens: None,
            temperature: None,
            tool_choice: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Set model override.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set max tokens.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Set temperature.
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Set tool choice mode.
    pub fn with_tool_choice(mut self, choice: impl Into<String>) -> Self {
        self.tool_choice = Some(choice.into());
        self
    }
}

/// Response from a completion with potential tool calls.
#[derive(Debug, Clone)]
pub struct ToolCompletionResponse {
    /// Text content (may be empty if tool calls are present).
    pub content: Option<String>,
    /// Tool calls requested by the model.
    pub tool_calls: Vec<ToolCall>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub finish_reason: FinishReason,
    /// Provider-specific response ID (e.g. for NEAR AI response chaining).
    pub response_id: Option<String>,
}

/// Metadata about a model returned by the provider's API.
#[derive(Debug, Clone)]
pub struct ModelMetadata {
    pub id: String,
    /// Total context window size in tokens.
    pub context_length: Option<u32>,
}

/// Trait for LLM providers.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Get the model name.
    fn model_name(&self) -> &str;

    /// Get cost per token (input, output).
    fn cost_per_token(&self) -> (Decimal, Decimal);

    /// Complete a chat conversation.
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError>;

    /// Complete with tool use support.
    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError>;

    /// List available models from the provider.
    /// Default implementation returns empty list.
    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        Ok(Vec::new())
    }

    /// Fetch metadata for the current model (context length, etc.).
    /// Default returns the model name with no size info.
    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        Ok(ModelMetadata {
            id: self.model_name().to_string(),
            context_length: None,
        })
    }

    /// Resolve which model should be reported for a given request.
    ///
    /// Providers that ignore per-request model overrides should override this
    /// and return `active_model_name()`.
    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        requested_model
            .map(std::borrow::ToOwned::to_owned)
            .unwrap_or_else(|| self.active_model_name())
    }

    /// Get the currently active model name.
    ///
    /// May differ from `model_name()` if the model was switched at runtime
    /// via `set_model()`. Default returns `model_name()`.
    fn active_model_name(&self) -> String {
        self.model_name().to_string()
    }

    /// Switch the active model at runtime. Not all providers support this.
    fn set_model(&self, _model: &str) -> Result<(), LlmError> {
        Err(LlmError::RequestFailed {
            provider: "unknown".to_string(),
            reason: "Runtime model switching not supported by this provider".to_string(),
        })
    }

    /// Seed a response chain for a thread (e.g. restoring from DB).
    ///
    /// Providers that support response chaining (e.g. NEAR AI `previous_response_id`)
    /// store this so subsequent calls send only delta messages.
    fn seed_response_chain(&self, _thread_id: &str, _response_id: String) {}

    /// Get the last response chain ID for a thread.
    ///
    /// Returns `None` if the provider doesn't support chaining or has no
    /// stored state for this thread.
    fn get_response_chain_id(&self, _thread_id: &str) -> Option<String> {
        None
    }

    /// Calculate cost for a completion.
    fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> Decimal {
        let (input_cost, output_cost) = self.cost_per_token();
        input_cost * Decimal::from(input_tokens) + output_cost * Decimal::from(output_tokens)
    }
}

/// Sanitize a message list to ensure tool_use / tool_result integrity.
///
/// LLM APIs (especially Anthropic) require every tool_result to reference a
/// tool_call_id that exists in an immediately preceding assistant message's
/// tool_calls. Orphaned tool_results cause HTTP 400 errors.
///
/// This function:
/// 1. Tracks all tool_call_ids emitted by assistant messages.
/// 2. Rewrites orphaned tool_result messages (whose tool_call_id has no
///    matching assistant tool_call) as user messages so the content is
///    preserved without violating the protocol.
///
/// Call this before sending messages to any LLM provider.
pub fn sanitize_tool_messages(messages: &mut [ChatMessage]) {
    use std::collections::HashSet;

    // Collect all tool_call_ids from assistant messages with tool_calls.
    let mut known_ids: HashSet<String> = HashSet::new();
    for msg in messages.iter() {
        if msg.role == Role::Assistant
            && let Some(ref calls) = msg.tool_calls
        {
            for tc in calls {
                known_ids.insert(tc.id.clone());
            }
        }
    }

    // Rewrite orphaned tool_result messages as user messages.
    for msg in messages.iter_mut() {
        if msg.role != Role::Tool {
            continue;
        }
        let is_orphaned = match &msg.tool_call_id {
            Some(id) => !known_ids.contains(id),
            None => true,
        };
        if is_orphaned {
            let tool_name = msg.name.as_deref().unwrap_or("unknown");
            tracing::debug!(
                tool_call_id = ?msg.tool_call_id,
                tool_name,
                "Rewriting orphaned tool_result as user message",
            );
            msg.role = Role::User;
            msg.content = format!("[Tool `{}` returned: {}]", tool_name, msg.content);
            msg.tool_call_id = None;
            msg.name = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_preserves_valid_pairs() {
        let tc = ToolCall {
            id: "call_1".to_string(),
            name: "echo".to_string(),
            arguments: serde_json::json!({}),
        };
        let mut messages = vec![
            ChatMessage::user("hello"),
            ChatMessage::assistant_with_tool_calls(None, vec![tc]),
            ChatMessage::tool_result("call_1", "echo", "result"),
        ];
        sanitize_tool_messages(&mut messages);
        assert_eq!(messages[2].role, Role::Tool);
        assert_eq!(messages[2].tool_call_id, Some("call_1".to_string()));
    }

    #[test]
    fn test_sanitize_rewrites_orphaned_tool_result() {
        let mut messages = vec![
            ChatMessage::user("hello"),
            ChatMessage::assistant("I'll use a tool"),
            ChatMessage::tool_result("call_missing", "search", "some result"),
        ];
        sanitize_tool_messages(&mut messages);
        assert_eq!(messages[2].role, Role::User);
        assert!(messages[2].content.contains("[Tool `search` returned:"));
        assert!(messages[2].tool_call_id.is_none());
        assert!(messages[2].name.is_none());
    }

    #[test]
    fn test_sanitize_handles_no_tool_messages() {
        let mut messages = vec![
            ChatMessage::system("prompt"),
            ChatMessage::user("hello"),
            ChatMessage::assistant("hi"),
        ];
        let original_len = messages.len();
        sanitize_tool_messages(&mut messages);
        assert_eq!(messages.len(), original_len);
    }

    #[test]
    fn test_sanitize_multiple_orphaned() {
        let tc = ToolCall {
            id: "call_1".to_string(),
            name: "echo".to_string(),
            arguments: serde_json::json!({}),
        };
        let mut messages = vec![
            ChatMessage::user("test"),
            ChatMessage::assistant_with_tool_calls(None, vec![tc]),
            ChatMessage::tool_result("call_1", "echo", "ok"),
            // These are orphaned (call_2 and call_3 have no matching assistant message)
            ChatMessage::tool_result("call_2", "search", "orphan 1"),
            ChatMessage::tool_result("call_3", "http", "orphan 2"),
        ];
        sanitize_tool_messages(&mut messages);
        assert_eq!(messages[2].role, Role::Tool); // call_1 is valid
        assert_eq!(messages[3].role, Role::User); // call_2 orphaned
        assert_eq!(messages[4].role, Role::User); // call_3 orphaned
    }
}
