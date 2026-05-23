-- Durable predicate sliding-window state for the reborn hook framework
-- (libSQL / SQLite backend).
--
-- This crate owns its own schema (per-crate pattern, like
-- ironclaw_reborn_event_store / ironclaw_filesystem). The DDL is embedded
-- verbatim into `schema.rs` via `include_str!` and applied as an idempotent
-- batch by `run_migrations()`; this file is the single human-reviewable
-- canonical source for that schema. (Previously a hand-maintained Rust const;
-- now file-sourced so it cannot drift from a second copy.)
--
-- ## Canonical typed two-table shape (cross-backend invariant)
--
-- The two durable backends (libSQL + Postgres) share ONE logical schema:
-- two typed tables — one for invocation-count samples, one for numeric-value
-- samples — with identical column names and semantics. Storage TYPES differ
-- per backend (libSQL stores occurred_at as epoch-ms INTEGER and value as the
-- exact rust_decimal string in a TEXT column; Postgres uses native TIMESTAMPTZ
-- + NUMERIC) but the table count, column names, primary keys, and
-- eviction/dedup/quota semantics are identical. The cross-backend parity suite
-- (ironclaw_hooks_parity) proves they are behaviorally interchangeable.
--
-- ## Hash columns
--
--   scope_hash = blake3(len-prefixed tenant_id)                  -- tenant grain
--   key_hash   = blake3(map-discriminant ++ hook_id ++ tenant_id
--                        ++ capability [++ field])               -- full bucket
-- `scope_hash` is the trust boundary + per-tenant LRU-quota grain (replacing
-- the earlier raw `tenant_id` TEXT column — the digest carries the same tenant
-- identity and aligns the column set with the Postgres sibling). `key_hash` is
-- the full bucket identity: the dedup + count/sum grain and the PRIMARY KEY.
-- Both are BLOB (32-byte blake3 digests).
--
-- ## event_id column type (Codex #3635 finding)
--
-- `event_id` is TEXT, not a uuid: a synthesized PredicateEventId is a 64-char
-- blake3 hex digest that does not fit a 36-char uuid (and SQLite has no native
-- uuid type). Matches the Postgres sibling's TEXT event_id.
--
-- ## Replay-dedup
--
-- PRIMARY KEY (key_hash, event_id) is the durable equivalent of the in-memory
-- `dedup_ids` set, scoped to the full counter key (NOT global). Records use
-- INSERT … ON CONFLICT (key_hash, event_id) DO NOTHING so a replayed event_id
-- against the same key is a no-op, while the same event_id against a different
-- key (or the other table) still records. Because the invocation and value
-- tables are separate, the same event_id in both does not collide.

CREATE TABLE IF NOT EXISTS hooks_predicate_invocations (
    scope_hash  BLOB    NOT NULL,
    key_hash    BLOB    NOT NULL,
    event_id    TEXT    NOT NULL,
    occurred_at INTEGER NOT NULL,
    PRIMARY KEY (key_hash, event_id)
);
CREATE INDEX IF NOT EXISTS idx_hooks_predicate_invocations_key_ts
    ON hooks_predicate_invocations (key_hash, occurred_at);
CREATE INDEX IF NOT EXISTS idx_hooks_predicate_invocations_scope
    ON hooks_predicate_invocations (scope_hash);
CREATE INDEX IF NOT EXISTS idx_hooks_predicate_invocations_ts
    ON hooks_predicate_invocations (occurred_at);

CREATE TABLE IF NOT EXISTS hooks_predicate_values (
    scope_hash  BLOB    NOT NULL,
    key_hash    BLOB    NOT NULL,
    event_id    TEXT    NOT NULL,
    occurred_at INTEGER NOT NULL,
    value       TEXT    NOT NULL,
    PRIMARY KEY (key_hash, event_id)
);
CREATE INDEX IF NOT EXISTS idx_hooks_predicate_values_key_ts
    ON hooks_predicate_values (key_hash, occurred_at);
CREATE INDEX IF NOT EXISTS idx_hooks_predicate_values_scope
    ON hooks_predicate_values (scope_hash);
CREATE INDEX IF NOT EXISTS idx_hooks_predicate_values_ts
    ON hooks_predicate_values (occurred_at);
