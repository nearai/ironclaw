use async_trait::async_trait;
#[cfg(feature = "libsql")]
use std::sync::Arc;

use crate::{
    CancelRunRequest, CancelRunResponse, GetRunStateRequest, InMemoryTurnStateStore,
    InMemoryTurnStateStoreLimits, ResumeTurnRequest, ResumeTurnResponse, SubmitTurnRequest,
    SubmitTurnResponse, TurnActiveLockRecord, TurnAdmissionPolicy, TurnCheckpointRecord, TurnError,
    TurnIdempotencyRecord, TurnPersistenceSnapshot, TurnRecord, TurnRunRecord, TurnRunState,
    TurnStateStore,
    runner::{
        BlockRunRequest, CancelRunCompletionRequest, ClaimRunRequest, ClaimedTurnRun,
        CompleteRunRequest, FailRunRequest, HeartbeatRequest, RecoverExpiredLeasesRequest,
        RecoverExpiredLeasesResponse, TurnRunTransitionPort,
    },
};

#[cfg(feature = "libsql")]
const LIBSQL_TURN_STATE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS turn_records (
    turn_id TEXT PRIMARY KEY,
    scope_key TEXT NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_turn_records_scope ON turn_records(scope_key);

CREATE TABLE IF NOT EXISTS turn_run_records (
    run_id TEXT PRIMARY KEY,
    turn_id TEXT NOT NULL,
    scope_key TEXT NOT NULL,
    status TEXT NOT NULL,
    event_cursor INTEGER NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_turn_run_records_scope ON turn_run_records(scope_key);
CREATE INDEX IF NOT EXISTS idx_turn_run_records_status ON turn_run_records(status);

CREATE TABLE IF NOT EXISTS turn_active_locks (
    scope_key TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    status TEXT NOT NULL,
    lock_version INTEGER NOT NULL,
    payload TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS turn_checkpoints (
    checkpoint_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_turn_checkpoints_run ON turn_checkpoints(run_id, sequence);

CREATE TABLE IF NOT EXISTS turn_idempotency_records (
    record_key TEXT PRIMARY KEY,
    scope_key TEXT NOT NULL,
    operation TEXT NOT NULL,
    run_id TEXT,
    idempotency_key TEXT NOT NULL,
    created_at TEXT NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_turn_idempotency_scope ON turn_idempotency_records(scope_key, operation);
"#;

#[cfg(feature = "postgres")]
const POSTGRES_TURN_STATE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS turn_records (
    turn_id TEXT PRIMARY KEY,
    scope_key TEXT NOT NULL,
    payload JSONB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_turn_records_scope ON turn_records(scope_key);

CREATE TABLE IF NOT EXISTS turn_run_records (
    run_id TEXT PRIMARY KEY,
    turn_id TEXT NOT NULL,
    scope_key TEXT NOT NULL,
    status TEXT NOT NULL,
    event_cursor BIGINT NOT NULL,
    payload JSONB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_turn_run_records_scope ON turn_run_records(scope_key);
CREATE INDEX IF NOT EXISTS idx_turn_run_records_status ON turn_run_records(status);

CREATE TABLE IF NOT EXISTS turn_active_locks (
    scope_key TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    status TEXT NOT NULL,
    lock_version BIGINT NOT NULL,
    payload JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS turn_checkpoints (
    checkpoint_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    sequence BIGINT NOT NULL,
    payload JSONB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_turn_checkpoints_run ON turn_checkpoints(run_id, sequence);

CREATE TABLE IF NOT EXISTS turn_idempotency_records (
    record_key TEXT PRIMARY KEY,
    scope_key TEXT NOT NULL,
    operation TEXT NOT NULL,
    run_id TEXT,
    idempotency_key TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    payload JSONB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_turn_idempotency_scope ON turn_idempotency_records(scope_key, operation);
"#;

#[cfg(feature = "libsql")]
pub struct LibSqlTurnStateStore {
    db: Arc<libsql::Database>,
    limits: InMemoryTurnStateStoreLimits,
}

#[cfg(feature = "libsql")]
impl LibSqlTurnStateStore {
    pub fn new(db: Arc<libsql::Database>) -> Self {
        Self {
            db,
            limits: InMemoryTurnStateStoreLimits::default(),
        }
    }

    pub fn with_limits(mut self, limits: InMemoryTurnStateStoreLimits) -> Self {
        self.limits = limits;
        self
    }

    pub async fn run_migrations(&self) -> Result<(), TurnError> {
        let conn = self.connect().await?;
        conn.execute_batch(LIBSQL_TURN_STATE_SCHEMA)
            .await
            .map_err(db_error)?;
        Ok(())
    }

    pub async fn persistence_snapshot(&self) -> Result<TurnPersistenceSnapshot, TurnError> {
        self.load_snapshot().await
    }

    async fn connect(&self) -> Result<libsql::Connection, TurnError> {
        let conn = self.db.connect().map_err(db_error)?;
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(db_error)?;
        Ok(conn)
    }

    async fn begin_immediate(&self) -> Result<libsql::Connection, TurnError> {
        let conn = self.connect().await?;
        conn.execute("BEGIN IMMEDIATE", ())
            .await
            .map_err(db_error)?;
        Ok(conn)
    }

    async fn load_store_from_conn(
        &self,
        conn: &libsql::Connection,
    ) -> Result<InMemoryTurnStateStore, TurnError> {
        InMemoryTurnStateStore::from_persistence_snapshot(
            libsql_load_snapshot(conn).await?,
            self.limits,
        )
    }

    async fn load_snapshot(&self) -> Result<TurnPersistenceSnapshot, TurnError> {
        let conn = self.connect().await?;
        conn.execute("BEGIN", ()).await.map_err(db_error)?;
        let result = libsql_load_snapshot(&conn).await;
        finish_libsql_transaction(&conn, result).await
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl TurnStateStore for LibSqlTurnStateStore {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let conn = self.begin_immediate().await?;
        let result = async {
            let store = self.load_store_from_conn(&conn).await?;
            let result = store.submit_turn(request, admission_policy).await;
            libsql_replace_snapshot(&conn, &store.persistence_snapshot()).await?;
            Ok(result)
        }
        .await;
        finish_libsql_transaction(&conn, result).await?
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        let conn = self.begin_immediate().await?;
        let result = async {
            let store = self.load_store_from_conn(&conn).await?;
            let result = store.resume_turn(request).await;
            libsql_replace_snapshot(&conn, &store.persistence_snapshot()).await?;
            Ok(result)
        }
        .await;
        finish_libsql_transaction(&conn, result).await?
    }

    async fn request_cancel(
        &self,
        request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        let conn = self.begin_immediate().await?;
        let result = async {
            let store = self.load_store_from_conn(&conn).await?;
            let result = store.request_cancel(request).await;
            libsql_replace_snapshot(&conn, &store.persistence_snapshot()).await?;
            Ok(result)
        }
        .await;
        finish_libsql_transaction(&conn, result).await?
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        self.load_snapshot()
            .await
            .and_then(|snapshot| {
                InMemoryTurnStateStore::from_persistence_snapshot(snapshot, self.limits)
            })?
            .get_run_state(request)
            .await
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl TurnRunTransitionPort for LibSqlTurnStateStore {
    async fn claim_next_run(
        &self,
        request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        let conn = self.begin_immediate().await?;
        let result = async {
            let store = self.load_store_from_conn(&conn).await?;
            let result = store.claim_next_run(request).await;
            libsql_replace_snapshot(&conn, &store.persistence_snapshot()).await?;
            Ok(result)
        }
        .await;
        finish_libsql_transaction(&conn, result).await?
    }

    async fn heartbeat(
        &self,
        request: HeartbeatRequest,
    ) -> Result<crate::events::EventCursor, TurnError> {
        let conn = self.begin_immediate().await?;
        let result = async {
            let store = self.load_store_from_conn(&conn).await?;
            let result = store.heartbeat(request).await;
            libsql_replace_snapshot(&conn, &store.persistence_snapshot()).await?;
            Ok(result)
        }
        .await;
        finish_libsql_transaction(&conn, result).await?
    }

    async fn recover_expired_leases(
        &self,
        request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        let conn = self.begin_immediate().await?;
        let result = async {
            let store = self.load_store_from_conn(&conn).await?;
            let result = store.recover_expired_leases(request).await;
            libsql_replace_snapshot(&conn, &store.persistence_snapshot()).await?;
            Ok(result)
        }
        .await;
        finish_libsql_transaction(&conn, result).await?
    }

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        let conn = self.begin_immediate().await?;
        let result = async {
            let store = self.load_store_from_conn(&conn).await?;
            let result = store.block_run(request).await;
            libsql_replace_snapshot(&conn, &store.persistence_snapshot()).await?;
            Ok(result)
        }
        .await;
        finish_libsql_transaction(&conn, result).await?
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        let conn = self.begin_immediate().await?;
        let result = async {
            let store = self.load_store_from_conn(&conn).await?;
            let result = store.complete_run(request).await;
            libsql_replace_snapshot(&conn, &store.persistence_snapshot()).await?;
            Ok(result)
        }
        .await;
        finish_libsql_transaction(&conn, result).await?
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        let conn = self.begin_immediate().await?;
        let result = async {
            let store = self.load_store_from_conn(&conn).await?;
            let result = store.cancel_run(request).await;
            libsql_replace_snapshot(&conn, &store.persistence_snapshot()).await?;
            Ok(result)
        }
        .await;
        finish_libsql_transaction(&conn, result).await?
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        let conn = self.begin_immediate().await?;
        let result = async {
            let store = self.load_store_from_conn(&conn).await?;
            let result = store.fail_run(request).await;
            libsql_replace_snapshot(&conn, &store.persistence_snapshot()).await?;
            Ok(result)
        }
        .await;
        finish_libsql_transaction(&conn, result).await?
    }
}

#[cfg(feature = "postgres")]
pub struct PostgresTurnStateStore {
    pool: deadpool_postgres::Pool,
    limits: InMemoryTurnStateStoreLimits,
}

#[cfg(feature = "postgres")]
impl PostgresTurnStateStore {
    pub fn new(pool: deadpool_postgres::Pool) -> Self {
        Self {
            pool,
            limits: InMemoryTurnStateStoreLimits::default(),
        }
    }

    pub fn with_limits(mut self, limits: InMemoryTurnStateStoreLimits) -> Self {
        self.limits = limits;
        self
    }

    pub async fn run_migrations(&self) -> Result<(), TurnError> {
        let client = self.client().await?;
        client
            .batch_execute(POSTGRES_TURN_STATE_SCHEMA)
            .await
            .map_err(db_error)?;
        Ok(())
    }

    pub async fn persistence_snapshot(&self) -> Result<TurnPersistenceSnapshot, TurnError> {
        self.load_snapshot().await
    }

    async fn client(&self) -> Result<deadpool_postgres::Object, TurnError> {
        self.pool.get().await.map_err(db_error)
    }

    async fn load_store_from_txn(
        &self,
        txn: &impl deadpool_postgres::GenericClient,
    ) -> Result<InMemoryTurnStateStore, TurnError> {
        InMemoryTurnStateStore::from_persistence_snapshot(
            postgres_load_snapshot(txn).await?,
            self.limits,
        )
    }

    async fn load_snapshot(&self) -> Result<TurnPersistenceSnapshot, TurnError> {
        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        lock_postgres_turn_tables(&txn, "SHARE MODE").await?;
        let snapshot = postgres_load_snapshot(&txn).await?;
        txn.commit().await.map_err(db_error)?;
        Ok(snapshot)
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl TurnStateStore for PostgresTurnStateStore {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        lock_postgres_turn_tables(&txn, "SHARE ROW EXCLUSIVE MODE").await?;
        let store = self.load_store_from_txn(&txn).await?;
        let result = store.submit_turn(request, admission_policy).await;
        postgres_replace_snapshot(&txn, &store.persistence_snapshot()).await?;
        txn.commit().await.map_err(db_error)?;
        result
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        lock_postgres_turn_tables(&txn, "SHARE ROW EXCLUSIVE MODE").await?;
        let store = self.load_store_from_txn(&txn).await?;
        let result = store.resume_turn(request).await;
        postgres_replace_snapshot(&txn, &store.persistence_snapshot()).await?;
        txn.commit().await.map_err(db_error)?;
        result
    }

    async fn request_cancel(
        &self,
        request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        lock_postgres_turn_tables(&txn, "SHARE ROW EXCLUSIVE MODE").await?;
        let store = self.load_store_from_txn(&txn).await?;
        let result = store.request_cancel(request).await;
        postgres_replace_snapshot(&txn, &store.persistence_snapshot()).await?;
        txn.commit().await.map_err(db_error)?;
        result
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        self.load_snapshot()
            .await
            .and_then(|snapshot| {
                InMemoryTurnStateStore::from_persistence_snapshot(snapshot, self.limits)
            })?
            .get_run_state(request)
            .await
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl TurnRunTransitionPort for PostgresTurnStateStore {
    async fn claim_next_run(
        &self,
        request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        lock_postgres_turn_tables(&txn, "SHARE ROW EXCLUSIVE MODE").await?;
        let store = self.load_store_from_txn(&txn).await?;
        let result = store.claim_next_run(request).await;
        postgres_replace_snapshot(&txn, &store.persistence_snapshot()).await?;
        txn.commit().await.map_err(db_error)?;
        result
    }

    async fn heartbeat(
        &self,
        request: HeartbeatRequest,
    ) -> Result<crate::events::EventCursor, TurnError> {
        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        lock_postgres_turn_tables(&txn, "SHARE ROW EXCLUSIVE MODE").await?;
        let store = self.load_store_from_txn(&txn).await?;
        let result = store.heartbeat(request).await;
        postgres_replace_snapshot(&txn, &store.persistence_snapshot()).await?;
        txn.commit().await.map_err(db_error)?;
        result
    }

    async fn recover_expired_leases(
        &self,
        request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        lock_postgres_turn_tables(&txn, "SHARE ROW EXCLUSIVE MODE").await?;
        let store = self.load_store_from_txn(&txn).await?;
        let result = store.recover_expired_leases(request).await;
        postgres_replace_snapshot(&txn, &store.persistence_snapshot()).await?;
        txn.commit().await.map_err(db_error)?;
        result
    }

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        lock_postgres_turn_tables(&txn, "SHARE ROW EXCLUSIVE MODE").await?;
        let store = self.load_store_from_txn(&txn).await?;
        let result = store.block_run(request).await;
        postgres_replace_snapshot(&txn, &store.persistence_snapshot()).await?;
        txn.commit().await.map_err(db_error)?;
        result
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        lock_postgres_turn_tables(&txn, "SHARE ROW EXCLUSIVE MODE").await?;
        let store = self.load_store_from_txn(&txn).await?;
        let result = store.complete_run(request).await;
        postgres_replace_snapshot(&txn, &store.persistence_snapshot()).await?;
        txn.commit().await.map_err(db_error)?;
        result
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        lock_postgres_turn_tables(&txn, "SHARE ROW EXCLUSIVE MODE").await?;
        let store = self.load_store_from_txn(&txn).await?;
        let result = store.cancel_run(request).await;
        postgres_replace_snapshot(&txn, &store.persistence_snapshot()).await?;
        txn.commit().await.map_err(db_error)?;
        result
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        lock_postgres_turn_tables(&txn, "SHARE ROW EXCLUSIVE MODE").await?;
        let store = self.load_store_from_txn(&txn).await?;
        let result = store.fail_run(request).await;
        postgres_replace_snapshot(&txn, &store.persistence_snapshot()).await?;
        txn.commit().await.map_err(db_error)?;
        result
    }
}

#[cfg(feature = "libsql")]
async fn libsql_load_payloads<T>(conn: &libsql::Connection, sql: &str) -> Result<Vec<T>, TurnError>
where
    T: serde::de::DeserializeOwned,
{
    let mut rows = conn.query(sql, ()).await.map_err(db_error)?;
    let mut payloads = Vec::new();
    while let Some(row) = rows.next().await.map_err(db_error)? {
        let payload: String = row.get(0).map_err(db_error)?;
        payloads.push(serde_json::from_str(&payload).map_err(db_error)?);
    }
    Ok(payloads)
}

#[cfg(feature = "libsql")]
async fn libsql_load_snapshot(
    conn: &libsql::Connection,
) -> Result<TurnPersistenceSnapshot, TurnError> {
    let turns = libsql_load_payloads::<TurnRecord>(
        conn,
        "SELECT payload FROM turn_records ORDER BY turn_id",
    )
    .await?;
    let runs = libsql_load_payloads::<TurnRunRecord>(
        conn,
        "SELECT payload FROM turn_run_records ORDER BY event_cursor, run_id",
    )
    .await?;
    let active_locks = libsql_load_payloads::<TurnActiveLockRecord>(
        conn,
        "SELECT payload FROM turn_active_locks ORDER BY scope_key",
    )
    .await?;
    let checkpoints = libsql_load_payloads::<TurnCheckpointRecord>(
        conn,
        "SELECT payload FROM turn_checkpoints ORDER BY run_id, sequence",
    )
    .await?;
    let idempotency_records = libsql_load_payloads::<TurnIdempotencyRecord>(
        conn,
        "SELECT payload FROM turn_idempotency_records ORDER BY created_at, record_key",
    )
    .await?;
    Ok(TurnPersistenceSnapshot {
        turns,
        runs,
        active_locks,
        checkpoints,
        idempotency_records,
    })
}

#[cfg(feature = "libsql")]
async fn finish_libsql_transaction<T>(
    conn: &libsql::Connection,
    result: Result<T, TurnError>,
) -> Result<T, TurnError> {
    match result {
        Ok(value) => {
            conn.execute("COMMIT", ()).await.map_err(db_error)?;
            Ok(value)
        }
        Err(error) => {
            let _ = conn.execute("ROLLBACK", ()).await;
            Err(error)
        }
    }
}

#[cfg(feature = "libsql")]
async fn libsql_replace_snapshot(
    conn: &libsql::Connection,
    snapshot: &TurnPersistenceSnapshot,
) -> Result<(), TurnError> {
    for table in [
        "turn_idempotency_records",
        "turn_checkpoints",
        "turn_active_locks",
        "turn_run_records",
        "turn_records",
    ] {
        let sql = format!("DELETE FROM {table}");
        conn.execute(sql.as_str(), ()).await.map_err(db_error)?;
    }

    for record in &snapshot.turns {
        conn.execute(
            "INSERT INTO turn_records (turn_id, scope_key, payload) VALUES (?1, ?2, ?3)",
            libsql::params![
                record.turn_id.to_string(),
                scope_key(&record.scope)?,
                to_json(record)?
            ],
        )
        .await
        .map_err(db_error)?;
    }
    for record in &snapshot.runs {
        conn.execute(
            "INSERT INTO turn_run_records (run_id, turn_id, scope_key, status, event_cursor, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            libsql::params![
                record.run_id.to_string(),
                record.turn_id.to_string(),
                scope_key(&record.scope)?,
                status_key(record.status)?,
                record.event_cursor.0 as i64,
                to_json(record)?,
            ],
        )
        .await
        .map_err(db_error)?;
    }
    for record in &snapshot.active_locks {
        conn.execute(
            "INSERT INTO turn_active_locks (scope_key, run_id, status, lock_version, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            libsql::params![
                scope_key(&record.key.scope)?,
                record.run_id.to_string(),
                status_key(record.status)?,
                record.lock_version.as_u64() as i64,
                to_json(record)?,
            ],
        )
        .await
        .map_err(db_error)?;
    }
    for record in &snapshot.checkpoints {
        conn.execute(
            "INSERT INTO turn_checkpoints (checkpoint_id, run_id, sequence, payload) VALUES (?1, ?2, ?3, ?4)",
            libsql::params![
                record.checkpoint_id.as_uuid().to_string(),
                record.run_id.to_string(),
                record.sequence as i64,
                to_json(record)?,
            ],
        )
        .await
        .map_err(db_error)?;
    }
    for record in &snapshot.idempotency_records {
        conn.execute(
            "INSERT INTO turn_idempotency_records (record_key, scope_key, operation, run_id, idempotency_key, created_at, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            libsql::params![
                idempotency_record_key(record)?,
                scope_key(&record.scope)?,
                operation_key(record)?,
                record.run_id.map(|run_id| run_id.to_string()),
                record.key.as_str(),
                record.created_at.to_rfc3339(),
                to_json(record)?,
            ],
        )
        .await
        .map_err(db_error)?;
    }
    Ok(())
}

#[cfg(feature = "postgres")]
async fn lock_postgres_turn_tables(
    client: &impl deadpool_postgres::GenericClient,
    mode: &str,
) -> Result<(), TurnError> {
    let statement = format!(
        "LOCK TABLE turn_records, turn_run_records, turn_active_locks, turn_checkpoints, turn_idempotency_records IN {mode}"
    );
    client.batch_execute(&statement).await.map_err(db_error)
}

#[cfg(feature = "postgres")]
async fn postgres_load_payloads<T>(
    client: &impl deadpool_postgres::GenericClient,
    sql: &str,
) -> Result<Vec<T>, TurnError>
where
    T: serde::de::DeserializeOwned,
{
    let rows = client.query(sql, &[]).await.map_err(db_error)?;
    rows.into_iter()
        .map(|row| {
            let payload: String = row.get(0);
            serde_json::from_str(&payload).map_err(db_error)
        })
        .collect()
}

#[cfg(feature = "postgres")]
async fn postgres_load_snapshot(
    client: &impl deadpool_postgres::GenericClient,
) -> Result<TurnPersistenceSnapshot, TurnError> {
    let turns = postgres_load_payloads::<TurnRecord>(
        client,
        "SELECT payload::text FROM turn_records ORDER BY turn_id",
    )
    .await?;
    let runs = postgres_load_payloads::<TurnRunRecord>(
        client,
        "SELECT payload::text FROM turn_run_records ORDER BY event_cursor, run_id",
    )
    .await?;
    let active_locks = postgres_load_payloads::<TurnActiveLockRecord>(
        client,
        "SELECT payload::text FROM turn_active_locks ORDER BY scope_key",
    )
    .await?;
    let checkpoints = postgres_load_payloads::<TurnCheckpointRecord>(
        client,
        "SELECT payload::text FROM turn_checkpoints ORDER BY run_id, sequence",
    )
    .await?;
    let idempotency_records = postgres_load_payloads::<TurnIdempotencyRecord>(
        client,
        "SELECT payload::text FROM turn_idempotency_records ORDER BY created_at, record_key",
    )
    .await?;
    Ok(TurnPersistenceSnapshot {
        turns,
        runs,
        active_locks,
        checkpoints,
        idempotency_records,
    })
}

#[cfg(feature = "postgres")]
async fn postgres_replace_snapshot(
    txn: &impl deadpool_postgres::GenericClient,
    snapshot: &TurnPersistenceSnapshot,
) -> Result<(), TurnError> {
    for table in [
        "turn_idempotency_records",
        "turn_checkpoints",
        "turn_active_locks",
        "turn_run_records",
        "turn_records",
    ] {
        let sql = format!("DELETE FROM {table}");
        txn.execute(sql.as_str(), &[]).await.map_err(db_error)?;
    }

    for record in &snapshot.turns {
        let payload = to_json(record)?;
        txn.execute(
            "INSERT INTO turn_records (turn_id, scope_key, payload) VALUES ($1, $2, $3::jsonb)",
            &[
                &record.turn_id.to_string(),
                &scope_key(&record.scope)?,
                &payload,
            ],
        )
        .await
        .map_err(db_error)?;
    }
    for record in &snapshot.runs {
        let payload = to_json(record)?;
        txn.execute(
            "INSERT INTO turn_run_records (run_id, turn_id, scope_key, status, event_cursor, payload) VALUES ($1, $2, $3, $4, $5, $6::jsonb)",
            &[
                &record.run_id.to_string(),
                &record.turn_id.to_string(),
                &scope_key(&record.scope)?,
                &status_key(record.status)?,
                &(record.event_cursor.0 as i64),
                &payload,
            ],
        )
        .await
        .map_err(db_error)?;
    }
    for record in &snapshot.active_locks {
        let payload = to_json(record)?;
        txn.execute(
            "INSERT INTO turn_active_locks (scope_key, run_id, status, lock_version, payload) VALUES ($1, $2, $3, $4, $5::jsonb)",
            &[
                &scope_key(&record.key.scope)?,
                &record.run_id.to_string(),
                &status_key(record.status)?,
                &(record.lock_version.as_u64() as i64),
                &payload,
            ],
        )
        .await
        .map_err(db_error)?;
    }
    for record in &snapshot.checkpoints {
        let payload = to_json(record)?;
        txn.execute(
            "INSERT INTO turn_checkpoints (checkpoint_id, run_id, sequence, payload) VALUES ($1, $2, $3, $4::jsonb)",
            &[
                &record.checkpoint_id.as_uuid().to_string(),
                &record.run_id.to_string(),
                &(record.sequence as i64),
                &payload,
            ],
        )
        .await
        .map_err(db_error)?;
    }
    for record in &snapshot.idempotency_records {
        let payload = to_json(record)?;
        txn.execute(
            "INSERT INTO turn_idempotency_records (record_key, scope_key, operation, run_id, idempotency_key, created_at, payload) VALUES ($1, $2, $3, $4, $5, $6::timestamptz, $7::jsonb)",
            &[
                &idempotency_record_key(record)?,
                &scope_key(&record.scope)?,
                &operation_key(record)?,
                &record.run_id.map(|run_id| run_id.to_string()),
                &record.key.as_str(),
                &record.created_at.to_rfc3339(),
                &payload,
            ],
        )
        .await
        .map_err(db_error)?;
    }
    Ok(())
}

fn to_json<T>(value: &T) -> Result<String, TurnError>
where
    T: serde::Serialize,
{
    serde_json::to_string(value).map_err(db_error)
}

fn scope_key(scope: &crate::TurnScope) -> Result<String, TurnError> {
    to_json(scope)
}

fn status_key(status: crate::TurnStatus) -> Result<String, TurnError> {
    to_json(&status)
}

fn operation_key(record: &TurnIdempotencyRecord) -> Result<String, TurnError> {
    to_json(&record.operation)
}

fn idempotency_record_key(record: &TurnIdempotencyRecord) -> Result<String, TurnError> {
    #[derive(serde::Serialize)]
    struct IdempotencyRecordKey<'a> {
        scope: &'a crate::TurnScope,
        operation: crate::TurnIdempotencyOperationKind,
        run_id: Option<String>,
        key: &'a str,
    }

    to_json(&IdempotencyRecordKey {
        scope: &record.scope,
        operation: record.operation,
        run_id: record.run_id.map(|run_id| run_id.to_string()),
        key: record.key.as_str(),
    })
}

fn db_error(error: impl std::fmt::Display) -> TurnError {
    tracing::debug!(%error, "turn state persistence operation failed");
    TurnError::Unavailable {
        reason: "turn state persistence temporarily unavailable".to_string(),
    }
}
