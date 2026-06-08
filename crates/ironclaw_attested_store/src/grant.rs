//! Durable [`SealedGrantStore`] backends.
//!
//! The one-shot claim is enforced by a single conditional UPDATE:
//!
//! ```sql
//! UPDATE attested_sealed_grants
//!    SET status = 'claimed', claimed_at_ms = $now
//!  WHERE key_hash = $key AND status = 'sealed'
//!    AND (expiry_ms IS NULL OR expiry_ms > $now)
//! ```
//!
//! Exactly one concurrent claimer can match `status = 'sealed'` and flip the
//! row; the row count is the verdict. We never read-then-write, so a
//! `Stuck -> InProgress` job recovery that double-claims still resolves to one
//! winner. Rows are never deleted — a claim stamps `claimed_at_ms` in place.
//!
//! Expiry is enforced in the same CAS: a still-`sealed` row whose `expiry_ms`
//! has passed (`expiry_ms <= $now`) is NOT flipped, leaving it `sealed` (never
//! consumed, never deleted). When the CAS matches nothing the backend
//! disambiguates `NotFound` (no row), `AlreadyClaimed` (row is `claimed`), and
//! `Expired` (row is still `sealed` but past `expiry_ms`). `$now` is the runtime
//! clock supplied by the caller — the store never reads the wall clock, so
//! resume stays deterministic and expiry is testable.

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

        async fn claim(&self, key: &GrantKey, now_ms: i64) -> Result<ClaimedGrant, GrantError> {
            let client = self.client().await?;
            let key_hash = grant_key_hash(key);
            // Atomic one-shot CAS + expiry guard + read-back of created_at in a
            // single statement. RETURNING avoids a read-then-write race entirely.
            // An expired (or claimed) row matches nothing and is left untouched.
            let claimed = client
                .query_opt(
                    "UPDATE attested_sealed_grants \
                     SET status = 'claimed', claimed_at_ms = $2 \
                     WHERE key_hash = $1 AND status = 'sealed' \
                       AND (expiry_ms IS NULL OR expiry_ms > $2) \
                     RETURNING created_at_ms",
                    &[&key_hash, &now_ms],
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

            // The CAS matched nothing: the row never existed (NotFound), was
            // already claimed (AlreadyClaimed), or is still sealed but past its
            // expiry (Expired — distinct, and never consumed). Disambiguate from
            // the persisted status + expiry.
            let existing = client
                .query_opt(
                    "SELECT status, expiry_ms FROM attested_sealed_grants WHERE key_hash = $1",
                    &[&key_hash],
                )
                .await
                .map_err(|error| backend(&error))?;
            match existing {
                None => Err(GrantError::NotFound),
                Some(row) => {
                    let status: String = row.get(0);
                    let expiry_ms: Option<i64> = row.get(1);
                    if status == "sealed"
                        && let Some(expiry_ms) = expiry_ms
                        && now_ms >= expiry_ms
                    {
                        return Err(GrantError::Expired { expiry_ms, now_ms });
                    }
                    Err(GrantError::AlreadyClaimed)
                }
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

        async fn claim(&self, key: &GrantKey, now_ms: i64) -> Result<ClaimedGrant, GrantError> {
            let conn = self.connect().await?;
            let key_hash = grant_key_hash(key);
            // Atomic one-shot CAS + expiry guard: only a row still 'sealed' AND
            // not past its expiry is flipped. An expired row matches nothing and
            // is left 'sealed' (never consumed, never deleted).
            let updated = conn
                .execute(
                    "UPDATE attested_sealed_grants \
                     SET status = 'claimed', claimed_at_ms = ?2 \
                     WHERE key_hash = ?1 AND status = 'sealed' \
                       AND (expiry_ms IS NULL OR expiry_ms > ?2)",
                    libsql::params![key_hash.clone(), now_ms],
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

            // CAS matched nothing: disambiguate NotFound, AlreadyClaimed, and
            // Expired (still 'sealed' but past expiry — never consumed).
            let mut rows = conn
                .query(
                    "SELECT status, expiry_ms FROM attested_sealed_grants WHERE key_hash = ?1",
                    libsql::params![key_hash],
                )
                .await
                .map_err(|error| backend(&error))?;
            match rows.next().await.map_err(|error| backend(&error))? {
                None => Err(GrantError::NotFound),
                Some(row) => {
                    let status = row.get::<String>(0).map_err(|error| backend(&error))?;
                    let expiry_ms = row.get::<Option<i64>>(1).map_err(|error| backend(&error))?;
                    if status == "sealed"
                        && let Some(expiry_ms) = expiry_ms
                        && now_ms >= expiry_ms
                    {
                        return Err(GrantError::Expired { expiry_ms, now_ms });
                    }
                    Err(GrantError::AlreadyClaimed)
                }
            }
        }
    }
}

#[cfg(feature = "libsql")]
pub use libsql_backend::LibSqlSealedGrantStore;
