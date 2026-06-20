//! Deployment-wide leader-lock for the background credential keepalive worker.
//!
//! # Leader-lock model
//!
//! Only ONE process per deployment should sweep all Google credential accounts
//! and refresh idle tokens per tick. This module provides a utility the worker
//! uses to elect a single leader per tick:
//!
//! - **Postgres path**: the worker calls [`CredentialRefreshLeaderLock::run_as_leader`]
//!   each tick. That call opens a transaction and tries
//!   `pg_try_advisory_xact_lock` using a single fixed deployment-wide key. If
//!   the lock is already held by another process, the current process is not the
//!   leader — it skips the sweep and returns [`LeaderOutcome::NotLeader`]. If the
//!   lock is acquired, the sweep runs inside the transaction, which is then
//!   committed (releasing the lock) and [`LeaderOutcome::Ran`] is returned. A
//!   single connection is held only for the duration of the sweep; because the
//!   worker ticks at ~6 h the connection cost is negligible.
//!
//!   The lock is **transaction-level**, not session-level, on purpose: a
//!   transaction-level advisory lock is released automatically when the
//!   transaction ends for any reason — commit, rollback, panic, or the future
//!   being dropped on shutdown/cancellation. This is cancellation-safe: an
//!   aborted sweep can never strand the lock on a pooled connection and block
//!   all future leader elections.
//! - **libsql / single-process path** (`pool = None`): this process is
//!   trivially the only process, so the sweep always runs. No connection is
//!   held.
//!
//! # Two-layer concurrency ownership
//!
//! - **Cross-process** (this module): one leader per deployment tick; all
//!   other processes skip.
//! - **Intra-process** (in [`ironclaw_auth::ProviderBackedCredentialAccountService`]):
//!   the existing `refresh_locks: Mutex<HashMap<CredentialAccountId, …>>` prevents
//!   multiple tokio tasks inside one process from racing to the token endpoint.
//!   That guard is retained and is NOT removed.
//!
//! The inline dispatch path (hot path) is **unaffected** by this module: it
//! uses only the in-process `refresh_locks` guard and never acquires a DB
//! connection.
//!
//! # Advisory lock key
//!
//! Postgres advisory locks live in ONE global 64-bit namespace shared by all
//! connections in the cluster. The key is two `i32` values (`(key1, key2)`)
//! that together form a 64-bit slot. We use two fixed literal constants chosen
//! to be statistically unlikely to alias with other advisory-lock users in the
//! same cluster (hooks predicate, etc.). This is the same approach as
//! `ironclaw_hooks_postgres` — collision avoidance by statistical improbability,
//! not structural isolation.
//!
//! # Pool / connection error fallback
//!
//! If the pool returns a connection error, or `pg_try_advisory_lock` itself
//! fails, the worker logs at `debug!` and skips the tick entirely
//! (fail-closed). Keepalive is best-effort: missing one tick is harmless
//! because the worker retries on the next scheduled interval. The in-process
//! guard inside `ProviderBackedCredentialAccountService` still prevents local
//! stampede if connectivity recovers mid-tick.

#[cfg(feature = "postgres")]
use tracing::debug;

// ---------------------------------------------------------------------------
// Fixed deployment-wide advisory-lock key
// ---------------------------------------------------------------------------
// Two arbitrary i32 literals that uniquely identify "credential keepalive
// worker leader election" in the Postgres advisory-lock namespace.
// These were chosen as stable constants; do NOT derive them from a runtime
// value — the key must be identical across all processes in the deployment.
#[cfg(feature = "postgres")]
const KEEPALIVE_LOCK_KEY: (i32, i32) = (0x4B45_4550i32, 0x414C_4956i32); // "KEEP", "ALIV"

// ---------------------------------------------------------------------------
// Public outcome type
// ---------------------------------------------------------------------------

/// Outcome of [`CredentialRefreshLeaderLock::run_as_leader`].
pub(crate) enum LeaderOutcome<T> {
    /// Another process holds the leader lock; sweep was skipped.
    ///
    /// Only constructed on the Postgres path; under non-postgres builds the
    /// worker is always the trivial leader, so this variant is unreachable.
    #[cfg_attr(not(feature = "postgres"), allow(dead_code))]
    NotLeader,
    /// This process was the leader; the sweep ran and returned `result`.
    Ran(T),
}

// ---------------------------------------------------------------------------
// Leader-lock utility
// ---------------------------------------------------------------------------

/// Deployment-wide leader-lock for the background credential keepalive worker.
///
/// Constructed by the composition root (`runtime.rs`) and threaded into the
/// worker's [`crate::credential_refresh_worker::CredentialRefreshWorkerDeps`].
///
/// The Postgres pool is intentionally `Option`: `None` on the libsql /
/// local-dev path means this process is trivially the leader (single writer by
/// deployment topology), so the sweep always runs without touching the DB.
pub(crate) struct CredentialRefreshLeaderLock {
    /// Postgres pool for leader-lock acquisition. `None` on the libsql /
    /// local-dev path → always-leader (pass-through). MUST stay private;
    /// never exposed through any public API or the composition facade.
    #[cfg(feature = "postgres")]
    pool: Option<deadpool_postgres::Pool>,
    /// Marker field so the struct is non-empty when the postgres feature is
    /// off. ZST — zero runtime cost.
    #[cfg(not(feature = "postgres"))]
    _marker: (),
}

impl CredentialRefreshLeaderLock {
    /// Build a leader lock backed by a Postgres pool (Postgres path).
    ///
    /// Pass `None` for `pool` to get the always-leader (libsql) behaviour.
    #[cfg(feature = "postgres")]
    pub(crate) fn new(pool: Option<deadpool_postgres::Pool>) -> Self {
        Self { pool }
    }

