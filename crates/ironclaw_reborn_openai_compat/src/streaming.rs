//! OpenAI-compatible SSE translation for projection-backed streams.
//!
//! This module is intentionally route-owned: it consumes projection-safe
//! outbound envelopes supplied by host composition and emits only OpenAI wire
//! events. Projection cursors stay internal to drain requests and never appear
//! as SSE ids or payload fields.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_core::Stream;
use ironclaw_product_adapters::{
    ProductInboundAck, ProductOutboundEnvelope, ProductOutboundPayload, ProductProjectionItem,
    ProductProjectionState, ProjectionCursor, ProjectionSubscriptionRequest,
};
use serde::Serialize;
use serde_json::json;

use crate::{
    OpenAiChatCompletionChunk, OpenAiChatCompletionId, OpenAiChatDelta, OpenAiChatFinishReason,
    OpenAiChatMessageRole, OpenAiChatModelOnlyTools, OpenAiChatStreamChoice,
    OpenAiCompatActorScope, OpenAiCompatErrorCode, OpenAiCompatErrorKind,
    OpenAiCompatErrorResponse, OpenAiCompatHttpError, OpenAiCompatResourceMapping,
    OpenAiResponseErrorObject, OpenAiResponseId, OpenAiResponseObject, OpenAiResponseOutputItem,
    OpenAiResponseOutputItemStatus, OpenAiResponseStatus, OpenAiResponsesMessageRole,
};

