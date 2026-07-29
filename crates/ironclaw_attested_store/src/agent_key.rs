//! Durable [`AgentSigningKeyStore`] backends (attested-signing Phase B).
//!
//! Only the **public** half of each agent key lives here. The private half is
//! sealed by `ironclaw_attested_runtime::SealedAgentKeyStore` under an AAD bound
//! to `(tenant, agent, generation)` and never reaches this table — a dump of
//! this table yields verification material and lifecycle, nothing signable.
//!
//! ## The window is computed in Rust, not in SQL
//!
//! [`AgentSigningKeyStore::verifying_key`] loads the row and then defers to the
//! shared [`verification_admits`] predicate rather than expressing the overlap
//! window as a `WHERE` clause. Two backends each re-deriving that arithmetic in
//! their own dialect is exactly how a revoked key stays live in one of them —
//! and a revoked key that still verifies is a signing key an operator believes
//! is dead. There is one definition of admission and every backend calls it.
//!
//! ## Lifecycle transitions are conditional UPDATEs
//!
//! `Revoked` is terminal, so retirement is `... WHERE state <> 'revoked'`; a
//! zero row count is then disambiguated by a follow-up read into `Revoked`
//! (the row exists and is terminal) or `NotFound` (no row). Rows are never
//! deleted — a key's whole history stays auditable.

#[cfg(any(feature = "postgres", feature = "libsql"))]
use async_trait::async_trait;
#[cfg(any(feature = "postgres", feature = "libsql"))]
use ironclaw_attestation::{
    AGENT_PUBLIC_KEY_LEN, AgentKeyError, AgentKeyId, AgentKeyState, AgentSigningKey,
    AgentSigningKeyStore, verification_admits,
};
#[cfg(any(feature = "postgres", feature = "libsql"))]
use ironclaw_signing_provider::TenantId;

/// Schema shared by both backends.
///
/// The primary key IS the [`AgentKeyId`], which is what makes registration
/// insert-only: a re-register of a generation cannot replace a public key that
/// already-minted intents name.
#[cfg(any(feature = "postgres", feature = "libsql"))]
const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS attested_agent_signing_keys (
    tenant             TEXT NOT NULL,
    agent              TEXT NOT NULL,
    generation         BIGINT NOT NULL,
    public_key         TEXT NOT NULL,
    state              TEXT NOT NULL,
    created_at_ms      BIGINT NOT NULL,
    retiring_since_ms  BIGINT,
    PRIMARY KEY (tenant, agent, generation)
);";

/// The wire spelling of a lifecycle state. Explicit rather than derived so a
/// rename in Rust cannot silently orphan every persisted row.
#[cfg(any(feature = "postgres", feature = "libsql"))]
fn state_to_str(state: AgentKeyState) -> &'static str {
    match state {
        AgentKeyState::Active => "active",
        AgentKeyState::Retiring => "retiring",
        AgentKeyState::Revoked => "revoked",
    }
}

/// Parse a persisted state. An unrecognized value fails closed as `Revoked`'s
/// stricter sibling — a backend error — rather than defaulting to `Active`.
#[cfg(any(feature = "postgres", feature = "libsql"))]
fn state_from_str(raw: &str) -> Result<AgentKeyState, AgentKeyError> {
    match raw {
        "active" => Ok(AgentKeyState::Active),
        "retiring" => Ok(AgentKeyState::Retiring),
        "revoked" => Ok(AgentKeyState::Revoked),
        other => Err(AgentKeyError::Backend {
            reason: format!("unrecognized agent key state {other:?}"),
        }),
    }
}

/// Decode the stored hex public key.
#[cfg(any(feature = "postgres", feature = "libsql"))]
fn public_key_from_hex(raw: &str) -> Result<[u8; AGENT_PUBLIC_KEY_LEN], AgentKeyError> {
    let bytes = hex::decode(raw).map_err(|error| AgentKeyError::Backend {
        reason: format!("agent public key is not hex: {error}"),
    })?;
    bytes.try_into().map_err(|_| AgentKeyError::Backend {
        reason: format!("agent public key must be {AGENT_PUBLIC_KEY_LEN} bytes"),
    })
}

