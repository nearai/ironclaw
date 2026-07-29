use std::sync::Arc;

use ironclaw_host_api::ThreadId;
use ironclaw_threads::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
    AppendAssistantDraftRequest, AppendCapabilityDisplayPreviewRequest,
    AppendFinalizedAssistantMessageRequest, AppendToolResultReferenceRequest, ContextMessages,
    ContextWindow, CreateSummaryArtifactRequest, EnsureThreadRequest, LoadContextMessagesRequest,
    LoadContextWindowRequest, MessageContent, RedactMessageRequest,
    ReplayAcceptedInboundMessageRequest, SessionThreadError, SessionThreadRecord,
    SessionThreadService, SummaryArtifact, ThreadHistory, ThreadHistoryRequest, ThreadMessageId,
    ThreadMessageRecord, ThreadScope, UpdateAssistantDraftRequest,
    UpdateToolResultReferenceRequest,
};

pub const TRANSCRIPT_FAILURE_SECRET: &str = "sk-TRANSCRIPT0123456789SECRET";

/// Runtime transcript seam that rejects the single-write final assistant
/// persistence method before any draft exists. Reads and inbound writes still
/// delegate to the real filesystem-backed service.
pub struct FailingAppendFinalizedAssistantThreadService {
    inner: Arc<dyn SessionThreadService>,
}

impl FailingAppendFinalizedAssistantThreadService {
    pub fn new(inner: Arc<dyn SessionThreadService>) -> Self {
        Self { inner }
    }
}

#[async_trait::async_trait]
impl SessionThreadService for FailingAppendFinalizedAssistantThreadService {
    async fn ensure_thread(
        &self,
        request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        self.inner.ensure_thread(request).await
    }

    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        self.inner.accept_inbound_message(request).await
    }

    async fn replay_accepted_inbound_message(
        &self,
        request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
        self.inner.replay_accepted_inbound_message(request).await
    }

    async fn mark_message_submitted(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        turn_id: String,
        turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner
            .mark_message_submitted(scope, thread_id, message_id, turn_id, turn_run_id)
            .await
    }

    async fn mark_message_rejected_busy(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner
            .mark_message_rejected_busy(scope, thread_id, message_id)
            .await
    }

    async fn append_assistant_draft(
        &self,
        request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.append_assistant_draft(request).await
    }

    async fn append_finalized_assistant_message(
        &self,
        request: AppendFinalizedAssistantMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        Err(SessionThreadError::Backend(format!(
            "write rejected for raw transcript {:?} using token {TRANSCRIPT_FAILURE_SECRET}",
            request.content.as_text()
        )))
    }

    async fn append_tool_result_reference(
        &self,
        request: AppendToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.append_tool_result_reference(request).await
    }

    async fn append_capability_display_preview(
        &self,
        request: AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.append_capability_display_preview(request).await
    }

    async fn update_tool_result_reference(
        &self,
        request: UpdateToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.update_tool_result_reference(request).await
    }

    async fn update_assistant_draft(
        &self,
        request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.update_assistant_draft(request).await
    }

    async fn finalize_assistant_message(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner
            .finalize_assistant_message(scope, thread_id, message_id, content)
            .await
    }

    async fn redact_message(
        &self,
        request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.redact_message(request).await
    }

    async fn load_context_window(
        &self,
        request: LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        self.inner.load_context_window(request).await
    }

    async fn load_context_messages(
        &self,
        request: LoadContextMessagesRequest,
    ) -> Result<ContextMessages, SessionThreadError> {
        self.inner.load_context_messages(request).await
    }

    async fn list_thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        self.inner.list_thread_history(request).await
    }

    async fn create_summary_artifact(
        &self,
        request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError> {
        self.inner.create_summary_artifact(request).await
    }
}
