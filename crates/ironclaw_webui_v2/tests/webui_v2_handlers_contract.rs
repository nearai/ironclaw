//! Caller-level contract tests for the WebChat v2 axum handlers.
//!
//! Per `.claude/rules/testing.md` "Test Through the Caller", these tests
//! drive a real axum [`Router`] (built from [`webui_v2_router`]) against a
//! stub [`RebornServicesApi`] so the regression target is the wire
//! contract — body shape, path/query plumbing, error mapping — not just
//! the facade method bodies that are already covered in
//! `ironclaw_product_workflow`.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalConversationRef, FinalReplyView, ProductAdapterId,
    ProductOutboundEnvelope, ProductOutboundPayload, ProductOutboundTarget, ProjectionCursor,
};
use ironclaw_product_workflow::{
    ExtensionName, RebornCancelRunResponse, RebornCreateThreadResponse, RebornGetRunStateRequest,
    RebornGetRunStateResponse, RebornListThreadsResponse, RebornResolveGateResponse,
    RebornResumeGateResponse, RebornServicesApi, RebornServicesError, RebornServicesErrorCode,
    RebornSetupExtensionResponse, RebornSetupExtensionStatus, RebornStreamEventsRequest,
    RebornStreamEventsResponse, RebornSubmitTurnResponse, RebornTimelineRequest,
    RebornTimelineResponse, WebUiAuthenticatedCaller, WebUiCancelRunRequest,
    WebUiCreateThreadRequest, WebUiListThreadsRequest, WebUiResolveGateRequest,
    WebUiSendMessageRequest, WebUiSetupExtensionRequest,
};
use ironclaw_threads::SessionThreadRecord;
use ironclaw_turns::{
    EventCursor, ReplyTargetBindingRef, RunProfileId, RunProfileVersion, TurnRunId, TurnStatus,
};
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
    /// Per-call queued responses for `stream_events`. When non-empty, the
    /// front entry is popped and returned on each call so SSE tests can
    /// drive the handler through specific projection envelopes, error
    /// branches, or empty drains in a deterministic order.
    next_stream_events: Mutex<VecDeque<Result<RebornStreamEventsResponse, RebornServicesError>>>,
    stream_events_notify: Arc<Notify>,
}

impl StubServices {
    fn fail_create_thread(&self, error: RebornServicesError) {
        *self.next_create_thread_error.lock().expect("lock") = Some(error);
    }

    /// Queue one response for the next `stream_events` call. Tests use this
    /// to drive the SSE handler through programmable projection envelopes
    /// or error branches. Falls back to an empty `Ok` drain when the queue
    /// is empty.
    fn enqueue_stream_events(
        &self,
        response: Result<RebornStreamEventsResponse, RebornServicesError>,
    ) {
        self.next_stream_events
            .lock()
            .expect("lock")
            .push_back(response);
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
            next_cursor: None,
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
        if let Some(response) = self.next_stream_events.lock().expect("lock").pop_front() {
            return response;
        }
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

    async fn list_threads(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiListThreadsRequest,
    ) -> Result<RebornListThreadsResponse, RebornServicesError> {
        Ok(RebornListThreadsResponse {
            threads: Vec::new(),
            next_cursor: None,
        })
    }

    async fn setup_extension(
        &self,
        _caller: WebUiAuthenticatedCaller,
        extension_name: ExtensionName,
        _request: WebUiSetupExtensionRequest,
    ) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
        Ok(RebornSetupExtensionResponse {
            extension_name,
            status: RebornSetupExtensionStatus::NotImplemented,
            payload: None,
        })
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

// Regression for the timeline pagination wire plumbing. Per
// `.claude/rules/testing.md` "Test Through the Caller", a facade-only
// test on `get_timeline` is not enough — the Query<TimelineQuery>
// extractor sits between the URL and the facade, and a future refactor
// that drops or renames a query field would only fail here.
#[tokio::test]
async fn get_timeline_forwards_query_params_to_facade() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(
                    "/api/webchat/v2/threads/thread-x/timeline?limit=42&cursor=opaque-from-browser",
                )
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let calls = services.get_timeline_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].thread_id, "thread-x");
    assert_eq!(calls[0].limit, Some(42), "?limit= must reach the facade");
    assert_eq!(
        calls[0].cursor.as_deref(),
        Some("opaque-from-browser"),
        "?cursor= must reach the facade"
    );
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

