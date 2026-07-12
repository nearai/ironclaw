// arch-exempt: large_file, filesystem thread service decomposition, plan #5662
//! Filesystem-backed canonical session thread and transcript service.
//!
//! Records live under the `/threads` mount alias on a
//! [`ScopedFilesystem`](ironclaw_filesystem::ScopedFilesystem). The paths in
//! this module are alias-relative [`ScopedPath`](ironclaw_host_api::ScopedPath)
//! strings — at every op the [`ScopedFilesystem`] resolves the alias against
//! its caller-supplied [`MountView`](ironclaw_host_api::MountView) and enforces
//! per-grant ACL before backend dispatch. The composition layer wires the
//! alias to a tenant/user-scoped
//! [`VirtualPath`](ironclaw_host_api::VirtualPath), so tenant isolation is
//! structural rather than something this crate must re-derive from
//! `ThreadScope.tenant_id`.
//!
//! Within the alias, sub-scope (`agent_id`, `project_id`, `owner_user_id`,
//! `mission_id`) is encoded in the path so a single tenant/user can own
//! multiple agent/project/mission cells. Within a single thread, messages,
//! summary artifacts, and inbound idempotency records are stored as
//! individual records keyed by their identifiers:
//!
//! ```text
//! /threads[/agents/<agent>][/projects/<project>][/owners/<owner_user>][/missions/<mission>]/threads/<thread_id>/thread.json
//! /threads[/.../...]/threads/<thread_id>/messages/<message_id>.json
//! /threads[/.../...]/threads/<thread_id>/summaries/<summary_id>.json
//! /threads/idempotency/<sha256>.json
//! ```
//!
//! The idempotency record key SHA-256s the full (`scope`,
//! `source_binding_id`, `external_event_id`) tuple, so flat layout under one
//! `/threads/idempotency/` directory is safe — two different scopes with
//! identical binding/event id produce different on-disk keys. The
//! `replay_accepted_inbound_message` lookup, which has no scope input, scans
//! that directory and matches `source_binding_id`+`external_event_id` against
//! the persisted record body.

mod message_lookup_index;
mod message_sequence_index;
mod thread_index;

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, Weak},
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::{StreamExt, future::join_all};
use ironclaw_filesystem::{
    CasApply, CasExpectation, CasUpdateError, ContentType, Entry, FilesystemError,
    FilesystemOperation, Filter, Page, RecordKind, RecordVersion, RootFilesystem, ScopedFilesystem,
    SeqNo, cas_update,
};
use ironclaw_host_api::{HostApiError, InvocationId, ResourceScope, ScopedPath, ThreadId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

use crate::identifiers::SummaryArtifactId;
use crate::summary_artifacts::find_overlapping_summary;
use crate::title::derive_title_from_message;
use crate::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
    AppendAssistantDraftRequest, AppendCapabilityDisplayPreviewRequest,
    AppendFinalizedAssistantMessageRequest, AppendToolResultReferenceRequest,
    CapabilityDisplayPreviewEnvelope, ContextMessage, ContextMessages, ContextWindow,
    CreateSummaryArtifactRequest, EnsureThreadRequest, LatestThreadMessageRequest,
    ListThreadsForScopeRequest, ListThreadsForScopeResponse, LoadContextMessagesRequest,
    LoadContextWindowRequest, MessageContent, MessageKind, MessageStatus,
    ProviderToolCallReferenceEnvelope, RedactMessageRequest, ReplayAcceptedInboundMessageRequest,
    SessionThreadError, SessionThreadRecord, SessionThreadService, SummaryArtifact,
    SummaryModelContextPolicy, ThreadHistory, ThreadHistoryRequest, ThreadMessageId,
    ThreadMessageRange, ThreadMessageRangeRequest, ThreadMessageRecord, ThreadScope,
    ToolResultReferenceEnvelope, UpdateAssistantDraftRequest, UpdateToolResultReferenceRequest,
};
use message_lookup_index::MessageLookupIndexStore;
use message_sequence_index::{MessageSequenceIndexStore, message_sequence_index_entry_for_message};
use thread_index::ThreadIndexRecord;

/// Bound on the CAS retry loop. Mirrors the run-state / authorization
/// store budgets — enough to absorb routine cross-process contention,
/// small enough to surface pathological loops loudly.
const FILESYSTEM_CAS_RETRIES: usize = 8;

/// [`RecordKind`] discriminants for the four record types persisted by this
/// service. Setting `entry.kind` makes writes record-shaped so
/// [`LocalFilesystem`] (which rejects record-shaped puts) triggers the
/// fail-closed path on the CAS gate instead of accepting a byte-only first
/// write without CAS enforcement.
const SESSION_THREAD_KIND: &str = "session_thread";
const THREAD_MESSAGE_KIND: &str = "thread_message";
const THREAD_SUMMARY_KIND: &str = "thread_summary";
const THREAD_IDEMPOTENCY_KIND: &str = "thread_idempotency";

/// Conservative fan-out for indexed range materialization.
const INDEXED_RANGE_MESSAGE_READ_CONCURRENCY: usize = 8;
/// Conservative fan-out for per-thread title derivation during sidebar listing.
const TITLE_DERIVATION_READ_CONCURRENCY: usize = 8;
/// One-shot first-turn context windows are a hot-path handoff from inbound
/// accept to prompt construction; keep the cache bounded if a turn never runs.
const ONE_SHOT_CONTEXT_WINDOW_CACHE_MAX_ENTRIES: usize = 4096;

#[derive(Debug, Clone, Copy)]
enum MessageRangeFallbackPolicy {
    FullScan,
}

struct MaterializedMessageRange {
    thread: StoredThreadRecord,
    messages: Vec<ThreadMessageRecord>,
}

enum TransactionalMessageWrite {
    Unsupported,
    Written,
    IdempotencyAlreadyAccepted,
}

/// On-disk thread state record. The transcript boundary's
/// [`SessionThreadRecord`] is the user-visible shape; this struct adds
/// `next_sequence` so the per-thread monotonic counter is durable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StoredThreadRecord {
    #[serde(flatten)]
    record: SessionThreadRecord,
    next_sequence: u64,
}

/// On-disk transcript message record.
///
/// `ThreadMessageRecord` deliberately skips provider replay metadata when it is
/// serialized for product-facing transcript surfaces. The filesystem service is
/// the private backend for model context, so it stores that metadata explicitly
/// while history reads continue to scrub it before returning records.
#[derive(Serialize)]
struct StoredThreadMessageRecord<'a> {
    #[serde(flatten)]
    record: &'a ThreadMessageRecord,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_result_provider_call: &'a Option<ProviderToolCallReferenceEnvelope>,
}

impl<'a> From<&'a ThreadMessageRecord> for StoredThreadMessageRecord<'a> {
    fn from(record: &'a ThreadMessageRecord) -> Self {
        Self {
            record,
            tool_result_provider_call: &record.tool_result_provider_call,
        }
    }
}

/// On-disk inbound idempotency record. Includes the originating scope so
/// the scope-less `replay_accepted_inbound_message` can rehydrate the
/// replay reply.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct InboundIdempotencyRecord {
    scope: ThreadScope,
    source_binding_id: String,
    external_event_id: String,
    thread_id: ThreadId,
    message_id: ThreadMessageId,
}

/// Filesystem-backed [`SessionThreadService`].
///
/// Construct with an [`Arc<ScopedFilesystem<F>>`](ScopedFilesystem) over
/// any [`RootFilesystem`]. The [`ScopedFilesystem`] resolves the
/// `/threads` alias to a tenant/user-scoped
/// [`VirtualPath`](ironclaw_host_api::VirtualPath) per its
/// [`MountView`](ironclaw_host_api::MountView) and enforces per-op ACL
/// before backend dispatch — so tenant isolation is structural rather
/// than something this crate must re-derive from
/// `ThreadScope.tenant_id`. Within-tenant axes (`agent_id`,
/// `project_id`, `owner_user_id`, `mission_id`) stay in the
/// alias-relative path because they are not covered by the per-tenant
/// `MountAlias`.
pub struct FilesystemSessionThreadService<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    thread_index_cache: Mutex<HashMap<String, Arc<Vec<ThreadIndexRecord>>>>,
    thread_index_cursor_positions: Mutex<HashMap<String, Arc<HashMap<String, usize>>>>,
    thread_index_cache_epochs: Mutex<HashMap<String, u64>>,
    thread_index_manual_clear_epochs: Mutex<HashMap<String, u64>>,
    thread_index_load_locks: Mutex<HashMap<String, Weak<AsyncMutex<()>>>>,
    known_thread_index_rows: Mutex<HashSet<String>>,
    known_thread_source_rows: Mutex<HashMap<String, HashSet<ThreadId>>>,
    complete_thread_source_scopes: Mutex<HashSet<String>>,
    thread_index_force_validate_scopes: Mutex<HashSet<String>>,
    complete_thread_index_scopes: Mutex<HashSet<String>>,
    one_shot_context_windows: Mutex<HashMap<String, ContextWindow>>,
}

