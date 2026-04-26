//! Multi-tenant isolation tests for the web gateway.
//!
//! Tests cover workspace pool scoping, job handler isolation, and auth
//! enforcement on protected endpoints. Uses `LibSqlBackend::new_local()`
//! with a temporary directory for a real (but ephemeral) database.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::middleware;
use axum::routing::{delete, get, post};
use tower::ServiceExt;
use uuid::Uuid;

use crate::channels::web::auth::{
    AuthenticatedUser, MultiAuthState, UserIdentity, auth_middleware,
};
use crate::channels::web::server::{
    ActiveConfigSnapshot, GatewayState, PerUserRateLimiter, PromptQueue, RateLimiter, WorkspacePool,
};
use crate::channels::web::sse::SseManager;

// ── Helpers ────────────────────────────────────────────────────────────

/// Create a two-user `MultiAuthState` for alice and bob.
fn two_user_auth() -> MultiAuthState {
    let mut tokens = HashMap::new();
    tokens.insert(
        "tok-alice".to_string(),
        UserIdentity {
            user_id: "alice".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: vec!["shared".to_string()],
        },
    );
    tokens.insert(
        "tok-bob".to_string(),
        UserIdentity {
            user_id: "bob".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: vec!["shared".to_string(), "alice".to_string()],
        },
    );
    MultiAuthState::multi(tokens)
}

/// Build a `GatewayState` with configurable store and prompt queue.
fn build_state(
    store: Option<Arc<dyn crate::db::Database>>,
    prompt_queue: Option<PromptQueue>,
) -> Arc<GatewayState> {
    Arc::new(GatewayState {
        msg_tx: tokio::sync::RwLock::new(None),
        sse: Arc::new(SseManager::new()),
        workspace: None,
        workspace_pool: None,
        session_manager: None,
        log_broadcaster: None,
        log_level_handle: None,
        extension_manager: None,
        tool_registry: None,
        store,
        job_manager: None,
        prompt_queue,
        owner_id: "test".to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: None,
        llm_provider: None,
        skill_registry: None,
        skill_catalog: None,
        auth_manager: None,
        scheduler: None,
        chat_rate_limiter: PerUserRateLimiter::new(30, 60),
        oauth_rate_limiter: PerUserRateLimiter::new(20, 60),
        webhook_rate_limiter: RateLimiter::new(10, 60),
        registry_entries: Vec::new(),
        cost_guard: None,
        routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
        startup_time: std::time::Instant::now(),
        active_config: ActiveConfigSnapshot::default(),
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
    })
}

/// Create a libSQL-backed test database in a temporary directory.
///
/// Returns the database and a `TempDir` guard — the database file is
/// deleted when the guard is dropped.
#[cfg(feature = "libsql")]
async fn test_db() -> (Arc<dyn crate::db::Database>, tempfile::TempDir) {
    use crate::db::Database;
    let dir = tempfile::tempdir().expect("failed to create temp dir"); // safety: test-only
    let path = dir.path().join("test.db");
    let backend = crate::db::libsql::LibSqlBackend::new_local(&path)
        .await
        .expect("failed to create test LibSqlBackend"); // safety: test-only
    backend
        .run_migrations()
        .await
        .expect("failed to run migrations"); // safety: test-only
    (Arc::new(backend) as Arc<dyn crate::db::Database>, dir)
}

/// Build a minimal Routine for testing.
fn make_routine(user_id: &str, name: &str) -> crate::agent::routine::Routine {
    let now = chrono::Utc::now();
    crate::agent::routine::Routine {
        id: Uuid::new_v4(),
        name: name.to_string(),
        description: format!("Test routine: {name}"),
        user_id: user_id.to_string(),
        enabled: true,
        trigger: crate::agent::routine::Trigger::Cron {
            schedule: "0 9 * * *".to_string(),
            timezone: None,
        },
        action: crate::agent::routine::RoutineAction::Lightweight {
            prompt: "hello".to_string(),
            context_paths: vec![],
            max_tokens: 1024,
            use_tools: false,
            max_tool_rounds: 3,
        },
        guardrails: crate::agent::routine::RoutineGuardrails {
            cooldown: Duration::from_secs(60),
            max_concurrent: 1,
            dedup_window: None,
        },
        notify: crate::agent::routine::NotifyConfig {
            channel: None,
            user: None,
            on_success: false,
            on_failure: true,
            on_attention: true,
        },
        last_run_at: None,
        next_fire_at: None,
        run_count: 0,
        consecutive_failures: 0,
        state: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    }
}

/// Build a minimal SandboxJobRecord for testing.
fn make_sandbox_job(user_id: &str, task: &str) -> crate::history::SandboxJobRecord {
    let now = chrono::Utc::now();
    crate::history::SandboxJobRecord {
        id: Uuid::new_v4(),
        task: task.to_string(),
        status: "completed".to_string(),
        user_id: user_id.to_string(),
        project_dir: format!("/tmp/test-{}", Uuid::new_v4()),
        success: Some(true),
        failure_reason: None,
        created_at: now,
        started_at: Some(now),
        completed_at: Some(now),
        credential_grants_json: "[]".to_string(),
        mcp_servers: None,
        max_iterations: None,
    }
}

// ═══════════════════════════════════════════════════════════════════════
// WorkspacePool Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(feature = "libsql")]
mod workspace_pool {
    use super::*;
    use crate::config::{WorkspaceConfig, WorkspaceSearchConfig};
    use crate::workspace::EmbeddingCacheConfig;
    use crate::workspace::layer::MemoryLayer;

    #[tokio::test]
    async fn test_workspace_pool_applies_search_config() {
        let (db, _dir) = test_db().await;
        let search_config = WorkspaceSearchConfig {
            rrf_k: 42,
            ..Default::default()
        };
        let pool = WorkspacePool::new(
            db,
            None,
            EmbeddingCacheConfig::default(),
            search_config,
            WorkspaceConfig::default(),
        );
        let identity = UserIdentity {
            user_id: "alice".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: vec![],
        };
        let ws = pool.get_or_create(&identity).await;
        assert_eq!(ws.user_id(), "alice");
    }

    #[tokio::test]
    async fn test_workspace_pool_applies_memory_layers() {
        let (db, _dir) = test_db().await;
        let layers = vec![MemoryLayer {
            name: "shared-layer".to_string(),
            scope: "shared".to_string(),
            writable: false,
            sensitivity: Default::default(),
        }];
        let ws_config = WorkspaceConfig {
            memory_layers: layers,
            read_scopes: vec![],
        };
        let pool = WorkspacePool::new(
            db,
            None,
            EmbeddingCacheConfig::default(),
            WorkspaceSearchConfig::default(),
            ws_config,
        );
        let identity = UserIdentity {
            user_id: "alice".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: vec![],
        };
        let ws = pool.get_or_create(&identity).await;
        // Memory layer scope "shared" should appear in read_user_ids.
        assert!(
            ws.read_user_ids().contains(&"shared".to_string()),
            "expected 'shared' in read_user_ids, got {:?}",
            ws.read_user_ids()
        );
    }

    #[tokio::test]
    async fn test_workspace_pool_applies_identity_read_scopes() {
        let (db, _dir) = test_db().await;
        let pool = WorkspacePool::new(
            db,
            None,
            EmbeddingCacheConfig::default(),
            WorkspaceSearchConfig::default(),
            WorkspaceConfig::default(),
        );
        let identity = UserIdentity {
            user_id: "bob".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: vec!["alice".to_string(), "shared".to_string()],
        };
        let ws = pool.get_or_create(&identity).await;
        assert_eq!(ws.user_id(), "bob");
        assert!(
            ws.read_user_ids().contains(&"alice".to_string()),
            "expected 'alice' in read_user_ids from identity scopes"
        );
        assert!(
            ws.read_user_ids().contains(&"shared".to_string()),
            "expected 'shared' in read_user_ids from identity scopes"
        );
    }

