//! GitHub Copilot provider (direct HTTP with token exchange).
//!
//! The GitHub Copilot API at `api.githubcopilot.com` speaks OpenAI Chat
//! Completions format but requires a two-step authentication flow:
//! 1. A long-lived GitHub OAuth token (from device login or IDE sign-in)
//! 2. A short-lived Copilot session token (exchanged via GitHub API)
//!
//! The standard OpenAI rig-core client sends `Authorization: Bearer <token>`
//! with the raw OAuth token, which gets rejected with "Authorization header
//! is badly formatted". This provider handles the token exchange transparently.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::config::RegistryProviderConfig;
use crate::error::LlmError;
use crate::github_copilot_auth::CopilotTokenManager;
#[cfg(test)]
use crate::provider::PROMPT_CACHE_KEY_METADATA;
use crate::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, ContentPart, FinishReason, LlmProvider,
    Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse, ToolDefinition,
    openai_json_schema_response_format, prompt_cache_key_from_metadata,
    strip_unsupported_completion_params, strip_unsupported_tool_params,
};
use crate::tool_schema::{ToolSchemaPolicy, shape_tool_schema};
use ironclaw_common::llm_costs as costs;

/// Map an HTTP error status + response body to a context-length error when it
/// indicates the prompt exceeded the model's context window.
///
/// Returns `Some(LlmError::ContextLengthExceeded { .. })` for HTTP 413 or for
/// an HTTP 400 whose body matches a context-overflow pattern, and `None`
/// otherwise. Delegates to the shared `crate::error::context_length_error`
/// helper so detection stays consistent across direct-HTTP providers.
#[cfg(test)]
fn context_length_error_for_status(status_code: u16, response_text: &str) -> Option<LlmError> {
    crate::error::context_length_error(status_code, response_text)
}

/// GitHub Copilot provider with automatic token exchange.
pub(crate) struct GithubCopilotProvider {
    client: Client,
    token_manager: Arc<CopilotTokenManager>,
    model: String,
    base_url: String,
    active_model: std::sync::RwLock<String>,
    extra_headers: Vec<(String, String)>,
    /// Parameter names that this provider does not support.
    unsupported_params: HashSet<String>,
}

