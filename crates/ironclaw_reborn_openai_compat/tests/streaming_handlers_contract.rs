#![cfg(feature = "openai-compat-beta")]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use http::Request;
use http_body_util::BodyExt;
use ironclaw_host_api::{TenantId, ThreadId, UserId};
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, ExternalConversationRef, FakeProductWorkflow,
    FinalReplyView, ProductAdapterId, ProductInboundAck, ProductInboundEnvelope,
    ProductOutboundEnvelope, ProductOutboundPayload, ProductOutboundTarget, ProductProjectionItem,
    ProductProjectionState, ProductProjectionSubscribeInput, ProductWorkflow, ProjectionCursor,
    ProjectionSubscriptionRequest, ProtocolAuthEvidence,
};
use ironclaw_reborn_openai_compat::{
    InMemoryOpenAiCompatRefStore, OpenAiChatCompletionProjection,
    OpenAiChatCompletionProjectionReader, OpenAiChatCompletionProjectionRequest,
    OpenAiChatCompletionsWorkflow, OpenAiChatProjectionStreamRequest, OpenAiCompatActorScope,
    OpenAiCompatAuthenticatedCaller, OpenAiCompatHttpError, OpenAiCompatProjectionStreamer,
    OpenAiCompatRouterState, OpenAiResponseId, OpenAiResponseObject,
    OpenAiResponseProjectionStreamRequest, OpenAiResponseReadRequest, OpenAiResponseStatus,
    OpenAiResponseWaitRequest, OpenAiResponsesProjectionReader, OpenAiResponsesWorkflow,
    openai_compat_router_with_state,
};
use ironclaw_turns::{AcceptedMessageRef, ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};
use serde_json::json;
use tower::ServiceExt;

