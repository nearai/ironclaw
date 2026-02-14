//! PostgreSQL store for persisting agent data.

use chrono::{DateTime, Utc};
#[cfg(feature = "postgres")]
use deadpool_postgres::{Config, Pool, Runtime};
use rust_decimal::Decimal;
#[cfg(feature = "postgres")]
use tokio_postgres::NoTls;
use uuid::Uuid;

#[cfg(feature = "postgres")]
use crate::config::DatabaseConfig;
#[cfg(feature = "postgres")]
use crate::context::{ActionRecord, JobContext, JobState};
#[cfg(feature = "postgres")]
use crate::error::DatabaseError;

/// Record for an LLM call to be persisted.
#[derive(Debug, Clone)]
pub struct LlmCallRecord<'a> {
    pub job_id: Option<Uuid>,
    pub conversation_id: Option<Uuid>,
    pub provider: &'a str,
    pub model: &'a str,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost: Decimal,
    pub purpose: Option<&'a str>,
}

/// Database store for the agent.
#[cfg(feature = "postgres")]
pub struct Store {
    pool: Pool,
}

#[cfg(feature = "postgres")]
impl Store {
    /// Wrap an existing pool (useful when the caller already has a connection).
    pub fn from_pool(pool: Pool) -> Self {
        Self { pool }
    }

    /// Create a new store and connect to the database.
    pub async fn new(config: &DatabaseConfig) -> Result<Self, DatabaseError> {
        let mut cfg = Config::new();
        cfg.url = Some(config.url().to_string());
        cfg.pool = Some(deadpool_postgres::PoolConfig {
            max_size: config.pool_size,
            ..Default::default()
        });

        let pool = cfg
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| DatabaseError::Pool(e.to_string()))?;

        // Test connection
        let _ = pool.get().await?;

