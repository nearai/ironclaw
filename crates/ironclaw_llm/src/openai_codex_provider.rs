//! OpenAI Codex Responses API client.
//!
//! Implements `LlmProvider` using the Responses API at
//! `chatgpt.com/backend-api/codex/responses` -- the endpoint that works
//! with ChatGPT subscription OAuth tokens.
//!
//! This mirrors OpenClaw's Responses API flow translated to Rust.

use std::collections::HashMap;

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::error::LlmError;
use crate::openai_responses_session::ResponsesSessionRegistry;
use crate::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, ContentPart, FinishReason, LlmProvider,
    ModelMetadata, Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse, ToolDefinition,
};

/// OpenAI Codex Responses API provider.
///
/// Sends requests to `{api_base_url}/responses` using SSE streaming,
/// with JWT-based auth headers matching OpenClaw's approach.
/// Token + account ID pair, updated atomically.
struct AuthState {
    token: String,
    account_id: String,
}

pub struct OpenAiCodexProvider {
    client: Client,
    model: String,
    api_base_url: String,
    auth_epoch: RwLock<()>,
    auth: RwLock<AuthState>,
    /// Present only when the transport can reliably retain Responses state.
    ///
    /// The current production HTTP lane uses `store: false`, so constructors
    /// leave this disabled. Keeping the planner behind this provider-private
    /// capability gate prevents `previous_response_id` from becoming durable
    /// conversational truth or being used without a retention guarantee.
    responses_sessions: Option<ResponsesSessionRegistry>,
}

impl OpenAiCodexProvider {
    /// Create a new provider.
    ///
    /// Extracts the `chatgpt_account_id` from the JWT token.
    /// `request_timeout_secs` controls the HTTP client timeout (falls back to 300s).
    pub fn new(
        model: &str,
        api_base_url: &str,
        token: &str,
        request_timeout_secs: u64,
    ) -> Result<Self, LlmError> {
        let account_id = extract_account_id(token)?;
        Ok(Self {
            client: crate::config::hardened_client_builder(request_timeout_secs)
                .build()
                .map_err(|e| LlmError::RequestFailed {
                    provider: "openai_codex".to_string(),
                    reason: format!("Failed to create HTTP client: {e}"),
                })?,
            model: model.to_string(),
            api_base_url: api_base_url.trim_end_matches('/').to_string(),
            auth_epoch: RwLock::new(()),
            auth: RwLock::new(AuthState {
                token: token.to_string(),
                account_id,
            }),
            responses_sessions: None,
        })
    }

    /// Update the access token after a refresh.
    pub async fn update_token(&self, token: &str) -> Result<(), LlmError> {
        let account_id = extract_account_id(token)?;
        let _auth_epoch = self.auth_epoch.write().await;
        let mut auth = self.auth.write().await;
        let account_changed = auth.account_id != account_id;
        if account_changed && let Some(registry) = &self.responses_sessions {
            registry.clear().await;
        }
        auth.token = token.to_string();
        auth.account_id = account_id;
        tracing::debug!("Updated Codex provider token");
        Ok(())
    }

