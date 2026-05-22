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

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
};

use async_trait::async_trait;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, FilesystemOperation, RecordVersion,
    RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{HostApiError, ResourceScope, ScopedPath, ThreadId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::identifiers::SummaryArtifactId;
use crate::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
    AppendAssistantDraftRequest, AppendToolResultReferenceRequest, ContextMessage, ContextMessages,
    ContextWindow, CreateSummaryArtifactRequest, EnsureThreadRequest, LoadContextMessagesRequest,
    LoadContextWindowRequest, MessageContent, MessageKind, MessageStatus,
    ProviderToolCallReferenceEnvelope, RedactMessageRequest, ReplayAcceptedInboundMessageRequest,
    SessionThreadError, SessionThreadRecord, SessionThreadService, SummaryArtifact, ThreadHistory,
    ThreadHistoryRequest, ThreadMessageId, ThreadMessageRecord, ThreadScope,
    ToolResultReferenceEnvelope, UpdateAssistantDraftRequest, UpdateToolResultReferenceRequest,
};

/// Bound on the CAS retry loop. Mirrors the run-state / authorization
/// store budgets — enough to absorb routine cross-process contention,
/// small enough to surface pathological loops loudly.
const FILESYSTEM_CAS_RETRIES: usize = 8;

/// On-disk thread state record. The transcript boundary's
/// [`SessionThreadRecord`] is the user-visible shape; this struct adds
/// `next_sequence` so the per-thread monotonic counter is durable.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

