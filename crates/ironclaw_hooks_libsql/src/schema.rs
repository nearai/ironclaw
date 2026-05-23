//! libSQL / SQLite schema for the durable predicate-state backend.
//!
//! Two tables, one per predicate kind:
//!
//! ```sql
//! CREATE TABLE hooks_predicate_invocations (
//!     scope_hash   BLOB    NOT NULL,  -- blake3(hook_id ‖ tenant_id ‖ capability)
//!     event_id     TEXT    NOT NULL,  -- PredicateEventId, host-assigned, ≤64 char hex
//!     occurred_at  INTEGER NOT NULL,  -- epoch milliseconds (canonical host clock)
//!     tenant_id    TEXT    NOT NULL,  -- retained for the per-tenant LRU + reaper
//!     PRIMARY KEY (scope_hash, event_id)
//! );
//! CREATE TABLE hooks_predicate_values (
//!     scope_hash   BLOB    NOT NULL,  -- blake3(hook_id ‖ tenant_id ‖ capability ‖ field)
//!     event_id     TEXT    NOT NULL,
//!     occurred_at  INTEGER NOT NULL,
//!     value        TEXT    NOT NULL,  -- rust_decimal, exact string (NUMERIC convention)
//!     tenant_id    TEXT    NOT NULL,
//!     PRIMARY KEY (scope_hash, event_id)
//! );
//! ```
//!
//! ## id column type (Codex #3635 finding)
//!
//! `event_id` is **TEXT**, not a `uuid`-typed column: a synthesized
//! `PredicateEventId` is a 64-char blake3 hex digest which does not fit a
//! 36-char `uuid`. SQLite has no native `uuid` type regardless, but the TEXT
//! choice is called out here so the libSQL and Postgres schemas stay
//! semantically aligned (the Postgres sibling, PR 2/4, must also avoid a
//! `uuid` column for the same reason).
//!
//! ## Replay-dedup
//!
//! The `PRIMARY KEY (scope_hash, event_id)` is the durable equivalent of the
//! in-memory `dedup_ids` set, scoped to the counter key (NOT global). Records
//! use `INSERT … ON CONFLICT (scope_hash, event_id) DO NOTHING` so a replayed
//! `event_id` against the same key is a no-op, while the same `event_id`
//! against a different key (or the other table) still records — matching the
//! `PredicateStateBackend` replay-refusal contract. Because the invocation and
//! value tables are separate, the same `event_id` in both does not collide
//! (the `event_id_dedup_isolated_across_maps` contract).

pub(crate) const INVOCATIONS_TABLE: &str = "hooks_predicate_invocations";
pub(crate) const VALUES_TABLE: &str = "hooks_predicate_values";

pub(crate) const LIBSQL_PREDICATE_STATE_SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS hooks_predicate_invocations (
    scope_hash  BLOB    NOT NULL,
    event_id    TEXT    NOT NULL,
    occurred_at INTEGER NOT NULL,
    tenant_id   TEXT    NOT NULL,
    PRIMARY KEY (scope_hash, event_id)
);
CREATE INDEX IF NOT EXISTS idx_hooks_predicate_invocations_scope_ts
    ON hooks_predicate_invocations (scope_hash, occurred_at);
CREATE INDEX IF NOT EXISTS idx_hooks_predicate_invocations_ts
    ON hooks_predicate_invocations (occurred_at);
CREATE INDEX IF NOT EXISTS idx_hooks_predicate_invocations_tenant
    ON hooks_predicate_invocations (tenant_id);

CREATE TABLE IF NOT EXISTS hooks_predicate_values (
    scope_hash  BLOB    NOT NULL,
    event_id    TEXT    NOT NULL,
    occurred_at INTEGER NOT NULL,
    value       TEXT    NOT NULL,
    tenant_id   TEXT    NOT NULL,
    PRIMARY KEY (scope_hash, event_id)
);
CREATE INDEX IF NOT EXISTS idx_hooks_predicate_values_scope_ts
    ON hooks_predicate_values (scope_hash, occurred_at);
CREATE INDEX IF NOT EXISTS idx_hooks_predicate_values_ts
    ON hooks_predicate_values (occurred_at);
CREATE INDEX IF NOT EXISTS idx_hooks_predicate_values_tenant
    ON hooks_predicate_values (tenant_id);
";
