//! Postgres-backed contract tests for the product workflow storage layer.
//!
//! These run against a real Postgres instance, mirroring the pattern in
//! `crates/ironclaw_outbound/tests/outbound_state_store_contract.rs`. They
//! exist because the libSQL-only unit tests don't catch Postgres-specific
//! issues: `$1`/`$2` parameter binding, `TIMESTAMPTZ` mapping, `ON CONFLICT
//! DO NOTHING` race semantics, or schema typos.
//!
//! How to run:
//!
//!   # Skip entirely (CI without Postgres)
//!   IRONCLAW_SKIP_POSTGRES_TESTS=1 cargo test -p ironclaw_product_workflow_storage --features libsql,postgres
//!
//!   # Run against a local Postgres
//!   IRONCLAW_PRODUCT_STORAGE_POSTGRES_URL=postgres://postgres:postgres@127.0.0.1:5432/ironclaw_test \
//!     cargo test -p ironclaw_product_workflow_storage --features libsql,postgres -- --test-threads=1

#![cfg(feature = "postgres")]

use std::sync::Arc;

use chrono::Utc;
use ironclaw_host_api::{AgentId, TenantId};
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, ExternalActorRef, ExternalConversationRef,
    ProductAdapterId, ProductInboundAck, ProtocolAuthEvidence,
};
use ironclaw_product_workflow::{
    ActionFingerprintKey, ActionPhase, ConversationBindingService, IdempotencyDecision,
    IdempotencyLedger, ProductWorkflowError, ResolveBindingRequest, ResolvedBinding,
    SourceBindingKey,
};
use ironclaw_product_workflow_storage::{
    PostgresConversationBindingService, PostgresProductIdempotencyLedger,
};
use ironclaw_threads::{InMemorySessionThreadService, SessionThreadService};
use ironclaw_turns::{AcceptedMessageRef, TurnRunId};

const V28_SCHEMA: &str = r#"
DROP TABLE IF EXISTS product_inbound_actions;
DROP TABLE IF EXISTS product_bindings;

CREATE TABLE product_inbound_actions (
    action_id UUID PRIMARY KEY,
    adapter_id TEXT NOT NULL,
    installation_id TEXT NOT NULL,
    source_binding_key TEXT NOT NULL,
    external_event_id TEXT NOT NULL,
    phase TEXT NOT NULL,
    dispatch_kind_json TEXT,
    outcome_json TEXT,
    received_at TIMESTAMPTZ NOT NULL,
    settled_at TIMESTAMPTZ,
    UNIQUE (adapter_id, installation_id, source_binding_key, external_event_id)
);

CREATE TABLE product_bindings (
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
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (
        adapter_id,
        installation_id,
        external_conversation_fingerprint,
        external_actor_kind,
        external_actor_id
    )
);
"#;