// Regression for the typed-internals review (Medium): the
// `extension_name` route segment must be validated against
// `ExtensionName` at the handler/facade boundary so the typed value
// is what crosses into the facade contract — not a raw `String`. A
// well-formed name reaches the facade and the typed identifier
// round-trips into the response.
#[tokio::test]
async fn setup_extension_dispatches_typed_extension_name_to_facade() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/extensions/telegram/setup")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"action":"begin"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(
        body["extension_name"], "telegram",
        "facade must echo the typed extension name from the path",
    );
    assert_eq!(body["status"], "not_implemented");
}

// Companion to the typed-internals fix: a malformed identifier in
// the route path must be rejected at the handler/facade boundary
// before the facade is called, with the same `invalid_request` wire
// shape any other inbound validation failure produces. Without
// boundary validation, a path like `../etc` would silently flow
// into the facade as a raw `String` and the typed-internals rule in
// `.claude/rules/types.md` would be broken in practice.
#[tokio::test]
async fn setup_extension_rejects_malformed_extension_name_with_400() {
    let services = Arc::new(StubServices::default());
    let router = router_with(services.clone());

    // `..` triggers `IdentityError::PathTraversal` in `ExtensionName::new`.
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/extensions/..%2Fbad/setup")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_json(response).await;
    assert_eq!(body["error"], "invalid_request");
    assert_eq!(body["field"], "extension_name");
    assert_eq!(body["validation_code"], "invalid_id");
    assert_eq!(body["retryable"], false);
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

// Regression for the WS-shares-SSE-pool review (Medium): the WS
// transport must draw from the same `SseCapacity` pool as the SSE
// transport for the same `(tenant, user)`. If they kept independent
// counters, a caller could open `cap` SSE streams *and* `cap` WS
// streams concurrently — doubling the backend `stream_events` drain
// the cap is supposed to bound.
//
// The PR description claims this shared-pool semantic; this test
// locks it in by making the pool size 1, consuming the only slot
// with an held-open SSE response, then asserting a same-caller WS
// upgrade attempt returns 429 until the SSE body is dropped.
#[tokio::test]
async fn stream_events_ws_shares_capacity_with_sse_streams() {
    let services: Arc<dyn RebornServicesApi> = Arc::new(StubServices::default());
    // Pool size 1: any one open stream (SSE or WS) must exhaust the
    // budget for the caller.
    let router = webui_v2_router(WebUiV2State::with_sse_concurrency_limit(services, 1))
        .layer(axum::Extension(caller()));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let serve_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    // Step 1: consume the only slot with a held-open SSE connection
    // via a low-level reqwest-style raw HTTP GET. We use plain TCP
    // so we can hold the response open without consuming the body
    // — the `SseSlot` guard lives inside the response body and is
    // released only when the stream drops.
    let mut sse_stream = tokio::net::TcpStream::connect(addr).await.expect("tcp");
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    sse_stream
        .write_all(
            b"GET /api/webchat/v2/threads/thread-x/events HTTP/1.1\r\n\
              Host: localhost\r\n\
              Accept: text/event-stream\r\n\
              Connection: keep-alive\r\n\
              \r\n",
        )
        .await
        .expect("write sse request");
    // Read just enough to confirm we got a 200 OK + the start of
    // headers; this guarantees the handler ran `try_acquire`.
    let mut header_buf = [0u8; 512];
    let n = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        sse_stream.read(&mut header_buf),
    )
    .await
    .expect("sse header read within 5s")
    .expect("sse header read");
    let header_prefix = std::str::from_utf8(&header_buf[..n]).expect("utf8 headers");
    assert!(
        header_prefix.starts_with("HTTP/1.1 200"),
        "SSE handshake must return 200; got: {header_prefix:?}",
    );

    // Step 2: same-caller WS upgrade must hit the shared cap. Use a
    // real WS handshake against the same listener; the upgrade
    // response carries the 429 from `try_acquire` before any frames
    // flow.
    let ws_url = format!("ws://{addr}/api/webchat/v2/threads/thread-x/ws");
    let ws_attempt = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio_tungstenite::connect_async(ws_url.clone()),
    )
    .await
    .expect("ws connect attempt within 5s");
    match ws_attempt {
        Ok((_ws, response)) => panic!(
            "WS upgrade must be rejected while SSE holds the only slot; \
             instead the server returned status {} and completed the upgrade",
            response.status().as_u16(),
        ),
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            assert_eq!(
                response.status().as_u16(),
                429,
                "WS upgrade must hit the same per-caller cap as SSE",
            );
        }
        Err(other) => panic!("WS upgrade failed with unexpected error: {other:?}"),
    }

    // Step 3: drop the SSE stream → kernel closes the connection
    // → axum drops the response body → `SseSlot` decrements. After
    // a yield the slot is reusable and the WS upgrade succeeds.
    drop(sse_stream);
    tokio::task::yield_now().await;
    // Give the server task a moment to observe the EOF and drop
    // the body; we cannot await a specific signal, but a short
    // polling loop converges quickly without timing flakiness.
    let recovered = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match tokio_tungstenite::connect_async(ws_url.clone()).await {
                Ok((ws, response)) => return Ok::<_, ()>((ws, response)),
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(25)).await,
            }
        }
    })
    .await
    .expect("WS must complete upgrade within 5s after the SSE slot is released");
    let (mut ws, response) = recovered.expect("recovered ws");
    assert_eq!(
        response.status().as_u16(),
        101,
        "WS must complete the upgrade once the SSE slot has been released",
    );
    let _ = ws.close(None).await;
    serve_handle.abort();
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

