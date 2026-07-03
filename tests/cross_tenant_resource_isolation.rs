//! Cross-tenant access regression tests for non-thread resources.
//!
//! Companion to `thread_isolation_integration.rs`: drives the running
//! gateway over real HTTP and asserts that Bob cannot reach Alice's
//! sandbox-job artefacts (events, file list, file content) or routine
//! run history by guessing/stealing an id. Each handler that takes an
//! id from the request gets its own negative test plus a positive
//! "Alice can reach her own" pin so a future refactor that flips the
//! ownership predicate fails BOTH directions.
//!
//! Path-traversal pin lives in `alice_job_file_read_rejects_dotdot`:
//! even Alice's own `?path=../../etc/passwd` request must not escape
//! the project directory. The handler relies on `canonicalize()` +
//! `starts_with(base_canonical)`; if that guard regresses, the test
//! catches it before shipping.
//!
//! Gated on `feature = "libsql"` so the suite has a real DB to seed
//! sandbox jobs and routines into.

#![cfg(feature = "libsql")]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use chrono::Utc;
use ironclaw::agent::SessionManager;
use ironclaw::agent::routine::{
    Routine, RoutineAction, RoutineGuardrails, RoutineRun, RunStatus, Trigger,
};
use ironclaw::channels::web::auth::{MultiAuthState, UserIdentity};
use ironclaw::channels::web::platform::router::start_server;
use ironclaw::channels::web::platform::state::{GatewayState, PerUserRateLimiter, RateLimiter};
use ironclaw::channels::web::sse::SseManager;
use ironclaw::channels::web::ws::WsConnectionTracker;
use ironclaw::db::Database;
use ironclaw::history::SandboxJobRecord;
use uuid::Uuid;

const ALICE_TOKEN: &str = "tok-alice-resource-isolation";
const BOB_TOKEN: &str = "tok-bob-resource-isolation";
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

/// Seed a sandbox job owned by `user_id` whose `project_dir` is a fresh
/// temp directory the test can also write files into. Returns the job
/// id and the project root so callers can place fixtures and probe the
/// path-traversal guard against a real on-disk layout.
async fn seed_sandbox_job(db: &Arc<dyn Database>, user_id: &str) -> (Uuid, tempfile::TempDir) {
    let project_dir = tempfile::tempdir().expect("project tempdir");
    let job = SandboxJobRecord {
        id: Uuid::new_v4(),
        task: format!("{user_id} task"),
        status: "running".to_string(),
        user_id: user_id.to_string(),
        project_dir: project_dir.path().to_string_lossy().into_owned(),
        success: None,
        failure_reason: None,
        created_at: Utc::now(),
        started_at: Some(Utc::now()),
        completed_at: None,
        credential_grants_json: "[]".to_string(),
        mcp_servers: None,
        max_iterations: None,
    };
    let id = job.id;
    db.save_sandbox_job(&job).await.expect("save sandbox job");
    (id, project_dir)
}

async fn seed_routine(db: &Arc<dyn Database>, user_id: &str) -> Uuid {
    let id = Uuid::new_v4();
    let routine = Routine {
        id,
        name: format!("test-routine-{id}"),
        description: format!("{user_id}'s routine"),
        user_id: user_id.to_string(),
        enabled: true,
        trigger: Trigger::Manual,
        action: RoutineAction::FullJob {
            title: "task".to_string(),
            description: "desc".to_string(),
            max_iterations: 5,
        },
        guardrails: RoutineGuardrails {
            cooldown: std::time::Duration::from_secs(0),
            max_concurrent: 1,
            dedup_window: None,
        },
        notify: Default::default(),
        last_run_at: None,
        next_fire_at: None,
        run_count: 0,
        consecutive_failures: 0,
        state: serde_json::json!({}),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db.create_routine(&routine).await.expect("create routine");
    let run = RoutineRun {
        id: Uuid::new_v4(),
        routine_id: id,
        trigger_type: "manual".to_string(),
        trigger_detail: None,
        started_at: Utc::now(),
        completed_at: None,
        status: RunStatus::Running,
        result_summary: Some(format!("{user_id}'s secret run summary")),
        tokens_used: None,
        job_id: None,
        created_at: Utc::now(),
    };
    db.create_routine_run(&run).await.expect("seed run");
    id
}

// ---------------------------------------------------------------------------
// Sandbox jobs — persisted events history (`GET /api/jobs/{id}/events`)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn bob_job_events_for_alice_job_returns_404() {
    let (addr, _state, db, _tmp) = start_server_with_db().await;
    let (alice_job_id, _proj) = seed_sandbox_job(&db, ALICE_USER_ID).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{addr}/api/jobs/{alice_job_id}/events"))
        .header("Authorization", format!("Bearer {BOB_TOKEN}"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        404,
        "Bob requesting Alice's job events must get 404 (not 403, to prevent enumeration); body {}",
        resp.text().await.unwrap_or_default()
    );
}

#[tokio::test]
async fn alice_job_events_for_own_job_succeeds() {
    let (addr, _state, db, _tmp) = start_server_with_db().await;
    let (alice_job_id, _proj) = seed_sandbox_job(&db, ALICE_USER_ID).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{addr}/api/jobs/{alice_job_id}/events"))
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200, "Alice must reach her own job events");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["job_id"], alice_job_id.to_string());
}

// ---------------------------------------------------------------------------
// Sandbox jobs — workspace file list / read
// ---------------------------------------------------------------------------

