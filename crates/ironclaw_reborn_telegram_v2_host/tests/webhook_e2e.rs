//! End-to-end coverage for the Reborn Telegram v2 host.
//!
//! Wires the full stack — webhook router → NativeProductAdapterRunner →
//! parse_inbound → DefaultProductWorkflow → StubInboundTurnService → ledger
//! and binding — against an in-memory libSQL DB. The reply path is
//! intentionally not exercised because `StubInboundTurnService` does not
//! produce one (see crate docs).
//!
//! Tests cover the contract surface that matters for inbound delivery:
//!   * fail-closed shared-secret auth
//!   * idempotency replay on duplicate `update_id`
//!   * ledger settles after a successful workflow dispatch
//!   * binding row is persisted on first inbound from a new conversation

#![cfg(feature = "libsql")]

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use http_body_util::BodyExt;
use ironclaw_host_api::{AgentId, TenantId};
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, EgressCredentialHandle, ProductAdapterId,
};
use ironclaw_product_workflow::DefaultProductWorkflow;
use ironclaw_product_workflow_storage::{
    LibSqlConversationBindingService, LibSqlProductIdempotencyLedger,
};
use ironclaw_reborn_telegram_v2_host::inbound_turn::StubInboundTurnService;
use ironclaw_reborn_telegram_v2_host::router::{TelegramV2RouterState, telegram_v2_routes};
use ironclaw_telegram_v2_adapter::{
    GroupTriggerPolicy, TelegramV2Adapter, TelegramV2AdapterConfig,
};
use ironclaw_threads::{InMemorySessionThreadService, SessionThreadService};
use ironclaw_wasm_product_adapters::{
    NativeProductAdapterRunner, NativeProductAdapterRunnerConfig, SharedSecretHeaderAuth,
    WebhookAuth,
};
use tower::ServiceExt;

const INSTALLATION: &str = "e2e_install";
const WEBHOOK_SECRET: &str = "shh";
const TELEGRAM_SECRET_HEADER: &str = "X-Telegram-Bot-Api-Secret-Token";

/// Mirrors `migrations.rs::LIBSQL_SCHEMA` — kept in lockstep so tests don't
/// drift from the real migration.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS product_inbound_actions (
    action_id TEXT PRIMARY KEY,
    adapter_id TEXT NOT NULL,
    installation_id TEXT NOT NULL,
    source_binding_key TEXT NOT NULL,
    external_event_id TEXT NOT NULL,
    phase TEXT NOT NULL,
    dispatch_kind_json TEXT,
    outcome_json TEXT,
    received_at TEXT NOT NULL,
    settled_at TEXT,
    UNIQUE (adapter_id, installation_id, source_binding_key, external_event_id)
);

CREATE TABLE IF NOT EXISTS product_bindings (
    adapter_id TEXT NOT NULL,
    installation_id TEXT NOT NULL,
    external_conversation_fingerprint TEXT NOT NULL,
    external_actor_kind TEXT NOT NULL,
    external_actor_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    agent_id TEXT,
    project_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (
        adapter_id,
        installation_id,
        external_conversation_fingerprint,
        external_actor_kind,
        external_actor_id
    )
);
"#;

struct Harness {
    router: Router,
    db: Arc<libsql::Database>,
    _tempdir: tempfile::TempDir,
}

async fn build_harness() -> Harness {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let db = libsql::Builder::new_local(tempdir.path().join("e2e.db"))
        .build()
        .await
        .expect("build db");
    let conn = db.connect().expect("connect");
    conn.execute_batch(SCHEMA).await.expect("schema");
    let db = Arc::new(db);

    let thread_service: Arc<dyn SessionThreadService> =
        Arc::new(InMemorySessionThreadService::default());

    let default_tenant_id = TenantId::new("tenant_e2e").expect("tenant");
    let default_agent_id = AgentId::new("agent_e2e").expect("agent");

    let ledger = Arc::new(LibSqlProductIdempotencyLedger::new(Arc::clone(&db)));
    let binding = Arc::new(LibSqlConversationBindingService::new(
        Arc::clone(&db),
        Arc::clone(&thread_service),
        default_tenant_id.clone(),
        default_agent_id.clone(),
    ));

    let adapter_id = ProductAdapterId::new("telegram_v2").expect("adapter id");
    let installation_id = AdapterInstallationId::new(INSTALLATION).expect("install");
    let credential_handle = EgressCredentialHandle::new("telegram_bot_token").expect("handle");
    let adapter = Arc::new(TelegramV2Adapter::new(TelegramV2AdapterConfig {
        adapter_id: adapter_id.clone(),
        installation_id: installation_id.clone(),
        group_trigger_policy: GroupTriggerPolicy {
            bot_username: "ironclaw_tracer_bot".into(),
            bot_user_id: 0,
            recognized_commands: vec!["start".into()],
        },
        egress_credential_handle: credential_handle.clone(),
        auth_requirement: AuthRequirement::SharedSecretHeader {
            header_name: TELEGRAM_SECRET_HEADER.into(),
        },
        progress_push_enabled: false,
    }));

    let inbound_turn_service = StubInboundTurnService::new(Arc::clone(&binding) as _);
    let workflow = Arc::new(DefaultProductWorkflow::new(
        Arc::new(inbound_turn_service),
        Arc::clone(&ledger) as _,
    ));

    let auth = WebhookAuth::SharedSecretHeader(SharedSecretHeaderAuth {
        header_name: TELEGRAM_SECRET_HEADER.into(),
        expected_secret: WEBHOOK_SECRET.into(),
        subject: format!("telegram_v2:{INSTALLATION}"),
    });
    let runner = Arc::new(NativeProductAdapterRunner::with_config(
        Arc::clone(&adapter) as _,
        workflow,
        auth,
        NativeProductAdapterRunnerConfig::new(
            Duration::from_secs(5),
            NonZeroUsize::new(8).expect("> 0"),
        ),
    ));
    let mut runners = std::collections::HashMap::new();
    runners.insert(INSTALLATION.to_string(), runner);
    let state = TelegramV2RouterState {
        runners: Arc::new(runners),
    };
    let router = telegram_v2_routes(state);

    Harness {
        router,
        db,
        _tempdir: tempdir,
    }
}