// Regression for the "stalled facade drain" review point: SSE_MAX_LIFETIME
// must bound the await on `services.stream_events`, not just the top-of-loop
// check. If a projection drain stalls (e.g. an upstream wedge), an unbounded
// `.await` would keep the `SseSlot` held even after the configured lifetime
// elapses — defeating the per-caller concurrency recovery the cap promises.
//
// Drives a stub whose `stream_events` returns a future that never resolves,
// advances Tokio's virtual time past `SSE_MAX_LIFETIME`, and asserts the
// stream actually terminates and the slot is reusable for a new connection.
#[tokio::test(start_paused = true)]
async fn stream_events_releases_slot_when_facade_drain_stalls_past_max_lifetime() {
    /// Facade whose `stream_events` never returns; all other methods are
    /// unreachable for this regression.
    #[derive(Default)]
    struct StallingServices;

    #[async_trait]
    impl RebornServicesApi for StallingServices {
        async fn create_thread(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: WebUiCreateThreadRequest,
        ) -> Result<RebornCreateThreadResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn submit_turn(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: WebUiSendMessageRequest,
        ) -> Result<RebornSubmitTurnResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn get_timeline(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: RebornTimelineRequest,
        ) -> Result<RebornTimelineResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn stream_events(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: RebornStreamEventsRequest,
        ) -> Result<RebornStreamEventsResponse, RebornServicesError> {
            // Never resolves — simulates a wedged projection stream.
            std::future::pending().await
        }
        async fn cancel_run(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: WebUiCancelRunRequest,
        ) -> Result<RebornCancelRunResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn resolve_gate(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: WebUiResolveGateRequest,
        ) -> Result<RebornResolveGateResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn get_run_state(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: RebornGetRunStateRequest,
        ) -> Result<RebornGetRunStateResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn list_threads(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: WebUiListThreadsRequest,
        ) -> Result<RebornListThreadsResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
        async fn setup_extension(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _extension_name: ExtensionName,
            _request: WebUiSetupExtensionRequest,
        ) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
            unreachable!("not exercised by this test")
        }
    }

    // Cap of 1 so we can observe slot release directly: a second open
    // returns 429 while the first is held, and 200 once it's released.
    let services: Arc<dyn RebornServicesApi> = Arc::new(StallingServices);
    let router = webui_v2_router(WebUiV2State::with_sse_concurrency_limit(services, 1))
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

    // First open: handler acquires the slot and constructs the SSE body.
    let first = open_stream().await.expect("first oneshot");
    assert_eq!(first.status(), StatusCode::OK);

    // Spawn a task that drains the body so the SSE generator actually runs
    // and reaches the `tokio::time::timeout(...)` against the stalled drain.
    let mut first_body = first.into_body();
    let body_task = tokio::spawn(async move { while (first_body.frame().await).is_some() {} });

    // Yield so the spawned body poll runs at least once and parks inside
    // the drain timeout future.
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    // While the only stream is stalled, opening another must hit the cap.
    let blocked = open_stream().await.expect("blocked oneshot");
    assert_eq!(
        blocked.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "with the only stream stalled inside the drain, the slot must be held"
    );
    drop(blocked);

    // Advance virtual time past SSE_MAX_LIFETIME. The drain timeout fires,
    // the generator returns, the `SseSlot` Drop releases the slot.
    tokio::time::advance(Duration::from_secs(6 * 60)).await;

    // Body task completes when the generator returns. Cap with a real
    // timeout in case the body hangs (would surface a regression cleanly).
    tokio::time::timeout(Duration::from_secs(2), body_task)
        .await
        .expect("body task must complete after SSE_MAX_LIFETIME elapses")
        .expect("body task joined cleanly");

    // Slot must now be free; a fresh open succeeds.
    let recovered = open_stream().await.expect("oneshot after slot release");
    assert_eq!(
        recovered.status(),
        StatusCode::OK,
        "slot must be released after the lifetime budget bounds the stalled drain"
    );
    drop(recovered);
}