#[tokio::test]
async fn bob_job_files_list_for_alice_job_returns_404() {
    let (addr, _state, db, _tmp) = start_server_with_db().await;
    let (alice_job_id, alice_proj) = seed_sandbox_job(&db, ALICE_USER_ID).await;
    std::fs::write(alice_proj.path().join("secret.txt"), "alice's data")
        .expect("seed file in alice's project");

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{addr}/api/jobs/{alice_job_id}/files/list"))
        .header("Authorization", format!("Bearer {BOB_TOKEN}"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        404,
        "Bob listing files in Alice's job workspace must get 404; body {}",
        resp.text().await.unwrap_or_default()
    );
}

#[tokio::test]
async fn bob_job_file_read_for_alice_job_returns_404() {
    let (addr, _state, db, _tmp) = start_server_with_db().await;
    let (alice_job_id, alice_proj) = seed_sandbox_job(&db, ALICE_USER_ID).await;
    std::fs::write(alice_proj.path().join("secret.txt"), "alice's data")
        .expect("seed file in alice's project");

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "http://{addr}/api/jobs/{alice_job_id}/files/read?path=secret.txt"
        ))
        .header("Authorization", format!("Bearer {BOB_TOKEN}"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        404,
        "Bob reading a file from Alice's job workspace must get 404; body {}",
        resp.text().await.unwrap_or_default()
    );
}

#[tokio::test]
async fn alice_job_file_read_for_own_job_succeeds() {
    let (addr, _state, db, _tmp) = start_server_with_db().await;
    let (alice_job_id, alice_proj) = seed_sandbox_job(&db, ALICE_USER_ID).await;
    let payload = "hello from alice";
    std::fs::write(alice_proj.path().join("note.txt"), payload).expect("seed note");

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "http://{addr}/api/jobs/{alice_job_id}/files/read?path=note.txt"
        ))
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200, "Alice must reach her own job file");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["content"], payload);
}

/// Even the resource owner must not be able to escape the project
/// directory via `..`. The handler relies on `canonicalize()` +
/// `starts_with(base_canonical)` to enforce containment; this test
/// pins that guard so a refactor (e.g. switching to `clean()` or a
/// raw `join` without the starts_with check) regresses loudly.
///
/// The directory tree is built by hand instead of via `seed_sandbox_job`
/// so the planted file lives at exactly `<project_dir>/../outside.txt`.
/// A regression that allows `..` traversal then deterministically reads
/// the planted bytes and returns 200, rather than a 404 because the
/// probe happened to point at empty space.
#[tokio::test]
async fn alice_job_file_read_rejects_dotdot_traversal() {
    let (addr, _state, db, _outer) = start_server_with_db().await;

    let parent = tempfile::tempdir().expect("traversal-parent tempdir");
    let alice_proj_path = parent.path().join("alice_proj");
    std::fs::create_dir(&alice_proj_path).expect("create project dir");
    // Plant the file at exactly `<project_dir>/../outside.txt`. If
    // canonicalize+starts_with regresses, the handler resolves the
    // probe to this exact file and returns its content with a 200.
    let outside = parent.path().join("outside.txt");
    std::fs::write(&outside, "should not be reachable").expect("plant outside file");

    let alice_job_id = Uuid::new_v4();
    let job = SandboxJobRecord {
        id: alice_job_id,
        task: format!("{ALICE_USER_ID} task"),
        status: "running".to_string(),
        user_id: ALICE_USER_ID.to_string(),
        project_dir: alice_proj_path.to_string_lossy().into_owned(),
        success: None,
        failure_reason: None,
        created_at: Utc::now(),
        started_at: Some(Utc::now()),
        completed_at: None,
        credential_grants_json: "[]".to_string(),
        mcp_servers: None,
        max_iterations: None,
    };
    db.save_sandbox_job(&job).await.expect("save sandbox job");

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{addr}/api/jobs/{alice_job_id}/files/read"))
        .query(&[("path", "../outside.txt")])
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .send()
        .await
        .unwrap();

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    assert!(
        status == 403 || status == 404,
        "`..` traversal must be rejected; got {status}, body {body}"
    );
    assert!(
        !body.contains("should not be reachable"),
        "planted outside-project bytes leaked through `..` probe; body {body}"
    );
}

// ---------------------------------------------------------------------------
// Routines — runs list (`GET /api/routines/{id}/runs`)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn bob_routine_runs_for_alice_routine_returns_404() {
    let (addr, _state, db, _tmp) = start_server_with_db().await;
    let alice_routine_id = seed_routine(&db, ALICE_USER_ID).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "http://{addr}/api/routines/{alice_routine_id}/runs"
        ))
        .header("Authorization", format!("Bearer {BOB_TOKEN}"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        404,
        "Bob listing Alice's routine runs must get 404; body {}",
        resp.text().await.unwrap_or_default()
    );
}

#[tokio::test]
async fn alice_routine_runs_for_own_routine_succeeds() {
    let (addr, _state, db, _tmp) = start_server_with_db().await;
    let alice_routine_id = seed_routine(&db, ALICE_USER_ID).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "http://{addr}/api/routines/{alice_routine_id}/runs"
        ))
        .header("Authorization", format!("Bearer {ALICE_TOKEN}"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        200,
        "Alice must reach her own routine runs; body {}",
        resp.text().await.unwrap_or_default()
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    let runs = body["runs"].as_array().expect("runs array");
    assert!(!runs.is_empty(), "Alice's routine had a run seeded");
}
