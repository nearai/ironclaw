//! libSQL / SQLite schema for the durable predicate-state backend.
//!
//! Two typed tables, one per predicate kind, sharing the canonical
//! cross-backend column set (see `migrations/V1__predicate_state.sql`):
//!
//! ```sql
//! CREATE TABLE hooks_predicate_invocations (
//!     scope_hash  BLOB    NOT NULL,  -- blake3(tenant_id)  (tenant grain)
//!     key_hash    BLOB    NOT NULL,  -- blake3(hook_id ‖ tenant_id ‖ capability)
//!     event_id    TEXT    NOT NULL,  -- PredicateEventId, host-assigned hex
//!     occurred_at INTEGER NOT NULL,  -- epoch milliseconds (canonical host clock)
//!     PRIMARY KEY (key_hash, event_id)
//! );
//! CREATE TABLE hooks_predicate_values (
//!     scope_hash  BLOB    NOT NULL,
//!     key_hash    BLOB    NOT NULL,  -- … ‖ field
//!     event_id    TEXT    NOT NULL,
//!     occurred_at INTEGER NOT NULL,
//!     value       TEXT    NOT NULL,  -- rust_decimal, exact string (NUMERIC convention)
//!     PRIMARY KEY (key_hash, event_id)
//! );
//! ```
//!
//! ## Canonical typed two-table shape (cross-backend invariant)
//!
//! Both durable backends (libSQL + Postgres) share ONE logical schema: same
//! two tables, same column names (`scope_hash`, `key_hash`, `event_id`,
//! `occurred_at`, `value`), same primary keys, same eviction/dedup/quota
//! semantics. Only the native storage types differ (libSQL: epoch-ms INTEGER +
//! TEXT decimal; Postgres: TIMESTAMPTZ + NUMERIC). The `scope_hash` column is
//! the tenant grain (replacing the earlier raw `tenant_id` TEXT column — the
//! digest carries the same identity and aligns the column set with Postgres);
//! `key_hash` is the full bucket identity and the dedup grain.
//!
//! ## event_id column type (Codex #3635 finding)
//!
//! `event_id` is **TEXT**, not a `uuid`-typed column: a synthesized
//! `PredicateEventId` is a 64-char blake3 hex digest which does not fit a
//! 36-char `uuid`. SQLite has no native `uuid` type regardless, but the TEXT
//! choice is called out here so the libSQL and Postgres schemas stay
//! semantically aligned.
//!
//! ## Replay-dedup
//!
//! The `PRIMARY KEY (key_hash, event_id)` is the durable equivalent of the
//! in-memory `dedup_ids` set, scoped to the full counter key (NOT global).
//! Records use `INSERT … ON CONFLICT (key_hash, event_id) DO NOTHING` so a
//! replayed `event_id` against the same key is a no-op, while the same
//! `event_id` against a different key (or the other table) still records.
//! Because the invocation and value tables are separate, the same `event_id`
//! in both does not collide (the `event_id_dedup_isolated_across_maps`
//! contract).

pub(crate) const INVOCATIONS_TABLE: &str = "hooks_predicate_invocations";
pub(crate) const VALUES_TABLE: &str = "hooks_predicate_values";

/// Idempotent schema applied by `run_migrations()`. Sourced directly from
/// `migrations/V1__predicate_state.sql` via `include_str!` so the file is the
/// only copy — no hand-maintained Rust const can drift out of sync.
pub(crate) const LIBSQL_PREDICATE_STATE_SCHEMA: &str =
    include_str!("../migrations/V1__predicate_state.sql");
