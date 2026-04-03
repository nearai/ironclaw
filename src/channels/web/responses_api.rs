//! OpenAI Responses API (`POST /v1/responses`, `GET /v1/responses/{id}`).
//!
//! Unlike the Chat Completions proxy (`openai_compat.rs`) which is a raw LLM
//! passthrough, this module routes requests through the full agent loop —
//! giving callers access to tools, memory, safety, and server-side
//! conversation state via a standard OpenAI-compatible interface.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::Stream;
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;
use uuid::Uuid;

use crate::channels::IncomingMessage;
use crate::channels::web::types::AppEvent;

use super::server::GatewayState;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum time to wait for the agent to finish a turn (non-streaming).
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);

/// Prefix for response IDs.
const RESP_PREFIX: &str = "resp_";

/// Length of a UUID in simple (no-hyphen) hex form.
const UUID_HEX_LEN: usize = 32;

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ResponsesRequest {
    #[serde(default = "default_model")]
    pub model: String,
    pub input: ResponsesInput,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub previous_response_id: Option<String>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub tools: Option<Vec<ResponsesTool>>,
    #[serde(default)]
    pub tool_choice: Option<serde_json::Value>,
}

fn default_model() -> String {
    "default".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ResponsesInput {
    Text(String),
    Messages(Vec<ResponsesInputMessage>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResponsesInputMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct ResponsesTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub parameters: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ResponseObject {
    pub id: String,
    pub object: &'static str,
    pub created_at: i64,
    pub model: String,
    pub status: ResponseStatus,
    pub output: Vec<ResponseOutputItem>,
    pub usage: ResponseUsage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponseError {
    pub message: String,
    pub code: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum ResponseOutputItem {
    #[serde(rename = "message")]
    Message {
        id: String,
        role: String,
        content: Vec<MessageContent>,
    },
    #[serde(rename = "function_call")]
    FunctionCall {
        id: String,
        call_id: String,
        name: String,
        arguments: String,
    },
    #[serde(rename = "function_call_output")]
    FunctionCallOutput {
        id: String,
        call_id: String,
        output: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum MessageContent {
    #[serde(rename = "output_text")]
    OutputText { text: String },
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ResponseUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

// ---------------------------------------------------------------------------
// Streaming event types
// ---------------------------------------------------------------------------

/// Server-sent events emitted during a streaming response.
///
/// Each variant serialises with `"type": "response.xxx"` matching the OpenAI
/// Responses API wire format.
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum ResponseStreamEvent {
    #[serde(rename = "response.created")]
    ResponseCreated { response: ResponseObject },

    #[serde(rename = "response.in_progress")]
    ResponseInProgress { response: ResponseObject },

    #[serde(rename = "response.output_item.added")]
    OutputItemAdded {
        output_index: usize,
        item: ResponseOutputItem,
    },

    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta {
        output_index: usize,
        content_index: usize,
        delta: String,
    },

    #[serde(rename = "response.output_item.done")]
    OutputItemDone {
        output_index: usize,
        item: ResponseOutputItem,
    },

    #[serde(rename = "response.completed")]
    ResponseCompleted { response: ResponseObject },

    #[serde(rename = "response.failed")]
    ResponseFailed { response: ResponseObject },
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ResponsesApiError {
    pub error: ResponsesApiErrorDetail,
}

#[derive(Debug, Serialize)]
pub struct ResponsesApiErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

type ApiError = (StatusCode, Json<ResponsesApiError>);

fn api_error(status: StatusCode, message: impl Into<String>, error_type: &str) -> ApiError {
    (
        status,
        Json(ResponsesApiError {
            error: ResponsesApiErrorDetail {
                message: message.into(),
                error_type: error_type.to_string(),
                code: None,
            },
        }),
    )
}

// ---------------------------------------------------------------------------
// ID encoding/decoding
// ---------------------------------------------------------------------------

/// Encode a response ID: `resp_{response_uuid_hex}{thread_uuid_hex}`.
///
/// Each POST generates a unique `response_uuid` so that response IDs differ
/// across turns even when the underlying thread (conversation) is the same.
fn encode_response_id(response_uuid: &Uuid, thread_uuid: &Uuid) -> String {
    format!(
        "{}{}{}",
        RESP_PREFIX,
        response_uuid.simple(),
        thread_uuid.simple()
    )
}

/// Decode a response ID back to `(response_uuid, thread_uuid)`.
fn decode_response_id(id: &str) -> Result<(Uuid, Uuid), String> {
    let hex = id
        .strip_prefix(RESP_PREFIX)
        .ok_or_else(|| format!("response ID must start with '{RESP_PREFIX}'"))?;
    if hex.len() != UUID_HEX_LEN * 2 {
        return Err(format!(
            "response ID must contain exactly {} hex characters after prefix",
            UUID_HEX_LEN * 2
        ));
    }
    let (resp_hex, thread_hex) = hex.split_at(UUID_HEX_LEN);
    let response_uuid =
        Uuid::parse_str(resp_hex).map_err(|e| format!("invalid response UUID: {e}"))?;
    let thread_uuid =
        Uuid::parse_str(thread_hex).map_err(|e| format!("invalid thread UUID: {e}"))?;
    Ok((response_uuid, thread_uuid))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn unix_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn make_item_id() -> String {
    format!("item_{}", Uuid::new_v4().simple())
}

/// Extract the user message text from the input.
fn extract_user_content(input: &ResponsesInput) -> Result<String, String> {
    match input {
        ResponsesInput::Text(s) => {
            if s.is_empty() {
                Err("input must not be empty".to_string())
            } else {
                Ok(s.clone())
            }
        }
        ResponsesInput::Messages(msgs) => {
            // Find the last user message.
            let last_user = msgs
                .iter()
                .rev()
                .find(|m| m.role == "user")
                .ok_or("input messages must contain at least one user message")?;
            if last_user.content.is_empty() {
                Err("user message content must not be empty".to_string())
            } else {
                Ok(last_user.content.clone())
            }
        }
    }
}

/// Check whether an `AppEvent` belongs to the target thread.
fn event_matches_thread(event: &AppEvent, target: &str) -> bool {
    match event {
        AppEvent::Response { thread_id, .. } => thread_id == target,
        AppEvent::StreamChunk { thread_id, .. }
        | AppEvent::Thinking { thread_id, .. }
        | AppEvent::ToolStarted { thread_id, .. }
        | AppEvent::ToolCompleted { thread_id, .. }
        | AppEvent::ToolResult { thread_id, .. }
        | AppEvent::Error { thread_id, .. }
        | AppEvent::TurnCost { thread_id, .. }
        | AppEvent::ImageGenerated { thread_id, .. }
        | AppEvent::Suggestions { thread_id, .. }
        | AppEvent::ReasoningUpdate { thread_id, .. }
        | AppEvent::Status { thread_id, .. }
        | AppEvent::ApprovalNeeded { thread_id, .. }
        | AppEvent::GateRequired { thread_id, .. }
        | AppEvent::GateResolved { thread_id, .. } => thread_id.as_deref() == Some(target),
        // Global or job-scoped events are never matched.
        _ => false,
    }
}

/// Build an empty in-progress response shell.
fn in_progress_response(resp_id: &str, model: &str) -> ResponseObject {
    ResponseObject {
        id: resp_id.to_string(),
        object: "response",
        created_at: unix_timestamp(),
        model: model.to_string(),
        status: ResponseStatus::InProgress,
        output: Vec::new(),
        usage: ResponseUsage::default(),
        error: None,
    }
}

/// Send an `IncomingMessage` to the agent loop, returning an error response on
/// failure.
async fn send_to_agent(state: &GatewayState, msg: IncomingMessage) -> Result<(), ApiError> {
    let tx = {
        let guard = state.msg_tx.read().await;
        guard.as_ref().cloned().ok_or_else(|| {
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Agent loop not started",
                "server_error",
            )
        })?
    };
    tx.send(msg).await.map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Agent loop channel closed",
            "server_error",
        )
    })
}

// ---------------------------------------------------------------------------
// Non-streaming: collect AppEvents into a ResponseObject
// ---------------------------------------------------------------------------

/// Accumulator for building a `ResponseObject` from a stream of `AppEvent`s.
struct ResponseAccumulator {
    resp_id: String,
    model: String,
    created_at: i64,
    output: Vec<ResponseOutputItem>,
    text_chunks: Vec<String>,
    usage: ResponseUsage,
    failed: bool,
    error_message: Option<String>,
}

impl ResponseAccumulator {
    fn new(resp_id: String, model: String) -> Self {
        Self {
            resp_id,
            model,
            created_at: unix_timestamp(),
            output: Vec::new(),
            text_chunks: Vec::new(),
            usage: ResponseUsage::default(),
            failed: false,
            error_message: None,
        }
    }

    /// Process one `AppEvent` and return `true` if the turn is finished.
    fn process(&mut self, event: AppEvent) -> bool {
        match event {
            AppEvent::StreamChunk { content, .. } => {
                self.text_chunks.push(content);
                false
            }
            AppEvent::Response { content, .. } => {
                // Final response text supersedes any stream chunks.
                let text = if content.is_empty() {
                    self.text_chunks.join("")
                } else {
                    content
                };
                if !text.is_empty() {
                    self.output.push(ResponseOutputItem::Message {
                        id: make_item_id(),
                        role: "assistant".to_string(),
                        content: vec![MessageContent::OutputText { text }],
                    });
                }
                true // turn complete
            }
            AppEvent::ToolStarted { name, .. } => {
                // Emit function_call placeholder — arguments filled on ToolCompleted.
                let call_id = format!("call_{}", Uuid::new_v4().simple());
                self.output.push(ResponseOutputItem::FunctionCall {
                    id: make_item_id(),
                    call_id,
                    name,
                    arguments: String::new(),
                });
                false
            }
            AppEvent::ToolCompleted {
                name,
                success,
                error,
                parameters,
                ..
            } => {
                // Try to attach arguments to the matching FunctionCall.
                if let Some(args) = parameters {
                    for item in self.output.iter_mut().rev() {
                        if let ResponseOutputItem::FunctionCall {
                            name: n,
                            arguments: a,
                            ..
                        } = item
                            && *n == name
                            && a.is_empty()
                        {
                            *a = args;
                            break;
                        }
                    }
                }
                // On failure, record a FunctionCallOutput with the error.
                if !success && let Some(err) = error {
                    let call_id = self.last_call_id_for(&name);
                    self.output.push(ResponseOutputItem::FunctionCallOutput {
                        id: make_item_id(),
                        call_id,
                        output: format!("Error: {err}"),
                    });
                }
                false
            }
            AppEvent::ToolResult { name, preview, .. } => {
                let call_id = self.last_call_id_for(&name);
                self.output.push(ResponseOutputItem::FunctionCallOutput {
                    id: make_item_id(),
                    call_id,
                    output: preview,
                });
                false
            }
            AppEvent::TurnCost {
                input_tokens,
                output_tokens,
                ..
            } => {
                self.usage = ResponseUsage {
                    input_tokens,
                    output_tokens,
                    total_tokens: input_tokens + output_tokens,
                };
                false
            }
            AppEvent::Error { message, .. } => {
                self.failed = true;
                self.error_message = Some(message);
                true // turn complete (failed)
            }
            AppEvent::ApprovalNeeded {
                tool_name,
                parameters,
                ..
            } => {
                self.output.push(ResponseOutputItem::FunctionCall {
                    id: make_item_id(),
                    call_id: format!("call_{}", Uuid::new_v4().simple()),
                    name: tool_name.clone(),
                    arguments: parameters,
                });
                self.failed = true;
                self.error_message = Some(format!(
                    "Tool '{tool_name}' requires approval which is not supported via the Responses API"
                ));
                true
            }
            // Ignore events we don't map (Thinking, Status, etc.).
            _ => false,
        }
    }

    /// Find the `call_id` of the most recent `FunctionCall` for a given tool name.
    fn last_call_id_for(&self, name: &str) -> String {
        self.output
            .iter()
            .rev()
            .find_map(|item| match item {
                ResponseOutputItem::FunctionCall {
                    call_id, name: n, ..
                } if n == name => Some(call_id.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn finish(self) -> ResponseObject {
        ResponseObject {
            id: self.resp_id,
            object: "response",
            created_at: self.created_at,
            model: self.model,
            status: if self.failed {
                ResponseStatus::Failed
            } else {
                ResponseStatus::Completed
            },
            output: self.output,
            usage: self.usage,
            error: self.error_message.map(|msg| ResponseError {
                message: msg,
                code: None,
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub async fn create_response_handler(
    State(state): State<Arc<GatewayState>>,
    super::auth::AuthenticatedUser(user): super::auth::AuthenticatedUser,
    Json(req): Json<ResponsesRequest>,
) -> Result<Response, ApiError> {
    if !state.chat_rate_limiter.check(&user.user_id) {
        return Err(api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded. Please try again later.",
            "rate_limit_error",
        ));
    }

    // Reject fields that are accepted but not yet wired into the agent loop.
    if req.model != "default" {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Model selection is not yet supported; omit 'model' or use \"default\"",
            "invalid_request_error",
        ));
    }
    if req.instructions.is_some() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "The 'instructions' field is not yet supported",
            "invalid_request_error",
        ));
    }
    if req.tools.is_some() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "The 'tools' field is not yet supported",
            "invalid_request_error",
        ));
    }
    if req.tool_choice.is_some() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "The 'tool_choice' field is not yet supported",
            "invalid_request_error",
        ));
    }
    if req.temperature.is_some() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "The 'temperature' field is not yet supported",
            "invalid_request_error",
        ));
    }
    if req.max_output_tokens.is_some() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "The 'max_output_tokens' field is not yet supported",
            "invalid_request_error",
        ));
    }

    let content = extract_user_content(&req.input)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e, "invalid_request_error"))?;

    // Resolve or create thread.
    let thread_uuid = match &req.previous_response_id {
        Some(prev_id) => {
            let (_prev_resp, thread) = decode_response_id(prev_id)
                .map_err(|e| api_error(StatusCode::BAD_REQUEST, e, "invalid_request_error"))?;
            thread
        }
        None => Uuid::new_v4(),
    };
    let thread_id_str = thread_uuid.to_string();

    // Each POST gets its own unique response UUID.
    let response_uuid = Uuid::new_v4();

    // Build the message for the agent loop.
    let msg = IncomingMessage::new("gateway", &user.user_id, &content)
        .with_thread(&thread_id_str)
        .with_metadata(serde_json::json!({
            "thread_id": &thread_id_str,
            "user_id": &user.user_id,
            "source": "responses_api",
        }));

    let resp_id = encode_response_id(&response_uuid, &thread_uuid);
    let model = req.model.clone();
    let stream = req.stream.unwrap_or(false);
    let user_id = user.user_id.clone();

    if stream {
        handle_streaming(state, msg, resp_id, model, thread_id_str, user_id)
            .await
            .map(IntoResponse::into_response)
    } else {
        handle_non_streaming(state, msg, resp_id, model, thread_id_str, &user_id)
            .await
            .map(IntoResponse::into_response)
    }
}

