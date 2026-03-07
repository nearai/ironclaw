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
                    (SELECT COUNT(*) FROM conversation_messages m WHERE m.conversation_id = c.id AND m.role = 'user') AS message_count,
                    (SELECT substr(m2.content, 1, 100)
                     FROM conversation_messages m2
                     WHERE m2.conversation_id = c.id AND m2.role = 'user'
                     ORDER BY m2.created_at ASC, m2.rowid ASC
                     LIMIT 1
                    ) AS title
                FROM conversations c
                WHERE c.user_id = ?1 AND c.channel = ?2
                ORDER BY c.last_activity DESC
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

    async fn delete_conversation(&self, id: Uuid) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let tx = conn
            .transaction()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let id_str = id.to_string();

        // Nullify FK references (no cascade on these columns)
        tx.execute(
            "UPDATE agent_jobs SET conversation_id = NULL WHERE conversation_id = ?1",
            params![id_str.clone()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        tx.execute(
            "UPDATE llm_calls SET conversation_id = NULL WHERE conversation_id = ?1",
            params![id_str.clone()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        // Delete conversation (messages cascade via ON DELETE CASCADE)
        tx.execute("DELETE FROM conversations WHERE id = ?1", params![id_str])
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(())
    }

    async fn cherry_pick_messages(
        &self,
        message_ids: &[Uuid],
        channel: &str,
        user_id: &str,
        delete_source: bool,
    ) -> Result<(Uuid, usize), DatabaseError> {
        let conn = self.connect().await?;

        // Build dynamic IN clause (libSQL has no array params)
        let placeholders: Vec<String> = message_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect();
        let in_clause = placeholders.join(", ");
        let query = format!(
            "SELECT id, role, content, created_at FROM conversation_messages \
             WHERE id IN ({in_clause}) \
             ORDER BY created_at ASC, rowid ASC"
        );

        let mut query_params: Vec<libsql::Value> = Vec::new();
        for mid in message_ids {
            query_params.push(libsql::Value::from(mid.to_string()));
        }

        let mut rows = conn
            .query(&query, libsql::params_from_iter(query_params))
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut source_messages = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            source_messages.push((get_text(&row, 1), get_text(&row, 2)));
        }
        // Drop the rows/statement before starting a transaction on the same connection
        drop(rows);

        let tx = conn
            .transaction()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        // Create new conversation
        let new_id = Uuid::new_v4();
        let metadata = serde_json::json!({"thread_type": "thread"});
        tx.execute(
            "INSERT INTO conversations (id, channel, user_id, metadata) VALUES (?1, ?2, ?3, ?4)",
            params![new_id.to_string(), channel, user_id, metadata.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        // Copy messages with fresh UUIDs and monotonically increasing timestamps
        let base_ts = Utc::now();
        let mut copied = 0usize;
        for (i, (role, content)) in source_messages.iter().enumerate() {
            let fresh_id = Uuid::new_v4();
            let ts = base_ts + chrono::TimeDelta::milliseconds(i as i64);

            tx.execute(
                "INSERT INTO conversation_messages (id, conversation_id, role, content, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![fresh_id.to_string(), new_id.to_string(), role.as_str(), content.as_str(), fmt_ts(&ts)],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
            copied += 1;
        }

        // For move: delete originals within the same transaction
        if delete_source {
            let del_placeholders: Vec<String> = message_ids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 1))
                .collect();
            let del_in = del_placeholders.join(", ");
            let del_query =
                format!("DELETE FROM conversation_messages WHERE id IN ({del_in})");
            let del_params: Vec<libsql::Value> = message_ids
                .iter()
                .map(|id| libsql::Value::from(id.to_string()))
                .collect();
            tx.execute(&del_query, libsql::params_from_iter(del_params))
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
        }

        // Touch conversation activity within the transaction
        let now = fmt_ts(&Utc::now());
        tx.execute(
            "UPDATE conversations SET last_activity = ?2 WHERE id = ?1",
            params![new_id.to_string(), now],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok((new_id, copied))
    }

    async fn delete_messages_by_ids(&self, message_ids: &[Uuid]) -> Result<usize, DatabaseError> {
        let conn = self.connect().await?;

        let placeholders: Vec<String> = message_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect();
        let in_clause = placeholders.join(", ");
        let query = format!("DELETE FROM conversation_messages WHERE id IN ({in_clause})");

        let params: Vec<libsql::Value> = message_ids
            .iter()
            .map(|id| libsql::Value::from(id.to_string()))
            .collect();

        let deleted = conn
            .execute(&query, libsql::params_from_iter(params))
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(deleted as usize)
    }
}
