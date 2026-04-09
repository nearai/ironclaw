//! Job-related JobStore implementation for LibSqlBackend.

use async_trait::async_trait;
use libsql::params;
use rust_decimal::Decimal;
use uuid::Uuid;

use super::{
    LibSqlBackend, fmt_opt_ts, fmt_ts, get_decimal, get_i64, get_json, get_opt_decimal,
    get_opt_text, get_opt_ts, get_text, get_ts, opt_text, opt_text_owned, parse_job_state,
};
use crate::context::{ActionRecord, JobContext, JobState};
use crate::db::JobStore;
use crate::error::DatabaseError;
use crate::history::{AgentJobRecord, AgentJobSummary, LlmCallRecord};

use chrono::Utc;

#[async_trait]
impl JobStore for LibSqlBackend {
    async fn save_job(&self, ctx: &JobContext) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let status = ctx.state.to_string();
        let estimated_time_secs = ctx.estimated_duration.map(|d| d.as_secs() as i64);

        conn
            .execute(
                r#"
                INSERT INTO agent_jobs (
                    id, conversation_id, title, description, category, status, source,
                    user_id,
                    budget_amount, budget_token, bid_amount, estimated_cost, estimated_time_secs,
                    actual_cost, repair_attempts, max_tokens, total_tokens_used,
                    created_at, started_at, completed_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
                ON CONFLICT (id) DO UPDATE SET
                    title = excluded.title,
                    description = excluded.description,
                    category = excluded.category,
                    status = excluded.status,
                    user_id = excluded.user_id,
                    estimated_cost = excluded.estimated_cost,
                    estimated_time_secs = excluded.estimated_time_secs,
                    actual_cost = excluded.actual_cost,
                    repair_attempts = excluded.repair_attempts,
                    max_tokens = excluded.max_tokens,
                    total_tokens_used = excluded.total_tokens_used,
                    started_at = excluded.started_at,
                    completed_at = excluded.completed_at
                "#,
                params![
                    ctx.job_id.to_string(),
                    opt_text_owned(ctx.conversation_id.map(|id| id.to_string())),
                    ctx.title.as_str(),
                    ctx.description.as_str(),
                    opt_text(ctx.category.as_deref()),
                    status,
                    "direct",
                    ctx.user_id.as_str(),
                    opt_text_owned(ctx.budget.map(|d| d.to_string())),
                    opt_text(ctx.budget_token.as_deref()),
                    opt_text_owned(ctx.bid_amount.map(|d| d.to_string())),
                    opt_text_owned(ctx.estimated_cost.map(|d| d.to_string())),
                    estimated_time_secs,
                    ctx.actual_cost.to_string(),
                    ctx.repair_attempts as i64,
                    ctx.max_tokens as i64,
                    ctx.total_tokens_used as i64,
                    fmt_ts(&ctx.created_at),
                    fmt_opt_ts(&ctx.started_at),
                    fmt_opt_ts(&ctx.completed_at),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_job(&self, id: Uuid) -> Result<Option<JobContext>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, conversation_id, title, description, category, status, user_id,
                       budget_amount, budget_token, bid_amount, estimated_cost, estimated_time_secs,
                       actual_cost, repair_attempts, max_tokens, total_tokens_used,
                       created_at, started_at, completed_at
                FROM agent_jobs WHERE id = ?1
                "#,
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => {
                let status_str = get_text(&row, 5);
                let state = parse_job_state(&status_str);
                let estimated_time_secs: Option<i64> = row.get::<i64>(11).ok();

                Ok(Some(JobContext {
                    job_id: get_text(&row, 0).parse().unwrap_or_default(),
                    state,
                    user_id: get_text(&row, 6),
                    requester_id: None,
                    conversation_id: get_opt_text(&row, 1).and_then(|s| s.parse().ok()),
                    title: get_text(&row, 2),
                    description: get_text(&row, 3),
                    category: get_opt_text(&row, 4),
                    budget: get_opt_decimal(&row, 7),
                    budget_token: get_opt_text(&row, 8),
                    bid_amount: get_opt_decimal(&row, 9),
                    estimated_cost: get_opt_decimal(&row, 10),
                    estimated_duration: estimated_time_secs
                        .map(|s| std::time::Duration::from_secs(s as u64)),
                    actual_cost: get_decimal(&row, 12),
                    max_tokens: get_i64(&row, 14) as u64,
                    total_tokens_used: get_i64(&row, 15) as u64,
                    repair_attempts: get_i64(&row, 13) as u32,
                    created_at: get_ts(&row, 16),
                    started_at: get_opt_ts(&row, 17),
                    completed_at: get_opt_ts(&row, 18),
                    transitions: Vec::new(),
                    metadata: serde_json::Value::Null,
                    extra_env: std::sync::Arc::new(std::collections::HashMap::new()),
                    http_interceptor: None,
                    tool_output_stash: std::sync::Arc::new(tokio::sync::RwLock::new(
                        std::collections::HashMap::new(),
                    )),
                    // TODO(#661): persist user_timezone in agent_jobs table so
                    // background/routine jobs retain the session's timezone context.
                    user_timezone: "UTC".to_string(),
                    // TODO(#1125): approval_context is #[serde(skip)] so it's lost on
                    // DB restore. Tools that were allowed before restart will be blocked
                    // until the scheduler re-sets the context on the next dispatch.
                    approval_context: None,
                }))
            }
            None => Ok(None),
        }
    }

    async fn update_job_status(
        &self,
        id: Uuid,
        status: JobState,
        failure_reason: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE agent_jobs SET status = ?2, failure_reason = ?3 WHERE id = ?1",
            params![id.to_string(), status.to_string(), opt_text(failure_reason)],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn mark_job_stuck(&self, id: Uuid) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            "UPDATE agent_jobs SET status = 'stuck', stuck_since = ?2 WHERE id = ?1",
            params![id.to_string(), now],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_stuck_jobs(&self) -> Result<Vec<Uuid>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query("SELECT id FROM agent_jobs WHERE status = 'stuck'", ())
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut ids = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            if let Ok(id_str) = row.get::<String>(0)
                && let Ok(id) = id_str.parse()
            {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    async fn list_agent_jobs(&self) -> Result<Vec<AgentJobRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, title, status, user_id, failure_reason,
                       created_at, started_at, completed_at
                FROM agent_jobs WHERE source = 'direct'
                ORDER BY created_at DESC
                "#,
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut jobs = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let id_str = get_text(&row, 0);
            let Ok(id) = id_str.parse() else {
                tracing::warn!("Skipping agent job with invalid UUID: {}", id_str);
                continue;
            };
            jobs.push(AgentJobRecord {
                id,
                title: get_text(&row, 1),
                status: get_text(&row, 2),
                user_id: get_text(&row, 3),
                failure_reason: get_opt_text(&row, 4),
                created_at: get_ts(&row, 5),
                started_at: get_opt_ts(&row, 6),
                completed_at: get_opt_ts(&row, 7),
            });
        }
        Ok(jobs)
    }

    async fn list_agent_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<AgentJobRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, title, status, user_id, failure_reason,
                       created_at, started_at, completed_at
                FROM agent_jobs WHERE source = 'direct' AND user_id = ?1
                ORDER BY created_at DESC
                "#,
                params![user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut jobs = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let id_str = get_text(&row, 0);
            let Ok(id) = id_str.parse() else {
                tracing::warn!("Skipping agent job with invalid UUID: {}", id_str);
                continue;
            };
            jobs.push(AgentJobRecord {
                id,
                title: get_text(&row, 1),
                status: get_text(&row, 2),
                user_id: get_text(&row, 3),
                failure_reason: get_opt_text(&row, 4),
                created_at: get_ts(&row, 5),
                started_at: get_opt_ts(&row, 6),
                completed_at: get_opt_ts(&row, 7),
            });
        }
        Ok(jobs)
    }

