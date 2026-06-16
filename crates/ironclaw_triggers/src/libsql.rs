#[cfg(feature = "libsql")]
use std::{collections::HashMap, sync::Arc};

#[cfg(feature = "libsql")]
use async_trait::async_trait;
#[cfg(feature = "libsql")]
use chrono::{DateTime, SecondsFormat, Utc};
#[cfg(feature = "libsql")]
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, Timestamp, UserId};
#[cfg(feature = "libsql")]
use ironclaw_turns::TurnRunId;
#[cfg(feature = "libsql")]
use libsql::params;

#[cfg(feature = "libsql")]
use crate::{
    ActiveTriggerScanCursor, ClaimDueFireOutcome, ClaimDueFireRequest, ClaimedTriggerFire,
    ClearActiveFireRequest, FireAcceptedRequest, FirePermanentFailedRequest, FireReplayedRequest,
    FireRetryableFailedRequest, FireTerminalFailedRequest, TriggerCompletionPolicy, TriggerError,
    TriggerId, TriggerRecord, TriggerRepository, TriggerRunHistoryStatus, TriggerRunRecord,
    TriggerRunStatus, TriggerSchedule, TriggerSourceKind, TriggerState,
    reject_failed_result_after_active_run, reject_non_future_next_run_at, reject_run_ref_rewrite,
    trigger_run_history_status_text,
};

#[cfg(feature = "libsql")]
const TRIGGER_TABLE: &str = "trigger_records";
#[cfg(feature = "libsql")]
const TRIGGER_RUN_TABLE: &str = "trigger_run_history";

#[cfg(feature = "libsql")]
const TRIGGER_COLUMNS: &str = "\
    trigger_id, tenant_id, creator_user_id, agent_id, project_id, \
    name, source, schedule_expression, schedule_timezone, completion_policy, prompt, \
    state, next_run_at, last_run_at, last_fired_slot, last_status, \
    active_fire_slot, active_run_ref, created_at";

#[cfg(feature = "libsql")]
const TRIGGER_ID_COL: usize = 0;
#[cfg(feature = "libsql")]
const TENANT_ID_COL: usize = 1;
#[cfg(feature = "libsql")]
const CREATOR_USER_ID_COL: usize = 2;
#[cfg(feature = "libsql")]
const AGENT_ID_COL: usize = 3;
#[cfg(feature = "libsql")]
const PROJECT_ID_COL: usize = 4;
#[cfg(feature = "libsql")]
const NAME_COL: usize = 5;
#[cfg(feature = "libsql")]
const SOURCE_COL: usize = 6;
#[cfg(feature = "libsql")]
const SCHEDULE_EXPRESSION_COL: usize = 7;
#[cfg(feature = "libsql")]
const SCHEDULE_TIMEZONE_COL: usize = 8;
#[cfg(feature = "libsql")]
const COMPLETION_POLICY_COL: usize = 9;
#[cfg(feature = "libsql")]
const PROMPT_COL: usize = 10;
#[cfg(feature = "libsql")]
const STATE_COL: usize = 11;
#[cfg(feature = "libsql")]
const NEXT_RUN_AT_COL: usize = 12;
#[cfg(feature = "libsql")]
const LAST_RUN_AT_COL: usize = 13;
#[cfg(feature = "libsql")]
const LAST_FIRED_SLOT_COL: usize = 14;
#[cfg(feature = "libsql")]
const LAST_STATUS_COL: usize = 15;
#[cfg(feature = "libsql")]
const ACTIVE_FIRE_SLOT_COL: usize = 16;
#[cfg(feature = "libsql")]
const ACTIVE_RUN_REF_COL: usize = 17;
#[cfg(feature = "libsql")]
const CREATED_AT_COL: usize = 18;

#[cfg(feature = "libsql")]
const TRIGGER_RUN_COLUMNS: &str = "\
    tenant_id, trigger_id, fire_slot, run_id, thread_id, status, submitted_at, completed_at";
#[cfg(feature = "libsql")]
const RUN_TENANT_ID_COL: usize = 0;
#[cfg(feature = "libsql")]
const RUN_TRIGGER_ID_COL: usize = 1;
#[cfg(feature = "libsql")]
const RUN_FIRE_SLOT_COL: usize = 2;
#[cfg(feature = "libsql")]
const RUN_ID_COL: usize = 3;
#[cfg(feature = "libsql")]
const RUN_THREAD_ID_COL: usize = 4;
#[cfg(feature = "libsql")]
const RUN_STATUS_COL: usize = 5;
#[cfg(feature = "libsql")]
const RUN_SUBMITTED_AT_COL: usize = 6;
#[cfg(feature = "libsql")]
const RUN_COMPLETED_AT_COL: usize = 7;

/// Durable libSQL trigger repository.
#[cfg(feature = "libsql")]
pub struct LibSqlTriggerRepository {
    db: Arc<libsql::Database>,
}

#[cfg(feature = "libsql")]
impl LibSqlTriggerRepository {
    pub fn new(db: Arc<libsql::Database>) -> Self {
        Self { db }
    }