const STREAM_INITIAL_POLL_INTERVAL: Duration = Duration::from_millis(50);
const STREAM_MAX_POLL_INTERVAL: Duration = Duration::from_millis(500);
const STREAM_KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq)]
pub struct OpenAiChatProjectionStreamRequest {
    pub public_id: OpenAiChatCompletionId,
    pub actor_scope: OpenAiCompatActorScope,
    pub accepted_ack: ProductInboundAck,
    pub requested_model: String,
    pub model_only_tools: Option<OpenAiChatModelOnlyTools>,
    pub projection_subscription: ProjectionSubscriptionRequest,
    pub mapping: OpenAiCompatResourceMapping,
    pub wait_timeout: Duration,
    pub after_cursor: Option<ProjectionCursor>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenAiResponseProjectionStreamRequest {
    pub public_id: OpenAiResponseId,
    pub actor_scope: OpenAiCompatActorScope,
    pub accepted_ack: ProductInboundAck,
    pub requested_model: String,
    pub projection_subscription: ProjectionSubscriptionRequest,
    pub mapping: OpenAiCompatResourceMapping,
    pub wait_timeout: Duration,
    pub after_cursor: Option<ProjectionCursor>,
}

#[async_trait]
pub trait OpenAiCompatProjectionStreamer: Send + Sync {
    async fn drain_chat(
        &self,
        request: OpenAiChatProjectionStreamRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, OpenAiCompatHttpError>;

    async fn drain_response(
        &self,
        request: OpenAiResponseProjectionStreamRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, OpenAiCompatHttpError>;
}

pub(crate) fn chat_sse_response(
    streamer: Arc<dyn OpenAiCompatProjectionStreamer>,
    request: OpenAiChatProjectionStreamRequest,
) -> Response {
    Sse::new(chat_sse_stream(streamer, request))
        .keep_alive(KeepAlive::new().interval(STREAM_KEEP_ALIVE_INTERVAL))
        .into_response()
}

pub(crate) fn response_sse_response(
    streamer: Arc<dyn OpenAiCompatProjectionStreamer>,
    request: OpenAiResponseProjectionStreamRequest,
) -> Response {
    Sse::new(response_sse_stream(streamer, request))
        .keep_alive(KeepAlive::new().interval(STREAM_KEEP_ALIVE_INTERVAL))
        .into_response()
}

fn chat_sse_stream(
    streamer: Arc<dyn OpenAiCompatProjectionStreamer>,
    request: OpenAiChatProjectionStreamRequest,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        let created = request.mapping.created_at;
        let public_id = request.public_id.clone();
        let model = request.requested_model.clone();
        let mut after_cursor = request.after_cursor.clone();
        let mut state = TextDeltaState::default();
        let mut pacing = StreamPacing::new(request.wait_timeout);

        yield Ok(chat_chunk_event(OpenAiChatCompletionChunk {
            id: public_id.clone(),
            object: "chat.completion.chunk".to_string(),
            created,
            model: model.clone(),
            choices: vec![OpenAiChatStreamChoice {
                index: 0,
                delta: OpenAiChatDelta {
                    role: Some(OpenAiChatMessageRole::Assistant),
                    content: None,
                    tool_calls: None,
                },
                finish_reason: None,
            }],
            usage: None,
        }));

        loop {
            let mut drain_request = request.clone();
            drain_request.after_cursor = after_cursor.clone();
            let drain_result = match pacing
                .timeout(streamer.drain_chat(drain_request))
                .await
            {
                Ok(result) => result,
                Err(error) => {
                    yield Ok(openai_error_event(error));
                    return;
                }
            };
            match drain_result {
                Ok(envelopes) => {
                    if envelopes.is_empty() {
                        if let Err(error) = pacing.sleep_after_empty_poll().await {
                            yield Ok(openai_error_event(error));
                            return;
                        }
                        continue;
                    }
                    pacing.reset_backoff();
                    for envelope in envelopes {
                        after_cursor = Some(envelope.projection_cursor().clone());
                        let payload_view = payload_view(envelope.payload());
                        match payload_view.text {
                            PayloadText::None => {}
                            PayloadText::Update(text) => match state.delta_for(text) {
                                Ok(Some(delta)) => yield Ok(chat_text_delta_event(&public_id, created, &model, delta)),
                                Ok(None) => {}
                                Err(error) => {
                                    yield Ok(openai_error_event(error));
                                    return;
                                }
                            },
                            PayloadText::Final(text) => {
                                match state.delta_for(text) {
                                    Ok(Some(delta)) => yield Ok(chat_text_delta_event(&public_id, created, &model, delta)),
                                    Ok(None) => {}
                                    Err(error) => {
                                        yield Ok(openai_error_event(error));
                                        return;
                                    }
                                }
                                yield Ok(chat_finish_event(&public_id, created, &model));
                                yield Ok(Event::default().data("[DONE]"));
                                return;
                            }
                        }
                        match payload_view.terminal_status {
                            TerminalStatus::None => {}
                            TerminalStatus::Completed => {
                                yield Ok(chat_finish_event(&public_id, created, &model));
                                yield Ok(Event::default().data("[DONE]"));
                                return;
                            }
                            TerminalStatus::Failed | TerminalStatus::Cancelled => {
                                yield Ok(openai_error_event(OpenAiCompatHttpError::internal()));
                                return;
                            }
                        }
                    }
                }
                Err(error) => {
                    yield Ok(openai_error_event(error));
                    return;
                }
            }
        }
    }
}

fn response_sse_stream(
    streamer: Arc<dyn OpenAiCompatProjectionStreamer>,
    request: OpenAiResponseProjectionStreamRequest,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        let created = request.mapping.created_at;
        let public_id = request.public_id.clone();
        let model = request.requested_model.clone();
        let item_id = format!("msg_{}", public_id.as_str());
        let mut sequence_number = 0_u64;
        let mut after_cursor = request.after_cursor.clone();
        let mut state = TextDeltaState::default();
        let mut pacing = StreamPacing::new(request.wait_timeout);

        yield Ok(response_event(
            "response.created",
            json!({
                "type": "response.created",
                "sequence_number": sequence_number,
                "response": response_object(public_id.clone(), created, model.clone(), OpenAiResponseStatus::InProgress, ""),
            }),
        ));
        sequence_number += 1;

        loop {
            let mut drain_request = request.clone();
            drain_request.after_cursor = after_cursor.clone();
            let drain_result = match pacing
                .timeout(streamer.drain_response(drain_request))
                .await
            {
                Ok(result) => result,
                Err(error) => {
                    yield Ok(response_stream_error_event(error));
                    return;
                }
            };
            match drain_result {
                Ok(envelopes) => {
                    if envelopes.is_empty() {
                        if let Err(error) = pacing.sleep_after_empty_poll().await {
                            yield Ok(response_stream_error_event(error));
                            return;
                        }
                        continue;
                    }
                    pacing.reset_backoff();
                    for envelope in envelopes {
                        after_cursor = Some(envelope.projection_cursor().clone());
                        let payload_view = payload_view(envelope.payload());
                        match payload_view.text {
                            PayloadText::None => {}
                            PayloadText::Update(text) => match state.delta_for(text) {
                                Ok(Some(delta)) => {
                                    yield Ok(response_text_delta_event(
                                        &public_id,
                                        &item_id,
                                        sequence_number,
                                        delta,
                                    ));
                                    sequence_number += 1;
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    yield Ok(response_stream_error_event(error));
                                    return;
                                }
                            },
                            PayloadText::Final(text) => {
                                match state.delta_for(text) {
                                    Ok(Some(delta)) => {
                                        yield Ok(response_text_delta_event(
                                            &public_id,
                                            &item_id,
                                            sequence_number,
                                            delta,
                                        ));
                                        sequence_number += 1;
                                    }
                                    Ok(None) => {}
                                    Err(error) => {
                                        yield Ok(response_stream_error_event(error));
                                        return;
                                    }
                                }
                                if !state.is_empty() {
                                    yield Ok(response_text_done_event(&item_id, sequence_number, state.text()));
                                    sequence_number += 1;
                                }
                                yield Ok(response_terminal_event(
                                    "response.completed",
                                    sequence_number,
                                    public_id.clone(),
                                    created,
                                    model.clone(),
                                    OpenAiResponseStatus::Completed,
                                    state.text(),
                                ));
                                return;
                            }
                        }
                        match payload_view.terminal_status {
                            TerminalStatus::None => {}
                            TerminalStatus::Completed => {
                                if !state.is_empty() {
                                    yield Ok(response_text_done_event(&item_id, sequence_number, state.text()));
                                    sequence_number += 1;
                                }
                                yield Ok(response_terminal_event(
                                    "response.completed",
                                    sequence_number,
                                    public_id.clone(),
                                    created,
                                    model.clone(),
                                    OpenAiResponseStatus::Completed,
                                    state.text(),
                                ));
                                return;
                            }
                            TerminalStatus::Failed => {
                                yield Ok(response_terminal_event(
                                    "response.failed",
                                    sequence_number,
                                    public_id.clone(),
                                    created,
                                    model.clone(),
                                    OpenAiResponseStatus::Failed,
                                    state.text(),
                                ));
                                return;
                            }
                            TerminalStatus::Cancelled => {
                                yield Ok(response_terminal_event(
                                    "response.cancelled",
                                    sequence_number,
                                    public_id.clone(),
                                    created,
                                    model.clone(),
                                    OpenAiResponseStatus::Cancelled,
                                    state.text(),
                                ));
                                return;
                            }
                        }
                    }
                }
                Err(error) => {
                    yield Ok(response_stream_error_event(error));
                    return;
                }
            }
        }
    }
}

struct StreamPacing {
    deadline: tokio::time::Instant,
    backoff: Duration,
}

impl StreamPacing {
    fn new(wait_timeout: Duration) -> Self {
        Self {
            deadline: tokio::time::Instant::now() + wait_timeout,
            backoff: STREAM_INITIAL_POLL_INTERVAL,
        }
    }