    async fn get_agent_job_failure_reason(
        &self,
        id: Uuid,
    ) -> Result<Option<String>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT failure_reason FROM agent_jobs WHERE id = ?1",
                [id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Ok(get_opt_text(&row, 0))
        } else {
            Ok(None)
        }
    }

    async fn agent_job_summary(&self) -> Result<AgentJobSummary, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT status, COUNT(*) as cnt FROM agent_jobs WHERE source = 'direct' GROUP BY status",
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut summary = AgentJobSummary::default();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let status = get_text(&row, 0);
            let count = get_i64(&row, 1) as usize;
            summary.add_count(&status, count);
        }
        Ok(summary)
    }

    async fn agent_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<AgentJobSummary, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT status, COUNT(*) as cnt FROM agent_jobs WHERE source = 'direct' AND user_id = ?1 GROUP BY status",
                params![user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut summary = AgentJobSummary::default();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let status = get_text(&row, 0);
            let count = get_i64(&row, 1) as usize;
            summary.add_count(&status, count);
        }
        Ok(summary)
    }

    async fn save_action(&self, job_id: Uuid, action: &ActionRecord) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let duration_ms = action.duration.as_millis() as i64;
        let warnings_json = serde_json::to_string(&action.sanitization_warnings)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        conn.execute(
            r#"
                INSERT INTO job_actions (
                    id, job_id, sequence_num, tool_name, input, output_raw, output_sanitized,
                    sanitization_warnings, cost, duration_ms, success, error_message, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                "#,
            params![
                action.id.to_string(),
                job_id.to_string(),
                action.sequence as i64,
                action.tool_name.as_str(),
                action.input.to_string(),
                opt_text(action.output_raw.as_deref()),
                opt_text_owned(action.output_sanitized.as_ref().map(|v| v.to_string())),
                warnings_json,
                opt_text_owned(action.cost.map(|d| d.to_string())),
                duration_ms,
                action.success as i64,
                opt_text(action.error.as_deref()),
                fmt_ts(&action.executed_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_job_actions(&self, job_id: Uuid) -> Result<Vec<ActionRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, sequence_num, tool_name, input, output_raw, output_sanitized,
                       sanitization_warnings, cost, duration_ms, success, error_message, created_at
                FROM job_actions WHERE job_id = ?1 ORDER BY sequence_num
                "#,
                params![job_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut actions = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let warnings: Vec<String> =
                serde_json::from_str(&get_text(&row, 6)).unwrap_or_default();
            actions.push(ActionRecord {
                id: get_text(&row, 0).parse().unwrap_or_default(),
                sequence: get_i64(&row, 1) as u32,
                tool_name: get_text(&row, 2),
                input: get_json(&row, 3),
                output_raw: get_opt_text(&row, 4),
                output_sanitized: get_opt_text(&row, 5).and_then(|s| serde_json::from_str(&s).ok()),
                sanitization_warnings: warnings,
                cost: get_opt_decimal(&row, 7),
                duration: std::time::Duration::from_millis(get_i64(&row, 8) as u64),
                success: get_i64(&row, 9) != 0,
                error: get_opt_text(&row, 10),
                executed_at: get_ts(&row, 11),
            });
        }
        Ok(actions)
    }

    async fn record_llm_call(&self, record: &LlmCallRecord<'_>) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        conn.execute(
                r#"
                INSERT INTO llm_calls (id, job_id, conversation_id, provider, model, input_tokens, output_tokens, cost, purpose)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                "#,
                params![
                    id.to_string(),
                    opt_text_owned(record.job_id.map(|id| id.to_string())),
                    opt_text_owned(record.conversation_id.map(|id| id.to_string())),
                    record.provider,
                    record.model,
                    record.input_tokens as i64,
                    record.output_tokens as i64,
                    record.cost.to_string(),
                    opt_text(record.purpose),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
    }

    async fn save_estimation_snapshot(
        &self,
        job_id: Uuid,
        category: &str,
        tool_names: &[String],
        estimated_cost: Decimal,
        estimated_time_secs: i32,
        estimated_value: Decimal,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        let tools_json = serde_json::to_string(tool_names)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        conn.execute(
                r#"
                INSERT INTO estimation_snapshots (id, job_id, category, tool_names, estimated_cost, estimated_time_secs, estimated_value)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                "#,
                params![
                    id.to_string(),
                    job_id.to_string(),
                    category,
                    tools_json,
                    estimated_cost.to_string(),
                    estimated_time_secs as i64,
                    estimated_value.to_string(),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
    }

    async fn update_estimation_actuals(
        &self,
        id: Uuid,
        actual_cost: Decimal,
        actual_time_secs: i32,
        actual_value: Option<Decimal>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
                "UPDATE estimation_snapshots SET actual_cost = ?2, actual_time_secs = ?3, actual_value = ?4 WHERE id = ?1",
                params![
                    id.to_string(),
                    actual_cost.to_string(),
                    actual_time_secs as i64,
                    actual_value.map(|d| d.to_string()).unwrap_or_default(),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn create_system_job(&self, user_id: &str, source: &str) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        let now = Utc::now();
        let status = JobState::Completed.to_string();

        // System jobs represent synchronous channel/CLI-initiated dispatches
        // that begin and end in the same instant. Write `created_at =
        // started_at = completed_at` so audit queries computing duration
        // (`completed_at - started_at`) see 0, not NULL, and dashboards that
        // filter for "started but not yet completed" don't pick these up as
        // never-started rows.
        //
        // ⚠️ Row growth: every ToolDispatcher::dispatch() call (gateway
        // handlers, CLI commands, routine ticks) creates one system job row.
        // `agent_jobs` is the durable audit anchor, not ephemeral LLM data,
        // so these rows are intentionally retained forever. If row count
        // becomes a concern, prefer adding a partial index (`WHERE category
        // != 'system'`) for the agent-job listing queries rather than
        // deleting rows — deleting would violate the "LLM data is never
        // deleted" rule (CLAUDE.md).
        conn.execute(
            r#"
            INSERT INTO agent_jobs (
                id, title, description, category, status, source,
                user_id, actual_cost, repair_attempts, max_tokens,
                total_tokens_used, created_at, started_at, completed_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            "#,
            params![
                id.to_string(),
                format!("System: {source}"),
                format!("System operation: {source}"),
                "system",
                status,
                "system",
                user_id,
                "0",  // actual_cost as TEXT decimal
                0i64, // repair_attempts
                0i64, // max_tokens
                0i64, // total_tokens_used
                fmt_ts(&now),
                fmt_ts(&now), // started_at = created_at (instant start)
                fmt_ts(&now), // completed_at = created_at (instant completion)
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(id)
    }
}