impl<F> FilesystemSessionThreadService<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    fn thread_entry(record: &StoredThreadRecord) -> Result<Entry, SessionThreadError> {
        let body = serialize_pretty(record)?;
        Ok(Entry::bytes(body).with_content_type(ContentType::json()))
    }

    fn message_entry(record: &ThreadMessageRecord) -> Result<Entry, SessionThreadError> {
        let body = serialize_pretty(&StoredThreadMessageRecord::from(record))?;
        Ok(Entry::bytes(body).with_content_type(ContentType::json()))
    }

    fn summary_entry(record: &SummaryArtifact) -> Result<Entry, SessionThreadError> {
        let body = serialize_pretty(record)?;
        Ok(Entry::bytes(body).with_content_type(ContentType::json()))
    }

    fn idempotency_entry(record: &InboundIdempotencyRecord) -> Result<Entry, SessionThreadError> {
        let body = serialize_pretty(record)?;
        Ok(Entry::bytes(body).with_content_type(ContentType::json()))
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
            return Ok(None);
        };
        let record = deserialize::<ThreadMessageRecord>(&versioned.entry.body)?;
        if &record.thread_id != thread_id || record.message_id != message_id {
            return Ok(None);
        }
        Ok(Some((record, versioned.version)))
    }

    async fn list_thread_messages(
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
            // `list_dir` returns post-resolution `VirtualPath`s; reconstruct
            // the alias-relative `ScopedPath` so the follow-up `get` still
            // runs through the per-op ACL (mirrors the run-state /
            // processes store `join_scoped` shape).
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
        messages.sort_by_key(|message| message.sequence);
        Ok(messages)
    }

    async fn list_thread_summaries(
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

    /// Read-modify-write the `next_sequence` counter on the thread record
    /// with optimistic CAS and bounded retry. Returns the sequence
    /// assigned to the caller (i.e. `next_sequence` before the bump) plus
    /// a clone of the post-bump record.
    async fn reserve_sequence(
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
            let (mut message, version) = self
                .read_message_versioned(scope, thread_id, message_id)
                .await?
                .ok_or(SessionThreadError::UnknownMessage { message_id })?;
            mutate(&mut message)?;
            let entry = Self::message_entry(&message)?;
            match put_with_cas(
                self.filesystem.as_ref(),
                &scope.to_resource_scope(),
                &path,
                entry,
                CasExpectation::Version(version),
            )
            .await
            {
                Ok(()) => return Ok(message),
                Err(PutError::VersionMismatch) => continue,
                Err(PutError::Other(error)) => return Err(error),
            }
        }
        Err(SessionThreadError::Backend(format!(
            "filesystem CAS retries exhausted updating message at {}",
            path.as_str()
        )))
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
        let record_lock = filesystem_record_lock(&path);
        let _guard = record_lock.lock().await;
        if let Some((existing, _)) = self
            .read_thread_versioned(&request.scope, &thread_id)
            .await?
        {
            if existing.record.scope != request.scope {
                return Err(SessionThreadError::ThreadScopeMismatch { thread_id });
            }
            return Ok(existing.record);
        }
        // Cross-scope collision: a thread with this id may exist under a
        // sibling scope. Check by re-reading the path (which is scope-keyed
        // here, so a sibling scope's record lives at a different path),
        // then surface as `ThreadScopeMismatch` once we discover one. The
        // path-keyed read above only catches same-scope existence; sibling
        // existence is racy across an outer caller. For now we rely on the
        // path uniqueness — a sibling scope cannot create the same path.
        let record = SessionThreadRecord {
            scope: request.scope,
            thread_id: thread_id.clone(),
            created_by_actor_id: request.created_by_actor_id,
            title: request.title,
            metadata_json: request.metadata_json,
        };
        let stored = StoredThreadRecord {
            record: record.clone(),
            next_sequence: 1,
        };
        let entry = Self::thread_entry(&stored)?;
        let resource_scope = record.scope.to_resource_scope();
        match put_with_cas(
            self.filesystem.as_ref(),
            &resource_scope,
            &path,
            entry,
            CasExpectation::Absent,
        )
        .await
        {
            Ok(()) => Ok(record),
            Err(PutError::VersionMismatch) => {
                // Someone else won the race; re-read and reconcile against
                // the requested scope.
                let (existing, _) = self
                    .read_thread_versioned(&record.scope, &thread_id)
                    .await?
                    .ok_or_else(|| {
                        SessionThreadError::Backend(format!(
                            "filesystem CAS Absent rejected ensure_thread at {} but record is missing",
                            path.as_str()
                        ))
                    })?;
                if existing.record.scope != record.scope {
                    return Err(SessionThreadError::ThreadScopeMismatch { thread_id });
                }
                Ok(existing.record)
            }
            Err(PutError::Other(error)) => Err(error),
        }
    }

    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        // First, check idempotency. The on-disk key SHA-256s the full
        // (scope, source_binding_id, external_event_id) tuple, so a
        // same-binding/event from a different scope hashes to a different
        // key (and we only see records under the current MountView).
        if let Some(idempotency_key) = InboundIdempotencyKey::from_request(&request) {
            let record_key = idempotency_record_key(&idempotency_key)?;
            let path = idempotency_record_path(&record_key)?;
            if let Some(versioned) = self
                .filesystem
                .get(&request.scope.to_resource_scope(), &path)
                .await?
            {
                let record = deserialize::<InboundIdempotencyRecord>(&versioned.entry.body)?;
                if record.thread_id != request.thread_id {
                    return Err(SessionThreadError::IdempotentReplayThreadMismatch {
                        stored_thread_id: record.thread_id,
                        requested_thread_id: request.thread_id,
                    });
                }
                let (_, _) = self
                    .read_thread_versioned(&request.scope, &record.thread_id)
                    .await?
                    .ok_or_else(|| SessionThreadError::UnknownThread {
                        thread_id: record.thread_id.clone(),
                    })?;
                let existing = self
                    .read_message_versioned(&request.scope, &record.thread_id, record.message_id)
                    .await?
                    .map(|(message, _)| message)
                    .ok_or(SessionThreadError::UnknownMessage {
                        message_id: record.message_id,
                    })?;
                if existing.actor_id.as_deref() != Some(request.actor_id.as_str()) {
                    return Err(SessionThreadError::IdempotentReplayActorMismatch {
                        stored_actor_id: existing.actor_id.clone().unwrap_or_default(),
                        requested_actor_id: request.actor_id,
                    });
                }
                return Ok(AcceptedInboundMessage {
                    thread_id: existing.thread_id,
                    message_id: record.message_id,
                    sequence: existing.sequence,
                    idempotent_replay: true,
                });
            }
        }

        let sequence = self
            .reserve_sequence(&request.scope, &request.thread_id)
            .await?;
        let message_id = ThreadMessageId::new();
        let message = ThreadMessageRecord {
            message_id,
            thread_id: request.thread_id.clone(),
            sequence,
            kind: MessageKind::User,
            status: MessageStatus::Accepted,
            actor_id: Some(request.actor_id.clone()),
            source_binding_id: request.source_binding_id.clone(),
            reply_target_binding_id: request.reply_target_binding_id.clone(),
            turn_id: None,
            turn_run_id: None,
            tool_result_ref: None,
            tool_result_provider_call: None,
            content: Some(request.content.clone().into_text()),
            redaction_ref: None,
        };
        let message_path = message_record_path(&request.scope, &request.thread_id, message_id)?;
        let entry = Self::message_entry(&message)?;
        match put_with_cas(
            self.filesystem.as_ref(),
            &request.scope.to_resource_scope(),
            &message_path,
            entry,
            CasExpectation::Absent,
        )
        .await
        {
            Ok(()) => {}
            Err(PutError::VersionMismatch) => {
                return Err(SessionThreadError::Backend(format!(
                    "filesystem CAS Absent rejected new message at {}",
                    message_path.as_str()
                )));
            }
            Err(PutError::Other(error)) => return Err(error),
        }

        if let Some(idempotency_key) = InboundIdempotencyKey::from_request(&request) {
            let idem_record = InboundIdempotencyRecord {
                scope: idempotency_key.scope.clone(),
                source_binding_id: idempotency_key.source_binding_id.clone(),
                external_event_id: idempotency_key.external_event_id.clone(),
                thread_id: request.thread_id.clone(),
                message_id,
            };
            let record_key = idempotency_record_key(&idempotency_key)?;
            let path = idempotency_record_path(&record_key)?;
            let entry = Self::idempotency_entry(&idem_record)?;
            // `Any` here: the SHA-256 key already encodes the full tuple,
            // so a duplicate write simply overwrites with the same
            // (binding, event, thread, message) — equivalent to the
            // legacy in-memory HashMap upsert.
            self.filesystem
                .put(
                    &request.scope.to_resource_scope(),
                    &path,
                    entry,
                    CasExpectation::Any,
                )
                .await?;
        }

        Ok(AcceptedInboundMessage {
            thread_id: request.thread_id,
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
        self.apply_message_update(scope, thread_id, message_id, |message| {
            ensure_user_accepted(message, "mark_message_submitted")?;
            message.status = MessageStatus::Submitted;
            message.turn_id = Some(turn_id.clone());
            message.turn_run_id = Some(turn_run_id.clone());
            Ok(())
        })
        .await
    }

    async fn mark_message_deferred_busy(
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
            ensure_user_accepted(message, "mark_message_deferred_busy")?;
            message.status = MessageStatus::DeferredBusy;
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
        // Dedup-by-turn-run-id: scan existing messages and return the
        // matching draft if one is already present. This matches the
        // legacy in-memory semantics where retrying a draft append with
        // the same `turn_run_id` returned the existing record rather than
        // creating a sibling.
        let existing = self
            .list_thread_messages(&request.scope, &request.thread_id)
            .await?;
        if let Some(existing) = existing.into_iter().find(|message| {
            message.kind == MessageKind::Assistant
                && message.turn_run_id.as_deref() == Some(request.turn_run_id.as_str())
        }) {
            return Ok(existing);
        }
        let sequence = self
            .reserve_sequence(&request.scope, &request.thread_id)
            .await?;
        let message = ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
            thread_id: request.thread_id.clone(),
            sequence,
            kind: MessageKind::Assistant,
            status: MessageStatus::Draft,
            actor_id: None,
            source_binding_id: None,
            reply_target_binding_id: None,
            turn_id: None,
            turn_run_id: Some(request.turn_run_id),
            tool_result_ref: None,
            tool_result_provider_call: None,
            content: Some(request.content.into_text()),
            redaction_ref: None,
        };
        let path = message_record_path(&request.scope, &request.thread_id, message.message_id)?;
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
            Ok(()) => Ok(message),
            Err(PutError::VersionMismatch) => Err(SessionThreadError::Backend(format!(
                "filesystem CAS Absent rejected new assistant draft at {}",
                path.as_str()
            ))),
            Err(PutError::Other(error)) => Err(error),
        }
    }

    async fn append_tool_result_reference(
        &self,
        request: AppendToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let provider_call = request.provider_call;
        let envelope = ToolResultReferenceEnvelope::new(request.result_ref, request.safe_summary)
            .map_err(SessionThreadError::Serialization)?;
        let existing = self
            .list_thread_messages(&request.scope, &request.thread_id)
            .await?;
        if let Some(existing) = existing.into_iter().find(|message| {
            message.kind == MessageKind::ToolResultReference
                && message.status == MessageStatus::Finalized
                && message.turn_run_id.as_deref() == Some(request.turn_run_id.as_str())
                && message.tool_result_ref.as_deref() == Some(envelope.result_ref.as_str())
        }) {
            // Idempotent replay. If new provider metadata arrives, validate
            // and attach it (or reject on conflict) — matching the in-memory
            // contract semantics.
            if let Some(provider_call) = provider_call.as_ref() {
                provider_call
                    .validate()
                    .map_err(SessionThreadError::Serialization)?;
                match existing.tool_result_provider_call.as_ref() {
                    Some(existing_call) if existing_call == provider_call => {
                        return Ok(existing);
                    }
                    Some(_) => {
                        return Err(SessionThreadError::Serialization(
                            "tool result provider metadata conflicts with existing record"
                                .to_string(),
                        ));
                    }
                    None => {
                        let provider_call = provider_call.clone();
                        return self
                            .apply_message_update(
                                &request.scope,
                                &request.thread_id,
                                existing.message_id,
                                |message| {
                                    message.tool_result_provider_call = Some(provider_call.clone());
                                    Ok(())
                                },
                            )
                            .await;
                    }
                }
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
        let message = ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
            thread_id: request.thread_id.clone(),
            sequence,
            kind: MessageKind::ToolResultReference,
            status: MessageStatus::Finalized,
            actor_id: None,
            source_binding_id: None,
            reply_target_binding_id: None,
            turn_id: None,
            turn_run_id: Some(request.turn_run_id),
            tool_result_ref: Some(envelope.result_ref),
            tool_result_provider_call: provider_call,
            content: Some(content),
            redaction_ref: None,
        };
        let path = message_record_path(&request.scope, &request.thread_id, message.message_id)?;
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
            Ok(()) => Ok(message),
            Err(PutError::VersionMismatch) => Err(SessionThreadError::Backend(format!(
                "filesystem CAS Absent rejected new tool result reference at {}",
                path.as_str()
            ))),
            Err(PutError::Other(error)) => Err(error),
        }
    }

    async fn update_tool_result_reference(
        &self,
        request: UpdateToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let envelope =
            ToolResultReferenceEnvelope::new(request.result_ref.clone(), request.safe_summary)
                .map_err(SessionThreadError::Serialization)?;
        let content = serde_json::to_string(&envelope)
            .map_err(|error| SessionThreadError::Serialization(error.to_string()))?;
        let existing = self
            .list_thread_messages(&request.scope, &request.thread_id)
            .await?;
        let message = existing
            .into_iter()
            .find(|message| {
                message.kind == MessageKind::ToolResultReference
                    && message.status == MessageStatus::Finalized
                    && message.turn_run_id.as_deref() == Some(request.turn_run_id.as_str())
                    && message.tool_result_ref.as_deref() == Some(request.result_ref.as_str())
            })
            .ok_or_else(|| {
                SessionThreadError::Backend(format!(
                    "tool result reference {} was not found in thread {}",
                    request.result_ref, request.thread_id
                ))
            })?;
        self.apply_message_update(
            &request.scope,
            &request.thread_id,
            message.message_id,
            |message| {
                message.content = Some(content.clone());
                Ok(())
            },
        )
        .await
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
        self.apply_message_update(
            &request.scope,
            &request.thread_id,
            request.message_id,
            |message| {
                ensure_draft(message)?;
                message.content = Some(request.content.clone().into_text());
                Ok(())
            },
        )
        .await
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
        self.apply_message_update(scope, thread_id, message_id, |message| {
            ensure_draft(message)?;
            message.status = MessageStatus::Finalized;
            message.content = Some(content.clone().into_text());
            Ok(())
        })
        .await
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
        self.apply_message_update(
            &request.scope,
            &request.thread_id,
            request.message_id,
            |message| {
                message.status = MessageStatus::Redacted;
                message.content = None;
                message.tool_result_provider_call = None;
                message.redaction_ref = Some(request.redaction_ref.clone());
                Ok(())
            },
        )
        .await
    }

    async fn load_context_window(
        &self,
        request: LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
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
        Ok(ThreadHistory {
            thread: thread.record,
            summary_artifacts: history_summary_artifacts(&messages, summaries),
            messages: history_messages(&messages),
        })
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
        Ok(thread.record)
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
        self.read_thread_versioned(&request.scope, &request.thread_id)
            .await?
            .ok_or_else(|| SessionThreadError::UnknownThread {
                thread_id: request.thread_id.clone(),
            })?;
        let messages = self
            .list_thread_messages(&request.scope, &request.thread_id)
            .await?;
        let has_start = messages
            .iter()
            .any(|message| message.sequence == request.start_sequence);
        let has_end = messages
            .iter()
            .any(|message| message.sequence == request.end_sequence);
        if !has_start || !has_end {
            return Err(SessionThreadError::InvalidSummaryRange {
                start_sequence: request.start_sequence,
                end_sequence: request.end_sequence,
            });
        }
        let existing_summaries = self
            .list_thread_summaries(&request.scope, &request.thread_id)
            .await?;
        if request.model_context_policy.as_deref() == Some("replace_range_when_selected")
            && existing_summaries.iter().any(|summary| {
                summary.model_context_policy.as_deref() == Some("replace_range_when_selected")
                    && ranges_overlap(
                        request.start_sequence,
                        request.end_sequence,
                        summary.start_sequence,
                        summary.end_sequence,
                    )
            })
        {
            return Err(SessionThreadError::OverlappingSummaryRange {
                start_sequence: request.start_sequence,
                end_sequence: request.end_sequence,
            });
        }
        let artifact = SummaryArtifact {
            summary_id: SummaryArtifactId::new(),
            thread_id: request.thread_id.clone(),
            start_sequence: request.start_sequence,
            end_sequence: request.end_sequence,
            summary_kind: request.summary_kind,
            content: request.content.into_text(),
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
            Ok(()) => Ok(artifact),
            Err(PutError::VersionMismatch) => Err(SessionThreadError::Backend(format!(
                "filesystem CAS Absent rejected new summary artifact at {}",
                path.as_str()
            ))),
            Err(PutError::Other(error)) => Err(error),
        }
    }
}

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

