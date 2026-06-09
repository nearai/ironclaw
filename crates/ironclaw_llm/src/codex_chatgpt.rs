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
use eventsource_stream::Eventsource;
use futures::{Stream, StreamExt};
use reqwest::Client;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use std::borrow::Cow;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};

use super::codex_auth;
use crate::error::LlmError;

use super::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, ContentPart, FinishReason, LlmProvider,
    Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse, ToolDefinition,
};

/// Sanitize a tool name to match the Responses API pattern `^[a-zA-Z0-9_-]+$`.
fn sanitize_tool_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Default Codex CLI client version reported to the `/models` endpoint when the
/// installed `codex` binary can't be queried and no override is set.
///
/// The Codex backend gates newer models behind the reported `client_version`,
/// so this must track a reasonably recent Codex CLI release. It is only the
/// last-resort fallback — the installed binary's real version is preferred.
const DEFAULT_CODEX_CLIENT_VERSION: &str = "0.137.0";

/// Parse the version token out of `codex --version` output.
///
/// Accepts shapes like `codex-cli 0.137.0` or `codex 0.140.1` and returns the
/// first dotted, all-numeric version (`0.137.0`). Any SemVer pre-release
/// (`-beta`, `-rc.1`) or build-metadata (`+build.7`) suffix is stripped first,
/// so pre-release / custom Codex builds resolve to their numeric release
/// version. Returns `None` when no version-like token is present.
fn parse_codex_cli_version(output: &str) -> Option<String> {
    output.split_whitespace().find_map(|token| {
        // Drop any `-<pre-release>` / `+<build-metadata>` suffix before parsing.
        let core = token.split(['-', '+']).next().unwrap_or(token);
        let segments: Vec<&str> = core.split('.').collect();
        let is_version = segments.len() >= 2
            && segments
                .iter()
                .all(|seg| !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_digit()));
        is_version.then(|| core.to_string())
    })
}

/// Max time to wait for `codex --version` before giving up and using the
/// default. The command is effectively instant; the bound only guards against a
/// wedged or slow binary stalling provider construction.
const CODEX_VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Query the installed `codex` binary for its version (e.g. `0.137.0`).
///
/// Returns `None` if the binary is absent, times out, exits non-zero, or its
/// output has no parseable version. Spawned via async `tokio::process` under a
/// timeout so it never blocks the runtime or stalls startup; runs at most once
/// per provider instance via the caller's `OnceCell`.
async fn detect_installed_codex_version() -> Option<String> {
    let output = tokio::time::timeout(
        CODEX_VERSION_PROBE_TIMEOUT,
        tokio::process::Command::new("codex")
            .arg("--version")
            .output(),
    )
    .await
    .ok()? // timed out
    .ok()?; // spawn / I/O error (e.g. binary not found)
    if !output.status.success() {
        return None;
    }
    parse_codex_cli_version(&String::from_utf8_lossy(&output.stdout))
}

/// Resolve the `client_version` to report to the Codex `/models` endpoint:
/// the version detected from the installed `codex` binary, or
/// [`DEFAULT_CODEX_CLIENT_VERSION`] when it is unavailable or blank. Split from
/// [`codex_client_version`] so the fallback logic is unit-testable without
/// spawning a process.
fn resolve_codex_client_version(detected: Option<&str>) -> String {
    detected
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_CODEX_CLIENT_VERSION)
        .to_string()
}

/// Determine the `client_version` to report, auto-detecting it from the
/// installed Codex CLI so newly released models (gated behind newer client
/// versions) are not silently hidden by a stale hardcoded constant.
async fn codex_client_version() -> String {
    resolve_codex_client_version(detect_installed_codex_version().await.as_deref())
}

fn convert_tool_definition(tool: &ToolDefinition) -> Value {
    use crate::tool_schema::{ToolSchemaPolicy, shape_tool_schema};

    let mut description = tool.description.clone();
    let parameters = shape_tool_schema(
        ToolSchemaPolicy::StrictOpenAi,
        &tool.parameters,
        &mut description,
    );

    json!({
        "type": "function",
        "name": sanitize_tool_name(&tool.name),
        "description": description,
        "parameters": parameters,
    })
}

