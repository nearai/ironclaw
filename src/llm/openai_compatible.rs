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

/// Maximum response size in bytes (10MB) to prevent DoS from oversized responses.
const MAX_RESPONSE_SIZE_BYTES: usize = 10 * 1024 * 1024; // 10MB

/// Cache TTL for model list (5 minutes).
const MODEL_CACHE_TTL_SECONDS: u64 = 300; // 5 minutes

/// Cached model list with timestamp for TTL validation.
struct CachedModels {
    models: Vec<ApiModelEntry>,
    fetched_at: std::time::Instant,
}

/// OpenAI-compatible Chat Completions API provider.
///
/// Note on std::sync::RwLock: While tokio::sync::RwLock is preferred for async code,
/// we use std::sync::RwLock here because all read/write operations are short-lived
/// and never held across await points. This avoids the overhead of async locking
/// for simple model name synchronization.
pub struct OpenAiCompatibleProvider {
    client: Client,
    config: OpenAiCompatibleConfig,
    active_model: std::sync::RwLock<String>,
    models_cache: std::sync::RwLock<Option<CachedModels>>,
}

impl std::fmt::Debug for OpenAiCompatibleProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatibleProvider")
            .field("base_url", &self.config.base_url)
            .field("model", &self.config.model)
            .field("has_api_key", &self.config.api_key.is_some())
            .field("timeout_secs", &self.config.timeout_secs)
            .field("active_model", &self.active_model_name())
            .field(
                "has_cached_models",
                &self
                    .models_cache
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .is_some(),
            )
            .finish()
    }
}

/// Truncate a string safely at UTF-8 character boundaries.
/// Returns at most `max_chars` characters from the start of the string.
fn truncate_utf8_safe(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((idx, _)) => &text[..idx],
        None => text,
    }
}

