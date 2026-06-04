//! Durable [`SealedGrantStore`] backends.
//!
//! The one-shot claim is enforced by a single conditional UPDATE:
//!
//! ```sql
//! UPDATE attested_sealed_grants
//!    SET status = 'claimed', claimed_at_ms = $now
//!  WHERE key_hash = $key AND status = 'sealed'
//! ```
//!
//! Exactly one concurrent claimer can match `status = 'sealed'` and flip the
//! row; the row count is the verdict. We never read-then-write, so a
//! `Stuck -> InProgress` job recovery that double-claims still resolves to one
//! winner. Rows are never deleted — a claim stamps `claimed_at_ms` in place.

#[cfg(any(feature = "postgres", feature = "libsql"))]
use async_trait::async_trait;
#[cfg(any(feature = "postgres", feature = "libsql"))]
use ironclaw_attestation::{
    AttestedSigningGrant, ClaimedGrant, GrantError, GrantKey, SealedGrantStore,
};

#[cfg(any(feature = "postgres", feature = "libsql"))]
use crate::grant_key_hash;

/// Schema shared by both backends. `created_at_ms` is the sealer's clock;
/// `claimed_at_ms` is stamped on the winning claim and is `NULL` while sealed.
/// `status` is the CAS column. The seven key components are stored for audit.
#[cfg(any(feature = "postgres", feature = "libsql"))]
const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS attested_sealed_grants (
    key_hash          TEXT PRIMARY KEY,
    tenant            TEXT NOT NULL,
    user_id           TEXT NOT NULL,
    run_id            TEXT NOT NULL,
    gate_ref          TEXT NOT NULL,
    approved_tx_hash  TEXT NOT NULL,
    key_or_account_id TEXT NOT NULL,
    chain_id          TEXT NOT NULL,
    status            TEXT NOT NULL,
    created_at_ms     BIGINT NOT NULL,
    expiry_ms         BIGINT,
    claimed_at_ms     BIGINT
);";

#[cfg(any(feature = "postgres", feature = "libsql"))]
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// PostgreSQL
// ---------------------------------------------------------------------------

#[cfg(feature = "postgres")]
mod postgres {
    use super::*;
    use deadpool_postgres::Pool;

    /// Durable PostgreSQL [`SealedGrantStore`].
    pub struct PostgresSealedGrantStore {
        pool: Pool,
    }

    impl PostgresSealedGrantStore {
        /// Wrap a connection pool (TLS/rustls is configured by the pool owner).
        pub fn new(pool: Pool) -> Self {
            Self { pool }
        }

        /// Create the table if absent. Idempotent.
        pub async fn run_migrations(&self) -> Result<(), GrantError> {
            let client = self.client().await?;
            client
                .batch_execute(SCHEMA)
                .await
                .map_err(|error| backend(&error))?;
            Ok(())
        }

        async fn client(&self) -> Result<deadpool_postgres::Object, GrantError> {
            self.pool.get().await.map_err(|error| backend(&error))
        }
    }

    fn backend(error: &dyn std::fmt::Display) -> GrantError {
        GrantError::Backend {
            reason: error.to_string(),
        }
    }

    #[async_trait]
    impl SealedGrantStore for PostgresSealedGrantStore {
        async fn seal(&self, grant: AttestedSigningGrant) -> Result<(), GrantError> {
            let client = self.client().await?;
            let key_hash = grant_key_hash(&grant.key);
            // INSERT with the PK doubling as the one-shot-seal guard: a second
            // seal of the same key violates the primary key.
            let rows = client
                .execute(
                    "INSERT INTO attested_sealed_grants \
                     (key_hash, tenant, user_id, run_id, gate_ref, approved_tx_hash, \
                      key_or_account_id, chain_id, status, created_at_ms, expiry_ms, claimed_at_ms) \
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'sealed',$9,$10,NULL) \
                     ON CONFLICT (key_hash) DO NOTHING",
                    &[
                        &key_hash,
                        &grant.key.tenant.as_str(),
                        &grant.key.user.as_str(),
                        &grant.key.run_id.as_str(),
                        &grant.key.gate_ref.as_str(),
                        &hex::encode(grant.key.approved_tx_hash.as_bytes()),
                        &grant.key.key_or_account_id.as_str(),
                        &grant.key.chain_id.as_str(),
                        &grant.created_at_ms,
                        &grant.expiry_ms,
                    ],
                )
                .await
                .map_err(|error| backend(&error))?;
            if rows == 0 {
                return Err(GrantError::AlreadySealed);
            }
            Ok(())
        }

        async fn claim(&self, key: &GrantKey) -> Result<ClaimedGrant, GrantError> {
            let client = self.client().await?;
            let key_hash = grant_key_hash(key);
            // Atomic one-shot CAS + read-back of created_at in a single
            // statement. RETURNING avoids a read-then-write race entirely.
            let claimed = client
                .query_opt(
                    "UPDATE attested_sealed_grants \
                     SET status = 'claimed', claimed_at_ms = $2 \
                     WHERE key_hash = $1 AND status = 'sealed' \
                     RETURNING created_at_ms",
                    &[&key_hash, &now_ms()],
                )
                .await
                .map_err(|error| backend(&error))?;

            if let Some(row) = claimed {
                let created_at_ms: i64 = row.get(0);
                return Ok(ClaimedGrant {
                    key: key.clone(),
                    created_at_ms,
                });
            }

            // The CAS matched nothing: either the row never existed (NotFound)
            // or it was already claimed (AlreadyClaimed). Disambiguate.
            let existing = client
                .query_opt(
                    "SELECT status FROM attested_sealed_grants WHERE key_hash = $1",
                    &[&key_hash],
                )
                .await
                .map_err(|error| backend(&error))?;
            match existing {
                None => Err(GrantError::NotFound),
                Some(_) => Err(GrantError::AlreadyClaimed),
            }
        }
    }
}

