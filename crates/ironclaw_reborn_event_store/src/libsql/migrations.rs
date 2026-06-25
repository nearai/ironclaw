//! libSQL DDL migrations for Reborn durable stores.
//!
//! Migration-script convention: spec §8.5.
//!
//! Version numbers are independent from `src/db/libsql_migrations.rs` —
//! this crate owns its own `_reborn_migrations` tracking table so there is
//! no collision with the legacy V1 schema.
//!
//! Each entry is `(version, name, ddl)`. All DDL uses `CREATE TABLE IF NOT
//! EXISTS` and `CREATE INDEX IF NOT EXISTS` for idempotency. Migrations are
//! applied in version order; only versions absent from `_reborn_migrations`
//! are executed.
//!
//! **Parallel-branch safety:** only version 1 (`subagent_gate_resolution`) is
//! added by WU-C2. Versions 2, 3, 4 are reserved for parallel workstreams.

/// Tracking table DDL — created once on first use.
pub const MIGRATIONS_TABLE_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS _reborn_migrations (
    version     INTEGER NOT NULL PRIMARY KEY,
    name        TEXT    NOT NULL,
    applied_at  TEXT    NOT NULL DEFAULT (datetime('now'))
)"#;

/// Incremental migrations. Entries are `(version, name, ddl)`.
///
/// WU-C2 adds version 1 only. Parallel workstreams add 2, 3, 4 without
/// conflict because each PR adds a single entry to this array at the
/// appropriate position.
pub const INCREMENTAL_MIGRATIONS: &[(i64, &str, &str)] = &[
    (
        1,
        "subagent_gate_resolution",
        // ── subagent_gate_awaited_children ────────────────────────────────
        // Primary record table. Mirrors GateResolutionInner.by_gate.
        // Boolean columns use INTEGER (0/1) per SQLite convention.
        r#"
CREATE TABLE IF NOT EXISTS subagent_gate_awaited_children (
    tenant_id                               TEXT    NOT NULL,
    user_id                                 TEXT    NOT NULL,
    agent_id                                TEXT,
    gate_ref                                TEXT    NOT NULL,
    parent_run_id                           TEXT    NOT NULL,
    tree_root_run_id                        TEXT    NOT NULL,
    child_run_id                            TEXT    NOT NULL,
    child_thread_id                         TEXT    NOT NULL,
    child_scope_json                        TEXT    NOT NULL,
    parent_run_context_json                 TEXT    NOT NULL,
    source_binding_ref                      TEXT    NOT NULL,
    reply_target_binding_ref                TEXT    NOT NULL,
    subagent_kind                           TEXT    NOT NULL,
    spawn_capability_id                     TEXT    NOT NULL,
    result_ref                              TEXT    NOT NULL,
    spawn_mode                              TEXT    NOT NULL,
    counter_bucket                          INTEGER NOT NULL,
    terminal_status                         TEXT,
    terminal_event_json                     TEXT,
    terminal_result_written                 INTEGER NOT NULL DEFAULT 0,
    terminal_byte_len                       INTEGER NOT NULL DEFAULT 0,
    descendant_reservation_release_claimed  INTEGER NOT NULL DEFAULT 0,
    descendant_reservation_released         INTEGER NOT NULL DEFAULT 0,
    delivery_claimed                        INTEGER NOT NULL DEFAULT 0,
    delivered_to_parent                     INTEGER NOT NULL DEFAULT 0,
    created_at                              TEXT    NOT NULL DEFAULT (datetime('now')),
    settled_at                              TEXT,
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
    WHERE delivered_to_parent = 0;

-- ── subagent_gate_child_index ────────────────────────────────────────────
-- Reverse-index table. Mirrors GateResolutionInner.gates_by_child.

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
-- Delivery queue table. Mirrors GateResolutionInner.deliverable_by_child.

CREATE TABLE IF NOT EXISTS subagent_gate_deliverable_queue (
    tenant_id    TEXT NOT NULL,
    user_id      TEXT NOT NULL,
    agent_id     TEXT,
    child_run_id TEXT NOT NULL,
    gate_ref     TEXT NOT NULL,
    queued_at    TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (child_run_id, gate_ref)
);
CREATE INDEX IF NOT EXISTS idx_sgdq_scope
    ON subagent_gate_deliverable_queue (tenant_id, user_id, agent_id, child_run_id);

-- ── subagent_gate_capacity_counter ──────────────────────────────────────
-- Bucketed capacity counter (K=CAPACITY_COUNTER_BUCKETS, default 16).
-- Replaces per-spawn SELECT COUNT(*) on the hot path.
-- libSQL uses BEGIN IMMEDIATE for serialization (no per-row locking).
-- No GREATEST() in libSQL: use MAX(undelivered - 1, 0) for floor-at-zero.
--
-- No declared PRIMARY KEY because SQLite treats NULL as distinct in composite
-- PKs: each agentless spawn would create a NEW counter row for the same
-- (tenant_id, user_id, bucket), drifting capacity upward. Instead we use a
-- COALESCE expression index (mirroring the Postgres migration) so that all
-- rows for agent_id IS NULL map to the same unique (tenant, user, '', bucket)
-- tuple. Conflict target for INSERT must use the COALESCE expression:
--   ON CONFLICT (tenant_id, user_id, COALESCE(agent_id, ''), bucket)

CREATE TABLE IF NOT EXISTS subagent_gate_capacity_counter (
    tenant_id    TEXT    NOT NULL,
    user_id      TEXT    NOT NULL,
    agent_id     TEXT,
    bucket       INTEGER NOT NULL,
    undelivered  INTEGER NOT NULL DEFAULT 0
        CHECK (undelivered >= 0)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_sgcc_pk
    ON subagent_gate_capacity_counter
       (tenant_id, user_id, COALESCE(agent_id, ''), bucket);
CREATE INDEX IF NOT EXISTS idx_sgcc_scope
    ON subagent_gate_capacity_counter (tenant_id, user_id, agent_id);

-- ── subagent_gate_settlement_log ────────────────────────────────────────
-- Append-only settlement log for SubagentRestartReconciler replay.
-- Rows are NEVER deleted — they are the replay source of truth.

CREATE TABLE IF NOT EXISTS subagent_gate_settlement_log (
    id               INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    tenant_id        TEXT    NOT NULL,
    user_id          TEXT    NOT NULL,
    agent_id         TEXT,
    gate_ref         TEXT    NOT NULL,
    child_run_id     TEXT    NOT NULL,
    result_ref       TEXT    NOT NULL,
    parent_run_id    TEXT    NOT NULL,
    terminal_status  TEXT    NOT NULL,
    terminal_kind    TEXT    NOT NULL,
    event_cursor     INTEGER NOT NULL,
    terminal_byte_len INTEGER NOT NULL DEFAULT 0,
    sanitized_reason TEXT,
    owner_user_id    TEXT,
    settled_at       TEXT    NOT NULL DEFAULT (datetime('now'))
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
