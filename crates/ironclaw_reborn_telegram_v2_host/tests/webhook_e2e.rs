//! End-to-end coverage for the Reborn Telegram v2 host.
//!
//! Wires the full stack — webhook router → NativeProductAdapterRunner →
//! parse_inbound → DefaultProductWorkflow → StubInboundTurnService → ledger
//! and binding — against an in-memory libSQL DB. The reply path is
//! intentionally not exercised because `StubInboundTurnService` does not
//! produce one (see crate docs).
//!
//! Binding goes through the **shared** `ProductConversationBindingService`
//! (PR #3727), which fails closed on unpaired actors. The harness installs
//! one trusted pairing for the fixture's `from.id = 777` Telegram user, then
//! exercises:
//!
//!   * fail-closed shared-secret auth
//!   * unknown installation 404s
//!   * webhook for a paired user settles in the ledger
//!   * duplicate `update_id` replays through the ledger without double-insert
//!   * webhook for an unpaired user is rejected before the ledger settles
//!     (the security invariant introduced by PR #3727)
//!   * binding persistence survives a host restart against the same DB

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
use ironclaw_reborn_telegram_v2_host::composition::{
    BackendHandles, RebornProductRuntime, RebornProductRuntimeConfig, build_reborn_product_runtime,
};
use ironclaw_reborn_telegram_v2_host::config::TelegramPairing;
use ironclaw_reborn_telegram_v2_host::inbound_turn::StubInboundTurnService;
use ironclaw_reborn_telegram_v2_host::migrations::run_libsql_migrations;
use ironclaw_reborn_telegram_v2_host::router::{TelegramV2RouterState, telegram_v2_routes};
use ironclaw_telegram_v2_adapter::{
    GroupTriggerPolicy, TelegramV2Adapter, TelegramV2AdapterConfig, telegram_declared_egress_hosts,
};
use ironclaw_wasm_product_adapters::{
    NativeProductAdapterRunner, NativeProductAdapterRunnerConfig, SharedSecretHeaderAuth,
    WebhookAuth,
};
use tower::ServiceExt;

const INSTALLATION: &str = "e2e_install";
const WEBHOOK_SECRET: &str = "shh";
const TELEGRAM_SECRET_HEADER: &str = "X-Telegram-Bot-Api-Secret-Token";
/// The Telegram user id (`from.id`) baked into
/// `private_chat_message.json`. Tests that want the inbound to resolve
/// must pre-pair this id; tests that want fail-closed behavior must NOT.
const FIXTURE_TG_USER_ID: &str = "777";
const TEST_TENANT: &str = "tenant_e2e";
const TEST_AGENT: &str = "agent_e2e";

struct Harness {
    router: Router,
    runtime: RebornProductRuntime,
    db: Arc<libsql::Database>,
    _tempdir: tempfile::TempDir,
}

/// Build a libSQL-backed host. `pairings` is the operator-supplied bootstrap
/// list — pass an empty slice to test the fail-closed unpaired path.
async fn build_harness(pairings: Vec<TelegramPairing>) -> Harness {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,ironclaw=debug")),
        )
        .with_test_writer()
        .try_init();
    let tempdir = tempfile::tempdir().expect("tempdir");
    let db_path = tempdir.path().join("e2e.db");
    let db = Arc::new(
        libsql::Builder::new_local(&db_path)
            .build()
            .await
            .expect("build db"),
    );
    run_libsql_migrations(&db).await.expect("ledger migration");

    let (router, runtime) = build_router(Arc::clone(&db), pairings).await;

    Harness {
        router,
        runtime,
        db,
        _tempdir: tempdir,
    }
}

