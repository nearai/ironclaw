//! OpenAI-compatible HTTP API (`/v1/chat/completions`, `/v1/models`).
//!
//! This module provides a direct LLM proxy through the web gateway so any
//! standard OpenAI client library can use IronClaw as a backend by simply
//! changing the `base_url`.

use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::{HeaderValue, StatusCode},
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use serde::{Deserialize, Serialize};

use crate::llm::{
    ChatMessage, CompletionRequest, FinishReason, Role, ToolCall, ToolCompletionRequest,
    ToolDefinition,
};

use super::server::GatewayState;

// ---------------------------------------------------------------------------
// OpenAI request types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct OpenAiChatRequest {
    pub model: String,
    pub messages: Vec<OpenAiMessage>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub tools: Option<Vec<OpenAiTool>>,
    #[serde(default)]
    pub tool_choice: Option<serde_json::Value>,
    #[serde(default)]
    pub stop: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: OpenAiFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiFunction {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: OpenAiToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiToolCallFunction {
    pub name: String,
    pub arguments: String,
}

// ---------------------------------------------------------------------------
// OpenAI response types (non-streaming)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct OpenAiChatResponse {
    pub id: String,
    pub object: &'static str,
    pub created: u64,
    pub model: String,
    pub choices: Vec<OpenAiChoice>,
    pub usage: OpenAiUsage,
}

#[derive(Debug, Serialize)]
pub struct OpenAiChoice {
    pub index: u32,
    pub message: OpenAiMessage,
    pub finish_reason: String,
}

#[derive(Debug, Serialize)]
pub struct OpenAiUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

// ---------------------------------------------------------------------------
// OpenAI response types (streaming)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct OpenAiChatChunk {
    pub id: String,
    pub object: &'static str,
    pub created: u64,
    pub model: String,
    pub choices: Vec<OpenAiChunkChoice>,
}

#[derive(Debug, Serialize)]
pub struct OpenAiChunkChoice {
    pub index: u32,
    pub delta: OpenAiDelta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OpenAiDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OpenAiToolCallDelta>>,
}

#[derive(Debug, Serialize)]
pub struct OpenAiToolCallDelta {
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub call_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<OpenAiToolCallFunctionDelta>,
}

#[derive(Debug, Serialize)]
pub struct OpenAiToolCallFunctionDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

// ---------------------------------------------------------------------------
// Error response
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct OpenAiErrorResponse {
    pub error: OpenAiErrorDetail,
}

#[derive(Debug, Serialize)]
pub struct OpenAiErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: String,
    pub param: Option<String>,
    pub code: Option<String>,
}

// ---------------------------------------------------------------------------
// Conversion functions
// ---------------------------------------------------------------------------

fn parse_role(s: &str) -> Role {
    match s {
        "system" => Role::System,
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        _ => Role::User,
    }
}

pub fn convert_messages(messages: &[OpenAiMessage]) -> Vec<ChatMessage> {
    messages
        .iter()
        .map(|m| {
            let role = parse_role(&m.role);
            match role {
                Role::Tool => ChatMessage::tool_result(
                    m.tool_call_id.as_deref().unwrap_or(""),
                    m.name.as_deref().unwrap_or(""),
                    m.content.as_deref().unwrap_or(""),
                ),
                Role::Assistant => {
                    if let Some(ref tcs) = m.tool_calls {
                        let calls: Vec<ToolCall> = tcs
                            .iter()
                            .map(|tc| ToolCall {
                                id: tc.id.clone(),
                                name: tc.function.name.clone(),
                                arguments: serde_json::from_str(&tc.function.arguments)
                                    .unwrap_or(serde_json::Value::Object(Default::default())),
                            })
                            .collect();
                        ChatMessage::assistant_with_tool_calls(m.content.clone(), calls)
                    } else {
                        ChatMessage::assistant(m.content.as_deref().unwrap_or(""))
                    }
                }
                _ => ChatMessage {
                    role,
                    content: m.content.as_deref().unwrap_or("").to_string(),
                    tool_call_id: None,
                    name: m.name.clone(),
                    tool_calls: None,
                },
            }
        })
        .collect()
}

