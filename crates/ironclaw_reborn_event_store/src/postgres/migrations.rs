//! PostgreSQL DDL migrations for Reborn durable stores.
//!
//! Migration-script convention: spec §8.5.
//!
//! Version numbers are independent from the V1 PostgreSQL schema in
//! `src/db/postgres.rs`. This crate owns its own `_reborn_migrations`
//! tracking table.
//!
//! **Parallel-branch safety:** only version 1 (`subagent_gate_resolution`) is
//! added by WU-C2. Versions 2, 3, 4 are reserved for parallel workstreams.

/// Tracking table DDL — created once on first use.
pub const MIGRATIONS_TABLE_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS _reborn_migrations (
    version     BIGINT      NOT NULL PRIMARY KEY,
    name        TEXT        NOT NULL,
    applied_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
)"#;

/// Incremental migrations. Entries are `(version, name, ddl)`.
pub const INCREMENTAL_MIGRATIONS: &[(i64, &str, &str)] = &[
    (
        1,
        "subagent_gate_resolution",
        // ── subagent_gate_awaited_children ────────────────────────────────
        // PostgreSQL dialect: BOOLEAN, BIGINT, TIMESTAMPTZ, JSONB.
        // NULL agent_id uses COALESCE in expression indexes for uniqueness.
        r#"
CREATE TABLE IF NOT EXISTS subagent_gate_awaited_children (
    tenant_id                               TEXT        NOT NULL,
    user_id                                 TEXT        NOT NULL,
    agent_id                                TEXT,
    gate_ref                                TEXT        NOT NULL,
    parent_run_id                           TEXT        NOT NULL,
    tree_root_run_id                        TEXT        NOT NULL,
    child_run_id                            TEXT        NOT NULL,
    child_thread_id                         TEXT        NOT NULL,
    child_scope_json                        JSONB       NOT NULL,
    parent_run_context_json                 JSONB       NOT NULL,
    source_binding_ref                      TEXT        NOT NULL,
    reply_target_binding_ref                TEXT        NOT NULL,
    subagent_kind                           TEXT        NOT NULL,
    spawn_capability_id                     TEXT        NOT NULL,
    result_ref                              TEXT        NOT NULL,
    spawn_mode                              TEXT        NOT NULL,
    counter_bucket                          SMALLINT    NOT NULL,
    terminal_status                         TEXT,
    terminal_event_json                     JSONB,
    terminal_result_written                 BOOLEAN     NOT NULL DEFAULT FALSE,
    terminal_byte_len                       BIGINT      NOT NULL DEFAULT 0,
    descendant_reservation_release_claimed  BOOLEAN     NOT NULL DEFAULT FALSE,
    descendant_reservation_released         BOOLEAN     NOT NULL DEFAULT FALSE,
    delivery_claimed                        BOOLEAN     NOT NULL DEFAULT FALSE,
    delivered_to_parent                     BOOLEAN     NOT NULL DEFAULT FALSE,
    created_at                              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    settled_at                              TIMESTAMPTZ,
    PRIMARY KEY (gate_ref, child_run_id)
);

CREATE INDEX IF NOT EXISTS idx_sgac_tenant_user_agent
    ON subagent_gate_awaited_children (tenant_id, user_id, agent_id);
CREATE INDEX IF NOT EXISTS idx_sgac_child_run_id
    ON subagent_gate_awaited_children (child_run_id);
CREATE INDEX IF NOT EXISTS idx_sgac_parent_run_id
    ON subagent_gate_awaited_children (parent_run_id);
CREATE INDEX IF NOT EXISTS idx_sgac_undelivered_terminal
    ON subagent_gate_awaited_children (tenant_id, user_id, agent_id, delivered_to_parent, terminal_status)
    WHERE delivered_to_parent = FALSE;

-- ── subagent_gate_child_index ────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS subagent_gate_child_index (
    tenant_id    TEXT NOT NULL,
    user_id      TEXT NOT NULL,
    agent_id     TEXT,
    child_run_id TEXT NOT NULL,
    gate_ref     TEXT NOT NULL,
    PRIMARY KEY (child_run_id, gate_ref)
);
CREATE INDEX IF NOT EXISTS idx_sgci_scope
    ON subagent_gate_child_index (tenant_id, user_id, agent_id, child_run_id);

-- ── subagent_gate_deliverable_queue ─────────────────────────────────────

CREATE TABLE IF NOT EXISTS subagent_gate_deliverable_queue (
    tenant_id    TEXT        NOT NULL,
    user_id      TEXT        NOT NULL,
    agent_id     TEXT,
    child_run_id TEXT        NOT NULL,
    gate_ref     TEXT        NOT NULL,
    queued_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (child_run_id, gate_ref)
);
CREATE INDEX IF NOT EXISTS idx_sgdq_scope
    ON subagent_gate_deliverable_queue (tenant_id, user_id, agent_id, child_run_id);

-- ── subagent_gate_capacity_counter ──────────────────────────────────────
-- No declared PK due to nullable agent_id — use COALESCE expression index.
-- Conflict target for INSERT must name the expression index explicitly:
--   ON CONFLICT (tenant_id, user_id, COALESCE(agent_id, '__non_agent__'), bucket)

CREATE TABLE IF NOT EXISTS subagent_gate_capacity_counter (
    tenant_id    TEXT     NOT NULL,
    user_id      TEXT     NOT NULL,
    agent_id     TEXT,
    bucket       SMALLINT NOT NULL,
    undelivered  INTEGER  NOT NULL DEFAULT 0
        CHECK (undelivered >= 0)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_sgcc_pk
    ON subagent_gate_capacity_counter
       (tenant_id, user_id, COALESCE(agent_id, '__non_agent__'), bucket);
CREATE INDEX IF NOT EXISTS idx_sgcc_scope
    ON subagent_gate_capacity_counter (tenant_id, user_id, agent_id);

-- ── subagent_gate_settlement_log ────────────────────────────────────────

CREATE TABLE IF NOT EXISTS subagent_gate_settlement_log (
    id                BIGSERIAL   NOT NULL PRIMARY KEY,
    tenant_id         TEXT        NOT NULL,
    user_id           TEXT        NOT NULL,
    agent_id          TEXT,
    gate_ref          TEXT        NOT NULL,
    child_run_id      TEXT        NOT NULL,
    result_ref        TEXT        NOT NULL,
    parent_run_id     TEXT        NOT NULL,
    terminal_status   TEXT        NOT NULL,
    terminal_kind     TEXT        NOT NULL,
    event_cursor      BIGINT      NOT NULL,
    terminal_byte_len BIGINT      NOT NULL DEFAULT 0,
    sanitized_reason  TEXT,
    owner_user_id     TEXT,
    settled_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_sgsl_tenant_user_agent
    ON subagent_gate_settlement_log (tenant_id, user_id, agent_id);
CREATE INDEX IF NOT EXISTS idx_sgsl_parent_run_id
    ON subagent_gate_settlement_log (parent_run_id);
CREATE INDEX IF NOT EXISTS idx_sgsl_child_run_id
    ON subagent_gate_settlement_log (child_run_id)
"#,
    ),
    // Version 2: subagent_capability_result      (WU-C3 — parallel branch)
    // Version 3: subagent_settlement_event_log   (WU-C3 — parallel branch)
    // Version 4: subagent_idempotency_ledger     (WU-C3 — parallel branch)
];
