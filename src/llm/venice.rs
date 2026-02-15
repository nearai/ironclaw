//! Venice AI inference API provider implementation.
//!
//! Native reqwest-based provider that supports Venice-specific `venice_parameters`
//! (web search, web scraping, thinking control) and dynamic model pricing
//! fetched from the Venice `/models` API.

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use crate::config::VeniceConfig;
use crate::error::LlmError;
use crate::llm::costs;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ModelMetadata,
    Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse,
};

/// Venice AI inference API provider.
///
/// # Concurrency safety
///
/// Uses `std::sync::RwLock` (not `tokio::sync::RwLock`) for `active_model` and
/// `model_catalog`. This is safe because lock guards are **never** held across
/// `.await` points — all lock-guarded sections do in-memory reads/writes only.
/// If you add async work inside a lock scope, you must switch to `tokio::sync::RwLock`
/// or restructure to drop the guard before the `.await`.
pub struct VeniceProvider {
    client: Client,
    config: VeniceConfig,
    active_model: std::sync::RwLock<String>,
    /// Cached model catalog with pricing and context lengths.
    model_catalog: std::sync::RwLock<ModelCatalog>,
}

/// Cached model catalog fetched from the Venice `/models` API.
struct ModelCatalog {
    models: Vec<VeniceModelInfo>,
    /// When the catalog was last fetched. `None` means never fetched.
    fetched_at: Option<std::time::Instant>,
}

/// Model info parsed from the Venice `/models` API response.
#[derive(Debug, Clone)]
struct VeniceModelInfo {
    id: String,
    context_length: Option<u32>,
    input_cost_per_million: Option<Decimal>,
    output_cost_per_million: Option<Decimal>,
}

/// One hour TTL for the model catalog cache.
const CATALOG_TTL: std::time::Duration = std::time::Duration::from_secs(3600);

impl VeniceProvider {
    /// Create a new Venice provider. The model catalog is fetched lazily on first use.
    pub fn new(config: VeniceConfig) -> Result<Self, LlmError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_else(|_| Client::new());

        let active_model = std::sync::RwLock::new(config.model.clone());

        // Start with an empty catalog; it will be populated on first use.
        let model_catalog = std::sync::RwLock::new(ModelCatalog {
            models: Vec::new(),
            fetched_at: None,
        });