/// Assemble a key from its columns.
#[cfg(any(feature = "postgres", feature = "libsql"))]
fn key_from_columns(
    tenant: String,
    agent: String,
    generation: i64,
    public_key: String,
    state: String,
    created_at_ms: i64,
    retiring_since_ms: Option<i64>,
) -> Result<AgentSigningKey, AgentKeyError> {
    let generation = u32::try_from(generation).map_err(|_| AgentKeyError::Backend {
        reason: "agent key generation is out of range".to_string(),
    })?;
    Ok(AgentSigningKey {
        key_id: AgentKeyId::new(TenantId::new(tenant), agent, generation),
        public_key: public_key_from_hex(&public_key)?,
        state: state_from_str(&state)?,
        created_at_ms,
        retiring_since_ms,
    })
}

// ---------------------------------------------------------------------------
// PostgreSQL
// ---------------------------------------------------------------------------

#[cfg(feature = "postgres")]
mod postgres {
    use super::*;
    use deadpool_postgres::Pool;

    /// Durable PostgreSQL [`AgentSigningKeyStore`].
    pub struct PostgresAgentSigningKeyStore {
        pool: Pool,
    }

    impl PostgresAgentSigningKeyStore {
        /// Wrap a connection pool (TLS/rustls is configured by the pool owner).
        pub fn new(pool: Pool) -> Self {
            Self { pool }
        }

        /// Create the table if absent. Idempotent.
        pub async fn run_migrations(&self) -> Result<(), AgentKeyError> {
            let client = self.client().await?;
            client
                .batch_execute(SCHEMA)
                .await
                .map_err(|error| backend(&error))?;
            Ok(())
        }

        async fn client(&self) -> Result<deadpool_postgres::Object, AgentKeyError> {
            self.pool.get().await.map_err(|error| backend(&error))
        }

        async fn load(&self, key_id: &AgentKeyId) -> Result<AgentSigningKey, AgentKeyError> {
            let client = self.client().await?;
            let row = client
                .query_opt(
                    "SELECT tenant, agent, generation, public_key, state, created_at_ms, \
                            retiring_since_ms \
                       FROM attested_agent_signing_keys \
                      WHERE tenant = $1 AND agent = $2 AND generation = $3",
                    &[
                        &key_id.tenant.as_str(),
                        &key_id.agent.as_str(),
                        &i64::from(key_id.generation),
                    ],
                )
                .await
                .map_err(|error| backend(&error))?
                .ok_or(AgentKeyError::NotFound)?;
            key_from_columns(
                row.get(0),
                row.get(1),
                row.get(2),
                row.get(3),
                row.get(4),
                row.get(5),
                row.get(6),
            )
        }
    }

    fn backend(error: &dyn std::fmt::Display) -> AgentKeyError {
        AgentKeyError::Backend {
            reason: error.to_string(),
        }
    }

    #[async_trait]
    impl AgentSigningKeyStore for PostgresAgentSigningKeyStore {
        async fn register(&self, key: AgentSigningKey) -> Result<(), AgentKeyError> {
            let client = self.client().await?;
            // The primary key doubles as the insert-only guard.
            let rows = client
                .execute(
                    "INSERT INTO attested_agent_signing_keys \
                     (tenant, agent, generation, public_key, state, created_at_ms, \
                      retiring_since_ms) \
                     VALUES ($1,$2,$3,$4,$5,$6,$7) \
                     ON CONFLICT (tenant, agent, generation) DO NOTHING",
                    &[
                        &key.key_id.tenant.as_str(),
                        &key.key_id.agent.as_str(),
                        &i64::from(key.key_id.generation),
                        &hex::encode(key.public_key),
                        &state_to_str(key.state),
                        &key.created_at_ms,
                        &key.retiring_since_ms,
                    ],
                )
                .await
                .map_err(|error| backend(&error))?;
            if rows == 0 {
                return Err(AgentKeyError::AlreadyExists);
            }
            Ok(())
        }

        async fn active_key(
            &self,
            tenant: &TenantId,
            agent: &str,
        ) -> Result<AgentSigningKey, AgentKeyError> {
            let client = self.client().await?;
            // Highest generation wins if more than one is somehow active,
            // matching the in-memory reference.
            let row = client
                .query_opt(
                    "SELECT tenant, agent, generation, public_key, state, created_at_ms, \
                            retiring_since_ms \
                       FROM attested_agent_signing_keys \
                      WHERE tenant = $1 AND agent = $2 AND state = 'active' \
                      ORDER BY generation DESC LIMIT 1",
                    &[&tenant.as_str(), &agent],
                )
                .await
                .map_err(|error| backend(&error))?
                .ok_or(AgentKeyError::NotFound)?;
            key_from_columns(
                row.get(0),
                row.get(1),
                row.get(2),
                row.get(3),
                row.get(4),
                row.get(5),
                row.get(6),
            )
        }

