//! Durable [`IdempotencyLedger`] implementations.

use chrono::{DateTime, Duration, Utc};

use crate::{
    ActionFingerprintKey, ActionPhase, IdempotencyDecision, ProductInboundAction,
    ProductWorkflowError,
};

const DEFAULT_IN_FLIGHT_LEASE: Duration = Duration::seconds(60);

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reborn_product_workflow_actions (
    adapter_id TEXT NOT NULL,
    installation_id TEXT NOT NULL,
    source_binding_key TEXT NOT NULL,
    external_event_id TEXT NOT NULL,
    action_id TEXT NOT NULL,
    phase TEXT NOT NULL,
    received_at TEXT NOT NULL,
    settled_at TEXT,
    payload TEXT NOT NULL,
    PRIMARY KEY (adapter_id, installation_id, source_binding_key, external_event_id)
);

CREATE INDEX IF NOT EXISTS idx_reborn_product_workflow_actions_phase
    ON reborn_product_workflow_actions(phase, received_at);
"#;

fn transient(reason: impl Into<String>) -> ProductWorkflowError {
    ProductWorkflowError::Transient {
        reason: reason.into(),
    }
}

fn durable_error(operation: &'static str, error: impl std::fmt::Display) -> ProductWorkflowError {
    transient(format!("idempotency ledger failed to {operation}: {error}"))
}

fn to_json(action: &ProductInboundAction) -> Result<String, ProductWorkflowError> {
    serde_json::to_string(action).map_err(|error| durable_error("serialize action", error))
}

fn from_json(payload: &str) -> Result<ProductInboundAction, ProductWorkflowError> {
    serde_json::from_str(payload).map_err(|error| durable_error("deserialize action", error))
}

fn phase_label(phase: ActionPhase) -> &'static str {
    match phase {
        ActionPhase::Received => "received",
        ActionPhase::Dispatched => "dispatched",
        ActionPhase::Settled => "settled",
        ActionPhase::DeduplicatedReplay => "deduplicated_replay",
    }
}

fn fresh_in_flight(
    action: &ProductInboundAction,
    received_at: DateTime<Utc>,
    lease: Duration,
) -> bool {
    !action.is_terminal() && action.received_at + lease > received_at
}

fn in_flight_error() -> ProductWorkflowError {
    transient("idempotency fingerprint already in flight; retry after recovery lease")
}

#[cfg(feature = "libsql")]
mod libsql_impl {
    use std::sync::Arc;

    use async_trait::async_trait;
    use libsql::params;

    use super::*;
    use crate::IdempotencyLedger;

    /// libSQL-backed product workflow idempotency ledger.
    pub struct RebornLibSqlIdempotencyLedger {
        db: Arc<libsql::Database>,
        in_flight_lease: Duration,
    }

    impl RebornLibSqlIdempotencyLedger {
        pub fn new(db: Arc<libsql::Database>) -> Self {
            Self::with_in_flight_lease(db, DEFAULT_IN_FLIGHT_LEASE)
        }

        pub fn with_in_flight_lease(db: Arc<libsql::Database>, in_flight_lease: Duration) -> Self {
            Self {
                db,
                in_flight_lease,
            }
        }

        pub async fn run_migrations(&self) -> Result<(), ProductWorkflowError> {
            let conn = self.connect().await?;
            conn.execute_batch(SCHEMA)
                .await
                .map(|_| ())
                .map_err(|error| durable_error("run migrations", error))
        }

        async fn connect(&self) -> Result<libsql::Connection, ProductWorkflowError> {
            let conn = self
                .db
                .connect()
                .map_err(|error| durable_error("connect", error))?;
            conn.query("PRAGMA busy_timeout = 5000", ())
                .await
                .map_err(|error| durable_error("configure busy timeout", error))?;
            Ok(conn)
        }

