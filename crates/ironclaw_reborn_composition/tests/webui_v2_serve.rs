//! Caller-level tests for the Reborn-owned WebChat v2 HTTP gateway
//! composition (`webui_serve`).
//!
//! These tests drive [`webui_v2_app`] through `tower::ServiceExt::oneshot`
//! so the middleware stack — bearer auth, `?token=` shim for SSE,
//! CORS, body limit, static security headers — is exercised end-to-end
//! against the same axum `Router` `serve_webui_v2` binds at runtime.
//! No TCP listener and no real Reborn runtime are required; the v2
//! facade is mocked so the regression target stays the gateway-layer
//! composition.

#![cfg(feature = "webui-v2-beta")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_workflow::{
    RebornCancelRunResponse, RebornCreateThreadResponse, RebornGetRunStateRequest,
    RebornGetRunStateResponse, RebornResolveGateResponse, RebornServicesApi, RebornServicesError,
    RebornServicesErrorCode, RebornStreamEventsRequest, RebornStreamEventsResponse,
    RebornSubmitTurnResponse, RebornTimelineRequest, RebornTimelineResponse,
    WebUiAuthenticatedCaller, WebUiCancelRunRequest, WebUiCreateThreadRequest,
    WebUiResolveGateRequest, WebUiSendMessageRequest,
};
use ironclaw_reborn_composition::{
    RebornReadiness, RebornWebuiBundle, WebuiAuthenticator, WebuiServeConfig, webui_v2_app,
};
use ironclaw_threads::{SessionThreadRecord, ThreadScope};
use ironclaw_turns::{EventCursor, RunProfileId, RunProfileVersion, TurnRunId, TurnStatus};
use serde_json::json;
use tower::ServiceExt;

const TENANT: &str = "tenant-alpha";
const USER: &str = "user-alpha";
const VALID_TOKEN: &str = "valid-bearer-token";

// ─── stubs ────────────────────────────────────────────────────────────

/// `WebuiAuthenticator` accepting only [`VALID_TOKEN`].
struct OnlyValidToken;

#[async_trait]
impl WebuiAuthenticator for OnlyValidToken {
    async fn authenticate(&self, token: &str) -> Option<UserId> {
        if token == VALID_TOKEN {
            Some(UserId::new(USER).expect("user id"))
        } else {
            None
        }
    }
}

#[derive(Default)]
struct StubServices {
    create_thread_calls: Mutex<Vec<WebUiAuthenticatedCaller>>,
    stream_events_calls: Mutex<Vec<WebUiAuthenticatedCaller>>,
}

#[async_trait]
impl RebornServicesApi for StubServices {
    async fn create_thread(
        &self,
        caller: WebUiAuthenticatedCaller,
        _request: WebUiCreateThreadRequest,
    ) -> Result<RebornCreateThreadResponse, RebornServicesError> {
        self.create_thread_calls.lock().expect("lock").push(caller);
        Ok(RebornCreateThreadResponse {
            thread: SessionThreadRecord {
                thread_id: ThreadId::new("thread.fake").expect("thread"),
                scope: ThreadScope {
                    tenant_id: TenantId::new(TENANT).expect("tenant"),
                    agent_id: AgentId::new("agent.fake").expect("agent"),
                    project_id: Some(ProjectId::new("project.fake").expect("project")),
                    owner_user_id: Some(UserId::new(USER).expect("user")),
                    mission_id: None,
                },
                created_by_actor_id: USER.to_string(),
                title: None,
                metadata_json: None,
            },
        })
    }

