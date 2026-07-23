//! Durable [`SigningLedger`] backends driven through the canonical
//! `ironclaw_attestation` contract cases (the broadcast-idempotency guard,
//! one-shot create, transition validation), proving the DB-level conditional
//! `UPDATE ... WHERE state = <from>` enforces the same state machine.

#![cfg(all(feature = "integration", feature = "contract-suite"))]

use ironclaw_attestation::ledger::contract;

/// Concurrent-CAS proof for the durable `UPDATE ... WHERE state = <from>`.
///
/// The sequential `broadcast_idempotency_guard` case proves the transition
/// rules, but not that the DB-level conditional UPDATE actually serializes
/// concurrent advances. This drives many tasks racing to advance the SAME row
/// `Signed -> BroadcastSubmitted` (the exact transition a `Stuck -> InProgress`
/// recovery double-fire would attempt) and asserts EXACTLY ONE wins — so a
/// second broadcast with fresh chain metadata can never be produced under
/// contention. Defined here (not in `ironclaw_attestation`) because the durable
/// backends are the only impls whose atomicity is in question; the in-memory
/// reference is single-mutex by construction.
mod concurrent {
    use std::sync::Arc;

    use ironclaw_attestation::{LedgerError, LedgerKey, SigningLedger, SigningLedgerState};
    use ironclaw_signing_provider::TenantId;
    use ironclaw_signing_provider::GateRef;

    pub async fn advance_to_broadcast_yields_one_winner<L>(ledger: L)
    where
        L: SigningLedger + Send + Sync + 'static,
    {
        use SigningLedgerState::*;
        let ledger = Arc::new(ledger);
        let gate = LedgerKey::new(TenantId::new("tenant-a"), GateRef::new("gate:ledger-concurrent"));

        // Drive the row up to `Signed` sequentially.
        ledger.create(&gate).await.expect("create");
        ledger.advance(&gate, Signing).await.expect("signing");
        ledger.advance(&gate, Signed).await.expect("signed");

        // Now race 32 tasks all attempting Signed -> BroadcastSubmitted.
        let mut handles = Vec::new();
        for _ in 0..32 {
            let ledger = Arc::clone(&ledger);
            let gate = gate.clone();
            handles.push(tokio::spawn(async move {
                ledger.advance(&gate, BroadcastSubmitted).await
            }));
        }

        // Losers fall into one of two correct buckets depending on whether they
        // read the row before or after the winner's UPDATE landed:
        //   * read `Signed` (valid), then the conditional UPDATE matches zero
        //     rows because the row is now `BroadcastSubmitted` -> `ConcurrentAdvance`
        //   * read `BroadcastSubmitted` directly -> `InvalidTransition`
        // Either way the loser does NOT advance the row a second time. The
        // load-bearing assertion is `ok == 1`: the one-shot transition holds.
        let mut ok = 0usize;
        let mut lost = 0usize;
        for handle in handles {
            match handle.await.expect("task join") {
                Ok(()) => ok += 1,
                Err(LedgerError::ConcurrentAdvance { observed, .. }) => {
                    assert_eq!(
                        observed, BroadcastSubmitted,
                        "lost-CAS re-read must observe the winner's state"
                    );
                    lost += 1;
                }
                Err(LedgerError::InvalidTransition { from, .. }) => {
                    assert_eq!(
                        from, BroadcastSubmitted,
                        "InvalidTransition loser must have read the post-win state"
                    );
                    lost += 1;
                }
                Err(other) => panic!("unexpected error under contention: {other:?}"),
            }
        }
        assert_eq!(ok, 1, "exactly one advance must win the CAS (one-shot)");
        assert_eq!(
            lost, 31,
            "all 31 losers must fail-closed, never double-advance"
        );
        assert_eq!(
            ledger.state(&gate).await.expect("state"),
            BroadcastSubmitted
        );
    }
}

#[cfg(feature = "libsql")]
mod libsql_backend {
    use super::*;
    use std::sync::Arc;

