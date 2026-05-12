use ironclaw_host_api::{ApprovalRequest, ApprovalRequestId, InvocationId, ResourceScope};

use crate::{
    ApprovalRecord, ApprovalRequestStore, ApprovalStatus, RunRecord, RunStart,
    RunStateApprovalStore, RunStateError, RunStateStore, RunStatus,
};

#[cfg(feature = "libsql")]
use std::sync::Arc;

#[cfg(feature = "libsql")]
const LIBSQL_RUN_STATE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reborn_run_state_records (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    mission_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    invocation_id TEXT NOT NULL,
    capability_id TEXT NOT NULL,
    status TEXT NOT NULL,
    approval_request_id TEXT,
    error_kind TEXT,
    payload TEXT NOT NULL,
    PRIMARY KEY (tenant_id, user_id, agent_id, project_id, mission_id, thread_id, invocation_id)
);
CREATE INDEX IF NOT EXISTS idx_reborn_run_state_records_scope_status
    ON reborn_run_state_records(tenant_id, user_id, agent_id, project_id, mission_id, thread_id, status);

CREATE TABLE IF NOT EXISTS reborn_approval_request_records (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    mission_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    status TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (tenant_id, user_id, agent_id, project_id, mission_id, thread_id, request_id)
);
CREATE INDEX IF NOT EXISTS idx_reborn_approval_request_records_scope_status
    ON reborn_approval_request_records(tenant_id, user_id, agent_id, project_id, mission_id, thread_id, status);
"#;

#[cfg(feature = "postgres")]
const POSTGRES_RUN_STATE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reborn_run_state_records (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    mission_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    invocation_id TEXT NOT NULL,
    capability_id TEXT NOT NULL,
    status TEXT NOT NULL,
    approval_request_id TEXT,
    error_kind TEXT,
    payload JSONB NOT NULL,
    PRIMARY KEY (tenant_id, user_id, agent_id, project_id, mission_id, thread_id, invocation_id)
);
CREATE INDEX IF NOT EXISTS idx_reborn_run_state_records_scope_status
    ON reborn_run_state_records(tenant_id, user_id, agent_id, project_id, mission_id, thread_id, status);

CREATE TABLE IF NOT EXISTS reborn_approval_request_records (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    mission_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    status TEXT NOT NULL,
    payload JSONB NOT NULL,
    PRIMARY KEY (tenant_id, user_id, agent_id, project_id, mission_id, thread_id, request_id)
);
CREATE INDEX IF NOT EXISTS idx_reborn_approval_request_records_scope_status
    ON reborn_approval_request_records(tenant_id, user_id, agent_id, project_id, mission_id, thread_id, status);
"#;

/// libSQL-backed invocation lifecycle store.
#[cfg(feature = "libsql")]
pub struct LibSqlRunStateStore {
    db: Arc<libsql::Database>,
}

#[cfg(feature = "libsql")]
impl LibSqlRunStateStore {
    pub fn new(db: Arc<libsql::Database>) -> Self {
        Self { db }
    }

    pub async fn run_migrations(&self) -> Result<(), RunStateError> {
        run_libsql_migrations(&self.db).await
    }

    async fn connect(&self) -> Result<libsql::Connection, RunStateError> {
        libsql_connect(&self.db).await
    }
}

