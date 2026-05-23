//! `PostgresPredicateStateBackend` run against the shared trait-level
//! contract harness from `ironclaw_hooks::predicate_state::contract`.
//!
//! All nine contract functions are exercised. The harness is the same
//! one the in-memory backend is wired through (PR 1/4), so this proves
//! the durable backend honors the identical isolation / dedup / window /
//! atomicity invariants by construction.
//!
//! # DB gating
//!
//! These tests need a reachable Postgres. Set
//! `IRONCLAW_HOOKS_POSTGRES_URL` (or `DATABASE_URL`) to a libpq URL. With
//! no URL the tests print a skip notice and pass, matching the
//! env-gated-skip pattern used by `ironclaw_reborn_event_store` and
//! `ironclaw_filesystem` (no testcontainers dependency).
//!
//! # Isolation
//!
//! The contract functions use fixed tenant/key names ("alpha", "beta"),
//! so concurrent runs against a shared table would collide. We serialize
//! these tests behind a process-global async mutex and `TRUNCATE` the
//! table at the start of each, giving every contract a fresh-empty
//! backend exactly as the in-memory factory does.

#![cfg(feature = "postgres")]

use std::sync::Arc;

use deadpool_postgres::Pool;
use ironclaw_hooks_postgres::PostgresPredicateStateBackend;

/// Process-global serialization lock. The contract suite reuses fixed
/// keys ("alpha", "beta"), so tests must not interleave against a shared
/// table. This is a `std::sync::Mutex` (not tokio) because each
/// `#[tokio::test]` runs on its OWN runtime — a tokio Mutex / pool tied
/// to one test's runtime cannot be reused from another's reactor (that
/// caused spurious `kind: Closed` connection errors). We hold the lock
/// across the whole test via a guard.
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn db_url() -> Option<String> {
    std::env::var("IRONCLAW_HOOKS_POSTGRES_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()
}

/// Dedicated Postgres schema for THIS test binary so it cannot collide
/// with other integration-test binaries (e.g. the adversarial suite) that
/// `cargo test` runs in parallel against the same database. Every pooled
/// connection sets `search_path` to this schema via a `post_create` hook,
/// so the backend's unqualified `hook_predicate_counters` resolves here.
const TEST_SCHEMA: &str = "hooks_predicate_contract_test";

/// Build a pool ON THE CURRENT runtime pinned to the dedicated schema,
/// ensure the schema + table, truncate, and return a fresh backend. Each
/// test calls this so the pool's connections are bound to that test's own
/// reactor.
async fn fresh_backend(url: &str) -> Option<Arc<PostgresPredicateStateBackend>> {
    // Create the isolated schema first via a one-off connection.
    {
        let (client, conn) = tokio_postgres::connect(url, tokio_postgres::NoTls)
            .await
            .ok()?;
        tokio::spawn(conn);
        client
            .batch_execute(&format!("CREATE SCHEMA IF NOT EXISTS {TEST_SCHEMA}"))
            .await
            .ok()?;
    }

    let config = url.parse::<tokio_postgres::Config>().ok()?;
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool: Pool = deadpool_postgres::Pool::builder(manager)
        .max_size(8)
        .post_create(deadpool_postgres::Hook::async_fn(|client, _| {
            Box::pin(async move {
                client
                    .batch_execute(&format!("SET search_path TO {TEST_SCHEMA}"))
                    .await
                    .map_err(|e| deadpool_postgres::HookError::message(e.to_string()))?;
                Ok(())
            })
        }))
        .build()
        .ok()?;
    let backend = PostgresPredicateStateBackend::new(pool.clone());
    backend.run_migrations().await.ok()?;
    let client = pool.get().await.ok()?;
    client
        .batch_execute("TRUNCATE TABLE hook_predicate_counters")
        .await
        .ok()?;
    Some(Arc::new(backend))
}

/// Macro: one `#[tokio::test]` per contract function. Each acquires the
/// global lock, truncates, then drives the shared contract with a factory
/// returning a clone of the freshly-prepared backend handle.
macro_rules! pg_contract {
    ($name:ident) => {
        #[tokio::test]
        // The std Mutex guard is intentionally held across awaits to
        // serialize tests that share a fixed-key table across separate
        // per-test runtimes; a tokio Mutex would be runtime-bound.
        #[allow(clippy::await_holding_lock)]
        async fn $name() {
            let Some(url) = db_url() else {
                eprintln!(
                    "skipping postgres predicate contract `{}`: \
                     IRONCLAW_HOOKS_POSTGRES_URL / DATABASE_URL not set",
                    stringify!($name)
                );
                return;
            };
            // Serialize across tests' separate runtimes. Recover from a
            // poisoned lock (a panicking test still released the table via
            // the next test's TRUNCATE).
            let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            let backend = fresh_backend(&url).await.expect("postgres setup");
            ironclaw_hooks::predicate_state::contract::$name(move || {
                // Factory is called once by the contract; hand back a
                // clone over the same isolated (truncated) table.
                PgBackendHandle(backend.clone())
            })
            .await;
        }
    };
}

/// Newtype wrapper so the contract's `B: PredicateStateBackend` bound is
/// satisfied by a cheaply-cloneable `Arc` handle. Delegates every method
/// to the inner backend.
#[derive(Clone)]
struct PgBackendHandle(Arc<PostgresPredicateStateBackend>);

#[async_trait::async_trait]
impl ironclaw_hooks::predicate_state::PredicateStateBackend for PgBackendHandle {
    async fn record_invocation(
        &self,
        key: &ironclaw_hooks::predicate_state::InvocationKey,
        event_id: &ironclaw_hooks::predicate_state::PredicateEventId,
        now: chrono::DateTime<chrono::Utc>,
        window: std::time::Duration,
    ) -> Result<u32, ironclaw_hooks::predicate_state::PredicateBackendError> {
        self.0.record_invocation(key, event_id, now, window).await
    }

    async fn record_value(
        &self,
        key: &ironclaw_hooks::predicate_state::ValueKey,
        event_id: &ironclaw_hooks::predicate_state::PredicateEventId,
        now: chrono::DateTime<chrono::Utc>,
        value: rust_decimal::Decimal,
        window: std::time::Duration,
    ) -> Result<rust_decimal::Decimal, ironclaw_hooks::predicate_state::PredicateBackendError> {
        self.0.record_value(key, event_id, now, value, window).await
    }

    fn evictions_observed(&self) -> u64 {
        self.0.evictions_observed()
    }

    async fn evict_older_than(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64, ironclaw_hooks::predicate_state::PredicateBackendError> {
        self.0.evict_older_than(cutoff).await
    }
}

pg_contract!(invocation_counts_within_window);
pg_contract!(invocation_trims_outside_window);
pg_contract!(value_sums_within_window);
pg_contract!(tenant_isolation);
pg_contract!(duplicate_event_id_is_noop_for_invocations);
pg_contract!(duplicate_event_id_is_noop_for_values);
pg_contract!(invocation_retains_entry_at_exact_window_cutoff);
pg_contract!(event_id_dedup_isolated_across_maps);
pg_contract!(record_invocation_overflow_is_fail_closed);
