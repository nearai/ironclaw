//! NEAR AI Chat Completions API provider implementation.
//!
//! This provider uses the standard OpenAI-compatible chat completions API
//! with API key authentication (for cloud-api).

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use crate::config::NearAiConfig;
use crate::error::LlmError;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ModelMetadata,
    Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse,
};

/// NEAR AI Chat Completions API provider.
pub struct NearAiChatProvider {
    client: Client,
    config: NearAiConfig,
    active_model: std::sync::RwLock<String>,
    flatten_tool_messages: bool,
}

impl NearAiChatProvider {
    /// Create a new NEAR AI chat completions provider with API key auth.
    ///
    /// By default this enables tool-message flattening for compatibility with
    /// providers that reject `role: "tool"` messages (e.g. NEAR cloud-api).
    pub fn new(config: NearAiConfig) -> Result<Self, LlmError> {
        Self::new_with_flatten(config, true)
    }

    /// Create a chat completions provider with configurable tool-message flattening.
    pub fn new_with_flatten(
        config: NearAiConfig,
        flatten_tool_messages: bool,
    ) -> Result<Self, LlmError> {
        if config.api_key.is_none() {
            return Err(LlmError::AuthFailed {
                provider: "nearai_chat".to_string(),
            });
        }

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: "nearai_chat".to_string(),
                reason: format!("Failed to build HTTP client: {}", e),
            })?;

        let active_model = std::sync::RwLock::new(config.model.clone());
        Ok(Self {
            client,
            config,
            active_model,
            flatten_tool_messages,
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
            .unwrap_or_default()
    }

    /// Send a single request to the chat completions API.
    ///
    /// Does not retry internally — retries are handled by the external
    /// `RetryProvider` wrapper in the composition chain.
    async fn send_request<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        body: &T,
    ) -> Result<R, LlmError> {
        let url = self.api_url("chat/completions");

        tracing::debug!("Sending request to NEAR AI Chat: {}", url);

        if tracing::enabled!(tracing::Level::DEBUG)
            && let Ok(json) = serde_json::to_string(body)
        {
            tracing::debug!("NEAR AI Chat request body: {}", json);
        }

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key()))
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: "nearai_chat".to_string(),
                reason: e.to_string(),
            })?;

        let status = response.status();
        let response_text = response.text().await.map_err(|e| LlmError::RequestFailed {
            provider: "nearai_chat".to_string(),
            reason: format!("Failed to read response body: {}", e),
        })?;

        tracing::debug!("NEAR AI Chat response status: {}", status);
        tracing::debug!("NEAR AI Chat response body: {}", response_text);

        if !status.is_success() {
            let status_code = status.as_u16();

            if status_code == 401 {
                return Err(LlmError::AuthFailed {
                    provider: "nearai_chat".to_string(),
                });
            }

            if status_code == 429 {
                return Err(LlmError::RateLimited {
                    provider: "nearai_chat".to_string(),
                    retry_after: None,
                });
            }

            let truncated = crate::agent::truncate_for_preview(&response_text, 512);
            return Err(LlmError::RequestFailed {
                provider: "nearai_chat".to_string(),
                reason: format!("HTTP {}: {}", status, truncated),
            });
        }

        serde_json::from_str(&response_text).map_err(|e| {
            let truncated = crate::agent::truncate_for_preview(&response_text, 512);
            LlmError::InvalidResponse {
                provider: "nearai_chat".to_string(),
                reason: format!("JSON parse error: {}. Raw: {}", e, truncated),
            }
        })
    }

    /// Fetch available models with full metadata from the `/v1/models` endpoint.
    async fn fetch_models(&self) -> Result<Vec<ApiModelEntry>, LlmError> {
        let url = self.api_url("models");

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key()))
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: "nearai_chat".to_string(),
                reason: format!("Failed to fetch models: {}", e),
            })?;

        let status = response.status();
        let response_text = response.text().await.map_err(|e| LlmError::RequestFailed {
            provider: "nearai_chat".to_string(),
            reason: format!("Failed to read response body: {}", e),
        })?;

        if !status.is_success() {
            let truncated = crate::agent::truncate_for_preview(&response_text, 512);
            return Err(LlmError::RequestFailed {
                provider: "nearai_chat".to_string(),
                reason: format!("HTTP {}: {}", status, truncated),
            });
        }

        #[derive(Deserialize)]
        struct ModelsResponse {
            data: Vec<ApiModelEntry>,
        }

        let resp: ModelsResponse =
            serde_json::from_str(&response_text).map_err(|e| LlmError::InvalidResponse {
                provider: "nearai_chat".to_string(),
                reason: format!("JSON parse error: {}", e),
            })?;

        Ok(resp.data)
    }
}