    pub async fn run_migrations(&self) -> Result<(), TriggerError> {
        let conn = self.connect().await?;
        conn.execute("BEGIN IMMEDIATE", ())
            .await
            .map_err(|error| backend_error("begin trigger migration", error))?;

        let result = async {
            conn.execute(
                &format!(
                    "CREATE TABLE IF NOT EXISTS {TRIGGER_TABLE} (
                        trigger_id TEXT NOT NULL,
                        tenant_id TEXT NOT NULL,
                        creator_user_id TEXT NOT NULL,
                        agent_id TEXT,
                        project_id TEXT,
                        name TEXT NOT NULL,
                        source TEXT NOT NULL,
                        schedule_expression TEXT NOT NULL,
                        completion_policy TEXT NOT NULL,
                        prompt TEXT NOT NULL,
                        state TEXT NOT NULL,
                        next_run_at TEXT NOT NULL,
                        last_run_at TEXT,
                        last_fired_slot TEXT,
                        last_status TEXT,
                        active_fire_slot TEXT,
                        active_run_ref TEXT,
                        created_at TEXT NOT NULL,
                        PRIMARY KEY (tenant_id, trigger_id)
                    )"
                ),
                (),
            )
            .await
            .map_err(|error| backend_error("create trigger_records table", error))?;
            conn.execute(
                &format!(
                    "CREATE INDEX IF NOT EXISTS trigger_records_state_next_run_at_idx
                     ON {TRIGGER_TABLE} (state, next_run_at, tenant_id, trigger_id)"
                ),
                (),
            )
            .await
            .map_err(|error| backend_error("create trigger due index", error))?;
            conn.execute(
                &format!(
                    "CREATE INDEX IF NOT EXISTS trigger_records_tenant_created_at_idx
                     ON {TRIGGER_TABLE} (tenant_id, created_at, trigger_id)"
                ),
                (),
            )
            .await
            .map_err(|error| backend_error("create trigger tenant list index", error))?;
            conn.execute(
                &format!(
                    "CREATE INDEX IF NOT EXISTS trigger_records_scoped_list_idx
                     ON {TRIGGER_TABLE} (
                        tenant_id, creator_user_id, agent_id, project_id, created_at, trigger_id
                     )"
                ),
                (),
            )
            .await
            .map_err(|error| backend_error("create trigger scoped list index", error))?;
            conn.execute(
                &format!(
                    "CREATE INDEX IF NOT EXISTS trigger_records_active_fire_slot_idx
                     ON {TRIGGER_TABLE} (active_fire_slot, tenant_id, trigger_id)
                     WHERE active_fire_slot IS NOT NULL"
                ),
                (),
            )
            .await
            .map_err(|error| backend_error("create trigger active scan index", error))?;
            conn.execute(
                &format!(
                    "CREATE TABLE IF NOT EXISTS {TRIGGER_RUN_TABLE} (
                        tenant_id TEXT NOT NULL,
                        trigger_id TEXT NOT NULL,
                        fire_slot TEXT NOT NULL,
                        run_id TEXT,
                        thread_id TEXT NOT NULL,
                        status TEXT NOT NULL,
                        submitted_at TEXT NOT NULL,
                        completed_at TEXT,
                        PRIMARY KEY (tenant_id, trigger_id, fire_slot)
                    )"
                ),
                (),
            )
            .await
            .map_err(|error| backend_error("create trigger_run_history table", error))?;
            conn.execute(
                &format!(
                    "CREATE INDEX IF NOT EXISTS trigger_run_history_trigger_fire_slot_idx
                     ON {TRIGGER_RUN_TABLE} (tenant_id, trigger_id, fire_slot DESC)"
                ),
                (),
            )
            .await
            .map_err(|error| backend_error("create trigger run history list index", error))?;
            // Index supporting find_trigger_run_by_thread_id — idempotent.
            // thread_id is nullable; WHERE tenant_id = ? AND thread_id = ? naturally
            // skips NULL rows so no partial-index condition is needed.
            conn.execute(
                &format!(
                    "CREATE INDEX IF NOT EXISTS trigger_run_history_tenant_thread_id_idx
                     ON {TRIGGER_RUN_TABLE} (tenant_id, thread_id)"
                ),
                (),
            )
            .await
            .map_err(|error| {
                backend_error("create trigger run history thread_id index", error)
            })?;
            // Add schedule_timezone column if it doesn't already exist (idempotent migration).
            // SQLite does not support ADD COLUMN IF NOT EXISTS, so we attempt the ALTER and
            // ignore the "duplicate column" error that indicates it was already applied.
            if let Err(error) = conn
                .execute(
                    &format!(
                        "ALTER TABLE {TRIGGER_TABLE} ADD COLUMN schedule_timezone TEXT NOT NULL DEFAULT 'UTC'"
                    ),
                    (),
                )
                .await
            {
                let msg = error.to_string();
                if !msg.contains("duplicate column") && !msg.contains("already exists") {
                    return Err(backend_error("add schedule_timezone column", error));
                }
            }
            // Make thread_id nullable in trigger_run_history if it was created NOT NULL.
            // SQLite does not support ALTER COLUMN, so we rebuild the table when the
            // notnull constraint is still set on that column.
            let needs_thread_id_migration = {
                let mut rows = conn
                    .query(
                        &format!("PRAGMA table_info({TRIGGER_RUN_TABLE})"),
                        (),
                    )
                    .await
                    .map_err(|error| backend_error("pragma trigger_run_history table_info", error))?;
                // PRAGMA table_info returns columns: cid, name, type, notnull, dflt_value, pk.
                // notnull=1 means the column has NOT NULL. We iterate until we find thread_id.
                let mut found_not_null = false;
                while let Some(row) = rows
                    .next()
                    .await
                    .map_err(|error| backend_error("read pragma trigger_run_history table_info", error))?
                {
                    let col_name: String = row.get(1).map_err(|error| {
                        backend_error("read pragma column name", error)
                    })?;
                    if col_name == "thread_id" {
                        let not_null: i64 = row.get(3).map_err(|error| {
                            backend_error("read pragma notnull flag", error)
                        })?;
                        found_not_null = not_null != 0;
                        break;
                    }
                }
                found_not_null
            };
            if needs_thread_id_migration {
                // Rebuild trigger_run_history with thread_id nullable.
                conn.execute_batch(&format!(
                    "CREATE TABLE {TRIGGER_RUN_TABLE}_new (
                        tenant_id TEXT NOT NULL,
                        trigger_id TEXT NOT NULL,
                        fire_slot TEXT NOT NULL,
                        run_id TEXT,
                        thread_id TEXT,
                        status TEXT NOT NULL,
                        submitted_at TEXT NOT NULL,
                        completed_at TEXT,
                        PRIMARY KEY (tenant_id, trigger_id, fire_slot)
                    );
                    INSERT INTO {TRIGGER_RUN_TABLE}_new
                        SELECT tenant_id, trigger_id, fire_slot, run_id, thread_id, status, submitted_at, completed_at
                        FROM {TRIGGER_RUN_TABLE};
                    DROP TABLE {TRIGGER_RUN_TABLE};
                    ALTER TABLE {TRIGGER_RUN_TABLE}_new RENAME TO {TRIGGER_RUN_TABLE};
                    CREATE INDEX IF NOT EXISTS trigger_run_history_trigger_fire_slot_idx
                        ON {TRIGGER_RUN_TABLE} (tenant_id, trigger_id, fire_slot DESC);
                    CREATE INDEX IF NOT EXISTS trigger_run_history_tenant_thread_id_idx
                        ON {TRIGGER_RUN_TABLE} (tenant_id, thread_id);"
                ))
                .await
                .map_err(|error| backend_error("make trigger_run_history thread_id nullable", error))?;
            }
            Ok::<(), TriggerError>(())
        }
        .await;

        match result {
            Ok(()) => conn
                .execute("COMMIT", ())
                .await
                .map(|_| ())
                .map_err(|error| backend_error("commit trigger migration", error)),
            Err(error) => {
                if let Err(rollback_error) = conn.execute("ROLLBACK", ()).await {
                    tracing::debug!(
                        migration_error = %error,
                        rollback_error = %rollback_error,
                        "ROLLBACK failed after libSQL trigger migration error"
                    );
                }
                Err(error)
            }
        }
    }

    async fn connect(&self) -> Result<libsql::Connection, TriggerError> {
        let conn = self
            .db
            .connect()
            .map_err(|error| backend_error("connect trigger repository", error))?;
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(|error| backend_error("set trigger repository busy_timeout", error))?;
        Ok(conn)
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl TriggerRepository for LibSqlTriggerRepository {
    async fn upsert_trigger(&self, record: TriggerRecord) -> Result<(), TriggerError> {
        record.validate()?;
        let conn = self.connect().await?;
        write_record(&conn, &record).await?;
        Ok(())
    }

    async fn get_trigger(
        &self,
        tenant_id: TenantId,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRIGGER_COLUMNS}
                     FROM {TRIGGER_TABLE}
                     WHERE tenant_id = ?1 AND trigger_id = ?2
                     LIMIT 1"
                ),
                params![tenant_id.as_str(), trigger_id.to_string()],
            )
            .await
            .map_err(|error| backend_error("query trigger record", error))?;
        match rows.next().await {
            Ok(Some(row)) => Ok(Some(row_to_record(&row)?)),
            Ok(None) => Ok(None),
            Err(error) => Err(backend_error("read trigger record row", error)),
        }
    }

    async fn list_triggers(&self, tenant_id: TenantId) -> Result<Vec<TriggerRecord>, TriggerError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRIGGER_COLUMNS}
                     FROM {TRIGGER_TABLE}
                     WHERE tenant_id = ?1
                     ORDER BY created_at, trigger_id"
                ),
                params![tenant_id.as_str()],
            )
            .await
            .map_err(|error| backend_error("query tenant trigger records", error))?;
        let mut records = Vec::new();
        loop {
            match rows.next().await {
                Ok(Some(row)) => records.push(row_to_record(&row)?),
                Ok(None) => break,
                Err(error) => return Err(backend_error("read tenant trigger record row", error)),
            }
        }
        Ok(records)
    }

    async fn list_scoped_triggers(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(crate::MAX_TRIGGER_LIST_LIMIT) as i64;
        let conn = self.connect().await?;
        let agent_id = agent_id.as_ref().map(AgentId::as_str);
        let project_id = project_id.as_ref().map(ProjectId::as_str);
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRIGGER_COLUMNS}
                     FROM {TRIGGER_TABLE}
                     WHERE tenant_id = ?1
                       AND creator_user_id = ?2
                       AND agent_id IS ?3
                       AND project_id IS ?4
                     ORDER BY created_at, trigger_id
                     LIMIT ?5"
                ),
                params![
                    tenant_id.as_str(),
                    creator_user_id.as_str(),
                    agent_id,
                    project_id,
                    limit
                ],
            )
            .await
            .map_err(|error| backend_error("query scoped trigger records", error))?;
        let mut records = Vec::new();
        loop {
            match rows.next().await {
                Ok(Some(row)) => records.push(row_to_record(&row)?),
                Ok(None) => break,
                Err(error) => return Err(backend_error("read scoped trigger record row", error)),
            }
        }
        Ok(records)
    }

    async fn remove_trigger(
        &self,
        tenant_id: TenantId,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "DELETE FROM {TRIGGER_TABLE}
                     WHERE tenant_id = ?1 AND trigger_id = ?2
                     RETURNING {TRIGGER_COLUMNS}"
                ),
                params![tenant_id.as_str(), trigger_id.to_string()],
            )
            .await
            .map_err(|error| backend_error("remove trigger record", error))?;
        match rows.next().await {
            Ok(Some(row)) => Ok(Some(row_to_record(&row)?)),
            Ok(None) => Ok(None),
            Err(error) => Err(backend_error("read removed trigger record row", error)),
        }
    }

    async fn remove_scoped_trigger(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let conn = self.connect().await?;
        let agent_id = agent_id.as_ref().map(AgentId::as_str);
        let project_id = project_id.as_ref().map(ProjectId::as_str);
        let mut rows = conn
            .query(
                &format!(
                    "DELETE FROM {TRIGGER_TABLE}
                     WHERE tenant_id = ?1
                       AND creator_user_id = ?2
                       AND agent_id IS ?3
                       AND project_id IS ?4
                       AND trigger_id = ?5
                     RETURNING {TRIGGER_COLUMNS}"
                ),
                params![
                    tenant_id.as_str(),
                    creator_user_id.as_str(),
                    agent_id,
                    project_id,
                    trigger_id.to_string(),
                ],
            )
            .await
            .map_err(|error| backend_error("remove scoped trigger record", error))?;
        match rows.next().await {
            Ok(Some(row)) => Ok(Some(row_to_record(&row)?)),
            Ok(None) => Ok(None),
            Err(error) => Err(backend_error(
                "read removed scoped trigger record row",
                error,
            )),
        }
    }

    async fn list_due_triggers(
        &self,
        now: Timestamp,
        limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(super::MAX_DUE_TRIGGER_POLL_LIMIT);
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRIGGER_COLUMNS}
                     FROM {TRIGGER_TABLE}
                     WHERE state = ?1
                       AND next_run_at <= ?2
                       AND active_fire_slot IS NULL
                       AND active_run_ref IS NULL
                     ORDER BY next_run_at, tenant_id, trigger_id
                     LIMIT ?3"
                ),
                params![
                    state_text(TriggerState::Scheduled),
                    fmt_ts(&now),
                    limit as i64,
                ],
            )
            .await
            .map_err(|error| backend_error("query due trigger records", error))?;
        let mut records = Vec::new();
        loop {
            match rows.next().await {
                Ok(Some(row)) => records.push(row_to_record(&row)?),
                Ok(None) => break,
                Err(error) => return Err(backend_error("read due trigger record row", error)),
            }
        }
        Ok(records)
    }

    async fn list_active_triggers(&self, limit: usize) -> Result<Vec<TriggerRecord>, TriggerError> {
        self.list_active_triggers_after(None, limit).await
    }

    async fn list_active_triggers_after(
        &self,
        after: Option<ActiveTriggerScanCursor>,
        limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(super::MAX_DUE_TRIGGER_POLL_LIMIT);
        let conn = self.connect().await?;
        let mut rows = match after {
            Some(cursor) => {
                conn.query(
                    &format!(
                        "SELECT {TRIGGER_COLUMNS}
                         FROM {TRIGGER_TABLE}
                         WHERE active_fire_slot IS NOT NULL
                           AND (
                             active_fire_slot > ?1
                             OR (active_fire_slot = ?1 AND tenant_id > ?2)
                             OR (active_fire_slot = ?1 AND tenant_id = ?2 AND trigger_id > ?3)
                           )
                         ORDER BY active_fire_slot, tenant_id, trigger_id
                         LIMIT ?4"
                    ),
                    params![
                        fmt_ts(&cursor.active_fire_slot()),
                        cursor.tenant_id().as_str(),
                        cursor.trigger_id().to_string(),
                        limit as i64,
                    ],
                )
                .await
            }
            None => {
                conn.query(
                    &format!(
                        "SELECT {TRIGGER_COLUMNS}
                         FROM {TRIGGER_TABLE}
                         WHERE active_fire_slot IS NOT NULL
                         ORDER BY active_fire_slot, tenant_id, trigger_id
                         LIMIT ?1"
                    ),
                    params![limit as i64],
                )
                .await
            }
        }
        .map_err(|error| backend_error("query active trigger records", error))?;
        let mut records = Vec::new();
        loop {
            match rows.next().await {
                Ok(Some(row)) => records.push(row_to_record(&row)?),
                Ok(None) => break,
                Err(error) => return Err(backend_error("read active trigger record row", error)),
            }
        }
        Ok(records)
    }

    async fn claim_due_fire(
        &self,
        request: ClaimDueFireRequest,
    ) -> Result<ClaimDueFireOutcome, TriggerError> {
        let conn = self.connect().await?;
        let fire_slot = fmt_ts(&request.fire_slot);
        let now = fmt_ts(&request.now);
        begin_immediate(&conn, "begin trigger fire claim").await?;
        let claim_result = async {
            let mut rows = conn
                .query(
                    &format!(
                        "UPDATE {TRIGGER_TABLE}
                         SET active_fire_slot = ?4,
                             active_run_ref = NULL
                         WHERE tenant_id = ?1
                           AND trigger_id = ?2
                           AND state = ?3
                           AND next_run_at = ?4
                           AND ?4 <= ?5
                           AND active_fire_slot IS NULL
                           AND active_run_ref IS NULL
                         RETURNING {TRIGGER_COLUMNS}"
                    ),
                    params![
                        request.tenant_id.as_str(),
                        request.trigger_id.to_string(),
                        state_text(TriggerState::Scheduled),
                        fire_slot,
                        now,
                    ],
                )
                .await
                .map_err(|error| backend_error("claim trigger fire", error))?;
            let Some(record) = returned_record(&mut rows, "read claimed trigger fire").await?
            else {
                return Ok(None);
            };
            upsert_run_history(
                &conn,
                &TriggerRunRecord::running(
                    request.tenant_id.clone(),
                    request.trigger_id,
                    request.fire_slot,
                    None,
                    request.now,
                ),
            )
            .await?;
            Ok(Some(record))
        }
        .await;
        match claim_result {
            Ok(Some(record)) => {
                commit(&conn, "commit trigger fire claim").await?;
                return Ok(ClaimDueFireOutcome::Claimed(ClaimedTriggerFire {
                    record,
                    fire_slot: request.fire_slot,
                }));
            }
            Ok(None) => rollback(&conn, "rollback missed trigger fire claim").await?,
            Err(error) => {
                rollback(&conn, "rollback failed trigger fire claim").await?;
                return Err(error);
            }
        }

        let Some(record) = fetch_record(&conn, &request.tenant_id, request.trigger_id).await?
        else {
            return Ok(ClaimDueFireOutcome::NotFound);
        };
        if record.state != TriggerState::Scheduled
            || record.next_run_at != request.fire_slot
            || request.fire_slot > request.now
        {
            return Ok(ClaimDueFireOutcome::NotDue { record });
        }
        if record.has_active_fire() {
            return Ok(ClaimDueFireOutcome::AlreadyActive {
                active_fire_slot: record.active_fire_slot,
                active_run_ref: record.active_run_ref,
            });
        }
        // A competing poller can claim and clear the fire before this
        // diagnostic read. The row may be due again, but this attempt did not
        // claim it; let a later poll cycle observe it normally.
        Ok(ClaimDueFireOutcome::NotDue { record })
    }

    async fn mark_fire_accepted(
        &self,
        request: FireAcceptedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let conn = self.connect().await?;
        mark_successful_fire_result(
            &conn,
            SuccessfulFireResultUpdate {
                tenant_id: &request.tenant_id,
                trigger_id: request.trigger_id,
                fire_slot: request.fire_slot,
                run_id: request.run_id,
                thread_id: Some(request.thread_id),
                result_at: request.submitted_at,
                next_run_at: request.next_run_at,
                update_operation: "mark accepted trigger fire",
                read_operation: "read accepted trigger fire",
            },
        )
        .await
    }

    async fn mark_fire_replayed(
        &self,
        request: FireReplayedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let conn = self.connect().await?;
        mark_successful_fire_result(
            &conn,
            SuccessfulFireResultUpdate {
                tenant_id: &request.tenant_id,
                trigger_id: request.trigger_id,
                fire_slot: request.fire_slot,
                run_id: request.original_run_id,
                thread_id: request.thread_id,
                result_at: request.replayed_at,
                next_run_at: request.next_run_at,
                update_operation: "mark replayed trigger fire",
                read_operation: "read replayed trigger fire",
            },
        )
        .await
    }

    async fn mark_fire_retryable_failed(
        &self,
        request: FireRetryableFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let FireRetryableFailedRequest {
            tenant_id,
            trigger_id,
            fire_slot,
        } = request;
        let conn = self.connect().await?;
        let Some(record) = fetch_record(&conn, &tenant_id, trigger_id).await? else {
            return Ok(None);
        };
        if record.active_fire_slot != Some(fire_slot) {
            return Ok(None);
        }
        reject_failed_result_after_active_run(record.active_run_ref)?;
        if record.next_run_at > fire_slot {
            return Err(TriggerError::InvalidRecord {
                reason: "retryable fire failure must leave next_run_at at or before the failed fire slot"
                    .to_string(),
            });
        }

        let fire_slot_text = fmt_ts(&fire_slot);
        let last_status = status_text(TriggerRunStatus::Error);
        begin_immediate(&conn, "begin retryable trigger fire failure").await?;
        let update_result = async {
            let mut rows = conn
                .query(
                    &format!(
                        "UPDATE {TRIGGER_TABLE}
                         SET last_status = ?3,
                             active_fire_slot = NULL,
                             active_run_ref = NULL
                         WHERE tenant_id = ?1
                           AND trigger_id = ?2
                           AND active_fire_slot = ?4
                           AND active_run_ref IS NULL
                           AND next_run_at <= ?4
                         RETURNING {TRIGGER_COLUMNS}"
                    ),
                    params![
                        tenant_id.as_str(),
                        trigger_id.to_string(),
                        last_status,
                        fire_slot_text,
                    ],
                )
                .await
                .map_err(|error| backend_error("mark retryable trigger fire failure", error))?;
            let Some(record) =
                returned_record(&mut rows, "read retryable trigger fire failure").await?
            else {
                return Ok(None);
            };
            complete_run_history(
                &conn,
                &tenant_id,
                trigger_id,
                fire_slot,
                None,
                TriggerRunHistoryStatus::Error,
                Utc::now(),
            )
            .await?;
            Ok(Some(record))
        }
        .await;
        match update_result {
            Ok(Some(record)) => {
                commit(&conn, "commit retryable trigger fire failure").await?;
                return Ok(Some(record));
            }
            Ok(None) => rollback(&conn, "rollback missed retryable trigger fire failure").await?,
            Err(error) => {
                rollback(&conn, "rollback failed retryable trigger fire failure").await?;
                return Err(error);
            }
        }
        resolve_missed_fire_result_update(&conn, &tenant_id, trigger_id, fire_slot, None, None)
            .await
    }

    async fn mark_fire_permanently_failed(
        &self,
        request: FirePermanentFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let FirePermanentFailedRequest {
            tenant_id,
            trigger_id,
            fire_slot,
            next_run_at,
        } = request;
        let conn = self.connect().await?;
        let Some(record) = fetch_record(&conn, &tenant_id, trigger_id).await? else {
            return Ok(None);
        };
        if record.active_fire_slot != Some(fire_slot) {
            return Ok(None);
        }
        reject_failed_result_after_active_run(record.active_run_ref)?;
        reject_non_future_next_run_at(fire_slot, next_run_at)?;

        let fire_slot_text = fmt_ts(&fire_slot);
        let next_run_at = fmt_ts(&next_run_at);
        let last_status = status_text(TriggerRunStatus::Error);
        begin_immediate(&conn, "begin permanent trigger fire failure").await?;
        let update_result = async {
            let mut rows = conn
                .query(
                    &format!(
                        "UPDATE {TRIGGER_TABLE}
                         SET last_status = ?3,
                             next_run_at = ?5,
                             active_fire_slot = NULL,
                             active_run_ref = NULL
                         WHERE tenant_id = ?1
                           AND trigger_id = ?2
                           AND active_fire_slot = ?4
                           AND active_run_ref IS NULL
                         RETURNING {TRIGGER_COLUMNS}"
                    ),
                    params![
                        tenant_id.as_str(),
                        trigger_id.to_string(),
                        last_status,
                        fire_slot_text,
                        next_run_at,
                    ],
                )
                .await
                .map_err(|error| backend_error("mark permanent trigger fire failure", error))?;
            let Some(record) =
                returned_record(&mut rows, "read permanent trigger fire failure").await?
            else {
                return Ok(None);
            };
            complete_run_history(
                &conn,
                &tenant_id,
                trigger_id,
                fire_slot,
                None,
                TriggerRunHistoryStatus::Error,
                Utc::now(),
            )
            .await?;
            Ok(Some(record))
        }
        .await;
        match update_result {
            Ok(Some(record)) => {
                commit(&conn, "commit permanent trigger fire failure").await?;
                return Ok(Some(record));
            }
            Ok(None) => rollback(&conn, "rollback missed permanent trigger fire failure").await?,
            Err(error) => {
                rollback(&conn, "rollback failed permanent trigger fire failure").await?;
                return Err(error);
            }
        }
        resolve_missed_fire_result_update(&conn, &tenant_id, trigger_id, fire_slot, None, None)
            .await
    }

    async fn mark_fire_terminally_failed(
        &self,
        request: FireTerminalFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let FireTerminalFailedRequest {
            tenant_id,
            trigger_id,
            fire_slot,
        } = request;
        let fire_slot_text = fmt_ts(&fire_slot);
        let last_status = status_text(TriggerRunStatus::Error);
        let completed = state_text(TriggerState::Completed);
        let conn = self.connect().await?;
        begin_immediate(&conn, "begin terminal trigger fire failure").await?;
        let update_result = async {
            let mut rows = conn
                .query(
                    &format!(
                        "UPDATE {TRIGGER_TABLE}
                         SET state = ?3,
                             last_status = ?4,
                             active_fire_slot = NULL,
                             active_run_ref = NULL
                         WHERE tenant_id = ?1
                           AND trigger_id = ?2
                           AND active_fire_slot = ?5
                           AND active_run_ref IS NULL
                         RETURNING {TRIGGER_COLUMNS}"
                    ),
                    params![
                        tenant_id.as_str(),
                        trigger_id.to_string(),
                        completed,
                        last_status,
                        fire_slot_text,
                    ],
                )
                .await
                .map_err(|error| backend_error("mark terminal trigger fire failure", error))?;
            let Some(record) =
                returned_record(&mut rows, "read terminal trigger fire failure").await?
            else {
                return Ok(None);
            };
            complete_run_history(
                &conn,
                &tenant_id,
                trigger_id,
                fire_slot,
                None,
                TriggerRunHistoryStatus::Error,
                Utc::now(),
            )
            .await?;
            Ok(Some(record))
        }
        .await;
        match update_result {
            Ok(Some(record)) => {
                commit(&conn, "commit terminal trigger fire failure").await?;
                return Ok(Some(record));
            }
            Ok(None) => rollback(&conn, "rollback missed terminal trigger fire failure").await?,
            Err(error) => {
                rollback(&conn, "rollback failed terminal trigger fire failure").await?;
                return Err(error);
            }
        }
        resolve_missed_fire_result_update(&conn, &tenant_id, trigger_id, fire_slot, None, None)
            .await
    }

    async fn clear_active_fire(
        &self,
        request: ClearActiveFireRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let conn = self.connect().await?;
        begin_immediate(&conn, "begin clear active trigger fire").await?;
        let clear_result = async {
            // Keep active-fire clearing atomic as one predicate-guarded write.
            let mut rows = conn
                .query(
                    &format!(
                        "UPDATE {TRIGGER_TABLE}
                         SET active_fire_slot = NULL,
                             active_run_ref = NULL
                         WHERE tenant_id = ?1
                           AND trigger_id = ?2
                           AND active_fire_slot = ?3
                           AND active_run_ref = ?4
                         RETURNING {TRIGGER_COLUMNS}"
                    ),
                    params![
                        request.tenant_id.as_str(),
                        request.trigger_id.to_string(),
                        fmt_ts(&request.fire_slot),
                        request.run_id.to_string(),
                    ],
                )
                .await
                .map_err(|error| backend_error("clear active trigger fire", error))?;
            let Some(record) = returned_record(&mut rows, "read cleared trigger fire").await?
            else {
                return Ok(None);
            };
            complete_run_history(
                &conn,
                &request.tenant_id,
                request.trigger_id,
                request.fire_slot,
                Some(request.run_id),
                request.status,
                Utc::now(),
            )
            .await?;
            Ok(Some(record))
        }
        .await;
        match clear_result {
            Ok(Some(record)) => {
                commit(&conn, "commit clear active trigger fire").await?;
                Ok(Some(record))
            }
            Ok(None) => {
                rollback(&conn, "rollback missed clear active trigger fire").await?;
                Ok(None)
            }
            Err(error) => {
                rollback(&conn, "rollback failed clear active trigger fire").await?;
                Err(error)
            }
        }
    }

    async fn find_trigger_run_by_thread_id(
        &self,
        tenant_id: TenantId,
        thread_id: &crate::ThreadId,
    ) -> Result<Option<(crate::TriggerRecord, crate::TriggerRunRecord)>, crate::TriggerError> {
        let conn = self.connect().await?;
        // Look up the run row by (tenant_id, thread_id) using the dedicated index.
        let mut run_rows = conn
            .query(
                &format!(
                    "SELECT {TRIGGER_RUN_COLUMNS}
                     FROM {TRIGGER_RUN_TABLE}
                     WHERE tenant_id = ?1 AND thread_id = ?2
                     LIMIT 1"
                ),
                params![tenant_id.as_str(), thread_id.as_str()],
            )
            .await
            .map_err(|error| backend_error("query trigger run by thread_id", error))?;
        let run = match run_rows.next().await {
            Ok(Some(row)) => row_to_run_record(&row)?,
            Ok(None) => return Ok(None),
            Err(error) => return Err(backend_error("read trigger run by thread_id row", error)),
        };
        // Then load the parent trigger record.
        let mut trigger_rows = conn
            .query(
                &format!(
                    "SELECT {TRIGGER_COLUMNS}
                     FROM {TRIGGER_TABLE}
                     WHERE tenant_id = ?1 AND trigger_id = ?2
                     LIMIT 1"
                ),
                params![tenant_id.as_str(), run.trigger_id.to_string()],
            )
            .await
            .map_err(|error| backend_error("query parent trigger for thread_id lookup", error))?;
        match trigger_rows.next().await {
            Ok(Some(row)) => Ok(Some((row_to_record(&row)?, run))),
            Ok(None) => Ok(None),
            Err(error) => Err(backend_error(
                "read parent trigger record for thread_id lookup",
                error,
            )),
        }
    }

    async fn list_trigger_run_history(
        &self,
        tenant_id: TenantId,
        trigger_id: TriggerId,
        limit: usize,
    ) -> Result<Vec<TriggerRunRecord>, TriggerError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(crate::MAX_TRIGGER_RUN_HISTORY_LIMIT) as i64;
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRIGGER_RUN_COLUMNS}
                     FROM {TRIGGER_RUN_TABLE}
                     WHERE tenant_id = ?1 AND trigger_id = ?2
                     ORDER BY fire_slot DESC
                     LIMIT ?3"
                ),
                params![tenant_id.as_str(), trigger_id.to_string(), limit],
            )
            .await
            .map_err(|error| backend_error("query trigger run history", error))?;
        let mut runs = Vec::new();
        loop {
            match rows.next().await {
                Ok(Some(row)) => runs.push(row_to_run_record(&row)?),
                Ok(None) => break,
                Err(error) => return Err(backend_error("read trigger run history row", error)),
            }
        }
        Ok(runs)
    }

    async fn list_trigger_run_history_batch(
        &self,
        tenant_id: TenantId,
        trigger_ids: &[TriggerId],
        limit: usize,
    ) -> Result<HashMap<TriggerId, Vec<TriggerRunRecord>>, TriggerError> {
        let mut runs_by_trigger = HashMap::with_capacity(trigger_ids.len());
        if limit == 0 || trigger_ids.is_empty() {
            return Ok(runs_by_trigger);
        }
        let limit = limit.min(crate::MAX_TRIGGER_RUN_HISTORY_LIMIT) as i64;
        let trigger_ids_json = trigger_ids_json_array(trigger_ids);
        let sql = format!(
            "SELECT {TRIGGER_RUN_COLUMNS}
             FROM (
                 SELECT {TRIGGER_RUN_COLUMNS},
                        ROW_NUMBER() OVER (PARTITION BY trigger_id ORDER BY fire_slot DESC) AS row_rank
                 FROM {TRIGGER_RUN_TABLE}
                 WHERE tenant_id = ?1 AND trigger_id IN (SELECT value FROM json_each(?2))
             )
             WHERE row_rank <= ?3
             ORDER BY trigger_id, fire_slot DESC"
        );
        let conn = self.connect().await?;
        let mut rows = conn
            .query(&sql, params![tenant_id.as_str(), trigger_ids_json, limit])
            .await
            .map_err(|error| backend_error("query trigger run history batch", error))?;
        loop {
            match rows.next().await {
                Ok(Some(row)) => {
                    let run = row_to_run_record(&row)?;
                    runs_by_trigger.entry(run.trigger_id).or_default().push(run);
                }
                Ok(None) => break,
                Err(error) => {
                    return Err(backend_error("read trigger run history batch row", error));
                }
            }
        }
        Ok(runs_by_trigger)
    }
}

