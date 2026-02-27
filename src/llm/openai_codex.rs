//! OpenAI Codex provider (Responses API).
//!
//! Supports two auth modes:
//! - **API key**: Standard OpenAI billing via `api.openai.com/v1/responses`
//! - **OAuth**: ChatGPT subscription billing via `chatgpt.com/backend-api/codex/responses`,
//!   using tokens from the Codex CLI (`~/.codex/auth.json`)
//!
//! The Responses API has a fundamentally different wire format from Chat Completions:
//! flat `input` array, `instructions` instead of system messages, flat tool definitions
//! (no `function` wrapper nesting), and `output` array with typed items.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use crate::config::OpenAiCodexConfig;
use crate::error::LlmError;
use crate::llm::costs;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse, ToolDefinition,
};

// ---------------------------------------------------------------------------
// Responses API request types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ResponsesRequest {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    input: Vec<InputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<CodexToolDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
enum InputItem {
    #[serde(rename = "message")]
    Message { role: String, content: String },
    #[serde(rename = "function_call_output")]
    FunctionCallOutput { call_id: String, output: String },
}

/// Flat tool definition for the Responses API.
///
/// Unlike Chat Completions, tools are NOT nested under a `function` key.
#[derive(Serialize)]
struct CodexToolDef {
    r#type: String,
    name: String,
    description: String,
    parameters: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Responses API response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ResponsesResponse {
    #[allow(dead_code)]
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    output: Vec<OutputItem>,
    #[serde(default)]
    usage: Option<ResponsesUsage>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum OutputItem {
    #[serde(rename = "message")]
    Message {
        #[allow(dead_code)]
        role: String,
        content: Vec<ContentBlock>,
    },
    #[serde(rename = "function_call")]
    FunctionCall {
        call_id: String,
        name: String,
        arguments: String,
    },
}

#[derive(Deserialize)]
struct ContentBlock {
    #[allow(dead_code)]
    r#type: String,
    text: String,
}

#[derive(Deserialize, Default)]
struct ResponsesUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

// ---------------------------------------------------------------------------
// Token management for OAuth mode
// ---------------------------------------------------------------------------

struct CodexTokenManager {
    auth_path: PathBuf,
    current_token: tokio::sync::RwLock<Option<TokenState>>,
}

struct TokenState {
    access_token: String,
    #[allow(dead_code)]
    refresh_token: Option<String>,
}

impl CodexTokenManager {
    fn new(auth_path: PathBuf) -> Self {
        Self {
            auth_path,
            current_token: tokio::sync::RwLock::new(None),
        }
    }

    /// Get a valid access token, loading from disk if not yet cached.
    async fn get_token(&self) -> Result<String, LlmError> {
        // Check cached token first
        {
            let guard = self.current_token.read().await;
            if let Some(ref state) = *guard {
                return Ok(state.access_token.clone());
            }
        }

        // Load from disk
        self.load_from_disk().await
    }

    /// Load token from the auth.json file on disk.
    async fn load_from_disk(&self) -> Result<String, LlmError> {
        let path = self.auth_path.clone();
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| LlmError::AuthFailed {
                provider: format!("openai_codex (cannot read {}): {}", path.display(), e),
            })?;

        let json: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| LlmError::AuthFailed {
                provider: format!("openai_codex (cannot parse {}): {}", path.display(), e),
            })?;

        // Extract access token (try multiple field paths)
        let access_token = json
            .get("tokens")
            .and_then(|t| t.get("access_token"))
            .and_then(|v| v.as_str())
            .or_else(|| json.get("token").and_then(|v| v.as_str()))
            .or_else(|| json.get("api_key").and_then(|v| v.as_str()))
            .or_else(|| json.get("access_token").and_then(|v| v.as_str()))
            .ok_or_else(|| LlmError::AuthFailed {
                provider: format!("openai_codex (no token found in {})", path.display()),
            })?
            .to_string();

        // Extract refresh token if available
        let refresh_token = json
            .get("tokens")
            .and_then(|t| t.get("refresh_token"))
            .and_then(|v| v.as_str())
            .or_else(|| json.get("refresh_token").and_then(|v| v.as_str()))
            .map(String::from);

        let state = TokenState {
            access_token: access_token.clone(),
            refresh_token,
        };

        let mut guard = self.current_token.write().await;
        *guard = Some(state);

        Ok(access_token)
    }