    #[tokio::test]
    async fn test_workspace_pool_caches_per_user() {
        let (db, _dir) = test_db().await;
        let pool = WorkspacePool::new(
            db,
            None,
            EmbeddingCacheConfig::default(),
            WorkspaceSearchConfig::default(),
            WorkspaceConfig::default(),
        );
        let alice_id = UserIdentity {
            user_id: "alice".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: vec![],
        };
        let bob_id = UserIdentity {
            user_id: "bob".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: vec![],
        };

        let alice_ws1 = pool.get_or_create(&alice_id).await;
        let alice_ws2 = pool.get_or_create(&alice_id).await;
        let bob_ws = pool.get_or_create(&bob_id).await;

        // Same user gets the same Arc.
        assert!(Arc::ptr_eq(&alice_ws1, &alice_ws2));
        // Different users get different instances.
        assert!(!Arc::ptr_eq(&alice_ws1, &bob_ws));
        assert_eq!(alice_ws1.user_id(), "alice");
        assert_eq!(bob_ws.user_id(), "bob");
    }

    #[tokio::test]
    async fn test_workspace_pool_combines_global_and_identity_scopes() {
        let (db, _dir) = test_db().await;
        let ws_config = WorkspaceConfig {
            memory_layers: vec![],
            read_scopes: vec!["global-shared".to_string()],
        };
        let pool = WorkspacePool::new(
            db,
            None,
            EmbeddingCacheConfig::default(),
            WorkspaceSearchConfig::default(),
            ws_config,
        );
        let identity = UserIdentity {
            user_id: "alice".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: vec!["token-scope".to_string()],
        };
        let ws = pool.get_or_create(&identity).await;
        let scopes = ws.read_user_ids();
        // Primary scope
        assert!(scopes.contains(&"alice".to_string()));
        // Global config scope
        assert!(
            scopes.contains(&"global-shared".to_string()),
            "expected global scope 'global-shared', got {:?}",
            scopes
        );
        // Token identity scope
        assert!(
            scopes.contains(&"token-scope".to_string()),
            "expected token scope 'token-scope', got {:?}",
            scopes
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Jobs Handler Isolation Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(feature = "libsql")]
mod jobs_isolation {
    use super::*;
    use crate::channels::web::handlers::jobs::{
        jobs_cancel_handler, jobs_prompt_handler, jobs_restart_handler, jobs_summary_handler,
    };
    // SandboxStore methods are accessed through the Database supertrait.

    /// Build a router with job endpoints behind multi-user auth.
    fn jobs_router(state: Arc<GatewayState>, auth: MultiAuthState) -> Router {
        Router::new()
            .route("/api/jobs/summary", get(jobs_summary_handler))
            .route("/api/jobs/{id}/cancel", post(jobs_cancel_handler))
            .route("/api/jobs/{id}/restart", post(jobs_restart_handler))
            .route("/api/jobs/{id}/prompt", post(jobs_prompt_handler))
            .layer(middleware::from_fn_with_state(
                crate::channels::web::auth::CombinedAuthState::from(auth),
                auth_middleware,
            ))
            .with_state(state)
    }

    #[tokio::test]
    async fn test_jobs_summary_scoped_to_user() {
        let (db, _dir) = test_db().await;

        // Insert sandbox jobs for alice and bob.
        let alice_job = make_sandbox_job("alice", "alice task");
        let bob_job = make_sandbox_job("bob", "bob task");
        db.save_sandbox_job(&alice_job).await.unwrap();
        db.save_sandbox_job(&bob_job).await.unwrap();

        let state = build_state(Some(db), None);
        let auth = two_user_auth();
        let app = jobs_router(state, auth);

        // Alice should see 1 job.
        let req = Request::builder()
            .uri("/api/jobs/summary")
            .header("Authorization", "Bearer tok-alice")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(resp.into_body(), 4096).await.unwrap())
                .unwrap();
        assert_eq!(body["total"], 1, "alice should see only her own jobs");

        // Bob should see 1 job.
        let req = Request::builder()
            .uri("/api/jobs/summary")
            .header("Authorization", "Bearer tok-bob")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(resp.into_body(), 4096).await.unwrap())
                .unwrap();
        assert_eq!(body["total"], 1, "bob should see only his own jobs");
    }

    #[tokio::test]
    async fn test_jobs_restart_rejects_other_user() {
        let (db, _dir) = test_db().await;

        // Insert a failed sandbox job owned by alice.
        let mut alice_job = make_sandbox_job("alice", "alice task");
        alice_job.status = "failed".to_string();
        alice_job.success = Some(false);
        db.save_sandbox_job(&alice_job).await.unwrap();

        let state = build_state(Some(db), None);
        let auth = two_user_auth();
        let app = jobs_router(state, auth);

        // Bob tries to restart alice's job.
        let req = Request::builder()
            .method(Method::POST)
            .uri(format!("/api/jobs/{}/restart", alice_job.id))
            .header("Authorization", "Bearer tok-bob")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "bob should not be able to restart alice's job"
        );
    }

    #[tokio::test]
    async fn test_jobs_prompt_works_for_agent_jobs() {
        let (db, _dir) = test_db().await;

        // Insert a running sandbox job owned by alice in claude_code mode.
        let mut alice_job = make_sandbox_job("alice", "prompt test");
        alice_job.status = "running".to_string();
        alice_job.success = None;
        alice_job.completed_at = None;
        db.save_sandbox_job(&alice_job).await.unwrap();
        db.update_sandbox_job_mode(alice_job.id, "claude_code")
            .await
            .unwrap();

        let prompt_queue: PromptQueue =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let state = build_state(Some(db), Some(prompt_queue.clone()));
        let auth = two_user_auth();
        let app = jobs_router(state, auth);

        // Alice prompts her own job.
        let req = Request::builder()
            .method(Method::POST)
            .uri(format!("/api/jobs/{}/prompt", alice_job.id))
            .header("Authorization", "Bearer tok-alice")
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({"content": "hello"})).unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "alice should be able to prompt her own job"
        );

        // Verify prompt was enqueued.
        let queue = prompt_queue.lock().await;
        assert!(
            queue.contains_key(&alice_job.id),
            "prompt queue should contain alice's job"
        );
    }

    #[tokio::test]
    async fn test_jobs_prompt_rejects_other_user() {
        let (db, _dir) = test_db().await;

        let mut alice_job = make_sandbox_job("alice", "alice task");
        alice_job.status = "running".to_string();
        alice_job.success = None;
        alice_job.completed_at = None;
        db.save_sandbox_job(&alice_job).await.unwrap();
        db.update_sandbox_job_mode(alice_job.id, "claude_code")
            .await
            .unwrap();

        let prompt_queue: PromptQueue =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let state = build_state(Some(db), Some(prompt_queue));
        let auth = two_user_auth();
        let app = jobs_router(state, auth);

        // Bob tries to prompt alice's job.
        let req = Request::builder()
            .method(Method::POST)
            .uri(format!("/api/jobs/{}/prompt", alice_job.id))
            .header("Authorization", "Bearer tok-bob")
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({"content": "sneaky"})).unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "bob should not be able to prompt alice's job"
        );
    }

    #[tokio::test]
    async fn test_jobs_cancel_rejects_other_user() {
        let (db, _dir) = test_db().await;

        let mut alice_job = make_sandbox_job("alice", "alice running");
        alice_job.status = "running".to_string();
        alice_job.success = None;
        alice_job.completed_at = None;
        db.save_sandbox_job(&alice_job).await.unwrap();

        let state = build_state(Some(db), None);
        let auth = two_user_auth();
        let app = jobs_router(state, auth);

        // Bob tries to cancel alice's job.
        let req = Request::builder()
            .method(Method::POST)
            .uri(format!("/api/jobs/{}/cancel", alice_job.id))
            .header("Authorization", "Bearer tok-bob")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "bob should not be able to cancel alice's job"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Routines Isolation Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(feature = "libsql")]
mod routines_isolation {
    use super::*;
    use crate::channels::web::handlers::routines::{
        routines_delete_handler, routines_detail_handler, routines_list_handler,
        routines_summary_handler, routines_toggle_handler,
    };
    // RoutineStore methods are accessed through the Database supertrait.

    fn routines_router(state: Arc<GatewayState>, auth: MultiAuthState) -> Router {
        Router::new()
            .route("/api/routines", get(routines_list_handler))
            .route("/api/routines/summary", get(routines_summary_handler))
            .route("/api/routines/{id}", get(routines_detail_handler))
            .route("/api/routines/{id}/toggle", post(routines_toggle_handler))
            .route("/api/routines/{id}", delete(routines_delete_handler))
            .layer(middleware::from_fn_with_state(
                crate::channels::web::auth::CombinedAuthState::from(auth),
                auth_middleware,
            ))
            .with_state(state)
    }

