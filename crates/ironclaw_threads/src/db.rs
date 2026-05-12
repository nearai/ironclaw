use std::collections::HashMap;

use async_trait::async_trait;
use ironclaw_host_api::ThreadId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
    AppendAssistantDraftRequest, ContextMessage, ContextWindow, CreateSummaryArtifactRequest,
    EnsureThreadRequest, LoadContextWindowRequest, MessageContent, MessageKind, MessageStatus,
    RedactMessageRequest, ReplayAcceptedInboundMessageRequest, SessionThreadError,
    SessionThreadRecord, SessionThreadService, SummaryArtifact, SummaryArtifactId, ThreadHistory,
    ThreadHistoryRequest, ThreadMessageId, ThreadMessageRecord, ThreadScope,
    UpdateAssistantDraftRequest,
};

#[cfg(feature = "libsql")]
use std::sync::Arc;

#[cfg(feature = "libsql")]
const LIBSQL_SESSION_THREAD_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reborn_session_thread_records (
    thread_id TEXT PRIMARY KEY,
    scope_key TEXT NOT NULL,
    next_sequence INTEGER NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_reborn_session_thread_records_scope
    ON reborn_session_thread_records(scope_key);

CREATE TABLE IF NOT EXISTS reborn_thread_message_records (
    message_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    scope_key TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    turn_run_id TEXT,
    payload TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_reborn_thread_message_records_thread_sequence
    ON reborn_thread_message_records(thread_id, sequence);
CREATE INDEX IF NOT EXISTS idx_reborn_thread_message_records_thread
    ON reborn_thread_message_records(thread_id, sequence);
CREATE INDEX IF NOT EXISTS idx_reborn_thread_message_records_turn_run
    ON reborn_thread_message_records(thread_id, turn_run_id)
    WHERE turn_run_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS reborn_thread_summary_artifacts (
    summary_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    scope_key TEXT NOT NULL,
    start_sequence INTEGER NOT NULL,
    end_sequence INTEGER NOT NULL,
    model_context_policy TEXT,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_reborn_thread_summary_artifacts_thread
    ON reborn_thread_summary_artifacts(thread_id, start_sequence, end_sequence);

CREATE TABLE IF NOT EXISTS reborn_thread_inbound_idempotency (
    record_key TEXT PRIMARY KEY,
    scope_key TEXT NOT NULL,
    source_binding_id TEXT NOT NULL,
    external_event_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_reborn_thread_inbound_idempotency_scope
    ON reborn_thread_inbound_idempotency(scope_key);
"#;

#[cfg(feature = "postgres")]
const POSTGRES_SESSION_THREAD_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reborn_session_thread_records (
    thread_id TEXT PRIMARY KEY,
    scope_key TEXT NOT NULL,
    next_sequence BIGINT NOT NULL,
    payload JSONB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_reborn_session_thread_records_scope
    ON reborn_session_thread_records(scope_key);

CREATE TABLE IF NOT EXISTS reborn_thread_message_records (
    message_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    scope_key TEXT NOT NULL,
    sequence BIGINT NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    turn_run_id TEXT,
    payload JSONB NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_reborn_thread_message_records_thread_sequence
    ON reborn_thread_message_records(thread_id, sequence);
CREATE INDEX IF NOT EXISTS idx_reborn_thread_message_records_thread
    ON reborn_thread_message_records(thread_id, sequence);
CREATE INDEX IF NOT EXISTS idx_reborn_thread_message_records_turn_run
    ON reborn_thread_message_records(thread_id, turn_run_id)
    WHERE turn_run_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS reborn_thread_summary_artifacts (
    summary_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    scope_key TEXT NOT NULL,
    start_sequence BIGINT NOT NULL,
    end_sequence BIGINT NOT NULL,
    model_context_policy TEXT,
    payload JSONB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_reborn_thread_summary_artifacts_thread
    ON reborn_thread_summary_artifacts(thread_id, start_sequence, end_sequence);

CREATE TABLE IF NOT EXISTS reborn_thread_inbound_idempotency (
    record_key TEXT PRIMARY KEY,
    scope_key TEXT NOT NULL,
    source_binding_id TEXT NOT NULL,
    external_event_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    payload JSONB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_reborn_thread_inbound_idempotency_scope
    ON reborn_thread_inbound_idempotency(scope_key);
"#;

/// libSQL-backed canonical session thread and transcript service.
#[cfg(feature = "libsql")]
pub struct LibSqlSessionThreadService {
    db: Arc<libsql::Database>,
}

#[cfg(feature = "libsql")]
impl LibSqlSessionThreadService {
    pub fn new(db: Arc<libsql::Database>) -> Self {
        Self { db }
    }

    pub async fn run_migrations(&self) -> Result<(), SessionThreadError> {
        let conn = self.connect().await?;
        conn.execute_batch(LIBSQL_SESSION_THREAD_SCHEMA)
            .await
            .map_err(db_error)?;
        Ok(())
    }

    async fn connect(&self) -> Result<libsql::Connection, SessionThreadError> {
        let conn = self.db.connect().map_err(db_error)?;
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(db_error)?;
        Ok(conn)
    }

    async fn mutate<T>(
        &self,
        operation: impl FnOnce(&mut DurableState) -> Result<T, SessionThreadError>,
    ) -> Result<T, SessionThreadError> {
        let conn = self.connect().await?;
        conn.execute("BEGIN IMMEDIATE", ())
            .await
            .map_err(db_error)?;
        let result = async {
            let mut state = libsql_load_state(&conn).await?;
            let output = operation(&mut state)?;
            libsql_replace_state(&conn, &state).await?;
            Ok(output)
        }
        .await;
        finish_libsql_transaction(&conn, result).await
    }

    async fn read<T>(
        &self,
        operation: impl FnOnce(&DurableState) -> Result<T, SessionThreadError>,
    ) -> Result<T, SessionThreadError> {
        let conn = self.connect().await?;
        conn.execute("BEGIN", ()).await.map_err(db_error)?;
        let result = async {
            let state = libsql_load_state(&conn).await?;
            operation(&state)
        }
        .await;
        finish_libsql_transaction(&conn, result).await
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl SessionThreadService for LibSqlSessionThreadService {
    async fn ensure_thread(
        &self,
        request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        self.mutate(|state| state.ensure_thread(request)).await
    }

    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        self.mutate(|state| state.accept_inbound_message(request))
            .await
    }

    async fn replay_accepted_inbound_message(
        &self,
        request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
        self.read(|state| state.replay_accepted_inbound_message(&request))
            .await
    }

    async fn mark_message_submitted(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        turn_id: String,
        turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let scope = scope.clone();
        let thread_id = thread_id.clone();
        self.mutate(|state| {
            state.mark_message_submitted(&scope, &thread_id, message_id, turn_id, turn_run_id)
        })
        .await
    }

    async fn mark_message_deferred_busy(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let scope = scope.clone();
        let thread_id = thread_id.clone();
        self.mutate(|state| state.mark_message_deferred_busy(&scope, &thread_id, message_id))
            .await
    }

    async fn append_assistant_draft(
        &self,
        request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.mutate(|state| state.append_assistant_draft(request))
            .await
    }

    async fn update_assistant_draft(
        &self,
        request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.mutate(|state| state.update_assistant_draft(request))
            .await
    }

    async fn finalize_assistant_message(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let scope = scope.clone();
        let thread_id = thread_id.clone();
        self.mutate(|state| {
            state.finalize_assistant_message(&scope, &thread_id, message_id, content)
        })
        .await
    }

    async fn redact_message(
        &self,
        request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.mutate(|state| state.redact_message(request)).await
    }

    async fn load_context_window(
        &self,
        request: LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        self.read(|state| state.load_context_window(request)).await
    }

    async fn list_thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        self.read(|state| state.list_thread_history(request)).await
    }

    async fn create_summary_artifact(
        &self,
        request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError> {
        self.mutate(|state| state.create_summary_artifact(request))
            .await
    }
}

/// PostgreSQL-backed canonical session thread and transcript service.
#[cfg(feature = "postgres")]
pub struct PostgresSessionThreadService {
    pool: deadpool_postgres::Pool,
}

#[cfg(feature = "postgres")]
impl PostgresSessionThreadService {
    pub fn new(pool: deadpool_postgres::Pool) -> Self {
        Self { pool }
    }

    pub async fn run_migrations(&self) -> Result<(), SessionThreadError> {
        const MIGRATION_LOCK_ID: i64 = 0x1c10_0003;

        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        txn.query_one("SELECT pg_advisory_xact_lock($1)", &[&MIGRATION_LOCK_ID])
            .await
            .map_err(db_error)?;
        txn.batch_execute(POSTGRES_SESSION_THREAD_SCHEMA)
            .await
            .map_err(db_error)?;
        txn.commit().await.map_err(db_error)
    }

    async fn client(&self) -> Result<deadpool_postgres::Object, SessionThreadError> {
        self.pool.get().await.map_err(db_error)
    }

    async fn mutate<T>(
        &self,
        operation: impl FnOnce(&mut DurableState) -> Result<T, SessionThreadError>,
    ) -> Result<T, SessionThreadError> {
        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        lock_postgres_tables(&txn, "EXCLUSIVE").await?;
        let mut state = postgres_load_state(&txn).await?;
        let output = operation(&mut state)?;
        postgres_replace_state(&txn, &state).await?;
        txn.commit().await.map_err(db_error)?;
        Ok(output)
    }

    async fn read<T>(
        &self,
        operation: impl FnOnce(&DurableState) -> Result<T, SessionThreadError>,
    ) -> Result<T, SessionThreadError> {
        let mut client = self.client().await?;
        let txn = client.transaction().await.map_err(db_error)?;
        lock_postgres_tables(&txn, "SHARE").await?;
        let state = postgres_load_state(&txn).await?;
        let output = operation(&state)?;
        txn.commit().await.map_err(db_error)?;
        Ok(output)
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl SessionThreadService for PostgresSessionThreadService {
    async fn ensure_thread(
        &self,
        request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        self.mutate(|state| state.ensure_thread(request)).await
    }

    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        self.mutate(|state| state.accept_inbound_message(request))
            .await
    }

    async fn replay_accepted_inbound_message(
        &self,
        request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
        self.read(|state| state.replay_accepted_inbound_message(&request))
            .await
    }

    async fn mark_message_submitted(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        turn_id: String,
        turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let scope = scope.clone();
        let thread_id = thread_id.clone();
        self.mutate(|state| {
            state.mark_message_submitted(&scope, &thread_id, message_id, turn_id, turn_run_id)
        })
        .await
    }

    async fn mark_message_deferred_busy(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let scope = scope.clone();
        let thread_id = thread_id.clone();
        self.mutate(|state| state.mark_message_deferred_busy(&scope, &thread_id, message_id))
            .await
    }

    async fn append_assistant_draft(
        &self,
        request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.mutate(|state| state.append_assistant_draft(request))
            .await
    }

    async fn update_assistant_draft(
        &self,
        request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.mutate(|state| state.update_assistant_draft(request))
            .await
    }

    async fn finalize_assistant_message(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let scope = scope.clone();
        let thread_id = thread_id.clone();
        self.mutate(|state| {
            state.finalize_assistant_message(&scope, &thread_id, message_id, content)
        })
        .await
    }

    async fn redact_message(
        &self,
        request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.mutate(|state| state.redact_message(request)).await
    }

    async fn load_context_window(
        &self,
        request: LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        self.read(|state| state.load_context_window(request)).await
    }

    async fn list_thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        self.read(|state| state.list_thread_history(request)).await
    }

    async fn create_summary_artifact(
        &self,
        request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError> {
        self.mutate(|state| state.create_summary_artifact(request))
            .await
    }
}

#[derive(Debug, Default)]
struct DurableState {
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct InboundIdempotencyKey {
    scope: ThreadScope,
    source_binding_id: String,
    external_event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InboundIdempotencyRecord {
    scope: ThreadScope,
    source_binding_id: String,
    external_event_id: String,
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

impl InboundIdempotencyRecord {
    fn key(&self) -> InboundIdempotencyKey {
        InboundIdempotencyKey {
            scope: self.scope.clone(),
            source_binding_id: self.source_binding_id.clone(),
            external_event_id: self.external_event_id.clone(),
        }
    }
}

impl DurableState {
    fn ensure_thread(
        &mut self,
        request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        let thread_id = match request.thread_id {
            Some(thread_id) => thread_id,
            None => generated_thread_id()?,
        };
        if let Some(existing) = self.threads.get(&thread_id) {
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
        self.threads.insert(
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

    fn accept_inbound_message(
        &mut self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        if let Some(key) = InboundIdempotencyKey::from_request(&request)
            && let Some(record) = self.inbound_idempotency.get(&key)
        {
            if record.thread_id != request.thread_id {
                return Err(SessionThreadError::IdempotentReplayThreadMismatch {
                    stored_thread_id: record.thread_id.clone(),
                    requested_thread_id: request.thread_id,
                });
            }
            let thread = get_thread(self, &request.scope, &record.thread_id)?;
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
        let thread = get_thread_mut(self, &request.scope, &request.thread_id)?;
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
            self.inbound_idempotency.insert(
                key.clone(),
                InboundIdempotencyRecord {
                    scope: key.scope,
                    source_binding_id: key.source_binding_id,
                    external_event_id: key.external_event_id,
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

    fn replay_accepted_inbound_message(
        &self,
        request: &ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
        let Some(record) = self.inbound_idempotency.values().find(|record| {
            record.source_binding_id == request.source_binding_id
                && record.external_event_id == request.external_event_id
        }) else {
            return Ok(None);
        };
        let thread = get_thread(self, &record.scope, &record.thread_id)?;
        let message = thread
            .messages
            .iter()
            .find(|message| message.message_id == record.message_id)
            .ok_or(SessionThreadError::UnknownMessage {
                message_id: record.message_id,
            })?;
        Ok(Some(AcceptedInboundMessageReplay {
            scope: record.scope.clone(),
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

    fn mark_message_submitted(
        &mut self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        turn_id: String,
        turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let message = get_message_mut(self, scope, thread_id, message_id)?;
        ensure_user_accepted(message, "mark_message_submitted")?;
        message.status = MessageStatus::Submitted;
        message.turn_id = Some(turn_id);
        message.turn_run_id = Some(turn_run_id);
        Ok(message.clone())
    }

    fn mark_message_deferred_busy(
        &mut self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let message = get_message_mut(self, scope, thread_id, message_id)?;
        ensure_user_accepted(message, "mark_message_deferred_busy")?;
        message.status = MessageStatus::DeferredBusy;
        message.turn_id = None;
        message.turn_run_id = None;
        Ok(message.clone())
    }

    fn append_assistant_draft(
        &mut self,
        request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let thread = get_thread_mut(self, &request.scope, &request.thread_id)?;
        if let Some(existing) = thread.messages.iter().find(|message| {
            message.kind == MessageKind::Assistant
                && message.turn_run_id.as_deref() == Some(request.turn_run_id.as_str())
        }) {
            return Ok(existing.clone());
        }
        let message = ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
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

    fn update_assistant_draft(
        &mut self,
        request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let message =
            get_message_mut(self, &request.scope, &request.thread_id, request.message_id)?;
        ensure_draft(message)?;
        message.content = Some(request.content.into_text());
        Ok(message.clone())
    }

    fn finalize_assistant_message(
        &mut self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let message = get_message_mut(self, scope, thread_id, message_id)?;
        ensure_draft(message)?;
        message.status = MessageStatus::Finalized;
        message.content = Some(content.into_text());
        Ok(message.clone())
    }

    fn redact_message(
        &mut self,
        request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let message =
            get_message_mut(self, &request.scope, &request.thread_id, request.message_id)?;
        message.status = MessageStatus::Redacted;
        message.content = None;
        message.redaction_ref = Some(request.redaction_ref);
        Ok(message.clone())
    }

    fn load_context_window(
        &self,
        request: LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        let thread = get_thread(self, &request.scope, &request.thread_id)?;
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

    fn list_thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        let thread = get_thread(self, &request.scope, &request.thread_id)?;
        Ok(ThreadHistory {
            thread: thread.record.clone(),
            messages: thread.messages.clone(),
            summary_artifacts: history_summary_artifacts(thread),
        })
    }

    fn create_summary_artifact(
        &mut self,
        request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError> {
        if request.start_sequence == 0 || request.start_sequence > request.end_sequence {
            return Err(SessionThreadError::InvalidSummaryRange {
                start_sequence: request.start_sequence,
                end_sequence: request.end_sequence,
            });
        }
        let thread = get_thread_mut(self, &request.scope, &request.thread_id)?;
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
    ThreadId::new(uuid::Uuid::new_v4().to_string())
        .map_err(|error| SessionThreadError::GeneratedThreadId(error.to_string()))
}

fn get_thread<'a>(
    state: &'a DurableState,
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
    state: &'a mut DurableState,
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
    state: &'a mut DurableState,
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
        })
        .collect::<Vec<_>>();
    let mut skip_through = 0;
    let mut emitted_summaries = Vec::new();
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
                && !summary_covers_hidden_content(thread, summary)
        }) {
            context.push(ContextMessage {
                message_id: None,
                summary_id: Some(summary.summary_id),
                sequence: summary.start_sequence,
                kind: MessageKind::Summary,
                content: summary.content.clone(),
            });
            emitted_summaries.push(summary.summary_id);
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

#[cfg(feature = "libsql")]
async fn finish_libsql_transaction<T>(
    conn: &libsql::Connection,
    result: Result<T, SessionThreadError>,
) -> Result<T, SessionThreadError> {
    match result {
        Ok(value) => {
            conn.execute("COMMIT", ()).await.map_err(db_error)?;
            Ok(value)
        }
        Err(error) => {
            let _ = conn.execute("ROLLBACK", ()).await;
            Err(error)
        }
    }
}

#[cfg(feature = "libsql")]
async fn libsql_load_state(conn: &libsql::Connection) -> Result<DurableState, SessionThreadError> {
    let mut state = DurableState::default();
    let mut rows = conn
        .query(
            "SELECT thread_id, scope_key, next_sequence, payload FROM reborn_session_thread_records ORDER BY thread_id",
            (),
        )
        .await
        .map_err(db_error)?;
    while let Some(row) = rows.next().await.map_err(db_error)? {
        let thread_id: String = row.get(0).map_err(db_error)?;
        let scope_key: String = row.get(1).map_err(db_error)?;
        let next_sequence: i64 = row.get(2).map_err(db_error)?;
        let payload: String = row.get(3).map_err(db_error)?;
        let record = validate_thread_row(
            from_json::<SessionThreadRecord>(&payload)?,
            &thread_id,
            &scope_key,
        )?;
        state.threads.insert(
            record.thread_id.clone(),
            StoredThread {
                record,
                messages: Vec::new(),
                summary_artifacts: Vec::new(),
                next_sequence: non_negative_i64_to_u64(next_sequence, "next_sequence")?,
            },
        );
    }

    let mut rows = conn
        .query(
            "SELECT message_id, thread_id, scope_key, sequence, kind, status, turn_run_id, payload FROM reborn_thread_message_records ORDER BY thread_id, sequence",
            (),
        )
        .await
        .map_err(db_error)?;
    while let Some(row) = rows.next().await.map_err(db_error)? {
        let message_id: String = row.get(0).map_err(db_error)?;
        let thread_id: String = row.get(1).map_err(db_error)?;
        let scope_key: String = row.get(2).map_err(db_error)?;
        let sequence: i64 = row.get(3).map_err(db_error)?;
        let kind: String = row.get(4).map_err(db_error)?;
        let status: String = row.get(5).map_err(db_error)?;
        let turn_run_id: Option<String> = row.get(6).map_err(db_error)?;
        let payload: String = row.get(7).map_err(db_error)?;
        let record = validate_message_row(
            from_json::<ThreadMessageRecord>(&payload)?,
            MessageRowColumns {
                message_id: &message_id,
                thread_id: &thread_id,
                sequence: non_negative_i64_to_u64(sequence, "sequence")?,
                kind: &kind,
                status: &status,
                turn_run_id: turn_run_id.as_deref(),
            },
        )?;
        let thread = state
            .threads
            .get_mut(&record.thread_id)
            .ok_or_else(|| row_integrity_error("thread-message", "thread_id"))?;
        if thread_scope_key(&thread.record.scope)? != scope_key {
            return Err(row_integrity_error("thread-message", "scope_key"));
        }
        thread.messages.push(record);
    }

    let mut rows = conn
        .query(
            "SELECT summary_id, thread_id, scope_key, start_sequence, end_sequence, model_context_policy, payload FROM reborn_thread_summary_artifacts ORDER BY thread_id, start_sequence, end_sequence",
            (),
        )
        .await
        .map_err(db_error)?;
    while let Some(row) = rows.next().await.map_err(db_error)? {
        let summary_id: String = row.get(0).map_err(db_error)?;
        let thread_id: String = row.get(1).map_err(db_error)?;
        let scope_key: String = row.get(2).map_err(db_error)?;
        let start_sequence: i64 = row.get(3).map_err(db_error)?;
        let end_sequence: i64 = row.get(4).map_err(db_error)?;
        let model_context_policy: Option<String> = row.get(5).map_err(db_error)?;
        let payload: String = row.get(6).map_err(db_error)?;
        let record = validate_summary_row(
            from_json::<SummaryArtifact>(&payload)?,
            &summary_id,
            &thread_id,
            &scope_key,
            non_negative_i64_to_u64(start_sequence, "start_sequence")?,
            non_negative_i64_to_u64(end_sequence, "end_sequence")?,
            model_context_policy.as_deref(),
        )?;
        let thread = state
            .threads
            .get_mut(&record.thread_id)
            .ok_or_else(|| row_integrity_error("summary", "thread_id"))?;
        if thread_scope_key(&thread.record.scope)? != scope_key {
            return Err(row_integrity_error("summary", "scope_key"));
        }
        thread.summary_artifacts.push(record);
    }

    let mut rows = conn
        .query(
            "SELECT record_key, scope_key, source_binding_id, external_event_id, thread_id, message_id, payload FROM reborn_thread_inbound_idempotency ORDER BY record_key",
            (),
        )
        .await
        .map_err(db_error)?;
    while let Some(row) = rows.next().await.map_err(db_error)? {
        let record_key: String = row.get(0).map_err(db_error)?;
        let scope_key: String = row.get(1).map_err(db_error)?;
        let source_binding_id: String = row.get(2).map_err(db_error)?;
        let external_event_id: String = row.get(3).map_err(db_error)?;
        let thread_id: String = row.get(4).map_err(db_error)?;
        let message_id: String = row.get(5).map_err(db_error)?;
        let payload: String = row.get(6).map_err(db_error)?;
        let record = validate_idempotency_row(
            from_json::<InboundIdempotencyRecord>(&payload)?,
            &record_key,
            &scope_key,
            &source_binding_id,
            &external_event_id,
            &thread_id,
            &message_id,
        )?;
        state.inbound_idempotency.insert(record.key(), record);
    }

    sort_loaded_state(&mut state);
    Ok(state)
}

#[cfg(feature = "libsql")]
async fn libsql_replace_state(
    conn: &libsql::Connection,
    state: &DurableState,
) -> Result<(), SessionThreadError> {
    conn.execute("DELETE FROM reborn_thread_inbound_idempotency", ())
        .await
        .map_err(db_error)?;
    conn.execute("DELETE FROM reborn_thread_summary_artifacts", ())
        .await
        .map_err(db_error)?;
    conn.execute("DELETE FROM reborn_thread_message_records", ())
        .await
        .map_err(db_error)?;
    conn.execute("DELETE FROM reborn_session_thread_records", ())
        .await
        .map_err(db_error)?;

    for thread in sorted_threads(state) {
        conn.execute(
            "INSERT INTO reborn_session_thread_records (thread_id, scope_key, next_sequence, payload) VALUES (?1, ?2, ?3, ?4)",
            libsql::params![
                thread.record.thread_id.to_string(),
                thread_scope_key(&thread.record.scope)?,
                u64_to_i64(thread.next_sequence, "next_sequence")?,
                to_json(&thread.record)?,
            ],
        )
        .await
        .map_err(db_error)?;
        for message in &thread.messages {
            conn.execute(
                "INSERT INTO reborn_thread_message_records (message_id, thread_id, scope_key, sequence, kind, status, turn_run_id, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                libsql::params![
                    message.message_id.to_string(),
                    message.thread_id.to_string(),
                    thread_scope_key(&thread.record.scope)?,
                    u64_to_i64(message.sequence, "sequence")?,
                    message_kind_key(message.kind),
                    message_status_key(message.status),
                    message.turn_run_id.clone(),
                    to_json(message)?,
                ],
            )
            .await
            .map_err(db_error)?;
        }
        for summary in &thread.summary_artifacts {
            conn.execute(
                "INSERT INTO reborn_thread_summary_artifacts (summary_id, thread_id, scope_key, start_sequence, end_sequence, model_context_policy, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                libsql::params![
                    summary.summary_id.to_string(),
                    summary.thread_id.to_string(),
                    thread_scope_key(&thread.record.scope)?,
                    u64_to_i64(summary.start_sequence, "start_sequence")?,
                    u64_to_i64(summary.end_sequence, "end_sequence")?,
                    summary.model_context_policy.clone(),
                    to_json(summary)?,
                ],
            )
            .await
            .map_err(db_error)?;
        }
    }

    for record in sorted_idempotency_records(state) {
        conn.execute(
            "INSERT INTO reborn_thread_inbound_idempotency (record_key, scope_key, source_binding_id, external_event_id, thread_id, message_id, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            libsql::params![
                idempotency_record_key(&record.key())?,
                thread_scope_key(&record.scope)?,
                record.source_binding_id.clone(),
                record.external_event_id.clone(),
                record.thread_id.to_string(),
                record.message_id.to_string(),
                to_json(record)?,
            ],
        )
        .await
        .map_err(db_error)?;
    }

    Ok(())
}

#[cfg(feature = "postgres")]
async fn lock_postgres_tables(
    client: &impl deadpool_postgres::GenericClient,
    mode: &str,
) -> Result<(), SessionThreadError> {
    let statement = format!(
        "LOCK TABLE reborn_session_thread_records, reborn_thread_message_records, reborn_thread_summary_artifacts, reborn_thread_inbound_idempotency IN {mode} MODE"
    );
    client.batch_execute(&statement).await.map_err(db_error)
}

#[cfg(feature = "postgres")]
async fn postgres_load_state(
    client: &impl deadpool_postgres::GenericClient,
) -> Result<DurableState, SessionThreadError> {
    let mut state = DurableState::default();
    let rows = client
        .query(
            "SELECT thread_id, scope_key, next_sequence, payload::text FROM reborn_session_thread_records ORDER BY thread_id",
            &[],
        )
        .await
        .map_err(db_error)?;
    for row in rows {
        let thread_id: String = row.get(0);
        let scope_key: String = row.get(1);
        let next_sequence: i64 = row.get(2);
        let payload: String = row.get(3);
        let record = validate_thread_row(
            from_json::<SessionThreadRecord>(&payload)?,
            &thread_id,
            &scope_key,
        )?;
        state.threads.insert(
            record.thread_id.clone(),
            StoredThread {
                record,
                messages: Vec::new(),
                summary_artifacts: Vec::new(),
                next_sequence: non_negative_i64_to_u64(next_sequence, "next_sequence")?,
            },
        );
    }

    let rows = client
        .query(
            "SELECT message_id, thread_id, scope_key, sequence, kind, status, turn_run_id, payload::text FROM reborn_thread_message_records ORDER BY thread_id, sequence",
            &[],
        )
        .await
        .map_err(db_error)?;
    for row in rows {
        let message_id: String = row.get(0);
        let thread_id: String = row.get(1);
        let scope_key: String = row.get(2);
        let sequence: i64 = row.get(3);
        let kind: String = row.get(4);
        let status: String = row.get(5);
        let turn_run_id: Option<String> = row.get(6);
        let payload: String = row.get(7);
        let record = validate_message_row(
            from_json::<ThreadMessageRecord>(&payload)?,
            MessageRowColumns {
                message_id: &message_id,
                thread_id: &thread_id,
                sequence: non_negative_i64_to_u64(sequence, "sequence")?,
                kind: &kind,
                status: &status,
                turn_run_id: turn_run_id.as_deref(),
            },
        )?;
        let thread = state
            .threads
            .get_mut(&record.thread_id)
            .ok_or_else(|| row_integrity_error("thread-message", "thread_id"))?;
        if thread_scope_key(&thread.record.scope)? != scope_key {
            return Err(row_integrity_error("thread-message", "scope_key"));
        }
        thread.messages.push(record);
    }

    let rows = client
        .query(
            "SELECT summary_id, thread_id, scope_key, start_sequence, end_sequence, model_context_policy, payload::text FROM reborn_thread_summary_artifacts ORDER BY thread_id, start_sequence, end_sequence",
            &[],
        )
        .await
        .map_err(db_error)?;
    for row in rows {
        let summary_id: String = row.get(0);
        let thread_id: String = row.get(1);
        let scope_key: String = row.get(2);
        let start_sequence: i64 = row.get(3);
        let end_sequence: i64 = row.get(4);
        let model_context_policy: Option<String> = row.get(5);
        let payload: String = row.get(6);
        let record = validate_summary_row(
            from_json::<SummaryArtifact>(&payload)?,
            &summary_id,
            &thread_id,
            &scope_key,
            non_negative_i64_to_u64(start_sequence, "start_sequence")?,
            non_negative_i64_to_u64(end_sequence, "end_sequence")?,
            model_context_policy.as_deref(),
        )?;
        let thread = state
            .threads
            .get_mut(&record.thread_id)
            .ok_or_else(|| row_integrity_error("summary", "thread_id"))?;
        if thread_scope_key(&thread.record.scope)? != scope_key {
            return Err(row_integrity_error("summary", "scope_key"));
        }
        thread.summary_artifacts.push(record);
    }

    let rows = client
        .query(
            "SELECT record_key, scope_key, source_binding_id, external_event_id, thread_id, message_id, payload::text FROM reborn_thread_inbound_idempotency ORDER BY record_key",
            &[],
        )
        .await
        .map_err(db_error)?;
    for row in rows {
        let record_key: String = row.get(0);
        let scope_key: String = row.get(1);
        let source_binding_id: String = row.get(2);
        let external_event_id: String = row.get(3);
        let thread_id: String = row.get(4);
        let message_id: String = row.get(5);
        let payload: String = row.get(6);
        let record = validate_idempotency_row(
            from_json::<InboundIdempotencyRecord>(&payload)?,
            &record_key,
            &scope_key,
            &source_binding_id,
            &external_event_id,
            &thread_id,
            &message_id,
        )?;
        state.inbound_idempotency.insert(record.key(), record);
    }

    sort_loaded_state(&mut state);
    Ok(state)
}

#[cfg(feature = "postgres")]
async fn postgres_replace_state(
    client: &impl deadpool_postgres::GenericClient,
    state: &DurableState,
) -> Result<(), SessionThreadError> {
    client
        .batch_execute(
            "DELETE FROM reborn_thread_inbound_idempotency;
             DELETE FROM reborn_thread_summary_artifacts;
             DELETE FROM reborn_thread_message_records;
             DELETE FROM reborn_session_thread_records;",
        )
        .await
        .map_err(db_error)?;

    for thread in sorted_threads(state) {
        let payload = to_json(&thread.record)?;
        client
            .execute(
                "INSERT INTO reborn_session_thread_records (thread_id, scope_key, next_sequence, payload) VALUES ($1, $2, $3, $4::text::jsonb)",
                &[
                    &thread.record.thread_id.to_string(),
                    &thread_scope_key(&thread.record.scope)?,
                    &u64_to_i64(thread.next_sequence, "next_sequence")?,
                    &payload,
                ],
            )
            .await
            .map_err(db_error)?;
        for message in &thread.messages {
            let payload = to_json(message)?;
            client
                .execute(
                    "INSERT INTO reborn_thread_message_records (message_id, thread_id, scope_key, sequence, kind, status, turn_run_id, payload) VALUES ($1, $2, $3, $4, $5, $6, $7, $8::text::jsonb)",
                    &[
                        &message.message_id.to_string(),
                        &message.thread_id.to_string(),
                        &thread_scope_key(&thread.record.scope)?,
                        &u64_to_i64(message.sequence, "sequence")?,
                        &message_kind_key(message.kind),
                        &message_status_key(message.status),
                        &message.turn_run_id,
                        &payload,
                    ],
                )
                .await
                .map_err(db_error)?;
        }
        for summary in &thread.summary_artifacts {
            let payload = to_json(summary)?;
            client
                .execute(
                    "INSERT INTO reborn_thread_summary_artifacts (summary_id, thread_id, scope_key, start_sequence, end_sequence, model_context_policy, payload) VALUES ($1, $2, $3, $4, $5, $6, $7::text::jsonb)",
                    &[
                        &summary.summary_id.to_string(),
                        &summary.thread_id.to_string(),
                        &thread_scope_key(&thread.record.scope)?,
                        &u64_to_i64(summary.start_sequence, "start_sequence")?,
                        &u64_to_i64(summary.end_sequence, "end_sequence")?,
                        &summary.model_context_policy,
                        &payload,
                    ],
                )
                .await
                .map_err(db_error)?;
        }
    }

    for record in sorted_idempotency_records(state) {
        let payload = to_json(record)?;
        client
            .execute(
                "INSERT INTO reborn_thread_inbound_idempotency (record_key, scope_key, source_binding_id, external_event_id, thread_id, message_id, payload) VALUES ($1, $2, $3, $4, $5, $6, $7::text::jsonb)",
                &[
                    &idempotency_record_key(&record.key())?,
                    &thread_scope_key(&record.scope)?,
                    &record.source_binding_id,
                    &record.external_event_id,
                    &record.thread_id.to_string(),
                    &record.message_id.to_string(),
                    &payload,
                ],
            )
            .await
            .map_err(db_error)?;
    }

    Ok(())
}

fn sorted_threads(state: &DurableState) -> Vec<&StoredThread> {
    let mut threads = state.threads.values().collect::<Vec<_>>();
    threads.sort_by_key(|thread| thread.record.thread_id.to_string());
    threads
}

fn sorted_idempotency_records(state: &DurableState) -> Vec<&InboundIdempotencyRecord> {
    let mut records = state.inbound_idempotency.values().collect::<Vec<_>>();
    records.sort_by_key(|record| {
        (
            record.thread_id.to_string(),
            record.source_binding_id.clone(),
            record.external_event_id.clone(),
        )
    });
    records
}

fn sort_loaded_state(state: &mut DurableState) {
    for thread in state.threads.values_mut() {
        thread.messages.sort_by_key(|message| message.sequence);
        thread.summary_artifacts.sort_by_key(|summary| {
            (
                summary.start_sequence,
                summary.end_sequence,
                summary.summary_id.to_string(),
            )
        });
    }
}

fn validate_thread_row(
    record: SessionThreadRecord,
    row_thread_id: &str,
    row_scope_key: &str,
) -> Result<SessionThreadRecord, SessionThreadError> {
    if record.thread_id.to_string() != row_thread_id {
        return Err(row_integrity_error("session-thread", "thread_id"));
    }
    if thread_scope_key(&record.scope)? != row_scope_key {
        return Err(row_integrity_error("session-thread", "scope_key"));
    }
    Ok(record)
}

struct MessageRowColumns<'a> {
    message_id: &'a str,
    thread_id: &'a str,
    sequence: u64,
    kind: &'a str,
    status: &'a str,
    turn_run_id: Option<&'a str>,
}

fn validate_message_row(
    record: ThreadMessageRecord,
    row: MessageRowColumns<'_>,
) -> Result<ThreadMessageRecord, SessionThreadError> {
    if record.message_id.to_string() != row.message_id {
        return Err(row_integrity_error("thread-message", "message_id"));
    }
    if record.thread_id.to_string() != row.thread_id {
        return Err(row_integrity_error("thread-message", "thread_id"));
    }
    if record.sequence != row.sequence {
        return Err(row_integrity_error("thread-message", "sequence"));
    }
    if message_kind_key(record.kind) != row.kind {
        return Err(row_integrity_error("thread-message", "kind"));
    }
    if message_status_key(record.status) != row.status {
        return Err(row_integrity_error("thread-message", "status"));
    }
    if record.turn_run_id.as_deref() != row.turn_run_id {
        return Err(row_integrity_error("thread-message", "turn_run_id"));
    }
    Ok(record)
}

fn validate_summary_row(
    record: SummaryArtifact,
    row_summary_id: &str,
    row_thread_id: &str,
    row_scope_key: &str,
    row_start_sequence: u64,
    row_end_sequence: u64,
    row_model_context_policy: Option<&str>,
) -> Result<SummaryArtifact, SessionThreadError> {
    if record.summary_id.to_string() != row_summary_id {
        return Err(row_integrity_error("summary", "summary_id"));
    }
    if record.thread_id.to_string() != row_thread_id {
        return Err(row_integrity_error("summary", "thread_id"));
    }
    if record.start_sequence != row_start_sequence || record.end_sequence != row_end_sequence {
        return Err(row_integrity_error("summary", "sequence_range"));
    }
    if record.model_context_policy.as_deref() != row_model_context_policy {
        return Err(row_integrity_error("summary", "model_context_policy"));
    }
    let thread_scope = record_scope_key_placeholder(row_scope_key)?;
    if thread_scope != row_scope_key {
        return Err(row_integrity_error("summary", "scope_key"));
    }
    Ok(record)
}

fn validate_idempotency_row(
    record: InboundIdempotencyRecord,
    row_record_key: &str,
    row_scope_key: &str,
    row_source_binding_id: &str,
    row_external_event_id: &str,
    row_thread_id: &str,
    row_message_id: &str,
) -> Result<InboundIdempotencyRecord, SessionThreadError> {
    if idempotency_record_key(&record.key())? != row_record_key {
        return Err(row_integrity_error("inbound-idempotency", "record_key"));
    }
    if thread_scope_key(&record.scope)? != row_scope_key {
        return Err(row_integrity_error("inbound-idempotency", "scope_key"));
    }
    if record.source_binding_id != row_source_binding_id {
        return Err(row_integrity_error(
            "inbound-idempotency",
            "source_binding_id",
        ));
    }
    if record.external_event_id != row_external_event_id {
        return Err(row_integrity_error(
            "inbound-idempotency",
            "external_event_id",
        ));
    }
    if record.thread_id.to_string() != row_thread_id {
        return Err(row_integrity_error("inbound-idempotency", "thread_id"));
    }
    if record.message_id.to_string() != row_message_id {
        return Err(row_integrity_error("inbound-idempotency", "message_id"));
    }
    Ok(record)
}

fn record_scope_key_placeholder(row_scope_key: &str) -> Result<&str, SessionThreadError> {
    if row_scope_key.is_empty() {
        Err(row_integrity_error("row", "scope_key"))
    } else {
        Ok(row_scope_key)
    }
}

fn thread_scope_key(scope: &ThreadScope) -> Result<String, SessionThreadError> {
    #[derive(Serialize)]
    struct ScopeKey<'a> {
        tenant_id: &'a str,
        agent_id: &'a str,
        project_id: Option<&'a str>,
        owner_user_id: Option<&'a str>,
        mission_id: Option<&'a str>,
    }

    to_json(&ScopeKey {
        tenant_id: scope.tenant_id.as_str(),
        agent_id: scope.agent_id.as_str(),
        project_id: scope.project_id.as_ref().map(|id| id.as_str()),
        owner_user_id: scope.owner_user_id.as_ref().map(|id| id.as_str()),
        mission_id: scope.mission_id.as_ref().map(|id| id.as_str()),
    })
}

fn idempotency_record_key(key: &InboundIdempotencyKey) -> Result<String, SessionThreadError> {
    let payload = to_json(key)?;
    let digest = Sha256::digest(payload.as_bytes());
    let mut output = String::with_capacity("sha256:".len() + digest.len() * 2);
    output.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}")
            .map_err(|error| SessionThreadError::Serialization(error.to_string()))?;
    }
    Ok(output)
}

fn message_kind_key(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::User => "user",
        MessageKind::Assistant => "assistant",
        MessageKind::System => "system",
        MessageKind::Summary => "summary",
        MessageKind::CheckpointReference => "checkpoint_reference",
        MessageKind::ToolResultReference => "tool_result_reference",
    }
}

fn message_status_key(status: MessageStatus) -> &'static str {
    match status {
        MessageStatus::Accepted => "accepted",
        MessageStatus::Submitted => "submitted",
        MessageStatus::DeferredBusy => "deferred_busy",
        MessageStatus::Draft => "draft",
        MessageStatus::Finalized => "finalized",
        MessageStatus::Interrupted => "interrupted",
        MessageStatus::Superseded => "superseded",
        MessageStatus::Redacted => "redacted",
        MessageStatus::Deleted => "deleted",
    }
}

fn non_negative_i64_to_u64(value: i64, column: &'static str) -> Result<u64, SessionThreadError> {
    u64::try_from(value).map_err(|_| {
        SessionThreadError::Deserialization(format!("negative {column} column in durable threads"))
    })
}

fn u64_to_i64(value: u64, column: &'static str) -> Result<i64, SessionThreadError> {
    i64::try_from(value).map_err(|_| {
        SessionThreadError::Serialization(format!("{column} value exceeds database range"))
    })
}

fn row_integrity_error(entity: &'static str, field: &'static str) -> SessionThreadError {
    SessionThreadError::Deserialization(format!(
        "{entity} row payload does not match {field} column"
    ))
}

fn to_json<T>(value: &T) -> Result<String, SessionThreadError>
where
    T: Serialize,
{
    serde_json::to_string(value)
        .map_err(|error| SessionThreadError::Serialization(error.to_string()))
}

fn from_json<T>(payload: &str) -> Result<T, SessionThreadError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(payload)
        .map_err(|error| SessionThreadError::Deserialization(error.to_string()))
}

fn db_error(error: impl std::fmt::Display) -> SessionThreadError {
    tracing::debug!(%error, "session-thread database operation failed");
    SessionThreadError::Backend("session-thread database unavailable".to_string())
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};

    use crate::ThreadScope;

    use super::{InboundIdempotencyKey, idempotency_record_key};

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

        assert!(record_key.starts_with("sha256:"));
        assert_eq!(record_key.len(), "sha256:".len() + 64);
    }
}
