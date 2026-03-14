//! SessionSearchStore implementation for libSQL/Turso.

use async_trait::async_trait;
use uuid::Uuid;

use crate::db::{SessionSearchStore, SessionSummaryRow};
use crate::error::DatabaseError;

use super::{LibSqlBackend, fmt_ts, get_i64, get_text, get_ts};

/// Sanitize a user-supplied query for FTS5 MATCH.
///
/// Wraps the query in double quotes to treat it as a phrase query,
/// preventing FTS5 syntax injection (OR, NOT, NEAR, column filters).
/// Empty queries are rejected early.
fn sanitize_fts_query(query: &str) -> Result<String, DatabaseError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err(DatabaseError::Query(
            "search query must not be empty".to_string(),
        ));
    }
    // Limit query length to prevent DoS via large FTS MATCH operations
    if trimmed.len() > 512 {
        return Err(DatabaseError::Query(
            "search query too long (max 512 chars)".to_string(),
        ));
    }
    // Escape internal double quotes and wrap as phrase query
    let escaped = trimmed.replace('"', "\"\"");
    Ok(format!("\"{escaped}\""))
}

#[async_trait]
impl SessionSearchStore for LibSqlBackend {
    async fn upsert_session_summary(
        &self,
        conversation_id: Uuid,
        user_id: &str,
        agent_id: &str,
        summary: &str,
        topics: &[String],
        tool_names: &[String],
        message_count: i32,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        let now = fmt_ts(&chrono::Utc::now());
        let topics_json = serde_json::to_string(topics)
            .map_err(|e| DatabaseError::Query(format!("Failed to serialize topics: {e}")))?;
        let tool_names_json = serde_json::to_string(tool_names)
            .map_err(|e| DatabaseError::Query(format!("Failed to serialize tool_names: {e}")))?;

        let embedding_blob: Option<Vec<u8>> =
            embedding.map(|e| e.iter().flat_map(|f| f.to_le_bytes()).collect());

        // Wrap INSERT + SELECT-back in a transaction for atomicity
        let tx = conn
            .transaction()
            .await
            .map_err(|e| DatabaseError::Query(format!("upsert_session_summary begin tx: {e}")))?;

        tx.execute(
            r#"
            INSERT INTO session_summaries
                (id, conversation_id, user_id, agent_id, summary, topics, tool_names,
                 message_count, embedding, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
            ON CONFLICT (conversation_id) DO UPDATE SET
                summary = excluded.summary,
                topics = excluded.topics,
                tool_names = excluded.tool_names,
                message_count = excluded.message_count,
                embedding = excluded.embedding,
                updated_at = ?10
            "#,
            libsql::params![
                id.to_string(),
                conversation_id.to_string(),
                user_id,
                agent_id,
                summary,
                topics_json,
                tool_names_json,
                message_count as i64,
                embedding_blob
                    .map(libsql::Value::Blob)
                    .unwrap_or(libsql::Value::Null),
                now
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("upsert_session_summary: {e}")))?;

        let mut rows = tx
            .query(
                "SELECT id FROM session_summaries WHERE conversation_id = ?1",
                libsql::params![conversation_id.to_string()],
            )
            .await
            .map_err(|e| {
                DatabaseError::Query(format!("upsert_session_summary select-back: {e}"))
            })?;

        let row = rows
            .next()
            .await
            .map_err(|e| {
                DatabaseError::Query(format!("upsert_session_summary select-back row: {e}"))
            })?
            .ok_or_else(|| {
                DatabaseError::Query(
                    "upsert_session_summary: record not found after upsert".to_string(),
                )
            })?;

        let real_id_str = get_text(&row, 0);
        let result = Uuid::parse_str(&real_id_str)
            .map_err(|e| DatabaseError::Query(format!("Invalid UUID after upsert: {e}")))?;

        tx.commit()
            .await
            .map_err(|e| DatabaseError::Query(format!("upsert_session_summary commit: {e}")))?;

        Ok(result)
    }

    async fn search_sessions_fts(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SessionSummaryRow>, DatabaseError> {
        let sanitized_query = sanitize_fts_query(query)?;
        let conn = self.connect().await?;

        // FTS5 pattern: FTS table as leading table in FROM, base table in JOIN.
        // rank is negative in FTS5 (more negative = more relevant), ORDER BY rank ASC
        // gives most relevant first. We negate the value to produce a positive score.
        let mut rows = conn
            .query(
                r#"
                SELECT s.id, s.conversation_id, s.user_id, s.agent_id, s.summary, s.topics,
                       s.tool_names, s.message_count, s.created_at,
                       f.rank AS score
                FROM session_summaries_fts f
                JOIN session_summaries s ON s._rowid = f.rowid
                WHERE s.user_id = ?1
                  AND session_summaries_fts MATCH ?2
                ORDER BY f.rank
                LIMIT ?3
                "#,
                libsql::params![user_id, sanitized_query, limit as i64],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("search_sessions_fts: {e}")))?;

        let mut results = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(format!("search_sessions_fts row: {e}")))?
        {
            let id_str = get_text(&row, 0);
            let conv_str = get_text(&row, 1);
            let topics_str = get_text(&row, 5);
            let tool_names_str = get_text(&row, 6);

            let raw_rank = row
                .get::<f64>(9)
                .map_err(|e| DatabaseError::Query(format!("rank read error: {e}")))?;

            results.push(SessionSummaryRow {
                id: Uuid::parse_str(&id_str)
                    .map_err(|e| DatabaseError::Query(format!("Invalid UUID: {e}")))?,
                conversation_id: Uuid::parse_str(&conv_str)
                    .map_err(|e| DatabaseError::Query(format!("Invalid UUID: {e}")))?,
                user_id: get_text(&row, 2),
                agent_id: get_text(&row, 3),
                summary: get_text(&row, 4),
                topics: serde_json::from_str(&topics_str).unwrap_or_default(),
                tool_names: serde_json::from_str(&tool_names_str).unwrap_or_default(),
                message_count: get_i64(&row, 7) as i32,
                created_at: get_ts(&row, 8),
                // FTS5 rank is negative; negate to produce positive score
                score: (-raw_rank) as f32,
            });
        }

        Ok(results)
    }

