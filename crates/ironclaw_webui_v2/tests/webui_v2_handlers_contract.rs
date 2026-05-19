//! Caller-level contract tests for the WebChat v2 axum handlers.
//!
//! Per `.claude/rules/testing.md` "Test Through the Caller", these tests
//! drive a real axum [`Router`] (built from [`webui_v2_router`]) against a
//! stub [`RebornServicesApi`] so the regression target is the wire
//! contract — body shape, path/query plumbing, error mapping — not just
//! the facade method bodies that are already covered in
//! `ironclaw_product_workflow`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_product_workflow::{
    RebornCancelRunResponse, RebornCreateThreadResponse, RebornGetRunStateRequest,
    RebornGetRunStateResponse, RebornResolveGateResponse, RebornResumeGateResponse,
    RebornServicesApi, RebornServicesError, RebornServicesErrorCode, RebornStreamEventsRequest,
    RebornStreamEventsResponse, RebornSubmitTurnResponse, RebornTimelineRequest,
    RebornTimelineResponse, WebUiAuthenticatedCaller, WebUiCancelRunRequest,
    WebUiCreateThreadRequest, WebUiResolveGateRequest, WebUiSendMessageRequest,
};
use ironclaw_threads::SessionThreadRecord;
use ironclaw_turns::{EventCursor, RunProfileId, RunProfileVersion, TurnRunId, TurnStatus};
use ironclaw_webui_v2::{WebUiV2State, webui_v2_router};
use serde_json::Value;
use tokio::sync::Notify;
use tower::ServiceExt;

fn caller() -> WebUiAuthenticatedCaller {
    WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("user-alpha").expect("user"),
        Some(AgentId::new("agent-alpha").expect("agent")),
        Some(ProjectId::new("project-alpha").expect("project")),
    )
}

fn router_with(services: Arc<dyn RebornServicesApi>) -> Router {
    webui_v2_router(WebUiV2State::new(services))
        // Production composition runs the bearer-token middleware that
        // constructs this `Extension`; tests bypass auth and inject the
        // caller directly so the regression target is the handler itself.
        .layer(axum::Extension(caller()))
}

#[derive(Default)]
struct StubServices {
    create_thread_calls: Mutex<Vec<WebUiCreateThreadRequest>>,
    submit_turn_calls: Mutex<Vec<WebUiSendMessageRequest>>,
    get_timeline_calls: Mutex<Vec<RebornTimelineRequest>>,
    stream_events_calls: Mutex<Vec<RebornStreamEventsRequest>>,
    cancel_run_calls: Mutex<Vec<WebUiCancelRunRequest>>,
    resolve_gate_calls: Mutex<Vec<WebUiResolveGateRequest>>,
    next_create_thread_error: Mutex<Option<RebornServicesError>>,
    stream_events_notify: Arc<Notify>,
}

impl StubServices {
    fn fail_create_thread(&self, error: RebornServicesError) {
        *self.next_create_thread_error.lock().expect("lock") = Some(error);
    }

    /// Triggered the first time `stream_events` is invoked. Lets the SSE
    /// test wait on the actual facade call rather than guessing at a
    /// timeout — axum's SSE body is lazy, so the handler does not run
    /// until the client polls the body.
    fn stream_events_signal(&self) -> Arc<Notify> {
        self.stream_events_notify.clone()
    }
}

#[async_trait]
impl RebornServicesApi for StubServices {
    async fn create_thread(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: WebUiCreateThreadRequest,
    ) -> Result<RebornCreateThreadResponse, RebornServicesError> {
        self.create_thread_calls
            .lock()
            .expect("lock")
            .push(request.clone());
        if let Some(error) = self.next_create_thread_error.lock().expect("lock").take() {
            return Err(error);
        }
        Ok(RebornCreateThreadResponse {
            thread: SessionThreadRecord {
                thread_id: ironclaw_host_api::ThreadId::new("thread:fake").expect("thread id"),
                scope: ironclaw_threads::ThreadScope {
                    tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
                    agent_id: AgentId::new("agent-alpha").expect("agent"),
                    project_id: Some(ProjectId::new("project-alpha").expect("project")),
                    owner_user_id: Some(UserId::new("user-alpha").expect("user")),
                    mission_id: None,
                },
                created_by_actor_id: "user-alpha".to_string(),
                title: None,
                metadata_json: request
                    .client_action_id
                    .as_ref()
                    .map(|id| format!("{{\"client_action_id\":\"{id}\"}}")),
            },
        })
    }