    async fn timeout<F, T>(&self, future: F) -> Result<T, OpenAiCompatHttpError>
    where
        F: std::future::Future<Output = T>,
    {
        let remaining = self.remaining()?;
        tokio::time::timeout(remaining, future)
            .await
            .map_err(|_| stream_timeout_error())
    }

    async fn sleep_after_empty_poll(&mut self) -> Result<(), OpenAiCompatHttpError> {
        let remaining = self.remaining()?;
        tokio::time::sleep(self.backoff.min(remaining)).await;
        self.backoff = (self.backoff * 2).min(STREAM_MAX_POLL_INTERVAL);
        self.remaining().map(|_| ())
    }

    fn reset_backoff(&mut self) {
        self.backoff = STREAM_INITIAL_POLL_INTERVAL;
    }

    fn remaining(&self) -> Result<Duration, OpenAiCompatHttpError> {
        let remaining = self
            .deadline
            .checked_duration_since(tokio::time::Instant::now())
            .ok_or_else(stream_timeout_error)?;
        if remaining.is_zero() {
            return Err(stream_timeout_error());
        }
        Ok(remaining)
    }
}

fn stream_timeout_error() -> OpenAiCompatHttpError {
    OpenAiCompatHttpError::from_kind(503, true, OpenAiCompatErrorKind::ServiceUnavailable, None)
}

#[derive(Default)]
struct TextDeltaState {
    text: String,
}

impl TextDeltaState {
    fn delta_for(&mut self, next: &str) -> Result<Option<String>, OpenAiCompatHttpError> {
        let consumed = self.text.len();
        if next.len() == consumed {
            if next.as_bytes() != self.text.as_bytes() {
                return Err(OpenAiCompatHttpError::from_kind(
                    500,
                    false,
                    OpenAiCompatErrorKind::Internal,
                    None,
                ));
            }
            return Ok(None);
        }
        if next.len() < consumed
            || !next.is_char_boundary(consumed)
            || !next.as_bytes().starts_with(self.text.as_bytes())
        {
            return Err(OpenAiCompatHttpError::from_kind(
                500,
                false,
                OpenAiCompatErrorKind::Internal,
                None,
            ));
        }
        let delta = &next[consumed..];
        self.text.push_str(delta);
        Ok(Some(delta.to_string()))
    }