    /// Build request headers matching OpenClaw's `buildHeaders`.
    async fn build_headers(&self) -> Result<reqwest::header::HeaderMap, LlmError> {
        use reqwest::header::{
            ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, USER_AGENT,
        };

        let auth = self.auth.read().await;

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", auth.token)).map_err(|e| {
                LlmError::RequestFailed {
                    provider: "openai_codex".to_string(),
                    reason: format!("Invalid token for header: {e}"),
                }
            })?,
        );
        headers.insert(
            HeaderName::from_static("chatgpt-account-id"),
            HeaderValue::from_str(&auth.account_id).map_err(|e| LlmError::RequestFailed {
                provider: "openai_codex".to_string(),
                reason: format!("Invalid account ID for header: {e}"),
            })?,
        );
        headers.insert(
            HeaderName::from_static("openai-beta"),
            HeaderValue::from_static("responses=experimental"),
        );
        headers.insert(
            HeaderName::from_static("originator"),
            HeaderValue::from_static("ironclaw"),
        );
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(concat!("ironclaw/", env!("CARGO_PKG_VERSION"))),
        );
        headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        Ok(headers)
    }

    /// Build the request body for the Responses API.
    #[cfg(test)]
    fn build_request_body(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolDefinition]>,
    ) -> serde_json::Value {
        self.build_request_body_with_input(messages, tools, normalized_input(messages), None)
    }

    fn build_request_body_with_input(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolDefinition]>,
        input: Vec<serde_json::Value>,
        previous_response_id: Option<&str>,
    ) -> serde_json::Value {
        // Separate system messages into `instructions`
        let instructions: String = messages
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");

        let mut body = serde_json::json!({
            "model": self.model,
            "store": false,
            "stream": true,
            "input": input,
            "text": { "verbosity": "medium" },
        });

        if crate::reasoning_models::supports_openai_reasoning(&self.model) {
            body["reasoning"] = crate::responses_reasoning::summary_request();
            body["include"] = serde_json::json!(["reasoning.encrypted_content"]);
        }

        if !instructions.is_empty() {
            body["instructions"] = serde_json::Value::String(instructions);
        }

        if let Some(previous_response_id) = previous_response_id {
            body["previous_response_id"] =
                serde_json::Value::String(previous_response_id.to_string());
        }

        if let Some(tools) = tools
            && !tools.is_empty()
        {
            let tools_json: Vec<serde_json::Value> =
                tools.iter().map(convert_tool_definition).collect();
            body["tools"] = serde_json::Value::Array(tools_json);
            body["tool_choice"] = serde_json::Value::String("auto".to_string());
            body["parallel_tool_calls"] = serde_json::Value::Bool(true);
        }

        body
    }

    async fn send_completion_request(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolDefinition]>,
        metadata: &HashMap<String, String>,
    ) -> Result<ParsedResponse, LlmError> {
        let full_input = normalized_input(messages);
        let Some(registry) = &self.responses_sessions else {
            let body = self.build_request_body_with_input(messages, tools, full_input, None);
            return self.send_request(body).await;
        };
        let _auth_epoch = self.auth_epoch.read().await;
        let Some(session) = registry.session_for_metadata(metadata).await else {
            let body = self.build_request_body_with_input(messages, tools, full_input, None);
            return self.send_request(body).await;
        };

        // Serialize requests only within the same explicitly-discriminated
        // agent-loop session. Generic run/turn IDs are intentionally ignored:
        // system-inference calls may share them with the parent loop.
        let mut state = session.lock().await;
        let plan = state.plan(&full_input);
        let body = self.build_request_body_with_input(
            messages,
            tools,
            plan.input,
            plan.previous_response_id.as_deref(),
        );

        match self.send_request(body).await {
            Ok(response) => {
                state.commit(
                    &full_input,
                    response.response_id.as_deref(),
                    response.response_status.as_deref(),
                    response.output_items.as_deref(),
                );
                Ok(response)
            }
            Err(error) => {
                // A failed or truncated attempt may have consumed unknown
                // server-side state. The next attempt must replay in full.
                state.reset();
                Err(error)
            }
        }
    }

    /// Send a request and parse the SSE response stream.
    async fn send_request(&self, body: serde_json::Value) -> Result<ParsedResponse, LlmError> {
        let url = format!("{}/responses", self.api_base_url);
        let headers = self.build_headers().await?;

        tracing::debug!(
            url = %url,
            model = %self.model,
            "Sending Responses API request"
        );

        let response = self
            .client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: "openai_codex".to_string(),
                reason: format!("HTTP request failed: {e}"),
            })?;

        let status = response.status();
        if !status.is_success() {
            // Extract Retry-After header before consuming the response body.
            // Supports both delay-seconds (RFC 7231 §7.1.3) and HTTP-date formats.
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| {
                    if let Ok(secs) = v.trim().parse::<u64>() {
                        return Some(std::time::Duration::from_secs(secs));
                    }
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(v.trim()) {
                        let now = chrono::Utc::now();
                        let delta = dt.signed_duration_since(now);
                        return Some(std::time::Duration::from_secs(
                            delta.num_seconds().max(0) as u64
                        ));
                    }
                    None
                });

            let body_text = response.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(LlmError::AuthFailed {
                    provider: "openai_codex".to_string(),
                });
            }
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(LlmError::RateLimited {
                    provider: "openai_codex".to_string(),
                    retry_after,
                });
            }
            // Context-overflow (HTTP 413, or a 400 whose body names a
            // context-length error) must surface as ContextLengthExceeded so
            // the loop's context-shrink recovery fires instead of a generic
            // RequestFailed.
            if let Some(error) = crate::error::context_length_error(status.as_u16(), &body_text) {
                return Err(error);
            }
            return Err(LlmError::RequestFailed {
                provider: "openai_codex".to_string(),
                reason: format!("HTTP {status}: {body_text}"),
            });
        }

        // Read the full body and parse SSE events
        let body_bytes = response
            .bytes()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: "openai_codex".to_string(),
                reason: format!("Failed to read response body: {e}"),
            })?;

        let body_text = String::from_utf8_lossy(&body_bytes);
        parse_sse_response(&body_text)
    }
}

