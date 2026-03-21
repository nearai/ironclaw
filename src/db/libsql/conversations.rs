//! Conversation-related ConversationStore implementation for LibSqlBackend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use libsql::{params, params_from_iter};
use uuid::Uuid;

use super::{LibSqlBackend, fmt_ts, get_i64, get_json, get_opt_text, get_text, get_ts, opt_text};
use crate::db::ConversationStore;
use crate::error::DatabaseError;
use crate::history::{ConversationMessage, ConversationNotification, ConversationSummary};

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

    #[allow(clippy::too_many_arguments)]
    async fn add_conversation_notification(
        &self,
        user_id: &str,
        channel: &str,
        conversation_scope_id: Option<&str>,
        source_kind: &str,
        source_id: &str,
        content: &str,
        metadata: &serde_json::Value,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        let now = fmt_ts(&Utc::now());
        conn.execute(
            r#"
            INSERT INTO conversation_notifications (
                id, user_id, channel, conversation_scope_id,
                source_kind, source_id, content, metadata,
                created_at, expires_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                id.to_string(),
                user_id,
                channel,
                opt_text(conversation_scope_id),
                source_kind,
                source_id,
                content,
                metadata.to_string(),
                now,
                expires_at.as_ref().map(fmt_ts)
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
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        let affected = conn
            .execute(
            r#"
                INSERT INTO conversations (id, channel, user_id, thread_id, started_at, last_activity)
                VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                ON CONFLICT (id) DO UPDATE SET last_activity = excluded.last_activity
                WHERE conversations.user_id = excluded.user_id
                  AND conversations.channel = excluded.channel
                "#,
            params![id.to_string(), channel, user_id, opt_text(thread_id), now],
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

    async fn list_unread_conversation_notifications(
        &self,
        user_id: &str,
        channel: &str,
        conversation_scope_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<ConversationNotification>, DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        let mut rows = if let Some(scope) = conversation_scope_id {
            conn.query(
                r#"
                SELECT id, user_id, channel, conversation_scope_id,
                       source_kind, source_id, content, metadata,
                       created_at, consumed_at, expires_at
                FROM conversation_notifications
                WHERE user_id = ?1
                  AND channel = ?2
                  AND consumed_at IS NULL
                  AND (expires_at IS NULL OR expires_at > ?3)
                  AND (conversation_scope_id IS NULL OR conversation_scope_id = ?4)
                ORDER BY datetime(created_at) ASC, rowid ASC
                LIMIT ?5
                "#,
                params![user_id, channel, now, scope, limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        } else {
            conn.query(
                r#"
                SELECT id, user_id, channel, conversation_scope_id,
                       source_kind, source_id, content, metadata,
                       created_at, consumed_at, expires_at
                FROM conversation_notifications
                WHERE user_id = ?1
                  AND channel = ?2
                  AND conversation_scope_id IS NULL
                  AND consumed_at IS NULL
                  AND (expires_at IS NULL OR expires_at > ?3)
                ORDER BY datetime(created_at) ASC, rowid ASC
                LIMIT ?4
                "#,
                params![user_id, channel, now, limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        };

        let mut notifications = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            notifications.push(ConversationNotification {
                id: get_text(&row, 0).parse().unwrap_or_default(),
                user_id: get_text(&row, 1),
                channel: get_text(&row, 2),
                conversation_scope_id: get_opt_text(&row, 3),
                source_kind: get_text(&row, 4),
                source_id: get_text(&row, 5),
                content: get_text(&row, 6),
                metadata: get_json(&row, 7),
                created_at: get_ts(&row, 8),
                consumed_at: get_opt_text(&row, 9).and_then(|ts| {
                    chrono::DateTime::parse_from_rfc3339(&ts)
                        .ok()
                        .map(|dt| dt.with_timezone(&Utc))
                }),
                expires_at: get_opt_text(&row, 10).and_then(|ts| {
                    chrono::DateTime::parse_from_rfc3339(&ts)
                        .ok()
                        .map(|dt| dt.with_timezone(&Utc))
                }),
            });
        }
        Ok(notifications)
    }

    async fn mark_conversation_notifications_consumed(
        &self,
        notification_ids: &[Uuid],
    ) -> Result<(), DatabaseError> {
        if notification_ids.is_empty() {
            return Ok(());
        }

        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        let placeholders: Vec<String> = (1..=notification_ids.len())
            .map(|idx| format!("?{}", idx))
            .collect();
        let sql = format!(
            "UPDATE conversation_notifications
             SET consumed_at = ?{}
             WHERE id IN ({}) AND consumed_at IS NULL",
            notification_ids.len() + 1,
            placeholders.join(", ")
        );
        let mut values: Vec<libsql::Value> = notification_ids
            .iter()
            .map(|id| libsql::Value::Text(id.to_string()))
            .collect();
        values.push(libsql::Value::Text(now));
        let params = params_from_iter(values);
        conn.execute(sql.as_str(), params)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use std::error::Error;

    #[tokio::test]
    async fn test_get_or_create_routine_conversation_is_idempotent() -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let db_path = dir.path().join("test_routine_conv.db");
        let backend = LibSqlBackend::new_local(&db_path).await?;
        backend.run_migrations().await?;

        let routine_id = Uuid::new_v4();
        let user_id = "test_user";

        // First call — creates the conversation
        let id1 = backend
            .get_or_create_routine_conversation(routine_id, "my-routine", user_id)
            .await?;

        // Second call — should return the SAME conversation
        let id2 = backend
            .get_or_create_routine_conversation(routine_id, "my-routine", user_id)
            .await?;

        assert_eq!(id1, id2, "Expected same conversation ID on repeated calls");

        // Third call — still the same
        let id3 = backend
            .get_or_create_routine_conversation(routine_id, "my-routine", user_id)
            .await?;

        assert_eq!(id1, id3);

        // Different routine_id should get a different conversation
        let other_routine_id = Uuid::new_v4();
        let id4 = backend
            .get_or_create_routine_conversation(other_routine_id, "other-routine", user_id)
            .await?;

        assert_ne!(
            id1, id4,
            "Different routines should get different conversations"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_routine_conversation_persists_across_messages() -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let db_path = dir.path().join("test_routine_persist.db");
        let backend = LibSqlBackend::new_local(&db_path).await?;
        backend.run_migrations().await?;

        let routine_id = Uuid::new_v4();
        let user_id = "test_user";

        // First invocation: create conversation and add a message
        let id1 = backend
            .get_or_create_routine_conversation(routine_id, "my-routine", user_id)
            .await?;

        backend
            .add_conversation_message(id1, "assistant", "[cron] Completed: all good")
            .await?;

        // Second invocation: should find existing conversation
        let id2 = backend
            .get_or_create_routine_conversation(routine_id, "my-routine", user_id)
            .await?;

        assert_eq!(id1, id2, "Second invocation should reuse same conversation");

        backend
            .add_conversation_message(id2, "assistant", "[cron] Completed: still good")
            .await?;

        // Verify only one routine conversation exists (not two)
        let convs = backend.list_conversations_all_channels(user_id, 50).await?;

        let routine_convs: Vec<_> = convs.iter().filter(|c| c.channel == "routine").collect();
        assert_eq!(
            routine_convs.len(),
            1,
            "Should have exactly 1 routine conversation, found {}",
            routine_convs.len()
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_get_or_create_heartbeat_conversation_is_idempotent() -> Result<(), Box<dyn Error>>
    {
        let dir = tempfile::tempdir()?;
        let db_path = dir.path().join("test_heartbeat_conv.db");
        let backend = LibSqlBackend::new_local(&db_path).await?;
        backend.run_migrations().await?;

        let user_id = "test_user";

        let id1 = backend
            .get_or_create_heartbeat_conversation(user_id)
            .await?;

        let id2 = backend
            .get_or_create_heartbeat_conversation(user_id)
            .await?;

        assert_eq!(
            id1, id2,
            "Expected same heartbeat conversation on repeated calls"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_conversation_notifications_scope_and_consumption() -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let db_path = dir.path().join("test_notifications.db");
        let backend = LibSqlBackend::new_local(&db_path).await?;
        backend.run_migrations().await?;

        let user_id = "test_user";
        let channel = "gateway";
        let scoped = "thread-123";

        let scoped_id = backend
            .add_conversation_notification(
                user_id,
                channel,
                Some(scoped),
                "routine",
                "routine-1",
                "Scoped notification",
                &serde_json::json!({"routine_name": "Scoped"}),
                None,
            )
            .await?;
        let global_id = backend
            .add_conversation_notification(
                user_id,
                channel,
                None,
                "routine",
                "routine-2",
                "Global notification",
                &serde_json::json!({"routine_name": "Global"}),
                None,
            )
            .await?;

        let scoped_notifications = backend
            .list_unread_conversation_notifications(user_id, channel, Some(scoped), 10)
            .await?;
        assert_eq!(scoped_notifications.len(), 2);
        assert_eq!(scoped_notifications[0].id, scoped_id);
        assert_eq!(scoped_notifications[1].id, global_id);

        let global_notifications = backend
            .list_unread_conversation_notifications(user_id, channel, None, 10)
            .await?;
        assert_eq!(global_notifications.len(), 1);
        assert_eq!(global_notifications[0].id, global_id);

        backend
            .mark_conversation_notifications_consumed(&[scoped_id])
            .await?;

        let scoped_notifications = backend
            .list_unread_conversation_notifications(user_id, channel, Some(scoped), 10)
            .await?;
        assert_eq!(scoped_notifications.len(), 1);
        assert_eq!(scoped_notifications[0].id, global_id);
        Ok(())
    }
}
