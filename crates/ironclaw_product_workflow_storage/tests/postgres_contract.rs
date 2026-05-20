//! Postgres-backed contract tests for the product workflow storage layer.
//!
//! These run against a real Postgres instance, mirroring the pattern in
//! `crates/ironclaw_outbound/tests/outbound_state_store_contract.rs`. They
//! exist because the libSQL-only unit tests don't catch Postgres-specific
//! issues: `$1`/`$2` parameter binding, `TIMESTAMPTZ` mapping, `ON CONFLICT
//! DO NOTHING` race semantics, or schema typos.
//!
//! Scope: ledger only. Conversation binding moved to the shared
//! `ProductConversationBindingService` (PR #3727) backed by
//! `ironclaw_conversations` over the unified-FS dispatch fabric; its
//! storage contract is exercised by
//! `crates/ironclaw_conversations/tests/filesystem_store_contract.rs`.
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

use chrono::Utc;
use ironclaw_product_adapters::{AdapterInstallationId, ProductAdapterId, ProductInboundAck};
use ironclaw_product_workflow::{
    ActionFingerprintKey, ActionPhase, IdempotencyDecision, IdempotencyLedger, SourceBindingKey,
};
use ironclaw_product_workflow_storage::PostgresProductIdempotencyLedger;
use ironclaw_turns::{AcceptedMessageRef, TurnRunId};

const LEDGER_SCHEMA: &str = r#"
DROP TABLE IF EXISTS product_inbound_actions;

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

/// Apply the ledger schema fresh (drops + recreates) so tests are isolated.
async fn reset_schema(pool: &deadpool_postgres::Pool) {
    let client = pool.get().await.expect("postgres pool get");
    client
        .batch_execute(LEDGER_SCHEMA)
        .await
        .expect("apply schema");
}

fn fingerprint(event_id: &str) -> ActionFingerprintKey {
    ActionFingerprintKey::new(
        ProductAdapterId::new("telegram_v2").expect("adapter"),
        AdapterInstallationId::new("install_pg_test").expect("install"),
        ironclaw_product_adapters::ExternalActorRef::new("user", "42", None::<String>)
            .expect("actor ref"),
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

    let decision = ledger
        .begin_or_replay(fp.clone(), Utc::now())
        .await
        .expect("begin");
    let mut action = match decision {
        IdempotencyDecision::New(a) => a,
        other => panic!("expected New on first begin, got {other:?}"),
    };

    action.settle(sample_ack());
    ledger.settle(action).await.expect("settle");

    // Replay must surface the prior settled action with its outcome.
    let replay = ledger
        .begin_or_replay(fp, Utc::now())
        .await
        .expect("replay");
    match replay {
        IdempotencyDecision::Replay(action) => {
            assert_eq!(action.phase, ActionPhase::Settled);
            assert!(action.outcome.is_some(), "settled action must have outcome");
        }
        other => panic!("expected Replay on second begin, got {other:?}"),
    }
}

#[tokio::test]
async fn postgres_ledger_release_allows_retry_as_new() {
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