/// Model entry as returned by the `/v1/models` API.
#[derive(Debug, Deserialize)]
struct ApiModelEntry {
    id: String,
    #[serde(default)]
    context_length: Option<u32>,
}

#[async_trait]
impl LlmProvider for NearAiChatProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let model = req.model.unwrap_or_else(|| self.active_model_name());
        let mut raw_messages = req.messages;
        crate::llm::provider::sanitize_tool_messages(&mut raw_messages);
        let messages: Vec<ChatCompletionMessage> =
            raw_messages.into_iter().map(|m| m.into()).collect();

        let request = ChatCompletionRequest {
            model,
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
                    provider: "nearai_chat".to_string(),
                    reason: "No choices in response".to_string(),
                })?;

        let content = choice.message.content.unwrap_or_default();
        let finish_reason = match choice.finish_reason.as_deref() {
            Some("stop") => FinishReason::Stop,
            Some("length") => FinishReason::Length,
            Some("tool_calls") => FinishReason::ToolUse,
            Some("content_filter") => FinishReason::ContentFilter,
            _ => FinishReason::Unknown,
        };

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
        let model = req.model.unwrap_or_else(|| self.active_model_name());
        let mut raw_messages = req.messages;
        crate::llm::provider::sanitize_tool_messages(&mut raw_messages);
        let messages: Vec<ChatCompletionMessage> =
            raw_messages.into_iter().map(|m| m.into()).collect();

        // Some OpenAI-compatible providers reject `role:"tool"` messages.
        // When enabled, rewrite tool-call / tool-result pairs into plain text.
        let messages = if self.flatten_tool_messages {
            flatten_tool_messages(messages)
        } else {
            messages
        };

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
            model,
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
                    provider: "nearai_chat".to_string(),
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
                ToolCall {
                    id: tc.id,
                    name: tc.function.name,
                    arguments,
                }
            })
            .collect();

        let finish_reason = match choice.finish_reason.as_deref() {
            Some("stop") => FinishReason::Stop,
            Some("length") => FinishReason::Length,
            Some("tool_calls") => FinishReason::ToolUse,
            Some("content_filter") => FinishReason::ContentFilter,
            _ => {
                if !tool_calls.is_empty() {
                    FinishReason::ToolUse
                } else {
                    FinishReason::Unknown
                }
            }
        };

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
        // Default costs - could be model-specific in the future
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

    fn set_model(&self, model: &str) -> Result<(), crate::error::LlmError> {
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

/// Rewrite tool-call / tool-result messages into plain assistant/user text.
///
/// NEAR AI cloud-api does not support the OpenAI multi-turn tool-calling
/// protocol (`role: "tool"` messages). This function converts:
///   - Assistant messages with `tool_calls` → assistant text describing the calls
///   - Tool result messages (`role: "tool"`) → user messages with the result
///
/// Non-tool messages pass through unchanged.
fn flatten_tool_messages(messages: Vec<ChatCompletionMessage>) -> Vec<ChatCompletionMessage> {
    let has_tool_msgs = messages.iter().any(|m| m.role == "tool");
    if !has_tool_msgs {
        return messages;
    }

    tracing::debug!("Flattening tool messages for NEAR AI compatibility");

    messages
        .into_iter()
        .map(|msg| {
            if let (true, Some(calls)) = (msg.role == "assistant", &msg.tool_calls) {
                // Convert assistant tool_calls into descriptive text
                let mut parts: Vec<String> = Vec::new();
                if let Some(ref text) = msg.content
                    && !text.is_empty()
                {
                    parts.push(text.clone());
                }
                for tc in calls {
                    parts.push(format!(
                        "[Called tool `{}` with arguments: {}]",
                        tc.function.name, tc.function.arguments
                    ));
                }
                ChatCompletionMessage {
                    role: "assistant".to_string(),
                    content: Some(parts.join("\n")),
                    tool_call_id: None,
                    name: None,
                    tool_calls: None,
                }
            } else if msg.role == "tool" {
                // Convert tool result into a user message
                let tool_name = msg.name.as_deref().unwrap_or("unknown");
                let result = msg.content.as_deref().unwrap_or("");
                ChatCompletionMessage {
                    role: "user".to_string(),
                    content: Some(format!("[Tool `{}` returned: {}]", tool_name, result)),
                    tool_call_id: None,
                    name: None,
                    tool_calls: None,
                }
            } else {
                msg
            }
        })
        .collect()
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

fn saturate_u32(val: u64) -> u32 {
    val.min(u32::MAX as u64) as u32
}

fn parse_usage(usage: Option<&ChatCompletionUsage>) -> (u32, u32) {
    let Some(u) = usage else {
        return (0, 0);
    };
    let input = u.prompt_tokens.map(saturate_u32).unwrap_or(0);
    let output = u.completion_tokens.map(saturate_u32).unwrap_or_else(|| {
        // Fall back to total - prompt if completion is missing.
        match (u.total_tokens, u.prompt_tokens) {
            (Some(total), Some(prompt)) => saturate_u32(total.saturating_sub(prompt)),
            (Some(total), None) => saturate_u32(total),
            _ => 0,
        }
    });
    (input, output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_nearai_config(base_url: &str) -> NearAiConfig {
        NearAiConfig {
            model: "test-model".to_string(),
            base_url: base_url.to_string(),
            auth_base_url: "https://private.near.ai".to_string(),
            session_path: std::path::PathBuf::from("/tmp/session.json"),
            api_mode: crate::config::NearAiApiMode::ChatCompletions,
            api_key: Some(secrecy::SecretString::from("test-key".to_string())),
            cheap_model: None,
            fallback_model: None,
            max_retries: 0,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
            failover_cooldown_secs: 300,
            failover_cooldown_threshold: 3,
        }
    }

    #[test]
    fn test_api_url_with_base_without_v1() {
        let mut cfg = test_nearai_config("http://127.0.0.1:8318");

        let provider = NearAiChatProvider::new(cfg.clone()).expect("provider");
        assert_eq!(
            provider.api_url("chat/completions"),
            "http://127.0.0.1:8318/v1/chat/completions"
        );

        cfg.base_url = "http://127.0.0.1:8318/".to_string();
        let provider = NearAiChatProvider::new(cfg).expect("provider");
        assert_eq!(
            provider.api_url("/chat/completions"),
            "http://127.0.0.1:8318/v1/chat/completions"
        );
    }

    #[test]
    fn test_api_url_with_base_already_v1() {
        let cfg = test_nearai_config("http://127.0.0.1:8318/v1");

        let provider = NearAiChatProvider::new(cfg).expect("provider");
        assert_eq!(
            provider.api_url("chat/completions"),
            "http://127.0.0.1:8318/v1/chat/completions"
        );
    }

    #[test]
    fn test_message_conversion() {
        let msg = ChatMessage::user("Hello");
        let chat_msg: ChatCompletionMessage = msg.into();
        assert_eq!(chat_msg.role, "user");
        assert_eq!(chat_msg.content, Some("Hello".to_string()));
    }

    #[test]
    fn test_tool_message_conversion() {
        let msg = ChatMessage::tool_result("call_123", "my_tool", "result");
        let chat_msg: ChatCompletionMessage = msg.into();
        assert_eq!(chat_msg.role, "tool");
        assert_eq!(chat_msg.tool_call_id, Some("call_123".to_string()));
        assert_eq!(chat_msg.name, Some("my_tool".to_string()));
    }

    #[test]
    fn test_assistant_with_tool_calls_conversion() {
        use crate::llm::ToolCall;

        let tool_calls = vec![
            ToolCall {
                id: "call_1".to_string(),
                name: "list_issues".to_string(),
                arguments: serde_json::json!({"owner": "foo", "repo": "bar"}),
            },
            ToolCall {
                id: "call_2".to_string(),
                name: "search".to_string(),
                arguments: serde_json::json!({"query": "test"}),
            },
        ];

        let msg = ChatMessage::assistant_with_tool_calls(None, tool_calls);
        let chat_msg: ChatCompletionMessage = msg.into();

        assert_eq!(chat_msg.role, "assistant");

        let tc = chat_msg.tool_calls.expect("tool_calls present");
        assert_eq!(tc.len(), 2);
        assert_eq!(tc[0].id, "call_1");
        assert_eq!(tc[0].function.name, "list_issues");
        assert_eq!(tc[0].call_type, "function");
        assert_eq!(tc[1].id, "call_2");
        assert_eq!(tc[1].function.name, "search");
    }

    #[test]
    fn test_assistant_without_tool_calls_has_none() {
        let msg = ChatMessage::assistant("Hello");
        let chat_msg: ChatCompletionMessage = msg.into();
        assert!(chat_msg.tool_calls.is_none());
    }

    #[test]
    fn test_tool_call_arguments_serialized_to_string() {
        use crate::llm::ToolCall;

        let tc = ToolCall {
            id: "call_1".to_string(),
            name: "test".to_string(),
            arguments: serde_json::json!({"key": "value"}),
        };
        let msg = ChatMessage::assistant_with_tool_calls(None, vec![tc]);
        let chat_msg: ChatCompletionMessage = msg.into();

        let calls = chat_msg.tool_calls.unwrap();
        // Arguments should be a JSON string, not a nested object
        let parsed: serde_json::Value =
            serde_json::from_str(&calls[0].function.arguments).expect("valid JSON string");
        assert_eq!(parsed["key"], "value");
    }

    #[test]
    fn test_flatten_no_tool_messages_passthrough() {
        let messages = vec![
            ChatCompletionMessage {
                role: "system".to_string(),
                content: Some("You are helpful.".to_string()),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            },
            ChatCompletionMessage {
                role: "user".to_string(),
                content: Some("Hello".to_string()),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            },
        ];
        let result = flatten_tool_messages(messages);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "system");
        assert_eq!(result[1].role, "user");
    }

    #[test]
    fn test_flatten_tool_call_and_result() {
        let messages = vec![
            ChatCompletionMessage {
                role: "user".to_string(),
                content: Some("test".to_string()),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            },
            ChatCompletionMessage {
                role: "assistant".to_string(),
                content: None,
                tool_call_id: None,
                name: None,
                tool_calls: Some(vec![ChatCompletionToolCall {
                    id: "call_1".to_string(),
                    call_type: "function".to_string(),
                    function: ChatCompletionToolCallFunction {
                        name: "echo".to_string(),
                        arguments: r#"{"message":"hi"}"#.to_string(),
                    },
                }]),
            },
            ChatCompletionMessage {
                role: "tool".to_string(),
                content: Some("hi".to_string()),
                tool_call_id: Some("call_1".to_string()),
                name: Some("echo".to_string()),
                tool_calls: None,
            },
        ];

        let result = flatten_tool_messages(messages);
        assert_eq!(result.len(), 3);

        // Assistant tool_calls → plain assistant text
        assert_eq!(result[1].role, "assistant");
        assert!(result[1].tool_calls.is_none());
        assert!(
            result[1]
                .content
                .as_ref()
                .unwrap()
                .contains("[Called tool `echo`")
        );

        // Tool result → user message
        assert_eq!(result[2].role, "user");
        assert!(result[2].tool_call_id.is_none());
        assert!(
            result[2]
                .content
                .as_ref()
                .unwrap()
                .contains("[Tool `echo` returned: hi]")
        );
    }

    #[test]
    fn test_flatten_preserves_assistant_text_with_tool_calls() {
        let messages = vec![
            ChatCompletionMessage {
                role: "assistant".to_string(),
                content: Some("Let me check that.".to_string()),
                tool_call_id: None,
                name: None,
                tool_calls: Some(vec![ChatCompletionToolCall {
                    id: "call_1".to_string(),
                    call_type: "function".to_string(),
                    function: ChatCompletionToolCallFunction {
                        name: "search".to_string(),
                        arguments: r#"{"q":"test"}"#.to_string(),
                    },
                }]),
            },
            ChatCompletionMessage {
                role: "tool".to_string(),
                content: Some("found it".to_string()),
                tool_call_id: Some("call_1".to_string()),
                name: Some("search".to_string()),
                tool_calls: None,
            },
        ];

        let result = flatten_tool_messages(messages);
        let text = result[0].content.as_ref().unwrap();
        assert!(text.starts_with("Let me check that."));
        assert!(text.contains("[Called tool `search`"));
    }
}