pub fn convert_tools(tools: &[OpenAiTool]) -> Vec<ToolDefinition> {
    tools
        .iter()
        .filter(|t| t.tool_type == "function")
        .map(|t| ToolDefinition {
            name: t.function.name.clone(),
            description: t.function.description.clone().unwrap_or_default(),
            parameters: t
                .function
                .parameters
                .clone()
                .unwrap_or(serde_json::json!({"type": "object", "properties": {}})),
        })
        .collect()
}

fn convert_tool_calls_to_openai(calls: &[ToolCall]) -> Vec<OpenAiToolCall> {
    calls
        .iter()
        .map(|tc| OpenAiToolCall {
            id: tc.id.clone(),
            call_type: "function".to_string(),
            function: OpenAiToolCallFunction {
                name: tc.name.clone(),
                arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
            },
        })
        .collect()
}

pub fn finish_reason_str(reason: FinishReason) -> String {
    match reason {
        FinishReason::Stop => "stop".to_string(),
        FinishReason::Length => "length".to_string(),
        FinishReason::ToolUse => "tool_calls".to_string(),
        FinishReason::ContentFilter => "content_filter".to_string(),
        FinishReason::Unknown => "stop".to_string(),
    }
}

fn normalize_tool_choice(val: &serde_json::Value) -> Option<String> {
    match val {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(obj) => {
            // { "type": "function", "function": { "name": "foo" } } → "required"
            if obj.contains_key("function") {
                Some("required".to_string())
            } else {
                obj.get("type")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            }
        }
        _ => None,
    }
}

fn map_llm_error(err: crate::error::LlmError) -> (StatusCode, Json<OpenAiErrorResponse>) {
    let (status, error_type, code) = match &err {
        crate::error::LlmError::AuthFailed { .. }
        | crate::error::LlmError::SessionExpired { .. } => (
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            "auth_error",
        ),
        crate::error::LlmError::RateLimited { .. } => (
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limit_error",
            "rate_limit",
        ),
        crate::error::LlmError::ContextLengthExceeded { .. } => (
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "context_length_exceeded",
        ),
        crate::error::LlmError::ModelNotAvailable { .. } => (
            StatusCode::NOT_FOUND,
            "invalid_request_error",
            "model_not_found",
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "internal_error",
        ),
    };

    (
        status,
        Json(OpenAiErrorResponse {
            error: OpenAiErrorDetail {
                message: err.to_string(),
                error_type: error_type.to_string(),
                param: None,
                code: Some(code.to_string()),
            },
        }),
    )
}

fn openai_error(
    status: StatusCode,
    message: impl Into<String>,
    error_type: &str,
) -> (StatusCode, Json<OpenAiErrorResponse>) {
    (
        status,
        Json(OpenAiErrorResponse {
            error: OpenAiErrorDetail {
                message: message.into(),
                error_type: error_type.to_string(),
                param: None,
                code: None,
            },
        }),
    )
}