        Ok(Self { pool })
    }

    /// Run database migrations (embedded via refinery).
    pub async fn run_migrations(&self) -> Result<(), DatabaseError> {
        use refinery::embed_migrations;
        embed_migrations!("migrations");

        let mut client = self.pool.get().await?;
        migrations::runner()
            .run_async(&mut **client)
            .await
            .map_err(|e| DatabaseError::Migration(e.to_string()))?;
        Ok(())
    }

    /// Get a connection from the pool.
    pub async fn conn(&self) -> Result<deadpool_postgres::Object, DatabaseError> {
        Ok(self.pool.get().await?)
    }

    /// Get a clone of the database pool.
    ///
    /// Useful for sharing the pool with other components like Workspace.
    pub fn pool(&self) -> Pool {
        self.pool.clone()
    }

    // ==================== Conversations ====================

    /// Create a new conversation.
    pub async fn create_conversation(
        &self,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = Uuid::new_v4();

        conn.execute(
            "INSERT INTO conversations (id, channel, user_id, thread_id) VALUES ($1, $2, $3, $4)",
            &[&id, &channel, &user_id, &thread_id],
        )
        .await?;

        Ok(id)
    }

    /// Update conversation last activity.
    pub async fn touch_conversation(&self, id: Uuid) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            "UPDATE conversations SET last_activity = NOW() WHERE id = $1",
            &[&id],
        )
        .await?;
        Ok(())
    }

    /// Add a message to a conversation.
    pub async fn add_conversation_message(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = Uuid::new_v4();

        conn.execute(
            "INSERT INTO conversation_messages (id, conversation_id, role, content) VALUES ($1, $2, $3, $4)",
            &[&id, &conversation_id, &role, &content],
        )
        .await?;

        // Update conversation activity
        self.touch_conversation(conversation_id).await?;

        Ok(id)
    }

    // ==================== Jobs ====================

    /// Save a job context to the database.
    pub async fn save_job(&self, ctx: &JobContext) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        let status = ctx.state.to_string();
        let estimated_time_secs = ctx.estimated_duration.map(|d| d.as_secs() as i32);

        conn.execute(
            r#"
            INSERT INTO agent_jobs (
                id, conversation_id, title, description, category, status, source,
                budget_amount, budget_token, bid_amount, estimated_cost, estimated_time_secs,
                actual_cost, repair_attempts, created_at, started_at, completed_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
            ON CONFLICT (id) DO UPDATE SET
                title = EXCLUDED.title,
                description = EXCLUDED.description,
                category = EXCLUDED.category,
                status = EXCLUDED.status,
                estimated_cost = EXCLUDED.estimated_cost,
                estimated_time_secs = EXCLUDED.estimated_time_secs,
                actual_cost = EXCLUDED.actual_cost,
                repair_attempts = EXCLUDED.repair_attempts,
                started_at = EXCLUDED.started_at,
                completed_at = EXCLUDED.completed_at
            "#,
            &[
                &ctx.job_id,
                &ctx.conversation_id,
                &ctx.title,
                &ctx.description,
                &ctx.category,
                &status,
                &"direct", // source
                &ctx.budget,
                &ctx.budget_token,
                &ctx.bid_amount,
                &ctx.estimated_cost,
                &estimated_time_secs,
                &ctx.actual_cost,
                &(ctx.repair_attempts as i32),
                &ctx.created_at,
                &ctx.started_at,
                &ctx.completed_at,
            ],
        )
        .await?;

        Ok(())
    }

    /// Get a job by ID.
    pub async fn get_job(&self, id: Uuid) -> Result<Option<JobContext>, DatabaseError> {
        let conn = self.conn().await?;

        let row = conn
            .query_opt(
                r#"
                SELECT id, conversation_id, title, description, category, status, user_id,
                       budget_amount, budget_token, bid_amount, estimated_cost, estimated_time_secs,
                       actual_cost, repair_attempts, created_at, started_at, completed_at
                FROM agent_jobs WHERE id = $1
                "#,
                &[&id],
            )
            .await?;

        match row {
            Some(row) => {
                let status_str: String = row.get("status");
                let state = parse_job_state(&status_str);
                let estimated_time_secs: Option<i32> = row.get("estimated_time_secs");

                Ok(Some(JobContext {
                    job_id: row.get("id"),
                    state,
                    user_id: row.get::<_, String>("user_id"),
                    conversation_id: row.get("conversation_id"),
                    title: row.get("title"),
                    description: row.get("description"),
                    category: row.get("category"),
                    budget: row.get("budget_amount"),
                    budget_token: row.get("budget_token"),
                    bid_amount: row.get("bid_amount"),
                    estimated_cost: row.get("estimated_cost"),
                    estimated_duration: estimated_time_secs
                        .map(|s| std::time::Duration::from_secs(s as u64)),
                    actual_cost: row
                        .get::<_, Option<Decimal>>("actual_cost")
                        .unwrap_or_default(),
                    repair_attempts: row.get::<_, i32>("repair_attempts") as u32,
                    created_at: row.get("created_at"),
                    started_at: row.get("started_at"),
                    completed_at: row.get("completed_at"),
                    transitions: Vec::new(), // Not loaded from DB for now
                    metadata: serde_json::Value::Null,
                    total_tokens_used: 0,
                    max_tokens: 0,
                }))
            }
            None => Ok(None),
        }
    }

    /// Update job status.
    pub async fn update_job_status(
        &self,
        id: Uuid,
        status: JobState,
        failure_reason: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let status_str = status.to_string();

        conn.execute(
            "UPDATE agent_jobs SET status = $2, failure_reason = $3 WHERE id = $1",
            &[&id, &status_str, &failure_reason],
        )
        .await?;

        Ok(())
    }

    /// Mark job as stuck.
    pub async fn mark_job_stuck(&self, id: Uuid) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        conn.execute(
            "UPDATE agent_jobs SET status = 'stuck', stuck_since = NOW() WHERE id = $1",
            &[&id],
        )
        .await?;

        Ok(())
    }

    /// Get stuck jobs.
    pub async fn get_stuck_jobs(&self) -> Result<Vec<Uuid>, DatabaseError> {
        let conn = self.conn().await?;

        let rows = conn
            .query("SELECT id FROM agent_jobs WHERE status = 'stuck'", &[])
            .await?;

        Ok(rows.iter().map(|r| r.get("id")).collect())
    }

    // ==================== Actions ====================

    /// Save a job action.
    pub async fn save_action(
        &self,
        job_id: Uuid,
        action: &ActionRecord,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        let duration_ms = action.duration.as_millis() as i32;
        let warnings_json = serde_json::to_value(&action.sanitization_warnings)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        conn.execute(
            r#"
            INSERT INTO job_actions (
                id, job_id, sequence_num, tool_name, input, output_raw, output_sanitized,
                sanitization_warnings, cost, duration_ms, success, error_message, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            "#,
            &[
                &action.id,
                &job_id,
                &(action.sequence as i32),
                &action.tool_name,
                &action.input,
                &action.output_raw,
                &action.output_sanitized,
                &warnings_json,
                &action.cost,
                &duration_ms,
                &action.success,
                &action.error,
                &action.executed_at,
            ],
        )
        .await?;

        Ok(())
    }

    /// Get actions for a job.
    pub async fn get_job_actions(&self, job_id: Uuid) -> Result<Vec<ActionRecord>, DatabaseError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                r#"
                SELECT id, sequence_num, tool_name, input, output_raw, output_sanitized,
                       sanitization_warnings, cost, duration_ms, success, error_message, created_at
                FROM job_actions WHERE job_id = $1 ORDER BY sequence_num
                "#,
                &[&job_id],
            )
            .await?;

        let mut actions = Vec::new();
        for row in rows {
            let duration_ms: i32 = row.get("duration_ms");
            let warnings_json: serde_json::Value = row.get("sanitization_warnings");
            let warnings: Vec<String> = serde_json::from_value(warnings_json).unwrap_or_default();

            actions.push(ActionRecord {
                id: row.get("id"),
                sequence: row.get::<_, i32>("sequence_num") as u32,
                tool_name: row.get("tool_name"),
                input: row.get("input"),
                output_raw: row.get("output_raw"),
                output_sanitized: row.get("output_sanitized"),
                sanitization_warnings: warnings,
                cost: row.get("cost"),
                duration: std::time::Duration::from_millis(duration_ms as u64),
                success: row.get("success"),
                error: row.get("error_message"),
                executed_at: row.get("created_at"),
            });
        }

        Ok(actions)
    }

    // ==================== LLM Calls ====================

    /// Record an LLM call.
    pub async fn record_llm_call(&self, record: &LlmCallRecord<'_>) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = Uuid::new_v4();

        conn.execute(
            r#"
            INSERT INTO llm_calls (id, job_id, conversation_id, provider, model, input_tokens, output_tokens, cost, purpose)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
            &[
                &id,
                &record.job_id,
                &record.conversation_id,
                &record.provider,
                &record.model,
                &(record.input_tokens as i32),
                &(record.output_tokens as i32),
                &record.cost,
                &record.purpose,
            ],
        )
        .await?;

        Ok(id)
    }

    // ==================== Estimation Snapshots ====================

    /// Save an estimation snapshot for learning.
    pub async fn save_estimation_snapshot(
        &self,
        job_id: Uuid,
        category: &str,
        tool_names: &[String],
        estimated_cost: Decimal,
        estimated_time_secs: i32,
        estimated_value: Decimal,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = Uuid::new_v4();

        conn.execute(
            r#"
            INSERT INTO estimation_snapshots (id, job_id, category, tool_names, estimated_cost, estimated_time_secs, estimated_value)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
            &[
                &id,
                &job_id,
                &category,
                &tool_names,
                &estimated_cost,
                &estimated_time_secs,
                &estimated_value,
            ],
        )
        .await?;

        Ok(id)
    }

    /// Update estimation snapshot with actual values.
    pub async fn update_estimation_actuals(
        &self,
        id: Uuid,
        actual_cost: Decimal,
        actual_time_secs: i32,
        actual_value: Option<Decimal>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        conn.execute(
            "UPDATE estimation_snapshots SET actual_cost = $2, actual_time_secs = $3, actual_value = $4 WHERE id = $1",
            &[&id, &actual_cost, &actual_time_secs, &actual_value],
        )
        .await?;

        Ok(())
    }
}