impl OpenAiCompatibleProvider {
    /// Create a new OpenAI-compatible provider.
    pub fn new(config: OpenAiCompatibleConfig) -> Result<Self, LlmError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("Failed to build reqwest client: {}", e),
            })?;

        let active_model = std::sync::RwLock::new(config.model.clone());
        let models_cache = std::sync::RwLock::new(None);
        Ok(Self {
            client,
            config,
            active_model,
            models_cache,
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

    /// Send a request to the chat completions API with retry logic.
    ///
    /// Implements exponential backoff with jitter for retryable errors (429, 5xx).
    /// Respects Retry-After header when present on 429 responses.
    async fn send_request_with_retry<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        body: &T,
    ) -> Result<R, LlmError> {
        let mut delay_ms = self.config.retry_initial_delay_ms;
        let max_retries = self.config.max_retries;

        for attempt in 0..=max_retries {
            match self.send_request_internal(body).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    // Check if this is the last attempt
                    if attempt == max_retries {
                        return Err(e);
                    }

                    // Check if error is retryable (429 or 5xx)
                    let should_retry = match &e {
                        LlmError::RateLimited { retry_after, .. } => {
                            // Use Retry-After header if present
                            if let Some(duration) = retry_after {
                                delay_ms = duration.as_millis() as u64;
                            }
                            true
                        }
                        LlmError::RequestFailed { reason, .. } => {
                            // Check for 5xx in error message (e.g., "HTTP 503: ...")
                            reason.contains("HTTP 5")
                        }
                        // Auth errors and other 4xx should not be retried
                        LlmError::AuthFailed { .. } => false,
                        _ => false,
                    };

                    if !should_retry {
                        return Err(e);
                    }

                    tracing::warn!(
                        "Request failed (attempt {}/{}), retrying after {}ms: {}",
                        attempt + 1,
                        max_retries,
                        delay_ms,
                        e
                    );

                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

                    // Exponential backoff: delay = delay * 2
                    // With ±25% jitter to prevent thundering herd
                    let jitter = (delay_ms as f64 * 0.25) as u64;
                    let jitter_adjustment = if rand::random::<bool>() {
                        jitter as i64
                    } else {
                        -(jitter as i64)
                    };
                    // Use i64 for safe arithmetic, then convert back to u64
                    let new_delay = (delay_ms as i64 * 2).saturating_add(jitter_adjustment);
                    delay_ms = new_delay.max(1) as u64;
                }
            }
        }

        // This should never be reached, but satisfies the compiler
        Err(LlmError::RequestFailed {
            provider: PROVIDER_NAME.to_string(),
            reason: "Max retries exceeded".to_string(),
        })
    }

    /// Send a request to the chat completions API (internal, no retries).
    async fn send_request_internal<T: Serialize, R: for<'de> Deserialize<'de>>(
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
        // Extract Retry-After header before consuming the body
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .map(std::time::Duration::from_secs);
        let response_text = response.text().await.map_err(|e| {
            tracing::error!("Failed to read response body: {}", e);
            LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("Response too large or failed to read: {}", e),
            }
        })?;

        // Check response size to prevent DoS from oversized responses
        if response_text.len() > MAX_RESPONSE_SIZE_BYTES {
            return Err(LlmError::InvalidResponse {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("Response size {} exceeds 10MB limit", response_text.len()),
            });
        }

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
                    retry_after,
                });
            }
            return Err(LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!(
                    "HTTP {}: {}",
                    status,
                    truncate_utf8_safe(&response_text, 200)
                ),
            });
        }

        serde_json::from_str(&response_text).map_err(|e| LlmError::InvalidResponse {
            provider: PROVIDER_NAME.to_string(),
            reason: format!(
                "JSON parse error: {}. Raw: {}",
                e,
                truncate_utf8_safe(&response_text, 200)
            ),
        })
    }

    /// Fetch available models with full metadata from the `/v1/models` endpoint.
    /// Uses caching with 5-minute TTL to reduce API calls.
    async fn fetch_models(&self) -> Result<Vec<ApiModelEntry>, LlmError> {
        // Check cache first
        {
            let cache_guard = self.models_cache.read().unwrap_or_else(|e| e.into_inner());
            if let Some(ref cached) = *cache_guard {
                let elapsed = cached.fetched_at.elapsed();
                if elapsed < std::time::Duration::from_secs(MODEL_CACHE_TTL_SECONDS) {
                    tracing::debug!("Using cached model list ({}s old)", elapsed.as_secs());
                    return Ok(cached.models.clone());
                }
            }
        } // Drop read lock before acquiring write lock

        // Fetch from API
        let url = self.api_url("models");

        let request = self.client.get(&url);
        let request = self.add_auth_header(request);

        let response = request.send().await.map_err(|e| {
            // Clear cache on failure to prevent stale data
            let mut cache_guard = self.models_cache.write().unwrap_or_else(|e| e.into_inner());
            *cache_guard = None;
            LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("Failed to fetch models: {}", e),
            }
        })?;

        let status = response.status();
        let response_text = response.text().await.map_err(|e| {
            tracing::error!("Failed to read models response: {}", e);
            // Clear cache on failure to prevent stale data
            let mut cache_guard = self.models_cache.write().unwrap_or_else(|e| e.into_inner());
            *cache_guard = None;
            LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("Response too large or failed to read: {}", e),
            }
        })?;

        if !status.is_success() {
            // Clear cache on failure to prevent stale data
            let mut cache_guard = self.models_cache.write().unwrap_or_else(|e| e.into_inner());
            *cache_guard = None;
            return Err(LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!(
                    "HTTP {}: {}",
                    status,
                    truncate_utf8_safe(&response_text, 200)
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

        // Update cache
        let mut cache_guard = self.models_cache.write().unwrap_or_else(|e| e.into_inner());
        *cache_guard = Some(CachedModels {
            models: resp.data.clone(),
            fetched_at: std::time::Instant::now(),
        });

        Ok(resp.data)
    }
}

/// Model entry as returned by the `/v1/models` API.
#[derive(Debug, Clone, Deserialize)]
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
            top_p: None,
            n: None,
            presence_penalty: None,
            frequency_penalty: None,
            seed: None,
            tools: None,
            tool_choice: None,
        };

        let response: ChatCompletionResponse = self.send_request_with_retry(&request).await?;

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
            top_p: req.top_p,
            n: req.n,
            presence_penalty: req.presence_penalty,
            frequency_penalty: req.frequency_penalty,
            seed: req.seed,
            tools: if tools.is_empty() { None } else { Some(tools) },
            tool_choice: req.tool_choice.map(serde_json::Value::String),
        };

        let response: ChatCompletionResponse = self.send_request_with_retry(&request).await?;

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
            .unwrap_or_else(|e| {
                tracing::warn!(
                    "Poisoned read lock detected in OpenAiCompatibleProvider, recovering. \
                     This may indicate a previous panic in the provider: {}",
                    e
                );
                e.into_inner()
            })
            .clone()
    }

    fn set_model(&self, model: &str) -> Result<(), crate::error::LlmError> {
        let mut guard = self.active_model.write().unwrap_or_else(|e| {
            tracing::warn!(
                "Poisoned write lock detected in OpenAiCompatibleProvider, recovering. \
                 This may indicate a previous panic in the provider: {}",
                e
            );
            e.into_inner()
        });
        *guard = model.to_string();
        // Invalidate model cache when model changes
        let mut cache_guard = self.models_cache.write().unwrap_or_else(|e| {
            tracing::warn!(
                "Poisoned write lock detected in models_cache, recovering. \
                 This may indicate a previous panic in the provider: {}",
                e
            );
            e.into_inner()
        });
        *cache_guard = None;
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
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    n: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ChatCompletionTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<serde_json::Value>,
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
    use axum::response::IntoResponse;
    use std::sync::Arc;

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

    // Tests for tool_choice serialization

    #[test]
    fn test_tool_choice_string_auto_serializes_correctly() {
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            top_p: None,
            n: None,
            presence_penalty: None,
            frequency_penalty: None,
            seed: None,
            tools: None,
            tool_choice: Some(serde_json::Value::String("auto".to_string())),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""tool_choice":"auto""#));
    }

    #[test]
    fn test_tool_choice_string_required_serializes_correctly() {
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            top_p: None,
            n: None,
            presence_penalty: None,
            frequency_penalty: None,
            seed: None,
            tools: None,
            tool_choice: Some(serde_json::Value::String("required".to_string())),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""tool_choice":"required""#));
    }

    #[test]
    fn test_tool_choice_string_none_serializes_correctly() {
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            top_p: None,
            n: None,
            presence_penalty: None,
            frequency_penalty: None,
            seed: None,
            tools: None,
            tool_choice: Some(serde_json::Value::String("none".to_string())),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""tool_choice":"none""#));
    }

    #[test]
    fn test_tool_choice_object_serializes_correctly() {
        let tool_choice_obj = serde_json::json!({
            "type": "function",
            "function": {
                "name": "my_function"
            }
        });
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            top_p: None,
            n: None,
            presence_penalty: None,
            frequency_penalty: None,
            seed: None,
            tools: None,
            tool_choice: Some(tool_choice_obj),
        };
        let json = serde_json::to_string(&req).unwrap();
        // Check that tool_choice contains the expected fields (whitespace may vary)
        assert!(json.contains("\"tool_choice\""));
        assert!(json.contains("\"type\":\"function\""));
        assert!(json.contains("\"name\":\"my_function\""));
    }

    #[test]
    fn test_tool_choice_none_is_omitted_from_serialization() {
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            top_p: None,
            n: None,
            presence_penalty: None,
            frequency_penalty: None,
            seed: None,
            tools: None,
            tool_choice: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("tool_choice"));
    }

    // Tests for new parameters serialization (top_p, n, presence_penalty, frequency_penalty, seed)

    #[test]
    fn test_top_p_serializes_correctly() {
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            top_p: Some(0.9),
            n: None,
            presence_penalty: None,
            frequency_penalty: None,
            seed: None,
            tools: None,
            tool_choice: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""top_p":0.9"#));
    }

    #[test]
    fn test_top_p_none_is_omitted_from_serialization() {
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            top_p: None,
            n: None,
            presence_penalty: None,
            frequency_penalty: None,
            seed: None,
            tools: None,
            tool_choice: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("top_p"));
    }

    #[test]
    fn test_n_serializes_correctly() {
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            top_p: None,
            n: Some(3),
            presence_penalty: None,
            frequency_penalty: None,
            seed: None,
            tools: None,
            tool_choice: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""n":3"#));
    }

    #[test]
    fn test_n_none_is_omitted_from_serialization() {
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            top_p: None,
            n: None,
            presence_penalty: None,
            frequency_penalty: None,
            seed: None,
            tools: None,
            tool_choice: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("n"));
    }

    #[test]
    fn test_presence_penalty_serializes_correctly() {
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            top_p: None,
            n: None,
            presence_penalty: Some(0.5),
            frequency_penalty: None,
            seed: None,
            tools: None,
            tool_choice: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""presence_penalty":0.5"#));
    }

    #[test]
    fn test_presence_penalty_none_is_omitted_from_serialization() {
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            top_p: None,
            n: None,
            presence_penalty: None,
            frequency_penalty: None,
            seed: None,
            tools: None,
            tool_choice: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("presence_penalty"));
    }

    #[test]
    fn test_frequency_penalty_serializes_correctly() {
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            top_p: None,
            n: None,
            presence_penalty: None,
            frequency_penalty: Some(1.0),
            seed: None,
            tools: None,
            tool_choice: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""frequency_penalty":1.0"#));
    }

    #[test]
    fn test_frequency_penalty_none_is_omitted_from_serialization() {
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            top_p: None,
            n: None,
            presence_penalty: None,
            frequency_penalty: None,
            seed: None,
            tools: None,
            tool_choice: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("frequency_penalty"));
    }

    #[test]
    fn test_seed_serializes_correctly() {
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            top_p: None,
            n: None,
            presence_penalty: None,
            frequency_penalty: None,
            seed: Some(42),
            tools: None,
            tool_choice: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""seed":42"#));
    }

    #[test]
    fn test_seed_none_is_omitted_from_serialization() {
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            top_p: None,
            n: None,
            presence_penalty: None,
            frequency_penalty: None,
            seed: None,
            tools: None,
            tool_choice: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("seed"));
    }

    #[test]
    fn test_all_new_parameters_serialized_together() {
        // Test that all new parameters serialize correctly when provided together
        let req = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: Some(0.7),
            max_tokens: Some(100),
            top_p: Some(0.9),
            n: Some(2),
            presence_penalty: Some(0.5),
            frequency_penalty: Some(0.5),
            seed: Some(12345),
            tools: None,
            tool_choice: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""top_p":0.9"#));
        assert!(json.contains(r#""n":2"#));
        assert!(json.contains(r#""presence_penalty":0.5"#));
        assert!(json.contains(r#""frequency_penalty":0.5"#));
        assert!(json.contains(r#""seed":12345"#));
    }

    // Tests for api_url() URL construction

    fn create_provider_with_base_url(base_url: &str) -> OpenAiCompatibleProvider {
        use secrecy::SecretString;
        let config = OpenAiCompatibleConfig {
            base_url: base_url.to_string(),
            model: "test-model".to_string(),
            api_key: Some(SecretString::new("test-key".to_string().into())),
            timeout_secs: 120,
            max_retries: 3,
            retry_initial_delay_ms: 1000,
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

    #[test]
    fn test_truncate_utf8_safe_basic() {
        // Basic truncation
        assert_eq!(truncate_utf8_safe("hello world", 5), "hello");
        assert_eq!(truncate_utf8_safe("hello", 10), "hello");
        assert_eq!(truncate_utf8_safe("", 5), "");
    }

    #[test]
    fn test_truncate_utf8_safe_multibyte() {
        // Multi-byte UTF-8 characters (Japanese)
        let japanese = "こんにちは世界"; // "Hello World" in Japanese
        // Each character is 3 bytes in UTF-8
        assert_eq!(truncate_utf8_safe(japanese, 3), "こんに");
        assert_eq!(truncate_utf8_safe(japanese, 5), "こんにちは");

        // Mixed ASCII and multi-byte
        let mixed = "Helloこんにちは";
        assert_eq!(truncate_utf8_safe(mixed, 7), "Helloこん");
    }

    #[test]
    fn test_debug_impl_does_not_expose_api_key() {
        use secrecy::SecretString;
        let config = OpenAiCompatibleConfig {
            base_url: "https://api.example.com".to_string(),
            model: "test-model".to_string(),
            api_key: Some(SecretString::new("secret-key".to_string().into())),
            timeout_secs: 60,
            max_retries: 3,
            retry_initial_delay_ms: 1000,
        };
        let provider = OpenAiCompatibleProvider::new(config).unwrap();

        let debug_str = format!("{:?}", provider);
        assert!(debug_str.contains("base_url"));
        assert!(debug_str.contains("has_api_key"));
        assert!(!debug_str.contains("secret-key")); // API key should NOT appear
    }

    #[test]
    fn test_lock_poisoning_recovery() {
        use std::panic::{self, AssertUnwindSafe};

        let config = OpenAiCompatibleConfig {
            base_url: "https://api.example.com".to_string(),
            model: "original-model".to_string(),
            api_key: None,
            timeout_secs: 120,
            max_retries: 3,
            retry_initial_delay_ms: 1000,
        };
        let provider = OpenAiCompatibleProvider::new(config).unwrap();

        // Poison the lock by panicking while holding the write lock
        let _ = panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = provider.active_model.write().unwrap();
            panic!("Intentional panic to poison lock");
        }));

        // Should still be able to read after poisoning
        let model = provider.active_model_name();
        assert_eq!(model, "original-model");

        // Should still be able to write after poisoning
        provider.set_model("new-model").unwrap();
        assert_eq!(provider.active_model_name(), "new-model");
    }

    #[test]
    fn test_max_response_size_constant() {
        // Verify the constant is correctly set to 10MB
        assert_eq!(MAX_RESPONSE_SIZE_BYTES, 10 * 1024 * 1024);
        assert_eq!(MAX_RESPONSE_SIZE_BYTES, 10_485_760);
    }

    // Tests for retry logic - using wiremock-style testing with proper async handlers

    /// Helper struct for mock server state
    #[derive(Clone)]
    struct MockServerState {
        request_count: Arc<std::sync::atomic::AtomicUsize>,
        fail_count: usize,
        status_code: axum::http::StatusCode,
    }

    impl MockServerState {
        fn new(fail_count: usize, status_code: axum::http::StatusCode) -> Self {
            Self {
                request_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                fail_count,
                status_code,
            }
        }

        fn count(&self) -> usize {
            self.request_count.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    async fn mock_handler(
        axum::extract::State(state): axum::extract::State<MockServerState>,
    ) -> impl IntoResponse {
        let n = state
            .request_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n < state.fail_count {
            // Return error status
            if state.status_code == axum::http::StatusCode::TOO_MANY_REQUESTS {
                (
                    state.status_code,
                    [(axum::http::header::RETRY_AFTER, "1")],
                    axum::Json(serde_json::json!({ "error": { "message": "Error" } })),
                )
                    .into_response()
            } else {
                (
                    state.status_code,
                    axum::Json(serde_json::json!({ "error": { "message": "Error" } })),
                )
                    .into_response()
            }
        } else {
            // Return success
            (
                axum::http::StatusCode::OK,
                axum::Json(serde_json::json!({
                    "id": "test-123",
                    "choices": [{
                        "message": { "role": "assistant", "content": "Hello" },
                        "finish_reason": "stop"
                    }],
                    "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
                })),
            )
                .into_response()
        }
    }

    #[tokio::test]
    async fn test_retry_on_429_rate_limit() {
        let state = MockServerState::new(2, axum::http::StatusCode::TOO_MANY_REQUESTS);
        let app = axum::Router::new()
            .route("/v1/chat/completions", axum::routing::post(mock_handler))
            .with_state(state.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let config = OpenAiCompatibleConfig {
            base_url: format!("http://127.0.0.1:{}", port),
            model: "test-model".to_string(),
            api_key: None,
            timeout_secs: 30,
            max_retries: 3,
            retry_initial_delay_ms: 10,
        };
        let provider = OpenAiCompatibleProvider::new(config).unwrap();

        let request = ChatCompletionRequest {
            model: "test-model".to_string(),
            messages: vec![ChatCompletionMessage {
                role: "user".to_string(),
                content: Some("Hello".to_string()),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            }],
            temperature: None,
            max_tokens: None,
            top_p: None,
            n: None,
            presence_penalty: None,
            frequency_penalty: None,
            seed: None,
            tools: None,
            tool_choice: None,
        };

        let result: Result<ChatCompletionResponse, LlmError> =
            provider.send_request_with_retry(&request).await;

        assert!(
            result.is_ok(),
            "Should succeed after retries: {:?}",
            result.err()
        );
        assert_eq!(state.count(), 3, "Should have made 3 requests");

        server.abort();
    }

    #[tokio::test]
    async fn test_retry_on_5xx_server_error() {
        let state = MockServerState::new(2, axum::http::StatusCode::SERVICE_UNAVAILABLE);

        // Build router with state
        let app = axum::Router::new()
            .route("/v1/chat/completions", axum::routing::post(mock_handler))
            .with_state(state.clone());

        // Bind to random port
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        // Spawn server with graceful shutdown
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await
                .unwrap();
        });

        // Wait for server to be ready
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let config = OpenAiCompatibleConfig {
            base_url: format!("http://127.0.0.1:{}", port),
            model: "test-model".to_string(),
            api_key: None,
            timeout_secs: 5, // Short timeout for tests
            max_retries: 3,
            retry_initial_delay_ms: 10,
        };
        let provider = OpenAiCompatibleProvider::new(config).unwrap();

        let request = ChatCompletionRequest {
            model: "test-model".to_string(),
            messages: vec![ChatCompletionMessage {
                role: "user".to_string(),
                content: Some("Hello".to_string()),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            }],
            temperature: None,
            max_tokens: None,
            top_p: None,
            n: None,
            presence_penalty: None,
            frequency_penalty: None,
            seed: None,
            tools: None,
            tool_choice: None,
        };

        // Use timeout to prevent hanging
        let result: Result<Result<ChatCompletionResponse, LlmError>, tokio::time::error::Elapsed> =
            tokio::time::timeout(
                std::time::Duration::from_secs(30),
                provider.send_request_with_retry(&request),
            )
            .await;

        // Shutdown server
        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(std::time::Duration::from_millis(500), server).await;

        match result {
            Ok(Ok(_)) => {
                assert_eq!(state.count(), 3, "Should have made 3 requests");
            }
            Ok(Err(e)) => {
                panic!("Request failed: {:?}. Request count: {}", e, state.count());
            }
            Err(_) => {
                panic!("Test timed out. Request count: {}", state.count());
            }
        }
    }

    #[tokio::test]
    async fn test_max_retries_respected() {
        // Server that always fails
        let state = MockServerState::new(usize::MAX, axum::http::StatusCode::SERVICE_UNAVAILABLE);
        let app = axum::Router::new()
            .route("/v1/chat/completions", axum::routing::post(mock_handler))
            .with_state(state.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await
                .unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let config = OpenAiCompatibleConfig {
            base_url: format!("http://127.0.0.1:{}", port),
            model: "test-model".to_string(),
            api_key: None,
            timeout_secs: 5,
            max_retries: 2, // Will try 3 times total (initial + 2 retries)
            retry_initial_delay_ms: 10,
        };
        let provider = OpenAiCompatibleProvider::new(config).unwrap();

        let request = ChatCompletionRequest {
            model: "test-model".to_string(),
            messages: vec![ChatCompletionMessage {
                role: "user".to_string(),
                content: Some("Hello".to_string()),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            }],
            temperature: None,
            max_tokens: None,
            top_p: None,
            n: None,
            presence_penalty: None,
            frequency_penalty: None,
            seed: None,
            tools: None,
            tool_choice: None,
        };

        let result: Result<Result<ChatCompletionResponse, LlmError>, tokio::time::error::Elapsed> =
            tokio::time::timeout(
                std::time::Duration::from_secs(30),
                provider.send_request_with_retry(&request),
            )
            .await;

        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(std::time::Duration::from_millis(500), server).await;

        match result {
            Ok(Err(_)) => {
                assert_eq!(
                    state.count(),
                    3,
                    "Should have made 3 requests (initial + 2 retries)"
                );
            }
            Ok(Ok(_)) => {
                panic!(
                    "Expected request to fail but it succeeded. Request count: {}",
                    state.count()
                );
            }
            Err(_) => {
                panic!("Test timed out. Request count: {}", state.count());
            }
        }
    }

    async fn auth_error_handler() -> impl IntoResponse {
        (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": { "message": "Invalid API key" } })),
        )
            .into_response()
    }

    #[tokio::test]
    async fn test_non_retryable_error_fails_immediately() {
        let _state = MockServerState::new(0, axum::http::StatusCode::OK);
        let app = axum::Router::new().route(
            "/v1/chat/completions",
            axum::routing::post(auth_error_handler),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let config = OpenAiCompatibleConfig {
            base_url: format!("http://127.0.0.1:{}", port),
            model: "test-model".to_string(),
            api_key: None,
            timeout_secs: 30,
            max_retries: 3,
            retry_initial_delay_ms: 10,
        };
        let provider = OpenAiCompatibleProvider::new(config).unwrap();

        let request = ChatCompletionRequest {
            model: "test-model".to_string(),
            messages: vec![ChatCompletionMessage {
                role: "user".to_string(),
                content: Some("Hello".to_string()),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            }],
            temperature: None,
            max_tokens: None,
            top_p: None,
            n: None,
            presence_penalty: None,
            frequency_penalty: None,
            seed: None,
            tools: None,
            tool_choice: None,
        };

        let result: Result<ChatCompletionResponse, LlmError> =
            provider.send_request_with_retry(&request).await;

        assert!(result.is_err(), "Should fail immediately");
        match result {
            Err(LlmError::AuthFailed { .. }) => (),
            Err(other) => panic!("Expected AuthFailed, got: {:?}", other),
            Ok(_) => panic!("Should have failed"),
        }

        server.abort();
    }

    async fn bad_request_handler() -> impl IntoResponse {
        (
            axum::http::StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "error": { "message": "Invalid request" } })),
        )
            .into_response()
    }

    #[tokio::test]
    async fn test_400_error_fails_immediately() {
        let app = axum::Router::new().route(
            "/v1/chat/completions",
            axum::routing::post(bad_request_handler),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let config = OpenAiCompatibleConfig {
            base_url: format!("http://127.0.0.1:{}", port),
            model: "test-model".to_string(),
            api_key: None,
            timeout_secs: 30,
            max_retries: 3,
            retry_initial_delay_ms: 10,
        };
        let provider = OpenAiCompatibleProvider::new(config).unwrap();

        let request = ChatCompletionRequest {
            model: "test-model".to_string(),
            messages: vec![ChatCompletionMessage {
                role: "user".to_string(),
                content: Some("Hello".to_string()),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            }],
            temperature: None,
            max_tokens: None,
            top_p: None,
            n: None,
            presence_penalty: None,
            frequency_penalty: None,
            seed: None,
            tools: None,
            tool_choice: None,
        };

        let result: Result<ChatCompletionResponse, LlmError> =
            provider.send_request_with_retry(&request).await;

        assert!(result.is_err(), "Should fail immediately");

        server.abort();
    }
}
