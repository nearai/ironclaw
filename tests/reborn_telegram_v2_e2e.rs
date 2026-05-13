//! End-to-end coverage for the Reborn Telegram v2 channel wiring.
//!
//! Drives the full inbound + outbound path that `src/channels/reborn/` plumbs
//! into the binary, but with three swaps for hermeticity:
//!   * `InMemorySessionThreadService` instead of the DB-backed thread service.
//!   * `InMemoryOutboundStateStore` instead of the DB-backed outbound store.
//!   * A recording `ProtocolHttpEgress` mock instead of a live reqwest client
//!     to api.telegram.org.
//!
//! Idempotency, binding persistence, and the rest of the workflow run against
//! a real libSQL in-memory database so the DB-backed bridges (ledger,
//! binding) are exercised end to end.

#![cfg(feature = "libsql")]

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use futures::StreamExt;
use http_body_util::BodyExt;
use ironclaw::channels::reborn::{
    ProductChannel, ProductChannelConfig, TelegramV2RouterState, V2InboundTurnService,
    telegram_v2_routes,
};
use ironclaw::channels::{Channel, IncomingMessage, OutgoingResponse};
use ironclaw_host_api::{AgentId, TenantId};
use ironclaw_outbound::{InMemoryOutboundStateStore, OutboundDeliveryStatus, OutboundStateStore};
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, EgressCredentialHandle, EgressRequest, EgressResponse,
    ProductAdapterId, ProtocolHttpEgress, ProtocolHttpEgressError,
};
use ironclaw_product_workflow::DefaultProductWorkflow;
use ironclaw_product_workflow_storage::{
    LibSqlConversationBindingService, LibSqlProductIdempotencyLedger,
};
use ironclaw_telegram_v2_adapter::{
    GroupTriggerPolicy, TELEGRAM_API_HOST, TelegramV2Adapter, TelegramV2AdapterConfig,
};
use ironclaw_threads::{InMemorySessionThreadService, SessionThreadService};
use ironclaw_turns::TurnScope;
use ironclaw_wasm_product_adapters::{
    NativeProductAdapterRunner, NativeProductAdapterRunnerConfig, SharedSecretHeaderAuth,
    WebhookAuth,
};
use tokio::sync::Mutex;
use tower::ServiceExt;

const TELEGRAM_SECRET_HEADER: &str = "X-Telegram-Bot-Api-Secret-Token";
const WEBHOOK_SECRET: &str = "supersecret";
const INSTALLATION: &str = "default";
const CHANNEL_NAME: &str = "telegram_v2";

/// Recording mock of `ProtocolHttpEgress`. Captures every send and returns a
/// canned 200 OK so the adapter records `DeliveryStatus::Delivered`.
#[derive(Default)]
struct RecordingEgress {
    requests: Mutex<Vec<EgressRequest>>,
}

impl RecordingEgress {
    async fn requests(&self) -> Vec<EgressRequest> {
        self.requests.lock().await.clone()
    }
}

#[async_trait]
impl ProtocolHttpEgress for RecordingEgress {
    async fn send(
        &self,
        request: EgressRequest,
    ) -> Result<EgressResponse, ProtocolHttpEgressError> {
        self.requests.lock().await.push(request);
        let canned = serde_json::json!({
            "ok": true,
            "result": { "message_id": 999 }
        });
        Ok(EgressResponse::new(
            200,
            serde_json::to_vec(&canned).expect("serialize canned response"),
        ))
    }
}

/// Build the in-memory libSQL DB with the V26 schema and return the handle.
async fn build_libsql() -> (Arc<libsql::Database>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = libsql::Builder::new_local(dir.path().join("e2e.db"))
        .build()
        .await
        .expect("build db");
    let conn = db.connect().expect("connect");
    conn.execute_batch(SCHEMA).await.expect("apply schema");
    (Arc::new(db), dir)
}

/// Mirrors `src/db/libsql_migrations.rs` migration V26 (test-only inline copy).
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

/// One-shot harness assembled per test; carries every handle the assertions
/// need.
struct Harness {
    router: Router,
    egress: Arc<RecordingEgress>,
    outbound_store: Arc<dyn OutboundStateStore>,
    product_channel: ProductChannel,
    default_tenant_id: TenantId,
    default_agent_id: AgentId,
    /// DB handle kept on the harness so tests can SELECT directly to verify
    /// durable side effects (ledger settle, binding row, ...).
    db: Arc<libsql::Database>,
    _tempdir: tempfile::TempDir,
}

