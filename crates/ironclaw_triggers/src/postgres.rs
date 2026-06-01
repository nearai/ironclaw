use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, Timestamp, UserId};
use ironclaw_turns::TurnRunId;
use tokio_postgres::Row;

use crate::{
    TriggerCompletionPolicy, TriggerError, TriggerId, TriggerRecord, TriggerRepository,
    TriggerRunStatus, TriggerSchedule, TriggerSourceKind, TriggerState,
};

const TRIGGER_TABLE: &str = "trigger_records";
const TRIGGER_COLUMNS: &str = "\
    trigger_id, tenant_id, creator_user_id, agent_id, project_id, \
    name, source, schedule_expression, completion_policy, prompt, \
    state, next_run_at, last_run_at, last_fired_slot, last_status, \
    active_fire_slot, active_run_ref, created_at";

/// PostgreSQL-backed [`TriggerRepository`] storing trigger records.
pub struct PostgresTriggerRepository {
    pool: deadpool_postgres::Pool,
}

impl PostgresTriggerRepository {
    pub fn new(pool: deadpool_postgres::Pool) -> Self {
        Self { pool }
    }

    pub async fn run_migrations(&self) -> Result<(), TriggerError> {
        let mut client = self.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|error| backend_error("begin trigger migration", error))?;
        tx.batch_execute(POSTGRES_TRIGGER_SCHEMA)
            .await
            .map_err(|error| backend_error("run trigger migrations", error))?;
        tx.commit()
            .await
            .map_err(|error| backend_error("commit trigger migration", error))
    }

    async fn connect(&self) -> Result<deadpool_postgres::Object, TriggerError> {
        self.pool
            .get()
            .await
            .map_err(|error| backend_error("connect trigger repository", error))
    }
}

#[async_trait]
impl TriggerRepository for PostgresTriggerRepository {
    async fn upsert_trigger(&self, record: TriggerRecord) -> Result<(), TriggerError> {
        record.validate()?;
        let client = self.connect().await?;
        let trigger_id = record.trigger_id.to_string();
        let tenant_id = record.tenant_id.as_str().to_string();
        let creator_user_id = record.creator_user_id.as_str().to_string();
        let agent_id = record
            .agent_id
            .as_ref()
            .map(|value| value.as_str().to_string());
        let project_id = record
            .project_id
            .as_ref()
            .map(|value| value.as_str().to_string());
        let source = source_kind_text(record.source);
        let schedule_expression = schedule_expression_text(&record.schedule);
        let completion_policy = completion_policy_text(record.completion_policy);
        let state = state_text(record.state);
        let next_run_at = fmt_ts(&record.next_run_at);
        let last_run_at = record.last_run_at.as_ref().map(fmt_ts);
        let last_fired_slot = record.last_fired_slot.as_ref().map(fmt_ts);
        let last_status = record.last_status.map(status_text);
        let active_fire_slot = record.active_fire_slot.as_ref().map(fmt_ts);
        let active_run_ref = record.active_run_ref.as_ref().map(ToString::to_string);
        let created_at = fmt_ts(&record.created_at);

        client
            .execute(
                r#"
                INSERT INTO trigger_records (
                    trigger_id, tenant_id, creator_user_id, agent_id, project_id,
                    name, source, schedule_expression, completion_policy, prompt,
                    state, next_run_at, last_run_at, last_fired_slot, last_status,
                    active_fire_slot, active_run_ref, created_at
                ) VALUES (
                    $1, $2, $3, $4, $5,
                    $6, $7, $8, $9, $10,
                    $11, $12, $13, $14, $15,
                    $16, $17, $18
                )
                ON CONFLICT (tenant_id, trigger_id) DO UPDATE SET
                    creator_user_id = EXCLUDED.creator_user_id,
                    agent_id = EXCLUDED.agent_id,
                    project_id = EXCLUDED.project_id,
                    name = EXCLUDED.name,
                    source = EXCLUDED.source,
                    schedule_expression = EXCLUDED.schedule_expression,
                    completion_policy = EXCLUDED.completion_policy,
                    prompt = EXCLUDED.prompt,
                    state = EXCLUDED.state,
                    next_run_at = EXCLUDED.next_run_at,
                    last_run_at = EXCLUDED.last_run_at,
                    last_fired_slot = EXCLUDED.last_fired_slot,
                    last_status = EXCLUDED.last_status,
                    active_fire_slot = EXCLUDED.active_fire_slot,
                    active_run_ref = EXCLUDED.active_run_ref
                "#,
                &[
                    &trigger_id,
                    &tenant_id,
                    &creator_user_id,
                    &agent_id,
                    &project_id,
                    &record.name,
                    &source,
                    &schedule_expression,
                    &completion_policy,
                    &record.prompt,
                    &state,
                    &next_run_at,
                    &last_run_at,
                    &last_fired_slot,
                    &last_status,
                    &active_fire_slot,
                    &active_run_ref,
                    &created_at,
                ],
            )
            .await
            .map_err(|error| backend_error("upsert trigger record", error))?;
        Ok(())
    }

