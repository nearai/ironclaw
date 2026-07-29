//! Durable [`IntentStore`] backends MUST satisfy the SAME behavioural contract
//! as the in-memory reference impl.
//!
//! Two cases carry most of the weight for a durable backend. `the_raw_token_is
//! _never_stored_or_rendered` reads the record back OUT of the database, so a
//! backend that persisted the raw review token — turning a table read into a
//! working approval credential — fails here. `resolution_is_one_shot_and_
//! terminal` proves the conditional `UPDATE ... WHERE state = 'pending'` really
//! rejects a late outcome rather than overwriting a decision that landed.
//!
//! * libSQL runs against a local temp-file database (no external infra).
//! * PostgreSQL is gated on `ATTESTED_STORE_TEST_PG_URL`; absent it, the PG
//!   cases skip so CI without a database still passes while the code stays
//!   compiled.

#![cfg(all(feature = "integration", feature = "contract-suite"))]

use ironclaw_attestation::intent_store::contract;

// ---------------------------------------------------------------------------
// libSQL (local temp-file; always runs under the integration feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "libsql")]
mod libsql_backend {
    use super::*;
    use std::sync::Arc;

    use ironclaw_attested_store::LibSqlIntentStore;
    use tempfile::TempDir;

    /// The caller must hold the [`TempDir`] for the store's lifetime so the
    /// on-disk file is not reaped before the case finishes.
    async fn fresh() -> (LibSqlIntentStore, TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("intents.db");
        let db = Arc::new(
            libsql::Builder::new_local(path)
                .build()
                .await
                .expect("build libsql db"),
        );
        let store = LibSqlIntentStore::new(db);
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

    libsql_case!(put_then_get_round_trips);
    libsql_case!(writes_are_insert_only);
    libsql_case!(an_intent_does_not_resolve_under_another_tenant);
    libsql_case!(the_same_intent_id_is_distinct_per_tenant);
    libsql_case!(a_token_hash_resolves_its_intent_and_nothing_else);
    libsql_case!(the_raw_token_is_never_stored_or_rendered);
    libsql_case!(resolution_is_one_shot_and_terminal);
    libsql_case!(a_cross_tenant_resolve_is_not_found_and_changes_nothing);
    libsql_case!(an_intent_is_reachable_by_its_gate_ref);
}

// ---------------------------------------------------------------------------
// PostgreSQL (env-gated on a live database URL)
// ---------------------------------------------------------------------------

#[cfg(feature = "postgres")]
mod postgres_backend {
    use super::*;

    use deadpool_postgres::{Config, Runtime};
    use ironclaw_attested_store::PostgresIntentStore;
    use tokio_postgres::NoTls;

    /// Drops any prior table so each run starts clean. `None` when no test
    /// database is configured.
    async fn fresh() -> Option<PostgresIntentStore> {
        let url = std::env::var("ATTESTED_STORE_TEST_PG_URL").ok()?;
        let mut config = Config::new();
        config.url = Some(url);
        let pool = config
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .expect("create pool");
        {
            let client = pool.get().await.expect("client");
            client
                .batch_execute("DROP TABLE IF EXISTS attested_intents")
                .await
                .expect("drop");
        }
        let store = PostgresIntentStore::new(pool);
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

    pg_case!(put_then_get_round_trips);
    pg_case!(writes_are_insert_only);
    pg_case!(an_intent_does_not_resolve_under_another_tenant);
    pg_case!(the_same_intent_id_is_distinct_per_tenant);
    pg_case!(a_token_hash_resolves_its_intent_and_nothing_else);
    pg_case!(the_raw_token_is_never_stored_or_rendered);
    pg_case!(resolution_is_one_shot_and_terminal);
    pg_case!(a_cross_tenant_resolve_is_not_found_and_changes_nothing);
    pg_case!(an_intent_is_reachable_by_its_gate_ref);
}
