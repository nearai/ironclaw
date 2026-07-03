//! Cross-tenant thread access regression tests.
//!
//! Drives the running gateway over real HTTP and asserts that Bob cannot
//! read Alice's threads, messages, or engine-thread metadata. Each
//! handler that takes a thread-id from the request gets its own negative
//! test: success means a foreign-id request is rejected at the boundary
//! (typically 404 to prevent enumeration, never 200).
//!
//! Modeled on the job-isolation tests in
//! `tests/multi_tenant_integration.rs` (`full_server_*_jobs_*`).
//!
//! Gated on `feature = "libsql"` so the suite has a real DB to seed
//! conversations into.

#![cfg(feature = "libsql")]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use ironclaw::agent::SessionManager;
use ironclaw::channels::web::auth::{MultiAuthState, UserIdentity};
use ironclaw::channels::web::platform::router::start_server;
use ironclaw::channels::web::platform::state::{GatewayState, PerUserRateLimiter, RateLimiter};
use ironclaw::channels::web::sse::SseManager;
use ironclaw::channels::web::ws::WsConnectionTracker;
use ironclaw::db::Database;

const ALICE_TOKEN: &str = "tok-alice-thread-isolation";
const BOB_TOKEN: &str = "tok-bob-thread-isolation";
const ALICE_USER_ID: &str = "alice";
const BOB_USER_ID: &str = "bob";

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
            workspace_read_scopes: Vec::new(),
        },
    );
    MultiAuthState::multi(tokens)
}

/// Spin up a real Axum server backed by an in-memory libSQL database
/// with a `SessionManager` attached. The chat history / threads
/// endpoints require both, and the Responses-API GET requires the DB.
async fn start_server_with_db() -> (
    SocketAddr,
    Arc<GatewayState>,
    Arc<dyn Database>,
    tempfile::TempDir,
) {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("test.db");
    let backend = ironclaw::db::libsql::LibSqlBackend::new_local(&path)
        .await
        .expect("backend");
    backend.run_migrations().await.expect("migrations");
    let db: Arc<dyn Database> = Arc::new(backend);

    let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(64);
    let auth = two_user_auth();
    let session_manager = Arc::new(SessionManager::new());

    let state = Arc::new(GatewayState {
        msg_tx: tokio::sync::RwLock::new(Some(agent_tx)),
        sse: Arc::new(SseManager::new()),
        workspace: None,
        workspace_pool: None,
        multi_tenant_mode: true,
        session_manager: Some(session_manager),
        log_broadcaster: None,
        log_level_handle: None,
        extension_manager: None,
        tool_registry: None,
        store: Some(Arc::clone(&db)),
        settings_cache: None,
        job_manager: None,
        prompt_queue: None,
        scheduler: None,
        owner_id: ALICE_USER_ID.to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
        llm_provider: None,
        llm_reload: None,
        llm_session_manager: None,
        config_toml_path: None,
        skill_registry: None,
        skill_catalog: None,
        auth_manager: None,
        chat_rate_limiter: PerUserRateLimiter::new(30, 60),
        oauth_rate_limiter: PerUserRateLimiter::new(20, 60),
        registry_entries: Vec::new(),
        cost_guard: None,
        routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
        startup_time: std::time::Instant::now(),
        webhook_rate_limiter: RateLimiter::new(10, 60),
        active_config: Arc::new(tokio::sync::RwLock::new(Default::default())),
        secrets_store: None,
        db_auth: None,
        pairing_store: None,
        oauth_providers: None,
        oauth_state_store: None,
        oauth_base_url: None,
        oauth_allowed_domains: Vec::new(),
        near_nonce_store: None,
        near_rpc_url: None,
        near_network: None,
        oauth_sweep_shutdown: None,
        frontend_html_cache: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        tool_dispatcher: None,
    });

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let bound = start_server(addr, state.clone(), auth.into())
        .await
        .expect("start_server");

    (bound, state, db, temp_dir)
}