impl<F> FilesystemSessionThreadService<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            filesystem,
            thread_index_cache: Mutex::new(HashMap::new()),
            thread_index_cursor_positions: Mutex::new(HashMap::new()),
            thread_index_cache_epochs: Mutex::new(HashMap::new()),
            thread_index_manual_clear_epochs: Mutex::new(HashMap::new()),
            thread_index_load_locks: Mutex::new(HashMap::new()),
            known_thread_index_rows: Mutex::new(HashSet::new()),
            known_thread_source_rows: Mutex::new(HashMap::new()),
            complete_thread_source_scopes: Mutex::new(HashSet::new()),
            thread_index_force_validate_scopes: Mutex::new(HashSet::new()),
            complete_thread_index_scopes: Mutex::new(HashSet::new()),
            one_shot_context_windows: Mutex::new(HashMap::new()),
        }
    }

    pub fn clear_thread_index_cache_for_scope(&self, scope: &ThreadScope) {
        self.clear_thread_index_cache_for_scope_once(scope);
    }

    fn seed_one_shot_context_window(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message: &ThreadMessageRecord,
    ) {
        let messages = vec![message.clone()];
        let context = ContextWindow {
            thread_id: thread_id.clone(),
            messages: context_messages_with_summary_replacements(&messages, &[]),
        };
        if let Ok(mut cache) = self.one_shot_context_windows.lock() {
            let key = one_shot_context_window_cache_key(scope, thread_id);
            cache.insert(key.clone(), context);
            evict_hash_map_entry_over_limit(
                &mut cache,
                ONE_SHOT_CONTEXT_WINDOW_CACHE_MAX_ENTRIES,
                &key,
            );
        }
    }

    fn take_one_shot_context_window(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        max_messages: usize,
    ) -> Option<ContextWindow> {
        let key = one_shot_context_window_cache_key(scope, thread_id);
        let mut context = self.one_shot_context_windows.lock().ok()?.remove(&key)?;
        if max_messages < context.messages.len() {
            let start = context.messages.len() - max_messages;
            context.messages = context.messages.split_off(start);
        }
        Some(context)
    }

    fn invalidate_one_shot_context_window(&self, scope: &ThreadScope, thread_id: &ThreadId) {
        if let Ok(mut cache) = self.one_shot_context_windows.lock() {
            cache.remove(&one_shot_context_window_cache_key(scope, thread_id));
        }
    }

    fn thread_entry(record: &StoredThreadRecord) -> Result<Entry, SessionThreadError> {
        let body = serialize_pretty(record)?;
        let kind = RecordKind::new(SESSION_THREAD_KIND).map_err(|error| {
            SessionThreadError::Backend(format!("invalid session_thread record kind: {error}"))
        })?;
        let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
        entry.kind = Some(kind);
        Ok(entry)
    }

    fn message_entry(record: &ThreadMessageRecord) -> Result<Entry, SessionThreadError> {
        let body = serialize_pretty(&StoredThreadMessageRecord::from(record))?;
        let kind = RecordKind::new(THREAD_MESSAGE_KIND).map_err(|error| {
            SessionThreadError::Backend(format!("invalid thread_message record kind: {error}"))
        })?;
        let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
        entry.kind = Some(kind);
        Ok(entry)
    }

    fn summary_entry(record: &SummaryArtifact) -> Result<Entry, SessionThreadError> {
        let body = serialize_pretty(record)?;
        let kind = RecordKind::new(THREAD_SUMMARY_KIND).map_err(|error| {
            SessionThreadError::Backend(format!("invalid thread_summary record kind: {error}"))
        })?;
        let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
        entry.kind = Some(kind);
        Ok(entry)
    }

    fn idempotency_entry(record: &InboundIdempotencyRecord) -> Result<Entry, SessionThreadError> {
        let body = serialize_pretty(record)?;
        let kind = RecordKind::new(THREAD_IDEMPOTENCY_KIND).map_err(|error| {
            SessionThreadError::Backend(format!("invalid thread_idempotency record kind: {error}"))
        })?;
        let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
        entry.kind = Some(kind);
        Ok(entry)
    }

    async fn read_thread_versioned(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<Option<(StoredThreadRecord, RecordVersion)>, SessionThreadError> {
        let path = thread_record_path(scope, thread_id)?;
        let Some(versioned) = self
            .filesystem
            .get(&scope.to_resource_scope(), &path)
            .await?
        else {
            return Ok(None);
        };
        let record = deserialize::<StoredThreadRecord>(&versioned.entry.body)?;
        if &record.record.scope != scope || &record.record.thread_id != thread_id {
            return Ok(None);
        }
        Ok(Some((record, versioned.version)))
    }

    async fn read_message_versioned(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<Option<(ThreadMessageRecord, RecordVersion)>, SessionThreadError> {
        let path = message_record_path(scope, thread_id, message_id)?;
        let Some(versioned) = self
            .filesystem
            .get(&scope.to_resource_scope(), &path)
            .await?
        else {
            let Some(events) = self.read_message_append_events(scope, thread_id).await? else {
                return Ok(None);
            };
            return Ok(events
                .into_iter()
                .find(|(record, _)| record.message_id == message_id));
        };
        let record = deserialize::<ThreadMessageRecord>(&versioned.entry.body)?;
        if &record.thread_id != thread_id || record.message_id != message_id {
            return Ok(None);
        }
        Ok(Some((record, versioned.version)))
    }

    async fn write_message_sequence_index(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message: &ThreadMessageRecord,
    ) -> Result<(), SessionThreadError> {
        MessageSequenceIndexStore::new(self.filesystem.as_ref())
            .write_new(scope, thread_id, message)
            .await
    }

    async fn write_message_lookup_indexes(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message: &ThreadMessageRecord,
    ) -> Result<(), SessionThreadError> {
        MessageLookupIndexStore::new(self.filesystem.as_ref())
            .write_for_message(scope, thread_id, message)
            .await
    }

    async fn write_message_lookup_indexes_best_effort(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message: &ThreadMessageRecord,
        context: &'static str,
    ) {
        if let Err(error) = self
            .write_message_lookup_indexes(scope, thread_id, message)
            .await
        {
            // Lookup indexes are acceleration/backfill records. The message
            // record is the source of truth and lookup reads fall back to a
            // transcript scan, so index failures must not turn an already
            // persisted message write into an apparent append/update failure.
            tracing::debug!(
                ?error,
                ?scope,
                thread_id = %thread_id.as_str(),
                message_id = %message.message_id,
                kind = ?message.kind,
                status = ?message.status,
                context = context,
                "message lookup index write failed; continuing with source-of-truth message record",
            );
        }
    }

    async fn write_new_message(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message: &ThreadMessageRecord,
        description: &'static str,
    ) -> Result<(), SessionThreadError> {
        crate::contract::validate_new_message_timestamps(message, description)?;
        let path = message_record_path(scope, thread_id, message.message_id)?;
        let entry = Self::message_entry(message)?;
        match put_with_cas(
            self.filesystem.as_ref(),
            &scope.to_resource_scope(),
            &path,
            entry,
            CasExpectation::Absent,
        )
        .await
        {
            Ok(()) => {
                self.write_message_sequence_index(scope, thread_id, message)
                    .await?;
                self.write_message_lookup_indexes_best_effort(
                    scope,
                    thread_id,
                    message,
                    "new message",
                )
                .await;
                self.invalidate_one_shot_context_window(scope, thread_id);
                Ok(())
            }
            Err(PutError::VersionMismatch) => Err(SessionThreadError::Backend(format!(
                "filesystem CAS Absent rejected new {description} at {}",
                path.as_str()
            ))),
            Err(PutError::Other(error)) => Err(error),
        }
    }

    async fn append_message_event(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message: &ThreadMessageRecord,
    ) -> Result<bool, SessionThreadError> {
        crate::contract::validate_new_message_timestamps(message, "message append event")?;
        let path = message_append_log_path(scope, thread_id)?;
        let payload = serialize_pretty(&StoredThreadMessageRecord::from(message))?;
        match self
            .filesystem
            .append(&scope.to_resource_scope(), &path, payload)
            .await
        {
            Ok(_) => {
                self.invalidate_one_shot_context_window(scope, thread_id);
                Ok(true)
            }
            Err(FilesystemError::Unsupported {
                operation: FilesystemOperation::Append,
                ..
            }) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    async fn read_message_append_events(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<Option<Vec<(ThreadMessageRecord, RecordVersion)>>, SessionThreadError> {
        let path = message_append_log_path(scope, thread_id)?;
        let events = match self
            .filesystem
            .tail(&scope.to_resource_scope(), &path, SeqNo::ZERO)
            .await
        {
            Ok(events) => events,
            Err(FilesystemError::Unsupported {
                operation: FilesystemOperation::Tail,
                ..
            }) => return Ok(None),
            Err(FilesystemError::NotFound { .. }) => return Ok(Some(Vec::new())),
            Err(error) => return Err(error.into()),
        };

        let mut messages = Vec::with_capacity(events.len());
        for event in events {
            let message = deserialize::<ThreadMessageRecord>(&event.payload)?;
            if &message.thread_id == thread_id {
                messages.push((message, RecordVersion::from_backend(event.seq.get())));
            }
        }
        messages.sort_by_key(|(message, _)| message.sequence);
        Ok(Some(messages))
    }

    async fn list_message_append_events(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<Vec<ThreadMessageRecord>, SessionThreadError> {
        Ok(self
            .read_message_append_events(scope, thread_id)
            .await?
            .unwrap_or_default()
            .into_iter()
            .map(|(message, _)| message)
            .collect())
    }

    async fn merge_message_append_events(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        messages: &mut Vec<ThreadMessageRecord>,
    ) -> Result<(), SessionThreadError> {
        let mut event_messages = self.list_message_append_events(scope, thread_id).await?;
        if event_messages.is_empty() {
            return Ok(());
        }
        // File-authoritative merge: a per-message file always wins over its
        // append-log entry. The log only contributes messages that have no
        // file yet (a finalized message written solely via
        // `append_message_event`). This matches the single-message read in
        // `read_message_versioned` (file first, log fallback) and lets
        // `apply_message_update` materialize a file on mutation
        // (redaction/status change) and have that file shadow the original
        // log entry. Were the log to win, a redacted message's file would be
        // masked by its stale log record.
        event_messages.retain(|event| {
            !messages
                .iter()
                .any(|existing| existing.message_id == event.message_id)
        });
        messages.append(&mut event_messages);
        Ok(())
    }

    async fn try_write_new_message_transactionally(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message: &mut ThreadMessageRecord,
        idempotency_record: Option<(&ScopedPath, &Entry)>,
    ) -> Result<TransactionalMessageWrite, SessionThreadError> {
        crate::contract::validate_new_message_timestamps(message, "transactional message")?;
        let resource_scope = scope.to_resource_scope();
        let txn_prefix = scoped_path(THREADS_PREFIX)?;
        let thread_path = thread_record_path(scope, thread_id)?;
        let thread_virtual_path = self.filesystem.resolve(&resource_scope, &thread_path)?;
        let message_path = message_record_path(scope, thread_id, message.message_id)?;
        let message_virtual_path = self.filesystem.resolve(&resource_scope, &message_path)?;
        let idempotency_record = idempotency_record
            .map(|(path, entry)| {
                self.filesystem
                    .resolve(&resource_scope, path)
                    .map(|virtual_path| (path, virtual_path, entry))
            })
            .transpose()?;

        for _ in 0..FILESYSTEM_CAS_RETRIES {
            let mut txn = match self.filesystem.begin(&resource_scope, &txn_prefix).await {
                Ok(txn) => txn,
                Err(FilesystemError::Unsupported {
                    operation: FilesystemOperation::BeginTxn,
                    ..
                }) => return Ok(TransactionalMessageWrite::Unsupported),
                Err(error) => return Err(error.into()),
            };

            if let Some((_, virtual_path, entry)) = &idempotency_record {
                match txn
                    .put(virtual_path, (*entry).clone(), CasExpectation::Absent)
                    .await
                {
                    Ok(_) => {}
                    Err(error) => {
                        txn.rollback().await;
                        return match error {
                            FilesystemError::VersionMismatch { .. } => {
                                Ok(TransactionalMessageWrite::IdempotencyAlreadyAccepted)
                            }
                            error => Err(error.into()),
                        };
                    }
                }
            }

            let Some(versioned_thread) = txn.get(&thread_virtual_path).await? else {
                txn.rollback().await;
                return Err(SessionThreadError::UnknownThread {
                    thread_id: thread_id.clone(),
                });
            };
            let mut stored = deserialize::<StoredThreadRecord>(&versioned_thread.entry.body)?;
            let thread_version = versioned_thread.version;
            if &stored.record.scope != scope || &stored.record.thread_id != thread_id {
                txn.rollback().await;
                return Err(SessionThreadError::UnknownThread {
                    thread_id: thread_id.clone(),
                });
            }

            if message.sequence == 0 {
                if stored.next_sequence > 1 {
                    let assigned = stored.next_sequence;
                    stored.next_sequence = assigned + 1;
                    stored.record.updated_at = Some(Utc::now());
                    let entry = Self::thread_entry(&stored)?;
                    if let Err(error) = txn
                        .put(
                            &thread_virtual_path,
                            entry,
                            CasExpectation::Version(thread_version),
                        )
                        .await
                    {
                        txn.rollback().await;
                        return Err(absent_put_error(error, "thread", &thread_path));
                    }
                    message.sequence = assigned;
                } else {
                    let sequence_path = message_sequence_counter_path(scope, thread_id)?;
                    let sequence_virtual_path =
                        self.filesystem.resolve(&resource_scope, &sequence_path)?;
                    match txn.reserve_sequence(&sequence_virtual_path).await {
                        Ok(sequence) => message.sequence = sequence.get(),
                        Err(FilesystemError::Unsupported {
                            operation: FilesystemOperation::ReserveSeq,
                            ..
                        }) => {
                            txn.rollback().await;
                            return Ok(TransactionalMessageWrite::Unsupported);
                        }
                        Err(error) => {
                            txn.rollback().await;
                            return Err(error.into());
                        }
                    }
                }
            }
            let message_entry = Self::message_entry(message)?;
            if let Err(error) = txn
                .put(&message_virtual_path, message_entry, CasExpectation::Absent)
                .await
            {
                txn.rollback().await;
                return Err(absent_put_error(error, "message", &message_path));
            }

            let (sequence_path, sequence_entry) =
                message_sequence_index_entry_for_message(scope, thread_id, message)?;
            let sequence_virtual_path = self.filesystem.resolve(&resource_scope, &sequence_path)?;
            if let Err(error) = txn
                .put(
                    &sequence_virtual_path,
                    sequence_entry,
                    CasExpectation::Absent,
                )
                .await
            {
                txn.rollback().await;
                return Err(absent_put_error(
                    error,
                    "message sequence index",
                    &sequence_path,
                ));
            }

            match txn.commit().await {
                Ok(()) => return Ok(TransactionalMessageWrite::Written),
                // Optimistic-concurrency conflict on the thread record: another
                // writer committed between our `get` and `commit`. Retry the
                // whole transaction — this is the bounded CAS-retry budget the
                // loop exists to provide. (libSQL/in-memory never reach here;
                // they return `Unsupported` from `begin` above.)
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(error.into()),
            }
        }

        Err(SessionThreadError::Backend(format!(
            "filesystem CAS retries exhausted accepting inbound message at {}",
            thread_path.as_str()
        )))
    }

    async fn list_thread_messages(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<Vec<ThreadMessageRecord>, SessionThreadError> {
        let root = messages_root(scope, thread_id)?;
        let mut messages = Vec::new();
        let mut offset = 0_u64;

        loop {
            let entries = match self
                .filesystem
                .query(
                    &scope.to_resource_scope(),
                    &root,
                    &Filter::All,
                    Page::new(offset, Page::MAX_LIMIT),
                )
                .await
            {
                Ok(entries) => entries,
                Err(FilesystemError::Unsupported {
                    operation: FilesystemOperation::Query,
                    ..
                }) => {
                    return self
                        .list_thread_messages_by_directory(scope, thread_id)
                        .await;
                }
                Err(error) => return Err(error.into()),
            };
            let entry_count = entries.len();

            for versioned in entries {
                if !versioned.path.as_str().ends_with(".json") {
                    continue;
                }
                let record = deserialize::<ThreadMessageRecord>(&versioned.entry.body)?;
                if &record.thread_id == thread_id {
                    messages.push(record);
                }
            }

            if entry_count < Page::MAX_LIMIT as usize {
                break;
            }
            offset = offset.saturating_add(entry_count as u64);
        }

        self.merge_message_append_events(scope, thread_id, &mut messages)
            .await?;
        messages.sort_by_key(|message| message.sequence);
        Ok(messages)
    }

    async fn list_thread_messages_by_directory(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<Vec<ThreadMessageRecord>, SessionThreadError> {
        let root = messages_root(scope, thread_id)?;
        let entries = match self
            .filesystem
            .list_dir(&scope.to_resource_scope(), &root)
            .await
        {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut messages = Vec::new();
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            let child = join_scoped(&root, &entry.name)?;
            let Some(versioned) = self
                .filesystem
                .get(&scope.to_resource_scope(), &child)
                .await?
            else {
                continue;
            };
            let record = deserialize::<ThreadMessageRecord>(&versioned.entry.body)?;
            if &record.thread_id == thread_id {
                messages.push(record);
            }
        }
        self.merge_message_append_events(scope, thread_id, &mut messages)
            .await?;
        messages.sort_by_key(|message| message.sequence);
        Ok(messages)
    }

    async fn find_assistant_message_by_run(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        turn_run_id: &str,
        required_status: Option<MessageStatus>,
    ) -> Result<Option<ThreadMessageRecord>, SessionThreadError> {
        let index_store = MessageLookupIndexStore::new(self.filesystem.as_ref());
        let indexed_message_id = match index_store
            .read_assistant_run(scope, thread_id, turn_run_id)
            .await
        {
            Ok(message_id) => message_id,
            Err(error) => {
                // silent-ok: assistant-run lookup index is acceleration; transcript scan below is authoritative.
                tracing::debug!(
                    ?error,
                    ?scope,
                    thread_id = %thread_id.as_str(),
                    turn_run_id,
                    "assistant lookup index read failed; scanning thread messages",
                );
                None
            }
        };
        if let Some(message_id) = indexed_message_id
            && let Some((message, _)) = self
                .read_message_versioned(scope, thread_id, message_id)
                .await?
            && assistant_message_matches_run(&message, turn_run_id, required_status)
        {
            return Ok(Some(message));
        }

        let found = self
            .list_thread_messages(scope, thread_id)
            .await?
            .into_iter()
            .rev()
            .find(|message| assistant_message_matches_run(message, turn_run_id, required_status));
        if let Some(message) = found.as_ref() {
            self.write_message_lookup_indexes_best_effort(
                scope,
                thread_id,
                message,
                "assistant lookup backfill",
            )
            .await;
        }
        Ok(found)
    }

    async fn find_tool_result_reference_message(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        turn_run_id: &str,
        result_ref: &str,
    ) -> Result<Option<ThreadMessageRecord>, SessionThreadError> {
        let index_store = MessageLookupIndexStore::new(self.filesystem.as_ref());
        let indexed_message_id = match index_store
            .read_tool_result(scope, thread_id, turn_run_id, result_ref)
            .await
        {
            Ok(message_id) => message_id,
            Err(error) => {
                // silent-ok: tool-result lookup index is acceleration; transcript scan below is authoritative.
                tracing::debug!(
                    ?error,
                    ?scope,
                    thread_id = %thread_id.as_str(),
                    turn_run_id,
                    result_ref,
                    "tool-result lookup index read failed; scanning thread messages",
                );
                None
            }
        };
        if let Some(message_id) = indexed_message_id
            && let Some((message, _)) = self
                .read_message_versioned(scope, thread_id, message_id)
                .await?
            && matches_tool_result_reference(&message, turn_run_id, result_ref)
        {
            return Ok(Some(message));
        }

        let found = self
            .list_thread_messages(scope, thread_id)
            .await?
            .into_iter()
            .rev()
            .find(|message| matches_tool_result_reference(message, turn_run_id, result_ref));
        if let Some(message) = found.as_ref() {
            self.write_message_lookup_indexes_best_effort(
                scope,
                thread_id,
                message,
                "tool-result lookup backfill",
            )
            .await;
        }
        Ok(found)
    }

    async fn list_thread_messages_range_indexed(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        after_sequence: u64,
        through_sequence: u64,
    ) -> Result<Option<Vec<ThreadMessageRecord>>, SessionThreadError> {
        let Some(indexes) = MessageSequenceIndexStore::new(self.filesystem.as_ref())
            .read_range(scope, thread_id, after_sequence, through_sequence)
            .await?
        else {
            return Ok(None);
        };
        let mut messages = Vec::with_capacity(indexes.len());
        for chunk in indexes.chunks(INDEXED_RANGE_MESSAGE_READ_CONCURRENCY) {
            let reads = chunk
                .iter()
                .map(|index| self.read_message_versioned(scope, thread_id, index.message_id));
            let results = join_all(reads).await;
            for (index, result) in chunk.iter().zip(results) {
                let Some((message, _)) = result? else {
                    return Err(SessionThreadError::UnknownMessage {
                        message_id: index.message_id,
                    });
                };
                messages.push(message);
            }
        }
        messages.sort_by_key(|message| message.sequence);
        Ok(Some(messages))
    }

    async fn first_user_message_for_title(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        _next_sequence: u64,
    ) -> Result<Option<ThreadMessageRecord>, SessionThreadError> {
        Ok(self
            .list_thread_messages(scope, thread_id)
            .await?
            .into_iter()
            .find(|message| message.kind == MessageKind::User))
    }

    async fn materialize_message_range(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        after_sequence: u64,
        through_sequence: u64,
        fallback_policy: MessageRangeFallbackPolicy,
    ) -> Result<MaterializedMessageRange, SessionThreadError> {
        let thread = self
            .read_thread_versioned(scope, thread_id)
            .await?
            .ok_or_else(|| SessionThreadError::UnknownThread {
                thread_id: thread_id.clone(),
            })?
            .0;
        let messages = match self
            .list_thread_messages_range_indexed(scope, thread_id, after_sequence, through_sequence)
            .await?
        {
            Some(messages) => messages,
            None => match fallback_policy {
                MessageRangeFallbackPolicy::FullScan => self
                    .list_thread_messages(scope, thread_id)
                    .await?
                    .into_iter()
                    .filter(|message| {
                        message.sequence > after_sequence && message.sequence <= through_sequence
                    })
                    .collect(),
            },
        };
        Ok(MaterializedMessageRange { thread, messages })
    }

    async fn find_capability_display_preview_message(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        turn_run_id: &str,
        invocation_id: InvocationId,
    ) -> Result<Option<ThreadMessageRecord>, SessionThreadError> {
        let messages = self.list_thread_messages(scope, thread_id).await?;
        for message in messages {
            if message.kind != MessageKind::CapabilityDisplayPreview
                || message.status != MessageStatus::Finalized
                || message.turn_run_id.as_deref() != Some(turn_run_id)
            {
                continue;
            }
            if CapabilityDisplayPreviewEnvelope::invocation_id_from_json(message.content.as_deref())
                .map_err(SessionThreadError::Serialization)?
                == Some(invocation_id)
            {
                return Ok(Some(message));
            }
        }
        Ok(None)
    }

    async fn list_thread_summaries(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<Vec<SummaryArtifact>, SessionThreadError> {
        let root = summaries_root(scope, thread_id)?;
        let mut summaries = Vec::new();
        let mut offset = 0_u64;

        loop {
            let entries = match self
                .filesystem
                .query(
                    &scope.to_resource_scope(),
                    &root,
                    &Filter::All,
                    Page::new(offset, Page::MAX_LIMIT),
                )
                .await
            {
                Ok(entries) => entries,
                Err(FilesystemError::Unsupported {
                    operation: FilesystemOperation::Query,
                    ..
                }) => {
                    return self
                        .list_thread_summaries_by_directory(scope, thread_id)
                        .await;
                }
                Err(error) => return Err(error.into()),
            };
            let entry_count = entries.len();

            for versioned in entries {
                if !versioned.path.as_str().ends_with(".json") {
                    continue;
                }
                let record = deserialize::<SummaryArtifact>(&versioned.entry.body)?;
                if &record.thread_id == thread_id {
                    summaries.push(record);
                }
            }

            if entry_count < Page::MAX_LIMIT as usize {
                break;
            }
            offset = offset.saturating_add(entry_count as u64);
        }

        summaries.sort_by_key(|summary| {
            (
                summary.start_sequence,
                summary.end_sequence,
                summary.summary_id.to_string(),
            )
        });
        Ok(summaries)
    }

    async fn list_thread_summaries_by_directory(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<Vec<SummaryArtifact>, SessionThreadError> {
        let root = summaries_root(scope, thread_id)?;
        let entries = match self
            .filesystem
            .list_dir(&scope.to_resource_scope(), &root)
            .await
        {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut summaries = Vec::new();
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            let child = join_scoped(&root, &entry.name)?;
            let Some(versioned) = self
                .filesystem
                .get(&scope.to_resource_scope(), &child)
                .await?
            else {
                continue;
            };
            let record = deserialize::<SummaryArtifact>(&versioned.entry.body)?;
            if &record.thread_id == thread_id {
                summaries.push(record);
            }
        }
        summaries.sort_by_key(|summary| {
            (
                summary.start_sequence,
                summary.end_sequence,
                summary.summary_id.to_string(),
            )
        });
        Ok(summaries)
    }

    async fn find_idempotency_record(
        &self,
        match_predicate: impl Fn(&InboundIdempotencyRecord) -> bool,
    ) -> Result<Option<InboundIdempotencyRecord>, SessionThreadError> {
        let root = idempotency_root()?;
        // Idempotency records are scope-keyed at the path level and don't
        // need a per-tenant filesystem rewrite; use the system scope to
        // route through the global idempotency root.
        let scope = ResourceScope::system();
        let entries = match self.filesystem.list_dir(&scope, &root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            let child = join_scoped(&root, &entry.name)?;
            let Some(versioned) = self.filesystem.get(&scope, &child).await? else {
                continue;
            };
            let record = deserialize::<InboundIdempotencyRecord>(&versioned.entry.body)?;
            if match_predicate(&record) {
                return Ok(Some(record));
            }
        }
        Ok(None)
    }

    async fn accepted_message_from_idempotency_path(
        &self,
        scope: &ThreadScope,
        requested_thread_id: &ThreadId,
        requested_actor_id: &str,
        path: &ScopedPath,
    ) -> Result<Option<AcceptedInboundMessage>, SessionThreadError> {
        let Some(versioned) = self
            .filesystem
            .get(&scope.to_resource_scope(), path)
            .await?
        else {
            return Ok(None);
        };
        let record = deserialize::<InboundIdempotencyRecord>(&versioned.entry.body)?;
        self.accepted_message_from_idempotency_record(
            scope,
            requested_thread_id,
            requested_actor_id,
            record,
        )
        .await
        .map(Some)
    }

    async fn accepted_message_from_idempotency_record(
        &self,
        scope: &ThreadScope,
        requested_thread_id: &ThreadId,
        requested_actor_id: &str,
        record: InboundIdempotencyRecord,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        if &record.thread_id != requested_thread_id {
            return Err(SessionThreadError::IdempotentReplayThreadMismatch {
                stored_thread_id: record.thread_id,
                requested_thread_id: requested_thread_id.clone(),
            });
        }
        let (_, _) = self
            .read_thread_versioned(scope, &record.thread_id)
            .await?
            .ok_or_else(|| SessionThreadError::UnknownThread {
                thread_id: record.thread_id.clone(),
            })?;
        let existing = self
            .read_message_versioned(scope, &record.thread_id, record.message_id)
            .await?
            .map(|(message, _)| message)
            .ok_or(SessionThreadError::UnknownMessage {
                message_id: record.message_id,
            })?;
        if existing.actor_id.as_deref() != Some(requested_actor_id) {
            return Err(SessionThreadError::IdempotentReplayActorMismatch {
                stored_actor_id: existing.actor_id.clone().unwrap_or_default(),
                requested_actor_id: requested_actor_id.to_string(),
            });
        }
        Ok(AcceptedInboundMessage {
            thread_id: existing.thread_id,
            message_id: record.message_id,
            sequence: existing.sequence,
            idempotent_replay: true,
        })
    }

    async fn delete_idempotency_records_for_thread(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<(), SessionThreadError> {
        let root = idempotency_root()?;
        let resource_scope = scope.to_resource_scope();
        let entries = match self.filesystem.list_dir(&resource_scope, &root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            let child = join_scoped(&root, &entry.name)?;
            let Some(versioned) = self.filesystem.get(&resource_scope, &child).await? else {
                continue;
            };
            let record = deserialize::<InboundIdempotencyRecord>(&versioned.entry.body)?;
            if record.scope == *scope && record.thread_id == *thread_id {
                match self.filesystem.delete(&resource_scope, &child).await {
                    Ok(()) => {}
                    Err(error) if is_not_found(&error) => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(())
    }

    /// Reserve a per-thread message sequence without rewriting the thread
    /// metadata record. SQL-backed filesystems serve this with an atomic
    /// path-local counter row; older/backends without the sequence primitive
    /// fall back to the legacy `thread.json` CAS counter.
    async fn reserve_sequence(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<u64, SessionThreadError> {
        let (stored, _) = self
            .read_thread_versioned(scope, thread_id)
            .await?
            .ok_or_else(|| SessionThreadError::UnknownThread {
                thread_id: thread_id.clone(),
            })?;
        // Migration safety: a thread that already assigned message sequences
        // under the legacy per-thread-record counter (`next_sequence > 1`) must
        // keep using it. The native path-local counter starts at 1 for a path
        // with no row, so switching an *existing* thread onto it would restart
        // at 1 and collide with messages already at sequences 1..N — corrupting
        // ordering and clobbering the sequence index on instances that predate
        // this change. New/empty threads (`next_sequence == 1`, no messages
        // yet) take the fast native counter; because the native path never
        // rewrites `next_sequence`, such a thread's record stays at 1 and
        // deterministically keeps using the native path for its whole life,
        // while a pre-existing thread stays on the legacy counter for its whole
        // life. No thread ever switches counters mid-stream.
        if stored.next_sequence > 1 {
            return self
                .reserve_sequence_via_thread_record(scope, thread_id)
                .await;
        }
        let sequence_path = message_sequence_counter_path(scope, thread_id)?;
        match self
            .filesystem
            .reserve_sequence(&scope.to_resource_scope(), &sequence_path)
            .await
        {
            Ok(sequence) => return Ok(sequence.get()),
            Err(FilesystemError::Unsupported {
                operation: FilesystemOperation::ReserveSeq,
                ..
            }) => {}
            Err(error) => return Err(error.into()),
        }
        self.reserve_sequence_via_thread_record(scope, thread_id)
            .await
    }

    /// Legacy fallback for backends that cannot atomically reserve a
    /// path-local sequence. This preserves compatibility but retains the old
    /// shared-thread-record CAS bottleneck.
    async fn reserve_sequence_via_thread_record(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<u64, SessionThreadError> {
        let path = thread_record_path(scope, thread_id)?;
        for _ in 0..FILESYSTEM_CAS_RETRIES {
            let (mut stored, version) = self
                .read_thread_versioned(scope, thread_id)
                .await?
                .ok_or_else(|| SessionThreadError::UnknownThread {
                    thread_id: thread_id.clone(),
                })?;
            let assigned = stored.next_sequence;
            stored.next_sequence = assigned + 1;
            // Every appended message is thread activity; bump the
            // last-activity stamp so the sidebar surfaces freshly-used
            // threads first. Reserving a sequence is the single choke
            // point all append paths share.
            stored.record.updated_at = Some(Utc::now());
            let entry = Self::thread_entry(&stored)?;
            match put_with_cas(
                self.filesystem.as_ref(),
                &scope.to_resource_scope(),
                &path,
                entry,
                CasExpectation::Version(version),
            )
            .await
            {
                Ok(()) => return Ok(assigned),
                Err(PutError::VersionMismatch) => continue,
                Err(PutError::Other(error)) => return Err(error),
            }
        }
        Err(SessionThreadError::Backend(format!(
            "filesystem CAS retries exhausted reserving thread sequence at {}",
            path.as_str()
        )))
    }

    /// Stamp `thread.updated_at = now` at a turn boundary (inbound accept,
    /// finalized assistant append) so `list_threads_for_scope` orders by
    /// genuine recency without scanning transcripts. The index row is the
    /// recency authority for listing; avoiding a full `thread.json` CAS here
    /// keeps activity writes row-shaped.
    ///
    /// The message itself is already durably written before most callers reach
    /// this path, so best-effort wrappers log and continue on failure.
    async fn touch_thread_updated_at(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        updated_at: DateTime<Utc>,
    ) -> Result<(), SessionThreadError> {
        self.touch_thread_index_updated_at(scope, thread_id, updated_at)
            .await
    }

    /// Best-effort recency stamp for after-commit call sites. The message is
    /// already durably written when these run, so a touch failure must not
    /// fail the enclosing operation: `accept_inbound_message` permits requests
    /// without an idempotency key, and propagating an error here could make an
    /// un-idempotent caller retry and duplicate the message. Logs and
    /// continues; the advisory `updated_at` stamp simply stays at its prior
    /// value until the next activity.
    async fn touch_thread_updated_at_best_effort_at(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        updated_at: DateTime<Utc>,
    ) {
        if let Err(error) = self
            .touch_thread_updated_at(scope, thread_id, updated_at)
            .await
        {
            // silent-ok: recency stamp is advisory after the message is durable.
            tracing::debug!(
                ?error,
                thread_id = %thread_id.as_str(),
                "message persisted but thread recency touch failed",
            );
        }
    }

    /// Read-modify-write a single message record with optimistic CAS and
    /// bounded retry. The `mutate` closure projects the staged record onto
    /// its new shape.
    async fn apply_message_update<M>(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        mut mutate: M,
    ) -> Result<ThreadMessageRecord, SessionThreadError>
    where
        M: FnMut(&mut ThreadMessageRecord) -> Result<(), SessionThreadError>,
    {
        let path = message_record_path(scope, thread_id, message_id)?;
        for _ in 0..FILESYSTEM_CAS_RETRIES {
            // Read the individual message file first. A finalized message that
            // was written solely to the per-thread append log (see
            // `append_finalized_assistant_message`) has no file yet; in that
            // case materialize the file on first mutation with
            // `CasExpectation::Absent`. Because `merge_message_append_events`
            // is file-authoritative, the materialized (e.g. redacted) file
            // then shadows the original append-log entry on reads. A
            // concurrent materialization races to `Absent` and loses with
            // `VersionMismatch`, so the retry re-reads and takes the
            // file-versioned path.
            let (mut message, cas) = match self
                .filesystem
                .get(&scope.to_resource_scope(), &path)
                .await?
            {
                Some(versioned) => {
                    let record = deserialize::<ThreadMessageRecord>(&versioned.entry.body)?;
                    if &record.thread_id != thread_id || record.message_id != message_id {
                        return Err(SessionThreadError::UnknownMessage { message_id });
                    }
                    (record, CasExpectation::Version(versioned.version))
                }
                None => {
                    let Some(events) = self.read_message_append_events(scope, thread_id).await?
                    else {
                        return Err(SessionThreadError::UnknownMessage { message_id });
                    };
                    let Some((record, _)) = events
                        .into_iter()
                        .find(|(record, _)| record.message_id == message_id)
                    else {
                        return Err(SessionThreadError::UnknownMessage { message_id });
                    };
                    (record, CasExpectation::Absent)
                }
            };
            let before_created_at = message.created_at;
            let before_updated_at = message.updated_at;
            mutate(&mut message)?;
            crate::contract::validate_message_timestamp_fields_not_cleared(
                message.message_id,
                before_created_at,
                before_updated_at,
                message.created_at,
                message.updated_at,
                "filesystem message update",
            )?;
            let entry = Self::message_entry(&message)?;
            match put_with_cas(
                self.filesystem.as_ref(),
                &scope.to_resource_scope(),
                &path,
                entry,
                cas,
            )
            .await
            {
                Ok(()) => {
                    self.write_message_lookup_indexes_best_effort(
                        scope,
                        thread_id,
                        &message,
                        "message update",
                    )
                    .await;
                    self.invalidate_one_shot_context_window(scope, thread_id);
                    return Ok(message);
                }
                Err(PutError::VersionMismatch) => continue,
                Err(PutError::Other(error)) => return Err(error),
            }
        }
        Err(SessionThreadError::Backend(format!(
            "filesystem CAS retries exhausted updating message at {}",
            path.as_str()
        )))
    }

    /// Force-set a persisted message's status to `DeferredBusy` and clear its
    /// turn refs, exactly as the retired `mark_message_deferred_busy` writer
    /// would have. Never call from production code.
    ///
    /// Gated behind `#[cfg(any(test, feature = "test-support"))]` so it is
    /// absent from production builds. Integration tests in a separate
    /// compilation unit must enable the `test-support` feature.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn inject_legacy_deferred_busy_for_test(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.apply_message_update(scope, thread_id, message_id, |message| {
            message.status = MessageStatus::DeferredBusy;
            message.turn_id = None;
            message.turn_run_id = None;
            Ok(())
        })
        .await
    }
}

#[async_trait]
impl<F> SessionThreadService for FilesystemSessionThreadService<F>
where
    F: RootFilesystem,
{
    async fn ensure_thread(
        &self,
        request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        let thread_id = match request.thread_id {
            Some(id) => id,
            None => generated_thread_id()?,
        };
        let path = thread_record_path(&request.scope, &thread_id)?;
        let resource_scope = request.scope.to_resource_scope();
        // Capture request fields so the apply closure can re-run per CAS retry
        // without moving them out.
        let scope = request.scope;
        let created_by_actor_id = request.created_by_actor_id;
        let title = request.title;
        let metadata_json = request.metadata_json;
        let thread_id_clone = thread_id.clone();
        let (record, created) = cas_update(
            self.filesystem.as_ref(),
            &resource_scope,
            &path,
            |bytes: &[u8]| deserialize::<StoredThreadRecord>(bytes),
            |stored: &StoredThreadRecord| Self::thread_entry(stored),
            |current: Option<StoredThreadRecord>| {
                // Clone all request fields for this retry iteration.
                let scope = scope.clone();
                let created_by_actor_id = created_by_actor_id.clone();
                let title = title.clone();
                let metadata_json = metadata_json.clone();
                let thread_id = thread_id_clone.clone();
                let outcome: Result<
                    CasApply<StoredThreadRecord, (SessionThreadRecord, bool)>,
                    SessionThreadError,
                > = match current {
                    Some(existing) => {
                        // Thread already exists: scope- and identity-check before
                        // returning it (no write). Mirrors the guard in
                        // `read_thread_versioned` which rejects both a scope
                        // mismatch and a thread_id mismatch — defensive parity
                        // even though the path already encodes thread_id.
                        if existing.record.scope != scope || existing.record.thread_id != thread_id
                        {
                            Err(SessionThreadError::ThreadScopeMismatch { thread_id })
                        } else {
                            // Unchanged snapshot → cas_update skips the write.
                            Ok(CasApply::new(existing.clone(), (existing.record, false)))
                        }
                    }
                    None => {
                        // First writer: build a fresh record and let cas_update
                        // persist it with CasExpectation::Absent. A concurrent
                        // winner causes VersionMismatch → the helper re-reads and
                        // re-runs apply, which will then see Some(existing) above
                        // and take the scope-reconcile path.
                        let now = Utc::now();
                        let record = SessionThreadRecord {
                            scope,
                            thread_id,
                            created_by_actor_id,
                            title,
                            metadata_json,
                            goal: None,
                            created_at: Some(now),
                            updated_at: Some(now),
                        };
                        let stored = StoredThreadRecord {
                            record: record.clone(),
                            next_sequence: 1,
                        };
                        Ok(CasApply::new(stored, (record, true)))
                    }
                };
                async move { outcome }
            },
        )
        .await
        .map_err(map_cas_error)?;
        if created || !self.is_thread_index_known(&record.scope, &record.thread_id) {
            self.refresh_thread_index_from_source(&record.scope, &record.thread_id)
                .await?;
        }
        Ok(record)
    }

    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        let AcceptInboundMessageRequest {
            scope,
            thread_id,
            actor_id,
            source_binding_id,
            reply_target_binding_id,
            external_event_id,
            content,
        } = request;
        let idempotency_key = match (&source_binding_id, &external_event_id) {
            (Some(source_binding_id), Some(external_event_id)) => Some(InboundIdempotencyKey {
                scope: scope.clone(),
                source_binding_id: source_binding_id.clone(),
                external_event_id: external_event_id.clone(),
            }),
            _ => None,
        };
        let idempotency_path = idempotency_key
            .as_ref()
            .map(|key| {
                let record_key = idempotency_record_key(key)?;
                idempotency_record_path(&record_key)
            })
            .transpose()?;

        // First, check idempotency. The on-disk key SHA-256s the full
        // (scope, source_binding_id, external_event_id) tuple, so a
        // same-binding/event from a different scope hashes to a different
        // key (and we only see records under the current MountView).
        if let Some(path) = &idempotency_path
            && let Some(accepted) = self
                .accepted_message_from_idempotency_path(&scope, &thread_id, &actor_id, path)
                .await?
        {
            return Ok(accepted);
        }

        let message_id = ThreadMessageId::new();
        let (content_text, attachments) = content.into_parts();
        crate::contract::validate_attachment_refs(&attachments)?;
        // Sequence assignment happens only after payload validation. On
        // transactional backends the thread counter, message, sequence index,
        // and idempotency record commit together; fallback backends reserve
        // immediately before the legacy message write.
        let now = Utc::now();
        let mut message = ThreadMessageRecord {
            message_id,
            thread_id: thread_id.clone(),
            sequence: 0,
            kind: MessageKind::User,
            status: MessageStatus::Accepted,
            created_at: Some(now),
            updated_at: Some(now),
            actor_id: Some(actor_id.clone()),
            source_binding_id: source_binding_id.clone(),
            reply_target_binding_id: reply_target_binding_id.clone(),
            turn_id: None,
            turn_run_id: None,
            tool_result_ref: None,
            tool_result_provider_call: None,
            content: Some(content_text),
            attachments,
            redaction_ref: None,
        };
        let idempotency_write =
            if let (Some(idempotency_key), Some(path)) = (&idempotency_key, &idempotency_path) {
                let idem_record = InboundIdempotencyRecord {
                    scope: idempotency_key.scope.clone(),
                    source_binding_id: idempotency_key.source_binding_id.clone(),
                    external_event_id: idempotency_key.external_event_id.clone(),
                    thread_id: thread_id.clone(),
                    message_id,
                };
                let entry = Self::idempotency_entry(&idem_record)?;
                Some((path.clone(), entry))
            } else {
                None
            };

        let sequence = match self
            .try_write_new_message_transactionally(
                &scope,
                &thread_id,
                &mut message,
                idempotency_write
                    .as_ref()
                    .map(|(path, entry)| (path, entry)),
            )
            .await?
        {
            TransactionalMessageWrite::Written => message.sequence,
            TransactionalMessageWrite::IdempotencyAlreadyAccepted => {
                let path = idempotency_path.as_ref().ok_or_else(|| {
                    SessionThreadError::Backend(
                        "transaction reported idempotency conflict without an idempotency path"
                            .to_string(),
                    )
                })?;
                let accepted = self
                    .accepted_message_from_idempotency_path(&scope, &thread_id, &actor_id, path)
                    .await?
                    .ok_or_else(|| {
                        SessionThreadError::Backend(format!(
                            "filesystem transaction rejected duplicate inbound idempotency at {} but record is missing",
                            path.as_str()
                        ))
                })?;
                return Ok(accepted);
            }
            TransactionalMessageWrite::Unsupported => {
                let sequence = self.reserve_sequence(&scope, &thread_id).await?;
                message.sequence = sequence;
                self.write_new_message(&scope, &thread_id, &message, "message")
                    .await?;

                // Non-transactional backends keep the legacy best-effort
                // shape: the message is authoritative, and the idempotency
                // record accelerates later replays when it can be written.
                if let Some((path, entry)) = idempotency_write {
                    self.filesystem
                        .put(
                            &scope.to_resource_scope(),
                            &path,
                            entry,
                            CasExpectation::Any,
                        )
                        .await?;
                }
                sequence
            }
        };

        if sequence == 1 {
            self.seed_one_shot_context_window(&scope, &thread_id, &message);
        } else {
            self.invalidate_one_shot_context_window(&scope, &thread_id);
        }

        // Inbound user message is thread activity — stamp recency so the
        // sidebar surfaces this thread first. Best-effort: the message is
        // already durable, so a touch failure must not fail (and retry) the
        // accept.
        self.touch_thread_updated_at_best_effort_at(&scope, &thread_id, now)
            .await;

        Ok(AcceptedInboundMessage {
            thread_id,
            message_id,
            sequence,
            idempotent_replay: false,
        })
    }

    async fn replay_accepted_inbound_message(
        &self,
        request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
        let Some(record) = self
            .find_idempotency_record(|candidate| {
                candidate.source_binding_id == request.source_binding_id
                    && candidate.external_event_id == request.external_event_id
            })
            .await?
        else {
            return Ok(None);
        };
        let Some((_, _)) = self
            .read_thread_versioned(&record.scope, &record.thread_id)
            .await?
        else {
            return Err(SessionThreadError::UnknownThread {
                thread_id: record.thread_id,
            });
        };
        let message = self
            .read_message_versioned(&record.scope, &record.thread_id, record.message_id)
            .await?
            .map(|(message, _)| message)
            .ok_or(SessionThreadError::UnknownMessage {
                message_id: record.message_id,
            })?;
        Ok(Some(AcceptedInboundMessageReplay {
            scope: record.scope,
            thread_id: record.thread_id,
            message_id: record.message_id,
            sequence: message.sequence,
            status: message.status,
            actor_id: message.actor_id,
            source_binding_id: message.source_binding_id,
            reply_target_binding_id: message.reply_target_binding_id,
            turn_run_id: message.turn_run_id,
        }))
    }

    async fn mark_message_submitted(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        turn_id: String,
        turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        // Confirm the thread is in this scope before opening the message
        // record. `read_thread_versioned` returns `None` on scope mismatch,
        // which we surface as `UnknownThread` to match the in-memory shape.
        self.read_thread_versioned(scope, thread_id)
            .await?
            .ok_or_else(|| SessionThreadError::UnknownThread {
                thread_id: thread_id.clone(),
            })?;
        let updated = self
            .apply_message_update(scope, thread_id, message_id, |message| {
                ensure_user_accepted(message, "mark_message_submitted")?;
                message.status = MessageStatus::Submitted;
                message.turn_id = Some(turn_id.clone());
                message.turn_run_id = Some(turn_run_id.clone());
                Ok(())
            })
            .await?;
        if updated.sequence == 1 {
            self.seed_one_shot_context_window(scope, thread_id, &updated);
        }
        Ok(updated)
    }

    async fn mark_message_rejected_busy(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.read_thread_versioned(scope, thread_id)
            .await?
            .ok_or_else(|| SessionThreadError::UnknownThread {
                thread_id: thread_id.clone(),
            })?;
        self.apply_message_update(scope, thread_id, message_id, |message| {
            ensure_user_accepted(message, "mark_message_rejected_busy")?;
            message.status = MessageStatus::RejectedBusy;
            message.turn_id = None;
            message.turn_run_id = None;
            Ok(())
        })
        .await
    }

    async fn append_assistant_draft(
        &self,
        request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        // Dedup-by-turn-run-id: read the secondary index first and fall back
        // to the legacy scan for rows written before the index existed.
        // Retrying a draft append with the same `turn_run_id` returns the
        // existing record rather than creating a sibling.
        if let Some(existing) = self
            .find_assistant_message_by_run(
                &request.scope,
                &request.thread_id,
                &request.turn_run_id,
                None,
            )
            .await?
        {
            return Ok(existing);
        }
        let sequence = self
            .reserve_sequence(&request.scope, &request.thread_id)
            .await?;
        let now = Utc::now();
        let message = ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
            thread_id: request.thread_id.clone(),
            sequence,
            kind: MessageKind::Assistant,
            status: MessageStatus::Draft,
            created_at: Some(now),
            updated_at: Some(now),
            actor_id: None,
            source_binding_id: None,
            reply_target_binding_id: None,
            turn_id: None,
            turn_run_id: Some(request.turn_run_id),
            tool_result_ref: None,
            tool_result_provider_call: None,
            content: Some(request.content.into_text()),
            attachments: Vec::new(),
            redaction_ref: None,
        };
        self.write_new_message(
            &request.scope,
            &request.thread_id,
            &message,
            "assistant draft",
        )
        .await?;
        Ok(message)
    }

    async fn append_finalized_assistant_message(
        &self,
        request: AppendFinalizedAssistantMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        if let Some(existing) = self
            .find_assistant_message_by_run(
                &request.scope,
                &request.thread_id,
                &request.turn_run_id,
                None,
            )
            .await?
        {
            if existing.status != MessageStatus::Draft {
                // Idempotent-retry repair. A prior call may have appended the
                // finalized message event (durable) but failed before writing
                // the sequence index. On retry we resolve the finalized message
                // through the append-log fallback and would otherwise return
                // without the index, leaving a durable LLM message invisible to
                // indexed range/context reads. Re-assert the index before
                // returning — `write_new` is idempotent when the same message
                // is already indexed, so this is a no-op on the fully-persisted
                // path and a repair on the partial-failure path. Fail loud (`?`)
                // rather than silently returning an unindexed message.
                self.write_message_sequence_index(&request.scope, &request.thread_id, &existing)
                    .await?;
                return Ok(existing);
            }
            let content = request.content.clone();
            let now = Utc::now();
            let finalized = self
                .apply_message_update(
                    &request.scope,
                    &request.thread_id,
                    existing.message_id,
                    |message| {
                        ensure_draft(message)?;
                        message.status = MessageStatus::Finalized;
                        message.content = Some(content.clone().into_text());
                        message.attachments = Vec::new();
                        message.updated_at = Some(now);
                        Ok(())
                    },
                )
                .await?;
            // Finalizing the in-flight draft is thread activity — stamp recency
            // (best-effort; the draft update above is already durable).
            self.touch_thread_updated_at_best_effort_at(&request.scope, &request.thread_id, now)
                .await;
            return Ok(finalized);
        }
        let sequence = self
            .reserve_sequence(&request.scope, &request.thread_id)
            .await?;
        let now = Utc::now();
        let message = ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
            thread_id: request.thread_id.clone(),
            sequence,
            kind: MessageKind::Assistant,
            status: MessageStatus::Finalized,
            created_at: Some(now),
            updated_at: Some(now),
            actor_id: None,
            source_binding_id: None,
            reply_target_binding_id: None,
            turn_id: None,
            turn_run_id: Some(request.turn_run_id),
            tool_result_ref: None,
            tool_result_provider_call: None,
            content: Some(request.content.into_text()),
            attachments: Vec::new(),
            redaction_ref: None,
        };
        if self
            .append_message_event(&request.scope, &request.thread_id, &message)
            .await?
        {
            // Append-only path: the log event is durable, but unlike
            // `write_new_message` it wrote neither the per-message file nor
            // the sequence index. Write the sequence index so indexed range
            // reads (`list_thread_messages_range` and the summary/context
            // paths built on it) surface this finalized message — full-history
            // and context reads already see it via `merge_message_append_events`,
            // but the index-backed range path would otherwise omit it. The id
            // resolves through `read_message_versioned`'s append-log fallback
            // until a later mutation materializes the file. Treated as
            // must-write (matching `write_new_message`), not best-effort,
            // because a missing index entry silently drops the message from
            // range reads.
            self.write_message_sequence_index(&request.scope, &request.thread_id, &message)
                .await?;
        } else {
            self.write_new_message(
                &request.scope,
                &request.thread_id,
                &message,
                "finalized assistant message",
            )
            .await?;
        }
        // Finalized assistant reply is thread activity — stamp recency
        // (best-effort; the append above is already durable).
        self.touch_thread_updated_at_best_effort_at(&request.scope, &request.thread_id, now)
            .await;
        Ok(message)
    }

    async fn append_tool_result_reference(
        &self,
        request: AppendToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let provider_call = request.provider_call;
        let envelope = ToolResultReferenceEnvelope::new_best_effort_model_observation(
            request.result_ref,
            request.safe_summary,
            request.model_observation,
        )
        .map_err(SessionThreadError::Serialization)?;
        if let Some(existing) = self
            .find_tool_result_reference_message(
                &request.scope,
                &request.thread_id,
                &request.turn_run_id,
                &envelope.result_ref,
            )
            .await?
        {
            // Idempotent replay. If new provider metadata arrives, validate
            // and attach it (or reject on conflict) — matching the in-memory
            // contract semantics.
            let provider_call_update = if let Some(provider_call) = provider_call.as_ref() {
                provider_call
                    .validate()
                    .map_err(SessionThreadError::Serialization)?;
                match existing.tool_result_provider_call.as_ref() {
                    Some(existing_call) if existing_call == provider_call => None,
                    Some(_) => {
                        return Err(SessionThreadError::Serialization(
                            "tool result provider metadata conflicts with existing record"
                                .to_string(),
                        ));
                    }
                    None => Some(provider_call.clone()),
                }
            } else {
                None
            };
            let model_observation = envelope.model_observation.clone();
            if provider_call_update.is_some() || model_observation.is_some() {
                let now = Utc::now();
                let updated = self
                    .apply_message_update(
                        &request.scope,
                        &request.thread_id,
                        existing.message_id,
                        |message| {
                            let mut changed = false;
                            if let Some(provider_call) = provider_call_update.as_ref() {
                                message.tool_result_provider_call = Some(provider_call.clone());
                                changed = true;
                            }
                            if let Some(model_observation) = model_observation.as_ref() {
                                let content = message.content.as_deref().ok_or_else(|| {
                                    SessionThreadError::Serialization(
                                        "tool result reference content is missing".to_string(),
                                    )
                                })?;
                                if let Some(content) = ToolResultReferenceEnvelope::merge_model_observation_content_if_absent(
                                    content,
                                    model_observation.clone(),
                                )
                                .map_err(SessionThreadError::Serialization)?
                                {
                                    message.content = Some(content);
                                    changed = true;
                                }
                            }
                            if changed {
                                message.updated_at = Some(now);
                            }
                            Ok(())
                        },
                    )
                    .await?;
                if updated != existing {
                    self.touch_thread_updated_at_best_effort_at(
                        &request.scope,
                        &request.thread_id,
                        now,
                    )
                    .await;
                }
                return Ok(updated);
            }
            return Ok(existing);
        }
        if let Some(provider_call) = &provider_call {
            provider_call
                .validate()
                .map_err(SessionThreadError::Serialization)?;
        }
        let content = serde_json::to_string(&envelope)
            .map_err(|error| SessionThreadError::Serialization(error.to_string()))?;
        let sequence = self
            .reserve_sequence(&request.scope, &request.thread_id)
            .await?;
        let now = Utc::now();
        let message = ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
            thread_id: request.thread_id.clone(),
            sequence,
            kind: MessageKind::ToolResultReference,
            status: MessageStatus::Finalized,
            created_at: Some(now),
            updated_at: Some(now),
            actor_id: None,
            source_binding_id: None,
            reply_target_binding_id: None,
            turn_id: None,
            turn_run_id: Some(request.turn_run_id),
            tool_result_ref: Some(envelope.result_ref),
            tool_result_provider_call: provider_call,
            content: Some(content),
            attachments: Vec::new(),
            redaction_ref: None,
        };
        self.write_new_message(
            &request.scope,
            &request.thread_id,
            &message,
            "tool result reference",
        )
        .await?;
        Ok(message)
    }

    async fn append_capability_display_preview(
        &self,
        request: AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        request
            .preview
            .validate()
            .map_err(SessionThreadError::Serialization)?;
        let existing = self
            .find_capability_display_preview_message(
                &request.scope,
                &request.thread_id,
                &request.turn_run_id,
                request.preview.invocation_id,
            )
            .await?;
        if let Some(existing) = existing {
            return Ok(existing);
        }
        let message_id = capability_display_preview_message_id(
            &request.scope,
            &request.thread_id,
            &request.turn_run_id,
            request.preview.invocation_id,
        )?;
        let content = serde_json::to_string(&request.preview)
            .map_err(|error| SessionThreadError::Serialization(error.to_string()))?;
        let sequence = self
            .reserve_sequence(&request.scope, &request.thread_id)
            .await?;
        let now = Utc::now();
        let message = ThreadMessageRecord {
            message_id,
            thread_id: request.thread_id.clone(),
            sequence,
            kind: MessageKind::CapabilityDisplayPreview,
            status: MessageStatus::Finalized,
            created_at: Some(now),
            updated_at: Some(now),
            actor_id: None,
            source_binding_id: None,
            reply_target_binding_id: None,
            turn_id: None,
            turn_run_id: Some(request.turn_run_id),
            tool_result_ref: request.preview.result_ref.clone(),
            tool_result_provider_call: None,
            content: Some(content),
            attachments: Vec::new(),
            redaction_ref: None,
        };
        let path = message_record_path(&request.scope, &request.thread_id, message.message_id)?;
        crate::contract::validate_new_message_timestamps(&message, "capability display preview")?;
        let entry = Self::message_entry(&message)?;
        match put_with_cas(
            self.filesystem.as_ref(),
            &request.scope.to_resource_scope(),
            &path,
            entry,
            CasExpectation::Absent,
        )
        .await
        {
            Ok(()) => {
                self.write_message_sequence_index(&request.scope, &request.thread_id, &message)
                    .await?;
                Ok(message)
            }
            Err(PutError::VersionMismatch) => self
                .read_message_versioned(&request.scope, &request.thread_id, message_id)
                .await?
                .map(|(existing, _)| existing)
                .ok_or_else(|| {
                    SessionThreadError::Backend(format!(
                        "filesystem CAS Absent rejected new capability display preview at {} but no existing message could be read",
                        path.as_str()
                    ))
                }),
            Err(PutError::Other(error)) => Err(error),
        }
    }

    async fn update_tool_result_reference(
        &self,
        request: UpdateToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let message = self
            .find_tool_result_reference_message(
                &request.scope,
                &request.thread_id,
                &request.turn_run_id,
                &request.result_ref,
            )
            .await?
            .ok_or_else(|| {
                SessionThreadError::Backend(format!(
                    "tool result reference {} was not found in thread {}",
                    request.result_ref, request.thread_id
                ))
            })?;
        // Re-validate inside the CAS closure: on retry the pre-scan record is
        // stale, so a concurrent writer that flipped status, changed
        // turn_run_id, or rewrote tool_result_ref between the scan and our
        // retry must not be silently overwritten. The closure refuses the
        // mutation in that case and surfaces the same "not found" error as
        // the pre-scan path.
        let turn_run_id = request.turn_run_id.clone();
        let result_ref = request.result_ref.clone();
        let thread_id_for_error = request.thread_id.clone();
        let safe_summary = request.safe_summary;
        let now = Utc::now();
        let updated = self
            .apply_message_update(
            &request.scope,
            &request.thread_id,
            message.message_id,
            |message| {
                if !matches_tool_result_reference(message, &turn_run_id, &result_ref) {
                    return Err(SessionThreadError::Backend(format!(
                        "tool result reference {result_ref} was not found in thread {thread_id_for_error}",
                    )));
                }
                let content = message.content.as_deref().ok_or_else(|| {
                    SessionThreadError::Serialization(
                        "tool result reference content is missing".to_string(),
                    )
                })?;
                let envelope = ToolResultReferenceEnvelope::from_json_str(content)
                    .map_err(SessionThreadError::Serialization)?
                    .with_safe_summary(safe_summary.clone());
                let content = serde_json::to_string(&envelope)
                    .map_err(|error| SessionThreadError::Serialization(error.to_string()))?;
                message.content = Some(content.clone());
                message.updated_at = Some(now);
                Ok(())
            },
        )
        .await?;
        self.touch_thread_updated_at_best_effort_at(&request.scope, &request.thread_id, now)
            .await;
        Ok(updated)
    }

    async fn update_assistant_draft(
        &self,
        request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.read_thread_versioned(&request.scope, &request.thread_id)
            .await?
            .ok_or_else(|| SessionThreadError::UnknownThread {
                thread_id: request.thread_id.clone(),
            })?;
        let now = Utc::now();
        let updated = self
            .apply_message_update(
                &request.scope,
                &request.thread_id,
                request.message_id,
                |message| {
                    ensure_draft(message)?;
                    message.content = Some(request.content.clone().into_text());
                    // Keep content and attachments in lockstep (as redaction does):
                    // a content update must not leave stale attachment refs behind.
                    message.attachments = Vec::new();
                    message.updated_at = Some(now);
                    Ok(())
                },
            )
            .await?;
        self.touch_thread_updated_at_best_effort_at(&request.scope, &request.thread_id, now)
            .await;
        Ok(updated)
    }

    async fn finalize_assistant_message(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.read_thread_versioned(scope, thread_id)
            .await?
            .ok_or_else(|| SessionThreadError::UnknownThread {
                thread_id: thread_id.clone(),
            })?;
        let now = Utc::now();
        let finalized = self
            .apply_message_update(scope, thread_id, message_id, |message| {
                ensure_draft(message)?;
                message.status = MessageStatus::Finalized;
                message.content = Some(content.clone().into_text());
                message.attachments = Vec::new();
                message.updated_at = Some(now);
                Ok(())
            })
            .await?;
        // Finalizing the assistant draft is thread activity — stamp recency
        // (best-effort; the finalize above is already durable). Without this,
        // the draft/update/finalize path would leave active threads stale in
        // the `updated_at`-sorted sidebar.
        self.touch_thread_updated_at_best_effort_at(scope, thread_id, now)
            .await;
        Ok(finalized)
    }

    async fn redact_message(
        &self,
        request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.read_thread_versioned(&request.scope, &request.thread_id)
            .await?
            .ok_or_else(|| SessionThreadError::UnknownThread {
                thread_id: request.thread_id.clone(),
            })?;
        let now = Utc::now();
        let updated = self
            .apply_message_update(
                &request.scope,
                &request.thread_id,
                request.message_id,
                |message| {
                    message.status = MessageStatus::Redacted;
                    message.content = None;
                    message.attachments = Vec::new();
                    message.tool_result_provider_call = None;
                    message.redaction_ref = Some(request.redaction_ref.clone());
                    message.updated_at = Some(now);
                    Ok(())
                },
            )
            .await?;
        self.touch_thread_updated_at_best_effort_at(&request.scope, &request.thread_id, now)
            .await;
        Ok(updated)
    }

    async fn load_context_window(
        &self,
        request: LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        if let Some(context) = self.take_one_shot_context_window(
            &request.scope,
            &request.thread_id,
            request.max_messages,
        ) {
            return Ok(context);
        }

        self.read_thread_versioned(&request.scope, &request.thread_id)
            .await?
            .ok_or_else(|| SessionThreadError::UnknownThread {
                thread_id: request.thread_id.clone(),
            })?;
        let messages = self
            .list_thread_messages(&request.scope, &request.thread_id)
            .await?;
        let summaries = self
            .list_thread_summaries(&request.scope, &request.thread_id)
            .await?;
        let mut context = context_messages_with_summary_replacements(&messages, &summaries);
        if request.max_messages < context.len() {
            let start = context.len() - request.max_messages;
            context = context.split_off(start);
        }
        Ok(ContextWindow {
            thread_id: request.thread_id,
            messages: context,
        })
    }

    async fn load_context_messages(
        &self,
        request: LoadContextMessagesRequest,
    ) -> Result<ContextMessages, SessionThreadError> {
        self.read_thread_versioned(&request.scope, &request.thread_id)
            .await?
            .ok_or_else(|| SessionThreadError::UnknownThread {
                thread_id: request.thread_id.clone(),
            })?;
        let messages = self
            .list_thread_messages(&request.scope, &request.thread_id)
            .await?;
        Ok(ContextMessages {
            thread_id: request.thread_id,
            messages: context_messages_by_id(&messages, &request.message_ids),
        })
    }

    async fn list_thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        let thread = self
            .read_thread_versioned(&request.scope, &request.thread_id)
            .await?
            .ok_or_else(|| SessionThreadError::UnknownThread {
                thread_id: request.thread_id.clone(),
            })?
            .0;
        let messages = self
            .list_thread_messages(&request.scope, &request.thread_id)
            .await?;
        let summaries = self
            .list_thread_summaries(&request.scope, &request.thread_id)
            .await?;
        let thread_record = self.thread_record_with_index_overlay(thread).await?;
        Ok(ThreadHistory {
            thread: thread_record,
            summary_artifacts: history_summary_artifacts(&messages, summaries),
            messages: history_messages(&messages),
        })
    }

    async fn list_thread_messages_range(
        &self,
        request: ThreadMessageRangeRequest,
    ) -> Result<ThreadMessageRange, SessionThreadError> {
        let range = self
            .materialize_message_range(
                &request.scope,
                &request.thread_id,
                request.after_sequence,
                request.through_sequence,
                MessageRangeFallbackPolicy::FullScan,
            )
            .await?;
        Ok(ThreadMessageRange {
            thread: range.thread.record,
            messages: range.messages.iter().map(history_message).collect(),
        })
    }

    async fn latest_thread_message(
        &self,
        request: LatestThreadMessageRequest,
    ) -> Result<Option<ThreadMessageRecord>, SessionThreadError> {
        self.read_thread_versioned(&request.scope, &request.thread_id)
            .await?
            .ok_or_else(|| SessionThreadError::UnknownThread {
                thread_id: request.thread_id.clone(),
            })?;
        let Some(message) = self
            .list_thread_messages(&request.scope, &request.thread_id)
            .await?
            .into_iter()
            .rev()
            .find(|message| message.kind == request.kind && message.status == request.status)
        else {
            return Ok(None);
        };
        Ok(Some(history_message(&message)))
    }

    async fn finalized_assistant_message_by_run(
        &self,
        request: crate::FinalizedAssistantMessageByRunRequest,
    ) -> Result<Option<ThreadMessageRecord>, SessionThreadError> {
        self.read_thread_versioned(&request.scope, &request.thread_id)
            .await?
            .ok_or_else(|| SessionThreadError::UnknownThread {
                thread_id: request.thread_id.clone(),
            })?;
        let Some(message) = self
            .find_assistant_message_by_run(
                &request.scope,
                &request.thread_id,
                &request.turn_run_id,
                Some(MessageStatus::Finalized),
            )
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(history_message(&message)))
    }

    async fn read_thread(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        let thread = self
            .read_thread_versioned(&request.scope, &request.thread_id)
            .await?
            .ok_or_else(|| SessionThreadError::UnknownThread {
                thread_id: request.thread_id.clone(),
            })?
            .0;
        self.thread_record_with_index_overlay(thread).await
    }

    async fn delete_thread(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<(), SessionThreadError> {
        // read_thread/read_thread_versioned enforce exact-scope ownership and
        // preserve the same UnknownThread shape for absent or cross-scope rows.
        match self
            .read_thread(ThreadHistoryRequest {
                scope: scope.clone(),
                thread_id: thread_id.clone(),
            })
            .await
        {
            Ok(_) => {}
            Err(SessionThreadError::UnknownThread { .. }) => {
                self.delete_thread_index_record(scope, thread_id).await?;
                return Err(SessionThreadError::UnknownThread {
                    thread_id: thread_id.clone(),
                });
            }
            Err(error) => return Err(error),
        }
        self.delete_idempotency_records_for_thread(scope, thread_id)
            .await?;
        match self
            .filesystem
            .delete(&scope.to_resource_scope(), &thread_root(scope, thread_id)?)
            .await
        {
            Ok(()) => {
                self.invalidate_one_shot_context_window(scope, thread_id);
                self.delete_thread_index_record(scope, thread_id).await
            }
            Err(error) if is_not_found(&error) => Err(SessionThreadError::UnknownThread {
                thread_id: thread_id.clone(),
            }),
            Err(error) => Err(error.into()),
        }
    }

    async fn create_summary_artifact(
        &self,
        request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError> {
        if request.start_sequence == 0 || request.start_sequence > request.end_sequence {
            return Err(SessionThreadError::InvalidSummaryRange {
                start_sequence: request.start_sequence,
                end_sequence: request.end_sequence,
            });
        }
        let range_messages = self
            .materialize_message_range(
                &request.scope,
                &request.thread_id,
                request.start_sequence.saturating_sub(1),
                request.end_sequence,
                MessageRangeFallbackPolicy::FullScan,
            )
            .await?;
        if !range_messages
            .messages
            .iter()
            .any(|message| message.sequence == request.start_sequence)
            || !range_messages
                .messages
                .iter()
                .any(|message| message.sequence == request.end_sequence)
        {
            return Err(SessionThreadError::InvalidSummaryRange {
                start_sequence: request.start_sequence,
                end_sequence: request.end_sequence,
            });
        }
        let existing_summaries = self
            .list_thread_summaries(&request.scope, &request.thread_id)
            .await?;
        let content = request.content.as_text().to_string();
        if let Some(overlapping) =
            find_overlapping_summary(&existing_summaries, &request, &content)?
        {
            return Ok(overlapping.clone());
        }
        let artifact = SummaryArtifact {
            summary_id: SummaryArtifactId::new(),
            thread_id: request.thread_id.clone(),
            start_sequence: request.start_sequence,
            end_sequence: request.end_sequence,
            summary_kind: request.summary_kind,
            content,
            model_context_policy: request.model_context_policy,
        };
        let path = summary_record_path(&request.scope, &request.thread_id, artifact.summary_id)?;
        let entry = Self::summary_entry(&artifact)?;
        match put_with_cas(
            self.filesystem.as_ref(),
            &request.scope.to_resource_scope(),
            &path,
            entry,
            CasExpectation::Absent,
        )
        .await
        {
            Ok(()) => {
                self.invalidate_one_shot_context_window(&request.scope, &request.thread_id);
                Ok(artifact)
            }
            Err(PutError::VersionMismatch) => Err(SessionThreadError::Backend(format!(
                "filesystem CAS Absent rejected new summary artifact at {}",
                path.as_str()
            ))),
            Err(PutError::Other(error)) => Err(error),
        }
    }

    async fn list_threads_for_scope(
        &self,
        request: ListThreadsForScopeRequest,
    ) -> Result<ListThreadsForScopeResponse, SessionThreadError> {
        let limit = request
            .limit
            .map(|n| (n as usize).clamp(1, LIST_THREADS_MAX_PAGE_SIZE))
            .unwrap_or(LIST_THREADS_DEFAULT_PAGE_SIZE);
        let listed = self.cached_thread_index_for_scope(&request.scope).await?;
        // Opaque cursor is the last thread_id of the previous page; find
        // it in the freshly-sorted list and resume after it. A cursor
        // that no longer resolves (thread deleted between pages) ends the
        // stream rather than restarting from the top.
        let start_index = match request.cursor.as_deref() {
            Some(cursor) => self.thread_index_start_after_cursor(&request.scope, cursor, || {
                listed
                    .iter()
                    .position(|index| index.record.thread_id.as_str() == cursor)
                    .map(|index| index + 1)
                    .unwrap_or(listed.len())
            }),
            None => 0,
        };
        let end_index = start_index.saturating_add(limit).min(listed.len());
        let has_more = end_index < listed.len();
        let mut page: Vec<SessionThreadRecord> = Vec::with_capacity(end_index - start_index);
        // Records whose `title` is `None` need a sidebar-friendly label
        // derived from their first user message. We collect their page
        // indices here and fan-out the indexed first-user reads below so
        // we don't serialize N transcript probes inline.
        let mut needs_title: Vec<(usize, ThreadId, u64)> = Vec::new();
        for index in &listed[start_index..end_index] {
            let idx = page.len();
            let record = index.record.clone();
            if record.title.is_none() {
                needs_title.push((idx, record.thread_id.clone(), index.next_sequence));
            }
            page.push(record);
        }
        // Derive titles in parallel from each thread's first user
        // message. v1's libSQL list path did the same thing in SQL
        // (`SELECT substr(content, 1, 100) FROM conversation_messages
        // WHERE role='user' ORDER BY created_at LIMIT 1`); Reborn's
        // filesystem layout reads via `RootFilesystem` instead. Errors
        // are silent-ok — the sidebar entry simply falls back to its
        // thread-id label, matching the WebUI fallback path.
        if !needs_title.is_empty() {
            let title_results: Vec<(
                usize,
                ThreadId,
                Result<Option<ThreadMessageRecord>, SessionThreadError>,
            )> = futures::stream::iter(needs_title)
                .map(|(idx, thread_id, next_sequence)| {
                    let scope = request.scope.clone();
                    async move {
                        let result = self
                            .first_user_message_for_title(&scope, &thread_id, next_sequence)
                            .await;
                        (idx, thread_id, result)
                    }
                })
                .buffer_unordered(TITLE_DERIVATION_READ_CONCURRENCY)
                .collect()
                .await;
            for (idx, thread_id, msg_result) in title_results {
                match msg_result {
                    Ok(first_user) => {
                        if let Some(title) = first_user
                            .as_ref()
                            .and_then(|message| message.content.as_deref())
                            .and_then(derive_title_from_message)
                        {
                            page[idx].title = Some(title);
                        }
                    }
                    Err(error) => {
                        // Internal diagnostic — `debug!`, not `warn!`, to keep
                        // the REPL/TUI display intact (project logging rule).
                        tracing::debug!(
                            thread_id = %thread_id.as_str(),
                            scope = ?request.scope,
                            ?error,
                            "skipping thread-title derivation during list_threads_for_scope",
                        );
                    }
                }
            }
        }
        // The cursor is the last thread_id on this page; the next
        // request resumes after it in the activity-sorted order. Only
        // emit one when more records remain beyond this slice.
        let next_cursor = if has_more {
            page.last()
                .map(|record| record.thread_id.as_str().to_string())
        } else {
            None
        };
        Ok(ListThreadsForScopeResponse {
            threads: page,
            next_cursor,
        })
    }
}

const LIST_THREADS_DEFAULT_PAGE_SIZE: usize = 50;
const LIST_THREADS_MAX_PAGE_SIZE: usize = 200;
/// Bounded fan-out used only when a scope has legacy source threads without
/// derived index rows. Normal indexed listing should not read source bodies.
const LIST_THREADS_MISSING_INDEX_READ_CONCURRENCY: usize = 16;
// ── Idempotency key shape ──────────────────────────────────────
//
// Mirrors the legacy `DurableState` key shape so on-disk hashes are
// byte-stable. Two callers with the same `(scope, source_binding_id,
// external_event_id)` tuple compute identical record keys; mismatched
// scopes hash to different keys, which is why a flat
// `/threads/idempotency/<sha256>.json` directory is safe.

#[derive(Debug, Clone, Serialize)]
struct InboundIdempotencyKey {
    scope: ThreadScope,
    source_binding_id: String,
    external_event_id: String,
}

fn idempotency_record_key(key: &InboundIdempotencyKey) -> Result<String, SessionThreadError> {
    let payload = serialize_pretty(key)?;
    let digest = Sha256::digest(&payload);
    let mut output = String::with_capacity("sha256-".len() + digest.len() * 2);
    output.push_str("sha256-");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}")
            .map_err(|error| SessionThreadError::Serialization(error.to_string()))?;
    }
    Ok(output)
}

// ── Paths ──────────────────────────────────────────────────────
//
// Every path is alias-relative under the `/threads` mount alias. The
// leading tenant/user prefix that the legacy implementation hand-formatted
// into the path is gone: the MountView's
// `/threads → /tenants/<tenant>/users/<user>/threads` grant supplies it
// at every op. Within-tenant axes (agent/project/owner_user/mission)
// remain in the alias-relative path because they are within-tenant scoping
// not covered by the per-tenant `MountAlias`.

const THREADS_PREFIX: &str = "/threads";

fn thread_record_path(
    scope: &ThreadScope,
    thread_id: &ThreadId,
) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!(
        "{}/thread.json",
        thread_root_string(scope, thread_id)
    ))
}

fn thread_root(
    scope: &ThreadScope,
    thread_id: &ThreadId,
) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&thread_root_string(scope, thread_id))
}

fn messages_root(
    scope: &ThreadScope,
    thread_id: &ThreadId,
) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!(
        "{}/messages",
        thread_root_string(scope, thread_id)
    ))
}

