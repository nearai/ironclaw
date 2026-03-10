//! Codex ChatGPT Responses API provider.
//!
//! Implements `LlmProvider` by speaking the OpenAI Responses API protocol
//! (`POST /responses`) used by the ChatGPT backend at
//! `chatgpt.com/backend-api/codex`. This bypasses `rig-core`'s Chat
//! Completions path, which is incompatible with this endpoint.
//!
//! # Warning
//!
//! The ChatGPT backend endpoint (`chatgpt.com/backend-api/codex`) is a
//! **private, undocumented API**. Using subscriber OAuth tokens from a
//! third-party application may violate the token's intended scope or
//! OpenAI's Terms of Service. This feature is provided as-is for
//! convenience and may break without notice.

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};

use super::codex_auth;
use crate::error::LlmError;

use super::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, ContentPart, FinishReason, LlmProvider,
    Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse, ToolDefinition,
};

/// Provider that speaks the Responses API protocol against the ChatGPT backend.
pub struct CodexChatGptProvider {
    client: Client,
    base_url: String,
    api_key: RwLock<String>,
    /// User-configured model name (or empty/"default" for auto-detect).
    configured_model: String,
    /// Lazily resolved model name (populated on first LLM call).
    resolved_model: tokio::sync::OnceCell<String>,
    /// OAuth refresh token for automatic 401 retry.
    refresh_token: Option<SecretString>,
    /// Path to auth.json for persisting refreshed tokens.
    auth_path: Option<PathBuf>,
    /// Timeout for actual `/responses` requests.
    request_timeout: Duration,
    /// Prevent concurrent 401 handlers from racing the same refresh token.
    refresh_lock: Mutex<()>,
}

