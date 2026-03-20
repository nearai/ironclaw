//! NEAR AI provider implementation (Chat Completions API).
//!
//! This provider uses the OpenAI-compatible Chat Completions endpoint with
//! dual auth support:
//! - **API key auth**: When `NEARAI_API_KEY` is set, uses Bearer API key
//! - **Session token auth**: Otherwise, uses `SessionManager` for Bearer session token
//!   with automatic renewal on 401 errors

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use rust_decimal::prelude::MathematicalOps;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use crate::llm::config::NearAiConfig;
use crate::llm::error::LlmError;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse,
};
use crate::llm::{costs, session::SessionManager};

/// Information about an available model from NEAR AI API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Model identifier.
    #[serde(alias = "id", alias = "model")]
    pub name: String,
    /// Optional provider name.
    #[serde(default)]
    pub provider: Option<String>,
}

/// NEAR AI provider (Chat Completions API, dual auth).
pub struct NearAiChatProvider {
    client: Client,
    config: NearAiConfig,
    /// Session manager for session token auth (used when no API key is set).
    session: Arc<SessionManager>,
    active_model: std::sync::RwLock<String>,
    flatten_tool_messages: bool,
    /// Per-model pricing fetched from the NEAR AI `/v1/model/list` endpoint.
    /// Maps model ID → (input_cost_per_token, output_cost_per_token).
    pricing: Arc<std::sync::RwLock<HashMap<String, (Decimal, Decimal)>>>,
}

impl NearAiChatProvider {
    /// Create a new NEAR AI Chat Completions provider.
    ///
    /// Auth mode is determined by `config.api_key`:
    /// - If set, uses Bearer API key auth
    /// - If not set, uses session token auth via `SessionManager`
    ///
    /// By default this enables tool-message flattening for compatibility with
    /// providers that reject `role: "tool"` messages.
    pub fn new(config: NearAiConfig, session: Arc<SessionManager>) -> Result<Self, LlmError> {
        Self::new_with_options(config, session, true, 120)
    }

    /// Create a new provider with a custom request timeout.
    pub fn new_with_timeout(
        config: NearAiConfig,
        session: Arc<SessionManager>,
        request_timeout_secs: u64,
    ) -> Result<Self, LlmError> {
        Self::new_with_options(config, session, true, request_timeout_secs)
    }

