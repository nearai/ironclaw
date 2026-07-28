use crate::{
    ActiveTriggerScanCursor, ClaimDueFireOutcome, ClaimDueFireRequest, ClaimedTriggerFire,
    ClearActiveFireRequest, FireAcceptedRequest, FirePermanentFailedRequest, FireReplayedRequest,
    FireRetryableFailedRequest, FireTerminalFailedRequest, TriggerError, TriggerId, TriggerRecord,
    TriggerRepository, TriggerRunHistoryStatus, TriggerRunRecord, TriggerRunStatus,
    TriggerSchedule, TriggerState, reject_failed_result_after_active_run,
    reject_non_future_next_run_at, reject_run_ref_rewrite, trigger_run_history_status_text,
};
use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use ironclaw_common::AutomationName;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, Timestamp, UserId};
use ironclaw_turns::TurnRunId;
use libsql::params;
use std::{collections::HashMap, sync::Arc};
const TRIGGER_TABLE: &str = "trigger_records";
const TRIGGER_RUN_TABLE: &str = "trigger_run_history";
const TRIGGER_COLUMNS: &str = "\
    trigger_id, tenant_id, creator_user_id, agent_id, project_id, \
    name, source, schedule_expression, schedule_timezone, schedule_kind, prompt, \
    state, next_run_at, last_run_at, last_fired_slot, last_status, \
    active_fire_slot, active_run_ref, created_at, schedule_at, delivery_target";
const RENAME_SCOPED_TRIGGER_SQL: &str = "\
    UPDATE trigger_records
       SET name = ?6
     WHERE tenant_id = ?1
       AND creator_user_id = ?2
       AND (CASE WHEN ?3 IS NULL THEN agent_id IS NULL ELSE agent_id = ?3 END)
       AND (CASE WHEN ?4 IS NULL THEN project_id IS NULL ELSE project_id = ?4 END)
       AND trigger_id = ?5
     RETURNING trigger_id, tenant_id, creator_user_id, agent_id, project_id,
       name, source, schedule_expression, schedule_timezone, schedule_kind, prompt,
       state, next_run_at, last_run_at, last_fired_slot, last_status,
       active_fire_slot, active_run_ref, created_at, schedule_at, delivery_target";
const TRIGGER_ID_COL: usize = 0;
const TENANT_ID_COL: usize = 1;
const CREATOR_USER_ID_COL: usize = 2;
const AGENT_ID_COL: usize = 3;
const PROJECT_ID_COL: usize = 4;
const NAME_COL: usize = 5;
const SOURCE_COL: usize = 6;
const SCHEDULE_EXPRESSION_COL: usize = 7;
const SCHEDULE_TIMEZONE_COL: usize = 8;
const SCHEDULE_KIND_COL: usize = 9;
const PROMPT_COL: usize = 10;
const STATE_COL: usize = 11;
const NEXT_RUN_AT_COL: usize = 12;
const LAST_RUN_AT_COL: usize = 13;
const LAST_FIRED_SLOT_COL: usize = 14;
const LAST_STATUS_COL: usize = 15;
const ACTIVE_FIRE_SLOT_COL: usize = 16;
const ACTIVE_RUN_REF_COL: usize = 17;
const CREATED_AT_COL: usize = 18;
const SCHEDULE_AT_COL: usize = 19;
const DELIVERY_TARGET_COL: usize = 20;
const TRIGGER_RUN_COLUMNS: &str = "\
    tenant_id, trigger_id, fire_slot, run_id, thread_id, status, submitted_at, completed_at";
const RUN_TENANT_ID_COL: usize = 0;
const RUN_TRIGGER_ID_COL: usize = 1;
const RUN_FIRE_SLOT_COL: usize = 2;
const RUN_ID_COL: usize = 3;
const RUN_THREAD_ID_COL: usize = 4;
const RUN_STATUS_COL: usize = 5;
const RUN_SUBMITTED_AT_COL: usize = 6;
const RUN_COMPLETED_AT_COL: usize = 7;

