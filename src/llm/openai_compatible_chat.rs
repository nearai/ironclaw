//! OpenAI-compatible Chat Completions provider.
//!
//! Designed for third-party OpenAI-compatible endpoints (vLLM, LiteLLM,
//! local proxies, VibeProxy, etc.).
//!
//! Key guarantees:
//! - Robust usage parsing (never panics on malformed/missing token fields)
//! - Provider-bound tool-call name reconciliation for prefixed aliases
//! - No changes required in agent/tool execution core

use std::collections::HashSet;

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use crate::config::OpenAiCompatibleConfig;
use crate::error::LlmError;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ModelMetadata,
    Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse,
};
use crate::llm::retry::{is_retryable_status, retry_backoff_delay};

const DEFAULT_MAX_RETRIES: u32 = 3;

/// OpenAI-compatible provider implementation over `/v1/chat/completions`.
pub struct OpenAiCompatibleChatProvider {
    client: Client,
    config: OpenAiCompatibleConfig,
    active_model: std::sync::RwLock<String>,
    max_retries: u32,
}

impl OpenAiCompatibleChatProvider {
    /// Create a new OpenAI-compatible provider.
    pub fn new(config: OpenAiCompatibleConfig) -> Result<Self, LlmError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: "openai_compatible_chat".to_string(),
                reason: format!("Failed to build HTTP client: {}", e),
            })?;

        let active_model = std::sync::RwLock::new(config.model.clone());

        Ok(Self {
            client,
            config,
            active_model,
            max_retries: DEFAULT_MAX_RETRIES,
        })
    }

    fn api_url(&self, path: &str) -> String {
        let base = self.config.base_url.trim_end_matches('/');
        let path = path.trim_start_matches('/');

        if base.ends_with("/v1") {
            format!("{}/{}", base, path)
        } else {
            format!("{}/v1/{}", base, path)
        }
    }

    fn api_key(&self) -> String {
        self.config
            .api_key
            .as_ref()
            .map(|k| k.expose_secret().to_string())
            .unwrap_or_else(|| "no-key".to_string())
    }

    async fn send_request<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        body: &T,
    ) -> Result<R, LlmError> {
        let url = self.api_url("chat/completions");

        for attempt in 0..=self.max_retries {
            tracing::debug!(
                "Sending request to OpenAI-compatible chat: {} (attempt {})",
                url,
                attempt + 1,
            );

            let response = self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key()))
                .header("Content-Type", "application/json")
                .json(body)
                .send()
                .await;

            let response = match response {
                Ok(r) => r,
                Err(e) => {
                    if attempt < self.max_retries {
                        let delay = retry_backoff_delay(attempt);
                        tracing::warn!(
                            "OpenAI-compatible request error (attempt {}/{}), retrying in {:?}: {}",
                            attempt + 1,
                            self.max_retries + 1,
                            delay,
                            e,
                        );
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return Err(LlmError::RequestFailed {
                        provider: "openai_compatible_chat".to_string(),
                        reason: e.to_string(),
                    });
                }
            };

            let status = response.status();
            let response_text = response.text().await.unwrap_or_default();

            tracing::debug!("OpenAI-compatible response status: {}", status);

            if !status.is_success() {
                let status_code = status.as_u16();

                if status_code == 401 {
                    return Err(LlmError::AuthFailed {
                        provider: "openai_compatible_chat".to_string(),
                    });
                }

                if is_retryable_status(status_code) && attempt < self.max_retries {
                    let delay = retry_backoff_delay(attempt);
                    tracing::warn!(
                        "OpenAI-compatible endpoint returned HTTP {} (attempt {}/{}), retrying in {:?}",
                        status_code,
                        attempt + 1,
                        self.max_retries + 1,
                        delay,
                    );
                    tokio::time::sleep(delay).await;
                    continue;
                }

                if status_code == 429 {
                    return Err(LlmError::RateLimited {
                        provider: "openai_compatible_chat".to_string(),
                        retry_after: None,
                    });
                }

                let truncated = crate::agent::truncate_for_preview(&response_text, 512);
                return Err(LlmError::RequestFailed {
                    provider: "openai_compatible_chat".to_string(),
                    reason: format!("HTTP {}: {}", status, truncated),
                });
            }

            return serde_json::from_str(&response_text).map_err(|e| LlmError::InvalidResponse {
                provider: "openai_compatible_chat".to_string(),
                reason: format!("JSON parse error: {}", e),
            });
        }

        Err(LlmError::RequestFailed {
            provider: "openai_compatible_chat".to_string(),
            reason: "retry loop exited unexpectedly".to_string(),
        })
    }

    async fn fetch_models(&self) -> Result<Vec<ApiModelEntry>, LlmError> {
        let url = self.api_url("models");

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key()))
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: "openai_compatible_chat".to_string(),
                reason: format!("Failed to fetch models: {}", e),
            })?;

        let status = response.status();
        let response_text = response.text().await.unwrap_or_default();

        if !status.is_success() {
            let truncated = crate::agent::truncate_for_preview(&response_text, 512);
            return Err(LlmError::RequestFailed {
                provider: "openai_compatible_chat".to_string(),
                reason: format!("HTTP {}: {}", status, truncated),
            });
        }

        #[derive(Deserialize)]
        struct ModelsResponse {
            data: Vec<ApiModelEntry>,
        }

        let resp: ModelsResponse =
            serde_json::from_str(&response_text).map_err(|e| LlmError::InvalidResponse {
                provider: "openai_compatible_chat".to_string(),
                reason: format!("JSON parse error: {}", e),
            })?;

        Ok(resp.data)
    }
}