fn build_sanitized_tool_name_map(
    tools: &[ToolDefinition],
) -> Result<std::collections::HashMap<String, String>, LlmError> {
    let mut name_map = std::collections::HashMap::new();
    for tool in tools {
        let sanitized = sanitize_tool_name(&tool.name);
        if let Some(existing) = name_map.insert(sanitized.clone(), tool.name.clone())
            && existing != tool.name
        {
            return Err(LlmError::InvalidResponse {
                provider: "codex_chatgpt".to_string(),
                reason: format!(
                    "tool names `{existing}` and `{}` both map to provider name `{sanitized}`",
                    tool.name
                ),
            });
        }
    }
    Ok(name_map)
}

/// Provider that speaks the Responses API protocol against the ChatGPT backend.
pub(crate) struct CodexChatGptProvider {
    client: Client,
    base_url: String,
    api_key: RwLock<SecretString>,
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
    #[cfg(test)]
    fn new(base_url: &str, api_key: &str, model: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: RwLock::new(SecretString::from(api_key.to_string())),
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
    pub(crate) fn with_lazy_model(
        base_url: &str,
        api_key: SecretString,
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
            api_key: RwLock::new(api_key),
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
                let available = Self::fetch_available_models(&self.client, &self.base_url, &api_key)
                    .await;

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

    /// Query `/models?client_version=<ver>` and return the list of available
    /// model slugs, ordered by priority (highest first).
    ///
    /// The Codex backend gates newer models (e.g. `gpt-5.5`) behind the
    /// reported `client_version`, so a stale value silently hides models the
    /// account is entitled to. The version is auto-detected from the installed
    /// `codex` binary (see [`codex_client_version`]).
    async fn fetch_available_models(
        client: &Client,
        base_url: &str,
        api_key: &SecretString,
    ) -> Vec<String> {
        let client_version = codex_client_version().await;
        let url = format!("{base_url}/models?client_version={client_version}");
        let resp = match client
            .get(&url)
            .bearer_auth(api_key.expose_secret())
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
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
                    .filter_map(|m| {
                        m.get("slug")
                            .and_then(|s| s.as_str())
                            .map(|s| s.to_string())
                    })
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
            .flat_map(Self::message_to_input_items)
            .collect();

        // Convert tool definitions. Responses API function names allow only
        // `[a-zA-Z0-9_-]`, while host capability ids can contain dots
        // (`builtin.echo`, MCP ids, etc.). Keep provider-facing names sanitized
        // and map them back to original names after the response is parsed.
        let api_tools: Vec<Value> = tools.iter().map(convert_tool_definition).collect();

        let mut body = json!({
            "model": model,
            "instructions": instructions,
            "input": input,
            "stream": true,
            "store": false,
        });

        // Only add `reasoning` for models that support it;
        // the Responses API hard-rejects it on non-reasoning models.
        if crate::reasoning_models::supports_openai_reasoning(model) {
            body["reasoning"] = crate::responses_reasoning::summary_request();
        }

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
                            "name": sanitize_tool_name(&tc.name),
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
        let resp =
            Self::send_http_request(&self.client, &url, &api_key, &body, self.request_timeout)
                .await?;

        let status = resp.status();
        if status.as_u16() == 401 {
            // Attempt token refresh if we have a refresh token
            if let Some(ref rt) = self.refresh_token {
                let _refresh_guard = self.refresh_lock.lock().await;
                let current_token = self.api_key.read().await.clone();

                if current_token.expose_secret() != api_key.expose_secret() {
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
                        let body_text =
                            tokio::time::timeout(Duration::from_secs(5), retry_resp.text())
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
                    return Self::parse_sse_response_stream(retry_resp, self.request_timeout).await;
                }

                tracing::info!("Received 401, attempting token refresh");
                if let Some(new_token) =
                    codex_auth::refresh_access_token(&self.client, rt, self.auth_path.as_deref())
                        .await
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
                        let body_text =
                            tokio::time::timeout(Duration::from_secs(5), retry_resp.text())
                                .await
                                .unwrap_or(Ok(String::new()))
                                .unwrap_or_default();
                        return Err(LlmError::RequestFailed {
                            provider: "codex_chatgpt".to_string(),
                            reason: format!(
                                "HTTP {retry_status} from {url} (after token refresh): {body_text}"
                            ),
                        });
                    }

                    return Self::parse_sse_response_stream(retry_resp, self.request_timeout).await;
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
            let body_text = tokio::time::timeout(Duration::from_secs(5), resp.text())
                .await
                .unwrap_or(Ok(String::new()))
                .unwrap_or_default();
            return Err(LlmError::RequestFailed {
                provider: "codex_chatgpt".to_string(),
                reason: format!("HTTP {status} from {url}: {body_text}",),
            });
        }

        Self::parse_sse_response_stream(resp, self.request_timeout).await
    }

    /// Low-level HTTP POST to the /responses endpoint.
    async fn send_http_request(
        client: &Client,
        url: &str,
        api_key: &SecretString,
        body: &Value,
        timeout: Duration,
    ) -> Result<reqwest::Response, LlmError> {
        client
            .post(url)
            .bearer_auth(api_key.expose_secret())
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

    async fn parse_sse_response_stream(
        resp: reqwest::Response,
        idle_timeout: Duration,
    ) -> Result<ResponsesResult, LlmError> {
        let stream = resp
            .bytes_stream()
            .map(|chunk| chunk.map_err(|e| e.to_string()));
        Self::parse_sse_stream(stream, idle_timeout).await
    }

    async fn parse_sse_stream<S>(
        stream: S,
        idle_timeout: Duration,
    ) -> Result<ResponsesResult, LlmError>
    where
        S: Stream<Item = Result<bytes::Bytes, String>> + Unpin,
    {
        let mut result = ResponsesResult::default();
        let mut stream = stream.eventsource();

        loop {
            match tokio::time::timeout(idle_timeout, stream.next()).await {
                Ok(Some(Ok(event))) => {
                    let data = event.data.trim();
                    if data.is_empty() {
                        continue;
                    }
                    if data == "[DONE]" {
                        return Ok(result);
                    }

                    let parsed: Value = match serde_json::from_str(data) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    let event_type = Self::resolve_sse_event_type(event.event.as_str(), &parsed);
                    if Self::handle_sse_event(&mut result, event_type.as_ref(), &parsed) {
                        return Ok(result);
                    }
                }
                Ok(Some(Err(e))) => {
                    return Err(LlmError::RequestFailed {
                        provider: "codex_chatgpt".to_string(),
                        reason: format!("Failed to read SSE stream: {e}"),
                    });
                }
                Ok(None) => return Ok(result),
                Err(_) => {
                    return Err(LlmError::RequestFailed {
                        provider: "codex_chatgpt".to_string(),
                        reason: format!(
                            "Timed out waiting for SSE event after {}s",
                            idle_timeout.as_secs()
                        ),
                    });
                }
            }
        }
    }

    /// Parse SSE events from the response text.
    #[cfg(test)]
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
                if data == "[DONE]" {
                    return Ok(result);
                }

                let parsed: Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let event_type = Self::resolve_sse_event_type(current_event_type.as_str(), &parsed);
                if Self::handle_sse_event(&mut result, event_type.as_ref(), &parsed) {
                    return Ok(result);
                }
            }
        }

        Ok(result)
    }