    fn text(&self) -> &str {
        &self.text
    }

    fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalStatus {
    None,
    Completed,
    Failed,
    Cancelled,
}

enum PayloadText<'a> {
    None,
    Update(&'a str),
    Final(&'a str),
}

struct PayloadView<'a> {
    text: PayloadText<'a>,
    terminal_status: TerminalStatus,
}

fn payload_view(payload: &ProductOutboundPayload) -> PayloadView<'_> {
    match payload {
        ProductOutboundPayload::FinalReply(reply) => PayloadView {
            text: PayloadText::Final(&reply.text),
            terminal_status: TerminalStatus::None,
        },
        ProductOutboundPayload::ProjectionSnapshot { state }
        | ProductOutboundPayload::ProjectionUpdate { state } => projection_state_view(state),
        ProductOutboundPayload::KeepAlive
        | ProductOutboundPayload::Progress(_)
        | ProductOutboundPayload::CapabilityActivity(_)
        | ProductOutboundPayload::CapabilityDisplayPreview(_)
        | ProductOutboundPayload::GatePrompt(_)
        | ProductOutboundPayload::AuthPrompt(_) => PayloadView {
            text: PayloadText::None,
            terminal_status: TerminalStatus::None,
        },
    }
}

fn projection_state_view(state: &ProductProjectionState) -> PayloadView<'_> {
    let mut text = PayloadText::None;
    let mut terminal_status = TerminalStatus::None;
    for item in state.items.iter().rev() {
        match item {
            ProductProjectionItem::Text { body, .. } if matches!(text, PayloadText::None) => {
                text = PayloadText::Update(body.as_str());
            }
            ProductProjectionItem::RunStatus { status, .. }
                if matches!(terminal_status, TerminalStatus::None) =>
            {
                terminal_status = match status.as_str() {
                    "completed" => TerminalStatus::Completed,
                    "failed" | "killed" => TerminalStatus::Failed,
                    "cancelled" => TerminalStatus::Cancelled,
                    _ => TerminalStatus::None,
                };
            }
            _ => {}
        }
        if !matches!(text, PayloadText::None) && !matches!(terminal_status, TerminalStatus::None) {
            break;
        }
    }
    PayloadView {
        text,
        terminal_status,
    }
}

fn chat_text_delta_event(
    public_id: &OpenAiChatCompletionId,
    created: u64,
    model: &str,
    delta: String,
) -> Event {
    chat_chunk_event(OpenAiChatCompletionChunk {
        id: public_id.clone(),
        object: "chat.completion.chunk".to_string(),
        created,
        model: model.to_string(),
        choices: vec![OpenAiChatStreamChoice {
            index: 0,
            delta: OpenAiChatDelta {
                role: None,
                content: Some(delta),
                tool_calls: None,
            },
            finish_reason: None,
        }],
        usage: None,
    })
}

