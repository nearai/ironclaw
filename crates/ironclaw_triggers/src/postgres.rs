use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, Timestamp, UserId};
use ironclaw_turns::TurnRunId;
use tokio_postgres::Row;

use crate::{
    ClaimDueFireOutcome, ClaimDueFireRequest, ClaimedTriggerFire, FireAcceptedRequest,
    FirePermanentFailedRequest, FireReplayedRequest, FireRetryableFailedRequest,
    TriggerCompletionPolicy, TriggerError, TriggerId, TriggerRecord, TriggerRepository,
    TriggerRunStatus, TriggerSchedule, TriggerSourceKind, TriggerState,
    reject_failed_result_after_active_run, reject_non_future_next_run_at, reject_run_ref_rewrite,
};

const TRIGGER_TABLE: &str = "trigger_records";
const TRIGGER_COLUMNS: &str = "\
    trigger_id, tenant_id, creator_user_id, agent_id, project_id, \
    name, source, schedule_expression, completion_policy, prompt, \
    state, next_run_at, last_run_at, last_fired_slot, last_status, \
    active_fire_slot, active_run_ref, created_at";
const TRIGGER_MIGRATION_ADVISORY_LOCK: i64 = 717_263_529;

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
        tx.execute(
            "SELECT pg_advisory_xact_lock($1)",
            &[&TRIGGER_MIGRATION_ADVISORY_LOCK],
        )
        .await
        .map_err(|error| backend_error("acquire trigger migration advisory lock", error))?;
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
        let tenant_id = record.tenant_id.as_str();
        let creator_user_id = record.creator_user_id.as_str();
        let agent_id = record.agent_id.as_ref().map(AgentId::as_str);
        let project_id = record.project_id.as_ref().map(ProjectId::as_str);
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
                     WHERE state = $1
                       AND next_run_at <= $2
                       AND active_fire_slot IS NULL
                       AND active_run_ref IS NULL
                     ORDER BY next_run_at, tenant_id, trigger_id
                     LIMIT $3"
                ),
                &[&state_text(TriggerState::Scheduled), &now, &limit],
            )
            .await
            .map_err(|error| backend_error("query due trigger records", error))?;
        rows.into_iter().map(|row| row_to_record(&row)).collect()
    }

    async fn claim_due_fire(
        &self,
        request: ClaimDueFireRequest,
    ) -> Result<ClaimDueFireOutcome, TriggerError> {
        let mut client = self.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|error| backend_error("begin trigger fire claim", error))?;
        let trigger_id = request.trigger_id.to_string();
        let Some(record) = locked_record(&tx, request.tenant_id.as_str(), &trigger_id).await?
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

        let fire_slot = fmt_ts(&request.fire_slot);
        let row = tx
            .query_one(
                &format!(
                    "UPDATE {TRIGGER_TABLE}
                     SET active_fire_slot = $3,
                         active_run_ref = NULL
                     WHERE tenant_id = $1 AND trigger_id = $2
                     RETURNING {TRIGGER_COLUMNS}"
                ),
                &[&request.tenant_id.as_str(), &trigger_id, &fire_slot],
            )
            .await
            .map_err(|error| backend_error("claim trigger fire", error))?;
        let record = row_to_record(&row)?;
        tx.commit()
            .await
            .map_err(|error| backend_error("commit trigger fire claim", error))?;
        Ok(ClaimDueFireOutcome::Claimed(ClaimedTriggerFire {
            record,
            fire_slot: request.fire_slot,
        }))
    }

    async fn mark_fire_accepted(
        &self,
        request: FireAcceptedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let mut client = self.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|error| backend_error("begin accepted trigger fire update", error))?;
        let trigger_id = request.trigger_id.to_string();
        let Some(record) = locked_record(&tx, request.tenant_id.as_str(), &trigger_id).await?
        else {
            return Ok(None);
        };
        if record.active_fire_slot != Some(request.fire_slot) {
            return Ok(None);
        }
        if let Some(active_run_ref) = record.active_run_ref {
            reject_run_ref_rewrite(active_run_ref, request.run_id)?;
            return Ok(Some(record));
        }
        reject_non_future_next_run_at(request.fire_slot, request.next_run_at)?;

        let submitted_at = fmt_ts(&request.submitted_at);
        let fire_slot = fmt_ts(&request.fire_slot);
        let next_run_at = fmt_ts(&request.next_run_at);
        let active_run_ref = request.run_id.to_string();
        let last_status = status_text(TriggerRunStatus::Ok);
        let row = tx
            .query_one(
                &format!(
                    "UPDATE {TRIGGER_TABLE}
                     SET last_run_at = $3,
                         last_fired_slot = $4,
                         last_status = $5,
                         next_run_at = $6,
                         active_fire_slot = $4,
                         active_run_ref = $7
                     WHERE tenant_id = $1
                       AND trigger_id = $2
                       AND active_fire_slot = $4
                       AND active_run_ref IS NULL
                     RETURNING {TRIGGER_COLUMNS}"
                ),
                &[
                    &request.tenant_id.as_str(),
                    &trigger_id,
                    &submitted_at,
                    &fire_slot,
                    &last_status,
                    &next_run_at,
                    &active_run_ref,
                ],
            )
            .await
            .map_err(|error| backend_error("mark accepted trigger fire", error))?;
        let record = row_to_record(&row)?;
        tx.commit()
            .await
            .map_err(|error| backend_error("commit accepted trigger fire", error))?;
        Ok(Some(record))
    }

    async fn mark_fire_replayed(
        &self,
        request: FireReplayedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let mut client = self.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|error| backend_error("begin replayed trigger fire update", error))?;
        let trigger_id = request.trigger_id.to_string();
        let Some(record) = locked_record(&tx, request.tenant_id.as_str(), &trigger_id).await?
        else {
            return Ok(None);
        };
        if record.active_fire_slot != Some(request.fire_slot) {
            return Ok(None);
        }
        if let Some(active_run_ref) = record.active_run_ref {
            reject_run_ref_rewrite(active_run_ref, request.original_run_id)?;
            return Ok(Some(record));
        }
        reject_non_future_next_run_at(request.fire_slot, request.next_run_at)?;

        let replayed_at = fmt_ts(&request.replayed_at);
        let fire_slot = fmt_ts(&request.fire_slot);
        let next_run_at = fmt_ts(&request.next_run_at);
        let active_run_ref = request.original_run_id.to_string();
        let last_status = status_text(TriggerRunStatus::Ok);
        let row = tx
            .query_one(
                &format!(
                    "UPDATE {TRIGGER_TABLE}
                     SET last_run_at = $3,
                         last_fired_slot = $4,
                         last_status = $5,
                         next_run_at = $6,
                         active_fire_slot = $4,
                         active_run_ref = $7
                     WHERE tenant_id = $1
                       AND trigger_id = $2
                       AND active_fire_slot = $4
                       AND active_run_ref IS NULL
                     RETURNING {TRIGGER_COLUMNS}"
                ),
                &[
                    &request.tenant_id.as_str(),
                    &trigger_id,
                    &replayed_at,
                    &fire_slot,
                    &last_status,
                    &next_run_at,
                    &active_run_ref,
                ],
            )
            .await
            .map_err(|error| backend_error("mark replayed trigger fire", error))?;
        let record = row_to_record(&row)?;
        tx.commit()
            .await
            .map_err(|error| backend_error("commit replayed trigger fire", error))?;
        Ok(Some(record))
    }

    async fn mark_fire_retryable_failed(
        &self,
        request: FireRetryableFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let mut client = self.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|error| backend_error("begin retryable trigger fire failure", error))?;
        let trigger_id = request.trigger_id.to_string();
        let Some(record) = locked_record(&tx, request.tenant_id.as_str(), &trigger_id).await?
        else {
            return Ok(None);
        };
        if record.active_fire_slot != Some(request.fire_slot) {
            return Ok(None);
        }
        reject_failed_result_after_active_run(record.active_run_ref)?;
        if record.next_run_at > request.fire_slot {
            return Err(TriggerError::InvalidRecord {
                reason: "retryable fire failure must leave next_run_at at or before the failed fire slot"
                    .to_string(),
            });
        }

        let last_status = status_text(TriggerRunStatus::Error);
        let fire_slot = fmt_ts(&request.fire_slot);
        let row = tx
            .query_one(
                &format!(
                    "UPDATE {TRIGGER_TABLE}
                     SET last_status = $3,
                         active_fire_slot = NULL,
                         active_run_ref = NULL
                     WHERE tenant_id = $1
                       AND trigger_id = $2
                       AND active_fire_slot = $4
                       AND active_run_ref IS NULL
                       AND next_run_at <= $4
                     RETURNING {TRIGGER_COLUMNS}"
                ),
                &[
                    &request.tenant_id.as_str(),
                    &trigger_id,
                    &last_status,
                    &fire_slot,
                ],
            )
            .await
            .map_err(|error| backend_error("mark retryable trigger fire failure", error))?;
        let record = row_to_record(&row)?;
        tx.commit()
            .await
            .map_err(|error| backend_error("commit retryable trigger fire failure", error))?;
        Ok(Some(record))
    }

    async fn mark_fire_permanently_failed(
        &self,
        request: FirePermanentFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let mut client = self.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|error| backend_error("begin permanent trigger fire failure", error))?;
        let trigger_id = request.trigger_id.to_string();
        let Some(record) = locked_record(&tx, request.tenant_id.as_str(), &trigger_id).await?
        else {
            return Ok(None);
        };
        if record.active_fire_slot != Some(request.fire_slot) {
            return Ok(None);
        }
        reject_failed_result_after_active_run(record.active_run_ref)?;
        reject_non_future_next_run_at(request.fire_slot, request.next_run_at)?;

        let last_status = status_text(TriggerRunStatus::Error);
        let next_run_at = fmt_ts(&request.next_run_at);
        let fire_slot = fmt_ts(&request.fire_slot);
        let row = tx
            .query_one(
                &format!(
                    "UPDATE {TRIGGER_TABLE}
                     SET last_status = $3,
                         next_run_at = $4,
                         active_fire_slot = NULL,
                         active_run_ref = NULL
                     WHERE tenant_id = $1
                       AND trigger_id = $2
                       AND active_fire_slot = $5
                       AND active_run_ref IS NULL
                     RETURNING {TRIGGER_COLUMNS}"
                ),
                &[
                    &request.tenant_id.as_str(),
                    &trigger_id,
                    &last_status,
                    &next_run_at,
                    &fire_slot,
                ],
            )
            .await
            .map_err(|error| backend_error("mark permanent trigger fire failure", error))?;
        let record = row_to_record(&row)?;
        tx.commit()
            .await
            .map_err(|error| backend_error("commit permanent trigger fire failure", error))?;
        Ok(Some(record))
    }
}

async fn locked_record(
    tx: &tokio_postgres::Transaction<'_>,
    tenant_id: &str,
    trigger_id: &str,
) -> Result<Option<TriggerRecord>, TriggerError> {
    let row = tx
        .query_opt(
            &format!(
                "SELECT {TRIGGER_COLUMNS}
                 FROM {TRIGGER_TABLE}
                 WHERE tenant_id = $1 AND trigger_id = $2
                 FOR UPDATE"
            ),
            &[&tenant_id, &trigger_id],
        )
        .await
        .map_err(|error| backend_error("lock trigger record", error))?;
    row.map(|row| row_to_record(&row)).transpose()
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
