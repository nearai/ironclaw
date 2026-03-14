//! UserProfileStore implementation for libSQL/Turso.

use async_trait::async_trait;
use uuid::Uuid;

use crate::db::{ProfileFactRow, UserProfileStore};
use crate::error::DatabaseError;

use super::{LibSqlBackend, fmt_ts, get_text, get_ts};

#[async_trait]
impl UserProfileStore for LibSqlBackend {
    async fn upsert_profile_fact(
        &self,
        user_id: &str,
        agent_id: &str,
        category: &str,
        fact_key: &str,
        fact_value_encrypted: &[u8],
        key_salt: &[u8],
        confidence: f32,
        source: &str,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        let now = fmt_ts(&chrono::Utc::now());

        conn.execute(
            r#"
            INSERT INTO user_profile_facts
                (id, user_id, agent_id, category, fact_key, fact_value_encrypted,
                 key_salt, confidence, source, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
            ON CONFLICT (user_id, agent_id, category, fact_key) DO UPDATE SET
                fact_value_encrypted = excluded.fact_value_encrypted,
                key_salt = excluded.key_salt,
                confidence = excluded.confidence,
                source = excluded.source,
                updated_at = ?10
            "#,
            libsql::params![
                id.to_string(),
                user_id,
                agent_id,
                category,
                fact_key,
                fact_value_encrypted.to_vec(),
                key_salt.to_vec(),
                confidence as f64,
                source,
                now
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("upsert_profile_fact: {e}")))?;

        // SELECT the real id back (ON CONFLICT UPDATE keeps the original id).
        let mut rows = conn
            .query(
                "SELECT id FROM user_profile_facts WHERE user_id = ?1 AND agent_id = ?2 AND category = ?3 AND fact_key = ?4",
                libsql::params![user_id, agent_id, category, fact_key],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("upsert_profile_fact select-back: {e}")))?;

        let row = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(format!("upsert_profile_fact select-back row: {e}")))?
            .ok_or_else(|| {
                DatabaseError::Query(
                    "upsert_profile_fact: record not found after upsert".to_string(),
                )
            })?;

        let real_id_str = get_text(&row, 0);
        Uuid::parse_str(&real_id_str)
            .map_err(|e| DatabaseError::Query(format!("Invalid UUID after upsert: {e}")))
    }

    async fn get_profile_facts(
        &self,
        user_id: &str,
        agent_id: &str,
    ) -> Result<Vec<ProfileFactRow>, DatabaseError> {
        let conn = self.connect().await?;

        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, agent_id, category, fact_key, fact_value_encrypted,
                       key_salt, confidence, source, created_at, updated_at
                FROM user_profile_facts
                WHERE user_id = ?1 AND agent_id = ?2
                ORDER BY category, fact_key
                "#,
                libsql::params![user_id, agent_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("get_profile_facts: {e}")))?;

        let mut results = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(format!("get_profile_facts row: {e}")))?
        {
            let id_str = get_text(&row, 0);
            results.push(ProfileFactRow {
                id: Uuid::parse_str(&id_str)
                    .map_err(|e| DatabaseError::Query(format!("Invalid UUID: {e}")))?,
                user_id: get_text(&row, 1),
                agent_id: get_text(&row, 2),
                category: get_text(&row, 3),
                fact_key: get_text(&row, 4),
                fact_value_encrypted: row
                    .get::<Vec<u8>>(5)
                    .map_err(|e| DatabaseError::Query(format!("BLOB read error: {e}")))?,
                key_salt: row
                    .get::<Vec<u8>>(6)
                    .map_err(|e| DatabaseError::Query(format!("BLOB read error: {e}")))?,
                confidence: row
                    .get::<f64>(7)
                    .map_err(|e| DatabaseError::Query(format!("confidence read error: {e}")))?
                    as f32,
                source: get_text(&row, 8),
                created_at: get_ts(&row, 9),
                updated_at: get_ts(&row, 10),
            });
        }

        Ok(results)
    }

    async fn get_profile_facts_by_category(
        &self,
        user_id: &str,
        agent_id: &str,
        category: &str,
    ) -> Result<Vec<ProfileFactRow>, DatabaseError> {
        let conn = self.connect().await?;

        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, agent_id, category, fact_key, fact_value_encrypted,
                       key_salt, confidence, source, created_at, updated_at
                FROM user_profile_facts
                WHERE user_id = ?1 AND agent_id = ?2 AND category = ?3
                ORDER BY fact_key
                "#,
                libsql::params![user_id, agent_id, category],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("get_profile_facts_by_category: {e}")))?;

        let mut results = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(format!("get_profile_facts_by_category row: {e}")))?
        {
            let id_str = get_text(&row, 0);
            results.push(ProfileFactRow {
                id: Uuid::parse_str(&id_str)
                    .map_err(|e| DatabaseError::Query(format!("Invalid UUID: {e}")))?,
                user_id: get_text(&row, 1),
                agent_id: get_text(&row, 2),
                category: get_text(&row, 3),
                fact_key: get_text(&row, 4),
                fact_value_encrypted: row
                    .get::<Vec<u8>>(5)
                    .map_err(|e| DatabaseError::Query(format!("BLOB read error: {e}")))?,
                key_salt: row
                    .get::<Vec<u8>>(6)
                    .map_err(|e| DatabaseError::Query(format!("BLOB read error: {e}")))?,
                confidence: row
                    .get::<f64>(7)
                    .map_err(|e| DatabaseError::Query(format!("confidence read error: {e}")))?
                    as f32,
                source: get_text(&row, 8),
                created_at: get_ts(&row, 9),
                updated_at: get_ts(&row, 10),
            });
        }

        Ok(results)
    }

    async fn delete_profile_fact(
        &self,
        user_id: &str,
        agent_id: &str,
        category: &str,
        fact_key: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;

        let n = conn
            .execute(
                r#"
                DELETE FROM user_profile_facts
                WHERE user_id = ?1 AND agent_id = ?2 AND category = ?3 AND fact_key = ?4
                "#,
                libsql::params![user_id, agent_id, category, fact_key],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("delete_profile_fact: {e}")))?;

        Ok(n > 0)
    }
}
