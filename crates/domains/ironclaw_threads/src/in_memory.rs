use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::ids::ThreadId;
use ironclaw_host_api::turn::TurnRunId;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::identifiers::SummaryArtifactId;
use crate::stored_message::serialize_stored_thread_message;
use crate::summary_artifacts::{
    find_overlapping_summary, newest_context_barrier, sorted_context_summaries,
};
use crate::title::derive_thread_title;
use crate::tool_result_records::{
    validate_tool_result_record_content, validate_tool_result_record_ref,
};
use crate::{
    AcceptInboundMessageRequest, AcceptSubagentResultRequest, AcceptedInboundMessage,
    AcceptedInboundMessageReplay, AcceptedSubagentResult, AppendAssistantDraftRequest,
    AppendCapabilityDisplayPreviewRequest, AppendFinalizedAssistantMessageRequest,
    AppendToolResultReferenceRequest, BoundedThreadMessageSnapshot, BoundedThreadMessages,
    BoundedThreadMessagesRequest, CapabilityDisplayPreviewEnvelope, ContextMessage,
    ContextMessages, ContextWindow, CreateSummaryArtifactRequest, DeleteToolResultRecordRequest,
    EnsureThreadRequest, InboundMessageReplayMetadata, LatestThreadMessageRequest,
    ListThreadsForScopeRequest, ListThreadsForScopeResponse, LoadContextMessagesRequest,
    LoadContextWindowRequest, MessageContent, MessageKind, MessageStatus,
    PublishStructuredFinalizationMessageRequest, PutStructuredFinalizationRequest,
    PutToolResultRecordRequest, ReadStructuredFinalizationRequest, ReadToolResultRecordRequest,
    RedactMessageRequest, ReplayAcceptedInboundMessageRequest, SessionThreadError,
    SessionThreadRecord, SessionThreadService, StructuredFinalizationRecord, SummaryArtifact,
    SummaryModelContextPolicy, ThreadHistory, ThreadHistoryRequest, ThreadMessageId,
    ThreadMessageRange, ThreadMessageRangeRequest, ThreadMessageRecord, ThreadScope,
    ToolResultRecordRead, ToolResultReferenceEnvelope, UpdateAssistantDraftRequest,
    UpdateToolResultRecordRequest, UpdateToolResultReferenceRequest,
};

#[derive(Debug, Clone, Default)]
pub struct InMemorySessionThreadService {
    state: Arc<Mutex<InMemoryState>>,
}

#[derive(Debug, Default)]
struct InMemoryState {
    threads: HashMap<ThreadId, StoredThread>,
    inbound_idempotency: HashMap<InboundIdempotencyKey, InboundIdempotencyRecord>,
    prepared_contexts: HashMap<ThreadId, crate::PreparedContextRecord>,
    structured_finalizations: HashMap<StructuredFinalizationKey, StructuredFinalizationRecord>,
}

#[derive(Debug, Clone)]
struct StoredThread {
    record: SessionThreadRecord,
    /// Backend-owned identity for this lifetime of an explicit thread id.
    /// Finalization evidence uses it as its archive partition so deleting and
    /// recreating an id can never replay the predecessor's output.
    incarnation_id: Uuid,
    messages: Vec<ThreadMessageRecord>,
    summary_artifacts: Vec<SummaryArtifact>,
    tool_result_records: HashMap<String, Vec<u8>>,
    next_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct InboundIdempotencyKey {
    scope: ThreadScope,
    source_binding_id: String,
    external_event_id: String,
}

#[derive(Debug, Clone)]
struct InboundIdempotencyRecord {
    thread_id: ThreadId,
    message_id: ThreadMessageId,
    replay_metadata: InboundMessageReplayMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct StructuredFinalizationKey {
    scope: ThreadScope,
    thread_id: ThreadId,
    incarnation_id: Uuid,
    turn_run_id: TurnRunId,
}

impl StructuredFinalizationKey {
    fn from_read(request: &ReadStructuredFinalizationRequest, incarnation_id: Uuid) -> Self {
        Self {
            scope: request.scope.clone(),
            thread_id: request.thread_id.clone(),
            incarnation_id,
            turn_run_id: request.turn_run_id,
        }
    }

    fn from_record(record: &StructuredFinalizationRecord, incarnation_id: Uuid) -> Self {
        Self {
            scope: record.scope.clone(),
            thread_id: record.thread_id.clone(),
            incarnation_id,
            turn_run_id: record.turn_run_id,
        }
    }
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

/// Shared mint for the two thread-creating doors (`ensure_thread` and
/// `accept_prepared_context`): one construction site so the stored shape —
/// timestamps, empty collections, first sequence — cannot drift between the
/// conversation and prepared-context paths. Callers scope-check BEFORE
/// minting; this only inserts-or-returns.
fn mint_thread_locked<'a>(
    threads: &'a mut HashMap<ThreadId, StoredThread>,
    scope: &ThreadScope,
    thread_id: &ThreadId,
    created_by_actor_id: &str,
    title: Option<String>,
    metadata_json: Option<String>,
    now: chrono::DateTime<Utc>,
) -> &'a mut StoredThread {
    threads
        .entry(thread_id.clone())
        .or_insert_with(|| StoredThread {
            record: SessionThreadRecord {
                scope: scope.clone(),
                thread_id: thread_id.clone(),
                created_by_actor_id: created_by_actor_id.to_string(),
                title,
                metadata_json,
                goal: None,
                created_at: Some(now),
                updated_at: Some(now),
            },
            incarnation_id: Uuid::new_v4(),
            messages: Vec::new(),
            summary_artifacts: Vec::new(),
            tool_result_records: HashMap::new(),
            next_sequence: 1,
        })
}

#[async_trait]
impl SessionThreadService for InMemorySessionThreadService {
    async fn ensure_thread(
        &self,
        request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        let mut state = self.state.lock().await;
        let thread_id = match request.thread_id {
            Some(thread_id) => thread_id,
            None => generated_thread_id()?,
        };
        if let Some(existing) = state.threads.get(&thread_id) {
            if existing.record.scope != request.scope {
                return Err(SessionThreadError::ThreadScopeMismatch { thread_id });
            }
            return Ok(existing.record.clone());
        }

        let now = Utc::now();
        Ok(mint_thread_locked(
            &mut state.threads,
            &request.scope,
            &thread_id,
            &request.created_by_actor_id,
            request.title,
            request.metadata_json,
            now,
        )
        .record
        .clone())
    }

    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        self.accept_inbound_message_with_replay_metadata(
            request,
            InboundMessageReplayMetadata::default(),
        )
        .await
    }

