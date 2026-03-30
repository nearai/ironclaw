//! JudgeVerdictStore implementation for LibSqlBackend.

use async_trait::async_trait;
use libsql::params;
use uuid::Uuid;

use super::{LibSqlBackend, fmt_ts};
use crate::db::JudgeVerdictStore;
use crate::error::DatabaseError;

use chrono::Utc;

#[async_trait]
impl JudgeVerdictStore for LibSqlBackend {
    async fn record_judge_verdict(
        &self,
        tool_name: &str,
        verdict: &str,
        attack_type: Option<&str>,
        confidence: f64,
        reasoning: &str,
        latency_ms: u64,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            r#"
            INSERT INTO judge_verdicts (id, tool_name, verdict, attack_type, confidence, reasoning, latency_ms, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                Uuid::new_v4().to_string(),
                tool_name,
                verdict,
                attack_type,
                confidence,
                reasoning,
                latency_ms as i64,
                now
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }
}
