-- Durable predicate sliding-window state for the reborn hook framework.
--
-- This crate owns its own schema (per-crate pattern, like
-- ironclaw_reborn_event_store / ironclaw_filesystem) rather than going
-- through the legacy main-binary refinery `migrations/` directory. The
-- same DDL is embedded as an idempotent `CREATE TABLE IF NOT EXISTS`
-- batch in `schema.rs` and applied via `run_migrations()`; this file is
-- the human-reviewable canonical source for that schema.
--
-- ## Hash columns
--
-- `scope_hash` and `key_hash` are blake3 digests (32 raw bytes, BYTEA) of a
-- length-prefixed canonical serialization of the bucket identity:
--   scope_hash = blake3(len-prefixed tenant_id)
--   key_hash   = blake3(map-discriminant ++ hook_id(32) ++ tenant_id
--                        ++ capability [++ field])
-- BYTEA (not TEXT) keeps the index keys fixed-width and avoids any
-- collation surprises.
--
-- ## id column type (codex #3635 finding)
--
-- The replay-dedup id is a `PredicateEventId` — an opaque host-assigned
-- string whose canonical synth shape is a 64-char blake3 hex digest, but
-- callers may stamp other formats. Postgres `uuid` is a fixed 128-bit
-- type and will REJECT a 64-char hex digest, so the id column is `TEXT`,
-- NOT `uuid`. (#3635 docs pinned a 64-char id while the schema said uuid;
-- this resolves that contradiction in favor of TEXT.)
--
-- ## Window-clock basis
--
-- `ts` is the wall-clock timestamp passed by the caller (TIMESTAMPTZ).
-- Window comparisons are performed DB-side against a caller-supplied
-- cutoff computed from the same clock the in-memory backend uses, so the
-- trim semantics (`ts < cutoff`, entry at exact cutoff retained) match
-- the in-memory backend bit-for-bit. See `schema.rs` for the DB-clock
-- rationale.

CREATE TABLE IF NOT EXISTS hook_predicate_counters (
    scope_hash         BYTEA       NOT NULL,
    key_hash           BYTEA       NOT NULL,
    -- discriminator: 'i' = invocation counter, 'v' = numeric-value sum.
    -- Folded into key_hash too, but kept as an explicit column for
    -- index/debug clarity.
    kind               CHAR(1)     NOT NULL,
    id                 TEXT        NOT NULL,
    ts                 TIMESTAMPTZ NOT NULL,
    -- NULL for invocation rows; the recorded numeric value for value rows.
    value              NUMERIC,
    PRIMARY KEY (key_hash, id)
);

-- Window-trim + count/sum scan: every record_* call prunes and aggregates
-- over (key_hash, ts).
CREATE INDEX IF NOT EXISTS hook_predicate_counters_key_ts_idx
    ON hook_predicate_counters (key_hash, ts);

-- Per-scope (tenant) distinct-key LRU eviction scans by scope.
CREATE INDEX IF NOT EXISTS hook_predicate_counters_scope_idx
    ON hook_predicate_counters (scope_hash, kind);

-- Operator reaper (`evict_older_than`) deletes globally by age.
CREATE INDEX IF NOT EXISTS hook_predicate_counters_ts_idx
    ON hook_predicate_counters (ts);