    async fn get_trigger(
        &self,
        tenant_id: TenantId,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let client = self.connect().await?;
        let trigger_id = trigger_id.to_string();
        let row = client
            .query_opt(
                &format!(
                    "SELECT {TRIGGER_COLUMNS}
                     FROM {TRIGGER_TABLE}
                     WHERE tenant_id = $1 AND trigger_id = $2
                     LIMIT 1"
                ),
                &[&tenant_id.as_str(), &trigger_id],
            )
            .await
            .map_err(|error| backend_error("query trigger record", error))?;
        match row {
            Some(row) => Ok(Some(row_to_record(&row)?)),
            None => Ok(None),
        }
    }

    async fn list_triggers(&self, tenant_id: TenantId) -> Result<Vec<TriggerRecord>, TriggerError> {
        let client = self.connect().await?;
        let rows = client
            .query(
                &format!(
                    "SELECT {TRIGGER_COLUMNS}
                     FROM {TRIGGER_TABLE}
                     WHERE tenant_id = $1
                     ORDER BY created_at, trigger_id"
                ),
                &[&tenant_id.as_str()],
            )
            .await
            .map_err(|error| backend_error("query tenant trigger records", error))?;
        rows.into_iter().map(|row| row_to_record(&row)).collect()
    }

    async fn remove_trigger(
        &self,
        tenant_id: TenantId,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let client = self.connect().await?;
        let trigger_id = trigger_id.to_string();
        let row = client
            .query_opt(
                &format!(
                    "DELETE FROM {TRIGGER_TABLE}
                     WHERE tenant_id = $1 AND trigger_id = $2
                     RETURNING {TRIGGER_COLUMNS}"
                ),
                &[&tenant_id.as_str(), &trigger_id],
            )
            .await
            .map_err(|error| backend_error("remove trigger record", error))?;
        match row {
            Some(row) => Ok(Some(row_to_record(&row)?)),
            None => Ok(None),
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
        let limit = limit.min(super::MAX_DUE_TRIGGER_POLL_LIMIT) as i64;
        let client = self.connect().await?;
        let now = fmt_ts(&now);
        let rows = client
            .query(
                &format!(
                    "SELECT {TRIGGER_COLUMNS}
                     FROM {TRIGGER_TABLE}
                     WHERE state = $1 AND next_run_at <= $2
                     ORDER BY next_run_at, tenant_id, trigger_id
                     LIMIT $3"
                ),
                &[&state_text(TriggerState::Scheduled), &now, &limit],
            )
            .await
            .map_err(|error| backend_error("query due trigger records", error))?;
        rows.into_iter().map(|row| row_to_record(&row)).collect()
    }
}

fn row_to_record(row: &Row) -> Result<TriggerRecord, TriggerError> {
    let trigger_id = TriggerId::parse(&required_text(row, "trigger_id")?)?;
    let tenant_id = TenantId::new(required_text(row, "tenant_id")?)
        .map_err(|error| invalid_record("tenant_id", error.to_string()))?;
    let creator_user_id = UserId::new(required_text(row, "creator_user_id")?)
        .map_err(|error| invalid_record("creator_user_id", error.to_string()))?;
    let agent_id = optional_text(row, "agent_id")?
        .map(|value| {
            AgentId::new(value).map_err(|error| invalid_record("agent_id", error.to_string()))
        })
        .transpose()?;
    let project_id = optional_text(row, "project_id")?
        .map(|value| {
            ProjectId::new(value).map_err(|error| invalid_record("project_id", error.to_string()))
        })
        .transpose()?;
    let schedule = TriggerSchedule::cron(required_text(row, "schedule_expression")?)?;
    let last_run_at = optional_text(row, "last_run_at")?
        .map(|value| parse_timestamp(&value, "last_run_at"))
        .transpose()?;
    let last_fired_slot = optional_text(row, "last_fired_slot")?
        .map(|value| parse_timestamp(&value, "last_fired_slot"))
        .transpose()?;
    let last_status = optional_text(row, "last_status")?
        .map(|value| parse_run_status(&value))
        .transpose()?;
    let active_fire_slot = optional_text(row, "active_fire_slot")?
        .map(|value| parse_timestamp(&value, "active_fire_slot"))
        .transpose()?;
    let active_run_ref = optional_text(row, "active_run_ref")?
        .map(|value| parse_turn_run_id(&value))
        .transpose()?;

    let record = TriggerRecord {
        trigger_id,
        tenant_id,
        creator_user_id,
        agent_id,
        project_id,
        name: required_text(row, "name")?,
        source: parse_source_kind(&required_text(row, "source")?)?,
        schedule,
        completion_policy: parse_completion_policy(&required_text(row, "completion_policy")?)?,
        prompt: required_text(row, "prompt")?,
        state: parse_state(&required_text(row, "state")?)?,
        next_run_at: parse_timestamp(&required_text(row, "next_run_at")?, "next_run_at")?,
        last_run_at,
        last_fired_slot,
        last_status,
        active_fire_slot,
        active_run_ref,
        created_at: parse_timestamp(&required_text(row, "created_at")?, "created_at")?,
    };
    record.validate()?;
    Ok(record)
}

fn required_text(row: &Row, field: &str) -> Result<String, TriggerError> {
    row.try_get(field)
        .map_err(|error| invalid_record(field, error.to_string()))
}

fn optional_text(row: &Row, field: &str) -> Result<Option<String>, TriggerError> {
    row.try_get(field)
        .map_err(|error| backend_error(&format!("read optional trigger field {field}"), error))
}

fn parse_timestamp(value: &str, field: &str) -> Result<Timestamp, TriggerError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| invalid_record(field, error.to_string()))
}

