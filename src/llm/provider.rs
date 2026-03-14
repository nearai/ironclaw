//! LLM provider trait and types.

use async_trait::async_trait;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::llm::error::LlmError;

/// Role in a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A part of multimodal message content (OpenAI Chat Completions format).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    /// Text content part.
    #[serde(rename = "text")]
    Text { text: String },
    /// Image URL content part (supports data: URLs for inline base64 images).
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrl },
}

/// Image URL reference for multimodal content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    /// URL or data: URI (e.g., "data:image/jpeg;base64,...").
    pub url: String,
    /// Detail level hint: "auto", "low", or "high".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// Multimodal content parts (images, etc.).
    /// When non-empty, providers serialize content as an array of parts
    /// (with `content` included as a text part) instead of a plain string.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_parts: Vec<ContentPart>,
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
            content_parts: Vec::new(),
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
            content_parts: Vec::new(),
            tool_call_id: None,
            name: None,
            tool_calls: None,
        }
    }

    /// Create a user message with multimodal content parts (e.g., images).
    ///
    /// The text `content` is included as the primary text alongside the parts.
    pub fn user_with_parts(content: impl Into<String>, parts: Vec<ContentPart>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            content_parts: parts,
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
            content_parts: Vec::new(),
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
            content_parts: Vec::new(),
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
            content_parts: Vec::new(),
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
    /// Tokens read from the provider's server-side prompt cache (Anthropic).
    /// Zero when caching is not supported or on a cache miss.
    pub cache_read_input_tokens: u32,
    /// Tokens written to the provider's server-side prompt cache (Anthropic).
    /// Zero when caching is not supported or no new prefix was cached.
    pub cache_creation_input_tokens: u32,
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
    pub stop_sequences: Option<Vec<String>>,
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
            stop_sequences: None,
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

    /// Set stop sequences.
    pub fn with_stop_sequences(mut self, stop_sequences: Vec<String>) -> Self {
        self.stop_sequences = Some(stop_sequences);
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
    /// Tokens read from the provider's server-side prompt cache (Anthropic).
    pub cache_read_input_tokens: u32,
    /// Tokens written to the provider's server-side prompt cache (Anthropic).
    pub cache_creation_input_tokens: u32,
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

    /// Calculate cost for a completion.
    fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> Decimal {
        let (input_cost, output_cost) = self.cost_per_token();
        input_cost * Decimal::from(input_tokens) + output_cost * Decimal::from(output_tokens)
    }

    /// Cost multiplier for cache-creation tokens (Anthropic prompt caching).
    ///
    /// Returns `1.0` by default (no surcharge). Anthropic providers return
    /// `1.25` for 5-minute TTL or `2.0` for 1-hour TTL.
    fn cache_write_multiplier(&self) -> Decimal {
        Decimal::ONE
    }

    /// Discount divisor for cache-read tokens.
    ///
    /// Cached-read cost = `input_rate / cache_read_discount()`.
    /// Returns `1` by default (no discount). Anthropic returns `10` (90% off),
    /// OpenAI would return `2` (50% off).
    fn cache_read_discount(&self) -> Decimal {
        Decimal::ONE
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

/// Represents a request parameter that may not be supported by all LLM providers.
///
/// This typed enum replaces stringly-typed parameter names across the codebase,
/// providing type safety and single-point-of-maintenance for parameter handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnsupportedParam {
    Temperature,
    MaxTokens,
    StopSequences,
}

impl UnsupportedParam {
    /// Get the string name of this parameter for config/error messages.
    pub fn name(&self) -> &'static str {
        match self {
            UnsupportedParam::Temperature => "temperature",
            UnsupportedParam::MaxTokens => "max_tokens",
            UnsupportedParam::StopSequences => "stop_sequences",
        }
    }
}

/// Strip unsupported parameters from a `CompletionRequest` in place.
///
/// This is the single helper function used by all providers to remove
/// parameters they don't support, replacing duplicate stringly-typed logic.
pub fn strip_unsupported_completion_params(
    unsupported: &std::collections::HashSet<String>,
    req: &mut CompletionRequest,
) {
    if unsupported.is_empty() {
        return;
    }
    if unsupported.contains(UnsupportedParam::Temperature.name()) {
        req.temperature = None;
    }
    if unsupported.contains(UnsupportedParam::MaxTokens.name()) {
        req.max_tokens = None;
    }
    if unsupported.contains(UnsupportedParam::StopSequences.name()) {
        req.stop_sequences = None;
    }
}

/// Strip unsupported parameters from a `ToolCompletionRequest` in place.
///
/// This is the single helper function used by all providers to remove
/// parameters they don't support from tool calls, replacing duplicate stringly-typed logic.
///
pub fn strip_unsupported_tool_params(
    unsupported: &std::collections::HashSet<String>,
    req: &mut ToolCompletionRequest,
) {
    if unsupported.is_empty() {
        return;
    }
    if unsupported.contains(UnsupportedParam::Temperature.name()) {
        req.temperature = None;
    }
    if unsupported.contains(UnsupportedParam::MaxTokens.name()) {
        req.max_tokens = None;
    }
    if unsupported.contains(UnsupportedParam::StopSequences.name()) {
        req.stop_sequences = None;
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

    /// Regression: worker's select_tools/execute_plan now emit
    /// assistant_with_tool_calls before tool_result messages.
    /// Verify sanitize_tool_messages preserves all tool_results when
    /// each has a matching assistant tool_call.
    #[test]
    fn test_sanitize_preserves_tool_results_with_matching_assistant() {
        let tc1 = ToolCall {
            id: "call_sel_1".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"q": "test"}),
        };
        let tc2 = ToolCall {
            id: "call_sel_2".to_string(),
            name: "http".to_string(),
            arguments: serde_json::json!({"url": "https://example.com"}),
        };
        let mut messages = vec![
            ChatMessage::system("You are a helpful assistant."),
            ChatMessage::assistant_with_tool_calls(None, vec![tc1, tc2]),
            ChatMessage::tool_result("call_sel_1", "search", "found 3 results"),
            ChatMessage::tool_result("call_sel_2", "http", "200 OK"),
        ];
        sanitize_tool_messages(&mut messages);

        // All tool_results must keep Role::Tool -- none should be rewritten.
        assert_eq!(messages[2].role, Role::Tool);
        assert_eq!(messages[2].tool_call_id, Some("call_sel_1".to_string()));
        assert_eq!(messages[2].content, "found 3 results");

        assert_eq!(messages[3].role, Role::Tool);
        assert_eq!(messages[3].tool_call_id, Some("call_sel_2".to_string()));
        assert_eq!(messages[3].content, "200 OK");
    }

    /// Regression: the OLD buggy worker code pushed tool_result messages
    /// without a preceding assistant_with_tool_calls, causing
    /// sanitize_tool_messages to rewrite them as orphaned user messages.
    /// This test reproduces that buggy sequence and confirms the rewrite.
    #[test]
    fn test_sanitize_rewrites_orphaned_tool_results() {
        let mut messages = vec![
            ChatMessage::system("You are a helpful assistant."),
            // No assistant_with_tool_calls -- mimics the old bug.
            ChatMessage::tool_result("call_bug_1", "search", "found 3 results"),
            ChatMessage::tool_result("call_bug_2", "http", "200 OK"),
        ];
        sanitize_tool_messages(&mut messages);

        // Both tool_results must be rewritten to Role::User.
        assert_eq!(messages[1].role, Role::User);
        assert!(messages[1].content.contains("[Tool `search` returned:"));
        assert!(messages[1].content.contains("found 3 results"));
        assert!(messages[1].tool_call_id.is_none());
        assert!(messages[1].name.is_none());

        assert_eq!(messages[2].role, Role::User);
        assert!(messages[2].content.contains("[Tool `http` returned:"));
        assert!(messages[2].content.contains("200 OK"));
        assert!(messages[2].tool_call_id.is_none());
        assert!(messages[2].name.is_none());
    }

    #[test]
    fn test_strip_unsupported_tool_params_strips_stop_sequences() {
        let mut unsupported = std::collections::HashSet::new();
        unsupported.insert(UnsupportedParam::StopSequences.name().to_string());

        let mut req = ToolCompletionRequest::new(vec![ChatMessage::user("hello")], vec![]);
        req.stop_sequences = Some(vec!["STOP".to_string()]);

        strip_unsupported_tool_params(&unsupported, &mut req);

        assert!( // safety: test assertion for explicit strip behavior
            req.stop_sequences.is_none(),
            "stop_sequences should be stripped for tool requests"
        );
    }
}
