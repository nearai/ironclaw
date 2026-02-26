//! Anthropic OAuth provider (direct HTTP, `Authorization: Bearer`).
//!
//! This provider exists because the `rig-core` Anthropic client hardcodes the
//! `x-api-key` header, which is rejected by Anthropic's OAuth tokens from
//! `claude login`. OAuth tokens require `Authorization: Bearer <token>` instead.
//!
//! Pattern follows `nearai_chat.rs`: direct HTTP calls via `reqwest::Client`.

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::config::AnthropicDirectConfig;
use crate::error::LlmError;
use crate::llm::costs;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse,
};

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
/// OAuth beta requires 2023-06-01; the 2024-10-22 version is not valid with the beta flag.
const ANTHROPIC_API_VERSION: &str = "2023-06-01";
/// Required beta flag to enable OAuth Bearer auth on api.anthropic.com.
/// Without this header, the API returns 401 "OAuth authentication is currently not supported."
const ANTHROPIC_OAUTH_BETA: &str = "oauth-2025-04-20";
const DEFAULT_MAX_TOKENS: u32 = 8192;

/// Anthropic provider using OAuth Bearer authentication.
pub struct AnthropicOAuthProvider {
    client: Client,
    token: SecretString,
    model: String,
    base_url: Option<String>,
    active_model: std::sync::RwLock<String>,
}

impl AnthropicOAuthProvider {
    pub fn new(config: &AnthropicDirectConfig) -> Result<Self, LlmError> {
        let token = config
            .oauth_token
            .clone()
            .ok_or_else(|| LlmError::AuthFailed {
                provider: "anthropic_oauth".to_string(),
            })?;

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: "anthropic_oauth".to_string(),
                reason: format!("Failed to build HTTP client: {}", e),
            })?;

        let active_model = std::sync::RwLock::new(config.model.clone());

        Ok(Self {
            client,
            token,
            model: config.model.clone(),
            base_url: config.base_url.clone(),
            active_model,
        })
    }

    fn api_url(&self) -> String {
        if let Some(ref base) = self.base_url {
            let base = base.trim_end_matches('/');
            format!("{}/v1/messages", base)
        } else {
            ANTHROPIC_API_URL.to_string()
        }
    }

    async fn send_request<R: for<'de> Deserialize<'de>>(
        &self,
        body: &AnthropicRequest,
    ) -> Result<R, LlmError> {
        let url = self.api_url();

        tracing::debug!("Sending request to Anthropic OAuth: {}", url);

        let response = self
            .client
            .post(&url)
            .bearer_auth(self.token.expose_secret())
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .header("anthropic-beta", ANTHROPIC_OAUTH_BETA)
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: "anthropic_oauth".to_string(),
                reason: e.to_string(),
            })?;

        let status = response.status();

        if !status.is_success() {
            // Parse Retry-After header before consuming the body.
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(std::time::Duration::from_secs);

            let response_text = response.text().await.unwrap_or_default();

            if status.as_u16() == 401 {
                return Err(LlmError::AuthFailed {
                    provider: "anthropic_oauth".to_string(),
                });
            }
            if status.as_u16() == 429 {
                return Err(LlmError::RateLimited {
                    provider: "anthropic_oauth".to_string(),
                    retry_after,
                });
            }
            let truncated = crate::agent::truncate_for_preview(&response_text, 512);
            return Err(LlmError::RequestFailed {
                provider: "anthropic_oauth".to_string(),
                reason: format!("HTTP {}: {}", status, truncated),
            });
        }

        let response_text = response.text().await.map_err(|e| LlmError::RequestFailed {
            provider: "anthropic_oauth".to_string(),
            reason: format!("Failed to read response body: {}", e),
        })?;

        tracing::debug!(
            "Anthropic OAuth response: status={}, bytes={}",
            status,
            response_text.len()
        );

        serde_json::from_str(&response_text).map_err(|e| {
            let truncated = crate::agent::truncate_for_preview(&response_text, 512);
            LlmError::InvalidResponse {
                provider: "anthropic_oauth".to_string(),
                reason: format!("JSON parse error: {}. Raw: {}", e, truncated),
            }
        })
    }
}