#[derive(Debug, Deserialize)]
struct ApiModelEntry {
    id: String,
    #[serde(default)]
    context_length: Option<u32>,
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleChatProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut raw_messages = req.messages;
        crate::llm::provider::sanitize_tool_messages(&mut raw_messages);
        let messages: Vec<ChatCompletionMessage> = raw_messages
            .into_iter()
            .map(ChatCompletionMessage::from)
            .collect();

        let request = ChatCompletionRequest {
            model: self.active_model_name(),
            messages,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            tools: None,
            tool_choice: None,
        };

        let response: ChatCompletionResponse = self.send_request(&request).await?;

        let choice =
            response
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| LlmError::InvalidResponse {
                    provider: "openai_compatible_chat".to_string(),
                    reason: "No choices in response".to_string(),
                })?;

        let content = choice.message.content.unwrap_or_default();
        let finish_reason = parse_finish_reason(choice.finish_reason.as_deref(), false);
        let (input_tokens, output_tokens) = parse_usage(response.usage.as_ref());

        Ok(CompletionResponse {
            content,
            finish_reason,
            input_tokens,
            output_tokens,
            response_id: None,
        })
    }

    async fn complete_with_tools(
        &self,
        req: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let known_tool_names: HashSet<String> = req.tools.iter().map(|t| t.name.clone()).collect();

        let mut raw_messages = req.messages;
        crate::llm::provider::sanitize_tool_messages(&mut raw_messages);
        let messages: Vec<ChatCompletionMessage> = raw_messages
            .into_iter()
            .map(ChatCompletionMessage::from)
            .collect();

        let tools: Vec<ChatCompletionTool> = req
            .tools
            .into_iter()
            .map(|t| ChatCompletionTool {
                tool_type: "function".to_string(),
                function: ChatCompletionFunction {
                    name: t.name,
                    description: Some(t.description),
                    parameters: Some(t.parameters),
                },
            })
            .collect();

        let request = ChatCompletionRequest {
            model: self.active_model_name(),
            messages,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            tools: if tools.is_empty() { None } else { Some(tools) },
            tool_choice: req.tool_choice,
        };

        let response: ChatCompletionResponse = self.send_request(&request).await?;

        let choice =
            response
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| LlmError::InvalidResponse {
                    provider: "openai_compatible_chat".to_string(),
                    reason: "No choices in response".to_string(),
                })?;

        let content = choice.message.content;

        let tool_calls: Vec<ToolCall> = choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|tc| {
                let arguments = serde_json::from_str(&tc.function.arguments)
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                let normalized_name = normalize_tool_name(&tc.function.name, &known_tool_names);
                if normalized_name != tc.function.name {
                    tracing::debug!(
                        original = %tc.function.name,
                        normalized = %normalized_name,
                        "Normalized tool call name from provider",
                    );
                }

                ToolCall {
                    id: tc.id,
                    name: normalized_name,
                    arguments,
                }
            })
            .collect();

        let finish_reason =
            parse_finish_reason(choice.finish_reason.as_deref(), !tool_calls.is_empty());
        let (input_tokens, output_tokens) = parse_usage(response.usage.as_ref());

        Ok(ToolCompletionResponse {
            content,
            tool_calls,
            finish_reason,
            input_tokens,
            output_tokens,
            response_id: None,
        })
    }

    fn model_name(&self) -> &str {
        &self.config.model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        // Conservative defaults; may be overridden by future model-specific pricing.
        (dec!(0.000003), dec!(0.000015))
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let models = self.fetch_models().await?;
        Ok(models.into_iter().map(|m| m.id).collect())
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        let active = self.active_model_name();
        let models = self.fetch_models().await?;
        let current = models.iter().find(|m| m.id == active);
        Ok(ModelMetadata {
            id: active,
            context_length: current.and_then(|m| m.context_length),
        })
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

fn parse_finish_reason(reason: Option<&str>, has_tool_calls: bool) -> FinishReason {
    match reason {
        Some("stop") => FinishReason::Stop,
        Some("length") => FinishReason::Length,
        Some("tool_calls") => FinishReason::ToolUse,
        Some("content_filter") => FinishReason::ContentFilter,
        _ if has_tool_calls => FinishReason::ToolUse,
        _ => FinishReason::Unknown,
    }
}

fn normalize_tool_name(name: &str, known_tools: &HashSet<String>) -> String {
    if known_tools.contains(name) {
        return name.to_string();
    }

    if let Some(stripped) = name.strip_prefix("proxy_")
        && known_tools.contains(stripped)
    {
        return stripped.to_string();
    }

    name.to_string()
}

fn saturate_u32(val: u64) -> u32 {
    val.min(u32::MAX as u64) as u32
}

fn parse_usage(usage: Option<&ChatCompletionUsage>) -> (u32, u32) {
    let Some(usage) = usage else {
        return (0, 0);
    };

    if let Some(completion) = usage.completion_tokens {
        return (
            usage.prompt_tokens.map(saturate_u32).unwrap_or(0),
            saturate_u32(completion),
        );
    }

    if let (Some(total), Some(prompt)) = (usage.total_tokens, usage.prompt_tokens) {
        let output = total.saturating_sub(prompt);
        if total < prompt {
            tracing::warn!(
                total_tokens = total,
                prompt_tokens = prompt,
                "OpenAI-compatible usage had total_tokens < prompt_tokens; clamping output tokens to 0"
            );
        }
        return (saturate_u32(prompt), saturate_u32(output));
    }

    if let Some(total) = usage.total_tokens {
        tracing::warn!(
            total_tokens = total,
            "OpenAI-compatible usage missing prompt/completion tokens; treating total as output"
        );
        return (0, saturate_u32(total));
    }

    if let Some(prompt) = usage.prompt_tokens {
        tracing::warn!(
            prompt_tokens = prompt,
            "OpenAI-compatible usage missing total/completion tokens; returning prompt only"
        );
        return (saturate_u32(prompt), 0);
    }

    (0, 0)
}

// OpenAI-compatible Chat Completions API types

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatCompletionMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ChatCompletionTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatCompletionMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ChatCompletionToolCall>>,
}