/// Build the router + runtime for a libSQL DB. Reusable so the restart test
/// can drop the first runtime, then rebuild against the same DB file.
async fn build_router(
    db: Arc<libsql::Database>,
    pairings: Vec<TelegramPairing>,
) -> (Router, RebornProductRuntime) {
    let adapter_id = ProductAdapterId::new("telegram_v2").expect("adapter id");
    let installation_id = AdapterInstallationId::new(INSTALLATION).expect("install");
    let credential_handle = EgressCredentialHandle::new("telegram_bot_token").expect("handle");
    let default_tenant_id = TenantId::new(TEST_TENANT).expect("tenant");
    let default_agent_id = AgentId::new(TEST_AGENT).expect("agent");

    let runtime = build_reborn_product_runtime(
        BackendHandles::LibSql(db),
        RebornProductRuntimeConfig {
            default_tenant_id: default_tenant_id.clone(),
            default_agent_id: default_agent_id.clone(),
            adapter_id: adapter_id.clone(),
            installation_id: installation_id.clone(),
            telegram_bot_token: "test-bot-token".into(),
            telegram_credential_handle: credential_handle.clone(),
            telegram_declared_hosts: telegram_declared_egress_hosts(),
            pairings,
        },
    )
    .await
    .expect("build runtime");

    let adapter = Arc::new(TelegramV2Adapter::new(TelegramV2AdapterConfig {
        adapter_id,
        installation_id: installation_id.clone(),
        group_trigger_policy: GroupTriggerPolicy {
            bot_username: "ironclaw_tracer_bot".into(),
            bot_user_id: 0,
            recognized_commands: vec!["start".into()],
        },
        egress_credential_handle: credential_handle,
        auth_requirement: AuthRequirement::SharedSecretHeader {
            header_name: TELEGRAM_SECRET_HEADER.into(),
        },
        progress_push_enabled: false,
    }));

    let inbound_turn_service = StubInboundTurnService::new(Arc::clone(&runtime.binding));
    let workflow = Arc::new(DefaultProductWorkflow::new(
        Arc::new(inbound_turn_service),
        Arc::clone(&runtime.ledger),
        Arc::clone(&runtime.binding),
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
    (router, runtime)
}

/// Default test pairing: the fixture's `from.id` mapped to `user_alice`.
fn default_pairing() -> TelegramPairing {
    TelegramPairing {
        external_user_id: FIXTURE_TG_USER_ID.to_string(),
        user_id: "user_alice".to_string(),
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

/// SELECT the phase column for a given external_event_id. Returns None if no
/// ledger row exists, otherwise the rendered phase string.
async fn ledger_phase(db: &libsql::Database, external_event_id: &str) -> Option<String> {
    let conn = db.connect().expect("connect");
    let mut rows = conn
        .query(
            "SELECT phase FROM product_inbound_actions WHERE external_event_id = ?1",
            ::libsql::params![external_event_id],
        )
        .await
        .expect("query");
    rows.next()
        .await
        .expect("rows")
        .map(|row| row.get::<String>(0).expect("phase"))
}

async fn ledger_count(db: &libsql::Database, external_event_id: &str) -> i64 {
    let conn = db.connect().expect("connect");
    let mut rows = conn
        .query(
            "SELECT COUNT(*) FROM product_inbound_actions WHERE external_event_id = ?1",
            ::libsql::params![external_event_id],
        )
        .await
        .expect("query");
    let row = rows.next().await.expect("rows").expect("count row");
    row.get(0).expect("count")
}

/// Resolve the binding through the shared service, returning the canonical
/// thread_id the conversation state is bound to. Drives the same code path as
/// the production workflow, which is the source of truth for "is the binding
/// real and durable" — querying a SQL table would couple the test to the
/// removed Telegram-specific schema.
async fn resolve_binding_thread_id(
    runtime: &RebornProductRuntime,
    external_user_id: &str,
) -> ironclaw_host_api::ThreadId {
    use ironclaw_product_adapters::{
        AdapterInstallationId, ExternalActorRef, ExternalConversationRef, ExternalEventId,
        ProductAdapterId,
    };
    use ironclaw_product_workflow::{
        ConversationBindingService, ProductConversationRouteKind, ResolveBindingRequest,
    };

    // Build a synthetic claim equivalent to what the runner threads through
    // on inbound — `ProductConversationBindingService::lookup_binding` does
    // not consult the claim, so we just need a verified one to satisfy the
    // request type.
    let evidence = ironclaw_product_adapters::ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: TELEGRAM_SECRET_HEADER.into(),
        },
        "telegram_install_default",
    );
    let auth_claim = evidence.claim().expect("claim").clone();

    let request = ResolveBindingRequest {
        adapter_id: ProductAdapterId::new("telegram_v2").expect("adapter"),
        installation_id: AdapterInstallationId::new(INSTALLATION).expect("install"),
        external_actor_ref: ExternalActorRef::new(
            ironclaw_telegram_v2_adapter::TELEGRAM_USER_ACTOR_KIND,
            external_user_id,
            None::<String>,
        )
        .expect("actor"),
        external_conversation_ref: ExternalConversationRef::new(None, external_user_id, None, None)
            .expect("conv"),
        external_event_id: ExternalEventId::new(format!("lookup:{external_user_id}"))
            .expect("event id"),
        route_kind: ProductConversationRouteKind::Direct,
        auth_claim,
    };
    let resolved = runtime
        .binding
        .lookup_binding(request)
        .await
        .expect("lookup binding");
    resolved.thread_id
}

#[tokio::test]
async fn webhook_post_drives_workflow_to_settled() {
    let h = build_harness(vec![default_pairing()]).await;
    let (status, body) =
        post_webhook(h.router.clone(), telegram_update_payload(1), WEBHOOK_SECRET).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "webhook failed with body: {}",
        String::from_utf8_lossy(&body)
    );

    // Ledger row reaches `settled` for this external_event_id. The adapter
    // stores it as `tg-{installation}-{update_id}`.
    let phase = ledger_phase(&h.db, &format!("tg-{INSTALLATION}-1")).await;
    assert_eq!(
        phase.as_deref(),
        Some("settled"),
        "ledger phase must reach `settled`",
    );

    // The shared binding service reports a durable thread_id for the
    // paired Telegram user — confirms first-contact resolution wired the
    // conversation state through the `/conversations` mount.
    let thread_id = resolve_binding_thread_id(&h.runtime, FIXTURE_TG_USER_ID).await;
    assert!(
        !thread_id.as_str().is_empty(),
        "lookup_binding must return a non-empty thread_id after inbound resolves",
    );
}

#[tokio::test]
async fn duplicate_update_replays_through_ledger() {
    let h = build_harness(vec![default_pairing()]).await;
    let body = telegram_update_payload(2);
    let (s1, _) = post_webhook(h.router.clone(), body.clone(), WEBHOOK_SECRET).await;
    let (s2, _) = post_webhook(h.router.clone(), body, WEBHOOK_SECRET).await;
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(s2, StatusCode::OK, "duplicate should ack 200, not error");

    let count = ledger_count(&h.db, &format!("tg-{INSTALLATION}-2")).await;
    assert_eq!(count, 1, "idempotency must not double-insert");
}

#[tokio::test]
async fn invalid_secret_returns_401() {
    let h = build_harness(vec![default_pairing()]).await;
    let (status, _) = post_webhook(h.router, telegram_update_payload(3), "WRONG").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn missing_secret_returns_401() {
    let h = build_harness(vec![default_pairing()]).await;
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
    let h = build_harness(vec![default_pairing()]).await;
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

/// Security regression for PR #3727's `BindingRequired` invariant: inbound
/// from a Telegram user with no operator-installed pairing must NOT succeed
/// — the workflow returns `WorkflowRejected{ScopeNotFound}` and the host
/// translates that into a non-success HTTP status. The ledger row IS settled
/// (with a rejection outcome, so retries are deduplicated), but the binding
/// itself never lands and the conversation lookup fails closed.
#[tokio::test]
async fn unpaired_actor_does_not_settle() {
    // Start with NO pairings — fixture's external user `777` is unknown.
    let h = build_harness(Vec::new()).await;
    let (status, _) =
        post_webhook(h.router.clone(), telegram_update_payload(6), WEBHOOK_SECRET).await;
    // The runner translates `BindingRequired` (mapped to `ScopeNotFound`,
    // 404 at the workflow boundary) into a non-success HTTP status. The
    // exact code depends on `is_retryable()` on the runner-level error —
    // assert it's NOT a 2xx (the security invariant) without coupling to
    // the specific 4xx/5xx mapping.
    assert!(
        !status.is_success(),
        "unpaired actor must produce a non-success status, got {status}"
    );
    // The binding store must not contain a record for the unpaired actor —
    // this is the durable half of the invariant.
    use ironclaw_product_workflow::ProductWorkflowError;
    let req = unpaired_lookup_request();
    let err = h
        .runtime
        .binding
        .lookup_binding(req)
        .await
        .expect_err("lookup must fail for unpaired actor");
    assert!(
        matches!(err, ProductWorkflowError::BindingRequired { .. }),
        "expected BindingRequired, got {err:?}",
    );
}

fn unpaired_lookup_request() -> ironclaw_product_workflow::ResolveBindingRequest {
    use ironclaw_product_adapters::{
        AdapterInstallationId, ExternalActorRef, ExternalConversationRef, ExternalEventId,
        ProductAdapterId,
    };
    use ironclaw_product_workflow::{ProductConversationRouteKind, ResolveBindingRequest};

    let evidence = ironclaw_product_adapters::ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: TELEGRAM_SECRET_HEADER.into(),
        },
        "telegram_install_default",
    );
    let auth_claim = evidence.claim().expect("claim").clone();
    ResolveBindingRequest {
        adapter_id: ProductAdapterId::new("telegram_v2").expect("adapter"),
        installation_id: AdapterInstallationId::new(INSTALLATION).expect("install"),
        external_actor_ref: ExternalActorRef::new(
            ironclaw_telegram_v2_adapter::TELEGRAM_USER_ACTOR_KIND,
            FIXTURE_TG_USER_ID,
            None::<String>,
        )
        .expect("actor"),
        external_conversation_ref: ExternalConversationRef::new(
            None,
            FIXTURE_TG_USER_ID,
            None,
            None,
        )
        .expect("conv"),
        external_event_id: ExternalEventId::new("unpaired-lookup").expect("event id"),
        route_kind: ProductConversationRouteKind::Direct,
        auth_claim,
    }
}

/// Durability regression: pair, resolve once, drop the runtime, rebuild
/// against the same DB file, resolve again — must return the same
/// `thread_id`. Proves the binding persists through the unified-FS dispatch
/// fabric (PR #3679) rather than living only in process memory.
#[tokio::test]
async fn binding_survives_host_restart() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let db_path = tempdir.path().join("durable.db");

    let thread_id_first: String;
    {
        let db = Arc::new(
            libsql::Builder::new_local(&db_path)
                .build()
                .await
                .expect("build db"),
        );
        run_libsql_migrations(&db).await.expect("ledger migration");
        let (router, runtime) = build_router(Arc::clone(&db), vec![default_pairing()]).await;
        let (status, _) = post_webhook(router, telegram_update_payload(7), WEBHOOK_SECRET).await;
        assert_eq!(status, StatusCode::OK);
        thread_id_first = resolve_binding_thread_id(&runtime, FIXTURE_TG_USER_ID)
            .await
            .as_str()
            .to_string();
        // runtime + db drop at end of scope.
    }

    let db = Arc::new(
        libsql::Builder::new_local(&db_path)
            .build()
            .await
            .expect("rebuild db"),
    );
    let (_router, runtime) = build_router(db, vec![default_pairing()]).await;
    let thread_id_second = resolve_binding_thread_id(&runtime, FIXTURE_TG_USER_ID)
        .await
        .as_str()
        .to_string();
    assert_eq!(
        thread_id_first, thread_id_second,
        "thread_id must be stable across host restart (binding is durable)"
    );
}

/// Restarting the host with the same pairing must not error — the pairing
/// install path is idempotent. (`try_pair_external_actor` returns `Ok(())`
/// on duplicates.)
#[tokio::test]
async fn duplicate_pairing_install_is_idempotent() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let db_path = tempdir.path().join("rebound.db");
    let db = Arc::new(
        libsql::Builder::new_local(&db_path)
            .build()
            .await
            .expect("build db"),
    );
    run_libsql_migrations(&db).await.expect("ledger migration");

    // First build installs the pairing.
    let _ = build_router(Arc::clone(&db), vec![default_pairing()]).await;
    // Second build over the same DB sees the pairing already exists; must
    // not error.
    let _ = build_router(db, vec![default_pairing()]).await;
}
