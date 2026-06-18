use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::ThreadId;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::identifiers::SummaryArtifactId;
use crate::summary_artifacts::find_overlapping_summary;
use crate::title::derive_thread_title;
use crate::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
    AppendAssistantDraftRequest, AppendCapabilityDisplayPreviewRequest,
    AppendToolResultReferenceRequest, CapabilityDisplayPreviewEnvelope, ContextMessage,
    ContextMessages, ContextWindow, CreateSummaryArtifactRequest, EnsureThreadRequest,
    LatestThreadMessageRequest, ListThreadsForScopeRequest, ListThreadsForScopeResponse,
    LoadContextMessagesRequest, LoadContextWindowRequest, MessageContent, MessageKind,
    MessageStatus, RedactMessageRequest, ReplayAcceptedInboundMessageRequest, SessionThreadError,
    SessionThreadRecord, SessionThreadService, SummaryArtifact, SummaryModelContextPolicy,
    ThreadHistory, ThreadHistoryRequest, ThreadMessageId, ThreadMessageRange,
    ThreadMessageRangeRequest, ThreadMessageRecord, ThreadScope, ToolResultReferenceEnvelope,
    UpdateAssistantDraftRequest, UpdateToolResultReferenceRequest,
};

#[derive(Debug, Clone, Default)]
pub struct InMemorySessionThreadService {
    state: Arc<Mutex<InMemoryState>>,
}

#[derive(Debug, Default)]
struct InMemoryState {
    threads: HashMap<ThreadId, StoredThread>,
    inbound_idempotency: HashMap<InboundIdempotencyKey, InboundIdempotencyRecord>,
}

