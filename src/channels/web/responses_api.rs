//! OpenAI Responses API (`POST /api/v1/responses`, `GET /api/v1/responses/{id}`).
//!
//! Unlike the Chat Completions proxy (`openai_compat.rs`) which is a raw LLM
//! passthrough, this module routes requests through the full agent loop —
//! giving callers access to tools, memory, safety, and server-side
//! conversation state via a standard OpenAI-compatible interface.
//!
//! The canonical path is `/api/v1/responses` so the Responses API shares the
//! `/api/...` prefix used by the rest of IronClaw's HTTP surface. The legacy
//! `/v1/responses` path is still accepted as an alias for backward
//! compatibility with clients configured against it (see ironclaw#2201).
//!
//! ## Externally-provided tools
//!
//! Callers can declare their own tools alongside IronClaw's built-in
//! registry by passing `tools: [{type: "function", name, description,
//! parameters}]` and feeding back results via `function_call_output` items
//! in the next request's `input`. The integration is engine-native, not
//! prompt-level:
//!
//! 1. The handler validates the request, registers the caller's tools in
//!    the per-thread [`ExternalToolCatalog`] keyed by the engine
//!    `ThreadId`, and routes the user message through the agent loop. The
//!    catalog merges into the LLM-visible action surface via
//!    `EffectBridgeAdapter::available_actions`, so the model sees caller
//!    tools alongside internal ones.
//! 2. When the LLM invokes a caller tool, `EffectBridgeAdapter::execute_action`
//!    short-circuits to `EngineError::GatePaused { resume_kind:
//!    External { callback_id: ext_tool:<call_id> } }`. The bridge router
//!    projects that pause to `AppEvent::ExternalToolCall` carrying the
//!    OpenAI-shaped `function_call` fields. This handler emits it as a
//!    `function_call` `ResponseOutputItem` (both streaming
//!    `output_item.added`+`done` and non-streaming) and returns
//!    `status: "completed"`. The thread sits in `Waiting`.
//! 3. The caller resumes by POSTing a follow-up request whose `input`
//!    array contains `function_call_output` items. The handler converts
//!    them to `Submission::ExternalCallback { request_id, payload }`,
//!    routed through `bridge::handle_external_callback`, which
//!    materialises an `ActionResult` ThreadMessage from the payload and
//!    resumes the thread. The LLM sees the result on its next call.
//!
//! Caller-supplied tool names that shadow registered (built-in or
//! extension) actions are rejected at request validation with 400 — see
//! the confused-deputy note in `create_response_handler`.
//!
//! [`ExternalToolCatalog`]: crate::bridge::ExternalToolCatalog

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

use super::platform::state::GatewayState;

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
    /// IronClaw extension: structured context injected into the agent's conversation.
    ///
    /// NOT part of the OpenAI Responses API spec — IronClaw extension.
    /// The `context` alias is kept for convenience but may collide with
    /// a future OpenAI field; prefer `x_context`.
    ///
    /// Used by integrations to pass structured data (notification responses,
    /// approval status). Should be a flat `{key: {flat_object}}` structure;
    /// nested objects are serialized as raw JSON. Max 10 KB.
    #[serde(default, alias = "context")]
    pub x_context: Option<serde_json::Value>,
}

