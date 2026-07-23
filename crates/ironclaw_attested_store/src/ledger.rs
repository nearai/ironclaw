//! Durable [`SigningLedger`] backends.
//!
//! Two DB-level guards:
//!
//! * **One-shot create** — `INSERT` against the `gate_ref` primary key; a
//!   duplicate is a unique violation mapped to [`LedgerError::AlreadyExists`].
//! * **Idempotent advance** — the legal transition is checked in Rust via
//!   [`SigningLedgerState::can_advance_to`], then applied with a conditional
//!   `UPDATE ... WHERE gate_ref = ? AND state = <from>`. The `WHERE state`
//!   clause is the real guard: under a concurrent `Stuck -> InProgress`
//!   recovery, only the advance whose observed `from` still matches the row
//!   wins, so a `BroadcastSubmitted` row can never be dragged back to
//!   `Signing`/`Signed`. A zero-row update is re-classified by re-reading the
//!   row.
//!
//! Rows are never deleted; every advance stamps `last_transition_ms`.

#[cfg(any(feature = "postgres", feature = "libsql"))]
use async_trait::async_trait;
#[cfg(any(feature = "postgres", feature = "libsql"))]
use ironclaw_attestation::{LedgerKey, LedgerError, SigningLedger, SigningLedgerState};
#[cfg(any(feature = "postgres", feature = "libsql"))]
use ironclaw_signing_provider::GateRef;

#[cfg(any(feature = "postgres", feature = "libsql"))]
const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS attested_signing_ledger (
    tenant             TEXT NOT NULL,
    gate_ref           TEXT NOT NULL,
    state              TEXT NOT NULL,
    created_at_ms      BIGINT NOT NULL,
    last_transition_ms BIGINT NOT NULL,
    PRIMARY KEY (tenant, gate_ref)
);";