    /// Create a chat completions provider with configurable tool-message flattening
    /// and request timeout.
    pub fn new_with_options(
        config: NearAiConfig,
        session: Arc<SessionManager>,
        flatten_tool_messages: bool,
        request_timeout_secs: u64,
    ) -> Result<Self, LlmError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(request_timeout_secs))
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: "nearai_chat".to_string(),
                reason: format!("Failed to build HTTP client: {}", e),
            })?;

        let active_model = std::sync::RwLock::new(config.model.clone());
        let pricing = Arc::new(std::sync::RwLock::new(HashMap::new()));

        let provider = Self {
            client,
            config,
            session,
            active_model,
            flatten_tool_messages,
            pricing,
        };

        // Fire-and-forget background pricing fetch — don't block startup.
        // Only spawns when a tokio runtime is active (skipped in sync tests).
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let client = provider.client.clone();
            let base_url = provider.config.base_url.clone();
            let api_key = provider.config.api_key.clone();
            let session = provider.session.clone();
            let pricing = provider.pricing.clone();

            handle.spawn(async move {
                match fetch_pricing(&client, &base_url, api_key.as_ref(), &session).await {
                    Ok(map) if !map.is_empty() => {
                        tracing::debug!("Loaded NEAR AI pricing for {} model(s)", map.len());
                        match pricing.write() {
                            Ok(mut guard) => *guard = map,
                            Err(poisoned) => *poisoned.into_inner() = map,
                        }
                    }
                    Ok(_) => {
                        tracing::debug!("NEAR AI pricing endpoint returned no pricing data");
                    }
                    Err(e) => {
                        tracing::debug!(
                            "Could not fetch NEAR AI pricing (will use fallback): {}",
                            e
                        );
                    }
                }
            });
        }

        Ok(provider)
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

    /// Returns true if using API key auth, false if session token auth.
    fn uses_api_key(&self) -> bool {
        self.config.api_key.is_some()
    }

    /// Resolve the Bearer token for the current auth mode.
    ///
    /// Priority order:
    /// 1. `config.api_key` (set at construction from env/config)
    /// 2. Session token (OAuth flow)
    /// 3. `NEARAI_API_KEY` env var (set by interactive `api_key_login()`)
    ///
    /// The env var fallback (#3) only triggers after `ensure_authenticated()`
    /// runs, because `api_key_login()` sets the env var but not a session token.
    async fn resolve_bearer_token(&self) -> Result<String, LlmError> {
        // 1. Config-level API key takes priority
        if let Some(ref api_key) = self.config.api_key {
            return Ok(api_key.expose_secret().to_string());
        }

        // 2. Existing session token (OAuth was already completed)
        if self.session.has_token().await {
            let token = self.session.get_token().await?;
            return Ok(token.expose_secret().to_string());
        }

        // No token yet, trigger interactive login
        self.session.ensure_authenticated().await?;

        // 3. After login, check if a session token was stored (OAuth path)
        if self.session.has_token().await {
            let token = self.session.get_token().await?;
            return Ok(token.expose_secret().to_string());
        }

        // 4. api_key_login() sets NEARAI_API_KEY env var but not a session token
        if let Ok(key) = std::env::var("NEARAI_API_KEY")
            && !key.is_empty()
        {
            return Ok(key);
        }

        Err(LlmError::AuthFailed {
            provider: "nearai".to_string(),
        })
    }

    /// Send a single request to the chat completions API.
    ///
    /// For session token auth, handles 401 by calling `session.handle_auth_failure()`
    /// and retrying once.
    ///
    /// Does not retry on other errors — retries are handled by the external
    /// `RetryProvider` wrapper in the composition chain.
    async fn send_request<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        body: &T,
    ) -> Result<R, LlmError> {
        match self.send_request_inner(body).await {
            Ok(result) => Ok(result),
            Err(LlmError::SessionExpired { .. }) if !self.uses_api_key() => {
                // Session expired, attempt renewal and retry once
                self.session.handle_auth_failure().await?;
                self.send_request_inner(body).await
            }
            Err(e) => Err(e),
        }
    }

    /// Inner request implementation (single attempt).
    async fn send_request_inner<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        body: &T,
    ) -> Result<R, LlmError> {
        let url = self.api_url("chat/completions");
        let token = self.resolve_bearer_token().await?;

        tracing::debug!("Sending request to NEAR AI Chat: {}", url);

        if tracing::enabled!(tracing::Level::DEBUG)
            && let Ok(json) = serde_json::to_string(body)
        {
            tracing::debug!("NEAR AI Chat request body: {}", json);
        }

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: "nearai_chat".to_string(),
                reason: e.to_string(),
            })?;

        let status = response.status();
        // Extract Retry-After header before consuming the response body.
        let retry_after_header = Some(crate::llm::retry::parse_retry_after(
            response.headers().get("retry-after"),
        ));
        let response_text = response.text().await.map_err(|e| LlmError::RequestFailed {
            provider: "nearai_chat".to_string(),
            reason: format!("Failed to read response body: {}", e),
        })?;

        // Log response body only at TRACE level to avoid exposing sensitive content
        // (user-generated data, tool outputs, leaked secrets) in DEBUG logs
        if tracing::enabled!(tracing::Level::TRACE) {
            tracing::trace!("NEAR AI Chat response body: {}", response_text);
        }

        if !status.is_success() {
            let status_code = status.as_u16();

            if status_code == 401 {
                // For session token auth, distinguish session expired from plain auth failure
                if !self.uses_api_key() {
                    let lower = response_text.to_lowercase();
                    let is_session_expired = lower.contains("session")
                        && (lower.contains("expired") || lower.contains("invalid"));
                    if is_session_expired {
                        return Err(LlmError::SessionExpired {
                            provider: "nearai_chat".to_string(),
                        });
                    }
                }
                return Err(LlmError::AuthFailed {
                    provider: "nearai_chat".to_string(),
                });
            }

            if status_code == 429 {
                return Err(LlmError::RateLimited {
                    provider: "nearai_chat".to_string(),
                    retry_after: retry_after_header,
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

    /// Fetch available models from the NEAR AI API.
    ///
    /// Handles session renewal on 401 (same pattern as `send_request`).
    /// Supports multiple response formats: `{models: [...]}`, `{data: [...]}`, and plain array.
    pub async fn list_models_full(&self) -> Result<Vec<ModelInfo>, LlmError> {
        match self.list_models_inner().await {
            Ok(models) => Ok(models),
            Err(LlmError::SessionExpired { .. }) if !self.uses_api_key() => {
                self.session.handle_auth_failure().await?;
                self.list_models_inner().await
            }
            Err(e) => Err(e),
        }
    }

    async fn list_models_inner(&self) -> Result<Vec<ModelInfo>, LlmError> {
        let url = self.api_url("models");
        let token = self.resolve_bearer_token().await?;

        tracing::debug!("Fetching models from: {}", url);

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
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
            if status.as_u16() == 401 && !self.uses_api_key() {
                return Err(LlmError::SessionExpired {
                    provider: "nearai_chat".to_string(),
                });
            }
            let truncated = crate::agent::truncate_for_preview(&response_text, 512);
            return Err(LlmError::RequestFailed {
                provider: "nearai_chat".to_string(),
                reason: format!("HTTP {}: {}", status, truncated),
            });
        }

        // Flexible model entry parsing -- handle various field names
        #[derive(Deserialize)]
        struct ModelMetadataInner {
            #[serde(default)]
            name: Option<String>,
            #[serde(default, alias = "modelName", alias = "model_name")]
            model_name: Option<String>,
        }

        #[derive(Deserialize)]
        struct ModelEntry {
            #[serde(default)]
            name: Option<String>,
            #[serde(default)]
            id: Option<String>,
            #[serde(default)]
            model: Option<String>,
            #[serde(default, alias = "modelName", alias = "model_name")]
            model_name: Option<String>,
            #[serde(default, alias = "modelId", alias = "model_id")]
            model_id: Option<String>,
            #[serde(default)]
            metadata: Option<ModelMetadataInner>,
        }

        impl ModelEntry {
            fn get_name(&self) -> Option<String> {
                self.name
                    .clone()
                    .or_else(|| self.id.clone())
                    .or_else(|| self.model.clone())
                    .or_else(|| self.model_name.clone())
                    .or_else(|| self.model_id.clone())
                    .or_else(|| self.metadata.as_ref().and_then(|m| m.name.clone()))
                    .or_else(|| self.metadata.as_ref().and_then(|m| m.model_name.clone()))
            }
        }

        #[derive(Deserialize)]
        struct ModelsResponse {
            #[serde(default)]
            models: Option<Vec<ModelEntry>>,
            #[serde(default)]
            data: Option<Vec<ModelEntry>>,
        }

        // Try {models: [...]} or {data: [...]} format
        if let Ok(resp) = serde_json::from_str::<ModelsResponse>(&response_text)
            && let Some(entries) = resp.models.or(resp.data)
        {
            let models: Vec<ModelInfo> = entries
                .into_iter()
                .filter_map(|e| {
                    e.get_name().map(|name| ModelInfo {
                        name,
                        provider: None,
                    })
                })
                .collect();
            if !models.is_empty() {
                return Ok(models);
            }
        }

        // Try direct array format
        if let Ok(entries) = serde_json::from_str::<Vec<ModelEntry>>(&response_text) {
            let models: Vec<ModelInfo> = entries
                .into_iter()
                .filter_map(|e| {
                    e.get_name().map(|name| ModelInfo {
                        name,
                        provider: None,
                    })
                })
                .collect();
            if !models.is_empty() {
                return Ok(models);
            }
        }

        // Couldn't find model names in response
        Err(LlmError::InvalidResponse {
            provider: "nearai_chat".to_string(),
            reason: format!(
                "No model names found in response: {}",
                &response_text[..response_text.len().min(300)]
            ),
        })
    }
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
            stop: req.stop_sequences,
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

        // Fall back to reasoning_content when content is null (same as
        // complete_with_tools — reasoning models may put the answer there).
        let content = choice
            .message
            .content
            .or(choice.message.reasoning_content)
            .unwrap_or_default();
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
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
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
            stop: req.stop_sequences,
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

        // Fall back to reasoning_content when content is null (e.g. GLM-5
        // returns its answer in reasoning_content instead of content), but
        // only for final text responses. Tool-call responses often have
        // content: null + reasoning_content filled with chain-of-thought;
        // leaking that into conversation history inflates context and
        // confuses the model.
        let content = if tool_calls.is_empty() {
            choice.message.content.or(choice.message.reasoning_content)
        } else {
            choice.message.content
        };

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
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    fn model_name(&self) -> &str {
        &self.config.model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        let model = self.active_model_name();
        // Try fetched pricing first, then static lookup table, then default
        if let Ok(guard) = self.pricing.read()
            && let Some(&rates) = guard.get(&model)
        {
            return rates;
        }
        costs::model_cost(&model).unwrap_or_else(costs::default_cost)
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let models = self.list_models_full().await?;
        Ok(models.into_iter().map(|m| m.name).collect())
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
    stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ChatCompletionTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

/// Content field that serializes as either a string or an array of content parts.
///
/// - `Text("hello")` → `"content": "hello"`
/// - `Parts([...])` → `"content": [{"type": "text", ...}, {"type": "image_url", ...}]`
#[derive(Debug, Clone)]
enum MessageContent {
    Text(String),
    Parts(Vec<crate::llm::ContentPart>),
}

impl Serialize for MessageContent {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            MessageContent::Text(s) => serializer.serialize_str(s),
            MessageContent::Parts(parts) => parts.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for MessageContent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de;
        use serde_json::Value;

        let val = Value::deserialize(deserializer)?;
        match val {
            Value::String(s) => Ok(MessageContent::Text(s)),
            Value::Array(arr) => Ok(MessageContent::Text(
                // For deserialization (responses), we only need the text content
                arr.iter()
                    .find_map(|v| {
                        if v.get("type")?.as_str()? == "text" {
                            v.get("text")?.as_str().map(String::from)
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default(),
            )),
            Value::Null => Ok(MessageContent::Text(String::new())),
            _ => Err(de::Error::custom(
                "expected string, array, or null for content",
            )),
        }
    }
}

impl MessageContent {
    fn as_text(&self) -> Option<&str> {
        match self {
            MessageContent::Text(s) if !s.is_empty() => Some(s),
            MessageContent::Text(_) => None,
            MessageContent::Parts(_) => None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatCompletionMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<MessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ChatCompletionToolCall>>,
}

// -- Pricing fetch types and logic -----------------------------------------

/// Cost amount from the NEAR AI `/v1/model/list` response.
///
/// Real cost per token = `amount * 10^(-scale)`.
#[derive(Debug, Deserialize)]
struct ModelCost {
    amount: f64,
    #[serde(default)]
    scale: i32,
}

/// A single model entry from the pricing response.
#[derive(Debug, Deserialize)]
struct PricingModelEntry {
    #[serde(default, alias = "modelId", alias = "model_id")]
    model_id: Option<String>,
    #[serde(default, alias = "inputCostPerToken")]
    input_cost_per_token: Option<ModelCost>,
    #[serde(default, alias = "outputCostPerToken")]
    output_cost_per_token: Option<ModelCost>,
    #[serde(default)]
    metadata: Option<PricingMetadata>,
}

#[derive(Debug, Deserialize)]
struct PricingMetadata {
    #[serde(default)]
    aliases: Vec<String>,
}

/// Wrapper for the `/v1/model/list` response body.
#[derive(Debug, Deserialize)]
struct PricingResponse {
    #[serde(default)]
    models: Option<Vec<PricingModelEntry>>,
    #[serde(default)]
    data: Option<Vec<PricingModelEntry>>,
}

/// Convert a `ModelCost` to a `Decimal` per-token price.
fn model_cost_to_decimal(mc: &ModelCost) -> Option<Decimal> {
    if mc.amount == 0.0 {
        return Some(Decimal::ZERO);
    }
    // amount * 10^(-scale)
    let base = Decimal::try_from(mc.amount).ok()?;
    let factor = Decimal::TEN.checked_powi(-i64::from(mc.scale))?;
    base.checked_mul(factor)
}

/// Fetch pricing from the NEAR AI `/v1/model/list` endpoint.
///
/// Returns a map of model_id → (input_cost_per_token, output_cost_per_token).
/// Errors are non-fatal; callers should fall back to the static lookup table.
async fn fetch_pricing(
    client: &Client,
    base_url: &str,
    api_key: Option<&secrecy::SecretString>,
    session: &SessionManager,
) -> Result<HashMap<String, (Decimal, Decimal)>, LlmError> {
    let base = base_url.trim_end_matches('/');
    let url = if base.ends_with("/v1") {
        format!("{}/model/list", base)
    } else {
        format!("{}/v1/model/list", base)
    };

    let token = if let Some(key) = api_key {
        key.expose_secret().to_string()
    } else {
        let tok = session.get_token().await?;
        tok.expose_secret().to_string()
    };

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| LlmError::RequestFailed {
            provider: "nearai_chat".to_string(),
            reason: format!("Failed to fetch pricing: {}", e),
        })?;

    if !response.status().is_success() {
        return Err(LlmError::RequestFailed {
            provider: "nearai_chat".to_string(),
            reason: format!("Pricing endpoint returned HTTP {}", response.status()),
        });
    }

    let body = response.text().await.map_err(|e| LlmError::RequestFailed {
        provider: "nearai_chat".to_string(),
        reason: format!("Failed to read pricing response: {}", e),
    })?;

    // Parse as {models: [...]} or {data: [...]} or direct array
    let entries: Vec<PricingModelEntry> =
        if let Ok(resp) = serde_json::from_str::<PricingResponse>(&body) {
            resp.models.or(resp.data).unwrap_or_default()
        } else if let Ok(arr) = serde_json::from_str::<Vec<PricingModelEntry>>(&body) {
            arr
        } else {
            return Ok(HashMap::new());
        };

    let mut map = HashMap::new();
    for entry in &entries {
        let (Some(input_mc), Some(output_mc)) =
            (&entry.input_cost_per_token, &entry.output_cost_per_token)
        else {
            continue;
        };
        let (Some(input), Some(output)) = (
            model_cost_to_decimal(input_mc),
            model_cost_to_decimal(output_mc),
        ) else {
            continue;
        };

        // Insert under the primary model_id
        if let Some(ref id) = entry.model_id {
            map.insert(id.clone(), (input, output));
        }
        // Also insert under any aliases
        if let Some(ref meta) = entry.metadata {
            for alias in &meta.aliases {
                map.insert(alias.clone(), (input, output));
            }
        }
    }

    Ok(map)
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
                if let Some(text) = msg.content.as_ref().and_then(|c| c.as_text()) {
                    parts.push(text.to_string());
                }
                for tc in calls {
                    parts.push(format!(
                        "[Called tool `{}` with arguments: {}]",
                        tc.function.name, tc.function.arguments
                    ));
                }
                ChatCompletionMessage {
                    role: "assistant".to_string(),
                    content: Some(MessageContent::Text(parts.join("\n"))),

                    tool_call_id: None,
                    name: None,
                    tool_calls: None,
                }
            } else if msg.role == "tool" {
                // Convert tool result into a user message
                let tool_name = msg.name.as_deref().unwrap_or("unknown");
                let result = msg.content.as_ref().and_then(|c| c.as_text()).unwrap_or("");
                ChatCompletionMessage {
                    role: "user".to_string(),
                    content: Some(MessageContent::Text(format!(
                        "[Tool `{}` returned: {}]",
                        tool_name, result
                    ))),

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
        } else if !msg.content_parts.is_empty() {
            // Build multimodal content array: text + image parts
            let mut parts = vec![crate::llm::ContentPart::Text { text: msg.content }];
            parts.extend(msg.content_parts);
            Some(MessageContent::Parts(parts))
        } else {
            Some(MessageContent::Text(msg.content))
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
    /// Some models (e.g. GLM-5) return chain-of-thought reasoning here
    /// instead of in `content`.
    #[serde(default)]
    reasoning_content: Option<String>,
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
    use crate::llm::session::SessionConfig;
    use rust_decimal_macros::dec;

    fn test_nearai_config(base_url: &str) -> NearAiConfig {
        NearAiConfig {
            model: "test-model".to_string(),
            base_url: base_url.to_string(),
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
            smart_routing_cascade: true,
        }
    }

    fn test_session() -> Arc<SessionManager> {
        Arc::new(SessionManager::new(SessionConfig::default()))
    }

    #[test]
    fn test_api_url_with_base_without_v1() {
        let mut cfg = test_nearai_config("http://127.0.0.1:8318");

        let provider = NearAiChatProvider::new(cfg.clone(), test_session()).expect("provider");
        assert_eq!(
            provider.api_url("chat/completions"),
            "http://127.0.0.1:8318/v1/chat/completions"
        );

        cfg.base_url = "http://127.0.0.1:8318/".to_string();
        let provider = NearAiChatProvider::new(cfg, test_session()).expect("provider");
        assert_eq!(
            provider.api_url("/chat/completions"),
            "http://127.0.0.1:8318/v1/chat/completions"
        );
    }

    #[test]
    fn test_api_url_with_base_already_v1() {
        let cfg = test_nearai_config("http://127.0.0.1:8318/v1");

        let provider = NearAiChatProvider::new(cfg, test_session()).expect("provider");
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
        assert_eq!(
            chat_msg.content.as_ref().and_then(|c| c.as_text()),
            Some("Hello")
        );
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
                content: Some(MessageContent::Text("You are helpful.".to_string())),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            },
            ChatCompletionMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("Hello".to_string())),
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
                content: Some(MessageContent::Text("test".to_string())),
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
                content: Some(MessageContent::Text("hi".to_string())),
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
                .and_then(|c| c.as_text())
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
                .and_then(|c| c.as_text())
                .unwrap()
                .contains("[Tool `echo` returned: hi]")
        );
    }

    #[test]
    fn test_flatten_preserves_assistant_text_with_tool_calls() {
        let messages = vec![
            ChatCompletionMessage {
                role: "assistant".to_string(),
                content: Some(MessageContent::Text("Let me check that.".to_string())),
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
                content: Some(MessageContent::Text("found it".to_string())),
                tool_call_id: Some("call_1".to_string()),
                name: Some("search".to_string()),
                tool_calls: None,
            },
        ];

        let result = flatten_tool_messages(messages);
        let text = result[0]
            .content
            .as_ref()
            .and_then(|c| c.as_text())
            .unwrap();
        assert!(text.starts_with("Let me check that."));
        assert!(text.contains("[Called tool `search`"));
    }

    #[test]
    fn test_model_cost_to_decimal_basic() {
        // amount=3, scale=6 → 3 * 10^-6 = 0.000003
        let mc = ModelCost {
            amount: 3.0,
            scale: 6,
        };
        let result = model_cost_to_decimal(&mc).unwrap();
        assert_eq!(result, dec!(0.000003));
    }

    #[test]
    fn test_model_cost_to_decimal_zero() {
        let mc = ModelCost {
            amount: 0.0,
            scale: 6,
        };
        assert_eq!(model_cost_to_decimal(&mc), Some(Decimal::ZERO));
    }

    #[test]
    fn test_model_cost_to_decimal_larger_scale() {
        // amount=85, scale=8 → 85 * 10^-8 = 0.00000085
        let mc = ModelCost {
            amount: 85.0,
            scale: 8,
        };
        let result = model_cost_to_decimal(&mc).unwrap();
        assert_eq!(result, dec!(0.00000085));
    }

    #[test]
    fn test_cost_per_token_uses_pricing_map() {
        let cfg = test_nearai_config("http://127.0.0.1:8318");
        let provider = NearAiChatProvider::new(cfg, test_session()).expect("provider");

        // Inject pricing directly
        {
            let mut guard = provider.pricing.write().unwrap();
            guard.insert("test-model".to_string(), (dec!(0.000001), dec!(0.000005)));
        }

        let (input, output) = provider.cost_per_token();
        assert_eq!(input, dec!(0.000001));
        assert_eq!(output, dec!(0.000005));
    }

    #[test]
    fn test_cost_per_token_falls_back_to_static() {
        let mut cfg = test_nearai_config("http://127.0.0.1:8318");
        cfg.model = "gpt-4o".to_string();
        let provider = NearAiChatProvider::new(cfg, test_session()).expect("provider");

        // No pricing in map, should fall back to static costs::model_cost
        let (input, output) = provider.cost_per_token();
        let (expected_in, expected_out) = costs::model_cost("gpt-4o").unwrap();
        assert_eq!(input, expected_in);
        assert_eq!(output, expected_out);
    }

    #[test]
    fn test_cost_per_token_falls_back_to_default() {
        let mut cfg = test_nearai_config("http://127.0.0.1:8318");
        cfg.model = "some-unknown-nearai-model".to_string();
        let provider = NearAiChatProvider::new(cfg, test_session()).expect("provider");

        // No pricing in map, not in static table, should use default_cost
        let (input, output) = provider.cost_per_token();
        let (default_in, default_out) = costs::default_cost();
        assert_eq!(input, default_in);
        assert_eq!(output, default_out);
    }

    /// Regression: reasoning_content must NOT leak into tool-call responses.
    #[test]
    fn test_reasoning_content_not_leaked_into_tool_call_response() {
        let response: ChatCompletionResponse = serde_json::from_value(serde_json::json!({
            "id": "chatcmpl-test",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "reasoning_content": "Let me think about which tool to call...",
                    "tool_calls": [{
                        "id": "call_abc123",
                        "type": "function",
                        "function": {
                            "name": "search",
                            "arguments": "{\"query\":\"test\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": { "prompt_tokens": 100, "completion_tokens": 50 }
        }))
        .unwrap();

        let choice = response.choices.into_iter().next().unwrap();
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

        let content = if tool_calls.is_empty() {
            choice.message.content.or(choice.message.reasoning_content)
        } else {
            choice.message.content
        };

        assert!(
            content.is_none(),
            "reasoning_content should NOT leak into tool-call responses, got: {:?}",
            content
        );
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].name, "search");
    }

    /// Regression: reasoning_content SHOULD be used as fallback for text responses.
    #[test]
    fn test_reasoning_content_used_for_text_response() {
        let response: ChatCompletionResponse = serde_json::from_value(serde_json::json!({
            "id": "chatcmpl-test",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "reasoning_content": "The answer is 42."
                },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 50, "completion_tokens": 20 }
        }))
        .unwrap();

        let choice = response.choices.into_iter().next().unwrap();
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

        let content = if tool_calls.is_empty() {
            choice.message.content.or(choice.message.reasoning_content)
        } else {
            choice.message.content
        };

        assert_eq!(
            content,
            Some("The answer is 42.".to_string()),
            "reasoning_content should be used as fallback for text responses"
        );
        assert!(tool_calls.is_empty());
    }

    #[tokio::test]
    async fn test_resolve_bearer_token_config_api_key() {
        // When config.api_key is set, it takes top priority.
        let cfg = test_nearai_config("http://localhost:8318");
        let provider = NearAiChatProvider::new(cfg, test_session()).expect("provider");
        let token = provider
            .resolve_bearer_token()
            .await
            .expect("should resolve");
        assert_eq!(token, "test-key");
    }

    #[tokio::test]
    async fn test_resolve_bearer_token_session_token() {
        // When config.api_key is None but session has a token, use session token.
        let mut cfg = test_nearai_config("http://localhost:8318");
        cfg.api_key = None;
        let session = test_session();
        session
            .set_token(secrecy::SecretString::from("session-tok-123".to_string()))
            .await;
        let provider = NearAiChatProvider::new(cfg, session).expect("provider");
        let token = provider
            .resolve_bearer_token()
            .await
            .expect("should resolve");
        assert_eq!(token, "session-tok-123");
    }

    #[tokio::test]
    async fn test_resolve_bearer_token_session_beats_env_var() {
        // Session token takes priority over NEARAI_API_KEY env var.
        // This prevents unexpected auth mode switches mid-run.
        let mut cfg = test_nearai_config("http://localhost:8318");
        cfg.api_key = None;
        let session = test_session();
        session
            .set_token(secrecy::SecretString::from("oauth-token".to_string()))
            .await;

        // Set env var that should NOT be used when session token exists
        #[allow(unused_unsafe)]
        unsafe {
            std::env::set_var("NEARAI_API_KEY", "env-api-key-should-not-win");
        }

        let provider = NearAiChatProvider::new(cfg, session).expect("provider");
        let token = provider
            .resolve_bearer_token()
            .await
            .expect("should resolve");
        assert_eq!(
            token, "oauth-token",
            "session token must take priority over env var"
        );

        #[allow(unused_unsafe)]
        unsafe {
            std::env::remove_var("NEARAI_API_KEY");
        }
    }

    #[tokio::test]
    async fn test_resolve_bearer_token_config_beats_session_and_env() {
        // Config API key should win even when session token AND env var are set.
        let cfg = test_nearai_config("http://localhost:8318");
        let session = test_session();
        session
            .set_token(secrecy::SecretString::from("session-tok".to_string()))
            .await;

        #[allow(unused_unsafe)]
        unsafe {
            std::env::set_var("NEARAI_API_KEY", "env-key");
        }

        let provider = NearAiChatProvider::new(cfg, session).expect("provider");
        let token = provider
            .resolve_bearer_token()
            .await
            .expect("should resolve");
        assert_eq!(
            token, "test-key",
            "config api_key must win over session token and env var"
        );

        #[allow(unused_unsafe)]
        unsafe {
            std::env::remove_var("NEARAI_API_KEY");
        }
    }

    // -- ModelInfo serde alias tests ------------------------------------------

    #[test]
    fn test_model_info_deserialize_with_name_field() {
        let json = r#"{"name": "claude-3-5-sonnet"}"#;
        let info: ModelInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.name, "claude-3-5-sonnet");
        assert!(info.provider.is_none());
    }

    #[test]
    fn test_model_info_deserialize_with_id_alias() {
        let json = r#"{"id": "gpt-4o", "provider": "openai"}"#;
        let info: ModelInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.name, "gpt-4o");
        assert_eq!(info.provider, Some("openai".to_string()));
    }

    #[test]
    fn test_model_info_deserialize_with_model_alias() {
        let json = r#"{"model": "llama-3.1-70b"}"#;
        let info: ModelInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.name, "llama-3.1-70b");
    }

    #[test]
    fn test_model_info_roundtrip_serializes_as_name() {
        let info = ModelInfo {
            name: "test-model".to_string(),
            provider: Some("nearai".to_string()),
        };
        let json = serde_json::to_value(&info).unwrap();
        // Serialization always uses the field name "name", not the aliases
        assert_eq!(json["name"], "test-model");
        assert_eq!(json["provider"], "nearai");
        assert!(json.get("id").is_none());
        assert!(json.get("model").is_none());
    }

    // -- ChatCompletionRequest serialization ----------------------------------

    #[test]
    fn test_request_serialization_minimal() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".to_string(),
            messages: vec![ChatCompletionMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("Hello".to_string())),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            }],
            temperature: None,
            max_tokens: None,
            stop: None,
            tools: None,
            tool_choice: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["model"], "gpt-4o");
        assert_eq!(json["messages"][0]["role"], "user");
        assert_eq!(json["messages"][0]["content"], "Hello");
        // Optional fields should be absent, not null
        assert!(json.get("temperature").is_none());
        assert!(json.get("max_tokens").is_none());
        assert!(json.get("tools").is_none());
        assert!(json.get("tool_choice").is_none());
    }

    #[test]
    fn test_request_serialization_with_tools() {
        let req = ChatCompletionRequest {
            model: "gpt-4o".to_string(),
            messages: vec![],
            temperature: Some(0.7),
            max_tokens: Some(1024),
            stop: None,
            tools: Some(vec![ChatCompletionTool {
                tool_type: "function".to_string(),
                function: ChatCompletionFunction {
                    name: "get_weather".to_string(),
                    description: Some("Get the weather".to_string()),
                    parameters: Some(serde_json::json!({
                        "type": "object",
                        "properties": {
                            "city": {"type": "string"}
                        }
                    })),
                },
            }]),
            tool_choice: Some("auto".to_string()),
        };
        let json = serde_json::to_value(&req).unwrap();
        // f32 precision: 0.7f32 serializes as 0.699999988... in JSON
        let temp = json["temperature"].as_f64().unwrap();
        assert!(
            (temp - 0.7).abs() < 0.001,
            "temperature should be ~0.7, got {temp}"
        );
        assert_eq!(json["max_tokens"], 1024);
        assert_eq!(json["tool_choice"], "auto");
        // Tool uses "type" key (via rename), not "tool_type"
        assert_eq!(json["tools"][0]["type"], "function");
        assert_eq!(json["tools"][0]["function"]["name"], "get_weather");
    }

    #[test]
    fn test_request_omits_null_content_on_assistant_messages() {
        // When an assistant message has tool_calls but no content, content
        // should serialize as absent (skip_serializing_if) not "content": null.
        let msg = ChatCompletionMessage {
            role: "assistant".to_string(),
            content: None,
            tool_call_id: None,
            name: None,
            tool_calls: Some(vec![ChatCompletionToolCall {
                id: "call_1".to_string(),
                call_type: "function".to_string(),
                function: ChatCompletionToolCallFunction {
                    name: "echo".to_string(),
                    arguments: "{}".to_string(),
                },
            }]),
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert!(
            json.get("content").is_none(),
            "content should be omitted when None"
        );
        assert!(json.get("tool_call_id").is_none());
        assert!(json.get("name").is_none());
        assert!(json["tool_calls"].is_array());
    }

    // -- ChatCompletionResponse deserialization -------------------------------

    #[test]
    fn test_response_deserialize_basic() {
        let json = serde_json::json!({
            "id": "chatcmpl-abc123",
            "object": "chat.completion",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Hello!"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        });
        let resp: ChatCompletionResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.id, Some("chatcmpl-abc123".to_string()));
        assert_eq!(resp.choices.len(), 1);
        assert_eq!(resp.choices[0].message.content, Some("Hello!".to_string()));
        assert_eq!(resp.choices[0].finish_reason, Some("stop".to_string()));
        let usage = resp.usage.unwrap();
        assert_eq!(usage.prompt_tokens, Some(10));
        assert_eq!(usage.completion_tokens, Some(5));
        assert_eq!(usage.total_tokens, Some(15));
    }

    #[test]
    fn test_response_deserialize_missing_optional_fields() {
        // Minimal response: no id, no usage, no finish_reason
        let json = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Hi"
                },
                "finish_reason": null
            }]
        });
        let resp: ChatCompletionResponse = serde_json::from_value(json).unwrap();
        assert!(resp.id.is_none());
        assert!(resp.usage.is_none());
        assert!(resp.choices[0].finish_reason.is_none());
    }

    #[test]
    fn test_response_deserialize_with_tool_calls() {
        let json = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "call_abc",
                            "type": "function",
                            "function": {
                                "name": "get_weather",
                                "arguments": "{\"city\":\"NYC\"}"
                            }
                        },
                        {
                            "id": "call_def",
                            "type": "function",
                            "function": {
                                "name": "get_time",
                                "arguments": "{}"
                            }
                        }
                    ]
                },
                "finish_reason": "tool_calls"
            }]
        });
        let resp: ChatCompletionResponse = serde_json::from_value(json).unwrap();
        let tc = resp.choices[0].message.tool_calls.as_ref().unwrap();
        assert_eq!(tc.len(), 2);
        assert_eq!(tc[0].id, "call_abc");
        assert_eq!(tc[0].function.name, "get_weather");
        assert_eq!(tc[0].function.arguments, "{\"city\":\"NYC\"}");
        assert_eq!(tc[1].id, "call_def");
        assert_eq!(tc[1].function.name, "get_time");
    }

    #[test]
    fn test_response_deserialize_ignores_unknown_fields() {
        // Real API responses have extra fields like "object", "created", "model"
        let json = serde_json::json!({
            "id": "chatcmpl-xyz",
            "object": "chat.completion",
            "created": 1700000000,
            "model": "gpt-4o",
            "system_fingerprint": "fp_abc123",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "ok"
                },
                "finish_reason": "stop",
                "logprobs": null
            }],
            "usage": {
                "prompt_tokens": 5,
                "completion_tokens": 1,
                "total_tokens": 6
            }
        });
        let resp: ChatCompletionResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.choices[0].message.content, Some("ok".to_string()));
    }

    // -- parse_usage and saturate_u32 -----------------------------------------

    #[test]
    fn test_parse_usage_with_all_fields() {
        let usage = ChatCompletionUsage {
            prompt_tokens: Some(100),
            completion_tokens: Some(50),
            total_tokens: Some(150),
        };
        assert_eq!(parse_usage(Some(&usage)), (100, 50));
    }

    #[test]
    fn test_parse_usage_none() {
        assert_eq!(parse_usage(None), (0, 0));
    }

    #[test]
    fn test_parse_usage_missing_completion_falls_back_to_total_minus_prompt() {
        let usage = ChatCompletionUsage {
            prompt_tokens: Some(100),
            completion_tokens: None,
            total_tokens: Some(180),
        };
        // output = total - prompt = 80
        assert_eq!(parse_usage(Some(&usage)), (100, 80));
    }

    #[test]
    fn test_parse_usage_missing_completion_and_prompt_uses_total() {
        let usage = ChatCompletionUsage {
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: Some(200),
        };
        // input = 0 (no prompt), output = total = 200
        assert_eq!(parse_usage(Some(&usage)), (0, 200));
    }

    #[test]
    fn test_parse_usage_all_none() {
        let usage = ChatCompletionUsage {
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
        };
        assert_eq!(parse_usage(Some(&usage)), (0, 0));
    }

    #[test]
    fn test_saturate_u32_within_range() {
        assert_eq!(saturate_u32(0), 0);
        assert_eq!(saturate_u32(42), 42);
        assert_eq!(saturate_u32(u32::MAX as u64), u32::MAX);
    }

    #[test]
    fn test_saturate_u32_overflow_clamps() {
        assert_eq!(saturate_u32(u32::MAX as u64 + 1), u32::MAX);
        assert_eq!(saturate_u32(u64::MAX), u32::MAX);
    }

    // -- Pricing types deserialization ----------------------------------------

    #[test]
    fn test_model_cost_deserialize() {
        let json = r#"{"amount": 3.0, "scale": 6}"#;
        let mc: ModelCost = serde_json::from_str(json).unwrap();
        assert_eq!(mc.amount, 3.0);
        assert_eq!(mc.scale, 6);
    }

    #[test]
    fn test_model_cost_scale_defaults_to_zero() {
        let json = r#"{"amount": 0.5}"#;
        let mc: ModelCost = serde_json::from_str(json).unwrap();
        assert_eq!(mc.scale, 0);
    }

    #[test]
    fn test_model_cost_to_decimal_negative_scale() {
        // amount=2, scale=-3 → 2 * 10^3 = 2000
        let mc = ModelCost {
            amount: 2.0,
            scale: -3,
        };
        let result = model_cost_to_decimal(&mc).unwrap();
        assert_eq!(result, dec!(2000));
    }

    #[test]
    fn test_pricing_model_entry_deserialize_camel_case_aliases() {
        let json = serde_json::json!({
            "modelId": "claude-3-5-sonnet",
            "inputCostPerToken": {"amount": 3.0, "scale": 6},
            "outputCostPerToken": {"amount": 15.0, "scale": 6},
            "metadata": {"aliases": ["claude-sonnet", "claude-3.5-sonnet"]}
        });
        let entry: PricingModelEntry = serde_json::from_value(json).unwrap();
        assert_eq!(entry.model_id, Some("claude-3-5-sonnet".to_string()));
        let input = model_cost_to_decimal(entry.input_cost_per_token.as_ref().unwrap()).unwrap();
        assert_eq!(input, dec!(0.000003));
        let output = model_cost_to_decimal(entry.output_cost_per_token.as_ref().unwrap()).unwrap();
        assert_eq!(output, dec!(0.000015));
        assert_eq!(
            entry.metadata.unwrap().aliases,
            vec!["claude-sonnet", "claude-3.5-sonnet"]
        );
    }

    #[test]
    fn test_pricing_model_entry_deserialize_snake_case() {
        let json = serde_json::json!({
            "model_id": "gpt-4o",
            "input_cost_per_token": {"amount": 5.0, "scale": 6},
            "output_cost_per_token": {"amount": 15.0, "scale": 6}
        });
        let entry: PricingModelEntry = serde_json::from_value(json).unwrap();
        assert_eq!(entry.model_id, Some("gpt-4o".to_string()));
        assert!(entry.input_cost_per_token.is_some());
        assert!(entry.metadata.is_none());
    }

    #[test]
    fn test_pricing_response_models_wrapper() {
        let json = serde_json::json!({
            "models": [
                {"model_id": "m1", "input_cost_per_token": {"amount": 1.0, "scale": 6},
                 "output_cost_per_token": {"amount": 2.0, "scale": 6}}
            ]
        });
        let resp: PricingResponse = serde_json::from_value(json).unwrap();
        assert!(resp.models.is_some());
        assert_eq!(resp.models.unwrap().len(), 1);
        assert!(resp.data.is_none());
    }

    #[test]
    fn test_pricing_response_data_wrapper() {
        let json = serde_json::json!({
            "data": [
                {"model_id": "m1"},
                {"model_id": "m2"}
            ]
        });
        let resp: PricingResponse = serde_json::from_value(json).unwrap();
        assert!(resp.models.is_none());
        assert_eq!(resp.data.unwrap().len(), 2);
    }

    // -- flatten_tool_messages edge cases -------------------------------------

    #[test]
    fn test_flatten_tool_result_missing_name_uses_unknown() {
        let messages = vec![ChatCompletionMessage {
            role: "tool".to_string(),
            content: Some(MessageContent::Text("result data".to_string())),
            tool_call_id: Some("call_1".to_string()),
            name: None,
            tool_calls: None,
        }];
        let result = flatten_tool_messages(messages);
        assert_eq!(result[0].role, "user");
        assert!(
            result[0]
                .content
                .as_ref()
                .unwrap()
                .as_text()
                .unwrap()
                .contains("[Tool `unknown` returned:")
        );
    }

    #[test]
    fn test_flatten_tool_result_missing_content_uses_empty() {
        let messages = vec![ChatCompletionMessage {
            role: "tool".to_string(),
            content: None,
            tool_call_id: Some("call_1".to_string()),
            name: Some("my_tool".to_string()),
            tool_calls: None,
        }];
        let result = flatten_tool_messages(messages);
        assert_eq!(result[0].role, "user");
        assert!(
            result[0]
                .content
                .as_ref()
                .unwrap()
                .as_text()
                .unwrap()
                .contains("[Tool `my_tool` returned: ]")
        );
    }

    #[test]
    fn test_flatten_multiple_tool_calls_in_single_assistant_message() {
        let messages = vec![
            ChatCompletionMessage {
                role: "assistant".to_string(),
                content: None,
                tool_call_id: None,
                name: None,
                tool_calls: Some(vec![
                    ChatCompletionToolCall {
                        id: "call_1".to_string(),
                        call_type: "function".to_string(),
                        function: ChatCompletionToolCallFunction {
                            name: "search".to_string(),
                            arguments: r#"{"q":"a"}"#.to_string(),
                        },
                    },
                    ChatCompletionToolCall {
                        id: "call_2".to_string(),
                        call_type: "function".to_string(),
                        function: ChatCompletionToolCallFunction {
                            name: "fetch".to_string(),
                            arguments: r#"{"url":"http://x"}"#.to_string(),
                        },
                    },
                ]),
            },
            ChatCompletionMessage {
                role: "tool".to_string(),
                content: Some(MessageContent::Text("found".to_string())),
                tool_call_id: Some("call_1".to_string()),
                name: Some("search".to_string()),
                tool_calls: None,
            },
            ChatCompletionMessage {
                role: "tool".to_string(),
                content: Some(MessageContent::Text("fetched".to_string())),
                tool_call_id: Some("call_2".to_string()),
                name: Some("fetch".to_string()),
                tool_calls: None,
            },
        ];
        let result = flatten_tool_messages(messages);
        assert_eq!(result.len(), 3);
        // Assistant message has both calls described
        let assistant_text = result[0].content.as_ref().unwrap().as_text().unwrap();
        assert!(assistant_text.contains("[Called tool `search`"));
        assert!(assistant_text.contains("[Called tool `fetch`"));
        assert!(result[0].tool_calls.is_none());
        // Both tool results become user messages
        assert_eq!(result[1].role, "user");
        assert_eq!(result[2].role, "user");
    }

    // -- ChatMessage → ChatCompletionMessage edge cases -----------------------

    #[test]
    fn test_assistant_empty_content_with_tool_calls_becomes_none() {
        // When content is empty string and tool_calls are present, content
        // should be None to avoid sending `"content": ""` which some APIs reject.
        let msg = ChatMessage::assistant_with_tool_calls(
            None,
            vec![ToolCall {
                id: "call_1".to_string(),
                name: "test".to_string(),
                arguments: serde_json::json!({}),
            }],
        );
        let chat_msg: ChatCompletionMessage = msg.into();
        assert!(
            chat_msg.content.is_none(),
            "empty content with tool_calls should serialize as None"
        );
    }

    #[test]
    fn test_system_message_conversion() {
        let msg = ChatMessage::system("You are a helpful assistant.");
        let chat_msg: ChatCompletionMessage = msg.into();
        assert_eq!(chat_msg.role, "system");
        assert_eq!(
            chat_msg.content.as_ref().unwrap().as_text().unwrap(),
            "You are a helpful assistant."
        );
        assert!(chat_msg.tool_calls.is_none());
        assert!(chat_msg.tool_call_id.is_none());
    }

    // -- ChatCompletionUsage deserialization -----------------------------------

    #[test]
    fn test_usage_deserialize_partial_fields() {
        // Some providers only return total_tokens
        let json = r#"{"total_tokens": 500}"#;
        let usage: ChatCompletionUsage = serde_json::from_str(json).unwrap();
        assert!(usage.prompt_tokens.is_none());
        assert!(usage.completion_tokens.is_none());
        assert_eq!(usage.total_tokens, Some(500));
    }

    #[test]
    fn test_usage_deserialize_empty_object() {
        let json = "{}";
        let usage: ChatCompletionUsage = serde_json::from_str(json).unwrap();
        assert!(usage.prompt_tokens.is_none());
        assert!(usage.completion_tokens.is_none());
        assert!(usage.total_tokens.is_none());
    }

    // -- ChatCompletionToolCall serde roundtrip --------------------------------

    #[test]
    fn test_tool_call_serde_roundtrip() {
        let tc = ChatCompletionToolCall {
            id: "call_abc".to_string(),
            call_type: "function".to_string(),
            function: ChatCompletionToolCallFunction {
                name: "get_weather".to_string(),
                arguments: r#"{"city":"London"}"#.to_string(),
            },
        };
        let json = serde_json::to_value(&tc).unwrap();
        // "type" not "call_type" in serialized form
        assert_eq!(json["type"], "function");
        assert!(json.get("call_type").is_none());
        assert_eq!(json["id"], "call_abc");

        // Deserialize back
        let deserialized: ChatCompletionToolCall = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.id, "call_abc");
        assert_eq!(deserialized.call_type, "function");
        assert_eq!(deserialized.function.name, "get_weather");
        assert_eq!(deserialized.function.arguments, r#"{"city":"London"}"#);
    }

    // -- api_url edge cases ---------------------------------------------------

    #[test]
    fn test_api_url_with_trailing_v1_slash() {
        let cfg = test_nearai_config("http://example.com/v1/");
        let provider = NearAiChatProvider::new(cfg, test_session()).expect("provider");
        // Trailing slash gets trimmed, then /v1 is detected
        assert_eq!(provider.api_url("models"), "http://example.com/v1/models");
    }

    #[test]
    fn test_api_url_with_deep_base_path() {
        let cfg = test_nearai_config("http://example.com/api/proxy");
        let provider = NearAiChatProvider::new(cfg, test_session()).expect("provider");
        assert_eq!(
            provider.api_url("chat/completions"),
            "http://example.com/api/proxy/v1/chat/completions"
        );
    }
}
