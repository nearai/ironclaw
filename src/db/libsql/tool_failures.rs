//! Tool failure-related ToolFailureStore implementation for LibSqlBackend.

use async_trait::async_trait;
use libsql::params;
use uuid::Uuid;

use super::{LibSqlBackend, fmt_ts, get_i64, get_opt_text, get_text, get_ts};
use crate::agent::BrokenTool;
use crate::db::ToolFailureStore;
use crate::error::DatabaseError;

use chrono::Utc;

#[async_trait]
impl ToolFailureStore for LibSqlBackend {
    async fn record_tool_failure(
        &self,
        tool_name: &str,
        error_message: &str,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            r#"
                INSERT INTO tool_failures (id, tool_name, error_message, error_count, last_failure)
                VALUES (?1, ?2, ?3, 1, ?4)
                ON CONFLICT (tool_name) DO UPDATE SET
                    error_message = ?3,
                    error_count = tool_failures.error_count + 1,
                    last_failure = ?4
                "#,
            params![Uuid::new_v4().to_string(), tool_name, error_message, now],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_broken_tools(&self, threshold: i32) -> Result<Vec<BrokenTool>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT tool_name, error_message, error_count, first_failure, last_failure,
                       last_build_result, repair_attempts
                FROM tool_failures
                WHERE error_count >= ?1 AND repaired_at IS NULL
                ORDER BY error_count DESC
                "#,
                params![threshold as i64],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut tools = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            tools.push(BrokenTool {
                name: get_text(&row, 0),
                last_error: get_opt_text(&row, 1),
                failure_count: get_i64(&row, 2) as u32,
                first_failure: get_ts(&row, 3),
                last_failure: get_ts(&row, 4),
                last_build_result: get_opt_text(&row, 5)
                    .and_then(|s| serde_json::from_str(&s).ok()),
                repair_attempts: get_i64(&row, 6) as u32,
            });
        }
        Ok(tools)
    }

    async fn mark_tool_repaired(&self, tool_name: &str) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            "UPDATE tool_failures SET repaired_at = ?2, error_count = 0 WHERE tool_name = ?1",
            params![tool_name, now],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE tool_failures SET repair_attempts = repair_attempts + 1 WHERE tool_name = ?1",
            params![tool_name],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }
}