#[cfg(feature = "libsql")]
fn row_to_record(row: &libsql::Row) -> Result<TriggerRecord, TriggerError> {
    let trigger_id = TriggerId::parse(&required_text(row, TRIGGER_ID_COL, "trigger_id")?)?;
    let tenant_id = TenantId::new(required_text(row, TENANT_ID_COL, "tenant_id")?)
        .map_err(|error| invalid_record("tenant_id", error.to_string()))?;
    let creator_user_id = UserId::new(required_text(row, CREATOR_USER_ID_COL, "creator_user_id")?)
        .map_err(|error| invalid_record("creator_user_id", error.to_string()))?;
    let agent_id = optional_text(row, AGENT_ID_COL, "agent_id")?
        .map(|value| {
            AgentId::new(value).map_err(|error| invalid_record("agent_id", error.to_string()))
        })
        .transpose()?;
    let project_id = optional_text(row, PROJECT_ID_COL, "project_id")?
        .map(|value| {
            ProjectId::new(value).map_err(|error| invalid_record("project_id", error.to_string()))
        })
        .transpose()?;
    let schedule_expression = required_text(row, SCHEDULE_EXPRESSION_COL, "schedule_expression")?;
    let schedule_timezone = required_text(row, SCHEDULE_TIMEZONE_COL, "schedule_timezone")?;
    let schedule = TriggerSchedule::cron_with_timezone(schedule_expression, schedule_timezone)?;
    let last_run_at = optional_text(row, LAST_RUN_AT_COL, "last_run_at")?
        .map(|value| parse_timestamp(&value, "last_run_at"))
        .transpose()?;
    let last_fired_slot = optional_text(row, LAST_FIRED_SLOT_COL, "last_fired_slot")?
        .map(|value| parse_timestamp(&value, "last_fired_slot"))
        .transpose()?;
    let last_status = optional_text(row, LAST_STATUS_COL, "last_status")?
        .map(|value| parse_run_status(&value))
        .transpose()?;
    let active_fire_slot = optional_text(row, ACTIVE_FIRE_SLOT_COL, "active_fire_slot")?
        .map(|value| parse_timestamp(&value, "active_fire_slot"))
        .transpose()?;
    let active_run_ref = optional_text(row, ACTIVE_RUN_REF_COL, "active_run_ref")?
        .map(|value| parse_turn_run_id(&value))
        .transpose()?;

    let record = TriggerRecord {
        trigger_id,
        tenant_id,
        creator_user_id,
        agent_id,
        project_id,
        name: required_text(row, NAME_COL, "name")?,
        source: parse_source_kind(&required_text(row, SOURCE_COL, "source")?)?,
        schedule,
        completion_policy: parse_completion_policy(&required_text(
            row,
            COMPLETION_POLICY_COL,
            "completion_policy",
        )?)?,
        prompt: required_text(row, PROMPT_COL, "prompt")?,
        state: parse_state(&required_text(row, STATE_COL, "state")?)?,
        next_run_at: parse_timestamp(
            &required_text(row, NEXT_RUN_AT_COL, "next_run_at")?,
            "next_run_at",
        )?,
        last_run_at,
        last_fired_slot,
        last_status,
        active_fire_slot,
        active_run_ref,
        created_at: parse_timestamp(
            &required_text(row, CREATED_AT_COL, "created_at")?,
            "created_at",
        )?,
    };
    record.validate()?;
    Ok(record)
}

