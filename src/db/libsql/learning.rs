//! LearningStore implementation for libSQL/Turso.

use async_trait::async_trait;
use uuid::Uuid;

use crate::db::{LearningStore, SynthesizedSkillRow};
use crate::error::DatabaseError;

use super::{LibSqlBackend, fmt_ts, get_i64, get_opt_text, get_opt_ts, get_text, get_ts};

#[async_trait]
impl LearningStore for LibSqlBackend {
    async fn record_synthesized_skill(
        &self,
        user_id: &str,
        agent_id: &str,
        skill_name: &str,
        skill_content: Option<&str>,
        content_hash: &str,
        source_conversation_id: Option<Uuid>,
        status: &str,
        safety_scan_passed: bool,
        quality_score: i32,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        let now = fmt_ts(&chrono::Utc::now());
        let conv_id_str = source_conversation_id.map(|u| u.to_string());

        conn.execute(
            r#"
            INSERT INTO synthesized_skills
                (id, user_id, agent_id, skill_name, skill_content, skill_content_hash,
                 source_conversation_id, status, safety_scan_passed, quality_score, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            libsql::params![
                id.to_string(),
                user_id,
                agent_id,
                skill_name,
                skill_content.map(|s| s.to_string()),
                content_hash,
                conv_id_str,
                status,
                if safety_scan_passed { 1i64 } else { 0i64 },
                quality_score as i64,
                now
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("record_synthesized_skill: {e}")))?;

        Ok(id)
    }

    async fn update_synthesized_skill_status(
        &self,
        id: Uuid,
        user_id: &str,
        status: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&chrono::Utc::now());

        let n = conn
            .execute(
                r#"
                UPDATE synthesized_skills
                SET status = ?3, reviewed_at = ?4
                WHERE id = ?1 AND user_id = ?2
                "#,
                libsql::params![id.to_string(), user_id, status, now],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("update_synthesized_skill_status: {e}")))?;

        Ok(n > 0)
    }

    async fn list_synthesized_skills(
        &self,
        user_id: &str,
        agent_id: &str,
        status: Option<&str>,
    ) -> Result<Vec<SynthesizedSkillRow>, DatabaseError> {
        let conn = self.connect().await?;

        let mut rows = if let Some(status) = status {
            conn.query(
                r#"
                SELECT id, user_id, agent_id, skill_name, skill_content,
                       skill_content_hash, source_conversation_id, status,
                       safety_scan_passed, quality_score, created_at, reviewed_at
                FROM synthesized_skills
                WHERE user_id = ?1 AND agent_id = ?2 AND status = ?3
                ORDER BY created_at DESC
                "#,
                libsql::params![user_id, agent_id, status],
            )
            .await
        } else {
            conn.query(
                r#"
                SELECT id, user_id, agent_id, skill_name, skill_content,
                       skill_content_hash, source_conversation_id, status,
                       safety_scan_passed, quality_score, created_at, reviewed_at
                FROM synthesized_skills
                WHERE user_id = ?1 AND agent_id = ?2
                ORDER BY created_at DESC
                "#,
                libsql::params![user_id, agent_id],
            )
            .await
        }
        .map_err(|e| DatabaseError::Query(format!("list_synthesized_skills: {e}")))?;

        let mut results = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(format!("list_synthesized_skills row: {e}")))?
        {
            results.push(parse_skill_row(&row)?);
        }

        Ok(results)
    }

    async fn get_synthesized_skill(
        &self,
        id: Uuid,
        user_id: &str,
    ) -> Result<Option<SynthesizedSkillRow>, DatabaseError> {
        let conn = self.connect().await?;

        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, agent_id, skill_name, skill_content,
                       skill_content_hash, source_conversation_id, status,
                       safety_scan_passed, quality_score, created_at, reviewed_at
                FROM synthesized_skills
                WHERE id = ?1 AND user_id = ?2
                "#,
                libsql::params![id.to_string(), user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("get_synthesized_skill: {e}")))?;

        let row = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(format!("get_synthesized_skill row: {e}")))?;

        match row {
            Some(row) => Ok(Some(parse_skill_row(&row)?)),
            None => Ok(None),
        }
    }
}

fn parse_skill_row(row: &libsql::Row) -> Result<SynthesizedSkillRow, DatabaseError> {
    let id_str = get_text(row, 0);
    let conv_str = get_opt_text(row, 6);

    Ok(SynthesizedSkillRow {
        id: Uuid::parse_str(&id_str)
            .map_err(|e| DatabaseError::Query(format!("Invalid UUID: {e}")))?,
        user_id: get_text(row, 1),
        agent_id: get_text(row, 2),
        skill_name: get_text(row, 3),
        skill_content: get_opt_text(row, 4),
        skill_content_hash: get_text(row, 5),
        source_conversation_id: conv_str.and_then(|s| Uuid::parse_str(&s).ok()),
        status: get_text(row, 7),
        safety_scan_passed: get_i64(row, 8) != 0,
        quality_score: get_i64(row, 9) as i32,
        created_at: get_ts(row, 10),
        reviewed_at: get_opt_ts(row, 11),
    })
}