fn message_record_path(
    scope: &ThreadScope,
    thread_id: &ThreadId,
    message_id: ThreadMessageId,
) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!(
        "{}/messages/{message_id}.json",
        thread_root_string(scope, thread_id)
    ))
}

fn message_sequence_counter_path(
    scope: &ThreadScope,
    thread_id: &ThreadId,
) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!(
        "{}/message_sequence",
        thread_root_string(scope, thread_id)
    ))
}

fn message_append_log_path(
    scope: &ThreadScope,
    thread_id: &ThreadId,
) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!(
        "{}/message_appends",
        thread_root_string(scope, thread_id)
    ))
}

fn summaries_root(
    scope: &ThreadScope,
    thread_id: &ThreadId,
) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!(
        "{}/summaries",
        thread_root_string(scope, thread_id)
    ))
}

fn summary_record_path(
    scope: &ThreadScope,
    thread_id: &ThreadId,
    summary_id: SummaryArtifactId,
) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!(
        "{}/summaries/{summary_id}.json",
        thread_root_string(scope, thread_id)
    ))
}

fn idempotency_root() -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!("{}/idempotency", THREADS_PREFIX))
}

fn idempotency_record_path(record_key: &str) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!("{}/idempotency/{record_key}.json", THREADS_PREFIX))
}