impl InboundIdempotencyKey {
    fn from_request(request: &AcceptInboundMessageRequest) -> Option<Self> {
        Some(Self {
            scope: request.scope.clone(),
            source_binding_id: request.source_binding_id.clone()?,
            external_event_id: request.external_event_id.clone()?,
        })
    }
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

fn ranges_overlap(left_start: u64, left_end: u64, right_start: u64, right_end: u64) -> bool {
    left_start <= right_end && right_start <= left_end
}

fn is_model_visible(status: MessageStatus) -> bool {
    matches!(
        status,
        MessageStatus::Accepted | MessageStatus::Submitted | MessageStatus::Finalized
    )
}

const REDACTED_SUMMARY_CONTENT: &str = "[redacted]";

fn context_messages_with_summary_replacements(
    messages: &[ThreadMessageRecord],
    summaries: &[SummaryArtifact],
) -> Vec<ContextMessage> {
    let replacement_summaries = summaries
        .iter()
        .filter(|summary| {
            summary.model_context_policy.as_deref() == Some("replace_range_when_selected")
                && !summary_covers_hidden_content(messages, summary)
        })
        .collect::<Vec<_>>();
    let mut skip_through = 0u64;
    let mut emitted_summaries: std::collections::HashSet<_> = std::collections::HashSet::new();
    let mut context = Vec::new();
    for message in messages
        .iter()
        .filter(|message| is_model_visible(message.status))
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
            });
            emitted_summaries.insert(summary.summary_id);
            skip_through = summary.end_sequence;
            continue;
        }
        if let Some(content) = message.content.clone() {
            context.push(ContextMessage {
                message_id: Some(message.message_id),
                summary_id: None,
                sequence: message.sequence,
                kind: message.kind,
                tool_result_provider_call: message.tool_result_provider_call.clone(),
                content,
            });
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
        .filter(|message| is_model_visible(message.status))
        .map(|message| (message.message_id, message))
        .collect();
    message_ids
        .iter()
        .filter_map(|message_id| {
            let message = visible_messages.get(message_id)?;
            Some(ContextMessage {
                message_id: Some(message.message_id),
                summary_id: None,
                sequence: message.sequence,
                kind: message.kind,
                tool_result_provider_call: message.tool_result_provider_call.clone(),
                content: message.content.clone()?,
            })
        })
        .collect()
}

