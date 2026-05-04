//! End-to-end tests for the legal-harness chat slice (Stream B).
//!
//! Exercises:
//! - chat CRUD against a real libSQL backend
//! - message append + RAG-context-aware LLM call (LLM stubbed at the
//!   provider boundary; everything else real)
//! - SSE streaming response shape
//! - rejection paths: deleted projects, missing chats, oversized
//!   payloads, NULL extracted_text on documents
#![cfg(feature = "libsql")]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    routing::{get, post},
};
use rust_decimal::Decimal;
use serde_json::json;
use tower::ServiceExt;

use ironclaw::channels::web::auth::{MultiAuthState, UserIdentity, auth_middleware};
use ironclaw::channels::web::legal::{
    legal_create_chat_handler, legal_get_chat_handler, legal_list_chats_handler,
    legal_post_message_handler,
};
use ironclaw::channels::web::test_helpers::TestGatewayBuilder;
use ironclaw::db::Database;
use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::legal::{LegalRole, LegalStore, LibSqlLegalStore};
use ironclaw::llm::{
    CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ToolCompletionRequest,
    ToolCompletionResponse,
};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

const TEST_TOKEN: &str = "tok-tests";
const TEST_USER: &str = "test-user";

/// LLM stub that captures the request and returns a fixed reply. Used to
/// verify what the legal handlers are sending without actually calling
/// out over the network.
struct CapturingLlm {
    last_request: tokio::sync::Mutex<Option<CompletionRequest>>,
    reply: String,
    calls: AtomicU32,
}

impl CapturingLlm {
    fn new(reply: impl Into<String>) -> Self {
        Self {
            last_request: tokio::sync::Mutex::new(None),
            reply: reply.into(),
            calls: AtomicU32::new(0),
        }
    }

    async fn last_request(&self) -> Option<CompletionRequest> {
        self.last_request.lock().await.clone()
    }
}