    async fn submit_turn(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: WebUiSendMessageRequest,
    ) -> Result<RebornSubmitTurnResponse, RebornServicesError> {
        self.submit_turn_calls
            .lock()
            .expect("lock")
            .push(request.clone());
        Ok(RebornSubmitTurnResponse::Submitted {
            thread_id: ironclaw_host_api::ThreadId::new(
                request.thread_id.clone().unwrap_or_default(),
            )
            .expect("thread id"),
            accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new("msg:fake").expect("ref"),
            turn_id: "turn:fake".to_string(),
            run_id: TurnRunId::new(),
            status: TurnStatus::Queued,
            resolved_run_profile_id: RunProfileId::default_profile().as_str().to_string(),
            resolved_run_profile_version: RunProfileVersion::new(1).as_u64(),
            event_cursor: EventCursor(1),
        })
    }

    async fn get_timeline(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: RebornTimelineRequest,
    ) -> Result<RebornTimelineResponse, RebornServicesError> {
        self.get_timeline_calls
            .lock()
            .expect("lock")
            .push(request.clone());
        Ok(RebornTimelineResponse {
            thread: SessionThreadRecord {
                thread_id: ironclaw_host_api::ThreadId::new(request.thread_id.clone())
                    .expect("thread id"),
                scope: ironclaw_threads::ThreadScope {
                    tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
                    agent_id: AgentId::new("agent-alpha").expect("agent"),
                    project_id: Some(ProjectId::new("project-alpha").expect("project")),
                    owner_user_id: Some(UserId::new("user-alpha").expect("user")),
                    mission_id: None,
                },
                created_by_actor_id: "user-alpha".to_string(),
                title: None,
                metadata_json: None,
            },
            messages: Vec::new(),
            summary_artifacts: Vec::new(),
        })
    }

    async fn stream_events(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, RebornServicesError> {
        self.stream_events_calls
            .lock()
            .expect("lock")
            .push(request.clone());
        self.stream_events_notify.notify_waiters();
        // Empty drain; SSE handler will keep-alive until the test drops it.
        Ok(RebornStreamEventsResponse { events: Vec::new() })
    }

    async fn get_run_state(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornGetRunStateRequest,
    ) -> Result<RebornGetRunStateResponse, RebornServicesError> {
        // Not exercised by any current handler test — `get_run_state` is on
        // the facade trait but not wired to a WebChat v2 HTTP route. Fail
        // loud rather than fabricate a response so a future caller-level
        // test that forgets to program this path can't quietly pass.
        Err(RebornServicesError {
            code: RebornServicesErrorCode::Internal,
            status_code: 500,
            retryable: false,
            field: None,
            validation_code: None,
        })
    }

    async fn cancel_run(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: WebUiCancelRunRequest,
    ) -> Result<RebornCancelRunResponse, RebornServicesError> {
        self.cancel_run_calls
            .lock()
            .expect("lock")
            .push(request.clone());
        Ok(RebornCancelRunResponse {
            run_id: TurnRunId::new(),
            status: TurnStatus::Cancelled,
            event_cursor: EventCursor(2),
            already_terminal: false,
        })
    }

    async fn resolve_gate(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: WebUiResolveGateRequest,
    ) -> Result<RebornResolveGateResponse, RebornServicesError> {
        self.resolve_gate_calls
            .lock()
            .expect("lock")
            .push(request.clone());
        Ok(RebornResolveGateResponse::Resumed(
            RebornResumeGateResponse {
                run_id: TurnRunId::new(),
                status: TurnStatus::Queued,
                event_cursor: EventCursor(3),
            },
        ))
    }
}

async fn read_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body bytes");
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes.as_ref()).into_owned()))
}