/// Build a minimal `ProductOutboundEnvelope` with a caller-supplied
/// projection cursor and reply text. The exact payload shape is not the
/// contract under test (it lives in `ironclaw_product_adapters`); these
/// tests only care that whatever the facade hands back becomes a
/// well-formed SSE event.
fn make_projection_envelope(cursor: &str, text: &str) -> ProductOutboundEnvelope {
    ProductOutboundEnvelope::new(
        ProductAdapterId::new("webui_v2").expect("adapter id"), // safety: literal valid id
        AdapterInstallationId::new("install:alpha").expect("install id"), // safety: literal valid id
        ProductOutboundTarget::new(
            ReplyTargetBindingRef::new("reply:fake").expect("reply ref"), // safety: literal valid ref
            ExternalConversationRef::new(None, "conv-1", None, None).expect("conv ref"), // safety: literal valid ref
            None,
        ),
        ProjectionCursor::new(cursor).expect("cursor"), // safety: test-supplied
        ProductOutboundPayload::FinalReply(FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: text.into(),
            generated_at: chrono::Utc::now(),
        }),
    )
}

/// One parsed SSE event from the wire bytes. `event:`, `id:`, and `data:`
/// fields are extracted; everything else (comments, keep-alives) is
/// ignored.
#[derive(Default, Debug)]
struct ParsedSseEvent {
    event: Option<String>,
    id: Option<String>,
    data: Option<String>,
}

/// Minimal SSE chunk parser tailored to the handler's emit shape. The
/// handler writes each event as `event: <name>\n[id: <cursor>\n]data:
/// <json>\n\n`; this helper splits the buffer on the blank-line
/// separator and pulls out the three fields. It is deliberately not a
/// general SSE parser — the handler's emit shape is fixed and any drift
/// would be the regression the surrounding tests are pinning.
fn parse_sse_events(bytes: &[u8]) -> Vec<ParsedSseEvent> {
    let text = String::from_utf8_lossy(bytes);
    let mut events = Vec::new();
    for block in text.split("\n\n") {
        let block = block.trim_matches(|c| c == '\n' || c == '\r');
        if block.is_empty() {
            continue;
        }
        let mut parsed = ParsedSseEvent::default();
        for line in block.split('\n') {
            let line = line.trim_end_matches('\r');
            if let Some(rest) = line.strip_prefix("event:") {
                parsed.event = Some(rest.trim_start().to_string());
            } else if let Some(rest) = line.strip_prefix("id:") {
                parsed.id = Some(rest.trim_start().to_string());
            } else if let Some(rest) = line.strip_prefix("data:") {
                parsed.data = Some(rest.trim_start().to_string());
            }
        }
        if parsed.event.is_some() || parsed.data.is_some() {
            events.push(parsed);
        }
    }
    events
}

