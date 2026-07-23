//! Durable [`SealedGrantStore`] backends MUST satisfy the SAME behavioural
//! contract as the in-memory reference impl. We drive the canonical contract
//! case functions from `ironclaw_attestation` (exposed via `contract-suite`)
//! against a fresh durable store per case — including the concurrent one-shot
//! CAS case, which proves the DB-level `UPDATE ... WHERE status='sealed'`
//! actually serializes claims.
//!
//! * libSQL runs against a local temp-file database (no external infra), so it
//!   executes on any `--features "libsql,contract-suite,integration"` build.
//! * PostgreSQL is gated on `ATTESTED_STORE_TEST_PG_URL`; absent it, the PG
//!   cases are skipped so CI without a database still passes while the code
//!   stays compiled (run with `--features "postgres,contract-suite,integration"`).

#![cfg(all(feature = "integration", feature = "contract-suite"))]

use ironclaw_attestation::grant::contract;

// ---------------------------------------------------------------------------
// libSQL (local temp-file; always runs under the integration feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "libsql")]
mod libsql_backend {
    use super::*;
    use std::sync::Arc;

    use ironclaw_attested_store::LibSqlSealedGrantStore;
    use tempfile::TempDir;

    /// Returns the store alongside the owning [`TempDir`]; the caller must hold
    /// the `TempDir` for the lifetime of the store so the on-disk db file is not
    /// reaped (and is cleaned up on drop — no `mem::forget` leak).
    async fn fresh() -> (LibSqlSealedGrantStore, TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("grants.db");
        let db = Arc::new(
            libsql::Builder::new_local(path)
                .build()
                .await
                .expect("build libsql db"),
        );
        let store = LibSqlSealedGrantStore::new(db);
        store.run_migrations().await.expect("migrate");
        (store, dir)
    }

    #[tokio::test]
    async fn seal_then_claim_succeeds() {
        let (store, _dir) = fresh().await;
        contract::seal_then_claim_succeeds(store).await;
    }
    #[tokio::test]
    async fn second_claim_is_already_claimed() {
        let (store, _dir) = fresh().await;
        contract::second_claim_is_already_claimed(store).await;
    }
    #[tokio::test]
    async fn claim_unsealed_is_not_found() {
        let (store, _dir) = fresh().await;
        contract::claim_unsealed_is_not_found(store).await;
    }
    #[tokio::test]
    async fn claim_mismatched_component_is_not_found() {
        let (store, _dir) = fresh().await;
        contract::claim_mismatched_component_is_not_found(store).await;
    }
    #[tokio::test]
    async fn double_seal_is_already_sealed() {
        let (store, _dir) = fresh().await;
        contract::double_seal_is_already_sealed(store).await;
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_claims_yield_exactly_one_winner() {
        let (store, _dir) = fresh().await;
        contract::concurrent_claims_yield_exactly_one_winner(store).await;
    }
}

// ---------------------------------------------------------------------------
// PostgreSQL (env-gated on a live database URL)
// ---------------------------------------------------------------------------

#[cfg(feature = "postgres")]
mod postgres_backend {
    use super::*;

    use deadpool_postgres::{Config, Runtime};
    use ironclaw_attested_store::PostgresSealedGrantStore;
    use tokio_postgres::NoTls;

    /// Build a store against the test database, dropping any prior table so each
    /// run starts clean. Returns `None` when no test DB is configured.
    async fn fresh() -> Option<PostgresSealedGrantStore> {
        let url = std::env::var("ATTESTED_STORE_TEST_PG_URL").ok()?;
        let mut config = Config::new();
        config.url = Some(url);
        let pool = config
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .expect("create pool");
        {
            let client = pool.get().await.expect("client");
            client
                .batch_execute("DROP TABLE IF EXISTS attested_sealed_grants")
                .await
                .expect("drop");
        }
        let store = PostgresSealedGrantStore::new(pool);
        store.run_migrations().await.expect("migrate");
        Some(store)
    }

    macro_rules! pg_case {
        ($name:ident, $flavor:meta) => {
            #[tokio::test($flavor)]
            async fn $name() {
                let Some(store) = fresh().await else {
                    eprintln!(
                        "ATTESTED_STORE_TEST_PG_URL unset; skipping {}",
                        stringify!($name)
                    );
                    return;
                };
                contract::$name(store).await;
            }
        };
        ($name:ident) => {
            #[tokio::test]
            async fn $name() {
                let Some(store) = fresh().await else {
                    eprintln!(
                        "ATTESTED_STORE_TEST_PG_URL unset; skipping {}",
                        stringify!($name)
                    );
                    return;
                };
                contract::$name(store).await;
            }
        };
    }

    pg_case!(seal_then_claim_succeeds);
    pg_case!(second_claim_is_already_claimed);
    pg_case!(claim_unsealed_is_not_found);
    pg_case!(claim_mismatched_component_is_not_found);
    pg_case!(double_seal_is_already_sealed);
    pg_case!(
        concurrent_claims_yield_exactly_one_winner,
        flavor = "multi_thread"
    );
}