        async fn begin_immediate(&self) -> Result<libsql::Connection, ProductWorkflowError> {
            let conn = self.connect().await?;
            conn.execute("BEGIN IMMEDIATE", ())
                .await
                .map_err(|error| durable_error("begin transaction", error))?;
            Ok(conn)
        }
    }

    #[async_trait]
    impl IdempotencyLedger for RebornLibSqlIdempotencyLedger {
        async fn begin_or_replay(
            &self,
            fingerprint: ActionFingerprintKey,
            received_at: DateTime<Utc>,
        ) -> Result<IdempotencyDecision, ProductWorkflowError> {
            self.run_migrations().await?;
            let conn = self.begin_immediate().await?;
            let result =
                begin_or_replay_in_conn(&conn, fingerprint, received_at, self.in_flight_lease)
                    .await;
            finish_transaction(&conn, result).await
        }

        async fn settle(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
            self.run_migrations().await?;
            let conn = self.begin_immediate().await?;
            let result = settle_in_conn(&conn, action).await;
            finish_transaction(&conn, result).await
        }

        async fn release(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
            self.run_migrations().await?;
            let conn = self.begin_immediate().await?;
            let result = release_in_conn(&conn, action).await;
            finish_transaction(&conn, result).await
        }
    }

    async fn begin_or_replay_in_conn(
        conn: &libsql::Connection,
        fingerprint: ActionFingerprintKey,
        received_at: DateTime<Utc>,
        in_flight_lease: Duration,
    ) -> Result<IdempotencyDecision, ProductWorkflowError> {
        let action = ProductInboundAction::begin(fingerprint.clone(), received_at);
        let inserted = insert_action(conn, &action, "INSERT OR IGNORE").await?;
        if inserted == 1 {
            return Ok(IdempotencyDecision::New(action));
        }

        let Some(prior) = load_action(conn, &fingerprint).await? else {
            return Err(transient("idempotency ledger conflict row disappeared"));
        };
        if prior.is_terminal() {
            return Ok(IdempotencyDecision::Replay(prior));
        }
        if fresh_in_flight(&prior, received_at, in_flight_lease) {
            return Err(in_flight_error());
        }

        update_action(conn, &action).await?;
        Ok(IdempotencyDecision::New(action))
    }

    async fn settle_in_conn(
        conn: &libsql::Connection,
        action: ProductInboundAction,
    ) -> Result<(), ProductWorkflowError> {
        let Some(current) = load_action(conn, &action.fingerprint).await? else {
            return Err(transient(
                "idempotency reservation missing before terminal settle",
            ));
        };
        if current.is_terminal() {
            if current.action_id == action.action_id {
                return Ok(());
            }
            return Err(transient(
                "idempotency reservation was superseded before terminal settle",
            ));
        }
        if current.action_id != action.action_id {
            return Err(transient(
                "idempotency reservation was superseded before terminal settle",
            ));
        }
        update_action(conn, &action).await
    }

    async fn release_in_conn(
        conn: &libsql::Connection,
        action: ProductInboundAction,
    ) -> Result<(), ProductWorkflowError> {
        conn.execute(
            "DELETE FROM reborn_product_workflow_actions
             WHERE adapter_id = ?1
               AND installation_id = ?2
               AND source_binding_key = ?3
               AND external_event_id = ?4
               AND action_id = ?5
               AND phase NOT IN ('settled', 'deduplicated_replay')",
            params![
                action.fingerprint.adapter_id.as_str(),
                action.fingerprint.installation_id.as_str(),
                action.fingerprint.source_binding_key.as_str(),
                action.fingerprint.external_event_id.as_str(),
                action.action_id.to_string(),
            ],
        )
        .await
        .map_err(|error| durable_error("release action", error))?;
        Ok(())
    }

