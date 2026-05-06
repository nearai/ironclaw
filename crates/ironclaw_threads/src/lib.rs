//! Canonical session thread and transcript contracts for IronClaw Reborn.
//!
//! This crate owns the contract-first boundary for canonical Reborn threads and
//! transcript history. It deliberately starts with an in-memory service so caller
//! tests can lock semantics before PostgreSQL/libSQL adapters are introduced.

use std::{collections::HashMap, fmt, sync::Arc};

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, MissionId, ProjectId, TenantId, ThreadId, UserId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Canonical scope carried by a Reborn session thread.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ThreadScope {
    pub tenant_id: TenantId,
    pub agent_id: AgentId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<UserId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_id: Option<MissionId>,
}

/// Stable canonical transcript message identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ThreadMessageId(Uuid);

impl ThreadMessageId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(value: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(value).map(Self)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for ThreadMessageId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ThreadMessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Stable summary artifact identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SummaryArtifactId(Uuid);

impl SummaryArtifactId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SummaryArtifactId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SummaryArtifactId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// User/model-visible transcript content accepted by this boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageContent {
    text: String,
}

impl MessageContent {
    pub fn text(value: impl Into<String>) -> Self {
        Self { text: value.into() }
    }

    pub fn as_text(&self) -> &str {
        &self.text
    }

    pub fn into_text(self) -> String {
        self.text
    }
}

/// Canonical kind of a transcript message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    User,
    Assistant,
    System,
    Summary,
    CheckpointReference,
    ToolResultReference,
}

/// Explicit transcript status. Callers must not infer this from nullable refs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    Accepted,
    Submitted,
    DeferredBusy,
    Draft,
    Finalized,
    Interrupted,
    Superseded,
    Redacted,
    Deleted,
}

/// Canonical thread metadata returned by the service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionThreadRecord {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub created_by_actor_id: String,
    pub title: Option<String>,
    pub metadata_json: Option<String>,
}

/// Transcript message snapshot for UI/projection reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMessageRecord {
    pub message_id: ThreadMessageId,
    pub thread_id: ThreadId,
    pub sequence: u64,
    pub kind: MessageKind,
    pub status: MessageStatus,
    pub actor_id: Option<String>,
    pub source_binding_id: Option<String>,
    pub reply_target_binding_id: Option<String>,
    pub turn_id: Option<String>,
    pub turn_run_id: Option<String>,
    pub content: Option<String>,
    pub redaction_ref: Option<String>,
}

