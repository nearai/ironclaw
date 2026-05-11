use async_trait::async_trait;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use chrono::Utc;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use std::sync::Arc;

use crate::{
    AllowAllTurnAdmissionLimitProvider, CancelRunRequest, CancelRunResponse,
    GetLoopCheckpointRequest, GetRunStateRequest, InMemoryTurnStateStore,
    InMemoryTurnStateStoreLimits, LoopCheckpointRecord, LoopCheckpointStore,
    PutLoopCheckpointRequest, ResolvedRunProfile, ResumeTurnRequest, ResumeTurnResponse,
    RunProfileResolutionError, RunProfileResolutionRequest, RunProfileResolver, SubmitTurnRequest,
    SubmitTurnResponse, TurnActiveLockRecord, TurnAdmissionLimitProvider, TurnAdmissionPolicy,
    TurnAdmissionReservationRecord, TurnCheckpointRecord, TurnError, TurnIdempotencyRecord,
    TurnLifecycleEvent, TurnPersistenceSnapshot, TurnRecord, TurnRunRecord, TurnRunState,
    TurnScope, TurnStateStore,
    events::{EventCursor, TurnEventPage, TurnEventProjectionSource, project_turn_events},
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
        RecordRecoveryRequiredRequest, RecoverExpiredLeasesRequest, RecoverExpiredLeasesResponse,
        TurnRunTransitionPort,
    },
};

struct PreResolvedRunProfileResolver {
    result: Result<ResolvedRunProfile, RunProfileResolutionError>,
}

impl PreResolvedRunProfileResolver {
    fn new(result: Result<ResolvedRunProfile, RunProfileResolutionError>) -> Self {
        Self { result }
    }
}

#[async_trait]
impl RunProfileResolver for PreResolvedRunProfileResolver {
    async fn resolve_run_profile(
        &self,
        _request: RunProfileResolutionRequest,
    ) -> Result<ResolvedRunProfile, RunProfileResolutionError> {
        self.result.clone()
    }
}

async fn resolve_run_profile_for_submit(
    request: &SubmitTurnRequest,
    resolver: &dyn RunProfileResolver,
) -> Result<ResolvedRunProfile, RunProfileResolutionError> {
    resolver
        .resolve_run_profile(RunProfileResolutionRequest {
            requested_run_profile: request.requested_run_profile.clone(),
            ..RunProfileResolutionRequest::interactive_default()
        })
        .await
}

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
    scope_key TEXT NOT NULL DEFAULT '',
    kind TEXT NOT NULL DEFAULT 'before_block',
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_turn_checkpoints_run ON turn_checkpoints(run_id, sequence);