    async fn load_action(
        conn: &libsql::Connection,
        fingerprint: &ActionFingerprintKey,
    ) -> Result<Option<ProductInboundAction>, ProductWorkflowError> {
        let mut rows = conn
            .query(
                "SELECT payload FROM reborn_product_workflow_actions
                 WHERE adapter_id = ?1
                   AND installation_id = ?2
                   AND source_binding_key = ?3
                   AND external_event_id = ?4",
                params![
                    fingerprint.adapter_id.as_str(),
                    fingerprint.installation_id.as_str(),
                    fingerprint.source_binding_key.as_str(),
                    fingerprint.external_event_id.as_str(),
                ],
            )
            .await
            .map_err(|error| durable_error("load action", error))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|error| durable_error("read action", error))?
        else {
            return Ok(None);
        };
        let payload = row
            .get::<String>(0)
            .map_err(|error| durable_error("read action payload", error))?;
        Ok(Some(from_json(&payload)?))
    }

    async fn insert_action(
        conn: &libsql::Connection,
        action: &ProductInboundAction,
        insert_prefix: &str,
    ) -> Result<u64, ProductWorkflowError> {
        let payload = to_json(action)?;
        conn.execute(
            &format!(
                "{insert_prefix} INTO reborn_product_workflow_actions
                 (adapter_id, installation_id, source_binding_key, external_event_id,
                  action_id, phase, received_at, settled_at, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
            ),
            params![
                action.fingerprint.adapter_id.as_str(),
                action.fingerprint.installation_id.as_str(),
                action.fingerprint.source_binding_key.as_str(),
                action.fingerprint.external_event_id.as_str(),
                action.action_id.to_string(),
                phase_label(action.phase),
                action.received_at.to_rfc3339(),
                action.settled_at.map(|value| value.to_rfc3339()),
                payload,
            ],
        )
        .await
        .map_err(|error| durable_error("insert action", error))
    }

    async fn update_action(
        conn: &libsql::Connection,
        action: &ProductInboundAction,
    ) -> Result<(), ProductWorkflowError> {
        let payload = to_json(action)?;
        conn.execute(
            "UPDATE reborn_product_workflow_actions
             SET action_id = ?5, phase = ?6, received_at = ?7, settled_at = ?8, payload = ?9
             WHERE adapter_id = ?1
               AND installation_id = ?2
               AND source_binding_key = ?3
               AND external_event_id = ?4",
            params![
                action.fingerprint.adapter_id.as_str(),
                action.fingerprint.installation_id.as_str(),
                action.fingerprint.source_binding_key.as_str(),
                action.fingerprint.external_event_id.as_str(),
                action.action_id.to_string(),
                phase_label(action.phase),
                action.received_at.to_rfc3339(),
                action.settled_at.map(|value| value.to_rfc3339()),
                payload,
            ],
        )
        .await
        .map_err(|error| durable_error("update action", error))?;
        Ok(())
    }

    async fn finish_transaction<T>(
        conn: &libsql::Connection,
        result: Result<T, ProductWorkflowError>,
    ) -> Result<T, ProductWorkflowError> {
        match result {
            Ok(value) => {
                conn.execute("COMMIT", ())
                    .await
                    .map_err(|error| durable_error("commit transaction", error))?;
                Ok(value)
            }
            Err(error) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(error)
            }
        }
    }
}

#[cfg(feature = "postgres")]
mod postgres_impl {
    use async_trait::async_trait;

    use super::*;
    use crate::IdempotencyLedger;

    /// PostgreSQL-backed product workflow idempotency ledger.
    pub struct RebornPostgresIdempotencyLedger {
        pool: deadpool_postgres::Pool,
        in_flight_lease: Duration,
    }

    impl RebornPostgresIdempotencyLedger {
        pub fn new(pool: deadpool_postgres::Pool) -> Self {
            Self::with_in_flight_lease(pool, DEFAULT_IN_FLIGHT_LEASE)
        }

        pub fn with_in_flight_lease(
            pool: deadpool_postgres::Pool,
            in_flight_lease: Duration,
        ) -> Self {
            Self {
                pool,
                in_flight_lease,
            }
        }