#[tokio::test]
async fn create_thread_dispatches_through_facade() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"client_action_id":"act-1"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert!(body["thread"]["thread_id"].is_string(), "thread_id present");
    assert_eq!(
        services.create_thread_calls.lock().expect("lock").len(),
        1,
        "facade called exactly once"
    );
}

#[tokio::test]
async fn send_message_path_overrides_body_thread_id() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads/thread-from-path/messages")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"client_action_id":"act-1","thread_id":"thread-from-body","content":"hi"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let calls = services.submit_turn_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].thread_id.as_deref(),
        Some("thread-from-path"),
        "path segment must win over body field"
    );
}

#[tokio::test]
async fn get_timeline_threads_path_into_request() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-x/timeline")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let calls = services.get_timeline_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].thread_id, "thread-x");
}

#[tokio::test]
async fn cancel_run_path_overrides_body_run_id() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads/thread-x/runs/run-from-path/cancel")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"client_action_id":"cancel-1","thread_id":"other","run_id":"run-from-body","reason":"user_requested"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let calls = services.cancel_run_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].thread_id.as_deref(), Some("thread-x"));
    assert_eq!(calls[0].run_id.as_deref(), Some("run-from-path"));
}

#[tokio::test]
async fn resolve_gate_path_overrides_body_gate_ref() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(
                    "/api/webchat/v2/threads/thread-x/runs/run-y/gates/gate-from-path/resolve",
                )
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"client_action_id":"gate-1","thread_id":"other","run_id":"other","gate_ref":"gate-from-body","resolution":"approved"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let calls = services.resolve_gate_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].thread_id.as_deref(), Some("thread-x"));
    assert_eq!(calls[0].run_id.as_deref(), Some("run-y"));
    assert_eq!(calls[0].gate_ref.as_deref(), Some("gate-from-path"));
}

#[tokio::test]
async fn create_thread_error_maps_to_http_status() {
    let services = Arc::new(StubServices::default());
    services.fail_create_thread(RebornServicesError {
        code: RebornServicesErrorCode::Forbidden,
        status_code: 403,
        retryable: false,
        field: None,
        validation_code: None,
    });
    let router = router_with(services);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"client_action_id":"act-1"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = read_json(response).await;
    assert_eq!(body["error"], "forbidden");
    assert_eq!(body["retryable"], false);
}

#[tokio::test]
async fn stream_events_emits_sse_content_type_and_drains_facade() {
    let services = Arc::new(StubServices::default());
    let signal = services.stream_events_signal();
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-x/events")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.starts_with("text/event-stream"),
        "SSE content type expected, got: {content_type}"
    );

    // The SSE body is lazy — drive it by polling the first frame, which
    // forces the handler's stream future to run. Notify resolves the
    // moment the stub's stream_events is hit, decoupling the assertion
    // from the SSE polling cadence.
    let mut body = response.into_body();
    let _poll = tokio::spawn(async move {
        let _ = body.frame().await;
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), signal.notified())
        .await
        .expect("stream_events must be called within 2s after the body is polled");

    let calls = services.stream_events_calls.lock().expect("lock").len();
    assert!(
        calls >= 1,
        "facade.stream_events must be called at least once after the first SSE frame is read"
    );
}

