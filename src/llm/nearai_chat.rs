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

use crate::config::NearAiConfig;
use crate::error::LlmError;
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
        Self::new_with_flatten(config, session, true)
    }

    /// Create a chat completions provider with configurable tool-message flattening.
    pub fn new_with_flatten(
        config: NearAiConfig,
        session: Arc<SessionManager>,
        flatten_tool_messages: bool,
    ) -> Result<Self, LlmError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
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
                        tracing::info!("Loaded NEAR AI pricing for {} model(s)", map.len());
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
    async fn resolve_bearer_token(&self) -> Result<String, LlmError> {
        if let Some(ref api_key) = self.config.api_key {
            Ok(api_key.expose_secret().to_string())
        } else {
            let token = self.session.get_token().await?;
            Ok(token.expose_secret().to_string())
        }
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

        if tracing::enabled!(tracing::Level::TRACE)
            && let Ok(json) = serde_json::to_string(body)
        {
            tracing::trace!("NEAR AI Chat request body: {}", json);
        } else if tracing::enabled!(tracing::Level::DEBUG)
            && let Ok(json_val) = serde_json::to_value(body)
        {
            let msg_count = json_val
                .get("messages")
                .and_then(|m| m.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let model = json_val
                .get("model")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            let body_len = serde_json::to_string(&json_val)
                .map(|s| s.len())
                .unwrap_or(0);
            tracing::debug!(
                model = %model,
                message_count = msg_count,
                body_bytes = body_len,
                "NEAR AI Chat request metadata"
            );
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
        let response_text = response.text().await.map_err(|e| LlmError::RequestFailed {
            provider: "nearai_chat".to_string(),
            reason: format!("Failed to read response body: {}", e),
        })?;

        tracing::debug!(
            status = %status,
            body_bytes = response_text.len(),
            "NEAR AI Chat response metadata"
        );
        tracing::trace!("NEAR AI Chat response body: {}", response_text);

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
        let boundary = crate::util::floor_char_boundary(&response_text, 300);
        Err(LlmError::InvalidResponse {
            provider: "nearai_chat".to_string(),
            reason: format!(
                "No model names found in response: {}",
                &response_text[..boundary]
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
            cached: false,
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

        // Fall back to reasoning_content when content is null (e.g. GLM-5
        // returns its answer in reasoning_content instead of content).
        let content = choice.message.content.or(choice.message.reasoning_content);
        let tool_calls: Vec<ToolCall> = choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|tc| {
                let arguments =
                    serde_json::from_str(&tc.function.arguments).unwrap_or_else(|err| {
                        let boundary =
                            crate::util::floor_char_boundary(&tc.function.arguments, 200);
                        tracing::warn!(
                            tool = %tc.function.name,
                            error = %err,
                            raw_arguments = %&tc.function.arguments[..boundary],
                            "Failed to parse tool call arguments; defaulting to {{}}"
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

        let (input_tokens, output_tokens) = parse_usage(response.usage.as_ref());

        Ok(ToolCompletionResponse {
            content,
            tool_calls,
            finish_reason,
            input_tokens,
            output_tokens,
        })
    }

    fn provider_name(&self) -> &str {
        "nearai"
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
            auth_base_url: "https://private.near.ai".to_string(),
            session_path: std::path::PathBuf::from("/tmp/session.json"),
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

    /// H1 regression: byte-slicing a multi-byte UTF-8 string at an arbitrary
    /// byte offset panics when the offset lands mid-character. This test
    /// constructs a 301-byte string where byte 300 is inside a 4-byte emoji,
    /// proving the old pattern `&s[..s.len().min(300)]` would panic.
    #[test]
    fn test_truncate_multibyte_response_no_panic() {
        // 298 ASCII bytes + one 4-byte emoji (U+1F600 = 😀) = 302 bytes total.
        // Byte 300 falls inside the emoji (bytes 298..302), so slicing at 300
        // would hit a non-char-boundary.
        let mut text = "x".repeat(298);
        text.push('😀'); // 4-byte character
        assert_eq!(text.len(), 302);
        assert!(!text.is_char_boundary(300)); // confirms the trap

        // Old code would panic here:
        let result = std::panic::catch_unwind(|| {
            let _ = &text[..text.len().min(300)];
        });
        assert!(result.is_err(), "byte-slice at 300 should panic");

        // Safe truncation via truncate_for_preview must not panic.
        let safe = crate::agent::truncate_for_preview(&text, 300);
        assert!(safe.len() <= 302); // at most the full string
        assert!(!safe.is_empty());
    }

    #[test]
    fn test_malformed_tool_call_arguments_defaults_to_empty_object() {
        // Simulate the parsing logic from complete_with_tools: when the LLM
        // returns unparseable JSON in tool call arguments, the code should
        // default to {} (empty object) rather than panicking.
        let malformed = "this is not json {{{";
        let result: serde_json::Value = serde_json::from_str(malformed)
            .unwrap_or_else(|_err| serde_json::Value::Object(Default::default()));
        assert_eq!(result, serde_json::json!({}));

        // Valid JSON should parse normally.
        let valid = r#"{"query": "test"}"#;
        let result: serde_json::Value = serde_json::from_str(valid)
            .unwrap_or_else(|_err| serde_json::Value::Object(Default::default()));
        assert_eq!(result, serde_json::json!({"query": "test"}));
    }

    #[test]
    fn test_malformed_tool_call_arguments_multibyte_truncation() {
        // The warn! log truncates raw arguments to 200 bytes using
        // floor_char_boundary. Verify it doesn't panic on multi-byte input.
        let mut args = "invalid json: ".to_string();
        args.push_str(&"🔧".repeat(60)); // 60 × 4 = 240 bytes of emoji
        assert!(args.len() > 200);

        let boundary = crate::util::floor_char_boundary(&args, 200);
        let truncated = &args[..boundary];
        assert!(truncated.len() <= 200);
        // Must not panic — boundary is on a char boundary.
        assert!(args.is_char_boundary(boundary));
    }
}
