//! Native Anthropic Messages API provider.
//!
//! Supports two authentication methods:
//! - **API key**: Standard `x-api-key` header (pay-per-token billing)
//! - **OAuth token**: `Authorization: Bearer` header from Claude Code /
//!   Anthropic Max subscription (included in subscription, no per-token cost).
//!   Includes framework to refresh expired tokens, but requires a refresh
//!   token which is not yet collected in the setup flow.

use std::sync::RwLock;

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::config::{AnthropicAuth, AnthropicDirectConfig};
use crate::error::LlmError;
use crate::llm::costs;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ModelMetadata,
    Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse, ToolDefinition,
};
use crate::llm::retry::{is_retryable_status, retry_backoff_delay};

const API_BASE: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";
const OAUTH_BETA_HEADER: &str = "oauth-2025-04-20";
const TOKEN_REFRESH_URL: &str = "https://console.anthropic.com/v1/oauth/token";
const PROVIDER_NAME: &str = "anthropic";

/// Native Anthropic Messages API provider.
pub struct AnthropicProvider {
    client: Client,
    config: AnthropicDirectConfig,
    active_model: RwLock<String>,
    /// Mutable access token for OAuth refresh.
    oauth_access_token: RwLock<Option<SecretString>>,
}

impl AnthropicProvider {
    /// Create a new Anthropic provider.
    pub fn new(config: AnthropicDirectConfig) -> Self {
        let active_model = RwLock::new(config.model.clone());
        let oauth_access_token = match &config.auth {
            AnthropicAuth::OAuthToken { access_token, .. } => {
                RwLock::new(Some(access_token.clone()))
            }
            AnthropicAuth::ApiKey(_) => RwLock::new(None),
        };

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            config,
            active_model,
            oauth_access_token,
        }
    }

    /// Build authorization headers based on auth method.
    fn auth_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.config.auth {
            AnthropicAuth::ApiKey(key) => builder.header("x-api-key", key.expose_secret()),
            AnthropicAuth::OAuthToken { .. } => {
                let token = self
                    .oauth_access_token
                    .read()
                    .expect("oauth_access_token lock poisoned");
                let token_str = token
                    .as_ref()
                    .map(|t| t.expose_secret().to_string())
                    .unwrap_or_default();
                builder
                    .header("Authorization", format!("Bearer {}", token_str))
                    .header("anthropic-beta", OAUTH_BETA_HEADER)
            }
        }
    }

    fn is_oauth(&self) -> bool {
        matches!(self.config.auth, AnthropicAuth::OAuthToken { .. })
    }

    /// Attempt to refresh the OAuth access token.
    async fn refresh_oauth_token(&self) -> Result<(), LlmError> {
        let refresh_token = match &self.config.auth {
            AnthropicAuth::OAuthToken {
                refresh_token: Some(rt),
                ..
            } => rt.expose_secret().to_string(),
            _ => {
                return Err(LlmError::AuthFailed {
                    provider: PROVIDER_NAME.to_string(),
                });
            }
        };

        tracing::info!("Refreshing Anthropic OAuth token");

        let resp = self
            .client
            .post(TOKEN_REFRESH_URL)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", &refresh_token),
            ])
            .send()
            .await
            .map_err(|e| LlmError::SessionRenewalFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: e.to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(LlmError::SessionRenewalFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("HTTP {}: {}", status, body),
            });
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
        }

        let token_resp: TokenResponse =
            resp.json()
                .await
                .map_err(|e| LlmError::SessionRenewalFailed {
                    provider: PROVIDER_NAME.to_string(),
                    reason: format!("Failed to parse token response: {}", e),
                })?;

        let mut guard = self
            .oauth_access_token
            .write()
            .expect("oauth_access_token lock poisoned");
        *guard = Some(SecretString::from(token_resp.access_token));

        tracing::info!("Anthropic OAuth token refreshed successfully");
        Ok(())
    }

    /// Send a request to the Messages API with retry and optional OAuth refresh.
    async fn send_request<R: for<'de> Deserialize<'de>>(
        &self,
        body: &MessagesRequest,
    ) -> Result<R, LlmError> {
        let url = format!("{}/v1/messages", API_BASE);
        let max_retries = self.config.max_retries;
        let mut refreshed = false;

        for attempt in 0..=max_retries {
            tracing::debug!(
                "Sending request to Anthropic Messages API (attempt {})",
                attempt + 1,
            );

            let builder = self
                .client
                .post(&url)
                .header("content-type", "application/json")
                .header("anthropic-version", API_VERSION);
            let builder = self.auth_headers(builder);

            let response = match builder.json(body).send().await {
                Ok(r) => r,
                Err(e) => {
                    if attempt < max_retries {
                        let delay = retry_backoff_delay(attempt);
                        tracing::warn!(
                            "Anthropic request error (attempt {}/{}), retrying in {:?}: {}",
                            attempt + 1,
                            max_retries + 1,
                            delay,
                            e,
                        );
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return Err(LlmError::RequestFailed {
                        provider: PROVIDER_NAME.to_string(),
                        reason: e.to_string(),
                    });
                }
            };

            let status = response.status();
            let status_code = status.as_u16();

            // OAuth token refresh on 401
            if status_code == 401 && self.is_oauth() && !refreshed {
                tracing::info!("Anthropic returned 401, attempting OAuth token refresh");
                match self.refresh_oauth_token().await {
                    Ok(()) => {
                        refreshed = true;
                        continue;
                    }
                    Err(e) => {
                        tracing::error!("OAuth token refresh failed: {}", e);
                        return Err(LlmError::AuthFailed {
                            provider: PROVIDER_NAME.to_string(),
                        });
                    }
                }
            }

            let response_text = response.text().await.unwrap_or_default();

            tracing::debug!("Anthropic response status: {}", status);
            if tracing::enabled!(tracing::Level::TRACE) {
                tracing::trace!("Anthropic response body: {}", response_text);
            }

            if !status.is_success() {
                if status_code == 401 {
                    return Err(LlmError::AuthFailed {
                        provider: PROVIDER_NAME.to_string(),
                    });
                }

                if is_retryable_status(status_code) && attempt < max_retries {
                    let delay = retry_backoff_delay(attempt);
                    tracing::warn!(
                        "Anthropic returned HTTP {} (attempt {}/{}), retrying in {:?}",
                        status_code,
                        attempt + 1,
                        max_retries + 1,
                        delay,
                    );
                    tokio::time::sleep(delay).await;
                    continue;
                }

                if status_code == 429 {
                    return Err(LlmError::RateLimited {
                        provider: PROVIDER_NAME.to_string(),
                        retry_after: None,
                    });
                }

                return Err(LlmError::RequestFailed {
                    provider: PROVIDER_NAME.to_string(),
                    reason: format!("HTTP {}: {}", status, response_text),
                });
            }

            return serde_json::from_str(&response_text).map_err(|e| LlmError::InvalidResponse {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("JSON parse error: {}. Raw: {}", e, response_text),
            });
        }

        Err(LlmError::RequestFailed {
            provider: PROVIDER_NAME.to_string(),
            reason: "retry loop exited unexpectedly".to_string(),
        })
    }
}

