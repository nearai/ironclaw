//! Aggregate usage analytics for the `ironclaw insights` CLI (libSQL backend).
//!
//! All queries are bounded by `agent_jobs.created_at >= ?since` so the work
//! scales with the time window, not the table size. Tool frequency joins
//! `job_actions` to `agent_jobs` so we don't aggregate actions whose parent
//! job falls outside the window.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use libsql::params;

use super::{LibSqlBackend, fmt_ts, get_i64, get_text};
use crate::db::{DailyActivity, InsightsAggregate, InsightsStore, ToolFrequency};
use crate::error::DatabaseError;

#[async_trait]
impl InsightsStore for LibSqlBackend {
    async fn aggregate_insights(
        &self,
        since: DateTime<Utc>,
        top_tools_limit: i64,
    ) -> Result<InsightsAggregate, DatabaseError> {
        let since_text = fmt_ts(&since);
        let conn = self.connect().await?;

        // Single round-trip per metric. Each query uses the
        // `idx_agent_jobs_created` index for the time-window scan.

        // Total jobs and total tokens used in the window.
        let mut row = conn
            .query(
                r#"
                SELECT
                    COUNT(*) AS total_jobs,
                    COALESCE(SUM(total_tokens_used), 0) AS total_tokens
                FROM agent_jobs
                WHERE created_at >= ?1
                "#,
                params![since_text.clone()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let (total_jobs, total_tokens_used) = match row
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(r) => (get_i64(&r, 0).max(0) as u64, get_i64(&r, 1).max(0) as u64),
            None => (0, 0),
        };

        // Total routine runs in the window (use created_at to align with jobs).
        let mut row = conn
            .query(
                "SELECT COUNT(*) FROM routine_runs WHERE created_at >= ?1",
                params![since_text.clone()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let total_routine_runs = match row
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(r) => get_i64(&r, 0).max(0) as u64,
            None => 0,
        };

        // Top tools by invocation count, joined to agent_jobs so the time
        // window applies. `top_tools_limit` is clamped to >=1 to avoid a
        // zero/negative LIMIT producing surprises.
        let limit = top_tools_limit.max(1);
        let mut rows = conn
            .query(
                r#"
                SELECT ja.tool_name, COUNT(*) AS invocations
                FROM job_actions ja
                INNER JOIN agent_jobs aj ON aj.id = ja.job_id
                WHERE aj.created_at >= ?1
                GROUP BY ja.tool_name
                ORDER BY invocations DESC, ja.tool_name ASC
                LIMIT ?2
                "#,
                params![since_text.clone(), limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut top_tools = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            top_tools.push(ToolFrequency {
                tool_name: get_text(&row, 0),
                invocations: get_i64(&row, 1).max(0) as u64,
            });
        }

        // Daily activity histogram. SQLite's `substr(created_at, 1, 10)` works
        // because created_at is an RFC 3339 ISO string starting with the date.
        let mut rows = conn
            .query(
                r#"
                SELECT substr(created_at, 1, 10) AS day, COUNT(*) AS jobs
                FROM agent_jobs
                WHERE created_at >= ?1
                GROUP BY day
                ORDER BY day ASC
                "#,
                params![since_text],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut daily_activity = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            daily_activity.push(DailyActivity {
                date: get_text(&row, 0),
                jobs: get_i64(&row, 1).max(0) as u64,
            });
        }

        Ok(InsightsAggregate {
            total_jobs,
            total_routine_runs,
            total_tokens_used,
            top_tools,
            daily_activity,
        })
    }
}
