-- Durable predicate sliding-window state for the reborn hook framework.
--
-- This crate owns its own schema (per-crate pattern, like
-- ironclaw_reborn_event_store / ironclaw_filesystem) rather than going
-- through the legacy main-binary refinery `migrations/` directory. The
-- DDL is embedded verbatim into `schema.rs` via `include_str!` and applied
-- as an idempotent `CREATE TABLE IF NOT EXISTS` batch by `run_migrations()`;
-- this file is the single human-reviewable canonical source for that schema.
--
-- ## Canonical typed two-table shape (cross-backend invariant)
--
-- The two durable backends (Postgres + libSQL) share ONE logical schema:
-- two typed tables — one for invocation-count samples, one for
-- numeric-value samples — with identical column names and semantics. The
-- storage TYPES differ per backend (Postgres uses native TIMESTAMPTZ +
-- NUMERIC; libSQL uses epoch-ms INTEGER + TEXT) but the table count, column
-- names, primary keys, and eviction/dedup/quota semantics are identical.
-- The cross-backend parity suite (ironclaw_hooks_parity) proves they are
-- behaviorally interchangeable. Replacing the earlier single
-- `hook_predicate_counters(kind CHAR(1), …)` table with two explicit typed
-- tables removes the `kind` discriminator AND the `value: Option<Decimal>`
-- "count smuggled through a NUMERIC" abstraction the shared record path used.
--
-- ## Hash columns
--
-- `scope_hash` and `key_hash` are blake3 digests (32 raw bytes, BYTEA):
--   scope_hash = blake3(len-prefixed tenant_id)                  -- tenant grain
--   key_hash   = blake3(map-discriminant ++ hook_id ++ tenant_id
--                        ++ capability [++ field])               -- full bucket
-- `scope_hash` is the trust boundary + per-tenant LRU-quota grain; `key_hash`
-- is the full bucket identity (the dedup + count/sum grain). BYTEA (not TEXT)
-- keeps the index keys fixed-width and avoids collation surprises.
--
-- ## event_id column type (codex #3635 finding)
--
-- The replay-dedup id is a `PredicateEventId` — an opaque host-assigned
-- string whose canonical synth shape is a 64-char blake3 hex digest, but
-- callers may stamp other formats. Postgres `uuid` is a fixed 128-bit type
-- and will REJECT a 64-char hex digest, so `event_id` is `TEXT`, NOT `uuid`.
-- (#3635 docs pinned a 64-char id while the old schema said uuid; TEXT
-- resolves that contradiction, and matches the libSQL sibling.)
--
-- ## Window-clock basis
--
-- `occurred_at` is the wall-clock timestamp passed by the caller
-- (TIMESTAMPTZ). Window comparisons are performed against a caller-supplied
-- cutoff computed from the same clock the in-memory backend uses, so the trim
-- semantics (`occurred_at < cutoff`, entry at exact cutoff retained) match the
-- in-memory backend bit-for-bit. See `schema.rs` for the DB-clock rationale.

-- Invocation-count samples. One row per recorded invocation event; the
-- in-window COUNT(*) is the invocation count.
CREATE TABLE IF NOT EXISTS hooks_predicate_invocations (
    scope_hash   BYTEA       NOT NULL,
    key_hash     BYTEA       NOT NULL,
    event_id     TEXT        NOT NULL,
    occurred_at  TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (key_hash, event_id)
);

-- Per-key window-trim + COUNT scan: every record_invocation prunes and
-- aggregates over (key_hash, occurred_at).
CREATE INDEX IF NOT EXISTS hooks_predicate_invocations_key_ts_idx
    ON hooks_predicate_invocations (key_hash, occurred_at);
-- Per-scope (tenant) distinct-key LRU eviction. enforce_scope_quota runs
-- COUNT(DISTINCT key_hash) and ranks victims by MIN(occurred_at) per key,
-- both scoped by scope_hash; the (scope_hash, key_hash, occurred_at) cover
-- lets those run as index-only scans for tenants with many recorded keys.
CREATE INDEX IF NOT EXISTS hooks_predicate_invocations_scope_idx
    ON hooks_predicate_invocations (scope_hash, key_hash, occurred_at);
-- Operator reaper (`evict_older_than`) deletes globally by age.
CREATE INDEX IF NOT EXISTS hooks_predicate_invocations_ts_idx
    ON hooks_predicate_invocations (occurred_at);

-- Numeric-value samples. One row per recorded value event; the in-window
-- SUM(value) is the running sum. `value` is NOT NULL here (the typed tables
-- make the count-vs-sum distinction explicit, so no nullable double-duty).
CREATE TABLE IF NOT EXISTS hooks_predicate_values (
    scope_hash   BYTEA       NOT NULL,
    key_hash     BYTEA       NOT NULL,
    event_id     TEXT        NOT NULL,
    occurred_at  TIMESTAMPTZ NOT NULL,
    value        NUMERIC     NOT NULL,
    PRIMARY KEY (key_hash, event_id)
);

CREATE INDEX IF NOT EXISTS hooks_predicate_values_key_ts_idx
    ON hooks_predicate_values (key_hash, occurred_at);
-- Same index-only-scan cover for the value table's scope-quota pass.
CREATE INDEX IF NOT EXISTS hooks_predicate_values_scope_idx
    ON hooks_predicate_values (scope_hash, key_hash, occurred_at);
CREATE INDEX IF NOT EXISTS hooks_predicate_values_ts_idx
    ON hooks_predicate_values (occurred_at);
