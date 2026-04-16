//! Sandbox-related SandboxStore implementation for LibSqlBackend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use libsql::params;
use uuid::Uuid;

use super::{
    LibSqlBackend, fmt_opt_ts, fmt_ts, get_i64, get_json, get_opt_bool, get_opt_text, get_opt_ts,
    get_text, get_ts, opt_text, opt_text_owned,
};
use crate::db::SandboxStore;
use crate::error::DatabaseError;
use crate::history::{JobEventRecord, SandboxJobRecord, SandboxJobSummary, SandboxRestartParams};

#[async_trait]
impl SandboxStore for LibSqlBackend {
    async fn save_sandbox_job(&self, job: &SandboxJobRecord) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let restart_params_json =
            SandboxRestartParams::from_record(job.mcp_servers.as_deref(), job.max_iterations)
                .as_ref()
                .and_then(SandboxRestartParams::to_json);
        conn.execute(
            r#"
                INSERT INTO agent_jobs (
                    id, title, description, status, source, user_id, workspace_id, project_dir,
                    success, failure_reason, created_at, started_at, completed_at,
                    restart_params
                ) VALUES (?1, ?2, ?3, ?4, 'sandbox', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                ON CONFLICT (id) DO UPDATE SET
                    status = excluded.status,
                    workspace_id = excluded.workspace_id,
                    success = excluded.success,
                    failure_reason = excluded.failure_reason,
                    started_at = excluded.started_at,
                    completed_at = excluded.completed_at,
                    restart_params = excluded.restart_params
                "#,
            params![
                job.id.to_string(),
                job.task.as_str(),
                job.credential_grants_json.as_str(),
                job.status.as_str(),
                job.user_id.as_str(),
                opt_text_owned(job.workspace_id.map(|id| id.to_string())),
                job.project_dir.as_str(),
                job.success.map(|b| b as i64),
                opt_text(job.failure_reason.as_deref()),
                fmt_ts(&job.created_at),
                fmt_opt_ts(&job.started_at),
                fmt_opt_ts(&job.completed_at),
                opt_text(restart_params_json.as_deref()),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_sandbox_job(&self, id: Uuid) -> Result<Option<SandboxJobRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, title, description, status, user_id, workspace_id, project_dir,
                       success, failure_reason, created_at, started_at, completed_at,
                       restart_params
                FROM agent_jobs WHERE id = ?1 AND source = 'sandbox'
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
                let restart_params =
                    SandboxRestartParams::from_column(get_opt_text(&row, 12).as_deref());
                Ok(Some(SandboxJobRecord {
                    id: get_text(&row, 0).parse().unwrap_or_default(),
                    task: get_text(&row, 1),
                    credential_grants_json: get_text(&row, 2),
                    status: get_text(&row, 3),
                    user_id: get_text(&row, 4),
                    workspace_id: get_opt_text(&row, 5).and_then(|s| s.parse().ok()),
                    project_dir: get_text(&row, 6),
                    success: get_opt_bool(&row, 7),
                    failure_reason: get_opt_text(&row, 8),
                    created_at: get_ts(&row, 9),
                    started_at: get_opt_ts(&row, 10),
                    completed_at: get_opt_ts(&row, 11),
                    mcp_servers: restart_params.mcp_servers,
                    max_iterations: restart_params.max_iterations,
                }))
            }
            None => Ok(None),
        }
    }