/// Summary artifact over a stable transcript sequence range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryArtifact {
    pub summary_id: SummaryArtifactId,
    pub thread_id: ThreadId,
    pub start_sequence: u64,
    pub end_sequence: u64,
    pub summary_kind: String,
    pub content: String,
    pub model_context_policy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsureThreadRequest {
    pub scope: ThreadScope,
    pub thread_id: Option<ThreadId>,
    pub created_by_actor_id: String,
    pub title: Option<String>,
    pub metadata_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptInboundMessageRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub actor_id: String,
    pub source_binding_id: Option<String>,
    pub reply_target_binding_id: Option<String>,
    pub external_event_id: Option<String>,
    pub content: MessageContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedInboundMessage {
    pub thread_id: ThreadId,
    pub message_id: ThreadMessageId,
    pub sequence: u64,
    pub idempotent_replay: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendAssistantDraftRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub turn_run_id: String,
    pub content: MessageContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateAssistantDraftRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub message_id: ThreadMessageId,
    pub content: MessageContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactMessageRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub message_id: ThreadMessageId,
    pub redaction_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadHistoryRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadHistory {
    pub thread: SessionThreadRecord,
    pub messages: Vec<ThreadMessageRecord>,
    pub summary_artifacts: Vec<SummaryArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadContextWindowRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub max_messages: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMessage {
    pub message_id: Option<ThreadMessageId>,
    pub summary_id: Option<SummaryArtifactId>,
    pub sequence: u64,
    pub kind: MessageKind,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextWindow {
    pub thread_id: ThreadId,
    pub messages: Vec<ContextMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSummaryArtifactRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub start_sequence: u64,
    pub end_sequence: u64,
    pub summary_kind: String,
    pub content: MessageContent,
    pub model_context_policy: Option<String>,
}

/// Canonical thread/transcript service errors.
#[derive(Debug, Error)]
pub enum SessionThreadError {
    #[error("unknown thread {thread_id}")]
    UnknownThread { thread_id: ThreadId },
    #[error("unknown message {message_id}")]
    UnknownMessage { message_id: ThreadMessageId },
    #[error("thread {thread_id} already exists in a different scope")]
    ThreadScopeMismatch { thread_id: ThreadId },
    #[error("message {message_id} is not an assistant draft")]
    MessageNotDraft { message_id: ThreadMessageId },
    #[error("message {message_id} cannot transition from {from:?} via {attempted}")]
    InvalidMessageTransition {
        message_id: ThreadMessageId,
        from: MessageStatus,
        attempted: &'static str,
    },
    #[error(
        "idempotent inbound event belongs to thread {stored_thread_id}, not requested thread {requested_thread_id}"
    )]
    IdempotentReplayThreadMismatch {
        stored_thread_id: ThreadId,
        requested_thread_id: ThreadId,
    },
    #[error("invalid summary range {start_sequence}..={end_sequence}")]
    InvalidSummaryRange {
        start_sequence: u64,
        end_sequence: u64,
    },
    #[error("failed to create generated thread id: {0}")]
    GeneratedThreadId(String),
}

/// Canonical Reborn session thread and transcript boundary.
#[async_trait]
pub trait SessionThreadService: Send + Sync {
    async fn ensure_thread(
        &self,
        request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError>;

    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError>;

    async fn mark_message_submitted(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        turn_id: String,
        turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError>;

    async fn mark_message_deferred_busy(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError>;

    async fn append_assistant_draft(
        &self,
        request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError>;

    async fn update_assistant_draft(
        &self,
        request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError>;

    async fn finalize_assistant_message(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError>;

    async fn redact_message(
        &self,
        request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError>;

    async fn load_context_window(
        &self,
        request: LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError>;

    async fn list_thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError>;

    async fn create_summary_artifact(
        &self,
        request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError>;
}

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
    tenant_id: TenantId,
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
            tenant_id: request.scope.tenant_id.clone(),
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
            content: Some(request.content.into_text()),
            redaction_ref: None,
        };
        thread.next_sequence += 1;
        thread.messages.push(message.clone());
        Ok(message)
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

    async fn list_thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        let state = self.state.lock().await;
        let thread = get_thread(&state, &request.scope, &request.thread_id)?;
        Ok(ThreadHistory {
            thread: thread.record.clone(),
            messages: thread.messages.clone(),
            summary_artifacts: thread.summary_artifacts.clone(),
        })
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
    if message.kind == MessageKind::User && message.status == MessageStatus::Accepted {
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
            summary.model_context_policy.as_deref() == Some("replace_range_when_selected")
        })
        .collect::<Vec<_>>();
    let mut skip_through = 0;
    let mut context = Vec::new();
    for message in thread
        .messages
        .iter()
        .filter(|message| is_model_visible(message.status))
    {
        if message.sequence <= skip_through {
            continue;
        }
        if let Some(summary) = replacement_summaries
            .iter()
            .find(|summary| summary.start_sequence == message.sequence)
        {
            context.push(ContextMessage {
                message_id: None,
                summary_id: Some(summary.summary_id),
                sequence: summary.start_sequence,
                kind: MessageKind::Summary,
                content: summary.content.clone(),
            });
            skip_through = summary.end_sequence;
            continue;
        }
        if let Some(content) = message.content.clone() {
            context.push(ContextMessage {
                message_id: Some(message.message_id),
                summary_id: None,
                sequence: message.sequence,
                kind: message.kind,
                content,
            });
        }
    }
    context
}

fn is_model_visible(status: MessageStatus) -> bool {
    matches!(
        status,
        MessageStatus::Accepted | MessageStatus::Submitted | MessageStatus::Finalized
    )
}