        async fn verifying_key(
            &self,
            key_id: &AgentKeyId,
            now_ms: i64,
            overlap_ms: i64,
        ) -> Result<[u8; AGENT_PUBLIC_KEY_LEN], AgentKeyError> {
            let key = self.load(key_id).await?;
            verification_admits(&key, now_ms, overlap_ms)?;
            Ok(key.public_key)
        }

        async fn retire(&self, key_id: &AgentKeyId, now_ms: i64) -> Result<(), AgentKeyError> {
            let client = self.client().await?;
            let rows = client
                .execute(
                    "UPDATE attested_agent_signing_keys \
                        SET state = 'retiring', retiring_since_ms = $4 \
                      WHERE tenant = $1 AND agent = $2 AND generation = $3 \
                        AND state <> 'revoked'",
                    &[
                        &key_id.tenant.as_str(),
                        &key_id.agent.as_str(),
                        &i64::from(key_id.generation),
                        &now_ms,
                    ],
                )
                .await
                .map_err(|error| backend(&error))?;
            if rows == 0 {
                // Either the row is revoked (terminal) or it is not there.
                return Err(match self.load(key_id).await {
                    Ok(_) => AgentKeyError::Revoked,
                    Err(error) => error,
                });
            }
            Ok(())
        }

        async fn revoke(&self, key_id: &AgentKeyId) -> Result<(), AgentKeyError> {
            let client = self.client().await?;
            let rows = client
                .execute(
                    "UPDATE attested_agent_signing_keys SET state = 'revoked' \
                      WHERE tenant = $1 AND agent = $2 AND generation = $3",
                    &[
                        &key_id.tenant.as_str(),
                        &key_id.agent.as_str(),
                        &i64::from(key_id.generation),
                    ],
                )
                .await
                .map_err(|error| backend(&error))?;
            if rows == 0 {
                return Err(AgentKeyError::NotFound);
            }
            Ok(())
        }
    }
}

#[cfg(feature = "postgres")]
pub use postgres::PostgresAgentSigningKeyStore;

// ---------------------------------------------------------------------------
// libSQL
// ---------------------------------------------------------------------------

#[cfg(feature = "libsql")]
mod libsql_backend {
    use super::*;
    use std::sync::Arc;

    /// Durable libSQL / Turso [`AgentSigningKeyStore`].
    pub struct LibSqlAgentSigningKeyStore {
        db: Arc<libsql::Database>,
    }

    impl LibSqlAgentSigningKeyStore {
        /// Wrap a libSQL database handle.
        pub fn new(db: Arc<libsql::Database>) -> Self {
            Self { db }
        }

        /// Create the table if absent. Idempotent.
        pub async fn run_migrations(&self) -> Result<(), AgentKeyError> {
            let conn = self.connect().await?;
            conn.execute_batch(SCHEMA)
                .await
                .map_err(|error| backend(&error))?;
            Ok(())
        }

        async fn connect(&self) -> Result<libsql::Connection, AgentKeyError> {
            let conn = self.db.connect().map_err(|error| backend(&error))?;
            conn.query("PRAGMA busy_timeout = 5000", ())
                .await
                .map_err(|error| backend(&error))?;
            Ok(conn)
        }

        async fn load(&self, key_id: &AgentKeyId) -> Result<AgentSigningKey, AgentKeyError> {
            let conn = self.connect().await?;
            let mut rows = conn
                .query(
                    "SELECT tenant, agent, generation, public_key, state, created_at_ms, \
                            retiring_since_ms \
                       FROM attested_agent_signing_keys \
                      WHERE tenant = ?1 AND agent = ?2 AND generation = ?3",
                    libsql::params![
                        key_id.tenant.as_str(),
                        key_id.agent.as_str(),
                        i64::from(key_id.generation),
                    ],
                )
                .await
                .map_err(|error| backend(&error))?;
            let row = rows
                .next()
                .await
                .map_err(|error| backend(&error))?
                .ok_or(AgentKeyError::NotFound)?;
            row_to_key(&row)
        }
    }

    fn backend(error: &dyn std::fmt::Display) -> AgentKeyError {
        AgentKeyError::Backend {
            reason: error.to_string(),
        }
    }

