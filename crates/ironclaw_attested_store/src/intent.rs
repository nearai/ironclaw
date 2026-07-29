//! Durable [`IntentStore`] backends (attested-signing Phase B §B3).
//!
//! ## The review token is stored hashed, never raw
//!
//! Only the SHA-256 of the review token reaches this table, as lowercase hex.
//! That is the property that makes a dump of this table useless for approving
//! anything: the raw token exists only in the chat message that carried it.
//! The hash column is `UNIQUE` because it is a lookup key presented before any
//! session exists — two intents sharing one are indistinguishable at the URL.
//!
//! ## The state column is a PROJECTION, never an authorization
//!
//! `state` mirrors the gate outcome; it does not decide it. The sealed one-shot
//! grant CAS in [`crate::PostgresSealedGrantStore`] and its libSQL twin remain
//! the only thing that authorizes advancing a turn. `resolve` is nonetheless a
//! conditional `UPDATE ... WHERE state = 'pending'` so a late or duplicate
//! outcome cannot rewrite a decision that already landed — the projection
//! stays a faithful mirror rather than drifting into its own answer.
//!
//! ## Every read is tenant-qualified except the token lookup
//!
//! `find_by_token_hash` deliberately is not: the token arrives from an
//! unauthenticated URL, so there is no tenant to qualify by yet. The caller
//! runs the authorization checks in `ironclaw_attestation::intent_review` on
//! the returned record and answers a uniform 404 on any failure.
//!
//! Rows are never deleted.

#[cfg(any(feature = "postgres", feature = "libsql"))]
use async_trait::async_trait;
#[cfg(any(feature = "postgres", feature = "libsql"))]
use ironclaw_attestation::{
    INTENT_SIGNATURE_LEN, IntentId, IntentRecord, IntentState, IntentStore, IntentStoreError,
    ReviewTokenHash, SignedIntent, UnsignedIntent,
};
#[cfg(any(feature = "postgres", feature = "libsql"))]
use ironclaw_signing_provider::{GateRef, TenantId};

/// Schema shared by both backends.
///
/// The intent body rides as one JSON column: this crate must not re-model a
/// chain-specific transaction, and the authoritative decode already happened
/// before the intent was minted. `signature` is stored beside it so read-back
/// reconstructs through `SignedIntent::from_parts`.
#[cfg(any(feature = "postgres", feature = "libsql"))]
const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS attested_intents (
    tenant             TEXT NOT NULL,
    intent_id          TEXT NOT NULL,
    gate_ref           TEXT NOT NULL,
    review_token_hash  TEXT NOT NULL UNIQUE,
    state              TEXT NOT NULL,
    intent_json        TEXT NOT NULL,
    signature          TEXT NOT NULL,
    PRIMARY KEY (tenant, intent_id)
);";

/// The wire spelling of a lifecycle state. Explicit so a Rust-side rename
/// cannot silently orphan persisted rows.
#[cfg(any(feature = "postgres", feature = "libsql"))]
fn state_to_str(state: IntentState) -> &'static str {
    match state {
        IntentState::Pending => "pending",
        IntentState::Approved => "approved",
        IntentState::Rejected => "rejected",
        IntentState::Expired => "expired",
    }
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
fn state_from_str(raw: &str) -> Result<IntentState, IntentStoreError> {
    match raw {
        "pending" => Ok(IntentState::Pending),
        "approved" => Ok(IntentState::Approved),
        "rejected" => Ok(IntentState::Rejected),
        "expired" => Ok(IntentState::Expired),
        other => Err(IntentStoreError::Backend {
            reason: format!("unrecognized intent state {other:?}"),
        }),
    }
}