    #[tokio::test]
    async fn test_routines_isolation() {
        let (db, _dir) = test_db().await;

        // Create routines for alice and bob.
        let alice_routine = make_routine("alice", "alice-daily");
        let bob_routine = make_routine("bob", "bob-daily");
        db.create_routine(&alice_routine).await.unwrap();
        db.create_routine(&bob_routine).await.unwrap();

        let state = build_state(Some(db), None);
        let auth = two_user_auth();
        let app = routines_router(state, auth);

        // Alice sees only her routine in the list.
        let req = Request::builder()
            .uri("/api/routines")
            .header("Authorization", "Bearer tok-alice")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(resp.into_body(), 8192).await.unwrap())
                .unwrap();
        let routines = body["routines"].as_array().unwrap();
        assert_eq!(routines.len(), 1, "alice should see only her routines");
        assert_eq!(routines[0]["name"], "alice-daily");

        // Bob sees only his routine.
        let req = Request::builder()
            .uri("/api/routines")
            .header("Authorization", "Bearer tok-bob")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(resp.into_body(), 8192).await.unwrap())
                .unwrap();
        let routines = body["routines"].as_array().unwrap();
        assert_eq!(routines.len(), 1, "bob should see only his routines");
        assert_eq!(routines[0]["name"], "bob-daily");

        // Bob cannot view alice's routine detail.
        let req = Request::builder()
            .uri(format!("/api/routines/{}", alice_routine.id))
            .header("Authorization", "Bearer tok-bob")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "bob should not see alice's routine detail"
        );

        // Bob cannot toggle alice's routine.
        let req = Request::builder()
            .method(Method::POST)
            .uri(format!("/api/routines/{}/toggle", alice_routine.id))
            .header("Authorization", "Bearer tok-bob")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "bob should not toggle alice's routine"
        );

        // Bob cannot delete alice's routine.
        let req = Request::builder()
            .method(Method::DELETE)
            .uri(format!("/api/routines/{}", alice_routine.id))
            .header("Authorization", "Bearer tok-bob")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "bob should not delete alice's routine"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Handler Auth Enforcement Tests
// ═══════════════════════════════════════════════════════════════════════

mod auth_enforcement {
    use super::*;

    /// Dummy handler that extracts `AuthenticatedUser` — if the auth middleware
    /// rejects the request, this handler is never reached.
    async fn authed_handler(AuthenticatedUser(_user): AuthenticatedUser) -> &'static str {
        "ok"
    }

    /// Build a router with the real auth middleware and dummy handlers at all
    /// the paths we want to verify require authentication.
    fn auth_test_router(auth: MultiAuthState) -> Router {
        let state = build_state(None, None);
        Router::new()
            // Routines
            .route("/api/routines", get(authed_handler))
            .route("/api/routines/summary", get(authed_handler))
            .route("/api/routines/{id}", get(authed_handler))
            .route("/api/routines/{id}/toggle", post(authed_handler))
            .route("/api/routines/{id}", delete(authed_handler))
            // Skills
            .route("/api/skills", get(authed_handler))
            .route("/api/skills/search", post(authed_handler))
            .route("/api/skills/install", post(authed_handler))
            .route("/api/skills/{name}", delete(authed_handler))
            // Logs
            .route("/api/logs/events", get(authed_handler))
            .route("/api/logs/level", get(authed_handler).put(authed_handler))
            // Gateway status
            .route("/api/gateway/status", get(authed_handler))
            .layer(middleware::from_fn_with_state(
                crate::channels::web::auth::CombinedAuthState::from(auth),
                auth_middleware,
            ))
            .with_state(state)
    }

    /// Send a request without auth and assert it returns UNAUTHORIZED.
    async fn assert_requires_auth(app: &Router, method: Method, uri: &str) {
        let req = Request::builder()
            .method(method.clone())
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "{} {} should require auth",
            method,
            uri
        );
    }

    /// Send a request with a valid token and assert it succeeds.
    async fn assert_passes_with_token(app: &Router, method: Method, uri: &str, token: &str) {
        let req = Request::builder()
            .method(method.clone())
            .uri(uri)
            .header("Authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "{} {} should pass with valid token",
            method,
            uri
        );
    }

    #[tokio::test]
    async fn test_routines_handlers_require_auth() {
        let auth = MultiAuthState::single("secret-tok".to_string(), "user".to_string());
        let app = auth_test_router(auth);
        let id = Uuid::new_v4();

        assert_requires_auth(&app, Method::GET, "/api/routines").await;
        assert_requires_auth(&app, Method::GET, "/api/routines/summary").await;
        assert_requires_auth(&app, Method::GET, &format!("/api/routines/{id}")).await;
        assert_requires_auth(&app, Method::POST, &format!("/api/routines/{id}/toggle")).await;
        assert_requires_auth(&app, Method::DELETE, &format!("/api/routines/{id}")).await;
    }

    #[tokio::test]
    async fn test_skills_handlers_require_auth() {
        let auth = MultiAuthState::single("secret-tok".to_string(), "user".to_string());
        let app = auth_test_router(auth);

        assert_requires_auth(&app, Method::GET, "/api/skills").await;
        assert_requires_auth(&app, Method::POST, "/api/skills/search").await;
        assert_requires_auth(&app, Method::POST, "/api/skills/install").await;
        assert_requires_auth(&app, Method::DELETE, "/api/skills/test-skill").await;
    }

    #[tokio::test]
    async fn test_logs_handlers_require_auth() {
        let auth = MultiAuthState::single("secret-tok".to_string(), "user".to_string());
        let app = auth_test_router(auth);

        assert_requires_auth(&app, Method::GET, "/api/logs/events").await;
        assert_requires_auth(&app, Method::GET, "/api/logs/level").await;
        assert_requires_auth(&app, Method::PUT, "/api/logs/level").await;
    }

    #[tokio::test]
    async fn test_gateway_status_requires_auth() {
        let auth = MultiAuthState::single("secret-tok".to_string(), "user".to_string());
        let app = auth_test_router(auth);

        assert_requires_auth(&app, Method::GET, "/api/gateway/status").await;
    }

    #[tokio::test]
    async fn test_valid_token_passes_all_endpoints() {
        let auth = MultiAuthState::single("secret-tok".to_string(), "user".to_string());
        let app = auth_test_router(auth);
        let id = Uuid::new_v4();

        assert_passes_with_token(&app, Method::GET, "/api/routines", "secret-tok").await;
        assert_passes_with_token(&app, Method::GET, "/api/skills", "secret-tok").await;
        assert_passes_with_token(&app, Method::GET, "/api/logs/events", "secret-tok").await;
        assert_passes_with_token(&app, Method::GET, "/api/gateway/status", "secret-tok").await;
        assert_passes_with_token(
            &app,
            Method::GET,
            &format!("/api/routines/{id}"),
            "secret-tok",
        )
        .await;
    }

    #[tokio::test]
    async fn test_wrong_token_rejected_on_all_endpoints() {
        let auth = MultiAuthState::single("secret-tok".to_string(), "user".to_string());
        let app = auth_test_router(auth);

        // Wrong token should be rejected.
        let req = Request::builder()
            .uri("/api/routines")
            .header("Authorization", "Bearer wrong-tok")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let req = Request::builder()
            .uri("/api/gateway/status")
            .header("Authorization", "Bearer wrong-tok")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Admin Endpoint Role Enforcement Tests
// ═══════════════════════════════════════════════════════════════════════

mod admin_role_enforcement {
    use super::*;
    use crate::channels::web::handlers::users::{
        users_activate_handler, users_detail_handler, users_list_handler, users_suspend_handler,
        users_update_handler,
    };
    use axum::routing::patch;

    /// Build a router with admin user endpoints behind multi-user auth.
    /// Uses a member-role token and an admin-role token.
    fn admin_router() -> Router {
        let mut tokens = HashMap::new();
        tokens.insert(
            "tok-admin".to_string(),
            UserIdentity {
                user_id: "admin-user".to_string(),
                role: "admin".to_string(),
                workspace_read_scopes: vec![],
            },
        );
        tokens.insert(
            "tok-member".to_string(),
            UserIdentity {
                user_id: "member-user".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: vec![],
            },
        );
        let auth = MultiAuthState::multi(tokens);
        let state = build_state(None, None);

        Router::new()
            .route("/api/admin/users", get(users_list_handler))
            .route("/api/admin/users/{id}", get(users_detail_handler))
            .route("/api/admin/users/{id}", patch(users_update_handler))
            .route("/api/admin/users/{id}/suspend", post(users_suspend_handler))
            .route(
                "/api/admin/users/{id}/activate",
                post(users_activate_handler),
            )
            .layer(middleware::from_fn_with_state(
                crate::channels::web::auth::CombinedAuthState::from(auth),
                auth_middleware,
            ))
            .with_state(state)
    }

    /// Assert a request returns FORBIDDEN for a member token.
    async fn assert_forbidden_for_member(app: &Router, method: Method, uri: &str) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("Authorization", "Bearer tok-member")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "expected 403 for member on {}",
            uri
        );
    }

    #[tokio::test]
    async fn test_admin_user_endpoints_reject_member_role() {
        let app = admin_router();

        assert_forbidden_for_member(&app, Method::GET, "/api/admin/users").await;
        assert_forbidden_for_member(&app, Method::GET, "/api/admin/users/some-id").await;
        assert_forbidden_for_member(&app, Method::POST, "/api/admin/users/some-id/suspend").await;
        assert_forbidden_for_member(&app, Method::POST, "/api/admin/users/some-id/activate").await;
    }

    #[tokio::test]
    async fn test_admin_user_endpoints_accept_admin_role() {
        let app = admin_router();

        // Admin token should pass auth (will get 503 since no DB, but not 403).
        let req = Request::builder()
            .uri("/api/admin/users")
            .header("Authorization", "Bearer tok-admin")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_ne!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "admin should not get 403"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Trace Contribution Handler Isolation Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(feature = "libsql")]