#[async_trait]
impl LlmProvider for AnthropicOAuthProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let model = req.model.unwrap_or_else(|| self.active_model_name());
        let (system, messages) = convert_messages(req.messages);

        let request = AnthropicRequest {
            model,
            messages,
            system,
            max_tokens: req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            temperature: req.temperature,
            tools: None,
            tool_choice: None,
        };

        let response: AnthropicResponse = self.send_request(&request).await?;
        let (content, _tool_calls) = extract_response_content(&response);

        let finish_reason = match response.stop_reason.as_deref() {
            Some("end_turn") | Some("stop") => FinishReason::Stop,
            Some("max_tokens") => FinishReason::Length,
            Some("tool_use") => FinishReason::ToolUse,
            _ => FinishReason::Unknown,
        };

        Ok(CompletionResponse {
            content: content.unwrap_or_default(),
            finish_reason,
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
        })
    }

    async fn complete_with_tools(
        &self,
        req: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let model = req.model.unwrap_or_else(|| self.active_model_name());
        let (system, messages) = convert_messages(req.messages);

        let tools: Vec<AnthropicTool> = req
            .tools
            .into_iter()
            .map(|t| AnthropicTool {
                name: t.name,
                description: t.description,
                input_schema: t.parameters,
            })
            .collect();

        // Map tool_choice from OpenAI format to Anthropic format
        let tool_choice = req.tool_choice.map(|tc| match tc.as_str() {
            "auto" => AnthropicToolChoice {
                choice_type: "auto".to_string(),
                name: None,
            },
            "required" => AnthropicToolChoice {
                choice_type: "any".to_string(),
                name: None,
            },
            "none" => AnthropicToolChoice {
                choice_type: "none".to_string(),
                name: None,
            },
            specific => AnthropicToolChoice {
                choice_type: "tool".to_string(),
                name: Some(specific.to_string()),
            },
        });

        let request = AnthropicRequest {
            model,
            messages,
            system,
            max_tokens: req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            temperature: req.temperature,
            tools: if tools.is_empty() { None } else { Some(tools) },
            tool_choice,
        };

        let response: AnthropicResponse = self.send_request(&request).await?;
        let (content, tool_calls) = extract_response_content(&response);

        let finish_reason = match response.stop_reason.as_deref() {
            Some("end_turn") | Some("stop") => FinishReason::Stop,
            Some("max_tokens") => FinishReason::Length,
            Some("tool_use") => FinishReason::ToolUse,
            _ => {
                if !tool_calls.is_empty() {
                    FinishReason::ToolUse
                } else {
                    FinishReason::Unknown
                }
            }
        };

        Ok(ToolCompletionResponse {
            content,
            tool_calls,
            finish_reason,
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
        })
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        let model = self.active_model_name();
        costs::model_cost(&model).unwrap_or_else(costs::default_cost)
    }

    fn active_model_name(&self) -> String {
        match self.active_model.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        match self.active_model.write() {
            Ok(mut guard) => {
                *guard = model.to_string();
            }
            Err(poisoned) => {
                *poisoned.into_inner() = model.to_string();
            }
        }
        Ok(())
    }
}

// --- Anthropic Messages API types ---

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<AnthropicToolChoice>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: AnthropicContent,
}

/// Anthropic content can be a simple string or a list of content blocks.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum AnthropicContent {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum AnthropicContentBlock {
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
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct AnthropicToolChoice {
    #[serde(rename = "type")]
    choice_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicResponseBlock>,
    #[serde(default)]
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum AnthropicResponseBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

/// Convert ChatMessage list to Anthropic format.
///
/// Extracts system messages to the top-level `system` parameter (Anthropic
/// doesn't allow system messages in the `messages` array). Tool-call/tool-result
/// pairs are converted to content blocks.
fn convert_messages(messages: Vec<ChatMessage>) -> (Option<String>, Vec<AnthropicMessage>) {
    let mut system_parts: Vec<String> = Vec::new();
    let mut anthropic_msgs: Vec<AnthropicMessage> = Vec::new();

    for msg in messages {
        match msg.role {
            Role::System => {
                if !msg.content.is_empty() {
                    system_parts.push(msg.content);
                }
            }
            Role::User => {
                anthropic_msgs.push(AnthropicMessage {
                    role: "user".to_string(),
                    content: AnthropicContent::Text(msg.content),
                });
            }
            Role::Assistant => {
                if let Some(tool_calls) = msg.tool_calls {
                    // Assistant message with tool calls → content blocks
                    let mut blocks: Vec<AnthropicContentBlock> = Vec::new();
                    if !msg.content.is_empty() {
                        blocks.push(AnthropicContentBlock::Text { text: msg.content });
                    }
                    for tc in tool_calls {
                        blocks.push(AnthropicContentBlock::ToolUse {
                            id: tc.id,
                            name: tc.name,
                            input: tc.arguments,
                        });
                    }
                    anthropic_msgs.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: AnthropicContent::Blocks(blocks),
                    });
                } else {
                    anthropic_msgs.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: AnthropicContent::Text(msg.content),
                    });
                }
            }
            Role::Tool => {
                // Tool results go into a user message with tool_result blocks
                let block = AnthropicContentBlock::ToolResult {
                    tool_use_id: msg.tool_call_id.unwrap_or_default(),
                    content: msg.content,
                };
                // If the last message is already a user message with blocks,
                // append to it (Anthropic requires consecutive tool results
                // in one user message).
                if let Some(last) = anthropic_msgs.last_mut()
                    && last.role == "user"
                    && let AnthropicContent::Blocks(ref mut blocks) = last.content
                {
                    blocks.push(block);
                    continue;
                }
                anthropic_msgs.push(AnthropicMessage {
                    role: "user".to_string(),
                    content: AnthropicContent::Blocks(vec![block]),
                });
            }
        }
    }

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };

    (system, anthropic_msgs)
}