    async fn submit_turn(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: WebUiSendMessageRequest,
    ) -> Result<RebornSubmitTurnResponse, RebornServicesError> {
        Ok(RebornSubmitTurnResponse::Submitted {
            thread_id: ThreadId::new(request.thread_id.clone().unwrap_or_default())
                .expect("thread id"),
            accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new("msg.fake").expect("ref"),
            turn_id: "turn.fake".to_string(),
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
        Ok(RebornTimelineResponse {
            thread: SessionThreadRecord {
                thread_id: ThreadId::new(request.thread_id.clone()).expect("thread id"),
                scope: ThreadScope {
                    tenant_id: TenantId::new(TENANT).expect("tenant"),
                    agent_id: AgentId::new("agent.fake").expect("agent"),
                    project_id: Some(ProjectId::new("project.fake").expect("project")),
                    owner_user_id: Some(UserId::new(USER).expect("user")),
                    mission_id: None,
                },
                created_by_actor_id: USER.to_string(),
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
        caller: WebUiAuthenticatedCaller,
        _request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, RebornServicesError> {
        self.stream_events_calls.lock().expect("lock").push(caller);
        Ok(RebornStreamEventsResponse { events: Vec::new() })
    }

    async fn get_run_state(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornGetRunStateRequest,
    ) -> Result<RebornGetRunStateResponse, RebornServicesError> {
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
        _request: WebUiCancelRunRequest,
    ) -> Result<RebornCancelRunResponse, RebornServicesError> {
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
        _request: WebUiResolveGateRequest,
    ) -> Result<RebornResolveGateResponse, RebornServicesError> {
        Err(RebornServicesError {
            code: RebornServicesErrorCode::Internal,
            status_code: 500,
            retryable: false,
            field: None,
            validation_code: None,
        })
    }
}

// ─── harness ──────────────────────────────────────────────────────────

fn build_app() -> (axum::Router, Arc<StubServices>) {
    let services = Arc::new(StubServices::default());
    let bundle = RebornWebuiBundle {
        api: services.clone(),
        readiness: RebornReadiness::disabled(),
    };
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        Arc::new(OnlyValidToken),
        vec![HeaderValue::from_static("http://localhost:1234")],
    );
    let app = webui_v2_app(bundle, config).expect("webui v2 app");
    (app, services)
}

async fn read_body_string(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body bytes");
    String::from_utf8_lossy(&bytes).into_owned()
}

// ─── tests ────────────────────────────────────────────────────────────

#[tokio::test]
async fn bearer_happy_path_dispatches_to_facade_with_host_tenant() {
    let (app, services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"client_action_id": "act-1"}).to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    let calls = services.create_thread_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1, "facade reached exactly once");
    assert_eq!(calls[0].tenant_id.as_str(), TENANT);
    assert_eq!(calls[0].user_id.as_str(), USER);
}

#[tokio::test]
async fn missing_bearer_returns_401_before_facade() {
    let (app, services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({}).to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        services
            .create_thread_calls
            .lock()
            .expect("lock")
            .is_empty()
    );
}

#[tokio::test]
async fn invalid_bearer_returns_401() {
    let (app, services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header(header::AUTHORIZATION, "Bearer wrong-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({}).to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        services
            .create_thread_calls
            .lock()
            .expect("lock")
            .is_empty()
    );
}

#[tokio::test]
async fn sse_query_token_authenticates_event_stream() {
    let (app, services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/webchat/v2/threads/thread-x/events?token={VALID_TOKEN}"
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream"),
    );
    // The SSE handler runs on the background body task and polls the
    // facade on a 1-second cadence. Pull one frame to drive the
    // generator far enough to record at least the first poll, then
    // drop the body so the long-lived stream does not pin the test.
    let mut body = response.into_body();
    let _ = tokio::time::timeout(Duration::from_secs(2), body.frame()).await;
    drop(body);
    let calls = services.stream_events_calls.lock().expect("lock").clone();
    assert!(
        !calls.is_empty(),
        "?token= shim authenticated the SSE handler (calls={})",
        calls.len(),
    );
    assert_eq!(calls[0].user_id.as_str(), USER);
    assert_eq!(calls[0].tenant_id.as_str(), TENANT);
}

#[tokio::test]
async fn sse_without_bearer_or_query_token_returns_401() {
    let (app, services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-x/events")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        services
            .stream_events_calls
            .lock()
            .expect("lock")
            .is_empty()
    );
}

#[tokio::test]
async fn timeline_route_rejects_query_token_shim() {
    // Mutation / read routes must stay bearer-only — only the SSE
    // endpoint accepts `?token=` (browsers' `EventSource` cannot set
    // headers). A query-token leaked via referer must not authenticate
    // a state read.
    let (app, _services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/webchat/v2/threads/thread-x/timeline?token={VALID_TOKEN}"
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn v2_response_carries_static_security_headers() {
    let (app, _services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({}).to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(
        headers
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .and_then(|v| v.to_str().ok()),
        Some("nosniff"),
    );
    assert_eq!(
        headers
            .get(header::X_FRAME_OPTIONS)
            .and_then(|v| v.to_str().ok()),
        Some("DENY"),
    );
    assert!(
        headers.contains_key("content-security-policy"),
        "CSP header present on v2 responses",
    );
}

#[tokio::test]
async fn cors_does_not_echo_disallowed_origin_on_preflight() {
    let (app, _services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/api/webchat/v2/threads")
                .header("origin", "http://evil.example.com")
                .header("access-control-request-method", "POST")
                .header("access-control-request-headers", "authorization")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let echoed = response
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok());
    assert_ne!(
        echoed,
        Some("http://evil.example.com"),
        "CORS must not echo an attacker-supplied origin",
    );
}

#[tokio::test]
async fn cors_allows_configured_origin() {
    let (app, _services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/api/webchat/v2/threads")
                .header("origin", "http://localhost:1234")
                .header("access-control-request-method", "POST")
                .header("access-control-request-headers", "authorization")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("http://localhost:1234"),
    );
}

#[tokio::test]
async fn malformed_user_id_from_authenticator_rejects_with_401() {
    // If a host authenticator returns a user id that doesn't satisfy
    // `UserId`'s grammar at construction time it never reaches the
    // composition. The authenticator's contract is `Option<UserId>`,
    // so the only way to produce a "malformed" id is to return None —
    // which the composition treats as auth failure. This test locks
    // the contract: a `None` decision becomes 401, never 500.
    struct AlwaysReject;
    #[async_trait]
    impl WebuiAuthenticator for AlwaysReject {
        async fn authenticate(&self, _token: &str) -> Option<UserId> {
            None
        }
    }

    let services = Arc::new(StubServices::default());
    let bundle = RebornWebuiBundle {
        api: services.clone(),
        readiness: RebornReadiness::disabled(),
    };
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        Arc::new(AlwaysReject),
        vec![HeaderValue::from_static("http://localhost:1234")],
    );
    let app = webui_v2_app(bundle, config).expect("app");
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({}).to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        services
            .create_thread_calls
            .lock()
            .expect("lock")
            .is_empty()
    );
    // body content is opaque to clients — just confirm it's the
    // expected 401 string, not an internal traceback.
    let body = read_body_string(response).await;
    assert!(
        body.contains("Invalid or missing auth token"),
        "401 body should be the generic message, got: {body}",
    );
}

#[tokio::test]
async fn mutation_route_returns_429_after_descriptor_rate_limit_exhausted() {
    // `create_thread`'s descriptor declares 60 requests / 60s
    // per-caller. We send 60 successful POSTs from the same bearer
    // token and then expect the 61st to come back 429 — the rate-limit
    // middleware reads the descriptor at composition time, so this
    // test locks the contract that production-shape policies are
    // enforced (not just unit-test stubs).
    let (app, services) = build_app();
    let body = json!({}).to_string();
    let make_request = || {
        Request::builder()
            .method(Method::POST)
            .uri("/api/webchat/v2/threads")
            .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.clone()))
            .expect("request")
    };

    for i in 0..60 {
        let response = app.clone().oneshot(make_request()).await.expect("oneshot");
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "request {i} should be within the mutation budget",
        );
    }

    let response = app.clone().oneshot(make_request()).await.expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "61st mutation should exceed the per-caller rate-limit window",
    );
    let body = read_body_string(response).await;
    assert!(
        body.contains("Rate limit exceeded"),
        "429 body should explain the limit, got: {body}",
    );

    // Auth ran but the rate-limit middleware short-circuited, so the
    // facade only saw the 60 successful requests.
    let facade_calls = services.create_thread_calls.lock().expect("lock").len();
    assert_eq!(
        facade_calls, 60,
        "rate-limit must short-circuit BEFORE the v2 handler",
    );
}

#[tokio::test]
async fn oversized_mutation_body_is_rejected_with_413_before_facade() {
    // `create_thread`'s descriptor caps the body at 16 KiB. Send 16 KiB
    // + 1 of JSON and expect 413 from the per-route body limit, with
    // the facade untouched (the limit middleware sits in front of both
    // auth and the v2 handler).
    let (app, services) = build_app();
    let payload = format!(
        "{{\"client_action_id\":\"act-1\",\"padding\":\"{}\"}}",
        "x".repeat(16 * 1024 + 1)
    );
    assert!(
        payload.len() > 16 * 1024,
        "fixture must exceed the create_thread cap; got {} bytes",
        payload.len()
    );
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(payload))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = read_body_string(response).await;
    assert!(
        body.contains("Request body exceeds the route's body limit."),
        "413 body should explain the cap, got: {body}",
    );
    assert!(
        services
            .create_thread_calls
            .lock()
            .expect("lock")
            .is_empty(),
        "facade must not be reached on an oversized request",
    );
}