async fn postgres_pool() -> Option<deadpool_postgres::Pool> {
    if std::env::var("IRONCLAW_SKIP_POSTGRES_TESTS").is_ok() {
        return None;
    }
    let url = std::env::var("IRONCLAW_PRODUCT_STORAGE_POSTGRES_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;
    let config = url.parse::<tokio_postgres::Config>().ok()?;
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .ok()
}

/// Apply the V28 schema fresh (drops + recreates) so tests are isolated.
async fn reset_schema(pool: &deadpool_postgres::Pool) {
    let client = pool.get().await.expect("postgres pool get");
    client
        .batch_execute(V28_SCHEMA)
        .await
        .expect("apply schema");
}

fn fingerprint(event_id: &str) -> ActionFingerprintKey {
    ActionFingerprintKey::new(
        ProductAdapterId::new("telegram_v2").expect("adapter"),
        AdapterInstallationId::new("install_pg_test").expect("install"),
        SourceBindingKey::new("chat:42").expect("binding key"),
        ironclaw_product_adapters::ExternalEventId::new(event_id).expect("event id"),
    )
}

fn sample_ack() -> ProductInboundAck {
    ProductInboundAck::Accepted {
        accepted_message_ref: AcceptedMessageRef::new("msg-pg-1").expect("ref"),
        submitted_run_id: TurnRunId::new(),
    }
}

#[tokio::test]
async fn postgres_ledger_round_trips_begin_settle_replay() {
    let Some(pool) = postgres_pool().await else {
        eprintln!("skipping postgres test: set IRONCLAW_PRODUCT_STORAGE_POSTGRES_URL");
        return;
    };
    reset_schema(&pool).await;

    let ledger = PostgresProductIdempotencyLedger::new(pool.clone());
    let fp = fingerprint("pg_evt_1");

    // 1. Begin a fresh action.
    let decision = ledger
        .begin_or_replay(fp.clone(), Utc::now())
        .await
        .expect("begin");
    let mut action = match decision {
        IdempotencyDecision::New(a) => a,
        other => panic!("expected New, got {other:?}"),
    };
    assert_eq!(action.phase, ActionPhase::Received);

    // 2. Second begin while in-flight must be Transient.
    let result = ledger.begin_or_replay(fp.clone(), Utc::now()).await;
    assert!(
        matches!(result, Err(ProductWorkflowError::Transient { .. })),
        "in-flight duplicate must return Transient, got: {result:?}"
    );

    // 3. Settle the action.
    action.settle(sample_ack());
    ledger.settle(action.clone()).await.expect("settle");

    // 4. Begin again on the settled row returns Replay with the original action_id.
    let replay = ledger
        .begin_or_replay(fp, Utc::now())
        .await
        .expect("replay");
    match replay {
        IdempotencyDecision::Replay(prior) => {
            assert_eq!(prior.action_id, action.action_id);
            assert_eq!(prior.phase, ActionPhase::Settled);
            assert!(
                prior.outcome.is_some(),
                "replayed action must carry outcome"
            );
        }
        other => panic!("expected Replay, got {other:?}"),
    }
}

/// Regression for Henry's PR #3590 review item #1 — stale in-flight rows
/// must be reclaimed by `begin_or_replay`, not block retries forever.
/// Mirrors `stale_inflight_row_is_reclaimed_on_next_begin` from the libSQL
/// unit-tests but drives the Postgres implementation through its public API.
#[tokio::test]
async fn postgres_ledger_reclaims_stale_inflight_row() {
    let Some(pool) = postgres_pool().await else {
        eprintln!("skipping postgres test: set IRONCLAW_PRODUCT_STORAGE_POSTGRES_URL");
        return;
    };
    reset_schema(&pool).await;

    let ledger = PostgresProductIdempotencyLedger::with_recovery_lease(
        pool.clone(),
        std::time::Duration::from_millis(1),
    );
    let fp = fingerprint("pg_stale_reclaim");

    let first = ledger
        .begin_or_replay(fp.clone(), Utc::now())
        .await
        .expect("first begin");
    let first_action_id = match first {
        IdempotencyDecision::New(a) => a.action_id,
        other => panic!("expected New, got {other:?}"),
    };

    // Hand-age the row past the 1ms lease.
    let aged = Utc::now() - chrono::Duration::seconds(10);
    let client = pool.get().await.expect("client");
    let affected = client
        .execute(
            "UPDATE product_inbound_actions SET received_at = $1 \
             WHERE adapter_id = $2 AND installation_id = $3 \
               AND source_binding_key = $4 AND external_event_id = $5",
            &[
                &aged,
                &fp.adapter_id.as_str(),
                &fp.installation_id.as_str(),
                &fp.source_binding_key.as_str(),
                &fp.external_event_id.as_str(),
            ],
        )
        .await
        .expect("age the row");
    assert_eq!(affected, 1);

    let second = ledger
        .begin_or_replay(fp, Utc::now())
        .await
        .expect("second begin");
    match second {
        IdempotencyDecision::New(reclaimed) => {
            assert_ne!(
                reclaimed.action_id, first_action_id,
                "reclaim must mint a fresh action_id"
            );
        }
        other => panic!("stale row must be reclaimed and surface as New, got {other:?}"),
    }
}

#[tokio::test]
async fn postgres_ledger_release_removes_only_inflight() {
    let Some(pool) = postgres_pool().await else {
        eprintln!("skipping postgres test: set IRONCLAW_PRODUCT_STORAGE_POSTGRES_URL");
        return;
    };
    reset_schema(&pool).await;

    let ledger = PostgresProductIdempotencyLedger::new(pool.clone());
    let fp = fingerprint("pg_release_test");

    // Begin → release → begin again must succeed as New (not Transient).
    let decision = ledger
        .begin_or_replay(fp.clone(), Utc::now())
        .await
        .expect("begin");
    let action = match decision {
        IdempotencyDecision::New(a) => a,
        other => panic!("expected New, got {other:?}"),
    };
    ledger.release(action).await.expect("release");
    let next = ledger
        .begin_or_replay(fp, Utc::now())
        .await
        .expect("next begin");
    assert!(matches!(next, IdempotencyDecision::New(_)));
}

fn binding_request(actor_id: &str, conv_id: &str) -> ResolveBindingRequest {
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Telegram-Bot-Api-Secret-Token".into(),
        },
        "telegram_install_pg_test",
    );
    let auth_claim = evidence.claim().expect("claim").clone();
    ResolveBindingRequest {
        adapter_id: ProductAdapterId::new("telegram_v2").expect("adapter"),
        installation_id: AdapterInstallationId::new("install_pg_test").expect("install"),
        external_actor_ref: ExternalActorRef::new("user", actor_id, None::<String>).expect("actor"),
        external_conversation_ref: ExternalConversationRef::new(None, conv_id, None, None)
            .expect("conv"),
        auth_claim,
    }
}

