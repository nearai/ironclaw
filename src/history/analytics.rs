//! Analytics and aggregation for learning.
//!
//! Analytics methods are implemented directly on [`Store`] for convenience.

use rust_decimal::Decimal;

use crate::error::DatabaseError;
use crate::history::Store;

/// Statistics about jobs.
#[derive(Debug, Default)]
pub struct JobStats {
    pub total_jobs: u64,
    pub completed_jobs: u64,
    pub failed_jobs: u64,
    pub success_rate: f64,
    pub avg_duration_secs: f64,
    pub avg_cost: Decimal,
    pub total_cost: Decimal,
}

/// Statistics about tool usage.
#[derive(Debug)]
pub struct ToolStats {
    pub tool_name: String,
    pub total_calls: u64,
    pub successful_calls: u64,
    pub failed_calls: u64,
    pub success_rate: f64,
    pub avg_duration_ms: f64,
    pub total_cost: Decimal,
}

impl Store {
    /// Get job statistics.
    pub async fn get_job_stats(&self) -> Result<JobStats, DatabaseError> {
        let conn = self.conn().await?;

        let row = conn
            .query_one(
                r#"
                SELECT
                    COUNT(*) as total,
                    COUNT(*) FILTER (WHERE status = 'accepted') as completed,
                    COUNT(*) FILTER (WHERE status = 'failed') as failed,
                    AVG(EXTRACT(EPOCH FROM (completed_at - started_at))) FILTER (WHERE completed_at IS NOT NULL) as avg_duration,
                    AVG(actual_cost) as avg_cost,
                    SUM(actual_cost) as total_cost
                FROM agent_jobs
                "#,
                &[],
            )
            .await?;

        let total: i64 = row.get("total");
        let completed: i64 = row.get("completed");
        let failed: i64 = row.get("failed");

        Ok(JobStats {
            total_jobs: total as u64,
            completed_jobs: completed as u64,
            failed_jobs: failed as u64,
            success_rate: if total > 0 {
                completed as f64 / total as f64
            } else {
                0.0
            },
            avg_duration_secs: row.get::<_, Option<f64>>("avg_duration").unwrap_or(0.0),
            avg_cost: row
                .get::<_, Option<Decimal>>("avg_cost")
                .unwrap_or_default(),
            total_cost: row
                .get::<_, Option<Decimal>>("total_cost")
                .unwrap_or_default(),
        })
    }

    /// Get tool usage statistics.
    pub async fn get_tool_stats(&self) -> Result<Vec<ToolStats>, DatabaseError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                r#"
                SELECT
                    tool_name,
                    COUNT(*) as total,
                    COUNT(*) FILTER (WHERE success = true) as successful,
                    COUNT(*) FILTER (WHERE success = false) as failed,
                    AVG(duration_ms) as avg_duration,
                    SUM(cost) as total_cost
                FROM job_actions
                GROUP BY tool_name
                ORDER BY total DESC
                "#,
                &[],
            )
            .await?;

        let mut stats = Vec::new();
        for row in rows {
            let total: i64 = row.get("total");
            let successful: i64 = row.get("successful");
            let failed: i64 = row.get("failed");

            stats.push(ToolStats {
                tool_name: row.get("tool_name"),
                total_calls: total as u64,
                successful_calls: successful as u64,
                failed_calls: failed as u64,
                success_rate: if total > 0 {
                    successful as f64 / total as f64
                } else {
                    0.0
                },
                avg_duration_ms: row.get::<_, Option<f64>>("avg_duration").unwrap_or(0.0),
                total_cost: row
                    .get::<_, Option<Decimal>>("total_cost")
                    .unwrap_or_default(),
            });
        }

        Ok(stats)
    }

    /// Get estimation accuracy for learning.
    pub async fn get_estimation_accuracy(
        &self,
        category: Option<&str>,
    ) -> Result<EstimationAccuracy, DatabaseError> {
        let conn = self.conn().await?;

        let query = if category.is_some() {
            r#"
            SELECT
                AVG(ABS(actual_cost - estimated_cost) / NULLIF(estimated_cost, 0)) as cost_error,
                AVG(ABS(actual_time_secs - estimated_time_secs)::float / NULLIF(estimated_time_secs, 0)) as time_error,
                COUNT(*) as sample_count
            FROM estimation_snapshots
            WHERE actual_cost IS NOT NULL AND category = $1
            "#
        } else {
            r#"
            SELECT
                AVG(ABS(actual_cost - estimated_cost) / NULLIF(estimated_cost, 0)) as cost_error,
                AVG(ABS(actual_time_secs - estimated_time_secs)::float / NULLIF(estimated_time_secs, 0)) as time_error,
                COUNT(*) as sample_count
            FROM estimation_snapshots
            WHERE actual_cost IS NOT NULL
            "#
        };

        let row = if let Some(cat) = category {
            conn.query_one(query, &[&cat]).await?
        } else {
            conn.query_one(query, &[]).await?
        };

        Ok(EstimationAccuracy {
            cost_error_rate: row.get::<_, Option<f64>>("cost_error").unwrap_or(0.0),
            time_error_rate: row.get::<_, Option<f64>>("time_error").unwrap_or(0.0),
            sample_count: row.get::<_, i64>("sample_count") as u64,
        })
    }

    /// Get historical data for a category (for learning).
    pub async fn get_category_history(
        &self,
        category: &str,
        limit: i64,
    ) -> Result<Vec<CategoryHistoryEntry>, DatabaseError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                r#"
                SELECT
                    tool_names,
                    estimated_cost,
                    actual_cost,
                    estimated_time_secs,
                    actual_time_secs,
                    created_at
                FROM estimation_snapshots
                WHERE category = $1 AND actual_cost IS NOT NULL
                ORDER BY created_at DESC
                LIMIT $2
                "#,
                &[&category, &limit],
            )
            .await?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(CategoryHistoryEntry {
                tool_names: row.get("tool_names"),
                estimated_cost: row.get("estimated_cost"),
                actual_cost: row.get("actual_cost"),
                estimated_time_secs: row.get("estimated_time_secs"),
                actual_time_secs: row.get("actual_time_secs"),
                created_at: row.get("created_at"),
            });
        }

        Ok(entries)
    }
}

/// Estimation accuracy metrics.
#[derive(Debug, Default)]
pub struct EstimationAccuracy {
    pub cost_error_rate: f64,
    pub time_error_rate: f64,
    pub sample_count: u64,
}

/// Historical entry for a category.
#[derive(Debug)]
pub struct CategoryHistoryEntry {
    pub tool_names: Vec<String>,
    pub estimated_cost: Decimal,
    pub actual_cost: Option<Decimal>,
    pub estimated_time_secs: i32,
    pub actual_time_secs: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