#[derive(Debug, Clone)]
struct StoredThread {
    record: SessionThreadRecord,
    messages: Vec<ThreadMessageRecord>,
    summary_artifacts: Vec<SummaryArtifact>,
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
        let record = SessionThreadRecord {
            scope: request.scope,
            thread_id: thread_id.clone(),
            created_by_actor_id: request.created_by_actor_id,
            title: request.title,
            metadata_json: request.metadata_json,
            goal: None,
            created_at: Some(now),
            updated_at: Some(now),
        };
        state.threads.insert(
            thread_id,
            StoredThread {
                record: record.clone(),
                messages: Vec::new(),
                summary_artifacts: Vec::new(),
                next_sequence: 1,
            },
        );
        Ok(record)
    }

    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
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
        thread.record.updated_at = Some(Utc::now());
        thread.messages.push(ThreadMessageRecord {
            message_id,
            thread_id: request.thread_id.clone(),
            sequence,
            kind: MessageKind::User,
            status: MessageStatus::Accepted,
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
        });

        if let Some(key) = key {
            state.inbound_idempotency.insert(
                key,
                InboundIdempotencyRecord {
                    thread_id: request.thread_id.clone(),
                    message_id,
                },
            );
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
        let message = get_message_mut(&mut state, scope, thread_id, message_id)?;
        ensure_user_accepted(message, "mark_message_submitted")?;
        message.status = MessageStatus::Submitted;
        message.turn_id = Some(turn_id);
        message.turn_run_id = Some(turn_run_id);
        Ok(message.clone())
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
        message.status = MessageStatus::RejectedBusy;
        message.turn_id = None;
        message.turn_run_id = None;
        Ok(message.clone())
    }

    async fn append_assistant_draft(
        &self,
        request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let mut state = self.state.lock().await;
        let thread = get_thread_mut(&mut state, &request.scope, &request.thread_id)?;
        if let Some(existing) = thread.messages.iter().find(|message| {
            message.kind == MessageKind::Assistant
                && message.turn_run_id.as_deref() == Some(request.turn_run_id.as_str())
        }) {
            return Ok(existing.clone());
        }
        let message_id = ThreadMessageId::new();
        let message = ThreadMessageRecord {
            message_id,
            thread_id: request.thread_id.clone(),
            sequence: thread.next_sequence,
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
            attachments: Vec::new(),
            redaction_ref: None,
        };
        thread.next_sequence += 1;
        // Appending a message is thread activity; bump the last-activity
        // stamp so activity-ordered listings surface this thread first.
        thread.record.updated_at = Some(Utc::now());
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
        let envelope = ToolResultReferenceEnvelope::new_best_effort_model_observation(
            request.result_ref,
            request.safe_summary,
            request.model_observation,
        )
        .map_err(SessionThreadError::Serialization)?;
        if let Some(existing) = thread.messages.iter_mut().find(|message| {
            message.kind == MessageKind::ToolResultReference
                && message.status == MessageStatus::Finalized
                && message.turn_run_id.as_deref() == Some(request.turn_run_id.as_str())
                && message.tool_result_ref.as_deref() == Some(envelope.result_ref.as_str())
        }) {
            if let Some(provider_call) = provider_call.as_ref() {
                provider_call
                    .validate()
                    .map_err(SessionThreadError::Serialization)?;
                match existing.tool_result_provider_call.as_ref() {
                    None => existing.tool_result_provider_call = Some(provider_call.clone()),
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
                }
            }
            return Ok(existing.clone());
        }
        if let Some(provider_call) = &provider_call {
            provider_call
                .validate()
                .map_err(SessionThreadError::Serialization)?;
        }
        let content = serde_json::to_string(&envelope)
            .map_err(|error| SessionThreadError::Serialization(error.to_string()))?;
        let message = ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
            thread_id: request.thread_id.clone(),
            sequence: thread.next_sequence,
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
            attachments: Vec::new(),
            redaction_ref: None,
        };
        thread.next_sequence += 1;
        // Appending a message is thread activity; bump the last-activity
        // stamp so activity-ordered listings surface this thread first.
        thread.record.updated_at = Some(Utc::now());
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
        let message = ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
            thread_id: request.thread_id.clone(),
            sequence: thread.next_sequence,
            kind: MessageKind::CapabilityDisplayPreview,
            status: MessageStatus::Finalized,
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
        thread.record.updated_at = Some(Utc::now());
        thread.messages.push(message.clone());
        Ok(message)
    }

    async fn update_tool_result_reference(
        &self,
        request: UpdateToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let mut state = self.state.lock().await;
        let thread = get_thread_mut(&mut state, &request.scope, &request.thread_id)?;
        let message = thread
            .messages
            .iter_mut()
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
        Ok(message.clone())
    }

    async fn update_assistant_draft(
        &self,
        request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let mut state = self.state.lock().await;
        let message = get_message_mut(
            &mut state,
            &request.scope,
            &request.thread_id,
            request.message_id,
        )?;
        ensure_draft(message)?;
        message.content = Some(request.content.into_text());
        // Keep content and attachments in lockstep (as redaction does): an
        // assistant draft carries no attachments, so a content update must not
        // leave stale refs behind if a future draft path ever sets them.
        message.attachments = Vec::new();
        Ok(message.clone())
    }

    async fn finalize_assistant_message(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let mut state = self.state.lock().await;
        let message = get_message_mut(&mut state, scope, thread_id, message_id)?;
        ensure_draft(message)?;
        message.status = MessageStatus::Finalized;
        message.content = Some(content.into_text());
        message.attachments = Vec::new();
        Ok(message.clone())
    }

    async fn redact_message(
        &self,
        request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let mut state = self.state.lock().await;
        let message = get_message_mut(
            &mut state,
            &request.scope,
            &request.thread_id,
            request.message_id,
        )?;
        message.status = MessageStatus::Redacted;
        message.content = None;
        message.attachments = Vec::new();
        message.tool_result_provider_call = None;
        message.redaction_ref = Some(request.redaction_ref);
        Ok(message.clone())
    }

    async fn load_context_window(
        &self,
        request: LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        let state = self.state.lock().await;
        let thread = get_thread(&state, &request.scope, &request.thread_id)?;
        let mut messages = context_messages_with_summary_replacements(thread);
        if request.max_messages < messages.len() {
            let start = messages.len() - request.max_messages;
            messages = messages.split_off(start);
        }
        Ok(ContextWindow {
            thread_id: request.thread_id,
            messages,
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
        };
        thread.summary_artifacts.push(artifact.clone());
        Ok(artifact)
    }
    async fn list_threads_for_scope(
        &self,
        request: ListThreadsForScopeRequest,
    ) -> Result<ListThreadsForScopeResponse, SessionThreadError> {
        // In-memory enumeration for local-dev. Production backends
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
        // map would help with very large stores but local-dev never
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

fn context_messages_with_summary_replacements(thread: &StoredThread) -> Vec<ContextMessage> {
    let replacement_summaries = thread
        .summary_artifacts
        .iter()
        .filter(|summary| {
            summary.model_context_policy
                == Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected)
                && !summary_covers_hidden_content(thread, summary)
        })
        .collect::<Vec<_>>();
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
