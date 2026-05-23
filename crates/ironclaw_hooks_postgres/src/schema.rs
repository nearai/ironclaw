//! Embedded idempotent schema for the durable predicate backend.
//!
//! The DDL is the *single source of truth* file
//! `migrations/V1__predicate_counters.sql`, pulled in verbatim at compile
//! time via `include_str!`. There is no hand-maintained second copy to
//! drift against. It is applied via
//! [`PostgresPredicateStateBackend::run_migrations`] using
//! a single `batch_execute` (which tolerates the file's `--` SQL
//! comments), the same per-crate pattern
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
//! running prune + dedup-check + insert + aggregate inside one
//! `READ COMMITTED` transaction guarded by a per-key advisory lock, also
//! clock-independent.

/// Idempotent schema applied by `run_migrations()`. Sourced directly from
/// `migrations/V1__predicate_counters.sql` via `include_str!` so the file
/// is the only copy — no embedded duplicate can drift out of sync.
pub const POSTGRES_PREDICATE_SCHEMA: &str =
    include_str!("../migrations/V1__predicate_counters.sql");