    /// Clear cached token (forces reload on next get_token call).
    async fn invalidate(&self) {
        let mut guard = self.current_token.write().await;
        *guard = None;
    }
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

/// Known Codex models for the wizard (OAuth tokens can't call /v1/models).
pub const CODEX_MODELS: &[(&str, &str)] = &[
    ("gpt-5.3-codex", "GPT-5.3 Codex (flagship)"),
    ("gpt-5.3-codex-spark", "GPT-5.3 Codex Spark (fast)"),
    ("gpt-5.2-codex", "GPT-5.2 Codex"),
    ("gpt-5.1-codex", "GPT-5.1 Codex"),
    ("gpt-5.1-codex-mini", "GPT-5.1 Codex Mini"),
    ("gpt-5-codex", "GPT-5 Codex"),
    ("o3", "o3 (reasoning)"),
    ("o4-mini", "o4-mini (reasoning)"),
];

/// OpenAI Codex provider using the Responses API.
pub struct OpenAiCodexProvider {
    client: Client,
    config: OpenAiCodexConfig,
    /// Token manager for OAuth mode. `None` when using API key.
    token_manager: Option<Arc<CodexTokenManager>>,
    active_model: std::sync::RwLock<String>,
}

impl OpenAiCodexProvider {
    /// Create a new Codex provider.
    pub fn new(config: OpenAiCodexConfig) -> Result<Self, LlmError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: "openai_codex".to_string(),
                reason: format!("Failed to build HTTP client: {}", e),
            })?;

        let token_manager = if config.api_key.is_none() {
            Some(Arc::new(CodexTokenManager::new(config.auth_path.clone())))
        } else {
            None
        };

        let active_model = std::sync::RwLock::new(config.model.clone());

        Ok(Self {
            client,
            config,
            token_manager,
            active_model,
        })
    }

    /// Build the full URL for the Responses API endpoint.
    fn responses_url(&self) -> String {
        let base = self.config.base_url.trim_end_matches('/');
        if base.ends_with("/v1") {
            format!("{}/responses", base)
        } else if base.contains("chatgpt.com") {
            // ChatGPT endpoint: base is already .../codex
            format!("{}/responses", base)
        } else {
            format!("{}/v1/responses", base)
        }
    }

    /// Whether we're using API key auth (vs OAuth).
    fn uses_api_key(&self) -> bool {
        self.config.api_key.is_some()
    }

    /// Resolve the Bearer token for the current auth mode.
    async fn resolve_bearer_token(&self) -> Result<String, LlmError> {
        if let Some(ref api_key) = self.config.api_key {
            Ok(api_key.expose_secret().to_string())
        } else if let Some(ref tm) = self.token_manager {
            tm.get_token().await
        } else {
            Err(LlmError::AuthFailed {
                provider: "openai_codex".to_string(),
            })
        }
    }

    /// Send a request to the Responses API, with 401 retry for OAuth mode.
    async fn send_request(&self, body: &ResponsesRequest) -> Result<ResponsesResponse, LlmError> {
        match self.send_request_inner(body).await {
            Ok(result) => Ok(result),
            Err(LlmError::AuthFailed { .. }) if !self.uses_api_key() => {
                // OAuth token may have expired — reload from disk and retry once
                if let Some(ref tm) = self.token_manager {
                    tm.invalidate().await;
                }
                self.send_request_inner(body).await
            }
            Err(e) => Err(e),
        }
    }

    /// Inner request implementation (single attempt).
    async fn send_request_inner(
        &self,
        body: &ResponsesRequest,
    ) -> Result<ResponsesResponse, LlmError> {
        let url = self.responses_url();
        let token = self.resolve_bearer_token().await?;

        tracing::debug!("Sending request to OpenAI Codex: {}", url);

        let mut req = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json");

        // Add account ID header for ChatGPT endpoint
        if let Some(ref account_id) = self.config.account_id {
            req = req.header("openai-account-id", account_id);
        }

        let response = req
            .json(body)
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: "openai_codex".to_string(),
                reason: e.to_string(),
            })?;

        let status = response.status();
        let response_text = response.text().await.map_err(|e| LlmError::RequestFailed {
            provider: "openai_codex".to_string(),
            reason: format!("Failed to read response body: {}", e),
        })?;

        tracing::debug!("OpenAI Codex response status: {}", status);
        tracing::debug!("OpenAI Codex response body: {}", response_text);

        if !status.is_success() {
            let status_code = status.as_u16();

            if status_code == 401 {
                return Err(LlmError::AuthFailed {
                    provider: "openai_codex".to_string(),
                });
            }

            if status_code == 429 {
                return Err(LlmError::RateLimited {
                    provider: "openai_codex".to_string(),
                    retry_after: None,
                });
            }

            let truncated = crate::agent::truncate_for_preview(&response_text, 512);
            return Err(LlmError::RequestFailed {
                provider: "openai_codex".to_string(),
                reason: format!("HTTP {}: {}", status, truncated),
            });
        }

        serde_json::from_str(&response_text).map_err(|e| {
            let truncated = crate::agent::truncate_for_preview(&response_text, 512);
            LlmError::InvalidResponse {
                provider: "openai_codex".to_string(),
                reason: format!("JSON parse error: {}. Raw: {}", e, truncated),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Message / tool conversion
// ---------------------------------------------------------------------------

/// Convert IronClaw messages to Responses API format.
///
/// System messages are extracted into a single `instructions` string.
/// User/Assistant messages become `InputItem::Message`.
/// Tool result messages become `InputItem::FunctionCallOutput`.
fn convert_messages(messages: &[ChatMessage]) -> (Option<String>, Vec<InputItem>) {
    let mut instructions_parts: Vec<String> = Vec::new();
    let mut input: Vec<InputItem> = Vec::new();

    for msg in messages {
        match msg.role {
            Role::System => {
                instructions_parts.push(msg.content.clone());
            }
            Role::User => {
                input.push(InputItem::Message {
                    role: "user".to_string(),
                    content: msg.content.clone(),
                });
            }
            Role::Assistant => {
                if !msg.content.is_empty() {
                    input.push(InputItem::Message {
                        role: "assistant".to_string(),
                        content: msg.content.clone(),
                    });
                }
            }
            Role::Tool => {
                if let Some(ref call_id) = msg.tool_call_id {
                    input.push(InputItem::FunctionCallOutput {
                        call_id: call_id.clone(),
                        output: msg.content.clone(),
                    });
                } else {
                    tracing::warn!(
                        "Skipping tool message without tool_call_id (tool: {:?})",
                        msg.name
                    );
                }
            }
        }
    }

    let instructions = if instructions_parts.is_empty() {
        None
    } else {
        Some(instructions_parts.join("\n\n"))
    };

    (instructions, input)
}

/// Convert IronClaw tool definitions to Responses API flat format.
fn convert_tools(tools: &[ToolDefinition]) -> Vec<CodexToolDef> {
    tools
        .iter()
        .map(|t| CodexToolDef {
            r#type: "function".to_string(),
            name: t.name.clone(),
            description: t.description.clone(),
            parameters: t.parameters.clone(),
        })
        .collect()
}

/// Parse output items into text content and tool calls.
fn parse_output(output: Vec<OutputItem>) -> (Option<String>, Vec<ToolCall>) {
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    for item in output {
        match item {
            OutputItem::Message { content, .. } => {
                for block in content {
                    if !block.text.is_empty() {
                        text_parts.push(block.text);
                    }
                }
            }
            OutputItem::FunctionCall {
                call_id,
                name,
                arguments,
            } => {
                let args = serde_json::from_str(&arguments)
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                tool_calls.push(ToolCall {
                    id: call_id,
                    name,
                    arguments: args,
                });
            }
        }
    }

    let content = if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join(""))
    };

    (content, tool_calls)
}

/// Map Responses API status to FinishReason.
fn map_status(status: Option<&str>, has_tool_calls: bool) -> FinishReason {
    match status {
        Some("completed") => FinishReason::Stop,
        Some("incomplete") => FinishReason::Length,
        Some("failed") => FinishReason::Unknown,
        _ => {
            if has_tool_calls {
                FinishReason::ToolUse
            } else {
                FinishReason::Unknown
            }
        }
    }
}

// ---------------------------------------------------------------------------
// LlmProvider implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmProvider for OpenAiCodexProvider {
    fn model_name(&self) -> &str {
        &self.config.model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        let model = self.active_model_name();
        costs::model_cost(&model).unwrap_or_else(costs::default_cost)
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let model = req.model.unwrap_or_else(|| self.active_model_name());
        let (instructions, input) = convert_messages(&req.messages);

        let request = ResponsesRequest {
            model,
            instructions,
            input,
            tools: None,
            max_output_tokens: req.max_tokens,
            temperature: req.temperature,
        };

        let response = self.send_request(&request).await?;
        let (input_tokens, output_tokens) = match response.usage {
            Some(u) => (u.input_tokens, u.output_tokens),
            None => (0, 0),
        };

        let (content, _) = parse_output(response.output);
        let finish_reason = map_status(response.status.as_deref(), false);

        Ok(CompletionResponse {
            content: content.unwrap_or_default(),
            input_tokens,
            output_tokens,
            finish_reason,
        })
    }

    async fn complete_with_tools(
        &self,
        req: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let model = req.model.unwrap_or_else(|| self.active_model_name());
        let (instructions, input) = convert_messages(&req.messages);
        let tools = convert_tools(&req.tools);

        let request = ResponsesRequest {
            model,
            instructions,
            input,
            tools: if tools.is_empty() { None } else { Some(tools) },
            max_output_tokens: req.max_tokens,
            temperature: req.temperature,
        };

        let response = self.send_request(&request).await?;
        let (input_tokens, output_tokens) = match response.usage {
            Some(u) => (u.input_tokens, u.output_tokens),
            None => (0, 0),
        };

        let (content, tool_calls) = parse_output(response.output);
        let finish_reason = map_status(response.status.as_deref(), !tool_calls.is_empty());

        Ok(ToolCompletionResponse {
            content,
            tool_calls,
            input_tokens,
            output_tokens,
            finish_reason,
        })
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        Ok(CODEX_MODELS.iter().map(|(id, _)| id.to_string()).collect())
    }

    fn active_model_name(&self) -> String {
        match self.active_model.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => {
                tracing::warn!("active_model lock poisoned while reading; continuing");
                poisoned.into_inner().clone()
            }
        }
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        match self.active_model.write() {
            Ok(mut guard) => {
                *guard = model.to_string();
            }
            Err(poisoned) => {
                tracing::warn!("active_model lock poisoned while writing; continuing");
                *poisoned.into_inner() = model.to_string();
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_messages_system_to_instructions() {
        let messages = vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::system("Be concise."),
            ChatMessage::user("Hello"),
        ];

        let (instructions, input) = convert_messages(&messages);
        assert_eq!(
            instructions,
            Some("You are helpful.\n\nBe concise.".to_string())
        );
        assert_eq!(input.len(), 1);

        // Verify it's a user message
        match &input[0] {
            InputItem::Message { role, content } => {
                assert_eq!(role, "user");
                assert_eq!(content, "Hello");
            }
            _ => panic!("Expected Message"),
        }
    }

    #[test]
    fn test_convert_messages_tool_result() {
        let messages = vec![ChatMessage::tool_result(
            "call_123",
            "my_tool",
            "result data",
        )];

        let (instructions, input) = convert_messages(&messages);
        assert!(instructions.is_none());
        assert_eq!(input.len(), 1);

        match &input[0] {
            InputItem::FunctionCallOutput { call_id, output } => {
                assert_eq!(call_id, "call_123");
                assert_eq!(output, "result data");
            }
            _ => panic!("Expected FunctionCallOutput"),
        }
    }

    #[test]
    fn test_convert_messages_skips_tool_without_call_id() {
        let msg = ChatMessage {
            role: Role::Tool,
            content: "orphan result".to_string(),
            tool_call_id: None,
            name: Some("broken_tool".to_string()),
            tool_calls: None,
        };

        let (_, input) = convert_messages(&[msg]);
        assert!(input.is_empty());
    }

    #[test]
    fn test_convert_messages_full_conversation() {
        let messages = vec![
            ChatMessage::system("Be helpful"),
            ChatMessage::user("What time is it?"),
            ChatMessage::assistant("Let me check."),
            ChatMessage::tool_result("call_1", "time", "14:30"),
        ];

        let (instructions, input) = convert_messages(&messages);
        assert_eq!(instructions, Some("Be helpful".to_string()));
        assert_eq!(input.len(), 3);

        match &input[0] {
            InputItem::Message { role, .. } => assert_eq!(role, "user"),
            _ => panic!("Expected Message"),
        }
        match &input[1] {
            InputItem::Message { role, .. } => assert_eq!(role, "assistant"),
            _ => panic!("Expected Message"),
        }
        match &input[2] {
            InputItem::FunctionCallOutput { call_id, .. } => assert_eq!(call_id, "call_1"),
            _ => panic!("Expected FunctionCallOutput"),
        }
    }

    #[test]
    fn test_convert_tools_flat_format() {
        let tools = vec![
            ToolDefinition {
                name: "search".to_string(),
                description: "Search the web".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" }
                    },
                    "required": ["query"]
                }),
            },
            ToolDefinition {
                name: "read".to_string(),
                description: "Read a file".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    }
                }),
            },
        ];

        let converted = convert_tools(&tools);
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0].r#type, "function");
        assert_eq!(converted[0].name, "search");
        assert_eq!(converted[1].name, "read");

        // Verify flat format by serializing
        let json = serde_json::to_value(&converted[0]).expect("serialize");
        assert!(json.get("type").is_some());
        assert!(json.get("name").is_some());
        // Should NOT have a nested "function" key
        assert!(json.get("function").is_none());
    }

    #[test]
    fn test_parse_output_message() {
        let output = vec![OutputItem::Message {
            role: "assistant".to_string(),
            content: vec![ContentBlock {
                r#type: "output_text".to_string(),
                text: "Hello world".to_string(),
            }],
        }];

        let (content, tool_calls) = parse_output(output);
        assert_eq!(content, Some("Hello world".to_string()));
        assert!(tool_calls.is_empty());
    }

    #[test]
    fn test_parse_output_function_call() {
        let output = vec![OutputItem::FunctionCall {
            call_id: "call_abc".to_string(),
            name: "search".to_string(),
            arguments: r#"{"query":"test"}"#.to_string(),
        }];

        let (content, tool_calls) = parse_output(output);
        assert!(content.is_none());
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "call_abc");
        assert_eq!(tool_calls[0].name, "search");
        assert_eq!(tool_calls[0].arguments["query"], "test");
    }

    #[test]
    fn test_parse_output_mixed() {
        let output = vec![
            OutputItem::Message {
                role: "assistant".to_string(),
                content: vec![ContentBlock {
                    r#type: "output_text".to_string(),
                    text: "I'll search for that.".to_string(),
                }],
            },
            OutputItem::FunctionCall {
                call_id: "call_1".to_string(),
                name: "search".to_string(),
                arguments: r#"{"q":"rust"}"#.to_string(),
            },
        ];

        let (content, tool_calls) = parse_output(output);
        assert_eq!(content, Some("I'll search for that.".to_string()));
        assert_eq!(tool_calls.len(), 1);
    }

    #[test]
    fn test_map_status() {
        assert_eq!(map_status(Some("completed"), false), FinishReason::Stop);
        assert_eq!(map_status(Some("incomplete"), false), FinishReason::Length);
        assert_eq!(map_status(Some("failed"), false), FinishReason::Unknown);
        assert_eq!(map_status(None, true), FinishReason::ToolUse);
        assert_eq!(map_status(None, false), FinishReason::Unknown);
    }

    #[test]
    fn test_responses_request_serialization() {
        let req = ResponsesRequest {
            model: "gpt-5.3-codex".to_string(),
            instructions: Some("Be helpful".to_string()),
            input: vec![InputItem::Message {
                role: "user".to_string(),
                content: "Hello".to_string(),
            }],
            tools: None,
            max_output_tokens: None,
            temperature: None,
        };

        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["model"], "gpt-5.3-codex");
        assert_eq!(json["instructions"], "Be helpful");
        assert_eq!(json["input"][0]["type"], "message");
        assert_eq!(json["input"][0]["role"], "user");
        assert!(json.get("tools").is_none()); // skip_serializing_if
    }

    #[test]
    fn test_responses_response_deserialization() {
        let json = serde_json::json!({
            "id": "resp_123",
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        { "type": "output_text", "text": "Hello!" }
                    ]
                }
            ],
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5
            },
            "status": "completed"
        });

        let resp: ResponsesResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(resp.status, Some("completed".to_string()));
        assert_eq!(resp.output.len(), 1);
        assert_eq!(resp.usage.as_ref().map(|u| u.input_tokens), Some(10));
    }

    #[test]
    fn test_responses_response_with_function_calls() {
        let json = serde_json::json!({
            "id": "resp_456",
            "output": [
                {
                    "type": "function_call",
                    "call_id": "call_xyz",
                    "name": "read_file",
                    "arguments": "{\"path\": \"/tmp/test.txt\"}"
                }
            ],
            "usage": {
                "input_tokens": 20,
                "output_tokens": 15
            },
            "status": "completed"
        });

        let resp: ResponsesResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(resp.output.len(), 1);
        match &resp.output[0] {
            OutputItem::FunctionCall {
                call_id,
                name,
                arguments,
            } => {
                assert_eq!(call_id, "call_xyz");
                assert_eq!(name, "read_file");
                assert!(arguments.contains("test.txt"));
            }
            _ => panic!("Expected FunctionCall"),
        }
    }

    #[test]
    fn test_input_item_serialization() {
        let msg = InputItem::Message {
            role: "user".to_string(),
            content: "hi".to_string(),
        };
        let json = serde_json::to_value(&msg).expect("serialize");
        assert_eq!(json["type"], "message");
        assert_eq!(json["role"], "user");

        let fc = InputItem::FunctionCallOutput {
            call_id: "call_1".to_string(),
            output: "result".to_string(),
        };
        let json = serde_json::to_value(&fc).expect("serialize");
        assert_eq!(json["type"], "function_call_output");
        assert_eq!(json["call_id"], "call_1");
    }

    #[test]
    fn test_codex_models_list() {
        assert!(!CODEX_MODELS.is_empty());
        assert!(CODEX_MODELS.iter().any(|(id, _)| *id == "gpt-5.3-codex"));
    }

    fn test_config_api_key() -> OpenAiCodexConfig {
        OpenAiCodexConfig {
            model: "gpt-5.3-codex".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: Some(secrecy::SecretString::from("sk-test")),
            auth_path: std::path::PathBuf::from("/tmp/nonexistent-auth.json"),
            account_id: None,
        }
    }

    fn test_config_oauth() -> OpenAiCodexConfig {
        OpenAiCodexConfig {
            model: "gpt-5.3-codex".to_string(),
            base_url: "https://chatgpt.com/backend-api/codex".to_string(),
            api_key: None,
            auth_path: std::path::PathBuf::from("/tmp/nonexistent-auth.json"),
            account_id: Some("acct_123".to_string()),
        }
    }

    #[test]
    fn test_provider_creates_with_api_key() {
        let provider = OpenAiCodexProvider::new(test_config_api_key()).expect("create provider");
        assert_eq!(provider.model_name(), "gpt-5.3-codex");
        assert!(provider.uses_api_key());
        assert!(provider.token_manager.is_none());
    }

    #[test]
    fn test_provider_creates_with_oauth() {
        let provider = OpenAiCodexProvider::new(test_config_oauth()).expect("create provider");
        assert!(!provider.uses_api_key());
        assert!(provider.token_manager.is_some());
    }

    #[test]
    fn test_responses_url_api_key_mode() {
        let provider = OpenAiCodexProvider::new(test_config_api_key()).expect("create provider");
        assert_eq!(
            provider.responses_url(),
            "https://api.openai.com/v1/responses"
        );
    }

    #[test]
    fn test_responses_url_oauth_mode() {
        let provider = OpenAiCodexProvider::new(test_config_oauth()).expect("create provider");
        assert_eq!(
            provider.responses_url(),
            "https://chatgpt.com/backend-api/codex/responses"
        );
    }

    #[test]
    fn test_responses_url_custom_base() {
        let config = OpenAiCodexConfig {
            base_url: "https://custom.example.com/v1".to_string(),
            ..test_config_api_key()
        };
        let provider = OpenAiCodexProvider::new(config).expect("create provider");
        assert_eq!(
            provider.responses_url(),
            "https://custom.example.com/v1/responses"
        );
    }

    #[test]
    fn test_set_model_and_active_model() {
        let provider = OpenAiCodexProvider::new(test_config_api_key()).expect("create provider");
        assert_eq!(provider.active_model_name(), "gpt-5.3-codex");

        provider.set_model("gpt-5.1-codex").expect("set model");
        assert_eq!(provider.active_model_name(), "gpt-5.1-codex");
        // model_name() still returns the original config model
        assert_eq!(provider.model_name(), "gpt-5.3-codex");
    }

    #[test]
    fn test_cost_per_token_known_model() {
        let provider = OpenAiCodexProvider::new(test_config_api_key()).expect("create provider");
        let (input_cost, output_cost) = provider.cost_per_token();
        // gpt-5.3-codex is in costs.rs
        assert!(input_cost > Decimal::ZERO);
        assert!(output_cost > Decimal::ZERO);
    }

    #[tokio::test]
    async fn test_list_models_returns_hardcoded() {
        let provider = OpenAiCodexProvider::new(test_config_api_key()).expect("create provider");
        let models = provider.list_models().await.expect("list models");
        assert!(!models.is_empty());
        assert!(models.contains(&"gpt-5.3-codex".to_string()));
    }
}
