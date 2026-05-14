//! Migration runner for the standalone Reborn host.
//!
//! Owns the two tables backing the ProductWorkflow:
//!   * `product_inbound_actions` — idempotency ledger
//!   * `product_bindings`        — external → (tenant, user, thread) mapping
//!
//! These migrations are intentionally separate from any v1 migration set —
//! this crate does not depend on the v1 `ironclaw` lib. Operators run this
//! binary against its own database (or, in dev, share a DB and accept the
//! reborn tables sit alongside v1's).
//!
//! Phase enum values mirror `ActionPhase`'s serde `rename_all = "snake_case"`:
//!   received | dispatched | settled | deduplicated_replay.

use crate::error::HostError;

#[cfg(feature = "libsql")]
const LIBSQL_SCHEMA: &str = r#"
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

CREATE INDEX IF NOT EXISTS idx_product_inbound_actions_phase
    ON product_inbound_actions(phase, received_at);

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

CREATE INDEX IF NOT EXISTS idx_product_bindings_thread
    ON product_bindings(thread_id);

CREATE INDEX IF NOT EXISTS idx_product_bindings_user
    ON product_bindings(user_id);
"#;

#[cfg(feature = "postgres")]
const POSTGRES_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS product_inbound_actions (
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

CREATE INDEX IF NOT EXISTS idx_product_inbound_actions_phase
    ON product_inbound_actions(phase, received_at);

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
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (
        adapter_id,
        installation_id,
        external_conversation_fingerprint,
        external_actor_kind,
        external_actor_id
    )
);

CREATE INDEX IF NOT EXISTS idx_product_bindings_thread
    ON product_bindings(thread_id);

CREATE INDEX IF NOT EXISTS idx_product_bindings_user
    ON product_bindings(user_id);
"#;

#[cfg(feature = "libsql")]
pub async fn run_libsql_migrations(db: &libsql::Database) -> Result<(), HostError> {
    let conn = db
        .connect()
        .map_err(|e| HostError::Storage(format!("libsql connect: {e}")))?;
    conn.execute_batch(LIBSQL_SCHEMA)
        .await
        .map_err(|e| HostError::Storage(format!("libsql migration: {e}")))?;
    Ok(())
}

#[cfg(feature = "postgres")]
pub async fn run_postgres_migrations(pool: &deadpool_postgres::Pool) -> Result<(), HostError> {
    let client = pool
        .get()
        .await
        .map_err(|e| HostError::Storage(format!("postgres pool: {e}")))?;
    client
        .batch_execute(POSTGRES_SCHEMA)
        .await
        .map_err(|e| HostError::Storage(format!("postgres migration: {e}")))?;
    Ok(())
}