        Ok(Self {
            client,
            config,
            active_model,
            model_catalog,
        })
    }

    fn api_url(&self, path: &str) -> String {
        let base = self.config.base_url.trim_end_matches('/');
        let path = path.trim_start_matches('/');
        format!("{}/{}", base, path)
    }

    fn api_key(&self) -> String {
        self.config.api_key.expose_secret().to_string()
    }

    /// Send a request to the Venice chat completions API.
    async fn send_request<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        body: &T,
    ) -> Result<R, LlmError> {
        let url = self.api_url("chat/completions");

        tracing::debug!("Sending request to Venice API: {}", url);

        if tracing::enabled!(tracing::Level::DEBUG) {
            if let Ok(json) = serde_json::to_string(body) {
                tracing::debug!("Venice request body: {}", json);
            }
        }

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key()))
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await
            .map_err(|e| {
                tracing::error!("Venice API request failed: {}", e);
                LlmError::RequestFailed {
                    provider: "venice".to_string(),
                    reason: e.to_string(),
                }
            })?;

        let status = response.status();
        let response_text = response.text().await.unwrap_or_default();

        tracing::debug!("Venice response status: {}", status);
        tracing::debug!("Venice response body: {}", response_text);

        if !status.is_success() {
            if status.as_u16() == 401 {
                return Err(LlmError::AuthFailed {
                    provider: "venice".to_string(),
                });
            }
            if status.as_u16() == 429 {
                return Err(LlmError::RateLimited {
                    provider: "venice".to_string(),
                    retry_after: None,
                });
            }
            return Err(LlmError::RequestFailed {
                provider: "venice".to_string(),
                reason: format!("HTTP {}: {}", status, response_text),
            });
        }

        serde_json::from_str(&response_text).map_err(|e| LlmError::InvalidResponse {
            provider: "venice".to_string(),
            reason: format!("JSON parse error: {}. Raw: {}", e, response_text),
        })
    }

    /// Fetch available models from the Venice `/models` API.
    async fn fetch_models(&self) -> Result<Vec<VeniceModelInfo>, LlmError> {
        let url = self.api_url("models");

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key()))
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: "venice".to_string(),
                reason: format!("Failed to fetch models: {}", e),
            })?;

        let status = response.status();
        let response_text = response.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(LlmError::RequestFailed {
                provider: "venice".to_string(),
                reason: format!("HTTP {}: {}", status, response_text),
            });
        }

        let resp: VeniceModelsResponse =
            serde_json::from_str(&response_text).map_err(|e| LlmError::InvalidResponse {
                provider: "venice".to_string(),
                reason: format!("JSON parse error: {}", e),
            })?;

        Ok(resp
            .data
            .into_iter()
            .map(|entry| {
                let spec = entry.model_spec;
                let context_length = spec
                    .as_ref()
                    .and_then(|s| s.available_context_tokens);
                let input_cost_per_million = spec
                    .as_ref()
                    .and_then(|s| s.pricing.as_ref())
                    .and_then(|p| p.input.as_ref())
                    .and_then(|i| i.usd);
                let output_cost_per_million = spec
                    .as_ref()
                    .and_then(|s| s.pricing.as_ref())
                    .and_then(|p| p.output.as_ref())
                    .and_then(|o| o.usd);
                VeniceModelInfo {
                    id: entry.id,
                    context_length,
                    input_cost_per_million,
                    output_cost_per_million,
                }
            })
            .collect())
    }

    /// Refresh the model catalog if it's stale (older than CATALOG_TTL) or never fetched.
    async fn refresh_catalog_if_stale(&self) {
        let is_stale = {
            let catalog = self
                .model_catalog
                .read()
                .expect("model_catalog lock poisoned");
            match catalog.fetched_at {
                None => true,
                Some(t) => t.elapsed() > CATALOG_TTL,
            }
        };

        if is_stale {
            match self.fetch_models().await {
                Ok(models) => {
                    let mut catalog = self
                        .model_catalog
                        .write()
                        .expect("model_catalog lock poisoned");
                    catalog.models = models;
                    catalog.fetched_at = Some(std::time::Instant::now());
                    tracing::debug!(
                        "Venice model catalog refreshed ({} models)",
                        catalog.models.len()
                    );
                }
                Err(e) => {
                    tracing::warn!("Failed to refresh Venice model catalog: {}", e);
                    // Keep the stale cache rather than erroring
                }
            }
        }
    }

    /// Look up cost per token for the active model from the cached catalog.
    fn cost_from_catalog(&self) -> Option<(Decimal, Decimal)> {
        let active = self
            .active_model
            .read()
            .expect("active_model lock poisoned")
            .clone();
        let catalog = self
            .model_catalog
            .read()
            .expect("model_catalog lock poisoned");

        catalog.models.iter().find(|m| m.id == active).and_then(|m| {
            match (m.input_cost_per_million, m.output_cost_per_million) {
                (Some(input_per_m), Some(output_per_m)) => {
                    // Convert USD per million tokens → USD per token
                    let million = Decimal::from(1_000_000);
                    Some((input_per_m / million, output_per_m / million))
                }
                _ => None,
            }
        })
    }

    /// Build the `venice_parameters` object from config fields.
    fn build_venice_parameters(&self) -> Option<VeniceParameters> {
        let has_any = self.config.web_search.is_some()
            || self.config.web_scraping.is_some()
            || self.config.include_venice_system_prompt.is_some();

        if !has_any {
            return None;
        }

        Some(VeniceParameters {
            enable_web_search: self.config.web_search.clone(),
            enable_web_scraping: self.config.web_scraping,
            include_venice_system_prompt: self.config.include_venice_system_prompt,
        })
    }
}