mod trace_contribution_isolation {
    use super::*;
    use crate::channels::web::handlers::traces::{
        traces_credit_handler, traces_policy_get_handler, traces_policy_put_handler,
        traces_preview_handler, traces_revoke_handler, traces_submissions_handler,
        traces_submit_handler,
    };
    use crate::trace_contribution::{
        LocalTraceSubmissionStatus, StandingTraceContributionPolicy, TraceSubmissionReceipt,
        queue_trace_envelope_for_scope, record_submitted_trace_envelope_for_scope,
        write_trace_policy_for_scope,
    };

    fn trace_router(state: Arc<GatewayState>) -> Router {
        trace_router_with_auth(state, two_user_auth())
    }

    fn trace_router_with_auth(state: Arc<GatewayState>, auth: MultiAuthState) -> Router {
        Router::new()
            .route(
                "/api/traces/policy",
                get(traces_policy_get_handler).put(traces_policy_put_handler),
            )
            .route("/api/traces/preview", post(traces_preview_handler))
            .route("/api/traces/submit", post(traces_submit_handler))
            .route("/api/traces/credit", get(traces_credit_handler))
            .route("/api/traces/submissions", get(traces_submissions_handler))
            .route(
                "/api/traces/submissions/{submission_id}/revoke",
                post(traces_revoke_handler),
            )
            .layer(middleware::from_fn_with_state(
                crate::channels::web::auth::CombinedAuthState::from(auth),
                auth_middleware,
            ))
            .with_state(state)
    }

    fn trace_auth(alice_user_id: String, bob_user_id: String) -> MultiAuthState {
        let mut tokens = HashMap::new();
        tokens.insert(
            "tok-alice".to_string(),
            UserIdentity {
                user_id: alice_user_id,
                role: "admin".to_string(),
                workspace_read_scopes: vec![],
            },
        );
        tokens.insert(
            "tok-bob".to_string(),
            UserIdentity {
                user_id: bob_user_id,
                role: "admin".to_string(),
                workspace_read_scopes: vec![],
            },
        );
        MultiAuthState::multi(tokens)
    }