#[tokio::test]
async fn postgres_binding_creates_then_returns_same_binding() {
    let Some(pool) = postgres_pool().await else {
        eprintln!("skipping postgres test: set IRONCLAW_PRODUCT_STORAGE_POSTGRES_URL");
        return;
    };
    reset_schema(&pool).await;

    let thread_service: Arc<dyn SessionThreadService> =
        Arc::new(InMemorySessionThreadService::default());
    let svc = PostgresConversationBindingService::new(
        pool,
        thread_service,
        TenantId::new("tenant_pg").expect("tenant"),
        AgentId::new("agent_pg").expect("agent"),
    );

    let first: ResolvedBinding = svc
        .resolve_binding(binding_request("u1", "c1"))
        .await
        .expect("first resolve");
    let second: ResolvedBinding = svc
        .resolve_binding(binding_request("u1", "c1"))
        .await
        .expect("second resolve");
    assert_eq!(first.user_id.as_str(), second.user_id.as_str());
    assert_eq!(first.thread_id.as_str(), second.thread_id.as_str());
}

#[tokio::test]
async fn postgres_binding_different_actor_same_conversation_distinct_binding() {
    let Some(pool) = postgres_pool().await else {
        eprintln!("skipping postgres test: set IRONCLAW_PRODUCT_STORAGE_POSTGRES_URL");
        return;
    };
    reset_schema(&pool).await;

    let thread_service: Arc<dyn SessionThreadService> =
        Arc::new(InMemorySessionThreadService::default());
    let svc = PostgresConversationBindingService::new(
        pool,
        thread_service,
        TenantId::new("tenant_pg").expect("tenant"),
        AgentId::new("agent_pg").expect("agent"),
    );

    let alice = svc
        .resolve_binding(binding_request("alice", "shared_chat"))
        .await
        .expect("alice");
    let bob = svc
        .resolve_binding(binding_request("bob", "shared_chat"))
        .await
        .expect("bob");
    assert_ne!(alice.user_id.as_str(), bob.user_id.as_str());
    assert_ne!(alice.thread_id.as_str(), bob.thread_id.as_str());
}
