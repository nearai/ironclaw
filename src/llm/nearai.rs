//! NEAR AI Chat API provider implementation.
//!
//! This provider uses the NEAR AI chat-api which provides a unified interface
//! to multiple LLM models (OpenAI, Anthropic, etc.) with user authentication.

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use crate::config::NearAiConfig;
use crate::error::LlmError;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse,
};

/// NEAR AI Chat API provider.
pub struct NearAiProvider {
    client: Client,
    config: NearAiConfig,
}

impl NearAiProvider {
    /// Create a new NEAR AI provider.
    pub fn new(config: NearAiConfig) -> Self {
        // Create client with reasonable timeout
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120)) // 2 minute timeout for LLM calls
            .build()
            .unwrap_or_else(|_| Client::new());

        Self { client, config }
    }

    fn api_url(&self, path: &str) -> String {
        format!(
            "{}/v1/{}",
            self.config.base_url,
            path.trim_start_matches('/')
        )
    }

    async fn send_request<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<R, LlmError> {
        let url = self.api_url(path);

        tracing::debug!("Sending request to NEAR AI: {}", url);

        let response = self
            .client
            .post(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.config.session_token.expose_secret()),
            )
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await
            .map_err(|e| {
                tracing::error!("NEAR AI request failed: {}", e);
                e
            })?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();

            // Try to parse as JSON error
            if let Ok(error) = serde_json::from_str::<NearAiErrorResponse>(&error_text) {
                if status.as_u16() == 429 {
                    return Err(LlmError::RateLimited {
                        provider: "nearai".to_string(),
                        retry_after: None,
                    });
                }
                return Err(LlmError::RequestFailed {
                    provider: "nearai".to_string(),
                    reason: error.error,
                });
            }

            return Err(LlmError::RequestFailed {
                provider: "nearai".to_string(),
                reason: format!("HTTP {}: {}", status, error_text),
            });
        }

        response
            .json()
            .await
            .map_err(|e| LlmError::InvalidResponse {
                provider: "nearai".to_string(),
                reason: e.to_string(),
            })
    }
}

#[async_trait]
impl LlmProvider for NearAiProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let messages: Vec<NearAiMessage> = req.messages.into_iter().map(Into::into).collect();

        let request = NearAiRequest {
            model: self.config.model.clone(),
            input: messages,
            temperature: req.temperature,
            max_output_tokens: req.max_tokens,
            stream: Some(false),
            tools: None,
        };

        let response: NearAiResponse = self.send_request("responses", &request).await?;

        // Extract text from response output
        let text = response
            .output
            .iter()
            .filter_map(|item| {
                if item.item_type == "message" {
                    item.content.as_ref().and_then(|contents| {
                        contents
                            .iter()
                            .filter_map(|c| {
                                if c.content_type == "output_text" {
                                    c.text.clone()
                                } else {
                                    None
                                }
                            })
                            .next()
                    })
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");

        Ok(CompletionResponse {
            content: text,
            finish_reason: FinishReason::Stop,
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
        })
    }

    async fn complete_with_tools(
        &self,
        req: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let messages: Vec<NearAiMessage> = req.messages.into_iter().map(Into::into).collect();

        let tools: Vec<NearAiTool> = req
            .tools
            .into_iter()
            .map(|t| NearAiTool {
                tool_type: "function".to_string(),
                name: t.name,
                description: Some(t.description),
                parameters: Some(t.parameters),
            })
            .collect();

        let request = NearAiRequest {
            model: self.config.model.clone(),
            input: messages,
            temperature: req.temperature,
            max_output_tokens: req.max_tokens,
            stream: Some(false),
            tools: if tools.is_empty() { None } else { Some(tools) },
        };

        let response: NearAiResponse = self.send_request("responses", &request).await?;

        // Extract text and tool calls from response
        let mut text = String::new();
        let mut tool_calls = Vec::new();

        for item in &response.output {
            if item.item_type == "message" {
                if let Some(contents) = &item.content {
                    for content in contents {
                        if content.content_type == "output_text" {
                            if let Some(t) = &content.text {
                                text.push_str(t);
                            }
                        }
                    }
                }
            } else if item.item_type == "function_call" {
                if let (Some(name), Some(call_id)) = (&item.name, &item.call_id) {
                    // Parse arguments JSON string into Value
                    let arguments = item
                        .arguments
                        .as_ref()
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or(serde_json::Value::Object(Default::default()));

                    tool_calls.push(ToolCall {
                        id: call_id.clone(),
                        name: name.clone(),
                        arguments,
                    });
                }
            }
        }

        let finish_reason = if tool_calls.is_empty() {
            FinishReason::Stop
        } else {
            FinishReason::ToolUse
        };

        Ok(ToolCompletionResponse {
            content: if text.is_empty() { None } else { Some(text) },
            tool_calls,
            finish_reason,
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
        })
    }

    fn model_name(&self) -> &str {
        &self.config.model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        // Default costs - could be model-specific in the future
        // These are approximate and may vary by model
        (dec!(0.000003), dec!(0.000015))
    }
}

// NEAR AI API types

#[derive(Debug, Serialize)]
struct NearAiRequest {
    model: String,
    input: Vec<NearAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<NearAiTool>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct NearAiMessage {
    role: String,
    content: String,
}

impl From<ChatMessage> for NearAiMessage {
    fn from(msg: ChatMessage) -> Self {
        let role = match msg.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        Self {
            role: role.to_string(),
            content: msg.content,
        }
    }
}

#[derive(Debug, Serialize)]
struct NearAiTool {
    #[serde(rename = "type")]
    tool_type: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct NearAiResponse {
    #[allow(dead_code)]
    id: String,
    output: Vec<NearAiOutputItem>,
    usage: NearAiUsage,
}

#[derive(Debug, Deserialize)]
struct NearAiOutputItem {
    #[serde(rename = "type")]
    item_type: String,
    #[serde(default)]
    content: Option<Vec<NearAiContent>>,
    // For function calls
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NearAiContent {
    #[serde(rename = "type")]
    content_type: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NearAiUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct NearAiErrorResponse {
    error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_conversion() {
        let msg = ChatMessage::user("Hello");
        let nearai_msg: NearAiMessage = msg.into();
        assert_eq!(nearai_msg.role, "user");
        assert_eq!(nearai_msg.content, "Hello");
    }

    #[test]
    fn test_system_message_conversion() {
        let msg = ChatMessage::system("You are helpful");
        let nearai_msg: NearAiMessage = msg.into();
        assert_eq!(nearai_msg.role, "system");
    }
}