impl GithubCopilotProvider {
    pub(crate) fn new(
        config: &RegistryProviderConfig,
        request_timeout_secs: u64,
    ) -> Result<Self, LlmError> {
        let oauth_token = config
            .api_key
            .as_ref()
            .map(|k| k.expose_secret().to_string())
            .ok_or_else(|| {
                tracing::error!("No API key configured for github_copilot — check GITHUB_COPILOT_TOKEN env var or secrets store");
                LlmError::AuthFailed {
                    provider: "github_copilot".to_string(),
                }
            })?;

        let client = crate::config::hardened_client_builder(request_timeout_secs)
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: "github_copilot".to_string(),
                reason: format!("Failed to build HTTP client: {e}"),
            })?;

        let token_manager = Arc::new(CopilotTokenManager::new(client.clone(), oauth_token));

        let base_url = if config.base_url.is_empty() {
            "https://api.githubcopilot.com".to_string()
        } else {
            config.base_url.clone()
        };

        let active_model = std::sync::RwLock::new(config.model.clone());
        let unsupported_params: HashSet<String> =
            config.unsupported_params.iter().cloned().collect();

        Ok(Self {
            client,
            token_manager,
            model: config.model.clone(),
            base_url,
            active_model,
            extra_headers: config.extra_headers.clone(),
            unsupported_params,
        })
    }

    fn api_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        format!("{base}/chat/completions")
    }

    /// Strip unsupported fields from a `CompletionRequest` in place.
    fn strip_unsupported_completion_params(&self, req: &mut CompletionRequest) {
        strip_unsupported_completion_params(&self.unsupported_params, req);
    }

    /// Strip unsupported fields from a `ToolCompletionRequest` in place.
    fn strip_unsupported_tool_params(&self, req: &mut ToolCompletionRequest) {
        strip_unsupported_tool_params(&self.unsupported_params, req);
    }

    /// Resolve the derived prompt-cache-key value for this request's
    /// metadata, unless an operator listed `"prompt_cache_key"` in
    /// `unsupported_params` to suppress it (e.g. a Copilot-compatible proxy
    /// that 400s on unknown fields).
    fn prompt_cache_key(
        &self,
        metadata: &std::collections::HashMap<String, String>,
    ) -> Option<String> {
        prompt_cache_key_from_metadata(&self.unsupported_params, metadata)
    }

    fn map_token_error(error: crate::github_copilot_auth::GithubCopilotAuthError) -> LlmError {
        tracing::warn!(error = %error, "Copilot: token exchange failed");
        match error {
            crate::github_copilot_auth::GithubCopilotAuthError::AccessDenied
            | crate::github_copilot_auth::GithubCopilotAuthError::Expired => LlmError::AuthFailed {
                provider: "github_copilot".to_string(),
            },
            _ => LlmError::RequestFailed {
                provider: "github_copilot".to_string(),
                reason: format!("Token exchange failed: {error}"),
            },
        }
    }

    async fn send_authenticated_request(
        &self,
        url: &str,
        token: &SecretString,
        body: &impl Serialize,
    ) -> Result<reqwest::Response, LlmError> {
        let mut request = self
            .client
            .post(url)
            .bearer_auth(token.expose_secret())
            .header("Content-Type", "application/json");
        for (key, value) in &self.extra_headers {
            request = request.header(key.as_str(), value.as_str());
        }
        request.json(body).send().await.map_err(|error| {
            tracing::warn!(error = %error, "Copilot: HTTP request failed");
            LlmError::RequestFailed {
                provider: "github_copilot".to_string(),
                reason: error.to_string(),
            }
        })
    }

    async fn send_request<R: for<'de> Deserialize<'de>>(
        &self,
        body: &impl Serialize,
    ) -> Result<R, LlmError> {
        let url = self.api_url();
        // Distinguish permanent auth errors (non-retryable) from transient
        // network failures (retryable) so RetryProvider handles them correctly.
        let token = self
            .token_manager
            .get_token()
            .await
            .map_err(Self::map_token_error)?;
        let mut response = self.send_authenticated_request(&url, &token, body).await?;

        if response.status().as_u16() == 401 {
            tracing::warn!(
                "Copilot: 401 Unauthorized — refreshing the session token and retrying once"
            );
            let _ = response.text().await;
            self.token_manager.invalidate().await;
            let refreshed = self
                .token_manager
                .get_token()
                .await
                .map_err(Self::map_token_error)?;
            response = self
                .send_authenticated_request(&url, &refreshed, body)
                .await?;
        }

        let status = response.status();

        if !status.is_success() {
            // Use shared retry-after parser (supports HTTP-date, default 60s)
            let retry_after = crate::retry::retry_after_for_status(
                status.as_u16(),
                response.headers().get(reqwest::header::RETRY_AFTER),
            );

            let response_text = response
                .text()
                .await
                .unwrap_or_else(|e| format!("(failed to read error body: {e})"));

            tracing::warn!(
                status = %status,
                body = %ironclaw_common::truncate_for_preview(&response_text, 256),
                "Copilot: API error response"
            );

            return Err(crate::error::map_provider_http_error(
                crate::error::ProviderHttpError {
                    adapter: crate::error::ProductionModelAdapter::GithubCopilot,
                    model: &self.active_model_name(),
                    status: status.as_u16(),
                    body: &response_text,
                    retry_after,
                },
            ));
        }

        let response_text = response.text().await.map_err(|e| LlmError::RequestFailed {
            provider: "github_copilot".to_string(),
            reason: format!("Failed to read response body: {e}"),
        })?;

        serde_json::from_str(&response_text).map_err(|e| {
            let truncated = ironclaw_common::truncate_for_preview(&response_text, 512);
            tracing::warn!(
                error = %e,
                body = %truncated,
                "Copilot: failed to parse response JSON"
            );
            LlmError::InvalidResponse {
                provider: "github_copilot".to_string(),
                reason: format!("JSON parse error: {e}. Raw: {truncated}"),
            }
        })
    }
}

#[async_trait]
impl LlmProvider for GithubCopilotProvider {
    fn provider_id(&self) -> String {
        "github_copilot".to_string()
    }