fn history_messages(messages: &[ThreadMessageRecord]) -> Vec<ThreadMessageRecord> {
    messages
        .iter()
        .map(|message| ThreadMessageRecord {
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
            tool_result_ref: message.tool_result_ref.clone(),
            tool_result_provider_call: None,
            content: message.content.clone(),
            redaction_ref: message.redaction_ref.clone(),
        })
        .collect()
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

fn summary_covers_hidden_content(
    messages: &[ThreadMessageRecord],
    summary: &SummaryArtifact,
) -> bool {
    messages.iter().any(|message| {
        summary.start_sequence <= message.sequence
            && message.sequence <= summary.end_sequence
            && !is_model_visible(message.status)
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
// Mirrors the run-state / authorization / outbound stores: every
// multi-step transition is implemented with
// `put(_, _, CasExpectation::Version)` + retry on
// `FilesystemError::VersionMismatch`. Byte-only backends (LocalFilesystem)
// reject anything but `Any`; we fall back to `Any` so the existing
// single-instance guarantee from the per-path lock map carries the safety
// invariant.

/// Local error classification for the CAS-aware put helper.
enum PutError {
    /// Backend reported `VersionMismatch` (cross-process raced us). The
    /// caller retries by re-reading the current record.
    VersionMismatch,
    /// Any other backend or serialization failure; surface to caller.
    Other(SessionThreadError),
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

// ── Per-path async serialization (Unsupported→Any fallback) ────
//
// Backends without per-record versioning (LocalFilesystem) take the
// `CasExpectation::Any` fallback path. The per-path mutex below is the
// process-local ordering guarantee that fills in for CAS in that case.
// Values are `Weak<Mutex<()>>` so the map does not pin lock entries alive
// once all in-flight operations on a path have released their `Arc`
// clones — mirrors the run-state store's lock map.

type FilesystemRecordLock = Arc<tokio::sync::Mutex<()>>;

static FILESYSTEM_RECORD_LOCKS: OnceLock<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>> =
    OnceLock::new();

fn filesystem_record_lock(path: &ScopedPath) -> FilesystemRecordLock {
    let locks = FILESYSTEM_RECORD_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = locks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.retain(|_, weak| weak.strong_count() > 0);
    let key = path.as_str();
    if let Some(existing) = guard.get(key).and_then(Weak::upgrade) {
        return existing;
    }
    let fresh: FilesystemRecordLock = Arc::new(tokio::sync::Mutex::new(()));
    guard.insert(key.to_string(), Arc::downgrade(&fresh));
    fresh
}

impl From<FilesystemError> for SessionThreadError {
    fn from(error: FilesystemError) -> Self {
        Self::Backend(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};

    use super::{InboundIdempotencyKey, idempotency_record_key};
    use crate::ThreadScope;

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
}
