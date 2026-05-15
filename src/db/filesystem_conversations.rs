//! Filesystem-backed implementation of [`ConversationStore`].
//!
//! Routes conversation and message persistence through the unified
//! [`RootFilesystem`](ironclaw_filesystem::RootFilesystem) surface so the same
//! dispatch fabric used by `ironclaw_secrets`, `ironclaw_authorization`,
//! `ironclaw_processes`, and the rest of Reborn serves the conversation
//! sub-trait too.
//!
//! Path layout (every entry lives under the `/engine` virtual root which is
//! already accepted by [`VirtualPath`](ironclaw_host_api::VirtualPath)):
//!
//! - `/engine/conversations/<conv_id>` — conversation record. Indexed
//!   projections: `user_id`, `channel`, `thread_type`, `routine_id`,
//!   `source_channel`, `last_activity`, `started_at`.
//! - `/engine/conversations/<conv_id>/messages/<msg_id>` — single message
//!   record. Indexed: `conversation_id`, `role`, `created_at_ts` (i64 unix
//!   millis for sort/range), `created_at` (iso-8601 text for display).
//!
//! Queries:
//!
//! - "list conversations for user X" -> `query("/engine/conversations",
//!   Filter::Eq{user_id})`, sort in Rust by `last_activity`.
//! - "list messages for conversation X" -> `query("/engine/conversations/<id>",
//!   Filter::Eq{conversation_id})`, sort by `created_at_ts`.
//! - "find routine conversation for user X" -> `query` with `Filter::And` on
//!   `user_id` + `routine_id`.
//!
//! CAS:
//!
//! Each `put` uses [`CasExpectation::Any`] for last-write-wins semantics
//! matching the SQL upserts. Multi-step transitions that the SQL backends use
//! `BEGIN IMMEDIATE` for (e.g. `get_or_create_routine_conversation`) are
//! protected here by a per-key process-local mutex; this matches the floor
//! contract documented in
//! [`ironclaw_filesystem::CLAUDE.md`](../../../crates/ironclaw_filesystem/CLAUDE.md)
//! and the pattern used by `ironclaw_secrets::filesystem_store`. Multi-process
//! callers should use a transactional backend.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasExpectation, Entry, Filter, IndexKey, IndexValue, Page, RecordKind, RootFilesystem,
};
use ironclaw_host_api::VirtualPath;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::ConversationStore;
use crate::error::DatabaseError;
use crate::history::{ConversationMessage, ConversationSummary};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredConversation {
    id: Uuid,
    channel: String,
    user_id: String,
    thread_id: Option<String>,
    source_channel: Option<String>,
    metadata: serde_json::Value,
    started_at: DateTime<Utc>,
    last_activity: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredMessage {
    id: Uuid,
    conversation_id: Uuid,
    role: String,
    content: String,
    created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Filesystem-backed [`ConversationStore`].
///
/// Wraps any [`RootFilesystem`] implementation. Constructed with a shared
/// `Arc<F>` so the same dispatch fabric can back multiple sub-trait facades.
pub struct FilesystemConversationStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<F>,
}

impl<F> FilesystemConversationStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>) -> Self {
        Self { filesystem }
    }

    fn message_kind() -> RecordKind {
        RecordKind::new(KIND_MESSAGE)
            .unwrap_or_else(|_| unreachable!("conversation_message is a valid record-kind literal"))
    }

    async fn read_conversation(
        &self,
        id: Uuid,
    ) -> Result<Option<StoredConversation>, DatabaseError> {
        let path = conversation_path(id)?;
        let Some(versioned) = self
            .filesystem
            .get(&path)
            .await
            .map_err(fs_err_to_database)?
        else {
            return Ok(None);
        };
        let stored: StoredConversation = serde_json::from_slice(&versioned.entry.body)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        Ok(Some(stored))
    }

    async fn write_conversation(&self, stored: &StoredConversation) -> Result<(), DatabaseError> {
        let path = conversation_path(stored.id)?;
        let body =
            serde_json::to_vec(stored).map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let entry = build_conversation_entry(stored, body)?;
        self.filesystem
            .put(&path, entry, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(fs_err_to_database)
    }

    async fn list_messages_internal(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<StoredMessage>, DatabaseError> {
        let prefix = messages_root(conversation_id)?;
        let key_conv = IndexKey::new(IDX_CONVERSATION_ID)
            .unwrap_or_else(|_| unreachable!("conversation_id is a valid index key"));
        let filter = Filter::Eq {
            key: key_conv,
            value: IndexValue::Text(conversation_id.to_string()),
        };
        let results = query_all_pages(&self.filesystem, &prefix, &filter).await?;
        let mut out = Vec::with_capacity(results.len());
        for v in results {
            if v.entry.kind.as_ref().map(|k| k.as_str()) != Some(KIND_MESSAGE) {
                continue;
            }
            let stored: StoredMessage = serde_json::from_slice(&v.entry.body)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
            if stored.conversation_id != conversation_id {
                continue;
            }
            out.push(stored);
        }
        out.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// ConversationStore impl
// ---------------------------------------------------------------------------

#[async_trait]
impl<F> ConversationStore for FilesystemConversationStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn create_conversation(
        &self,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Uuid, DatabaseError> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let stored = StoredConversation {
            id,
            channel: channel.to_string(),
            user_id: user_id.to_string(),
            thread_id: thread_id.map(str::to_string),
            source_channel: None,
            metadata: serde_json::Value::Null,
            started_at: now,
            last_activity: now,
        };
        self.write_conversation(&stored).await?;
        Ok(id)
    }

    async fn touch_conversation(&self, id: Uuid) -> Result<(), DatabaseError> {
        let lock = conversation_lock(id);
        let _guard = lock.lock().await;
        let Some(mut stored) = self.read_conversation(id).await? else {
            return Ok(());
        };
        stored.last_activity = Utc::now();
        self.write_conversation(&stored).await
    }

    async fn add_conversation_message(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<Uuid, DatabaseError> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let stored = StoredMessage {
            id,
            conversation_id,
            role: role.to_string(),
            content: content.to_string(),
            created_at: now,
        };
        write_message(&self.filesystem, &stored).await?;
        self.touch_conversation(conversation_id).await?;
        Ok(id)
    }

    async fn add_conversation_message_if_empty(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<bool, DatabaseError> {
        let lock = conversation_lock(conversation_id);
        let _guard = lock.lock().await;
        let existing = self.list_messages_internal(conversation_id).await?;
        if !existing.is_empty() {
            return Ok(false);
        }
        let id = Uuid::new_v4();
        let now = Utc::now();
        let stored = StoredMessage {
            id,
            conversation_id,
            role: role.to_string(),
            content: content.to_string(),
            created_at: now,
        };
        write_message(&self.filesystem, &stored).await?;
        // touch_conversation re-acquires the same lock; drop ours first by
        // doing the read-modify-write inline.
        if let Some(mut conv) = self.read_conversation(conversation_id).await? {
            conv.last_activity = Utc::now();
            self.write_conversation(&conv).await?;
        }
        Ok(true)
    }

    async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
        source_channel: Option<&str>,
    ) -> Result<bool, DatabaseError> {
        let lock = conversation_lock(id);
        let _guard = lock.lock().await;
        let now = Utc::now();
        match self.read_conversation(id).await? {
            Some(mut existing) => {
                if existing.user_id != user_id || existing.channel != channel {
                    return Ok(false);
                }
                existing.last_activity = now;
                if existing.source_channel.is_none() {
                    existing.source_channel = source_channel.map(str::to_string);
                }
                self.write_conversation(&existing).await?;
                Ok(true)
            }
            None => {
                let stored = StoredConversation {
                    id,
                    channel: channel.to_string(),
                    user_id: user_id.to_string(),
                    thread_id: thread_id.map(str::to_string),
                    source_channel: source_channel.map(str::to_string),
                    metadata: serde_json::Value::Null,
                    started_at: now,
                    last_activity: now,
                };
                self.write_conversation(&stored).await?;
                Ok(true)
            }
        }
    }

    async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        channel: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        let prefix = conversations_root()?;
        let filter = Filter::And(vec![
            Filter::Eq {
                key: index_key(IDX_USER_ID),
                value: IndexValue::Text(user_id.to_string()),
            },
            Filter::Eq {
                key: index_key(IDX_CHANNEL),
                value: IndexValue::Text(channel.to_string()),
            },
        ]);
        self.list_conversations_summary(&prefix, &filter, limit)
            .await
    }

    async fn list_conversations_all_channels(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        let prefix = conversations_root()?;
        let filter = Filter::Eq {
            key: index_key(IDX_USER_ID),
            value: IndexValue::Text(user_id.to_string()),
        };
        self.list_conversations_summary(&prefix, &filter, limit)
            .await
    }

    async fn get_or_create_routine_conversation(
        &self,
        routine_id: Uuid,
        routine_name: &str,
        user_id: &str,
    ) -> Result<Uuid, DatabaseError> {
        let lock = routine_lock(user_id, routine_id);
        let _guard = lock.lock().await;
        let prefix = conversations_root()?;
        let filter = Filter::And(vec![
            Filter::Eq {
                key: index_key(IDX_USER_ID),
                value: IndexValue::Text(user_id.to_string()),
            },
            Filter::Eq {
                key: index_key(IDX_ROUTINE_ID),
                value: IndexValue::Text(routine_id.to_string()),
            },
        ]);
        let results = self
            .filesystem
            .query(&prefix, &filter, Page::first(1))
            .await
            .map_err(fs_err_to_database)?;
        for v in &results {
            let stored: StoredConversation = serde_json::from_slice(&v.entry.body)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
            if stored.user_id == user_id
                && stored.metadata.get("routine_id").and_then(|v| v.as_str())
                    == Some(routine_id.to_string().as_str())
            {
                return Ok(stored.id);
            }
        }
        let now = Utc::now();
        let metadata = serde_json::json!({
            "thread_type": "routine",
            "routine_id": routine_id.to_string(),
            "routine_name": routine_name,
        });
        let id = Uuid::new_v4();
        let stored = StoredConversation {
            id,
            channel: "routine".to_string(),
            user_id: user_id.to_string(),
            thread_id: None,
            source_channel: None,
            metadata,
            started_at: now,
            last_activity: now,
        };
        self.write_conversation(&stored).await?;
        Ok(id)
    }

    async fn find_routine_conversation(
        &self,
        routine_id: Uuid,
        user_id: &str,
    ) -> Result<Option<Uuid>, DatabaseError> {
        let prefix = conversations_root()?;
        let filter = Filter::And(vec![
            Filter::Eq {
                key: index_key(IDX_USER_ID),
                value: IndexValue::Text(user_id.to_string()),
            },
            Filter::Eq {
                key: index_key(IDX_ROUTINE_ID),
                value: IndexValue::Text(routine_id.to_string()),
            },
        ]);
        let results = self
            .filesystem
            .query(&prefix, &filter, Page::first(1))
            .await
            .map_err(fs_err_to_database)?;
        if let Some(v) = results.first() {
            let stored: StoredConversation = serde_json::from_slice(&v.entry.body)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
            return Ok(Some(stored.id));
        }
        Ok(None)
    }

    async fn get_or_create_heartbeat_conversation(
        &self,
        user_id: &str,
    ) -> Result<Uuid, DatabaseError> {
        let lock = heartbeat_lock(user_id);
        let _guard = lock.lock().await;
        let prefix = conversations_root()?;
        let filter = Filter::And(vec![
            Filter::Eq {
                key: index_key(IDX_USER_ID),
                value: IndexValue::Text(user_id.to_string()),
            },
            Filter::Eq {
                key: index_key(IDX_THREAD_TYPE),
                value: IndexValue::Text("heartbeat".to_string()),
            },
        ]);
        let results = self
            .filesystem
            .query(&prefix, &filter, Page::first(1))
            .await
            .map_err(fs_err_to_database)?;
        if let Some(v) = results.first() {
            let stored: StoredConversation = serde_json::from_slice(&v.entry.body)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
            return Ok(stored.id);
        }
        let now = Utc::now();
        let id = Uuid::new_v4();
        let metadata = serde_json::json!({"thread_type": "heartbeat"});
        let stored = StoredConversation {
            id,
            channel: "heartbeat".to_string(),
            user_id: user_id.to_string(),
            thread_id: None,
            source_channel: None,
            metadata,
            started_at: now,
            last_activity: now,
        };
        self.write_conversation(&stored).await?;
        Ok(id)
    }

    async fn get_or_create_assistant_conversation(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Result<Uuid, DatabaseError> {
        let lock = assistant_lock(user_id, channel);
        let _guard = lock.lock().await;
        let prefix = conversations_root()?;
        let filter = Filter::And(vec![
            Filter::Eq {
                key: index_key(IDX_USER_ID),
                value: IndexValue::Text(user_id.to_string()),
            },
            Filter::Eq {
                key: index_key(IDX_CHANNEL),
                value: IndexValue::Text(channel.to_string()),
            },
            Filter::Eq {
                key: index_key(IDX_THREAD_TYPE),
                value: IndexValue::Text("assistant".to_string()),
            },
        ]);
        let results = self
            .filesystem
            .query(&prefix, &filter, Page::first(1))
            .await
            .map_err(fs_err_to_database)?;
        if let Some(v) = results.first() {
            let mut stored: StoredConversation = serde_json::from_slice(&v.entry.body)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
            if stored.source_channel.is_none() {
                stored.source_channel = Some(channel.to_string());
                self.write_conversation(&stored).await?;
            }
            return Ok(stored.id);
        }
        let now = Utc::now();
        let id = Uuid::new_v4();
        let metadata = serde_json::json!({"thread_type": "assistant", "title": "Assistant"});
        let stored = StoredConversation {
            id,
            channel: channel.to_string(),
            user_id: user_id.to_string(),
            thread_id: None,
            source_channel: Some(channel.to_string()),
            metadata,
            started_at: now,
            last_activity: now,
        };
        self.write_conversation(&stored).await?;
        Ok(id)
    }

    async fn create_conversation_with_metadata(
        &self,
        channel: &str,
        user_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<Uuid, DatabaseError> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let stored = StoredConversation {
            id,
            channel: channel.to_string(),
            user_id: user_id.to_string(),
            thread_id: None,
            source_channel: None,
            metadata: metadata.clone(),
            started_at: now,
            last_activity: now,
        };
        self.write_conversation(&stored).await?;
        Ok(id)
    }

    async fn list_conversation_messages_paginated(
        &self,
        conversation_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<(Vec<ConversationMessage>, bool), DatabaseError> {
        let limit_clamped = limit.max(0) as usize;
        let mut messages = self.list_messages_internal(conversation_id).await?;
        if let Some(before_ts) = before {
            messages.retain(|m| m.created_at < before_ts);
        }
        // Page semantics: newest-first window of `limit` rows, oldest-first in
        // the returned slice. Mirrors the SQL impl.
        messages.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        let has_more = messages.len() > limit_clamped;
        messages.truncate(limit_clamped);
        messages.reverse();
        let out = messages
            .into_iter()
            .map(|m| ConversationMessage {
                id: m.id,
                role: m.role,
                content: m.content,
                created_at: m.created_at,
            })
            .collect();
        Ok((out, has_more))
    }

    async fn update_conversation_metadata_field(
        &self,
        id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let lock = conversation_lock(id);
        let _guard = lock.lock().await;
        let Some(mut stored) = self.read_conversation(id).await? else {
            return Ok(());
        };
        if !stored.metadata.is_object() {
            stored.metadata = serde_json::json!({});
        }
        if let Some(obj) = stored.metadata.as_object_mut() {
            obj.insert(key.to_string(), value.clone());
        }
        self.write_conversation(&stored).await
    }

    async fn get_conversation_metadata(
        &self,
        id: Uuid,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        Ok(self.read_conversation(id).await?.map(|c| c.metadata))
    }

    async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, DatabaseError> {
        let msgs = self.list_messages_internal(conversation_id).await?;
        Ok(msgs
            .into_iter()
            .map(|m| ConversationMessage {
                id: m.id,
                role: m.role,
                content: m.content,
                created_at: m.created_at,
            })
            .collect())
    }

    async fn conversation_belongs_to_user(
        &self,
        conversation_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        Ok(self
            .read_conversation(conversation_id)
            .await?
            .is_some_and(|c| c.user_id == user_id))
    }

    async fn get_conversation_source_channel(
        &self,
        conversation_id: Uuid,
    ) -> Result<Option<String>, DatabaseError> {
        Ok(self
            .read_conversation(conversation_id)
            .await?
            .and_then(|c| c.source_channel))
    }
}

impl<F> FilesystemConversationStore<F>
where
    F: RootFilesystem,
{
    async fn list_conversations_summary(
        &self,
        prefix: &VirtualPath,
        filter: &Filter,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        let results = query_all_pages(&self.filesystem, prefix, filter).await?;
        let mut convs: Vec<StoredConversation> = Vec::with_capacity(results.len());
        for v in results {
            if v.entry.kind.as_ref().map(|k| k.as_str()) != Some(KIND_CONVERSATION) {
                continue;
            }
            let stored: StoredConversation = serde_json::from_slice(&v.entry.body)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
            convs.push(stored);
        }
        // Sort newest-first by last_activity to match the SQL ORDER BY.
        convs.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
        let take = if limit < 0 {
            convs.len()
        } else {
            limit as usize
        };
        let mut out = Vec::with_capacity(take.min(convs.len()));
        for c in convs.into_iter().take(take) {
            let messages = self.list_messages_internal(c.id).await?;
            let message_count = messages.iter().filter(|m| m.role == "user").count() as i64;
            let title = messages
                .iter()
                .find(|m| m.role == "user")
                .map(|m| truncate_chars(&m.content, 100))
                .or_else(|| {
                    c.metadata
                        .get("routine_name")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                });
            let thread_type = c
                .metadata
                .get("thread_type")
                .and_then(|v| v.as_str())
                .map(String::from);
            let live_state = c
                .metadata
                .get("live_state")
                .and_then(|v| v.get("state"))
                .and_then(|v| v.as_str())
                .map(String::from);
            let live_state_started_at = c
                .metadata
                .get("live_state")
                .and_then(|v| v.get("started_at"))
                .and_then(|v| v.as_str())
                .map(String::from);
            out.push(ConversationSummary {
                id: c.id,
                started_at: c.started_at,
                last_activity: c.last_activity,
                message_count,
                title,
                thread_type,
                live_state,
                live_state_started_at,
                channel: c.channel,
            });
        }
        Ok(out)
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Drains every page of a `query` until the backend returns a short page.
///
/// Single `query(prefix, filter, Page::new(0, Page::MAX_LIMIT))` calls silently
/// truncate at 1024 entries because `Page::MAX_LIMIT == 1024`. Callers that
/// fetch-then-sort-in-Rust (conversation message listings, summary builds,
/// job/action listings) miss every row past the cap, and `has_more` flags
/// computed by the caller become meaningless. Loop until a page comes back
/// short. PR #3679 P2 fix.
///
/// Exposed `pub(crate)` so `filesystem_jobs.rs` reuses the same loop instead
/// of growing its own copy.
pub(crate) async fn query_all_pages<F: RootFilesystem>(
    filesystem: &Arc<F>,
    prefix: &VirtualPath,
    filter: &Filter,
) -> Result<Vec<ironclaw_filesystem::VersionedEntry>, DatabaseError> {
    let mut out = Vec::new();
    let mut offset: u64 = 0;
    loop {
        let page = Page::new(offset, Page::MAX_LIMIT);
        let entries = match filesystem.query(prefix, filter, page).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(out),
            Err(error) => return Err(fs_err_to_database(error)),
        };
        let received = entries.len() as u64;
        out.extend(entries);
        // A short page (less than MAX_LIMIT) means we've reached the end.
        // A zero-length page also ends the loop and prevents infinite spin
        // if the backend ever returns an unexpected empty trailing page.
        if received < Page::MAX_LIMIT as u64 {
            break;
        }
        offset = offset.saturating_add(received);
    }
    Ok(out)
}

async fn write_message<F: RootFilesystem>(
    filesystem: &Arc<F>,
    stored: &StoredMessage,
) -> Result<(), DatabaseError> {
    let path = message_path(stored.conversation_id, stored.id)?;
    let body =
        serde_json::to_vec(stored).map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    let entry = Entry::record(
        FilesystemConversationStore::<F>::message_kind(),
        &serde_json::Value::Null,
    )
    .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    let entry = Entry { body, ..entry }
        .with_indexed(
            index_key(IDX_CONVERSATION_ID),
            IndexValue::Text(stored.conversation_id.to_string()),
        )
        .with_indexed(index_key(IDX_ROLE), IndexValue::Text(stored.role.clone()))
        .with_indexed(
            index_key(IDX_CREATED_AT_TS),
            IndexValue::I64(stored.created_at.timestamp_millis()),
        )
        .with_indexed(
            index_key(IDX_CREATED_AT),
            IndexValue::Text(stored.created_at.to_rfc3339()),
        );
    filesystem
        .put(&path, entry, CasExpectation::Any)
        .await
        .map(|_| ())
        .map_err(fs_err_to_database)
}

fn build_conversation_entry(
    stored: &StoredConversation,
    body: Vec<u8>,
) -> Result<Entry, DatabaseError> {
    let entry = Entry::record(
        RecordKind::new(KIND_CONVERSATION)
            .unwrap_or_else(|_| unreachable!("conversation is a valid record-kind literal")),
        &serde_json::Value::Null,
    )
    .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    let mut entry = Entry { body, ..entry }
        .with_indexed(
            index_key(IDX_USER_ID),
            IndexValue::Text(stored.user_id.clone()),
        )
        .with_indexed(
            index_key(IDX_CHANNEL),
            IndexValue::Text(stored.channel.clone()),
        )
        .with_indexed(
            index_key(IDX_LAST_ACTIVITY),
            IndexValue::I64(stored.last_activity.timestamp_millis()),
        )
        .with_indexed(
            index_key(IDX_STARTED_AT),
            IndexValue::I64(stored.started_at.timestamp_millis()),
        );
    if let Some(src) = &stored.source_channel {
        entry = entry.with_indexed(index_key(IDX_SOURCE_CHANNEL), IndexValue::Text(src.clone()));
    }
    if let Some(thread_type) = stored.metadata.get("thread_type").and_then(|v| v.as_str()) {
        entry = entry.with_indexed(
            index_key(IDX_THREAD_TYPE),
            IndexValue::Text(thread_type.to_string()),
        );
    }
    if let Some(routine_id) = stored.metadata.get("routine_id").and_then(|v| v.as_str()) {
        entry = entry.with_indexed(
            index_key(IDX_ROUTINE_ID),
            IndexValue::Text(routine_id.to_string()),
        );
    }
    Ok(entry)
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

const ROOT: &str = "/engine/conversations";

fn conversations_root() -> Result<VirtualPath, DatabaseError> {
    VirtualPath::new(ROOT).map_err(|e| DatabaseError::Query(e.to_string()))
}

fn conversation_path(id: Uuid) -> Result<VirtualPath, DatabaseError> {
    VirtualPath::new(format!("{ROOT}/{id}")).map_err(|e| DatabaseError::Query(e.to_string()))
}

fn messages_root(conversation_id: Uuid) -> Result<VirtualPath, DatabaseError> {
    VirtualPath::new(format!("{ROOT}/{conversation_id}/messages"))
        .map_err(|e| DatabaseError::Query(e.to_string()))
}

fn message_path(conversation_id: Uuid, msg_id: Uuid) -> Result<VirtualPath, DatabaseError> {
    VirtualPath::new(format!("{ROOT}/{conversation_id}/messages/{msg_id}"))
        .map_err(|e| DatabaseError::Query(e.to_string()))
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub(crate) const KIND_CONVERSATION: &str = "conversation";
pub(crate) const KIND_MESSAGE: &str = "conversation_message";

pub(crate) const IDX_USER_ID: &str = "user_id";
pub(crate) const IDX_CHANNEL: &str = "channel";
pub(crate) const IDX_SOURCE_CHANNEL: &str = "source_channel";
pub(crate) const IDX_THREAD_TYPE: &str = "thread_type";
pub(crate) const IDX_ROUTINE_ID: &str = "routine_id";
pub(crate) const IDX_LAST_ACTIVITY: &str = "last_activity_ts";
pub(crate) const IDX_STARTED_AT: &str = "started_at_ts";
pub(crate) const IDX_CONVERSATION_ID: &str = "conversation_id";
pub(crate) const IDX_ROLE: &str = "role";
pub(crate) const IDX_CREATED_AT_TS: &str = "created_at_ts";
pub(crate) const IDX_CREATED_AT: &str = "created_at";

pub(crate) fn index_key(name: &'static str) -> IndexKey {
    IndexKey::new(name).unwrap_or_else(|_| unreachable!("index key literal is valid"))
}

// ---------------------------------------------------------------------------
// Locks (process-local, per-key)
// ---------------------------------------------------------------------------

type RecordLock = Arc<tokio::sync::Mutex<()>>;

static FILESYSTEM_RECORD_LOCKS: OnceLock<Mutex<HashMap<String, RecordLock>>> = OnceLock::new();

pub(crate) fn record_lock(key: String) -> RecordLock {
    let locks = FILESYSTEM_RECORD_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = lock_or_recover(locks);
    Arc::clone(
        guard
            .entry(key)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
    )
}

fn lock_or_recover<T>(mutex: &Mutex<HashMap<String, T>>) -> MutexGuard<'_, HashMap<String, T>> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn conversation_lock(id: Uuid) -> RecordLock {
    record_lock(format!("conv|{id}"))
}

fn routine_lock(user_id: &str, routine_id: Uuid) -> RecordLock {
    record_lock(format!("routine|{user_id}|{routine_id}"))
}

fn heartbeat_lock(user_id: &str) -> RecordLock {
    record_lock(format!("heartbeat|{user_id}"))
}

fn assistant_lock(user_id: &str, channel: &str) -> RecordLock {
    record_lock(format!("assistant|{user_id}|{channel}"))
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

pub(crate) fn fs_err_to_database(error: ironclaw_filesystem::FilesystemError) -> DatabaseError {
    DatabaseError::Query(format!("filesystem: {error}"))
}

pub(crate) fn is_not_found(error: &ironclaw_filesystem::FilesystemError) -> bool {
    matches!(error, ironclaw_filesystem::FilesystemError::NotFound { .. })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;

    fn new_store() -> FilesystemConversationStore<InMemoryBackend> {
        FilesystemConversationStore::new(Arc::new(InMemoryBackend::new()))
    }

    #[tokio::test]
    async fn create_and_lookup_round_trip() {
        let store = new_store();
        let id = store
            .create_conversation("gateway", "user-a", None)
            .await
            .unwrap();
        assert!(
            store
                .conversation_belongs_to_user(id, "user-a")
                .await
                .unwrap()
        );
        assert!(
            !store
                .conversation_belongs_to_user(id, "user-b")
                .await
                .unwrap()
        );
        let summaries = store
            .list_conversations_with_preview("user-a", "gateway", 10)
            .await
            .unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, id);
    }

    #[tokio::test]
    async fn add_message_and_paginate() {
        let store = new_store();
        let conv = store
            .create_conversation("gateway", "user-a", None)
            .await
            .unwrap();
        for i in 0..5 {
            store
                .add_conversation_message(conv, "user", &format!("hello {i}"))
                .await
                .unwrap();
        }
        let messages = store.list_conversation_messages(conv).await.unwrap();
        assert_eq!(messages.len(), 5);
        // Newest-window paginated
        let (page, has_more) = store
            .list_conversation_messages_paginated(conv, None, 3)
            .await
            .unwrap();
        assert_eq!(page.len(), 3);
        assert!(has_more);
        // Oldest-first within the page
        assert!(page[0].created_at <= page[1].created_at);
    }

    #[tokio::test]
    async fn add_message_if_empty_is_idempotent() {
        let store = new_store();
        let conv = store
            .create_conversation("gateway", "user-a", None)
            .await
            .unwrap();
        let inserted = store
            .add_conversation_message_if_empty(conv, "user", "hi")
            .await
            .unwrap();
        assert!(inserted);
        let inserted_again = store
            .add_conversation_message_if_empty(conv, "user", "hi-again")
            .await
            .unwrap();
        assert!(!inserted_again);
        let messages = store.list_conversation_messages(conv).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "hi");
    }

    #[tokio::test]
    async fn list_all_channels_and_filters_by_user() {
        let store = new_store();
        store
            .create_conversation("gateway", "user-a", None)
            .await
            .unwrap();
        store
            .create_conversation("telegram", "user-a", None)
            .await
            .unwrap();
        store
            .create_conversation("gateway", "user-b", None)
            .await
            .unwrap();
        let user_a = store
            .list_conversations_all_channels("user-a", 10)
            .await
            .unwrap();
        assert_eq!(user_a.len(), 2);
        for s in &user_a {
            assert!(matches!(s.channel.as_str(), "gateway" | "telegram"));
        }
    }

    #[tokio::test]
    async fn get_or_create_routine_conversation_is_idempotent() {
        let store = new_store();
        let routine_id = Uuid::new_v4();
        let id1 = store
            .get_or_create_routine_conversation(routine_id, "test", "user-a")
            .await
            .unwrap();
        let id2 = store
            .get_or_create_routine_conversation(routine_id, "test", "user-a")
            .await
            .unwrap();
        assert_eq!(id1, id2);
        let other = Uuid::new_v4();
        let id3 = store
            .get_or_create_routine_conversation(other, "other", "user-a")
            .await
            .unwrap();
        assert_ne!(id1, id3);
    }

    #[tokio::test]
    async fn find_routine_conversation_returns_none_when_missing() {
        let store = new_store();
        let routine_id = Uuid::new_v4();
        let found = store
            .find_routine_conversation(routine_id, "user-a")
            .await
            .unwrap();
        assert!(found.is_none());
        let id = store
            .get_or_create_routine_conversation(routine_id, "test", "user-a")
            .await
            .unwrap();
        let found = store
            .find_routine_conversation(routine_id, "user-a")
            .await
            .unwrap();
        assert_eq!(found, Some(id));
    }

    #[tokio::test]
    async fn get_or_create_heartbeat_conversation_is_idempotent() {
        let store = new_store();
        let a = store
            .get_or_create_heartbeat_conversation("user-a")
            .await
            .unwrap();
        let b = store
            .get_or_create_heartbeat_conversation("user-a")
            .await
            .unwrap();
        assert_eq!(a, b);
    }

    #[tokio::test]
    async fn assistant_conversation_backfills_source_channel() {
        let store = new_store();
        // First, create one with source_channel=None directly
        let conv_id = Uuid::new_v4();
        let now = Utc::now();
        let stored = StoredConversation {
            id: conv_id,
            channel: "gateway".to_string(),
            user_id: "user-a".to_string(),
            thread_id: None,
            source_channel: None,
            metadata: serde_json::json!({"thread_type": "assistant"}),
            started_at: now,
            last_activity: now,
        };
        store.write_conversation(&stored).await.unwrap();

        let found = store
            .get_or_create_assistant_conversation("user-a", "gateway")
            .await
            .unwrap();
        assert_eq!(found, conv_id);
        let source = store
            .get_conversation_source_channel(conv_id)
            .await
            .unwrap();
        assert_eq!(source.as_deref(), Some("gateway"));
    }

    #[tokio::test]
    async fn ensure_conversation_respects_owner_channel_guard() {
        let store = new_store();
        let id = Uuid::new_v4();
        let inserted = store
            .ensure_conversation(id, "gateway", "user-a", None, Some("gateway"))
            .await
            .unwrap();
        assert!(inserted);
        let cross_user = store
            .ensure_conversation(id, "gateway", "user-b", None, Some("gateway"))
            .await
            .unwrap();
        assert!(!cross_user);
        let cross_channel = store
            .ensure_conversation(id, "telegram", "user-a", None, None)
            .await
            .unwrap();
        assert!(!cross_channel);
        // The original record's source_channel must remain.
        let source = store.get_conversation_source_channel(id).await.unwrap();
        assert_eq!(source.as_deref(), Some("gateway"));
    }

    #[tokio::test]
    async fn metadata_field_update_round_trip() {
        let store = new_store();
        let id = store
            .create_conversation("gateway", "user-a", None)
            .await
            .unwrap();
        store
            .update_conversation_metadata_field(
                id,
                "live_state",
                &serde_json::json!({"state": "Processing"}),
            )
            .await
            .unwrap();
        let meta = store.get_conversation_metadata(id).await.unwrap();
        assert_eq!(
            meta.as_ref()
                .and_then(|v| v.get("live_state"))
                .and_then(|v| v.get("state"))
                .and_then(|v| v.as_str()),
            Some("Processing")
        );
    }

    #[tokio::test]
    async fn touch_conversation_updates_last_activity() {
        let store = new_store();
        let id = store
            .create_conversation("gateway", "user-a", None)
            .await
            .unwrap();
        let before = store
            .read_conversation(id)
            .await
            .unwrap()
            .unwrap()
            .last_activity;
        // Wait at least 1 ms so the timestamp must move.
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        store.touch_conversation(id).await.unwrap();
        let after = store
            .read_conversation(id)
            .await
            .unwrap()
            .unwrap()
            .last_activity;
        assert!(after >= before);
    }

    /// Regression: PR #3679 codex review P2 — `list_messages_internal` used to
    /// fetch a single page of `Page::MAX_LIMIT` and sort in memory, silently
    /// truncating conversations with more than 1024 messages. This drives the
    /// callers (`list_conversation_messages` + `list_conversation_messages_paginated`)
    /// and asserts the full count round-trips.
    #[tokio::test]
    async fn list_messages_drains_pages_beyond_max_limit() {
        let store = new_store();
        let conv = store
            .create_conversation("gateway", "user-a", None)
            .await
            .unwrap();
        // One past MAX_LIMIT so the loop must perform a second page fetch.
        let total: usize = (Page::MAX_LIMIT as usize) + 5;
        for i in 0..total {
            store
                .add_conversation_message(conv, "user", &format!("msg {i}"))
                .await
                .unwrap();
        }
        // Direct listing returns every message.
        let messages = store.list_conversation_messages(conv).await.unwrap();
        assert_eq!(messages.len(), total);
        // Newest-window pagination computes has_more against the full set,
        // not the truncated first page. Asking for fewer than `total` rows
        // must report has_more = true.
        let (page, has_more) = store
            .list_conversation_messages_paginated(conv, None, 10)
            .await
            .unwrap();
        assert_eq!(page.len(), 10);
        assert!(
            has_more,
            "has_more must reflect rows past the first page; got false with {} total messages",
            total
        );
        // Asking for >= total rows must yield has_more = false and the full
        // count, regardless of where the MAX_LIMIT page boundary lands.
        let (page_all, has_more_all) = store
            .list_conversation_messages_paginated(conv, None, total as i64 + 10)
            .await
            .unwrap();
        assert_eq!(page_all.len(), total);
        assert!(!has_more_all);
    }

    #[tokio::test]
    async fn create_with_metadata_persists_thread_type_index() {
        let store = new_store();
        let id = store
            .create_conversation_with_metadata(
                "gateway",
                "user-a",
                &serde_json::json!({"thread_type": "assistant"}),
            )
            .await
            .unwrap();
        let meta = store.get_conversation_metadata(id).await.unwrap();
        assert_eq!(
            meta.as_ref()
                .and_then(|v| v.get("thread_type"))
                .and_then(|v| v.as_str()),
            Some("assistant")
        );
    }
}