/// Build the alias-relative per-thread root for a scope under `/threads`.
fn thread_root_string(scope: &ThreadScope, thread_id: &ThreadId) -> String {
    let mut base = scope_axes_string(scope);
    base.push_str("/threads/");
    base.push_str(thread_id.as_str());
    base
}

fn one_shot_context_window_cache_key(scope: &ThreadScope, thread_id: &ThreadId) -> String {
    format!(
        "{}:{}",
        scope.tenant_id.as_str(),
        thread_root_string(scope, thread_id)
    )
}

fn evict_hash_map_entry_over_limit<T>(
    map: &mut HashMap<String, T>,
    max_entries: usize,
    keep: &str,
) {
    if map.len() <= max_entries {
        return;
    }
    let mut keys = map.keys();
    let victim = match keys.next() {
        Some(first) if first.as_str() == keep => keys.next().cloned(),
        Some(first) => Some(first.clone()),
        None => None,
    };
    if let Some(victim) = victim {
        map.remove(&victim);
    }
}

/// Within-tenant sub-scope axes encoded into the path. Tenant + user
/// identity lives in the caller's MountView and is intentionally absent.
fn scope_axes_string(scope: &ThreadScope) -> String {
    let mut base = String::from(THREADS_PREFIX);
    base.push_str("/agents/");
    base.push_str(scope.agent_id.as_str());
    if let Some(project_id) = &scope.project_id {
        base.push_str("/projects/");
        base.push_str(project_id.as_str());
    }
    if let Some(owner_user_id) = &scope.owner_user_id {
        base.push_str("/owners/");
        base.push_str(owner_user_id.as_str());
    }
    if let Some(mission_id) = &scope.mission_id {
        base.push_str("/missions/");
        base.push_str(mission_id.as_str());
    }
    base
}