        pub async fn run_migrations(&self) -> Result<(), ProductWorkflowError> {
            let client = self.client().await?;
            client
                .batch_execute(SCHEMA)
                .await
                .map_err(|error| durable_error("run migrations", error))
        }

        async fn client(&self) -> Result<deadpool_postgres::Object, ProductWorkflowError> {
            self.pool
                .get()
                .await
                .map_err(|error| durable_error("connect", error))
        }
    }

    #[async_trait]
    impl IdempotencyLedger for RebornPostgresIdempotencyLedger {
        async fn begin_or_replay(
            &self,
            fingerprint: ActionFingerprintKey,
            received_at: DateTime<Utc>,
        ) -> Result<IdempotencyDecision, ProductWorkflowError> {
            self.run_migrations().await?;
            let mut client = self.client().await?;
            let txn = client
                .transaction()
                .await
                .map_err(|error| durable_error("begin transaction", error))?;
            let result =
                begin_or_replay_in_txn(&txn, fingerprint, received_at, self.in_flight_lease).await;
            finish_transaction(txn, result).await
        }

        async fn settle(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
            self.run_migrations().await?;
            let mut client = self.client().await?;
            let txn = client
                .transaction()
                .await
                .map_err(|error| durable_error("begin transaction", error))?;
            let result = settle_in_txn(&txn, action).await;
            finish_transaction(txn, result).await
        }

        async fn release(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
            self.run_migrations().await?;
            let mut client = self.client().await?;
            let txn = client
                .transaction()
                .await
                .map_err(|error| durable_error("begin transaction", error))?;
            let result = release_in_txn(&txn, action).await;
            finish_transaction(txn, result).await
        }
    }

    async fn begin_or_replay_in_txn(
        txn: &deadpool_postgres::Transaction<'_>,
        fingerprint: ActionFingerprintKey,
        received_at: DateTime<Utc>,
        in_flight_lease: Duration,
    ) -> Result<IdempotencyDecision, ProductWorkflowError> {
        let action = ProductInboundAction::begin(fingerprint.clone(), received_at);
        let inserted = insert_action(txn, &action).await?;
        if inserted == 1 {
            return Ok(IdempotencyDecision::New(action));
        }

        let Some(prior) = load_action_for_update(txn, &fingerprint).await? else {
            return Err(transient("idempotency ledger conflict row disappeared"));
        };
        if prior.is_terminal() {
            return Ok(IdempotencyDecision::Replay(prior));
        }
        if fresh_in_flight(&prior, received_at, in_flight_lease) {
            return Err(in_flight_error());
        }

        update_action(txn, &action).await?;
        Ok(IdempotencyDecision::New(action))
    }

    async fn settle_in_txn(
        txn: &deadpool_postgres::Transaction<'_>,
        action: ProductInboundAction,
    ) -> Result<(), ProductWorkflowError> {
        let Some(current) = load_action_for_update(txn, &action.fingerprint).await? else {
            return Err(transient(
                "idempotency reservation missing before terminal settle",
            ));
        };
        if current.is_terminal() {
            if current.action_id == action.action_id {
                return Ok(());
            }
            return Err(transient(
                "idempotency reservation was superseded before terminal settle",
            ));
        }
        if current.action_id != action.action_id {
            return Err(transient(
                "idempotency reservation was superseded before terminal settle",
            ));
        }
        update_action(txn, &action).await
    }

    async fn release_in_txn(
        txn: &deadpool_postgres::Transaction<'_>,
        action: ProductInboundAction,
    ) -> Result<(), ProductWorkflowError> {
        txn.execute(
            "DELETE FROM reborn_product_workflow_actions
             WHERE adapter_id = $1
               AND installation_id = $2
               AND source_binding_key = $3
               AND external_event_id = $4
               AND action_id = $5
               AND phase NOT IN ('settled', 'deduplicated_replay')",
            &[
                &action.fingerprint.adapter_id.as_str(),
                &action.fingerprint.installation_id.as_str(),
                &action.fingerprint.source_binding_key.as_str(),
                &action.fingerprint.external_event_id.as_str(),
                &action.action_id.to_string(),
            ],
        )
        .await
        .map_err(|error| durable_error("release action", error))?;
        Ok(())
    }

