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
        channel_manager: None,
        log_broadcaster: None,
        log_level_handle: None,
        extension_manager: None,
        tool_registry: None,
        store,
        job_manager: None,
        prompt_queue,
        owner_id: "test".to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        server_handle: tokio::sync::RwLock::new(None),
        server_started: std::sync::atomic::AtomicBool::new(false),
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
        standby_control: None,
        runtime_overrides: crate::channels::web::server::GatewayRuntimeOverrides::default(),
        channel_reconnect_notify: None,
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
            false,
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
            false,
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
            false,
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
            false,
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
            false,
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

    #[tokio::test]
    async fn test_workspace_pool_preserves_owner_scope_for_agent_identity_writes() {
        let (db, _dir) = test_db().await;
        let pool = WorkspacePool::new_with_owner(
            db,
            None,
            EmbeddingCacheConfig::default(),
            WorkspaceSearchConfig::default(),
            WorkspaceConfig::default(),
            false,
            Some("owner".to_string()),
        );
        let guest_identity = UserIdentity {
            user_id: "dingtalk-user".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: vec![],
        };
        let owner_identity = UserIdentity {
            user_id: "owner".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: vec![],
        };

        let guest_ws = pool.get_or_create(&guest_identity).await;
        guest_ws
            .write(
                crate::workspace::paths::IDENTITY,
                "# Identity\n\nName: Owner persona",
            )
            .await
            .expect("write owner-scoped identity");

        assert!(
            guest_ws
                .read(crate::workspace::paths::IDENTITY)
                .await
                .is_err(),
            "guest scope must not materialize its own IDENTITY.md copy"
        );

        let owner_ws = pool.get_or_create(&owner_identity).await;
        let owner_doc = owner_ws
            .read(crate::workspace::paths::IDENTITY)
            .await
            .expect("read owner identity");
        assert_eq!(owner_doc.content, "# Identity\n\nName: Owner persona");

        let scoped_doc = guest_ws
            .read_primary(crate::workspace::paths::IDENTITY)
            .await
            .expect("guest should resolve owner identity via primary read");
        assert_eq!(scoped_doc.content, owner_doc.content);
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
        usage_summary_handler, users_activate_handler, users_detail_handler, users_list_handler,
        users_suspend_handler, users_update_handler,
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
            .route("/api/admin/usage/summary", get(usage_summary_handler))
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
        assert_forbidden_for_member(&app, Method::GET, "/api/admin/usage/summary").await;
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

        let req = Request::builder()
            .uri("/api/admin/usage/summary")
            .header("Authorization", "Bearer tok-admin")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_ne!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "admin should not get 403 for usage summary"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Admin API Contract Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(feature = "libsql")]
mod admin_api_contracts {
    use super::*;
    use crate::channels::web::handlers::users::{
        usage_stats_handler, usage_summary_handler, users_create_handler, users_detail_handler,
        users_list_handler, users_update_handler,
    };
    use crate::channels::web::types::{
        AdminUsageStatsResponse, AdminUsageSummaryResponse, AdminUserCreateResponse,
        AdminUserDetailResponse, AdminUserListResponse,
    };
    use serde::de::DeserializeOwned;

    fn admin_router(state: Arc<GatewayState>, auth: MultiAuthState) -> Router {
        Router::new()
            .route(
                "/api/admin/users",
                get(users_list_handler).post(users_create_handler),
            )
            .route(
                "/api/admin/users/{id}",
                get(users_detail_handler).patch(users_update_handler),
            )
            .route("/api/admin/usage", get(usage_stats_handler))
            .route("/api/admin/usage/summary", get(usage_summary_handler))
            .layer(middleware::from_fn_with_state(
                crate::channels::web::auth::CombinedAuthState::from(auth),
                auth_middleware,
            ))
            .with_state(state)
    }

    fn test_user(
        id: &str,
        display_name: &str,
        email: Option<&str>,
        status: &str,
        role: &str,
        metadata: serde_json::Value,
    ) -> crate::db::UserRecord {
        let now = chrono::Utc::now();
        crate::db::UserRecord {
            id: id.to_string(),
            email: email.map(str::to_string),
            display_name: display_name.to_string(),
            status: status.to_string(),
            role: role.to_string(),
            created_at: now,
            updated_at: now,
            last_login_at: Some(now),
            created_by: None,
            metadata,
        }
    }