/// Load the adapter crate's `private_chat_message.json` fixture and rewrite
/// the `update_id` so callers can vary it per test (idempotency replay needs
/// the same id twice; the other tests want unique ids).
fn telegram_update_payload(update_id: u64) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ironclaw_telegram_v2_adapter/tests/fixtures/private_chat_message.json");
    let raw =
        std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));
    let mut value: serde_json::Value = serde_json::from_slice(&raw).expect("parse fixture");
    if let Some(map) = value.as_object_mut() {
        map.insert("update_id".to_string(), serde_json::Value::from(update_id));
    }
    serde_json::to_vec(&value).expect("re-serialize")
}

async fn post_webhook(router: Router, body: Vec<u8>, secret: &str) -> (StatusCode, Vec<u8>) {
    let request = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/webhook/telegram-v2/{INSTALLATION}"))
        .header(TELEGRAM_SECRET_HEADER, secret)
        .body(axum::body::Body::from(body))
        .expect("request");
    let response = router.oneshot(request).await.expect("oneshot");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes()
        .to_vec();
    (status, body)
}

#[tokio::test]
async fn webhook_post_drives_workflow_to_settled() {
    let h = build_harness().await;
    let (status, _body) =
        post_webhook(h.router.clone(), telegram_update_payload(1), WEBHOOK_SECRET).await;
    assert_eq!(status, StatusCode::OK);

    // Ledger row reaches `settled`. The adapter stores
    // external_event_id as `tg-{installation}-{update_id}` so we query
    // by the rendered shape.
    let conn = h.db.connect().expect("connect");
    let mut rows = conn
        .query(
            "SELECT phase FROM product_inbound_actions WHERE external_event_id = ?1",
            ::libsql::params![format!("tg-{INSTALLATION}-1")],
        )
        .await
        .expect("query");
    let row = rows.next().await.expect("rows").expect("ledger row");
    let phase: String = row.get(0).expect("phase");
    assert_eq!(phase, "settled", "ledger phase must reach `settled`");

    // Binding row exists for the inbound conversation.
    let mut rows = conn
        .query(
            "SELECT user_id, thread_id FROM product_bindings WHERE adapter_id = ?1",
            ::libsql::params!["telegram_v2"],
        )
        .await
        .expect("query");
    let row = rows
        .next()
        .await
        .expect("rows")
        .expect("binding row must exist after first inbound");
    let user_id: String = row.get(0).expect("user_id");
    let thread_id: String = row.get(1).expect("thread_id");
    assert!(!user_id.is_empty());
    assert!(!thread_id.is_empty());
}

#[tokio::test]
async fn duplicate_update_replays_through_ledger() {
    let h = build_harness().await;
    let body = telegram_update_payload(2);
    let (s1, _) = post_webhook(h.router.clone(), body.clone(), WEBHOOK_SECRET).await;
    let (s2, _) = post_webhook(h.router.clone(), body, WEBHOOK_SECRET).await;
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(s2, StatusCode::OK, "duplicate should ack 200, not error");

    // Still exactly one ledger row.
    let conn = h.db.connect().expect("connect");
    let mut rows = conn
        .query(
            "SELECT COUNT(*) FROM product_inbound_actions WHERE external_event_id = ?1",
            ::libsql::params![format!("tg-{INSTALLATION}-2")],
        )
        .await
        .expect("query");
    let row = rows.next().await.expect("rows").expect("count row");
    let count: i64 = row.get(0).expect("count");
    assert_eq!(count, 1, "idempotency must not double-insert");
}

#[tokio::test]
async fn invalid_secret_returns_401() {
    let h = build_harness().await;
    let (status, _) = post_webhook(h.router, telegram_update_payload(3), "WRONG").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn missing_secret_returns_401() {
    let h = build_harness().await;
    let request = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/webhook/telegram-v2/{INSTALLATION}"))
        // No secret header at all.
        .body(axum::body::Body::from(telegram_update_payload(4)))
        .expect("request");
    let response = h.router.oneshot(request).await.expect("oneshot");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unknown_installation_returns_404() {
    let h = build_harness().await;
    let mut headers = HeaderMap::new();
    headers.insert(
        TELEGRAM_SECRET_HEADER,
        HeaderValue::from_static(WEBHOOK_SECRET),
    );
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/webhook/telegram-v2/not_a_real_install")
        .header(TELEGRAM_SECRET_HEADER, WEBHOOK_SECRET)
        .body(axum::body::Body::from(telegram_update_payload(5)))
        .expect("request");
    let response = h.router.oneshot(request).await.expect("oneshot");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
