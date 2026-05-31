#[cfg(feature = "libsql")]
use std::sync::Arc;

#[cfg(feature = "libsql")]
use async_trait::async_trait;
#[cfg(feature = "libsql")]
use chrono::{DateTime, SecondsFormat, Utc};
#[cfg(feature = "libsql")]
use ironclaw_host_api::{AgentId, ProjectId, TenantId, Timestamp, UserId};
#[cfg(feature = "libsql")]
use libsql::params;

#[cfg(feature = "libsql")]
use crate::{
    TriggerCompletionPolicy, TriggerError, TriggerId, TriggerRecord, TriggerRepository,
    TriggerRunStatus, TriggerSchedule, TriggerSourceKind, TriggerState,
};

#[cfg(feature = "libsql")]
const TRIGGER_TABLE: &str = "trigger_records";

#[cfg(feature = "libsql")]
const TRIGGER_COLUMNS: &str = "\
    trigger_id, tenant_id, creator_user_id, agent_id, project_id, \
    name, source, schedule_expression, completion_policy, prompt, \
    state, next_run_at, last_run_at, last_fired_slot, last_status, \
    created_at";

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
const COMPLETION_POLICY_COL: usize = 8;
#[cfg(feature = "libsql")]
const PROMPT_COL: usize = 9;
#[cfg(feature = "libsql")]
const STATE_COL: usize = 10;
#[cfg(feature = "libsql")]
const NEXT_RUN_AT_COL: usize = 11;
#[cfg(feature = "libsql")]
const LAST_RUN_AT_COL: usize = 12;
#[cfg(feature = "libsql")]
const LAST_FIRED_SLOT_COL: usize = 13;
#[cfg(feature = "libsql")]
const LAST_STATUS_COL: usize = 14;
#[cfg(feature = "libsql")]
const CREATED_AT_COL: usize = 15;

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
                    tracing::warn!(
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
        let affected = conn
            .execute(
                &format!(
                    "INSERT INTO {TRIGGER_TABLE} (
                    trigger_id, tenant_id, creator_user_id, agent_id, project_id,
                    name, source, schedule_expression, completion_policy, prompt,
                    state, next_run_at, last_run_at, last_fired_slot, last_status,
                    created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                ON CONFLICT (tenant_id, trigger_id) DO UPDATE SET
                    creator_user_id = excluded.creator_user_id,
                    agent_id = excluded.agent_id,
                    project_id = excluded.project_id,
                    name = excluded.name,
                    source = excluded.source,
                    schedule_expression = excluded.schedule_expression,
                    completion_policy = excluded.completion_policy,
                    prompt = excluded.prompt,
                    state = excluded.state,
                    next_run_at = excluded.next_run_at,
                    last_run_at = excluded.last_run_at,
                    last_fired_slot = excluded.last_fired_slot,
                    last_status = excluded.last_status"
                ),
                params![
                    record.trigger_id.to_string(),
                    record.tenant_id.as_str(),
                    record.creator_user_id.as_str(),
                    opt_text(record.agent_id.as_ref().map(AgentId::as_str)),
                    opt_text(record.project_id.as_ref().map(ProjectId::as_str)),
                    record.name,
                    source_kind_text(record.source),
                    schedule_expression_text(&record.schedule),
                    completion_policy_text(record.completion_policy),
                    record.prompt,
                    state_text(record.state),
                    fmt_ts(&record.next_run_at),
                    opt_ts(record.last_run_at.as_ref()),
                    opt_ts(record.last_fired_slot.as_ref()),
                    opt_status(record.last_status),
                    fmt_ts(&record.created_at),
                ],
            )
            .await
            .map_err(|error| backend_error("upsert trigger record", error))?;
        if affected == 0 {
            return Err(backend_error(
                "upsert trigger record",
                "libSQL reported 0 affected rows",
            ));
        }
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
                     WHERE state = ?1 AND next_run_at <= ?2
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
    let schedule = TriggerSchedule::cron(required_text(
        row,
        SCHEDULE_EXPRESSION_COL,
        "schedule_expression",
    )?)?;
    let last_run_at = optional_text(row, LAST_RUN_AT_COL, "last_run_at")?
        .map(|value| parse_timestamp(&value, "last_run_at"))
        .transpose()?;
    let last_fired_slot = optional_text(row, LAST_FIRED_SLOT_COL, "last_fired_slot")?
        .map(|value| parse_timestamp(&value, "last_fired_slot"))
        .transpose()?;
    let last_status = optional_text(row, LAST_STATUS_COL, "last_status")?
        .map(|value| parse_run_status(&value))
        .transpose()?;

    Ok(TriggerRecord {
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
        created_at: parse_timestamp(
            &required_text(row, CREATED_AT_COL, "created_at")?,
            "created_at",
        )?,
    })
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
fn schedule_expression_text(schedule: &TriggerSchedule) -> String {
    match schedule {
        TriggerSchedule::Cron { expression } => expression.clone(),
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