#[async_trait]
impl LlmProvider for OpenAiCodexProvider {
    fn model_name(&self) -> &str {
        &self.model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    fn calculate_cost(&self, _input_tokens: u32, _output_tokens: u32) -> Decimal {
        Decimal::ZERO
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut messages = request.messages;
        crate::provider::sanitize_tool_messages(&mut messages);
        let parsed = self
            .send_completion_request(&messages, None, &request.metadata)
            .await?;

        Ok(CompletionResponse {
            content: parsed.text_content,
            input_tokens: parsed.input_tokens,
            output_tokens: parsed.output_tokens,
            finish_reason: parsed.finish_reason,
            reasoning: parsed.reasoning,
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

        // Build a reverse map so we can translate sanitized names back to originals.
        // Only needed when sanitization actually changes a name (e.g. MCP tools with dots).
        let name_map: std::collections::HashMap<String, String> = request
            .tools
            .iter()
            .filter_map(|t| {
                let sanitized = sanitize_tool_name(&t.name);
                if sanitized != t.name {
                    Some((sanitized, t.name.clone()))
                } else {
                    None
                }
            })
            .collect();

        let mut parsed = self
            .send_completion_request(&messages, Some(&request.tools), &request.metadata)
            .await?;

        // Reverse-map sanitized tool names back to originals so the caller
        // can look them up in the tool registry.
        if !name_map.is_empty() {
            for tc in &mut parsed.tool_calls {
                if let Some(original) = name_map.get(&tc.name) {
                    tc.name = original.clone();
                }
            }
        }

        // Strict-mode tool schemas advertise every optional as required+nullable,
        // so the model fills unset optionals with `null` (or `""` for some codex
        // models). Strip those placeholders against each tool's original schema so
        // only provided values reach the tool.
        crate::tool_schema::strip_unset_optional_fields(
            &mut parsed.tool_calls,
            &request.tools,
            crate::tool_schema::PlaceholderStrippingMode::NullAndEmptyStrings,
        );

        let finish_reason = if !parsed.tool_calls.is_empty() {
            FinishReason::ToolUse
        } else {
            parsed.finish_reason
        };

        Ok(ToolCompletionResponse {
            content: if parsed.text_content.is_empty() {
                None
            } else {
                Some(parsed.text_content)
            },
            tool_calls: parsed.tool_calls,
            input_tokens: parsed.input_tokens,
            output_tokens: parsed.output_tokens,
            finish_reason,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
            reasoning: parsed.reasoning,
            reasoning_details: None,
        })
    }

    /// Returns empty — Codex uses subscription-based access with a fixed model,
    /// no model enumeration API is available.
    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        Ok(vec![])
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        Ok(ModelMetadata {
            id: self.model.clone(),
            context_length: None,
        })
    }

    fn set_model(&self, _model: &str) -> Result<(), LlmError> {
        Err(LlmError::RequestFailed {
            provider: "openai_codex".to_string(),
            reason: "Cannot change model on Codex provider at runtime".to_string(),
        })
    }

    fn effective_model_name(&self, _requested_model: Option<&str>) -> String {
        self.model.clone()
    }
}

// ---------------------------------------------------------------------------
// JWT account ID extraction
// ---------------------------------------------------------------------------

/// Extract `chatgpt_account_id` from a JWT token's payload.
///
/// Matches OpenClaw's `extractAccountId` which reads:
/// `payload["https://api.openai.com/auth"]["chatgpt_account_id"]`
fn extract_account_id(token: &str) -> Result<String, LlmError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return Err(LlmError::RequestFailed {
            provider: "openai_codex".to_string(),
            reason: "JWT token has fewer than 2 parts".to_string(),
        });
    }

    use base64::Engine;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;

    // JWT base64url may need padding
    let payload_b64 = parts[1];
    let decoded = engine
        .decode(payload_b64)
        .map_err(|e| LlmError::RequestFailed {
            provider: "openai_codex".to_string(),
            reason: format!("Failed to decode JWT payload: {e}"),
        })?;

    let payload: serde_json::Value =
        serde_json::from_slice(&decoded).map_err(|e| LlmError::RequestFailed {
            provider: "openai_codex".to_string(),
            reason: format!("Failed to parse JWT payload as JSON: {e}"),
        })?;