    /// Build an always-leader lock (no pool, libsql / local-dev path).
    #[cfg(not(feature = "postgres"))]
    pub(crate) fn always_leader() -> Self {
        Self { _marker: () }
    }

    /// Try to become the deployment-wide sweep leader for one tick.
    ///
    /// - On the libsql path (pool is conceptually absent): always invokes
    ///   `sweep` and returns [`LeaderOutcome::Ran`].
    /// - On the Postgres path: opens a transaction and acquires
    ///   `pg_try_advisory_xact_lock` (non-blocking). If the lock is not acquired,
    ///   returns [`LeaderOutcome::NotLeader`] without invoking `sweep`. If
    ///   acquired, invokes `sweep` inside the transaction, then commits to
    ///   release the lock. On pool/connection/transaction errors, skips the tick
    ///   and returns [`LeaderOutcome::NotLeader`] (fail-closed; retries next tick).
    pub(crate) async fn run_as_leader<F, Fut, T>(&self, sweep: F) -> LeaderOutcome<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        #[cfg(feature = "postgres")]
        {
            if let Some(pool) = &self.pool {
                return run_as_leader_postgres(pool, sweep).await;
            }
        }
        // No pool (libsql / local-dev / no-postgres): trivially the leader.
        LeaderOutcome::Ran(sweep().await)
    }
}

// ---------------------------------------------------------------------------
// Postgres leader-lock logic
// ---------------------------------------------------------------------------

#[cfg(feature = "postgres")]
async fn run_as_leader_postgres<F, Fut, T>(
    pool: &deadpool_postgres::Pool,
    sweep: F,
) -> LeaderOutcome<T>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let (key_a, key_b) = KEEPALIVE_LOCK_KEY;

    // Obtain a connection from the pool. On failure, skip the tick (fail-closed):
    // keepalive is best-effort and will retry on the next scheduled interval.
    let mut conn = match pool.get().await {
        Ok(c) => c,
        Err(err) => {
            debug!(
                error = %err,
                "credential-refresh leader lock: pool error; skipping tick (fail-closed)"
            );
            return LeaderOutcome::NotLeader;
        }
    };

    // Acquire the lock as a TRANSACTION-level advisory lock
    // (`pg_try_advisory_xact_lock`), not a session-level one. A transaction-level
    // lock is released automatically when the transaction ends for ANY reason —
    // commit, rollback, or the connection/future being dropped on panic or
    // cancellation. That makes the leader lock cancellation-safe: a panicking or
    // aborted sweep can never strand the lock on a pooled connection (which would
    // block every future tick from electing a leader). The transaction is held
    // open across the sweep so the lock is owned for the whole leader window; at
    // ~6 h ticks with a small `max_per_tick` the held connection is negligible.
    let tx = match conn.transaction().await {
        Ok(tx) => tx,
        Err(err) => {
            debug!(
                error = %err,
                "credential-refresh leader lock: begin transaction failed; skipping tick (fail-closed)"
            );
            return LeaderOutcome::NotLeader;
        }
    };

    // pg_try_advisory_xact_lock is non-blocking: returns true if this transaction
    // acquired the lock, false if another session/transaction already holds it.
    let acquired: bool = match tx
        .query_one(
            "SELECT pg_try_advisory_xact_lock($1, $2)",
            &[&key_a, &key_b],
        )
        .await
    {
        Ok(row) => row.get(0),
        Err(err) => {
            debug!(
                error = %err,
                "credential-refresh leader lock: pg_try_advisory_xact_lock failed; skipping tick (fail-closed)"
            );
            return LeaderOutcome::NotLeader;
        }
    };

    if !acquired {
        debug!("credential-refresh leader lock: not the leader this tick; skipping sweep");
        // `tx` drops here → rolled back → no lock is held.
        return LeaderOutcome::NotLeader;
    }

    // We are the leader. Run the sweep, then commit to release the xact lock and
    // return the connection cleanly to the pool. If commit fails, dropping `tx`
    // rolls the transaction back, which also releases the lock.
    let result = sweep().await;
    if let Err(err) = tx.commit().await {
        debug!(
            error = %err,
            "credential-refresh leader lock: commit failed; lock releases on transaction rollback"
        );
    }

    LeaderOutcome::Ran(result)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    // ---------------------------------------------------------------------------
    // Always-leader path (no pool)
    // ---------------------------------------------------------------------------

    /// On the no-pool path the sweep always runs.
    #[tokio::test]
    async fn always_leader_runs_sweep() {
        #[cfg(not(feature = "postgres"))]
        let lock = CredentialRefreshLeaderLock::always_leader();
        #[cfg(feature = "postgres")]
        let lock = CredentialRefreshLeaderLock::new(None);

        let ran = Arc::new(AtomicBool::new(false));
        let ran_clone = Arc::clone(&ran);
        let outcome = lock
            .run_as_leader(|| async move {
                ran_clone.store(true, Ordering::SeqCst);
                42u32
            })
            .await;
        assert!(ran.load(Ordering::SeqCst), "sweep must have run");
        assert!(
            matches!(outcome, LeaderOutcome::Ran(42)),
            "outcome must be Ran(42)"
        );
    }

    /// Sweep runs twice on successive calls (no state leaks between calls).
    #[tokio::test]
    async fn always_leader_runs_sweep_twice() {
        #[cfg(not(feature = "postgres"))]
        let lock = CredentialRefreshLeaderLock::always_leader();
        #[cfg(feature = "postgres")]
        let lock = CredentialRefreshLeaderLock::new(None);

        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for _ in 0..2 {
            let count_clone = Arc::clone(&count);
            lock.run_as_leader(|| async move {
                count_clone.fetch_add(1, Ordering::SeqCst);
            })
            .await;
        }
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }
}