async fn handle_non_streaming(
    state: Arc<GatewayState>,
    msg: IncomingMessage,
    resp_id: String,
    model: String,
    thread_id: String,
    user_id: &str,
) -> Result<Json<ResponseObject>, ApiError> {
    // Subscribe BEFORE sending so we don't miss events.
    let mut event_stream = state
        .sse
        .subscribe_raw(Some(user_id.to_string()), false)
        .ok_or_else(|| {
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Too many concurrent connections",
                "server_error",
            )
        })?;

    send_to_agent(&state, msg).await?;

    let mut acc = ResponseAccumulator::new(resp_id, model);

    let result = tokio::time::timeout(RESPONSE_TIMEOUT, async {
        while let Some(event) = event_stream.next().await {
            if !event_matches_thread(&event, &thread_id) {
                continue;
            }
            if acc.process(event) {
                break;
            }
        }
    })
    .await;

    if result.is_err() {
        acc.failed = true;
        acc.error_message = Some("Response timed out".to_string());
    }

    Ok(Json(acc.finish()))
}

async fn handle_streaming(
    state: Arc<GatewayState>,
    msg: IncomingMessage,
    resp_id: String,
    model: String,
    thread_id: String,
    user_id: String,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>> + Send>, ApiError> {
    let event_stream = state
        .sse
        .subscribe_raw(Some(user_id), false)
        .ok_or_else(|| {
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Too many concurrent connections",
                "server_error",
            )
        })?;

    send_to_agent(&state, msg).await?;

    // Use a channel to bridge the spawned task and the SSE stream.
    let (tx, rx) = tokio::sync::mpsc::channel::<Event>(64);

    tokio::spawn(streaming_worker(
        tx,
        event_stream,
        resp_id,
        model,
        thread_id,
    ));

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(Ok::<_, Infallible>);

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)).text("")))
}