// ==================== Sandbox Jobs ====================

/// Record for a sandbox container job, persisted in the `agent_jobs` table
/// with `source = 'sandbox'`.
#[derive(Debug, Clone)]
pub struct SandboxJobRecord {
    pub id: Uuid,
    pub task: String,
    pub status: String,
    pub user_id: String,
    pub project_dir: String,
    pub success: Option<bool>,
    pub failure_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// Summary of sandbox job counts grouped by status.
#[derive(Debug, Clone, Default)]
pub struct SandboxJobSummary {
    pub total: usize,
    pub creating: usize,
    pub running: usize,
    pub completed: usize,
    pub failed: usize,
    pub interrupted: usize,
}

#[cfg(feature = "postgres")]
impl Store {
    /// Insert a new sandbox job into `agent_jobs`.
    pub async fn save_sandbox_job(&self, job: &SandboxJobRecord) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO agent_jobs (
                id, title, description, status, source, user_id, project_dir,
                success, failure_reason, created_at, started_at, completed_at
            ) VALUES ($1, $2, '', $3, 'sandbox', $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (id) DO UPDATE SET
                status = EXCLUDED.status,
                success = EXCLUDED.success,
                failure_reason = EXCLUDED.failure_reason,
                started_at = EXCLUDED.started_at,
                completed_at = EXCLUDED.completed_at
            "#,
            &[
                &job.id,
                &job.task,
                &job.status,
                &job.user_id,
                &job.project_dir,
                &job.success,
                &job.failure_reason,
                &job.created_at,
                &job.started_at,
                &job.completed_at,
            ],
        )
        .await?;
        Ok(())
    }