/// snake_case wire token for a ledger state (matches the serde repr).
#[cfg(any(feature = "postgres", feature = "libsql"))]
fn state_token(state: SigningLedgerState) -> &'static str {
    use SigningLedgerState::*;
    match state {
        Approved => "approved",
        Signing => "signing",
        Signed => "signed",
        BroadcastSubmitted => "broadcast_submitted",
        Finalized => "finalized",
        Unknown => "unknown",
        ManualReview => "manual_review",
    }
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
fn parse_state(token: &str) -> Option<SigningLedgerState> {
    use SigningLedgerState::*;
    Some(match token {
        "approved" => Approved,
        "signing" => Signing,
        "signed" => Signed,
        "broadcast_submitted" => BroadcastSubmitted,
        "finalized" => Finalized,
        "unknown" => Unknown,
        "manual_review" => ManualReview,
        _ => return None,
    })
}

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

    /// Durable PostgreSQL [`SigningLedger`].
    pub struct PostgresSigningLedger {
        pool: Pool,
    }

    impl PostgresSigningLedger {
        /// Wrap a connection pool.
        pub fn new(pool: Pool) -> Self {
            Self { pool }
        }

        /// Create the table if absent. Idempotent.
        pub async fn run_migrations(&self) -> Result<(), LedgerError> {
            let client = self.client().await?;
            client
                .batch_execute(SCHEMA)
                .await
                .map_err(|error| backend(&error))?;
            Ok(())
        }

        async fn client(&self) -> Result<deadpool_postgres::Object, LedgerError> {
            self.pool.get().await.map_err(|error| backend(&error))
        }

        /// Read the current state on a *caller-supplied* client. `advance` uses
        /// this so the read and the conditional `UPDATE` run on a single pooled
        /// connection — they must not straddle two connections, or the
        /// read-then-CAS pair loses its one-connection consistency guarantee.
        async fn read_state_on(
            client: &deadpool_postgres::Object,
            key: &LedgerKey,
        ) -> Result<SigningLedgerState, LedgerError> {
            let row = client
                .query_opt(
                    "SELECT state FROM attested_signing_ledger WHERE tenant = $1 AND gate_ref = $2",
                    &[&key.tenant.as_str(), &key.gate_ref.as_str()],
                )
                .await
                .map_err(|error| backend(&error))?
                .ok_or(LedgerError::NotFound)?;
            let token: String = row.get(0);
            parse_state(&token).ok_or_else(|| LedgerError::Backend {
                reason: format!("unknown ledger state token: {token}"),
            })
        }
    }

    fn backend(error: &dyn std::fmt::Display) -> LedgerError {
        LedgerError::Backend {
            reason: error.to_string(),
        }
    }

    #[async_trait]
    impl SigningLedger for PostgresSigningLedger {
        async fn create(&self, key: &LedgerKey) -> Result<(), LedgerError> {
            let client = self.client().await?;
            let now = now_ms();
            let rows = client
                .execute(
                    "INSERT INTO attested_signing_ledger \
                     (tenant, gate_ref, state, created_at_ms, last_transition_ms) \
                     VALUES ($1, $2, 'approved', $3, $3) \
                     ON CONFLICT (tenant, gate_ref) DO NOTHING",
                    &[&key.tenant.as_str(), &key.gate_ref.as_str(), &now],
                )
                .await
                .map_err(|error| backend(&error))?;
            if rows == 0 {
                return Err(LedgerError::AlreadyExists);
            }
            Ok(())
        }

        async fn state(&self, key: &LedgerKey) -> Result<SigningLedgerState, LedgerError> {
            let client = self.client().await?;
            Self::read_state_on(&client, key).await
        }

        async fn advance(
            &self,
            key: &LedgerKey,
            to: SigningLedgerState,
        ) -> Result<(), LedgerError> {
            let client = self.client().await?;
            // Read the current state, validate the transition, then apply it
            // with a conditional UPDATE keyed on that same `from`. The read and
            // the CAS UPDATE run on THE SAME pooled connection (read_state_on
            // takes the client we just acquired) so the read-then-CAS pair never
            // straddles two connections. The `WHERE state = <from>` clause is
            // the real one-shot guard: if a concurrent advance moved the row
            // between our read and our UPDATE, our UPDATE matches zero rows and
            // we fall into the lost-CAS branch below — the row can never be
            // double-advanced.
            let from = Self::read_state_on(&client, key).await?;
            if !from.can_advance_to(to) {
                return Err(LedgerError::InvalidTransition { from, to });
            }
            let updated = client
                .execute(
                    "UPDATE attested_signing_ledger \
                     SET state = $4, last_transition_ms = $5 \
                     WHERE tenant = $1 AND gate_ref = $2 AND state = $3",
                    &[
                        &key.tenant.as_str(),
                        &key.gate_ref.as_str(),
                        &state_token(from),
                        &state_token(to),
                        &now_ms(),
                    ],
                )
                .await
                .map_err(|error| backend(&error))?;
            if updated == 1 {
                return Ok(());
            }
            // Lost the CAS: another writer moved the row between our read and our
            // conditional UPDATE (or it vanished). Re-read on the same client and
            // report a distinct ConcurrentAdvance — this is a lost race, NOT a
            // caller-supplied illegal transition.
            let observed = Self::read_state_on(&client, key).await?;
            Err(LedgerError::ConcurrentAdvance { observed, to })
        }
    }
}

#[cfg(feature = "postgres")]
pub use postgres::PostgresSigningLedger;

// ---------------------------------------------------------------------------
// libSQL
// ---------------------------------------------------------------------------

#[cfg(feature = "libsql")]
mod libsql_backend {
    use super::*;
    use std::sync::Arc;

    /// Durable libSQL / Turso [`SigningLedger`].
    pub struct LibSqlSigningLedger {
        db: Arc<libsql::Database>,
    }