CREATE TABLE IF NOT EXISTS turn_loop_checkpoints (
    checkpoint_id TEXT PRIMARY KEY,
    scope_key TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_turn_loop_checkpoints_run ON turn_loop_checkpoints(scope_key, turn_id, run_id);

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

CREATE TABLE IF NOT EXISTS turn_lifecycle_events (
    event_key TEXT PRIMARY KEY,
    scope_key TEXT NOT NULL,
    run_id TEXT NOT NULL,
    event_cursor INTEGER NOT NULL,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_turn_events_scope_cursor ON turn_lifecycle_events(scope_key, event_cursor);

CREATE TABLE IF NOT EXISTS turn_store_metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS turn_admission_reservations (
    run_id TEXT PRIMARY KEY,
    released INTEGER NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_turn_admission_reservations_released ON turn_admission_reservations(released);
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
    scope_key TEXT NOT NULL DEFAULT '',
    kind TEXT NOT NULL DEFAULT 'before_block',
    payload JSONB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_turn_checkpoints_run ON turn_checkpoints(run_id, sequence);

CREATE TABLE IF NOT EXISTS turn_loop_checkpoints (
    checkpoint_id TEXT PRIMARY KEY,
    scope_key TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    payload JSONB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_turn_loop_checkpoints_run ON turn_loop_checkpoints(scope_key, turn_id, run_id);

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

CREATE TABLE IF NOT EXISTS turn_lifecycle_events (
    event_key TEXT PRIMARY KEY,
    scope_key TEXT NOT NULL,
    run_id TEXT NOT NULL,
    event_cursor BIGINT NOT NULL,
    kind TEXT NOT NULL,
    payload JSONB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_turn_events_scope_cursor ON turn_lifecycle_events(scope_key, event_cursor);

CREATE TABLE IF NOT EXISTS turn_store_metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS turn_admission_reservations (
    run_id TEXT PRIMARY KEY,
    released BOOLEAN NOT NULL,
    payload JSONB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_turn_admission_reservations_released ON turn_admission_reservations(released);
"#;

#[cfg(feature = "libsql")]
pub struct LibSqlTurnStateStore {
    db: Arc<libsql::Database>,
    limits: InMemoryTurnStateStoreLimits,
    admission_limit_provider: Arc<dyn TurnAdmissionLimitProvider>,
}

#[cfg(feature = "libsql")]
impl LibSqlTurnStateStore {
    pub fn new(db: Arc<libsql::Database>) -> Self {
        Self {
            db,
            limits: InMemoryTurnStateStoreLimits::default(),
            admission_limit_provider: Arc::new(AllowAllTurnAdmissionLimitProvider),
        }
    }

    pub fn with_limits(mut self, limits: InMemoryTurnStateStoreLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn with_admission_limit_provider(
        mut self,
        admission_limit_provider: Arc<dyn TurnAdmissionLimitProvider>,
    ) -> Self {
        self.admission_limit_provider = admission_limit_provider;
        self
    }

    pub async fn run_migrations(&self) -> Result<(), TurnError> {
        let conn = self.connect().await?;
        conn.execute_batch(LIBSQL_TURN_STATE_SCHEMA)
            .await
            .map_err(db_error)?;

        // Migration: add new columns to existing turn_checkpoints tables.
        // For libSQL, ALTER TABLE ADD COLUMN fails if the column already exists,
        // so we ignore "duplicate column name" errors.
        for alter in [
            "ALTER TABLE turn_checkpoints ADD COLUMN scope_key TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE turn_checkpoints ADD COLUMN kind TEXT NOT NULL DEFAULT 'before_block'",
        ] {
            match conn.execute(alter, ()).await {
                Ok(_) => {}
                Err(e) if e.to_string().contains("duplicate column name") => {}
                Err(e) => return Err(db_error(e)),
            }
        }
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
        InMemoryTurnStateStore::from_persistence_snapshot_with_admission_limit_provider(
            libsql_load_snapshot(conn).await?,
            self.limits,
            self.admission_limit_provider.clone(),
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
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let profile_resolution =
            resolve_run_profile_for_submit(&request, run_profile_resolver).await;
        let pre_resolved = PreResolvedRunProfileResolver::new(profile_resolution);
        let conn = self.begin_immediate().await?;
        let result = async {
            let store = self.load_store_from_conn(&conn).await?;
            let result = store
                .submit_turn(request, admission_policy, &pre_resolved)
                .await;
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
impl TurnEventProjectionSource for LibSqlTurnStateStore {
    async fn read_turn_events_after(
        &self,
        scope: &TurnScope,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        let snapshot = self.load_snapshot().await?;
        Ok(project_turn_events(
            &snapshot.events,
            scope,
            after,
            limit,
            snapshot.event_retention_floor,
        ))
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl LoopCheckpointStore for LibSqlTurnStateStore {
    async fn put_loop_checkpoint(
        &self,
        request: PutLoopCheckpointRequest,
    ) -> Result<LoopCheckpointRecord, TurnError> {
        let conn = self.begin_immediate().await?;
        let record = LoopCheckpointRecord {
            checkpoint_id: crate::TurnCheckpointId::new(),
            scope: request.scope,
            turn_id: request.turn_id,
            run_id: request.run_id,
            state_ref: request.state_ref,
            schema_id: request.schema_id,
            schema_version: request.schema_version,
            kind: request.kind,
            created_at: Utc::now(),
        };
        let result = libsql_insert_loop_checkpoint_record(&conn, &record)
            .await
            .map(|()| record.clone());
        finish_libsql_transaction(&conn, result).await
    }

    async fn get_loop_checkpoint(
        &self,
        request: GetLoopCheckpointRequest,
    ) -> Result<Option<LoopCheckpointRecord>, TurnError> {
        let conn = self.connect().await?;
        let scope_key = scope_key(&request.scope)?;
        let mut rows = conn
            .query(
                "SELECT payload FROM turn_loop_checkpoints WHERE checkpoint_id = ?1 AND scope_key = ?2 AND turn_id = ?3 AND run_id = ?4",
                libsql::params![
                    request.checkpoint_id.as_uuid().to_string(),
                    scope_key,
                    request.turn_id.to_string(),
                    request.run_id.to_string(),
                ],
            )
            .await
            .map_err(db_error)?;
        let Some(row) = rows.next().await.map_err(db_error)? else {
            return Ok(None);
        };
        let payload: String = row.get(0).map_err(db_error)?;
        let record = serde_json::from_str(&payload).map_err(db_error)?;
        if loop_checkpoint_record_matches_request(&record, &request) {
            Ok(Some(record))
        } else {
            Ok(None)
        }
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

    async fn record_recovery_required(
        &self,
        request: RecordRecoveryRequiredRequest,
    ) -> Result<TurnRunState, TurnError> {
        let conn = self.begin_immediate().await?;
        let result = async {
            let store = self.load_store_from_conn(&conn).await?;
            let result = store.record_recovery_required(request).await;
            libsql_replace_snapshot(&conn, &store.persistence_snapshot()).await?;
            Ok(result)
        }
        .await;
        finish_libsql_transaction(&conn, result).await?
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        let conn = self.begin_immediate().await?;
        let result = async {
            let store = self.load_store_from_conn(&conn).await?;
            let result = store.apply_validated_loop_exit(request).await;
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
    admission_limit_provider: Arc<dyn TurnAdmissionLimitProvider>,
}

#[cfg(feature = "postgres")]
impl PostgresTurnStateStore {
    pub fn new(pool: deadpool_postgres::Pool) -> Self {
        Self {
            pool,
            limits: InMemoryTurnStateStoreLimits::default(),
            admission_limit_provider: Arc::new(AllowAllTurnAdmissionLimitProvider),
        }
    }

    pub fn with_limits(mut self, limits: InMemoryTurnStateStoreLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn with_admission_limit_provider(
        mut self,
        admission_limit_provider: Arc<dyn TurnAdmissionLimitProvider>,
    ) -> Self {
        self.admission_limit_provider = admission_limit_provider;
        self
    }

    pub async fn run_migrations(&self) -> Result<(), TurnError> {
        let client = self.client().await?;
        client
            .batch_execute(POSTGRES_TURN_STATE_SCHEMA)
            .await
            .map_err(db_error)?;

        // Migration: add new columns to existing turn_checkpoints tables.
        // Postgres supports ADD COLUMN IF NOT EXISTS natively.
        client
            .batch_execute(
                "ALTER TABLE turn_checkpoints ADD COLUMN IF NOT EXISTS scope_key TEXT NOT NULL DEFAULT '';
                 ALTER TABLE turn_checkpoints ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'before_block';",
            )
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
        InMemoryTurnStateStore::from_persistence_snapshot_with_admission_limit_provider(
            postgres_load_snapshot(txn).await?,
            self.limits,
            self.admission_limit_provider.clone(),
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
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let profile_resolution =
            resolve_run_profile_for_submit(&request, run_profile_resolver).await;
        let pre_resolved = PreResolvedRunProfileResolver::new(profile_resolution);
        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        lock_postgres_turn_tables(&txn, "SHARE ROW EXCLUSIVE MODE").await?;
        let store = self.load_store_from_txn(&txn).await?;
        let result = store
            .submit_turn(request, admission_policy, &pre_resolved)
            .await;
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
impl TurnEventProjectionSource for PostgresTurnStateStore {
    async fn read_turn_events_after(
        &self,
        scope: &TurnScope,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        let snapshot = self.load_snapshot().await?;
        Ok(project_turn_events(
            &snapshot.events,
            scope,
            after,
            limit,
            snapshot.event_retention_floor,
        ))
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl LoopCheckpointStore for PostgresTurnStateStore {
    async fn put_loop_checkpoint(
        &self,
        request: PutLoopCheckpointRequest,
    ) -> Result<LoopCheckpointRecord, TurnError> {
        let client = self.client().await?;
        let record = LoopCheckpointRecord {
            checkpoint_id: crate::TurnCheckpointId::new(),
            scope: request.scope,
            turn_id: request.turn_id,
            run_id: request.run_id,
            state_ref: request.state_ref,
            schema_id: request.schema_id,
            schema_version: request.schema_version,
            kind: request.kind,
            created_at: Utc::now(),
        };
        postgres_insert_loop_checkpoint_record(&client, &record).await?;
        Ok(record)
    }

    async fn get_loop_checkpoint(
        &self,
        request: GetLoopCheckpointRequest,
    ) -> Result<Option<LoopCheckpointRecord>, TurnError> {
        let client = self.client().await?;
        let scope_key = scope_key(&request.scope)?;
        let row = client
            .query_opt(
                "SELECT payload::text FROM turn_loop_checkpoints WHERE checkpoint_id = $1 AND scope_key = $2 AND turn_id = $3 AND run_id = $4",
                &[
                    &request.checkpoint_id.as_uuid().to_string(),
                    &scope_key,
                    &request.turn_id.to_string(),
                    &request.run_id.to_string(),
                ],
            )
            .await
            .map_err(db_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let payload: String = row.get(0);
        let record = serde_json::from_str(&payload).map_err(db_error)?;
        if loop_checkpoint_record_matches_request(&record, &request) {
            Ok(Some(record))
        } else {
            Ok(None)
        }
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

    async fn record_recovery_required(
        &self,
        request: RecordRecoveryRequiredRequest,
    ) -> Result<TurnRunState, TurnError> {
        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        lock_postgres_turn_tables(&txn, "SHARE ROW EXCLUSIVE MODE").await?;
        let store = self.load_store_from_txn(&txn).await?;
        let result = store.record_recovery_required(request).await;
        postgres_replace_snapshot(&txn, &store.persistence_snapshot()).await?;
        txn.commit().await.map_err(db_error)?;
        result
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        lock_postgres_turn_tables(&txn, "SHARE ROW EXCLUSIVE MODE").await?;
        let store = self.load_store_from_txn(&txn).await?;
        let result = store.apply_validated_loop_exit(request).await;
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
async fn libsql_insert_loop_checkpoint_record(
    conn: &libsql::Connection,
    record: &LoopCheckpointRecord,
) -> Result<(), TurnError> {
    let rows = conn
        .execute(
            "INSERT OR IGNORE INTO turn_loop_checkpoints (checkpoint_id, scope_key, turn_id, run_id, created_at, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            libsql::params![
                record.checkpoint_id.as_uuid().to_string(),
                scope_key(&record.scope)?,
                record.turn_id.to_string(),
                record.run_id.to_string(),
                record.created_at.to_rfc3339(),
                to_json(record)?,
            ],
        )
        .await
        .map_err(db_error)?;
    if rows == 1 {
        return Ok(());
    }

    let mut rows = conn
        .query(
            "SELECT payload FROM turn_loop_checkpoints WHERE checkpoint_id = ?1",
            libsql::params![record.checkpoint_id.as_uuid().to_string()],
        )
        .await
        .map_err(db_error)?;
    let Some(row) = rows.next().await.map_err(db_error)? else {
        return Err(TurnError::Conflict {
            reason: "loop checkpoint id insert conflicted but existing row was not readable"
                .to_string(),
        });
    };
    let payload: String = row.get(0).map_err(db_error)?;
    let existing: LoopCheckpointRecord = serde_json::from_str(&payload).map_err(db_error)?;
    ensure_loop_checkpoint_insert_is_idempotent(&existing, record)
}

#[cfg(feature = "libsql")]
async fn libsql_load_event_retention_floor(
    conn: &libsql::Connection,
) -> Result<EventCursor, TurnError> {
    let mut rows = conn
        .query(
            "SELECT value FROM turn_store_metadata WHERE key = 'event_retention_floor'",
            (),
        )
        .await
        .map_err(db_error)?;
    let Some(row) = rows.next().await.map_err(db_error)? else {
        return Ok(EventCursor::default());
    };
    let value: String = row.get(0).map_err(db_error)?;
    value.parse::<u64>().map(EventCursor).map_err(db_error)
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
    let loop_checkpoints = libsql_load_payloads::<LoopCheckpointRecord>(
        conn,
        "SELECT payload FROM turn_loop_checkpoints ORDER BY created_at, checkpoint_id",
    )
    .await?;
    let idempotency_records = libsql_load_payloads::<TurnIdempotencyRecord>(
        conn,
        "SELECT payload FROM turn_idempotency_records ORDER BY created_at, record_key",
    )
    .await?;
    let events = libsql_load_payloads::<TurnLifecycleEvent>(
        conn,
        "SELECT payload FROM turn_lifecycle_events ORDER BY event_cursor, event_key",
    )
    .await?;
    let event_retention_floor = libsql_load_event_retention_floor(conn).await?;
    let admission_reservations = libsql_load_payloads::<TurnAdmissionReservationRecord>(
        conn,
        "SELECT payload FROM turn_admission_reservations ORDER BY run_id",
    )
    .await?;
    Ok(TurnPersistenceSnapshot {
        turns,
        runs,
        active_locks,
        checkpoints,
        loop_checkpoints,
        idempotency_records,
        events,
        event_retention_floor,
        admission_reservations,
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
        "turn_store_metadata",
        "turn_lifecycle_events",
        "turn_admission_reservations",
        "turn_idempotency_records",
        "turn_loop_checkpoints",
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
            "INSERT INTO turn_checkpoints (checkpoint_id, run_id, sequence, scope_key, kind, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            libsql::params![
                record.checkpoint_id.as_uuid().to_string(),
                record.run_id.to_string(),
                record.sequence as i64,
                record.scope.as_ref().map(scope_key).transpose()?.unwrap_or_default(),
                record.kind.as_str(),
                to_json(record)?,
            ],
        )
        .await
        .map_err(db_error)?;
    }
    for record in &snapshot.loop_checkpoints {
        conn.execute(
            "INSERT INTO turn_loop_checkpoints (checkpoint_id, scope_key, turn_id, run_id, created_at, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            libsql::params![
                record.checkpoint_id.as_uuid().to_string(),
                scope_key(&record.scope)?,
                record.turn_id.to_string(),
                record.run_id.to_string(),
                record.created_at.to_rfc3339(),
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
    for event in &snapshot.events {
        conn.execute(
            "INSERT INTO turn_lifecycle_events (event_key, scope_key, run_id, event_cursor, kind, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            libsql::params![
                turn_event_key(event)?,
                scope_key(&event.scope)?,
                event.run_id.to_string(),
                event.cursor.0 as i64,
                turn_event_kind_key(event)?,
                to_json(event)?,
            ],
        )
        .await
        .map_err(db_error)?;
    }
    for record in &snapshot.admission_reservations {
        conn.execute(
            "INSERT INTO turn_admission_reservations (run_id, released, payload) VALUES (?1, ?2, ?3)",
            libsql::params![
                record.run_id.to_string(),
                if record.released { 1_i64 } else { 0_i64 },
                to_json(record)?,
            ],
        )
        .await
        .map_err(db_error)?;
    }
    conn.execute(
        "INSERT INTO turn_store_metadata (key, value) VALUES ('event_retention_floor', ?1)",
        libsql::params![snapshot.event_retention_floor.0.to_string()],
    )
    .await
    .map_err(db_error)?;
    Ok(())
}

#[cfg(feature = "postgres")]
async fn lock_postgres_turn_tables(
    client: &impl deadpool_postgres::GenericClient,
    mode: &str,
) -> Result<(), TurnError> {
    let statement = format!(
        "LOCK TABLE turn_records, turn_run_records, turn_active_locks, turn_checkpoints, turn_loop_checkpoints, turn_idempotency_records, turn_lifecycle_events, turn_store_metadata, turn_admission_reservations IN {mode}"
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
async fn postgres_insert_loop_checkpoint_record(
    client: &impl deadpool_postgres::GenericClient,
    record: &LoopCheckpointRecord,
) -> Result<(), TurnError> {
    let payload = to_json(record)?;
    let rows = client
        .execute(
            "INSERT INTO turn_loop_checkpoints (checkpoint_id, scope_key, turn_id, run_id, created_at, payload)
             VALUES ($1, $2, $3, $4, $5::timestamptz, $6::jsonb)
             ON CONFLICT (checkpoint_id) DO NOTHING",
            &[
                &record.checkpoint_id.as_uuid().to_string(),
                &scope_key(&record.scope)?,
                &record.turn_id.to_string(),
                &record.run_id.to_string(),
                &record.created_at.to_rfc3339(),
                &payload,
            ],
        )
        .await
        .map_err(db_error)?;
    if rows == 1 {
        return Ok(());
    }

    let row = client
        .query_opt(
            "SELECT payload::text FROM turn_loop_checkpoints WHERE checkpoint_id = $1",
            &[&record.checkpoint_id.as_uuid().to_string()],
        )
        .await
        .map_err(db_error)?;
    let Some(row) = row else {
        return Err(TurnError::Conflict {
            reason: "loop checkpoint id insert conflicted but existing row was not readable"
                .to_string(),
        });
    };
    let payload: String = row.get(0);
    let existing: LoopCheckpointRecord = serde_json::from_str(&payload).map_err(db_error)?;
    ensure_loop_checkpoint_insert_is_idempotent(&existing, record)
}

#[cfg(feature = "postgres")]
async fn postgres_load_event_retention_floor(
    client: &impl deadpool_postgres::GenericClient,
) -> Result<EventCursor, TurnError> {
    let Some(row) = client
        .query_opt(
            "SELECT value FROM turn_store_metadata WHERE key = 'event_retention_floor'",
            &[],
        )
        .await
        .map_err(db_error)?
    else {
        return Ok(EventCursor::default());
    };
    let value: String = row.get(0);
    value.parse::<u64>().map(EventCursor).map_err(db_error)
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
    let loop_checkpoints = postgres_load_payloads::<LoopCheckpointRecord>(
        client,
        "SELECT payload::text FROM turn_loop_checkpoints ORDER BY created_at, checkpoint_id",
    )
    .await?;
    let idempotency_records = postgres_load_payloads::<TurnIdempotencyRecord>(
        client,
        "SELECT payload::text FROM turn_idempotency_records ORDER BY created_at, record_key",
    )
    .await?;
    let events = postgres_load_payloads::<TurnLifecycleEvent>(
        client,
        "SELECT payload::text FROM turn_lifecycle_events ORDER BY event_cursor, event_key",
    )
    .await?;
    let event_retention_floor = postgres_load_event_retention_floor(client).await?;
    let admission_reservations = postgres_load_payloads::<TurnAdmissionReservationRecord>(
        client,
        "SELECT payload::text FROM turn_admission_reservations ORDER BY run_id",
    )
    .await?;
    Ok(TurnPersistenceSnapshot {
        turns,
        runs,
        active_locks,
        checkpoints,
        loop_checkpoints,
        idempotency_records,
        events,
        event_retention_floor,
        admission_reservations,
    })
}

#[cfg(feature = "postgres")]
async fn postgres_replace_snapshot(
    txn: &impl deadpool_postgres::GenericClient,
    snapshot: &TurnPersistenceSnapshot,
) -> Result<(), TurnError> {
    for table in [
        "turn_store_metadata",
        "turn_lifecycle_events",
        "turn_admission_reservations",
        "turn_idempotency_records",
        "turn_loop_checkpoints",
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
        let checkpoint_scope_key = record
            .scope
            .as_ref()
            .map(scope_key)
            .transpose()?
            .unwrap_or_default();
        txn.execute(
            "INSERT INTO turn_checkpoints (checkpoint_id, run_id, sequence, scope_key, kind, payload) VALUES ($1, $2, $3, $4, $5, $6::jsonb)",
            &[
                &record.checkpoint_id.as_uuid().to_string(),
                &record.run_id.to_string(),
                &(record.sequence as i64),
                &checkpoint_scope_key,
                &record.kind.as_str(),
                &payload,
            ],
        )
        .await
        .map_err(db_error)?;
    }
    for record in &snapshot.loop_checkpoints {
        let payload = to_json(record)?;
        txn.execute(
            "INSERT INTO turn_loop_checkpoints (checkpoint_id, scope_key, turn_id, run_id, created_at, payload) VALUES ($1, $2, $3, $4, $5::timestamptz, $6::jsonb)",
            &[
                &record.checkpoint_id.as_uuid().to_string(),
                &scope_key(&record.scope)?,
                &record.turn_id.to_string(),
                &record.run_id.to_string(),
                &record.created_at.to_rfc3339(),
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
    for event in &snapshot.events {
        let payload = to_json(event)?;
        txn.execute(
            "INSERT INTO turn_lifecycle_events (event_key, scope_key, run_id, event_cursor, kind, payload) VALUES ($1, $2, $3, $4, $5, $6::jsonb)",
            &[
                &turn_event_key(event)?,
                &scope_key(&event.scope)?,
                &event.run_id.to_string(),
                &(event.cursor.0 as i64),
                &turn_event_kind_key(event)?,
                &payload,
            ],
        )
        .await
        .map_err(db_error)?;
    }
    for record in &snapshot.admission_reservations {
        let payload = to_json(record)?;
        txn.execute(
            "INSERT INTO turn_admission_reservations (run_id, released, payload) VALUES ($1, $2, $3::jsonb)",
            &[&record.run_id.to_string(), &record.released, &payload],
        )
        .await
        .map_err(db_error)?;
    }
    txn.execute(
        "INSERT INTO turn_store_metadata (key, value) VALUES ('event_retention_floor', $1)",
        &[&snapshot.event_retention_floor.0.to_string()],
    )
    .await
    .map_err(db_error)?;
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

fn turn_event_key(event: &TurnLifecycleEvent) -> Result<String, TurnError> {
    #[derive(serde::Serialize)]
    struct TurnEventKey<'a> {
        scope: &'a crate::TurnScope,
        cursor: EventCursor,
        run_id: String,
        kind: &'a crate::TurnEventKind,
    }

    to_json(&TurnEventKey {
        scope: &event.scope,
        cursor: event.cursor,
        run_id: event.run_id.to_string(),
        kind: &event.kind,
    })
}

fn turn_event_kind_key(event: &TurnLifecycleEvent) -> Result<String, TurnError> {
    to_json(&event.kind)
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn loop_checkpoint_record_matches_request(
    record: &LoopCheckpointRecord,
    request: &GetLoopCheckpointRequest,
) -> bool {
    record.scope == request.scope
        && record.turn_id == request.turn_id
        && record.run_id == request.run_id
        && record.checkpoint_id == request.checkpoint_id
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn ensure_loop_checkpoint_insert_is_idempotent(
    existing: &LoopCheckpointRecord,
    incoming: &LoopCheckpointRecord,
) -> Result<(), TurnError> {
    if existing == incoming {
        Ok(())
    } else {
        Err(TurnError::Conflict {
            reason: "loop checkpoint id already belongs to a different checkpoint mapping"
                .to_string(),
        })
    }
}

fn db_error(error: impl std::fmt::Display) -> TurnError {
    tracing::debug!(%error, "turn state persistence operation failed");
    TurnError::Unavailable {
        reason: "turn state persistence temporarily unavailable".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};

    fn test_scope(thread: &str) -> TurnScope {
        TurnScope::new(
            TenantId::new("tenant-db-checkpoint").unwrap(),
            Some(AgentId::new("agent-db-checkpoint").unwrap()),
            Some(ProjectId::new("project-db-checkpoint").unwrap()),
            ThreadId::new(thread).unwrap(),
        )
    }

    fn loop_checkpoint_record(thread: &str) -> LoopCheckpointRecord {
        LoopCheckpointRecord {
            checkpoint_id: crate::TurnCheckpointId::new(),
            scope: test_scope(thread),
            turn_id: crate::TurnId::new(),
            run_id: crate::TurnRunId::new(),
            state_ref: crate::LoopCheckpointStateRef::new("checkpoint:db-conflict").unwrap(),
            schema_id: crate::CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap(),
            schema_version: crate::RunProfileVersion::new(1),
            kind: crate::LoopCheckpointKind::BeforeBlock,
            created_at: Utc::now(),
        }
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn libsql_loop_checkpoint_insert_conflicts_on_same_id_different_scope_or_run() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("turns.db");
        let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
        let store = LibSqlTurnStateStore::new(Arc::clone(&db));
        store.run_migrations().await.unwrap();
        let conn = db.connect().unwrap();

        let record = loop_checkpoint_record("libsql-conflict-a");
        libsql_insert_loop_checkpoint_record(&conn, &record)
            .await
            .unwrap();
        libsql_insert_loop_checkpoint_record(&conn, &record)
            .await
            .unwrap();

        let mut conflicting = record.clone();
        conflicting.scope = test_scope("libsql-conflict-b");
        conflicting.run_id = crate::TurnRunId::new();
        let error = libsql_insert_loop_checkpoint_record(&conn, &conflicting)
            .await
            .unwrap_err();
        assert!(matches!(error, TurnError::Conflict { .. }));
    }

    #[cfg(feature = "postgres")]
    #[tokio::test]
    async fn postgres_loop_checkpoint_insert_conflicts_on_same_id_different_scope_or_run() {
        let Some(pool) = postgres_pool().await else {
            return;
        };
        let store = PostgresTurnStateStore::new(pool.clone());
        store.run_migrations().await.unwrap();
        let client = pool.get().await.unwrap();

        let record = loop_checkpoint_record("postgres-conflict-a");
        postgres_insert_loop_checkpoint_record(&client, &record)
            .await
            .unwrap();
        postgres_insert_loop_checkpoint_record(&client, &record)
            .await
            .unwrap();

        let mut conflicting = record.clone();
        conflicting.scope = test_scope("postgres-conflict-b");
        conflicting.run_id = crate::TurnRunId::new();
        let error = postgres_insert_loop_checkpoint_record(&client, &conflicting)
            .await
            .unwrap_err();
        assert!(matches!(error, TurnError::Conflict { .. }));
    }

    #[cfg(feature = "postgres")]
    async fn postgres_pool() -> Option<deadpool_postgres::Pool> {
        let Ok(url) = std::env::var("IRONCLAW_TURNS_POSTGRES_URL") else {
            eprintln!(
                "skipping postgres loop checkpoint conflict test: IRONCLAW_TURNS_POSTGRES_URL not set"
            );
            return None;
        };
        let config: tokio_postgres::Config = match url.parse() {
            Ok(config) => config,
            Err(error) => {
                eprintln!("skipping postgres loop checkpoint conflict test: invalid url ({error})");
                return None;
            }
        };
        let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
        let pool = deadpool_postgres::Pool::builder(manager)
            .max_size(4)
            .build()
            .unwrap();
        if let Err(error) = pool.get().await {
            eprintln!(
                "skipping postgres loop checkpoint conflict test: database unavailable ({error})"
            );
            return None;
        }
        Some(pool)
    }
}