#[tokio::test]
async fn chat_stream_emits_openai_chunks_without_projection_cursor() {
    let streamer = Arc::new(QueuedStreamer::new());
    streamer.push_chat(vec![projection_text_envelope("chat-1", "hello")]);
    let router = router(streamer.clone());

    let response = router
        .oneshot(post_json(
            "/v1/chat/completions",
            json!({
                "model": "gpt-reborn",
                "stream": true,
                "messages": [{"role": "user", "content": "hello"}]
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::OK);
    let raw = read_until(response, "hello").await;
    assert!(raw.contains("chat.completion.chunk"), "raw SSE: {raw}");
    assert!(raw.contains("\"content\":\"hello\""), "raw SSE: {raw}");
    assert!(!raw.contains("ProjectionCursor"), "raw SSE: {raw}");
    assert!(!raw.contains("cursor:chat-1"), "raw SSE: {raw}");
    assert_eq!(streamer.chat_calls(), 1);
}

#[tokio::test]
async fn responses_stream_emits_response_events_without_projection_cursor() {
    let streamer = Arc::new(QueuedStreamer::new());
    streamer.push_response(vec![projection_text_envelope("resp-1", "hello")]);
    let router = router(streamer.clone());

    let response = router
        .oneshot(post_json(
            "/api/v1/responses",
            json!({"model": "gpt-reborn", "stream": true, "input": "hello"}),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::OK);
    let raw = read_until(response, "response.output_text.delta").await;
    assert!(raw.contains("event: response.created"), "raw SSE: {raw}");
    assert!(
        raw.contains("event: response.output_text.delta"),
        "raw SSE: {raw}"
    );
    assert!(raw.contains("\"delta\":\"hello\""), "raw SSE: {raw}");
    assert!(!raw.contains("cursor:resp-1"), "raw SSE: {raw}");
    assert_eq!(streamer.response_calls(), 1);
}

#[tokio::test]
async fn keepalive_is_suppressed_and_non_monotonic_rebase_fails_safely() {
    let streamer = Arc::new(QueuedStreamer::new());
    streamer.push_response(vec![keepalive_envelope("keepalive")]);
    streamer.push_response(vec![projection_text_envelope("first", "hello")]);
    streamer.push_response(vec![projection_text_envelope("rebase", "he")]);
    let router = router(streamer);

    let response = router
        .oneshot(post_json(
            "/v1/responses",
            json!({"model": "gpt-reborn", "stream": true, "input": "hello"}),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::OK);
    let raw = read_until(response, "event: error").await;
    assert!(
        raw.contains("event: response.output_text.delta"),
        "raw SSE: {raw}"
    );
    assert!(raw.contains("event: error"), "raw SSE: {raw}");
    assert!(!raw.contains("keep_alive"), "raw SSE: {raw}");
    assert!(!raw.contains("cursor:rebase"), "raw SSE: {raw}");
    assert!(!raw.contains("RebaseRequired"), "raw SSE: {raw}");
    assert!(!raw.contains("Lagged"), "raw SSE: {raw}");
    assert!(!raw.contains("SECRET_TOKEN"), "raw SSE: {raw}");
}

#[tokio::test]
async fn chat_stream_non_monotonic_rebase_fails_safely() {
    let streamer = Arc::new(QueuedStreamer::new());
    streamer.push_chat(vec![projection_text_envelope("first", "hello")]);
    streamer.push_chat(vec![projection_text_envelope("rebase", "jello")]);
    let router = router(streamer);

    let response = router
        .oneshot(post_json(
            "/v1/chat/completions",
            json!({
                "model": "gpt-reborn",
                "stream": true,
                "messages": [{"role": "user", "content": "hello"}]
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::OK);
    let raw = read_until(response, "event: error").await;
    assert!(raw.contains("\"content\":\"hello\""), "raw SSE: {raw}");
    assert!(raw.contains("event: error"), "raw SSE: {raw}");
    assert!(
        raw.contains("\"code\":\"internal_error\""),
        "raw SSE: {raw}"
    );
    assert!(!raw.contains("cursor:rebase"), "raw SSE: {raw}");
    assert!(!raw.contains("RebaseRequired"), "raw SSE: {raw}");
}

#[tokio::test]
async fn chat_stream_completes_on_terminal_run_status_projection() {
    let streamer = Arc::new(QueuedStreamer::new());
    streamer.push_chat(vec![
        projection_text_envelope("chat-terminal-text", "hello"),
        run_status_envelope("chat-terminal-done", "completed"),
    ]);
    let router = router(streamer);

    let response = router
        .oneshot(post_json(
            "/v1/chat/completions",
            json!({
                "model": "gpt-reborn",
                "stream": true,
                "messages": [{"role": "user", "content": "hello"}]
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::OK);
    let raw = read_until(response, "[DONE]").await;
    assert!(raw.contains("\"content\":\"hello\""), "raw SSE: {raw}");
    assert!(raw.contains("\"finish_reason\":\"stop\""), "raw SSE: {raw}");
    assert!(raw.contains("[DONE]"), "raw SSE: {raw}");
}

#[tokio::test]
async fn chat_stream_completes_on_final_reply_payload() {
    let streamer = Arc::new(QueuedStreamer::new());
    streamer.push_chat(vec![final_reply_envelope("chat-final", "hello final")]);
    let router = router(streamer);

    let response = router
        .oneshot(post_json(
            "/v1/chat/completions",
            json!({
                "model": "gpt-reborn",
                "stream": true,
                "messages": [{"role": "user", "content": "hello"}]
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::OK);
    let raw = read_until(response, "[DONE]").await;
    assert!(
        raw.contains("\"content\":\"hello final\""),
        "raw SSE: {raw}"
    );
    assert!(raw.contains("\"finish_reason\":\"stop\""), "raw SSE: {raw}");
    assert!(raw.contains("[DONE]"), "raw SSE: {raw}");
}

#[tokio::test]
async fn chat_stream_errors_on_failed_run_status() {
    let streamer = Arc::new(QueuedStreamer::new());
    streamer.push_chat(vec![run_status_envelope("chat-failed", "failed")]);
    let router = router(streamer);

    let response = router
        .oneshot(post_json(
            "/v1/chat/completions",
            json!({
                "model": "gpt-reborn",
                "stream": true,
                "messages": [{"role": "user", "content": "hello"}]
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::OK);
    let raw = read_until(response, "event: error").await;
    assert!(
        raw.contains("\"code\":\"internal_error\""),
        "raw SSE: {raw}"
    );
    assert!(!raw.contains("failure_summary"), "raw SSE: {raw}");
}

#[tokio::test]
async fn chat_stream_drain_error_emits_sanitized_in_stream_error() {
    let streamer = Arc::new(QueuedStreamer::new());
    streamer.push_chat_error(OpenAiCompatHttpError::internal());
    let router = router(streamer);

    let response = router
        .oneshot(post_json(
            "/v1/chat/completions",
            json!({
                "model": "gpt-reborn",
                "stream": true,
                "messages": [{"role": "user", "content": "hello"}]
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::OK);
    let raw = read_until(response, "event: error").await;
    assert!(
        raw.contains("\"code\":\"internal_error\""),
        "raw SSE: {raw}"
    );
    assert!(!raw.contains("SECRET_TOKEN"), "raw SSE: {raw}");
}

#[tokio::test]
async fn chat_stream_times_out_when_projection_stalls() {
    let streamer = Arc::new(QueuedStreamer::new());
    let router = router_with_wait_timeout(streamer, Duration::from_millis(5));

    let response = router
        .oneshot(post_json(
            "/v1/chat/completions",
            json!({
                "model": "gpt-reborn",
                "stream": true,
                "messages": [{"role": "user", "content": "hello"}]
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::OK);
    let raw = read_until(response, "event: error").await;
    assert!(
        raw.contains("\"code\":\"service_unavailable\""),
        "raw SSE: {raw}"
    );
}

#[tokio::test]
async fn responses_stream_completes_on_terminal_run_status_projection() {
    let streamer = Arc::new(QueuedStreamer::new());
    streamer.push_response(vec![
        projection_text_envelope("resp-terminal-text", "hello"),
        run_status_envelope("resp-terminal-done", "completed"),
    ]);
    let router = router(streamer);

    let response = router
        .oneshot(post_json(
            "/api/v1/responses",
            json!({"model": "gpt-reborn", "stream": true, "input": "hello"}),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::OK);
    let raw = read_until(response, "event: response.completed").await;
    assert!(
        raw.contains("event: response.output_text.done"),
        "raw SSE: {raw}"
    );
    assert!(raw.contains("event: response.completed"), "raw SSE: {raw}");
    assert!(raw.contains("\"status\":\"completed\""), "raw SSE: {raw}");
}

#[tokio::test]
async fn responses_stream_completes_on_final_reply_payload() {
    let streamer = Arc::new(QueuedStreamer::new());
    streamer.push_response(vec![final_reply_envelope("resp-final", "hello final")]);
    let router = router(streamer);

    let response = router
        .oneshot(post_json(
            "/api/v1/responses",
            json!({"model": "gpt-reborn", "stream": true, "input": "hello"}),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::OK);
    let raw = read_until(response, "event: response.completed").await;
    assert!(
        raw.contains("event: response.output_text.done"),
        "raw SSE: {raw}"
    );
    assert!(raw.contains("event: response.completed"), "raw SSE: {raw}");
    assert!(raw.contains("\"text\":\"hello final\""), "raw SSE: {raw}");
    assert!(raw.contains("\"status\":\"completed\""), "raw SSE: {raw}");
}

#[tokio::test]
async fn responses_stream_emits_failed_event_on_failed_run_status() {
    let streamer = Arc::new(QueuedStreamer::new());
    streamer.push_response(vec![run_status_envelope("resp-failed", "failed")]);
    let router = router(streamer);

    let response = router
        .oneshot(post_json(
            "/api/v1/responses",
            json!({"model": "gpt-reborn", "stream": true, "input": "hello"}),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::OK);
    let raw = read_until(response, "event: response.failed").await;
    assert!(raw.contains("event: response.failed"), "raw SSE: {raw}");
    assert!(raw.contains("\"status\":\"failed\""), "raw SSE: {raw}");
    assert!(
        raw.contains("\"code\":\"internal_error\""),
        "raw SSE: {raw}"
    );
    assert!(!raw.contains("failure_summary"), "raw SSE: {raw}");
}

#[tokio::test]
async fn responses_stream_emits_cancelled_event_on_cancelled_run_status() {
    let streamer = Arc::new(QueuedStreamer::new());
    streamer.push_response(vec![run_status_envelope("resp-cancelled", "cancelled")]);
    let router = router(streamer);

    let response = router
        .oneshot(post_json(
            "/api/v1/responses",
            json!({"model": "gpt-reborn", "stream": true, "input": "hello"}),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::OK);
    let raw = read_until(response, "event: response.cancelled").await;
    assert!(raw.contains("event: response.cancelled"), "raw SSE: {raw}");
    assert!(raw.contains("\"status\":\"cancelled\""), "raw SSE: {raw}");
    assert!(!raw.contains("failure_summary"), "raw SSE: {raw}");
}

#[tokio::test]
async fn responses_stream_omits_text_done_when_completed_without_text() {
    let streamer = Arc::new(QueuedStreamer::new());
    streamer.push_response(vec![run_status_envelope("resp-empty-done", "completed")]);
    let router = router(streamer);

    let response = router
        .oneshot(post_json(
            "/api/v1/responses",
            json!({"model": "gpt-reborn", "stream": true, "input": "hello"}),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::OK);
    let raw = read_until(response, "event: response.completed").await;
    assert!(!raw.contains("response.output_text.done"), "raw SSE: {raw}");
    assert!(raw.contains("\"status\":\"completed\""), "raw SSE: {raw}");
}

#[tokio::test]
async fn responses_stream_times_out_when_projection_stalls() {
    let streamer = Arc::new(QueuedStreamer::new());
    let router = router_with_wait_timeout(streamer, Duration::from_millis(5));

    let response = router
        .oneshot(post_json(
            "/api/v1/responses",
            json!({"model": "gpt-reborn", "stream": true, "input": "hello"}),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::OK);
    let raw = read_until(response, "event: error").await;
    assert!(
        raw.contains("\"code\":\"service_unavailable\""),
        "raw SSE: {raw}"
    );
}

#[tokio::test]
async fn stream_true_without_wired_streamer_returns_not_wired() {
    let router = router_without_streamer();

    let chat = router
        .clone()
        .oneshot(post_json(
            "/v1/chat/completions",
            json!({
                "model": "gpt-reborn",
                "stream": true,
                "messages": [{"role": "user", "content": "hello"}]
            }),
        ))
        .await
        .expect("chat response");
    assert_eq!(chat.status(), http::StatusCode::NOT_IMPLEMENTED);
    let chat_body = response_body(chat).await;
    assert!(
        chat_body.contains("\"code\":\"unsupported\""),
        "{chat_body}"
    );

    let responses = router
        .oneshot(post_json(
            "/api/v1/responses",
            json!({"model": "gpt-reborn", "stream": true, "input": "hello"}),
        ))
        .await
        .expect("responses response");
    assert_eq!(responses.status(), http::StatusCode::NOT_IMPLEMENTED);
    let responses_body = response_body(responses).await;
    assert!(
        responses_body.contains("\"code\":\"unsupported\""),
        "{responses_body}"
    );
}

#[tokio::test]
async fn chat_stream_idempotency_replay_uses_recorded_ack_without_resubmit() {
    let streamer = Arc::new(QueuedStreamer::new());
    streamer.push_chat(vec![run_status_envelope("first-done", "completed")]);
    streamer.push_chat(vec![run_status_envelope("replay-done", "completed")]);
    let workflow = Arc::new(FakeProductWorkflow::new());
    workflow.program_projection_resolution(sample_projection_subscription_request());
    let router = router_with_workflow(streamer.clone(), workflow.clone());
    let body = json!({
        "model": "gpt-reborn",
        "stream": true,
        "messages": [{"role": "user", "content": "hello"}]
    });

    let first = router
        .clone()
        .oneshot(post_json_with_key(
            "/v1/chat/completions",
            body.clone(),
            "same-key",
        ))
        .await
        .expect("first");
    assert_eq!(first.status(), http::StatusCode::OK);
    let _ = read_until(first, "[DONE]").await;

    let replay = router
        .oneshot(post_json_with_key("/v1/chat/completions", body, "same-key"))
        .await
        .expect("replay");
    assert_eq!(replay.status(), http::StatusCode::OK);
    let _ = read_until(replay, "[DONE]").await;

    assert_eq!(workflow.accepted_count(), 1);
    let requests = streamer.chat_requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| { matches!(&request.accepted_ack, ProductInboundAck::Accepted { .. }) })
    );
}

#[tokio::test]
async fn chat_stream_idempotency_retries_pending_mapping_after_busy() {
    let streamer = Arc::new(QueuedStreamer::new());
    let workflow = Arc::new(FixedAckWorkflow::new(deferred_busy_ack()));
    let router = router_with_workflow(streamer, workflow.clone());
    let body = json!({
        "model": "gpt-reborn",
        "stream": true,
        "messages": [{"role": "user", "content": "hello"}]
    });

    let first = router
        .clone()
        .oneshot(post_json_with_key(
            "/v1/chat/completions",
            body.clone(),
            "busy-key",
        ))
        .await
        .expect("first");
    let retry = router
        .oneshot(post_json_with_key("/v1/chat/completions", body, "busy-key"))
        .await
        .expect("retry");

    assert_eq!(first.status(), http::StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(retry.status(), http::StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(workflow.seen_count(), 2);
}

#[tokio::test]
async fn chat_stream_rejected_busy_ack_returns_429() {
    let streamer = Arc::new(QueuedStreamer::new());
    let workflow = Arc::new(FixedAckWorkflow::new(rejected_busy_ack()));
    let router = router_with_workflow(streamer, workflow.clone());
    let body = json!({
        "model": "gpt-reborn",
        "stream": true,
        "messages": [{"role": "user", "content": "hello"}]
    });

    let response = router
        .oneshot(post_json_with_key(
            "/v1/chat/completions",
            body,
            "rejected-busy-key",
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(workflow.seen_count(), 1);
}

#[derive(Default)]
struct QueuedStreamer {
    chat: Mutex<VecDeque<Result<Vec<ProductOutboundEnvelope>, OpenAiCompatHttpError>>>,
    response: Mutex<VecDeque<Result<Vec<ProductOutboundEnvelope>, OpenAiCompatHttpError>>>,
    chat_calls: Mutex<usize>,
    response_calls: Mutex<usize>,
    chat_requests: Mutex<Vec<OpenAiChatProjectionStreamRequest>>,
    response_requests: Mutex<Vec<OpenAiResponseProjectionStreamRequest>>,
}

impl QueuedStreamer {
    fn new() -> Self {
        Self::default()
    }

    fn push_chat(&self, envelopes: Vec<ProductOutboundEnvelope>) {
        self.chat.lock().expect("lock").push_back(Ok(envelopes));
    }

    fn push_chat_error(&self, error: OpenAiCompatHttpError) {
        self.chat.lock().expect("lock").push_back(Err(error));
    }

    fn push_response(&self, envelopes: Vec<ProductOutboundEnvelope>) {
        self.response.lock().expect("lock").push_back(Ok(envelopes));
    }

    fn chat_calls(&self) -> usize {
        *self.chat_calls.lock().expect("lock")
    }

    fn response_calls(&self) -> usize {
        *self.response_calls.lock().expect("lock")
    }

    fn chat_requests(&self) -> Vec<OpenAiChatProjectionStreamRequest> {
        self.chat_requests.lock().expect("lock").clone()
    }

    #[allow(dead_code)]
    fn response_requests(&self) -> Vec<OpenAiResponseProjectionStreamRequest> {
        self.response_requests.lock().expect("lock").clone()
    }
}

#[async_trait]
impl OpenAiCompatProjectionStreamer for QueuedStreamer {
    async fn drain_chat(
        &self,
        request: OpenAiChatProjectionStreamRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, OpenAiCompatHttpError> {
        *self.chat_calls.lock().expect("lock") += 1;
        self.chat_requests.lock().expect("lock").push(request);
        Ok(self
            .chat
            .lock()
            .expect("lock")
            .pop_front()
            .transpose()?
            .unwrap_or_default())
    }

    async fn drain_response(
        &self,
        request: OpenAiResponseProjectionStreamRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, OpenAiCompatHttpError> {
        *self.response_calls.lock().expect("lock") += 1;
        self.response_requests.lock().expect("lock").push(request);
        Ok(self
            .response
            .lock()
            .expect("lock")
            .pop_front()
            .transpose()?
            .unwrap_or_default())
    }
}

struct FixedAckWorkflow {
    ack: ProductInboundAck,
    seen_envelopes: Mutex<Vec<ProductInboundEnvelope>>,
}

impl FixedAckWorkflow {
    fn new(ack: ProductInboundAck) -> Self {
        Self {
            ack,
            seen_envelopes: Mutex::new(Vec::new()),
        }
    }

    fn seen_count(&self) -> usize {
        self.seen_envelopes.lock().expect("workflow lock").len()
    }
}

#[async_trait]
impl ProductWorkflow for FixedAckWorkflow {
    async fn submit_inbound(
        &self,
        envelope: ProductInboundEnvelope,
    ) -> Result<ProductInboundAck, ironclaw_product_adapters::ProductAdapterError> {
        self.seen_envelopes
            .lock()
            .expect("workflow lock")
            .push(envelope);
        Ok(self.ack.clone())
    }

    async fn subscribe_projection(
        &self,
        _request: ProductProjectionSubscribeInput,
    ) -> Result<ProjectionSubscriptionRequest, ironclaw_product_adapters::ProductAdapterError> {
        Ok(sample_projection_subscription_request())
    }
}

struct StaticChatReader;

#[async_trait]
impl OpenAiChatCompletionProjectionReader for StaticChatReader {
    async fn read_chat_completion_projection(
        &self,
        _request: OpenAiChatCompletionProjectionRequest,
    ) -> Result<OpenAiChatCompletionProjection, OpenAiCompatHttpError> {
        Ok(OpenAiChatCompletionProjection::text("unused"))
    }
}

struct StaticResponsesReader;

#[async_trait]
impl OpenAiResponsesProjectionReader for StaticResponsesReader {
    async fn wait_for_response_completion(
        &self,
        request: OpenAiResponseWaitRequest,
    ) -> Result<ironclaw_reborn_openai_compat::OpenAiResponseProjection, OpenAiCompatHttpError>
    {
        Ok(
            ironclaw_reborn_openai_compat::OpenAiResponseProjection::new(completed_response(
                request.public_id,
                "unused",
            )),
        )
    }

    async fn read_response(
        &self,
        request: OpenAiResponseReadRequest,
    ) -> Result<OpenAiResponseObject, OpenAiCompatHttpError> {
        Ok(completed_response(request.public_id, "unused"))
    }
}

fn router(streamer: Arc<QueuedStreamer>) -> axum::Router {
    router_with_workflow(streamer, fake_workflow())
}

fn router_with_wait_timeout(streamer: Arc<QueuedStreamer>, wait_timeout: Duration) -> axum::Router {
    router_with_options(Some(streamer), fake_workflow(), wait_timeout)
}

fn router_with_workflow(
    streamer: Arc<QueuedStreamer>,
    workflow: Arc<dyn ProductWorkflow>,
) -> axum::Router {
    router_with_options(Some(streamer), workflow, Duration::from_secs(30))
}

fn router_without_streamer() -> axum::Router {
    router_with_options(None, fake_workflow(), Duration::from_secs(30))
}

fn router_with_options(
    streamer: Option<Arc<QueuedStreamer>>,
    workflow: Arc<dyn ProductWorkflow>,
    wait_timeout: Duration,
) -> axum::Router {
    let ref_store = Arc::new(InMemoryOpenAiCompatRefStore::new());
    let mut chat = OpenAiChatCompletionsWorkflow::new(
        workflow.clone(),
        ref_store.clone(),
        Arc::new(StaticChatReader),
    )
    .with_wait_timeout(wait_timeout);
    let mut responses =
        OpenAiResponsesWorkflow::new(workflow, ref_store, Arc::new(StaticResponsesReader))
            .with_wait_timeout(wait_timeout);
    if let Some(streamer) = streamer {
        chat = chat.with_projection_streamer(streamer.clone());
        responses = responses.with_projection_streamer(streamer);
    }
    openai_compat_router_with_state(
        OpenAiCompatRouterState::default()
            .with_chat_completions_workflow(Arc::new(chat))
            .with_responses_workflow(Arc::new(responses)),
    )
}

fn fake_workflow() -> Arc<FakeProductWorkflow> {
    let workflow = Arc::new(FakeProductWorkflow::new());
    workflow.program_projection_resolution(sample_projection_subscription_request());
    workflow
}

fn sample_projection_subscription_request() -> ProjectionSubscriptionRequest {
    ProjectionSubscriptionRequest {
        actor: TurnActor::new(UserId::new("user-a").expect("user")),
        scope: TurnScope::new_with_owner(
            TenantId::new("tenant-a").expect("tenant"),
            None,
            None,
            ThreadId::new("thread-openai-stream").expect("thread"),
            Some(UserId::new("user-a").expect("user")),
        ),
        after_cursor: None,
    }
}

fn post_json(path: &str, body: serde_json::Value) -> Request<Body> {
    post_json_request(path, body, None)
}

fn post_json_with_key(path: &str, body: serde_json::Value, idempotency_key: &str) -> Request<Body> {
    post_json_request(path, body, Some(idempotency_key))
}

fn post_json_request(
    path: &str,
    body: serde_json::Value,
    idempotency_key: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .extension(caller());
    if let Some(idempotency_key) = idempotency_key {
        builder = builder.header("idempotency-key", idempotency_key);
    }
    builder.body(Body::from(body.to_string())).expect("request")
}

fn caller() -> OpenAiCompatAuthenticatedCaller {
    OpenAiCompatAuthenticatedCaller::new(
        OpenAiCompatActorScope::new(
            TenantId::new("tenant-a").expect("tenant"),
            UserId::new("user-a").expect("user"),
            None,
            None,
        ),
        ProtocolAuthEvidence::test_verified_for_tenant(
            AuthRequirement::BearerToken,
            "user-a",
            TenantId::new("tenant-a").expect("tenant"),
        ),
    )
    .expect("caller")
}

async fn read_until(response: axum::response::Response, needle: &str) -> String {
    let mut body = response.into_body();
    let mut raw = String::new();
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(2), body.frame())
            .await
            .expect("timed out waiting for SSE frame")
            .expect("body frame")
            .expect("frame result");
        if let Some(data) = frame.data_ref() {
            raw.push_str(std::str::from_utf8(data).expect("utf8 SSE"));
            if raw.contains(needle) {
                return raw;
            }
        }
    }
}

async fn response_body(response: axum::response::Response) -> String {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    String::from_utf8(bytes.to_vec()).expect("utf8 body")
}

fn projection_text_envelope(cursor: &str, text: &str) -> ProductOutboundEnvelope {
    envelope(
        cursor,
        ProductOutboundPayload::ProjectionUpdate {
            state: ProductProjectionState::new(
                "thread-a",
                vec![ProductProjectionItem::Text {
                    id: format!("text-{cursor}"),
                    body: text.to_string(),
                }],
            )
            .expect("projection state"),
        },
    )
}

fn final_reply_envelope(cursor: &str, text: &str) -> ProductOutboundEnvelope {
    envelope(
        cursor,
        ProductOutboundPayload::FinalReply(FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: text.to_string(),
            generated_at: chrono::Utc::now(),
        }),
    )
}

fn run_status_envelope(cursor: &str, status: &str) -> ProductOutboundEnvelope {
    envelope(
        cursor,
        ProductOutboundPayload::ProjectionUpdate {
            state: ProductProjectionState::new(
                "thread-a",
                vec![ProductProjectionItem::RunStatus {
                    run_id: TurnRunId::new(),
                    status: status.to_string(),
                    failure_category: None,
                    failure_summary: None,
                }],
            )
            .expect("projection state"),
        },
    )
}

fn keepalive_envelope(cursor: &str) -> ProductOutboundEnvelope {
    envelope(cursor, ProductOutboundPayload::KeepAlive)
}

fn envelope(cursor: &str, payload: ProductOutboundPayload) -> ProductOutboundEnvelope {
    ProductOutboundEnvelope::new(
        ProductAdapterId::new("openai_compat").expect("adapter id"),
        AdapterInstallationId::new("openai_compat_default").expect("installation id"),
        ProductOutboundTarget::new(
            ReplyTargetBindingRef::new("reply:test").expect("reply target"),
            ExternalConversationRef::new(None, "conversation:test", None, None)
                .expect("conversation ref"),
            None,
        ),
        ProjectionCursor::new(format!("cursor:{cursor}")).expect("cursor"),
        payload,
    )
}

fn deferred_busy_ack() -> ProductInboundAck {
    ProductInboundAck::DeferredBusy {
        accepted_message_ref: AcceptedMessageRef::new("msg:busy").expect("accepted ref"),
        active_run_id: TurnRunId::new(),
    }
}

fn rejected_busy_ack() -> ProductInboundAck {
    ProductInboundAck::RejectedBusy {
        accepted_message_ref: AcceptedMessageRef::new("msg:rejected-busy").expect("accepted ref"),
        active_run_id: None,
    }
}

fn completed_response(public_id: OpenAiResponseId, text: &str) -> OpenAiResponseObject {
    OpenAiResponseObject {
        id: public_id,
        object: "response".to_string(),
        created_at: 1,
        status: OpenAiResponseStatus::Completed,
        model: "gpt-reborn".to_string(),
        output: vec![],
        error: None,
        incomplete_details: None,
        usage: Some(ironclaw_reborn_openai_compat::OpenAiResponseUsage {
            input_tokens: 1,
            output_tokens: text.len() as u32,
            total_tokens: 1 + text.len() as u32,
        }),
    }
}