fn chat_finish_event(public_id: &OpenAiChatCompletionId, created: u64, model: &str) -> Event {
    chat_chunk_event(OpenAiChatCompletionChunk {
        id: public_id.clone(),
        object: "chat.completion.chunk".to_string(),
        created,
        model: model.to_string(),
        choices: vec![OpenAiChatStreamChoice {
            index: 0,
            delta: OpenAiChatDelta {
                role: None,
                content: None,
                tool_calls: None,
            },
            finish_reason: Some(OpenAiChatFinishReason::Stop),
        }],
        usage: None,
    })
}

fn chat_chunk_event(chunk: OpenAiChatCompletionChunk) -> Event {
    data_event(None, &chunk)
}

fn response_text_delta_event(
    public_id: &OpenAiResponseId,
    item_id: &str,
    sequence_number: u64,
    delta: String,
) -> Event {
    response_event(
        "response.output_text.delta",
        json!({
            "type": "response.output_text.delta",
            "sequence_number": sequence_number,
            "response_id": public_id.as_str(),
            "item_id": item_id,
            "output_index": 0,
            "content_index": 0,
            "delta": delta,
        }),
    )
}

fn response_text_done_event(item_id: &str, sequence_number: u64, text: &str) -> Event {
    response_event(
        "response.output_text.done",
        json!({
            "type": "response.output_text.done",
            "sequence_number": sequence_number,
            "item_id": item_id,
            "output_index": 0,
            "content_index": 0,
            "text": text,
        }),
    )
}

fn response_terminal_event(
    event_name: &'static str,
    sequence_number: u64,
    public_id: OpenAiResponseId,
    created: u64,
    model: String,
    status: OpenAiResponseStatus,
    text: &str,
) -> Event {
    response_event(
        event_name,
        json!({
            "type": event_name,
            "sequence_number": sequence_number,
            "response": response_object(public_id, created, model, status, text),
        }),
    )
}

fn response_event(event_name: &'static str, payload: serde_json::Value) -> Event {
    data_event(Some(event_name), &payload)
}

fn openai_error_event(error: OpenAiCompatHttpError) -> Event {
    data_event(Some("error"), error.body())
}

fn response_stream_error_event(error: OpenAiCompatHttpError) -> Event {
    let body = compat_error_body(error.body());
    data_event(
        Some("error"),
        &json!({
            "type": "error",
            "error": body.get("error").cloned().unwrap_or_else(generic_error_value),
        }),
    )
}

fn data_event<T: Serialize>(event_name: Option<&'static str>, payload: &T) -> Event {
    let data = serde_json::to_string(payload).unwrap_or_else(|_| generic_error_json());
    let event = Event::default().data(data);
    if let Some(event_name) = event_name {
        event.event(event_name)
    } else {
        event
    }
}

fn compat_error_body(body: &OpenAiCompatErrorResponse) -> serde_json::Value {
    serde_json::to_value(body).unwrap_or_else(|_| json!({ "error": generic_error_value() }))
}

fn generic_error_json() -> String {
    json!({ "error": generic_error_value() }).to_string()
}

fn generic_error_value() -> serde_json::Value {
    json!({
        "message": OpenAiCompatErrorCode::InternalError.sanitized_message(),
        "type": "server_error",
        "param": null,
        "code": "internal_error",
    })
}

fn response_object(
    public_id: OpenAiResponseId,
    created_at: u64,
    model: String,
    status: OpenAiResponseStatus,
    text: &str,
) -> OpenAiResponseObject {
    let output = if text.is_empty() {
        Vec::new()
    } else {
        vec![OpenAiResponseOutputItem::Message {
            id: format!("msg_{}", public_id.as_str()),
            status: Some(OpenAiResponseOutputItemStatus::Completed),
            role: OpenAiResponsesMessageRole::Assistant,
            content: json!([{ "type": "output_text", "text": text }]),
        }]
    };
    let error = if matches!(status, OpenAiResponseStatus::Failed) {
        Some(OpenAiResponseErrorObject::from_kind(
            OpenAiCompatErrorKind::Internal,
        ))
    } else {
        None
    };
    OpenAiResponseObject {
        id: public_id,
        object: "response".to_string(),
        created_at,
        status,
        model,
        output,
        error,
        incomplete_details: None,
        usage: None,
    }
}