#[cfg(feature = "libsql")]
#[async_trait::async_trait]
impl RunStateStore for LibSqlRunStateStore {
    async fn start(&self, start: RunStart) -> Result<RunRecord, RunStateError> {
        let conn = libsql_begin_immediate(&self.db).await?;
        let result = async {
            if libsql_get_run(&conn, &start.scope, start.invocation_id)
                .await?
                .is_some()
            {
                return Err(RunStateError::InvocationAlreadyExists {
                    invocation_id: start.invocation_id,
                });
            }
            let record = RunRecord {
                invocation_id: start.invocation_id,
                capability_id: start.capability_id,
                scope: start.scope,
                status: RunStatus::Running,
                approval_request_id: None,
                error_kind: None,
            };
            libsql_insert_run(&conn, &record).await?;
            Ok(record)
        }
        .await;
        finish_libsql_transaction(&conn, result).await
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        self.update_run(scope, invocation_id, |record| {
            record.status = RunStatus::BlockedApproval;
            record.approval_request_id = Some(approval.id);
            record.error_kind = None;
        })
        .await
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.update_run(scope, invocation_id, |record| {
            record.status = RunStatus::BlockedAuth;
            record.approval_request_id = None;
            record.error_kind = Some(error_kind);
        })
        .await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        self.update_run(scope, invocation_id, |record| {
            record.status = RunStatus::Completed;
            record.approval_request_id = None;
            record.error_kind = None;
        })
        .await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.update_run(scope, invocation_id, |record| {
            record.status = RunStatus::Failed;
            record.approval_request_id = None;
            record.error_kind = Some(error_kind);
        })
        .await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<RunRecord>, RunStateError> {
        let conn = self.connect().await?;
        libsql_get_run(&conn, scope, invocation_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError> {
        let conn = self.connect().await?;
        let key = ScopeKey::new(scope);
        let mut rows = conn
            .query(
                "SELECT invocation_id, capability_id, status, approval_request_id, error_kind, payload FROM reborn_run_state_records WHERE tenant_id = ?1 AND user_id = ?2 AND agent_id = ?3 AND project_id = ?4 AND mission_id = ?5 AND thread_id = ?6 ORDER BY invocation_id",
                libsql::params![
                    key.tenant_id,
                    key.user_id,
                    key.agent_id,
                    key.project_id,
                    key.mission_id,
                    key.thread_id,
                ],
            )
            .await
            .map_err(db_error)?;
        let mut records = Vec::new();
        while let Some(row) = rows.next().await.map_err(db_error)? {
            let row_invocation_id: String = row.get(0).map_err(db_error)?;
            let row_invocation_id = parse_invocation_id_column(&row_invocation_id)?;
            let capability_id: String = row.get(1).map_err(db_error)?;
            let status: String = row.get(2).map_err(db_error)?;
            let approval_request_id: Option<String> = row.get(3).map_err(db_error)?;
            let error_kind: Option<String> = row.get(4).map_err(db_error)?;
            let payload: String = row.get(5).map_err(db_error)?;
            let record = validate_run_row(
                from_json::<RunRecord>(&payload)?,
                scope,
                row_invocation_id,
                &capability_id,
                &status,
                approval_request_id.as_deref(),
                error_kind.as_deref(),
            )?;
            records.push(record);
        }
        Ok(records)
    }
}

#[cfg(feature = "libsql")]
impl LibSqlRunStateStore {
    async fn update_run(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        update: impl FnOnce(&mut RunRecord),
    ) -> Result<RunRecord, RunStateError> {
        let conn = libsql_begin_immediate(&self.db).await?;
        let result = async {
            let mut record = libsql_get_run(&conn, scope, invocation_id)
                .await?
                .ok_or(RunStateError::UnknownInvocation { invocation_id })?;
            update(&mut record);
            libsql_update_run(&conn, &record).await?;
            Ok(record)
        }
        .await;
        finish_libsql_transaction(&conn, result).await
    }
}

/// Combined libSQL run-state and approval-request store.
#[cfg(feature = "libsql")]
pub struct LibSqlRunStateApprovalStore {
    db: Arc<libsql::Database>,
}

#[cfg(feature = "libsql")]
impl LibSqlRunStateApprovalStore {
    pub fn new(db: Arc<libsql::Database>) -> Self {
        Self { db }
    }

    pub async fn run_migrations(&self) -> Result<(), RunStateError> {
        run_libsql_migrations(&self.db).await
    }
}

#[cfg(feature = "libsql")]
#[async_trait::async_trait]
impl RunStateStore for LibSqlRunStateApprovalStore {
    async fn start(&self, start: RunStart) -> Result<RunRecord, RunStateError> {
        LibSqlRunStateStore::new(Arc::clone(&self.db))
            .start(start)
            .await
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        LibSqlRunStateStore::new(Arc::clone(&self.db))
            .block_approval(scope, invocation_id, approval)
            .await
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        LibSqlRunStateStore::new(Arc::clone(&self.db))
            .block_auth(scope, invocation_id, error_kind)
            .await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        LibSqlRunStateStore::new(Arc::clone(&self.db))
            .complete(scope, invocation_id)
            .await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        LibSqlRunStateStore::new(Arc::clone(&self.db))
            .fail(scope, invocation_id, error_kind)
            .await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<RunRecord>, RunStateError> {
        LibSqlRunStateStore::new(Arc::clone(&self.db))
            .get(scope, invocation_id)
            .await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError> {
        LibSqlRunStateStore::new(Arc::clone(&self.db))
            .records_for_scope(scope)
            .await
    }
}

#[cfg(feature = "libsql")]
#[async_trait::async_trait]
impl ApprovalRequestStore for LibSqlRunStateApprovalStore {
    async fn save_pending(
        &self,
        scope: ResourceScope,
        request: ApprovalRequest,
    ) -> Result<ApprovalRecord, RunStateError> {
        LibSqlApprovalRequestStore::new(Arc::clone(&self.db))
            .save_pending(scope, request)
            .await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<Option<ApprovalRecord>, RunStateError> {
        LibSqlApprovalRequestStore::new(Arc::clone(&self.db))
            .get(scope, request_id)
            .await
    }

    async fn approve(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        LibSqlApprovalRequestStore::new(Arc::clone(&self.db))
            .approve(scope, request_id)
            .await
    }

    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        LibSqlApprovalRequestStore::new(Arc::clone(&self.db))
            .deny(scope, request_id)
            .await
    }

    async fn discard_pending(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        LibSqlApprovalRequestStore::new(Arc::clone(&self.db))
            .discard_pending(scope, request_id)
            .await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ApprovalRecord>, RunStateError> {
        LibSqlApprovalRequestStore::new(Arc::clone(&self.db))
            .records_for_scope(scope)
            .await
    }
}

#[cfg(feature = "libsql")]
#[async_trait::async_trait]
impl RunStateApprovalStore for LibSqlRunStateApprovalStore {
    async fn save_pending_and_block_approval(
        &self,
        scope: ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        let conn = libsql_begin_immediate(&self.db).await?;
        let result = async {
            if libsql_get_approval(&conn, &scope, approval.id)
                .await?
                .is_some()
            {
                return Err(RunStateError::ApprovalRequestAlreadyExists {
                    request_id: approval.id,
                });
            }
            let mut run = libsql_get_run(&conn, &scope, invocation_id)
                .await?
                .ok_or(RunStateError::UnknownInvocation { invocation_id })?;
            let approval_record = ApprovalRecord {
                scope: scope.clone(),
                request: approval.clone(),
                status: ApprovalStatus::Pending,
            };
            libsql_insert_approval(&conn, &approval_record).await?;
            run.status = RunStatus::BlockedApproval;
            run.approval_request_id = Some(approval.id);
            run.error_kind = None;
            libsql_update_run(&conn, &run).await?;
            Ok(run)
        }
        .await;
        finish_libsql_transaction(&conn, result).await
    }
}

/// libSQL-backed approval request store.
#[cfg(feature = "libsql")]
pub struct LibSqlApprovalRequestStore {
    db: Arc<libsql::Database>,
}

#[cfg(feature = "libsql")]
impl LibSqlApprovalRequestStore {
    pub fn new(db: Arc<libsql::Database>) -> Self {
        Self { db }
    }

    pub async fn run_migrations(&self) -> Result<(), RunStateError> {
        run_libsql_migrations(&self.db).await
    }

    async fn connect(&self) -> Result<libsql::Connection, RunStateError> {
        libsql_connect(&self.db).await
    }
}

#[cfg(feature = "libsql")]
#[async_trait::async_trait]
impl ApprovalRequestStore for LibSqlApprovalRequestStore {
    async fn save_pending(
        &self,
        scope: ResourceScope,
        request: ApprovalRequest,
    ) -> Result<ApprovalRecord, RunStateError> {
        let conn = libsql_begin_immediate(&self.db).await?;
        let result = async {
            if libsql_get_approval(&conn, &scope, request.id)
                .await?
                .is_some()
            {
                return Err(RunStateError::ApprovalRequestAlreadyExists {
                    request_id: request.id,
                });
            }
            let record = ApprovalRecord {
                scope,
                request,
                status: ApprovalStatus::Pending,
            };
            libsql_insert_approval(&conn, &record).await?;
            Ok(record)
        }
        .await;
        finish_libsql_transaction(&conn, result).await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<Option<ApprovalRecord>, RunStateError> {
        let conn = self.connect().await?;
        libsql_get_approval(&conn, scope, request_id).await
    }

    async fn approve(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        self.update_status(scope, request_id, ApprovalStatus::Approved)
            .await
    }

    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        self.update_status(scope, request_id, ApprovalStatus::Denied)
            .await
    }

    async fn discard_pending(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        let conn = libsql_begin_immediate(&self.db).await?;
        let result = async {
            let record = libsql_get_approval(&conn, scope, request_id)
                .await?
                .ok_or(RunStateError::UnknownApprovalRequest { request_id })?;
            if record.status != ApprovalStatus::Pending {
                return Err(RunStateError::ApprovalNotPending {
                    request_id,
                    status: record.status,
                });
            }
            let key = ScopeKey::new(scope);
            let affected = conn
                .execute(
                    "DELETE FROM reborn_approval_request_records WHERE tenant_id = ?1 AND user_id = ?2 AND agent_id = ?3 AND project_id = ?4 AND mission_id = ?5 AND thread_id = ?6 AND request_id = ?7",
                    libsql::params![
                        key.tenant_id,
                        key.user_id,
                        key.agent_id,
                        key.project_id,
                        key.mission_id,
                        key.thread_id,
                        request_id.to_string(),
                    ],
                )
                .await
                .map_err(db_error)?;
            require_single_affected_row(affected, "discard approval")?;
            Ok(record)
        }
        .await;
        finish_libsql_transaction(&conn, result).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ApprovalRecord>, RunStateError> {
        let conn = self.connect().await?;
        let key = ScopeKey::new(scope);
        let mut rows = conn
            .query(
                "SELECT request_id, status, payload FROM reborn_approval_request_records WHERE tenant_id = ?1 AND user_id = ?2 AND agent_id = ?3 AND project_id = ?4 AND mission_id = ?5 AND thread_id = ?6 ORDER BY request_id",
                libsql::params![
                    key.tenant_id,
                    key.user_id,
                    key.agent_id,
                    key.project_id,
                    key.mission_id,
                    key.thread_id,
                ],
            )
            .await
            .map_err(db_error)?;
        let mut records = Vec::new();
        while let Some(row) = rows.next().await.map_err(db_error)? {
            let request_id: String = row.get(0).map_err(db_error)?;
            let request_id = parse_approval_request_id_column(&request_id)?;
            let status: String = row.get(1).map_err(db_error)?;
            let payload: String = row.get(2).map_err(db_error)?;
            let record = validate_approval_row(
                from_json::<ApprovalRecord>(&payload)?,
                scope,
                request_id,
                &status,
            )?;
            records.push(record);
        }
        Ok(records)
    }
}

#[cfg(feature = "libsql")]
impl LibSqlApprovalRequestStore {
    async fn update_status(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        status: ApprovalStatus,
    ) -> Result<ApprovalRecord, RunStateError> {
        let conn = libsql_begin_immediate(&self.db).await?;
        let result = async {
            let mut record = libsql_get_approval(&conn, scope, request_id)
                .await?
                .ok_or(RunStateError::UnknownApprovalRequest { request_id })?;
            if record.status != ApprovalStatus::Pending {
                return Err(RunStateError::ApprovalNotPending {
                    request_id,
                    status: record.status,
                });
            }
            record.status = status;
            libsql_update_approval(&conn, &record).await?;
            Ok(record)
        }
        .await;
        finish_libsql_transaction(&conn, result).await
    }
}

/// PostgreSQL-backed invocation lifecycle store.
#[cfg(feature = "postgres")]
pub struct PostgresRunStateStore {
    pool: deadpool_postgres::Pool,
}

#[cfg(feature = "postgres")]
impl PostgresRunStateStore {
    pub fn new(pool: deadpool_postgres::Pool) -> Self {
        Self { pool }
    }

    pub async fn run_migrations(&self) -> Result<(), RunStateError> {
        run_postgres_migrations(&self.pool).await
    }

    async fn client(&self) -> Result<deadpool_postgres::Object, RunStateError> {
        self.pool.get().await.map_err(db_error)
    }
}

#[cfg(feature = "postgres")]
#[async_trait::async_trait]
impl RunStateStore for PostgresRunStateStore {
    async fn start(&self, start: RunStart) -> Result<RunRecord, RunStateError> {
        let client = self.client().await?;
        let record = RunRecord {
            invocation_id: start.invocation_id,
            capability_id: start.capability_id,
            scope: start.scope,
            status: RunStatus::Running,
            approval_request_id: None,
            error_kind: None,
        };
        if !postgres_insert_run(&client, &record).await? {
            return Err(RunStateError::InvocationAlreadyExists {
                invocation_id: record.invocation_id,
            });
        }
        Ok(record)
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        self.update_run(scope, invocation_id, |record| {
            record.status = RunStatus::BlockedApproval;
            record.approval_request_id = Some(approval.id);
            record.error_kind = None;
        })
        .await
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.update_run(scope, invocation_id, |record| {
            record.status = RunStatus::BlockedAuth;
            record.approval_request_id = None;
            record.error_kind = Some(error_kind);
        })
        .await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        self.update_run(scope, invocation_id, |record| {
            record.status = RunStatus::Completed;
            record.approval_request_id = None;
            record.error_kind = None;
        })
        .await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.update_run(scope, invocation_id, |record| {
            record.status = RunStatus::Failed;
            record.approval_request_id = None;
            record.error_kind = Some(error_kind);
        })
        .await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<RunRecord>, RunStateError> {
        let client = self.client().await?;
        postgres_get_run(&client, scope, invocation_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError> {
        let client = self.client().await?;
        let key = ScopeKey::new(scope);
        let rows = client
            .query(
                "SELECT invocation_id, capability_id, status, approval_request_id, error_kind, payload::text FROM reborn_run_state_records WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND project_id = $4 AND mission_id = $5 AND thread_id = $6 ORDER BY invocation_id",
                &[
                    &key.tenant_id,
                    &key.user_id,
                    &key.agent_id,
                    &key.project_id,
                    &key.mission_id,
                    &key.thread_id,
                ],
            )
            .await
            .map_err(db_error)?;
        let records = rows
            .into_iter()
            .map(|row| {
                let row_invocation_id: String = row.get(0);
                let row_invocation_id = parse_invocation_id_column(&row_invocation_id)?;
                let capability_id: String = row.get(1);
                let status: String = row.get(2);
                let approval_request_id: Option<String> = row.get(3);
                let error_kind: Option<String> = row.get(4);
                let payload: String = row.get(5);
                validate_run_row(
                    from_json::<RunRecord>(&payload)?,
                    scope,
                    row_invocation_id,
                    &capability_id,
                    &status,
                    approval_request_id.as_deref(),
                    error_kind.as_deref(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }
}

#[cfg(feature = "postgres")]
impl PostgresRunStateStore {
    async fn update_run(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        update: impl FnOnce(&mut RunRecord),
    ) -> Result<RunRecord, RunStateError> {
        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        let mut record = match postgres_get_run_for_update(&txn, scope, invocation_id).await? {
            Some(record) => record,
            None => {
                txn.rollback().await.map_err(db_error)?;
                return Err(RunStateError::UnknownInvocation { invocation_id });
            }
        };
        update(&mut record);
        postgres_update_run(&txn, &record).await?;
        txn.commit().await.map_err(db_error)?;
        Ok(record)
    }
}

/// Combined PostgreSQL run-state and approval-request store.
#[cfg(feature = "postgres")]
pub struct PostgresRunStateApprovalStore {
    pool: deadpool_postgres::Pool,
}

#[cfg(feature = "postgres")]
impl PostgresRunStateApprovalStore {
    pub fn new(pool: deadpool_postgres::Pool) -> Self {
        Self { pool }
    }

    pub async fn run_migrations(&self) -> Result<(), RunStateError> {
        run_postgres_migrations(&self.pool).await
    }
}

#[cfg(feature = "postgres")]
#[async_trait::async_trait]
impl RunStateStore for PostgresRunStateApprovalStore {
    async fn start(&self, start: RunStart) -> Result<RunRecord, RunStateError> {
        PostgresRunStateStore::new(self.pool.clone())
            .start(start)
            .await
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        PostgresRunStateStore::new(self.pool.clone())
            .block_approval(scope, invocation_id, approval)
            .await
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        PostgresRunStateStore::new(self.pool.clone())
            .block_auth(scope, invocation_id, error_kind)
            .await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        PostgresRunStateStore::new(self.pool.clone())
            .complete(scope, invocation_id)
            .await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        PostgresRunStateStore::new(self.pool.clone())
            .fail(scope, invocation_id, error_kind)
            .await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<RunRecord>, RunStateError> {
        PostgresRunStateStore::new(self.pool.clone())
            .get(scope, invocation_id)
            .await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError> {
        PostgresRunStateStore::new(self.pool.clone())
            .records_for_scope(scope)
            .await
    }
}

#[cfg(feature = "postgres")]
#[async_trait::async_trait]
impl ApprovalRequestStore for PostgresRunStateApprovalStore {
    async fn save_pending(
        &self,
        scope: ResourceScope,
        request: ApprovalRequest,
    ) -> Result<ApprovalRecord, RunStateError> {
        PostgresApprovalRequestStore::new(self.pool.clone())
            .save_pending(scope, request)
            .await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<Option<ApprovalRecord>, RunStateError> {
        PostgresApprovalRequestStore::new(self.pool.clone())
            .get(scope, request_id)
            .await
    }

    async fn approve(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        PostgresApprovalRequestStore::new(self.pool.clone())
            .approve(scope, request_id)
            .await
    }

    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        PostgresApprovalRequestStore::new(self.pool.clone())
            .deny(scope, request_id)
            .await
    }

    async fn discard_pending(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        PostgresApprovalRequestStore::new(self.pool.clone())
            .discard_pending(scope, request_id)
            .await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ApprovalRecord>, RunStateError> {
        PostgresApprovalRequestStore::new(self.pool.clone())
            .records_for_scope(scope)
            .await
    }
}

#[cfg(feature = "postgres")]
#[async_trait::async_trait]
impl RunStateApprovalStore for PostgresRunStateApprovalStore {
    async fn save_pending_and_block_approval(
        &self,
        scope: ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        let mut client = self.pool.get().await.map_err(db_error)?;
        let txn = client.transaction().await.map_err(db_error)?;
        let mut run = match postgres_get_run_for_update(&txn, &scope, invocation_id).await? {
            Some(run) => run,
            None => {
                txn.rollback().await.map_err(db_error)?;
                return Err(RunStateError::UnknownInvocation { invocation_id });
            }
        };
        let approval_record = ApprovalRecord {
            scope: scope.clone(),
            request: approval.clone(),
            status: ApprovalStatus::Pending,
        };
        if !postgres_insert_approval(&txn, &approval_record).await? {
            txn.rollback().await.map_err(db_error)?;
            return Err(RunStateError::ApprovalRequestAlreadyExists {
                request_id: approval.id,
            });
        }
        run.status = RunStatus::BlockedApproval;
        run.approval_request_id = Some(approval.id);
        run.error_kind = None;
        postgres_update_run(&txn, &run).await?;
        txn.commit().await.map_err(db_error)?;
        Ok(run)
    }
}

/// PostgreSQL-backed approval request store.
#[cfg(feature = "postgres")]
pub struct PostgresApprovalRequestStore {
    pool: deadpool_postgres::Pool,
}

#[cfg(feature = "postgres")]
impl PostgresApprovalRequestStore {
    pub fn new(pool: deadpool_postgres::Pool) -> Self {
        Self { pool }
    }

    pub async fn run_migrations(&self) -> Result<(), RunStateError> {
        run_postgres_migrations(&self.pool).await
    }

    async fn client(&self) -> Result<deadpool_postgres::Object, RunStateError> {
        self.pool.get().await.map_err(db_error)
    }
}

#[cfg(feature = "postgres")]
#[async_trait::async_trait]
impl ApprovalRequestStore for PostgresApprovalRequestStore {
    async fn save_pending(
        &self,
        scope: ResourceScope,
        request: ApprovalRequest,
    ) -> Result<ApprovalRecord, RunStateError> {
        let client = self.client().await?;
        let record = ApprovalRecord {
            scope,
            request,
            status: ApprovalStatus::Pending,
        };
        if !postgres_insert_approval(&client, &record).await? {
            return Err(RunStateError::ApprovalRequestAlreadyExists {
                request_id: record.request.id,
            });
        }
        Ok(record)
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<Option<ApprovalRecord>, RunStateError> {
        let client = self.client().await?;
        postgres_get_approval(&client, scope, request_id).await
    }

    async fn approve(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        self.update_status(scope, request_id, ApprovalStatus::Approved)
            .await
    }

    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        self.update_status(scope, request_id, ApprovalStatus::Denied)
            .await
    }

    async fn discard_pending(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        let record = match postgres_get_approval_for_update(&txn, scope, request_id).await? {
            Some(record) => record,
            None => {
                txn.rollback().await.map_err(db_error)?;
                return Err(RunStateError::UnknownApprovalRequest { request_id });
            }
        };
        if record.status != ApprovalStatus::Pending {
            txn.rollback().await.map_err(db_error)?;
            return Err(RunStateError::ApprovalNotPending {
                request_id,
                status: record.status,
            });
        }
        let key = ScopeKey::new(scope);
        let affected = txn
            .execute(
                "DELETE FROM reborn_approval_request_records WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND project_id = $4 AND mission_id = $5 AND thread_id = $6 AND request_id = $7",
                &[
                    &key.tenant_id,
                    &key.user_id,
                    &key.agent_id,
                    &key.project_id,
                    &key.mission_id,
                    &key.thread_id,
                    &request_id.to_string(),
                ],
            )
            .await
            .map_err(db_error)?;
        require_single_affected_row(affected, "discard approval")?;
        txn.commit().await.map_err(db_error)?;
        Ok(record)
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ApprovalRecord>, RunStateError> {
        let client = self.client().await?;
        let key = ScopeKey::new(scope);
        let rows = client
            .query(
                "SELECT request_id, status, payload::text FROM reborn_approval_request_records WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND project_id = $4 AND mission_id = $5 AND thread_id = $6 ORDER BY request_id",
                &[
                    &key.tenant_id,
                    &key.user_id,
                    &key.agent_id,
                    &key.project_id,
                    &key.mission_id,
                    &key.thread_id,
                ],
            )
            .await
            .map_err(db_error)?;
        let records = rows
            .into_iter()
            .map(|row| {
                let request_id: String = row.get(0);
                let request_id = parse_approval_request_id_column(&request_id)?;
                let status: String = row.get(1);
                let payload: String = row.get(2);
                validate_approval_row(
                    from_json::<ApprovalRecord>(&payload)?,
                    scope,
                    request_id,
                    &status,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }
}

#[cfg(feature = "postgres")]
impl PostgresApprovalRequestStore {
    async fn update_status(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        status: ApprovalStatus,
    ) -> Result<ApprovalRecord, RunStateError> {
        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        let mut record = match postgres_get_approval_for_update(&txn, scope, request_id).await? {
            Some(record) => record,
            None => {
                txn.rollback().await.map_err(db_error)?;
                return Err(RunStateError::UnknownApprovalRequest { request_id });
            }
        };
        if record.status != ApprovalStatus::Pending {
            txn.rollback().await.map_err(db_error)?;
            return Err(RunStateError::ApprovalNotPending {
                request_id,
                status: record.status,
            });
        }
        record.status = status;
        postgres_update_approval(&txn, &record).await?;
        txn.commit().await.map_err(db_error)?;
        Ok(record)
    }
}

#[cfg(feature = "libsql")]
async fn run_libsql_migrations(db: &libsql::Database) -> Result<(), RunStateError> {
    let conn = libsql_connect(db).await?;
    conn.execute_batch(LIBSQL_RUN_STATE_SCHEMA)
        .await
        .map_err(db_error)?;
    Ok(())
}

#[cfg(feature = "libsql")]
async fn libsql_connect(db: &libsql::Database) -> Result<libsql::Connection, RunStateError> {
    let conn = db.connect().map_err(db_error)?;
    conn.query("PRAGMA busy_timeout = 5000", ())
        .await
        .map_err(db_error)?;
    Ok(conn)
}

#[cfg(feature = "libsql")]
async fn libsql_begin_immediate(
    db: &libsql::Database,
) -> Result<libsql::Connection, RunStateError> {
    let conn = libsql_connect(db).await?;
    conn.execute("BEGIN IMMEDIATE", ())
        .await
        .map_err(db_error)?;
    Ok(conn)
}

#[cfg(feature = "libsql")]
async fn finish_libsql_transaction<T>(
    conn: &libsql::Connection,
    result: Result<T, RunStateError>,
) -> Result<T, RunStateError> {
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
async fn libsql_get_run(
    conn: &libsql::Connection,
    scope: &ResourceScope,
    invocation_id: InvocationId,
) -> Result<Option<RunRecord>, RunStateError> {
    let key = ScopeKey::new(scope);
    let mut rows = conn
        .query(
            "SELECT capability_id, status, approval_request_id, error_kind, payload FROM reborn_run_state_records WHERE tenant_id = ?1 AND user_id = ?2 AND agent_id = ?3 AND project_id = ?4 AND mission_id = ?5 AND thread_id = ?6 AND invocation_id = ?7",
            libsql::params![
                key.tenant_id,
                key.user_id,
                key.agent_id,
                key.project_id,
                key.mission_id,
                key.thread_id,
                invocation_id.to_string(),
            ],
        )
        .await
        .map_err(db_error)?;
    let Some(row) = rows.next().await.map_err(db_error)? else {
        return Ok(None);
    };
    let capability_id: String = row.get(0).map_err(db_error)?;
    let status: String = row.get(1).map_err(db_error)?;
    let approval_request_id: Option<String> = row.get(2).map_err(db_error)?;
    let error_kind: Option<String> = row.get(3).map_err(db_error)?;
    let payload: String = row.get(4).map_err(db_error)?;
    validate_run_row(
        from_json::<RunRecord>(&payload)?,
        scope,
        invocation_id,
        &capability_id,
        &status,
        approval_request_id.as_deref(),
        error_kind.as_deref(),
    )
    .map(Some)
}

#[cfg(feature = "libsql")]
async fn libsql_insert_run(
    conn: &libsql::Connection,
    record: &RunRecord,
) -> Result<(), RunStateError> {
    let key = ScopeKey::new(&record.scope);
    conn.execute(
        "INSERT INTO reborn_run_state_records (tenant_id, user_id, agent_id, project_id, mission_id, thread_id, invocation_id, capability_id, status, approval_request_id, error_kind, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        libsql::params![
            key.tenant_id,
            key.user_id,
            key.agent_id,
            key.project_id,
            key.mission_id,
            key.thread_id,
            record.invocation_id.to_string(),
            record.capability_id.as_str(),
            run_status_key(record.status),
            record.approval_request_id.map(|id| id.to_string()),
            record.error_kind.clone(),
            to_json(record)?,
        ],
    )
    .await
    .map_err(db_error)?;
    Ok(())
}

#[cfg(feature = "libsql")]
async fn libsql_update_run(
    conn: &libsql::Connection,
    record: &RunRecord,
) -> Result<(), RunStateError> {
    let key = ScopeKey::new(&record.scope);
    let affected = conn
        .execute(
            "UPDATE reborn_run_state_records SET capability_id = ?8, status = ?9, approval_request_id = ?10, error_kind = ?11, payload = ?12 WHERE tenant_id = ?1 AND user_id = ?2 AND agent_id = ?3 AND project_id = ?4 AND mission_id = ?5 AND thread_id = ?6 AND invocation_id = ?7",
            libsql::params![
                key.tenant_id,
                key.user_id,
                key.agent_id,
                key.project_id,
                key.mission_id,
                key.thread_id,
                record.invocation_id.to_string(),
                record.capability_id.as_str(),
                run_status_key(record.status),
                record.approval_request_id.map(|id| id.to_string()),
                record.error_kind.clone(),
                to_json(record)?,
            ],
        )
        .await
        .map_err(db_error)?;
    require_single_affected_row(affected, "update run")
}

#[cfg(feature = "libsql")]
async fn libsql_get_approval(
    conn: &libsql::Connection,
    scope: &ResourceScope,
    request_id: ApprovalRequestId,
) -> Result<Option<ApprovalRecord>, RunStateError> {
    let key = ScopeKey::new(scope);
    let mut rows = conn
        .query(
            "SELECT status, payload FROM reborn_approval_request_records WHERE tenant_id = ?1 AND user_id = ?2 AND agent_id = ?3 AND project_id = ?4 AND mission_id = ?5 AND thread_id = ?6 AND request_id = ?7",
            libsql::params![
                key.tenant_id,
                key.user_id,
                key.agent_id,
                key.project_id,
                key.mission_id,
                key.thread_id,
                request_id.to_string(),
            ],
        )
        .await
        .map_err(db_error)?;
    let Some(row) = rows.next().await.map_err(db_error)? else {
        return Ok(None);
    };
    let status: String = row.get(0).map_err(db_error)?;
    let payload: String = row.get(1).map_err(db_error)?;
    validate_approval_row(
        from_json::<ApprovalRecord>(&payload)?,
        scope,
        request_id,
        &status,
    )
    .map(Some)
}

#[cfg(feature = "libsql")]
async fn libsql_insert_approval(
    conn: &libsql::Connection,
    record: &ApprovalRecord,
) -> Result<(), RunStateError> {
    let key = ScopeKey::new(&record.scope);
    conn.execute(
        "INSERT INTO reborn_approval_request_records (tenant_id, user_id, agent_id, project_id, mission_id, thread_id, request_id, status, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        libsql::params![
            key.tenant_id,
            key.user_id,
            key.agent_id,
            key.project_id,
            key.mission_id,
            key.thread_id,
            record.request.id.to_string(),
            approval_status_key(record.status),
            to_json(record)?,
        ],
    )
    .await
    .map_err(db_error)?;
    Ok(())
}

#[cfg(feature = "libsql")]
async fn libsql_update_approval(
    conn: &libsql::Connection,
    record: &ApprovalRecord,
) -> Result<(), RunStateError> {
    let key = ScopeKey::new(&record.scope);
    let affected = conn
        .execute(
            "UPDATE reborn_approval_request_records SET status = ?8, payload = ?9 WHERE tenant_id = ?1 AND user_id = ?2 AND agent_id = ?3 AND project_id = ?4 AND mission_id = ?5 AND thread_id = ?6 AND request_id = ?7",
            libsql::params![
                key.tenant_id,
                key.user_id,
                key.agent_id,
                key.project_id,
                key.mission_id,
                key.thread_id,
                record.request.id.to_string(),
                approval_status_key(record.status),
                to_json(record)?,
            ],
        )
        .await
        .map_err(db_error)?;
    require_single_affected_row(affected, "update approval")
}

#[cfg(feature = "postgres")]
async fn run_postgres_migrations(pool: &deadpool_postgres::Pool) -> Result<(), RunStateError> {
    const MIGRATION_LOCK_ID: i64 = 0x1c10_0001;

    let mut client = pool.get().await.map_err(db_error)?;
    let txn = client.transaction().await.map_err(db_error)?;
    txn.query_one("SELECT pg_advisory_xact_lock($1)", &[&MIGRATION_LOCK_ID])
        .await
        .map_err(db_error)?;
    if postgres_table_has_column(&txn, "reborn_run_state_records", "owner_key").await? {
        postgres_migrate_run_owner_key_schema(&txn).await?;
    }
    if postgres_table_has_column(&txn, "reborn_approval_request_records", "owner_key").await? {
        postgres_migrate_approval_owner_key_schema(&txn).await?;
    }
    if !postgres_schema_present(&txn).await? {
        txn.batch_execute(POSTGRES_RUN_STATE_SCHEMA)
            .await
            .map_err(db_error)?;
    }
    txn.commit().await.map_err(db_error)
}

#[cfg(feature = "postgres")]
async fn postgres_table_has_column(
    client: &impl deadpool_postgres::GenericClient,
    table_name: &str,
    column_name: &str,
) -> Result<bool, RunStateError> {
    let row = client
        .query_one(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.columns
                WHERE table_schema = 'public'
                  AND table_name = $1
                  AND column_name = $2
            )",
            &[&table_name, &column_name],
        )
        .await
        .map_err(db_error)?;
    Ok(row.get(0))
}

#[cfg(feature = "postgres")]
async fn postgres_migrate_run_owner_key_schema(
    client: &impl deadpool_postgres::GenericClient,
) -> Result<(), RunStateError> {
    client
        .batch_execute(
            "ALTER TABLE reborn_run_state_records DROP CONSTRAINT IF EXISTS reborn_run_state_records_pkey;
             ALTER TABLE reborn_run_state_records ADD COLUMN IF NOT EXISTS tenant_id TEXT;
             ALTER TABLE reborn_run_state_records ADD COLUMN IF NOT EXISTS user_id TEXT;
             ALTER TABLE reborn_run_state_records ADD COLUMN IF NOT EXISTS agent_id TEXT;
             ALTER TABLE reborn_run_state_records ADD COLUMN IF NOT EXISTS project_id TEXT;
             ALTER TABLE reborn_run_state_records ADD COLUMN IF NOT EXISTS mission_id TEXT;
             ALTER TABLE reborn_run_state_records ADD COLUMN IF NOT EXISTS thread_id TEXT;
             UPDATE reborn_run_state_records
                SET tenant_id = owner_key::jsonb->>'tenant_id',
                    user_id = owner_key::jsonb->>'user_id',
                    agent_id = COALESCE(owner_key::jsonb->>'agent_id', ''),
                    project_id = COALESCE(owner_key::jsonb->>'project_id', ''),
                    mission_id = COALESCE(owner_key::jsonb->>'mission_id', ''),
                    thread_id = COALESCE(owner_key::jsonb->>'thread_id', '')
              WHERE owner_key IS NOT NULL;
             ALTER TABLE reborn_run_state_records ALTER COLUMN tenant_id SET NOT NULL;
             ALTER TABLE reborn_run_state_records ALTER COLUMN user_id SET NOT NULL;
             ALTER TABLE reborn_run_state_records ALTER COLUMN agent_id SET NOT NULL;
             ALTER TABLE reborn_run_state_records ALTER COLUMN project_id SET NOT NULL;
             ALTER TABLE reborn_run_state_records ALTER COLUMN mission_id SET NOT NULL;
             ALTER TABLE reborn_run_state_records ALTER COLUMN thread_id SET NOT NULL;
             ALTER TABLE reborn_run_state_records DROP COLUMN IF EXISTS owner_key;
             ALTER TABLE reborn_run_state_records ADD PRIMARY KEY (tenant_id, user_id, agent_id, project_id, mission_id, thread_id, invocation_id);
             DROP INDEX IF EXISTS idx_reborn_run_state_records_owner_status;
             CREATE INDEX IF NOT EXISTS idx_reborn_run_state_records_scope_status
                ON reborn_run_state_records(tenant_id, user_id, agent_id, project_id, mission_id, thread_id, status);",
        )
        .await
        .map_err(db_error)
}

#[cfg(feature = "postgres")]
async fn postgres_migrate_approval_owner_key_schema(
    client: &impl deadpool_postgres::GenericClient,
) -> Result<(), RunStateError> {
    client
        .batch_execute(
            "ALTER TABLE reborn_approval_request_records DROP CONSTRAINT IF EXISTS reborn_approval_request_records_pkey;
             ALTER TABLE reborn_approval_request_records ADD COLUMN IF NOT EXISTS tenant_id TEXT;
             ALTER TABLE reborn_approval_request_records ADD COLUMN IF NOT EXISTS user_id TEXT;
             ALTER TABLE reborn_approval_request_records ADD COLUMN IF NOT EXISTS agent_id TEXT;
             ALTER TABLE reborn_approval_request_records ADD COLUMN IF NOT EXISTS project_id TEXT;
             ALTER TABLE reborn_approval_request_records ADD COLUMN IF NOT EXISTS mission_id TEXT;
             ALTER TABLE reborn_approval_request_records ADD COLUMN IF NOT EXISTS thread_id TEXT;
             UPDATE reborn_approval_request_records
                SET tenant_id = owner_key::jsonb->>'tenant_id',
                    user_id = owner_key::jsonb->>'user_id',
                    agent_id = COALESCE(owner_key::jsonb->>'agent_id', ''),
                    project_id = COALESCE(owner_key::jsonb->>'project_id', ''),
                    mission_id = COALESCE(owner_key::jsonb->>'mission_id', ''),
                    thread_id = COALESCE(owner_key::jsonb->>'thread_id', '')
              WHERE owner_key IS NOT NULL;
             ALTER TABLE reborn_approval_request_records ALTER COLUMN tenant_id SET NOT NULL;
             ALTER TABLE reborn_approval_request_records ALTER COLUMN user_id SET NOT NULL;
             ALTER TABLE reborn_approval_request_records ALTER COLUMN agent_id SET NOT NULL;
             ALTER TABLE reborn_approval_request_records ALTER COLUMN project_id SET NOT NULL;
             ALTER TABLE reborn_approval_request_records ALTER COLUMN mission_id SET NOT NULL;
             ALTER TABLE reborn_approval_request_records ALTER COLUMN thread_id SET NOT NULL;
             ALTER TABLE reborn_approval_request_records DROP COLUMN IF EXISTS owner_key;
             ALTER TABLE reborn_approval_request_records ADD PRIMARY KEY (tenant_id, user_id, agent_id, project_id, mission_id, thread_id, request_id);
             DROP INDEX IF EXISTS idx_reborn_approval_request_records_owner_status;
             CREATE INDEX IF NOT EXISTS idx_reborn_approval_request_records_scope_status
                ON reborn_approval_request_records(tenant_id, user_id, agent_id, project_id, mission_id, thread_id, status);",
        )
        .await
        .map_err(db_error)
}

#[cfg(feature = "postgres")]
async fn postgres_schema_present(
    client: &impl deadpool_postgres::GenericClient,
) -> Result<bool, RunStateError> {
    let row = client
        .query_one(
            "SELECT
                to_regclass('public.reborn_run_state_records') IS NOT NULL,
                to_regclass('public.idx_reborn_run_state_records_scope_status') IS NOT NULL,
                to_regclass('public.reborn_approval_request_records') IS NOT NULL,
                to_regclass('public.idx_reborn_approval_request_records_scope_status') IS NOT NULL,
                NOT EXISTS (
                    SELECT 1 FROM information_schema.columns
                    WHERE table_schema = 'public'
                      AND table_name IN ('reborn_run_state_records', 'reborn_approval_request_records')
                      AND column_name = 'owner_key'
                ),
                (
                    SELECT count(*) FROM information_schema.columns
                    WHERE table_schema = 'public'
                      AND table_name IN ('reborn_run_state_records', 'reborn_approval_request_records')
                      AND column_name IN ('tenant_id', 'user_id', 'agent_id', 'project_id', 'mission_id', 'thread_id')
                ) = 12",
            &[],
        )
        .await
        .map_err(db_error)?;
    Ok(row.get::<_, bool>(0)
        && row.get::<_, bool>(1)
        && row.get::<_, bool>(2)
        && row.get::<_, bool>(3)
        && row.get::<_, bool>(4)
        && row.get::<_, bool>(5))
}

#[cfg(feature = "postgres")]
async fn postgres_get_run(
    client: &impl deadpool_postgres::GenericClient,
    scope: &ResourceScope,
    invocation_id: InvocationId,
) -> Result<Option<RunRecord>, RunStateError> {
    let key = ScopeKey::new(scope);
    let row = client
        .query_opt(
            "SELECT capability_id, status, approval_request_id, error_kind, payload::text FROM reborn_run_state_records WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND project_id = $4 AND mission_id = $5 AND thread_id = $6 AND invocation_id = $7",
            &[
                &key.tenant_id,
                &key.user_id,
                &key.agent_id,
                &key.project_id,
                &key.mission_id,
                &key.thread_id,
                &invocation_id.to_string(),
            ],
        )
        .await
        .map_err(db_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let capability_id: String = row.get(0);
    let status: String = row.get(1);
    let approval_request_id: Option<String> = row.get(2);
    let error_kind: Option<String> = row.get(3);
    let payload: String = row.get(4);
    validate_run_row(
        from_json::<RunRecord>(&payload)?,
        scope,
        invocation_id,
        &capability_id,
        &status,
        approval_request_id.as_deref(),
        error_kind.as_deref(),
    )
    .map(Some)
}

#[cfg(feature = "postgres")]
async fn postgres_get_run_for_update(
    client: &impl deadpool_postgres::GenericClient,
    scope: &ResourceScope,
    invocation_id: InvocationId,
) -> Result<Option<RunRecord>, RunStateError> {
    let key = ScopeKey::new(scope);
    let row = client
        .query_opt(
            "SELECT capability_id, status, approval_request_id, error_kind, payload::text FROM reborn_run_state_records WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND project_id = $4 AND mission_id = $5 AND thread_id = $6 AND invocation_id = $7 FOR UPDATE",
            &[
                &key.tenant_id,
                &key.user_id,
                &key.agent_id,
                &key.project_id,
                &key.mission_id,
                &key.thread_id,
                &invocation_id.to_string(),
            ],
        )
        .await
        .map_err(db_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let capability_id: String = row.get(0);
    let status: String = row.get(1);
    let approval_request_id: Option<String> = row.get(2);
    let error_kind: Option<String> = row.get(3);
    let payload: String = row.get(4);
    validate_run_row(
        from_json::<RunRecord>(&payload)?,
        scope,
        invocation_id,
        &capability_id,
        &status,
        approval_request_id.as_deref(),
        error_kind.as_deref(),
    )
    .map(Some)
}

#[cfg(feature = "postgres")]
async fn postgres_insert_run(
    client: &impl deadpool_postgres::GenericClient,
    record: &RunRecord,
) -> Result<bool, RunStateError> {
    let key = ScopeKey::new(&record.scope);
    let payload = to_json(record)?;
    let affected = client
        .execute(
            "INSERT INTO reborn_run_state_records (tenant_id, user_id, agent_id, project_id, mission_id, thread_id, invocation_id, capability_id, status, approval_request_id, error_kind, payload) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12::text::jsonb) ON CONFLICT DO NOTHING",
            &[
                &key.tenant_id,
                &key.user_id,
                &key.agent_id,
                &key.project_id,
                &key.mission_id,
                &key.thread_id,
                &record.invocation_id.to_string(),
                &record.capability_id.as_str(),
                &run_status_key(record.status),
                &record.approval_request_id.map(|id| id.to_string()),
                &record.error_kind,
                &payload,
            ],
        )
        .await
        .map_err(db_error)?;
    Ok(affected == 1)
}

#[cfg(feature = "postgres")]
async fn postgres_update_run(
    client: &impl deadpool_postgres::GenericClient,
    record: &RunRecord,
) -> Result<(), RunStateError> {
    let key = ScopeKey::new(&record.scope);
    let payload = to_json(record)?;
    let affected = client
        .execute(
            "UPDATE reborn_run_state_records SET capability_id = $8, status = $9, approval_request_id = $10, error_kind = $11, payload = $12::text::jsonb WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND project_id = $4 AND mission_id = $5 AND thread_id = $6 AND invocation_id = $7",
            &[
                &key.tenant_id,
                &key.user_id,
                &key.agent_id,
                &key.project_id,
                &key.mission_id,
                &key.thread_id,
                &record.invocation_id.to_string(),
                &record.capability_id.as_str(),
                &run_status_key(record.status),
                &record.approval_request_id.map(|id| id.to_string()),
                &record.error_kind,
                &payload,
            ],
        )
        .await
        .map_err(db_error)?;
    require_single_affected_row(affected, "update run")
}

#[cfg(feature = "postgres")]
async fn postgres_get_approval(
    client: &impl deadpool_postgres::GenericClient,
    scope: &ResourceScope,
    request_id: ApprovalRequestId,
) -> Result<Option<ApprovalRecord>, RunStateError> {
    let key = ScopeKey::new(scope);
    let row = client
        .query_opt(
            "SELECT status, payload::text FROM reborn_approval_request_records WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND project_id = $4 AND mission_id = $5 AND thread_id = $6 AND request_id = $7",
            &[
                &key.tenant_id,
                &key.user_id,
                &key.agent_id,
                &key.project_id,
                &key.mission_id,
                &key.thread_id,
                &request_id.to_string(),
            ],
        )
        .await
        .map_err(db_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let status: String = row.get(0);
    let payload: String = row.get(1);
    validate_approval_row(
        from_json::<ApprovalRecord>(&payload)?,
        scope,
        request_id,
        &status,
    )
    .map(Some)
}

#[cfg(feature = "postgres")]
async fn postgres_get_approval_for_update(
    client: &impl deadpool_postgres::GenericClient,
    scope: &ResourceScope,
    request_id: ApprovalRequestId,
) -> Result<Option<ApprovalRecord>, RunStateError> {
    let key = ScopeKey::new(scope);
    let row = client
        .query_opt(
            "SELECT status, payload::text FROM reborn_approval_request_records WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND project_id = $4 AND mission_id = $5 AND thread_id = $6 AND request_id = $7 FOR UPDATE",
            &[
                &key.tenant_id,
                &key.user_id,
                &key.agent_id,
                &key.project_id,
                &key.mission_id,
                &key.thread_id,
                &request_id.to_string(),
            ],
        )
        .await
        .map_err(db_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let status: String = row.get(0);
    let payload: String = row.get(1);
    validate_approval_row(
        from_json::<ApprovalRecord>(&payload)?,
        scope,
        request_id,
        &status,
    )
    .map(Some)
}

#[cfg(feature = "postgres")]
async fn postgres_insert_approval(
    client: &impl deadpool_postgres::GenericClient,
    record: &ApprovalRecord,
) -> Result<bool, RunStateError> {
    let key = ScopeKey::new(&record.scope);
    let payload = to_json(record)?;
    let affected = client
        .execute(
            "INSERT INTO reborn_approval_request_records (tenant_id, user_id, agent_id, project_id, mission_id, thread_id, request_id, status, payload) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::text::jsonb) ON CONFLICT DO NOTHING",
            &[
                &key.tenant_id,
                &key.user_id,
                &key.agent_id,
                &key.project_id,
                &key.mission_id,
                &key.thread_id,
                &record.request.id.to_string(),
                &approval_status_key(record.status),
                &payload,
            ],
        )
        .await
        .map_err(db_error)?;
    Ok(affected == 1)
}

#[cfg(feature = "postgres")]
async fn postgres_update_approval(
    client: &impl deadpool_postgres::GenericClient,
    record: &ApprovalRecord,
) -> Result<(), RunStateError> {
    let key = ScopeKey::new(&record.scope);
    let payload = to_json(record)?;
    let affected = client
        .execute(
            "UPDATE reborn_approval_request_records SET status = $8, payload = $9::text::jsonb WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND project_id = $4 AND mission_id = $5 AND thread_id = $6 AND request_id = $7",
            &[
                &key.tenant_id,
                &key.user_id,
                &key.agent_id,
                &key.project_id,
                &key.mission_id,
                &key.thread_id,
                &record.request.id.to_string(),
                &approval_status_key(record.status),
                &payload,
            ],
        )
        .await
        .map_err(db_error)?;
    require_single_affected_row(affected, "update approval")
}

fn parse_invocation_id_column(value: &str) -> Result<InvocationId, RunStateError> {
    InvocationId::parse(value).map_err(|error| {
        RunStateError::Deserialization(format!("invalid invocation_id column: {error}"))
    })
}

fn parse_approval_request_id_column(value: &str) -> Result<ApprovalRequestId, RunStateError> {
    ApprovalRequestId::parse(value).map_err(|error| {
        RunStateError::Deserialization(format!("invalid request_id column: {error}"))
    })
}

fn validate_run_row(
    record: RunRecord,
    expected_scope: &ResourceScope,
    row_invocation_id: InvocationId,
    row_capability_id: &str,
    row_status: &str,
    row_approval_request_id: Option<&str>,
    row_error_kind: Option<&str>,
) -> Result<RunRecord, RunStateError> {
    if !crate::same_scope_owner(&record.scope, expected_scope) {
        return Err(row_integrity_error("run-state", "scope columns"));
    }
    if record.invocation_id != row_invocation_id {
        return Err(row_integrity_error("run-state", "invocation_id"));
    }
    if record.capability_id.as_str() != row_capability_id {
        return Err(row_integrity_error("run-state", "capability_id"));
    }
    if run_status_key(record.status) != row_status {
        return Err(row_integrity_error("run-state", "status"));
    }
    let record_approval_request_id = record.approval_request_id.map(|id| id.to_string());
    if record_approval_request_id.as_deref() != row_approval_request_id {
        return Err(row_integrity_error("run-state", "approval_request_id"));
    }
    if record.error_kind.as_deref() != row_error_kind {
        return Err(row_integrity_error("run-state", "error_kind"));
    }
    Ok(record)
}

fn validate_approval_row(
    record: ApprovalRecord,
    expected_scope: &ResourceScope,
    row_request_id: ApprovalRequestId,
    row_status: &str,
) -> Result<ApprovalRecord, RunStateError> {
    if !crate::same_scope_owner(&record.scope, expected_scope) {
        return Err(row_integrity_error("approval-request", "scope columns"));
    }
    if record.request.id != row_request_id {
        return Err(row_integrity_error("approval-request", "request_id"));
    }
    if approval_status_key(record.status) != row_status {
        return Err(row_integrity_error("approval-request", "status"));
    }
    Ok(record)
}

fn row_integrity_error(entity: &'static str, field: &'static str) -> RunStateError {
    RunStateError::Deserialization(format!(
        "{entity} row payload does not match {field} column"
    ))
}

fn require_single_affected_row(
    affected: u64,
    operation: &'static str,
) -> Result<(), RunStateError> {
    if affected == 1 {
        Ok(())
    } else {
        Err(RunStateError::Backend(format!(
            "{operation} affected unexpected row count"
        )))
    }
}

struct ScopeKey<'a> {
    tenant_id: &'a str,
    user_id: &'a str,
    agent_id: &'a str,
    project_id: &'a str,
    mission_id: &'a str,
    thread_id: &'a str,
}

impl<'a> ScopeKey<'a> {
    fn new(scope: &'a ResourceScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.as_str(),
            user_id: scope.user_id.as_str(),
            agent_id: scope.agent_id.as_ref().map_or("", |id| id.as_str()),
            project_id: scope.project_id.as_ref().map_or("", |id| id.as_str()),
            mission_id: scope.mission_id.as_ref().map_or("", |id| id.as_str()),
            thread_id: scope.thread_id.as_ref().map_or("", |id| id.as_str()),
        }
    }
}

fn run_status_key(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Running => "running",
        RunStatus::BlockedApproval => "blocked_approval",
        RunStatus::BlockedAuth => "blocked_auth",
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
    }
}

fn approval_status_key(status: ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Pending => "pending",
        ApprovalStatus::Approved => "approved",
        ApprovalStatus::Denied => "denied",
        ApprovalStatus::Expired => "expired",
    }
}

fn to_json<T>(value: &T) -> Result<String, RunStateError>
where
    T: serde::Serialize,
{
    serde_json::to_string(value).map_err(|error| RunStateError::Serialization(error.to_string()))
}

fn from_json<T>(payload: &str) -> Result<T, RunStateError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(payload).map_err(|error| RunStateError::Deserialization(error.to_string()))
}

fn db_error(error: impl std::fmt::Display) -> RunStateError {
    tracing::debug!(%error, "run-state database operation failed");
    RunStateError::Backend("run-state database unavailable".to_string())
}
