//! Integration tests for multi-tenant auth, isolation, and per-user scoping.
//!
//! These tests verify that multi-tenant infrastructure works correctly:
//! - Token-to-identity mapping via MultiAuthState
//! - Per-user SSE event scoping (user A doesn't see user B's events)
//! - Per-user rate limiting (user A exhausting limit doesn't block user B)
//! - Auth middleware inserts correct UserIdentity into request extensions
//! - WebSocket connections are scoped to the authenticated user

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware;
use axum::routing::{get, post};
use tower::ServiceExt;

use ironclaw::channels::IncomingMessage;
use ironclaw::channels::web::auth::{
    AuthenticatedUser, MultiAuthState, UserIdentity, auth_middleware,
};
use ironclaw::channels::web::server::{
    GatewayState, PerUserRateLimiter, RateLimiter, start_server,
};
use ironclaw::channels::web::sse::SseManager;
use ironclaw::channels::web::test_helpers::TestGatewayBuilder;
use ironclaw::channels::web::ws::WsConnectionTracker;
use ironclaw::context::JobContext;
use ironclaw::db::Database;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const ALICE_TOKEN: &str = "tok-alice-secret";
const BOB_TOKEN: &str = "tok-bob-secret";
const ALICE_USER_ID: &str = "alice";
const BOB_USER_ID: &str = "bob";
const OWNER_TOKEN: &str = "tok-owner-secret";
const OWNER_SCOPE_ID: &str = "owner-scope";
/// Build a MultiAuthState with two users.
fn two_user_auth() -> MultiAuthState {
    let mut tokens = HashMap::new();
    tokens.insert(
        ALICE_TOKEN.to_string(),
        UserIdentity {
            user_id: ALICE_USER_ID.to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        },
    );
    tokens.insert(
        BOB_TOKEN.to_string(),
        UserIdentity {
            user_id: BOB_USER_ID.to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: vec!["shared".to_string()],
        },
    );
    MultiAuthState::multi(tokens)
}

/// Build a test Router that echoes the authenticated user_id back.
fn user_echo_app(auth: MultiAuthState) -> Router {
    async fn echo_user(AuthenticatedUser(user): AuthenticatedUser) -> String {
        user.user_id
    }

    async fn echo_user_with_scopes(AuthenticatedUser(user): AuthenticatedUser) -> String {
        format!("{}:{}", user.user_id, user.workspace_read_scopes.join(","))
    }

    Router::new()
        .route("/api/whoami", get(echo_user))
        .route("/api/whoami/scopes", get(echo_user_with_scopes))
        .route("/api/action", post(echo_user))
        .route("/api/chat/events", get(echo_user)) // SSE endpoint (allows query token)
        .layer(middleware::from_fn_with_state(
            ironclaw::channels::web::auth::CombinedAuthState::from(auth),
            auth_middleware,
        ))
}

// ===========================================================================
// Auth: token-to-identity mapping
// ===========================================================================

#[tokio::test]
async fn alice_token_resolves_to_alice_identity() {
    let app = user_echo_app(two_user_auth());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/whoami")
                .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    assert_eq!(std::str::from_utf8(&body).unwrap(), ALICE_USER_ID);
}