/// Seed a conversation with a single message, owned by `user_id`.
/// Returns the conversation id so the caller can probe access by id.
async fn seed_conversation(db: &Arc<dyn Database>, user_id: &str, content: &str) -> uuid::Uuid {
    let id = db
        .create_conversation_with_metadata(
            "gateway",
            user_id,
            &serde_json::json!({"title": format!("{user_id}'s conversation")}),
        )
        .await
        .expect("create conversation");
    db.add_conversation_message(id, "user", content)
        .await
        .expect("add message");
    id
}

// ---------------------------------------------------------------------------
// Chat history — paginated read by thread_id
// ---------------------------------------------------------------------------

#[tokio::test]
async fn bob_history_for_alice_thread_returns_404() {
    let (addr, _state, db, _tmp) = start_server_with_db().await;
    let alice_thread = seed_conversation(&db, ALICE_USER_ID, "alice secret").await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "http://{}/api/chat/history?thread_id={}",
            addr, alice_thread
        ))
        .header("Authorization", format!("Bearer {}", BOB_TOKEN))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        404,
        "Bob requesting Alice's thread_id must get 404; got status {} body {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
}

#[tokio::test]
async fn bob_history_paginated_for_alice_thread_returns_404() {
    let (addr, _state, db, _tmp) = start_server_with_db().await;
    let alice_thread = seed_conversation(&db, ALICE_USER_ID, "alice secret").await;

    let client = reqwest::Client::new();
    // The handler's paginated branch parses `before` as RFC3339 then runs
    // an unfiltered `list_conversation_messages_paginated` against the DB.
    // Pre-fix that branch skipped the ownership check; this test pins the
    // post-fix behavior. Use `.query()` so reqwest URL-encodes `+`/`:`.
    let before = chrono::Utc::now().to_rfc3339();
    let resp = client
        .get(format!("http://{}/api/chat/history", addr))
        .query(&[
            ("thread_id", alice_thread.to_string()),
            ("before", before),
            ("limit", "10".to_string()),
        ])
        .header("Authorization", format!("Bearer {}", BOB_TOKEN))
        .send()
        .await
        .unwrap();

    // The paginated branch went straight to the unscoped DB query before
    // F4. With ownership pre-checked at the handler boundary, Bob's
    // request is rejected before any messages load.
    assert_eq!(
        resp.status(),
        404,
        "Bob's paginated history request for Alice's thread must not return messages"
    );
}

#[tokio::test]
async fn alice_history_for_own_thread_succeeds() {
    let (addr, _state, db, _tmp) = start_server_with_db().await;
    let alice_thread = seed_conversation(&db, ALICE_USER_ID, "alice writes this").await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "http://{}/api/chat/history?thread_id={}",
            addr, alice_thread
        ))
        .header("Authorization", format!("Bearer {}", ALICE_TOKEN))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200, "Alice must reach her own thread");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["thread_id"], alice_thread.to_string());
}

// ---------------------------------------------------------------------------
// Threads list — should never enumerate another user's conversations
// ---------------------------------------------------------------------------

#[tokio::test]
async fn bob_threads_list_excludes_alice_threads() {
    let (addr, _state, db, _tmp) = start_server_with_db().await;

    // Seed multiple conversations for each user.
    let alice_a = seed_conversation(&db, ALICE_USER_ID, "alice 1").await;
    let alice_b = seed_conversation(&db, ALICE_USER_ID, "alice 2").await;
    let bob_a = seed_conversation(&db, BOB_USER_ID, "bob 1").await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/api/chat/threads", addr))
        .header("Authorization", format!("Bearer {}", BOB_TOKEN))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();

    let visible_ids: Vec<String> = body["threads"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|t| t["id"].as_str().map(String::from))
        .collect();

    let alice_a_str = alice_a.to_string();
    let alice_b_str = alice_b.to_string();
    let bob_a_str = bob_a.to_string();

    assert!(
        !visible_ids.contains(&alice_a_str),
        "Bob's threads list must not include Alice's conversations: {visible_ids:?}"
    );
    assert!(
        !visible_ids.contains(&alice_b_str),
        "Bob's threads list must not include Alice's conversations: {visible_ids:?}"
    );
    // Bob's own conversation may appear; the assistant_thread is also
    // his own (auto-created by the handler). The point is the absence
    // of alice_*.
    let _ = bob_a_str;
}