    fn row_to_key(row: &libsql::Row) -> Result<AgentSigningKey, AgentKeyError> {
        let tenant: String = row.get(0).map_err(|error| backend(&error))?;
        let agent: String = row.get(1).map_err(|error| backend(&error))?;
        let generation: i64 = row.get(2).map_err(|error| backend(&error))?;
        let public_key: String = row.get(3).map_err(|error| backend(&error))?;
        let state: String = row.get(4).map_err(|error| backend(&error))?;
        let created_at_ms: i64 = row.get(5).map_err(|error| backend(&error))?;
        let retiring_since_ms: Option<i64> = row.get(6).map_err(|error| backend(&error))?;
        key_from_columns(
            tenant,
            agent,
            generation,
            public_key,
            state,
            created_at_ms,
            retiring_since_ms,
        )
    }

    #[async_trait]
    impl AgentSigningKeyStore for LibSqlAgentSigningKeyStore {
        async fn register(&self, key: AgentSigningKey) -> Result<(), AgentKeyError> {
            let conn = self.connect().await?;
            let rows = conn
                .execute(
                    "INSERT OR IGNORE INTO attested_agent_signing_keys \
                     (tenant, agent, generation, public_key, state, created_at_ms, \
                      retiring_since_ms) \
                     VALUES (?1,?2,?3,?4,?5,?6,?7)",
                    libsql::params![
                        key.key_id.tenant.as_str(),
                        key.key_id.agent.as_str(),
                        i64::from(key.key_id.generation),
                        hex::encode(key.public_key),
                        state_to_str(key.state),
                        key.created_at_ms,
                        key.retiring_since_ms,
                    ],
                )
                .await
                .map_err(|error| backend(&error))?;
            if rows == 0 {
                return Err(AgentKeyError::AlreadyExists);
            }
            Ok(())
        }

        async fn active_key(
            &self,
            tenant: &TenantId,
            agent: &str,
        ) -> Result<AgentSigningKey, AgentKeyError> {
            let conn = self.connect().await?;
            let mut rows = conn
                .query(
                    "SELECT tenant, agent, generation, public_key, state, created_at_ms, \
                            retiring_since_ms \
                       FROM attested_agent_signing_keys \
                      WHERE tenant = ?1 AND agent = ?2 AND state = 'active' \
                      ORDER BY generation DESC LIMIT 1",
                    libsql::params![tenant.as_str(), agent],
                )
                .await
                .map_err(|error| backend(&error))?;
            let row = rows
                .next()
                .await
                .map_err(|error| backend(&error))?
                .ok_or(AgentKeyError::NotFound)?;
            row_to_key(&row)
        }

        async fn verifying_key(
            &self,
            key_id: &AgentKeyId,
            now_ms: i64,
            overlap_ms: i64,
        ) -> Result<[u8; AGENT_PUBLIC_KEY_LEN], AgentKeyError> {
            let key = self.load(key_id).await?;
            verification_admits(&key, now_ms, overlap_ms)?;
            Ok(key.public_key)
        }

        async fn retire(&self, key_id: &AgentKeyId, now_ms: i64) -> Result<(), AgentKeyError> {
            let conn = self.connect().await?;
            let rows = conn
                .execute(
                    "UPDATE attested_agent_signing_keys \
                        SET state = 'retiring', retiring_since_ms = ?4 \
                      WHERE tenant = ?1 AND agent = ?2 AND generation = ?3 \
                        AND state <> 'revoked'",
                    libsql::params![
                        key_id.tenant.as_str(),
                        key_id.agent.as_str(),
                        i64::from(key_id.generation),
                        now_ms,
                    ],
                )
                .await
                .map_err(|error| backend(&error))?;
            if rows == 0 {
                return Err(match self.load(key_id).await {
                    Ok(_) => AgentKeyError::Revoked,
                    Err(error) => error,
                });
            }
            Ok(())
        }

        async fn revoke(&self, key_id: &AgentKeyId) -> Result<(), AgentKeyError> {
            let conn = self.connect().await?;
            let rows = conn
                .execute(
                    "UPDATE attested_agent_signing_keys SET state = 'revoked' \
                      WHERE tenant = ?1 AND agent = ?2 AND generation = ?3",
                    libsql::params![
                        key_id.tenant.as_str(),
                        key_id.agent.as_str(),
                        i64::from(key_id.generation),
                    ],
                )
                .await
                .map_err(|error| backend(&error))?;
            if rows == 0 {
                return Err(AgentKeyError::NotFound);
            }
            Ok(())
        }
    }
}

#[cfg(feature = "libsql")]
pub use libsql_backend::LibSqlAgentSigningKeyStore;