    async fn complete(&self, mut req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let model = req
            .take_model_override()
            .unwrap_or_else(|| self.active_model_name());
        self.strip_unsupported_completion_params(&mut req);
        let messages = convert_messages(req.messages);

        let prompt_cache_key = self.prompt_cache_key(&req.metadata);
        let request = OpenAiRequest {
            model,
            messages,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            stop: req.stop_sequences,
            response_format: req.response_format.map(openai_json_schema_response_format),
            tools: None,
            tool_choice: None,
            prompt_cache_key,
        };

        let response: OpenAiResponse = self.send_request(&request).await?;
        let choice =
            response
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| LlmError::EmptyResponse {
                    provider: "github_copilot".to_string(),
                })?;

        let (content, _tool_calls) = extract_choice_content(&choice);

        let finish_reason = match choice.finish_reason.as_deref() {
            Some("stop") => FinishReason::Stop,
            Some("length") => FinishReason::Length,
            Some("tool_calls") => FinishReason::ToolUse,
            Some("content_filter") => FinishReason::ContentFilter,
            _ => FinishReason::Unknown,
        };

        Ok(CompletionResponse {
            content: content.unwrap_or_default(),
            finish_reason,
            input_tokens: response
                .usage
                .as_ref()
                .map(|u| u.prompt_tokens)
                .unwrap_or(0),
            output_tokens: response
                .usage
                .as_ref()
                .map(|u| u.completion_tokens)
                .unwrap_or(0),
            reasoning: None,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        })
    }

    async fn complete_with_tools(
        &self,
        mut req: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let model = req
            .take_model_override()
            .unwrap_or_else(|| self.active_model_name());
        self.strip_unsupported_tool_params(&mut req);
        let messages = convert_messages(req.messages);

        let tools: Vec<OpenAiTool> = req.tools.into_iter().map(convert_tool_definition).collect();

        let tool_choice = req.tool_choice.map(|tc| match tc.as_str() {
            "auto" | "required" | "none" => serde_json::Value::String(tc),
            specific => serde_json::json!({
                "type": "function",
                "function": {"name": specific}
            }),
        });

        let prompt_cache_key = self.prompt_cache_key(&req.metadata);
        let request = OpenAiRequest {
            model,
            messages,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            stop: req.stop_sequences,
            response_format: req.response_format.map(openai_json_schema_response_format),
            tools: if tools.is_empty() { None } else { Some(tools) },
            tool_choice,
            prompt_cache_key,
        };

        let response: OpenAiResponse = self.send_request(&request).await?;
        let choice =
            response
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| LlmError::EmptyResponse {
                    provider: "github_copilot".to_string(),
                })?;

        let (content, tool_calls) = extract_choice_content(&choice);

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
            input_tokens: response
                .usage
                .as_ref()
                .map(|u| u.prompt_tokens)
                .unwrap_or(0),
            output_tokens: response
                .usage
                .as_ref()
                .map(|u| u.completion_tokens)
                .unwrap_or(0),
            cache_creation_input_tokens: 0,
            reasoning: None,
            reasoning_details: None,
            cache_read_input_tokens: 0,
        })
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        let model = self.active_model_name();
        costs::model_cost(&model).unwrap_or_else(costs::default_cost)
    }

    fn active_model_name(&self) -> String {
        match self.active_model.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        match self.active_model.write() {
            Ok(mut guard) => {
                *guard = model.to_string();
            }
            Err(poisoned) => {
                *poisoned.into_inner() = model.to_string();
            }
        }
        Ok(())
    }
}

// --- OpenAI Chat Completions API types ---

#[derive(Debug, Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<serde_json::Value>,
    /// Stable per-conversation routing key for the Copilot backend's
    /// OpenAI-compatible prompt cache (`crate::provider::PROMPT_CACHE_KEY_METADATA`).
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<String>,
}

#[derive(Debug, Serialize)]
struct OpenAiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<OpenAiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

/// OpenAI content can be a plain string or an array of parts (for multimodal).
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum OpenAiContent {
    Text(String),
    Parts(Vec<OpenAiContentPart>),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum OpenAiContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: OpenAiImageUrl },
}

