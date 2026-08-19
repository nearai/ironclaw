use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::ids::ThreadId;

use crate::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
    AppendAssistantDraftRequest, AppendCapabilityDisplayPreviewRequest,
    AppendFinalizedAssistantMessageRequest, AppendToolResultReferenceRequest,
    BoundedThreadMessages, BoundedThreadMessagesRequest, ContextMessages, ContextWindow,
    CreateSummaryArtifactRequest, DeleteToolResultRecordRequest, EnsureThreadRequest,
    FinalizedAssistantMessageByRunRequest, InboundMessageReplayMetadata,
    LatestThreadMessageRequest, ListThreadsForScopeRequest, ListThreadsForScopeResponse,
    LoadContextMessagesRequest, LoadContextWindowRequest, MessageContent,
    PublishStructuredFinalizationMessageRequest, PutStructuredFinalizationRequest,
    PutToolResultRecordRequest, ReadStructuredFinalizationRequest, ReadToolResultRecordRequest,
    RedactMessageRequest, ReplayAcceptedInboundMessageRequest, SessionThreadError,
    SessionThreadRecord, StructuredFinalizationRecord, SummaryArtifact, ThreadGoal, ThreadHistory,
    ThreadHistoryRequest, ThreadMessageId, ThreadMessageRange, ThreadMessageRangeRequest,
    ThreadMessageRecord, ThreadScope, ToolResultRecordChunk, UpdateAssistantDraftRequest,
    UpdateThreadGoalRequest, UpdateToolResultRecordRequest, UpdateToolResultReferenceRequest,
};