#[cfg(feature = "libsql")]
async fn fetch_record(
    conn: &libsql::Connection,
    tenant_id: &TenantId,
    trigger_id: TriggerId,
) -> Result<Option<TriggerRecord>, TriggerError> {
    let mut rows = conn
        .query(
            &format!(
                "SELECT {TRIGGER_COLUMNS}
                 FROM {TRIGGER_TABLE}
                 WHERE tenant_id = ?1 AND trigger_id = ?2
                 LIMIT 1"
            ),
            params![tenant_id.as_str(), trigger_id.to_string()],
        )
        .await
        .map_err(|error| backend_error("query trigger record", error))?;
    match rows.next().await {
        Ok(Some(row)) => Ok(Some(row_to_record(&row)?)),
        Ok(None) => Ok(None),
        Err(error) => Err(backend_error("read trigger record row", error)),
    }
}

#[cfg(feature = "libsql")]
async fn returned_record(
    rows: &mut libsql::Rows,
    operation: &str,
) -> Result<Option<TriggerRecord>, TriggerError> {
    match rows.next().await {
        Ok(Some(row)) => Ok(Some(row_to_record(&row)?)),
        Ok(None) => Ok(None),
        Err(error) => Err(backend_error(operation, error)),
    }
}

#[cfg(feature = "libsql")]
async fn begin_immediate(conn: &libsql::Connection, operation: &str) -> Result<(), TriggerError> {
    conn.execute("BEGIN IMMEDIATE", ())
        .await
        .map(|_| ())
        .map_err(|error| backend_error(operation, error))
}