// ---------------------------------------------------------------------------
// Engine v2 — detail / steps / events
//
// Scope: these tests pin the *handler shape* — an unknown thread id
// returns 404 / empty rather than 500ing or leaking. They do NOT
// exercise the cross-tenant ownership branch (Alice's actual id with
// Bob's token), because seeding an engine v2 thread requires
// `ENGINE_STATE` (a process-wide `OnceCell` in `bridge/router.rs`)
// to be initialized with a backing `Store`. That fixture doesn't
// exist for the integration test surface today; building it without
// breaking the singleton's invariants for parallel test runs is its
// own change.
//
// The ownership gate itself lives in
// `src/bridge/router.rs::{get_engine_thread, list_engine_thread_steps,
// list_engine_thread_events}` and uses `thread.is_owned_by(user_id)`.
// That predicate is unit-tested upstream; the missing piece here is
// the call-site test through the handler.
//
// Follow-up: introduce a per-test `ENGINE_STATE` injection seam (or a
// memory-backed `Store` test fixture) and add the Alice-seed/Bob-probe
// variant of each test below.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn bob_engine_thread_detail_for_unknown_returns_404() {
    let (addr, _state, _db, _tmp) = start_server_with_db().await;
    let foreign_id = uuid::Uuid::new_v4();

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/api/engine/threads/{}", addr, foreign_id))
        .header("Authorization", format!("Bearer {}", BOB_TOKEN))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        404,
        "Engine thread detail for an id Bob doesn't own must be 404"
    );
}

#[tokio::test]
async fn bob_engine_thread_steps_for_unknown_returns_empty() {
    let (addr, _state, _db, _tmp) = start_server_with_db().await;
    let foreign_id = uuid::Uuid::new_v4();

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "http://{}/api/engine/threads/{}/steps",
            addr, foreign_id
        ))
        .header("Authorization", format!("Bearer {}", BOB_TOKEN))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let steps = body["steps"].as_array().expect("steps array");
    assert!(
        steps.is_empty(),
        "Engine steps must be empty for an unowned thread id; got {body:?}"
    );
}

#[tokio::test]
async fn bob_engine_thread_events_for_unknown_returns_empty() {
    let (addr, _state, _db, _tmp) = start_server_with_db().await;
    let foreign_id = uuid::Uuid::new_v4();

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "http://{}/api/engine/threads/{}/events",
            addr, foreign_id
        ))
        .header("Authorization", format!("Bearer {}", BOB_TOKEN))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let events = body["events"].as_array().expect("events array");
    assert!(
        events.is_empty(),
        "Engine events must be empty for unowned thread id"
    );
}

// ---------------------------------------------------------------------------
// Responses API — GET /api/v1/responses/{id}
// ---------------------------------------------------------------------------

#[tokio::test]
async fn bob_responses_get_for_alice_response_returns_404() {
    let (addr, _state, db, _tmp) = start_server_with_db().await;
    // Seed alice's conversation to act as a response_id surrogate. The
    // Responses API's GET path resolves a UUID to a conversation owner
    // before reading messages, so any alice-owned conversation id is a
    // valid attack surface.
    let alice_thread = seed_conversation(&db, ALICE_USER_ID, "alice").await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/api/v1/responses/{}", addr, alice_thread))
        .header("Authorization", format!("Bearer {}", BOB_TOKEN))
        .send()
        .await
        .unwrap();

    assert!(
        resp.status() == 404 || resp.status() == 400,
        "Bob requesting Alice's response id must not get 200; got {}",
        resp.status()
    );
}

// ---------------------------------------------------------------------------
// Unauthenticated access never reaches any of the above
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unauthenticated_history_request_is_rejected() {
    let (addr, _state, db, _tmp) = start_server_with_db().await;
    let alice_thread = seed_conversation(&db, ALICE_USER_ID, "alice").await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "http://{}/api/chat/history?thread_id={}",
            addr, alice_thread
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}
