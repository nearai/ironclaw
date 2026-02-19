//! Conversation-related ConversationStore implementation for LibSqlBackend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use libsql::params;
use uuid::Uuid;

use super::{
    LibSqlBackend, fmt_opt_ts, fmt_ts, get_i64, get_json, get_opt_text, get_text, get_ts, opt_text,
};
use crate::db::ConversationStore;
use crate::error::DatabaseError;
use crate::history::{ConversationMessage, ConversationSummary};

#[async_trait]
impl ConversationStore for LibSqlBackend {
    async fn create_conversation(
        &self,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO conversations (id, channel, user_id, thread_id) VALUES (?1, ?2, ?3, ?4)",
            params![id.to_string(), channel, user_id, opt_text(thread_id)],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
    }

    async fn touch_conversation(&self, id: Uuid) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            "UPDATE conversations SET last_activity = ?2 WHERE id = ?1",
            params![id.to_string(), now],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn add_conversation_message(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        conn.execute(
                "INSERT INTO conversation_messages (id, conversation_id, role, content) VALUES (?1, ?2, ?3, ?4)",
                params![id.to_string(), conversation_id.to_string(), role, content],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        self.touch_conversation(conversation_id).await?;
        Ok(id)
    }

    async fn add_conversation_message_at(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
        created_at: DateTime<Utc>,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO conversation_messages (id, conversation_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                id.to_string(),
                conversation_id.to_string(),
                role,
                content,
                fmt_ts(&created_at)
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
    }

    async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            r#"
                INSERT INTO conversations (id, channel, user_id, thread_id)
                VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT (id) DO UPDATE SET last_activity = ?5
                "#,
            params![id.to_string(), channel, user_id, opt_text(thread_id), now],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        channel: &str,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT
                    c.id,
                    c.started_at,
                    c.last_activity,
                    c.metadata,
                    (SELECT COUNT(*) FROM conversation_messages m WHERE m.conversation_id = c.id) AS message_count,
                    (SELECT substr(m2.content, 1, 100)
                     FROM conversation_messages m2
                     WHERE m2.conversation_id = c.id AND m2.role = 'user'
                     ORDER BY m2.created_at ASC
                     LIMIT 1
                    ) AS title
                FROM conversations c
                WHERE c.user_id = ?1 AND c.channel = ?2
                  AND (
                    json_extract(c.metadata, '$.thread_type') IS NULL
                    OR json_extract(c.metadata, '$.thread_type') != 'assistant'
                  )
                ORDER BY c.last_activity DESC
                LIMIT ?3 OFFSET ?4
                "#,
                params![user_id, channel, limit, offset],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut results = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let metadata = get_json(&row, 3);
            let thread_type = metadata
                .get("thread_type")
                .and_then(|v| v.as_str())
                .map(String::from);
            results.push(ConversationSummary {
                id: row
                    .get::<String>(0)
                    .unwrap_or_default()
                    .parse()
                    .unwrap_or_default(),
                started_at: get_ts(&row, 1),
                last_activity: get_ts(&row, 2),
                message_count: get_i64(&row, 4),
                title: get_opt_text(&row, 5),
                thread_type,
            });
        }
        Ok(results)
    }