    async fn list_sandbox_jobs(&self) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, title, description, status, user_id, workspace_id, project_dir,
                       success, failure_reason, created_at, started_at, completed_at,
                       restart_params
                FROM agent_jobs WHERE source = 'sandbox'
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
            let restart_params =
                SandboxRestartParams::from_column(get_opt_text(&row, 12).as_deref());
            jobs.push(SandboxJobRecord {
                id: get_text(&row, 0).parse().unwrap_or_default(),
                task: get_text(&row, 1),
                credential_grants_json: get_text(&row, 2),
                status: get_text(&row, 3),
                user_id: get_text(&row, 4),
                workspace_id: get_opt_text(&row, 5).and_then(|s| s.parse().ok()),
                project_dir: get_text(&row, 6),
                success: get_opt_bool(&row, 7),
                failure_reason: get_opt_text(&row, 8),
                created_at: get_ts(&row, 9),
                started_at: get_opt_ts(&row, 10),
                completed_at: get_opt_ts(&row, 11),
                mcp_servers: restart_params.mcp_servers,
                max_iterations: restart_params.max_iterations,
            });
        }
        Ok(jobs)
    }

    async fn update_sandbox_job_status(
        &self,
        id: Uuid,
        status: &str,
        success: Option<bool>,
        message: Option<&str>,
        started_at: Option<DateTime<Utc>>,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
                UPDATE agent_jobs SET
                    status = ?2,
                    success = COALESCE(?3, success),
                    failure_reason = COALESCE(?4, failure_reason),
                    started_at = COALESCE(?5, started_at),
                    completed_at = COALESCE(?6, completed_at)
                WHERE id = ?1 AND source = 'sandbox'
                "#,
            params![
                id.to_string(),
                status,
                success.map(|b| b as i64),
                message,
                fmt_opt_ts(&started_at),
                fmt_opt_ts(&completed_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn cleanup_stale_sandbox_jobs(&self) -> Result<u64, DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        let count = conn
            .execute(
                r#"
                UPDATE agent_jobs SET
                    status = 'interrupted',
                    failure_reason = 'Process restarted',
                    completed_at = ?1
                WHERE source = 'sandbox' AND status IN ('running', 'creating')
                "#,
                params![now],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        if count > 0 {
            tracing::info!("Marked {} stale sandbox jobs as interrupted", count);
        }
        Ok(count)
    }

    async fn sandbox_job_summary(&self) -> Result<SandboxJobSummary, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT status, COUNT(*) as cnt FROM agent_jobs WHERE source = 'sandbox' GROUP BY status",
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut summary = SandboxJobSummary::default();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let status = get_text(&row, 0);
            let count = get_i64(&row, 1) as usize;
            summary.total += count;
            match status.as_str() {
                "creating" => summary.creating += count,
                "running" => summary.running += count,
                "completed" => summary.completed += count,
                "failed" => summary.failed += count,
                "interrupted" => summary.interrupted += count,
                _ => {}
            }
        }
        Ok(summary)
    }

    async fn list_sandbox_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, title, description, status, user_id, workspace_id, project_dir,
                       success, failure_reason, created_at, started_at, completed_at,
                       restart_params
                FROM agent_jobs WHERE source = 'sandbox' AND user_id = ?1 AND workspace_id IS NULL
                ORDER BY created_at DESC
                "#,
                libsql::params![user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut jobs = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let restart_params =
                SandboxRestartParams::from_column(get_opt_text(&row, 12).as_deref());
            jobs.push(SandboxJobRecord {
                id: get_text(&row, 0).parse().unwrap_or_default(),
                task: get_text(&row, 1),
                credential_grants_json: get_text(&row, 2),
                status: get_text(&row, 3),
                user_id: get_text(&row, 4),
                workspace_id: get_opt_text(&row, 5).and_then(|s| s.parse().ok()),
                project_dir: get_text(&row, 6),
                success: get_opt_bool(&row, 7),
                failure_reason: get_opt_text(&row, 8),
                created_at: get_ts(&row, 9),
                started_at: get_opt_ts(&row, 10),
                completed_at: get_opt_ts(&row, 11),
                mcp_servers: restart_params.mcp_servers,
                max_iterations: restart_params.max_iterations,
            });
        }
        Ok(jobs)
    }

    async fn list_sandbox_jobs_for_workspace(
        &self,
        workspace_id: Uuid,
    ) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, title, description, status, user_id, workspace_id, project_dir,
                       success, failure_reason, created_at, started_at, completed_at,
                       restart_params
                FROM agent_jobs WHERE source = 'sandbox' AND workspace_id = ?1
                ORDER BY created_at DESC
                "#,
                params![workspace_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut jobs = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let restart_params =
                SandboxRestartParams::from_column(get_opt_text(&row, 12).as_deref());
            jobs.push(SandboxJobRecord {
                id: get_text(&row, 0).parse().unwrap_or_default(),
                task: get_text(&row, 1),
                credential_grants_json: get_text(&row, 2),
                status: get_text(&row, 3),
                user_id: get_text(&row, 4),
                workspace_id: get_opt_text(&row, 5).and_then(|s| s.parse().ok()),
                project_dir: get_text(&row, 6),
                success: get_opt_bool(&row, 7),
                failure_reason: get_opt_text(&row, 8),
                created_at: get_ts(&row, 9),
                started_at: get_opt_ts(&row, 10),
                completed_at: get_opt_ts(&row, 11),
                mcp_servers: restart_params.mcp_servers,
                max_iterations: restart_params.max_iterations,
            });
        }
        Ok(jobs)
    }

    async fn sandbox_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<SandboxJobSummary, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT status, COUNT(*) as cnt FROM agent_jobs WHERE source = 'sandbox' AND user_id = ?1 AND workspace_id IS NULL GROUP BY status",
                libsql::params![user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut summary = SandboxJobSummary::default();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let status = get_text(&row, 0);
            let count = get_i64(&row, 1) as usize;
            summary.total += count;
            match status.as_str() {
                "creating" => summary.creating += count,
                "running" => summary.running += count,
                "completed" => summary.completed += count,
                "failed" => summary.failed += count,
                "interrupted" => summary.interrupted += count,
                _ => {}
            }
        }
        Ok(summary)
    }

    async fn sandbox_job_summary_for_workspace(
        &self,
        workspace_id: Uuid,
    ) -> Result<SandboxJobSummary, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT status, COUNT(*) as cnt FROM agent_jobs WHERE source = 'sandbox' AND workspace_id = ?1 GROUP BY status",
                params![workspace_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut summary = SandboxJobSummary::default();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let status = get_text(&row, 0);
            let count = get_i64(&row, 1) as usize;
            summary.total += count;
            match status.as_str() {
                "creating" => summary.creating += count,
                "running" => summary.running += count,
                "completed" => summary.completed += count,
                "failed" => summary.failed += count,
                "interrupted" => summary.interrupted += count,
                _ => {}
            }
        }
        Ok(summary)
    }

    async fn sandbox_job_belongs_to_user(
        &self,
        job_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT 1 FROM agent_jobs WHERE id = ?1 AND user_id = ?2 AND source = 'sandbox' AND workspace_id IS NULL",
                libsql::params![job_id.to_string(), user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let found = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(found.is_some())
    }

    async fn update_sandbox_job_mode(&self, id: Uuid, mode: &str) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE agent_jobs SET job_mode = ?2 WHERE id = ?1",
            params![id.to_string(), mode],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_sandbox_job_mode(&self, id: Uuid) -> Result<Option<String>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT job_mode FROM agent_jobs WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(get_text(&row, 0))),
            None => Ok(None),
        }
    }

    async fn save_job_event(
        &self,
        job_id: Uuid,
        event_type: &str,
        data: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO job_events (job_id, event_type, data) VALUES (?1, ?2, ?3)",
            params![job_id.to_string(), event_type, data.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_job_events(
        &self,
        job_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<JobEventRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = if let Some(n) = limit {
            conn.query(
                r#"
                SELECT id, job_id, event_type, data, created_at
                FROM (
                    SELECT id, job_id, event_type, data, created_at
                    FROM job_events WHERE job_id = ?1
                    ORDER BY id DESC
                    LIMIT ?2
                )
                ORDER BY id ASC
                "#,
                params![job_id.to_string(), n],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        } else {
            conn.query(
                r#"
                SELECT id, job_id, event_type, data, created_at
                FROM job_events WHERE job_id = ?1 ORDER BY id ASC
                "#,
                params![job_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        };

        let mut events = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            events.push(JobEventRecord {
                id: get_i64(&row, 0),
                job_id: get_text(&row, 1).parse().unwrap_or_default(),
                event_type: get_text(&row, 2),
                data: get_json(&row, 3),
                created_at: get_ts(&row, 4),
            });
        }
        Ok(events)
    }
}