#[cfg(feature = "libsql")]
async fn commit(conn: &libsql::Connection, operation: &str) -> Result<(), TriggerError> {
    conn.execute("COMMIT", ())
        .await
        .map(|_| ())
        .map_err(|error| backend_error(operation, error))
}

#[cfg(feature = "libsql")]
async fn rollback(conn: &libsql::Connection, operation: &str) -> Result<(), TriggerError> {
    conn.execute("ROLLBACK", ())
        .await
        .map(|_| ())
        .map_err(|error| backend_error(operation, error))
}

#[cfg(feature = "libsql")]
async fn write_record(
    conn: &libsql::Connection,
    record: &TriggerRecord,
) -> Result<(), TriggerError> {
    conn.execute(
        &format!(
            "INSERT INTO {TRIGGER_TABLE} (
                trigger_id, tenant_id, creator_user_id, agent_id, project_id,
                name, source, schedule_expression, schedule_timezone, completion_policy, prompt,
                state, next_run_at, last_run_at, last_fired_slot, last_status,
                active_fire_slot, active_run_ref, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
            ON CONFLICT (tenant_id, trigger_id) DO UPDATE SET
                creator_user_id = excluded.creator_user_id,
                agent_id = excluded.agent_id,
                project_id = excluded.project_id,
                name = excluded.name,
                source = excluded.source,
                schedule_expression = excluded.schedule_expression,
                schedule_timezone = excluded.schedule_timezone,
                completion_policy = excluded.completion_policy,
                prompt = excluded.prompt,
                state = excluded.state,
                next_run_at = excluded.next_run_at,
                last_run_at = excluded.last_run_at,
                last_fired_slot = excluded.last_fired_slot,
                last_status = excluded.last_status,
                active_fire_slot = excluded.active_fire_slot,
                active_run_ref = excluded.active_run_ref"
        ),
        params![
            record.trigger_id.to_string(),
            record.tenant_id.as_str(),
            record.creator_user_id.as_str(),
            opt_text(record.agent_id.as_ref().map(AgentId::as_str)),
            opt_text(record.project_id.as_ref().map(ProjectId::as_str)),
            record.name.clone(),
            source_kind_text(record.source),
            schedule_expression_text(&record.schedule),
            schedule_timezone_text(&record.schedule),
            completion_policy_text(record.completion_policy),
            record.prompt.clone(),
            state_text(record.state),
            fmt_ts(&record.next_run_at),
            opt_ts(record.last_run_at.as_ref()),
            opt_ts(record.last_fired_slot.as_ref()),
            opt_status(record.last_status),
            opt_ts(record.active_fire_slot.as_ref()),
            opt_turn_run_id(record.active_run_ref.as_ref()),
            fmt_ts(&record.created_at),
        ],
    )
    .await
    .map_err(|error| backend_error("upsert trigger record", error))?;
    Ok(())
}

