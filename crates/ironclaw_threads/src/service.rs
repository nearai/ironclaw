use async_trait::async_trait;
use ironclaw_host_api::ThreadId;

use crate::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
    AppendAssistantDraftRequest, AppendCapabilityDisplayPreviewRequest,
    AppendToolResultReferenceRequest, ContextMessages, ContextWindow, CreateSummaryArtifactRequest,
    EnsureThreadRequest, ListThreadsForScopeRequest, ListThreadsForScopeResponse,
    LoadContextMessagesRequest, LoadContextWindowRequest, MessageContent, RedactMessageRequest,
    ReplayAcceptedInboundMessageRequest, SessionThreadError, SessionThreadRecord, SummaryArtifact,
    ThreadHistory, ThreadHistoryRequest, ThreadMessageId, ThreadMessageRecord, ThreadScope,
    UpdateAssistantDraftRequest,
};

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

    async fn replay_accepted_inbound_message(
        &self,
        request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError>;

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

    async fn append_tool_result_reference(
        &self,
        request: AppendToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError>;

    async fn append_capability_display_preview(
        &self,
        _request: AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        Err(SessionThreadError::Backend(
            "append_capability_display_preview is not implemented by this SessionThreadService backend"
                .to_string(),
        ))
    }

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

    async fn load_context_messages(
        &self,
        request: LoadContextMessagesRequest,
    ) -> Result<ContextMessages, SessionThreadError>;

    async fn list_thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError>;

    /// Cheap, owner-scoped existence probe that returns *only* the
    /// thread record — no message transcript, no summary artifacts.
    ///
    /// Long-lived callers (e.g. the WebUI SSE handler) need to
    /// re-validate that the authenticated caller still owns the thread
    /// on every poll, but they have no use for the message body. Using
    /// `list_thread_history` for that probe forces a full transcript +
    /// summary load per poll, which on a large thread is hundreds of
    /// rows per second per active stream.
    ///
    /// The default implementation delegates to `list_thread_history` so
    /// existing stubs and test impls do not need to change; production
    /// backends override it with a metadata-only path.
    ///
    /// Implementations MUST preserve the same ownership-probe semantics
    /// as `list_thread_history`: returning `UnknownThread` for both
    /// "thread does not exist" and "thread exists but is owned by a
    /// different scope" so callers cannot use the response as an
    /// existence oracle.
    async fn read_thread(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        self.list_thread_history(request)
            .await
            .map(|history| history.thread)
    }

    async fn create_summary_artifact(
        &self,
        request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError>;

    /// List threads scoped to the supplied `ThreadScope`. The default
    /// impl fails closed (`SessionThreadError::Backend`) so backends
    /// that do not yet implement enumeration surface a clear
    /// `503 Service Unavailable` at the gateway instead of pretending
    /// the caller has zero threads. Production backends override this
    /// method with their own pagination strategy.
    ///
    /// Implementations MUST scope the listing by `owner_user_id` (or
    /// equivalent caller-binding fields on the scope) — otherwise a
    /// caller could enumerate threads owned by other users in the
    /// same `(tenant, agent, project)` triple.
    async fn list_threads_for_scope(
        &self,
        _request: ListThreadsForScopeRequest,
    ) -> Result<ListThreadsForScopeResponse, SessionThreadError> {
        Err(SessionThreadError::Backend(
            "list_threads_for_scope is not implemented by this SessionThreadService backend; \
             override this method before exposing the v2 list-threads route"
                .to_string(),
        ))
    }
}