    async fn load_action_for_update(
        txn: &deadpool_postgres::Transaction<'_>,
        fingerprint: &ActionFingerprintKey,
    ) -> Result<Option<ProductInboundAction>, ProductWorkflowError> {
        let row = txn
            .query_opt(
                "SELECT payload FROM reborn_product_workflow_actions
                 WHERE adapter_id = $1
                   AND installation_id = $2
                   AND source_binding_key = $3
                   AND external_event_id = $4
                 FOR UPDATE",
                &[
                    &fingerprint.adapter_id.as_str(),
                    &fingerprint.installation_id.as_str(),
                    &fingerprint.source_binding_key.as_str(),
                    &fingerprint.external_event_id.as_str(),
                ],
            )
            .await
            .map_err(|error| durable_error("load action", error))?;
        row.map(|row| from_json(row.get::<_, &str>(0))).transpose()
    }

    async fn insert_action(
        txn: &deadpool_postgres::Transaction<'_>,
        action: &ProductInboundAction,
    ) -> Result<u64, ProductWorkflowError> {
        let payload = to_json(action)?;
        txn.execute(
            "INSERT INTO reborn_product_workflow_actions
             (adapter_id, installation_id, source_binding_key, external_event_id,
              action_id, phase, received_at, settled_at, payload)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (adapter_id, installation_id, source_binding_key, external_event_id)
             DO NOTHING",
            &[
                &action.fingerprint.adapter_id.as_str(),
                &action.fingerprint.installation_id.as_str(),
                &action.fingerprint.source_binding_key.as_str(),
                &action.fingerprint.external_event_id.as_str(),
                &action.action_id.to_string(),
                &phase_label(action.phase),
                &action.received_at.to_rfc3339(),
                &action.settled_at.map(|value| value.to_rfc3339()),
                &payload,
            ],
        )
        .await
        .map_err(|error| durable_error("insert action", error))
    }

    async fn update_action(
        txn: &deadpool_postgres::Transaction<'_>,
        action: &ProductInboundAction,
    ) -> Result<(), ProductWorkflowError> {
        let payload = to_json(action)?;
        txn.execute(
            "UPDATE reborn_product_workflow_actions
             SET action_id = $5, phase = $6, received_at = $7, settled_at = $8, payload = $9
             WHERE adapter_id = $1
               AND installation_id = $2
               AND source_binding_key = $3
               AND external_event_id = $4",
            &[
                &action.fingerprint.adapter_id.as_str(),
                &action.fingerprint.installation_id.as_str(),
                &action.fingerprint.source_binding_key.as_str(),
                &action.fingerprint.external_event_id.as_str(),
                &action.action_id.to_string(),
                &phase_label(action.phase),
                &action.received_at.to_rfc3339(),
                &action.settled_at.map(|value| value.to_rfc3339()),
                &payload,
            ],
        )
        .await
        .map_err(|error| durable_error("update action", error))?;
        Ok(())
    }

    async fn finish_transaction<T>(
        txn: deadpool_postgres::Transaction<'_>,
        result: Result<T, ProductWorkflowError>,
    ) -> Result<T, ProductWorkflowError> {
        match result {
            Ok(value) => {
                txn.commit()
                    .await
                    .map_err(|error| durable_error("commit transaction", error))?;
                Ok(value)
            }
            Err(error) => {
                let _ = txn.rollback().await;
                Err(error)
            }
        }
    }
}

#[cfg(feature = "libsql")]
pub use libsql_impl::RebornLibSqlIdempotencyLedger;
#[cfg(feature = "postgres")]
pub use postgres_impl::RebornPostgresIdempotencyLedger;
