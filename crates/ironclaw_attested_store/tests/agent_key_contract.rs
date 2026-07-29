//! Durable [`AgentSigningKeyStore`] backends MUST satisfy the SAME behavioural
//! contract as the in-memory reference impl.
//!
//! Rotation and revocation are the cases that matter most here: an overlap
//! window computed off the wrong column, or a revoked key a `WHERE` clause
//! still hands out, is a live signing key an operator believes is dead. The
//! canonical cases live in `ironclaw_attestation` (exposed via
//! `contract-suite`) and every backend runs them unchanged.
//!
//! * libSQL runs against a local temp-file database (no external infra).
//! * PostgreSQL is gated on `ATTESTED_STORE_TEST_PG_URL`; absent it, the PG
//!   cases skip so CI without a database still passes while the code stays
//!   compiled.

#![cfg(all(feature = "integration", feature = "contract-suite"))]

use ironclaw_attestation::agent_key::contract;

// ---------------------------------------------------------------------------
// libSQL (local temp-file; always runs under the integration feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "libsql")]
mod libsql_backend {
    use super::*;
    use std::sync::Arc;

    use ironclaw_attested_store::LibSqlAgentSigningKeyStore;
    use tempfile::TempDir;

    /// The caller must hold the [`TempDir`] for the store's lifetime so the
    /// on-disk file is not reaped before the case finishes.
    async fn fresh() -> (LibSqlAgentSigningKeyStore, TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("agent_keys.db");
        let db = Arc::new(
            libsql::Builder::new_local(path)
                .build()
                .await
                .expect("build libsql db"),
        );
        let store = LibSqlAgentSigningKeyStore::new(db);
        store.run_migrations().await.expect("migrate");
        (store, dir)
    }

    macro_rules! libsql_case {
        ($name:ident) => {
            #[tokio::test]
            async fn $name() {
                let (store, _dir) = fresh().await;
                contract::$name(store).await;
            }
        };
    }

    libsql_case!(an_active_key_signs_and_verifies);
    libsql_case!(registration_is_insert_only);
    libsql_case!(rotation_moves_signing_forward_while_the_old_key_still_verifies);
    libsql_case!(the_overlap_window_closes_at_its_boundary);
    libsql_case!(revocation_is_immediate_with_no_overlap);
    libsql_case!(a_retiring_key_is_never_offered_for_signing);
    libsql_case!(keys_do_not_resolve_across_tenants);
    libsql_case!(an_unknown_key_is_not_found);
}

// ---------------------------------------------------------------------------
// PostgreSQL (env-gated on a live database URL)
// ---------------------------------------------------------------------------

#[cfg(feature = "postgres")]
mod postgres_backend {
    use super::*;

    use deadpool_postgres::{Config, Runtime};
    use ironclaw_attested_store::PostgresAgentSigningKeyStore;
    use tokio_postgres::NoTls;

    /// Drops any prior table so each run starts clean. `None` when no test
    /// database is configured.
    async fn fresh() -> Option<PostgresAgentSigningKeyStore> {
        let url = std::env::var("ATTESTED_STORE_TEST_PG_URL").ok()?;
        let mut config = Config::new();
        config.url = Some(url);
        let pool = config
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .expect("create pool");
        {
            let client = pool.get().await.expect("client");
            client
                .batch_execute("DROP TABLE IF EXISTS attested_agent_signing_keys")
                .await
                .expect("drop");
        }
        let store = PostgresAgentSigningKeyStore::new(pool);
        store.run_migrations().await.expect("migrate");
        Some(store)
    }

    macro_rules! pg_case {
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

    pg_case!(an_active_key_signs_and_verifies);
    pg_case!(registration_is_insert_only);
    pg_case!(rotation_moves_signing_forward_while_the_old_key_still_verifies);
    pg_case!(the_overlap_window_closes_at_its_boundary);
    pg_case!(revocation_is_immediate_with_no_overlap);
    pg_case!(a_retiring_key_is_never_offered_for_signing);
    pg_case!(keys_do_not_resolve_across_tenants);
    pg_case!(an_unknown_key_is_not_found);
}