/// Background task that reads `AppEvent`s and sends SSE `Event`s to the client.
async fn streaming_worker(
    tx: tokio::sync::mpsc::Sender<Event>,
    event_stream: impl Stream<Item = AppEvent> + Send + Unpin,
    resp_id: String,
    model: String,
    thread_id: String,
) {
    use std::pin::pin;

    fn sse_event(evt_type: &str, data: &str) -> Event {
        Event::default().event(evt_type).data(data)
    }

    fn emit(
        tx: &tokio::sync::mpsc::Sender<Event>,
        evt_type: &str,
        payload: &impl Serialize,
    ) -> bool {
        if let Ok(data) = serde_json::to_string(payload) {
            tx.try_send(sse_event(evt_type, &data)).is_ok()
        } else {
            true // serialization failure is non-fatal; keep going
        }
    }

    // Emit response.created
    let initial = in_progress_response(&resp_id, &model);
    if !emit(
        &tx,
        "response.created",
        &ResponseStreamEvent::ResponseCreated { response: initial },
    ) {
        return;
    }

    let mut acc = ResponseAccumulator::new(resp_id, model);
    let mut message_output_index: Option<usize> = None;
    let mut current_tool_index: Option<usize> = None;

    let mut event_stream = pin!(event_stream);
    let timeout = tokio::time::sleep(RESPONSE_TIMEOUT);
    tokio::pin!(timeout);

    loop {
        let event = tokio::select! {
            biased;
            ev = event_stream.next() => match ev {
                Some(e) => e,
                None => break,
            },
            () = &mut timeout => {
                acc.failed = true;
                let resp = acc.finish();
                let _ = emit(&tx, "response.failed", &ResponseStreamEvent::ResponseFailed { response: resp });
                return;
            }
        };

        if !event_matches_thread(&event, &thread_id) {
            continue;
        }

        match &event {
            AppEvent::StreamChunk { content, .. } => {
                let idx = match message_output_index {
                    Some(i) => i,
                    None => {
                        let i = acc.output.len();
                        let item = ResponseOutputItem::Message {
                            id: make_item_id(),
                            role: "assistant".to_string(),
                            content: vec![MessageContent::OutputText {
                                text: String::new(),
                            }],
                        };
                        emit(
                            &tx,
                            "response.output_item.added",
                            &ResponseStreamEvent::OutputItemAdded {
                                output_index: i,
                                item: item.clone(),
                            },
                        );
                        acc.output.push(item);
                        message_output_index = Some(i);
                        i
                    }
                };
                emit(
                    &tx,
                    "response.output_text.delta",
                    &ResponseStreamEvent::OutputTextDelta {
                        output_index: idx,
                        content_index: 0,
                        delta: content.clone(),
                    },
                );
                acc.text_chunks.push(content.clone());
            }
            AppEvent::ToolStarted { name, .. } => {
                let idx = acc.output.len();
                let call_id = format!("call_{}", Uuid::new_v4().simple());
                let item = ResponseOutputItem::FunctionCall {
                    id: make_item_id(),
                    call_id,
                    name: name.clone(),
                    arguments: String::new(),
                };
                emit(
                    &tx,
                    "response.output_item.added",
                    &ResponseStreamEvent::OutputItemAdded {
                        output_index: idx,
                        item: item.clone(),
                    },
                );
                acc.output.push(item);
                current_tool_index = Some(idx);
            }
            AppEvent::ToolCompleted {
                name,
                success,
                error,
                parameters,
                ..
            } => {
                if let Some(args) = parameters {
                    for item in acc.output.iter_mut().rev() {
                        if let ResponseOutputItem::FunctionCall {
                            name: n,
                            arguments: a,
                            ..
                        } = item
                            && *n == *name
                            && a.is_empty()
                        {
                            *a = args.clone();
                            break;
                        }
                    }
                }
                if let Some(idx) = current_tool_index.take()
                    && let Some(item) = acc.output.get(idx)
                {
                    emit(
                        &tx,
                        "response.output_item.done",
                        &ResponseStreamEvent::OutputItemDone {
                            output_index: idx,
                            item: item.clone(),
                        },
                    );
                }
                // On failure, emit a FunctionCallOutput with the error.
                if !*success && let Some(err) = error {
                    let call_id = acc.last_call_id_for(name);
                    let idx = acc.output.len();
                    let item = ResponseOutputItem::FunctionCallOutput {
                        id: make_item_id(),
                        call_id,
                        output: format!("Error: {err}"),
                    };
                    emit(
                        &tx,
                        "response.output_item.added",
                        &ResponseStreamEvent::OutputItemAdded {
                            output_index: idx,
                            item: item.clone(),
                        },
                    );
                    emit(
                        &tx,
                        "response.output_item.done",
                        &ResponseStreamEvent::OutputItemDone {
                            output_index: idx,
                            item: item.clone(),
                        },
                    );
                    acc.output.push(item);
                }
            }
            AppEvent::ToolResult { name, preview, .. } => {
                let call_id = acc.last_call_id_for(name);
                let idx = acc.output.len();
                let item = ResponseOutputItem::FunctionCallOutput {
                    id: make_item_id(),
                    call_id,
                    output: preview.clone(),
                };
                emit(
                    &tx,
                    "response.output_item.added",
                    &ResponseStreamEvent::OutputItemAdded {
                        output_index: idx,
                        item: item.clone(),
                    },
                );
                emit(
                    &tx,
                    "response.output_item.done",
                    &ResponseStreamEvent::OutputItemDone {
                        output_index: idx,
                        item: item.clone(),
                    },
                );
                acc.output.push(item);
            }
            AppEvent::TurnCost {
                input_tokens,
                output_tokens,
                ..
            } => {
                acc.usage = ResponseUsage {
                    input_tokens: *input_tokens,
                    output_tokens: *output_tokens,
                    total_tokens: input_tokens + output_tokens,
                };
            }
            _ => {}
        }

        // Terminal events.
        let is_terminal = matches!(
            &event,
            AppEvent::Response { .. } | AppEvent::Error { .. } | AppEvent::ApprovalNeeded { .. }
        );

        if is_terminal {
            if let AppEvent::Response { content, .. } = &event {
                let text = if content.is_empty() {
                    acc.text_chunks.join("")
                } else {
                    content.clone()
                };
                if !text.is_empty() {
                    match message_output_index {
                        Some(idx) => {
                            acc.output[idx] = ResponseOutputItem::Message {
                                id: make_item_id(),
                                role: "assistant".to_string(),
                                content: vec![MessageContent::OutputText { text }],
                            };
                            if let Some(item) = acc.output.get(idx) {
                                emit(
                                    &tx,
                                    "response.output_item.done",
                                    &ResponseStreamEvent::OutputItemDone {
                                        output_index: idx,
                                        item: item.clone(),
                                    },
                                );
                            }
                        }
                        None => {
                            let idx = acc.output.len();
                            let item = ResponseOutputItem::Message {
                                id: make_item_id(),
                                role: "assistant".to_string(),
                                content: vec![MessageContent::OutputText { text }],
                            };
                            emit(
                                &tx,
                                "response.output_item.added",
                                &ResponseStreamEvent::OutputItemAdded {
                                    output_index: idx,
                                    item: item.clone(),
                                },
                            );
                            emit(
                                &tx,
                                "response.output_item.done",
                                &ResponseStreamEvent::OutputItemDone {
                                    output_index: idx,
                                    item: item.clone(),
                                },
                            );
                            acc.output.push(item);
                        }
                    }
                }
            }

            if matches!(
                &event,
                AppEvent::Error { .. } | AppEvent::ApprovalNeeded { .. }
            ) {
                acc.process(event);
            }

            let resp = acc.finish();
            let (evt_type, evt) = if resp.status == ResponseStatus::Failed {
                (
                    "response.failed",
                    ResponseStreamEvent::ResponseFailed { response: resp },
                )
            } else {
                (
                    "response.completed",
                    ResponseStreamEvent::ResponseCompleted { response: resp },
                )
            };
            let _ = emit(&tx, evt_type, &evt);
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// GET /v1/responses/{id}
// ---------------------------------------------------------------------------

pub async fn get_response_handler(
    State(state): State<Arc<GatewayState>>,
    super::auth::AuthenticatedUser(user): super::auth::AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<ResponseObject>, ApiError> {
    let (_response_uuid, thread_uuid) = decode_response_id(&id)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e, "invalid_request_error"))?;

    let store = state.store.as_ref().ok_or_else(|| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Database not configured",
            "server_error",
        )
    })?;

    // Verify the authenticated user owns this conversation.
    let owns = store
        .conversation_belongs_to_user(thread_uuid, &user.user_id)
        .await
        .map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to verify ownership: {e}"),
                "server_error",
            )
        })?;
    if !owns {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            format!("Response '{id}' not found"),
            "invalid_request_error",
        ));
    }

    // Load messages for this conversation.
    let messages = store
        .list_conversation_messages(thread_uuid)
        .await
        .map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to load conversation: {e}"),
                "server_error",
            )
        })?;

    if messages.is_empty() {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            format!("Response '{id}' not found"),
            "invalid_request_error",
        ));
    }

    // Reconstruct output items from stored messages.
    let mut output = Vec::new();
    for msg in &messages {
        match msg.role.as_str() {
            "assistant" => {
                if !msg.content.is_empty() {
                    output.push(ResponseOutputItem::Message {
                        id: format!("msg_{}", msg.id.simple()),
                        role: "assistant".to_string(),
                        content: vec![MessageContent::OutputText {
                            text: msg.content.clone(),
                        }],
                    });
                }
            }
            "tool_calls" => {
                // Tool calls may be stored as a plain JSON array (legacy) or
                // as an object wrapper: `{ "calls": [...], "narrative": "..." }`.
                let calls = match serde_json::from_str::<serde_json::Value>(&msg.content) {
                    Ok(serde_json::Value::Array(arr)) => arr,
                    Ok(serde_json::Value::Object(ref obj)) => obj
                        .get("calls")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default(),
                    _ => Vec::new(),
                };
                for call in &calls {
                    let name = call
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    // Prefer `call_id`, fall back to `tool_call_id`, then `id`.
                    let call_id = call
                        .get("call_id")
                        .or_else(|| call.get("tool_call_id"))
                        .or_else(|| call.get("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let arguments = call
                        .get("parameters")
                        .or_else(|| call.get("arguments"))
                        .map(|v| {
                            if v.is_string() {
                                v.as_str().unwrap_or("{}").to_string()
                            } else {
                                serde_json::to_string(v).unwrap_or_default()
                            }
                        })
                        .unwrap_or_default();
                    output.push(ResponseOutputItem::FunctionCall {
                        id: make_item_id(),
                        call_id: call_id.clone(),
                        name,
                        arguments,
                    });
                    // If there's an inline result, emit a FunctionCallOutput too.
                    if let Some(result) = call
                        .get("result_preview")
                        .or_else(|| call.get("result"))
                        .and_then(|v| v.as_str())
                    {
                        output.push(ResponseOutputItem::FunctionCallOutput {
                            id: make_item_id(),
                            call_id,
                            output: result.to_string(),
                        });
                    }
                }
            }
            "tool" => {
                // Tool results — try to correlate with the preceding FunctionCall.
                let call_id = output
                    .iter()
                    .rev()
                    .find_map(|item| match item {
                        ResponseOutputItem::FunctionCall { call_id, .. } => Some(call_id.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                output.push(ResponseOutputItem::FunctionCallOutput {
                    id: make_item_id(),
                    call_id,
                    output: msg.content.clone(),
                });
            }
            _ => {} // Skip user/system messages (they are input, not output).
        }
    }

    Ok(Json(ResponseObject {
        id,
        object: "response",
        created_at: messages
            .first()
            .map(|m| m.created_at.timestamp())
            .unwrap_or_else(unix_timestamp),
        model: "default".to_string(),
        status: ResponseStatus::Completed,
        output,
        usage: ResponseUsage::default(), // Token usage is not persisted per-message.
        error: None,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_id_round_trip() {
        let resp_uuid = Uuid::new_v4();
        let thread_uuid = Uuid::new_v4();
        let encoded = encode_response_id(&resp_uuid, &thread_uuid);
        assert!(encoded.starts_with(RESP_PREFIX));
        let (decoded_resp, decoded_thread) = decode_response_id(&encoded).expect("should decode");
        assert_eq!(resp_uuid, decoded_resp);
        assert_eq!(thread_uuid, decoded_thread);
    }

    #[test]
    fn response_ids_differ_across_turns() {
        let thread_uuid = Uuid::new_v4();
        let id1 = encode_response_id(&Uuid::new_v4(), &thread_uuid);
        let id2 = encode_response_id(&Uuid::new_v4(), &thread_uuid);
        assert_ne!(id1, id2, "each turn must produce a distinct response ID");
    }

    #[test]
    fn decode_response_id_rejects_bad_prefix() {
        assert!(decode_response_id("bad_prefix").is_err());
    }

    #[test]
    fn decode_response_id_rejects_bad_uuid() {
        assert!(decode_response_id("resp_not_a_uuid").is_err());
    }

    #[test]
    fn extract_user_content_text() {
        let input = ResponsesInput::Text("hello".to_string());
        assert_eq!(extract_user_content(&input).unwrap(), "hello");
    }

    #[test]
    fn extract_user_content_empty_text_errors() {
        let input = ResponsesInput::Text(String::new());
        assert!(extract_user_content(&input).is_err());
    }

    #[test]
    fn extract_user_content_messages_uses_last_user() {
        let input = ResponsesInput::Messages(vec![
            ResponsesInputMessage {
                role: "user".to_string(),
                content: "first".to_string(),
            },
            ResponsesInputMessage {
                role: "assistant".to_string(),
                content: "middle".to_string(),
            },
            ResponsesInputMessage {
                role: "user".to_string(),
                content: "last".to_string(),
            },
        ]);
        assert_eq!(extract_user_content(&input).unwrap(), "last");
    }

    #[test]
    fn extract_user_content_no_user_message_errors() {
        let input = ResponsesInput::Messages(vec![ResponsesInputMessage {
            role: "system".to_string(),
            content: "hello".to_string(),
        }]);
        assert!(extract_user_content(&input).is_err());
    }

    #[test]
    fn event_matches_thread_filters_correctly() {
        let target = "abc-123";
        let matching = AppEvent::Response {
            content: "hi".to_string(),
            thread_id: "abc-123".to_string(),
        };
        assert!(event_matches_thread(&matching, target));

        let non_matching = AppEvent::Response {
            content: "hi".to_string(),
            thread_id: "other".to_string(),
        };
        assert!(!event_matches_thread(&non_matching, target));

        let global = AppEvent::Heartbeat;
        assert!(!event_matches_thread(&global, target));
    }

    #[test]
    fn accumulator_basic_response() {
        let mut acc = ResponseAccumulator::new("resp_test".to_string(), "m".to_string());
        let done = acc.process(AppEvent::Response {
            content: "Hello world".to_string(),
            thread_id: "t".to_string(),
        });
        assert!(done);
        let resp = acc.finish();
        assert_eq!(resp.status, ResponseStatus::Completed);
        assert_eq!(resp.output.len(), 1);
        match &resp.output[0] {
            ResponseOutputItem::Message { content, .. } => {
                assert!(
                    matches!(&content[0], MessageContent::OutputText { text } if text == "Hello world")
                );
            }
            _ => panic!("expected Message output item"),
        }
    }

    #[test]
    fn accumulator_stream_chunks_then_response() {
        let mut acc = ResponseAccumulator::new("resp_test".to_string(), "m".to_string());
        assert!(!acc.process(AppEvent::StreamChunk {
            content: "Hello ".to_string(),
            thread_id: Some("t".to_string()),
        }));
        assert!(!acc.process(AppEvent::StreamChunk {
            content: "world".to_string(),
            thread_id: Some("t".to_string()),
        }));
        // Empty response content → accumulator falls back to chunks.
        assert!(acc.process(AppEvent::Response {
            content: String::new(),
            thread_id: "t".to_string(),
        }));
        let resp = acc.finish();
        match &resp.output[0] {
            ResponseOutputItem::Message { content, .. } => {
                assert!(
                    matches!(&content[0], MessageContent::OutputText { text } if text == "Hello world")
                );
            }
            _ => panic!("expected Message output item"),
        }
    }

    #[test]
    fn accumulator_tool_flow() {
        let mut acc = ResponseAccumulator::new("resp_test".to_string(), "m".to_string());
        assert!(!acc.process(AppEvent::ToolStarted {
            name: "memory_search".to_string(),
            thread_id: Some("t".to_string()),
        }));
        assert!(!acc.process(AppEvent::ToolResult {
            name: "memory_search".to_string(),
            preview: "found 3 results".to_string(),
            thread_id: Some("t".to_string()),
        }));
        assert!(acc.process(AppEvent::Response {
            content: "Here are your results.".to_string(),
            thread_id: "t".to_string(),
        }));
        let resp = acc.finish();
        // FunctionCall + FunctionCallOutput + Message = 3 items
        assert_eq!(resp.output.len(), 3);
        assert!(
            matches!(&resp.output[0], ResponseOutputItem::FunctionCall { name, .. } if name == "memory_search")
        );
        assert!(
            matches!(&resp.output[1], ResponseOutputItem::FunctionCallOutput { output, .. } if output == "found 3 results")
        );
        assert!(matches!(
            &resp.output[2],
            ResponseOutputItem::Message { .. }
        ));
    }

    #[test]
    fn accumulator_error_marks_failed() {
        let mut acc = ResponseAccumulator::new("resp_test".to_string(), "m".to_string());
        assert!(acc.process(AppEvent::Error {
            message: "something broke".to_string(),
            thread_id: Some("t".to_string()),
        }));
        let resp = acc.finish();
        assert_eq!(resp.status, ResponseStatus::Failed);
    }

    #[test]
    fn accumulator_approval_needed_marks_failed() {
        let mut acc = ResponseAccumulator::new("resp_test".to_string(), "m".to_string());
        assert!(acc.process(AppEvent::ApprovalNeeded {
            request_id: "r1".to_string(),
            tool_name: "shell".to_string(),
            description: "run ls".to_string(),
            parameters: "{}".to_string(),
            thread_id: Some("t".to_string()),
            allow_always: true,
        }));
        let resp = acc.finish();
        assert_eq!(resp.status, ResponseStatus::Failed);
        assert!(matches!(
            &resp.output[0],
            ResponseOutputItem::FunctionCall { name, arguments, .. }
                if name == "shell" && arguments == "{}"
        ));
    }

    #[test]
    fn response_status_serializes_as_snake_case() {
        let json = serde_json::to_string(&ResponseStatus::InProgress).expect("serialize");
        assert_eq!(json, "\"in_progress\"");
        let json = serde_json::to_string(&ResponseStatus::Completed).expect("serialize");
        assert_eq!(json, "\"completed\"");
    }
}