    async fn parse_json<T: DeserializeOwned>(resp: axum::response::Response) -> T {
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn assert_rfc3339(ts: &str) {
        chrono::DateTime::parse_from_rfc3339(ts).unwrap();
    }

    #[tokio::test]
    async fn test_admin_create_user_response_contract() {
        let (db, _dir) = test_db().await;
        db.create_user(&test_user(
            "alice",
            "Alice",
            Some("alice@example.com"),
            "active",
            "admin",
            serde_json::json!({}),
        ))
        .await
        .unwrap();
        let state = build_state(Some(db), None);
        let app = admin_router(state, two_user_auth());

        let req = Request::builder()
            .method(Method::POST)
            .uri("/api/admin/users")
            .header("Authorization", "Bearer tok-alice")
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::to_string(
                    &serde_json::json!({"display_name":"Carol","email":"carol@example.com","role":"member"}),
                )
                .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body: AdminUserCreateResponse = parse_json(resp).await;

        assert_eq!(body.display_name, "Carol");
        assert_eq!(body.email.as_deref(), Some("carol@example.com"));
        assert_eq!(body.status, "active");
        assert_eq!(body.role, "member");
        assert_eq!(body.created_by.as_deref(), Some("alice"));
        assert_eq!(body.token.len(), 64);
    }

    #[tokio::test]
    async fn test_admin_user_list_response_contract() {
        let (db, _dir) = test_db().await;
        db.create_user(&test_user(
            "carol",
            "Carol",
            Some("carol@example.com"),
            "active",
            "member",
            serde_json::json!({"team":"ops"}),
        ))
        .await
        .unwrap();

        let state = build_state(Some(db), None);
        let app = admin_router(state, two_user_auth());

        let req = Request::builder()
            .uri("/api/admin/users")
            .header("Authorization", "Bearer tok-alice")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body: AdminUserListResponse = parse_json(resp).await;

        let user = body.users.iter().find(|u| u.id == "carol").unwrap();
        assert_eq!(user.display_name, "Carol");
        assert_eq!(user.email.as_deref(), Some("carol@example.com"));
        assert_eq!(user.job_count, 0);
        assert_eq!(user.total_cost, "0");
        assert_rfc3339(&user.created_at);
        assert_rfc3339(&user.updated_at);
        assert_rfc3339(user.last_login_at.as_deref().unwrap());
        assert!(user.last_active_at.is_some());
        assert_rfc3339(user.last_active_at.as_deref().unwrap());
    }

    #[tokio::test]
    async fn test_admin_user_detail_response_contract() {
        let (db, _dir) = test_db().await;
        db.create_user(&test_user(
            "carol",
            "Carol",
            Some("carol@example.com"),
            "active",
            "member",
            serde_json::json!({"team":"ops"}),
        ))
        .await
        .unwrap();

        let state = build_state(Some(db), None);
        let app = admin_router(state, two_user_auth());

        let req = Request::builder()
            .uri("/api/admin/users/carol")
            .header("Authorization", "Bearer tok-alice")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body: AdminUserDetailResponse = parse_json(resp).await;

        assert_eq!(body.id, "carol");
        assert_eq!(body.display_name, "Carol");
        assert_eq!(body.job_count, 0);
        assert_eq!(body.total_cost, "0");
        let metadata = body
            .metadata
            .as_ref()
            .expect("metadata populated on detail");
        assert_eq!(metadata["team"], "ops");
        assert_rfc3339(&body.created_at);
        assert_rfc3339(&body.updated_at);
        assert_rfc3339(body.last_login_at.as_deref().unwrap());
        assert!(body.last_active_at.is_some());
        assert_rfc3339(body.last_active_at.as_deref().unwrap());
    }

    #[tokio::test]
    async fn test_admin_usage_stats_response_contract() {
        let (db, _dir) = test_db().await;
        db.create_user(&test_user(
            "carol",
            "Carol",
            Some("carol@example.com"),
            "active",
            "member",
            serde_json::json!({}),
        ))
        .await
        .unwrap();

        let state = build_state(Some(db), None);
        let app = admin_router(state, two_user_auth());

        let req = Request::builder()
            .uri("/api/admin/usage?period=month")
            .header("Authorization", "Bearer tok-alice")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body: AdminUsageStatsResponse = parse_json(resp).await;

        assert_eq!(body.period, "month");
        assert_rfc3339(&body.since);
        assert!(body.usage.is_empty());
    }

    #[tokio::test]
    async fn test_admin_usage_summary_response_contract() {
        let (db, _dir) = test_db().await;
        db.create_user(&test_user(
            "alice",
            "Alice",
            Some("alice@example.com"),
            "active",
            "admin",
            serde_json::json!({}),
        ))
        .await
        .unwrap();
        db.create_user(&test_user(
            "bob",
            "Bob",
            Some("bob@example.com"),
            "suspended",
            "member",
            serde_json::json!({}),
        ))
        .await
        .unwrap();

        let state = build_state(Some(db), None);
        let app = admin_router(state, two_user_auth());

        let req = Request::builder()
            .uri("/api/admin/usage/summary")
            .header("Authorization", "Bearer tok-alice")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body: AdminUsageSummaryResponse = parse_json(resp).await;

        assert_eq!(body.users.total, 2);
        assert_eq!(body.users.active, 1);
        assert_eq!(body.users.suspended, 1);
        assert_eq!(body.users.admins, 1);
        assert_eq!(body.jobs.total, 0);
        assert_eq!(body.usage_30d.llm_calls, 0);
        assert_eq!(body.usage_30d.total_cost, "0");
    }

    #[tokio::test]
    async fn test_admin_cannot_demote_last_active_admin() {
        let (db, _dir) = test_db().await;
        db.create_user(&test_user(
            "alice",
            "Alice",
            Some("alice@example.com"),
            "active",
            "admin",
            serde_json::json!({}),
        ))
        .await
        .unwrap();

        let state = build_state(Some(db.clone()), None);
        let app = admin_router(state, two_user_auth());

        let req = Request::builder()
            .method(Method::PATCH)
            .uri("/api/admin/users/alice")
            .header("Authorization", "Bearer tok-alice")
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({"role":"member"})).unwrap(),
            ))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);

        let user = db.get_user("alice").await.unwrap().unwrap();
        assert_eq!(user.role, "admin");
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
            false,
        );
        Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: Arc::new(SseManager::new()),
            workspace: None,
            workspace_pool: Some(Arc::new(pool)),
            session_manager: None,
            channel_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store: Some(db),
            job_manager: None,
            prompt_queue: None,
            owner_id: "test".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            server_handle: tokio::sync::RwLock::new(None),
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
            standby_control: None,
            server_started: std::sync::atomic::AtomicBool::new(false),
            runtime_overrides: crate::channels::web::server::GatewayRuntimeOverrides::default(),
            channel_reconnect_notify: None,
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