#[tokio::test]
async fn stream_events_last_event_id_header_takes_precedence_over_query() {
    // Two distinct, parseable cursors so the precedence is observable in
    // the captured RebornStreamEventsRequest — if a future refactor flips
    // the `.or()` order, the facade will see cursor-B and this test fails.
    let header_cursor =
        ironclaw_product_workflow::ProjectionCursor::new("cursor-from-header").expect("cursor");
    let query_cursor =
        ironclaw_product_workflow::ProjectionCursor::new("cursor-from-query").expect("cursor");
    let header_json = serde_json::to_string(&header_cursor).expect("serialize header cursor");
    let query_json = serde_json::to_string(&query_cursor).expect("serialize query cursor");
    let query_encoded = url_encode(&query_json);

    let services = Arc::new(StubServices::default());
    let signal = services.stream_events_signal();
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/webchat/v2/threads/thread-x/events?after_cursor={query_encoded}"
                ))
                .header("Last-Event-ID", header_json)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let _poll = tokio::spawn(async move {
        let _ = body.frame().await;
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), signal.notified())
        .await
        .expect("stream_events must be called within 2s after the body is polled");

    let calls = services.stream_events_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1, "facade.stream_events called exactly once");
    assert_eq!(
        calls[0].after_cursor.as_ref(),
        Some(&header_cursor),
        "Last-Event-ID header must win over ?after_cursor= query param"
    );
}

fn url_encode(value: &str) -> String {
    // Minimal application/x-www-form-urlencoded helper: percent-encode every
    // byte that is not an unreserved character per RFC 3986. Avoids pulling
    // in a urlencoding dep just for one test value.
    let mut out = String::with_capacity(value.len() * 3);
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

// Regression for the per-caller SSE concurrency review (Medium): once the
// router is mounted, an authenticated caller must not be able to keep
// opening long-lived `EventSource` connections beyond the configured cap
// — even though each new request stays under the descriptor's per-caller
// rate limit. Without the cap, sustained reconnects would multiply
// backend `stream_events` drains at `connections × poll-interval`.
#[tokio::test]
async fn stream_events_caps_concurrent_streams_per_caller() {
    let services: Arc<dyn RebornServicesApi> = Arc::new(StubServices::default());
    // Use a low custom cap so the test runs without burning resources.
    let router = webui_v2_router(WebUiV2State::with_sse_concurrency_limit(services, 2))
        .layer(axum::Extension(caller()));

    let open_stream = || {
        router.clone().oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-x/events")
                .body(Body::empty())
                .expect("request"),
        )
    };

    let first = open_stream().await.expect("first oneshot");
    assert_eq!(first.status(), StatusCode::OK);
    let second = open_stream().await.expect("second oneshot");
    assert_eq!(second.status(), StatusCode::OK);

    // Third open must hit the cap. Keep the first two responses alive so
    // their slots stay reserved — the SSE generator (and the slot it
    // owns) lives inside the response body.
    let third = open_stream().await.expect("third oneshot");
    assert_eq!(
        third.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "third concurrent open from same caller must be rejected"
    );
    let body = read_json(third).await;
    assert_eq!(body["error"], "rate_limited");
    assert_eq!(body["retryable"], true);

    // Release the first stream — slot returns to the pool.
    drop(first);
    // The SSE body's drop chain runs synchronously, but yield once so any
    // pending wakers settle before we measure recovery.
    tokio::task::yield_now().await;

    let recovered = open_stream().await.expect("oneshot after release");
    assert_eq!(
        recovered.status(),
        StatusCode::OK,
        "slot must be reusable after the earlier stream is dropped"
    );

    drop(second);
    drop(recovered);
}

#[tokio::test]
async fn missing_caller_extension_returns_500() {
    // No `Extension(caller)` layer — exercises the failure mode if host
    // composition forgets to run the bearer middleware.
    let services: Arc<dyn RebornServicesApi> = Arc::new(StubServices::default());
    let router = webui_v2_router(WebUiV2State::new(services));

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"client_action_id":"act-1"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    // axum's `Extension` extractor maps a missing extension to 500.
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "missing caller extension must fail closed, not bypass auth"
    );

    // Drain the body to make sure no facade method was hit before the
    // extractor failed.
    let _ = response.into_body().collect().await.expect("drain body");
}