fn scoped_path(raw: &str) -> Result<ScopedPath, SessionThreadError> {
    ScopedPath::new(raw).map_err(invalid_path)
}

/// Join a leaf segment onto a [`ScopedPath`] prefix. Mirrors the
/// run-state / processes / secrets stores' `join_scoped` helper:
/// `list_dir` returns post-resolution [`VirtualPath`]s, but the follow-up
/// `get` must run through the `ScopedFilesystem` so the per-op ACL is
/// enforced — so callers strip the leaf name and rejoin it onto the
/// original `ScopedPath` prefix.
fn join_scoped(prefix: &ScopedPath, leaf: &str) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!(
        "{}/{}",
        prefix.as_str().trim_end_matches('/'),
        leaf
    ))
}

fn generated_thread_id() -> Result<ThreadId, SessionThreadError> {
    ThreadId::new(uuid::Uuid::new_v4().to_string())
        .map_err(|error| SessionThreadError::GeneratedThreadId(error.to_string()))
}

fn invalid_path(error: HostApiError) -> SessionThreadError {
    SessionThreadError::Backend(format!("invalid storage path: {error}"))
}

fn serialize_pretty<T>(value: &T) -> Result<Vec<u8>, SessionThreadError>
where
    T: Serialize,
{
    serde_json::to_vec_pretty(value)
        .map_err(|error| SessionThreadError::Serialization(error.to_string()))
}

