//! Aliyun Coding Plan provider implementation.
//!
//! This provider is specifically designed for Aliyun's Coding Plan service
//! which uses Anthropic API compatible interface but requires specific HTTP configuration.

use async_trait::async_trait;
use reqwest::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use rust_decimal::Decimal;
use secrecy::ExposeSecret;

use crate::llm::config::AliyunConfig;
use crate::llm::costs;
use crate::llm::error::LlmError;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse, ToolDefinition,
};

/// Aliyun Coding Plan provider.
pub struct AliyunProvider {
    client: Client,
    config: AliyunConfig,
}

impl AliyunProvider {
    pub fn new(config: AliyunConfig) -> Result<Self, LlmError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .http1_only()
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: "aliyun".to_string(),
                reason: format!("Failed to build HTTP client: {}", e),
            })?;

        Ok(Self { client, config })
    }

    fn build_url(&self) -> String {
        if self.config.base_url.contains("/apps/anthropic") {
            let base = self.config.base_url.trim_end_matches('/');
            if base.ends_with("/v1") {
                format!("{}/messages", base)
            } else {
                format!("{}/v1/messages", base)
            }
        } else {
            format!(
                "{}/chat/completions",
                self.config.base_url.trim_end_matches('/')
            )
        }
    }

    fn build_auth_header(&self) -> Result<HeaderValue, LlmError> {
        let api_key = self
            .config
            .api_key
            .as_ref()
            .ok_or_else(|| LlmError::AuthFailed {
                provider: "aliyun".to_string(),
            })?;

        let value = format!("Bearer {}", api_key.expose_secret());
        HeaderValue::from_str(&value).map_err(|e| LlmError::RequestFailed {
            provider: "aliyun".to_string(),
            reason: format!("Failed to build auth header: {}", e),
        })
    }

    fn convert_messages_for_anthropic(
        &self,
        messages: &[ChatMessage],
    ) -> (Option<String>, Vec<serde_json::Value>) {
        let mut system_text: Option<String> = None;
        let mut api_messages = Vec::new();

        for msg in messages {
            if msg.role == Role::System {
                match &mut system_text {
                    Some(existing) => {
                        existing.push('\n');
                        existing.push_str(&msg.content);
                    }
                    None => system_text = Some(msg.content.clone()),
                }
                continue;
            }

            if msg.role == Role::Tool {
                let tool_use_id = msg.tool_call_id.as_deref().unwrap_or("unknown");
                api_messages.push(serde_json::json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": msg.content
                    }]
                }));
                continue;
            }

            if msg.role == Role::Assistant
                && let Some(ref tool_calls) = msg.tool_calls
            {
                let mut content_blocks: Vec<serde_json::Value> = Vec::new();
                if !msg.content.is_empty() {
                    content_blocks.push(serde_json::json!({
                        "type": "text",
                        "text": msg.content
                    }));
                }
                for tc in tool_calls {
                    content_blocks.push(serde_json::json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.name,
                        "input": tc.arguments
                    }));
                }
                api_messages.push(serde_json::json!({
                    "role": "assistant",
                    "content": content_blocks
                }));
                continue;
            }

            let role = if msg.role == Role::User {
                "user"
            } else {
                "assistant"
            };
            if msg.content_parts.is_empty() {
                api_messages.push(serde_json::json!({
                    "role": role,
                    "content": msg.content
                }));
            } else {
                let mut content_parts: Vec<serde_json::Value> = Vec::new();
                if !msg.content.is_empty() {
                    content_parts.push(serde_json::json!({
                        "type": "text",
                        "text": msg.content
                    }));
                }
                for part in &msg.content_parts {
                    match part {
                        crate::llm::provider::ContentPart::Text { text } => {
                            content_parts.push(serde_json::json!({
                                "type": "text",
                                "text": text
                            }));
                        }
                        crate::llm::provider::ContentPart::ImageUrl { image_url } => {
                            content_parts.push(serde_json::json!({
                                "type": "image",
                                "source": {
                                    "type": "url",
                                    "url": image_url.url.clone()
                                }
                            }));
                        }
                    }
                }
                api_messages.push(serde_json::json!({
                    "role": role,
                    "content": content_parts
                }));
            }
        }

        (system_text, api_messages)
    }

    async fn complete_internal(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDefinition>>,
        model: Option<String>,
        max_tokens: Option<u32>,
        temperature: Option<f32>,
    ) -> Result<serde_json::Value, LlmError> {
        let url = self.build_url();
        let mut messages = messages;
        crate::llm::provider::sanitize_tool_messages(&mut messages);
        let auth = self.build_auth_header()?;

        let model_name = model.as_deref().unwrap_or(&self.config.model);
        let (system_text, api_messages) = self.convert_messages_for_anthropic(&messages);

        let mut body: serde_json::Value = serde_json::json!({
            "model": model_name,
            "messages": api_messages,
            "max_tokens": max_tokens.unwrap_or(4096),
        });

        if let Some(ref system) = system_text {
            body["system"] = serde_json::json!(system);
        }

        if let Some(temp) = temperature {
            body["temperature"] = serde_json::json!(temp);
        }

        if let Some(tools) = tools {
            let tool_defs: Vec<serde_json::Value> = tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.parameters
                    })
                })
                .collect();
            body["tools"] = serde_json::Value::Array(tool_defs);
        }

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(AUTHORIZATION, auth);
        headers.insert(
            reqwest::header::USER_AGENT,
            HeaderValue::from_str(&format!(
                "IronClaw/{} (compatible; Anthropic-API-Client)",
                env!("CARGO_PKG_VERSION")
            ))
            .unwrap_or_else(|_| HeaderValue::from_static("IronClaw")),
        );

        let response = self
            .client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: "aliyun".to_string(),
                reason: format!("HTTP request failed: {}", e),
            })?;

        let status = response.status();
        let text = response.text().await.map_err(|e| LlmError::RequestFailed {
            provider: "aliyun".to_string(),
            reason: format!("Failed to read response body: {}", e),
        })?;

        if !status.is_success() {
            return Err(LlmError::RequestFailed {
                provider: "aliyun".to_string(),
                reason: format!("HTTP {}: {}", status.as_u16(), text),
            });
        }

        serde_json::from_str(&text).map_err(|e| LlmError::InvalidResponse {
            provider: "aliyun".to_string(),
            reason: format!("Failed to parse response: {}", e),
        })
    }

    fn parse_completion_response(
        &self,
        response: serde_json::Value,
    ) -> Result<CompletionResponse, LlmError> {
        if let Some(content) = response.get("content").and_then(|c| c.as_array()) {
            let mut text_content = String::new();
            let mut thinking_content = String::new();

            for item in content {
                if let Some(item_type) = item.get("type").and_then(|t| t.as_str()) {
                    match item_type {
                        "text" => {
                            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                text_content.push_str(text);
                            }
                        }
                        "thinking" => {
                            if let Some(thinking) = item.get("thinking").and_then(|t| t.as_str()) {
                                thinking_content.push_str(thinking);
                            }
                        }
                        _ => {}
                    }
                }
            }

            let usage = response.get("usage");
            let input_tokens = usage
                .and_then(|u| u.get("input_tokens").and_then(|i| i.as_u64()))
                .unwrap_or(0) as u32;
            let output_tokens = usage
                .and_then(|u| u.get("output_tokens").and_then(|o| o.as_u64()))
                .unwrap_or(0) as u32;
            let cache_read = usage
                .and_then(|u| u.get("cache_read_input_tokens").and_then(|c| c.as_u64()))
                .unwrap_or(0) as u32;
            let cache_creation = usage
                .and_then(|u| {
                    u.get("cache_creation_input_tokens")
                        .and_then(|c| c.as_u64())
                })
                .unwrap_or(0) as u32;

            let finish_reason = response
                .get("stop_reason")
                .and_then(|r| r.as_str())
                .map(|r| match r {
                    "end_turn" | "stop" => FinishReason::Stop,
                    "max_tokens" => FinishReason::Length,
                    "tool_use" => FinishReason::ToolUse,
                    _ => FinishReason::Unknown,
                })
                .unwrap_or(FinishReason::Unknown);

            return Ok(CompletionResponse {
                content: if thinking_content.is_empty() {
                    text_content
                } else {
                    format!("{}\n\n{}", thinking_content, text_content)
                },
                input_tokens,
                output_tokens,
                finish_reason,
                cache_read_input_tokens: cache_read,
                cache_creation_input_tokens: cache_creation,
            });
        }

        if let Some(choices) = response.get("choices").and_then(|c| c.as_array())
            && let Some(choice) = choices.first()
        {
            let content = choice
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();

            let usage = response.get("usage");
            let input_tokens = usage
                .and_then(|u| u.get("prompt_tokens").and_then(|i| i.as_u64()))
                .unwrap_or(0) as u32;
            let output_tokens = usage
                .and_then(|u| u.get("completion_tokens").and_then(|o| o.as_u64()))
                .unwrap_or(0) as u32;

            let finish_reason = choice
                .get("finish_reason")
                .and_then(|r| r.as_str())
                .map(|r| match r {
                    "stop" => FinishReason::Stop,
                    "length" => FinishReason::Length,
                    _ => FinishReason::Unknown,
                })
                .unwrap_or(FinishReason::Unknown);

            return Ok(CompletionResponse {
                content,
                input_tokens,
                output_tokens,
                finish_reason,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            });
        }

        Err(LlmError::InvalidResponse {
            provider: "aliyun".to_string(),
            reason: "Unexpected response format".to_string(),
        })
    }

    fn parse_tool_response(
        &self,
        response: serde_json::Value,
    ) -> Result<ToolCompletionResponse, LlmError> {
        if let Some(content) = response.get("content").and_then(|c| c.as_array()) {
            let mut text_content: Option<String> = None;
            let mut tool_calls = Vec::new();

            for item in content {
                if let Some(item_type) = item.get("type").and_then(|t| t.as_str()) {
                    match item_type {
                        "text" => {
                            text_content =
                                item.get("text").and_then(|t| t.as_str()).map(String::from);
                        }
                        "tool_use" => {
                            if let Some(id) = item.get("id").and_then(|i| i.as_str())
                                && let Some(name) = item.get("name").and_then(|n| n.as_str())
                                && let Some(input) = item.get("input")
                            {
                                tool_calls.push(ToolCall {
                                    id: id.to_string(),
                                    name: name.to_string(),
                                    arguments: input.clone(),
                                    reasoning: None,
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }

            let usage = response.get("usage");
            let input_tokens = usage
                .and_then(|u| u.get("input_tokens").and_then(|i| i.as_u64()))
                .unwrap_or(0) as u32;
            let output_tokens = usage
                .and_then(|u| u.get("output_tokens").and_then(|o| o.as_u64()))
                .unwrap_or(0) as u32;
            let cache_read = usage
                .and_then(|u| u.get("cache_read_input_tokens").and_then(|c| c.as_u64()))
                .unwrap_or(0) as u32;
            let cache_creation = usage
                .and_then(|u| {
                    u.get("cache_creation_input_tokens")
                        .and_then(|c| c.as_u64())
                })
                .unwrap_or(0) as u32;

            let finish_reason = response
                .get("stop_reason")
                .and_then(|r| r.as_str())
                .map(|r| match r {
                    "end_turn" | "stop" => FinishReason::Stop,
                    "max_tokens" => FinishReason::Length,
                    "tool_use" => FinishReason::ToolUse,
                    _ => FinishReason::Unknown,
                })
                .unwrap_or(FinishReason::Unknown);

            return Ok(ToolCompletionResponse {
                content: text_content,
                tool_calls,
                input_tokens,
                output_tokens,
                finish_reason,
                cache_read_input_tokens: cache_read,
                cache_creation_input_tokens: cache_creation,
            });
        }

        if let Some(choices) = response.get("choices").and_then(|c| c.as_array())
            && let Some(choice) = choices.first()
        {
            let message = choice.get("message");

            let content = message
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
                .map(String::from);

            let tool_calls: Vec<ToolCall> = message
                .and_then(|m| m.get("tool_calls"))
                .and_then(|tc| tc.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|tc| {
                            let arguments_str = tc.get("function")?.get("arguments")?.as_str()?;
                            let arguments = serde_json::from_str(arguments_str).ok()?;
                            Some(ToolCall {
                                id: tc.get("id")?.as_str()?.to_string(),
                                name: tc.get("function")?.get("name")?.as_str()?.to_string(),
                                arguments,
                                reasoning: None,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();

            let usage = response.get("usage");
            let input_tokens = usage
                .and_then(|u| u.get("prompt_tokens").and_then(|i| i.as_u64()))
                .unwrap_or(0) as u32;
            let output_tokens = usage
                .and_then(|u| u.get("completion_tokens").and_then(|o| o.as_u64()))
                .unwrap_or(0) as u32;

            let finish_reason = choice
                .get("finish_reason")
                .and_then(|r| r.as_str())
                .map(|r| match r {
                    "stop" => FinishReason::Stop,
                    "length" => FinishReason::Length,
                    "tool_calls" => FinishReason::ToolUse,
                    _ => FinishReason::Unknown,
                })
                .unwrap_or(FinishReason::Unknown);

            return Ok(ToolCompletionResponse {
                content,
                tool_calls,
                input_tokens,
                output_tokens,
                finish_reason,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            });
        }

        Err(LlmError::InvalidResponse {
            provider: "aliyun".to_string(),
            reason: "Unexpected response format".to_string(),
        })
    }
}

#[async_trait]
impl LlmProvider for AliyunProvider {
    fn model_name(&self) -> &str {
        &self.config.model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        costs::model_cost(&self.config.model).unwrap_or_else(costs::default_cost)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let response = self
            .complete_internal(
                request.messages,
                None,
                request.model,
                request.max_tokens,
                request.temperature,
            )
            .await?;

        self.parse_completion_response(response)
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let response = self
            .complete_internal(
                request.messages,
                Some(request.tools),
                request.model,
                request.max_tokens,
                request.temperature,
            )
            .await?;

        self.parse_tool_response(response)
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        Ok(vec![
            "qwen3.5-plus".to_string(),
            "qwen3-max-2026-01-23".to_string(),
            "qwen3-coder-next".to_string(),
            "qwen3-coder-plus".to_string(),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> AliyunProvider {
        AliyunProvider::new(AliyunConfig {
            model: "test".to_string(),
            base_url: "https://example.com/apps/anthropic/v1".to_string(),
            api_key: None,
            timeout_secs: 30,
        })
        .unwrap()
    }

    #[test]
    fn test_build_url_anthropic_with_v1() {
        assert_eq!(
            provider().build_url(),
            "https://example.com/apps/anthropic/v1/messages"
        );
    }

    #[test]
    fn test_build_url_anthropic_without_v1() {
        let p = AliyunProvider::new(AliyunConfig {
            model: "test".to_string(),
            base_url: "https://example.com/apps/anthropic".to_string(),
            api_key: None,
            timeout_secs: 30,
        })
        .unwrap();
        assert_eq!(
            p.build_url(),
            "https://example.com/apps/anthropic/v1/messages"
        );
    }

    #[test]
    fn test_build_url_trailing_slash() {
        let p = AliyunProvider::new(AliyunConfig {
            model: "test".to_string(),
            base_url: "https://example.com/apps/anthropic/v1/".to_string(),
            api_key: None,
            timeout_secs: 30,
        })
        .unwrap();
        assert_eq!(
            p.build_url(),
            "https://example.com/apps/anthropic/v1/messages"
        );
    }

    #[test]
    fn test_convert_messages_system_accumulation() {
        let messages = vec![
            ChatMessage::system("First system"),
            ChatMessage::system("Second system"),
            ChatMessage::user("Hello"),
        ];
        let (system, api_msgs) = provider().convert_messages_for_anthropic(&messages);
        assert_eq!(system.unwrap(), "First system\nSecond system");
        assert_eq!(api_msgs.len(), 1);
    }

    #[test]
    fn test_convert_messages_tool_result() {
        let messages = vec![ChatMessage::tool_result("call_123", "tool", "result")];
        let (_, api_msgs) = provider().convert_messages_for_anthropic(&messages);
        assert_eq!(api_msgs.len(), 1);
        assert_eq!(api_msgs[0]["role"], "user");
        assert_eq!(api_msgs[0]["content"][0]["type"], "tool_result");
    }
}