#[cfg(feature = "libsql")]
async fn resolve_missed_fire_result_update(
    conn: &libsql::Connection,
    tenant_id: &TenantId,
    trigger_id: TriggerId,
    fire_slot: Timestamp,
    expected_run_ref: Option<TurnRunId>,
    next_run_at: Option<Timestamp>,
) -> Result<Option<TriggerRecord>, TriggerError> {
    let Some(record) = fetch_record(conn, tenant_id, trigger_id).await? else {
        return Ok(None);
    };
    if record.active_fire_slot != Some(fire_slot) {
        return Ok(None);
    }
    if let Some(active_run_ref) = record.active_run_ref {
        if let Some(expected_run_ref) = expected_run_ref {
            reject_run_ref_rewrite(active_run_ref, expected_run_ref)?;
            return Ok(Some(record));
        }
        reject_failed_result_after_active_run(Some(active_run_ref))?;
    }
    if let Some(next_run_at) = next_run_at {
        reject_non_future_next_run_at(fire_slot, next_run_at)?;
    }
    Err(backend_error(
        "reconcile missed trigger fire result update",
        "update predicate failed while claimed fire remained active without a run ref",
    ))
}

#[cfg(feature = "libsql")]
async fn mark_successful_fire_result(
    conn: &libsql::Connection,
    update: SuccessfulFireResultUpdate<'_>,
) -> Result<Option<TriggerRecord>, TriggerError> {
    let fire_slot_text = fmt_ts(&update.fire_slot);
    let result_at = fmt_ts(&update.result_at);
    let next_run_at_text = fmt_ts(&update.next_run_at);
    let active_run_ref = update.run_id.to_string();
    let last_status = status_text(TriggerRunStatus::Ok);
    begin_immediate(conn, "begin successful trigger fire result").await?;
    let update_result = async {
        let mut rows = conn
            .query(
                &format!(
                    "UPDATE {TRIGGER_TABLE}
                     SET last_run_at = ?3,
                         last_fired_slot = ?4,
                         last_status = ?5,
                         next_run_at = ?6,
                         active_fire_slot = ?4,
                         active_run_ref = ?7
                     WHERE tenant_id = ?1
                       AND trigger_id = ?2
                       AND active_fire_slot = ?4
                       AND active_run_ref IS NULL
                       AND ?6 > ?4
                     RETURNING {TRIGGER_COLUMNS}"
                ),
                params![
                    update.tenant_id.as_str(),
                    update.trigger_id.to_string(),
                    result_at,
                    fire_slot_text,
                    last_status,
                    next_run_at_text,
                    active_run_ref,
                ],
            )
            .await
            .map_err(|error| backend_error(update.update_operation, error))?;
        let Some(record) = returned_record(&mut rows, update.read_operation).await? else {
            return Ok(None);
        };
        let mut run_record = TriggerRunRecord::running(
            update.tenant_id.clone(),
            update.trigger_id,
            update.fire_slot,
            Some(update.run_id),
            record.last_run_at.unwrap_or(update.result_at),
        );
        run_record.thread_id = update.thread_id.clone();
        upsert_run_history(conn, &run_record).await?;
        Ok(Some(record))
    }
    .await;
    match update_result {
        Ok(Some(record)) => {
            commit(conn, "commit successful trigger fire result").await?;
            return Ok(Some(record));
        }
        Ok(None) => rollback(conn, "rollback missed successful trigger fire result").await?,
        Err(error) => {
            rollback(conn, "rollback failed successful trigger fire result").await?;
            return Err(error);
        }
    }
    resolve_missed_fire_result_update(
        conn,
        update.tenant_id,
        update.trigger_id,
        update.fire_slot,
        Some(update.run_id),
        Some(update.next_run_at),
    )
    .await
}