#[derive(Debug, Serialize)]
struct OpenAiImageUrl {
    url: String,
    detail: String,
}

#[derive(Debug, Serialize)]
struct OpenAiToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OpenAiToolCallFunction,
}

#[derive(Debug, Serialize)]
struct OpenAiToolCallFunction {
    name: String,
    arguments: String,
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

/// Convert a canonical tool definition to Copilot's OpenAI-compatible wire
/// shape. The runtime keeps the original schema for argument validation; only
/// this transient provider request flattens unsupported top-level combinators.
fn convert_tool_definition(tool: ToolDefinition) -> OpenAiTool {
    let mut description = tool.description;
    let parameters = shape_tool_schema(
        ToolSchemaPolicy::FlattenOnly,
        &tool.parameters,
        &mut description,
    );

    OpenAiTool {
        tool_type: "function".to_string(),
        function: OpenAiFunction {
            name: tool.name,
            description,
            parameters,
        },
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiResponseToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseToolCall {
    id: String,
    function: OpenAiResponseFunction,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

/// Convert IronClaw messages to OpenAI Chat Completions format.
fn convert_messages(messages: Vec<ChatMessage>) -> Vec<OpenAiMessage> {
    messages
        .into_iter()
        .map(|msg| match msg.role {
            Role::System => OpenAiMessage {
                role: "system".to_string(),
                content: Some(OpenAiContent::Text(msg.content)),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            Role::User | Role::HostReminder => {
                let content = if msg.content_parts.is_empty() {
                    Some(OpenAiContent::Text(msg.content))
                } else {
                    let mut parts = Vec::with_capacity(1 + msg.content_parts.len());
                    if !msg.content.is_empty() {
                        parts.push(OpenAiContentPart::Text { text: msg.content });
                    }
                    for part in msg.content_parts {
                        match part {
                            ContentPart::Text { text } => {
                                parts.push(OpenAiContentPart::Text { text });
                            }
                            ContentPart::ImageUrl { image_url } => {
                                let detail = image_url.normalized_openai_detail();
                                parts.push(OpenAiContentPart::ImageUrl {
                                    image_url: OpenAiImageUrl {
                                        url: image_url.url,
                                        detail,
                                    },
                                });
                            }
                        }
                    }
                    Some(OpenAiContent::Parts(parts))
                };
                OpenAiMessage {
                    role: "user".to_string(),
                    content,
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                }
            }
            Role::Assistant => {
                let tool_calls = msg.tool_calls.map(|calls| {
                    calls
                        .into_iter()
                        .map(|tc| OpenAiToolCall {
                            id: tc.id,
                            call_type: "function".to_string(),
                            function: OpenAiToolCallFunction {
                                name: tc.name,
                                arguments: tc.arguments.to_string(),
                            },
                        })
                        .collect()
                });
                let content = if msg.content.is_empty() {
                    None
                } else {
                    Some(OpenAiContent::Text(msg.content))
                };
                OpenAiMessage {
                    role: "assistant".to_string(),
                    content,
                    tool_calls,
                    tool_call_id: None,
                    name: None,
                }
            }
            Role::Tool => OpenAiMessage {
                role: "tool".to_string(),
                content: Some(OpenAiContent::Text(msg.content)),
                tool_calls: None,
                tool_call_id: msg.tool_call_id,
                name: msg.name,
            },
        })
        .collect()
}

/// Extract text and tool calls from an OpenAI response choice.
fn extract_choice_content(choice: &OpenAiChoice) -> (Option<String>, Vec<ToolCall>) {
    let content = choice.message.content.clone();
    let tool_calls = choice
        .message
        .tool_calls
        .as_ref()
        .map(|calls| {
            calls
                .iter()
                .map(|tc| ToolCall {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments: serde_json::from_str(&tc.function.arguments)
                        .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
                    reasoning: None,
                    signature: None,
                    arguments_parse_error: None,
                })
                .collect()
        })
        .unwrap_or_default();

    (content, tool_calls)
}

// Keep the provider implementation within the architecture file-size budget.
#[cfg(test)]
mod tests;