#[tokio::test]
async fn bob_token_resolves_to_bob_identity() {
    let app = user_echo_app(two_user_auth());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/whoami")
                .header("Authorization", format!("Bearer {BOB_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    assert_eq!(std::str::from_utf8(&body).unwrap(), BOB_USER_ID);
}

#[tokio::test]
async fn bob_identity_carries_workspace_read_scopes() {
    let app = user_echo_app(two_user_auth());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/whoami/scopes")
                .header("Authorization", format!("Bearer {BOB_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    assert_eq!(std::str::from_utf8(&body).unwrap(), "bob:shared");
}

#[tokio::test]
async fn unknown_token_rejected() {
    let app = user_echo_app(two_user_auth());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/whoami")
                .header("Authorization", "Bearer unknown-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn no_token_rejected() {
    let app = user_echo_app(two_user_auth());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/whoami")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn alice_token_does_not_authenticate_as_bob() {
    let app = user_echo_app(two_user_auth());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/whoami")
                .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let user_id = std::str::from_utf8(&body).unwrap();
    assert_eq!(user_id, ALICE_USER_ID);
    assert_ne!(user_id, BOB_USER_ID);
}

// ===========================================================================
// Auth: query token on SSE/WS endpoints
// ===========================================================================

#[tokio::test]
async fn query_token_works_for_sse_endpoint_multi_user() {
    let app = user_echo_app(two_user_auth());
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/chat/events?token={ALICE_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    assert_eq!(std::str::from_utf8(&body).unwrap(), ALICE_USER_ID);
}

#[tokio::test]
async fn query_token_rejected_for_non_sse_endpoint_multi_user() {
    let app = user_echo_app(two_user_auth());
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/whoami?token={ALICE_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn query_token_rejected_for_post_multi_user() {
    let app = user_echo_app(two_user_auth());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/action?token={ALICE_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ===========================================================================
// Per-user rate limiting
// ===========================================================================

#[test]
fn per_user_rate_limiter_isolates_users() {
    let limiter = PerUserRateLimiter::new(3, 60);

    // Alice uses all 3 requests
    assert!(limiter.check("alice"));
    assert!(limiter.check("alice"));
    assert!(limiter.check("alice"));
    // Alice is now rate-limited
    assert!(!limiter.check("alice"));

    // Bob is unaffected — gets his own 3 requests
    assert!(limiter.check("bob"));
    assert!(limiter.check("bob"));
    assert!(limiter.check("bob"));
    assert!(!limiter.check("bob"));
}

#[test]
fn per_user_rate_limiter_different_users_independent() {
    let limiter = PerUserRateLimiter::new(2, 60);

    // Interleave requests from different users
    assert!(limiter.check("alice"));
    assert!(limiter.check("bob"));
    assert!(limiter.check("alice"));
    assert!(limiter.check("bob"));

    // Both exhausted independently
    assert!(!limiter.check("alice"));
    assert!(!limiter.check("bob"));

    // Charlie is fresh
    assert!(limiter.check("charlie"));
}

#[test]
fn per_user_rate_limiter_single_user_mode() {
    // In single-user mode, only one user_id is used
    let limiter = PerUserRateLimiter::new(5, 60);
    for _ in 0..5 {
        assert!(limiter.check("default"));
    }
    assert!(!limiter.check("default"));
}

// ===========================================================================
// SSE event scoping
// ===========================================================================

#[tokio::test]
async fn sse_scoped_event_only_delivered_to_target_user() {
    use ironclaw_common::AppEvent;
    use tokio_stream::StreamExt;

    let manager = SseManager::new();
    let mut alice_stream = Box::pin(
        manager
            .subscribe_raw(Some(ALICE_USER_ID.to_string()))
            .expect("subscribe"),
    );
    let mut bob_stream = Box::pin(
        manager
            .subscribe_raw(Some(BOB_USER_ID.to_string()))
            .expect("subscribe"),
    );

    // Send event scoped to alice
    manager.broadcast_for_user(
        ALICE_USER_ID,
        AppEvent::Status {
            message: "alice's event".to_string(),
            thread_id: None,
        },
    );

    // Send global heartbeat (both should get it)
    manager.broadcast(AppEvent::Heartbeat);

    // Alice gets her scoped event first
    let e = alice_stream.next().await.unwrap();
    match &e {
        AppEvent::Status { message, .. } => assert_eq!(message, "alice's event"),
        _ => panic!("Expected Status, got {:?}", e),
    }

    // Alice also gets heartbeat
    let e = alice_stream.next().await.unwrap();
    assert!(matches!(e, AppEvent::Heartbeat));

    // Bob only gets the heartbeat (alice's event was filtered)
    let e = bob_stream.next().await.unwrap();
    assert!(matches!(e, AppEvent::Heartbeat));
}

#[tokio::test]
async fn sse_global_event_delivered_to_all_users() {
    use ironclaw_common::AppEvent;
    use tokio_stream::StreamExt;

    let manager = SseManager::new();
    let mut alice = Box::pin(
        manager
            .subscribe_raw(Some(ALICE_USER_ID.to_string()))
            .expect("subscribe"),
    );
    let mut bob = Box::pin(
        manager
            .subscribe_raw(Some(BOB_USER_ID.to_string()))
            .expect("subscribe"),
    );

    manager.broadcast(AppEvent::Status {
        message: "global announcement".to_string(),
        thread_id: None,
    });

    let ea = alice.next().await.unwrap();
    let eb = bob.next().await.unwrap();
    match (&ea, &eb) {
        (AppEvent::Status { message: a, .. }, AppEvent::Status { message: b, .. }) => {
            assert_eq!(a, "global announcement");
            assert_eq!(b, "global announcement");
        }
        _ => panic!("Expected Status events"),
    }
}

#[tokio::test]
async fn sse_user_b_event_not_visible_to_user_a() {
    use ironclaw_common::AppEvent;
    use tokio_stream::StreamExt;

    let manager = SseManager::new();
    let mut alice = Box::pin(
        manager
            .subscribe_raw(Some(ALICE_USER_ID.to_string()))
            .expect("subscribe"),
    );

    // Send event for bob only
    manager.broadcast_for_user(
        BOB_USER_ID,
        AppEvent::Response {
            content: "bob's secret".to_string(),
            thread_id: "t1".to_string(),
        },
    );

    // Send heartbeat so alice has something to receive
    manager.broadcast(AppEvent::Heartbeat);

    // Alice should only get heartbeat, not bob's response
    let e = alice.next().await.unwrap();
    assert!(
        matches!(e, AppEvent::Heartbeat),
        "Expected Heartbeat, got {:?}",
        e
    );
}

#[tokio::test]
async fn sse_unscoped_subscriber_receives_all_events() {
    use ironclaw_common::AppEvent;
    use tokio_stream::StreamExt;

    let manager = SseManager::new();
    // Unscoped subscriber (None user_id) — backwards-compatible single-user mode
    let mut stream = Box::pin(manager.subscribe_raw(None).expect("subscribe"));

    manager.broadcast_for_user(
        ALICE_USER_ID,
        AppEvent::Status {
            message: "alice only".to_string(),
            thread_id: None,
        },
    );
    manager.broadcast_for_user(
        BOB_USER_ID,
        AppEvent::Status {
            message: "bob only".to_string(),
            thread_id: None,
        },
    );
    manager.broadcast(AppEvent::Heartbeat);

    // Unscoped subscriber gets ALL three events
    let e1 = stream.next().await.unwrap();
    let e2 = stream.next().await.unwrap();
    let e3 = stream.next().await.unwrap();

    match &e1 {
        AppEvent::Status { message, .. } => assert_eq!(message, "alice only"),
        _ => panic!("Expected alice's Status"),
    }
    match &e2 {
        AppEvent::Status { message, .. } => assert_eq!(message, "bob only"),
        _ => panic!("Expected bob's Status"),
    }
    assert!(matches!(e3, AppEvent::Heartbeat));
}

// ===========================================================================
// MultiAuthState: edge cases
// ===========================================================================

#[test]
fn multi_auth_state_empty_token_not_valid() {
    let state = MultiAuthState::single("real-token".to_string(), "user1".to_string());
    assert!(state.authenticate("").is_none());
}

#[test]
fn multi_auth_state_first_token_is_none_in_multi_user_mode() {
    let auth = two_user_auth();
    // first_token() returns None in multi-user mode to avoid exposing tokens.
    assert!(auth.first_token().is_none());
}

#[test]
fn multi_auth_state_first_identity_returns_valid_user() {
    let auth = two_user_auth();
    let identity = auth.first_identity().unwrap();
    assert!(identity.user_id == ALICE_USER_ID || identity.user_id == BOB_USER_ID);
}

#[test]
fn multi_auth_state_token_prefix_not_valid() {
    // Ensure partial token matches don't authenticate
    let state = MultiAuthState::single("secret-token-123".to_string(), "user1".to_string());
    assert!(state.authenticate("secret-token").is_none());
    assert!(state.authenticate("secret-token-1234").is_none());
    assert!(state.authenticate("secret-token-123").is_some());
}

// ===========================================================================
// Connection counting with user scoping
// ===========================================================================

#[tokio::test]
async fn sse_connection_count_tracks_scoped_subscribers() {
    let manager = SseManager::new();
    assert_eq!(manager.connection_count(), 0);

    let _alice = Box::pin(
        manager
            .subscribe_raw(Some(ALICE_USER_ID.to_string()))
            .expect("subscribe"),
    );
    assert_eq!(manager.connection_count(), 1);

    let _bob = Box::pin(
        manager
            .subscribe_raw(Some(BOB_USER_ID.to_string()))
            .expect("subscribe"),
    );
    assert_eq!(manager.connection_count(), 2);

    drop(_alice);
    assert_eq!(manager.connection_count(), 1);

    drop(_bob);
    assert_eq!(manager.connection_count(), 0);
}

// ===========================================================================
// GatewayState construction: multi-user fields
// ===========================================================================

#[test]
fn gateway_state_has_multi_tenant_fields() {
    // Verify the GatewayState struct accepts all multi-tenant fields.
    // This is a compile-time check that the conflict resolution didn't
    // drop any fields.
    let state = GatewayState {
        msg_tx: tokio::sync::RwLock::new(None),
        sse: Arc::new(SseManager::new()),
        workspace: None,
        workspace_pool: None, // Multi-tenant: per-user workspace pool
        session_manager: None,
        log_broadcaster: None,
        log_level_handle: None,
        extension_manager: None,
        tool_registry: None,
        store: None,
        job_manager: None,
        prompt_queue: None,
        scheduler: None,
        owner_id: "fallback".to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
        llm_provider: None,
        skill_registry: None,
        skill_catalog: None,
        chat_rate_limiter: PerUserRateLimiter::new(30, 60), // Multi-tenant: per-user
        oauth_rate_limiter: PerUserRateLimiter::new(20, 60),
        registry_entries: Vec::new(),
        cost_guard: None,
        routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
        startup_time: std::time::Instant::now(),
        webhook_rate_limiter: RateLimiter::new(10, 60),
        active_config: Default::default(),
        secrets_store: None,
        db_auth: None,
        oauth_providers: None,
        oauth_state_store: None,
        oauth_base_url: None,
        oauth_allowed_domains: Vec::new(),
        near_nonce_store: None,
        near_rpc_url: None,
    };

    assert_eq!(state.owner_id, "fallback");
    assert!(state.workspace_pool.is_none());
}

// ===========================================================================
// Full-server handler-level tests (real HTTP through auth middleware)
// ===========================================================================

/// Build a MultiAuthState with two users and start a real server.
async fn start_multi_user_server() -> (SocketAddr, Arc<GatewayState>) {
    let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(64);
    let auth = two_user_auth();
    TestGatewayBuilder::new()
        .msg_tx(agent_tx)
        .start_multi(auth)
        .await
        .expect("Failed to start multi-user test server")
}

async fn start_owner_scoped_sender_server() -> (
    SocketAddr,
    Arc<GatewayState>,
    tokio::sync::mpsc::Receiver<IncomingMessage>,
) {
    let (agent_tx, agent_rx) = tokio::sync::mpsc::channel(64);

    let mut tokens = HashMap::new();
    tokens.insert(
        OWNER_TOKEN.to_string(),
        UserIdentity {
            user_id: OWNER_SCOPE_ID.to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        },
    );
    tokens.insert(
        BOB_TOKEN.to_string(),
        UserIdentity {
            user_id: BOB_USER_ID.to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        },
    );

    let state = Arc::new(GatewayState {
        msg_tx: tokio::sync::RwLock::new(Some(agent_tx)),
        sse: Arc::new(SseManager::new()),
        workspace: None,
        workspace_pool: None,
        session_manager: None,
        log_broadcaster: None,
        log_level_handle: None,
        extension_manager: None,
        tool_registry: None,
        store: None,
        job_manager: None,
        prompt_queue: None,
        scheduler: None,
        owner_id: OWNER_SCOPE_ID.to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
        llm_provider: None,
        skill_registry: None,
        skill_catalog: None,
        chat_rate_limiter: PerUserRateLimiter::new(30, 60),
        oauth_rate_limiter: PerUserRateLimiter::new(20, 60),
        webhook_rate_limiter: RateLimiter::new(10, 60),
        registry_entries: Vec::new(),
        cost_guard: None,
        routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
        startup_time: std::time::Instant::now(),
        active_config: Default::default(),
        secrets_store: None,
        db_auth: None,
        oauth_providers: None,
        oauth_state_store: None,
        oauth_base_url: None,
        oauth_allowed_domains: Vec::new(),
        near_nonce_store: None,
        near_rpc_url: None,
    });

    let auth = MultiAuthState::multi(tokens).into();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let bound = start_server(addr, state.clone(), auth)
        .await
        .expect("Failed to start owner-scoped sender test server");

    (bound, state, agent_rx)
}

#[tokio::test]
async fn full_server_alice_can_access_protected_endpoint() {
    let (addr, _state) = start_multi_user_server().await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/api/gateway/status", addr))
        .header("Authorization", format!("Bearer {}", ALICE_TOKEN))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn full_server_bob_can_access_protected_endpoint() {
    let (addr, _state) = start_multi_user_server().await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/api/gateway/status", addr))
        .header("Authorization", format!("Bearer {}", BOB_TOKEN))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn full_server_unknown_token_returns_401() {
    let (addr, _state) = start_multi_user_server().await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/api/gateway/status", addr))
        .header("Authorization", "Bearer wrong-token")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn full_server_no_auth_header_returns_401() {
    let (addr, _state) = start_multi_user_server().await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/api/gateway/status", addr))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn full_server_health_is_public() {
    let (addr, _state) = start_multi_user_server().await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/api/health", addr))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn full_server_chat_send_accepted_for_alice() {
    let (agent_tx, mut agent_rx) = tokio::sync::mpsc::channel(64);
    let auth = two_user_auth();
    let (addr, _state) = TestGatewayBuilder::new()
        .msg_tx(agent_tx)
        .start_multi(auth)
        .await
        .expect("Failed to start server");

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{}/api/chat/send", addr))
        .header("Authorization", format!("Bearer {}", ALICE_TOKEN))
        .header("Content-Type", "application/json")
        .body(r#"{"content":"hello from alice"}"#)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 202); // ACCEPTED

    // Verify the message reached the agent channel
    let msg = tokio::time::timeout(Duration::from_secs(2), agent_rx.recv())
        .await
        .expect("Timed out waiting for agent message")
        .expect("Agent channel closed");

    assert_eq!(msg.content, "hello from alice");
    assert_eq!(msg.channel, "gateway");
}

#[tokio::test]
async fn full_server_chat_send_rewrites_sender_only_for_owner_scope_rebind() {
    let (addr, _state, mut agent_rx) = start_owner_scoped_sender_server().await;

    let client = reqwest::Client::new();

    let owner_resp = client
        .post(format!("http://{}/api/chat/send", addr))
        .header("Authorization", format!("Bearer {}", OWNER_TOKEN))
        .header("Content-Type", "application/json")
        .body(r#"{"content":"hello from owner"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(owner_resp.status(), 202);

    let owner_msg = tokio::time::timeout(Duration::from_secs(2), agent_rx.recv())
        .await
        .expect("Timed out waiting for owner message")
        .expect("Agent channel closed");
    assert_eq!(owner_msg.user_id, OWNER_SCOPE_ID);
    assert_eq!(owner_msg.sender_id, OWNER_SCOPE_ID);
    assert_eq!(owner_msg.content, "hello from owner");

    let other_resp = client
        .post(format!("http://{}/api/chat/send", addr))
        .header("Authorization", format!("Bearer {}", BOB_TOKEN))
        .header("Content-Type", "application/json")
        .body(r#"{"content":"hello from bob"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(other_resp.status(), 202);

    let other_msg = tokio::time::timeout(Duration::from_secs(2), agent_rx.recv())
        .await
        .expect("Timed out waiting for non-owner message")
        .expect("Agent channel closed");
    assert_eq!(other_msg.user_id, BOB_USER_ID);
    assert_eq!(other_msg.sender_id, BOB_USER_ID);
    assert_eq!(other_msg.content, "hello from bob");
}

#[tokio::test]
async fn full_server_chat_send_rejected_without_auth() {
    let (addr, _state) = start_multi_user_server().await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{}/api/chat/send", addr))
        .header("Content-Type", "application/json")
        .body(r#"{"content":"unauthorized message"}"#)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn full_server_query_token_works_for_sse() {
    let (addr, _state) = start_multi_user_server().await;

    let client = reqwest::Client::new();
    // SSE endpoint should accept query token
    let resp = client
        .get(format!(
            "http://{}/api/chat/events?token={}",
            addr, ALICE_TOKEN
        ))
        .send()
        .await
        .unwrap();

    // Should get 200 (SSE stream starts)
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn full_server_query_token_rejected_for_non_sse() {
    let (addr, _state) = start_multi_user_server().await;

    let client = reqwest::Client::new();
    // Non-SSE endpoint should NOT accept query token
    let resp = client
        .get(format!(
            "http://{}/api/gateway/status?token={}",
            addr, ALICE_TOKEN
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn full_server_jobs_endpoint_returns_503_without_db() {
    let (addr, _state) = start_multi_user_server().await;

    let client = reqwest::Client::new();
    // Jobs endpoint requires database — should return 503 (no DB configured)
    // but NOT 401 (auth should pass)
    let resp = client
        .get(format!("http://{}/api/jobs", addr))
        .header("Authorization", format!("Bearer {}", ALICE_TOKEN))
        .send()
        .await
        .unwrap();

    // Without a database, this should return a server error, not an auth error
    let status = resp.status().as_u16();
    assert_ne!(status, 401, "Should not be auth error — token is valid");
    assert_ne!(status, 403, "Should not be forbidden — token is valid");
}

#[tokio::test]
async fn full_server_jobs_endpoint_rejected_without_auth() {
    let (addr, _state) = start_multi_user_server().await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/api/jobs", addr))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn full_server_ws_multi_user_event_isolation() {
    use futures::StreamExt;
    use ironclaw_common::AppEvent;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let (addr, state) = start_multi_user_server().await;

    // Connect Alice's WS
    let alice_url = format!("ws://{}/api/chat/ws?token={}", addr, ALICE_TOKEN);
    let mut alice_req = alice_url.into_client_request().unwrap();
    alice_req.headers_mut().insert(
        "Origin",
        format!("http://127.0.0.1:{}", addr.port()).parse().unwrap(),
    );
    let (mut alice_ws, _) = tokio_tungstenite::connect_async(alice_req)
        .await
        .expect("Alice WS connect failed");

    // Connect Bob's WS
    let bob_url = format!("ws://{}/api/chat/ws?token={}", addr, BOB_TOKEN);
    let mut bob_req = bob_url.into_client_request().unwrap();
    bob_req.headers_mut().insert(
        "Origin",
        format!("http://127.0.0.1:{}", addr.port()).parse().unwrap(),
    );
    let (mut bob_ws, _) = tokio_tungstenite::connect_async(bob_req)
        .await
        .expect("Bob WS connect failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Broadcast an event scoped to Alice only
    state.sse.broadcast_for_user(
        ALICE_USER_ID,
        AppEvent::Status {
            message: "alice-only-event".to_string(),
            thread_id: None,
        },
    );

    // Broadcast a global heartbeat so Bob has something to receive
    state.sse.broadcast(AppEvent::Heartbeat);

    // Alice should get her scoped event
    let alice_msg = tokio::time::timeout(Duration::from_secs(2), alice_ws.next())
        .await
        .expect("Alice WS timed out")
        .expect("Alice stream ended")
        .expect("Alice WS error");

    if let Message::Text(text) = alice_msg {
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["type"], "event");
        assert_eq!(parsed["event_type"], "status");
        assert_eq!(parsed["data"]["message"], "alice-only-event");
    } else {
        panic!("Expected Text frame from Alice WS, got {:?}", alice_msg);
    }

    // Bob should only get the heartbeat, NOT alice's event
    let bob_msg = tokio::time::timeout(Duration::from_secs(2), bob_ws.next())
        .await
        .expect("Bob WS timed out")
        .expect("Bob stream ended")
        .expect("Bob WS error");

    if let Message::Text(text) = bob_msg {
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["type"], "event");
        assert_eq!(
            parsed["event_type"], "heartbeat",
            "Bob should only see heartbeat, not alice's event. Got: {}",
            text
        );
    } else {
        panic!("Expected Text frame from Bob WS, got {:?}", bob_msg);
    }

    alice_ws.close(None).await.ok();
    bob_ws.close(None).await.ok();
}

// ===========================================================================
// DB-backed job ownership tests (libSQL in-memory)
// ===========================================================================

/// Start a multi-user server with a real (in-memory) database.
#[cfg(feature = "libsql")]
async fn start_multi_user_server_with_db() -> (
    SocketAddr,
    Arc<GatewayState>,
    Arc<dyn Database>,
    tempfile::TempDir,
) {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let path = temp_dir.path().join("test.db");
    let backend = ironclaw::db::libsql::LibSqlBackend::new_local(&path)
        .await
        .expect("failed to create test DB");
    backend
        .run_migrations()
        .await
        .expect("failed to run migrations");
    let db: Arc<dyn Database> = Arc::new(backend);
    let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(64);
    let auth = two_user_auth();

    // Build state manually so we can inject the DB
    let state = Arc::new(GatewayState {
        msg_tx: tokio::sync::RwLock::new(Some(agent_tx)),
        sse: Arc::new(SseManager::new()),
        workspace: None,
        workspace_pool: None,
        session_manager: None,
        log_broadcaster: None,
        log_level_handle: None,
        extension_manager: None,
        tool_registry: None,
        store: Some(Arc::clone(&db)),
        job_manager: None,
        prompt_queue: None,
        scheduler: None,
        owner_id: ALICE_USER_ID.to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
        llm_provider: None,
        skill_registry: None,
        skill_catalog: None,
        chat_rate_limiter: PerUserRateLimiter::new(30, 60),
        oauth_rate_limiter: PerUserRateLimiter::new(20, 60),
        registry_entries: Vec::new(),
        cost_guard: None,
        routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
        startup_time: std::time::Instant::now(),
        webhook_rate_limiter: RateLimiter::new(10, 60),
        active_config: Default::default(),
        secrets_store: None,
        db_auth: None,
        oauth_providers: None,
        oauth_state_store: None,
        oauth_base_url: None,
        oauth_allowed_domains: Vec::new(),
        near_nonce_store: None,
        near_rpc_url: None,
    });

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let bound = ironclaw::channels::web::server::start_server(addr, state.clone(), auth.into())
        .await
        .expect("Failed to start server with DB");

    (bound, state, db, temp_dir)
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn full_server_alice_sees_own_jobs_only() {
    let (addr, _state, db, _tmp) = start_multi_user_server_with_db().await;

    // Create jobs owned by Alice and Bob
    let alice_job = JobContext::with_user(ALICE_USER_ID, "Alice's job", "Alice's work");
    let bob_job = JobContext::with_user(BOB_USER_ID, "Bob's job", "Bob's work");
    let alice_job_id = alice_job.job_id;

    db.save_job(&alice_job).await.unwrap();
    db.save_job(&bob_job).await.unwrap();

    let client = reqwest::Client::new();

    // Alice lists jobs — should only see her own
    let resp = client
        .get(format!("http://{}/api/jobs", addr))
        .header("Authorization", format!("Bearer {}", ALICE_TOKEN))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let jobs = body["jobs"].as_array().unwrap();

    // Alice should see exactly 1 job
    assert_eq!(jobs.len(), 1, "Alice should see only her own job");
    assert_eq!(jobs[0]["id"], alice_job_id.to_string());
    assert_eq!(jobs[0]["title"], "Alice's job");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn full_server_bob_cannot_see_alice_job_detail() {
    let (addr, _state, db, _tmp) = start_multi_user_server_with_db().await;

    // Create a job owned by Alice
    let alice_job = JobContext::with_user(ALICE_USER_ID, "Alice's secret job", "Private");
    let alice_job_id = alice_job.job_id;
    db.save_job(&alice_job).await.unwrap();

    let client = reqwest::Client::new();

    // Bob tries to access Alice's job by ID — should get 404 (not 403, to prevent enumeration)
    let resp = client
        .get(format!("http://{}/api/jobs/{}", addr, alice_job_id))
        .header("Authorization", format!("Bearer {}", BOB_TOKEN))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        404,
        "Bob should not be able to see Alice's job"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn full_server_alice_can_see_own_job_detail() {
    let (addr, _state, db, _tmp) = start_multi_user_server_with_db().await;

    let alice_job = JobContext::with_user(ALICE_USER_ID, "Alice's visible job", "Details here");
    let alice_job_id = alice_job.job_id;
    db.save_job(&alice_job).await.unwrap();

    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://{}/api/jobs/{}", addr, alice_job_id))
        .header("Authorization", format!("Bearer {}", ALICE_TOKEN))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["id"], alice_job_id.to_string());
    assert_eq!(body["title"], "Alice's visible job");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn full_server_bob_sees_own_jobs_only() {
    let (addr, _state, db, _tmp) = start_multi_user_server_with_db().await;

    // Create multiple jobs for each user
    for i in 0..3 {
        let aj = JobContext::with_user(ALICE_USER_ID, format!("Alice job {}", i), "");
        db.save_job(&aj).await.unwrap();
    }
    for i in 0..2 {
        let bj = JobContext::with_user(BOB_USER_ID, format!("Bob job {}", i), "");
        db.save_job(&bj).await.unwrap();
    }

    let client = reqwest::Client::new();

    // Bob lists jobs
    let resp = client
        .get(format!("http://{}/api/jobs", addr))
        .header("Authorization", format!("Bearer {}", BOB_TOKEN))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let jobs = body["jobs"].as_array().unwrap();

    assert_eq!(
        jobs.len(),
        2,
        "Bob should see only his 2 jobs, not Alice's 3"
    );
    for job in jobs {
        let title = job["title"].as_str().unwrap();
        assert!(
            title.starts_with("Bob job"),
            "Bob should only see his own jobs, got: {}",
            title
        );
    }
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn full_server_nonexistent_job_returns_404() {
    let (addr, _state, _db, _tmp) = start_multi_user_server_with_db().await;

    let client = reqwest::Client::new();
    let fake_id = uuid::Uuid::new_v4();

    let resp = client
        .get(format!("http://{}/api/jobs/{}", addr, fake_id))
        .header("Authorization", format!("Bearer {}", ALICE_TOKEN))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);
}