    let account_id = payload
        .get("https://api.openai.com/auth")
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| LlmError::RequestFailed {
            provider: "openai_codex".to_string(),
            reason: "JWT payload missing chatgpt_account_id claim".to_string(),
        })?;

    Ok(account_id.to_string())
}

// ---------------------------------------------------------------------------
// Message conversion (matching OpenClaw's convertResponsesMessages)
// ---------------------------------------------------------------------------

fn normalized_input(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .filter(|message| message.role != Role::System)
        .enumerate()
        .flat_map(|(index, message)| convert_message(message, index))
        .collect()
}

/// Convert a single `ChatMessage` to Responses API `input` items.
///
/// Returns a Vec because assistant messages with tool_calls produce
/// one `function_call` item per tool call.
fn convert_message(msg: &ChatMessage, index: usize) -> Vec<serde_json::Value> {
    match msg.role {
        Role::System => {
            // System messages are handled separately as `instructions`
            vec![]
        }
        Role::User => {
            let image_count = msg
                .content_parts
                .iter()
                .filter(|p| matches!(p, ContentPart::ImageUrl { .. }))
                .count();
            if image_count > 0 {
                tracing::warn!(
                    "OpenAI Codex: {} image attachment(s) dropped — Responses API image support not yet implemented",
                    image_count
                );
            }
            vec![serde_json::json!({
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": msg.content,
                }],
            })]
        }
        Role::Assistant => {
            // Check if this message has tool calls
            if let Some(ref tool_calls) = msg.tool_calls {
                // Emit one function_call item per tool call
                tool_calls
                    .iter()
                    .map(|tc| {
                        let args_str = if tc.arguments.is_string() {
                            tc.arguments.as_str().unwrap_or("{}").to_string()
                        } else {
                            tc.arguments.to_string()
                        };
                        serde_json::json!({
                            "type": "function_call",
                            "call_id": tc.id,
                            "name": sanitize_tool_name(&tc.name),
                            "arguments": args_str,
                        })
                    })
                    .collect()
            } else {
                // Plain text assistant message
                vec![serde_json::json!({
                    "type": "message",
                    "role": "assistant",
                    "id": format!("msg_{index}"),
                    "status": "completed",
                    "content": [{
                        "type": "output_text",
                        "text": msg.content,
                        "annotations": [],
                    }],
                })]
            }
        }
        Role::Tool => {
            let call_id = msg.tool_call_id.as_deref().unwrap_or("unknown");
            vec![serde_json::json!({
                "type": "function_call_output",
                "call_id": call_id,
                "output": msg.content,
            })]
        }
    }
}

/// Sanitize a tool name to match the OpenAI Responses API pattern `^[a-zA-Z0-9_-]+$`.
/// Replaces any invalid character (e.g. dots in MCP tool names) with underscores.
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

/// Convert a `ToolDefinition` to Responses API tool format.
///
/// Applies the shared `tool_schema.rs` shaping entrypoint with the
/// `StrictOpenAi` policy, which performs strict-mode object normalization and
/// the top-level union flatten that the Responses API requires. The flatten
/// can append a hint to the tool description, so we pass an owned clone
/// through and read it back.
fn convert_tool_definition(tool: &ToolDefinition) -> serde_json::Value {
    use crate::tool_schema::{ToolSchemaPolicy, shape_tool_schema};

    let mut description = tool.description.clone();
    let parameters = shape_tool_schema(
        ToolSchemaPolicy::StrictOpenAi,
        &tool.parameters,
        &mut description,
    );

    serde_json::json!({
        "type": "function",
        "name": sanitize_tool_name(&tool.name),
        "description": description,
        "parameters": parameters,
    })
}

// ---------------------------------------------------------------------------
// SSE response parsing (matching OpenClaw's processResponsesStream)
// ---------------------------------------------------------------------------

/// Parsed result from the SSE stream.
#[derive(Debug)]
struct ParsedResponse {
    text_content: String,
    reasoning: Option<String>,
    tool_calls: Vec<ToolCall>,
    input_tokens: u32,
    output_tokens: u32,
    finish_reason: FinishReason,
    response_id: Option<String>,
    response_status: Option<String>,
    output_items: Option<Vec<serde_json::Value>>,
}

/// SSE event data from the Responses API.
#[derive(Debug, Deserialize)]
struct SseEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(flatten)]
    data: serde_json::Value,
}