    async fn accept_inbound_message_with_replay_metadata(
        &self,
        request: AcceptInboundMessageRequest,
        replay_metadata: InboundMessageReplayMetadata,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        let mut state = self.state.lock().await;
        if let Some(key) = InboundIdempotencyKey::from_request(&request)
            && let Some(record) = state.inbound_idempotency.get(&key)
        {
            if record.thread_id != request.thread_id {
                return Err(SessionThreadError::IdempotentReplayThreadMismatch {
                    stored_thread_id: record.thread_id.clone(),
                    requested_thread_id: request.thread_id,
                });
            }
            let thread = get_thread(&state, &request.scope, &record.thread_id)?;
            let existing = thread
                .messages
                .iter()
                .find(|message| message.message_id == record.message_id)
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
                thread_id: existing.thread_id.clone(),
                message_id: record.message_id,
                sequence: existing.sequence,
                idempotent_replay: true,
                replay_metadata: record.replay_metadata.clone(),
            });
        }

        let key = InboundIdempotencyKey::from_request(&request);
        let thread = get_thread_mut(&mut state, &request.scope, &request.thread_id)?;
        let message_id = ThreadMessageId::new();
        // Validate the payload before mutating any thread state. A rejected
        // attachment must not increment the sequence or bump the
        // last-activity stamp, or an invalid message would surface the
        // thread at the top of the sidebar without being appended.
        let (content_text, attachments) = request.content.into_parts();
        crate::contract::validate_attachment_refs(&attachments)?;
        let sequence = thread.next_sequence;
        thread.next_sequence += 1;
        // Appending a message is thread activity; bump the last-activity
        // stamp so activity-ordered listings surface this thread first.
        let now = Utc::now();
        thread.record.updated_at = Some(now);
        let message = ThreadMessageRecord {
            message_id,
            thread_id: request.thread_id.clone(),
            sequence,
            kind: MessageKind::User,
            status: MessageStatus::Accepted,
            created_at: Some(now),
            updated_at: Some(now),
            actor_id: Some(request.actor_id),
            source_binding_id: request.source_binding_id.clone(),
            reply_target_binding_id: request.reply_target_binding_id,
            turn_id: None,
            turn_run_id: None,
            tool_result_ref: None,
            tool_result_provider_call: None,
            content: Some(content_text),
            attachments,
            redaction_ref: None,
        };
        crate::contract::validate_new_message_timestamps(&message, "inbound user")?;
        thread.messages.push(message);

        if let Some(key) = key {
            state.inbound_idempotency.insert(
                key,
                InboundIdempotencyRecord {
                    thread_id: request.thread_id.clone(),
                    message_id,
                    replay_metadata: replay_metadata.clone(),
                },
            );
        }

        Ok(AcceptedInboundMessage {
            thread_id: request.thread_id,
            message_id,
            sequence,
            idempotent_replay: false,
            replay_metadata,
        })
    }

    async fn accept_subagent_result(
        &self,
        request: AcceptSubagentResultRequest,
    ) -> Result<AcceptedSubagentResult, SessionThreadError> {
        // Before the identity is hashed into the shared dedupe index: an empty
        // half hashes just fine and would silently merge unrelated children.
        crate::contract::validate_subagent_acceptance_identity(
            &request.source_binding_id,
            &request.external_event_id,
        )?;
        let mut state = self.state.lock().await;
        // The SAME index the inbound door writes: one dedupe index for the
        // whole acceptance boundary. Both halves of the identity arrive
        // already resolved — this crate derives neither.
        let key = InboundIdempotencyKey {
            scope: request.scope.clone(),
            source_binding_id: request.source_binding_id.clone(),
            external_event_id: request.external_event_id.clone(),
        };
        if let Some(record) = state.inbound_idempotency.get(&key) {
            if record.thread_id != request.thread_id {
                return Err(SessionThreadError::IdempotentReplayThreadMismatch {
                    stored_thread_id: record.thread_id.clone(),
                    requested_thread_id: request.thread_id,
                });
            }
            let message_id = record.message_id;
            let thread = get_thread(&state, &request.scope, &record.thread_id)?;
            let existing = thread
                .messages
                .iter()
                .find(|message| message.message_id == message_id)
                .ok_or(SessionThreadError::UnknownMessage { message_id })?;
            // A hit that landed on a user/steering row is an identity
            // collision, not a replay.
            if existing.kind != MessageKind::System {
                return Err(SessionThreadError::Backend(format!(
                    "subagent-result acceptance identity is already held by a non-system row \
                     ({:?})",
                    existing.kind
                )));
            }
            return Ok(AcceptedSubagentResult {
                message_id,
                sequence: existing.sequence,
                idempotent_replay: true,
            });
        }

        let thread = get_thread_mut(&mut state, &request.scope, &request.thread_id)?;
        let message_id = ThreadMessageId::new();
        // Already framed by construction: `FramedSubagentText` has no
        // constructor but `frame`, so no caller can land raw child text in a
        // row the model reads at its system role.
        let content_text = request.content.into_string();
        let sequence = thread.next_sequence;
        thread.next_sequence += 1;
        let now = Utc::now();
        thread.record.updated_at = Some(now);
        let message = ThreadMessageRecord {
            message_id,
            thread_id: request.thread_id.clone(),
            sequence,
            // System-class, never `MessageKind::User`: a child's output is
            // untrusted agent text, not a human instruction.
            kind: MessageKind::System,
            status: MessageStatus::Finalized,
            created_at: Some(now),
            updated_at: Some(now),
            actor_id: None,
            source_binding_id: Some(request.source_binding_id),
            reply_target_binding_id: None,
            turn_id: None,
            turn_run_id: None,
            tool_result_ref: None,
            tool_result_provider_call: None,
            content: Some(content_text),
            attachments: Vec::new(),
            redaction_ref: None,
        };
        crate::contract::validate_new_message_timestamps(&message, "subagent result")?;
        thread.messages.push(message);
        state.inbound_idempotency.insert(
            key,
            InboundIdempotencyRecord {
                thread_id: request.thread_id,
                message_id,
                replay_metadata: InboundMessageReplayMetadata::default(),
            },
        );

        Ok(AcceptedSubagentResult {
            message_id,
            sequence,
            idempotent_replay: false,
        })
    }

    async fn accept_prepared_context(
        &self,
        request: crate::PreparedContextRequest,
    ) -> Result<crate::AcceptedPreparedContext, SessionThreadError> {
        crate::prepared_context::validate_prepared_context_request(&request)?;
        let stamped_metadata =
            crate::prepared_context::stamped_metadata_json(request.metadata_json.as_deref())?;
        let thread_id = request.thread_id.clone();
        let now = Utc::now();
        let seed = crate::prepared_context::prepared_seed(&request, &thread_id, now)?;
        let crate::prepared_context::PreparedSeed {
            rows,
            tool_result_records,
        } = seed;

        let mut state = self.state.lock().await;
        if let Some(record) = state.prepared_contexts.get(&thread_id) {
            let stored_scope_matches = state
                .threads
                .get(&thread_id)
                .map(|stored| stored.record.scope == request.scope)
                .unwrap_or(false);
            if !stored_scope_matches {
                return Err(SessionThreadError::ThreadScopeMismatch { thread_id });
            }
            return crate::prepared_context::replay_prepared_context(record, &request, &thread_id);
        }

        if let Some(existing) = state.threads.get(&thread_id)
            && existing.record.scope != request.scope
        {
            return Err(SessionThreadError::ThreadScopeMismatch { thread_id });
        }
        let seeded_message_count = rows.len() as u64;
        let last_message_id = rows.last().map(|row| row.message_id).ok_or_else(|| {
            SessionThreadError::InvalidPreparedContext {
                reason: "seeded rows must not be empty".to_string(),
            }
        })?;
        let accepted_message_ref =
            crate::prepared_context::accepted_prepared_message_ref(last_message_id)?;

        let stored = mint_thread_locked(
            &mut state.threads,
            &request.scope,
            &thread_id,
            &request.actor_id,
            request.title.clone(),
            Some(stamped_metadata.clone()),
            now,
        );
        for mut row in rows {
            row.sequence = stored.next_sequence;
            stored.next_sequence += 1;
            stored.messages.push(row);
        }
        for (result_ref, bytes) in tool_result_records {
            stored.tool_result_records.insert(result_ref, bytes);
        }
        stored.record.updated_at = Some(now);

        let record = crate::PreparedContextRecord {
            schema_version: crate::PREPARED_CONTEXT_RECORD_SCHEMA_VERSION,
            idempotency_key: request.idempotency_key.clone(),
            actor_id: request.actor_id.clone(),
            accepted_message_ref: accepted_message_ref.as_str().to_string(),
            declarations: request.declarations.clone(),
            seeded_message_count,
            created_at: now,
        };
        state.prepared_contexts.insert(thread_id.clone(), record);

        Ok(crate::AcceptedPreparedContext {
            thread_id,
            accepted_message_ref,
            idempotent_replay: false,
        })
    }

    async fn read_prepared_context(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<Option<crate::PreparedContextRecord>, SessionThreadError> {
        let state = self.state.lock().await;
        let stored =
            state
                .threads
                .get(thread_id)
                .ok_or_else(|| SessionThreadError::UnknownThread {
                    thread_id: thread_id.clone(),
                })?;
        if &stored.record.scope != scope {
            return Err(SessionThreadError::UnknownThread {
                thread_id: thread_id.clone(),
            });
        }
        Ok(state.prepared_contexts.get(thread_id).cloned())
    }

    async fn read_structured_finalization(
        &self,
        request: ReadStructuredFinalizationRequest,
    ) -> Result<Option<StructuredFinalizationRecord>, SessionThreadError> {
        let state = self.state.lock().await;
        let stored = state.threads.get(&request.thread_id).ok_or_else(|| {
            SessionThreadError::UnknownThread {
                thread_id: request.thread_id.clone(),
            }
        })?;
        if stored.record.scope != request.scope {
            return Err(SessionThreadError::UnknownThread {
                thread_id: request.thread_id,
            });
        }
        Ok(state
            .structured_finalizations
            .get(&StructuredFinalizationKey::from_read(
                &request,
                stored.incarnation_id,
            ))
            .cloned())
    }

    async fn put_structured_finalization(
        &self,
        request: PutStructuredFinalizationRequest,
    ) -> Result<StructuredFinalizationRecord, SessionThreadError> {
        request
            .record
            .validate()
            .map_err(|reason| SessionThreadError::InvalidStructuredFinalization { reason })?;
        let mut state = self.state.lock().await;
        let thread = state
            .threads
            .get(&request.record.thread_id)
            .ok_or_else(|| SessionThreadError::UnknownThread {
                thread_id: request.record.thread_id.clone(),
            })?;
        if thread.record.scope != request.record.scope {
            return Err(SessionThreadError::UnknownThread {
                thread_id: request.record.thread_id,
            });
        }
        let key = StructuredFinalizationKey::from_record(&request.record, thread.incarnation_id);
        if let Some(existing) = state.structured_finalizations.get(&key) {
            if existing.same_immutable_content(&request.record) {
                return Ok(existing.clone());
            }
            return Err(SessionThreadError::StructuredFinalizationConflict {
                turn_run_id: request.record.turn_run_id,
            });
        }
        state
            .structured_finalizations
            .insert(key, request.record.clone());
        Ok(request.record)
    }

    async fn publish_structured_finalization_message(
        &self,
        request: PublishStructuredFinalizationMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let mut state = self.state.lock().await;
        let thread = state.threads.get(&request.thread_id).ok_or_else(|| {
            SessionThreadError::UnknownThread {
                thread_id: request.thread_id.clone(),
            }
        })?;
        if thread.record.scope != request.scope {
            return Err(SessionThreadError::UnknownThread {
                thread_id: request.thread_id.clone(),
            });
        }
        let run_id = request.turn_run_id;
        let key = StructuredFinalizationKey {
            scope: request.scope.clone(),
            thread_id: request.thread_id.clone(),
            incarnation_id: thread.incarnation_id,
            turn_run_id: run_id,
        };
        let record = state
            .structured_finalizations
            .get(&key)
            .ok_or(SessionThreadError::StructuredFinalizationPublishMismatch {
                message_id: request.message_id,
                reason: "durable finalization record is missing",
            })?
            .clone();
        if record.scope != request.scope
            || record.thread_id != request.thread_id
            || record.turn_run_id != run_id
        {
            return Err(SessionThreadError::StructuredFinalizationPublishMismatch {
                message_id: request.message_id,
                reason: "durable finalization record identity does not match request",
            });
        }
        if record.raw_json != request.replacement {
            return Err(SessionThreadError::StructuredFinalizationPublishMismatch {
                message_id: request.message_id,
                reason: "replacement does not match durable finalization output",
            });
        }
        let turn_run_id = request.turn_run_id.to_string();

        let thread = state.threads.get_mut(&request.thread_id).ok_or_else(|| {
            SessionThreadError::UnknownThread {
                thread_id: request.thread_id.clone(),
            }
        })?;
        let message = thread
            .messages
            .iter_mut()
            .find(|message| message.message_id == request.message_id)
            .ok_or(SessionThreadError::UnknownMessage {
                message_id: request.message_id,
            })?;
        if message.kind != MessageKind::Assistant || message.status != MessageStatus::Finalized {
            return Err(SessionThreadError::StructuredFinalizationPublishMismatch {
                message_id: request.message_id,
                reason: "message is not a finalized assistant",
            });
        }
        if message.thread_id != request.thread_id {
            return Err(SessionThreadError::StructuredFinalizationPublishMismatch {
                message_id: request.message_id,
                reason: "message belongs to a different thread",
            });
        }
        if message.turn_run_id.as_deref() != Some(turn_run_id.as_str()) {
            return Err(SessionThreadError::StructuredFinalizationPublishMismatch {
                message_id: request.message_id,
                reason: "message belongs to a different turn run",
            });
        }
        let current_content = message.content.as_deref().ok_or(
            SessionThreadError::StructuredFinalizationPublishMismatch {
                message_id: request.message_id,
                reason: "message content is missing",
            },
        )?;
        if current_content != record.candidate && current_content != request.replacement {
            return Err(SessionThreadError::StructuredFinalizationPublishMismatch {
                message_id: request.message_id,
                reason: "message content does not match durable candidate",
            });
        }
        if current_content == record.candidate {
            let now = Utc::now();
            message.content = Some(request.replacement);
            message.updated_at = Some(now);
            thread.record.updated_at = Some(now);
        }
        Ok(message.clone())
    }

    async fn replay_accepted_inbound_message(
        &self,
        request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
        let state = self.state.lock().await;
        let Some((key, record)) = state.inbound_idempotency.iter().find(|(key, _)| {
            key.scope == request.scope
                && key.source_binding_id == request.source_binding_id
                && key.external_event_id == request.external_event_id
        }) else {
            return Ok(None);
        };
        let thread = get_thread(&state, &key.scope, &record.thread_id)?;
        let message = thread
            .messages
            .iter()
            .find(|message| message.message_id == record.message_id)
            .ok_or(SessionThreadError::UnknownMessage {
                message_id: record.message_id,
            })?;
        if message.actor_id.as_deref() != Some(request.actor_id.as_str()) {
            return Ok(None);
        }
        Ok(Some(AcceptedInboundMessageReplay {
            scope: key.scope.clone(),
            thread_id: record.thread_id.clone(),
            message_id: record.message_id,
            sequence: message.sequence,
            status: message.status,
            actor_id: message.actor_id.clone(),
            source_binding_id: message.source_binding_id.clone(),
            reply_target_binding_id: message.reply_target_binding_id.clone(),
            turn_run_id: message.turn_run_id.clone(),
            replay_metadata: record.replay_metadata.clone(),
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
        let mut state = self.state.lock().await;
        let thread = get_thread_mut(&mut state, scope, thread_id)?;
        let message_index = thread
            .messages
            .iter()
            .position(|message| message.message_id == message_id)
            .ok_or(SessionThreadError::UnknownMessage { message_id })?;
        // Idempotent re-submit: if this exact run already submitted the message,
        // a redelivered/duplicate ack is a no-op rather than an
        // `InvalidMessageTransition`. Mirrors the filesystem backend so the
        // queued-message consumer's at-least-once ack path is tolerant on both
        // stores. A *different* run, or a `RejectedBusy` row, still fails below.
        {
            let message = &thread.messages[message_index];
            if message.status == MessageStatus::Submitted
                && message.turn_run_id.as_deref() == Some(turn_run_id.as_str())
            {
                return Ok(message.clone());
            }
        }
        // D14: a subagent result row (`MessageKind::System`, written by
        // `accept_subagent_result`) is terminal on arrival. The queue's
        // best-effort Submitted flip must treat it as already-settled, not as a
        // transition violation — an already-terminal row has nothing to flip.
        // Scoped to `System` specifically: any OTHER finalized kind (Assistant,
        // ToolResultReference, CapabilityDisplayPreview, ...) still falls through
        // to `ensure_user_accepted` and fails loud — a caller bug targeting the
        // wrong message id must surface, not silently succeed.
        {
            let message = &thread.messages[message_index];
            if message.kind == MessageKind::System && message.status == MessageStatus::Finalized {
                return Ok(message.clone());
            }
        }
        let was_queued = {
            let message = &thread.messages[message_index];
            ensure_user_accepted(message, "mark_message_submitted")?;
            message.status == MessageStatus::Queued
        };
        if was_queued {
            let sequence = thread.next_sequence;
            thread.next_sequence += 1;
            thread.record.updated_at = Some(Utc::now());
            thread.messages[message_index].sequence = sequence;
        }
        let message = &mut thread.messages[message_index];
        let before_created_at = message.created_at;
        let before_updated_at = message.updated_at;
        message.status = MessageStatus::Submitted;
        message.turn_id = Some(turn_id);
        message.turn_run_id = Some(turn_run_id);
        crate::contract::validate_message_timestamp_fields_not_cleared(
            message.message_id,
            before_created_at,
            before_updated_at,
            message.created_at,
            message.updated_at,
            "mark_message_submitted",
        )?;
        let submitted = message.clone();
        if was_queued {
            thread.messages.sort_by_key(|message| message.sequence);
        }
        Ok(submitted)
    }

    async fn mark_message_rejected_busy(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let mut state = self.state.lock().await;
        let message = get_message_mut(&mut state, scope, thread_id, message_id)?;
        ensure_user_accepted(message, "mark_message_rejected_busy")?;
        let before_created_at = message.created_at;
        let before_updated_at = message.updated_at;
        message.status = MessageStatus::RejectedBusy;
        message.turn_id = None;
        message.turn_run_id = None;
        crate::contract::validate_message_timestamp_fields_not_cleared(
            message.message_id,
            before_created_at,
            before_updated_at,
            message.created_at,
            message.updated_at,
            "mark_message_rejected_busy",
        )?;
        Ok(message.clone())
    }

    async fn mark_message_queued(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        active_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let mut state = self.state.lock().await;
        let message = get_message_mut(&mut state, scope, thread_id, message_id)?;
        ensure_user_accepted(message, "mark_message_queued")?;
        message.status = MessageStatus::Queued;
        message.turn_id = None;
        message.turn_run_id = Some(active_run_id);
        Ok(message.clone())
    }

    async fn read_thread_message(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<Option<ThreadMessageRecord>, SessionThreadError> {
        let mut state = self.state.lock().await;
        // Only genuine lookup misses read as absence — an unknown thread
        // (including scope-hidden threads) or an unknown message, matching the
        // filesystem backend's point-read contract. Every other error
        // propagates instead of masquerading as a miss.
        match get_message_mut(&mut state, scope, thread_id, message_id) {
            Ok(message) => Ok(Some(message.clone())),
            Err(
                SessionThreadError::UnknownThread { .. }
                | SessionThreadError::UnknownMessage { .. },
            ) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn append_assistant_draft(
        &self,
        request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let mut state = self.state.lock().await;
        let thread = get_thread_mut(&mut state, &request.scope, &request.thread_id)?;
        let requested_content = request.content.as_text().to_owned();
        if let Some(existing) = thread.messages.iter().rev().find(|message| {
            message.kind == MessageKind::Assistant
                && message.turn_run_id.as_deref() == Some(request.turn_run_id.as_str())
                && crate::contract::should_reuse_assistant_run_message(
                    message,
                    &requested_content,
                    request.content.attachments(),
                )
        }) {
            return Ok(existing.clone());
        }
        let message_id = ThreadMessageId::new();
        let now = Utc::now();
        let message = ThreadMessageRecord {
            message_id,
            thread_id: request.thread_id.clone(),
            sequence: thread.next_sequence,
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
            content: Some(requested_content),
            attachments: Vec::new(),
            redaction_ref: None,
        };
        thread.next_sequence += 1;
        // Appending a message is thread activity; bump the last-activity
        // stamp so activity-ordered listings surface this thread first.
        thread.record.updated_at = Some(now);
        crate::contract::validate_new_message_timestamps(&message, "assistant draft")?;
        thread.messages.push(message.clone());
        Ok(message)
    }

    async fn append_finalized_assistant_message(
        &self,
        request: AppendFinalizedAssistantMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let (content, attachments) = request.content.into_parts();
        crate::contract::validate_attachment_refs(&attachments)?;
        let mut state = self.state.lock().await;
        let thread = get_thread_mut(&mut state, &request.scope, &request.thread_id)?;
        // Latest sibling first: a steered run finalizes more than one reply, so
        // the dedup baseline is the most recent assistant message for the run.
        if let Some(existing) = thread.messages.iter_mut().rev().find(|message| {
            message.kind == MessageKind::Assistant
                && message.turn_run_id.as_deref() == Some(request.turn_run_id.as_str())
        }) {
            if existing.status == MessageStatus::Draft {
                let now = Utc::now();
                let before_created_at = existing.created_at;
                let before_updated_at = existing.updated_at;
                existing.status = MessageStatus::Finalized;
                existing.content = Some(content);
                existing.attachments = attachments;
                existing.updated_at = Some(now);
                crate::contract::validate_message_timestamp_fields_not_cleared(
                    existing.message_id,
                    before_created_at,
                    before_updated_at,
                    existing.created_at,
                    existing.updated_at,
                    "append_finalized_assistant_message",
                )?;
                thread.record.updated_at = Some(now);
                return Ok(existing.clone());
            }
            if crate::contract::should_reuse_assistant_run_message(existing, &content, &attachments)
            {
                // Retry of the same finalized reply (or a redacted/deleted
                // row that must not be resurrected): idempotent return.
                return Ok(existing.clone());
            }
            if existing.status == MessageStatus::Finalized
                && existing.content.as_deref() == Some(content.as_str())
            {
                // Same finalized text with a DIFFERENT attachment set is a
                // mismatched replay, not a steered second reply: appending a
                // sibling would duplicate the visible reply, and returning
                // the old row would silently drop the new attachments. Fail
                // loud instead (the loop transcript port surfaces this as a
                // transcript write failure).
                return Err(SessionThreadError::InvalidMessageTransition {
                    message_id: existing.message_id,
                    from: MessageStatus::Finalized,
                    attempted: "append_finalized_assistant_message with mismatched attachments",
                });
            }
            // A DIFFERENT finalized reply in the same run — a steered run
            // replying again. Fall through and append a sibling.
        }
        let now = Utc::now();
        let message = ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
            thread_id: request.thread_id.clone(),
            sequence: thread.next_sequence,
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
            content: Some(content),
            attachments,
            redaction_ref: None,
        };
        thread.next_sequence += 1;
        // Appending a message is thread activity; bump the last-activity
        // stamp so activity-ordered listings surface this thread first.
        thread.record.updated_at = Some(now);
        crate::contract::validate_new_message_timestamps(&message, "finalized assistant")?;
        thread.messages.push(message.clone());
        Ok(message)
    }

    async fn append_tool_result_reference(
        &self,
        request: AppendToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let mut state = self.state.lock().await;
        let thread = get_thread_mut(&mut state, &request.scope, &request.thread_id)?;
        let provider_call = request.provider_call;
        if let Some(provider_call) = &provider_call {
            provider_call
                .validate()
                .map_err(SessionThreadError::Serialization)?;
        }
        let mut envelope = ToolResultReferenceEnvelope::new_best_effort_model_observation(
            request.result_ref,
            request.safe_summary,
            request.model_observation,
        )
        .map_err(SessionThreadError::Serialization)?;
        envelope.intrinsic_outcome = request.intrinsic_outcome;
        if let Some(existing) = thread.messages.iter_mut().find(|message| {
            message.kind == MessageKind::ToolResultReference
                && message.status == MessageStatus::Finalized
                && message.turn_run_id.as_deref() == Some(request.turn_run_id.as_str())
                && message.tool_result_ref.as_deref() == Some(envelope.result_ref.as_str())
                && provider_call.as_ref().is_none_or(|requested| {
                    message
                        .tool_result_provider_call
                        .as_ref()
                        .is_none_or(|existing| {
                            existing.provider_call_id == requested.provider_call_id
                        })
                })
        }) {
            let now = Utc::now();
            let before_created_at = existing.created_at;
            let before_updated_at = existing.updated_at;
            let mut changed = false;
            if let Some(provider_call) = provider_call.as_ref() {
                match existing.tool_result_provider_call.as_ref() {
                    None => {
                        existing.tool_result_provider_call = Some(provider_call.clone());
                        changed = true;
                    }
                    Some(existing_provider_call) if existing_provider_call == provider_call => {}
                    Some(_) => {
                        return Err(SessionThreadError::Serialization(
                            "tool result provider metadata conflicts with existing record"
                                .to_string(),
                        ));
                    }
                }
            }
            if let Some(model_observation) = envelope.model_observation.as_ref() {
                let content = existing.content.as_deref().ok_or_else(|| {
                    SessionThreadError::Serialization(
                        "tool result reference content is missing".to_string(),
                    )
                })?;
                if let Some(content) =
                    ToolResultReferenceEnvelope::merge_model_observation_content_if_absent(
                        content,
                        model_observation.clone(),
                    )
                    .map_err(SessionThreadError::Serialization)?
                {
                    existing.content = Some(content);
                    changed = true;
                }
            }
            if let Some(intrinsic_outcome) = envelope.intrinsic_outcome {
                let content = existing.content.as_deref().ok_or_else(|| {
                    SessionThreadError::Serialization(
                        "tool result reference content is missing".to_string(),
                    )
                })?;
                if let Some(content) =
                    ToolResultReferenceEnvelope::merge_intrinsic_outcome_content_if_absent(
                        content,
                        intrinsic_outcome,
                    )
                    .map_err(SessionThreadError::Serialization)?
                {
                    existing.content = Some(content);
                    changed = true;
                }
            }
            if changed {
                existing.updated_at = Some(now);
            }
            crate::contract::validate_message_timestamp_fields_not_cleared(
                existing.message_id,
                before_created_at,
                before_updated_at,
                existing.created_at,
                existing.updated_at,
                "append_tool_result_reference",
            )?;
            let updated = existing.clone();
            if changed {
                thread.record.updated_at = Some(now);
            }
            return Ok(updated);
        }
        let content = serde_json::to_string(&envelope)
            .map_err(|error| SessionThreadError::Serialization(error.to_string()))?;
        let now = Utc::now();
        let message = ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
            thread_id: request.thread_id.clone(),
            sequence: thread.next_sequence,
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
        thread.next_sequence += 1;
        // Appending a message is thread activity; bump the last-activity
        // stamp so activity-ordered listings surface this thread first.
        thread.record.updated_at = Some(now);
        crate::contract::validate_new_message_timestamps(&message, "tool result reference")?;
        thread.messages.push(message.clone());
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
        let mut state = self.state.lock().await;
        let thread = get_thread_mut(&mut state, &request.scope, &request.thread_id)?;
        for message in thread.messages.iter() {
            if message.kind != MessageKind::CapabilityDisplayPreview
                || message.status != MessageStatus::Finalized
                || message.turn_run_id.as_deref() != Some(request.turn_run_id.as_str())
            {
                continue;
            }
            if CapabilityDisplayPreviewEnvelope::invocation_id_from_json(message.content.as_deref())
                .map_err(SessionThreadError::Serialization)?
                == Some(request.preview.invocation_id)
            {
                return Ok(message.clone());
            }
        }
        let content = serde_json::to_string(&request.preview)
            .map_err(|error| SessionThreadError::Serialization(error.to_string()))?;
        let now = Utc::now();
        let message = ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
            thread_id: request.thread_id.clone(),
            sequence: thread.next_sequence,
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
        thread.next_sequence += 1;
        // Appending a message is thread activity; bump the last-activity
        // stamp so activity-ordered listings surface this thread first.
        thread.record.updated_at = Some(now);
        crate::contract::validate_new_message_timestamps(&message, "capability display preview")?;
        thread.messages.push(message.clone());
        Ok(message)
    }

    async fn update_tool_result_reference(
        &self,
        request: UpdateToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let now = Utc::now();
        let mut state = self.state.lock().await;
        let thread = get_thread_mut(&mut state, &request.scope, &request.thread_id)?;
        let updated = {
            let matches_request = |message: &ThreadMessageRecord| {
                message.kind == MessageKind::ToolResultReference
                    && message.status == MessageStatus::Finalized
                    && message.turn_run_id.as_deref() == Some(request.turn_run_id.as_str())
                    && message.tool_result_ref.as_deref() == Some(request.result_ref.as_str())
            };
            let message_index = match request.provider_call_id.as_deref() {
                Some(requested) => thread
                    .messages
                    .iter()
                    .position(|message| {
                        matches_request(message)
                            && message
                                .tool_result_provider_call
                                .as_ref()
                                .is_some_and(|existing| existing.provider_call_id == requested)
                    })
                    .or_else(|| {
                        thread.messages.iter().position(|message| {
                            matches_request(message) && message.tool_result_provider_call.is_none()
                        })
                    }),
                None => thread.messages.iter().position(matches_request),
            }
            .ok_or_else(|| {
                SessionThreadError::Backend(format!(
                    "tool result reference {} was not found in thread {}",
                    request.result_ref, request.thread_id
                ))
            })?;
            let message = thread
                .messages
                .get_mut(message_index)
                .ok_or_else(|| SessionThreadError::Backend("tool result index vanished".into()))?;
            let before_created_at = message.created_at;
            let before_updated_at = message.updated_at;
            let content = message.content.as_deref().ok_or_else(|| {
                SessionThreadError::Serialization(
                    "tool result reference content is missing".to_string(),
                )
            })?;
            let envelope = ToolResultReferenceEnvelope::from_json_str(content)
                .map_err(SessionThreadError::Serialization)?
                .with_safe_summary(request.safe_summary);
            message.content = Some(
                serde_json::to_string(&envelope)
                    .map_err(|error| SessionThreadError::Serialization(error.to_string()))?,
            );
            message.updated_at = Some(now);
            crate::contract::validate_message_timestamp_fields_not_cleared(
                message.message_id,
                before_created_at,
                before_updated_at,
                message.created_at,
                message.updated_at,
                "update_tool_result_reference",
            )?;
            message.clone()
        };
        thread.record.updated_at = Some(now);
        Ok(updated)
    }

    async fn put_tool_result_record(
        &self,
        request: PutToolResultRecordRequest,
    ) -> Result<(), SessionThreadError> {
        validate_tool_result_record_ref(&request.result_ref)?;
        validate_tool_result_record_content(&request.content)?;
        let mut state = self.state.lock().await;
        let thread = get_thread_mut(&mut state, &request.scope, &request.thread_id)?;
        match thread.tool_result_records.get(&request.result_ref) {
            Some(existing) if existing == &request.content => Ok(()),
            Some(_) => Err(SessionThreadError::Backend(
                "tool result record conflicts with existing content".to_string(),
            )),
            None => {
                thread
                    .tool_result_records
                    .insert(request.result_ref, request.content);
                Ok(())
            }
        }
    }

    async fn read_tool_result_record(
        &self,
        request: ReadToolResultRecordRequest,
    ) -> Result<Option<ToolResultRecordRead>, SessionThreadError> {
        validate_tool_result_record_ref(&request.result_ref)?;
        let state = self.state.lock().await;
        let thread = get_thread(&state, &request.scope, &request.thread_id)?;
        thread
            .tool_result_records
            .get(&request.result_ref)
            .map(|content| {
                crate::tool_result_records::read_tool_result_record(
                    content,
                    &request.result_ref,
                    request.offset,
                    request.max_bytes,
                    &request.selection,
                )
            })
            .transpose()
    }

    async fn update_tool_result_record(
        &self,
        request: UpdateToolResultRecordRequest,
    ) -> Result<(), SessionThreadError> {
        validate_tool_result_record_ref(&request.result_ref)?;
        validate_tool_result_record_content(&request.content)?;
        let mut state = self.state.lock().await;
        let thread = get_thread_mut(&mut state, &request.scope, &request.thread_id)?;
        let Some(existing) = thread.tool_result_records.get_mut(&request.result_ref) else {
            return Err(SessionThreadError::Backend(
                "tool result record was not found in thread".to_string(),
            ));
        };
        *existing = request.content;
        Ok(())
    }

    async fn delete_tool_result_record(
        &self,
        request: DeleteToolResultRecordRequest,
    ) -> Result<(), SessionThreadError> {
        validate_tool_result_record_ref(&request.result_ref)?;
        let mut state = self.state.lock().await;
        let thread = get_thread_mut(&mut state, &request.scope, &request.thread_id)?;
        thread.tool_result_records.remove(&request.result_ref);
        Ok(())
    }

    async fn update_assistant_draft(
        &self,
        request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let now = Utc::now();
        let mut state = self.state.lock().await;
        let thread = get_thread_mut(&mut state, &request.scope, &request.thread_id)?;
        let updated = {
            let message = thread
                .messages
                .iter_mut()
                .find(|message| message.message_id == request.message_id)
                .ok_or(SessionThreadError::UnknownMessage {
                    message_id: request.message_id,
                })?;
            ensure_draft(message)?;
            let before_created_at = message.created_at;
            let before_updated_at = message.updated_at;
            message.content = Some(request.content.into_text());
            // Keep content and attachments in lockstep (as redaction does): an
            // assistant draft carries no attachments, so a content update must not
            // leave stale refs behind if a future draft path ever sets them.
            message.attachments = Vec::new();
            message.updated_at = Some(now);
            crate::contract::validate_message_timestamp_fields_not_cleared(
                message.message_id,
                before_created_at,
                before_updated_at,
                message.created_at,
                message.updated_at,
                "update_assistant_draft",
            )?;
            message.clone()
        };
        thread.record.updated_at = Some(now);
        Ok(updated)
    }

    async fn finalize_assistant_message(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let (content, attachments) = content.into_parts();
        crate::contract::validate_attachment_refs(&attachments)?;
        let mut state = self.state.lock().await;
        let thread = get_thread_mut(&mut state, scope, thread_id)?;
        let message = thread
            .messages
            .iter_mut()
            .find(|message| message.message_id == message_id)
            .ok_or(SessionThreadError::UnknownMessage { message_id })?;
        ensure_draft(message)?;
        let now = Utc::now();
        let before_created_at = message.created_at;
        let before_updated_at = message.updated_at;
        message.status = MessageStatus::Finalized;
        message.content = Some(content);
        message.attachments = attachments;
        message.updated_at = Some(now);
        crate::contract::validate_message_timestamp_fields_not_cleared(
            message.message_id,
            before_created_at,
            before_updated_at,
            message.created_at,
            message.updated_at,
            "finalize_assistant_message",
        )?;
        thread.record.updated_at = Some(now);
        Ok(message.clone())
    }

    async fn redact_message(
        &self,
        request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let now = Utc::now();
        let mut state = self.state.lock().await;
        let thread = get_thread_mut(&mut state, &request.scope, &request.thread_id)?;
        let updated = {
            let message = thread
                .messages
                .iter_mut()
                .find(|message| message.message_id == request.message_id)
                .ok_or(SessionThreadError::UnknownMessage {
                    message_id: request.message_id,
                })?;
            let before_created_at = message.created_at;
            let before_updated_at = message.updated_at;
            message.status = MessageStatus::Redacted;
            message.content = None;
            message.attachments = Vec::new();
            message.tool_result_provider_call = None;
            message.redaction_ref = Some(request.redaction_ref);
            message.updated_at = Some(now);
            crate::contract::validate_message_timestamp_fields_not_cleared(
                message.message_id,
                before_created_at,
                before_updated_at,
                message.created_at,
                message.updated_at,
                "redact_message",
            )?;
            message.clone()
        };
        thread.record.updated_at = Some(now);
        Ok(updated)
    }

    async fn load_context_window(
        &self,
        request: LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        let state = self.state.lock().await;
        let thread = get_thread(&state, &request.scope, &request.thread_id)?;
        let messages = context_messages_with_summary_replacements(thread);
        let (messages, recent_window_truncation) =
            crate::contract::truncate_context_window(messages, request.max_messages);
        Ok(ContextWindow {
            thread_id: request.thread_id,
            messages,
            recent_window_truncation,
        })
    }

    async fn load_context_messages(
        &self,
        request: LoadContextMessagesRequest,
    ) -> Result<ContextMessages, SessionThreadError> {
        let state = self.state.lock().await;
        let thread = get_thread(&state, &request.scope, &request.thread_id)?;
        Ok(ContextMessages {
            thread_id: request.thread_id,
            messages: context_messages_by_id(thread, &request.message_ids),
        })
    }

    async fn list_thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        let state = self.state.lock().await;
        let thread = get_thread(&state, &request.scope, &request.thread_id)?;
        Ok(ThreadHistory {
            thread: thread.record.clone(),
            messages: history_messages(thread),
            summary_artifacts: history_summary_artifacts(thread),
        })
    }

    async fn list_thread_messages_bounded(
        &self,
        request: BoundedThreadMessagesRequest,
    ) -> Result<BoundedThreadMessages, SessionThreadError> {
        let state = self.state.lock().await;
        let thread = get_thread(&state, &request.scope, &request.thread_id)?;
        if thread.messages.len() > request.max_messages {
            return Ok(BoundedThreadMessages::LimitExceeded);
        }
        let mut bytes = 0_usize;
        for message in &thread.messages {
            bytes = bytes.saturating_add(serialize_stored_thread_message(message)?.len());
            if bytes > request.max_bytes {
                return Ok(BoundedThreadMessages::LimitExceeded);
            }
        }
        let message_ids = thread
            .messages
            .iter()
            .map(|message| message.message_id)
            .collect::<Vec<_>>();
        Ok(BoundedThreadMessages::Complete(Box::new(
            BoundedThreadMessageSnapshot {
                history: ThreadMessageRange {
                    thread: thread.record.clone(),
                    messages: history_messages(thread),
                },
                context: ContextMessages {
                    thread_id: request.thread_id,
                    messages: context_messages_by_id(thread, &message_ids),
                },
            },
        )))
    }

    async fn list_thread_messages_range(
        &self,
        request: ThreadMessageRangeRequest,
    ) -> Result<ThreadMessageRange, SessionThreadError> {
        let state = self.state.lock().await;
        let thread = get_thread(&state, &request.scope, &request.thread_id)?;
        Ok(ThreadMessageRange {
            thread: thread.record.clone(),
            messages: thread
                .messages
                .iter()
                .filter(|message| {
                    message.sequence > request.after_sequence
                        && message.sequence <= request.through_sequence
                })
                .map(history_message)
                .collect(),
        })
    }

    async fn latest_thread_message(
        &self,
        request: LatestThreadMessageRequest,
    ) -> Result<Option<ThreadMessageRecord>, SessionThreadError> {
        let state = self.state.lock().await;
        let thread = get_thread(&state, &request.scope, &request.thread_id)?;
        Ok(thread
            .messages
            .iter()
            .rev()
            .find(|message| message.kind == request.kind && message.status == request.status)
            .map(history_message))
    }

    async fn finalized_assistant_message_by_run(
        &self,
        request: crate::FinalizedAssistantMessageByRunRequest,
    ) -> Result<Option<ThreadMessageRecord>, SessionThreadError> {
        let state = self.state.lock().await;
        let thread = get_thread(&state, &request.scope, &request.thread_id)?;
        Ok(thread
            .messages
            .iter()
            .rev()
            .find(|message| {
                message.kind == MessageKind::Assistant
                    && message.status == MessageStatus::Finalized
                    && message.turn_run_id.as_deref() == Some(request.turn_run_id.as_str())
            })
            .map(history_message))
    }

    async fn read_thread(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        let state = self.state.lock().await;
        let thread = get_thread(&state, &request.scope, &request.thread_id)?;
        Ok(thread.record.clone())
    }

    async fn delete_thread(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<(), SessionThreadError> {
        let mut state = self.state.lock().await;
        let existing =
            state
                .threads
                .get(thread_id)
                .ok_or_else(|| SessionThreadError::UnknownThread {
                    thread_id: thread_id.clone(),
                })?;
        if &existing.record.scope != scope {
            return Err(SessionThreadError::UnknownThread {
                thread_id: thread_id.clone(),
            });
        }
        state.threads.remove(thread_id);
        state
            .inbound_idempotency
            .retain(|_, record| &record.thread_id != thread_id);
        // The prepared-context journal rides the thread (the filesystem
        // backend stores it under the thread root, so its delete removes it
        // implicitly); an orphaned record here would replay a deleted thread.
        state.prepared_contexts.remove(thread_id);
        // Structured-finalization evidence is an append-only LLM audit record.
        // It is deliberately retained after transcript deletion; the
        // incarnation in its key prevents a recreated explicit id from
        // reading the predecessor's terminal output.
        Ok(())
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
        let mut state = self.state.lock().await;
        let thread = get_thread_mut(&mut state, &request.scope, &request.thread_id)?;
        let has_start = thread
            .messages
            .iter()
            .any(|message| message.sequence == request.start_sequence);
        let has_end = thread
            .messages
            .iter()
            .any(|message| message.sequence == request.end_sequence);
        if !has_start || !has_end {
            return Err(SessionThreadError::InvalidSummaryRange {
                start_sequence: request.start_sequence,
                end_sequence: request.end_sequence,
            });
        }
        let content = request.content.as_text().to_string();
        if let Some(overlapping) =
            find_overlapping_summary(&thread.summary_artifacts, &request, &content)?
        {
            return Ok(overlapping.clone());
        }
        let artifact = SummaryArtifact {
            summary_id: SummaryArtifactId::new(),
            thread_id: request.thread_id,
            start_sequence: request.start_sequence,
            end_sequence: request.end_sequence,
            summary_kind: request.summary_kind,
            content,
            model_context_policy: request.model_context_policy,
            context_mode: request.context_mode,
        };
        thread.summary_artifacts.push(artifact.clone());
        Ok(artifact)
    }
    async fn list_threads_for_scope(
        &self,
        request: ListThreadsForScopeRequest,
    ) -> Result<ListThreadsForScopeResponse, SessionThreadError> {
        // In-memory enumeration for standalone. Production backends
        // (filesystem / postgres) override with their own pagination
        // strategy; this impl is fine because the store is bounded
        // by tenant memory in the first place.
        let limit = request
            .limit
            .map(|n| (n as usize).clamp(1, LIST_THREADS_MAX_PAGE_SIZE))
            .unwrap_or(LIST_THREADS_DEFAULT_PAGE_SIZE);

        let state = self.state.lock().await;

        // Scope filter is exact equality on the full `ThreadScope`
        // tuple — tenant + agent + project + owner — so a caller
        // cannot see threads owned by other users in the same
        // (tenant, agent, project) triple. The trait contract
        // documents this invariant.
        //
        // Filter before cloning: matching on the borrowed scope avoids
        // cloning records owned by other tenants/projects only to throw
        // them away. The store is bounded by tenant memory so a full
        // scan is still acceptable here; a scope-indexed secondary
        // map would help with very large stores but standalone never
        // gets close to that scale.
        //
        // Derive a sidebar-friendly title from the first user message
        // when the record itself has none. Matches v1's libSQL list
        // semantics: titles aren't stored, they're computed on read
        // from the first user message in the transcript. The
        // filesystem backend does the same thing via
        // `list_thread_messages` + `derive_title_from_message`.
        let mut matching: Vec<SessionThreadRecord> = state
            .threads
            .values()
            .filter(|stored| stored.record.scope == request.scope)
            // Prepared-context (unbound/subagent) threads are working state,
            // not conversations: excluded from listings on every backend.
            .filter(|stored| {
                !crate::prepared_context::record_is_prepared_context_hidden(&stored.record)
            })
            .map(|stored| {
                let mut record = stored.record.clone();
                if record.title.is_none()
                    && let Some(title) = derive_thread_title(&stored.messages)
                {
                    record.title = Some(title);
                }
                record
            })
            .collect();
        // Newest activity first (`updated_at`, falling back to
        // `created_at`); legacy records without timestamps sort last.
        // Tie-break on thread_id ascending so the order is stable and
        // opaque cursors stay resumable, matching the filesystem backend
        // and the web sidebar's `byActivityDesc` comparator.
        matching.sort_by(|a, b| {
            let a_key = a.updated_at.or(a.created_at);
            let b_key = b.updated_at.or(b.created_at);
            std::cmp::Reverse(a_key)
                .cmp(&std::cmp::Reverse(b_key))
                .then_with(|| a.thread_id.as_str().cmp(b.thread_id.as_str()))
        });

        // Opaque cursor is the last thread_id of the previous page; find
        // it in the activity-sorted list and resume after it. A cursor
        // that no longer resolves ends the stream rather than restarting.
        let start_index = match request.cursor.as_deref() {
            Some(cursor) => matching
                .iter()
                .position(|record| record.thread_id.as_str() == cursor)
                .map(|index| index + 1)
                .unwrap_or(matching.len()),
            None => 0,
        };
        let end_index = start_index.saturating_add(limit).min(matching.len());
        let next_cursor = if end_index < matching.len() {
            matching[start_index..end_index]
                .last()
                .map(|record| record.thread_id.as_str().to_string())
        } else {
            None
        };
        let page: Vec<SessionThreadRecord> = matching[start_index..end_index].to_vec();

        Ok(ListThreadsForScopeResponse {
            threads: page,
            next_cursor,
        })
    }

    async fn read_thread_by_id(
        &self,
        thread_id: ThreadId,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        let state = self.state.lock().await;
        state
            .threads
            .get(&thread_id)
            .map(|thread| thread.record.clone())
            .ok_or(SessionThreadError::UnknownThread { thread_id })
    }

    fn supports_resolve_scope(&self) -> bool {
        true
    }

    async fn resolve_scope(&self, thread_id: ThreadId) -> Result<ThreadScope, SessionThreadError> {
        self.read_thread_by_id(thread_id)
            .await
            .map(|thread| thread.scope)
    }

    async fn update_thread_goal(
        &self,
        request: crate::UpdateThreadGoalRequest,
    ) -> Result<crate::ThreadGoal, SessionThreadError> {
        let mut state = self.state.lock().await;
        let thread =
            state
                .threads
                .get_mut(&request.thread_id)
                .ok_or(SessionThreadError::UnknownThread {
                    thread_id: request.thread_id,
                })?;
        thread.record.goal = Some(request.goal.clone());
        Ok(request.goal)
    }
}

impl InMemorySessionThreadService {
    /// Test-only back-door: force a message's status to `DeferredBusy` so
    /// that legacy-row read/replay tests can construct pre-existing
    /// `DeferredBusy` rows without going through the now-retired
    /// `mark_message_deferred_busy` writer.  Never call from production code.
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
        let mut state = self.state.lock().await;
        let message = get_message_mut(&mut state, scope, thread_id, message_id)?;
        message.status = MessageStatus::DeferredBusy;
        message.turn_id = None;
        message.turn_run_id = None;
        Ok(message.clone())
    }
}

/// Default page size when the caller omits `limit`.
const LIST_THREADS_DEFAULT_PAGE_SIZE: usize = 50;
/// Maximum page size — caller-supplied `limit` is clamped here so a
/// huge value cannot widen the response unboundedly.
const LIST_THREADS_MAX_PAGE_SIZE: usize = 200;

fn generated_thread_id() -> Result<ThreadId, SessionThreadError> {
    ThreadId::new(Uuid::new_v4().to_string())
        .map_err(|error| SessionThreadError::GeneratedThreadId(error.to_string()))
}

fn get_thread<'a>(
    state: &'a InMemoryState,
    scope: &ThreadScope,
    thread_id: &ThreadId,
) -> Result<&'a StoredThread, SessionThreadError> {
    let thread = state
        .threads
        .get(thread_id)
        .ok_or_else(|| SessionThreadError::UnknownThread {
            thread_id: thread_id.clone(),
        })?;
    if &thread.record.scope != scope {
        return Err(SessionThreadError::UnknownThread {
            thread_id: thread_id.clone(),
        });
    }
    Ok(thread)
}

fn get_thread_mut<'a>(
    state: &'a mut InMemoryState,
    scope: &ThreadScope,
    thread_id: &ThreadId,
) -> Result<&'a mut StoredThread, SessionThreadError> {
    let thread =
        state
            .threads
            .get_mut(thread_id)
            .ok_or_else(|| SessionThreadError::UnknownThread {
                thread_id: thread_id.clone(),
            })?;
    if &thread.record.scope != scope {
        return Err(SessionThreadError::UnknownThread {
            thread_id: thread_id.clone(),
        });
    }
    Ok(thread)
}