impl CodexChatGptProvider {
    pub fn new(base_url: &str, api_key: &str, model: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: RwLock::new(api_key.to_string()),
            configured_model: model.to_string(),
            resolved_model: tokio::sync::OnceCell::const_new(),
            refresh_token: None,
            auth_path: None,
            request_timeout: Duration::from_secs(120),
            refresh_lock: Mutex::new(()),
        }
    }

    /// Create a provider with lazy model detection.
    ///
    /// The model is **not** resolved during construction. Instead, it is
    /// resolved on the first LLM call via [`resolve_model`], avoiding the
    /// need for `block_in_place` / `block_on` during provider setup.
    ///
    /// **Model selection priority** (applied at resolution time):
    /// 1. If `configured_model` is non-empty, validate it against the
    ///    `/models` endpoint. If it isn't in the supported list, log a
    ///    warning with available models and fall back to the top model.
    /// 2. If `configured_model` is empty (or a generic placeholder like
    ///    "default"), auto-detect the highest-priority model from the API.
    pub fn with_lazy_model(
        base_url: &str,
        api_key: &str,
        configured_model: &str,
        refresh_token: Option<SecretString>,
        auth_path: Option<PathBuf>,
        request_timeout_secs: u64,
    ) -> Self {
        tracing::warn!(
            "Codex ChatGPT provider uses a private, undocumented API \
             (chatgpt.com/backend-api/codex). This may violate OpenAI's \
             Terms of Service and could break without notice."
        );

        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: RwLock::new(api_key.to_string()),
            configured_model: configured_model.to_string(),
            resolved_model: tokio::sync::OnceCell::const_new(),
            refresh_token,
            auth_path,
            request_timeout: Duration::from_secs(request_timeout_secs),
            refresh_lock: Mutex::new(()),
        }
    }

    /// Resolve the model to use, lazily on first call.
    ///
    /// Uses `OnceCell` so the `/models` fetch happens at most once.
    async fn resolve_model(&self) -> &str {
        self.resolved_model
            .get_or_init(|| async {
                let api_key = self.api_key.read().await.clone();
                let available =
                    Self::fetch_available_models(&self.client, &self.base_url, &api_key).await;

                let configured = &self.configured_model;
                if !configured.is_empty() && configured != "default" {
                    // User explicitly configured a model — validate it
                    if available.is_empty() {
                        tracing::warn!(
                            "Could not fetch model list; using configured model '{configured}'"
                        );
                        return configured.clone();
                    }
                    if available.iter().any(|m| m == configured) {
                        tracing::info!(model = %configured, "Codex ChatGPT: using configured model");
                        return configured.clone();
                    }
                    tracing::warn!(
                        configured = %configured,
                        available = ?available,
                        "Configured model not found in supported list, falling back to top model"
                    );
                    available
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| configured.clone())
                } else {
                    // No user preference — auto-detect
                    if let Some(top) = available.into_iter().next() {
                        tracing::info!(model = %top, "Codex ChatGPT: auto-detected model");
                        top
                    } else {
                        tracing::warn!(
                            "Could not auto-detect model, using fallback '{configured}'"
                        );
                        configured.clone()
                    }
                }
            })
            .await
    }

    /// Query `/models?client_version=0.111.0` and return the list of available
    /// model slugs, ordered by priority (highest first).
    async fn fetch_available_models(client: &Client, base_url: &str, api_key: &str) -> Vec<String> {
        let url = format!("{base_url}/models?client_version=0.111.0");
        let resp = match client.get(&url).bearer_auth(api_key).timeout(Duration::from_secs(10)).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Failed to fetch Codex models: {e}");
                return Vec::new();
            }
        };
        if !resp.status().is_success() {
            tracing::warn!(status = %resp.status(), "Failed to fetch Codex models");
            return Vec::new();
        }
        let body: Value = match resp.json().await {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        // The response has { "models": [ { "slug": "...", ... }, ... ] }
        body.get("models")
            .and_then(|m| m.as_array())
            .map(|models| {
                models
                    .iter()
                    .filter_map(|m| m.get("slug").and_then(|s| s.as_str()).map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Convert IronClaw messages to Responses API request JSON.
    fn build_request_body(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
        tool_choice: Option<&str>,
    ) -> Value {
        // Extract system instructions
        let instructions: String = messages
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");

        // Convert non-system messages to Responses API input items
        let input: Vec<Value> = messages
            .iter()
            .filter(|m| m.role != Role::System)
            .flat_map(|m| Self::message_to_input_items(m))
            .collect();

        // Convert tool definitions
        let api_tools: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                })
            })
            .collect();

        let mut body = json!({
            "model": model,
            "instructions": instructions,
            "input": input,
            "stream": true,
            "store": false,
        });

        if !api_tools.is_empty() {
            body["tools"] = json!(api_tools);
            body["tool_choice"] = json!(tool_choice.unwrap_or("auto"));
        }

        body
    }

    /// Convert a single ChatMessage to one or more Responses API input items.
    fn message_to_input_items(msg: &ChatMessage) -> Vec<Value> {
        let mut items = Vec::new();

        match msg.role {
            Role::User => {
                // Build content array: if content_parts is populated, use it
                // to include multimodal content (images). Otherwise fall back
                // to the plain text content field.
                let content = if !msg.content_parts.is_empty() {
                    msg.content_parts
                        .iter()
                        .map(|part| match part {
                            ContentPart::Text { text } => json!({
                                "type": "input_text",
                                "text": text,
                            }),
                            ContentPart::ImageUrl { image_url } => json!({
                                "type": "input_image",
                                "image_url": image_url.url,
                            }),
                        })
                        .collect::<Vec<_>>()
                } else {
                    vec![json!({
                        "type": "input_text",
                        "text": msg.content,
                    })]
                };

                items.push(json!({
                    "type": "message",
                    "role": "user",
                    "content": content,
                }));
            }
            Role::Assistant => {
                // If the assistant message has tool calls, emit function_call items
                if let Some(ref tool_calls) = msg.tool_calls {
                    // Emit the assistant text as a message if non-empty
                    if !msg.content.is_empty() {
                        items.push(json!({
                            "type": "message",
                            "role": "assistant",
                            "content": [{
                                "type": "output_text",
                                "text": msg.content,
                            }],
                        }));
                    }
                    for tc in tool_calls {
                        let args = if tc.arguments.is_string() {
                            tc.arguments.as_str().unwrap_or("{}").to_string()
                        } else {
                            serde_json::to_string(&tc.arguments).unwrap_or_default()
                        };
                        items.push(json!({
                            "type": "function_call",
                            "name": tc.name,
                            "arguments": args,
                            "call_id": tc.id,
                        }));
                    }
                } else {
                    items.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{
                            "type": "output_text",
                            "text": msg.content,
                        }],
                    }));
                }
            }
            Role::Tool => {
                items.push(json!({
                    "type": "function_call_output",
                    "call_id": msg.tool_call_id.as_deref().unwrap_or(""),
                    "output": msg.content,
                }));
            }
            Role::System => {
                // System messages are handled via `instructions` field
            }
        }

        items
    }

    /// Send a request and parse the SSE response.
    ///
    /// On HTTP 401, if a refresh token is available, attempts to refresh
    /// the access token and retry the request once.
    async fn send_request(&self, body: Value) -> Result<ResponsesResult, LlmError> {
        let url = format!("{}/responses", self.base_url);

        tracing::debug!(
            url = %url,
            model = %body.get("model").and_then(|m| m.as_str()).unwrap_or("?"),
            "Codex ChatGPT: sending request"
        );

        let api_key = self.api_key.read().await.clone();
        let resp = Self::send_http_request(
            &self.client,
            &url,
            &api_key,
            &body,
            self.request_timeout,
        )
        .await?;

        let status = resp.status();
        if status.as_u16() == 401 {
            // Attempt token refresh if we have a refresh token
            if let Some(ref rt) = self.refresh_token {
                let _refresh_guard = self.refresh_lock.lock().await;
                let current_token = self.api_key.read().await.clone();

                if current_token != api_key {
                    tracing::info!("Received 401, but another request already refreshed the token");
                    let retry_resp = Self::send_http_request(
                        &self.client,
                        &url,
                        &current_token,
                        &body,
                        self.request_timeout,
                    )
                    .await?;
                    let retry_status = retry_resp.status();
                    if !retry_status.is_success() {
                        let body_text = tokio::time::timeout(
                            Duration::from_secs(5),
                            retry_resp.text(),
                        )
                        .await
                        .unwrap_or(Ok(String::new()))
                        .unwrap_or_default();
                        return Err(LlmError::RequestFailed {
                            provider: "codex_chatgpt".to_string(),
                            reason: format!(
                                "HTTP {retry_status} from {url} (after concurrent token refresh): {body_text}"
                            ),
                        });
                    }
                    let sse_text = retry_resp.text().await.map_err(|e| LlmError::RequestFailed {
                        provider: "codex_chatgpt".to_string(),
                        reason: format!("Failed to read SSE response: {e}"),
                    })?;
                    return Self::parse_sse_response(&sse_text);
                }

                tracing::info!("Received 401, attempting token refresh");
                if let Some(new_token) =
                    codex_auth::refresh_access_token(rt, self.auth_path.as_deref()).await
                {
                    // Update stored api_key
                    *self.api_key.write().await = new_token.clone();
                    tracing::info!("Token refreshed, retrying request");

                    // Retry the request with the new token
                    let retry_resp = Self::send_http_request(
                        &self.client,
                        &url,
                        &new_token,
                        &body,
                        self.request_timeout,
                    )
                    .await?;

                    let retry_status = retry_resp.status();
                    if !retry_status.is_success() {
                        let body_text = tokio::time::timeout(
                            Duration::from_secs(5),
                            retry_resp.text(),
                        )
                        .await
                        .unwrap_or(Ok(String::new()))
                        .unwrap_or_default();
                        return Err(LlmError::RequestFailed {
                            provider: "codex_chatgpt".to_string(),
                            reason: format!("HTTP {retry_status} from {url} (after token refresh): {body_text}"),
                        });
                    }

                    let sse_text = retry_resp.text().await.map_err(|e| LlmError::RequestFailed {
                        provider: "codex_chatgpt".to_string(),
                        reason: format!("Failed to read SSE response: {e}"),
                    })?;
                    return Self::parse_sse_response(&sse_text);
                } else {
                    tracing::warn!(
                        "Token refresh failed. Please re-authenticate with: codex --login"
                    );
                }
            }

            // No refresh token or refresh failed — return the 401 error
            // Drain the response body to release the connection
            let _ = resp.text().await;
            return Err(LlmError::AuthFailed {
                provider: "codex_chatgpt".to_string(),
            });
        }

        if !status.is_success() {
            // Read the error body with a timeout to avoid hanging
            let body_text = tokio::time::timeout(
                Duration::from_secs(5),
                resp.text(),
            )
            .await
            .unwrap_or(Ok(String::new()))
            .unwrap_or_default();
            return Err(LlmError::RequestFailed {
                provider: "codex_chatgpt".to_string(),
                reason: format!(
                    "HTTP {status} from {url}: {body_text}",
                ),
            });
        }

        // Read the entire SSE stream
        let sse_text = resp.text().await.map_err(|e| LlmError::RequestFailed {
            provider: "codex_chatgpt".to_string(),
            reason: format!("Failed to read SSE response: {e}"),
        })?;

        Self::parse_sse_response(&sse_text)
    }

    /// Low-level HTTP POST to the /responses endpoint.
    async fn send_http_request(
        client: &Client,
        url: &str,
        api_key: &str,
        body: &Value,
        timeout: Duration,
    ) -> Result<reqwest::Response, LlmError> {
        client
            .post(url)
            .bearer_auth(api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .json(body)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: "codex_chatgpt".to_string(),
                reason: format!("HTTP request failed: {e}"),
            })
    }

    /// Parse SSE events from the response text.
    fn parse_sse_response(sse_text: &str) -> Result<ResponsesResult, LlmError> {
        let mut result = ResponsesResult::default();
        let mut current_event_type = String::new();

        for line in sse_text.lines() {
            if let Some(event) = line.strip_prefix("event: ") {
                current_event_type = event.trim().to_string();
                continue;
            }

            if let Some(data) = line.strip_prefix("data: ") {
                let data = data.trim();
                if data.is_empty() {
                    continue;
                }

                let parsed: Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                match current_event_type.as_str() {
                    "response.output_text.delta" => {
                        if let Some(delta) = parsed.get("delta").and_then(|d| d.as_str()) {
                            result.text.push_str(delta);
                        }
                    }
                    "response.output_item.added" => {
                        // Capture function call metadata when the item is first added.
                        // The item has: id (item_id), call_id, name, type.
                        let item = parsed.get("item").unwrap_or(&parsed);
                        if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                            let item_id = item
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let call_id = item
                                .get("call_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let name = item
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();

                            result
                                .pending_tool_calls
                                .entry(item_id)
                                .or_insert_with(|| PendingToolCall {
                                    call_id,
                                    name,
                                    arguments: String::new(),
                                });
                        }
                    }
                    "response.function_call_arguments.delta" => {
                        // Delta events use `item_id` (not `call_id`)
                        if let Some(item_id) =
                            parsed.get("item_id").and_then(|v| v.as_str())
                        {
                            if let Some(entry) = result.pending_tool_calls.get_mut(item_id) {
                                if let Some(delta) =
                                    parsed.get("delta").and_then(|d| d.as_str())
                                {
                                    entry.arguments.push_str(delta);
                                }
                            }
                        }
                    }
                    "response.completed" => {
                        if let Some(response) = parsed.get("response") {
                            if let Some(usage) = response.get("usage") {
                                result.input_tokens = usage
                                    .get("input_tokens")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0)
                                    as u32;
                                result.output_tokens = usage
                                    .get("output_tokens")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0)
                                    as u32;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(result)
    }

    /// Remove keys with empty-string values from a JSON object.
    ///
    /// gpt-5.2-codex fills optional tool parameters with `""` (e.g.
    /// `"timestamp": ""`). IronClaw's tool validation treats these as
    /// invalid "non-empty input expected". Stripping them makes the
    /// tool see only the actually-provided values.
    fn strip_empty_string_values(value: Value) -> Value {
        match value {
            Value::Object(map) => {
                let cleaned: serde_json::Map<String, Value> = map
                    .into_iter()
                    .filter(|(_, v)| !matches!(v, Value::String(s) if s.is_empty()))
                    .map(|(k, v)| (k, Self::strip_empty_string_values(v)))
                    .collect();
                Value::Object(cleaned)
            }
            other => other,
        }
    }
}

#[derive(Debug, Default)]
struct ResponsesResult {
    text: String,
    /// Keyed by item_id (the SSE item identifier, e.g. "fc_...").
    pending_tool_calls: std::collections::HashMap<String, PendingToolCall>,
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Debug)]
struct PendingToolCall {
    /// The call_id from the API (e.g. "call_..."), used to match results.
    call_id: String,
    name: String,
    arguments: String,
}

#[async_trait]
impl LlmProvider for CodexChatGptProvider {
    fn model_name(&self) -> &str {
        // Return resolved model if available, otherwise the configured name.
        self.resolved_model
            .get()
            .map(|s| s.as_str())
            .unwrap_or(&self.configured_model)
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        // ChatGPT backend doesn't expose per-token pricing
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let model = self.resolve_model().await;
        let body = self.build_request_body(model, &request.messages, &[], None);
        let result = self.send_request(body).await?;

        Ok(CompletionResponse {
            content: result.text,
            input_tokens: result.input_tokens,
            output_tokens: result.output_tokens,
            finish_reason: FinishReason::Stop,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let model = self.resolve_model().await;
        let body = self.build_request_body(
            model,
            &request.messages,
            &request.tools,
            request.tool_choice.as_deref(),
        );
        let result = self.send_request(body).await?;

        let tool_calls: Vec<ToolCall> = result
            .pending_tool_calls
            .into_values()
            .map(|tc| {
                let args: Value = serde_json::from_str(&tc.arguments)
                    .unwrap_or_else(|_| json!(tc.arguments));
                // gpt-5.2-codex fills optional parameters with empty strings (e.g.
                // `"timestamp": ""`), which IronClaw's tool validation rejects.
                // Strip them so only actually-provided values reach the tool.
                let args = Self::strip_empty_string_values(args);
                ToolCall {
                    id: tc.call_id,
                    name: tc.name,
                    arguments: args,
                }
            })
            .collect();

        let finish_reason = if tool_calls.is_empty() {
            FinishReason::Stop
        } else {
            FinishReason::ToolUse
        };

        Ok(ToolCompletionResponse {
            content: if result.text.is_empty() {
                None
            } else {
                Some(result.text)
            },
            tool_calls,
            input_tokens: result.input_tokens,
            output_tokens: result.output_tokens,
            finish_reason,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_conversion_user() {
        let items = CodexChatGptProvider::message_to_input_items(&ChatMessage::user("hello"));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "message");
        assert_eq!(items[0]["role"], "user");
        assert_eq!(items[0]["content"][0]["type"], "input_text");
        assert_eq!(items[0]["content"][0]["text"], "hello");
    }

    #[test]
    fn test_message_conversion_user_with_image() {
        use super::super::provider::ImageUrl;
        let parts = vec![
            ContentPart::Text {
                text: "What's in this image?".to_string(),
            },
            ContentPart::ImageUrl {
                image_url: ImageUrl {
                    url: "data:image/png;base64,iVBOR...".to_string(),
                    detail: None,
                },
            },
        ];
        let msg = ChatMessage::user_with_parts("", parts);
        let items = CodexChatGptProvider::message_to_input_items(&msg);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "message");
        assert_eq!(items[0]["role"], "user");
        let content = items[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "input_text");
        assert_eq!(content[0]["text"], "What's in this image?");
        assert_eq!(content[1]["type"], "input_image");
        assert_eq!(content[1]["image_url"], "data:image/png;base64,iVBOR...");
    }
    #[test]
    fn test_message_conversion_assistant() {
        let items = CodexChatGptProvider::message_to_input_items(&ChatMessage::assistant("hi"));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "message");
        assert_eq!(items[0]["role"], "assistant");
        assert_eq!(items[0]["content"][0]["type"], "output_text");
    }

    #[test]
    fn test_message_conversion_tool_result() {
        let msg = ChatMessage::tool_result("call_1", "search", "result text");
        let items = CodexChatGptProvider::message_to_input_items(&msg);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "function_call_output");
        assert_eq!(items[0]["call_id"], "call_1");
        assert_eq!(items[0]["output"], "result text");
    }

    #[test]
    fn test_message_conversion_assistant_with_tool_calls() {
        let tc = ToolCall {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: json!({"query": "rust"}),
        };
        let msg = ChatMessage::assistant_with_tool_calls(Some("thinking...".into()), vec![tc]);
        let items = CodexChatGptProvider::message_to_input_items(&msg);
        // Should produce: 1 text message + 1 function_call
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["type"], "message");
        assert_eq!(items[1]["type"], "function_call");
        assert_eq!(items[1]["name"], "search");
        assert_eq!(items[1]["call_id"], "call_1");
    }

    #[test]
    fn test_build_request_extracts_system_as_instructions() {
        let provider = CodexChatGptProvider::new("https://example.com", "key", "gpt-4o");
        let messages = vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("hello"),
        ];
        let body = provider.build_request_body("gpt-4o", &messages, &[], None);
        assert_eq!(body["instructions"], "You are helpful.");
        // input should only contain the user message, not the system message
        assert_eq!(body["input"].as_array().unwrap().len(), 1);
        // store must be false for ChatGPT backend
        assert_eq!(body["store"], false);
    }

    #[test]
    fn test_parse_sse_text_response() {
        let sse = r#"event: response.output_text.delta
data: {"delta":"Hello"}

event: response.output_text.delta
data: {"delta":" world!"}

event: response.completed
data: {"response":{"usage":{"input_tokens":10,"output_tokens":5}}}

"#;
        let result = CodexChatGptProvider::parse_sse_response(sse).unwrap();
        assert_eq!(result.text, "Hello world!");
        assert_eq!(result.input_tokens, 10);
        assert_eq!(result.output_tokens, 5);
        assert!(result.pending_tool_calls.is_empty());
    }

    #[test]
    fn test_parse_sse_tool_call() {
        // Real API format: output_item.added has item.id (item_id) + item.call_id,
        // delta events use item_id (not call_id)
        let sse = r#"event: response.output_item.added
data: {"item":{"id":"fc_1","type":"function_call","call_id":"call_1","name":"search"}}

event: response.function_call_arguments.delta
data: {"item_id":"fc_1","delta":"{\"query\":"}

event: response.function_call_arguments.delta
data: {"item_id":"fc_1","delta":"\"rust\"}"}

event: response.completed
data: {"response":{"usage":{"input_tokens":20,"output_tokens":15}}}

"#;
        let result = CodexChatGptProvider::parse_sse_response(sse).unwrap();
        assert!(result.text.is_empty());
        assert_eq!(result.pending_tool_calls.len(), 1);
        let tc = result.pending_tool_calls.get("fc_1").unwrap();
        assert_eq!(tc.call_id, "call_1");
        assert_eq!(tc.name, "search");
        assert_eq!(tc.arguments, "{\"query\":\"rust\"}");
    }

    #[test]
    fn test_strip_empty_string_values() {
        let input = json!({
            "format": "%Y-%m-%d",
            "operation": "now",
            "timestamp": "",
            "timestamp2": "",
        });
        let cleaned = CodexChatGptProvider::strip_empty_string_values(input);
        assert_eq!(cleaned, json!({"format": "%Y-%m-%d", "operation": "now"}));
    }
}