/// Tracking state for an in-progress function call.
#[derive(Debug, Default)]
struct FunctionCallState {
    call_id: String,
    name: String,
    arguments: String,
}

/// Parse the full SSE response body into a `ParsedResponse`.
fn parse_sse_response(body: &str) -> Result<ParsedResponse, LlmError> {
    let mut text_content = String::new();
    let mut reasoning_summary = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut input_tokens: u32 = 0;
    let mut output_tokens: u32 = 0;
    let mut finish_reason = FinishReason::Stop;
    let mut active_function_calls: std::collections::HashMap<String, FunctionCallState> =
        std::collections::HashMap::new();
    let mut response_id: Option<String> = None;
    let mut response_status: Option<String> = None;
    let mut output_items: Option<Vec<serde_json::Value>> = None;
    // Whether a terminal `response.completed` event was observed. A stream
    // that ends without it (mid-stream disconnect) is truncated and must not
    // be reported as a successful `Stop` — see the truncated-stream guard
    // after the loop.
    let mut saw_completed = false;

    for line in body.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with(':') {
            continue;
        }

        // Parse SSE data lines
        let data_str = if let Some(stripped) = line.strip_prefix("data: ") {
            stripped.trim()
        } else if let Some(stripped) = line.strip_prefix("data:") {
            stripped.trim()
        } else {
            continue;
        };

        // Skip [DONE] marker
        if data_str == "[DONE]" {
            break;
        }

        // Parse JSON
        let event: SseEvent = match serde_json::from_str(data_str) {
            Ok(e) => e,
            Err(e) => {
                tracing::trace!(data = data_str, error = %e, "Skipping unparseable SSE event");
                continue;
            }
        };

        match event.event_type.as_str() {
            // Text output
            "response.output_text.delta" => {
                if let Some(delta) = event.data.get("delta").and_then(|d| d.as_str()) {
                    text_content.push_str(delta);
                }
            }
            event_type
                if crate::responses_reasoning::apply_summary_event(
                    &mut reasoning_summary,
                    event_type,
                    &event.data,
                ) => {}

            // Output item added (could be message or function_call)
            "response.output_item.added" => {
                if let Some(item) = event.data.get("item") {
                    let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    if item_type == "function_call" {
                        let item_id = item
                            .get("id")
                            .or_else(|| item.get("call_id"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = item
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let call_id = item
                            .get("call_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&item_id)
                            .to_string();
                        active_function_calls.insert(
                            item_id.clone(),
                            FunctionCallState {
                                call_id,
                                name,
                                arguments: String::new(),
                            },
                        );
                    }
                }
            }

            // Function call arguments streaming
            "response.function_call_arguments.delta" => {
                if let Some(delta) = event.data.get("delta").and_then(|d| d.as_str()) {
                    let item_id = event
                        .data
                        .get("item_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if let Some(state) = active_function_calls.get_mut(item_id) {
                        state.arguments.push_str(delta);
                    }
                }
            }

            // Function call arguments done
            "response.function_call_arguments.done" => {
                // Arguments are finalized, item_id used to match
                if let Some(args_str) = event.data.get("arguments").and_then(|a| a.as_str()) {
                    let item_id = event
                        .data
                        .get("item_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if let Some(state) = active_function_calls.get_mut(item_id) {
                        state.arguments = args_str.to_string();
                    }
                }
            }

            // Output item done (finalize function call)
            "response.output_item.done" => {
                if let Some(item) = event.data.get("item") {
                    let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    if item_type == "function_call" {
                        let item_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        if let Some(state) = active_function_calls.remove(item_id) {
                            let arguments: serde_json::Value =
                                serde_json::from_str(&state.arguments).unwrap_or_else(|_| {
                                    serde_json::Value::String(state.arguments.clone())
                                });
                            tool_calls.push(ToolCall {
                                id: state.call_id,
                                name: state.name,
                                arguments,
                                reasoning: None,
                                signature: None,
                                arguments_parse_error: None,
                            });
                        } else {
                            // Fallback: extract directly from the item
                            let call_id = item
                                .get("call_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or(item_id)
                                .to_string();
                            let name = item
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let args_str = item
                                .get("arguments")
                                .and_then(|v| v.as_str())
                                .unwrap_or("{}");
                            let arguments: serde_json::Value = serde_json::from_str(args_str)
                                .unwrap_or_else(|_| {
                                    serde_json::Value::String(args_str.to_string())
                                });
                            tool_calls.push(ToolCall {
                                id: call_id,
                                name,
                                arguments,
                                reasoning: None,
                                signature: None,
                                arguments_parse_error: None,
                            });
                        }
                    }
                }
            }

            // Response completed
            "response.completed" => {
                saw_completed = true;
                if let Some(response) = event.data.get("response") {
                    response_id = response
                        .get("id")
                        .and_then(|value| value.as_str())
                        .map(str::to_string);
                    output_items = response
                        .get("output")
                        .and_then(|value| value.as_array())
                        .cloned();
                    // Extract usage
                    if let Some(usage) = response.get("usage") {
                        input_tokens = usage
                            .get("input_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32;
                        output_tokens = usage
                            .get("output_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32;
                    }
                    // Extract status
                    if let Some(status) = response.get("status").and_then(|s| s.as_str()) {
                        response_status = Some(status.to_string());
                    }
                }
            }

            // Response failed
            "response.failed" => {
                let reason = event
                    .data
                    .get("response")
                    .and_then(|r| r.get("status_details"))
                    .and_then(|d| d.get("error"))
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown error");
                // Prefer ContextLengthExceeded for context-overflow failures
                // so the loop's context-shrink recovery fires.
                if crate::error::is_context_length_error_message(&reason.to_ascii_lowercase()) {
                    let (used, limit) =
                        crate::error::parse_context_token_counts(&reason.to_ascii_lowercase());
                    return Err(LlmError::ContextLengthExceeded { used, limit });
                }
                return Err(LlmError::RequestFailed {
                    provider: "openai_codex".to_string(),
                    reason: format!("Response failed: {reason}"),
                });
            }

            // Error event
            "error" => {
                let code = event
                    .data
                    .get("code")
                    .and_then(|c| c.as_str())
                    .unwrap_or("unknown");
                let message = event
                    .data
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown error");
                // A context-overflow surfaced as an SSE error must become
                // ContextLengthExceeded so the loop's context-shrink recovery
                // fires instead of a generic failure.
                if crate::error::is_context_length_error_message(&message.to_ascii_lowercase()) {
                    let (used, limit) =
                        crate::error::parse_context_token_counts(&message.to_ascii_lowercase());
                    return Err(LlmError::ContextLengthExceeded { used, limit });
                }
                return Err(LlmError::RequestFailed {
                    provider: "openai_codex".to_string(),
                    reason: format!("Error {code}: {message}"),
                });
            }

            _ => {
                // Ignore unhandled event types (e.g. response.created,
                // response.output_item.added for messages, etc.)
            }
        }
    }

    // Finalize any remaining active function calls
    for (_, state) in active_function_calls {
        if !state.name.is_empty() {
            let arguments: serde_json::Value = serde_json::from_str(&state.arguments)
                .unwrap_or(serde_json::Value::String(state.arguments));
            tool_calls.push(ToolCall {
                id: state.call_id,
                name: state.name,
                arguments,
                reasoning: None,
                signature: None,
                arguments_parse_error: None,
            });
        }
    }

    // Truncated-stream guard: the Responses API always emits a terminal
    // `response.completed` (errors return early above). If the stream ended
    // without one, the connection was dropped mid-response. Returning the
    // partial content as a successful `Stop` would let a dropped connection
    // masquerade as a normal completion, so surface a RETRYABLE error
    // instead. `EmptyResponse` when nothing was produced; `InvalidResponse`
    // when partial content/tool calls were captured.
    if !saw_completed {
        if text_content.is_empty() && tool_calls.is_empty() {
            return Err(LlmError::EmptyResponse {
                provider: "openai_codex".to_string(),
            });
        }
        return Err(LlmError::InvalidResponse {
            provider: "openai_codex".to_string(),
            reason: "stream ended before response.completed".to_string(),
        });
    }

    // Map status to finish reason (matching OpenClaw's mapStopReason)
    if !tool_calls.is_empty() {
        finish_reason = FinishReason::ToolUse;
    } else if let Some(ref status) = response_status {
        finish_reason = match status.as_str() {
            "completed" => FinishReason::Stop,
            "incomplete" => FinishReason::Length,
            _ => FinishReason::Stop,
        };
    }

    Ok(ParsedResponse {
        text_content,
        reasoning: crate::responses_reasoning::finish_summary(reasoning_summary),
        tool_calls,
        input_tokens,
        output_tokens,
        finish_reason,
        response_id,
        response_status,
        output_items,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "openai_codex_provider/tests.rs"]
mod session_tests;

#[cfg(test)]
#[path = "openai_codex_provider/unit_tests.rs"]
mod tests;