    async fn get_or_create_assistant_conversation(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        // Try to find existing
        let mut rows = conn
            .query(
                r#"
                SELECT id FROM conversations
                WHERE user_id = ?1 AND channel = ?2
                  AND json_extract(metadata, '$.thread_type') = 'assistant'
                LIMIT 1
                "#,
                params![user_id, channel],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let id_str: String = row.get(0).unwrap_or_default();
            return id_str
                .parse()
                .map_err(|_| DatabaseError::Serialization("Invalid UUID".to_string()));
        }

        // Create new
        let id = Uuid::new_v4();
        let metadata = serde_json::json!({"thread_type": "assistant", "title": "Assistant"});
        conn.execute(
            "INSERT INTO conversations (id, channel, user_id, metadata) VALUES (?1, ?2, ?3, ?4)",
            params![id.to_string(), channel, user_id, metadata.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
    }

    async fn create_conversation_with_metadata(
        &self,
        channel: &str,
        user_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO conversations (id, channel, user_id, metadata) VALUES (?1, ?2, ?3, ?4)",
            params![id.to_string(), channel, user_id, metadata.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
    }

    async fn set_conversation_time_bounds(
        &self,
        id: Uuid,
        started_at: Option<DateTime<Utc>>,
        last_activity: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            UPDATE conversations
            SET started_at = COALESCE(?2, started_at),
                last_activity = COALESCE(?3, last_activity)
            WHERE id = ?1
            "#,
            params![
                id.to_string(),
                fmt_opt_ts(&started_at),
                fmt_opt_ts(&last_activity)
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn find_conversation_by_import_source(
        &self,
        user_id: &str,
        channel: &str,
        source: &str,
        source_id: &str,
    ) -> Result<Option<Uuid>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id
                FROM conversations
                WHERE user_id = ?1
                  AND channel = ?2
                  AND json_extract(metadata, '$.import.source') = ?3
                  AND json_extract(metadata, '$.import.source_id') = ?4
                LIMIT 1
                "#,
                params![user_id, channel, source, source_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let id = get_text(&row, 0);
            let parsed = id.parse().map_err(|e| {
                DatabaseError::Serialization(format!("invalid conversation id '{}': {}", id, e))
            })?;
            return Ok(Some(parsed));
        }

        Ok(None)
    }

    async fn delete_conversation(&self, id: Uuid) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "DELETE FROM conversations WHERE id = ?1",
            params![id.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_conversation_messages_paginated(
        &self,
        conversation_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<(Vec<ConversationMessage>, bool), DatabaseError> {
        let conn = self.connect().await?;
        let fetch_limit = limit + 1;
        let cid = conversation_id.to_string();

        let mut rows = if let Some(before_ts) = before {
            conn.query(
                r#"
                    SELECT id, role, content, created_at
                    FROM conversation_messages
                    WHERE conversation_id = ?1 AND created_at < ?2
                    ORDER BY created_at DESC
                    LIMIT ?3
                    "#,
                params![cid, fmt_ts(&before_ts), fetch_limit],
            )
            .await
        } else {
            conn.query(
                r#"
                    SELECT id, role, content, created_at
                    FROM conversation_messages
                    WHERE conversation_id = ?1
                    ORDER BY created_at DESC
                    LIMIT ?2
                    "#,
                params![cid, fetch_limit],
            )
            .await
        }
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut all = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            all.push(ConversationMessage {
                id: get_text(&row, 0).parse().unwrap_or_default(),
                role: get_text(&row, 1),
                content: get_text(&row, 2),
                created_at: get_ts(&row, 3),
            });
        }

        let has_more = all.len() as i64 > limit;
        all.truncate(limit as usize);
        all.reverse(); // oldest first
        Ok((all, has_more))
    }

    async fn update_conversation_metadata_field(
        &self,
        id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        // SQLite: use json_patch to merge the key
        let patch = serde_json::json!({ key: value });
        conn.execute(
            "UPDATE conversations SET metadata = json_patch(metadata, ?2) WHERE id = ?1",
            params![id.to_string(), patch.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_conversation_metadata(
        &self,
        id: Uuid,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT metadata FROM conversations WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(get_json(&row, 0))),
            None => Ok(None),
        }
    }

    async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, role, content, created_at
                FROM conversation_messages
                WHERE conversation_id = ?1
                ORDER BY created_at ASC
                "#,
                params![conversation_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut messages = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            messages.push(ConversationMessage {
                id: get_text(&row, 0).parse().unwrap_or_default(),
                role: get_text(&row, 1),
                content: get_text(&row, 2),
                created_at: get_ts(&row, 3),
            });
        }
        Ok(messages)
    }

    async fn conversation_belongs_to_user(
        &self,
        conversation_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT 1 FROM conversations WHERE id = ?1 AND user_id = ?2",
                libsql::params![conversation_id.to_string(), user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let found = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(found.is_some())
    }
}