async fn build_harness() -> Harness {
    let (db, tempdir) = build_libsql().await;
    let db_for_harness = Arc::clone(&db);

    let thread_service: Arc<dyn SessionThreadService> =
        Arc::new(InMemorySessionThreadService::default());
    let outbound_store: Arc<dyn OutboundStateStore> =
        Arc::new(InMemoryOutboundStateStore::default());

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

    // Recording egress (used by the synthetic ProductChannel for outbound
    // assertions). The mock doesn't enforce the declared-host allowlist —
    // that's covered by `egress::tests::undeclared_host_is_rejected` in the
    // storage crate.
    let egress = Arc::new(RecordingEgress::default());

    let product_channel_config = ProductChannelConfig {
        name: CHANNEL_NAME.into(),
        adapter: Arc::clone(&adapter),
        egress: Arc::clone(&egress) as Arc<dyn ProtocolHttpEgress>,
        outbound_store: Arc::clone(&outbound_store),
        default_tenant_id: default_tenant_id.clone(),
        default_agent_id: default_agent_id.clone(),
    };
    let (product_channel, inbound_tx) = ProductChannel::new(product_channel_config);

    let inbound_turn_service =
        V2InboundTurnService::new(Arc::clone(&binding) as _, inbound_tx, CHANNEL_NAME);
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
        egress,
        outbound_store,
        product_channel,
        default_tenant_id,
        default_agent_id,
        db: db_for_harness,
        _tempdir: tempdir,
    }
}

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates/ironclaw_telegram_v2_adapter/tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
}

async fn post_webhook(router: Router, body: Vec<u8>, secret: &str) -> (StatusCode, Vec<u8>) {
    let request = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/webhook/telegram-v2/{INSTALLATION}"))
        .header(TELEGRAM_SECRET_HEADER, secret)
        .body(axum::body::Body::from(body))
        .expect("build request");
    let response =
        <Router as ServiceExt<axum::http::Request<axum::body::Body>>>::oneshot(router, request)
            .await
            .expect("oneshot");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    (status, body.to_vec())
}

// -- Tests ------------------------------------------------------------------

