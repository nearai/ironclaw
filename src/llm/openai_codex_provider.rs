//! OpenAI Codex Responses API client.
//!
//! Implements `LlmProvider` using the Responses API at
//! `chatgpt.com/backend-api/codex/responses` -- the endpoint that works
//! with ChatGPT subscription OAuth tokens.
//!
//! This mirrors OpenClaw's Responses API flow translated to Rust.

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::error::LlmError;
use crate::llm::provider::{
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
    auth: RwLock<AuthState>,
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
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(request_timeout_secs))
                .build()
                .map_err(|e| LlmError::RequestFailed {
                    provider: "openai_codex".to_string(),
                    reason: format!("Failed to create HTTP client: {e}"),
                })?,
            model: model.to_string(),
            api_base_url: api_base_url.trim_end_matches('/').to_string(),
            auth: RwLock::new(AuthState {
                token: token.to_string(),
                account_id,
            }),
        })
    }

    /// Update the access token after a refresh.
    pub async fn update_token(&self, token: &str) -> Result<(), LlmError> {
        let account_id = extract_account_id(token)?;
        *self.auth.write().await = AuthState {
            token: token.to_string(),
            account_id,
        };
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
    fn build_request_body(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolDefinition]>,
    ) -> serde_json::Value {
        // Separate system messages into `instructions`
        let instructions: String = messages
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");

        // Convert non-system messages to Responses API format
        let input: Vec<serde_json::Value> = messages
            .iter()
            .filter(|m| m.role != Role::System)
            .enumerate()
            .flat_map(|(i, m)| convert_message(m, i))
            .collect();

        let mut body = serde_json::json!({
            "model": self.model,
            "store": false,
            "stream": true,
            "input": input,
            "text": { "verbosity": "medium" },
            // Safe for non-reasoning models — API ignores unrecognized include values
            "include": ["reasoning.encrypted_content"],
        });

        if !instructions.is_empty() {
            body["instructions"] = serde_json::Value::String(instructions);
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
        let body = self.build_request_body(&request.messages, None);
        let parsed = self.send_request(body).await?;

        Ok(CompletionResponse {
            content: parsed.text_content,
            input_tokens: parsed.input_tokens,
            output_tokens: parsed.output_tokens,
            finish_reason: parsed.finish_reason,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
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

        let body = self.build_request_body(&request.messages, Some(&request.tools));
        let mut parsed = self.send_request(body).await?;

        // Reverse-map sanitized tool names back to originals so the caller
        // can look them up in the tool registry.
        if !name_map.is_empty() {
            for tc in &mut parsed.tool_calls {
                if let Some(original) = name_map.get(&tc.name) {
                    tc.name = original.clone();
                }
            }
        }

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
/// Applies strict-mode schema normalization (same as OpenAI Chat Completions):
/// `additionalProperties: false`, all properties required, optional fields nullable.
fn convert_tool_definition(tool: &ToolDefinition) -> serde_json::Value {
    use crate::llm::rig_adapter::normalize_schema_strict;

    serde_json::json!({
        "type": "function",
        "name": sanitize_tool_name(&tool.name),
        "description": tool.description,
        "parameters": normalize_schema_strict(&tool.parameters),
    })
}

// ---------------------------------------------------------------------------
// SSE response parsing (matching OpenClaw's processResponsesStream)
// ---------------------------------------------------------------------------

/// Parsed result from the SSE stream.
#[derive(Debug)]
struct ParsedResponse {
    text_content: String,
    tool_calls: Vec<ToolCall>,
    input_tokens: u32,
    output_tokens: u32,
    finish_reason: FinishReason,
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
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut input_tokens: u32 = 0;
    let mut output_tokens: u32 = 0;
    let mut finish_reason = FinishReason::Stop;
    let mut active_function_calls: std::collections::HashMap<String, FunctionCallState> =
        std::collections::HashMap::new();
    let mut response_status: Option<String> = None;

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
                                thought_signature: None,
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
                                thought_signature: None,
                            });
                        }
                    }
                }
            }

            // Response completed
            "response.completed" => {
                if let Some(response) = event.data.get("response") {
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
                thought_signature: None,
            });
        }
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
        tool_calls,
        input_tokens,
        output_tokens,
        finish_reason,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::codex_test_helpers::make_test_jwt;

    #[test]
    fn test_extract_account_id_success() {
        let jwt = make_test_jwt("acct_abc123");
        let result = extract_account_id(&jwt);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "acct_abc123");
    }

    #[test]
    fn test_extract_account_id_missing_claim() {
        use base64::Engine;
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let header = engine.encode(b"{\"alg\":\"RS256\"}");
        let payload = engine.encode(b"{\"sub\":\"user123\"}");
        let sig = engine.encode(b"sig");
        let jwt = format!("{header}.{payload}.{sig}");

        let result = extract_account_id(&jwt);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_account_id_invalid_jwt() {
        let result = extract_account_id("not-a-jwt");
        assert!(result.is_err());
    }

    #[test]
    fn test_convert_user_message() {
        let msg = ChatMessage::user("Hello world");
        let items = convert_message(&msg, 0);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["role"], "user");
        assert_eq!(items[0]["content"][0]["type"], "input_text");
        assert_eq!(items[0]["content"][0]["text"], "Hello world");
    }

    #[test]
    fn test_convert_system_message_excluded() {
        let msg = ChatMessage::system("You are helpful");
        let items = convert_message(&msg, 0);
        assert!(items.is_empty());
    }

    #[test]
    fn test_convert_assistant_text_message() {
        let msg = ChatMessage::assistant("Sure, I can help");
        let items = convert_message(&msg, 3);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "message");
        assert_eq!(items[0]["role"], "assistant");
        assert_eq!(items[0]["id"], "msg_3");
        assert_eq!(items[0]["content"][0]["type"], "output_text");
    }

    #[test]
    fn test_convert_assistant_with_tool_calls() {
        let tool_calls = vec![
            ToolCall {
                id: "call_1".to_string(),
                name: "search".to_string(),
                arguments: serde_json::json!({"query": "test"}),
                reasoning: None,
                thought_signature: None,
            },
            ToolCall {
                id: "call_2".to_string(),
                name: "read".to_string(),
                arguments: serde_json::json!({"path": "/tmp"}),
                reasoning: None,
                thought_signature: None,
            },
        ];
        let msg =
            ChatMessage::assistant_with_tool_calls(Some("Let me check".to_string()), tool_calls);
        let items = convert_message(&msg, 0);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["type"], "function_call");
        assert_eq!(items[0]["call_id"], "call_1");
        assert_eq!(items[0]["name"], "search");
        assert_eq!(items[1]["type"], "function_call");
        assert_eq!(items[1]["call_id"], "call_2");
    }

    #[test]
    fn test_convert_tool_result_message() {
        let msg = ChatMessage::tool_result("call_1", "search", "found 3 results");
        let items = convert_message(&msg, 0);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "function_call_output");
        assert_eq!(items[0]["call_id"], "call_1");
        assert_eq!(items[0]["output"], "found 3 results");
    }

    #[test]
    fn test_convert_tool_definition() {
        let tool = ToolDefinition {
            name: "my_tool".to_string(),
            description: "Does things".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "x": { "type": "string" }
                }
            }),
        };
        let json = convert_tool_definition(&tool);
        assert_eq!(json["type"], "function");
        assert_eq!(json["name"], "my_tool");
        assert_eq!(json["description"], "Does things");
    }

    #[test]
    fn test_parse_sse_text_response() {
        let sse_body = r#"data: {"type":"response.output_item.added","item":{"type":"message","role":"assistant","id":"msg_1"}}

data: {"type":"response.output_text.delta","delta":"Hello "}

data: {"type":"response.output_text.delta","delta":"world!"}

data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":10,"output_tokens":5}}}

"#;
        let result = parse_sse_response(sse_body);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.text_content, "Hello world!");
        assert_eq!(parsed.input_tokens, 10);
        assert_eq!(parsed.output_tokens, 5);
        assert_eq!(parsed.finish_reason, FinishReason::Stop);
        assert!(parsed.tool_calls.is_empty());
    }

    #[test]
    fn test_parse_sse_tool_call_response() {
        let sse_body = r#"data: {"type":"response.output_item.added","item":{"type":"function_call","id":"fc_1","call_id":"call_abc","name":"search"}}

data: {"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"{\"query\":"}

data: {"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"\"test\"}"}

data: {"type":"response.output_item.done","item":{"type":"function_call","id":"fc_1","call_id":"call_abc","name":"search","arguments":"{\"query\":\"test\"}"}}

data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":15,"output_tokens":8}}}

"#;
        let result = parse_sse_response(sse_body);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert!(parsed.text_content.is_empty());
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].id, "call_abc");
        assert_eq!(parsed.tool_calls[0].name, "search");
        assert_eq!(
            parsed.tool_calls[0].arguments,
            serde_json::json!({"query": "test"})
        );
        assert_eq!(parsed.finish_reason, FinishReason::ToolUse);
    }

    #[test]
    fn test_parse_sse_error_response() {
        let sse_body = r#"data: {"type":"error","code":"rate_limit_exceeded","message":"Too many requests"}

"#;
        let result = parse_sse_response(sse_body);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("rate_limit_exceeded"));
    }

    #[test]
    fn test_parse_sse_failed_response() {
        let sse_body = r#"data: {"type":"response.failed","response":{"status":"failed","status_details":{"error":{"message":"Model overloaded"}}}}

"#;
        let result = parse_sse_response(sse_body);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Model overloaded"));
    }

    #[test]
    fn test_parse_sse_incomplete_status() {
        let sse_body = r#"data: {"type":"response.output_text.delta","delta":"partial"}

data: {"type":"response.completed","response":{"status":"incomplete","usage":{"input_tokens":5,"output_tokens":2}}}

"#;
        let result = parse_sse_response(sse_body);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.text_content, "partial");
        assert_eq!(parsed.finish_reason, FinishReason::Length);
    }

    #[test]
    fn test_parse_sse_done_marker() {
        let sse_body = r#"data: {"type":"response.output_text.delta","delta":"hello"}

data: [DONE]

data: {"type":"response.output_text.delta","delta":" ignored"}

"#;
        let result = parse_sse_response(sse_body);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.text_content, "hello");
    }

    #[tokio::test]
    async fn test_provider_new() {
        let jwt = make_test_jwt("acct_test");
        let provider = OpenAiCodexProvider::new(
            "gpt-5.3-codex",
            "https://chatgpt.com/backend-api/codex",
            &jwt,
            300,
        );
        assert!(provider.is_ok());
        let provider = provider.unwrap();
        assert_eq!(provider.model_name(), "gpt-5.3-codex");
        assert_eq!(provider.cost_per_token(), (Decimal::ZERO, Decimal::ZERO));
        assert_eq!(provider.calculate_cost(1000, 500), Decimal::ZERO);
    }

    #[tokio::test]
    async fn test_update_token() {
        let jwt1 = make_test_jwt("acct_old");
        let provider = OpenAiCodexProvider::new(
            "gpt-5.3-codex",
            "https://chatgpt.com/backend-api/codex",
            &jwt1,
            300,
        )
        .unwrap();

        let jwt2 = make_test_jwt("acct_new");
        let result = provider.update_token(&jwt2).await;
        assert!(result.is_ok());

        // Verify account_id was updated
        let auth = provider.auth.read().await;
        assert_eq!(auth.account_id, "acct_new");
    }

    #[test]
    fn test_build_request_body_structure() {
        let jwt = make_test_jwt("acct_test");
        let provider = OpenAiCodexProvider::new(
            "gpt-5.3-codex",
            "https://chatgpt.com/backend-api/codex",
            &jwt,
            300,
        )
        .unwrap();

        let messages = vec![
            ChatMessage::system("You are helpful"),
            ChatMessage::user("Hello"),
        ];

        let body = provider.build_request_body(&messages, None);

        assert_eq!(body["model"], "gpt-5.3-codex");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert_eq!(body["instructions"], "You are helpful");
        // input should only contain the user message, not system
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["role"], "user");
        // No tools
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn test_build_request_body_with_tools() {
        let jwt = make_test_jwt("acct_test");
        let provider = OpenAiCodexProvider::new(
            "gpt-5.3-codex",
            "https://chatgpt.com/backend-api/codex",
            &jwt,
            300,
        )
        .unwrap();

        let messages = vec![ChatMessage::user("Search for X")];
        let tools = vec![ToolDefinition {
            name: "search".to_string(),
            description: "Search for things".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        }];

        let body = provider.build_request_body(&messages, Some(&tools));

        assert!(body.get("tools").is_some());
        let tools_arr = body["tools"].as_array().unwrap();
        assert_eq!(tools_arr.len(), 1);
        assert_eq!(tools_arr[0]["type"], "function");
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["parallel_tool_calls"], true);
    }

    #[test]
    fn test_parse_sse_multiple_tool_calls() {
        let sse_body = r#"data: {"type":"response.output_item.added","item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"read_file"}}

data: {"type":"response.function_call_arguments.done","item_id":"fc_1","arguments":"{\"path\":\"/tmp/a\"}"}

data: {"type":"response.output_item.done","item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"read_file","arguments":"{\"path\":\"/tmp/a\"}"}}

data: {"type":"response.output_item.added","item":{"type":"function_call","id":"fc_2","call_id":"call_2","name":"read_file"}}

data: {"type":"response.function_call_arguments.done","item_id":"fc_2","arguments":"{\"path\":\"/tmp/b\"}"}

data: {"type":"response.output_item.done","item":{"type":"function_call","id":"fc_2","call_id":"call_2","name":"read_file","arguments":"{\"path\":\"/tmp/b\"}"}}

data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":20,"output_tokens":12}}}

"#;
        let result = parse_sse_response(sse_body);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.tool_calls.len(), 2);
        assert_eq!(parsed.tool_calls[0].id, "call_1");
        assert_eq!(parsed.tool_calls[0].name, "read_file");
        assert_eq!(parsed.tool_calls[1].id, "call_2");
        assert_eq!(parsed.tool_calls[1].name, "read_file");
        assert_eq!(parsed.finish_reason, FinishReason::ToolUse);
    }

    /// Regression test: tool names with dots (e.g. MCP tools) must be sanitized
    /// to match OpenAI's `^[a-zA-Z0-9_-]+$` pattern.
    #[test]
    fn test_sanitize_tool_name_replaces_dots() {
        assert_eq!(super::sanitize_tool_name("memory_search"), "memory_search");
        assert_eq!(
            super::sanitize_tool_name("mcp.server.tool"),
            "mcp_server_tool"
        );
        assert_eq!(super::sanitize_tool_name("tool@v2"), "tool_v2");
        assert_eq!(super::sanitize_tool_name("my-tool"), "my-tool");
    }

    /// Regression test: convert_tool_definition sanitizes the name.
    #[test]
    fn test_convert_tool_definition_sanitizes_name() {
        let tool = ToolDefinition {
            name: "mcp.server.search".to_string(),
            description: "Search".to_string(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        };
        let json = super::convert_tool_definition(&tool);
        assert_eq!(json["name"], "mcp_server_search");
    }

    /// Regression test: function_call items sanitize tool names.
    #[test]
    fn test_convert_message_sanitizes_tool_call_name() {
        let tool_calls = vec![ToolCall {
            id: "call_1".to_string(),
            name: "mcp.server.search".to_string(),
            arguments: serde_json::json!({"q": "test"}),
            reasoning: None,
            thought_signature: None,
        }];
        let msg = ChatMessage::assistant_with_tool_calls(None, tool_calls);
        let items = super::convert_message(&msg, 0);
        assert_eq!(items[0]["name"], "mcp_server_search");
    }

    /// Regression: sanitized tool names in API responses must be reverse-mapped
    /// back to original names so the tool registry can look them up.
    #[test]
    fn test_sanitized_name_reverse_mapping() {
        use std::collections::HashMap;

        let tools = [
            ToolDefinition {
                name: "mcp.server.search".to_string(),
                description: "Search".to_string(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            },
            ToolDefinition {
                name: "memory_search".to_string(),
                description: "Memory".to_string(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            },
        ];

        // Build name map (same logic as complete_with_tools)
        let name_map: HashMap<String, String> = tools
            .iter()
            .filter_map(|t| {
                let sanitized = super::sanitize_tool_name(&t.name);
                if sanitized != t.name {
                    Some((sanitized, t.name.clone()))
                } else {
                    None
                }
            })
            .collect();

        // Only the MCP tool should appear (its name changed)
        assert_eq!(name_map.len(), 1);
        assert_eq!(
            name_map.get("mcp_server_search"),
            Some(&"mcp.server.search".to_string())
        );

        // Simulate a tool call coming back with the sanitized name
        let mut tc = ToolCall {
            id: "call_1".to_string(),
            name: "mcp_server_search".to_string(),
            arguments: serde_json::json!({}),
            reasoning: None,
            thought_signature: None,
        };
        if let Some(original) = name_map.get(&tc.name) {
            tc.name = original.clone();
        }
        assert_eq!(tc.name, "mcp.server.search");
    }
}