    /// Get a sandbox job by ID.
    pub async fn get_sandbox_job(
        &self,
        id: Uuid,
    ) -> Result<Option<SandboxJobRecord>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                r#"
                SELECT id, title, status, user_id, project_dir,
                       success, failure_reason, created_at, started_at, completed_at
                FROM agent_jobs WHERE id = $1 AND source = 'sandbox'
                "#,
                &[&id],
            )
            .await?;

        Ok(row.map(|r| SandboxJobRecord {
            id: r.get("id"),
            task: r.get("title"),
            status: r.get("status"),
            user_id: r.get("user_id"),
            project_dir: r
                .get::<_, Option<String>>("project_dir")
                .unwrap_or_default(),
            success: r.get("success"),
            failure_reason: r.get("failure_reason"),
            created_at: r.get("created_at"),
            started_at: r.get("started_at"),
            completed_at: r.get("completed_at"),
        }))
    }

    /// List all sandbox jobs, most recent first.
    pub async fn list_sandbox_jobs(&self) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT id, title, status, user_id, project_dir,
                       success, failure_reason, created_at, started_at, completed_at
                FROM agent_jobs WHERE source = 'sandbox'
                ORDER BY created_at DESC
                "#,
                &[],
            )
            .await?;

        Ok(rows
            .iter()
            .map(|r| SandboxJobRecord {
                id: r.get("id"),
                task: r.get("title"),
                status: r.get("status"),
                user_id: r.get("user_id"),
                project_dir: r
                    .get::<_, Option<String>>("project_dir")
                    .unwrap_or_default(),
                success: r.get("success"),
                failure_reason: r.get("failure_reason"),
                created_at: r.get("created_at"),
                started_at: r.get("started_at"),
                completed_at: r.get("completed_at"),
            })
            .collect())
    }

    /// List sandbox jobs for a specific user, most recent first.
    pub async fn list_sandbox_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT id, title, status, user_id, project_dir,
                       success, failure_reason, created_at, started_at, completed_at
                FROM agent_jobs WHERE source = 'sandbox' AND user_id = $1
                ORDER BY created_at DESC
                "#,
                &[&user_id],
            )
            .await?;

        Ok(rows
            .iter()
            .map(|r| SandboxJobRecord {
                id: r.get("id"),
                task: r.get("title"),
                status: r.get("status"),
                user_id: r.get("user_id"),
                project_dir: r
                    .get::<_, Option<String>>("project_dir")
                    .unwrap_or_default(),
                success: r.get("success"),
                failure_reason: r.get("failure_reason"),
                created_at: r.get("created_at"),
                started_at: r.get("started_at"),
                completed_at: r.get("completed_at"),
            })
            .collect())
    }

    /// Get a summary of sandbox job counts by status for a specific user.
    pub async fn sandbox_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<SandboxJobSummary, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT status, COUNT(*) as cnt FROM agent_jobs WHERE source = 'sandbox' AND user_id = $1 GROUP BY status",
                &[&user_id],
            )
            .await?;

        let mut summary = SandboxJobSummary::default();
        for row in &rows {
            let status: String = row.get("status");
            let count: i64 = row.get("cnt");
            let c = count as usize;
            summary.total += c;
            match status.as_str() {
                "creating" => summary.creating += c,
                "running" => summary.running += c,
                "completed" => summary.completed += c,
                "failed" => summary.failed += c,
                "interrupted" => summary.interrupted += c,
                _ => {}
            }
        }
        Ok(summary)
    }

    /// Check if a sandbox job belongs to a specific user.
    pub async fn sandbox_job_belongs_to_user(
        &self,
        job_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                "SELECT 1 FROM agent_jobs WHERE id = $1 AND user_id = $2 AND source = 'sandbox'",
                &[&job_id, &user_id],
            )
            .await?;
        Ok(row.is_some())
    }

    /// Update sandbox job status and optional timestamps/result.
    pub async fn update_sandbox_job_status(
        &self,
        id: Uuid,
        status: &str,
        success: Option<bool>,
        message: Option<&str>,
        started_at: Option<DateTime<Utc>>,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE agent_jobs SET
                status = $2,
                success = COALESCE($3, success),
                failure_reason = COALESCE($4, failure_reason),
                started_at = COALESCE($5, started_at),
                completed_at = COALESCE($6, completed_at)
            WHERE id = $1 AND source = 'sandbox'
            "#,
            &[&id, &status, &success, &message, &started_at, &completed_at],
        )
        .await?;
        Ok(())
    }

    /// Mark any sandbox jobs left in "running" or "creating" as "interrupted".
    ///
    /// Called on startup to handle jobs that were running when the process died.
    pub async fn cleanup_stale_sandbox_jobs(&self) -> Result<u64, DatabaseError> {
        let conn = self.conn().await?;
        let count = conn
            .execute(
                r#"
                UPDATE agent_jobs SET
                    status = 'interrupted',
                    failure_reason = 'Process restarted',
                    completed_at = NOW()
                WHERE source = 'sandbox' AND status IN ('running', 'creating')
                "#,
                &[],
            )
            .await?;
        if count > 0 {
            tracing::info!("Marked {} stale sandbox jobs as interrupted", count);
        }
        Ok(count)
    }

    /// Get a summary of sandbox job counts by status.
    pub async fn sandbox_job_summary(&self) -> Result<SandboxJobSummary, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT status, COUNT(*) as cnt FROM agent_jobs WHERE source = 'sandbox' GROUP BY status",
                &[],
            )
            .await?;

        let mut summary = SandboxJobSummary::default();
        for row in &rows {
            let status: String = row.get("status");
            let count: i64 = row.get("cnt");
            let c = count as usize;
            summary.total += c;
            match status.as_str() {
                "creating" => summary.creating += c,
                "running" => summary.running += c,
                "completed" => summary.completed += c,
                "failed" => summary.failed += c,
                "interrupted" => summary.interrupted += c,
                _ => {}
            }
        }
        Ok(summary)
    }
}

// ==================== Job Events ====================

/// A persisted job streaming event (from worker or Claude Code bridge).
#[derive(Debug, Clone)]
pub struct JobEventRecord {
    pub id: i64,
    pub job_id: Uuid,
    pub event_type: String,
    pub data: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[cfg(feature = "postgres")]
impl Store {
    /// Persist a job event (fire-and-forget from orchestrator handler).
    pub async fn save_job_event(
        &self,
        job_id: Uuid,
        event_type: &str,
        data: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO job_events (job_id, event_type, data)
            VALUES ($1, $2, $3)
            "#,
            &[&job_id, &event_type, data],
        )
        .await?;
        Ok(())
    }

    /// Load all job events for a job, ordered by id.
    pub async fn list_job_events(
        &self,
        job_id: Uuid,
    ) -> Result<Vec<JobEventRecord>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT id, job_id, event_type, data, created_at
                FROM job_events
                WHERE job_id = $1
                ORDER BY id ASC
                "#,
                &[&job_id],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| JobEventRecord {
                id: r.get("id"),
                job_id: r.get("job_id"),
                event_type: r.get("event_type"),
                data: r.get("data"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    /// Update the job_mode column for a sandbox job.
    pub async fn update_sandbox_job_mode(&self, id: Uuid, mode: &str) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            "UPDATE agent_jobs SET job_mode = $2 WHERE id = $1",
            &[&id, &mode],
        )
        .await?;
        Ok(())
    }

    /// Get the job_mode for a sandbox job.
    pub async fn get_sandbox_job_mode(&self, id: Uuid) -> Result<Option<String>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt("SELECT job_mode FROM agent_jobs WHERE id = $1", &[&id])
            .await?;
        Ok(row.map(|r| r.get("job_mode")))
    }
}

// ==================== Routines ====================

#[cfg(feature = "postgres")]
use crate::agent::routine::{
    NotifyConfig, Routine, RoutineAction, RoutineGuardrails, RoutineRun, RunStatus, Trigger,
};

#[cfg(feature = "postgres")]
impl Store {
    /// Create a new routine.
    pub async fn create_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let trigger_type = routine.trigger.type_tag();
        let trigger_config = routine.trigger.to_config_json();
        let action_type = routine.action.type_tag();
        let action_config = routine.action.to_config_json();
        let cooldown_secs = routine.guardrails.cooldown.as_secs() as i32;
        let max_concurrent = routine.guardrails.max_concurrent as i32;
        let dedup_window_secs = routine.guardrails.dedup_window.map(|d| d.as_secs() as i32);