/// Extract text content and tool calls from an Anthropic response.
fn extract_response_content(response: &AnthropicResponse) -> (Option<String>, Vec<ToolCall>) {
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    for block in &response.content {
        match block {
            AnthropicResponseBlock::Text { text } => {
                text_parts.push(text.clone());
            }
            AnthropicResponseBlock::ToolUse { id, name, input } => {
                tool_calls.push(ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: input.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_messages_extracts_system() {
        let messages = vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("Hello"),
        ];
        let (system, msgs) = convert_messages(messages);
        assert_eq!(system, Some("You are helpful.".to_string()));
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
    }

    #[test]
    fn test_convert_messages_multiple_systems() {
        let messages = vec![
            ChatMessage::system("System 1"),
            ChatMessage::system("System 2"),
            ChatMessage::user("Hello"),
        ];
        let (system, msgs) = convert_messages(messages);
        assert_eq!(system, Some("System 1\n\nSystem 2".to_string()));
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn test_convert_messages_tool_calls() {
        let tool_calls = vec![ToolCall {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"q": "test"}),
        }];
        let messages = vec![
            ChatMessage::user("Search for test"),
            ChatMessage::assistant_with_tool_calls(Some("Let me search.".to_string()), tool_calls),
            ChatMessage::tool_result("call_1", "search", "found it"),
        ];
        let (system, msgs) = convert_messages(messages);
        assert!(system.is_none());
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
        // Tool result should be a user message
        assert_eq!(msgs[2].role, "user");
    }

    #[test]
    fn test_extract_response_text_only() {
        let response = AnthropicResponse {
            content: vec![AnthropicResponseBlock::Text {
                text: "Hello!".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: AnthropicUsage {
                input_tokens: 10,
                output_tokens: 5,
            },
        };
        let (content, tool_calls) = extract_response_content(&response);
        assert_eq!(content, Some("Hello!".to_string()));
        assert!(tool_calls.is_empty());
    }

    #[test]
    fn test_extract_response_with_tool_use() {
        let response = AnthropicResponse {
            content: vec![
                AnthropicResponseBlock::Text {
                    text: "Let me search.".to_string(),
                },
                AnthropicResponseBlock::ToolUse {
                    id: "call_1".to_string(),
                    name: "search".to_string(),
                    input: serde_json::json!({"q": "test"}),
                },
            ],
            stop_reason: Some("tool_use".to_string()),
            usage: AnthropicUsage {
                input_tokens: 20,
                output_tokens: 15,
            },
        };
        let (content, tool_calls) = extract_response_content(&response);
        assert_eq!(content, Some("Let me search.".to_string()));
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].name, "search");
    }

    #[test]
    fn test_codex_token_extraction_empty_json() {
        use crate::config::extract_codex_oauth_token;
        use crate::config::helpers::ENV_MUTEX;
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        // Write an auth.json with no usable token fields.
        let dir = tempfile::tempdir().expect("create temp dir");
        let auth_path = dir.path().join("auth.json");
        std::fs::write(&auth_path, r#"{"auth_mode":"none"}"#).expect("write auth.json");
        // SAFETY: Under ENV_MUTEX.
        unsafe { std::env::set_var("CODEX_HOME", dir.path().to_str().unwrap()) };
        let result = extract_codex_oauth_token();
        unsafe { std::env::remove_var("CODEX_HOME") };
        // CODEX_HOME path had no token fields; the function falls back to
        // ~/.codex/ which may or may not exist. Just verify no panic.
        let _ = result;
    }

    #[test]
    fn test_codex_token_extraction_valid_json() {
        use crate::config::helpers::ENV_MUTEX;
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        let dir = tempfile::tempdir().expect("create temp dir");
        let auth_path = dir.path().join("auth.json");
        std::fs::write(&auth_path, r#"{"token":"test-tok-123"}"#).expect("write auth.json");
        // SAFETY: Under ENV_MUTEX.
        unsafe { std::env::set_var("CODEX_HOME", dir.path().to_str().unwrap()) };
        let result = crate::config::extract_codex_oauth_token();
        unsafe { std::env::remove_var("CODEX_HOME") };
        assert_eq!(result, Some("test-tok-123".to_string()));
    }

    #[test]
    fn test_codex_token_extraction_nested_tokens_format() {
        use crate::config::helpers::ENV_MUTEX;
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        let dir = tempfile::tempdir().expect("create temp dir");
        let auth_path = dir.path().join("auth.json");
        let json = r#"{
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": null,
            "tokens": {
                "access_token": "eyJ-nested-tok",
                "refresh_token": "rt_xxx"
            }
        }"#;
        std::fs::write(&auth_path, json).expect("write auth.json");
        // SAFETY: Under ENV_MUTEX.
        unsafe { std::env::set_var("CODEX_HOME", dir.path().to_str().unwrap()) };
        let result = crate::config::extract_codex_oauth_token();
        unsafe { std::env::remove_var("CODEX_HOME") };
        assert_eq!(result, Some("eyJ-nested-tok".to_string()));
    }
}