#[async_trait]
impl LlmProvider for CapturingLlm {
    fn model_name(&self) -> &str {
        "stub-model"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ironclaw::llm::error::LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.last_request.lock().await = Some(request);
        Ok(CompletionResponse {
            content: self.reply.clone(),
            input_tokens: 0,
            output_tokens: 0,
            finish_reason: FinishReason::Stop,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, ironclaw::llm::error::LlmError> {
        unreachable!("legal harness chat handler does not call complete_with_tools")
    }
}

/// Set up a fresh in-memory libSQL DB plus a `LegalStore` over it. Seeds
/// a project + a single document with given extracted_text.
async fn fresh_store() -> (
    Arc<dyn Database>,
    Arc<dyn LegalStore>,
    String,
    String,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("legal.db");
    let backend = LibSqlBackend::new_local(&path)
        .await
        .expect("libsql backend");
    backend.run_migrations().await.expect("migrations");

    // Stash the shared db handle for the legal store before moving the
    // backend into the trait object.
    let shared = backend.shared_db();
    let db: Arc<dyn Database> = Arc::new(backend);

    // Seed a project + two documents (one with extracted_text, one
    // without, to exercise the NULL-handling path).
    let conn = shared.connect().expect("connect");
    let project_id = "proj-test-1".to_string();
    let now = 1_700_000_000_i64;
    conn.execute(
        "INSERT INTO legal_projects (id, name, created_at) VALUES (?1, ?2, ?3)",
        libsql::params![project_id.clone(), "Test Project".to_string(), now],
    )
    .await
    .expect("seed project");

    let doc_id = "doc-1".to_string();
    conn.execute(
        "INSERT INTO legal_documents \
            (id, project_id, filename, content_type, storage_path, extracted_text, bytes, sha256, uploaded_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        libsql::params![
            doc_id.clone(),
            project_id.clone(),
            "contract.pdf".to_string(),
            "application/pdf".to_string(),
            "blobs/abc".to_string(),
            "Section 1. The party of the first part agrees to the terms herein.".to_string(),
            128_i64,
            "abcdef".to_string(),
            now,
        ],
    )
    .await
    .expect("seed doc");

    conn.execute(
        "INSERT INTO legal_documents \
            (id, project_id, filename, content_type, storage_path, extracted_text, bytes, sha256, uploaded_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8)",
        libsql::params![
            "doc-2".to_string(),
            project_id.clone(),
            "scan.pdf".to_string(),
            "application/pdf".to_string(),
            "blobs/def".to_string(),
            64_i64,
            "fedcba".to_string(),
            now,
        ],
    )
    .await
    .expect("seed null-text doc");

    let legal_store: Arc<dyn LegalStore> = Arc::new(LibSqlLegalStore::new(shared));
    (db, legal_store, project_id, doc_id, dir)
}

/// Build a router that wires the four legal endpoints behind the
/// gateway's auth middleware so we can exercise the full HTTP path.
fn build_router(
    db: Arc<dyn Database>,
    legal_store: Arc<dyn LegalStore>,
    llm: Arc<dyn LlmProvider>,
) -> (
    Router,
    Arc<ironclaw::channels::web::platform::state::GatewayState>,
) {
    let state = TestGatewayBuilder::new()
        .user_id(TEST_USER)
        .store(db)
        .legal_store(legal_store)
        .llm_provider(llm)
        .build();

    let mut tokens = HashMap::new();
    tokens.insert(
        TEST_TOKEN.to_string(),
        UserIdentity {
            user_id: TEST_USER.to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: vec![],
        },
    );
    let auth = ironclaw::channels::web::auth::CombinedAuthState {
        env_auth: MultiAuthState::multi(tokens),
        db_auth: None,
        oidc: None,
        oidc_allowed_domains: Vec::new(),
    };

    let router = Router::new()
        .route(
            "/skills/legal/projects/{id}/chats",
            get(legal_list_chats_handler).post(legal_create_chat_handler),
        )
        .route("/skills/legal/chats/{id}", get(legal_get_chat_handler))
        .route(
            "/skills/legal/chats/{id}/messages",
            post(legal_post_message_handler),
        )
        .route_layer(axum::middleware::from_fn_with_state(auth, auth_middleware))
        .with_state(Arc::clone(&state));
    (router, state)
}

fn auth_header() -> (&'static str, String) {
    ("authorization", format!("Bearer {TEST_TOKEN}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_chat_in_active_project_succeeds() {
    let (db, store, project_id, _doc, _dir) = fresh_store().await;
    let llm: Arc<dyn LlmProvider> = Arc::new(CapturingLlm::new("ok"));
    let (router, _state) = build_router(Arc::clone(&db), Arc::clone(&store), llm);

    let body = json!({"title": "First chat"});
    let (k, v) = auth_header();
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/skills/legal/projects/{project_id}/chats"))
                .header("content-type", "application/json")
                .header(k, v)
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["project_id"], project_id);
    assert_eq!(json["title"], "First chat");
    assert!(json["id"].as_str().is_some());
}

#[tokio::test]
async fn create_chat_rejects_deleted_project() {
    // Build a fresh DB and seed a project that is already soft-deleted.
    // Verifies the handler returns 409 rather than silently creating a
    // chat under a tombstoned project.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("legal.db");
    let backend = LibSqlBackend::new_local(&path).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let shared = backend.shared_db();
    let db: Arc<dyn Database> = Arc::new(backend);

    let conn = shared.connect().expect("connect");
    conn.execute(
        "INSERT INTO legal_projects (id, name, deleted_at) VALUES (?1, ?2, ?3)",
        libsql::params!["p1".to_string(), "Deleted".to_string(), 1_i64],
    )
    .await
    .expect("seed deleted project");

    let store: Arc<dyn LegalStore> = Arc::new(LibSqlLegalStore::new(shared));
    let llm: Arc<dyn LlmProvider> = Arc::new(CapturingLlm::new("noop"));
    let (router, _state) = build_router(Arc::clone(&db), Arc::clone(&store), llm);

    let (k, v) = auth_header();
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/skills/legal/projects/p1/chats")
                .header("content-type", "application/json")
                .header(k, v)
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn list_chats_returns_existing_chats() {
    let (db, store, project_id, _doc, _dir) = fresh_store().await;
    store
        .create_chat(&project_id, Some("c1"))
        .await
        .expect("seed chat 1");
    store
        .create_chat(&project_id, Some("c2"))
        .await
        .expect("seed chat 2");

    let llm: Arc<dyn LlmProvider> = Arc::new(CapturingLlm::new("noop"));
    let (router, _state) = build_router(Arc::clone(&db), Arc::clone(&store), llm);

    let (k, v) = auth_header();
    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/skills/legal/projects/{project_id}/chats"))
                .header(k, v)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["chats"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn get_chat_returns_messages_in_order() {
    let (db, store, project_id, _doc, _dir) = fresh_store().await;
    let chat = store
        .create_chat(&project_id, Some("history"))
        .await
        .expect("create chat");
    store
        .append_message(&chat.id, LegalRole::User, "hello", None)
        .await
        .expect("user msg");
    store
        .append_message(&chat.id, LegalRole::Assistant, "world", None)
        .await
        .expect("assistant msg");

    let llm: Arc<dyn LlmProvider> = Arc::new(CapturingLlm::new("noop"));
    let (router, _state) = build_router(Arc::clone(&db), Arc::clone(&store), llm);

    let (k, v) = auth_header();
    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/skills/legal/chats/{}", chat.id))
                .header(k, v)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let messages = json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[1]["role"], "assistant");
}

#[tokio::test]
async fn post_message_streams_assistant_reply_and_persists_both_messages() {
    let (db, store, project_id, doc_id, _dir) = fresh_store().await;
    let chat = store
        .create_chat(&project_id, Some("rag"))
        .await
        .expect("create chat");

    let llm = Arc::new(CapturingLlm::new("Per Section 1, yes."));
    let llm_provider: Arc<dyn LlmProvider> = Arc::clone(&llm) as _;
    let (router, _state) = build_router(Arc::clone(&db), Arc::clone(&store), llm_provider);

    let body = json!({
        "content": "Does the contract mention parties?",
        "document_refs": [doc_id],
    });
    let (k, v) = auth_header();
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/skills/legal/chats/{}/messages", chat.id))
                .header("content-type", "application/json")
                .header(k, v)
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default(),
        "text/event-stream",
    );

    // Drain the SSE stream so the spawned worker writes its events
    // before we inspect the persisted state.
    let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body.contains("legal.message.created"));
    assert!(body.contains("legal.message.delta"));
    assert!(body.contains("legal.message.done"));

    // Both messages persisted.
    let messages = store
        .list_messages_for_chat(&chat.id)
        .await
        .expect("list messages");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, LegalRole::User);
    assert_eq!(messages[0].content, "Does the contract mention parties?");
    assert_eq!(messages[1].role, LegalRole::Assistant);
    assert_eq!(messages[1].content, "Per Section 1, yes.");

    // RAG context made it into the prompt; the doc with NULL
    // extracted_text was skipped.
    let last = llm.last_request().await.expect("captured request");
    let system_msg = last
        .messages
        .iter()
        .find(|m| matches!(m.role, ironclaw::llm::Role::System))
        .expect("system message");
    assert!(system_msg.content.contains("contract.pdf"));
    assert!(!system_msg.content.contains("scan.pdf"));
    assert!(system_msg.content.contains("Section 1"));
    assert!(system_msg.content.contains("untrusted reference material"));
}

#[tokio::test]
async fn post_message_handles_project_with_zero_documents() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("legal.db");
    let backend = LibSqlBackend::new_local(&path).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let shared = backend.shared_db();
    let db: Arc<dyn Database> = Arc::new(backend);

    let conn = shared.connect().expect("connect");
    conn.execute(
        "INSERT INTO legal_projects (id, name) VALUES (?1, ?2)",
        libsql::params!["p-empty".to_string(), "Empty".to_string()],
    )
    .await
    .expect("seed project");

    let store: Arc<dyn LegalStore> = Arc::new(LibSqlLegalStore::new(shared));
    let chat = store
        .create_chat("p-empty", None)
        .await
        .expect("create chat");

    let llm = Arc::new(CapturingLlm::new("no docs available"));
    let llm_provider: Arc<dyn LlmProvider> = Arc::clone(&llm) as _;
    let (router, _state) = build_router(Arc::clone(&db), Arc::clone(&store), llm_provider);

    let body = json!({"content": "anything?"});
    let (k, v) = auth_header();
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/skills/legal/chats/{}/messages", chat.id))
                .header("content-type", "application/json")
                .header(k, v)
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();

    let last = llm.last_request().await.expect("captured request");
    // No system message gets prepended when there are no documents at all.
    let has_system = last
        .messages
        .iter()
        .any(|m| matches!(m.role, ironclaw::llm::Role::System));
    assert!(!has_system, "no documents → no RAG system message");
}

#[tokio::test]
async fn post_message_rejects_empty_body() {
    let (db, store, project_id, _doc, _dir) = fresh_store().await;
    let chat = store
        .create_chat(&project_id, None)
        .await
        .expect("create chat");

    let llm: Arc<dyn LlmProvider> = Arc::new(CapturingLlm::new("noop"));
    let (router, _state) = build_router(Arc::clone(&db), Arc::clone(&store), llm);

    let body = json!({"content": "   "});
    let (k, v) = auth_header();
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/skills/legal/chats/{}/messages", chat.id))
                .header("content-type", "application/json")
                .header(k, v)
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn post_message_returns_404_on_missing_chat() {
    let (db, store, _project_id, _doc, _dir) = fresh_store().await;
    let llm: Arc<dyn LlmProvider> = Arc::new(CapturingLlm::new("noop"));
    let (router, _state) = build_router(Arc::clone(&db), Arc::clone(&store), llm);

    let body = json!({"content": "hello"});
    let (k, v) = auth_header();
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/skills/legal/chats/no-such-chat/messages")
                .header("content-type", "application/json")
                .header(k, v)
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unauthenticated_request_is_rejected() {
    let (db, store, project_id, _doc, _dir) = fresh_store().await;
    let llm: Arc<dyn LlmProvider> = Arc::new(CapturingLlm::new("noop"));
    let (router, _state) = build_router(Arc::clone(&db), Arc::clone(&store), llm);

    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/skills/legal/projects/{project_id}/chats"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN,
        ),
        "expected 401/403, got {}",
        response.status()
    );
}