// -- Anthropic Messages API request/response types --

#[derive(Debug, Serialize)]
struct MessagesRequest {
    model: String,
    messages: Vec<ApiMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ApiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<ApiToolChoice>,
}

#[derive(Debug, Serialize)]
struct ApiMessage {
    role: String,
    content: ApiContent,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ApiContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Debug, Serialize)]
struct ApiTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct ApiToolChoice {
    #[serde(rename = "type")]
    choice_type: String,
}

#[derive(Debug, Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
    stop_reason: Option<String>,
    usage: ApiUsage,
}

#[derive(Debug, Deserialize)]
struct ApiUsage {
    input_tokens: u32,
    output_tokens: u32,
}

// -- Message conversion --

/// Convert our ChatMessage list to Anthropic API format.
///
/// Anthropic requires:
/// - System messages extracted to top-level `system` field
/// - Tool results as `tool_result` content blocks inside `user` messages
/// - Tool calls as `tool_use` content blocks inside `assistant` messages
fn convert_messages(messages: Vec<ChatMessage>) -> (Option<String>, Vec<ApiMessage>) {
    let mut system_text: Option<String> = None;
    let mut api_messages: Vec<ApiMessage> = Vec::new();

    for msg in messages {
        match msg.role {
            Role::System => {
                // Accumulate system messages into one string
                if let Some(ref mut existing) = system_text {
                    existing.push_str("\n\n");
                    existing.push_str(&msg.content);
                } else {
                    system_text = Some(msg.content);
                }
            }
            Role::User => {
                api_messages.push(ApiMessage {
                    role: "user".to_string(),
                    content: ApiContent::Text(msg.content),
                });
            }
            Role::Assistant => {
                if let Some(tool_calls) = msg.tool_calls {
                    // Assistant message with tool calls -> content blocks
                    let mut blocks: Vec<ContentBlock> = Vec::new();
                    if !msg.content.is_empty() {
                        blocks.push(ContentBlock::Text {
                            text: msg.content.clone(),
                        });
                    }
                    for tc in tool_calls {
                        blocks.push(ContentBlock::ToolUse {
                            id: tc.id,
                            name: tc.name,
                            input: tc.arguments,
                        });
                    }
                    api_messages.push(ApiMessage {
                        role: "assistant".to_string(),
                        content: ApiContent::Blocks(blocks),
                    });
                } else {
                    api_messages.push(ApiMessage {
                        role: "assistant".to_string(),
                        content: ApiContent::Text(msg.content),
                    });
                }
            }
            Role::Tool => {
                // Tool results go as content blocks in a user message.
                // Anthropic expects tool_result blocks to be in the user role.
                let block = ContentBlock::ToolResult {
                    tool_use_id: msg.tool_call_id.unwrap_or_default(),
                    content: msg.content,
                };
                // If the last message is already a user message with blocks,
                // append to it (multiple tool results in one turn).
                if let Some(last) = api_messages.last_mut()
                    && last.role == "user"
                {
                    match &mut last.content {
                        ApiContent::Blocks(blocks) => {
                            blocks.push(block);
                            continue;
                        }
                        ApiContent::Text(_) => {}
                    }
                }
                api_messages.push(ApiMessage {
                    role: "user".to_string(),
                    content: ApiContent::Blocks(vec![block]),
                });
            }
        }
    }

    (system_text, api_messages)
}