fn parse_turn_run_id(value: &str) -> Result<TurnRunId, TriggerError> {
    TurnRunId::parse(value).map_err(|error| invalid_record("active_run_ref", error.to_string()))
}

fn fmt_ts(value: &Timestamp) -> String {
    value.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn source_kind_text(value: TriggerSourceKind) -> &'static str {
    match value {
        TriggerSourceKind::Schedule => "schedule",
    }
}

fn parse_source_kind(value: &str) -> Result<TriggerSourceKind, TriggerError> {
    match value {
        "schedule" => Ok(TriggerSourceKind::Schedule),
        other => Err(invalid_record(
            "source",
            format!("unsupported trigger source `{other}`"),
        )),
    }
}

fn state_text(value: TriggerState) -> &'static str {
    match value {
        TriggerState::Scheduled => "scheduled",
        TriggerState::Paused => "paused",
        TriggerState::Completed => "completed",
    }
}

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

fn completion_policy_text(value: TriggerCompletionPolicy) -> &'static str {
    match value {
        TriggerCompletionPolicy::Recurring => "recurring",
        TriggerCompletionPolicy::CompleteAfterFirstFire => "complete_after_first_fire",
    }
}

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

fn status_text(value: TriggerRunStatus) -> &'static str {
    match value {
        TriggerRunStatus::Ok => "ok",
        TriggerRunStatus::Error => "error",
    }
}

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

fn schedule_expression_text(schedule: &TriggerSchedule) -> String {
    match schedule {
        TriggerSchedule::Cron { expression } => expression.clone(),
    }
}

fn invalid_record(field: &str, reason: impl Into<String>) -> TriggerError {
    TriggerError::InvalidRecord {
        reason: format!("{field}: {}", reason.into()),
    }
}

fn backend_error(operation: &str, error: impl std::fmt::Display) -> TriggerError {
    TriggerError::Backend {
        reason: format!("{operation}: {error}"),
    }
}

const POSTGRES_TRIGGER_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS trigger_records (
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
);

CREATE INDEX IF NOT EXISTS trigger_records_state_next_run_at_idx
    ON trigger_records (state, next_run_at, tenant_id, trigger_id);

CREATE INDEX IF NOT EXISTS trigger_records_tenant_created_at_idx
    ON trigger_records (tenant_id, created_at, trigger_id);
"#;
