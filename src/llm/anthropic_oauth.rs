//! Anthropic OAuth provider (direct HTTP, `Authorization: Bearer`).
//!
//! This provider exists because the `rig-core` Anthropic client hardcodes the
//! `x-api-key` header, which is rejected by Anthropic's OAuth tokens from
//! `claude login`. OAuth tokens require `Authorization: Bearer <token>` instead.
//!
//! Pattern follows `nearai_chat.rs`: direct HTTP calls via `reqwest::Client`.

use std::collections::HashSet;

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::llm::config::RegistryProviderConfig;
use crate::llm::costs;
use crate::llm::error::LlmError;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse, strip_unsupported_completion_params,
    strip_unsupported_tool_params,
};
const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
/// OAuth beta requires 2023-06-01; the 2024-10-22 version is not valid with the beta flag.
const ANTHROPIC_API_VERSION: &str = "2023-06-01";
/// Required beta flag to enable OAuth Bearer auth on api.anthropic.com.
/// Without this header, the API returns 401 "OAuth authentication is currently not supported."
const ANTHROPIC_OAUTH_BETA: &str =
    "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14";
const CLAUDE_CLI_USER_AGENT: &str = "claude-cli/2.1.2 (external, cli)";
const CLAUDE_CLI_X_APP: &str = "cli";
const CLAUDE_CODE_SYSTEM_PREFIX: &str = "You are Claude Code, Anthropic's official CLI for Claude.";
const DEFAULT_MAX_TOKENS: u32 = 8192;

/// Anthropic provider using OAuth Bearer authentication.
pub struct AnthropicOAuthProvider {
    client: Client,
    /// OAuth token, wrapped in RwLock so it can be updated after a successful
    /// Keychain refresh (fixes #1136: stale token reuse after expiry).
    token: std::sync::RwLock<SecretString>,
    model: String,
    base_url: Option<String>,
    active_model: std::sync::RwLock<String>,
    /// Parameter names that this provider does not support.
    unsupported_params: HashSet<String>,
}

impl AnthropicOAuthProvider {
    pub fn new(config: &RegistryProviderConfig) -> Result<Self, LlmError> {
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
        let base_url = if config.base_url.is_empty() {
            None
        } else {
            Some(config.base_url.clone())
        };

        let unsupported_params: HashSet<String> =
            config.unsupported_params.iter().cloned().collect();

        Ok(Self {
            client,
            token: std::sync::RwLock::new(token),
            model: config.model.clone(),
            base_url,
            active_model,
            unsupported_params,
        })
    }

    /// Strip unsupported fields from a `CompletionRequest` in place.
    fn strip_unsupported_completion_params(&self, req: &mut CompletionRequest) {
        strip_unsupported_completion_params(&self.unsupported_params, req);
    }

    /// Strip unsupported fields from a `ToolCompletionRequest` in place.
    fn strip_unsupported_tool_params(&self, req: &mut ToolCompletionRequest) {
        strip_unsupported_tool_params(&self.unsupported_params, req);
    }

    fn api_url(&self) -> String {
        if let Some(ref base) = self.base_url {
            let base = base.trim_end_matches('/');
            format!("{}/v1/messages", base)
        } else {
            ANTHROPIC_API_URL.to_string()
        }
    }

    /// Read the current token from the RwLock.
    fn current_token(&self) -> String {
        match self.token.read() {
            Ok(guard) => guard.expose_secret().to_string(),
            Err(poisoned) => poisoned.into_inner().expose_secret().to_string(),
        }
    }

    /// Update the stored token after a successful Keychain refresh.
    fn update_token(&self, new_token: SecretString) {
        match self.token.write() {
            Ok(mut guard) => *guard = new_token,
            Err(poisoned) => *poisoned.into_inner() = new_token,
        }
    }

    /// Build system prompt blocks with required Claude Code identity prefix.
    /// OAuth tokens require the system prompt to include this identity block.
    fn build_system_blocks(system: Option<String>) -> Option<Vec<SystemBlock>> {
        let mut blocks = vec![SystemBlock {
            block_type: "text".to_string(),
            text: CLAUDE_CODE_SYSTEM_PREFIX.to_string(),
        }];
        if let Some(s) = system {
            blocks.push(SystemBlock {
                block_type: "text".to_string(),
                text: s,
            });
        }
        Some(blocks)
    }

