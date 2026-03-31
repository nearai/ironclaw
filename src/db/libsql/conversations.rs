//! Conversation-related ConversationStore implementation for LibSqlBackend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use libsql::params;
use uuid::Uuid;

use super::{LibSqlBackend, fmt_ts, get_i64, get_json, get_opt_text, get_text, get_ts, opt_text};
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
        let now = fmt_ts(&Utc::now());
        conn.execute(
            "INSERT INTO conversations (id, channel, user_id, thread_id, started_at, last_activity) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id.to_string(), channel, user_id, opt_text(thread_id), now],
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
        let now = fmt_ts(&Utc::now());
        conn.execute(
                "INSERT INTO conversation_messages (id, conversation_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id.to_string(), conversation_id.to_string(), role, content, now],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        self.touch_conversation(conversation_id).await?;
        Ok(id)
    }

    async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
        source_channel: Option<&str>,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        let affected = conn
            .execute(
            r#"
                INSERT INTO conversations (id, channel, user_id, thread_id, source_channel, started_at, last_activity)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
                ON CONFLICT (id) DO UPDATE SET last_activity = excluded.last_activity
                WHERE conversations.user_id = excluded.user_id
                  AND conversations.channel = excluded.channel
                "#,
            params![id.to_string(), channel, user_id, opt_text(thread_id), opt_text(source_channel), now],
        )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(affected > 0)
    }

    async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        channel: &str,
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
                    c.channel,
                    (SELECT COUNT(*) FROM conversation_messages m WHERE m.conversation_id = c.id AND m.role = 'user') AS message_count,
                    (SELECT substr(m2.content, 1, 100)
                     FROM conversation_messages m2
                     WHERE m2.conversation_id = c.id AND m2.role = 'user'
                     ORDER BY m2.created_at ASC, m2.rowid ASC
                     LIMIT 1
                    ) AS title
                FROM conversations c
                WHERE c.user_id = ?1 AND c.channel = ?2
                ORDER BY datetime(c.last_activity) DESC
                LIMIT ?3
                "#,
                params![user_id, channel, limit],
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
            let sql_title = get_opt_text(&row, 6);
            let title = sql_title.or_else(|| {
                metadata
                    .get("routine_name")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            });
            results.push(ConversationSummary {
                id: row
                    .get::<String>(0)
                    .unwrap_or_default()
                    .parse()
                    .unwrap_or_default(),
                started_at: get_ts(&row, 1),
                last_activity: get_ts(&row, 2),
                message_count: get_i64(&row, 5),
                title,
                thread_type,
                channel: get_text(&row, 4),
            });
        }
        Ok(results)
    }

    async fn list_conversations_all_channels(
        &self,
        user_id: &str,
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
                    c.channel,
                    (SELECT COUNT(*) FROM conversation_messages m WHERE m.conversation_id = c.id AND m.role = 'user') AS message_count,
                    (SELECT substr(m2.content, 1, 100)
                     FROM conversation_messages m2
                     WHERE m2.conversation_id = c.id AND m2.role = 'user'
                     ORDER BY m2.created_at ASC, m2.rowid ASC
                     LIMIT 1
                    ) AS title
                FROM conversations c
                WHERE c.user_id = ?1
                ORDER BY datetime(c.last_activity) DESC
                LIMIT ?2
                "#,
                params![user_id, limit],
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
            let sql_title = get_opt_text(&row, 6);
            let title = sql_title.or_else(|| {
                metadata
                    .get("routine_name")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            });
            results.push(ConversationSummary {
                id: row
                    .get::<String>(0)
                    .unwrap_or_default()
                    .parse()
                    .unwrap_or_default(),
                started_at: get_ts(&row, 1),
                last_activity: get_ts(&row, 2),
                message_count: get_i64(&row, 5),
                title,
                thread_type,
                channel: get_text(&row, 4),
            });
        }
        Ok(results)
    }

    /// Uses BEGIN IMMEDIATE to serialize concurrent writers and prevent
    /// duplicate routine conversations (TOCTOU race).
    async fn get_or_create_routine_conversation(
        &self,
        routine_id: Uuid,
        routine_name: &str,
        user_id: &str,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let rid = routine_id.to_string();

        conn.execute("BEGIN IMMEDIATE", params![])
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let result: Result<Uuid, DatabaseError> = async {
            let mut rows = conn
                .query(
                    r#"
                    SELECT id FROM conversations
                    WHERE user_id = ?1 AND json_extract(metadata, '$.routine_id') = ?2
                    LIMIT 1
                    "#,
                    params![user_id, rid],
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

            let id = Uuid::new_v4();
            let now = fmt_ts(&Utc::now());
            let metadata = serde_json::json!({
                "thread_type": "routine",
                "routine_id": routine_id.to_string(),
                "routine_name": routine_name,
            });
            conn.execute(
                "INSERT INTO conversations (id, channel, user_id, metadata, started_at, last_activity) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![id.to_string(), "routine", user_id, metadata.to_string(), now],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
            Ok(id)
        }
        .await;

        match &result {
            Ok(_) => {
                conn.execute("COMMIT", params![])
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
            }
            Err(_) => {
                let _ = conn.execute("ROLLBACK", params![]).await;
            }
        }
        result
    }

    async fn find_routine_conversation(
        &self,
        routine_id: Uuid,
        user_id: &str,
    ) -> Result<Option<Uuid>, DatabaseError> {
        let conn = self.connect().await?;
        let rid = routine_id.to_string();
        let mut rows = conn
            .query(
                r#"
                SELECT id FROM conversations
                WHERE user_id = ?1 AND json_extract(metadata, '$.routine_id') = ?2
                LIMIT 1
                "#,
                params![user_id, rid],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let id_str: String = row.get(0).map_err(|e| {
                DatabaseError::Query(format!("Failed to read conversation id: {e}"))
            })?;
            let id = id_str
                .parse()
                .map_err(|_| DatabaseError::Serialization("Invalid UUID".to_string()))?;
            return Ok(Some(id));
        }
        Ok(None)
    }

    /// Uses BEGIN IMMEDIATE to serialize concurrent writers and prevent
    /// duplicate heartbeat conversations (TOCTOU race).
    async fn get_or_create_heartbeat_conversation(
        &self,
        user_id: &str,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;

        conn.execute("BEGIN IMMEDIATE", params![])
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let result: Result<Uuid, DatabaseError> = async {
            let mut rows = conn
                .query(
                    r#"
                    SELECT id FROM conversations
                    WHERE user_id = ?1 AND json_extract(metadata, '$.thread_type') = 'heartbeat'
                    LIMIT 1
                    "#,
                    params![user_id],
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

            let id = Uuid::new_v4();
            let now = fmt_ts(&Utc::now());
            let metadata = serde_json::json!({ "thread_type": "heartbeat" });
            conn.execute(
                "INSERT INTO conversations (id, channel, user_id, metadata, started_at, last_activity) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![id.to_string(), "heartbeat", user_id, metadata.to_string(), now],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
            Ok(id)
        }
        .await;

        match &result {
            Ok(_) => {
                conn.execute("COMMIT", params![])
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
            }
            Err(_) => {
                let _ = conn.execute("ROLLBACK", params![]).await;
            }
        }
        result
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
        let now = fmt_ts(&Utc::now());
        let metadata = serde_json::json!({"thread_type": "assistant", "title": "Assistant"});
        conn.execute(
            "INSERT INTO conversations (id, channel, user_id, metadata, started_at, last_activity) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id.to_string(), channel, user_id, metadata.to_string(), now],
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
        let now = fmt_ts(&Utc::now());
        conn.execute(
            "INSERT INTO conversations (id, channel, user_id, metadata, started_at, last_activity) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id.to_string(), channel, user_id, metadata.to_string(), now],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
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
                    ORDER BY created_at DESC, rowid DESC
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
                    ORDER BY created_at DESC, rowid DESC
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
                ORDER BY created_at ASC, rowid ASC
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

    async fn get_conversation_source_channel(
        &self,
        conversation_id: Uuid,
    ) -> Result<Option<String>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT source_channel FROM conversations WHERE id = ?1",
                params![conversation_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(get_opt_text(&row, 0)),
            None => Ok(None),
        }
    }

    async fn delete_conversation(
        &self,
        conversation_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let cid = conversation_id.to_string();

        conn.execute("BEGIN IMMEDIATE", params![])
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let result: Result<bool, DatabaseError> = async {
            // Verify ownership first
            let mut rows = conn
                .query(
                    "SELECT 1 FROM conversations WHERE id = ?1 AND user_id = ?2",
                    params![cid.clone(), user_id],
                )
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
            let found = rows
                .next()
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
            if found.is_none() {
                return Ok(false);
            }

            // Delete messages first (foreign key child)
            conn.execute(
                "DELETE FROM conversation_messages WHERE conversation_id = ?1",
                params![cid.clone()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

            // Delete the conversation itself
            let affected = conn
                .execute(
                    "DELETE FROM conversations WHERE id = ?1 AND user_id = ?2",
                    params![cid, user_id],
                )
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;

            Ok(affected > 0)
        }
        .await;

        match &result {
            Ok(true) => {
                conn.execute("COMMIT", params![])
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
            }
            _ => {
                // Rollback on error or when no rows were affected (Ok(false))
                let _ = conn.execute("ROLLBACK", params![]).await;
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[tokio::test]
    async fn test_get_or_create_routine_conversation_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_routine_conv.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        let routine_id = Uuid::new_v4();
        let user_id = "test_user";

        // First call — creates the conversation
        let id1 = backend
            .get_or_create_routine_conversation(routine_id, "my-routine", user_id)
            .await
            .unwrap();

        // Second call — should return the SAME conversation
        let id2 = backend
            .get_or_create_routine_conversation(routine_id, "my-routine", user_id)
            .await
            .unwrap();

        assert_eq!(id1, id2, "Expected same conversation ID on repeated calls");

        // Third call — still the same
        let id3 = backend
            .get_or_create_routine_conversation(routine_id, "my-routine", user_id)
            .await
            .unwrap();

        assert_eq!(id1, id3);

        // Different routine_id should get a different conversation
        let other_routine_id = Uuid::new_v4();
        let id4 = backend
            .get_or_create_routine_conversation(other_routine_id, "other-routine", user_id)
            .await
            .unwrap();

        assert_ne!(
            id1, id4,
            "Different routines should get different conversations"
        );
    }

    #[tokio::test]
    async fn test_routine_conversation_persists_across_messages() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_routine_persist.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        let routine_id = Uuid::new_v4();
        let user_id = "test_user";

        // First invocation: create conversation and add a message
        let id1 = backend
            .get_or_create_routine_conversation(routine_id, "my-routine", user_id)
            .await
            .unwrap();

        backend
            .add_conversation_message(id1, "assistant", "[cron] Completed: all good")
            .await
            .unwrap();

        // Second invocation: should find existing conversation
        let id2 = backend
            .get_or_create_routine_conversation(routine_id, "my-routine", user_id)
            .await
            .unwrap();

        assert_eq!(id1, id2, "Second invocation should reuse same conversation");

        backend
            .add_conversation_message(id2, "assistant", "[cron] Completed: still good")
            .await
            .unwrap();

        // Verify only one routine conversation exists (not two)
        let convs = backend
            .list_conversations_all_channels(user_id, 50)
            .await
            .unwrap();

        let routine_convs: Vec<_> = convs.iter().filter(|c| c.channel == "routine").collect();
        assert_eq!(
            routine_convs.len(),
            1,
            "Should have exactly 1 routine conversation, found {}",
            routine_convs.len()
        );
    }

    #[tokio::test]
    async fn test_get_or_create_heartbeat_conversation_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_heartbeat_conv.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        let user_id = "test_user";

        let id1 = backend
            .get_or_create_heartbeat_conversation(user_id)
            .await
            .unwrap();

        let id2 = backend
            .get_or_create_heartbeat_conversation(user_id)
            .await
            .unwrap();

        assert_eq!(
            id1, id2,
            "Expected same heartbeat conversation on repeated calls"
        );
    }

    #[tokio::test]
    async fn test_source_channel_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_source_channel.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        let conv_id = Uuid::new_v4();
        let user_id = "user-src-chan";

        // Create conversation with a source_channel
        let created = backend
            .ensure_conversation(conv_id, "telegram", user_id, None, Some("telegram"))
            .await
            .unwrap();
        assert!(created, "first ensure should create");

        // Read it back
        let source = backend
            .get_conversation_source_channel(conv_id)
            .await
            .unwrap();
        assert_eq!(
            source.as_deref(),
            Some("telegram"),
            "source_channel should round-trip through DB"
        );
    }

    #[tokio::test]
    async fn test_source_channel_none_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_source_channel_none.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        let conv_id = Uuid::new_v4();
        let user_id = "user-no-src";

        // Create conversation without source_channel
        backend
            .ensure_conversation(conv_id, "http", user_id, None, None)
            .await
            .unwrap();

        let source = backend
            .get_conversation_source_channel(conv_id)
            .await
            .unwrap();
        assert!(
            source.is_none(),
            "None source_channel should persist as NULL"
        );
    }

    #[tokio::test]
    async fn test_source_channel_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_source_channel_404.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        let source = backend
            .get_conversation_source_channel(Uuid::new_v4())
            .await
            .unwrap();
        assert!(
            source.is_none(),
            "non-existent conversation should return None"
        );
    }

    #[tokio::test]
    async fn test_source_channel_not_overwritten_on_upsert() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_source_channel_upsert.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        let conv_id = Uuid::new_v4();
        let user_id = "user-upsert";

        // First insert with source_channel = "telegram"
        backend
            .ensure_conversation(conv_id, "telegram", user_id, None, Some("telegram"))
            .await
            .unwrap();

        // Upsert same conversation (same user/channel) — source_channel should
        // NOT be overwritten because the ON CONFLICT clause only updates
        // last_activity.
        backend
            .ensure_conversation(conv_id, "telegram", user_id, None, Some("different"))
            .await
            .unwrap();

        let source = backend
            .get_conversation_source_channel(conv_id)
            .await
            .unwrap();
        assert_eq!(
            source.as_deref(),
            Some("telegram"),
            "upsert should not overwrite original source_channel"
        );
    }
}