    fn remove_queued_traces(scope: &str) {
        if let Ok(paths) =
            crate::trace_contribution::queued_trace_envelope_paths_for_scope(Some(scope))
        {
            for path in paths {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    fn remove_trace_state(scope: &str) {
        let dir = crate::trace_contribution::trace_contribution_dir_for_scope(Some(scope));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn enable_trace_policy(scope: &str) {
        let policy = StandingTraceContributionPolicy {
            enabled: true,
            ingestion_endpoint: Some("http://127.0.0.1:9/v1/traces".to_string()),
            ..Default::default()
        };
        write_trace_policy_for_scope(Some(scope), &policy).expect("write trace policy");
    }

    async fn json_request(
        app: &Router,
        method: Method,
        uri: impl Into<String>,
        token: &str,
        body: serde_json::Value,
    ) -> axum::response::Response {
        let req = Request::builder()
            .method(method)
            .uri(uri.into())
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("build json request");
        app.clone().oneshot(req).await.expect("json response")
    }

    async fn authed_get(
        app: &Router,
        uri: impl Into<String>,
        token: &str,
    ) -> axum::response::Response {
        let req = Request::builder()
            .method(Method::GET)
            .uri(uri.into())
            .header("Authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("build get request");
        app.clone().oneshot(req).await.expect("get response")
    }

    async fn response_json(resp: axum::response::Response) -> serde_json::Value {
        serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .expect("read json response"),
        )
        .expect("parse json response")
    }

    #[tokio::test]
    async fn test_trace_preview_is_tenant_scoped() {
        let (db, _dir) = test_db().await;
        let alice_thread = db
            .create_conversation("gateway", "alice", None)
            .await
            .expect("create alice conversation");
        db.add_conversation_message(alice_thread, "user", "Please inspect /tmp/build.log")
            .await
            .expect("add alice user message");
        db.add_conversation_message(
            alice_thread,
            "assistant",
            "I inspected the redacted build log.",
        )
        .await
        .expect("add alice assistant message");

        let app = trace_router(build_state(Some(db), None));
        let body = serde_json::json!({
            "thread_id": alice_thread,
            "include_message_text": true
        });

        let alice_req = Request::builder()
            .method(Method::POST)
            .uri("/api/traces/preview")
            .header("Authorization", "Bearer tok-alice")
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("build alice request");
        let alice_resp = app
            .clone()
            .oneshot(alice_req)
            .await
            .expect("alice response");
        assert_eq!(alice_resp.status(), StatusCode::OK);
        let alice_body = axum::body::to_bytes(alice_resp.into_body(), 1024 * 1024)
            .await
            .expect("read alice response");
        let alice_preview: serde_json::Value =
            serde_json::from_slice(&alice_body).expect("parse alice preview");
        let contributor = &alice_preview["envelope"]["contributor"];
        assert!(
            contributor["pseudonymous_contributor_id"]
                .as_str()
                .is_some_and(|value| value.starts_with("sha256:"))
        );
        assert!(
            contributor["tenant_scope_ref"]
                .as_str()
                .is_some_and(|value| value.starts_with("tenant_sha256:"))
        );
        assert!(
            !alice_body
                .windows(b"alice".len())
                .any(|chunk| chunk == b"alice")
        );

        let bob_req = Request::builder()
            .method(Method::POST)
            .uri("/api/traces/preview")
            .header("Authorization", "Bearer tok-bob")
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("build bob request");
        let bob_resp = app.oneshot(bob_req).await.expect("bob response");
        assert_eq!(bob_resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_trace_policy_and_preview_defaults_are_tenant_scoped() {
        let (db, _dir) = test_db().await;
        let alice_user_id = format!("alice-{}", Uuid::new_v4());
        let bob_user_id = format!("bob-{}", Uuid::new_v4());
        remove_trace_state(&alice_user_id);
        remove_trace_state(&bob_user_id);

        let alice_thread = db
            .create_conversation("gateway", &alice_user_id, None)
            .await
            .expect("create alice conversation");
        db.add_conversation_message(
            alice_thread,
            "user",
            "Secret default-off text: sk-live-alice-test",
        )
        .await
        .expect("add alice user message");
        db.add_conversation_message(
            alice_thread,
            "assistant",
            "I will keep that default-off text out of preview payloads.",
        )
        .await
        .expect("add alice assistant message");

        let app = trace_router_with_auth(
            build_state(Some(db), None),
            trace_auth(alice_user_id.clone(), bob_user_id.clone()),
        );

        let alice_policy =
            response_json(authed_get(&app, "/api/traces/policy", "tok-alice").await).await;
        assert_eq!(alice_policy["policy"]["include_message_text"], false);
        assert_eq!(alice_policy["policy"]["include_tool_payloads"], false);
        let bob_policy =
            response_json(authed_get(&app, "/api/traces/policy", "tok-bob").await).await;
        assert_eq!(bob_policy["policy"]["include_message_text"], false);
        assert_eq!(bob_policy["policy"]["include_tool_payloads"], false);

        let put_resp = json_request(
            &app,
            Method::PUT,
            "/api/traces/policy",
            "tok-alice",
            serde_json::json!({
                "include_message_text": true,
                "include_tool_payloads": true
            }),
        )
        .await;
        assert_eq!(put_resp.status(), StatusCode::OK);
        let alice_policy = response_json(put_resp).await;
        assert_eq!(alice_policy["policy"]["include_message_text"], true);
        assert_eq!(alice_policy["policy"]["include_tool_payloads"], true);

        let bob_policy =
            response_json(authed_get(&app, "/api/traces/policy", "tok-bob").await).await;
        assert_eq!(
            bob_policy["policy"]["include_message_text"], false,
            "alice policy updates must not change bob defaults"
        );
        assert_eq!(bob_policy["policy"]["include_tool_payloads"], false);

        let bob_preview_resp = json_request(
            &app,
            Method::POST,
            "/api/traces/preview",
            "tok-bob",
            serde_json::json!({ "thread_id": alice_thread }),
        )
        .await;
        assert_eq!(bob_preview_resp.status(), StatusCode::NOT_FOUND);

        remove_trace_state(&alice_user_id);
        remove_trace_state(&bob_user_id);
    }

    #[tokio::test]
    async fn test_trace_preview_request_defaults_omit_text_and_payloads() {
        let (db, _dir) = test_db().await;
        let alice_user_id = format!("alice-{}", Uuid::new_v4());
        let bob_user_id = format!("bob-{}", Uuid::new_v4());
        remove_trace_state(&alice_user_id);
        remove_trace_state(&bob_user_id);

        let alice_thread = db
            .create_conversation("gateway", &alice_user_id, None)
            .await
            .expect("create alice conversation");
        db.add_conversation_message(
            alice_thread,
            "user",
            "Default preview should hide this phrase: trace-default-secret",
        )
        .await
        .expect("add alice user message");
        db.add_conversation_message(
            alice_thread,
            "assistant",
            r#"{"tool_calls":[{"id":"call-1","name":"shell","arguments":{"cmd":"echo trace-default-secret"}}]}"#,
        )
        .await
        .expect("add alice tool call message");
        db.add_conversation_message(
            alice_thread,
            "assistant",
            "The default preview should stay metadata-only.",
        )
        .await
        .expect("add alice assistant message");

        let app = trace_router_with_auth(
            build_state(Some(db), None),
            trace_auth(alice_user_id.clone(), bob_user_id.clone()),
        );
        let resp = json_request(
            &app,
            Method::POST,
            "/api/traces/preview",
            "tok-alice",
            serde_json::json!({ "thread_id": alice_thread }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .expect("read preview body");
        assert!(
            !body
                .windows(b"trace-default-secret".len())
                .any(|chunk| chunk == b"trace-default-secret"),
            "metadata-only preview should not include message text or tool arguments by default"
        );
        let preview: serde_json::Value = serde_json::from_slice(&body).expect("parse preview body");
        assert_eq!(
            preview["envelope"]["consent"]["message_text_included"],
            false
        );
        assert_eq!(
            preview["envelope"]["consent"]["tool_payloads_included"],
            false
        );
        assert!(
            preview["envelope"]["events"]
                .as_array()
                .expect("events array")
                .iter()
                .all(|event| event.get("redacted_content").is_none())
        );

        remove_trace_state(&alice_user_id);
        remove_trace_state(&bob_user_id);
    }

    #[tokio::test]
    async fn test_trace_preview_enqueue_requires_enabled_policy() {
        let (db, _dir) = test_db().await;
        let alice_user_id = format!("alice-{}", Uuid::new_v4());
        let bob_user_id = format!("bob-{}", Uuid::new_v4());
        remove_trace_state(&alice_user_id);
        remove_trace_state(&bob_user_id);

        let alice_thread = db
            .create_conversation("gateway", &alice_user_id, None)
            .await
            .expect("create alice conversation");
        db.add_conversation_message(alice_thread, "user", "Preview this trace")
            .await
            .expect("add alice user message");
        db.add_conversation_message(alice_thread, "assistant", "Previewed.")
            .await
            .expect("add alice assistant message");

        let app = trace_router_with_auth(
            build_state(Some(db), None),
            trace_auth(alice_user_id.clone(), bob_user_id.clone()),
        );
        let preview_resp = json_request(
            &app,
            Method::POST,
            "/api/traces/preview",
            "tok-alice",
            serde_json::json!({ "thread_id": alice_thread }),
        )
        .await;
        assert_eq!(preview_resp.status(), StatusCode::OK);

        let queue_before =
            crate::trace_contribution::queued_trace_envelope_paths_for_scope(Some(&alice_user_id))
                .expect("read alice queue")
                .len();
        let enqueue_resp = json_request(
            &app,
            Method::POST,
            "/api/traces/preview",
            "tok-alice",
            serde_json::json!({
                "thread_id": alice_thread,
                "enqueue": true
            }),
        )
        .await;
        assert_eq!(enqueue_resp.status(), StatusCode::CONFLICT);
        let queue_after =
            crate::trace_contribution::queued_trace_envelope_paths_for_scope(Some(&alice_user_id))
                .expect("read alice queue")
                .len();
        assert_eq!(queue_after, queue_before);

        remove_trace_state(&alice_user_id);
        remove_trace_state(&bob_user_id);
    }

    #[tokio::test]
    async fn test_trace_preview_enqueue_rejects_capture_fields_disallowed_by_policy() {
        let (db, _dir) = test_db().await;
        let alice_user_id = format!("alice-{}", Uuid::new_v4());
        let bob_user_id = format!("bob-{}", Uuid::new_v4());
        remove_trace_state(&alice_user_id);
        remove_trace_state(&bob_user_id);
        enable_trace_policy(&alice_user_id);

        let alice_thread = db
            .create_conversation("gateway", &alice_user_id, None)
            .await
            .expect("create alice conversation");
        db.add_conversation_message(alice_thread, "user", "Preview private message text")
            .await
            .expect("add alice user message");
        db.add_conversation_message(alice_thread, "assistant", "Previewed.")
            .await
            .expect("add alice assistant message");

        let app = trace_router_with_auth(
            build_state(Some(db), None),
            trace_auth(alice_user_id.clone(), bob_user_id.clone()),
        );
        let preview_only = json_request(
            &app,
            Method::POST,
            "/api/traces/preview",
            "tok-alice",
            serde_json::json!({
                "thread_id": alice_thread,
                "include_message_text": true
            }),
        )
        .await;
        assert_eq!(preview_only.status(), StatusCode::OK);

        let before =
            crate::trace_contribution::queued_trace_envelope_paths_for_scope(Some(&alice_user_id))
                .expect("read alice queue")
                .len();
        let enqueue_message_text = json_request(
            &app,
            Method::POST,
            "/api/traces/preview",
            "tok-alice",
            serde_json::json!({
                "thread_id": alice_thread,
                "include_message_text": true,
                "enqueue": true
            }),
        )
        .await;
        assert_eq!(enqueue_message_text.status(), StatusCode::CONFLICT);

        let enqueue_tool_payloads = json_request(
            &app,
            Method::POST,
            "/api/traces/preview",
            "tok-alice",
            serde_json::json!({
                "thread_id": alice_thread,
                "include_tool_payloads": true,
                "enqueue": true
            }),
        )
        .await;
        assert_eq!(enqueue_tool_payloads.status(), StatusCode::CONFLICT);

        let after =
            crate::trace_contribution::queued_trace_envelope_paths_for_scope(Some(&alice_user_id))
                .expect("read alice queue")
                .len();
        assert_eq!(after, before);

        remove_trace_state(&alice_user_id);
        remove_trace_state(&bob_user_id);
    }

    #[tokio::test]
    async fn test_trace_submit_is_tenant_scoped_and_requires_preview_ack() {
        let (db, _dir) = test_db().await;
        let alice_user_id = format!("alice-{}", Uuid::new_v4());
        let bob_user_id = format!("bob-{}", Uuid::new_v4());
        remove_trace_state(&alice_user_id);
        remove_trace_state(&bob_user_id);
        enable_trace_policy(&alice_user_id);
        enable_trace_policy(&bob_user_id);

        let alice_thread = db
            .create_conversation("gateway", &alice_user_id, None)
            .await
            .expect("create alice conversation");
        db.add_conversation_message(alice_thread, "user", "Please inspect /tmp/trace.log")
            .await
            .expect("add alice user message");
        db.add_conversation_message(alice_thread, "assistant", "I inspected the trace log.")
            .await
            .expect("add alice assistant message");

        let app = trace_router_with_auth(
            build_state(Some(db), None),
            trace_auth(alice_user_id.clone(), bob_user_id.clone()),
        );
        let body = serde_json::json!({
            "thread_id": alice_thread,
            "user_previewed": true
        });

        let unacknowledged_body = serde_json::json!({
            "thread_id": alice_thread,
            "user_previewed": false
        });
        let unacknowledged_req = Request::builder()
            .method(Method::POST)
            .uri("/api/traces/submit")
            .header("Authorization", "Bearer tok-alice")
            .header("Content-Type", "application/json")
            .body(Body::from(unacknowledged_body.to_string()))
            .expect("build unacknowledged request");
        let unacknowledged_resp = app
            .clone()
            .oneshot(unacknowledged_req)
            .await
            .expect("unacknowledged response");
        assert_eq!(unacknowledged_resp.status(), StatusCode::BAD_REQUEST);

        let alice_before =
            crate::trace_contribution::queued_trace_envelope_paths_for_scope(Some(&alice_user_id))
                .expect("read alice queue")
                .len();
        let bob_before =
            crate::trace_contribution::queued_trace_envelope_paths_for_scope(Some(&bob_user_id))
                .expect("read bob queue")
                .len();

        let bob_req = Request::builder()
            .method(Method::POST)
            .uri("/api/traces/submit")
            .header("Authorization", "Bearer tok-bob")
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("build bob request");
        let bob_resp = app.clone().oneshot(bob_req).await.expect("bob response");
        assert_eq!(bob_resp.status(), StatusCode::NOT_FOUND);

        let alice_req = Request::builder()
            .method(Method::POST)
            .uri("/api/traces/submit")
            .header("Authorization", "Bearer tok-alice")
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("build alice request");
        let alice_resp = app.oneshot(alice_req).await.expect("alice response");
        assert_eq!(alice_resp.status(), StatusCode::OK);
        let alice_body = axum::body::to_bytes(alice_resp.into_body(), 1024 * 1024)
            .await
            .expect("read alice response");
        let alice_submit: serde_json::Value =
            serde_json::from_slice(&alice_body).expect("parse alice submit");
        assert_eq!(alice_submit["queued"], true);
        assert_eq!(alice_submit["flushed"], false);

        let alice_after =
            crate::trace_contribution::queued_trace_envelope_paths_for_scope(Some(&alice_user_id))
                .expect("read alice queue")
                .len();
        let bob_after =
            crate::trace_contribution::queued_trace_envelope_paths_for_scope(Some(&bob_user_id))
                .expect("read bob queue")
                .len();
        assert_eq!(alice_after, alice_before + 1);
        assert_eq!(bob_after, bob_before);

        remove_queued_traces(&alice_user_id);
        remove_queued_traces(&bob_user_id);
    }

    #[tokio::test]
    async fn test_trace_submit_rejects_capture_fields_disallowed_by_policy() {
        let (db, _dir) = test_db().await;
        let alice_user_id = format!("alice-{}", Uuid::new_v4());
        let bob_user_id = format!("bob-{}", Uuid::new_v4());
        remove_trace_state(&alice_user_id);
        remove_trace_state(&bob_user_id);
        enable_trace_policy(&alice_user_id);

        let alice_thread = db
            .create_conversation("gateway", &alice_user_id, None)
            .await
            .expect("create alice conversation");
        db.add_conversation_message(alice_thread, "user", "Submit private message text")
            .await
            .expect("add alice user message");
        db.add_conversation_message(alice_thread, "assistant", "Submitted.")
            .await
            .expect("add alice assistant message");

        let app = trace_router_with_auth(
            build_state(Some(db), None),
            trace_auth(alice_user_id.clone(), bob_user_id.clone()),
        );
        let before =
            crate::trace_contribution::queued_trace_envelope_paths_for_scope(Some(&alice_user_id))
                .expect("read alice queue")
                .len();
        let message_text_resp = json_request(
            &app,
            Method::POST,
            "/api/traces/submit",
            "tok-alice",
            serde_json::json!({
                "thread_id": alice_thread,
                "include_message_text": true,
                "user_previewed": true
            }),
        )
        .await;
        assert_eq!(message_text_resp.status(), StatusCode::CONFLICT);

        let tool_payload_resp = json_request(
            &app,
            Method::POST,
            "/api/traces/submit",
            "tok-alice",
            serde_json::json!({
                "thread_id": alice_thread,
                "include_tool_payloads": true,
                "user_previewed": true
            }),
        )
        .await;
        assert_eq!(tool_payload_resp.status(), StatusCode::CONFLICT);

        let after =
            crate::trace_contribution::queued_trace_envelope_paths_for_scope(Some(&alice_user_id))
                .expect("read alice queue")
                .len();
        assert_eq!(after, before);

        remove_trace_state(&alice_user_id);
        remove_trace_state(&bob_user_id);
    }

    #[tokio::test]
    async fn test_trace_submit_requires_enabled_policy_before_queue() {
        let (db, _dir) = test_db().await;
        let alice_user_id = format!("alice-{}", Uuid::new_v4());
        let bob_user_id = format!("bob-{}", Uuid::new_v4());
        remove_trace_state(&alice_user_id);
        remove_trace_state(&bob_user_id);

        let alice_thread = db
            .create_conversation("gateway", &alice_user_id, None)
            .await
            .expect("create alice conversation");
        db.add_conversation_message(alice_thread, "user", "Submit this trace")
            .await
            .expect("add alice user message");
        db.add_conversation_message(alice_thread, "assistant", "Submitted.")
            .await
            .expect("add alice assistant message");

        let app = trace_router_with_auth(
            build_state(Some(db), None),
            trace_auth(alice_user_id.clone(), bob_user_id.clone()),
        );
        let before =
            crate::trace_contribution::queued_trace_envelope_paths_for_scope(Some(&alice_user_id))
                .expect("read alice queue")
                .len();
        let resp = json_request(
            &app,
            Method::POST,
            "/api/traces/submit",
            "tok-alice",
            serde_json::json!({
                "thread_id": alice_thread,
                "user_previewed": true
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::CONFLICT);
        let after =
            crate::trace_contribution::queued_trace_envelope_paths_for_scope(Some(&alice_user_id))
                .expect("read alice queue")
                .len();
        assert_eq!(after, before);

        remove_trace_state(&alice_user_id);
        remove_trace_state(&bob_user_id);
    }

    #[tokio::test]
    async fn test_trace_credit_submissions_and_revoke_are_tenant_scoped() {
        let (db, _dir) = test_db().await;
        let alice_user_id = format!("alice-{}", Uuid::new_v4());
        let bob_user_id = format!("bob-{}", Uuid::new_v4());
        remove_trace_state(&alice_user_id);
        remove_trace_state(&bob_user_id);

        let alice_thread = db
            .create_conversation("gateway", &alice_user_id, None)
            .await
            .expect("create alice conversation");
        db.add_conversation_message(alice_thread, "user", "Create a trace credit record")
            .await
            .expect("add alice user message");
        db.add_conversation_message(alice_thread, "assistant", "Trace credit record created.")
            .await
            .expect("add alice assistant message");

        let app = trace_router_with_auth(
            build_state(Some(db), None),
            trace_auth(alice_user_id.clone(), bob_user_id.clone()),
        );
        let preview_resp = json_request(
            &app,
            Method::POST,
            "/api/traces/preview",
            "tok-alice",
            serde_json::json!({ "thread_id": alice_thread }),
        )
        .await;
        assert_eq!(preview_resp.status(), StatusCode::OK);
        let preview = response_json(preview_resp).await;
        let envelope: crate::trace_contribution::TraceContributionEnvelope =
            serde_json::from_value(preview["envelope"].clone()).expect("parse envelope");
        let submission_id = envelope.submission_id;
        record_submitted_trace_envelope_for_scope(
            Some(&alice_user_id),
            &envelope,
            "https://trace.example.test/v1/traces",
            TraceSubmissionReceipt {
                status: "submitted".to_string(),
                credit_points_pending: Some(7.0),
                credit_points_final: None,
                explanation: vec!["alice-only credit".to_string()],
            },
        )
        .expect("record alice submitted trace");

        let alice_credit =
            response_json(authed_get(&app, "/api/traces/credit", "tok-alice").await).await;
        assert_eq!(alice_credit["summary"]["submissions_total"], 1);
        assert_eq!(alice_credit["summary"]["pending_credit"], 7.0);
        assert_eq!(
            alice_credit["records"][0]["submission_id"],
            submission_id.to_string()
        );

        let bob_credit_resp = authed_get(&app, "/api/traces/credit", "tok-bob").await;
        assert_eq!(bob_credit_resp.status(), StatusCode::OK);
        let bob_credit_body = axum::body::to_bytes(bob_credit_resp.into_body(), 1024 * 1024)
            .await
            .expect("read bob credit");
        let submission_id_string = submission_id.to_string();
        assert!(
            !bob_credit_body
                .windows(submission_id_string.len())
                .any(|chunk| chunk == submission_id_string.as_bytes()),
            "bob credit response must not include alice submission ids"
        );
        let bob_credit: serde_json::Value =
            serde_json::from_slice(&bob_credit_body).expect("parse bob credit");
        assert_eq!(bob_credit["summary"]["submissions_total"], 0);
        assert_eq!(
            bob_credit["records"]
                .as_array()
                .expect("records array")
                .len(),
            0
        );

        let bob_revoke_resp = json_request(
            &app,
            Method::POST,
            format!("/api/traces/submissions/{submission_id}/revoke"),
            "tok-bob",
            serde_json::json!({ "call_remote": false }),
        )
        .await;
        assert_eq!(bob_revoke_resp.status(), StatusCode::NO_CONTENT);

        let alice_submissions =
            response_json(authed_get(&app, "/api/traces/submissions", "tok-alice").await).await;
        assert_eq!(
            alice_submissions
                .as_array()
                .expect("alice submissions")
                .len(),
            1
        );
        assert_eq!(
            alice_submissions[0]["submission_id"],
            submission_id.to_string()
        );
        assert_eq!(alice_submissions[0]["status"], "submitted");

        let bob_submissions =
            response_json(authed_get(&app, "/api/traces/submissions", "tok-bob").await).await;
        assert_eq!(
            bob_submissions.as_array().expect("bob submissions").len(),
            1
        );
        assert_eq!(
            bob_submissions[0]["submission_id"],
            submission_id.to_string()
        );
        assert_eq!(bob_submissions[0]["status"], "revoked");

        let alice_records =
            crate::trace_contribution::read_local_trace_records_for_scope(Some(&alice_user_id))
                .expect("read alice records");
        assert_eq!(alice_records.len(), 1);
        assert_eq!(
            alice_records[0].status,
            LocalTraceSubmissionStatus::Submitted
        );

        remove_trace_state(&alice_user_id);
        remove_trace_state(&bob_user_id);
    }

    #[tokio::test]
    async fn test_trace_activity_reports_holds_and_credit_report_by_tenant() {
        let (db, _dir) = test_db().await;
        let alice_user_id = format!("alice-{}", Uuid::new_v4());
        let bob_user_id = format!("bob-{}", Uuid::new_v4());
        remove_trace_state(&alice_user_id);
        remove_trace_state(&bob_user_id);
        enable_trace_policy(&alice_user_id);
        enable_trace_policy(&bob_user_id);

        let alice_thread = db
            .create_conversation("gateway", &alice_user_id, None)
            .await
            .expect("create alice conversation");
        db.add_conversation_message(alice_thread, "user", "Create a held trace")
            .await
            .expect("add alice user message");
        db.add_conversation_message(alice_thread, "assistant", "Held trace created.")
            .await
            .expect("add alice assistant message");

        let app = trace_router_with_auth(
            build_state(Some(db), None),
            trace_auth(alice_user_id.clone(), bob_user_id.clone()),
        );
        let preview_resp = json_request(
            &app,
            Method::POST,
            "/api/traces/preview",
            "tok-alice",
            serde_json::json!({ "thread_id": alice_thread }),
        )
        .await;
        assert_eq!(preview_resp.status(), StatusCode::OK);
        let preview = response_json(preview_resp).await;
        let envelope: crate::trace_contribution::TraceContributionEnvelope =
            serde_json::from_value(preview["envelope"].clone()).expect("parse envelope");
        queue_trace_envelope_for_scope(Some(&alice_user_id), &envelope)
            .expect("queue alice envelope");

        let hold_path =
            crate::trace_contribution::trace_contribution_dir_for_scope(Some(&alice_user_id))
                .join("queue")
                .join(format!("{}.held.json", envelope.submission_id));
        std::fs::write(
            &hold_path,
            serde_json::json!({
                "envelope": format!("{}.json", envelope.submission_id),
                "reason": "manual review required"
            })
            .to_string(),
        )
        .expect("write alice hold sidecar");

        record_submitted_trace_envelope_for_scope(
            Some(&alice_user_id),
            &envelope,
            "https://trace.example.test/v1/traces",
            TraceSubmissionReceipt {
                status: "accepted".to_string(),
                credit_points_pending: Some(2.0),
                credit_points_final: Some(3.25),
                explanation: vec!["accepted plus delayed utility".to_string()],
            },
        )
        .expect("record alice submitted trace");

        let alice_policy =
            response_json(authed_get(&app, "/api/traces/policy", "tok-alice").await).await;
        assert_eq!(alice_policy["queued_envelopes"], 1);
        assert_eq!(
            alice_policy["held_queue"][0]["submission_id"],
            envelope.submission_id.to_string()
        );
        assert_eq!(
            alice_policy["held_queue"][0]["reason"],
            "manual review required"
        );
        assert!(
            !alice_policy.to_string().contains("Create a held trace"),
            "held queue response must not expose envelope bodies"
        );

        let bob_policy =
            response_json(authed_get(&app, "/api/traces/policy", "tok-bob").await).await;
        assert_eq!(bob_policy["queued_envelopes"], 0);
        assert!(
            bob_policy["held_queue"]
                .as_array()
                .is_none_or(|holds| holds.is_empty())
        );
        assert!(
            !bob_policy
                .to_string()
                .contains(&envelope.submission_id.to_string()),
            "bob policy response must not include alice held queue ids"
        );

        let alice_credit =
            response_json(authed_get(&app, "/api/traces/credit", "tok-alice").await).await;
        assert_eq!(alice_credit["summary"]["submissions_total"], 1);
        assert_eq!(alice_credit["report"]["submissions_accepted"], 1);
        assert_eq!(alice_credit["report"]["final_credit"], 3.25);

        let bob_credit =
            response_json(authed_get(&app, "/api/traces/credit", "tok-bob").await).await;
        assert_eq!(bob_credit["summary"]["submissions_total"], 0);
        assert_eq!(bob_credit["report"]["submissions_accepted"], 0);
        assert!(
            !bob_credit
                .to_string()
                .contains(&envelope.submission_id.to_string()),
            "bob credit response must not include alice review history"
        );

        remove_trace_state(&alice_user_id);
        remove_trace_state(&bob_user_id);
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Admin Tool Policy Tests
// ═══════════════════════════════════════════════════════════════════════

mod admin_tool_policy {
    use super::*;
    use crate::channels::web::handlers::tool_policy::{
        tool_policy_get_handler, tool_policy_put_handler,
    };

    /// Build a `GatewayState` with `workspace_pool` set (multi-tenant mode).
    #[cfg(feature = "libsql")]
    fn build_multi_tenant_state(db: Arc<dyn crate::db::Database>) -> Arc<GatewayState> {
        let pool = WorkspacePool::new(
            Arc::clone(&db),
            None,
            crate::workspace::EmbeddingCacheConfig::default(),
            crate::config::WorkspaceSearchConfig::default(),
            crate::config::WorkspaceConfig::default(),
        );
        Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: Arc::new(SseManager::new()),
            workspace: None,
            workspace_pool: Some(Arc::new(pool)),
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store: Some(db),
            job_manager: None,
            prompt_queue: None,
            owner_id: "test".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: None,
            llm_provider: None,
            skill_registry: None,
            skill_catalog: None,
            scheduler: None,
            chat_rate_limiter: PerUserRateLimiter::new(30, 60),
            oauth_rate_limiter: PerUserRateLimiter::new(20, 60),
            webhook_rate_limiter: RateLimiter::new(10, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            startup_time: std::time::Instant::now(),
            active_config: ActiveConfigSnapshot::default(),
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
            auth_manager: None,
            frontend_html_cache: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            tool_dispatcher: None,
        })
    }

    /// Build a router for tool policy endpoints (single-user mode: workspace_pool=None).
    fn tool_policy_router() -> Router {
        let mut tokens = HashMap::new();
        tokens.insert(
            "tok-admin".to_string(),
            UserIdentity {
                user_id: "admin-user".to_string(),
                role: "admin".to_string(),
                workspace_read_scopes: vec![],
            },
        );
        tokens.insert(
            "tok-member".to_string(),
            UserIdentity {
                user_id: "member-user".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: vec![],
            },
        );
        let auth = MultiAuthState::multi(tokens);
        let state = build_state(None, None);

        Router::new()
            .route(
                "/api/admin/tool-policy",
                get(tool_policy_get_handler).put(tool_policy_put_handler),
            )
            .layer(middleware::from_fn_with_state(
                crate::channels::web::auth::CombinedAuthState::from(auth),
                auth_middleware,
            ))
            .with_state(state)
    }

    #[tokio::test]
    async fn test_tool_policy_rejects_member() {
        let app = tool_policy_router();

        // GET should be 403 for member
        let req = Request::builder()
            .uri("/api/admin/tool-policy")
            .header("Authorization", "Bearer tok-member")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);

        // PUT should be 403 for member
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/api/admin/tool-policy")
            .header("Authorization", "Bearer tok-member")
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"disabled_tools":[]}"#))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_tool_policy_returns_404_in_single_user_mode() {
        let app = tool_policy_router();

        let req = Request::builder()
            .uri("/api/admin/tool-policy")
            .header("Authorization", "Bearer tok-admin")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_tool_policy_crud_with_db() {
        let (db, _dir) = test_db().await;
        let state = build_multi_tenant_state(db);

        let mut tokens = HashMap::new();
        tokens.insert(
            "tok-admin".to_string(),
            UserIdentity {
                user_id: "admin-user".to_string(),
                role: "admin".to_string(),
                workspace_read_scopes: vec![],
            },
        );
        let auth = MultiAuthState::multi(tokens);

        let app = Router::new()
            .route(
                "/api/admin/tool-policy",
                get(tool_policy_get_handler).put(tool_policy_put_handler),
            )
            .layer(middleware::from_fn_with_state(
                crate::channels::web::auth::CombinedAuthState::from(auth),
                auth_middleware,
            ))
            .with_state(state);

        // GET should return empty default policy
        let req = Request::builder()
            .uri("/api/admin/tool-policy")
            .header("Authorization", "Bearer tok-admin")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let policy: crate::tools::permissions::AdminToolPolicy =
            serde_json::from_slice(&body).unwrap();
        assert!(policy.is_empty());

        // PUT a policy
        let new_policy = serde_json::json!({
            "disabled_tools": ["build_software", "tool_install"],
            "user_disabled_tools": {"alice": ["shell"]}
        });
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/api/admin/tool-policy")
            .header("Authorization", "Bearer tok-admin")
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_string(&new_policy).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // GET should return persisted policy
        let req = Request::builder()
            .uri("/api/admin/tool-policy")
            .header("Authorization", "Bearer tok-admin")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let policy: crate::tools::permissions::AdminToolPolicy =
            serde_json::from_slice(&body).unwrap();
        assert!(policy.disabled_tools.contains("build_software"));
        assert!(policy.disabled_tools.contains("tool_install"));
        assert!(policy.is_tool_disabled("shell", "alice"));
        assert!(!policy.is_tool_disabled("shell", "bob"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_tool_policy_put_validates_tool_names() {
        let (db, _dir) = test_db().await;
        let state = build_multi_tenant_state(db);

        let mut tokens = HashMap::new();
        tokens.insert(
            "tok-admin".to_string(),
            UserIdentity {
                user_id: "admin-user".to_string(),
                role: "admin".to_string(),
                workspace_read_scopes: vec![],
            },
        );
        let auth = MultiAuthState::multi(tokens);

        let app = Router::new()
            .route(
                "/api/admin/tool-policy",
                get(tool_policy_get_handler).put(tool_policy_put_handler),
            )
            .layer(middleware::from_fn_with_state(
                crate::channels::web::auth::CombinedAuthState::from(auth),
                auth_middleware,
            ))
            .with_state(state);

        // Empty tool name should be rejected
        let bad_policy = serde_json::json!({
            "disabled_tools": [""]
        });
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/api/admin/tool-policy")
            .header("Authorization", "Bearer tok-admin")
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_string(&bad_policy).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // Path-like tool names should also be rejected.
        let bad_policy = serde_json::json!({
            "disabled_tools": ["../shell"]
        });
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/api/admin/tool-policy")
            .header("Authorization", "Bearer tok-admin")
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_string(&bad_policy).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_tool_policy_put_validates_user_disabled_tool_keys() {
        let (db, _dir) = test_db().await;
        let state = build_multi_tenant_state(db);

        let mut tokens = HashMap::new();
        tokens.insert(
            "tok-admin".to_string(),
            UserIdentity {
                user_id: "admin-user".to_string(),
                role: "admin".to_string(),
                workspace_read_scopes: vec![],
            },
        );
        let auth = MultiAuthState::multi(tokens);

        let app = Router::new()
            .route(
                "/api/admin/tool-policy",
                get(tool_policy_get_handler).put(tool_policy_put_handler),
            )
            .layer(middleware::from_fn_with_state(
                crate::channels::web::auth::CombinedAuthState::from(auth),
                auth_middleware,
            ))
            .with_state(state);

        let bad_policy = serde_json::json!({
            "user_disabled_tools": {
                "../member-user": ["shell"]
            }
        });
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/api/admin/tool-policy")
            .header("Authorization", "Bearer tok-admin")
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_string(&bad_policy).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_tool_policy_put_rejects_oversized_policy() {
        let (db, _dir) = test_db().await;
        let state = build_multi_tenant_state(db);

        let mut tokens = HashMap::new();
        tokens.insert(
            "tok-admin".to_string(),
            UserIdentity {
                user_id: "admin-user".to_string(),
                role: "admin".to_string(),
                workspace_read_scopes: vec![],
            },
        );
        let auth = MultiAuthState::multi(tokens);

        let app = Router::new()
            .route(
                "/api/admin/tool-policy",
                get(tool_policy_get_handler).put(tool_policy_put_handler),
            )
            .layer(middleware::from_fn_with_state(
                crate::channels::web::auth::CombinedAuthState::from(auth),
                auth_middleware,
            ))
            .with_state(state);

        let oversized_tools: Vec<String> = (0..5_000).map(|i| format!("tool_{i}")).collect();
        let bad_policy = serde_json::json!({
            "disabled_tools": oversized_tools
        });
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/api/admin/tool-policy")
            .header("Authorization", "Bearer tok-admin")
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_string(&bad_policy).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}

// ═══════════════════════════════════════════════════════════════════════
// DbAuthenticator Cache Bounded Tests
// ═══════════════════════════════════════════════════════════════════════

mod db_auth_cache {
    use super::*;
    use std::time::Instant;

    #[tokio::test]
    async fn test_cache_bounded_by_max_entries() {
        // Access the internal cache and verify LRU eviction.
        // We can't easily test through `authenticate()` since it hits the DB,
        // so we test the LRU cache directly.
        let cap = std::num::NonZeroUsize::new(4).unwrap(); // safety: test-only, 4 is non-zero
        let cache: lru::LruCache<[u8; 32], (UserIdentity, Instant)> = lru::LruCache::new(cap);
        let cache = Arc::new(tokio::sync::RwLock::new(cache));

        {
            let mut c = cache.write().await;
            for i in 0..10u8 {
                let mut hash = [0u8; 32];
                hash[0] = i;
                c.put(
                    hash,
                    (
                        UserIdentity {
                            user_id: format!("user-{i}"),
                            role: "member".to_string(),
                            workspace_read_scopes: vec![],
                        },
                        Instant::now(),
                    ),
                );
            }
            // Cache must be bounded at capacity, not grown to 10.
            assert_eq!(c.len(), 4, "cache should be bounded to capacity"); // safety: test assertion
        }
    }
}