#[cfg(feature = "libsql")]
struct SuccessfulFireResultUpdate<'a> {
    tenant_id: &'a TenantId,
    trigger_id: TriggerId,
    fire_slot: Timestamp,
    run_id: TurnRunId,
    /// Canonical thread id to persist in the run-history row. `Some` sets the
    /// thread id on the run-history row; `None` leaves it as `NULL` (no canonical
    /// thread known). Acceptance always passes `Some`; replay passes whatever the
    /// submission outcome resolved.
    thread_id: Option<ThreadId>,
    result_at: Timestamp,
    next_run_at: Timestamp,
    update_operation: &'static str,
    read_operation: &'static str,
}

#[cfg(feature = "libsql")]
fn row_to_run_record(row: &libsql::Row) -> Result<TriggerRunRecord, TriggerError> {
    let tenant_id = TenantId::new(required_text(row, RUN_TENANT_ID_COL, "tenant_id")?)
        .map_err(|error| invalid_record("tenant_id", error.to_string()))?;
    let trigger_id = TriggerId::parse(&required_text(row, RUN_TRIGGER_ID_COL, "trigger_id")?)?;
    let fire_slot = parse_timestamp(
        &required_text(row, RUN_FIRE_SLOT_COL, "fire_slot")?,
        "fire_slot",
    )?;
    let run_id = optional_text(row, RUN_ID_COL, "run_id")?
        .map(|value| parse_turn_run_id_with_field(&value, "run_id"))
        .transpose()?;
    let thread_id = optional_text(row, RUN_THREAD_ID_COL, "thread_id")?
        .map(|value| {
            ThreadId::new(value).map_err(|error| invalid_record("thread_id", error.to_string()))
        })
        .transpose()?;
    let status = parse_run_history_status(&required_text(row, RUN_STATUS_COL, "status")?)?;
    let submitted_at = parse_timestamp(
        &required_text(row, RUN_SUBMITTED_AT_COL, "submitted_at")?,
        "submitted_at",
    )?;
    let completed_at = optional_text(row, RUN_COMPLETED_AT_COL, "completed_at")?
        .map(|value| parse_timestamp(&value, "completed_at"))
        .transpose()?;
    Ok(TriggerRunRecord {
        tenant_id,
        trigger_id,
        fire_slot,
        run_id,
        thread_id,
        status,
        submitted_at,
        completed_at,
    })
}

#[cfg(feature = "libsql")]
async fn upsert_run_history(
    conn: &libsql::Connection,
    run: &TriggerRunRecord,
) -> Result<(), TriggerError> {
    conn.execute(
        &format!(
            "INSERT INTO {TRIGGER_RUN_TABLE} (
                tenant_id, trigger_id, fire_slot, run_id, thread_id, status, submitted_at, completed_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT (tenant_id, trigger_id, fire_slot) DO UPDATE SET
                run_id = excluded.run_id,
                thread_id = COALESCE(excluded.thread_id, {TRIGGER_RUN_TABLE}.thread_id),
                status = excluded.status,
                submitted_at = excluded.submitted_at,
                completed_at = excluded.completed_at"
        ),
        params![
            run.tenant_id.as_str(),
            run.trigger_id.to_string(),
            fmt_ts(&run.fire_slot),
            opt_turn_run_id(run.run_id.as_ref()),
            run.thread_id.as_ref().map(|t| t.as_str()),
            trigger_run_history_status_text(run.status),
            fmt_ts(&run.submitted_at),
            opt_ts(run.completed_at.as_ref()),
        ],
    )
    .await
    .map_err(|error| backend_error("upsert trigger run history", error))?;
    prune_run_history(conn, &run.tenant_id, run.trigger_id).await?;
    Ok(())
}

#[cfg(feature = "libsql")]
fn trigger_ids_json_array(trigger_ids: &[TriggerId]) -> String {
    let mut value = String::from("[");
    for (index, trigger_id) in trigger_ids.iter().enumerate() {
        if index > 0 {
            value.push(',');
        }
        value.push('"');
        value.push_str(&trigger_id.to_string());
        value.push('"');
    }
    value.push(']');
    value
}

