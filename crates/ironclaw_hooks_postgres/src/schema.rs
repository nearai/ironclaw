//! Embedded idempotent schema for the durable predicate backend.
//!
//! The DDL mirrors `migrations/V1__predicate_counters.sql` verbatim and
//! is applied via [`PostgresPredicateStateBackend::run_migrations`] using
//! a single `batch_execute`, the same per-crate pattern
//! `ironclaw_filesystem::PostgresRootFilesystem::run_migrations` uses. We
//! deliberately do NOT route through the legacy main-binary refinery
//! `migrations/` directory: that system is scoped to `src/db/` and the
//! reborn durable crates each own their schema.
//!
//! # DB-clock decision (cross-host correctness)
//!
//! The trait passes `now: DateTime<Utc>`. There are two candidate clocks
//! for the *window comparison basis*:
//!
//! 1. The caller's `now` (stored in `ts`, compared against a
//!    caller-computed `cutoff`).
//! 2. The database's `NOW()`.
//!
//! We use **the caller's `now`** as the comparison basis — `ts < cutoff`
//! where `cutoff = now - window` is computed host-side exactly as the
//! in-memory backend does. This is the choice that makes the Postgres
//! backend a *drop-in* for the in-memory backend under the shared
//! contract harness: the contract tests drive a deterministic fixed
//! clock (`at(0)`, `at(60)`, …) and assert exact counts at the window
//! boundary. If we substituted `NOW()` for the comparison basis those
//! tests could not pin a deterministic result, and a host whose clock
//! the operator already trusts (the same `Utc::now()` the in-memory
//! backend trusts) would silently disagree with the DB clock.
//!
//! The trade-off this accepts: cross-host window correctness now depends
//! on the hosts' wall clocks being roughly synchronized (NTP), the same
//! assumption the rest of the system makes for `occurred_at` timestamps.
//! The load-bearing cross-host property — *replay dedup* — does NOT
//! depend on clock agreement: it is enforced by the
//! `PRIMARY KEY (key_hash, id)` constraint and `ON CONFLICT DO NOTHING`,
//! which is exact regardless of clock skew. Atomicity is enforced by
//! running prune + insert + aggregate inside one `REPEATABLE READ`
//! transaction, also clock-independent.

/// Idempotent schema applied by `run_migrations()`. Kept byte-compatible
/// with `migrations/V1__predicate_counters.sql`.
pub const POSTGRES_PREDICATE_SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS hook_predicate_counters (
    scope_hash         BYTEA       NOT NULL,
    key_hash           BYTEA       NOT NULL,
    kind               CHAR(1)     NOT NULL,
    id                 TEXT        NOT NULL,
    ts                 TIMESTAMPTZ NOT NULL,
    value              NUMERIC,
    PRIMARY KEY (key_hash, id)
);
CREATE INDEX IF NOT EXISTS hook_predicate_counters_key_ts_idx
    ON hook_predicate_counters (key_hash, ts);
CREATE INDEX IF NOT EXISTS hook_predicate_counters_scope_idx
    ON hook_predicate_counters (scope_hash, kind);
CREATE INDEX IF NOT EXISTS hook_predicate_counters_ts_idx
    ON hook_predicate_counters (ts);
";