    async fn search_sessions_vector(
        &self,
        user_id: &str,
        _embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<SessionSummaryRow>, DatabaseError> {
        // libSQL does not have native vector distance operators like pgvector.
        // Fall back to returning most recent summaries for the user.
        let conn = self.connect().await?;

        let mut rows = conn
            .query(
                r#"
                SELECT id, conversation_id, user_id, agent_id, summary, topics,
                       tool_names, message_count, created_at
                FROM session_summaries
                WHERE user_id = ?1 AND embedding IS NOT NULL
                ORDER BY created_at DESC
                LIMIT ?2
                "#,
                libsql::params![user_id, limit as i64],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("search_sessions_vector: {e}")))?;

        let mut results = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(format!("search_sessions_vector row: {e}")))?
        {
            let id_str = get_text(&row, 0);
            let conv_str = get_text(&row, 1);
            let topics_str = get_text(&row, 5);
            let tool_names_str = get_text(&row, 6);

            results.push(SessionSummaryRow {
                id: Uuid::parse_str(&id_str)
                    .map_err(|e| DatabaseError::Query(format!("Invalid UUID: {e}")))?,
                conversation_id: Uuid::parse_str(&conv_str)
                    .map_err(|e| DatabaseError::Query(format!("Invalid UUID: {e}")))?,
                user_id: get_text(&row, 2),
                agent_id: get_text(&row, 3),
                summary: get_text(&row, 4),
                topics: serde_json::from_str(&topics_str).unwrap_or_default(),
                tool_names: serde_json::from_str(&tool_names_str).unwrap_or_default(),
                message_count: get_i64(&row, 7) as i32,
                created_at: get_ts(&row, 8),
                score: 0.0, // no vector score available
            });
        }

        Ok(results)
    }

    async fn get_session_summary(
        &self,
        conversation_id: Uuid,
    ) -> Result<Option<SessionSummaryRow>, DatabaseError> {
        let conn = self.connect().await?;

        let mut rows = conn
            .query(
                r#"
                SELECT id, conversation_id, user_id, agent_id, summary, topics,
                       tool_names, message_count, created_at
                FROM session_summaries
                WHERE conversation_id = ?1
                "#,
                libsql::params![conversation_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("get_session_summary: {e}")))?;

        let row = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(format!("get_session_summary row: {e}")))?;

        match row {
            Some(row) => {
                let id_str = get_text(&row, 0);
                let conv_str = get_text(&row, 1);
                let topics_str = get_text(&row, 5);
                let tool_names_str = get_text(&row, 6);

                Ok(Some(SessionSummaryRow {
                    id: Uuid::parse_str(&id_str)
                        .map_err(|e| DatabaseError::Query(format!("Invalid UUID: {e}")))?,
                    conversation_id: Uuid::parse_str(&conv_str)
                        .map_err(|e| DatabaseError::Query(format!("Invalid UUID: {e}")))?,
                    user_id: get_text(&row, 2),
                    agent_id: get_text(&row, 3),
                    summary: get_text(&row, 4),
                    topics: serde_json::from_str(&topics_str).unwrap_or_default(),
                    tool_names: serde_json::from_str(&tool_names_str).unwrap_or_default(),
                    message_count: get_i64(&row, 7) as i32,
                    created_at: get_ts(&row, 8),
                    score: 1.0,
                }))
            }
            None => Ok(None),
        }
    }
}