fn get_message_mut<'a>(
    state: &'a mut InMemoryState,
    scope: &ThreadScope,
    thread_id: &ThreadId,
    message_id: ThreadMessageId,
) -> Result<&'a mut ThreadMessageRecord, SessionThreadError> {
    let thread = get_thread_mut(state, scope, thread_id)?;
    thread
        .messages
        .iter_mut()
        .find(|message| message.message_id == message_id)
        .ok_or(SessionThreadError::UnknownMessage { message_id })
}

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
            MessageStatus::Accepted | MessageStatus::DeferredBusy | MessageStatus::Queued
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

fn context_messages_with_summary_replacements(thread: &StoredThread) -> Vec<ContextMessage> {
    if let Some(barrier) = newest_context_barrier(&thread.summary_artifacts, |summary| {
        !summary_covers_hidden_content(thread, summary)
    }) {
        let mut context = vec![ContextMessage {
            message_id: None,
            summary_id: Some(barrier.summary_id),
            sequence: barrier.end_sequence,
            kind: MessageKind::Summary,
            tool_result_provider_call: None,
            content: barrier.content.clone(),
            image_attachments: Vec::new(),
        }];
        context.extend(
            thread
                .messages
                .iter()
                .filter(|message| {
                    message.sequence > barrier.end_sequence && is_model_context_visible(message)
                })
                .filter_map(|message| {
                    let content = message.content.clone()?;
                    Some(ContextMessage::from_transcript_message(message, content))
                }),
        );
        return context;
    }

    let replacement_summaries = sorted_context_summaries(&thread.summary_artifacts, |summary| {
        summary.model_context_policy == Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected)
            && !summary_covers_hidden_content(thread, summary)
    });
    let mut skip_through = 0;
    let mut emitted_summaries = HashSet::new();
    let mut context = Vec::new();
    for message in thread
        .messages
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
    thread: &StoredThread,
    message_ids: &[ThreadMessageId],
) -> Vec<ContextMessage> {
    let visible_messages = thread
        .messages
        .iter()
        .filter(|message| is_model_context_visible(message))
        .map(|message| (message.message_id, message))
        .collect::<HashMap<_, _>>();
    message_ids
        .iter()
        .filter_map(|message_id| {
            let message = visible_messages.get(message_id)?;
            let content = message.content.clone()?;
            Some(ContextMessage::from_transcript_message(message, content))
        })
        .collect()
}

