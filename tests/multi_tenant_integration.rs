//! Integration tests for multi-tenant auth, isolation, and per-user scoping.
//!
//! These tests verify that multi-tenant infrastructure works correctly:
//! - Token-to-identity mapping via MultiAuthState
//! - Per-user SSE event scoping (user A doesn't see user B's events)
//! - Per-user rate limiting (user A exhausting limit doesn't block user B)
//! - Auth middleware inserts correct UserIdentity into request extensions
//! - WebSocket connections are scoped to the authenticated user

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware;
use axum::routing::{get, post};
use axum::Router;
use tower::ServiceExt;

use ironclaw::channels::web::auth::{
    AuthenticatedUser, MultiAuthState, UserIdentity, auth_middleware,
};
use ironclaw::channels::web::server::{GatewayState, PerUserRateLimiter, RateLimiter};
use ironclaw::channels::web::sse::SseManager;
use ironclaw::channels::web::ws::WsConnectionTracker;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const ALICE_TOKEN: &str = "tok-alice-secret";
const BOB_TOKEN: &str = "tok-bob-secret";
const ALICE_USER_ID: &str = "alice";
const BOB_USER_ID: &str = "bob";

/// Build a MultiAuthState with two users.
fn two_user_auth() -> MultiAuthState {
    let mut tokens = HashMap::new();
    tokens.insert(
        ALICE_TOKEN.to_string(),
        UserIdentity {
            user_id: ALICE_USER_ID.to_string(),
            workspace_read_scopes: Vec::new(),
        },
    );
    tokens.insert(
        BOB_TOKEN.to_string(),
        UserIdentity {
            user_id: BOB_USER_ID.to_string(),
            workspace_read_scopes: vec!["shared".to_string()],
        },
    );
    MultiAuthState::multi(tokens)
}

/// Build a test Router that echoes the authenticated user_id back.
fn user_echo_app(auth: MultiAuthState) -> Router {
    async fn echo_user(
        AuthenticatedUser(user): AuthenticatedUser,
    ) -> String {
        user.user_id
    }

    async fn echo_user_with_scopes(
        AuthenticatedUser(user): AuthenticatedUser,
    ) -> String {
        format!("{}:{}", user.user_id, user.workspace_read_scopes.join(","))
    }

    Router::new()
        .route("/api/whoami", get(echo_user))
        .route("/api/whoami/scopes", get(echo_user_with_scopes))
        .route("/api/action", post(echo_user))
        .route("/api/chat/events", get(echo_user)) // SSE endpoint (allows query token)
        .layer(middleware::from_fn_with_state(auth, auth_middleware))
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
    use ironclaw::channels::web::types::SseEvent;
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
        SseEvent::Status {
            message: "alice's event".to_string(),
            thread_id: None,
        },
    );

    // Send global heartbeat (both should get it)
    manager.broadcast(SseEvent::Heartbeat);

    // Alice gets her scoped event first
    let e = alice_stream.next().await.unwrap();
    match &e {
        SseEvent::Status { message, .. } => assert_eq!(message, "alice's event"),
        _ => panic!("Expected Status, got {:?}", e),
    }

    // Alice also gets heartbeat
    let e = alice_stream.next().await.unwrap();
    assert!(matches!(e, SseEvent::Heartbeat));

    // Bob only gets the heartbeat (alice's event was filtered)
    let e = bob_stream.next().await.unwrap();
    assert!(matches!(e, SseEvent::Heartbeat));
}

#[tokio::test]
async fn sse_global_event_delivered_to_all_users() {
    use ironclaw::channels::web::types::SseEvent;
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

    manager.broadcast(SseEvent::Status {
        message: "global announcement".to_string(),
        thread_id: None,
    });

    let ea = alice.next().await.unwrap();
    let eb = bob.next().await.unwrap();
    match (&ea, &eb) {
        (SseEvent::Status { message: a, .. }, SseEvent::Status { message: b, .. }) => {
            assert_eq!(a, "global announcement");
            assert_eq!(b, "global announcement");
        }
        _ => panic!("Expected Status events"),
    }
}

#[tokio::test]
async fn sse_user_b_event_not_visible_to_user_a() {
    use ironclaw::channels::web::types::SseEvent;
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
        SseEvent::Response {
            content: "bob's secret".to_string(),
            thread_id: "t1".to_string(),
        },
    );

    // Send heartbeat so alice has something to receive
    manager.broadcast(SseEvent::Heartbeat);

    // Alice should only get heartbeat, not bob's response
    let e = alice.next().await.unwrap();
    assert!(
        matches!(e, SseEvent::Heartbeat),
        "Expected Heartbeat, got {:?}",
        e
    );
}

#[tokio::test]
async fn sse_unscoped_subscriber_receives_all_events() {
    use ironclaw::channels::web::types::SseEvent;
    use tokio_stream::StreamExt;

    let manager = SseManager::new();
    // Unscoped subscriber (None user_id) — backwards-compatible single-user mode
    let mut stream = Box::pin(manager.subscribe_raw(None).expect("subscribe"));

    manager.broadcast_for_user(
        ALICE_USER_ID,
        SseEvent::Status {
            message: "alice only".to_string(),
            thread_id: None,
        },
    );
    manager.broadcast_for_user(
        BOB_USER_ID,
        SseEvent::Status {
            message: "bob only".to_string(),
            thread_id: None,
        },
    );
    manager.broadcast(SseEvent::Heartbeat);

    // Unscoped subscriber gets ALL three events
    let e1 = stream.next().await.unwrap();
    let e2 = stream.next().await.unwrap();
    let e3 = stream.next().await.unwrap();

    match &e1 {
        SseEvent::Status { message, .. } => assert_eq!(message, "alice only"),
        _ => panic!("Expected alice's Status"),
    }
    match &e2 {
        SseEvent::Status { message, .. } => assert_eq!(message, "bob only"),
        _ => panic!("Expected bob's Status"),
    }
    assert!(matches!(e3, SseEvent::Heartbeat));
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
fn multi_auth_state_first_token_returns_any_token() {
    let auth = two_user_auth();
    let first = auth.first_token().unwrap();
    // Should be one of the two tokens (HashMap ordering is non-deterministic)
    assert!(first == ALICE_TOKEN || first == BOB_TOKEN);
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
        default_user_id: "fallback".to_string(), // Multi-tenant: renamed from user_id
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
        llm_provider: None,
        skill_registry: None,
        skill_catalog: None,
        chat_rate_limiter: PerUserRateLimiter::new(30, 60), // Multi-tenant: per-user
        oauth_rate_limiter: RateLimiter::new(10, 60),
        registry_entries: Vec::new(),
        cost_guard: None,
        routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
        startup_time: std::time::Instant::now(),
    };

    assert_eq!(state.default_user_id, "fallback");
    assert!(state.workspace_pool.is_none());
}