/// The columns a record writes, in schema order.
#[cfg(any(feature = "postgres", feature = "libsql"))]
struct Columns {
    tenant: String,
    intent_id: String,
    gate_ref: String,
    review_token_hash: String,
    state: &'static str,
    intent_json: String,
    signature: String,
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
fn columns_of(record: &IntentRecord) -> Result<Columns, IntentStoreError> {
    let intent_json = serde_json::to_string(record.intent.intent()).map_err(|error| {
        IntentStoreError::Backend {
            reason: format!("intent body is not serializable: {error}"),
        }
    })?;
    Ok(Columns {
        tenant: record.tenant().as_str().to_string(),
        intent_id: record.intent_id().as_str().to_string(),
        gate_ref: record.gate_ref.as_str().to_string(),
        review_token_hash: hex::encode(record.review_token_hash.as_bytes()),
        state: state_to_str(record.state),
        intent_json,
        signature: hex::encode(record.intent.signature()),
    })
}

/// Rebuild a record from its columns.
#[cfg(any(feature = "postgres", feature = "libsql"))]
fn record_from_columns(
    gate_ref: String,
    review_token_hash: String,
    state: String,
    intent_json: String,
    signature: String,
) -> Result<IntentRecord, IntentStoreError> {
    let intent: UnsignedIntent =
        serde_json::from_str(&intent_json).map_err(|error| IntentStoreError::Backend {
            reason: format!("stored intent body did not decode: {error}"),
        })?;

    let signature_bytes = hex::decode(&signature).map_err(|error| IntentStoreError::Backend {
        reason: format!("stored intent signature is not hex: {error}"),
    })?;
    let signature: [u8; INTENT_SIGNATURE_LEN] =
        signature_bytes
            .try_into()
            .map_err(|_| IntentStoreError::Backend {
                reason: format!("stored intent signature must be {INTENT_SIGNATURE_LEN} bytes"),
            })?;

    let token_bytes =
        hex::decode(&review_token_hash).map_err(|error| IntentStoreError::Backend {
            reason: format!("stored review token hash is not hex: {error}"),
        })?;
    let token_hash: [u8; 32] = token_bytes
        .try_into()
        .map_err(|_| IntentStoreError::Backend {
            reason: "stored review token hash must be 32 bytes".to_string(),
        })?;

    Ok(IntentRecord {
        intent: SignedIntent::from_parts(intent, signature),
        gate_ref: GateRef::new(gate_ref),
        review_token_hash: ReviewTokenHash::from_bytes(token_hash),
        state: state_from_str(&state)?,
    })
}

/// The `SELECT` list every read shares, in the order `record_from_columns`
/// expects.
#[cfg(any(feature = "postgres", feature = "libsql"))]
const SELECT_COLUMNS: &str = "gate_ref, review_token_hash, state, intent_json, signature";

// ---------------------------------------------------------------------------
// PostgreSQL
// ---------------------------------------------------------------------------

#[cfg(feature = "postgres")]
mod postgres {
    use super::*;
    use deadpool_postgres::Pool;

    /// Durable PostgreSQL [`IntentStore`].
    pub struct PostgresIntentStore {
        pool: Pool,
    }

    impl PostgresIntentStore {
        /// Wrap a connection pool (TLS/rustls is configured by the pool owner).
        pub fn new(pool: Pool) -> Self {
            Self { pool }
        }

        /// Create the table if absent. Idempotent.
        pub async fn run_migrations(&self) -> Result<(), IntentStoreError> {
            let client = self.client().await?;
            client
                .batch_execute(SCHEMA)
                .await
                .map_err(|error| backend(&error))?;
            Ok(())
        }

        async fn client(&self) -> Result<deadpool_postgres::Object, IntentStoreError> {
            self.pool.get().await.map_err(|error| backend(&error))
        }
    }

    fn backend(error: &dyn std::fmt::Display) -> IntentStoreError {
        IntentStoreError::Backend {
            reason: error.to_string(),
        }
    }

    fn row_to_record(row: &tokio_postgres::Row) -> Result<IntentRecord, IntentStoreError> {
        record_from_columns(row.get(0), row.get(1), row.get(2), row.get(3), row.get(4))
    }

    #[async_trait]
    impl IntentStore for PostgresIntentStore {
        async fn put(&self, record: IntentRecord) -> Result<(), IntentStoreError> {
            let client = self.client().await?;
            let columns = columns_of(&record)?;
            // The primary key doubles as the insert-only guard: a second write
            // could otherwise swap the transaction under an approved intent.
            let rows = client
                .execute(
                    "INSERT INTO attested_intents \
                     (tenant, intent_id, gate_ref, review_token_hash, state, intent_json, \
                      signature) \
                     VALUES ($1,$2,$3,$4,$5,$6,$7) \
                     ON CONFLICT (tenant, intent_id) DO NOTHING",
                    &[
                        &columns.tenant,
                        &columns.intent_id,
                        &columns.gate_ref,
                        &columns.review_token_hash,
                        &columns.state,
                        &columns.intent_json,
                        &columns.signature,
                    ],
                )
                .await
                .map_err(|error| backend(&error))?;
            if rows == 0 {
                return Err(IntentStoreError::AlreadyExists);
            }
            Ok(())
        }