    impl LibSqlSigningLedger {
        /// Wrap a libSQL database handle.
        pub fn new(db: Arc<libsql::Database>) -> Self {
            Self { db }
        }

        /// Create the table if absent. Idempotent.
        pub async fn run_migrations(&self) -> Result<(), LedgerError> {
            let conn = self.connect().await?;
            conn.execute_batch(SCHEMA)
                .await
                .map_err(|error| backend(&error))?;
            Ok(())
        }

        async fn connect(&self) -> Result<libsql::Connection, LedgerError> {
            let conn = self.db.connect().map_err(|error| backend(&error))?;
            conn.query("PRAGMA busy_timeout = 5000", ())
                .await
                .map_err(|error| backend(&error))?;
            Ok(conn)
        }

        async fn read_state(
            &self,
            conn: &libsql::Connection,
            key: &LedgerKey,
        ) -> Result<SigningLedgerState, LedgerError> {
            let mut rows = conn
                .query(
                    "SELECT state FROM attested_signing_ledger WHERE tenant = ?1 AND gate_ref = ?2",
                    libsql::params![key.tenant.as_str(), key.gate_ref.as_str()],
                )
                .await
                .map_err(|error| backend(&error))?;
            let row = rows
                .next()
                .await
                .map_err(|error| backend(&error))?
                .ok_or(LedgerError::NotFound)?;
            let token: String = row.get(0).map_err(|error| backend(&error))?;
            parse_state(&token).ok_or_else(|| LedgerError::Backend {
                reason: format!("unknown ledger state token: {token}"),
            })
        }
    }

    fn backend(error: &dyn std::fmt::Display) -> LedgerError {
        LedgerError::Backend {
            reason: error.to_string(),
        }
    }

    #[async_trait]
    impl SigningLedger for LibSqlSigningLedger {
        async fn create(&self, key: &LedgerKey) -> Result<(), LedgerError> {
            let conn = self.connect().await?;
            let now = now_ms();
            let rows = conn
                .execute(
                    "INSERT OR IGNORE INTO attested_signing_ledger \
                     (tenant, gate_ref, state, created_at_ms, last_transition_ms) \
                     VALUES (?1, ?2, 'approved', ?3, ?3)",
                    libsql::params![key.tenant.as_str(), key.gate_ref.as_str(), now],
                )
                .await
                .map_err(|error| backend(&error))?;
            if rows == 0 {
                return Err(LedgerError::AlreadyExists);
            }
            Ok(())
        }

        async fn state(&self, key: &LedgerKey) -> Result<SigningLedgerState, LedgerError> {
            let conn = self.connect().await?;
            self.read_state(&conn, key).await
        }

        async fn advance(
            &self,
            key: &LedgerKey,
            to: SigningLedgerState,
        ) -> Result<(), LedgerError> {
            let conn = self.connect().await?;
            let from = self.read_state(&conn, key).await?;
            if !from.can_advance_to(to) {
                return Err(LedgerError::InvalidTransition { from, to });
            }
            let updated = conn
                .execute(
                    "UPDATE attested_signing_ledger \
                     SET state = ?4, last_transition_ms = ?5 \
                     WHERE tenant = ?1 AND gate_ref = ?2 AND state = ?3",
                    libsql::params![
                        key.tenant.as_str(),
                        key.gate_ref.as_str(),
                        state_token(from),
                        state_token(to),
                        now_ms()
                    ],
                )
                .await
                .map_err(|error| backend(&error))?;
            if updated == 1 {
                return Ok(());
            }
            // Lost the CAS: another writer moved the row between our read and the
            // conditional UPDATE. Re-read on the same connection and report a
            // distinct ConcurrentAdvance (a lost race, not an illegal transition).
            let observed = self.read_state(&conn, key).await?;
            Err(LedgerError::ConcurrentAdvance { observed, to })
        }
    }
}

#[cfg(feature = "libsql")]
pub use libsql_backend::LibSqlSigningLedger;