#[tokio::test]
async fn mutation_body_within_descriptor_cap_reaches_facade() {
    // Companion to the oversized test: a payload that fits inside the
    // 16 KiB `create_thread` cap should pass through to the facade.
    // Locks the contract that the limit is "above max", not "above
    // some-fraction-of-max".
    let (app, services) = build_app();
    let payload = format!(
        "{{\"client_action_id\":\"act-1\",\"padding\":\"{}\"}}",
        "x".repeat(8 * 1024)
    );
    assert!(payload.len() < 16 * 1024);
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(payload))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        services.create_thread_calls.lock().expect("lock").len(),
        1,
        "facade should be reached for in-budget payload",
    );
}

#[tokio::test]
async fn timeline_route_rejects_nonempty_body_with_413() {
    // `get_timeline`'s descriptor declares `BodyLimitPolicy::NoBody`.
    // A GET with a non-empty body must be rejected upfront — regardless
    // of bearer-token validity — so that the v2 handler never observes
    // a body shape its descriptor said wouldn't arrive.
    let (app, _services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-x/timeline")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("body-not-allowed"))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = read_body_string(response).await;
    assert!(
        body.contains("Request body not allowed for this route."),
        "413 body should name the NoBody policy, got: {body}",
    );
}

#[tokio::test]
async fn rate_limit_is_independent_per_caller() {
    // Two distinct authenticators / users — alice exhausts her budget
    // but bob's requests still get through.
    use ironclaw_reborn_composition::WebuiServeConfig;

    struct UserSwitch;
    #[async_trait]
    impl WebuiAuthenticator for UserSwitch {
        async fn authenticate(&self, token: &str) -> Option<UserId> {
            match token {
                "tok-alice" => Some(UserId::new("alice").expect("user")),
                "tok-bob" => Some(UserId::new("bob").expect("user")),
                _ => None,
            }
        }
    }

    let services = Arc::new(StubServices::default());
    let bundle = RebornWebuiBundle {
        api: services.clone(),
        readiness: RebornReadiness::disabled(),
    };
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        Arc::new(UserSwitch),
        vec![HeaderValue::from_static("http://localhost:1234")],
    );
    let app = webui_v2_app(bundle, config).expect("app");

    let make_request = |token: &str| -> Request<Body> {
        Request::builder()
            .method(Method::POST)
            .uri("/api/webchat/v2/threads")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({}).to_string()))
            .expect("request")
    };

    // Burn alice's full 60-request budget.
    for _ in 0..60 {
        let response = app
            .clone()
            .oneshot(make_request("tok-alice"))
            .await
            .expect("oneshot");
        assert_eq!(response.status(), StatusCode::OK);
    }
    // Next alice request → 429.
    let response = app
        .clone()
        .oneshot(make_request("tok-alice"))
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

    // Bob has a fresh window.
    let response = app
        .clone()
        .oneshot(make_request("tok-bob"))
        .await
        .expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "bob's per-caller budget must be independent of alice's",
    );
}