/// Canonical Reborn session thread and transcript boundary.
#[async_trait]
pub trait SessionThreadService: Send + Sync {
    async fn ensure_thread(
        &self,
        request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError>;

    /// Accept an inbound transcript row without product-routing metadata.
    ///
    /// An implementation that delegates this method to
    /// [`Self::accept_inbound_message_with_replay_metadata`] must override that
    /// method too. The default metadata method delegates default metadata back
    /// here for legacy compatibility, so delegating only this method creates a
    /// mutual-recursion trap.
    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError>;

    /// Accept an inbound transcript row together with product-routing metadata
    /// that must be committed atomically and returned on idempotent replay.
    /// Backends supporting non-default metadata must override this method. A
    /// backend that implements [`Self::accept_inbound_message`] by delegating
    /// here must also override this method; otherwise default metadata recurses
    /// back through the legacy method indefinitely.
    async fn accept_inbound_message_with_replay_metadata(
        &self,
        request: AcceptInboundMessageRequest,
        replay_metadata: InboundMessageReplayMetadata,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        if replay_metadata == InboundMessageReplayMetadata::default() {
            return self.accept_inbound_message(request).await;
        }
        Err(SessionThreadError::Backend(
            "thread service does not support durable inbound replay metadata".to_string(),
        ))
    }

    /// The ONE shared accept door for every non-channel caller (unbound
    /// turns design §4.2), sibling of [`Self::accept_inbound_message`]:
    /// mints an unbound thread, seeds the caller-authored context as
    /// transcript rows, journals the per-run declarations beside them, and
    /// replays idempotently by key — a crash-retry returns the SAME prepared
    /// context instead of minting an orphan.
    async fn accept_prepared_context(
        &self,
        request: crate::PreparedContextRequest,
    ) -> Result<crate::AcceptedPreparedContext, SessionThreadError> {
        let _ = request;
        Err(SessionThreadError::Backend(
            "accept_prepared_context is not implemented by this SessionThreadService backend"
                .to_string(),
        ))
    }

    /// Read the journaled unbound-context record for a thread, or `None`
    /// when the thread exists but is not a prepared context.
    /// Missing and cross-scope threads return the same non-enumerating
    /// `UnknownThread` shape as every other read on this trait.
    async fn read_prepared_context(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<Option<crate::PreparedContextRecord>, SessionThreadError> {
        let _ = (scope, thread_id);
        Err(SessionThreadError::Backend(
            "read_prepared_context is not implemented by this SessionThreadService backend"
                .to_string(),
        ))
    }

    /// Read immutable structured-finalization evidence for one exact run.
    /// Missing is the normal pre-finalization state; ownership is checked by
    /// the run host's lease fence, not inferred by this storage read.
    async fn read_structured_finalization(
        &self,
        _request: ReadStructuredFinalizationRequest,
    ) -> Result<Option<StructuredFinalizationRecord>, SessionThreadError> {
        Err(SessionThreadError::Backend(
            "structured finalization records are not implemented by this SessionThreadService backend"
                .to_string(),
        ))
    }

    /// Persist immutable structured-finalization evidence using an absent-write
    /// CAS. Identical retries return the existing record; conflicting content
    /// fails closed. Owner fences are checked before content comparison.
    async fn put_structured_finalization(
        &self,
        _request: PutStructuredFinalizationRequest,
    ) -> Result<StructuredFinalizationRecord, SessionThreadError> {
        Err(SessionThreadError::Backend(
            "structured finalization records are not implemented by this SessionThreadService backend"
                .to_string(),
        ))
    }

    /// Publish an already-durable structured-finalization result into one
    /// exact finalized assistant message. Implementations validate the
    /// supplied scope, message identity, run ownership, message status, and
    /// immutable record before changing only `content` through CAS.
    async fn publish_structured_finalization_message(
        &self,
        _request: PublishStructuredFinalizationMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        Err(SessionThreadError::Backend(
            "structured finalization message publication is not implemented by this SessionThreadService backend"
                .to_string(),
        ))
    }

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

    async fn mark_message_rejected_busy(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError>;

    async fn mark_message_queued(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        active_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let _ = (scope, thread_id, message_id, active_run_id);
        Err(SessionThreadError::Backend(
            "mark_message_queued is not implemented by this thread service".to_string(),
        ))
    }

    /// Read one message row by id, or `None` when the thread or message does
    /// not exist. A point read for race reconciliation (e.g. the busy-steering
    /// admission checking what a failed status transition actually raced
    /// with); listing the whole history to inspect one row is the wrong tool.
    async fn read_thread_message(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<Option<ThreadMessageRecord>, SessionThreadError> {
        let _ = (scope, thread_id, message_id);
        Err(SessionThreadError::Backend(
            "read_thread_message is not implemented by this thread service".to_string(),
        ))
    }

    async fn append_assistant_draft(
        &self,
        request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError>;

    async fn append_finalized_assistant_message(
        &self,
        request: AppendFinalizedAssistantMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        let scope = request.scope;
        let thread_id = request.thread_id;
        let content = request.content;
        let message = self
            .append_assistant_draft(AppendAssistantDraftRequest {
                scope: scope.clone(),
                thread_id: thread_id.clone(),
                turn_run_id: request.turn_run_id,
                content: crate::MessageContent::text(content.as_text()),
            })
            .await?;
        if message.status != crate::MessageStatus::Draft {
            return Ok(message);
        }
        self.finalize_assistant_message(&scope, &thread_id, message.message_id, content)
            .await
    }

    async fn append_tool_result_reference(
        &self,
        request: AppendToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError>;

    async fn append_capability_display_preview(
        &self,
        request: AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError>;

    async fn update_tool_result_reference(
        &self,
        request: UpdateToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError>;

    async fn put_tool_result_record(
        &self,
        _request: PutToolResultRecordRequest,
    ) -> Result<(), SessionThreadError> {
        Err(SessionThreadError::Backend(
            "tool result records are not implemented by this SessionThreadService backend"
                .to_string(),
        ))
    }

    async fn read_tool_result_record(
        &self,
        _request: ReadToolResultRecordRequest,
    ) -> Result<Option<ToolResultRecordChunk>, SessionThreadError> {
        Err(SessionThreadError::Backend(
            "tool result records are not implemented by this SessionThreadService backend"
                .to_string(),
        ))
    }

    async fn update_tool_result_record(
        &self,
        _request: UpdateToolResultRecordRequest,
    ) -> Result<(), SessionThreadError> {
        Err(SessionThreadError::Backend(
            "tool result records are not implemented by this SessionThreadService backend"
                .to_string(),
        ))
    }

    async fn delete_tool_result_record(
        &self,
        _request: DeleteToolResultRecordRequest,
    ) -> Result<(), SessionThreadError> {
        Err(SessionThreadError::Backend(
            "tool result records are not implemented by this SessionThreadService backend"
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

    /// Load a complete history only when it fits the supplied export budget.
    ///
    /// Implementations must enforce the budget while reading, rather than
    /// materializing an unbounded transcript and checking afterward.
    async fn list_thread_messages_bounded(
        &self,
        _request: BoundedThreadMessagesRequest,
    ) -> Result<BoundedThreadMessages, SessionThreadError> {
        Err(SessionThreadError::Backend(
            "bounded thread messages are not implemented by this SessionThreadService backend"
                .to_string(),
        ))
    }

    async fn list_thread_messages_range(
        &self,
        request: ThreadMessageRangeRequest,
    ) -> Result<ThreadMessageRange, SessionThreadError> {
        let history = self
            .list_thread_history(ThreadHistoryRequest {
                scope: request.scope,
                thread_id: request.thread_id,
            })
            .await?;
        Ok(ThreadMessageRange {
            thread: history.thread,
            messages: history
                .messages
                .into_iter()
                .filter(|message| {
                    message.sequence > request.after_sequence
                        && message.sequence <= request.through_sequence
                })
                .collect(),
        })
    }

    async fn latest_thread_message(
        &self,
        request: LatestThreadMessageRequest,
    ) -> Result<Option<ThreadMessageRecord>, SessionThreadError> {
        let history = self
            .list_thread_history(ThreadHistoryRequest {
                scope: request.scope,
                thread_id: request.thread_id,
            })
            .await?;
        Ok(history
            .messages
            .into_iter()
            .rev()
            .find(|message| message.kind == request.kind && message.status == request.status))
    }

    async fn finalized_assistant_message_by_run(
        &self,
        request: FinalizedAssistantMessageByRunRequest,
    ) -> Result<Option<ThreadMessageRecord>, SessionThreadError> {
        let history = self
            .list_thread_history(ThreadHistoryRequest {
                scope: request.scope,
                thread_id: request.thread_id,
            })
            .await?;
        Ok(history.messages.into_iter().rev().find(|message| {
            message.kind == crate::MessageKind::Assistant
                && message.status == crate::MessageStatus::Finalized
                && message.turn_run_id.as_deref() == Some(request.turn_run_id.as_str())
        }))
    }

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

    /// Delete a thread and its transcript only when it belongs to the supplied
    /// exact scope. Implementations must return the same non-enumerating
    /// missing shape for absent and cross-scope threads.
    async fn delete_thread(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
    ) -> Result<(), SessionThreadError> {
        Err(SessionThreadError::Backend(
            "delete_thread is not implemented by this SessionThreadService backend".to_string(),
        ))
    }

    async fn create_summary_artifact(
        &self,
        request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError>;

    /// Returns `true` when `resolve_scope` is a backend-supported operation.
    ///
    /// Callers use this to decide whether they should probe the backend for
    /// the thread's scope or fall back to the already trusted expected scope.
    /// Backends that cannot resolve scope directly should leave the default
    /// `false` in place.
    fn supports_resolve_scope(&self) -> bool {
        false
    }

    async fn resolve_scope(&self, _thread_id: ThreadId) -> Result<ThreadScope, SessionThreadError> {
        Err(SessionThreadError::Backend(
            "resolve_scope is not implemented by this SessionThreadService backend".to_string(),
        ))
    }

    async fn update_thread_goal(
        &self,
        _request: UpdateThreadGoalRequest,
    ) -> Result<ThreadGoal, SessionThreadError> {
        Err(SessionThreadError::Backend(
            "update_thread_goal is not implemented by this SessionThreadService backend"
                .to_string(),
        ))
    }

    async fn read_thread_by_id(
        &self,
        _thread_id: ThreadId,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        Err(SessionThreadError::Backend(
            "read_thread_by_id is not implemented by this SessionThreadService backend".to_string(),
        ))
    }

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

#[async_trait]
impl<S> SessionThreadService for Arc<S>
where
    S: SessionThreadService + ?Sized,
{
    async fn ensure_thread(
        &self,
        request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        self.as_ref().ensure_thread(request).await
    }

    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        self.as_ref().accept_inbound_message(request).await
    }

    async fn accept_inbound_message_with_replay_metadata(
        &self,
        request: AcceptInboundMessageRequest,
        replay_metadata: InboundMessageReplayMetadata,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        self.as_ref()
            .accept_inbound_message_with_replay_metadata(request, replay_metadata)
            .await
    }

    async fn accept_prepared_context(
        &self,
        request: crate::PreparedContextRequest,
    ) -> Result<crate::AcceptedPreparedContext, SessionThreadError> {
        self.as_ref().accept_prepared_context(request).await
    }

    async fn read_prepared_context(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<Option<crate::PreparedContextRecord>, SessionThreadError> {
        self.as_ref().read_prepared_context(scope, thread_id).await
    }

    async fn read_structured_finalization(
        &self,
        request: ReadStructuredFinalizationRequest,
    ) -> Result<Option<StructuredFinalizationRecord>, SessionThreadError> {
        self.as_ref().read_structured_finalization(request).await
    }

    async fn put_structured_finalization(
        &self,
        request: PutStructuredFinalizationRequest,
    ) -> Result<StructuredFinalizationRecord, SessionThreadError> {
        self.as_ref().put_structured_finalization(request).await
    }

    async fn publish_structured_finalization_message(
        &self,
        request: PublishStructuredFinalizationMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.as_ref()
            .publish_structured_finalization_message(request)
            .await
    }

    async fn replay_accepted_inbound_message(
        &self,
        request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
        self.as_ref().replay_accepted_inbound_message(request).await
    }

    async fn mark_message_submitted(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        turn_id: String,
        turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.as_ref()
            .mark_message_submitted(scope, thread_id, message_id, turn_id, turn_run_id)
            .await
    }

    async fn mark_message_rejected_busy(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.as_ref()
            .mark_message_rejected_busy(scope, thread_id, message_id)
            .await
    }

    async fn mark_message_queued(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        active_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.as_ref()
            .mark_message_queued(scope, thread_id, message_id, active_run_id)
            .await
    }

    async fn read_thread_message(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<Option<ThreadMessageRecord>, SessionThreadError> {
        self.as_ref()
            .read_thread_message(scope, thread_id, message_id)
            .await
    }

    async fn append_assistant_draft(
        &self,
        request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.as_ref().append_assistant_draft(request).await
    }

    async fn append_finalized_assistant_message(
        &self,
        request: AppendFinalizedAssistantMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.as_ref()
            .append_finalized_assistant_message(request)
            .await
    }

    async fn append_tool_result_reference(
        &self,
        request: AppendToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.as_ref().append_tool_result_reference(request).await
    }

    async fn append_capability_display_preview(
        &self,
        request: AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.as_ref()
            .append_capability_display_preview(request)
            .await
    }

    async fn update_tool_result_reference(
        &self,
        request: UpdateToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.as_ref().update_tool_result_reference(request).await
    }

    async fn put_tool_result_record(
        &self,
        request: PutToolResultRecordRequest,
    ) -> Result<(), SessionThreadError> {
        self.as_ref().put_tool_result_record(request).await
    }

    async fn read_tool_result_record(
        &self,
        request: ReadToolResultRecordRequest,
    ) -> Result<Option<ToolResultRecordChunk>, SessionThreadError> {
        self.as_ref().read_tool_result_record(request).await
    }

    async fn update_tool_result_record(
        &self,
        request: UpdateToolResultRecordRequest,
    ) -> Result<(), SessionThreadError> {
        self.as_ref().update_tool_result_record(request).await
    }

    async fn delete_tool_result_record(
        &self,
        request: DeleteToolResultRecordRequest,
    ) -> Result<(), SessionThreadError> {
        self.as_ref().delete_tool_result_record(request).await
    }

    async fn update_assistant_draft(
        &self,
        request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.as_ref().update_assistant_draft(request).await
    }

    async fn finalize_assistant_message(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.as_ref()
            .finalize_assistant_message(scope, thread_id, message_id, content)
            .await
    }

    async fn redact_message(
        &self,
        request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.as_ref().redact_message(request).await
    }

    async fn load_context_window(
        &self,
        request: LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        self.as_ref().load_context_window(request).await
    }

    async fn load_context_messages(
        &self,
        request: LoadContextMessagesRequest,
    ) -> Result<ContextMessages, SessionThreadError> {
        self.as_ref().load_context_messages(request).await
    }

    async fn list_thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        self.as_ref().list_thread_history(request).await
    }

    async fn list_thread_messages_bounded(
        &self,
        request: BoundedThreadMessagesRequest,
    ) -> Result<BoundedThreadMessages, SessionThreadError> {
        self.as_ref().list_thread_messages_bounded(request).await
    }

    async fn list_thread_messages_range(
        &self,
        request: ThreadMessageRangeRequest,
    ) -> Result<ThreadMessageRange, SessionThreadError> {
        self.as_ref().list_thread_messages_range(request).await
    }

    async fn latest_thread_message(
        &self,
        request: LatestThreadMessageRequest,
    ) -> Result<Option<ThreadMessageRecord>, SessionThreadError> {
        self.as_ref().latest_thread_message(request).await
    }

    async fn finalized_assistant_message_by_run(
        &self,
        request: FinalizedAssistantMessageByRunRequest,
    ) -> Result<Option<ThreadMessageRecord>, SessionThreadError> {
        self.as_ref()
            .finalized_assistant_message_by_run(request)
            .await
    }

    async fn read_thread(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        self.as_ref().read_thread(request).await
    }

    async fn delete_thread(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<(), SessionThreadError> {
        self.as_ref().delete_thread(scope, thread_id).await
    }

    async fn create_summary_artifact(
        &self,
        request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError> {
        self.as_ref().create_summary_artifact(request).await
    }

    fn supports_resolve_scope(&self) -> bool {
        self.as_ref().supports_resolve_scope()
    }

    async fn resolve_scope(&self, thread_id: ThreadId) -> Result<ThreadScope, SessionThreadError> {
        self.as_ref().resolve_scope(thread_id).await
    }

    async fn update_thread_goal(
        &self,
        request: UpdateThreadGoalRequest,
    ) -> Result<ThreadGoal, SessionThreadError> {
        self.as_ref().update_thread_goal(request).await
    }

    async fn read_thread_by_id(
        &self,
        thread_id: ThreadId,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        self.as_ref().read_thread_by_id(thread_id).await
    }

    async fn list_threads_for_scope(
        &self,
        request: ListThreadsForScopeRequest,
    ) -> Result<ListThreadsForScopeResponse, SessionThreadError> {
        self.as_ref().list_threads_for_scope(request).await
    }
}