fn deserialize<T>(bytes: &[u8]) -> Result<T, SessionThreadError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_slice(bytes)
        .map_err(|error| SessionThreadError::Deserialization(error.to_string()))
}

fn is_not_found(error: &FilesystemError) -> bool {
    matches!(error, FilesystemError::NotFound { .. })
}

// ── Transcript helpers (shared semantics) ──────────────────────
//
// Both the in-memory and filesystem stores compute the same model-visible
// context window and history-summary projection. These helpers are pure
// functions over message/summary lists so the two stores stay in sync.

fn ensure_draft(message: &ThreadMessageRecord) -> Result<(), SessionThreadError> {
    if message.kind != MessageKind::Assistant || message.status != MessageStatus::Draft {
        return Err(SessionThreadError::MessageNotDraft {
            message_id: message.message_id,
        });
    }
    Ok(())
}

fn ensure_user_accepted(
    message: &ThreadMessageRecord,
    attempted: &'static str,
) -> Result<(), SessionThreadError> {
    if message.kind == MessageKind::User
        && matches!(
            message.status,
            MessageStatus::Accepted | MessageStatus::DeferredBusy
        )
    {
        return Ok(());
    }
    Err(SessionThreadError::InvalidMessageTransition {
        message_id: message.message_id,
        from: message.status,
        attempted,
    })
}