fn chat_completion_id() -> String {
    format!("chatcmpl-{}", uuid::Uuid::new_v4().simple())
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Extract stop sequences from the flexible `stop` field.
fn parse_stop(val: &serde_json::Value) -> Option<Vec<String>> {
    match val {
        serde_json::Value::String(s) => Some(vec![s.clone()]),
        serde_json::Value::Array(arr) => {
            let strs: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            if strs.is_empty() { None } else { Some(strs) }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub async fn chat_completions_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<OpenAiChatRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<OpenAiErrorResponse>)> {
    let llm = state.llm_provider.as_ref().ok_or_else(|| {
        openai_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "LLM provider not configured",
            "server_error",
        )
    })?;

    if req.messages.is_empty() {
        return Err(openai_error(
            StatusCode::BAD_REQUEST,
            "messages must not be empty",
            "invalid_request_error",
        ));
    }

    // Validate the requested model matches the active model.
    // Per-request model switching is not yet supported (see GH issue).
    let active_model = llm.active_model_name();
    if req.model != active_model {
        return Err((
            StatusCode::NOT_FOUND,
            Json(OpenAiErrorResponse {
                error: OpenAiErrorDetail {
                    message: format!(
                        "Model '{}' not found. The active model is '{}'.",
                        req.model, active_model
                    ),
                    error_type: "invalid_request_error".to_string(),
                    param: Some("model".to_string()),
                    code: Some("model_not_found".to_string()),
                },
            }),
        ));
    }

    let has_tools = req.tools.as_ref().is_some_and(|t| !t.is_empty());
    let stream = req.stream.unwrap_or(false);

    if stream {
        return Ok(handle_streaming(llm.clone(), req, has_tools)
            .await
            .into_response());
    }

    // --- Non-streaming path ---

    let messages = convert_messages(&req.messages);
    let model_name = llm.active_model_name();
    let id = chat_completion_id();
    let created = unix_timestamp();

    if has_tools {
        let tools = convert_tools(req.tools.as_deref().unwrap_or(&[]));
        let mut tool_req = ToolCompletionRequest::new(messages, tools);
        if let Some(t) = req.temperature {
            tool_req = tool_req.with_temperature(t);
        }
        if let Some(mt) = req.max_tokens {
            tool_req = tool_req.with_max_tokens(mt);
        }
        if let Some(ref tc) = req.tool_choice {
            if let Some(choice) = normalize_tool_choice(tc) {
                tool_req = tool_req.with_tool_choice(choice);
            }
        }

        let resp = llm
            .complete_with_tools(tool_req)
            .await
            .map_err(map_llm_error)?;

        let tool_calls_openai = if resp.tool_calls.is_empty() {
            None
        } else {
            Some(convert_tool_calls_to_openai(&resp.tool_calls))
        };

        let response = OpenAiChatResponse {
            id,
            object: "chat.completion",
            created,
            model: model_name,
            choices: vec![OpenAiChoice {
                index: 0,
                message: OpenAiMessage {
                    role: "assistant".to_string(),
                    content: resp.content.clone(),
                    name: None,
                    tool_call_id: None,
                    tool_calls: tool_calls_openai,
                },
                finish_reason: finish_reason_str(resp.finish_reason),
            }],
            usage: OpenAiUsage {
                prompt_tokens: resp.input_tokens,
                completion_tokens: resp.output_tokens,
                total_tokens: resp.input_tokens + resp.output_tokens,
            },
        };

        Ok(Json(response).into_response())
    } else {
        let mut comp_req = CompletionRequest::new(messages);
        if let Some(t) = req.temperature {
            comp_req = comp_req.with_temperature(t);
        }
        if let Some(mt) = req.max_tokens {
            comp_req = comp_req.with_max_tokens(mt);
        }
        if let Some(ref stop_val) = req.stop {
            comp_req.stop_sequences = parse_stop(stop_val);
        }

        let resp = llm.complete(comp_req).await.map_err(map_llm_error)?;

        let response = OpenAiChatResponse {
            id,
            object: "chat.completion",
            created,
            model: model_name,
            choices: vec![OpenAiChoice {
                index: 0,
                message: OpenAiMessage {
                    role: "assistant".to_string(),
                    content: Some(resp.content),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
                finish_reason: finish_reason_str(resp.finish_reason),
            }],
            usage: OpenAiUsage {
                prompt_tokens: resp.input_tokens,
                completion_tokens: resp.output_tokens,
                total_tokens: resp.input_tokens + resp.output_tokens,
            },
        };

        Ok(Json(response).into_response())
    }
}

/// Handle streaming responses.
///
/// The current `LlmProvider` returns complete responses (no streaming method).
/// We call the LLM in a spawned task and simulate chunked delivery by splitting
/// the response into word-boundary chunks. True token streaming can be added
/// later by extending `LlmProvider` with a `complete_stream()` method.
async fn handle_streaming(
    llm: Arc<dyn crate::llm::LlmProvider>,
    req: OpenAiChatRequest,
    has_tools: bool,
) -> impl IntoResponse {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(64);

    let model_name = llm.active_model_name();
    let id = chat_completion_id();
    let created = unix_timestamp();

    tokio::spawn(async move {
        let messages = convert_messages(&req.messages);

        // Send initial chunk with role
        let role_chunk = OpenAiChatChunk {
            id: id.clone(),
            object: "chat.completion.chunk",
            created,
            model: model_name.clone(),
            choices: vec![OpenAiChunkChoice {
                index: 0,
                delta: OpenAiDelta {
                    role: Some("assistant".to_string()),
                    content: None,
                    tool_calls: None,
                },
                finish_reason: None,
            }],
        };
        let data = serde_json::to_string(&role_chunk).unwrap_or_default();
        let _ = tx.send(Ok(Event::default().data(data))).await;

        if has_tools {
            let tools = convert_tools(req.tools.as_deref().unwrap_or(&[]));
            let mut tool_req = ToolCompletionRequest::new(messages, tools);
            if let Some(t) = req.temperature {
                tool_req = tool_req.with_temperature(t);
            }
            if let Some(mt) = req.max_tokens {
                tool_req = tool_req.with_max_tokens(mt);
            }
            if let Some(ref tc) = req.tool_choice {
                if let Some(choice) = normalize_tool_choice(tc) {
                    tool_req = tool_req.with_tool_choice(choice);
                }
            }

            match llm.complete_with_tools(tool_req).await {
                Ok(resp) => {
                    // Stream content chunks
                    if let Some(ref content) = resp.content {
                        stream_content_chunks(&tx, &id, created, &model_name, content).await;
                    }

                    // Stream tool calls
                    if !resp.tool_calls.is_empty() {
                        let deltas: Vec<OpenAiToolCallDelta> = resp
                            .tool_calls
                            .iter()
                            .enumerate()
                            .map(|(i, tc)| OpenAiToolCallDelta {
                                index: i as u32,
                                id: Some(tc.id.clone()),
                                call_type: Some("function".to_string()),
                                function: Some(OpenAiToolCallFunctionDelta {
                                    name: Some(tc.name.clone()),
                                    arguments: Some(
                                        serde_json::to_string(&tc.arguments).unwrap_or_default(),
                                    ),
                                }),
                            })
                            .collect();

                        let chunk = OpenAiChatChunk {
                            id: id.clone(),
                            object: "chat.completion.chunk",
                            created,
                            model: model_name.clone(),
                            choices: vec![OpenAiChunkChoice {
                                index: 0,
                                delta: OpenAiDelta {
                                    role: None,
                                    content: None,
                                    tool_calls: Some(deltas),
                                },
                                finish_reason: None,
                            }],
                        };
                        let data = serde_json::to_string(&chunk).unwrap_or_default();
                        let _ = tx.send(Ok(Event::default().data(data))).await;
                    }

                    // Final chunk with finish_reason
                    send_finish_chunk(&tx, &id, created, &model_name, resp.finish_reason).await;
                }
                Err(e) => {
                    let err_data = serde_json::to_string(&OpenAiErrorResponse {
                        error: OpenAiErrorDetail {
                            message: e.to_string(),
                            error_type: "server_error".to_string(),
                            param: None,
                            code: None,
                        },
                    })
                    .unwrap_or_default();
                    let _ = tx
                        .send(Ok(Event::default().event("error").data(err_data)))
                        .await;
                }
            }
        } else {
            let mut comp_req = CompletionRequest::new(messages);
            if let Some(t) = req.temperature {
                comp_req = comp_req.with_temperature(t);
            }
            if let Some(mt) = req.max_tokens {
                comp_req = comp_req.with_max_tokens(mt);
            }
            if let Some(ref stop_val) = req.stop {
                comp_req.stop_sequences = parse_stop(stop_val);
            }

            match llm.complete(comp_req).await {
                Ok(resp) => {
                    stream_content_chunks(&tx, &id, created, &model_name, &resp.content).await;
                    send_finish_chunk(&tx, &id, created, &model_name, resp.finish_reason).await;
                }
                Err(e) => {
                    let err_data = serde_json::to_string(&OpenAiErrorResponse {
                        error: OpenAiErrorDetail {
                            message: e.to_string(),
                            error_type: "server_error".to_string(),
                            param: None,
                            code: None,
                        },
                    })
                    .unwrap_or_default();
                    let _ = tx
                        .send(Ok(Event::default().event("error").data(err_data)))
                        .await;
                }
            }
        }

        // Send [DONE] sentinel
        let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let sse = Sse::new(stream).keep_alive(KeepAlive::new().text(""));
    let mut response = sse.into_response();
    response.headers_mut().insert(
        "x-ironclaw-streaming",
        HeaderValue::from_static("simulated"),
    );
    response
}

/// Split content into word-boundary chunks and send as SSE events.
async fn stream_content_chunks(
    tx: &tokio::sync::mpsc::Sender<Result<Event, std::convert::Infallible>>,
    id: &str,
    created: u64,
    model: &str,
    content: &str,
) {
    // Split on word boundaries, grouping ~20 chars per chunk
    let mut buf = String::new();
    for word in content.split_inclusive(char::is_whitespace) {
        buf.push_str(word);
        if buf.len() >= 20 {
            let chunk = OpenAiChatChunk {
                id: id.to_string(),
                object: "chat.completion.chunk",
                created,
                model: model.to_string(),
                choices: vec![OpenAiChunkChoice {
                    index: 0,
                    delta: OpenAiDelta {
                        role: None,
                        content: Some(buf.clone()),
                        tool_calls: None,
                    },
                    finish_reason: None,
                }],
            };
            let data = serde_json::to_string(&chunk).unwrap_or_default();
            if tx.send(Ok(Event::default().data(data))).await.is_err() {
                return;
            }
            buf.clear();
        }
    }
    // Flush remaining
    if !buf.is_empty() {
        let chunk = OpenAiChatChunk {
            id: id.to_string(),
            object: "chat.completion.chunk",
            created,
            model: model.to_string(),
            choices: vec![OpenAiChunkChoice {
                index: 0,
                delta: OpenAiDelta {
                    role: None,
                    content: Some(buf),
                    tool_calls: None,
                },
                finish_reason: None,
            }],
        };
        let data = serde_json::to_string(&chunk).unwrap_or_default();
        let _ = tx.send(Ok(Event::default().data(data))).await;
    }
}

async fn send_finish_chunk(
    tx: &tokio::sync::mpsc::Sender<Result<Event, std::convert::Infallible>>,
    id: &str,
    created: u64,
    model: &str,
    reason: FinishReason,
) {
    let chunk = OpenAiChatChunk {
        id: id.to_string(),
        object: "chat.completion.chunk",
        created,
        model: model.to_string(),
        choices: vec![OpenAiChunkChoice {
            index: 0,
            delta: OpenAiDelta {
                role: None,
                content: None,
                tool_calls: None,
            },
            finish_reason: Some(finish_reason_str(reason)),
        }],
    };
    let data = serde_json::to_string(&chunk).unwrap_or_default();
    let _ = tx.send(Ok(Event::default().data(data))).await;
}

pub async fn models_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<OpenAiErrorResponse>)> {
    let llm = state.llm_provider.as_ref().ok_or_else(|| {
        openai_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "LLM provider not configured",
            "server_error",
        )
    })?;

    let model_name = llm.active_model_name();
    let created = unix_timestamp();

    // Try to fetch available models from the provider
    let models = match llm.list_models().await {
        Ok(names) if !names.is_empty() => names
            .into_iter()
            .map(|name| {
                serde_json::json!({
                    "id": name,
                    "object": "model",
                    "created": created,
                    "owned_by": "ironclaw"
                })
            })
            .collect(),
        _ => {
            // Fallback: return just the active model
            vec![serde_json::json!({
                "id": model_name,
                "object": "model",
                "created": created,
                "owned_by": "ironclaw"
            })]
        }
    };

    Ok(Json(serde_json::json!({
        "object": "list",
        "data": models
    })))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_role() {
        assert_eq!(parse_role("system"), Role::System);
        assert_eq!(parse_role("user"), Role::User);
        assert_eq!(parse_role("assistant"), Role::Assistant);
        assert_eq!(parse_role("tool"), Role::Tool);
        assert_eq!(parse_role("unknown"), Role::User);
    }

    #[test]
    fn test_finish_reason_str() {
        assert_eq!(finish_reason_str(FinishReason::Stop), "stop");
        assert_eq!(finish_reason_str(FinishReason::Length), "length");
        assert_eq!(finish_reason_str(FinishReason::ToolUse), "tool_calls");
        assert_eq!(
            finish_reason_str(FinishReason::ContentFilter),
            "content_filter"
        );
        assert_eq!(finish_reason_str(FinishReason::Unknown), "stop");
    }

    #[test]
    fn test_convert_messages_basic() {
        let msgs = vec![
            OpenAiMessage {
                role: "system".to_string(),
                content: Some("You are helpful.".to_string()),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
            OpenAiMessage {
                role: "user".to_string(),
                content: Some("Hello".to_string()),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
        ];

        let converted = convert_messages(&msgs);
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0].role, Role::System);
        assert_eq!(converted[0].content, "You are helpful.");
        assert_eq!(converted[1].role, Role::User);
        assert_eq!(converted[1].content, "Hello");
    }

    #[test]
    fn test_convert_messages_with_tool_results() {
        let msgs = vec![OpenAiMessage {
            role: "tool".to_string(),
            content: Some("42".to_string()),
            name: Some("calculator".to_string()),
            tool_call_id: Some("call_123".to_string()),
            tool_calls: None,
        }];

        let converted = convert_messages(&msgs);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, Role::Tool);
        assert_eq!(converted[0].content, "42");
        assert_eq!(converted[0].tool_call_id.as_deref(), Some("call_123"));
        assert_eq!(converted[0].name.as_deref(), Some("calculator"));
    }

    #[test]
    fn test_convert_tools() {
        let tools = vec![OpenAiTool {
            tool_type: "function".to_string(),
            function: OpenAiFunction {
                name: "get_weather".to_string(),
                description: Some("Get weather for a location".to_string()),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "location": { "type": "string" }
                    },
                    "required": ["location"]
                })),
            },
        }];

        let converted = convert_tools(&tools);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].name, "get_weather");
        assert_eq!(converted[0].description, "Get weather for a location");
    }

    #[test]
    fn test_convert_tool_calls_to_openai() {
        let calls = vec![ToolCall {
            id: "call_abc".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "rust"}),
        }];

        let converted = convert_tool_calls_to_openai(&calls);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].id, "call_abc");
        assert_eq!(converted[0].call_type, "function");
        assert_eq!(converted[0].function.name, "search");
        assert!(converted[0].function.arguments.contains("rust"));
    }

    #[test]
    fn test_normalize_tool_choice() {
        // String variant
        let v = serde_json::json!("auto");
        assert_eq!(normalize_tool_choice(&v), Some("auto".to_string()));

        // Object with function
        let v = serde_json::json!({"type": "function", "function": {"name": "foo"}});
        assert_eq!(normalize_tool_choice(&v), Some("required".to_string()));

        // Object with type only
        let v = serde_json::json!({"type": "none"});
        assert_eq!(normalize_tool_choice(&v), Some("none".to_string()));

        // Null
        let v = serde_json::Value::Null;
        assert_eq!(normalize_tool_choice(&v), None);
    }

    #[test]
    fn test_openai_request_deserialize_minimal() {
        let json = r#"{"model":"gpt-4","messages":[{"role":"user","content":"Hi"}]}"#;
        let req: OpenAiChatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.model, "gpt-4");
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.stream, None);
        assert_eq!(req.temperature, None);
    }

    #[test]
    fn test_openai_request_deserialize_streaming() {
        let json = r#"{"model":"gpt-4","messages":[{"role":"user","content":"Hi"}],"stream":true,"temperature":0.7}"#;
        let req: OpenAiChatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.stream, Some(true));
        assert_eq!(req.temperature, Some(0.7));
    }

    #[test]
    fn test_openai_response_serialize() {
        let resp = OpenAiChatResponse {
            id: "chatcmpl-test".to_string(),
            object: "chat.completion",
            created: 1234567890,
            model: "test-model".to_string(),
            choices: vec![OpenAiChoice {
                index: 0,
                message: OpenAiMessage {
                    role: "assistant".to_string(),
                    content: Some("Hello!".to_string()),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
                finish_reason: "stop".to_string(),
            }],
            usage: OpenAiUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            },
        };

        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["object"], "chat.completion");
        assert_eq!(json["choices"][0]["finish_reason"], "stop");
        assert_eq!(json["choices"][0]["message"]["content"], "Hello!");
        assert_eq!(json["usage"]["total_tokens"], 15);
    }

    #[test]
    fn test_openai_message_with_null_content() {
        let json = r#"{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"search","arguments":"{\"q\":\"test\"}"}}]}"#;
        let msg: OpenAiMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.role, "assistant");
        assert!(msg.content.is_none());
        assert!(msg.tool_calls.is_some());
        assert_eq!(msg.tool_calls.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_parse_stop_string() {
        let v = serde_json::json!("STOP");
        assert_eq!(parse_stop(&v), Some(vec!["STOP".to_string()]));
    }

    #[test]
    fn test_parse_stop_array() {
        let v = serde_json::json!(["STOP", "END"]);
        assert_eq!(
            parse_stop(&v),
            Some(vec!["STOP".to_string(), "END".to_string()])
        );
    }

    #[test]
    fn test_parse_stop_null() {
        let v = serde_json::Value::Null;
        assert_eq!(parse_stop(&v), None);
    }
}