    /// Validate temperature for thinking-enabled requests.
    /// When thinking is enabled, temperature must be None or 1.0.
    fn validate_temperature(temperature: Option<f32>) -> Option<f32> {
        match temperature {
            Some(t) if (t - 1.0).abs() < f32::EPSILON => Some(1.0),
            Some(t) => {
                tracing::warn!(
                    "Temperature {} is not supported when thinking is enabled, defaulting to None. \
                     Only None or 1.0 are allowed.",
                    t
                );
                None
            }
            None => None,
        }
    }

    async fn send_request<R: for<'de> Deserialize<'de>>(
        &self,
        body: &AnthropicRequest,
    ) -> Result<R, LlmError> {
        let url = self.api_url();

        tracing::debug!(url = %url, "Sending request to Anthropic OAuth");

        let response = self
            .client
            .post(&url)
            .bearer_auth(self.current_token())
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .header("anthropic-beta", ANTHROPIC_OAUTH_BETA)
            .header("user-agent", CLAUDE_CLI_USER_AGENT)
            .header("x-app", CLAUDE_CLI_X_APP)
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
            let retry_after = Some(crate::llm::retry::parse_retry_after(
                response.headers().get("retry-after"),
            ));

            let response_text = response
                .text()
                .await
                .unwrap_or_else(|e| format!("(failed to read error body: {e})"));

            if status.as_u16() == 401 {
                // OAuth tokens from `claude login` expire in ~8-12h. Attempt
                // to re-extract a fresh token from the OS credential store
                // (macOS Keychain / Linux credentials file) before giving up.
                //
                // Brief delay to give Claude Code time to complete its async
                // Keychain refresh write (fixes race in #1136).
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                if let Some(fresh) = crate::config::ClaudeCodeConfig::extract_oauth_token() {
                    let fresh_token = SecretString::from(fresh);
                    // Retry once with the refreshed token
                    let retry = self
                        .client
                        .post(&url)
                        .bearer_auth(fresh_token.expose_secret())
                        .header("anthropic-version", ANTHROPIC_API_VERSION)
                        .header("anthropic-beta", ANTHROPIC_OAUTH_BETA)
                        .header("user-agent", CLAUDE_CLI_USER_AGENT)
                        .header("x-app", CLAUDE_CLI_X_APP)
                        .header("Content-Type", "application/json")
                        .json(body)
                        .send()
                        .await
                        .map_err(|e| LlmError::RequestFailed {
                            provider: "anthropic_oauth".to_string(),
                            reason: e.to_string(),
                        })?;
                    if retry.status().is_success() {
                        // Persist the refreshed token so subsequent requests
                        // don't hit 401 again (fixes #1136).
                        self.update_token(fresh_token);
                        tracing::info!("Anthropic OAuth token refreshed from credential store");

                        let text = retry.text().await.map_err(|e| LlmError::RequestFailed {
                            provider: "anthropic_oauth".to_string(),
                            reason: format!("Failed to read response body: {}", e),
                        })?;
                        return serde_json::from_str(&text).map_err(|e| {
                            let truncated = crate::agent::truncate_for_preview(&text, 512);
                            LlmError::InvalidResponse {
                                provider: "anthropic_oauth".to_string(),
                                reason: format!("JSON parse error: {}. Raw: {}", e, truncated),
                            }
                        });
                    }
                    tracing::warn!(
                        "Anthropic OAuth 401 retry with refreshed token also failed ({})",
                        retry.status()
                    );
                }
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
    async fn complete(&self, mut req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let model = req.model.take().unwrap_or_else(|| self.active_model_name());
        self.strip_unsupported_completion_params(&mut req);
        let (system, messages) = convert_messages(req.messages);

        let request = AnthropicRequest {
            model,
            messages,
            system: Self::build_system_blocks(system),
            max_tokens: req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            temperature: Self::validate_temperature(req.temperature),
            tools: None,
            tool_choice: None,
            thinking: Some(ThinkingConfig {
                thinking_type: "adaptive".to_string(),
            }),
            stream: false,
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
            cache_creation_input_tokens: response.usage.cache_creation_input_tokens,
            cache_read_input_tokens: response.usage.cache_read_input_tokens,
        })
    }

    async fn complete_with_tools(
        &self,
        mut req: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let model = req.model.take().unwrap_or_else(|| self.active_model_name());
        self.strip_unsupported_tool_params(&mut req);
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
            system: Self::build_system_blocks(system),
            max_tokens: req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            temperature: Self::validate_temperature(req.temperature),
            tools: if tools.is_empty() { None } else { Some(tools) },
            tool_choice,
            thinking: Some(ThinkingConfig {
                thinking_type: "adaptive".to_string(),
            }),
            stream: false,
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
            cache_creation_input_tokens: response.usage.cache_creation_input_tokens,
            cache_read_input_tokens: response.usage.cache_read_input_tokens,
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
struct SystemBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: String,
}

#[derive(Debug, Serialize)]
struct ThinkingConfig {
    #[serde(rename = "type")]
    thinking_type: String,
}

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<Vec<SystemBlock>>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<AnthropicToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ThinkingConfig>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
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
    #[serde(rename = "thinking")]
    Thinking {
        #[allow(dead_code)]
        thinking: String,
        #[allow(dead_code)]
        signature: String,
    },
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
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
                let Some(tool_call_id) = msg.tool_call_id else {
                    tracing::warn!("Skipping Tool message without tool_call_id");
                    continue;
                };
                // Tool results go into a user message with tool_result blocks
                let block = AnthropicContentBlock::ToolResult {
                    tool_use_id: tool_call_id,
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
                    reasoning: None,
                });
            }
            AnthropicResponseBlock::Thinking { .. } => {
                // Skip thinking blocks - they're internal reasoning
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
            reasoning: None,
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
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
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
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        };
        let (content, tool_calls) = extract_response_content(&response);
        assert_eq!(content, Some("Let me search.".to_string()));
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].name, "search");
    }

    #[test]
    fn test_extract_response_with_thinking_blocks() {
        let response = AnthropicResponse {
            content: vec![
                AnthropicResponseBlock::Thinking {
                    thinking: "Let me consider this...".to_string(),
                    signature: "sig123".to_string(),
                },
                AnthropicResponseBlock::Text {
                    text: "Here is my answer.".to_string(),
                },
            ],
            stop_reason: Some("end_turn".to_string()),
            usage: AnthropicUsage {
                input_tokens: 10,
                output_tokens: 20,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        };
        let (content, tool_calls) = extract_response_content(&response);
        assert_eq!(content, Some("Here is my answer.".to_string()));
        assert!(tool_calls.is_empty());
    }

    #[test]
    fn test_build_system_blocks_with_existing_system() {
        let blocks =
            AnthropicOAuthProvider::build_system_blocks(Some("Custom prompt.".to_string()));
        let blocks = blocks.unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].text, CLAUDE_CODE_SYSTEM_PREFIX);
        assert_eq!(blocks[1].text, "Custom prompt.");
    }

    #[test]
    fn test_build_system_blocks_without_system() {
        let blocks = AnthropicOAuthProvider::build_system_blocks(None);
        let blocks = blocks.unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, CLAUDE_CODE_SYSTEM_PREFIX);
    }

    #[test]
    fn test_validate_temperature() {
        assert_eq!(AnthropicOAuthProvider::validate_temperature(None), None);
        assert_eq!(
            AnthropicOAuthProvider::validate_temperature(Some(1.0)),
            Some(1.0)
        );
        // Non-1.0 values should be rejected (returns None)
        assert_eq!(
            AnthropicOAuthProvider::validate_temperature(Some(0.7)),
            None
        );
    }

    /// Regression test for #1136: token field must be mutable via RwLock
    /// so that a refreshed token persists across subsequent requests.
    #[test]
    fn test_token_update_persists() {
        let original = SecretString::from("old_token".to_string());
        let token = std::sync::RwLock::new(original);

        // Read the original
        assert_eq!(token.read().unwrap().expose_secret(), "old_token");

        // Simulate a successful refresh
        let refreshed = SecretString::from("new_token".to_string());
        *token.write().unwrap() = refreshed;

        // Subsequent reads see the updated token
        assert_eq!(token.read().unwrap().expose_secret(), "new_token");
    }
}