    use ironclaw_attested_store::LibSqlSigningLedger;
    use tempfile::TempDir;

    /// Returns the ledger alongside the owning [`TempDir`]; the caller must hold
    /// the `TempDir` for the lifetime of the ledger so the on-disk db file is
    /// not reaped (and is cleaned up on drop — no `mem::forget` leak).
    async fn fresh() -> (LibSqlSigningLedger, TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ledger.db");
        let db = Arc::new(
            libsql::Builder::new_local(path)
                .build()
                .await
                .expect("build libsql db"),
        );
        let ledger = LibSqlSigningLedger::new(db);
        ledger.run_migrations().await.expect("migrate");
        (ledger, dir)
    }

    #[tokio::test]
    async fn full_valid_sequence() {
        let (ledger, _dir) = fresh().await;
        contract::full_valid_sequence(ledger).await;
    }
    #[tokio::test]
    async fn second_create_is_already_exists() {
        let (ledger, _dir) = fresh().await;
        contract::second_create_is_already_exists(ledger).await;
    }
    #[tokio::test]
    async fn advance_missing_is_not_found() {
        let (ledger, _dir) = fresh().await;
        contract::advance_missing_is_not_found(ledger).await;
    }
    #[tokio::test]
    async fn skip_forward_is_invalid() {
        let (ledger, _dir) = fresh().await;
        contract::skip_forward_is_invalid(ledger).await;
    }
    #[tokio::test]
    async fn regression_is_invalid() {
        let (ledger, _dir) = fresh().await;
        contract::regression_is_invalid(ledger).await;
    }
    #[tokio::test]
    async fn broadcast_idempotency_guard() {
        let (ledger, _dir) = fresh().await;
        contract::broadcast_idempotency_guard(ledger).await;
    }
    #[tokio::test]
    async fn terminal_states_never_advance() {
        let (ledger, _dir) = fresh().await;
        contract::terminal_states_never_advance(ledger).await;
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_advance_to_broadcast_yields_one_winner() {
        let (ledger, _dir) = fresh().await;
        super::concurrent::advance_to_broadcast_yields_one_winner(ledger).await;
    }
}

#[cfg(feature = "postgres")]
mod postgres_backend {
    use super::*;

    use deadpool_postgres::{Config, Runtime};
    use ironclaw_attested_store::PostgresSigningLedger;
    use tokio_postgres::NoTls;

    async fn fresh() -> Option<PostgresSigningLedger> {
        let url = std::env::var("ATTESTED_STORE_TEST_PG_URL").ok()?;
        let mut config = Config::new();
        config.url = Some(url);
        let pool = config
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .expect("create pool");
        {
            let client = pool.get().await.expect("client");
            client
                .batch_execute("DROP TABLE IF EXISTS attested_signing_ledger")
                .await
                .expect("drop");
        }
        let ledger = PostgresSigningLedger::new(pool);
        ledger.run_migrations().await.expect("migrate");
        Some(ledger)
    }

    macro_rules! pg_case {
        ($name:ident) => {
            #[tokio::test]
            async fn $name() {
                let Some(ledger) = fresh().await else {
                    eprintln!(
                        "ATTESTED_STORE_TEST_PG_URL unset; skipping {}",
                        stringify!($name)
                    );
                    return;
                };
                contract::$name(ledger).await;
            }
        };
    }

    pg_case!(full_valid_sequence);
    pg_case!(second_create_is_already_exists);
    pg_case!(advance_missing_is_not_found);
    pg_case!(skip_forward_is_invalid);
    pg_case!(regression_is_invalid);
    pg_case!(broadcast_idempotency_guard);
    pg_case!(terminal_states_never_advance);

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_advance_to_broadcast_yields_one_winner() {
        let Some(ledger) = fresh().await else {
            eprintln!(
                "ATTESTED_STORE_TEST_PG_URL unset; skipping \
                 concurrent_advance_to_broadcast_yields_one_winner"
            );
            return;
        };
        super::concurrent::advance_to_broadcast_yields_one_winner(ledger).await;
    }
}