impl From<ChatMessage> for ChatCompletionMessage {
    fn from(msg: ChatMessage) -> Self {
        let role = match msg.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };

        let tool_calls = msg.tool_calls.map(|calls| {
            calls
                .into_iter()
                .map(|tc| ChatCompletionToolCall {
                    id: tc.id,
                    call_type: "function".to_string(),
                    function: ChatCompletionToolCallFunction {
                        name: tc.name,
                        arguments: tc.arguments.to_string(),
                    },
                })
                .collect()
        });

        let content = if role == "assistant" && tool_calls.is_some() && msg.content.is_empty() {
            None
        } else {
            Some(msg.content)
        };

        Self {
            role: role.to_string(),
            content,
            tool_call_id: msg.tool_call_id,
            name: msg.name,
            tool_calls,
        }
    }
}

#[derive(Debug, Serialize)]
struct ChatCompletionTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: ChatCompletionFunction,
}

#[derive(Debug, Serialize)]
struct ChatCompletionFunction {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    #[allow(dead_code)]
    #[serde(default)]
    id: Option<String>,
    choices: Vec<ChatCompletionChoice>,
    #[serde(default)]
    usage: Option<ChatCompletionUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChoice {
    message: ChatCompletionResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponseMessage {
    #[allow(dead_code)]
    role: String,
    content: Option<String>,
    tool_calls: Option<Vec<ChatCompletionToolCall>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatCompletionToolCall {
    id: String,
    #[serde(rename = "type")]
    #[allow(dead_code)]
    call_type: String,
    function: ChatCompletionToolCallFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatCompletionToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize, Default)]
struct ChatCompletionUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
    #[serde(default)]
    total_tokens: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_url_without_v1_suffix() {
        let cfg = OpenAiCompatibleConfig {
            base_url: "http://127.0.0.1:8318".to_string(),
            api_key: None,
            model: "test-model".to_string(),
        };
        let provider = OpenAiCompatibleChatProvider::new(cfg).expect("provider");
        assert_eq!(
            provider.api_url("chat/completions"),
            "http://127.0.0.1:8318/v1/chat/completions"
        );
    }

    #[test]
    fn test_api_url_with_v1_suffix() {
        let cfg = OpenAiCompatibleConfig {
            base_url: "http://127.0.0.1:8318/v1".to_string(),
            api_key: None,
            model: "test-model".to_string(),
        };
        let provider = OpenAiCompatibleChatProvider::new(cfg).expect("provider");
        assert_eq!(
            provider.api_url("/chat/completions"),
            "http://127.0.0.1:8318/v1/chat/completions"
        );
    }

    #[test]
    fn test_normalize_tool_name_exact_match() {
        let known = HashSet::from(["echo".to_string(), "list_jobs".to_string()]);
        assert_eq!(normalize_tool_name("echo", &known), "echo");
    }

    #[test]
    fn test_normalize_tool_name_proxy_prefix_match() {
        let known = HashSet::from(["echo".to_string(), "list_jobs".to_string()]);
        assert_eq!(normalize_tool_name("proxy_echo", &known), "echo");
    }

    #[test]
    fn test_normalize_tool_name_proxy_prefix_no_match_kept() {
        let known = HashSet::from(["echo".to_string(), "list_jobs".to_string()]);
        assert_eq!(
            normalize_tool_name("proxy_unknown", &known),
            "proxy_unknown"
        );
    }

    #[test]
    fn test_parse_usage_prefers_completion_tokens() {
        let usage = ChatCompletionUsage {
            prompt_tokens: Some(10),
            completion_tokens: Some(7),
            total_tokens: Some(12),
        };
        assert_eq!(parse_usage(Some(&usage)), (10, 7));
    }

    #[test]
    fn test_parse_usage_uses_saturating_sub_when_completion_missing() {
        let usage = ChatCompletionUsage {
            prompt_tokens: Some(500),
            completion_tokens: None,
            total_tokens: Some(120),
        };
        assert_eq!(parse_usage(Some(&usage)), (500, 0));
    }

    #[test]
    fn test_parse_usage_handles_missing_fields() {
        let usage = ChatCompletionUsage::default();
        assert_eq!(parse_usage(Some(&usage)), (0, 0));
        assert_eq!(parse_usage(None), (0, 0));
    }

    #[test]
    fn test_parse_usage_total_only_maps_to_output() {
        let usage = ChatCompletionUsage {
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: Some(42),
        };
        assert_eq!(parse_usage(Some(&usage)), (0, 42));
    }

    #[test]
    fn test_parse_usage_prompt_only_maps_to_input() {
        let usage = ChatCompletionUsage {
            prompt_tokens: Some(17),
            completion_tokens: None,
            total_tokens: None,
        };
        assert_eq!(parse_usage(Some(&usage)), (17, 0));
    }
}