/// Durable libSQL trigger repository.
pub struct LibSqlTriggerRepository {
    db: Arc<libsql::Database>,
}
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
                        schedule_timezone TEXT NOT NULL DEFAULT 'UTC',
                        schedule_kind TEXT NOT NULL DEFAULT 'cron',
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
            // Add schedule_kind column if it doesn't already exist (idempotent migration).
            if let Err(error) = conn
                .execute(
                    &format!(
                        "ALTER TABLE {TRIGGER_TABLE} ADD COLUMN schedule_kind TEXT NOT NULL DEFAULT 'cron'"
                    ),
                    (),
                )
                .await
            {
                let msg = error.to_string();
                if !msg.contains("duplicate column") && !msg.contains("already exists") {
                    return Err(backend_error("add schedule_kind column", error));
                }
            }
            // Add schedule_at column if it doesn't already exist (idempotent migration).
            if let Err(error) = conn
                .execute(
                    &format!(
                        "ALTER TABLE {TRIGGER_TABLE} ADD COLUMN schedule_at TEXT"
                    ),
                    (),
                )
                .await
            {
                let msg = error.to_string();
                if !msg.contains("duplicate column") && !msg.contains("already exists") {
                    return Err(backend_error("add schedule_at column", error));
                }
            }
            // Add delivery_target column if it doesn't already exist (idempotent migration).
            if let Err(error) = conn
                .execute(
                    &format!("ALTER TABLE {TRIGGER_TABLE} ADD COLUMN delivery_target TEXT"),
                    (),
                )
                .await
            {
                let msg = error.to_string();
                if !msg.contains("duplicate column") && !msg.contains("already exists") {
                    return Err(backend_error("add delivery_target column", error));
                }
            }
            // Drop the legacy `completion_policy` column on tables created before the
            // schedule-derived rework. Completion is now derived from the schedule
            // (`Once` / exhausted cron), so the column is no longer written; leaving it
            // NOT NULL on an existing table would fail every insert that omits it.
            // `completion_policy` was only ever an interim (branch-only) column — it
            // never shipped, so this is dev-database cleanup, not a production migration.
            // Idempotent: ignore "no such column" when it was never present / already dropped.
            if let Err(error) = conn
                .execute(
                    &format!("ALTER TABLE {TRIGGER_TABLE} DROP COLUMN completion_policy"),
                    (),
                )
                .await
            {
                let msg = error.to_string();
                if !msg.contains("no such column") && !msg.contains("does not exist") {
                    return Err(backend_error("drop legacy completion_policy column", error));
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
        conn.execute_batch("PRAGMA busy_timeout = 5000;")
            .await
            .map_err(|error| backend_error("set trigger repository busy_timeout", error))?;
        Ok(conn)
    }
}
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
        excluded_states: &[TriggerState],
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(crate::MAX_TRIGGER_LIST_LIMIT) as i64;
        let conn = self.connect().await?;
        let agent_id = agent_id.as_ref().map(AgentId::as_str);
        let project_id = project_id.as_ref().map(ProjectId::as_str);
        let excluded_states_json: libsql::Value = if excluded_states.is_empty() {
            libsql::Value::Null
        } else {
            let states_json = format!(
                "[{}]",
                excluded_states
                    .iter()
                    .map(|s| format!("\"{}\"", crate::state_text_codec(*s)))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            libsql::Value::Text(states_json)
        };
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRIGGER_COLUMNS}
                     FROM {TRIGGER_TABLE}
                     WHERE tenant_id = ?1
                       AND creator_user_id = ?2
                       AND agent_id IS ?3
                       AND project_id IS ?4
                       AND (?6 IS NULL OR state NOT IN (SELECT value FROM json_each(?6)))
                     ORDER BY created_at, trigger_id
                     LIMIT ?5"
                ),
                libsql::params_from_iter([
                    libsql::Value::Text(tenant_id.as_str().to_string()),
                    libsql::Value::Text(creator_user_id.as_str().to_string()),
                    agent_id.map_or(libsql::Value::Null, |v| libsql::Value::Text(v.to_string())),
                    project_id.map_or(libsql::Value::Null, |v| libsql::Value::Text(v.to_string())),
                    libsql::Value::Integer(limit),
                    excluded_states_json,
                ]),
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

    async fn set_scoped_trigger_state(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        trigger_id: TriggerId,
        new_state: TriggerState,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        crate::validate_user_settable_trigger_state(new_state)?;
        let conn = self.connect().await?;
        let agent_id = agent_id.as_ref().map(AgentId::as_str);
        let project_id = project_id.as_ref().map(ProjectId::as_str);
        let mut rows = conn
            .query(
                &format!(
                    "UPDATE {TRIGGER_TABLE}
                     SET state = ?6
                     WHERE tenant_id = ?1
                       AND creator_user_id = ?2
                       AND agent_id IS ?3
                       AND project_id IS ?4
                       AND trigger_id = ?5
                       AND state <> ?7
                     RETURNING {TRIGGER_COLUMNS}"
                ),
                params![
                    tenant_id.as_str(),
                    creator_user_id.as_str(),
                    agent_id,
                    project_id,
                    trigger_id.to_string(),
                    crate::state_text_codec(new_state),
                    crate::state_text_codec(TriggerState::Completed),
                ],
            )
            .await
            .map_err(|error| backend_error("set scoped trigger state", error))?;
        match rows.next().await {
            Ok(Some(row)) => Ok(Some(row_to_record(&row)?)),
            Ok(None) => Ok(None),
            Err(error) => Err(backend_error("read scoped trigger state row", error)),
        }
    }

    async fn rename_scoped_trigger(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        trigger_id: TriggerId,
        name: AutomationName,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let conn = self.connect().await?;
        let agent_id = agent_id.as_ref().map(AgentId::as_str);
        let project_id = project_id.as_ref().map(ProjectId::as_str);
        let name = name.into_inner();
        let mut rows = conn
            .query(
                RENAME_SCOPED_TRIGGER_SQL,
                params![
                    tenant_id.as_str(),
                    creator_user_id.as_str(),
                    agent_id,
                    project_id,
                    trigger_id.to_string(),
                    name,
                ],
            )
            .await
            .map_err(|error| backend_error("rename scoped trigger", error))?;
        match rows.next().await {
            Ok(Some(row)) => Ok(Some(row_to_record(&row)?)),
            Ok(None) => Ok(None),
            Err(error) => Err(backend_error("read renamed scoped trigger row", error)),
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
                    crate::state_text_codec(TriggerState::Scheduled),
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
                        crate::state_text_codec(TriggerState::Scheduled),
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
        if matches!(record.schedule, TriggerSchedule::Cron { .. }) && record.next_run_at > fire_slot
        {
            return Err(TriggerError::InvalidRecord {
                kind: crate::TriggerRecordValidationKind::Other,
                reason: "retryable fire failure must leave next_run_at at or before the failed fire slot"
                    .to_string(),
            });
        }

        let fire_slot_text = fmt_ts(&fire_slot);
        let last_status = crate::status_text_codec(TriggerRunStatus::Error);
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
        resolve_missed_fire_result_update(&conn, &tenant_id, trigger_id, fire_slot, None).await
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
        let last_status = crate::status_text_codec(TriggerRunStatus::Error);
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
        resolve_missed_fire_result_update(&conn, &tenant_id, trigger_id, fire_slot, None).await
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
        let last_status = crate::status_text_codec(TriggerRunStatus::Error);
        let completed = crate::state_text_codec(TriggerState::Completed);
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
        resolve_missed_fire_result_update(&conn, &tenant_id, trigger_id, fire_slot, None).await
    }

    async fn clear_active_fire(
        &self,
        request: ClearActiveFireRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let conn = self.connect().await?;
        begin_immediate(&conn, "begin clear active trigger fire").await?;
        let clear_result = async {
            // Fetch the record inside the transaction to compute next state atomically.
            let Some(current) = fetch_record(&conn, &request.tenant_id, request.trigger_id).await?
            else {
                return Ok(None);
            };
            if current.active_fire_slot != Some(request.fire_slot)
                || current.active_run_ref != Some(request.run_id)
            {
                return Ok(None);
            }
            // Compute new state: None from next_slot_after → Completed, Some → preserve current state.
            let next_slot = current.schedule.next_slot_after(request.fire_slot)?;
            let new_state = if next_slot.is_none() {
                crate::state_text_codec(TriggerState::Completed)
            } else {
                crate::state_text_codec(current.state)
            };
            let fire_slot_text = fmt_ts(&request.fire_slot);
            let run_id_text = request.run_id.to_string();
            let mut rows = conn
                .query(
                    &format!(
                        "UPDATE {TRIGGER_TABLE}
                         SET active_fire_slot = NULL,
                             active_run_ref = NULL,
                             state = ?3
                         WHERE tenant_id = ?1
                           AND trigger_id = ?2
                           AND active_fire_slot = ?4
                           AND active_run_ref = ?5
                         RETURNING {TRIGGER_COLUMNS}"
                    ),
                    libsql::params_from_iter([
                        libsql::Value::Text(request.tenant_id.as_str().to_string()),
                        libsql::Value::Text(request.trigger_id.to_string()),
                        libsql::Value::Text(new_state.to_string()),
                        libsql::Value::Text(fire_slot_text),
                        libsql::Value::Text(run_id_text),
                    ]),
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
    let schedule_kind = required_text(row, SCHEDULE_KIND_COL, "schedule_kind")?;
    let schedule_at = optional_text(row, SCHEDULE_AT_COL, "schedule_at")?;
    let schedule = crate::TriggerSchedule::from_storage(
        &schedule_kind,
        &schedule_expression,
        schedule_at.as_deref(),
        &schedule_timezone,
    )?;
    let last_run_at = optional_text(row, LAST_RUN_AT_COL, "last_run_at")?
        .map(|value| parse_timestamp(&value, "last_run_at"))
        .transpose()?;
    let last_fired_slot = optional_text(row, LAST_FIRED_SLOT_COL, "last_fired_slot")?
        .map(|value| parse_timestamp(&value, "last_fired_slot"))
        .transpose()?;
    let last_status = optional_text(row, LAST_STATUS_COL, "last_status")?
        .map(|value| crate::parse_run_status_codec(&value))
        .transpose()?;
    let active_fire_slot = optional_text(row, ACTIVE_FIRE_SLOT_COL, "active_fire_slot")?
        .map(|value| parse_timestamp(&value, "active_fire_slot"))
        .transpose()?;
    let active_run_ref = optional_text(row, ACTIVE_RUN_REF_COL, "active_run_ref")?
        .map(|value| parse_turn_run_id(&value))
        .transpose()?;
    let delivery_target = optional_text(row, DELIVERY_TARGET_COL, "delivery_target")?
        .map(crate::parse_trigger_delivery_target_id)
        .transpose()?;

    let record = TriggerRecord {
        trigger_id,
        tenant_id,
        creator_user_id,
        agent_id,
        project_id,
        name: required_text(row, NAME_COL, "name")?,
        source: crate::parse_source_kind_codec(&required_text(row, SOURCE_COL, "source")?)?,
        schedule,
        prompt: required_text(row, PROMPT_COL, "prompt")?,
        delivery_target,
        state: crate::parse_state_codec(&required_text(row, STATE_COL, "state")?)?,
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
async fn begin_immediate(conn: &libsql::Connection, operation: &str) -> Result<(), TriggerError> {
    conn.execute("BEGIN IMMEDIATE", ())
        .await
        .map(|_| ())
        .map_err(|error| backend_error(operation, error))
}
async fn commit(conn: &libsql::Connection, operation: &str) -> Result<(), TriggerError> {
    conn.execute("COMMIT", ())
        .await
        .map(|_| ())
        .map_err(|error| backend_error(operation, error))
}
async fn rollback(conn: &libsql::Connection, operation: &str) -> Result<(), TriggerError> {
    conn.execute("ROLLBACK", ())
        .await
        .map(|_| ())
        .map_err(|error| backend_error(operation, error))
}
async fn write_record(
    conn: &libsql::Connection,
    record: &TriggerRecord,
) -> Result<(), TriggerError> {
    let (schedule_kind, schedule_expression, schedule_at) = record.schedule.to_storage();
    conn.execute(
        &format!(
            "INSERT INTO {TRIGGER_TABLE} (
                trigger_id, tenant_id, creator_user_id, agent_id, project_id,
                name, source, schedule_expression, schedule_timezone, schedule_kind, prompt,
                state, next_run_at, last_run_at, last_fired_slot, last_status,
                active_fire_slot, active_run_ref, created_at, schedule_at, delivery_target
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)
            ON CONFLICT (tenant_id, trigger_id) DO UPDATE SET
                creator_user_id = excluded.creator_user_id,
                agent_id = excluded.agent_id,
                project_id = excluded.project_id,
                name = excluded.name,
                source = excluded.source,
                schedule_expression = excluded.schedule_expression,
                schedule_timezone = excluded.schedule_timezone,
                schedule_kind = excluded.schedule_kind,
                prompt = excluded.prompt,
                state = excluded.state,
                next_run_at = excluded.next_run_at,
                last_run_at = excluded.last_run_at,
                last_fired_slot = excluded.last_fired_slot,
                last_status = excluded.last_status,
                active_fire_slot = excluded.active_fire_slot,
                active_run_ref = excluded.active_run_ref,
                schedule_at = excluded.schedule_at,
                delivery_target = excluded.delivery_target"
        ),
        libsql::params_from_iter([
            libsql::Value::Text(record.trigger_id.to_string()),
            libsql::Value::Text(record.tenant_id.as_str().to_string()),
            libsql::Value::Text(record.creator_user_id.as_str().to_string()),
            record.agent_id.as_ref().map_or(libsql::Value::Null, |v| libsql::Value::Text(v.as_str().to_string())),
            record.project_id.as_ref().map_or(libsql::Value::Null, |v| libsql::Value::Text(v.as_str().to_string())),
            libsql::Value::Text(record.name.clone()),
            libsql::Value::Text(crate::source_kind_text_codec(record.source).to_string()),
            libsql::Value::Text(schedule_expression.to_string()),
            libsql::Value::Text(record.schedule.timezone_text().to_string()),
            libsql::Value::Text(schedule_kind.to_string()),
            libsql::Value::Text(record.prompt.clone()),
            libsql::Value::Text(crate::state_text_codec(record.state).to_string()),
            libsql::Value::Text(fmt_ts(&record.next_run_at)),
            record.last_run_at.as_ref().map_or(libsql::Value::Null, |v| libsql::Value::Text(fmt_ts(v))),
            record.last_fired_slot.as_ref().map_or(libsql::Value::Null, |v| libsql::Value::Text(fmt_ts(v))),
            record.last_status.map_or(libsql::Value::Null, |v| libsql::Value::Text(crate::status_text_codec(v).to_string())),
            record.active_fire_slot.as_ref().map_or(libsql::Value::Null, |v| libsql::Value::Text(fmt_ts(v))),
            record.active_run_ref.as_ref().map_or(libsql::Value::Null, |v| libsql::Value::Text(v.to_string())),
            libsql::Value::Text(fmt_ts(&record.created_at)),
            schedule_at.map_or(libsql::Value::Null, libsql::Value::Text),
            record.delivery_target.as_ref().map_or(libsql::Value::Null, |v| libsql::Value::Text(v.as_str().to_string())),
        ]),
    )
    .await
    .map_err(|error| backend_error("upsert trigger record", error))?;
    Ok(())
}
async fn resolve_missed_fire_result_update(
    conn: &libsql::Connection,
    tenant_id: &TenantId,
    trigger_id: TriggerId,
    fire_slot: Timestamp,
    expected_run_ref: Option<TurnRunId>,
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
    Err(backend_error(
        "reconcile missed trigger fire result update",
        "update predicate failed while claimed fire remained active without a run ref",
    ))
}
async fn mark_successful_fire_result(
    conn: &libsql::Connection,
    update: SuccessfulFireResultUpdate<'_>,
) -> Result<Option<TriggerRecord>, TriggerError> {
    let fire_slot_text = fmt_ts(&update.fire_slot);
    let result_at = fmt_ts(&update.result_at);
    let active_run_ref = update.run_id.to_string();
    let last_status = crate::status_text_codec(TriggerRunStatus::Ok);
    begin_immediate(conn, "begin successful trigger fire result").await?;
    let update_result = async {
        // Fetch the record inside the transaction so we can compute next_run_at
        // atomically (no TOCTOU race between the schedule read and the UPDATE).
        let Some(current) = fetch_record(conn, update.tenant_id, update.trigger_id).await? else {
            return Ok(None);
        };
        if current.active_fire_slot != Some(update.fire_slot) {
            return Ok(None);
        }
        if let Some(active_run_ref) = current.active_run_ref {
            reject_run_ref_rewrite(active_run_ref, update.run_id)?;
            return Ok(Some(current));
        }
        let next_run_at = current.schedule.next_slot_after(update.fire_slot)?;
        if let Some(nra) = next_run_at {
            reject_non_future_next_run_at(update.fire_slot, nra)?;
        }
        let mut rows = match next_run_at {
            Some(next) => {
                let next_run_at_text = fmt_ts(&next);
                conn.query(
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
                         RETURNING {TRIGGER_COLUMNS}"
                    ),
                    libsql::params_from_iter([
                        libsql::Value::Text(update.tenant_id.as_str().to_string()),
                        libsql::Value::Text(update.trigger_id.to_string()),
                        libsql::Value::Text(result_at),
                        libsql::Value::Text(fire_slot_text),
                        libsql::Value::Text(last_status.to_string()),
                        libsql::Value::Text(next_run_at_text),
                        libsql::Value::Text(active_run_ref),
                    ]),
                )
                .await
                .map_err(|error| backend_error(update.update_operation, error))?
            }
            None => {
                // Once trigger (or exhausted schedule): omit next_run_at from the SET
                // clause entirely so the column keeps its current value.
                conn.query(
                    &format!(
                        "UPDATE {TRIGGER_TABLE}
                         SET last_run_at = ?3,
                             last_fired_slot = ?4,
                             last_status = ?5,
                             active_fire_slot = ?4,
                             active_run_ref = ?6
                         WHERE tenant_id = ?1
                           AND trigger_id = ?2
                         RETURNING {TRIGGER_COLUMNS}"
                    ),
                    libsql::params_from_iter([
                        libsql::Value::Text(update.tenant_id.as_str().to_string()),
                        libsql::Value::Text(update.trigger_id.to_string()),
                        libsql::Value::Text(result_at),
                        libsql::Value::Text(fire_slot_text),
                        libsql::Value::Text(last_status.to_string()),
                        libsql::Value::Text(active_run_ref),
                    ]),
                )
                .await
                .map_err(|error| backend_error(update.update_operation, error))?
            }
        };
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
    )
    .await
}
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
    update_operation: &'static str,
    read_operation: &'static str,
}
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
    let status =
        crate::parse_run_history_status_codec(&required_text(row, RUN_STATUS_COL, "status")?)?;
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
fn required_text(row: &libsql::Row, index: usize, field: &str) -> Result<String, TriggerError> {
    row.get(index as i32)
        .map_err(|error| invalid_record(field, error.to_string()))
}
fn optional_text(
    row: &libsql::Row,
    index: usize,
    field: &str,
) -> Result<Option<String>, TriggerError> {
    row.get(index as i32)
        .map_err(|error| backend_error(&format!("read optional trigger field {field}"), error))
}
fn parse_timestamp(value: &str, field: &str) -> Result<Timestamp, TriggerError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| invalid_record(field, error.to_string()))
}
fn parse_turn_run_id(value: &str) -> Result<TurnRunId, TriggerError> {
    parse_turn_run_id_with_field(value, "active_run_ref")
}
fn parse_turn_run_id_with_field(value: &str, field: &str) -> Result<TurnRunId, TriggerError> {
    TurnRunId::parse(value).map_err(|error| invalid_record(field, error.to_string()))
}
fn fmt_ts(value: &Timestamp) -> String {
    value.to_rfc3339_opts(SecondsFormat::Nanos, true)
}
fn opt_ts(value: Option<&Timestamp>) -> libsql::Value {
    match value {
        Some(value) => libsql::Value::Text(fmt_ts(value)),
        None => libsql::Value::Null,
    }
}
fn opt_turn_run_id(value: Option<&TurnRunId>) -> libsql::Value {
    match value {
        Some(value) => libsql::Value::Text(value.to_string()),
        None => libsql::Value::Null,
    }
}
fn invalid_record(field: &str, reason: impl Into<String>) -> TriggerError {
    TriggerError::InvalidRecord {
        kind: crate::TriggerRecordValidationKind::Other,
        reason: format!("{field}: {}", reason.into()),
    }
}
fn backend_error(operation: &str, error: impl std::fmt::Display) -> TriggerError {
    TriggerError::Backend {
        reason: format!("{operation}: {error}"),
    }
}