        conn.execute(
            r#"
            INSERT INTO routines (
                id, name, description, user_id, enabled,
                trigger_type, trigger_config, action_type, action_config,
                cooldown_secs, max_concurrent, dedup_window_secs,
                notify_channel, notify_user, notify_on_success, notify_on_failure, notify_on_attention,
                state, next_fire_at, created_at, updated_at
            ) VALUES (
                $1, $2, $3, $4, $5,
                $6, $7, $8, $9,
                $10, $11, $12,
                $13, $14, $15, $16, $17,
                $18, $19, $20, $21
            )
            "#,
            &[
                &routine.id,
                &routine.name,
                &routine.description,
                &routine.user_id,
                &routine.enabled,
                &trigger_type,
                &trigger_config,
                &action_type,
                &action_config,
                &cooldown_secs,
                &max_concurrent,
                &dedup_window_secs,
                &routine.notify.channel,
                &routine.notify.user,
                &routine.notify.on_success,
                &routine.notify.on_failure,
                &routine.notify.on_attention,
                &routine.state,
                &routine.next_fire_at,
                &routine.created_at,
                &routine.updated_at,
            ],
        )
        .await?;

        Ok(())
    }

    /// Get a routine by ID.
    pub async fn get_routine(&self, id: Uuid) -> Result<Option<Routine>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt("SELECT * FROM routines WHERE id = $1", &[&id])
            .await?;
        row.map(|r| row_to_routine(&r)).transpose()
    }

    /// Get a routine by user_id and name.
    pub async fn get_routine_by_name(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<Routine>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                "SELECT * FROM routines WHERE user_id = $1 AND name = $2",
                &[&user_id, &name],
            )
            .await?;
        row.map(|r| row_to_routine(&r)).transpose()
    }

    /// List routines for a user.
    pub async fn list_routines(&self, user_id: &str) -> Result<Vec<Routine>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT * FROM routines WHERE user_id = $1 ORDER BY name",
                &[&user_id],
            )
            .await?;
        rows.iter().map(row_to_routine).collect()
    }

    /// List all enabled routines with event triggers (for event matching).
    pub async fn list_event_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT * FROM routines WHERE enabled AND trigger_type = 'event'",
                &[],
            )
            .await?;
        rows.iter().map(row_to_routine).collect()
    }

    /// List all enabled cron routines whose next_fire_at <= now.
    pub async fn list_due_cron_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        let conn = self.conn().await?;
        let now = Utc::now();
        let rows = conn
            .query(
                r#"
                SELECT * FROM routines
                WHERE enabled
                  AND trigger_type = 'cron'
                  AND next_fire_at IS NOT NULL
                  AND next_fire_at <= $1
                "#,
                &[&now],
            )
            .await?;
        rows.iter().map(row_to_routine).collect()
    }

    /// Update a routine (full replacement of mutable fields).
    pub async fn update_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let trigger_type = routine.trigger.type_tag();
        let trigger_config = routine.trigger.to_config_json();
        let action_type = routine.action.type_tag();
        let action_config = routine.action.to_config_json();
        let cooldown_secs = routine.guardrails.cooldown.as_secs() as i32;
        let max_concurrent = routine.guardrails.max_concurrent as i32;
        let dedup_window_secs = routine.guardrails.dedup_window.map(|d| d.as_secs() as i32);

        conn.execute(
            r#"
            UPDATE routines SET
                name = $2, description = $3, enabled = $4,
                trigger_type = $5, trigger_config = $6,
                action_type = $7, action_config = $8,
                cooldown_secs = $9, max_concurrent = $10, dedup_window_secs = $11,
                notify_channel = $12, notify_user = $13,
                notify_on_success = $14, notify_on_failure = $15, notify_on_attention = $16,
                state = $17, next_fire_at = $18,
                updated_at = now()
            WHERE id = $1
            "#,
            &[
                &routine.id,
                &routine.name,
                &routine.description,
                &routine.enabled,
                &trigger_type,
                &trigger_config,
                &action_type,
                &action_config,
                &cooldown_secs,
                &max_concurrent,
                &dedup_window_secs,
                &routine.notify.channel,
                &routine.notify.user,
                &routine.notify.on_success,
                &routine.notify.on_failure,
                &routine.notify.on_attention,
                &routine.state,
                &routine.next_fire_at,
            ],
        )
        .await?;
        Ok(())
    }

    /// Update runtime state after a routine fires.
    pub async fn update_routine_runtime(
        &self,
        id: Uuid,
        last_run_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
        run_count: u64,
        consecutive_failures: u32,
        state: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE routines SET
                last_run_at = $2, next_fire_at = $3,
                run_count = $4, consecutive_failures = $5,
                state = $6, updated_at = now()
            WHERE id = $1
            "#,
            &[
                &id,
                &last_run_at,
                &next_fire_at,
                &(run_count as i64),
                &(consecutive_failures as i32),
                state,
            ],
        )
        .await?;
        Ok(())
    }

    /// Delete a routine.
    pub async fn delete_routine(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let count = conn
            .execute("DELETE FROM routines WHERE id = $1", &[&id])
            .await?;
        Ok(count > 0)
    }

    // ==================== Routine Runs ====================

    /// Record a routine run starting.
    pub async fn create_routine_run(&self, run: &RoutineRun) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let status = run.status.to_string();
        conn.execute(
            r#"
            INSERT INTO routine_runs (
                id, routine_id, trigger_type, trigger_detail,
                started_at, status, job_id
            ) VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
            &[
                &run.id,
                &run.routine_id,
                &run.trigger_type,
                &run.trigger_detail,
                &run.started_at,
                &status,
                &run.job_id,
            ],
        )
        .await?;
        Ok(())
    }

    /// Complete a routine run.
    pub async fn complete_routine_run(
        &self,
        id: Uuid,
        status: RunStatus,
        result_summary: Option<&str>,
        tokens_used: Option<i32>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let status_str = status.to_string();
        let now = Utc::now();
        conn.execute(
            r#"
            UPDATE routine_runs SET
                completed_at = $2, status = $3,
                result_summary = $4, tokens_used = $5
            WHERE id = $1
            "#,
            &[&id, &now, &status_str, &result_summary, &tokens_used],
        )
        .await?;
        Ok(())
    }

    /// List recent runs for a routine.
    pub async fn list_routine_runs(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT * FROM routine_runs
                WHERE routine_id = $1
                ORDER BY started_at DESC
                LIMIT $2
                "#,
                &[&routine_id, &limit],
            )
            .await?;
        rows.iter().map(row_to_routine_run).collect()
    }

    /// Count currently running runs for a routine.
    pub async fn count_running_routine_runs(&self, routine_id: Uuid) -> Result<i64, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_one(
                "SELECT COUNT(*) as cnt FROM routine_runs WHERE routine_id = $1 AND status = 'running'",
                &[&routine_id],
            )
            .await?;
        Ok(row.get("cnt"))
    }
}

