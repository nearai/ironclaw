//! PostgreSQL store for persisting agent data.

use chrono::{DateTime, Utc};
use deadpool_postgres::{Config, Pool, Runtime};
use rust_decimal::Decimal;
use tokio_postgres::NoTls;
use uuid::Uuid;

use crate::config::DatabaseConfig;
use crate::context::{ActionRecord, JobContext, JobState};
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
pub struct Store {
    pool: Pool,
}

impl Store {
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
                status = EXCLUDED.status,
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

use crate::agent::BrokenTool;

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