/// Pull body frames until the predicate fires or the timeout elapses,
/// returning whatever bytes were collected. SSE bodies in axum surface as
/// a stream of frames where each frame is a single `\n\n`-terminated
/// event; tests want to inspect the wire shape after N events arrive.
async fn collect_sse_until<F>(body: &mut Body, timeout: Duration, mut done: F) -> Vec<u8>
where
    F: FnMut(&[u8]) -> bool,
{
    let deadline = std::time::Instant::now() + timeout;
    let mut buf = Vec::<u8>::new();
    while std::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match tokio::time::timeout(remaining, body.frame()).await {
            Ok(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    buf.extend_from_slice(data.as_ref());
                    if done(&buf) {
                        return buf;
                    }
                }
            }
            // Stream closed or errored: return what we have so the caller
            // can still assert on the bytes we collected before close.
            Ok(_) => return buf,
            Err(_) => return buf,
        }
    }
    buf
}

// Regression for the "SSE projection payload shape is untested" review
// (Medium). Pins the *wire* contract the browser sees, not just the
// handler being called: each envelope must emit a `projection` event
// with the JSON-serialized projection cursor as the SSE `id` and the
// JSON-serialized envelope as `data`. Also asserts that the next poll
// carries the *latest* cursor in `after_cursor`, so a future refactor
// that loses cursor advancement (e.g. dropping the loop's bookkeeping)
// breaks loudly.
#[tokio::test]
async fn stream_events_emits_projection_events_with_cursor_ids() {
    let services = Arc::new(StubServices::default());

    let envelope_a = make_projection_envelope("cursor:a", "hello");
    let envelope_b = make_projection_envelope("cursor:b", "world");

    services.enqueue_stream_events(Ok(RebornStreamEventsResponse {
        events: vec![envelope_a.clone(), envelope_b.clone()],
    }));
    // Second drain is empty: lets the test observe `after_cursor`
    // advancement on the follow-up call without producing more events.
    services.enqueue_stream_events(Ok(RebornStreamEventsResponse { events: Vec::new() }));

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

    // Pump frames directly in this task — the body cannot be moved to a
    // background task and then dropped, since dropping kills the SSE
    // generator before the second `stream_events` call can run. Instead,
    // keep awaiting frames in-place, accumulating bytes, until we have
    // both (a) the two emitted SSE events and (b) the second drain call
    // observed via `services.stream_events_calls`.
    let mut body = response.into_body();
    let mut bytes = Vec::<u8>::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let have_events = bytes.windows(2).filter(|w| *w == b"\n\n").count() >= 2;
        let saw_second_call = services.stream_events_calls.lock().expect("lock").len() >= 2;
        if have_events && saw_second_call {
            break;
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match tokio::time::timeout(remaining, body.frame()).await {
            Ok(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    bytes.extend_from_slice(data.as_ref());
                }
            }
            _ => break,
        }
    }
    drop(body);

    let events = parse_sse_events(&bytes);
    assert!(
        events.len() >= 2,
        "expected at least two SSE events, got: {events:?}; raw: {}",
        String::from_utf8_lossy(&bytes)
    );

    let cursor_a_json =
        serde_json::to_string(envelope_a.projection_cursor()).expect("cursor-a json");
    let cursor_b_json =
        serde_json::to_string(envelope_b.projection_cursor()).expect("cursor-b json");

    assert_eq!(events[0].event.as_deref(), Some("projection"));
    assert_eq!(events[0].id.as_deref(), Some(cursor_a_json.as_str()));
    let envelope_a_json: Value =
        serde_json::from_str(events[0].data.as_deref().expect("data")).expect("envelope a json");
    let expected_a: Value = serde_json::to_value(&envelope_a).expect("envelope a to value");
    assert_eq!(
        envelope_a_json, expected_a,
        "envelope wire shape must match"
    );

    assert_eq!(events[1].event.as_deref(), Some("projection"));
    assert_eq!(events[1].id.as_deref(), Some(cursor_b_json.as_str()));
    let envelope_b_json: Value =
        serde_json::from_str(events[1].data.as_deref().expect("data")).expect("envelope b json");
    let expected_b: Value = serde_json::to_value(&envelope_b).expect("envelope b to value");
    assert_eq!(envelope_b_json, expected_b);

    let calls = services.stream_events_calls.lock().expect("lock").clone();
    assert!(
        calls.len() >= 2,
        "second poll must occur so cursor advancement is observable; saw {} call(s)",
        calls.len()
    );
    assert_eq!(
        calls[1].after_cursor.as_ref(),
        Some(envelope_b.projection_cursor()),
        "second poll must advance after_cursor to the last emitted cursor"
    );
}