#[cfg(feature = "postgres")]
fn row_to_routine(row: &tokio_postgres::Row) -> Result<Routine, DatabaseError> {
    let trigger_type: String = row.get("trigger_type");
    let trigger_config: serde_json::Value = row.get("trigger_config");
    let action_type: String = row.get("action_type");
    let action_config: serde_json::Value = row.get("action_config");
    let cooldown_secs: i32 = row.get("cooldown_secs");
    let max_concurrent: i32 = row.get("max_concurrent");
    let dedup_window_secs: Option<i32> = row.get("dedup_window_secs");

    let trigger =
        Trigger::from_db(&trigger_type, trigger_config).map_err(DatabaseError::Serialization)?;
    let action = RoutineAction::from_db(&action_type, action_config)
        .map_err(DatabaseError::Serialization)?;

    Ok(Routine {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        user_id: row.get("user_id"),
        enabled: row.get("enabled"),
        trigger,
        action,
        guardrails: RoutineGuardrails {
            cooldown: std::time::Duration::from_secs(cooldown_secs as u64),
            max_concurrent: max_concurrent as u32,
            dedup_window: dedup_window_secs.map(|s| std::time::Duration::from_secs(s as u64)),
        },
        notify: NotifyConfig {
            channel: row.get("notify_channel"),
            user: row.get("notify_user"),
            on_attention: row.get("notify_on_attention"),
            on_failure: row.get("notify_on_failure"),
            on_success: row.get("notify_on_success"),
        },
        last_run_at: row.get("last_run_at"),
        next_fire_at: row.get("next_fire_at"),
        run_count: row.get::<_, i64>("run_count") as u64,
        consecutive_failures: row.get::<_, i32>("consecutive_failures") as u32,
        state: row.get("state"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

#[cfg(feature = "postgres")]
fn row_to_routine_run(row: &tokio_postgres::Row) -> Result<RoutineRun, DatabaseError> {
    let status_str: String = row.get("status");
    let status: RunStatus = status_str
        .parse()
        .map_err(|e: String| DatabaseError::Serialization(e))?;

    Ok(RoutineRun {
        id: row.get("id"),
        routine_id: row.get("routine_id"),
        trigger_type: row.get("trigger_type"),
        trigger_detail: row.get("trigger_detail"),
        started_at: row.get("started_at"),
        completed_at: row.get("completed_at"),
        status,
        result_summary: row.get("result_summary"),
        tokens_used: row.get("tokens_used"),
        job_id: row.get("job_id"),
        created_at: row.get("created_at"),
    })
}

// ==================== Conversation Persistence ====================

/// Summary of a conversation for the thread list.
#[derive(Debug, Clone)]
pub struct ConversationSummary {
    pub id: Uuid,
    /// First user message, truncated to 100 chars.
    pub title: Option<String>,
    pub message_count: i64,
    pub started_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    /// Thread type extracted from metadata (e.g. "assistant", "thread").
    pub thread_type: Option<String>,
}

/// A single message in a conversation.
#[derive(Debug, Clone)]
pub struct ConversationMessage {
    pub id: Uuid,
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

#[cfg(feature = "postgres")]
impl Store {
    /// Ensure a conversation row exists for a given UUID.
    ///
    /// Idempotent: inserts on first call, bumps `last_activity` on subsequent calls.
    pub async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO conversations (id, channel, user_id, thread_id)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (id) DO UPDATE SET last_activity = NOW()
            "#,
            &[&id, &channel, &user_id, &thread_id],
        )
        .await?;
        Ok(())
    }

    /// List conversations with a title derived from the first user message.
    pub async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        channel: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT
                    c.id,
                    c.started_at,
                    c.last_activity,
                    c.metadata,
                    (SELECT COUNT(*) FROM conversation_messages m WHERE m.conversation_id = c.id) AS message_count,
                    (SELECT LEFT(m2.content, 100)
                     FROM conversation_messages m2
                     WHERE m2.conversation_id = c.id AND m2.role = 'user'
                     ORDER BY m2.created_at ASC
                     LIMIT 1
                    ) AS title
                FROM conversations c
                WHERE c.user_id = $1 AND c.channel = $2
                ORDER BY c.last_activity DESC
                LIMIT $3
                "#,
                &[&user_id, &channel, &limit],
            )
            .await?;

        Ok(rows
            .iter()
            .map(|r| {
                let metadata: serde_json::Value = r.get("metadata");
                let thread_type = metadata
                    .get("thread_type")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                ConversationSummary {
                    id: r.get("id"),
                    title: r.get("title"),
                    message_count: r.get("message_count"),
                    started_at: r.get("started_at"),
                    last_activity: r.get("last_activity"),
                    thread_type,
                }
            })
            .collect())
    }

    /// Get or create the singleton "assistant" conversation for a user+channel.
    ///
    /// Looks for a conversation where `metadata->>'thread_type' = 'assistant'`.
    /// Creates one if it doesn't exist.
    pub async fn get_or_create_assistant_conversation(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;

        // Try to find existing assistant conversation
        let row = conn
            .query_opt(
                r#"
                SELECT id FROM conversations
                WHERE user_id = $1 AND channel = $2 AND metadata->>'thread_type' = 'assistant'
                LIMIT 1
                "#,
                &[&user_id, &channel],
            )
            .await?;

        if let Some(row) = row {
            return Ok(row.get("id"));
        }

        // Create a new assistant conversation
        let id = Uuid::new_v4();
        let metadata = serde_json::json!({"thread_type": "assistant", "title": "Assistant"});
        conn.execute(
            r#"
            INSERT INTO conversations (id, channel, user_id, metadata)
            VALUES ($1, $2, $3, $4)
            "#,
            &[&id, &channel, &user_id, &metadata],
        )
        .await?;

        Ok(id)
    }

    /// Create a conversation with specific metadata.
    pub async fn create_conversation_with_metadata(
        &self,
        channel: &str,
        user_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = Uuid::new_v4();

        conn.execute(
            "INSERT INTO conversations (id, channel, user_id, metadata) VALUES ($1, $2, $3, $4)",
            &[&id, &channel, &user_id, metadata],
        )
        .await?;

        Ok(id)
    }

    /// Check whether a conversation belongs to the given user.
    pub async fn conversation_belongs_to_user(
        &self,
        conversation_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                "SELECT 1 FROM conversations WHERE id = $1 AND user_id = $2",
                &[&conversation_id, &user_id],
            )
            .await?;
        Ok(row.is_some())
    }

    /// Load messages for a conversation with cursor-based pagination.
    ///
    /// Returns `(messages_oldest_first, has_more)`.
    /// Pass `before` as a cursor to load older messages.
    pub async fn list_conversation_messages_paginated(
        &self,
        conversation_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<(Vec<ConversationMessage>, bool), DatabaseError> {
        let conn = self.conn().await?;
        let fetch_limit = limit + 1; // Fetch one extra to determine has_more

        let rows = if let Some(before_ts) = before {
            conn.query(
                r#"
                SELECT id, role, content, created_at
                FROM conversation_messages
                WHERE conversation_id = $1 AND created_at < $2
                ORDER BY created_at DESC
                LIMIT $3
                "#,
                &[&conversation_id, &before_ts, &fetch_limit],
            )
            .await?
        } else {
            conn.query(
                r#"
                SELECT id, role, content, created_at
                FROM conversation_messages
                WHERE conversation_id = $1
                ORDER BY created_at DESC
                LIMIT $2
                "#,
                &[&conversation_id, &fetch_limit],
            )
            .await?
        };

        let has_more = rows.len() as i64 > limit;
        let take_count = (rows.len() as i64).min(limit) as usize;

        // Rows come newest-first from DB; reverse so caller gets oldest-first
        let mut messages: Vec<ConversationMessage> = rows
            .iter()
            .take(take_count)
            .map(|r| ConversationMessage {
                id: r.get("id"),
                role: r.get("role"),
                content: r.get("content"),
                created_at: r.get("created_at"),
            })
            .collect();
        messages.reverse();

        Ok((messages, has_more))
    }

    /// Merge a single key into a conversation's metadata JSONB.
    pub async fn update_conversation_metadata_field(
        &self,
        id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let patch = serde_json::json!({ key: value });
        conn.execute(
            "UPDATE conversations SET metadata = metadata || $2 WHERE id = $1",
            &[&id, &patch],
        )
        .await?;
        Ok(())
    }

    /// Read the metadata JSONB for a conversation.
    pub async fn get_conversation_metadata(
        &self,
        id: Uuid,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt("SELECT metadata FROM conversations WHERE id = $1", &[&id])
            .await?;
        Ok(row.map(|r| r.get::<_, serde_json::Value>(0)))
    }

    /// Load all messages for a conversation, ordered chronologically.
    pub async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT id, role, content, created_at
                FROM conversation_messages
                WHERE conversation_id = $1
                ORDER BY created_at ASC
                "#,
                &[&conversation_id],
            )
            .await?;

        Ok(rows
            .iter()
            .map(|r| ConversationMessage {
                id: r.get("id"),
                role: r.get("role"),
                content: r.get("content"),
                created_at: r.get("created_at"),
            })
            .collect())
    }
}