fn default_model() -> String {
    "default".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ResponsesInput {
    Text(String),
    Items(Vec<ResponsesInputItem>),
}

/// A single item in the Responses API `input` array.
///
/// Items without an explicit `type` default to a user message — this preserves
/// backward compatibility with the simpler `[{"role":"user","content":"..."}]`
/// shape that pre-dates external tool support.
#[derive(Debug, Clone, Deserialize)]
pub struct ResponsesInputItem {
    /// Item type tag: `message`, `function_call`, or `function_call_output`.
    /// Absent or empty means `message` (legacy shape).
    #[serde(rename = "type", default)]
    pub item_type: Option<String>,
    /// For `message` items: role (`user`, `assistant`, `system`).
    #[serde(default)]
    pub role: Option<String>,
    /// For `message` items: text content.
    #[serde(default)]
    pub content: Option<String>,
    /// For `function_call` and `function_call_output` items: links a call to its result.
    #[serde(default)]
    pub call_id: Option<String>,
    /// For `function_call` items: the tool name the agent (previously) chose.
    #[serde(default)]
    pub name: Option<String>,
    /// For `function_call` items: stringified JSON arguments.
    #[serde(default)]
    pub arguments: Option<String>,
    /// For `function_call_output` items: the tool result the caller executed externally.
    #[serde(default)]
    pub output: Option<String>,
}

/// Externally-provided tool definition.
///
/// Per the OpenAI Responses API spec, only `type: "function"` is currently
/// honoured. Built-in tool types like `web_search`, `file_search`, or
/// `code_interpreter` are rejected with 400 — IronClaw routes those through
/// its own internal tool registry, not through caller-provided definitions.
#[derive(Debug, Clone, Deserialize)]
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

impl ResponseUsage {
    fn add_turn_cost(&mut self, input_tokens: u64, output_tokens: u64) {
        self.input_tokens = self.input_tokens.saturating_add(input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(output_tokens);
        self.total_tokens = self.input_tokens.saturating_add(self.output_tokens);
    }
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

/// Format structured context as a human-readable prefix for the agent.
fn format_context(ctx: &serde_json::Value) -> String {
    let obj = match ctx.as_object() {
        Some(o) => o,
        None => return format!("[Context: {}]", ctx),
    };
    let mut parts = Vec::new();
    for (key, value) in obj {
        let detail = match value.as_object() {
            Some(inner) => {
                let fields: Vec<String> = inner
                    .iter()
                    .map(|(k, v)| {
                        let s = match v {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        format!("{k}: {s}")
                    })
                    .collect();
                format!("[Context: {key} \u{2014} {}]", fields.join(", "))
            }
            None => format!("[Context: {key}: {value}]"),
        };
        parts.push(detail);
    }
    parts.join("\n")
}

/// Outputs of previously-executed tools the caller is feeding back in.
#[derive(Debug, Default, Clone)]
struct ExtractedInput {
    /// The latest user message text (the prompt for this turn).
    user_text: String,
    /// Previous tool outputs (`function_call_output` items) supplied by the
    /// caller. Pairs of `(call_id, output)`. Surfaced to the agent as a
    /// preamble so it can reason over the results of the tools it asked the
    /// caller to run on the prior turn.
    tool_outputs: Vec<(String, String)>,
}

/// Extract the latest user message and any caller-supplied tool outputs from
/// the input.
///
/// Accepts:
/// - a plain string (legacy text input)
/// - a list of `{role, content}` items (legacy message input, no `type` tag)
/// - a list of typed items: `message`, `function_call`, `function_call_output`
///
/// `function_call` items are accepted but ignored — they are echoes of what
/// the agent emitted on a previous turn. `function_call_output` items are
/// extracted into `tool_outputs` so we can surface them to the agent.
fn extract_user_content(input: &ResponsesInput) -> Result<ExtractedInput, String> {
    match input {
        ResponsesInput::Text(s) => {
            if s.is_empty() {
                Err("input must not be empty".to_string())
            } else {
                Ok(ExtractedInput {
                    user_text: s.clone(),
                    tool_outputs: Vec::new(),
                })
            }
        }
        ResponsesInput::Items(items) => {
            let mut last_user_text: Option<String> = None;
            let mut tool_outputs: Vec<(String, String)> = Vec::new();
            for item in items {
                match item.item_type.as_deref().unwrap_or("message") {
                    "message" => {
                        if item.role.as_deref() == Some("user")
                            && let Some(text) = item.content.as_ref()
                            && !text.is_empty()
                        {
                            last_user_text = Some(text.clone());
                        }
                    }
                    "function_call" => {
                        // Echo of a previous turn's output. The agent's own
                        // history is reconstructed from the conversation
                        // store, so we don't need to re-inject it.
                    }
                    "function_call_output" => {
                        let Some(output) = item.output.as_ref() else {
                            return Err(
                                "function_call_output items must include an `output` field"
                                    .to_string(),
                            );
                        };
                        let call_id = item.call_id.clone().unwrap_or_default();
                        tool_outputs.push((call_id, output.clone()));
                    }
                    other => {
                        return Err(format!(
                            "unsupported input item type: '{other}' \
                             (expected 'message', 'function_call', or 'function_call_output')"
                        ));
                    }
                }
            }

            // A turn without a user message is allowed *only* when the caller
            // is exclusively delivering function_call_output items — i.e. they
            // want the agent to react to tool results without saying anything
            // new. In that case we synthesise a minimal user prompt so the
            // engine still sees a "user turn".
            let user_text = match last_user_text {
                Some(t) => t,
                None if !tool_outputs.is_empty() => {
                    "[Continue with the tool results above.]".to_string()
                }
                None => {
                    return Err(
                        "input must contain at least one user message or function_call_output"
                            .to_string(),
                    );
                }
            };

            Ok(ExtractedInput {
                user_text,
                tool_outputs,
            })
        }
    }
}

/// Maximum total serialized size of caller-supplied tool definitions (16 KiB).
///
/// Caps prompt blow-up from a misconfigured client passing hundreds of tool
/// schemas. Mirrors the existing 10 KiB cap on `x_context`.
const MAX_TOOLS_BYTES: usize = 16 * 1024;

/// Validate and normalise the externally-provided tool list.
///
/// Returns an error string on the first violation so the handler can surface a
/// 400 with a precise message. Only `type: "function"` is supported.
fn validate_external_tools(tools: &[ResponsesTool]) -> Result<(), String> {
    if tools.is_empty() {
        return Ok(());
    }
    let serialized_size = serde_json::to_string(
        &tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": t.tool_type,
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                })
            })
            .collect::<Vec<_>>(),
    )
    .map(|s| s.len())
    // Fail closed: a serialization error means we can't measure the
    // payload, so we must reject rather than wave it through. Treating
    // the unknown size as `MAX + 1` falls into the size-cap branch
    // below and surfaces a precise 400 to the caller.
    .unwrap_or(MAX_TOOLS_BYTES + 1);
    if serialized_size > MAX_TOOLS_BYTES {
        return Err(format!(
            "tools exceed {MAX_TOOLS_BYTES}-byte limit ({serialized_size} bytes)"
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for tool in tools {
        if tool.tool_type != "function" {
            return Err(format!(
                "unsupported tool type '{}'; only 'function' is accepted",
                tool.tool_type
            ));
        }
        let name = tool
            .name
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "function tool missing 'name'".to_string())?;
        if !seen.insert(name.to_string()) {
            return Err(format!("duplicate tool name '{name}'"));
        }
    }
    Ok(())
}

/// Convert caller-supplied `tools[]` into engine-native `ActionDef`s for
/// registration in the per-thread external tool catalog. Caller schemas
/// pass through unchanged: `parameters` becomes `parameters_schema`.
/// Effects are stamped as `Compute` (no externally-claimed effect type)
/// and `requires_approval` is false — caller-side gating is the
/// caller's responsibility, not the engine's.
fn responses_tools_to_action_defs(tools: &[ResponsesTool]) -> Vec<ironclaw_engine::ActionDef> {
    tools
        .iter()
        .filter_map(|t| {
            let name = t.name.clone()?;
            Some(ironclaw_engine::ActionDef {
                name,
                description: t.description.clone().unwrap_or_default(),
                parameters_schema: t
                    .parameters
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({"type": "object"})),
                effects: vec![ironclaw_engine::EffectType::Compute],
                requires_approval: false,
                model_tool_surface: ironclaw_engine::ModelToolSurface::FullSchema,
                discovery: None,
            })
        })
        .collect()
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
        | AppEvent::GateResolved { thread_id, .. }
        | AppEvent::ExternalToolCall { thread_id, .. } => thread_id.as_deref() == Some(target),
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
            AppEvent::ExternalToolCall {
                call_id,
                name,
                arguments,
                ..
            } => {
                // Caller-supplied tool was invoked by the LLM. Emit a
                // `function_call` output item and complete the turn —
                // the thread is paused in `Waiting` until the caller
                // POSTs the matching `function_call_output`.
                //
                // If buffered stream chunks accumulated before the
                // pause (e.g. the model preceded the call with prose),
                // flush them as a leading message so the OpenAI client
                // sees both pieces in order.
                if !self.text_chunks.is_empty() {
                    let leading: String = self.text_chunks.drain(..).collect();
                    if !leading.is_empty() {
                        self.output.push(ResponseOutputItem::Message {
                            id: make_item_id(),
                            role: "assistant".to_string(),
                            content: vec![MessageContent::OutputText { text: leading }],
                        });
                    }
                }
                self.output.push(ResponseOutputItem::FunctionCall {
                    id: make_item_id(),
                    call_id,
                    name,
                    arguments,
                });
                true // turn complete (waiting for caller-supplied result)
            }
            AppEvent::ToolStarted { name, call_id, .. } => {
                // Emit function_call placeholder — arguments filled on ToolCompleted.
                let call_id =
                    call_id.unwrap_or_else(|| format!("call_{}", Uuid::new_v4().simple()));
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
                call_id,
                ..
            } => {
                // Try to attach arguments to the matching FunctionCall.
                if let Some(args) = parameters
                    && let Some(idx) =
                        self.find_function_call_index(&name, call_id.as_deref(), true)
                    && let Some(ResponseOutputItem::FunctionCall { arguments, .. }) =
                        self.output.get_mut(idx)
                {
                    *arguments = args;
                }
                // On failure, record a FunctionCallOutput with the error.
                if !success && let Some(err) = error {
                    let call_id = self.resolve_call_id(&name, call_id.as_deref());
                    self.output.push(ResponseOutputItem::FunctionCallOutput {
                        id: make_item_id(),
                        call_id,
                        output: format!("Error: {err}"),
                    });
                }
                false
            }
            AppEvent::ToolResult {
                name,
                preview,
                call_id,
                ..
            } => {
                let call_id = self.resolve_call_id(&name, call_id.as_deref());
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
                self.usage.add_turn_cost(input_tokens, output_tokens);
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
            AppEvent::GateRequired {
                tool_name,
                parameters,
                extension_name,
                ..
            } => {
                self.output.push(ResponseOutputItem::FunctionCall {
                    id: make_item_id(),
                    call_id: format!("call_{}", Uuid::new_v4().simple()),
                    name: tool_name.clone(),
                    arguments: parameters,
                });
                self.failed = true;
                self.error_message = Some(if let Some(extension_name) = extension_name {
                    format!(
                        "Extension '{extension_name}' requires user authentication which is not supported via the Responses API"
                    )
                } else {
                    format!(
                        "Tool '{tool_name}' requires user input which is not supported via the Responses API"
                    )
                });
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

    fn has_function_call_id(&self, call_id: &str) -> bool {
        self.output.iter().any(|item| {
            matches!(
                item,
                ResponseOutputItem::FunctionCall {
                    call_id: existing_call_id,
                    ..
                } if existing_call_id == call_id
            )
        })
    }

    fn resolve_call_id(&self, name: &str, call_id: Option<&str>) -> String {
        if let Some(id) = call_id.filter(|id| !id.is_empty())
            && self.has_function_call_id(id)
        {
            return id.to_owned();
        }

        self.last_call_id_for(name)
    }

    fn find_function_call_index(
        &self,
        name: &str,
        call_id: Option<&str>,
        require_empty_arguments: bool,
    ) -> Option<usize> {
        if let Some(id) = call_id.filter(|id| !id.is_empty()) {
            for idx in (0..self.output.len()).rev() {
                if let ResponseOutputItem::FunctionCall {
                    call_id, arguments, ..
                } = &self.output[idx]
                    && call_id == id
                    && (!require_empty_arguments || arguments.is_empty())
                {
                    return Some(idx);
                }
            }
        }

        for idx in (0..self.output.len()).rev() {
            if let ResponseOutputItem::FunctionCall {
                name: item_name,
                arguments,
                ..
            } = &self.output[idx]
                && item_name == name
                && (!require_empty_arguments || arguments.is_empty())
            {
                return Some(idx);
            }
        }

        None
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
    let external_tools: Vec<ResponsesTool> = req.tools.clone().unwrap_or_default();
    validate_external_tools(&external_tools)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e, "invalid_request_error"))?;

    // Reject names that shadow registered (built-in or extension)
    // tools. Without this check, the catalog short-circuits the
    // dispatcher, so an LLM call to (say) `shell` lands in
    // caller-side execution even though the LLM saw the internal
    // `shell` description in its action surface — a confused-deputy
    // surface where the caller can craft any output and the LLM
    // treats it as the trusted internal tool's reply.
    if !external_tools.is_empty()
        && let Some(registry) = state.tool_registry.as_ref()
    {
        let registered: std::collections::HashSet<String> = registry
            .tool_definitions()
            .await
            .into_iter()
            .map(|t| t.name)
            .collect();
        for tool in &external_tools {
            if let Some(name) = tool.name.as_deref()
                && registered.contains(name)
            {
                return Err(api_error(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "tool '{name}' shadows a built-in or extension action; \
                         pick a different name"
                    ),
                    "invalid_request_error",
                ));
            }
        }
    }
    if req.tool_choice.is_some() {
        // `tool_choice` (auto / none / required / specific function) is not
        // honoured because the engine doesn't have a per-request tool surface
        // to enforce it against. Reject explicitly so callers don't get
        // silently-ignored steering.
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "The 'tool_choice' field is not yet supported",
            "invalid_request_error",
        ));
    }
    if req.temperature.is_some() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Per-request 'temperature' is not supported on this endpoint; configure the default via settings",
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

    // Caller-supplied tools require engine v2 — the gate machinery that
    // pauses execution and emits `AppEvent::ExternalToolCall` only
    // exists on the v2 path. The presence of an initialized
    // `ExternalToolCatalog` is the canonical "engine v2 is up" signal
    // here: the catalog is constructed inside `init_engine` and shared
    // between the bridge and this handler. Falling back to the
    // `ENGINE_V2` env var would diverge from the agent loop's runtime
    // config (`Config::agent::engine_v2`), which is the actual switch.
    let catalog = if !external_tools.is_empty() {
        let cat = crate::bridge::engine_external_tool_catalog().await;
        if cat.is_none() {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "Caller-supplied 'tools' require engine v2 to be enabled on the server",
                "invalid_request_error",
            ));
        }
        cat
    } else {
        None
    };

    let extracted = extract_user_content(&req.input)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, e, "invalid_request_error"))?;
    let mut content = extracted.user_text;

    // Prepend structured context (e.g. notification approval/rejection).
    // Enforce a 10 KB size limit to prevent context window exhaustion.
    if let Some(ref ctx) = req.x_context {
        let ctx_bytes = serde_json::to_string(ctx).map(|s| s.len()).unwrap_or(0);
        if ctx_bytes > 10 * 1024 {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                format!("x_context exceeds 10 KB limit ({ctx_bytes} bytes)"),
                "invalid_request_error",
            ));
        }
        let prefix = format_context(ctx);
        content = format!("<user-context>\n{prefix}\n</user-context>\n\n{content}");
    }

    // Prepend per-request instructions. Per the OpenAI Responses API spec,
    // `instructions` is a system/developer message inserted at the start of
    // the model's context for this turn only — it does NOT carry over via
    // `previous_response_id`. We surface it to the agent as an `<instructions>`
    // block ahead of everything else so it takes precedence over
    // `<user-context>`.
    if let Some(instructions) = req.instructions.as_deref().map(str::trim)
        && !instructions.is_empty()
    {
        content = format!("<instructions>\n{instructions}\n</instructions>\n\n{content}");
    }

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

    // Register caller-supplied tools in the engine's per-thread external
    // tool catalog. The engine's `EffectBridgeAdapter` consults this on
    // every action call to short-circuit caller tools to a
    // `GatePaused { resume_kind: External { ext_tool:<call_id> } }`,
    // which the bridge router projects to `AppEvent::ExternalToolCall`.
    if let Some(catalog) = catalog.as_ref() {
        let action_defs = responses_tools_to_action_defs(&external_tools);
        catalog
            .register(ironclaw_engine::ThreadId(thread_uuid), action_defs)
            .await;
    }

    // Resume detection: a request that carries `function_call_output`
    // items resolves the most recent `ResumeKind::External` gate for
    // this thread. Without a pending gate the request is malformed —
    // there's nothing to resume against.
    if !extracted.tool_outputs.is_empty() {
        let pending = crate::bridge::get_engine_pending_gate(&user.user_id, Some(&thread_id_str))
            .await
            .map_err(|e| {
                api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to look up pending gate: {e}"),
                    "server_error",
                )
            })?;
        let pending = pending.ok_or_else(|| {
            api_error(
                StatusCode::BAD_REQUEST,
                "function_call_output supplied but no pending external tool call for this thread",
                "invalid_request_error",
            )
        })?;
        let request_uuid = uuid::Uuid::parse_str(&pending.request_id).map_err(|_| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "pending gate has malformed request_id",
                "server_error",
            )
        })?;
        let payload = serde_json::json!({
            "outputs": extracted
                .tool_outputs
                .iter()
                .map(|(call_id, output)| serde_json::json!({
                    "call_id": call_id,
                    "output": output,
                }))
                .collect::<Vec<_>>(),
        });
        let submission = crate::agent::submission::Submission::ExternalCallback {
            request_id: request_uuid,
            payload: Some(payload),
        };
        let placeholder = "[external tool callback]".to_string();
        let mut metadata = serde_json::json!({
            "thread_id": &thread_id_str,
            "user_id": &user.user_id,
            "source": "responses_api",
        });
        if let Some(ref ctx) = req.x_context {
            metadata["context"] = ctx.clone();
        }
        let resume_msg = crate::channels::web::util::web_incoming_message_with_metadata(
            "gateway",
            &user.user_id,
            &placeholder,
            Some(&thread_id_str),
            metadata,
        )
        .with_structured_submission(submission);

        let resp_id = encode_response_id(&response_uuid, &thread_uuid);
        let model = req.model.clone();
        let stream = req.stream.unwrap_or(false);
        let user_id = user.user_id.clone();
        if stream {
            return handle_streaming(state, resume_msg, resp_id, model, thread_id_str, user_id)
                .await
                .map(IntoResponse::into_response);
        } else {
            return handle_non_streaming(
                state,
                resume_msg,
                resp_id,
                model,
                thread_id_str,
                &user_id,
            )
            .await
            .map(IntoResponse::into_response);
        }
    }

    // Build the message for the agent loop.
    let mut metadata = serde_json::json!({
        "thread_id": &thread_id_str,
        "user_id": &user.user_id,
        "source": "responses_api",
    });
    if let Some(ref ctx) = req.x_context {
        metadata["context"] = ctx.clone();
    }
    let msg = crate::channels::web::util::web_incoming_message_with_metadata(
        "gateway",
        &user.user_id,
        &content,
        Some(&thread_id_str),
        metadata,
    );

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
            AppEvent::ToolStarted { name, call_id, .. } => {
                let idx = acc.output.len();
                let call_id = call_id
                    .clone()
                    .unwrap_or_else(|| format!("call_{}", Uuid::new_v4().simple()));
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
            }
            AppEvent::ToolCompleted {
                name,
                success,
                error,
                parameters,
                call_id,
                ..
            } => {
                if let Some(args) = parameters
                    && let Some(idx) = acc.find_function_call_index(name, call_id.as_deref(), true)
                    && let Some(ResponseOutputItem::FunctionCall { arguments, .. }) =
                        acc.output.get_mut(idx)
                {
                    *arguments = args.clone();
                }
                if let Some(idx) = acc.find_function_call_index(name, call_id.as_deref(), false)
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
                    let call_id = acc.resolve_call_id(name, call_id.as_deref());
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
            AppEvent::ToolResult {
                name,
                preview,
                call_id,
                ..
            } => {
                let call_id = acc.resolve_call_id(name, call_id.as_deref());
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
                acc.usage.add_turn_cost(*input_tokens, *output_tokens);
            }
            _ => {}
        }

        // External tool call (engine pause): emit added+done frames for the
        // function_call wire item, then close the response. The thread
        // stays in `Waiting` until the caller POSTs the matching
        // `function_call_output`.
        if let AppEvent::ExternalToolCall {
            ref call_id,
            ref name,
            ref arguments,
            ..
        } = event
        {
            // If buffered chunks accumulated before the pause, flush them
            // as a leading message item so the OpenAI client sees the
            // prose then the function_call in order.
            if !acc.text_chunks.is_empty() {
                let leading: String = acc.text_chunks.drain(..).collect();
                if !leading.is_empty() {
                    let idx = acc.output.len();
                    let item = ResponseOutputItem::Message {
                        id: make_item_id(),
                        role: "assistant".to_string(),
                        content: vec![MessageContent::OutputText { text: leading }],
                    };
                    let _ = emit(
                        &tx,
                        "response.output_item.added",
                        &ResponseStreamEvent::OutputItemAdded {
                            output_index: idx,
                            item: item.clone(),
                        },
                    );
                    let _ = emit(
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

            let idx = acc.output.len();
            let item = ResponseOutputItem::FunctionCall {
                id: make_item_id(),
                call_id: call_id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
            };
            let _ = emit(
                &tx,
                "response.output_item.added",
                &ResponseStreamEvent::OutputItemAdded {
                    output_index: idx,
                    item: item.clone(),
                },
            );
            let _ = emit(
                &tx,
                "response.output_item.done",
                &ResponseStreamEvent::OutputItemDone {
                    output_index: idx,
                    item: item.clone(),
                },
            );
            acc.output.push(item);

            let resp = acc.finish();
            let _ = emit(
                &tx,
                "response.completed",
                &ResponseStreamEvent::ResponseCompleted { response: resp },
            );
            return;
        }

        // Terminal events.
        let is_terminal = matches!(
            &event,
            AppEvent::Response { .. }
                | AppEvent::Error { .. }
                | AppEvent::ApprovalNeeded { .. }
                | AppEvent::GateRequired { .. }
        );

        if is_terminal {
            if let AppEvent::Response { content, .. } = &event {
                let text = if content.is_empty() {
                    acc.text_chunks.join("")
                } else {
                    content.clone()
                };
                if !text.is_empty() {
                    let message_text = text.clone();
                    let idx = match message_output_index {
                        Some(i) => i,
                        None => {
                            // Create the output item first.
                            let i = acc.output.len();
                            let placeholder = ResponseOutputItem::Message {
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
                                    item: placeholder.clone(),
                                },
                            );
                            acc.output.push(placeholder);
                            i
                        }
                    };

                    // Emit the full text as a delta so streaming clients
                    // receive it via response.output_text.delta, but only
                    // when StreamChunks haven't already delivered the content.
                    if acc.text_chunks.is_empty() {
                        emit(
                            &tx,
                            "response.output_text.delta",
                            &ResponseStreamEvent::OutputTextDelta {
                                output_index: idx,
                                content_index: 0,
                                delta: message_text.clone(),
                            },
                        );
                    }

                    // Reuse the placeholder's ID so added→done correlation works.
                    let item_id =
                        if let Some(ResponseOutputItem::Message { id, .. }) = acc.output.get(idx) {
                            id.clone()
                        } else {
                            make_item_id()
                        };
                    let item = ResponseOutputItem::Message {
                        id: item_id,
                        role: "assistant".to_string(),
                        content: vec![MessageContent::OutputText { text: message_text }],
                    };
                    acc.output[idx] = item.clone();
                    emit(
                        &tx,
                        "response.output_item.done",
                        &ResponseStreamEvent::OutputItemDone {
                            output_index: idx,
                            item,
                        },
                    );
                }
            }

            if matches!(
                &event,
                AppEvent::Error { .. }
                    | AppEvent::ApprovalNeeded { .. }
                    | AppEvent::GateRequired { .. }
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
// GET /api/v1/responses/{id}
// (also served as GET /v1/responses/{id} for backward compat — ironclaw#2201)
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
            "assistant" if !msg.content.is_empty() => {
                output.push(ResponseOutputItem::Message {
                    id: format!("msg_{}", msg.id.simple()),
                    role: "assistant".to_string(),
                    content: vec![MessageContent::OutputText {
                        text: msg.content.clone(),
                    }],
                });
            }
            "assistant" => {}
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

    fn message_item(role: &str, content: &str) -> ResponsesInputItem {
        ResponsesInputItem {
            item_type: Some("message".to_string()),
            role: Some(role.to_string()),
            content: Some(content.to_string()),
            call_id: None,
            name: None,
            arguments: None,
            output: None,
        }
    }

    fn legacy_message_item(role: &str, content: &str) -> ResponsesInputItem {
        // Backwards-compatible shape: no `type` field.
        ResponsesInputItem {
            item_type: None,
            role: Some(role.to_string()),
            content: Some(content.to_string()),
            call_id: None,
            name: None,
            arguments: None,
            output: None,
        }
    }

    fn function_call_output_item(call_id: &str, output: &str) -> ResponsesInputItem {
        ResponsesInputItem {
            item_type: Some("function_call_output".to_string()),
            role: None,
            content: None,
            call_id: Some(call_id.to_string()),
            name: None,
            arguments: None,
            output: Some(output.to_string()),
        }
    }

    #[test]
    fn extract_user_content_text() {
        let input = ResponsesInput::Text("hello".to_string());
        let extracted = extract_user_content(&input).expect("extract");
        assert_eq!(extracted.user_text, "hello");
        assert!(extracted.tool_outputs.is_empty());
    }

    #[test]
    fn extract_user_content_empty_text_errors() {
        let input = ResponsesInput::Text(String::new());
        assert!(extract_user_content(&input).is_err());
    }

    #[test]
    fn extract_user_content_messages_uses_last_user() {
        let input = ResponsesInput::Items(vec![
            legacy_message_item("user", "first"),
            legacy_message_item("assistant", "middle"),
            legacy_message_item("user", "last"),
        ]);
        let extracted = extract_user_content(&input).expect("extract");
        assert_eq!(extracted.user_text, "last");
        assert!(extracted.tool_outputs.is_empty());
    }

    #[test]
    fn extract_user_content_no_user_message_errors() {
        let input = ResponsesInput::Items(vec![legacy_message_item("system", "hello")]);
        assert!(extract_user_content(&input).is_err());
    }

    #[test]
    fn extract_user_content_collects_function_call_outputs() {
        let input = ResponsesInput::Items(vec![
            message_item("user", "what is the weather?"),
            function_call_output_item("call_abc", "{\"temp\":72}"),
            function_call_output_item("call_def", "sunny"),
        ]);
        let extracted = extract_user_content(&input).expect("extract");
        assert_eq!(extracted.user_text, "what is the weather?");
        assert_eq!(
            extracted.tool_outputs,
            vec![
                ("call_abc".to_string(), "{\"temp\":72}".to_string()),
                ("call_def".to_string(), "sunny".to_string()),
            ]
        );
    }

    #[test]
    fn extract_user_content_function_call_output_only_synthesises_prompt() {
        // A follow-up turn that only carries tool results (no new user text)
        // should still produce a non-empty `user_text` so the engine sees a
        // user turn.
        let input = ResponsesInput::Items(vec![function_call_output_item("call_x", "result")]);
        let extracted = extract_user_content(&input).expect("extract");
        assert!(!extracted.user_text.is_empty());
        assert_eq!(
            extracted.tool_outputs,
            vec![("call_x".to_string(), "result".to_string())]
        );
    }

    #[test]
    fn extract_user_content_rejects_unknown_item_type() {
        let input = ResponsesInput::Items(vec![ResponsesInputItem {
            item_type: Some("file_search".to_string()),
            ..legacy_message_item("user", "hi")
        }]);
        assert!(extract_user_content(&input).is_err());
    }

    #[test]
    fn validate_external_tools_accepts_function_type() {
        let tools = vec![ResponsesTool {
            tool_type: "function".to_string(),
            name: Some("get_weather".to_string()),
            description: Some("Look up the weather".to_string()),
            parameters: Some(serde_json::json!({"type":"object"})),
        }];
        assert!(validate_external_tools(&tools).is_ok());
    }

    #[test]
    fn validate_external_tools_rejects_other_types() {
        let tools = vec![ResponsesTool {
            tool_type: "web_search".to_string(),
            name: Some("search".to_string()),
            description: None,
            parameters: None,
        }];
        let err = validate_external_tools(&tools).expect_err("must reject");
        assert!(err.contains("web_search"));
    }

    #[test]
    fn validate_external_tools_rejects_missing_name() {
        let tools = vec![ResponsesTool {
            tool_type: "function".to_string(),
            name: None,
            description: None,
            parameters: None,
        }];
        assert!(validate_external_tools(&tools).is_err());
    }

    #[test]
    fn validate_external_tools_rejects_duplicate_names() {
        let tools = vec![
            ResponsesTool {
                tool_type: "function".to_string(),
                name: Some("dup".to_string()),
                description: None,
                parameters: None,
            },
            ResponsesTool {
                tool_type: "function".to_string(),
                name: Some("dup".to_string()),
                description: None,
                parameters: None,
            },
        ];
        let err = validate_external_tools(&tools).expect_err("must reject");
        assert!(err.contains("duplicate"));
    }

    #[test]
    fn validate_external_tools_rejects_oversized_payload() {
        // One tool with a giant description blows past the 16 KiB cap.
        let big = "x".repeat(MAX_TOOLS_BYTES + 1024);
        let tools = vec![ResponsesTool {
            tool_type: "function".to_string(),
            name: Some("big".to_string()),
            description: Some(big),
            parameters: None,
        }];
        let err = validate_external_tools(&tools).expect_err("must reject");
        assert!(err.contains("limit"));
    }

    #[test]
    fn responses_tools_to_action_defs_basic_round_trip() {
        let tools = vec![ResponsesTool {
            tool_type: "function".to_string(),
            name: Some("get_weather".to_string()),
            description: Some("Look up the weather".to_string()),
            parameters: Some(
                serde_json::json!({"type":"object","properties":{"city":{"type":"string"}}}),
            ),
        }];
        let defs = responses_tools_to_action_defs(&tools);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "get_weather");
        assert_eq!(defs[0].description, "Look up the weather");
        assert!(defs[0].parameters_schema.get("properties").is_some());
        assert!(!defs[0].requires_approval);
    }

    #[test]
    fn responses_tools_to_action_defs_skips_nameless() {
        let tools = vec![ResponsesTool {
            tool_type: "function".to_string(),
            name: None,
            description: None,
            parameters: None,
        }];
        let defs = responses_tools_to_action_defs(&tools);
        assert!(defs.is_empty());
    }

    #[test]
    fn accumulator_external_tool_call_emits_function_call_item() {
        let mut acc = ResponseAccumulator::new("resp_test".to_string(), "m".to_string());
        let done = acc.process(AppEvent::ExternalToolCall {
            request_id: "req-1".into(),
            call_id: "call_abc".into(),
            name: "lookup".into(),
            arguments: "{\"q\":\"x\"}".into(),
            thread_id: Some("t".into()),
        });
        assert!(done, "external tool call must complete the turn");
        let resp = acc.finish();
        assert_eq!(resp.output.len(), 1);
        assert!(matches!(
            &resp.output[0],
            ResponseOutputItem::FunctionCall { call_id, name, arguments, .. }
                if call_id == "call_abc"
                    && name == "lookup"
                    && arguments == "{\"q\":\"x\"}"
        ));
        assert_eq!(resp.status, ResponseStatus::Completed);
    }

    #[test]
    fn accumulator_external_tool_call_flushes_buffered_chunks() {
        let mut acc = ResponseAccumulator::new("resp_test".to_string(), "m".to_string());
        acc.process(AppEvent::StreamChunk {
            content: "Calling the tool.".into(),
            thread_id: Some("t".into()),
        });
        let done = acc.process(AppEvent::ExternalToolCall {
            request_id: "req-1".into(),
            call_id: "call_abc".into(),
            name: "lookup".into(),
            arguments: "{}".into(),
            thread_id: Some("t".into()),
        });
        assert!(done);
        let resp = acc.finish();
        assert_eq!(resp.output.len(), 2);
        assert!(matches!(
            &resp.output[0],
            ResponseOutputItem::Message { content, .. }
                if matches!(&content[0], MessageContent::OutputText { text } if text == "Calling the tool.")
        ));
        assert!(matches!(
            &resp.output[1],
            ResponseOutputItem::FunctionCall { name, .. } if name == "lookup"
        ));
    }

    /// Streaming round-trip: when the LLM streams a couple of text
    /// chunks then triggers a caller-supplied external tool, the SSE
    /// stream must (a) deliver the deltas as `output_text.delta`, (b)
    /// flush the buffered prose as a complete Message item *before*
    /// the function_call, (c) emit `output_item.added`+`done` for the
    /// function_call, and (d) close with `response.completed`.
    ///
    /// The earlier `accumulator_*` tests cover the in-memory state
    /// transitions; this test drives the full `streaming_worker` and
    /// verifies the wire-frame sequence that an OpenAI client would
    /// observe.
    #[tokio::test]
    async fn streaming_worker_external_tool_call_emits_correct_frame_sequence() {
        use axum::response::Sse;
        use axum::response::sse::KeepAlive;
        use http_body_util::BodyExt;
        use tokio_stream::wrappers::ReceiverStream;

        let thread_id = "thread-stream-test".to_string();

        // Input: synthetic AppEvent stream the worker will consume.
        let (input_tx, input_rx) = tokio::sync::mpsc::channel::<AppEvent>(8);
        let input_stream = ReceiverStream::new(input_rx);

        // Output: the worker pushes axum SSE Events here. Buffer
        // generously — the external-tool branch can emit up to 7
        // frames (created, delta, item.added/done × 2, completed).
        let (out_tx, out_rx) = tokio::sync::mpsc::channel::<axum::response::sse::Event>(16);

        // Drive the worker on a background task so we can feed it
        // synchronously from the test body.
        let worker = tokio::spawn(streaming_worker(
            out_tx,
            input_stream,
            "resp_stream_test".to_string(),
            "test-model".to_string(),
            thread_id.clone(),
        ));

        // Two text deltas, then the external-tool gate fires.
        input_tx
            .send(AppEvent::StreamChunk {
                content: "Looking up ".to_string(),
                thread_id: Some(thread_id.clone()),
            })
            .await
            .unwrap();
        input_tx
            .send(AppEvent::StreamChunk {
                content: "the weather.".to_string(),
                thread_id: Some(thread_id.clone()),
            })
            .await
            .unwrap();
        input_tx
            .send(AppEvent::ExternalToolCall {
                request_id: "req-stream-1".to_string(),
                call_id: "call_lookup_weather_1".to_string(),
                name: "lookup_weather".to_string(),
                arguments: "{\"city\":\"NYC\"}".to_string(),
                thread_id: Some(thread_id.clone()),
            })
            .await
            .unwrap();
        // Closing the input stream lets the worker exit if the
        // external-tool branch didn't already terminate it.
        drop(input_tx);

        // Collect Events. The external-tool branch returns from the
        // worker, which drops out_tx and closes the receiver — so
        // pulling from the stream until exhaustion gives us every
        // emitted frame.
        let out_stream = ReceiverStream::new(out_rx);
        let response = Sse::new(out_stream.map(Ok::<_, Infallible>))
            .keep_alive(KeepAlive::new().interval(Duration::from_secs(60)).text(""))
            .into_response();
        let body = response.into_body();
        let bytes = tokio::time::timeout(Duration::from_secs(5), body.collect())
            .await
            .expect("body collected within timeout")
            .expect("body bytes")
            .to_bytes();
        let text = std::str::from_utf8(&bytes).expect("utf8");

        // Worker must finish cleanly (no panic).
        worker.await.expect("worker panicked");

        // Parse SSE frames. axum emits `event: <type>\ndata: <json>\n\n`
        // for each event; a leading colon-only line is the keep-alive.
        let frames: Vec<(String, String)> = text
            .split("\n\n")
            .filter_map(|frame| {
                let mut event_type: Option<String> = None;
                let mut data: Option<String> = None;
                for line in frame.lines() {
                    if let Some(t) = line.strip_prefix("event:") {
                        event_type = Some(t.trim().to_string());
                    } else if let Some(d) = line.strip_prefix("data:") {
                        data = Some(d.trim().to_string());
                    }
                }
                match (event_type, data) {
                    (Some(t), Some(d)) => Some((t, d)),
                    _ => None,
                }
            })
            .collect();

        let event_types: Vec<&str> = frames.iter().map(|(t, _)| t.as_str()).collect();

        // Expected wire-frame sequence:
        //   response.created
        //   response.output_item.added       (Message placeholder for the streaming text)
        //   response.output_text.delta × 2   (one per chunk)
        //   response.output_item.added       (Message — flushed prose)
        //   response.output_item.done        (Message — flushed prose)
        //   response.output_item.added       (FunctionCall)
        //   response.output_item.done        (FunctionCall)
        //   response.completed
        assert_eq!(
            event_types,
            vec![
                "response.created",
                "response.output_item.added",
                "response.output_text.delta",
                "response.output_text.delta",
                "response.output_item.added",
                "response.output_item.done",
                "response.output_item.added",
                "response.output_item.done",
                "response.completed",
            ],
            "wire frame sequence does not match expected ordering"
        );

        // Find the function_call frames and assert they carry the
        // caller's tool identity.
        let function_call_frames: Vec<&(String, String)> = frames
            .iter()
            .filter(|(t, d)| {
                t == "response.output_item.added" && d.contains("\"type\":\"function_call\"")
            })
            .collect();
        assert_eq!(
            function_call_frames.len(),
            1,
            "expected exactly one function_call output_item.added frame"
        );
        let payload = &function_call_frames[0].1;
        assert!(
            payload.contains("\"call_id\":\"call_lookup_weather_1\""),
            "function_call frame missing caller-supplied call_id: {payload}"
        );
        assert!(
            payload.contains("\"name\":\"lookup_weather\""),
            "function_call frame missing tool name: {payload}"
        );
        assert!(
            payload.contains("\"arguments\":\"{\\\"city\\\":\\\"NYC\\\"}\""),
            "function_call frame missing arguments: {payload}"
        );

        // The flushed leading message must contain the concatenated
        // text from both StreamChunks, in order.
        let message_done_frames: Vec<&(String, String)> = frames
            .iter()
            .filter(|(t, d)| t == "response.output_item.done" && d.contains("\"type\":\"message\""))
            .collect();
        assert_eq!(message_done_frames.len(), 1, "expected one Message done");
        let message_payload = &message_done_frames[0].1;
        assert!(
            message_payload.contains("Looking up the weather."),
            "Message done frame missing concatenated stream text: {message_payload}"
        );
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
            detail: None,
            call_id: Some("call_memory_search".to_string()),
            thread_id: Some("t".to_string()),
        }));
        assert!(!acc.process(AppEvent::ToolResult {
            name: "memory_search".to_string(),
            preview: "found 3 results".to_string(),
            call_id: Some("call_memory_search".to_string()),
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
    fn accumulator_uses_call_id_for_duplicate_tool_names() {
        let mut acc = ResponseAccumulator::new("resp_test".to_string(), "m".to_string());

        assert!(!acc.process(AppEvent::ToolStarted {
            name: "memory_search".to_string(),
            detail: None,
            call_id: Some("call_a".to_string()),
            thread_id: Some("t".to_string()),
        }));
        assert!(!acc.process(AppEvent::ToolStarted {
            name: "memory_search".to_string(),
            detail: None,
            call_id: Some("call_b".to_string()),
            thread_id: Some("t".to_string()),
        }));
        assert!(!acc.process(AppEvent::ToolResult {
            name: "memory_search".to_string(),
            preview: "result for b".to_string(),
            call_id: Some("call_b".to_string()),
            thread_id: Some("t".to_string()),
        }));
        assert!(!acc.process(AppEvent::ToolResult {
            name: "memory_search".to_string(),
            preview: "result for a".to_string(),
            call_id: Some("call_a".to_string()),
            thread_id: Some("t".to_string()),
        }));
        assert!(acc.process(AppEvent::Response {
            content: "done".to_string(),
            thread_id: "t".to_string(),
        }));

        let resp = acc.finish();
        assert_eq!(resp.output.len(), 5);
        assert!(matches!(
            &resp.output[2],
            ResponseOutputItem::FunctionCallOutput { call_id, output, .. }
                if call_id == "call_b" && output == "result for b"
        ));
        assert!(matches!(
            &resp.output[3],
            ResponseOutputItem::FunctionCallOutput { call_id, output, .. }
                if call_id == "call_a" && output == "result for a"
        ));
    }

    #[test]
    fn accumulator_tool_result_falls_back_to_started_call_id_on_unknown_call_id() {
        let mut acc = ResponseAccumulator::new("resp_test".to_string(), "m".to_string());

        assert!(!acc.process(AppEvent::ToolStarted {
            name: "memory_search".to_string(),
            detail: None,
            call_id: None,
            thread_id: Some("t".to_string()),
        }));
        let started_call_id = match &acc.output[0] {
            ResponseOutputItem::FunctionCall { call_id, .. } => call_id.clone(),
            _ => panic!("expected FunctionCall output item"),
        };

        assert!(!acc.process(AppEvent::ToolResult {
            name: "memory_search".to_string(),
            preview: "found 3 results".to_string(),
            call_id: Some("unexpected_call_id".to_string()),
            thread_id: Some("t".to_string()),
        }));
        assert!(acc.process(AppEvent::Response {
            content: "done".to_string(),
            thread_id: "t".to_string(),
        }));

        let resp = acc.finish();
        assert!(matches!(
            &resp.output[1],
            ResponseOutputItem::FunctionCallOutput { call_id, output, .. }
                if call_id == &started_call_id && output == "found 3 results"
        ));
    }

    #[test]
    fn accumulator_tool_completed_error_falls_back_to_started_call_id_on_unknown_call_id() {
        let mut acc = ResponseAccumulator::new("resp_test".to_string(), "m".to_string());

        assert!(!acc.process(AppEvent::ToolStarted {
            name: "memory_search".to_string(),
            detail: None,
            call_id: None,
            thread_id: Some("t".to_string()),
        }));
        let started_call_id = match &acc.output[0] {
            ResponseOutputItem::FunctionCall { call_id, .. } => call_id.clone(),
            _ => panic!("expected FunctionCall output item"),
        };

        assert!(!acc.process(AppEvent::ToolCompleted {
            name: "memory_search".to_string(),
            success: false,
            error: Some("boom".to_string()),
            parameters: Some("{\"query\":\"rust\"}".to_string()),
            call_id: Some("unexpected_call_id".to_string()),
            duration_ms: None,
            thread_id: Some("t".to_string()),
        }));
        assert!(acc.process(AppEvent::Response {
            content: "done".to_string(),
            thread_id: "t".to_string(),
        }));

        let resp = acc.finish();
        assert!(matches!(
            &resp.output[1],
            ResponseOutputItem::FunctionCallOutput { call_id, output, .. }
                if call_id == &started_call_id && output == "Error: boom"
        ));
    }

    #[test]
    fn accumulator_turn_cost_populates_usage() {
        let mut acc = ResponseAccumulator::new("resp_test".to_string(), "m".to_string());
        assert!(!acc.process(AppEvent::TurnCost {
            input_tokens: 12,
            output_tokens: 3,
            cost_usd: "$0.0180".to_string(),
            thread_id: Some("t".to_string()),
        }));
        assert!(acc.process(AppEvent::Response {
            content: "Done".to_string(),
            thread_id: "t".to_string(),
        }));

        let resp = acc.finish();
        assert_eq!(resp.usage.input_tokens, 12);
        assert_eq!(resp.usage.output_tokens, 3);
        assert_eq!(resp.usage.total_tokens, 15);
    }

    #[test]
    fn accumulator_turn_cost_accumulates_multiple_segments() {
        let mut acc = ResponseAccumulator::new("resp_test".to_string(), "m".to_string());
        assert!(!acc.process(AppEvent::TurnCost {
            input_tokens: 12,
            output_tokens: 3,
            cost_usd: "$0.0180".to_string(),
            thread_id: Some("t".to_string()),
        }));
        assert!(!acc.process(AppEvent::TurnCost {
            input_tokens: 5,
            output_tokens: 7,
            cost_usd: "$0.0190".to_string(),
            thread_id: Some("t".to_string()),
        }));
        assert!(acc.process(AppEvent::Response {
            content: "Done".to_string(),
            thread_id: "t".to_string(),
        }));

        let resp = acc.finish();
        assert_eq!(resp.usage.input_tokens, 17);
        assert_eq!(resp.usage.output_tokens, 10);
        assert_eq!(resp.usage.total_tokens, 27);
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
    fn accumulator_gate_required_marks_failed() {
        let mut acc = ResponseAccumulator::new("resp_test".to_string(), "m".to_string());
        assert!(acc.process(AppEvent::GateRequired {
            request_id: "r1".to_string(),
            gate_name: "auth".to_string(),
            tool_name: "tool_install".to_string(),
            description: "Need auth".to_string(),
            parameters: "{\"name\":\"notion\"}".to_string(),
            extension_name: Some(ironclaw_common::ExtensionName::new("notion").unwrap()),
            resume_kind: serde_json::json!({
                "Authentication": {
                    "credential_name": "notion_api_token",
                    "instructions": "Complete authentication",
                    "auth_url": "https://example.test/oauth"
                }
            }),
            thread_id: Some("t".to_string()),
        }));
        let resp = acc.finish();
        assert_eq!(resp.status, ResponseStatus::Failed);
        assert!(matches!(
            &resp.output[0],
            ResponseOutputItem::FunctionCall { name, arguments, .. }
                if name == "tool_install" && arguments == "{\"name\":\"notion\"}"
        ));
        assert_eq!(
            resp.error.as_ref().map(|error| error.message.as_str()),
            Some(
                "Extension 'notion' requires user authentication which is not supported via the Responses API"
            )
        );
    }

    #[test]
    fn response_status_serializes_as_snake_case() {
        let json = serde_json::to_string(&ResponseStatus::InProgress).expect("serialize");
        assert_eq!(json, "\"in_progress\"");
        let json = serde_json::to_string(&ResponseStatus::Completed).expect("serialize");
        assert_eq!(json, "\"completed\"");
    }

    #[test]
    fn format_context_notification_response() {
        let ctx = serde_json::json!({
            "notification_response": {
                "notification_id": "msg_123",
                "action": "approved",
                "score": 72
            }
        });
        let result = format_context(&ctx);
        assert!(result.contains("[Context: notification_response"));
        assert!(result.contains("notification_id: msg_123"));
        assert!(result.contains("action: approved"));
        assert!(result.contains("score: 72"));
    }

    #[test]
    fn format_context_simple_value() {
        let ctx = serde_json::json!({"status": "ok"});
        let result = format_context(&ctx);
        assert!(result.contains("status"));
        assert!(result.contains("ok"));
    }
}