fn is_model_visible(status: MessageStatus) -> bool {
    matches!(
        status,
        MessageStatus::Accepted | MessageStatus::Submitted | MessageStatus::Finalized
    )
}

fn is_model_context_visible(message: &ThreadMessageRecord) -> bool {
    is_model_visible(message.status) && message.kind != MessageKind::CapabilityDisplayPreview
}

fn capability_display_preview_message_id(
    scope: &ThreadScope,
    thread_id: &ThreadId,
    turn_run_id: &str,
    invocation_id: InvocationId,
) -> Result<ThreadMessageId, SessionThreadError> {
    #[derive(Serialize)]
    struct PreviewMessageKey<'a> {
        scope: &'a ThreadScope,
        thread_id: &'a ThreadId,
        turn_run_id: &'a str,
        invocation_id: InvocationId,
    }
    let key = serde_json::to_vec(&PreviewMessageKey {
        scope,
        thread_id,
        turn_run_id,
        invocation_id,
    })
    .map_err(|error| SessionThreadError::Serialization(error.to_string()))?;
    let digest = Sha256::digest(&key);
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(ThreadMessageId::from_uuid(Uuid::from_bytes(bytes)))
}

fn matches_tool_result_reference(
    message: &ThreadMessageRecord,
    turn_run_id: &str,
    result_ref: &str,
) -> bool {
    message.kind == MessageKind::ToolResultReference
        && message.status == MessageStatus::Finalized
        && message.turn_run_id.as_deref() == Some(turn_run_id)
        && message.tool_result_ref.as_deref() == Some(result_ref)
}

fn assistant_message_matches_run(
    message: &ThreadMessageRecord,
    turn_run_id: &str,
    required_status: Option<MessageStatus>,
) -> bool {
    message.kind == MessageKind::Assistant
        && message.turn_run_id.as_deref() == Some(turn_run_id)
        && required_status.is_none_or(|status| message.status == status)
}

const REDACTED_SUMMARY_CONTENT: &str = "[redacted]";