        async fn get(
            &self,
            tenant: &TenantId,
            intent_id: &IntentId,
        ) -> Result<IntentRecord, IntentStoreError> {
            let client = self.client().await?;
            let row = client
                .query_opt(
                    &format!(
                        "SELECT {SELECT_COLUMNS} FROM attested_intents \
                          WHERE tenant = $1 AND intent_id = $2"
                    ),
                    &[&tenant.as_str(), &intent_id.as_str()],
                )
                .await
                .map_err(|error| backend(&error))?
                .ok_or(IntentStoreError::NotFound)?;
            row_to_record(&row)
        }

        async fn find_by_token_hash(
            &self,
            token_hash: &ReviewTokenHash,
        ) -> Result<IntentRecord, IntentStoreError> {
            let client = self.client().await?;
            let row = client
                .query_opt(
                    &format!(
                        "SELECT {SELECT_COLUMNS} FROM attested_intents \
                          WHERE review_token_hash = $1"
                    ),
                    &[&hex::encode(token_hash.as_bytes())],
                )
                .await
                .map_err(|error| backend(&error))?
                .ok_or(IntentStoreError::NotFound)?;
            row_to_record(&row)
        }

        async fn find_by_gate_ref(
            &self,
            tenant: &TenantId,
            gate_ref: &GateRef,
        ) -> Result<IntentRecord, IntentStoreError> {
            let client = self.client().await?;
            let row = client
                .query_opt(
                    &format!(
                        "SELECT {SELECT_COLUMNS} FROM attested_intents \
                          WHERE tenant = $1 AND gate_ref = $2"
                    ),
                    &[&tenant.as_str(), &gate_ref.as_str()],
                )
                .await
                .map_err(|error| backend(&error))?
                .ok_or(IntentStoreError::NotFound)?;
            row_to_record(&row)
        }

        async fn resolve(
            &self,
            tenant: &TenantId,
            intent_id: &IntentId,
            outcome: IntentState,
        ) -> Result<(), IntentStoreError> {
            let client = self.client().await?;
            // Conditional on 'pending': a late or duplicate outcome loses
            // rather than rewriting a decision that already landed.
            let rows = client
                .execute(
                    "UPDATE attested_intents SET state = $3 \
                      WHERE tenant = $1 AND intent_id = $2 AND state = 'pending'",
                    &[
                        &tenant.as_str(),
                        &intent_id.as_str(),
                        &state_to_str(outcome),
                    ],
                )
                .await
                .map_err(|error| backend(&error))?;
            if rows == 0 {
                // Either it is already terminal, or it is not there / not ours.
                return Err(match self.get(tenant, intent_id).await {
                    Ok(_) => IntentStoreError::AlreadyResolved,
                    Err(error) => error,
                });
            }
            Ok(())
        }
    }
}

#[cfg(feature = "postgres")]
pub use postgres::PostgresIntentStore;

// ---------------------------------------------------------------------------
// libSQL
// ---------------------------------------------------------------------------

#[cfg(feature = "libsql")]
mod libsql_backend {
    use super::*;
    use std::sync::Arc;

    /// Durable libSQL / Turso [`IntentStore`].
    pub struct LibSqlIntentStore {
        db: Arc<libsql::Database>,
    }

    impl LibSqlIntentStore {
        /// Wrap a libSQL database handle.
        pub fn new(db: Arc<libsql::Database>) -> Self {
            Self { db }
        }

        /// Create the table if absent. Idempotent.
        pub async fn run_migrations(&self) -> Result<(), IntentStoreError> {
            let conn = self.connect().await?;
            conn.execute_batch(SCHEMA)
                .await
                .map_err(|error| backend(&error))?;
            Ok(())
        }

        async fn connect(&self) -> Result<libsql::Connection, IntentStoreError> {
            let conn = self.db.connect().map_err(|error| backend(&error))?;
            conn.query("PRAGMA busy_timeout = 5000", ())
                .await
                .map_err(|error| backend(&error))?;
            Ok(conn)
        }