// Regression for the "SSE facade error event path is untested" review
// (Medium). When `RebornServicesApi::stream_events` returns Err, the
// handler must emit one SSE `error` frame carrying only the redacted
// `error` code + `retryable` flag (no `field`, no internal `detail`),
// then close the stream — never propagate an HTTP error on a long-lived
// SSE connection because the browser would replay it as a hard
// reconnect failure.
#[tokio::test]
async fn stream_events_facade_error_emits_redacted_error_event_and_closes() {
    let services = Arc::new(StubServices::default());
    services.enqueue_stream_events(Err(RebornServicesError {
        code: RebornServicesErrorCode::Forbidden,
        status_code: 403,
        retryable: false,
        // The handler must NOT echo these into the SSE payload — the
        // redacted shape carries only `error` + `retryable`.
        field: Some("thread_id".into()),
        validation_code: None,
    }));

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

    // The handler must surface the facade error as an SSE event, not as a
    // failed HTTP open. EventSource cannot recover from a non-OK status.
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "SSE open must succeed even when the facade drain errors; the error path is an in-stream event"
    );

    let mut body = response.into_body();
    // Read until we see an `error` event chunk, or the stream closes.
    let bytes = collect_sse_until(&mut body, Duration::from_secs(2), |buf| {
        buf.windows(b"event: error".len())
            .any(|w| w == b"event: error")
            && buf.windows(2).any(|w| w == b"\n\n")
    })
    .await;

    let events = parse_sse_events(&bytes);
    let error_event = events
        .iter()
        .find(|event| event.event.as_deref() == Some("error"))
        .unwrap_or_else(|| {
            panic!(
                "expected an SSE `error` event, got: {events:?}; raw: {}",
                String::from_utf8_lossy(&bytes)
            )
        });
    let payload: Value = serde_json::from_str(error_event.data.as_deref().expect("error data"))
        .expect("error data is JSON");
    assert_eq!(
        payload["error"], "forbidden",
        "error event must carry the redacted error code"
    );
    assert_eq!(
        payload["retryable"], false,
        "error event must carry the retryable flag verbatim"
    );
    assert!(
        payload.get("field").is_none(),
        "redacted SSE error payload must not leak the failing field name"
    );
    assert!(
        payload.get("validation_code").is_none(),
        "redacted SSE error payload must not leak validation metadata"
    );

    // The stream closes after the error event. Polling once more must
    // return `None` (end-of-stream) within a small budget.
    let final_frame = tokio::time::timeout(Duration::from_millis(500), body.frame()).await;
    let closed = matches!(final_frame, Ok(None) | Err(_));
    assert!(
        closed,
        "facade error must close the SSE stream, but body.frame() yielded another chunk"
    );
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