fn convert_tools(tools: Vec<ToolDefinition>) -> Vec<ApiTool> {
    tools
        .into_iter()
        .map(|t| ApiTool {
            name: t.name,
            description: t.description,
            input_schema: t.parameters,
        })
        .collect()
}

fn parse_stop_reason(reason: Option<&str>) -> FinishReason {
    match reason {
        Some("end_turn") | Some("stop") => FinishReason::Stop,
        Some("max_tokens") => FinishReason::Length,
        Some("tool_use") => FinishReason::ToolUse,
        _ => FinishReason::Unknown,
    }
}

fn extract_content(blocks: &[ContentBlock]) -> (Option<String>, Vec<ToolCall>) {
    let mut text_parts: Vec<&str> = Vec::new();
    let mut tool_calls = Vec::new();

    for block in blocks {
        match block {
            ContentBlock::Text { text } => {
                text_parts.push(text);
            }
            ContentBlock::ToolUse { id, name, input } => {
                tool_calls.push(ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: input.clone(),
                });
            }
            ContentBlock::ToolResult { .. } => {}
        }
    }

    let text = if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join(""))
    };

    (text, tool_calls)
}

// -- LlmProvider implementation --

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn model_name(&self) -> &str {
        &self.config.model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        // OAuth (Max subscription) users have zero per-token cost
        if self.is_oauth() {
            return (Decimal::ZERO, Decimal::ZERO);
        }
        costs::model_cost(&self.active_model_name()).unwrap_or_else(costs::default_cost)
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let (system, messages) = convert_messages(req.messages);

        let request = MessagesRequest {
            model: self.active_model_name(),
            messages,
            max_tokens: req.max_tokens.unwrap_or(4096),
            system,
            temperature: req.temperature,
            tools: None,
            tool_choice: None,
        };

        let response: MessagesResponse = self.send_request(&request).await?;

        let (text, _) = extract_content(&response.content);

        Ok(CompletionResponse {
            content: text.unwrap_or_default(),
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
            finish_reason: parse_stop_reason(response.stop_reason.as_deref()),
            response_id: None,
        })
    }

    async fn complete_with_tools(
        &self,
        req: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let (system, messages) = convert_messages(req.messages);
        let tools = convert_tools(req.tools);

        let tool_choice = req.tool_choice.map(|choice| {
            let choice_type = match choice.as_str() {
                "required" => "any",
                "none" => "none",
                _ => "auto",
            };
            ApiToolChoice {
                choice_type: choice_type.to_string(),
            }
        });

        let request = MessagesRequest {
            model: self.active_model_name(),
            messages,
            max_tokens: req.max_tokens.unwrap_or(4096),
            system,
            temperature: req.temperature,
            tools: if tools.is_empty() { None } else { Some(tools) },
            tool_choice,
        };

        let response: MessagesResponse = self.send_request(&request).await?;

        let (text, tool_calls) = extract_content(&response.content);
        let finish_reason = parse_stop_reason(response.stop_reason.as_deref());

        Ok(ToolCompletionResponse {
            content: text,
            tool_calls,
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
            finish_reason,
            response_id: None,
        })
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let url = format!("{}/v1/models", API_BASE);

        let builder = self
            .client
            .get(&url)
            .header("anthropic-version", API_VERSION);
        let builder = self.auth_headers(builder);

        let resp = builder.send().await.map_err(|e| LlmError::RequestFailed {
            provider: PROVIDER_NAME.to_string(),
            reason: format!("Failed to fetch models: {}", e),
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("HTTP {}: {}", status, body),
            });
        }

        #[derive(Deserialize)]
        struct ModelEntry {
            id: String,
        }
        #[derive(Deserialize)]
        struct ModelsResponse {
            data: Vec<ModelEntry>,
        }

        let body: ModelsResponse = resp.json().await.map_err(|e| LlmError::InvalidResponse {
            provider: PROVIDER_NAME.to_string(),
            reason: format!("JSON parse error: {}", e),
        })?;

        Ok(body.data.into_iter().map(|m| m.id).collect())
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        Ok(ModelMetadata {
            id: self.active_model_name(),
            context_length: None,
        })
    }

    fn active_model_name(&self) -> String {
        self.active_model
            .read()
            .expect("active_model lock poisoned")
            .clone()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        let mut guard = self
            .active_model
            .write()
            .expect("active_model lock poisoned");
        *guard = model.to_string();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::provider::ToolCall as ProviderToolCall;

    #[test]
    fn test_convert_messages_system_extracted() {
        let messages = vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("Hello"),
        ];

        let (system, api_msgs) = convert_messages(messages);
        assert_eq!(system, Some("You are helpful.".to_string()));
        assert_eq!(api_msgs.len(), 1);
        assert_eq!(api_msgs[0].role, "user");
    }

    #[test]
    fn test_convert_messages_multiple_system_merged() {
        let messages = vec![
            ChatMessage::system("First system."),
            ChatMessage::system("Second system."),
            ChatMessage::user("Hello"),
        ];

        let (system, api_msgs) = convert_messages(messages);
        assert_eq!(system, Some("First system.\n\nSecond system.".to_string()));
        assert_eq!(api_msgs.len(), 1);
    }

    #[test]
    fn test_convert_messages_tool_calls() {
        let tool_calls = vec![ProviderToolCall {
            id: "call_1".to_string(),
            name: "echo".to_string(),
            arguments: serde_json::json!({"message": "hi"}),
        }];

        let messages = vec![
            ChatMessage::user("test"),
            ChatMessage::assistant_with_tool_calls(Some("Let me check.".into()), tool_calls),
            ChatMessage::tool_result("call_1", "echo", "hi"),
        ];

        let (_, api_msgs) = convert_messages(messages);
        assert_eq!(api_msgs.len(), 3);

        // Assistant message should have content blocks
        match &api_msgs[1].content {
            ApiContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 2);
                assert!(
                    matches!(&blocks[0], ContentBlock::Text { text } if text == "Let me check.")
                );
                assert!(matches!(&blocks[1], ContentBlock::ToolUse { name, .. } if name == "echo"));
            }
            _ => panic!("Expected blocks for assistant with tool calls"),
        }

        // Tool result should be user message with blocks
        assert_eq!(api_msgs[2].role, "user");
        match &api_msgs[2].content {
            ApiContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 1);
                assert!(
                    matches!(&blocks[0], ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == "call_1")
                );
            }
            _ => panic!("Expected blocks for tool result"),
        }
    }

    #[test]
    fn test_convert_messages_multiple_tool_results_merged() {
        let messages = vec![
            ChatMessage::tool_result("call_1", "tool_a", "result A"),
            ChatMessage::tool_result("call_2", "tool_b", "result B"),
        ];

        let (_, api_msgs) = convert_messages(messages);
        // Both tool results should be merged into a single user message
        assert_eq!(api_msgs.len(), 1);
        assert_eq!(api_msgs[0].role, "user");
        match &api_msgs[0].content {
            ApiContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 2);
            }
            _ => panic!("Expected blocks"),
        }
    }

    #[test]
    fn test_convert_tools() {
        let tools = vec![ToolDefinition {
            name: "search".to_string(),
            description: "Search for things".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                }
            }),
        }];

        let api_tools = convert_tools(tools);
        assert_eq!(api_tools.len(), 1);
        assert_eq!(api_tools[0].name, "search");
        assert_eq!(api_tools[0].description, "Search for things");
    }

    #[test]
    fn test_parse_stop_reason() {
        assert_eq!(parse_stop_reason(Some("end_turn")), FinishReason::Stop);
        assert_eq!(parse_stop_reason(Some("stop")), FinishReason::Stop);
        assert_eq!(parse_stop_reason(Some("max_tokens")), FinishReason::Length);
        assert_eq!(parse_stop_reason(Some("tool_use")), FinishReason::ToolUse);
        assert_eq!(parse_stop_reason(None), FinishReason::Unknown);
        assert_eq!(parse_stop_reason(Some("unknown")), FinishReason::Unknown);
    }

    #[test]
    fn test_extract_content_text_only() {
        let blocks = vec![ContentBlock::Text {
            text: "Hello world".to_string(),
        }];

        let (text, tool_calls) = extract_content(&blocks);
        assert_eq!(text, Some("Hello world".to_string()));
        assert!(tool_calls.is_empty());
    }

    #[test]
    fn test_extract_content_tool_use() {
        let blocks = vec![
            ContentBlock::Text {
                text: "Let me search.".to_string(),
            },
            ContentBlock::ToolUse {
                id: "call_1".to_string(),
                name: "search".to_string(),
                input: serde_json::json!({"query": "test"}),
            },
        ];

        let (text, tool_calls) = extract_content(&blocks);
        assert_eq!(text, Some("Let me search.".to_string()));
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].name, "search");
        assert_eq!(tool_calls[0].id, "call_1");
    }

    #[test]
    fn test_extract_content_tool_use_only() {
        let blocks = vec![ContentBlock::ToolUse {
            id: "call_1".to_string(),
            name: "echo".to_string(),
            input: serde_json::json!({"message": "hi"}),
        }];

        let (text, tool_calls) = extract_content(&blocks);
        assert!(text.is_none());
        assert_eq!(tool_calls.len(), 1);
    }

    #[test]
    fn test_oauth_zero_cost() {
        let config = AnthropicDirectConfig {
            auth: AnthropicAuth::OAuthToken {
                access_token: SecretString::from("test-token".to_string()),
                refresh_token: None,
            },
            model: "claude-sonnet-4-20250514".to_string(),
            max_retries: 3,
        };

        let provider = AnthropicProvider::new(config);
        let (input, output) = provider.cost_per_token();
        assert_eq!(input, Decimal::ZERO);
        assert_eq!(output, Decimal::ZERO);
    }

    #[test]
    fn test_api_key_cost() {
        let config = AnthropicDirectConfig {
            auth: AnthropicAuth::ApiKey(SecretString::from("sk-test".to_string())),
            model: "claude-sonnet-4-20250514".to_string(),
            max_retries: 3,
        };

        let provider = AnthropicProvider::new(config);
        let (input, output) = provider.cost_per_token();
        assert!(input > Decimal::ZERO);
        assert!(output > Decimal::ZERO);
    }

    #[test]
    fn test_messages_request_serialization() {
        let request = MessagesRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            messages: vec![ApiMessage {
                role: "user".to_string(),
                content: ApiContent::Text("Hello".to_string()),
            }],
            max_tokens: 1024,
            system: Some("You are helpful.".to_string()),
            temperature: None,
            tools: None,
            tool_choice: None,
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["model"], "claude-sonnet-4-20250514");
        assert_eq!(json["max_tokens"], 1024);
        assert_eq!(json["system"], "You are helpful.");
        assert!(json.get("temperature").is_none());
        assert!(json.get("tools").is_none());
    }

    #[test]
    fn test_messages_response_deserialization() {
        let json = serde_json::json!({
            "content": [
                {"type": "text", "text": "Hello!"}
            ],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5
            }
        });

        let resp: MessagesResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.content.len(), 1);
        assert_eq!(resp.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(resp.usage.input_tokens, 10);
        assert_eq!(resp.usage.output_tokens, 5);
    }

    #[test]
    fn test_tool_use_response_deserialization() {
        let json = serde_json::json!({
            "content": [
                {"type": "text", "text": "Let me search."},
                {
                    "type": "tool_use",
                    "id": "toolu_abc123",
                    "name": "search",
                    "input": {"query": "test"}
                }
            ],
            "stop_reason": "tool_use",
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50
            }
        });

        let resp: MessagesResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.content.len(), 2);
        assert_eq!(resp.stop_reason.as_deref(), Some("tool_use"));

        let (text, tool_calls) = extract_content(&resp.content);
        assert_eq!(text, Some("Let me search.".to_string()));
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "toolu_abc123");
        assert_eq!(tool_calls[0].name, "search");
    }
}