const REDACTED_SUMMARY_CONTENT: &str = "[redacted]";

fn history_summary_artifacts(thread: &StoredThread) -> Vec<SummaryArtifact> {
    thread
        .summary_artifacts
        .iter()
        .map(|summary| {
            if summary_covers_redacted_or_deleted_content(thread, summary) {
                let mut redacted = summary.clone();
                redacted.content = REDACTED_SUMMARY_CONTENT.to_string();
                redacted.model_context_policy = None;
                redacted
            } else {
                summary.clone()
            }
        })
        .collect()
}

fn history_messages(thread: &StoredThread) -> Vec<ThreadMessageRecord> {
    thread.messages.iter().map(history_message).collect()
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

/// Returns true when a non-model-context-visible message within the summary
/// span could later become model-visible (i.e. it is in a resurfaceable pending
/// state).  Permanently-terminal non-visible messages (RejectedBusy, capability
/// previews) never resurface, so a compaction summary spanning them is safe to
/// apply — blocking it would silently drop a legitimate compacted range.
///
/// Resurfaceable statuses (must still block the summary):
///   Draft | Interrupted | Superseded | Queued | DeferredBusy
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
            | MessageStatus::Queued
            | MessageStatus::DeferredBusy
    )
}

fn summary_covers_hidden_content(thread: &StoredThread, summary: &SummaryArtifact) -> bool {
    thread.messages.iter().any(|message| {
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
    thread: &StoredThread,
    summary: &SummaryArtifact,
) -> bool {
    thread.messages.iter().any(|message| {
        summary.start_sequence <= message.sequence
            && message.sequence <= summary.end_sequence
            && matches!(
                message.status,
                MessageStatus::Redacted | MessageStatus::Deleted
            )
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
