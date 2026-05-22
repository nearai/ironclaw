use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use async_trait::async_trait;
use ironclaw_host_api::ThreadId;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::identifiers::SummaryArtifactId;
use crate::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
    AppendAssistantDraftRequest, AppendToolResultReferenceRequest, ContextMessage, ContextMessages,
    ContextWindow, CreateSummaryArtifactRequest, EnsureThreadRequest, LoadContextMessagesRequest,
    LoadContextWindowRequest, MessageContent, MessageKind, MessageStatus, RedactMessageRequest,
    ReplayAcceptedInboundMessageRequest, SessionThreadError, SessionThreadRecord,
    SessionThreadService, SummaryArtifact, ThreadHistory, ThreadHistoryRequest, ThreadMessageId,
    ThreadMessageRecord, ThreadScope, ToolResultReferenceEnvelope, UpdateAssistantDraftRequest,
    UpdateToolResultReferenceRequest,
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

        let record = SessionThreadRecord {
            scope: request.scope,
            thread_id: thread_id.clone(),
            created_by_actor_id: request.created_by_actor_id,
            title: request.title,
            metadata_json: request.metadata_json,
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
        let sequence = thread.next_sequence;
        thread.next_sequence += 1;
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
            content: Some(request.content.into_text()),
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

    async fn mark_message_deferred_busy(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let mut state = self.state.lock().await;
        let message = get_message_mut(&mut state, scope, thread_id, message_id)?;
        ensure_user_accepted(message, "mark_message_deferred_busy")?;
        message.status = MessageStatus::DeferredBusy;
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
            redaction_ref: None,
        };
        thread.next_sequence += 1;
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
        let envelope = ToolResultReferenceEnvelope::new(request.result_ref, request.safe_summary)
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
            redaction_ref: None,
        };
        thread.next_sequence += 1;
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
        let envelope = ToolResultReferenceEnvelope::new(request.result_ref, request.safe_summary)
            .map_err(SessionThreadError::Serialization)?;
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

    async fn read_thread(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        let state = self.state.lock().await;
        let thread = get_thread(&state, &request.scope, &request.thread_id)?;
        Ok(thread.record.clone())
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
        if request.model_context_policy.as_deref() == Some("replace_range_when_selected")
            && thread.summary_artifacts.iter().any(|summary| {
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
            thread_id: request.thread_id,
            start_sequence: request.start_sequence,
            end_sequence: request.end_sequence,
            summary_kind: request.summary_kind,
            content: request.content.into_text(),
            model_context_policy: request.model_context_policy,
        };
        thread.summary_artifacts.push(artifact.clone());
        Ok(artifact)
    }
}

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

fn ranges_overlap(left_start: u64, left_end: u64, right_start: u64, right_end: u64) -> bool {
    left_start <= right_end && right_start <= left_end
}

fn context_messages_with_summary_replacements(thread: &StoredThread) -> Vec<ContextMessage> {
    let replacement_summaries = thread
        .summary_artifacts
        .iter()
        .filter(|summary| {
            summary.model_context_policy.as_deref() == Some("replace_range_when_selected")
                && !summary_covers_hidden_content(thread, summary)
        })
        .collect::<Vec<_>>();
    let mut skip_through = 0;
    let mut emitted_summaries = HashSet::new();
    let mut context = Vec::new();
    for message in thread
        .messages
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
    thread: &StoredThread,
    message_ids: &[ThreadMessageId],
) -> Vec<ContextMessage> {
    let visible_messages = thread
        .messages
        .iter()
        .filter(|message| is_model_visible(message.status))
        .map(|message| (message.message_id, message))
        .collect::<HashMap<_, _>>();
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
    thread
        .messages
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

fn summary_covers_hidden_content(thread: &StoredThread, summary: &SummaryArtifact) -> bool {
    thread.messages.iter().any(|message| {
        summary.start_sequence <= message.sequence
            && message.sequence <= summary.end_sequence
            && !is_model_visible(message.status)
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