// Regression for the "WS transport's projection payload + redacted
// error frame untested" review (Medium). The composition crate's WS
// caller-level test verifies the upgrade returns 101, but only a real
// WS connection that pumps frames can catch breakage in the
// per-envelope JSON serialization, cursor advancement on the
// `after_cursor` field, or the redacted error frame the handler emits
// on facade failure.
#[tokio::test]
async fn stream_events_ws_emits_projection_frames_and_redacted_error() {
    use futures::StreamExt;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let services = Arc::new(StubServices::default());

    let envelope_a = make_projection_envelope("cursor:a", "hello");
    let envelope_b = make_projection_envelope("cursor:b", "world");
    services.enqueue_stream_events(Ok(RebornStreamEventsResponse {
        events: vec![envelope_a.clone(), envelope_b.clone()],
    }));
    // After draining the two real events, the next drain produces a
    // facade error so the handler exercises the redacted-error-frame +
    // close path before lifetime expiry.
    services.enqueue_stream_events(Err(RebornServicesError {
        code: RebornServicesErrorCode::Unavailable,
        status_code: 503,
        retryable: true,
        field: None,
        validation_code: None,
    }));

    let router = router_with(services.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let serve_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let url = format!("ws://{addr}/api/webchat/v2/threads/thread-x/ws");
    let (mut ws, response) = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(url),
    )
    .await
    .expect("ws connect within 5s")
    .expect("ws upgrade");
    assert_eq!(response.status().as_u16(), 101);

    // Read frames until we see both projection envelopes and the
    // redacted error frame, or the stream closes.
    let mut text_frames: Vec<String> = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline && text_frames.len() < 3 {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match tokio::time::timeout(remaining, ws.next()).await {
            Ok(Some(Ok(WsMessage::Text(text)))) => text_frames.push(text.to_string()),
            Ok(Some(Ok(WsMessage::Close(_)))) | Ok(None) => break,
            Ok(Some(Ok(_))) => continue, // ignore ping/pong/binary
            Ok(Some(Err(_))) => break,
            Err(_) => break,
        }
    }
    let _ = ws.close(None).await;
    serve_handle.abort();

    assert!(
        text_frames.len() >= 3,
        "expected projection envelopes + error frame; got {} text frame(s): {:?}",
        text_frames.len(),
        text_frames,
    );

    // First two frames carry the projection envelopes, in order.
    let envelope_a_json: Value = serde_json::from_str(&text_frames[0]).expect("envelope a parses");
    let expected_a: Value = serde_json::to_value(&envelope_a).expect("envelope a value");
    assert_eq!(
        envelope_a_json, expected_a,
        "first WS frame must carry the first ProductOutboundEnvelope verbatim",
    );
    let envelope_b_json: Value = serde_json::from_str(&text_frames[1]).expect("envelope b parses");
    let expected_b: Value = serde_json::to_value(&envelope_b).expect("envelope b value");
    assert_eq!(envelope_b_json, expected_b);

    // Third frame is the redacted error payload — `error` code +
    // `retryable` flag only. No `detail`, `field`, `validation_code`,
    // or any internal diagnostic must leak through.
    let error_json: Value =
        serde_json::from_str(&text_frames[2]).expect("error frame parses as json");
    assert_eq!(error_json["error"], serde_json::json!("unavailable"));
    assert_eq!(error_json["retryable"], serde_json::json!(true));
    assert!(
        error_json.get("detail").is_none(),
        "redacted error frame must not carry server diagnostics",
    );
    assert!(error_json.get("field").is_none());
    assert!(error_json.get("validation_code").is_none());

    // The handler must have advanced `after_cursor` between the two
    // drains so the browser would resume from cursor:b on reconnect.
    let calls = services.stream_events_calls.lock().expect("lock").clone();
    assert!(
        calls.len() >= 2,
        "second poll must occur for the redacted-error path to fire",
    );
    assert_eq!(
        calls[1].after_cursor.as_ref(),
        Some(envelope_b.projection_cursor()),
        "second WS poll must advance after_cursor to the last emitted projection cursor",
    );
}