#[cfg(feature = "libsql")]
async fn complete_run_history(
    conn: &libsql::Connection,
    tenant_id: &TenantId,
    trigger_id: TriggerId,
    fire_slot: Timestamp,
    run_id: Option<TurnRunId>,
    status: TriggerRunHistoryStatus,
    completed_at: Timestamp,
) -> Result<(), TriggerError> {
    let run_id_value = opt_turn_run_id(run_id.as_ref());
    conn.execute(
        &format!(
            "INSERT INTO {TRIGGER_RUN_TABLE} (
                tenant_id, trigger_id, fire_slot, run_id, thread_id, status, submitted_at, completed_at
            ) VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7)
            ON CONFLICT (tenant_id, trigger_id, fire_slot) DO UPDATE SET
                run_id = COALESCE(trigger_run_history.run_id, excluded.run_id),
                status = excluded.status,
                completed_at = excluded.completed_at"
        ),
        params![
            tenant_id.as_str(),
            trigger_id.to_string(),
            fmt_ts(&fire_slot),
            run_id_value,
            trigger_run_history_status_text(status),
            fmt_ts(&completed_at),
            fmt_ts(&completed_at),
        ],
    )
    .await
    .map_err(|error| backend_error("complete trigger run history", error))?;
    prune_run_history(conn, tenant_id, trigger_id).await?;
    Ok(())
}

#[cfg(feature = "libsql")]
async fn prune_run_history(
    conn: &libsql::Connection,
    tenant_id: &TenantId,
    trigger_id: TriggerId,
) -> Result<(), TriggerError> {
    conn.execute(
        &format!(
            "DELETE FROM {TRIGGER_RUN_TABLE}
             WHERE tenant_id = ?1
               AND trigger_id = ?2
               AND fire_slot NOT IN (
                   SELECT fire_slot
                   FROM {TRIGGER_RUN_TABLE}
                   WHERE tenant_id = ?1 AND trigger_id = ?2
                   ORDER BY fire_slot DESC
                   LIMIT ?3
               )"
        ),
        params![
            tenant_id.as_str(),
            trigger_id.to_string(),
            crate::MAX_TRIGGER_RUN_HISTORY_RETAINED as i64,
        ],
    )
    .await
    .map_err(|error| backend_error("prune trigger run history", error))?;
    Ok(())
}

#[cfg(feature = "libsql")]
fn required_text(row: &libsql::Row, index: usize, field: &str) -> Result<String, TriggerError> {
    row.get(index as i32)
        .map_err(|error| invalid_record(field, error.to_string()))
}

#[cfg(feature = "libsql")]
fn optional_text(
    row: &libsql::Row,
    index: usize,
    field: &str,
) -> Result<Option<String>, TriggerError> {
    row.get(index as i32)
        .map_err(|error| backend_error(&format!("read optional trigger field {field}"), error))
}

#[cfg(feature = "libsql")]
fn parse_timestamp(value: &str, field: &str) -> Result<Timestamp, TriggerError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| invalid_record(field, error.to_string()))
}

#[cfg(feature = "libsql")]
fn parse_turn_run_id(value: &str) -> Result<TurnRunId, TriggerError> {
    parse_turn_run_id_with_field(value, "active_run_ref")
}

#[cfg(feature = "libsql")]
fn parse_turn_run_id_with_field(value: &str, field: &str) -> Result<TurnRunId, TriggerError> {
    TurnRunId::parse(value).map_err(|error| invalid_record(field, error.to_string()))
}

#[cfg(feature = "libsql")]
fn fmt_ts(value: &Timestamp) -> String {
    value.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

#[cfg(feature = "libsql")]
fn opt_ts(value: Option<&Timestamp>) -> libsql::Value {
    match value {
        Some(value) => libsql::Value::Text(fmt_ts(value)),
        None => libsql::Value::Null,
    }
}

#[cfg(feature = "libsql")]
fn opt_text(value: Option<&str>) -> libsql::Value {
    match value {
        Some(value) => libsql::Value::Text(value.to_string()),
        None => libsql::Value::Null,
    }
}

#[cfg(feature = "libsql")]
fn opt_status(value: Option<TriggerRunStatus>) -> libsql::Value {
    match value {
        Some(value) => libsql::Value::Text(status_text(value).to_string()),
        None => libsql::Value::Null,
    }
}

#[cfg(feature = "libsql")]
fn opt_turn_run_id(value: Option<&TurnRunId>) -> libsql::Value {
    match value {
        Some(value) => libsql::Value::Text(value.to_string()),
        None => libsql::Value::Null,
    }
}

#[cfg(feature = "libsql")]
fn source_kind_text(value: TriggerSourceKind) -> &'static str {
    match value {
        TriggerSourceKind::Schedule => "schedule",
    }
}

#[cfg(feature = "libsql")]
fn parse_source_kind(value: &str) -> Result<TriggerSourceKind, TriggerError> {
    match value {
        "schedule" => Ok(TriggerSourceKind::Schedule),
        other => Err(invalid_record(
            "source",
            format!("unsupported trigger source `{other}`"),
        )),
    }
}

#[cfg(feature = "libsql")]
fn state_text(value: TriggerState) -> &'static str {
    match value {
        TriggerState::Scheduled => "scheduled",
        TriggerState::Paused => "paused",
        TriggerState::Completed => "completed",
    }
}

#[cfg(feature = "libsql")]
fn parse_state(value: &str) -> Result<TriggerState, TriggerError> {
    match value {
        "scheduled" => Ok(TriggerState::Scheduled),
        "paused" => Ok(TriggerState::Paused),
        "completed" => Ok(TriggerState::Completed),
        other => Err(invalid_record(
            "state",
            format!("unsupported trigger state `{other}`"),
        )),
    }
}

#[cfg(feature = "libsql")]
fn completion_policy_text(value: TriggerCompletionPolicy) -> &'static str {
    match value {
        TriggerCompletionPolicy::Recurring => "recurring",
        TriggerCompletionPolicy::CompleteAfterFirstFire => "complete_after_first_fire",
    }
}

#[cfg(feature = "libsql")]
fn parse_completion_policy(value: &str) -> Result<TriggerCompletionPolicy, TriggerError> {
    match value {
        "recurring" => Ok(TriggerCompletionPolicy::Recurring),
        "complete_after_first_fire" => Ok(TriggerCompletionPolicy::CompleteAfterFirstFire),
        other => Err(invalid_record(
            "completion_policy",
            format!("unsupported completion policy `{other}`"),
        )),
    }
}

#[cfg(feature = "libsql")]
fn status_text(value: TriggerRunStatus) -> &'static str {
    match value {
        TriggerRunStatus::Ok => "ok",
        TriggerRunStatus::Error => "error",
    }
}

#[cfg(feature = "libsql")]
fn parse_run_status(value: &str) -> Result<TriggerRunStatus, TriggerError> {
    match value {
        "ok" => Ok(TriggerRunStatus::Ok),
        "error" => Ok(TriggerRunStatus::Error),
        other => Err(invalid_record(
            "last_status",
            format!("unsupported trigger run status `{other}`"),
        )),
    }
}

#[cfg(feature = "libsql")]
fn parse_run_history_status(value: &str) -> Result<TriggerRunHistoryStatus, TriggerError> {
    match value {
        "running" => Ok(TriggerRunHistoryStatus::Running),
        "ok" => Ok(TriggerRunHistoryStatus::Ok),
        "error" => Ok(TriggerRunHistoryStatus::Error),
        other => Err(invalid_record(
            "status",
            format!("unsupported trigger run history status `{other}`"),
        )),
    }
}

#[cfg(feature = "libsql")]
fn schedule_expression_text(schedule: &TriggerSchedule) -> String {
    match schedule {
        TriggerSchedule::Cron { expression, .. } => expression.clone(),
    }
}

#[cfg(feature = "libsql")]
fn schedule_timezone_text(schedule: &TriggerSchedule) -> String {
    match schedule {
        TriggerSchedule::Cron { timezone, .. } => timezone.clone(),
    }
}

#[cfg(feature = "libsql")]
fn invalid_record(field: &str, reason: impl Into<String>) -> TriggerError {
    TriggerError::InvalidRecord {
        reason: format!("{field}: {}", reason.into()),
    }
}

#[cfg(feature = "libsql")]
fn backend_error(operation: &str, error: impl std::fmt::Display) -> TriggerError {
    TriggerError::Backend {
        reason: format!("{operation}: {error}"),
    }
}