#[async_trait]
impl LlmProvider for VeniceProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let messages: Vec<ChatCompletionMessage> =
            req.messages.into_iter().map(|m| m.into()).collect();

        let request = VeniceChatRequest {
            model: self.active_model_name(),
            messages,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            tools: None,
            tool_choice: None,
            venice_parameters: self.build_venice_parameters(),
        };

        let response: ChatCompletionResponse = self.send_request(&request).await?;

        let choice =
            response
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| LlmError::InvalidResponse {
                    provider: "venice".to_string(),
                    reason: "No choices in response".to_string(),
                })?;

        let content = choice.message.content.unwrap_or_default();
        let finish_reason = parse_finish_reason(choice.finish_reason.as_deref());

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

        // Venice supports full tool calling protocol natively (role: "tool" works),
        // so no flatten_tool_messages needed.

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

        let request = VeniceChatRequest {
            model: self.active_model_name(),
            messages,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            tools: if tools.is_empty() { None } else { Some(tools) },
            tool_choice: req.tool_choice,
            venice_parameters: self.build_venice_parameters(),
        };

        let response: ChatCompletionResponse = self.send_request(&request).await?;

        let choice =
            response
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| LlmError::InvalidResponse {
                    provider: "venice".to_string(),
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

        let finish_reason = match parse_finish_reason(choice.finish_reason.as_deref()) {
            FinishReason::Unknown if !tool_calls.is_empty() => FinishReason::ToolUse,
            other => other,
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
        self.cost_from_catalog().unwrap_or_else(costs::default_cost)
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.refresh_catalog_if_stale().await;
        let catalog = self
            .model_catalog
            .read()
            .expect("model_catalog lock poisoned");
        Ok(catalog.models.iter().map(|m| m.id.clone()).collect())
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.refresh_catalog_if_stale().await;
        let active = self.active_model_name();
        let catalog = self
            .model_catalog
            .read()
            .expect("model_catalog lock poisoned");
        let current = catalog.models.iter().find(|m| m.id == active);
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

fn parse_finish_reason(reason: Option<&str>) -> FinishReason {
    match reason {
        Some("stop") => FinishReason::Stop,
        Some("length") => FinishReason::Length,
        Some("tool_calls") => FinishReason::ToolUse,
        Some("content_filter") => FinishReason::ContentFilter,
        _ => FinishReason::Unknown,
    }
}

// ── Venice-specific request types ──────────────────────────────────────

#[derive(Debug, Serialize)]
struct VeniceChatRequest {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    venice_parameters: Option<VeniceParameters>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct VeniceParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    enable_web_search: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enable_web_scraping: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    include_venice_system_prompt: Option<bool>,
}

// ── OpenAI-compatible Chat Completions types ───────────────────────────

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

// ── Venice /models API response types ──────────────────────────────────

#[derive(Debug, Deserialize)]
struct VeniceModelsResponse {
    data: Vec<VeniceApiModelEntry>,
}

#[derive(Debug, Deserialize)]
struct VeniceApiModelEntry {
    id: String,
    #[serde(default)]
    model_spec: Option<VeniceModelSpec>,
}

#[derive(Debug, Deserialize)]
struct VeniceModelSpec {
    #[serde(default, rename = "availableContextTokens")]
    available_context_tokens: Option<u32>,
    #[serde(default)]
    pricing: Option<VeniceModelPricing>,
}

#[derive(Debug, Deserialize)]
struct VeniceModelPricing {
    #[serde(default)]
    input: Option<VeniceModelPriceTier>,
    #[serde(default)]
    output: Option<VeniceModelPriceTier>,
}

#[derive(Debug, Deserialize)]
struct VeniceModelPriceTier {
    #[serde(default)]
    usd: Option<Decimal>,
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_message_conversion() {
        let msg = ChatMessage::user("Hello");
        let chat_msg: ChatCompletionMessage = msg.into();
        assert_eq!(chat_msg.role, "user");
        assert_eq!(chat_msg.content, Some("Hello".to_string()));

        let msg = ChatMessage::assistant("Hi there");
        let chat_msg: ChatCompletionMessage = msg.into();
        assert_eq!(chat_msg.role, "assistant");
        assert_eq!(chat_msg.content, Some("Hi there".to_string()));
    }

    #[test]
    fn test_system_message_conversion() {
        let msg = ChatMessage::system("You are a helpful assistant");
        let chat_msg: ChatCompletionMessage = msg.into();
        assert_eq!(chat_msg.role, "system");
        assert_eq!(
            chat_msg.content,
            Some("You are a helpful assistant".to_string())
        );
    }

    #[test]
    fn test_tool_message_conversion() {
        let msg = ChatMessage::tool_result("call_123", "my_tool", "result");
        let chat_msg: ChatCompletionMessage = msg.into();
        assert_eq!(chat_msg.role, "tool");
        assert_eq!(chat_msg.tool_call_id, Some("call_123".to_string()));
        assert_eq!(chat_msg.name, Some("my_tool".to_string()));
        assert_eq!(chat_msg.content, Some("result".to_string()));
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
        let parsed: serde_json::Value =
            serde_json::from_str(&calls[0].function.arguments).expect("valid JSON string");
        assert_eq!(parsed["key"], "value");
    }

    #[test]
    fn test_venice_parameters_serialization() {
        let params = VeniceParameters {
            enable_web_search: Some("on".to_string()),
            enable_web_scraping: Some(true),
            include_venice_system_prompt: Some(false),
        };

        let json = serde_json::to_value(&params).expect("serialize");
        assert_eq!(json["enable_web_search"], "on");
        assert_eq!(json["enable_web_scraping"], true);
        assert_eq!(json["include_venice_system_prompt"], false);

        // Verify the fields appear in a full request
        let request = VeniceChatRequest {
            model: "test-model".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            tools: None,
            tool_choice: None,
            venice_parameters: Some(params),
        };
        let json = serde_json::to_value(&request).expect("serialize");
        assert!(json["venice_parameters"].is_object());
        assert_eq!(json["venice_parameters"]["enable_web_search"], "on");
    }

    #[test]
    fn test_venice_parameters_none_when_unconfigured() {
        let request = VeniceChatRequest {
            model: "test-model".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            tools: None,
            tool_choice: None,
            venice_parameters: None,
        };
        let json = serde_json::to_value(&request).expect("serialize");
        // venice_parameters should be absent from JSON when None
        assert!(json.get("venice_parameters").is_none());
    }

    #[test]
    fn test_build_venice_parameters_none_when_unconfigured() {
        let config = VeniceConfig {
            api_key: secrecy::SecretString::from("test-key".to_string()),
            base_url: "https://api.venice.ai/api/v1".to_string(),
            model: "test-model".to_string(),
            web_search: None,
            web_scraping: None,
            include_venice_system_prompt: None,
        };
        let provider = VeniceProvider::new(config).expect("create provider");
        assert!(provider.build_venice_parameters().is_none());
    }

    #[test]
    fn test_build_venice_parameters_some_when_any_field_set() {
        // Only web_search set
        let config = VeniceConfig {
            api_key: secrecy::SecretString::from("test-key".to_string()),
            base_url: "https://api.venice.ai/api/v1".to_string(),
            model: "test-model".to_string(),
            web_search: Some("on".to_string()),
            web_scraping: None,
            include_venice_system_prompt: None,
        };
        let provider = VeniceProvider::new(config).expect("create provider");
        let params = provider.build_venice_parameters().expect("should be Some");
        assert_eq!(params.enable_web_search, Some("on".to_string()));
        assert!(params.enable_web_scraping.is_none());
        assert!(params.include_venice_system_prompt.is_none());

        // Only include_venice_system_prompt set
        let config = VeniceConfig {
            api_key: secrecy::SecretString::from("test-key".to_string()),
            base_url: "https://api.venice.ai/api/v1".to_string(),
            model: "test-model".to_string(),
            web_search: None,
            web_scraping: None,
            include_venice_system_prompt: Some(false),
        };
        let provider = VeniceProvider::new(config).expect("create provider");
        let params = provider.build_venice_parameters().expect("should be Some");
        assert!(params.enable_web_search.is_none());
        assert_eq!(params.include_venice_system_prompt, Some(false));
    }

    #[test]
    fn test_cost_from_catalog() {
        let config = VeniceConfig {
            api_key: secrecy::SecretString::from("test-key".to_string()),
            base_url: "https://api.venice.ai/api/v1".to_string(),
            model: "test-model".to_string(),
            web_search: None,
            web_scraping: None,
            include_venice_system_prompt: None,
        };

        let provider = VeniceProvider::new(config).expect("create provider");

        // Populate catalog with a model that has pricing
        {
            let mut catalog = provider.model_catalog.write().unwrap();
            catalog.models = vec![VeniceModelInfo {
                id: "test-model".to_string(),
                context_length: Some(128_000),
                input_cost_per_million: Some(dec!(0.2)),
                output_cost_per_million: Some(dec!(0.9)),
            }];
            catalog.fetched_at = Some(std::time::Instant::now());
        }

        let (input, output) = provider.cost_per_token();
        // 0.2 USD/million = 0.0000002 USD/token
        assert_eq!(input, dec!(0.0000002));
        // 0.9 USD/million = 0.0000009 USD/token
        assert_eq!(output, dec!(0.0000009));
    }

    #[test]
    fn test_cost_fallback_when_model_missing() {
        let config = VeniceConfig {
            api_key: secrecy::SecretString::from("test-key".to_string()),
            base_url: "https://api.venice.ai/api/v1".to_string(),
            model: "nonexistent-model".to_string(),
            web_search: None,
            web_scraping: None,
            include_venice_system_prompt: None,
        };

        let provider = VeniceProvider::new(config).expect("create provider");

        // Empty catalog — model won't be found
        let (input, output) = provider.cost_per_token();
        let (default_input, default_output) = costs::default_cost();
        assert_eq!(input, default_input);
        assert_eq!(output, default_output);
    }
}