    fn resolve_sse_event_type<'a>(event_type: &'a str, parsed: &'a Value) -> Cow<'a, str> {
        if !event_type.is_empty() && event_type != "message" {
            return Cow::Borrowed(event_type);
        }
        parsed
            .get("type")
            .and_then(|value| value.as_str())
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Borrowed(event_type))
    }

    fn handle_sse_event(result: &mut ResponsesResult, event_type: &str, parsed: &Value) -> bool {
        match event_type {
            "response.output_text.delta" => {
                if let Some(delta) = parsed.get("delta").and_then(|d| d.as_str()) {
                    result.text.push_str(delta);
                }
            }
            "response.output_text.done" => {
                if result.text.is_empty()
                    && let Some(text) = parsed.get("text").and_then(|d| d.as_str())
                {
                    result.text.push_str(text);
                }
            }
            event_type
                if crate::responses_reasoning::apply_summary_event(
                    &mut result.reasoning,
                    event_type,
                    parsed,
                ) => {}
            "response.output_item.added" => {
                // Capture function call metadata when the item is first added.
                // The item has: id (item_id), call_id, name, type.
                let item = parsed.get("item").unwrap_or(parsed);
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
                if let Some(item_id) = parsed.get("item_id").and_then(|v| v.as_str())
                    && let Some(entry) = result.pending_tool_calls.get_mut(item_id)
                    && let Some(delta) = parsed.get("delta").and_then(|d| d.as_str())
                {
                    entry.arguments.push_str(delta);
                }
            }
            "response.function_call_arguments.done" => {
                if let Some(item_id) = parsed.get("item_id").and_then(|v| v.as_str())
                    && let Some(entry) = result.pending_tool_calls.get_mut(item_id)
                    && let Some(arguments) = parsed.get("arguments").and_then(|d| d.as_str())
                {
                    entry.arguments = arguments.to_string();
                }
            }
            "response.output_item.done" => {
                if let Some(item) = parsed.get("item").or_else(|| parsed.get("output")) {
                    Self::merge_completed_output_item(result, item, result.text.is_empty());
                }
            }
            "response.completed" => {
                if let Some(response) = parsed.get("response")
                    && let Some(usage) = response.get("usage")
                {
                    result.input_tokens = usage
                        .get("input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                    result.output_tokens = usage
                        .get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                }
                if let Some(response) = parsed.get("response") {
                    Self::merge_completed_response_output(result, response);
                }
                tracing::debug!(
                    content_bytes = result.text.len(),
                    tool_call_count = result.pending_tool_calls.len(),
                    input_tokens = result.input_tokens,
                    output_tokens = result.output_tokens,
                    "Codex ChatGPT: parsed completed response"
                );
                return true;
            }
            _ => {}
        }

        false
    }

    fn merge_completed_response_output(result: &mut ResponsesResult, response: &Value) {
        if result.text.is_empty()
            && let Some(output_text) = response.get("output_text").and_then(|value| value.as_str())
        {
            result.text.push_str(output_text);
        }

        let allow_text_fallback = result.text.is_empty();
        if let Some(output) = response.get("output").and_then(|value| value.as_array()) {
            for item in output {
                Self::merge_completed_output_item(result, item, allow_text_fallback);
            }
        }
    }

    fn merge_completed_output_item(
        result: &mut ResponsesResult,
        item: &Value,
        allow_text_fallback: bool,
    ) {
        match item.get("type").and_then(|value| value.as_str()) {
            Some("message") if allow_text_fallback => {
                Self::append_output_message_text(&mut result.text, item);
            }
            Some("function_call") => {
                let item_id = item
                    .get("id")
                    .and_then(|value| value.as_str())
                    .or_else(|| item.get("call_id").and_then(|value| value.as_str()))
                    .unwrap_or("")
                    .to_string();
                if item_id.is_empty() {
                    return;
                }
                let call_id = item
                    .get("call_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or(&item_id)
                    .to_string();
                let name = item
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_string();
                let arguments = item
                    .get("arguments")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_string();
                result
                    .pending_tool_calls
                    .entry(item_id)
                    .and_modify(|existing| {
                        if !call_id.is_empty() {
                            existing.call_id = call_id.clone();
                        }
                        if !name.is_empty() {
                            existing.name = name.clone();
                        }
                        if !arguments.is_empty() {
                            existing.arguments = arguments.clone();
                        }
                    })
                    .or_insert_with(|| PendingToolCall {
                        call_id,
                        name,
                        arguments,
                    });
            }
            _ => {}
        }
    }

    fn append_output_message_text(output: &mut String, item: &Value) {
        let Some(content) = item.get("content").and_then(|value| value.as_array()) else {
            return;
        };
        for part in content {
            if part.get("type").and_then(|value| value.as_str()) == Some("output_text")
                && let Some(text) = part.get("text").and_then(|value| value.as_str())
            {
                output.push_str(text);
            }
        }
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
    reasoning: String,
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
        let mut messages = request.messages;
        crate::provider::sanitize_tool_messages(&mut messages);
        let body = self.build_request_body(model, &messages, &[], None);
        let result = self.send_request(body).await?;

        Ok(CompletionResponse {
            content: result.text,
            input_tokens: result.input_tokens,
            output_tokens: result.output_tokens,
            finish_reason: FinishReason::Stop,
            reasoning: crate::responses_reasoning::finish_summary(result.reasoning),
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let mut messages = request.messages;
        crate::provider::sanitize_tool_messages(&mut messages);
        let name_map = build_sanitized_tool_name_map(&request.tools)?;
        let model = self.resolve_model().await;
        let body = self.build_request_body(
            model,
            &messages,
            &request.tools,
            request.tool_choice.as_deref(),
        );
        let result = self.send_request(body).await?;

        let tool_calls: Vec<ToolCall> = result
            .pending_tool_calls
            .into_values()
            .map(|tc| {
                let returned_name = tc.name;
                let name = name_map
                    .get(&returned_name)
                    .cloned()
                    .unwrap_or(returned_name);
                let args: Value =
                    serde_json::from_str(&tc.arguments).unwrap_or_else(|_| json!(tc.arguments));
                // gpt-5.2-codex fills optional parameters with empty strings (e.g.
                // `"timestamp": ""`), which IronClaw's tool validation rejects.
                // Strip them so only actually-provided values reach the tool.
                let args = Self::strip_empty_string_values(args);
                ToolCall {
                    id: tc.call_id,
                    name,
                    arguments: args,
                    reasoning: None,
                    signature: None,
                    arguments_parse_error: None,
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
            reasoning: crate::responses_reasoning::finish_summary(result.reasoning),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures::stream;

    #[test]
    fn parse_codex_cli_version_standard_output() {
        assert_eq!(
            parse_codex_cli_version("codex-cli 0.137.0"),
            Some("0.137.0".to_string())
        );
    }

    #[test]
    fn parse_codex_cli_version_tolerates_alt_shapes() {
        assert_eq!(
            parse_codex_cli_version("codex 0.140.1\n"),
            Some("0.140.1".to_string())
        );
        assert_eq!(
            parse_codex_cli_version("  codex-cli   1.2.3  "),
            Some("1.2.3".to_string())
        );
    }

    #[test]
    fn parse_codex_cli_version_none_when_absent() {
        assert_eq!(parse_codex_cli_version("codex-cli unknown"), None);
        assert_eq!(parse_codex_cli_version(""), None);
        // A bare integer is not a dotted version.
        assert_eq!(parse_codex_cli_version("codex 5"), None);
    }

    #[test]
    fn parse_codex_cli_version_strips_prerelease_and_build_metadata() {
        // SemVer pre-release and build-metadata suffixes resolve to the core.
        assert_eq!(
            parse_codex_cli_version("codex-cli 0.138.0-beta"),
            Some("0.138.0".to_string())
        );
        assert_eq!(
            parse_codex_cli_version("codex-cli 0.138.0+build.7"),
            Some("0.138.0".to_string())
        );
        assert_eq!(
            parse_codex_cli_version("codex 0.139.0-rc.1"),
            Some("0.139.0".to_string())
        );
        assert_eq!(
            parse_codex_cli_version("codex-cli 0.140.0-beta+exp.sha.5114f85"),
            Some("0.140.0".to_string())
        );
    }

    #[test]
    fn resolve_client_version_uses_detected() {
        assert_eq!(resolve_codex_client_version(Some("0.140.1")), "0.140.1");
    }

    #[test]
    fn resolve_client_version_falls_back_to_default() {
        assert_eq!(
            resolve_codex_client_version(None),
            DEFAULT_CODEX_CLIENT_VERSION
        );
    }

    #[test]
    fn resolve_client_version_ignores_blank_detected() {
        assert_eq!(
            resolve_codex_client_version(Some("   ")),
            DEFAULT_CODEX_CLIENT_VERSION
        );
    }

    /// Caller-level test: drives `fetch_available_models` against a loopback
    /// server and asserts both that it parses the returned slugs AND that the
    /// resolved `client_version` actually reaches the request URL (the side
    /// effect the version value gates). Covers the wrapper the pure
    /// `resolve_codex_client_version` unit tests can't reach.
    #[tokio::test]
    async fn fetch_available_models_sends_resolved_client_version() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept request");
            let mut buf = [0u8; 4096];
            let n = socket.read(&mut buf).await.expect("read request");
            let request_line = String::from_utf8_lossy(&buf[..n])
                .lines()
                .next()
                .unwrap_or_default()
                .to_string();
            let body = r#"{"models":[{"slug":"gpt-5.5"},{"slug":"gpt-5.4"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
            let _ = tx.send(request_line);
        });

        let base_url = format!("http://{addr}");
        let models = CodexChatGptProvider::fetch_available_models(
            &reqwest::Client::new(),
            &base_url,
            &SecretString::from("test-token".to_string()),
        )
        .await;

        // Output: slugs are parsed from the /models response, in order.
        assert_eq!(models, vec!["gpt-5.5".to_string(), "gpt-5.4".to_string()]);

        // Side-effect input: the resolved client_version reached the request URL.
        let request_line = rx.await.expect("server captured the request");
        let expected = codex_client_version().await;
        assert!(
            request_line.contains(&format!("client_version={expected}")),
            "expected client_version={expected} in request line; got: {request_line}"
        );
    }

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
            reasoning: None,
            signature: None,
            arguments_parse_error: None,
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
    fn test_message_conversion_sanitizes_tool_call_name() {
        let tc = ToolCall {
            id: "call_1".to_string(),
            name: "builtin.echo".to_string(),
            arguments: json!({"input": "hello"}),
            reasoning: None,
            signature: None,
            arguments_parse_error: None,
        };
        let msg = ChatMessage::assistant_with_tool_calls(None, vec![tc]);
        let items = CodexChatGptProvider::message_to_input_items(&msg);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "function_call");
        assert_eq!(items[0]["name"], "builtin_echo");
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
    fn test_build_request_sanitizes_tool_definition_names() {
        let provider = CodexChatGptProvider::new("https://example.com", "key", "gpt-4o");
        let messages = vec![ChatMessage::user("hello")];
        let tools = vec![ToolDefinition {
            name: "builtin.echo".to_string(),
            description: "Echo input".to_string(),
            parameters: json!({"type": "object"}),
        }];
        let body = provider.build_request_body("gpt-4o", &messages, &tools, None);
        assert_eq!(body["tools"][0]["name"], "builtin_echo");
    }

    #[test]
    fn test_build_request_flattens_ref_tool_schema() {
        let provider = CodexChatGptProvider::new("https://example.com", "key", "gpt-4o");
        let messages = vec![ChatMessage::user("hello")];
        let tools = vec![ToolDefinition {
            name: "builtin__apply_patch".to_string(),
            description: "Apply a patch".to_string(),
            parameters: json!({"$ref": "schemas/builtin/apply-patch.input.v1.json"}),
        }];
        let body = provider.build_request_body("gpt-4o", &messages, &tools, None);
        assert_eq!(body["tools"][0]["parameters"]["type"], "object");
        assert!(body["tools"][0]["parameters"].get("properties").is_some());
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
    fn test_parse_sse_data_only_type_field_text_response() {
        let sse = r#"data: {"type":"response.output_text.delta","delta":"Hello"}

data: {"type":"response.output_text.delta","delta":" world!"}

data: {"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":5}}}

"#;
        let result = CodexChatGptProvider::parse_sse_response(sse).unwrap();
        assert_eq!(result.text, "Hello world!");
        assert_eq!(result.input_tokens, 10);
        assert_eq!(result.output_tokens, 5);
        assert!(result.pending_tool_calls.is_empty());
    }

    #[test]
    fn test_parse_sse_output_text_done_without_delta() {
        let sse = r#"data: {"type":"response.output_text.done","text":"Hello from done."}

data: {"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":5}}}

"#;
        let result = CodexChatGptProvider::parse_sse_response(sse).unwrap();
        assert_eq!(result.text, "Hello from done.");
        assert_eq!(result.input_tokens, 10);
        assert_eq!(result.output_tokens, 5);
        assert!(result.pending_tool_calls.is_empty());
    }

    #[test]
    fn test_parse_sse_completed_response_output_text_without_deltas() {
        let sse = r#"data: {"type":"response.completed","response":{"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Hello from final output."}]}],"usage":{"input_tokens":10,"output_tokens":5}}}

"#;
        let result = CodexChatGptProvider::parse_sse_response(sse).unwrap();
        assert_eq!(result.text, "Hello from final output.");
        assert_eq!(result.input_tokens, 10);
        assert_eq!(result.output_tokens, 5);
        assert!(result.pending_tool_calls.is_empty());
    }

    #[test]
    fn test_parse_sse_completed_response_prefers_output_text_over_message_fallback() {
        let sse = r#"data: {"type":"response.completed","response":{"output_text":"Hello from output_text.","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Duplicate fallback text."}]}],"usage":{"input_tokens":10,"output_tokens":5}}}

"#;
        let result = CodexChatGptProvider::parse_sse_response(sse).unwrap();
        assert_eq!(result.text, "Hello from output_text.");
        assert_eq!(result.input_tokens, 10);
        assert_eq!(result.output_tokens, 5);
        assert!(result.pending_tool_calls.is_empty());
    }

    #[test]
    fn test_parse_sse_reasoning_summary_response() {
        let sse = r#"event: response.reasoning_summary_text.delta
data: {"delta":"Thinking Steps\n"}

event: response.reasoning_summary_text.delta
data: {"delta":"[] Inspect context."}

event: response.output_text.delta
data: {"delta":"Done."}

event: response.completed
data: {"response":{"usage":{"input_tokens":10,"output_tokens":5}}}

"#;
        let result = CodexChatGptProvider::parse_sse_response(sse).unwrap();
        assert_eq!(result.text, "Done.");
        assert_eq!(
            crate::responses_reasoning::finish_summary(result.reasoning).as_deref(),
            Some("Thinking Steps\n[] Inspect context.")
        );
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
    fn test_parse_sse_tool_call_done_events() {
        let sse = r#"event: response.output_item.added
data: {"item":{"id":"fc_1","type":"function_call","call_id":"call_1","name":"search"}}

event: response.function_call_arguments.done
data: {"item_id":"fc_1","arguments":"{\"query\":\"rust\"}"}

event: response.output_item.done
data: {"item":{"id":"fc_1","type":"function_call","call_id":"call_1","name":"search"}}

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
    fn test_parse_sse_completed_response_output_tool_call_without_deltas() {
        let sse = r#"data: {"type":"response.completed","response":{"output":[{"type":"function_call","id":"fc_1","call_id":"call_1","name":"search","arguments":"{\"query\":\"rust\"}"}],"usage":{"input_tokens":20,"output_tokens":15}}}

"#;
        let result = CodexChatGptProvider::parse_sse_response(sse).unwrap();
        assert!(result.text.is_empty());
        assert_eq!(result.pending_tool_calls.len(), 1);
        let tc = result.pending_tool_calls.get("fc_1").unwrap();
        assert_eq!(tc.call_id, "call_1");
        assert_eq!(tc.name, "search");
        assert_eq!(tc.arguments, "{\"query\":\"rust\"}");
    }

    #[tokio::test]
    async fn test_parse_sse_stream_response() {
        let stream = stream::iter(vec![
            Ok(Bytes::from_static(
                b"event: response.output_text.delta\ndata: {\"delta\":\"Hello\"}\n\n",
            )),
            Ok(Bytes::from_static(
                b"event: response.output_text.delta\ndata: {\"delta\":\" world\"}\n\n",
            )),
            Ok(Bytes::from_static(
                b"event: response.completed\ndata: {\"response\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}}\n\n",
            )),
        ]);

        let result = CodexChatGptProvider::parse_sse_stream(stream, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(result.text, "Hello world");
        assert_eq!(result.input_tokens, 3);
        assert_eq!(result.output_tokens, 2);
    }

    #[tokio::test]
    async fn test_parse_sse_stream_data_only_type_field_response() {
        let stream = stream::iter(vec![
            Ok(Bytes::from_static(
                b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
            )),
            Ok(Bytes::from_static(
                b"data: {\"type\":\"response.output_text.delta\",\"delta\":\" world\"}\n\n",
            )),
            Ok(Bytes::from_static(
                b"data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}}\n\n",
            )),
        ]);

        let result = CodexChatGptProvider::parse_sse_stream(stream, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(result.text, "Hello world");
        assert_eq!(result.input_tokens, 3);
        assert_eq!(result.output_tokens, 2);
    }

    #[tokio::test]
    async fn test_parse_sse_done_marker_stops_parsing() {
        let sse = r#"data: {"type":"response.output_text.delta","delta":"hello"}

data: [DONE]

data: {"type":"response.output_text.delta","delta":" ignored"}

"#;
        let result = CodexChatGptProvider::parse_sse_response(sse).unwrap();
        assert_eq!(result.text, "hello");

        let stream = stream::iter(vec![
            Ok(Bytes::from_static(
                b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\n",
            )),
            Ok(Bytes::from_static(b"data: [DONE]\n\n")),
            Ok(Bytes::from_static(
                b"data: {\"type\":\"response.output_text.delta\",\"delta\":\" ignored\"}\n\n",
            )),
        ]);
        let result = CodexChatGptProvider::parse_sse_stream(stream, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(result.text, "hello");
    }

    #[tokio::test]
    async fn complete_with_tools_remaps_sanitized_response_tool_names() {
        let base_url = responses_api_test_server::spawn(
            r#"event: response.output_item.added
data: {"item":{"id":"fc_1","type":"function_call","call_id":"call_1","name":"builtin_echo"}}

event: response.function_call_arguments.delta
data: {"item_id":"fc_1","delta":"{\"message\":\"hello\"}"}

event: response.completed
data: {"response":{"usage":{"input_tokens":3,"output_tokens":2}}}

"#,
        )
        .await;
        let provider = CodexChatGptProvider::new(&base_url, "test-key", "gpt-4o");
        let request = ToolCompletionRequest::new(
            vec![ChatMessage::user("use echo")],
            vec![ToolDefinition {
                name: "builtin.echo".to_string(),
                description: "Echo input".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "message": {"type": "string"}
                    }
                }),
            }],
        );

        let response = provider
            .complete_with_tools(request)
            .await
            .expect("tool completion succeeds");

        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "builtin.echo");
        assert_eq!(
            response.tool_calls[0].arguments,
            json!({"message": "hello"})
        );
    }

    #[tokio::test]
    async fn complete_with_tools_accepts_data_only_text_response() {
        let base_url = responses_api_test_server::spawn(
            r#"data: {"type":"response.output_text.delta","delta":"Hello from data-only SSE."}

data: {"type":"response.completed","response":{"usage":{"input_tokens":3,"output_tokens":2}}}

"#,
        )
        .await;
        let provider = CodexChatGptProvider::new(&base_url, "test-key", "gpt-4o");
        let request = ToolCompletionRequest::new(
            vec![ChatMessage::user("hello")],
            vec![ToolDefinition {
                name: "builtin.echo".to_string(),
                description: "Echo input".to_string(),
                parameters: json!({"type": "object"}),
            }],
        );

        let response = provider
            .complete_with_tools(request)
            .await
            .expect("tool-capable completion succeeds");

        assert_eq!(
            response.content.as_deref(),
            Some("Hello from data-only SSE.")
        );
        assert!(response.tool_calls.is_empty());
        assert_eq!(response.finish_reason, FinishReason::Stop);
    }

    #[tokio::test]
    async fn complete_with_tools_rejects_colliding_sanitized_tool_names_before_request() {
        let provider = CodexChatGptProvider::new("http://127.0.0.1:9", "test-key", "gpt-4o");
        let request = ToolCompletionRequest::new(
            vec![ChatMessage::user("use a tool")],
            vec![
                ToolDefinition {
                    name: "foo.bar".to_string(),
                    description: "First tool".to_string(),
                    parameters: json!({"type": "object"}),
                },
                ToolDefinition {
                    name: "foo_bar".to_string(),
                    description: "Second tool".to_string(),
                    parameters: json!({"type": "object"}),
                },
            ],
        );

        let error = provider
            .complete_with_tools(request)
            .await
            .expect_err("colliding provider tool names must fail closed");

        assert!(
            error
                .to_string()
                .contains("both map to provider name `foo_bar`"),
            "unexpected error: {error}"
        );
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

    mod responses_api_test_server {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        /// Tiny in-process Responses API fixture for caller-level provider tests.
        /// The test must exercise model resolution plus `/responses`, so a real
        /// loopback HTTP boundary is intentional here.
        pub(super) async fn spawn(sse_body: &'static str) -> String {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind test server");
            let addr = listener.local_addr().expect("local addr");
            tokio::spawn(async move {
                for _ in 0..2 {
                    let (mut socket, _) = listener.accept().await.expect("accept request");
                    let mut request = [0u8; 4096];
                    let bytes_read = socket.read(&mut request).await.expect("read request");
                    let request = String::from_utf8_lossy(&request[..bytes_read]);
                    if request.starts_with("GET /models") {
                        write_response(
                            &mut socket,
                            "application/json",
                            r#"{"models":[{"slug":"gpt-4o"}]}"#,
                        )
                        .await;
                    } else if request.starts_with("POST /responses") {
                        write_response(&mut socket, "text/event-stream", sse_body).await;
                    } else {
                        write_response(&mut socket, "text/plain", "not found").await;
                    }
                }
            });
            format!("http://{addr}")
        }

        async fn write_response(
            socket: &mut tokio::net::TcpStream,
            content_type: &str,
            body: &str,
        ) {
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
        }
    }
}