fn context_messages_with_summary_replacements(
    messages: &[ThreadMessageRecord],
    summaries: &[SummaryArtifact],
) -> Vec<ContextMessage> {
    let replacement_summaries = summaries
        .iter()
        .filter(|summary| {
            summary.model_context_policy
                == Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected)
                && !summary_covers_hidden_content(messages, summary)
        })
        .collect::<Vec<_>>();
    let mut skip_through = 0u64;
    let mut emitted_summaries: std::collections::HashSet<_> = std::collections::HashSet::new();
    let mut context = Vec::new();
    for message in messages
        .iter()
        .filter(|message| is_model_context_visible(message))
    {
        if message.sequence <= skip_through {
            continue;
        }
        if let Some(summary) = replacement_summaries.iter().find(|summary| {
            summary.start_sequence <= message.sequence
                && message.sequence <= summary.end_sequence
                && !emitted_summaries.contains(&summary.summary_id)
        }) {
            context.push(ContextMessage {
                message_id: None,
                summary_id: Some(summary.summary_id),
                sequence: summary.start_sequence,
                kind: MessageKind::Summary,
                tool_result_provider_call: None,
                content: summary.content.clone(),
                image_attachments: Vec::new(),
            });
            emitted_summaries.insert(summary.summary_id);
            skip_through = summary.end_sequence;
            continue;
        }
        if let Some(content) = message.content.clone() {
            context.push(ContextMessage::from_transcript_message(message, content));
        }
    }
    context
}

fn context_messages_by_id(
    messages: &[ThreadMessageRecord],
    message_ids: &[ThreadMessageId],
) -> Vec<ContextMessage> {
    let visible_messages: std::collections::HashMap<_, _> = messages
        .iter()
        .filter(|message| is_model_context_visible(message))
        .map(|message| (message.message_id, message))
        .collect();
    message_ids
        .iter()
        .filter_map(|message_id| {
            let message = visible_messages.get(message_id)?;
            let content = message.content.clone()?;
            Some(ContextMessage::from_transcript_message(message, content))
        })
        .collect()
}

fn history_messages(messages: &[ThreadMessageRecord]) -> Vec<ThreadMessageRecord> {
    messages.iter().map(history_message).collect()
}

// Deny-by-default projection: every field is listed deliberately so a newly
// added sensitive field does NOT auto-flow into persisted history. Do not
// collapse to `..message.clone()` — `tool_result_provider_call` is dropped
// here precisely because raw runtime/tool payloads must never surface as
// ordinary transcript content (see crate guardrails).
fn history_message(message: &ThreadMessageRecord) -> ThreadMessageRecord {
    ThreadMessageRecord {
        message_id: message.message_id,
        thread_id: message.thread_id.clone(),
        sequence: message.sequence,
        kind: message.kind,
        status: message.status,
        actor_id: message.actor_id.clone(),
        source_binding_id: message.source_binding_id.clone(),
        reply_target_binding_id: message.reply_target_binding_id.clone(),
        turn_id: message.turn_id.clone(),
        turn_run_id: message.turn_run_id.clone(),
        created_at: message.created_at,
        updated_at: message.updated_at,
        tool_result_ref: message.tool_result_ref.clone(),
        tool_result_provider_call: None,
        content: message.content.clone(),
        attachments: message.attachments.clone(),
        redaction_ref: message.redaction_ref.clone(),
    }
}

fn history_summary_artifacts(
    messages: &[ThreadMessageRecord],
    summaries: Vec<SummaryArtifact>,
) -> Vec<SummaryArtifact> {
    summaries
        .into_iter()
        .map(|summary| {
            if summary_covers_redacted_or_deleted_content(messages, &summary) {
                let mut redacted = summary;
                redacted.content = REDACTED_SUMMARY_CONTENT.to_string();
                redacted.model_context_policy = None;
                redacted
            } else {
                summary
            }
        })
        .collect()
}

/// Returns true when a non-model-context-visible message within the summary
/// span could later become model-visible (i.e. it is in a resurfaceable pending
/// state).  Permanently-terminal non-visible messages (RejectedBusy, capability
/// previews) never resurface, so a compaction summary spanning them is safe to
/// apply — blocking it would silently drop a legitimate compacted range.
///
/// Resurfaceable statuses (must still block the summary):
///   Draft | Interrupted | Superseded | DeferredBusy
/// Permanent non-visible (must NOT block):
///   RejectedBusy (terminal, user must explicitly resend)
///   CapabilityDisplayPreview kind (never model-visible regardless of status)
///
/// Note: Redacted/Deleted keep their blocking role here — they were never
/// model-visible and the separate `summary_covers_redacted_or_deleted_content`
/// guard (used for history display) doesn't cover the context-build path.
fn can_resurface_as_model_visible(message: &ThreadMessageRecord) -> bool {
    matches!(
        message.status,
        MessageStatus::Draft
            | MessageStatus::Interrupted
            | MessageStatus::Superseded
            | MessageStatus::DeferredBusy
    )
}

fn summary_covers_hidden_content(
    messages: &[ThreadMessageRecord],
    summary: &SummaryArtifact,
) -> bool {
    messages.iter().any(|message| {
        summary.start_sequence <= message.sequence
            && message.sequence <= summary.end_sequence
            && !is_model_context_visible(message)
            && (can_resurface_as_model_visible(message)
                || matches!(
                    message.status,
                    MessageStatus::Redacted | MessageStatus::Deleted
                ))
    })
}

fn summary_covers_redacted_or_deleted_content(
    messages: &[ThreadMessageRecord],
    summary: &SummaryArtifact,
) -> bool {
    messages.iter().any(|message| {
        summary.start_sequence <= message.sequence
            && message.sequence <= summary.end_sequence
            && matches!(
                message.status,
                MessageStatus::Redacted | MessageStatus::Deleted
            )
    })
}

// ── CAS-aware put with `Unsupported`→`Any` fallback ────────────
//
// Local, lock-free CAS-retry loop that predates the shared
// `ironclaw_filesystem::cas_update` helper (`write_new_message`,
// `reserve_sequence_via_thread_record` — the legacy fallback for
// backends without native sequence reservation; `reserve_sequence`
// itself is now row-native — `apply_message_update`,
// `append_capability_display_preview`, `create_summary_artifact`, and
// the message-sequence/message-lookup index writers). On CAS-capable
// production backends it always issues
// `put(_, _, CasExpectation::Version)` and retries on
// `FilesystemError::VersionMismatch` — that's correct and matches
// `cas_update`'s contract. The `Unsupported` → `CasExpectation::Any`
// fallback below only triggers on byte-only backends (e.g.
// `LocalFilesystem`), which production does not mount for these
// stores; it is not protected by any lock map. Migrating these
// single-record RMWs onto `cas_update` (fail-closed on a non-CAS
// backend) is a tracked, deferred follow-up sibling to the
// `ironclaw_turns` runner-lease migration (#5274) — see
// `docs/plans/2026-06-25-cas-migration.md`.

/// Local error classification for the CAS-aware put helper.
enum PutError {
    /// Backend reported `VersionMismatch` (cross-process raced us). The
    /// caller retries by re-reading the current record.
    VersionMismatch,
    /// Any other backend or serialization failure; surface to caller.
    Other(SessionThreadError),
}

fn absent_put_error(
    error: FilesystemError,
    description: &'static str,
    path: &ScopedPath,
) -> SessionThreadError {
    match error {
        FilesystemError::VersionMismatch { .. } => SessionThreadError::Backend(format!(
            "filesystem CAS Absent rejected new {description} at {}",
            path.as_str()
        )),
        error => error.into(),
    }
}

async fn put_with_cas<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    path: &ScopedPath,
    entry: Entry,
    cas: CasExpectation,
) -> Result<(), PutError>
where
    F: RootFilesystem,
{
    let fallback_entry = entry.clone();
    match filesystem.put(scope, path, entry, cas).await {
        Ok(_) => Ok(()),
        Err(FilesystemError::VersionMismatch { .. }) => Err(PutError::VersionMismatch),
        Err(FilesystemError::Unsupported {
            operation: FilesystemOperation::WriteFile,
            ..
        }) => {
            if matches!(cas, CasExpectation::Absent) {
                let existing = filesystem
                    .get(scope, path)
                    .await
                    .map_err(|error| PutError::Other(error.into()))?;
                if existing.is_some() {
                    return Err(PutError::VersionMismatch);
                }
            }
            filesystem
                .put(scope, path, fallback_entry, CasExpectation::Any)
                .await
                .map(|_| ())
                .map_err(|error| PutError::Other(error.into()))
        }
        Err(error) => Err(PutError::Other(error.into())),
    }
}

/// Map the shared CAS helper's [`CasUpdateError`] into a
/// [`SessionThreadError`].
///
/// [`CasUpdateError::Apply`] carries the caller's own error straight through;
/// all other variants are storage-layer failures. Fail-closed: a backend that
/// cannot honor versioned CAS surfaces as a [`SessionThreadError::Backend`]
/// rather than a silent blind overwrite.
fn map_cas_error(error: CasUpdateError<SessionThreadError>) -> SessionThreadError {
    match error {
        CasUpdateError::Apply(inner) => inner,
        CasUpdateError::Timeout | CasUpdateError::RetriesExhausted => {
            SessionThreadError::Backend("filesystem CAS retries exhausted".to_string())
        }
        CasUpdateError::CasUnsupported => SessionThreadError::Backend(
            "backend does not support versioned compare-and-swap".to_string(),
        ),
        CasUpdateError::Backend(fs_err) => SessionThreadError::Backend(fs_err.to_string()),
    }
}

impl From<FilesystemError> for SessionThreadError {
    fn from(error: FilesystemError) -> Self {
        Self::Backend(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};

    use super::message_sequence_index::{sequence_from_index_filename, sequence_index_filename};
    use super::{InboundIdempotencyKey, idempotency_record_key};
    use crate::ThreadScope;

    #[test]
    fn sequence_index_filenames_sort_by_sequence() {
        let names = [10, 2, 1]
            .into_iter()
            .map(sequence_index_filename)
            .collect::<Vec<_>>();
        let mut sorted = names.clone();
        sorted.sort();

        assert_eq!(
            sorted
                .iter()
                .filter_map(|name| sequence_from_index_filename(name))
                .collect::<Vec<_>>(),
            vec![1, 2, 10]
        );
        assert_eq!(sequence_from_index_filename("not-a-sequence.json"), None);
    }

    #[test]
    fn idempotency_record_key_is_fixed_size_for_long_external_ids() {
        let key = InboundIdempotencyKey {
            scope: ThreadScope {
                tenant_id: TenantId::new("tenant-a").unwrap(),
                agent_id: AgentId::new("agent-a").unwrap(),
                project_id: Some(ProjectId::new("project-a").unwrap()),
                owner_user_id: Some(UserId::new("user-a").unwrap()),
                mission_id: None,
            },
            source_binding_id: "web-client".into(),
            external_event_id: format!("event-{}", "x".repeat(10_000)),
        };

        let record_key = idempotency_record_key(&key).unwrap();

        assert!(record_key.starts_with("sha256-"));
        assert_eq!(record_key.len(), "sha256-".len() + 64);
    }

    /// Migration safety: a thread that already assigned message sequences under
    /// the legacy per-thread-record counter (`next_sequence > 1`) must keep
    /// resuming from it, never restart at 1 on the native path-local counter —
    /// otherwise deploying this change onto an existing instance would collide
    /// new messages with the existing 1..N sequences. New/empty threads
    /// (`next_sequence == 1`) take the native counter.
    #[tokio::test]
    async fn reserve_sequence_resumes_existing_thread_counter_not_native_restart() {
        use ironclaw_filesystem::{CasExpectation, InMemoryBackend, ScopedFilesystem};
        use ironclaw_host_api::{
            MountAlias, MountGrant, MountPermissions, MountView, ThreadId, VirtualPath,
        };

        use super::{FilesystemSessionThreadService, thread_record_path};
        use crate::{EnsureThreadRequest, SessionThreadService};

        let backend = std::sync::Arc::new(InMemoryBackend::new());
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/threads").unwrap(),
            VirtualPath::new("/tenants/t/users/u/threads").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap();
        let scoped = std::sync::Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts));
        let service = FilesystemSessionThreadService::new(scoped);
        let scope = ThreadScope {
            tenant_id: TenantId::new("t").unwrap(),
            agent_id: AgentId::new("a").unwrap(),
            project_id: Some(ProjectId::new("p").unwrap()),
            owner_user_id: Some(UserId::new("u").unwrap()),
            mission_id: None,
        };

        // Fresh thread (next_sequence == 1) → native path-local counter from 1.
        let fresh = ThreadId::new("fresh").unwrap();
        service
            .ensure_thread(EnsureThreadRequest {
                scope: scope.clone(),
                thread_id: Some(fresh.clone()),
                created_by_actor_id: "actor".into(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        assert_eq!(service.reserve_sequence(&scope, &fresh).await.unwrap(), 1);
        assert_eq!(service.reserve_sequence(&scope, &fresh).await.unwrap(), 2);

        // Simulate a pre-existing thread: bump its on-disk `next_sequence` to 5
        // (as the legacy per-record counter would have, for a thread with
        // messages at sequences 1..4) while leaving the native counter absent.
        let existing = ThreadId::new("existing").unwrap();
        service
            .ensure_thread(EnsureThreadRequest {
                scope: scope.clone(),
                thread_id: Some(existing.clone()),
                created_by_actor_id: "actor".into(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        let (mut stored, version) = service
            .read_thread_versioned(&scope, &existing)
            .await
            .unwrap()
            .unwrap();
        stored.next_sequence = 5;
        let record_path = thread_record_path(&scope, &existing).unwrap();
        service
            .filesystem
            .put(
                &scope.to_resource_scope(),
                &record_path,
                FilesystemSessionThreadService::<InMemoryBackend>::thread_entry(&stored).unwrap(),
                CasExpectation::Version(version),
            )
            .await
            .unwrap();

        // Reservation resumes the legacy counter at 5, not the native restart 1.
        assert_eq!(
            service.reserve_sequence(&scope, &existing).await.unwrap(),
            5
        );
        assert_eq!(
            service.reserve_sequence(&scope, &existing).await.unwrap(),
            6
        );
    }
}