#[cfg(feature = "postgres")]
fn parse_job_state(s: &str) -> JobState {
    match s {
        "pending" => JobState::Pending,
        "in_progress" => JobState::InProgress,
        "completed" => JobState::Completed,
        "submitted" => JobState::Submitted,
        "accepted" => JobState::Accepted,
        "failed" => JobState::Failed,
        "stuck" => JobState::Stuck,
        "cancelled" => JobState::Cancelled,
        _ => JobState::Pending,
    }
}

// ==================== Tool Failures ====================

#[cfg(feature = "postgres")]
use crate::agent::BrokenTool;

#[cfg(feature = "postgres")]
impl Store {
    /// Record a tool failure (upsert: increment count if exists).
    pub async fn record_tool_failure(
        &self,
        tool_name: &str,
        error_message: &str,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        conn.execute(
            r#"
            INSERT INTO tool_failures (tool_name, error_message, error_count, last_failure)
            VALUES ($1, $2, 1, NOW())
            ON CONFLICT (tool_name) DO UPDATE SET
                error_message = $2,
                error_count = tool_failures.error_count + 1,
                last_failure = NOW()
            "#,
            &[&tool_name, &error_message],
        )
        .await?;

        Ok(())
    }

    /// Get tools that have failed more than `threshold` times and haven't been repaired.
    pub async fn get_broken_tools(&self, threshold: i32) -> Result<Vec<BrokenTool>, DatabaseError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                r#"
                SELECT tool_name, error_message, error_count, first_failure, last_failure,
                       last_build_result, repair_attempts
                FROM tool_failures
                WHERE error_count >= $1 AND repaired_at IS NULL
                ORDER BY error_count DESC
                "#,
                &[&threshold],
            )
            .await?;

        Ok(rows
            .iter()
            .map(|row| BrokenTool {
                name: row.get("tool_name"),
                last_error: row.get("error_message"),
                failure_count: row.get::<_, i32>("error_count") as u32,
                first_failure: row.get("first_failure"),
                last_failure: row.get("last_failure"),
                last_build_result: row.get("last_build_result"),
                repair_attempts: row.get::<_, i32>("repair_attempts") as u32,
            })
            .collect())
    }

    /// Mark a tool as repaired.
    pub async fn mark_tool_repaired(&self, tool_name: &str) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        conn.execute(
            "UPDATE tool_failures SET repaired_at = NOW(), error_count = 0 WHERE tool_name = $1",
            &[&tool_name],
        )
        .await?;

        Ok(())
    }

    /// Increment repair attempts for a tool.
    pub async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        conn.execute(
            "UPDATE tool_failures SET repair_attempts = repair_attempts + 1 WHERE tool_name = $1",
            &[&tool_name],
        )
        .await?;

        Ok(())
    }
}