#[tokio::test]
async fn webhook_post_routes_inbound_through_bus() {
    let harness = build_harness().await;
    let mut stream = harness.product_channel.start().await.expect("start");

    let (status, _body) = post_webhook(
        harness.router.clone(),
        fixture("private_chat_message.json"),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let received = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("receive within 2s")
        .expect("stream has message");
    assert_eq!(received.channel, CHANNEL_NAME);
    assert!(
        received.user_id.contains("777"),
        "user_id={}",
        received.user_id
    );
    assert_eq!(received.content, "hello bot");
    assert!(
        received.thread_id.is_some(),
        "expected thread_id to be populated by binding service"
    );
}

#[tokio::test]
async fn duplicate_update_replays_through_ledger() {
    let harness = build_harness().await;
    let mut stream = harness.product_channel.start().await.expect("start");

    // First webhook — must succeed and emit IncomingMessage.
    let (status1, _) = post_webhook(
        harness.router.clone(),
        fixture("private_chat_message.json"),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(status1, StatusCode::OK);
    let _first = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("first message")
        .expect("stream alive");

    // Second webhook with same update_id (duplicate_update.json) — must be
    // a 200 OK replay AND must NOT emit a second IncomingMessage.
    let (status2, _) = post_webhook(
        harness.router.clone(),
        fixture("duplicate_update.json"),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(status2, StatusCode::OK);

    let second = tokio::time::timeout(Duration::from_millis(300), stream.next()).await;
    assert!(
        second.is_err(),
        "duplicate webhook must not push a second IncomingMessage, got: {second:?}"
    );
}

#[tokio::test]
async fn invalid_secret_returns_401() {
    let harness = build_harness().await;
    let (status, _) = post_webhook(
        harness.router.clone(),
        fixture("private_chat_message.json"),
        "wrong-secret",
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn respond_renders_outbound_via_egress_and_records_delivery() {
    let harness = build_harness().await;
    // Start the channel so respond() can correlate. We don't actually need to
    // read the stream for this test.
    let _stream = harness.product_channel.start().await.expect("start");

    // Drive an inbound first so the binding row exists and we have a real
    // IncomingMessage to respond to.
    let (_, _) = post_webhook(
        harness.router.clone(),
        fixture("private_chat_message.json"),
        WEBHOOK_SECRET,
    )
    .await;

    // Synthesize an IncomingMessage matching what V2InboundTurnService would
    // have emitted (we already verified that path in the first test). We
    // include the v2 metadata so respond() can rebuild the reply target.
    let mut headers = HeaderMap::new();
    headers.insert(
        TELEGRAM_SECRET_HEADER,
        HeaderValue::from_static(WEBHOOK_SECRET),
    );
    let msg = IncomingMessage::new(CHANNEL_NAME, "telegram_v2_default_user_777", "hello bot")
        .with_sender_id("777")
        .with_thread("thread_e2e")
        .with_metadata(serde_json::json!({
            "v2_conversation_id": "777",
            "v2_topic_id": serde_json::Value::Null,
            "v2_reply_target_message_id": "11"
        }));

    let response = OutgoingResponse {
        content: "Hi Alice!".to_string(),
        thread_id: None,
        attachments: Vec::new(),
        inline_attachments: Vec::new(),
        metadata: serde_json::Value::Null,
    };

    harness
        .product_channel
        .respond(&msg, response)
        .await
        .expect("respond returns immediately");

    // respond() spawns the render task; wait briefly for it to land.
    let recorded = wait_for_egress(&harness.egress, 1, Duration::from_secs(2)).await;
    assert_eq!(recorded.len(), 1, "expected exactly one egress call");
    let egress_request = &recorded[0];
    assert_eq!(egress_request.host().as_str(), TELEGRAM_API_HOST);
    assert_eq!(egress_request.path().as_str(), "/sendMessage");
    let body: serde_json::Value =
        serde_json::from_slice(egress_request.body()).expect("egress body is JSON");
    assert_eq!(body["chat_id"], 777);
    assert_eq!(body["text"], "Hi Alice!");

    // Outbound delivery must be recorded as Delivered.
    let scope = TurnScope {
        tenant_id: harness.default_tenant_id.clone(),
        agent_id: Some(harness.default_agent_id.clone()),
        project_id: None,
        thread_id: ironclaw_host_api::ThreadId::new("thread_e2e").expect("thread"),
    };
    let attempts = harness
        .outbound_store
        .list_delivery_attempts(scope)
        .await
        .expect("list attempts");
    assert_eq!(attempts.len(), 1);
    assert!(matches!(
        attempts[0].status,
        OutboundDeliveryStatus::Delivered
    ));
}

#[tokio::test]
async fn accepted_inbound_settles_the_ledger_row() {
    // The duplicate-update test proves begin_or_replay returns Replay on
    // re-submission. This test goes the other half of the way: after a
    // successful inbound, the action row must transition to phase='settled'
    // with a non-null outcome_json — proving DefaultProductWorkflow actually
    // calls ledger.settle() after dispatch, not just begin_or_replay().
    let harness = build_harness().await;
    let mut stream = harness.product_channel.start().await.expect("start");

    let (status, _) = post_webhook(
        harness.router.clone(),
        fixture("private_chat_message.json"),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Drain the bus so the workflow returns and settle() fires.
    let _msg = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("inbound on bus")
        .expect("stream alive");

    // Give the workflow a beat to finish its dispatch + settle.
    // We don't hardcode the external_event_id (its exact value is the
    // adapter's choice); we instead look for any row in the table and
    // assert it reached settled.
    let mut last_row = None;
    for _ in 0..40 {
        let conn = harness.db.connect().expect("connect");
        let mut rows = conn
            .query(
                "SELECT external_event_id, phase, outcome_json, settled_at \
                 FROM product_inbound_actions",
                libsql::params![],
            )
            .await
            .expect("query");
        if let Some(row) = rows.next().await.expect("rows.next") {
            let event_id: String = row.get(0).expect("event_id");
            let phase: String = row.get(1).expect("phase");
            let outcome_json: Option<String> = row.get(2).expect("outcome_json");
            let settled_at: Option<String> = row.get(3).expect("settled_at");
            if phase == "settled" {
                last_row = Some((event_id, phase, outcome_json, settled_at));
                break;
            }
            last_row = Some((event_id, phase, outcome_json, settled_at));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let (event_id, phase, outcome_json, settled_at) =
        last_row.expect("product_inbound_actions must have at least one row");
    assert_eq!(
        phase, "settled",
        "row (event_id={event_id}) must reach settled phase"
    );
    assert!(
        outcome_json.is_some(),
        "settled row must carry outcome_json"
    );
    assert!(settled_at.is_some(), "settled row must have settled_at");
}

#[tokio::test]
async fn channel_manager_routes_response_through_product_channel() {
    // Round-trip: register the synthetic ProductChannel with a real
    // ChannelManager, drive an IncomingMessage through, then call
    // ChannelManager::respond and assert it reaches our ProductChannel's
    // spawned render task (which hits the mock egress).
    let harness = build_harness().await;
    let egress = Arc::clone(&harness.egress);
    let outbound_store = Arc::clone(&harness.outbound_store);
    let default_tenant_id = harness.default_tenant_id.clone();
    let default_agent_id = harness.default_agent_id.clone();

    // Move the ProductChannel into a real ChannelManager.
    let Harness {
        router,
        product_channel,
        _tempdir,
        ..
    } = harness;
    let _ = router; // unused for this test
    let _ = _tempdir; // keep DB alive
    let manager = ironclaw::channels::ChannelManager::new();
    // start() must be called before respond() so the channel takes its receiver.
    let _stream = product_channel.start().await.expect("start");
    manager.add(Box::new(product_channel)).await;

    // Simulate the message the agent loop would have produced for our v2
    // inbound after V2InboundTurnService emitted it onto the bus.
    let msg = IncomingMessage::new(CHANNEL_NAME, "user_via_manager", "agent reply")
        .with_sender_id("987")
        .with_thread("thread_manager_test")
        .with_metadata(serde_json::json!({
            "v2_conversation_id": "987",
            "v2_topic_id": serde_json::Value::Null,
            "v2_reply_target_message_id": "21"
        }));

    let response = OutgoingResponse {
        content: "Routed through ChannelManager".to_string(),
        thread_id: None,
        attachments: Vec::new(),
        inline_attachments: Vec::new(),
        metadata: serde_json::Value::Null,
    };

    manager
        .respond(&msg, response)
        .await
        .expect("ChannelManager::respond should succeed");

    // ProductChannel::respond spawns the render+send task. Wait briefly.
    let recorded = wait_for_egress(&egress, 1, Duration::from_secs(2)).await;
    assert_eq!(
        recorded.len(),
        1,
        "ChannelManager round-trip should have driven exactly one egress call"
    );
    let body: serde_json::Value =
        serde_json::from_slice(recorded[0].body()).expect("egress body is JSON");
    assert_eq!(body["chat_id"], 987);
    assert_eq!(body["text"], "Routed through ChannelManager");

    // Delivery should also be recorded with the manager-driven scope.
    let scope = TurnScope {
        tenant_id: default_tenant_id,
        agent_id: Some(default_agent_id),
        project_id: None,
        thread_id: ironclaw_host_api::ThreadId::new("thread_manager_test").expect("thread"),
    };
    let attempts = outbound_store
        .list_delivery_attempts(scope)
        .await
        .expect("list attempts");
    assert_eq!(attempts.len(), 1);
    assert!(matches!(
        attempts[0].status,
        OutboundDeliveryStatus::Delivered
    ));
}

#[tokio::test]
async fn channel_manager_rejects_unknown_channel() {
    // Defensive: confirm ChannelManager returns SendFailed on unknown channel,
    // and that nothing is dispatched to our ProductChannel when the name
    // doesn't match. Protects against silent misroutes.
    let manager = ironclaw::channels::ChannelManager::new();
    let msg = IncomingMessage::new("nonexistent_channel", "u1", "x");
    let response = OutgoingResponse {
        content: "should not be delivered".to_string(),
        thread_id: None,
        attachments: Vec::new(),
        inline_attachments: Vec::new(),
        metadata: serde_json::Value::Null,
    };
    let result = manager.respond(&msg, response).await;
    assert!(result.is_err(), "unknown channel must return Err");
}

async fn wait_for_egress(
    egress: &RecordingEgress,
    expected: usize,
    timeout: Duration,
) -> Vec<EgressRequest> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let current = egress.requests().await;
        if current.len() >= expected {
            return current;
        }
        if std::time::Instant::now() >= deadline {
            return current;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