#[cfg(feature = "postgres")]
pub use postgres::PostgresSealedGrantStore;

// ---------------------------------------------------------------------------
// libSQL
// ---------------------------------------------------------------------------

#[cfg(feature = "libsql")]
mod libsql_backend {
    use super::*;
    use std::sync::Arc;

    /// Durable libSQL / Turso [`SealedGrantStore`].
    pub struct LibSqlSealedGrantStore {
        db: Arc<libsql::Database>,
    }

    impl LibSqlSealedGrantStore {
        /// Wrap a libSQL database handle.
        pub fn new(db: Arc<libsql::Database>) -> Self {
            Self { db }
        }

        /// Create the table if absent. Idempotent.
        pub async fn run_migrations(&self) -> Result<(), GrantError> {
            let conn = self.connect().await?;
            conn.execute_batch(SCHEMA)
                .await
                .map_err(|error| backend(&error))?;
            Ok(())
        }

        async fn connect(&self) -> Result<libsql::Connection, GrantError> {
            let conn = self.db.connect().map_err(|error| backend(&error))?;
            // Serialize writers so the CAS UPDATE is not lost to SQLITE_BUSY
            // under contention; the conditional UPDATE itself is the guard.
            conn.query("PRAGMA busy_timeout = 5000", ())
                .await
                .map_err(|error| backend(&error))?;
            Ok(conn)
        }
    }

    fn backend(error: &dyn std::fmt::Display) -> GrantError {
        GrantError::Backend {
            reason: error.to_string(),
        }
    }

    #[async_trait]
    impl SealedGrantStore for LibSqlSealedGrantStore {
        async fn seal(&self, grant: AttestedSigningGrant) -> Result<(), GrantError> {
            let conn = self.connect().await?;
            let key_hash = grant_key_hash(&grant.key);
            let rows = conn
                .execute(
                    "INSERT OR IGNORE INTO attested_sealed_grants \
                     (key_hash, tenant, user_id, run_id, gate_ref, approved_tx_hash, \
                      key_or_account_id, chain_id, status, created_at_ms, expiry_ms, claimed_at_ms) \
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'sealed',?9,?10,NULL)",
                    libsql::params![
                        key_hash,
                        grant.key.tenant.as_str(),
                        grant.key.user.as_str(),
                        grant.key.run_id.as_str(),
                        grant.key.gate_ref.as_str(),
                        hex::encode(grant.key.approved_tx_hash.as_bytes()),
                        grant.key.key_or_account_id.as_str(),
                        grant.key.chain_id.as_str(),
                        grant.created_at_ms,
                        grant.expiry_ms,
                    ],
                )
                .await
                .map_err(|error| backend(&error))?;
            if rows == 0 {
                return Err(GrantError::AlreadySealed);
            }
            Ok(())
        }

        async fn claim(&self, key: &GrantKey) -> Result<ClaimedGrant, GrantError> {
            let conn = self.connect().await?;
            let key_hash = grant_key_hash(key);
            // Atomic one-shot CAS: only a row still 'sealed' is flipped.
            let updated = conn
                .execute(
                    "UPDATE attested_sealed_grants \
                     SET status = 'claimed', claimed_at_ms = ?2 \
                     WHERE key_hash = ?1 AND status = 'sealed'",
                    libsql::params![key_hash.clone(), now_ms()],
                )
                .await
                .map_err(|error| backend(&error))?;

            if updated == 1 {
                // Read back created_at for the claimed grant.
                let mut rows = conn
                    .query(
                        "SELECT created_at_ms FROM attested_sealed_grants WHERE key_hash = ?1",
                        libsql::params![key_hash],
                    )
                    .await
                    .map_err(|error| backend(&error))?;
                let created_at_ms = match rows.next().await.map_err(|error| backend(&error))? {
                    Some(row) => row.get::<i64>(0).map_err(|error| backend(&error))?,
                    None => return Err(GrantError::NotFound),
                };
                return Ok(ClaimedGrant {
                    key: key.clone(),
                    created_at_ms,
                });
            }

            // CAS matched nothing: disambiguate NotFound vs AlreadyClaimed.
            let mut rows = conn
                .query(
                    "SELECT status FROM attested_sealed_grants WHERE key_hash = ?1",
                    libsql::params![key_hash],
                )
                .await
                .map_err(|error| backend(&error))?;
            match rows.next().await.map_err(|error| backend(&error))? {
                None => Err(GrantError::NotFound),
                Some(_) => Err(GrantError::AlreadyClaimed),
            }
        }
    }
}

#[cfg(feature = "libsql")]
pub use libsql_backend::LibSqlSealedGrantStore;
