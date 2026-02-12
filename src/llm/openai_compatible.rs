//! OpenAI-compatible LLM provider implementation.
//!
//! This provider connects to any endpoint that implements the OpenAI Chat Completions API,
//! such as local models (LM Studio, Ollama with OpenAI format), cloud endpoints, or custom backends.

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use crate::config::OpenAiCompatibleConfig;
use crate::error::LlmError;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ModelMetadata,
    Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse,
};

/// Provider name constant to avoid magic strings.
const PROVIDER_NAME: &str = "openai_compatible";

/// OpenAI-compatible Chat Completions API provider.
pub struct OpenAiCompatibleProvider {
    client: Client,
    config: OpenAiCompatibleConfig,
    active_model: std::sync::RwLock<String>,
}

impl OpenAiCompatibleProvider {
    /// Create a new OpenAI-compatible provider.
    pub fn new(config: OpenAiCompatibleConfig) -> Result<Self, LlmError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("Failed to build reqwest client: {}", e),
            })?;

        let active_model = std::sync::RwLock::new(config.model.clone());
        Ok(Self {
            client,
            config,
            active_model,
        })
    }

    /// Construct API URL for a given path.
    /// Uses the base_url as-is and appends `/v1/{path}`.
    /// Strips trailing `/v1` from base_url to avoid double `/v1` issues.
    fn api_url(&self, path: &str) -> String {
        let base = self.config.base_url.trim_end_matches('/');
        let base = base.strip_suffix("/v1").unwrap_or(base);
        format!("{}/v1/{}", base, path.trim_start_matches('/'))
    }

    /// Get the API key for authentication (borrowed, no allocation).
    fn api_key(&self) -> Option<&str> {
        self.config.api_key.as_ref().map(|k| k.expose_secret())
    }

    /// Add Authorization header if API key is present.
    fn add_auth_header(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.api_key() {
            Some(key) => request.header("Authorization", format!("Bearer {}", key)),
            None => request,
        }
    }

    /// Send a request to the chat completions API.
    async fn send_request<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        body: &T,
    ) -> Result<R, LlmError> {
        let url = self.api_url("chat/completions");

        tracing::debug!("Sending request to OpenAI-compatible endpoint: {}", url);

        let request = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(body);

        let request = self.add_auth_header(request);

        let response = request.send().await.map_err(|e| {
            tracing::error!("OpenAI-compatible request failed: {}", e);
            LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: e.to_string(),
            }
        })?;

        let status = response.status();
        let response_text = response.text().await.map_err(|e| {
            tracing::error!("Failed to read response body: {}", e);
            LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("Response too large or failed to read: {}", e),
            }
        })?;

        tracing::debug!("OpenAI-compatible response status: {}", status);

        if !status.is_success() {
            if status.as_u16() == 401 {
                return Err(LlmError::AuthFailed {
                    provider: PROVIDER_NAME.to_string(),
                });
            }
            if status.as_u16() == 429 {
                return Err(LlmError::RateLimited {
                    provider: PROVIDER_NAME.to_string(),
                    retry_after: None,
                });
            }
            return Err(LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!(
                    "HTTP {}: {}",
                    status,
                    &response_text[..response_text.len().min(200)]
                ),
            });
        }

        serde_json::from_str(&response_text).map_err(|e| LlmError::InvalidResponse {
            provider: PROVIDER_NAME.to_string(),
            reason: format!(
                "JSON parse error: {}. Raw: {}",
                e,
                &response_text[..response_text.len().min(200)]
            ),
        })
    }

    /// Fetch available models with full metadata from the `/v1/models` endpoint.
    async fn fetch_models(&self) -> Result<Vec<ApiModelEntry>, LlmError> {
        let url = self.api_url("models");

        let request = self.client.get(&url);
        let request = self.add_auth_header(request);

        let response = request.send().await.map_err(|e| LlmError::RequestFailed {
            provider: PROVIDER_NAME.to_string(),
            reason: format!("Failed to fetch models: {}", e),
        })?;

        let status = response.status();
        let response_text = response.text().await.map_err(|e| {
            tracing::error!("Failed to read models response: {}", e);
            LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("Response too large or failed to read: {}", e),
            }
        })?;

        if !status.is_success() {
            return Err(LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!(
                    "HTTP {}: {}",
                    status,
                    &response_text[..response_text.len().min(200)]
                ),
            });
        }

        #[derive(Deserialize)]
        struct ModelsResponse {
            data: Vec<ApiModelEntry>,
        }

        let resp: ModelsResponse =
            serde_json::from_str(&response_text).map_err(|e| LlmError::InvalidResponse {
                provider: PROVIDER_NAME.to_string(),
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
impl LlmProvider for OpenAiCompatibleProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let messages: Vec<ChatCompletionMessage> =
            req.messages.into_iter().map(|m| m.into()).collect();

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
                    provider: PROVIDER_NAME.to_string(),
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

        Ok(CompletionResponse {
            content,
            finish_reason,
            input_tokens: response.usage.prompt_tokens,
            output_tokens: response.usage.completion_tokens,
            response_id: None,
        })
    }

    async fn complete_with_tools(
        &self,
        req: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let messages: Vec<ChatCompletionMessage> =
            req.messages.into_iter().map(|m| m.into()).collect();

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
                    provider: PROVIDER_NAME.to_string(),
                    reason: "No choices in response".to_string(),
                })?;

        let content = choice.message.content;
        let tool_calls: Vec<ToolCall> = choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|tc| {
                let arguments = serde_json::from_str(&tc.function.arguments).unwrap_or_else(|e| {
                    tracing::warn!(
                        "Failed to parse tool call arguments from LLM: {}. Raw: '{}'. Defaulting to empty object.",
                        e,
                        tc.function.arguments
                    );
                    serde_json::Value::Object(Default::default())
                });
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

        Ok(ToolCompletionResponse {
            content,
            tool_calls,
            finish_reason,
            input_tokens: response.usage.prompt_tokens,
            output_tokens: response.usage.completion_tokens,
            response_id: None,
        })
    }

    fn model_name(&self) -> &str {
        &self.config.model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        crate::llm::costs::model_cost(&self.config.model)
            .unwrap_or_else(crate::llm::costs::default_cost)
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
        self.active_model
            .read()
            .expect("active_model lock poisoned")
            .clone()
    }

    fn set_model(&self, model: &str) -> Result<(), crate::error::LlmError> {
        let mut guard = self
            .active_model
            .write()
            .expect("active_model lock poisoned");
        *guard = model.to_string();
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
    id: String,
    choices: Vec<ChatCompletionChoice>,
    usage: ChatCompletionUsage,
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

#[derive(Debug, Deserialize)]
struct ChatCompletionUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    #[allow(dead_code)]
    total_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // Tests for api_url() URL construction

    fn create_provider_with_base_url(base_url: &str) -> OpenAiCompatibleProvider {
        use secrecy::SecretString;
        let config = OpenAiCompatibleConfig {
            base_url: base_url.to_string(),
            model: "test-model".to_string(),
            api_key: Some(SecretString::new("test-key".to_string().into())),
        };
        OpenAiCompatibleProvider::new(config).unwrap()
    }

    #[test]
    fn test_api_url_trailing_slash() {
        // trailing slash: https://api.example.com/ → https://api.example.com/v1/chat/completions
        let provider = create_provider_with_base_url("https://api.example.com/");
        let url = provider.api_url("chat/completions");
        assert_eq!(url, "https://api.example.com/v1/chat/completions");
    }

    #[test]
    fn test_api_url_no_trailing_slash() {
        // no trailing slash: https://api.example.com → https://api.example.com/v1/chat/completions
        let provider = create_provider_with_base_url("https://api.example.com");
        let url = provider.api_url("chat/completions");
        assert_eq!(url, "https://api.example.com/v1/chat/completions");
    }

    #[test]
    fn test_api_url_already_has_v1() {
        // already has /v1: https://openrouter.ai/api/v1 → should NOT produce /v1/v1
        let provider = create_provider_with_base_url("https://openrouter.ai/api/v1");
        let url = provider.api_url("chat/completions");
        assert_eq!(url, "https://openrouter.ai/api/v1/chat/completions");
    }

    #[test]
    fn test_api_url_strips_leading_slash_from_path() {
        // Path with leading slash should be handled correctly
        let provider = create_provider_with_base_url("https://api.example.com");
        let url = provider.api_url("/chat/completions");
        assert_eq!(url, "https://api.example.com/v1/chat/completions");
    }
}