        async fn query_one(
            &self,
            sql: &str,
            params: impl libsql::params::IntoParams,
        ) -> Result<IntentRecord, IntentStoreError> {
            let conn = self.connect().await?;
            let mut rows = conn
                .query(sql, params)
                .await
                .map_err(|error| backend(&error))?;
            let row = rows
                .next()
                .await
                .map_err(|error| backend(&error))?
                .ok_or(IntentStoreError::NotFound)?;
            row_to_record(&row)
        }
    }

    fn backend(error: &dyn std::fmt::Display) -> IntentStoreError {
        IntentStoreError::Backend {
            reason: error.to_string(),
        }
    }

    fn row_to_record(row: &libsql::Row) -> Result<IntentRecord, IntentStoreError> {
        let gate_ref: String = row.get(0).map_err(|error| backend(&error))?;
        let review_token_hash: String = row.get(1).map_err(|error| backend(&error))?;
        let state: String = row.get(2).map_err(|error| backend(&error))?;
        let intent_json: String = row.get(3).map_err(|error| backend(&error))?;
        let signature: String = row.get(4).map_err(|error| backend(&error))?;
        record_from_columns(gate_ref, review_token_hash, state, intent_json, signature)
    }

    #[async_trait]
    impl IntentStore for LibSqlIntentStore {
        async fn put(&self, record: IntentRecord) -> Result<(), IntentStoreError> {
            let conn = self.connect().await?;
            let columns = columns_of(&record)?;
            let rows = conn
                .execute(
                    "INSERT OR IGNORE INTO attested_intents \
                     (tenant, intent_id, gate_ref, review_token_hash, state, intent_json, \
                      signature) \
                     VALUES (?1,?2,?3,?4,?5,?6,?7)",
                    libsql::params![
                        columns.tenant,
                        columns.intent_id,
                        columns.gate_ref,
                        columns.review_token_hash,
                        columns.state,
                        columns.intent_json,
                        columns.signature,
                    ],
                )
                .await
                .map_err(|error| backend(&error))?;
            if rows == 0 {
                return Err(IntentStoreError::AlreadyExists);
            }
            Ok(())
        }

        async fn get(
            &self,
            tenant: &TenantId,
            intent_id: &IntentId,
        ) -> Result<IntentRecord, IntentStoreError> {
            self.query_one(
                &format!(
                    "SELECT {SELECT_COLUMNS} FROM attested_intents \
                      WHERE tenant = ?1 AND intent_id = ?2"
                ),
                libsql::params![tenant.as_str(), intent_id.as_str()],
            )
            .await
        }

        async fn find_by_token_hash(
            &self,
            token_hash: &ReviewTokenHash,
        ) -> Result<IntentRecord, IntentStoreError> {
            self.query_one(
                &format!(
                    "SELECT {SELECT_COLUMNS} FROM attested_intents \
                      WHERE review_token_hash = ?1"
                ),
                libsql::params![hex::encode(token_hash.as_bytes())],
            )
            .await
        }

        async fn find_by_gate_ref(
            &self,
            tenant: &TenantId,
            gate_ref: &GateRef,
        ) -> Result<IntentRecord, IntentStoreError> {
            self.query_one(
                &format!(
                    "SELECT {SELECT_COLUMNS} FROM attested_intents \
                      WHERE tenant = ?1 AND gate_ref = ?2"
                ),
                libsql::params![tenant.as_str(), gate_ref.as_str()],
            )
            .await
        }

        async fn resolve(
            &self,
            tenant: &TenantId,
            intent_id: &IntentId,
            outcome: IntentState,
        ) -> Result<(), IntentStoreError> {
            let conn = self.connect().await?;
            let rows = conn
                .execute(
                    "UPDATE attested_intents SET state = ?3 \
                      WHERE tenant = ?1 AND intent_id = ?2 AND state = 'pending'",
                    libsql::params![tenant.as_str(), intent_id.as_str(), state_to_str(outcome),],
                )
                .await
                .map_err(|error| backend(&error))?;
            if rows == 0 {
                return Err(match self.get(tenant, intent_id).await {
                    Ok(_) => IntentStoreError::AlreadyResolved,
                    Err(error) => error,
                });
            }
            Ok(())
        }
    }
}

#[cfg(feature = "libsql")]
pub use libsql_backend::LibSqlIntentStore;
