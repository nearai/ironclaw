//! Anthropic LLM provider implementation.

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use crate::config::AnthropicConfig;
use crate::error::LlmError;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse,
};

/// Anthropic API provider.
pub struct AnthropicProvider {
    client: Client,
    config: AnthropicConfig,
    base_url: String,
}

impl AnthropicProvider {
    /// Create a new Anthropic provider.
    pub fn new(config: AnthropicConfig) -> Self {
        let base_url = config
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.anthropic.com/v1".to_string());

        Self {
            client: Client::new(),
            config,
            base_url,
        }
    }

    fn build_messages(&self, messages: &[ChatMessage]) -> (Option<String>, Vec<AnthropicMessage>) {
        let mut system_message = None;
        let mut anthropic_messages = Vec::new();

        for msg in messages {
            match msg.role {
                Role::System => {
                    // Anthropic uses a separate system parameter
                    system_message = Some(msg.content.clone());
                }
                Role::User => {
                    anthropic_messages.push(AnthropicMessage {
                        role: "user".to_string(),
                        content: AnthropicContent::Text(msg.content.clone()),
                    });
                }
                Role::Assistant => {
                    anthropic_messages.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: AnthropicContent::Text(msg.content.clone()),
                    });
                }
                Role::Tool => {
                    // Tool results in Anthropic format
                    anthropic_messages.push(AnthropicMessage {
                        role: "user".to_string(),
                        content: AnthropicContent::ToolResult {
                            tool_use_id: msg.tool_call_id.clone().unwrap_or_default(),
                            content: msg.content.clone(),
                        },
                    });
                }
            }
        }

        (system_message, anthropic_messages)
    }
}

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
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

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum AnthropicContent {
    Text(String),
    #[serde(rename_all = "snake_case")]
    ToolResult {
        #[serde(rename = "type")]
        tool_use_id: String,
        content: String,
    },
    Blocks(Vec<AnthropicContentBlock>),
}

#[derive(Debug, Serialize, Deserialize)]
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
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContentBlock>,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct AnthropicError {
    error: AnthropicErrorDetail,
}

#[derive(Debug, Deserialize)]
struct AnthropicErrorDetail {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
}

fn parse_finish_reason(reason: Option<&str>) -> FinishReason {
    match reason {
        Some("end_turn") | Some("stop_sequence") => FinishReason::Stop,
        Some("max_tokens") => FinishReason::Length,
        Some("tool_use") => FinishReason::ToolUse,
        _ => FinishReason::Unknown,
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn model_name(&self) -> &str {
        &self.config.model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        // Pricing for Claude models (per 1M tokens, converted to per token)
        match self.config.model.as_str() {
            m if m.contains("opus") => {
                (dec!(0.000015), dec!(0.000075)) // $15/$75 per 1M
            }
            m if m.contains("sonnet") => {
                (dec!(0.000003), dec!(0.000015)) // $3/$15 per 1M
            }
            m if m.contains("haiku") => {
                (dec!(0.00000025), dec!(0.00000125)) // $0.25/$1.25 per 1M
            }
            _ => (dec!(0.000003), dec!(0.000015)), // Default to Sonnet pricing
        }
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let (system, messages) = self.build_messages(&request.messages);

        let anthropic_request = AnthropicRequest {
            model: self.config.model.clone(),
            messages,
            max_tokens: request.max_tokens.unwrap_or(4096),
            system,
            temperature: request.temperature,
            tools: None,
            tool_choice: None,
        };

        let response = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", self.config.api_key.expose_secret())
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&anthropic_request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error: AnthropicError =
                response
                    .json()
                    .await
                    .map_err(|e| LlmError::InvalidResponse {
                        provider: "anthropic".to_string(),
                        reason: format!("Failed to parse error response: {}", e),
                    })?;
            return Err(LlmError::RequestFailed {
                provider: "anthropic".to_string(),
                reason: error.error.message,
            });
        }

        let anthropic_response: AnthropicResponse = response.json().await?;

        // Extract text content
        let content = anthropic_response
            .content
            .iter()
            .filter_map(|block| match block {
                AnthropicContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(CompletionResponse {
            content,
            input_tokens: anthropic_response.usage.input_tokens,
            output_tokens: anthropic_response.usage.output_tokens,
            finish_reason: parse_finish_reason(anthropic_response.stop_reason.as_deref()),
        })
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let (system, messages) = self.build_messages(&request.messages);

        let tools: Vec<AnthropicTool> = request
            .tools
            .iter()
            .map(|t| AnthropicTool {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.parameters.clone(),
            })
            .collect();

        let tool_choice = request.tool_choice.as_ref().map(|c| AnthropicToolChoice {
            choice_type: match c.as_str() {
                "auto" => "auto".to_string(),
                "required" => "any".to_string(),
                "none" => "none".to_string(),
                _ => "auto".to_string(),
            },
        });

        let anthropic_request = AnthropicRequest {
            model: self.config.model.clone(),
            messages,
            max_tokens: request.max_tokens.unwrap_or(4096),
            system,
            temperature: None,
            tools: Some(tools),
            tool_choice,
        };

        let response = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", self.config.api_key.expose_secret())
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&anthropic_request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error: AnthropicError =
                response
                    .json()
                    .await
                    .map_err(|e| LlmError::InvalidResponse {
                        provider: "anthropic".to_string(),
                        reason: format!("Failed to parse error response: {}", e),
                    })?;
            return Err(LlmError::RequestFailed {
                provider: "anthropic".to_string(),
                reason: error.error.message,
            });
        }

        let anthropic_response: AnthropicResponse = response.json().await?;

        // Extract text and tool calls
        let mut content = None;
        let mut tool_calls = Vec::new();

        for block in anthropic_response.content {
            match block {
                AnthropicContentBlock::Text { text } => {
                    content = Some(text);
                }
                AnthropicContentBlock::ToolUse { id, name, input } => {
                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments: input,
                    });
                }
                _ => {}
            }
        }

        Ok(ToolCompletionResponse {
            content,
            tool_calls,
            input_tokens: anthropic_response.usage.input_tokens,
            output_tokens: anthropic_response.usage.output_tokens,
            finish_reason: parse_finish_reason(anthropic_response.stop_reason.as_deref()),
        })
    }
}
