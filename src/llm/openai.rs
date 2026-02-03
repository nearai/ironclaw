//! OpenAI LLM provider implementation.

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use crate::config::OpenAiConfig;
use crate::error::LlmError;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse,
};

/// OpenAI API provider.
pub struct OpenAiProvider {
    client: Client,
    config: OpenAiConfig,
    base_url: String,
}

impl OpenAiProvider {
    /// Create a new OpenAI provider.
    pub fn new(config: OpenAiConfig) -> Self {
        let base_url = config
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

        Self {
            client: Client::new(),
            config,
            base_url,
        }
    }

    fn build_messages(&self, messages: &[ChatMessage]) -> Vec<OpenAiMessage> {
        messages
            .iter()
            .map(|m| OpenAiMessage {
                role: match m.role {
                    Role::System => "system".to_string(),
                    Role::User => "user".to_string(),
                    Role::Assistant => "assistant".to_string(),
                    Role::Tool => "tool".to_string(),
                },
                content: Some(m.content.clone()),
                tool_call_id: m.tool_call_id.clone(),
                name: m.name.clone(),
                tool_calls: None,
            })
            .collect()
    }
}

#[derive(Debug, Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Debug, Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiFunction,
}

#[derive(Debug, Serialize)]
struct OpenAiFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
    usage: OpenAiUsage,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OpenAiToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OpenAiFunctionCall,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OpenAiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct OpenAiError {
    error: OpenAiErrorDetail,
}

#[derive(Debug, Deserialize)]
struct OpenAiErrorDetail {
    message: String,
    #[serde(rename = "type")]
    error_type: Option<String>,
}

fn parse_finish_reason(reason: Option<&str>) -> FinishReason {
    match reason {
        Some("stop") => FinishReason::Stop,
        Some("length") => FinishReason::Length,
        Some("tool_calls") => FinishReason::ToolUse,
        Some("content_filter") => FinishReason::ContentFilter,
        _ => FinishReason::Unknown,
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn model_name(&self) -> &str {
        &self.config.model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        // Pricing for GPT-4 Turbo (per 1M tokens, converted to per token)
        // These are approximate and should be updated based on actual pricing
        match self.config.model.as_str() {
            m if m.contains("gpt-4-turbo") || m.contains("gpt-4o") => {
                (dec!(0.00001), dec!(0.00003)) // $10/$30 per 1M
            }
            m if m.contains("gpt-4") => {
                (dec!(0.00003), dec!(0.00006)) // $30/$60 per 1M
            }
            m if m.contains("gpt-3.5") => {
                (dec!(0.0000005), dec!(0.0000015)) // $0.50/$1.50 per 1M
            }
            _ => (dec!(0.00001), dec!(0.00003)), // Default to GPT-4 Turbo pricing
        }
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let openai_request = OpenAiRequest {
            model: self.config.model.clone(),
            messages: self.build_messages(&request.messages),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            tools: None,
            tool_choice: None,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header(
                "Authorization",
                format!("Bearer {}", self.config.api_key.expose_secret()),
            )
            .header("Content-Type", "application/json")
            .json(&openai_request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error: OpenAiError =
                response
                    .json()
                    .await
                    .map_err(|e| LlmError::InvalidResponse {
                        provider: "openai".to_string(),
                        reason: format!("Failed to parse error response: {}", e),
                    })?;
            return Err(LlmError::RequestFailed {
                provider: "openai".to_string(),
                reason: error.error.message,
            });
        }

        let openai_response: OpenAiResponse = response.json().await?;

        let choice = openai_response
            .choices
            .first()
            .ok_or_else(|| LlmError::InvalidResponse {
                provider: "openai".to_string(),
                reason: "No choices in response".to_string(),
            })?;

        Ok(CompletionResponse {
            content: choice.message.content.clone().unwrap_or_default(),
            input_tokens: openai_response.usage.prompt_tokens,
            output_tokens: openai_response.usage.completion_tokens,
            finish_reason: parse_finish_reason(choice.finish_reason.as_deref()),
        })
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let tools: Vec<OpenAiTool> = request
            .tools
            .iter()
            .map(|t| OpenAiTool {
                tool_type: "function".to_string(),
                function: OpenAiFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                },
            })
            .collect();

        let tool_choice = request.tool_choice.as_ref().map(|c| match c.as_str() {
            "auto" => serde_json::json!("auto"),
            "required" => serde_json::json!("required"),
            "none" => serde_json::json!("none"),
            _ => serde_json::json!("auto"),
        });

        let openai_request = OpenAiRequest {
            model: self.config.model.clone(),
            messages: self.build_messages(&request.messages),
            max_tokens: request.max_tokens,
            temperature: None,
            tools: Some(tools),
            tool_choice,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header(
                "Authorization",
                format!("Bearer {}", self.config.api_key.expose_secret()),
            )
            .header("Content-Type", "application/json")
            .json(&openai_request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error: OpenAiError =
                response
                    .json()
                    .await
                    .map_err(|e| LlmError::InvalidResponse {
                        provider: "openai".to_string(),
                        reason: format!("Failed to parse error response: {}", e),
                    })?;
            return Err(LlmError::RequestFailed {
                provider: "openai".to_string(),
                reason: error.error.message,
            });
        }

        let openai_response: OpenAiResponse = response.json().await?;

        let choice = openai_response
            .choices
            .first()
            .ok_or_else(|| LlmError::InvalidResponse {
                provider: "openai".to_string(),
                reason: "No choices in response".to_string(),
            })?;

        let tool_calls: Vec<ToolCall> = choice
            .message
            .tool_calls
            .as_ref()
            .map(|calls| {
                calls
                    .iter()
                    .filter_map(|c| {
                        let args: serde_json::Value =
                            serde_json::from_str(&c.function.arguments).ok()?;
                        Some(ToolCall {
                            id: c.id.clone(),
                            name: c.function.name.clone(),
                            arguments: args,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(ToolCompletionResponse {
            content: choice.message.content.clone(),
            tool_calls,
            input_tokens: openai_response.usage.prompt_tokens,
            output_tokens: openai_response.usage.completion_tokens,
            finish_reason: parse_finish_reason(choice.finish_reason.as_deref()),
        })
    }
}