// ==================== Settings ====================

/// A single setting row from the database.
#[derive(Debug, Clone)]
pub struct SettingRow {
    pub key: String,
    pub value: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

#[cfg(feature = "postgres")]
impl Store {
    /// Get a single setting by key.
    pub async fn get_setting(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                "SELECT value FROM settings WHERE user_id = $1 AND key = $2",
                &[&user_id, &key],
            )
            .await?;
        Ok(row.map(|r| r.get("value")))
    }

    /// Get a single setting with full metadata.
    pub async fn get_setting_full(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<SettingRow>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                "SELECT key, value, updated_at FROM settings WHERE user_id = $1 AND key = $2",
                &[&user_id, &key],
            )
            .await?;
        Ok(row.map(|r| SettingRow {
            key: r.get("key"),
            value: r.get("value"),
            updated_at: r.get("updated_at"),
        }))
    }

    /// Set a single setting (upsert).
    pub async fn set_setting(
        &self,
        user_id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO settings (user_id, key, value, updated_at)
            VALUES ($1, $2, $3, NOW())
            ON CONFLICT (user_id, key) DO UPDATE SET
                value = EXCLUDED.value,
                updated_at = NOW()
            "#,
            &[&user_id, &key, value],
        )
        .await?;
        Ok(())
    }

    /// Delete a single setting (reset to default).
    pub async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let count = conn
            .execute(
                "DELETE FROM settings WHERE user_id = $1 AND key = $2",
                &[&user_id, &key],
            )
            .await?;
        Ok(count > 0)
    }

    /// List all settings for a user (with metadata).
    pub async fn list_settings(&self, user_id: &str) -> Result<Vec<SettingRow>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT key, value, updated_at FROM settings WHERE user_id = $1 ORDER BY key",
                &[&user_id],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| SettingRow {
                key: r.get("key"),
                value: r.get("value"),
                updated_at: r.get("updated_at"),
            })
            .collect())
    }

    /// Get all settings as a flat key-value map.
    pub async fn get_all_settings(
        &self,
        user_id: &str,
    ) -> Result<std::collections::HashMap<String, serde_json::Value>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT key, value FROM settings WHERE user_id = $1",
                &[&user_id],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| {
                let key: String = r.get("key");
                let value: serde_json::Value = r.get("value");
                (key, value)
            })
            .collect())
    }

    /// Bulk-write settings (used for migration/import).
    ///
    /// Each entry is upserted individually within a single transaction.
    pub async fn set_all_settings(
        &self,
        user_id: &str,
        settings: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        let mut conn = self.conn().await?;
        let tx = conn.transaction().await?;

        for (key, value) in settings {
            tx.execute(
                r#"
                INSERT INTO settings (user_id, key, value, updated_at)
                VALUES ($1, $2, $3, NOW())
                ON CONFLICT (user_id, key) DO UPDATE SET
                    value = EXCLUDED.value,
                    updated_at = NOW()
                "#,
                &[&user_id, &key, value],
            )
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Check if the settings table has any rows for a user.
    pub async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_one(
                "SELECT COUNT(*) as cnt FROM settings WHERE user_id = $1",
                &[&user_id],
            )
            .await?;
        let count: i64 = row.get("cnt");
        Ok(count > 0)
    }
}
