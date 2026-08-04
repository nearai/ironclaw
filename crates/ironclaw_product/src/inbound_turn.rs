//! InboundTurnService — the user-message turn submission path.
//!
//! This is the narrower user-message subset of [`ProductSurface`]. It
//! resolves product adapter envelopes into a thread-bound accepted message, then
//! hands off to the accepted-message turn submission seam. Keeping replay and
//! submit/deferred handling behind that seam prevents adapter-specific binding
//! code from owning the whole inbound turn pipeline.

// arch-exempt: large_file, busy-branch steering enqueue lands in the owning inbound-turn path, plan #5981
use std::time::Duration;
use std::{collections::BTreeSet, sync::Arc};

use crate::{
    ChannelAttachmentRef, ChannelError, ProductAdapterId, ProductInboundAck,
    ProductInboundEnvelope, ProductInboundPayload, ProductRejection, ProductSourceChannel,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS;
use ironclaw_extension_contracts::channel_adapter::ChannelAdapter;
use ironclaw_extension_contracts::tool_adapter::RestrictedEgress;
use ironclaw_host_api::attachment::InboundAttachment;
#[cfg(test)]
use ironclaw_host_api::ids::UserId;
use ironclaw_loop_host::HostInputEnqueuePort;
#[cfg(doc)]
use ironclaw_loop_host::RejectingInputEnqueue;
use ironclaw_threads::{
    AcceptInboundMessageRequest, AcceptedInboundMessageReplay, EnsureThreadRequest,
    ListThreadsForScopeRequest, MessageContent, MessageStatus, ReplayAcceptedInboundMessageRequest,
    SessionThreadService, ThreadHistoryRequest, ThreadMessageId, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCoordinator,
    TurnError, TurnRunId, TurnScope, TurnSurfaceType,
};
use uuid::Uuid;

use crate::binding::{
    ConversationBindingService, ProductConversationBindingCreationPolicy,
    ProductConversationRouteKind, ResolveBindingRequest, ResolvedBinding,
    binding_profile_for_trigger,
};
use crate::binding_ref::{
    DEFAULT_BINDING_REF_RAW_MAX_BYTES, bounded_idempotency_key, bounded_reply_target_binding_ref,
    bounded_source_binding_ref,
};
use crate::error::ProductSurfaceFailure;
use crate::policy::{
    BeforeInboundPolicy, BeforeInboundPolicyOutcome, BeforeInboundPolicyRequest,
    NoopBeforeInboundPolicy,
};
use ironclaw_attachments::InboundAttachmentLander;

#[cfg(not(any(test, feature = "test-support")))]
const BEFORE_INBOUND_POLICY_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(any(test, feature = "test-support"))]
const BEFORE_INBOUND_POLICY_TIMEOUT: Duration = Duration::from_millis(10);
const ATTACHMENT_CLEANUP_TIMEOUT: Duration = Duration::from_millis(250);
const ATTACHMENT_CLEANUP_MAX_THREADS: usize = 50;
const ATTACHMENT_CLEANUP_MAX_MESSAGES: usize = 10_000;

/// Run a before-inbound policy with the workflow-owned wall-clock budget.
///
/// The timeout keeps slow policy backends from holding an idempotency
/// fingerprint in-flight indefinitely. A timed-out policy maps to a transient,
/// non-permanent [`ProductSurfaceFailure::BeforeInboundPolicyFailed`] so the
/// workflow releases the fingerprint and lets the same inbound action retry.
pub(crate) async fn check_before_inbound_policy(
    before_inbound_policy: &dyn BeforeInboundPolicy,
    request: BeforeInboundPolicyRequest,
) -> Result<BeforeInboundPolicyOutcome, ProductSurfaceFailure> {
    tokio::time::timeout(
        BEFORE_INBOUND_POLICY_TIMEOUT,
        before_inbound_policy.check_user_message(request),
    )
    .await
    .map_err(|_| ProductSurfaceFailure::BeforeInboundPolicyFailed {
        reason: "before-inbound policy timed out".into(),
        permanent: false,
    })?
}

/// Result of the inbound turn submission flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboundTurnOutcome {
    /// Turn was accepted and submitted to the coordinator.
    Submitted {
        accepted_message_ref: AcceptedMessageRef,
        submitted_run_id: TurnRunId,
        binding: ResolvedBinding,
    },
    /// Turn submission was busy (thread already has an active run). The message
    /// was recorded as RejectedBusy — it will NOT be auto-resubmitted; the user
    /// must resend once the current task finishes.
    RejectedBusy {
        accepted_message_ref: AcceptedMessageRef,
        active_run_id: Option<TurnRunId>,
        binding: ResolvedBinding,
    },
    DeferredBusy {
        accepted_message_ref: AcceptedMessageRef,
        active_run_id: TurnRunId,
        binding: ResolvedBinding,
    },
}

impl InboundTurnOutcome {
    /// Convert to a product-safe acknowledgement for the adapter.
    pub fn to_ack(&self) -> ProductInboundAck {
        match self {
            Self::Submitted {
                accepted_message_ref,
                submitted_run_id,
                ..
            } => ProductInboundAck::Accepted {
                accepted_message_ref: accepted_message_ref.clone(),
                submitted_run_id: *submitted_run_id,
            },
            Self::RejectedBusy {
                accepted_message_ref,
                active_run_id,
                ..
            } => ProductInboundAck::RejectedBusy {
                accepted_message_ref: accepted_message_ref.clone(),
                active_run_id: *active_run_id,
            },
            Self::DeferredBusy {
                accepted_message_ref,
                active_run_id,
                ..
            } => ProductInboundAck::DeferredBusy {
                accepted_message_ref: accepted_message_ref.clone(),
                active_run_id: *active_run_id,
            },
        }
    }
}

/// Result of running replay, before-inbound policy, and fresh user-message acceptance.
pub enum InboundUserMessageDispatch {
    Accepted(InboundTurnOutcome),
    Rejected(ProductRejection),
}

struct PreparedUserMessage {
    binding: ResolvedBinding,
    thread_scope: ThreadScope,
    source_binding_id: String,
    submit_idempotency_key: String,
    adapter_id: ProductAdapterId,
    source_channel: ProductSourceChannel,
    surface_type: TurnSurfaceType,
}

struct ReplaySubmissionContext {
    binding: ResolvedBinding,
    thread_scope: ThreadScope,
    adapter_id: ProductAdapterId,
    source_channel: ProductSourceChannel,
    surface_type: TurnSurfaceType,
}

/// Port for the inbound turn submission path.
///
/// Implementations coordinate binding resolution, message acceptance into the
/// session thread service, and turn submission to the coordinator.
#[async_trait]
pub trait InboundTurnService: Send + Sync {
    /// Replay an already-accepted inbound message, if one exists.
    ///
    /// The product workflow calls this before before-inbound policy so retries
    /// of staged messages are not blocked by later policy changes. Implementors
    /// must keep this probe separate from fresh acceptance so callers never
    /// perform replay lookup twice for one inbound dispatch.
    async fn replay_accepted_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<Option<InboundTurnOutcome>, ProductSurfaceFailure>;

    /// Accept a user message envelope: resolve binding, stage message, submit turn.
    async fn accept_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<InboundTurnOutcome, ProductSurfaceFailure>;

    /// Accept a user message while preserving the replay-before-policy ordering.
    async fn accept_user_message_with_before_policy(
        &self,
        envelope: &ProductInboundEnvelope,
        before_inbound_policy: &dyn BeforeInboundPolicy,
    ) -> Result<InboundUserMessageDispatch, ProductSurfaceFailure>;

    /// Accept a user message together with host-staged inline attachment bytes.
    ///
    /// `attachments` carries decoded bytes a synchronous host surface (e.g. the
    /// OpenAI-compatible API) received inline — never serialized into the
    /// bytes-free [`ProductInboundEnvelope`]. The implementation lands them into
    /// project storage before message acceptance.
    ///
    /// The default supports only the no-attachment case: with no attachments it
    /// delegates to [`Self::accept_user_message_with_before_policy`], but a
    /// non-empty `attachments` list is **rejected** rather than silently
    /// dropped — an implementation that has no landing path must fail closed so
    /// a user's files never vanish. Implementations with an inline-bytes surface
    /// override this.
    async fn accept_user_message_with_before_policy_and_attachments(
        &self,
        envelope: &ProductInboundEnvelope,
        before_inbound_policy: &dyn BeforeInboundPolicy,
        attachments: Vec<InboundAttachment>,
    ) -> Result<InboundUserMessageDispatch, ProductSurfaceFailure> {
        if !attachments.is_empty() {
            return Err(ProductSurfaceFailure::TurnSubmissionRejected {
                reason: "inbound attachments are not supported by this turn service".into(),
            });
        }
        self.accept_user_message_with_before_policy(envelope, before_inbound_policy)
            .await
    }

    /// Accept a channel user message with the exact transient adapter and
    /// restricted egress authority pinned by the ingress router.
    async fn accept_user_message_with_before_policy_and_channel_transfer(
        &self,
        envelope: &ProductInboundEnvelope,
        before_inbound_policy: &dyn BeforeInboundPolicy,
        _channel_adapter: Arc<dyn ChannelAdapter>,
        _channel_egress: Arc<dyn RestrictedEgress>,
    ) -> Result<InboundUserMessageDispatch, ProductSurfaceFailure> {
        if !envelope.channel_attachment_refs().is_empty() {
            return Err(permanent_attachment_failure(
                "channel attachment transfer is not supported by this turn service",
            ));
        }
        self.accept_user_message_with_before_policy(envelope, before_inbound_policy)
            .await
    }
}

/// Default implementation that composes a [`ConversationBindingService`] with a
/// [`SessionThreadService`] and [`TurnCoordinator`].
pub struct DefaultInboundTurnService<B, T, C> {
    binding_service: B,
    thread_service: T,
    turn_coordinator: C,
    inbound_attachments: Option<Arc<dyn InboundAttachmentLander>>,
    input_enqueue: Arc<dyn HostInputEnqueuePort>,
}

impl<B, T, C> DefaultInboundTurnService<B, T, C>
where
    B: ConversationBindingService,
    T: SessionThreadService,
    C: TurnCoordinator,
{
    /// `input_enqueue` is REQUIRED: production always wires the real steering
    /// queue, and a defaulted null port would silently downgrade busy submits
    /// to reject-busy. A deployment that genuinely disables steering passes
    /// [`RejectingInputEnqueue`] explicitly — a chosen mode, never a forgotten
    /// wire-up.
    pub fn new(
        binding_service: B,
        thread_service: T,
        turn_coordinator: C,
        input_enqueue: Arc<dyn HostInputEnqueuePort>,
    ) -> Self {
        Self {
            binding_service,
            thread_service,
            turn_coordinator,
            inbound_attachments: None,
            input_enqueue,
        }
    }

    /// Wire the port that lands inline attachment bytes into project storage
    /// before message acceptance. Without it, a turn carrying attachments is
    /// rejected rather than silently dropping the files.
    pub fn with_inbound_attachments(
        mut self,
        inbound_attachments: Arc<dyn InboundAttachmentLander>,
    ) -> Self {
        self.inbound_attachments = Some(inbound_attachments);
        self
    }

    async fn reconcile_stale_attachment_batches(&self, thread_scope: &ThreadScope) {
        let Some(lander) = self.inbound_attachments.as_ref() else {
            return;
        };
        let reconciliation = async {
            let page = self
                .thread_service
                .list_threads_for_scope(ListThreadsForScopeRequest {
                    scope: thread_scope.clone(),
                    limit: Some((ATTACHMENT_CLEANUP_MAX_THREADS + 1) as u32),
                    cursor: None,
                })
                .await
                .map_err(|error| error.to_string())?;
            if page.next_cursor.is_some() || page.threads.len() > ATTACHMENT_CLEANUP_MAX_THREADS {
                return Ok(None);
            }

            let mut storage_keys = BTreeSet::new();
            let mut message_count = 0usize;
            for thread in page.threads {
                let history = self
                    .thread_service
                    .list_thread_history(ThreadHistoryRequest {
                        scope: thread_scope.clone(),
                        thread_id: thread.thread_id,
                    })
                    .await
                    .map_err(|error| error.to_string())?;
                message_count = message_count.saturating_add(history.messages.len());
                if message_count > ATTACHMENT_CLEANUP_MAX_MESSAGES {
                    return Ok(None);
                }
                storage_keys.extend(
                    history
                        .messages
                        .into_iter()
                        .flat_map(|message| message.attachments)
                        .filter_map(|attachment| attachment.storage_key),
                );
            }
            let storage_keys = storage_keys.into_iter().collect::<Vec<_>>();
            lander
                .cleanup_stale(thread_scope, &storage_keys)
                .await
                .map(Some)
                .map_err(|error| error.to_string())
        };

        match tokio::time::timeout(ATTACHMENT_CLEANUP_TIMEOUT, reconciliation).await {
            Ok(Ok(Some(report))) if report.deleted_batches > 0 => {
                tracing::debug!(
                    scanned_batches = report.scanned_batches,
                    deleted_batches = report.deleted_batches,
                    "reconciled stale inbound attachment batches"
                );
            }
            Ok(Ok(Some(_))) => {}
            Ok(Ok(None)) => {
                tracing::debug!(
                    "skipped stale inbound attachment cleanup because the durable reference scan \
                     exceeded its safety bound"
                );
            }
            Ok(Err(reason)) => {
                tracing::warn!(
                    reason = %reason,
                    "best-effort stale inbound attachment cleanup failed"
                );
            }
            Err(_) => {
                tracing::warn!("best-effort stale inbound attachment cleanup timed out");
            }
        }
    }
}

#[async_trait]
impl<B, T, C> InboundTurnService for DefaultInboundTurnService<B, T, C>
where
    B: ConversationBindingService,
    T: SessionThreadService,
    C: TurnCoordinator,
{
    async fn replay_accepted_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<Option<InboundTurnOutcome>, ProductSurfaceFailure> {
        let prepared = self.prepare_user_message(envelope).await?;
        self.replay_prepared_user_message(envelope, &prepared).await
    }

    async fn accept_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<InboundTurnOutcome, ProductSurfaceFailure> {
        let policy = NoopBeforeInboundPolicy;
        match self
            .accept_user_message_with_before_policy(envelope, &policy)
            .await?
        {
            InboundUserMessageDispatch::Accepted(outcome) => Ok(outcome),
            InboundUserMessageDispatch::Rejected(_) => {
                Err(ProductSurfaceFailure::TurnSubmissionRejected {
                    reason: "noop before-inbound policy unexpectedly rejected message".into(),
                })
            }
        }
    }

    async fn accept_user_message_with_before_policy(
        &self,
        envelope: &ProductInboundEnvelope,
        before_inbound_policy: &dyn BeforeInboundPolicy,
    ) -> Result<InboundUserMessageDispatch, ProductSurfaceFailure> {
        self.accept_with_before_policy_inner(
            envelope,
            before_inbound_policy,
            Vec::new(),
            None,
            None,
        )
        .await
    }

    async fn accept_user_message_with_before_policy_and_attachments(
        &self,
        envelope: &ProductInboundEnvelope,
        before_inbound_policy: &dyn BeforeInboundPolicy,
        attachments: Vec<InboundAttachment>,
    ) -> Result<InboundUserMessageDispatch, ProductSurfaceFailure> {
        self.accept_with_before_policy_inner(
            envelope,
            before_inbound_policy,
            attachments,
            None,
            None,
        )
        .await
    }

    async fn accept_user_message_with_before_policy_and_channel_transfer(
        &self,
        envelope: &ProductInboundEnvelope,
        before_inbound_policy: &dyn BeforeInboundPolicy,
        channel_adapter: Arc<dyn ChannelAdapter>,
        channel_egress: Arc<dyn RestrictedEgress>,
    ) -> Result<InboundUserMessageDispatch, ProductSurfaceFailure> {
        self.accept_with_before_policy_inner(
            envelope,
            before_inbound_policy,
            Vec::new(),
            Some(channel_adapter),
            Some(channel_egress),
        )
        .await
    }
}

impl<B, T, C> DefaultInboundTurnService<B, T, C>
where
    B: ConversationBindingService,
    T: SessionThreadService,
    C: TurnCoordinator,
{
    async fn accept_with_before_policy_inner(
        &self,
        envelope: &ProductInboundEnvelope,
        before_inbound_policy: &dyn BeforeInboundPolicy,
        attachments: Vec<InboundAttachment>,
        pinned_channel_adapter: Option<Arc<dyn ChannelAdapter>>,
        pinned_channel_egress: Option<Arc<dyn RestrictedEgress>>,
    ) -> Result<InboundUserMessageDispatch, ProductSurfaceFailure> {
        let ProductInboundPayload::UserMessage(payload) = envelope.payload() else {
            return Err(ProductSurfaceFailure::UnsupportedActionKind {
                kind: "non_user_message".into(),
            });
        };
        let original_trigger = payload.trigger;
        let prepared = self.prepare_user_message(envelope).await?;
        if let Some(outcome) = self
            .replay_prepared_user_message(envelope, &prepared)
            .await?
        {
            return Ok(InboundUserMessageDispatch::Accepted(outcome));
        }

        // Bound the original untrusted channel descriptor set before it
        // reaches a policy backend. Rewritten descriptors are reconciled and
        // revalidated later, immediately before provider fetch.
        if !envelope.channel_attachment_refs().is_empty() {
            validate_attachment_sources(
                payload.attachments.as_slice(),
                envelope.channel_attachment_refs(),
            )?;
        }

        let policy_outcome = check_before_inbound_policy(
            before_inbound_policy,
            BeforeInboundPolicyRequest::new(envelope, payload)?,
        )
        .await?;
        let dispatch_envelope;
        let (prepared_for_turn, envelope_for_turn) = match policy_outcome {
            BeforeInboundPolicyOutcome::Allow => (prepared, envelope),
            BeforeInboundPolicyOutcome::RewriteUserMessage(payload) => {
                let rewritten_trigger = payload.trigger;
                dispatch_envelope =
                    envelope.with_rewritten_user_message(payload).map_err(|_| {
                        ProductSurfaceFailure::TurnSubmissionRejected {
                            reason: "invalid policy-rewritten user message".into(),
                        }
                    })?;
                let prepared_for_turn = if rewritten_trigger == original_trigger {
                    prepared
                } else {
                    self.prepare_user_message(&dispatch_envelope).await?
                };
                (prepared_for_turn, &dispatch_envelope)
            }
            BeforeInboundPolicyOutcome::Reject(rejection) => {
                return Ok(InboundUserMessageDispatch::Rejected(rejection));
            }
        };

        let attachments = self
            .resolve_inbound_attachments(
                envelope_for_turn,
                attachments,
                pinned_channel_adapter.as_deref(),
                pinned_channel_egress.as_deref(),
            )
            .await?;

        self.accept_prepared_user_message(prepared_for_turn, envelope_for_turn, attachments)
            .await
            .map(InboundUserMessageDispatch::Accepted)
    }

    async fn resolve_inbound_attachments(
        &self,
        envelope: &ProductInboundEnvelope,
        inline_attachments: Vec<InboundAttachment>,
        pinned_channel_adapter: Option<&dyn ChannelAdapter>,
        pinned_channel_egress: Option<&dyn RestrictedEgress>,
    ) -> Result<Vec<InboundAttachment>, ProductSurfaceFailure> {
        let ProductInboundPayload::UserMessage(payload) = envelope.payload() else {
            return Err(ProductSurfaceFailure::UnsupportedActionKind {
                kind: "non_user_message".into(),
            });
        };
        let sources = envelope.channel_attachment_refs();
        if !inline_attachments.is_empty() {
            if !sources.is_empty() {
                return Err(permanent_attachment_failure(
                    "mixed inline and channel attachment sources are not allowed",
                ));
            }
            return Ok(inline_attachments);
        }
        if payload.attachments.is_empty() {
            return Ok(Vec::new());
        }
        if sources.is_empty() {
            return Err(permanent_attachment_failure(
                "attachment descriptors have no channel transfer references",
            ));
        }
        validate_attachment_sources(payload.attachments.as_slice(), sources)?;

        let adapter = pinned_channel_adapter.ok_or_else(|| {
            permanent_attachment_failure("channel attachment transfer is not configured")
        })?;
        let egress = pinned_channel_egress.ok_or_else(|| {
            permanent_attachment_failure("channel attachment transfer is not configured")
        })?;
        let mut fetched = Vec::with_capacity(sources.len());
        let mut total_bytes = 0usize;
        for source in sources {
            let mut attachment = adapter
                .fetch_attachment(source, egress)
                .await
                .map_err(channel_attachment_error)?;
            if attachment.id != source.descriptor.external_file_id {
                return Err(permanent_attachment_failure(
                    "fetched attachment id does not match its descriptor",
                ));
            }
            // Compare canonical forms on both sides. The descriptor keeps the
            // vendor's raw media type (which legitimately carries parameters
            // such as `; charset=utf-8`), so normalizing only the fetched side
            // would reject every attachment whose declared type is not already
            // canonical instead of catching a genuine provider mismatch.
            let mime_type = ironclaw_common::normalize_mime_type(&attachment.mime_type);
            let declared_mime_type =
                ironclaw_common::normalize_mime_type(&source.descriptor.mime_type);
            if mime_type != declared_mime_type || !ironclaw_common::is_supported_mime(&mime_type) {
                return Err(permanent_attachment_failure(
                    "fetched attachment MIME type does not match its descriptor",
                ));
            }
            attachment.mime_type = mime_type;
            // The descriptor's filename wins when the vendor supplied one;
            // otherwise keep whatever the adapter recovered. Some vendor
            // payload kinds (inline photos, voice notes, stickers) carry no
            // filename at all, and the adapter derives one from the provider
            // download path — overwriting unconditionally discarded it.
            if let Some(descriptor_filename) = source.descriptor.filename.clone() {
                attachment.filename = Some(descriptor_filename);
            }
            if attachment.bytes.len() > DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes {
                return Err(permanent_attachment_failure(
                    "attachment exceeds the per-file byte limit",
                ));
            }
            if let Some(declared_size) = source.descriptor.size_bytes
                && declared_size != attachment.bytes.len() as u64
            {
                return Err(ProductSurfaceFailure::InboundAttachmentFailed {
                    reason: "fetched attachment size does not match its descriptor".into(),
                    retryable: true,
                });
            }
            total_bytes = total_bytes.saturating_add(attachment.bytes.len());
            if total_bytes > DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes {
                return Err(permanent_attachment_failure(
                    "attachments exceed the total byte limit",
                ));
            }
            fetched.push(attachment);
        }
        Ok(fetched)
    }

    async fn prepare_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<PreparedUserMessage, ProductSurfaceFailure> {
        let ProductInboundPayload::UserMessage(payload) = envelope.payload() else {
            return Err(ProductSurfaceFailure::UnsupportedActionKind {
                kind: "non_user_message".into(),
            });
        };
        let (route_kind, creation_policy) = binding_profile_for_trigger(payload.trigger);
        let surface_type = match route_kind {
            ProductConversationRouteKind::Direct => TurnSurfaceType::Direct,
            ProductConversationRouteKind::Shared => TurnSurfaceType::Channel,
        };
        let binding_request = ResolveBindingRequest {
            adapter_id: envelope.adapter_id().clone(),
            installation_id: envelope.installation_id().clone(),
            external_actor_ref: envelope.external_actor_ref().clone(),
            external_conversation_ref: envelope.external_conversation_ref().clone(),
            external_event_id: envelope.external_event_id().clone(),
            route_kind,
            auth_claim: envelope.auth_claim().clone(),
        };
        let binding = match creation_policy {
            ProductConversationBindingCreationPolicy::CreateAllowed => {
                self.binding_service
                    .resolve_binding(binding_request)
                    .await?
            }
            ProductConversationBindingCreationPolicy::ExistingOnly => {
                self.binding_service.lookup_binding(binding_request).await?
            }
        };
        let source_binding_id = product_source_binding_id(envelope, &binding);
        let submit_idempotency_key = submit_idempotency_key(envelope, &binding);
        let thread_scope = thread_scope_from_binding(&binding)?;
        Ok(PreparedUserMessage {
            binding,
            thread_scope,
            source_binding_id,
            submit_idempotency_key,
            adapter_id: envelope.adapter_id().clone(),
            source_channel: envelope.source_channel().clone(),
            surface_type,
        })
    }

    async fn replay_prepared_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
        prepared: &PreparedUserMessage,
    ) -> Result<Option<InboundTurnOutcome>, ProductSurfaceFailure> {
        let Some(replay) = self
            .thread_service
            .replay_accepted_inbound_message(ReplayAcceptedInboundMessageRequest {
                scope: prepared.thread_scope.clone(),
                actor_id: prepared.binding.actor_user_id.as_str().to_string(),
                source_binding_id: prepared.source_binding_id.clone(),
                external_event_id: envelope.external_event_id().as_str().to_string(),
            })
            .await
            .map_err(|e| ProductSurfaceFailure::Transient {
                reason: format!("failed to replay accepted inbound message: {e}"),
            })?
        else {
            return Ok(None);
        };

        submit_or_replay_accepted_message(
            &self.thread_service,
            &self.turn_coordinator,
            self.input_enqueue.as_ref(),
            replay,
            prepared.submit_idempotency_key.clone(),
            envelope.received_at(),
            prepared,
        )
        .await
        .map(Some)
    }

    async fn accept_prepared_user_message(
        &self,
        prepared: PreparedUserMessage,
        envelope: &ProductInboundEnvelope,
        attachments: Vec<InboundAttachment>,
    ) -> Result<InboundTurnOutcome, ProductSurfaceFailure> {
        let ProductInboundPayload::UserMessage(payload) = envelope.payload() else {
            return Err(ProductSurfaceFailure::UnsupportedActionKind {
                kind: "non_user_message".into(),
            });
        };
        self.thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: prepared.thread_scope.clone(),
                thread_id: Some(prepared.binding.thread_id.clone()),
                created_by_actor_id: prepared.binding.actor_user_id.as_str().to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .map_err(|e| ProductSurfaceFailure::Transient {
                reason: format!("failed to ensure thread: {e}"),
            })?;

        // Inbound attachment bytes (inline or fetched after channel policy)
        // are landed into project storage through the same authority
        // the agent's file tools resolve through, then carried on the message as
        // refs — never as raw bytes through the bytes-free product envelope.
        let (content, landed_refs) = if attachments.is_empty() {
            (MessageContent::text(payload.text.clone()), None)
        } else {
            let lander = self.inbound_attachments.as_ref().ok_or_else(|| {
                ProductSurfaceFailure::TurnSubmissionRejected {
                    reason: "inbound attachment lander not configured".into(),
                }
            })?;
            let refs = lander
                .land(
                    &prepared.thread_scope,
                    envelope.external_event_id().as_str(),
                    attachments,
                )
                .await
                .map_err(|e| ProductSurfaceFailure::Transient {
                    reason: format!("failed to land inbound attachments: {e}"),
                })?;
            (
                MessageContent::with_attachments(payload.text.clone(), refs.clone()),
                Some(refs),
            )
        };

        let reply_target_binding_id = prepared.source_binding_id.clone();
        let accepted = match self
            .thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: prepared.thread_scope.clone(),
                thread_id: prepared.binding.thread_id.clone(),
                actor_id: prepared.binding.actor_user_id.as_str().to_string(),
                source_binding_id: Some(prepared.source_binding_id.clone()),
                reply_target_binding_id: Some(reply_target_binding_id.clone()),
                external_event_id: Some(envelope.external_event_id().as_str().to_string()),
                content,
            })
            .await
        {
            Ok(accepted) => accepted,
            Err(error) => {
                let acceptance_reason = format!("failed to accept inbound message: {error}");
                if let Some(refs) = landed_refs {
                    let lander = self.inbound_attachments.as_ref().ok_or_else(|| {
                        ProductSurfaceFailure::Transient {
                            reason: format!("{acceptance_reason}; attachment rollback unavailable"),
                        }
                    })?;
                    if let Err(rollback_error) =
                        lander.rollback(&prepared.thread_scope, &refs).await
                    {
                        return Err(ProductSurfaceFailure::Transient {
                            reason: format!(
                                "{acceptance_reason}; failed to roll back inbound attachments: \
                                 {rollback_error}"
                            ),
                        });
                    }
                }
                return Err(ProductSurfaceFailure::Transient {
                    reason: acceptance_reason,
                });
            }
        };

        let cleanup_scope = prepared.thread_scope.clone();
        let cleanup_needed = landed_refs.is_some();
        let outcome =
            ProductInboundTurnHandoff::NeedsSubmission(Box::new(AcceptedProductInboundTurn {
                binding: prepared.binding,
                thread_scope: prepared.thread_scope,
                message_id: accepted.message_id,
                source_binding_id: prepared.source_binding_id,
                reply_target_binding_id,
                idempotency_key_raw: prepared.submit_idempotency_key,
                received_at: envelope.received_at(),
                adapter_id: prepared.adapter_id,
                source_channel: prepared.source_channel,
                surface_type: prepared.surface_type,
                requested_model: payload.requested_model.clone(),
            }))
            .submit_or_replay(
                &self.thread_service,
                &self.turn_coordinator,
                self.input_enqueue.as_ref(),
            )
            .await?;
        if cleanup_needed {
            self.reconcile_stale_attachment_batches(&cleanup_scope)
                .await;
        }
        Ok(outcome)
    }
}

fn validate_attachment_sources(
    descriptors: &[crate::ProductAttachmentDescriptor],
    sources: &[ChannelAttachmentRef],
) -> Result<(), ProductSurfaceFailure> {
    if descriptors.len() != sources.len() {
        return Err(permanent_attachment_failure(
            "channel attachment references do not match message descriptors",
        ));
    }
    if sources.len() > DEFAULT_ATTACHMENT_BUDGETS.max_count {
        return Err(permanent_attachment_failure(
            "attachments exceed the count limit",
        ));
    }
    let mut declared_total = 0u64;
    for (descriptor, source) in descriptors.iter().zip(sources) {
        if descriptor != &source.descriptor {
            return Err(permanent_attachment_failure(
                "channel attachment references do not match message descriptors",
            ));
        }
        if !ironclaw_common::is_supported_mime(&descriptor.mime_type) {
            return Err(permanent_attachment_failure(
                "attachment MIME type is not supported",
            ));
        }
        if let Some(size) = descriptor.size_bytes {
            if size > DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes as u64 {
                return Err(permanent_attachment_failure(
                    "attachment exceeds the per-file byte limit",
                ));
            }
            declared_total = declared_total.saturating_add(size);
            if declared_total > DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes as u64 {
                return Err(permanent_attachment_failure(
                    "attachments exceed the total byte limit",
                ));
            }
        }
    }
    Ok(())
}

fn channel_attachment_error(error: ChannelError) -> ProductSurfaceFailure {
    match error {
        ChannelError::AttachmentTransfer { retryable, .. } => {
            ProductSurfaceFailure::InboundAttachmentFailed {
                reason: "channel attachment transfer failed".into(),
                retryable,
            }
        }
        ChannelError::Configuration { .. } => ProductSurfaceFailure::InboundAttachmentFailed {
            reason: "channel attachment transfer failed".into(),
            retryable: true,
        },
        ChannelError::Unsupported => permanent_attachment_failure(
            "channel adapter does not support inbound attachment transfer",
        ),
        ChannelError::Parse { .. }
        | ChannelError::Render { .. }
        | ChannelError::VendorWiring { .. } => {
            permanent_attachment_failure("channel attachment transfer failed")
        }
    }
}

fn permanent_attachment_failure(reason: impl Into<String>) -> ProductSurfaceFailure {
    ProductSurfaceFailure::InboundAttachmentFailed {
        reason: reason.into(),
        retryable: false,
    }
}

async fn submit_or_replay_accepted_message<T, C>(
    thread_service: &T,
    turn_coordinator: &C,
    input_enqueue: &dyn HostInputEnqueuePort,
    replay: AcceptedInboundMessageReplay,
    submit_idempotency_key: String,
    received_at: DateTime<Utc>,
    prepared: &PreparedUserMessage,
) -> Result<InboundTurnOutcome, ProductSurfaceFailure>
where
    T: SessionThreadService,
    C: TurnCoordinator,
{
    ProductInboundTurnHandoff::from_replay_with_prepared(
        replay,
        submit_idempotency_key,
        received_at,
        prepared,
    )?
    .submit_or_replay(thread_service, turn_coordinator, input_enqueue)
    .await
}

enum ProductInboundTurnHandoff {
    AlreadySubmitted {
        accepted_message_ref: AcceptedMessageRef,
        submitted_run_id: TurnRunId,
        binding: ResolvedBinding,
    },
    AlreadyRejected {
        accepted_message_ref: AcceptedMessageRef,
        binding: ResolvedBinding,
        active_run_id: Option<TurnRunId>,
    },
    AlreadyDeferred {
        accepted_message_ref: AcceptedMessageRef,
        binding: ResolvedBinding,
        active_run_id: TurnRunId,
        thread_scope: ThreadScope,
        message_id: ThreadMessageId,
    },
    NeedsSubmission(Box<AcceptedProductInboundTurn>),
}

impl ProductInboundTurnHandoff {
    #[cfg(test)]
    fn from_replay(
        replay: AcceptedInboundMessageReplay,
        submit_idempotency_key: String,
        received_at: DateTime<Utc>,
        adapter_id: ProductAdapterId,
    ) -> Result<Self, ProductSurfaceFailure> {
        let binding = binding_from_replay(&replay)?;
        let thread_scope = replay.scope.clone();
        let source_channel = ProductSourceChannel::new(adapter_id.as_str()).map_err(|e| {
            ProductSurfaceFailure::TurnSubmissionRejected {
                reason: format!("invalid source channel: {e}"),
            }
        })?;
        Self::from_replay_parts(
            replay,
            submit_idempotency_key,
            received_at,
            ReplaySubmissionContext {
                binding,
                thread_scope,
                adapter_id,
                source_channel,
                // Surface type is unknown at replay time without the original trigger.
                surface_type: TurnSurfaceType::Direct,
            },
        )
    }

    fn from_replay_with_prepared(
        replay: AcceptedInboundMessageReplay,
        submit_idempotency_key: String,
        received_at: DateTime<Utc>,
        prepared: &PreparedUserMessage,
    ) -> Result<Self, ProductSurfaceFailure> {
        Self::from_replay_parts(
            replay,
            submit_idempotency_key,
            received_at,
            ReplaySubmissionContext {
                binding: prepared.binding.clone(),
                thread_scope: prepared.thread_scope.clone(),
                adapter_id: prepared.adapter_id.clone(),
                source_channel: prepared.source_channel.clone(),
                surface_type: prepared.surface_type,
            },
        )
    }

    fn from_replay_parts(
        replay: AcceptedInboundMessageReplay,
        submit_idempotency_key: String,
        received_at: DateTime<Utc>,
        context: ReplaySubmissionContext,
    ) -> Result<Self, ProductSurfaceFailure> {
        let ReplaySubmissionContext {
            binding,
            thread_scope,
            adapter_id,
            source_channel,
            surface_type,
        } = context;
        let accepted_message_ref = accepted_message_ref(replay.message_id)?;

        if replay.status == MessageStatus::Submitted {
            let Some(turn_run_id) = replay.turn_run_id.as_deref() else {
                return Err(ProductSurfaceFailure::TurnSubmissionRejected {
                    reason: "submitted replay missing turn_run_id".into(),
                });
            };
            let submitted_run_id = Uuid::parse_str(turn_run_id)
                .map(TurnRunId::from_uuid)
                .map_err(|e| ProductSurfaceFailure::TurnSubmissionRejected {
                    reason: format!("invalid submitted turn_run_id: {e}"),
                })?;
            return Ok(Self::AlreadySubmitted {
                accepted_message_ref,
                submitted_run_id,
                binding,
            });
        }

        if replay.status == MessageStatus::RejectedBusy {
            let active_run_id = replay
                .turn_run_id
                .as_deref()
                .map(|s| {
                    Uuid::parse_str(s).map(TurnRunId::from_uuid).map_err(|e| {
                        ProductSurfaceFailure::TurnSubmissionRejected {
                            reason: format!("invalid rejected busy turn_run_id: {e}"),
                        }
                    })
                })
                .transpose()?;
            return Ok(Self::AlreadyRejected {
                accepted_message_ref,
                binding,
                active_run_id,
            });
        }

        if replay.status == MessageStatus::Queued {
            let active_run_id = crate::steering::parse_stored_run_id(replay.turn_run_id.as_deref())
                .map_err(|reason| ProductSurfaceFailure::TurnSubmissionRejected { reason })?;
            return Ok(Self::AlreadyDeferred {
                accepted_message_ref,
                binding,
                active_run_id,
                thread_scope: replay.scope.clone(),
                message_id: replay.message_id,
            });
        }

        if !matches!(
            replay.status,
            MessageStatus::Accepted | MessageStatus::DeferredBusy
        ) {
            return Err(ProductSurfaceFailure::TurnSubmissionRejected {
                reason: format!(
                    "cannot resubmit inbound message replay in {:?} status",
                    replay.status
                ),
            });
        }

        let source_binding_id = replay.source_binding_id.clone().ok_or_else(|| {
            ProductSurfaceFailure::TurnSubmissionRejected {
                reason: "accepted replay missing source_binding_id".into(),
            }
        })?;
        let reply_target_binding_id = replay.reply_target_binding_id.clone().ok_or_else(|| {
            ProductSurfaceFailure::TurnSubmissionRejected {
                reason: "accepted replay missing reply_target_binding_id".into(),
            }
        })?;

        Ok(Self::NeedsSubmission(Box::new(
            AcceptedProductInboundTurn {
                binding,
                thread_scope,
                message_id: replay.message_id,
                source_binding_id,
                reply_target_binding_id,
                idempotency_key_raw: submit_idempotency_key,
                received_at,
                adapter_id,
                source_channel,
                surface_type,
                // The requested model is not persisted in the message store, so an
                // idempotent resubmission of an accepted message falls back to the
                // deployment's active model rather than recovering the original hint.
                requested_model: None,
            },
        )))
    }

    async fn submit_or_replay<T, C>(
        self,
        thread_service: &T,
        turn_coordinator: &C,
        input_enqueue: &dyn HostInputEnqueuePort,
    ) -> Result<InboundTurnOutcome, ProductSurfaceFailure>
    where
        T: SessionThreadService,
        C: TurnCoordinator,
    {
        match self {
            Self::AlreadySubmitted {
                accepted_message_ref,
                submitted_run_id,
                binding,
            } => Ok(InboundTurnOutcome::Submitted {
                accepted_message_ref,
                submitted_run_id,
                binding,
            }),
            Self::AlreadyRejected {
                accepted_message_ref,
                binding,
                active_run_id,
            } => Ok(InboundTurnOutcome::RejectedBusy {
                accepted_message_ref,
                active_run_id,
                binding,
            }),
            Self::AlreadyDeferred {
                accepted_message_ref,
                binding,
                active_run_id,
                thread_scope,
                message_id,
            } => {
                let turn_scope = TurnScope::new_with_owner(
                    binding.tenant_id.clone(),
                    binding.agent_id.clone(),
                    binding.project_id.clone(),
                    binding.thread_id.clone(),
                    thread_scope.owner_user_id.clone(),
                );
                match crate::steering::readmit_queued_steering(
                    turn_coordinator,
                    input_enqueue,
                    thread_service,
                    crate::steering::SteeringAdmissionRequest {
                        turn_scope,
                        thread_scope,
                        message_id,
                        accepted_message_ref: accepted_message_ref.clone(),
                        active_run_id,
                    },
                )
                .await
                {
                    Ok(crate::steering::SteeringAdmission::Deferred { .. }) => {
                        Ok(InboundTurnOutcome::DeferredBusy {
                            accepted_message_ref,
                            active_run_id,
                            binding,
                        })
                    }
                    Ok(crate::steering::SteeringAdmission::Rejected) => {
                        Ok(InboundTurnOutcome::RejectedBusy {
                            accepted_message_ref,
                            active_run_id: Some(active_run_id),
                            binding,
                        })
                    }
                    Err(error) => Err(steering_admission_failure(error)),
                }
            }
            Self::NeedsSubmission(submission) => {
                submission
                    .submit(thread_service, turn_coordinator, input_enqueue)
                    .await
            }
        }
    }
}

struct AcceptedProductInboundTurn {
    binding: ResolvedBinding,
    thread_scope: ThreadScope,
    message_id: ThreadMessageId,
    source_binding_id: String,
    reply_target_binding_id: String,
    idempotency_key_raw: String,
    received_at: DateTime<Utc>,
    adapter_id: ProductAdapterId,
    source_channel: ProductSourceChannel,
    surface_type: TurnSurfaceType,
    requested_model: Option<String>,
}

impl AcceptedProductInboundTurn {
    async fn submit<T, C>(
        self,
        thread_service: &T,
        turn_coordinator: &C,
        input_enqueue: &dyn HostInputEnqueuePort,
    ) -> Result<InboundTurnOutcome, ProductSurfaceFailure>
    where
        T: SessionThreadService,
        C: TurnCoordinator,
    {
        let Self {
            binding,
            thread_scope,
            message_id,
            source_binding_id,
            reply_target_binding_id,
            idempotency_key_raw,
            received_at,
            adapter_id,
            source_channel,
            surface_type,
            requested_model,
        } = self;
        let turn_scope = TurnScope::new_with_owner(
            binding.tenant_id.clone(),
            binding.agent_id.clone(),
            binding.project_id.clone(),
            binding.thread_id.clone(),
            thread_scope.owner_user_id.clone(),
        );
        let actor = TurnActor::new(binding.actor_user_id.clone());
        let source_binding_ref = bounded_source_binding_ref(
            "src",
            &source_binding_id,
            DEFAULT_BINDING_REF_RAW_MAX_BYTES,
        )
        .map_err(|e| ProductSurfaceFailure::TurnSubmissionRejected {
            reason: format!("invalid src ref: {e}"),
        })?;
        let accepted_message_ref = accepted_message_ref(message_id)?;
        let reply_target_binding_ref = bounded_reply_target_binding_ref(
            "reply",
            &reply_target_binding_id,
            DEFAULT_BINDING_REF_RAW_MAX_BYTES,
        )
        .map_err(|e| ProductSurfaceFailure::TurnSubmissionRejected {
            reason: format!("invalid reply ref: {e}"),
        })?;
        let idempotency_key = bounded_idempotency_key(
            "turn",
            &idempotency_key_raw,
            DEFAULT_BINDING_REF_RAW_MAX_BYTES,
        )
        .map_err(|e| ProductSurfaceFailure::TurnSubmissionRejected {
            reason: format!("invalid turn ref: {e}"),
        })?;

        let run_adapter =
            ironclaw_turns::RunOriginAdapter::new(adapter_id.as_str()).map_err(|e| {
                ProductSurfaceFailure::TurnSubmissionRejected {
                    reason: e.to_string(),
                }
            })?;
        let run_source_channel = ironclaw_turns::RunOriginAdapter::new(source_channel.as_str())
            .map_err(|e| ProductSurfaceFailure::TurnSubmissionRejected {
                reason: e.to_string(),
            })?;
        let product_context = ironclaw_turns::product_context::resolve_inbound_with_source_channel(
            ironclaw_turns::product_context::InboundClassification::Untrusted,
            run_adapter,
            Some(run_source_channel),
            Some(surface_type),
            turn_scope.product_owner(&actor),
        );
        let request = SubmitTurnRequest {
            scope: turn_scope.clone(),
            actor,
            accepted_message_ref: accepted_message_ref.clone(),
            source_binding_ref,
            reply_target_binding_ref,
            requested_run_profile: None,
            requested_model,
            idempotency_key,
            received_at,
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: Some(product_context),
        };

        match turn_coordinator.submit_turn(request).await {
            Ok(SubmitTurnResponse::Accepted {
                turn_id, run_id, ..
            }) => {
                thread_service
                    .mark_message_submitted(
                        &thread_scope,
                        &binding.thread_id,
                        message_id,
                        turn_id.to_string(),
                        run_id.to_string(),
                    )
                    .await
                    .map_err(|e| ProductSurfaceFailure::Transient {
                        reason: format!("failed to mark message submitted: {e}"),
                    })?;
                Ok(InboundTurnOutcome::Submitted {
                    accepted_message_ref,
                    submitted_run_id: run_id,
                    binding,
                })
            }
            Err(TurnError::ThreadBusy(busy)) => {
                match crate::steering::admit_busy_steering(
                    turn_coordinator,
                    input_enqueue,
                    thread_service,
                    crate::steering::SteeringAdmissionRequest {
                        turn_scope,
                        thread_scope,
                        message_id,
                        accepted_message_ref: accepted_message_ref.clone(),
                        active_run_id: busy.active_run_id,
                    },
                )
                .await
                {
                    Ok(crate::steering::SteeringAdmission::Deferred { .. }) => {
                        Ok(InboundTurnOutcome::DeferredBusy {
                            accepted_message_ref,
                            active_run_id: busy.active_run_id,
                            binding,
                        })
                    }
                    Ok(crate::steering::SteeringAdmission::Rejected) => {
                        Ok(InboundTurnOutcome::RejectedBusy {
                            accepted_message_ref,
                            active_run_id: Some(busy.active_run_id),
                            binding,
                        })
                    }
                    Err(error) => Err(steering_admission_failure(error)),
                }
            }
            Err(error) => Err(ProductSurfaceFailure::TurnSubmissionFailed { error }),
        }
    }
}

/// Map a fatal steering-admission failure into this surface's error type.
/// The classification (what settles vs what fails) already happened in the
/// gateway; this is pure error-shape translation.
fn steering_admission_failure(
    error: crate::steering::SteeringAdmissionError,
) -> ProductSurfaceFailure {
    use crate::steering::SteeringAdmissionError;
    match error {
        SteeringAdmissionError::InvalidMessageRef(reason) => {
            ProductSurfaceFailure::TurnSubmissionRejected {
                reason: format!("invalid steering message ref: {reason}"),
            }
        }
        SteeringAdmissionError::RunState(error) => {
            ProductSurfaceFailure::TurnSubmissionFailed { error }
        }
        SteeringAdmissionError::MarkQueued(error) => ProductSurfaceFailure::Transient {
            reason: format!("failed to mark message queued: {error}"),
        },
        SteeringAdmissionError::SettleRejected(error) => ProductSurfaceFailure::Transient {
            reason: format!("failed to mark message rejected: {error}"),
        },
        SteeringAdmissionError::Enqueue(error) => ProductSurfaceFailure::Transient {
            reason: format!("failed to enqueue steering input: {error}"),
        },
    }
}

fn accepted_message_ref(
    message_id: ThreadMessageId,
) -> Result<AcceptedMessageRef, ProductSurfaceFailure> {
    AcceptedMessageRef::new(format!("msg:{message_id}")).map_err(|e| {
        ProductSurfaceFailure::TurnSubmissionRejected {
            reason: format!("invalid accepted message ref: {e}"),
        }
    })
}

#[cfg(test)]
fn binding_from_replay(
    replay: &AcceptedInboundMessageReplay,
) -> Result<ResolvedBinding, ProductSurfaceFailure> {
    let actor_user_id = match replay.actor_id.as_deref() {
        Some(actor_id) => {
            UserId::new(actor_id).map_err(|e| ProductSurfaceFailure::BindingResolutionFailed {
                reason: format!("invalid replay actor user id: {e}"),
            })?
        }
        None => replay.scope.owner_user_id.clone().ok_or_else(|| {
            ProductSurfaceFailure::BindingResolutionFailed {
                reason: "accepted replay missing actor user id and owner user id".into(),
            }
        })?,
    };
    Ok(ResolvedBinding {
        tenant_id: replay.scope.tenant_id.clone(),
        actor_user_id,
        subject_user_id: replay.scope.owner_user_id.clone(),
        thread_id: replay.thread_id.clone(),
        agent_id: Some(replay.scope.agent_id.clone()),
        project_id: replay.scope.project_id.clone(),
    })
}

fn thread_scope_from_binding(
    binding: &ResolvedBinding,
) -> Result<ThreadScope, ProductSurfaceFailure> {
    let Some(agent_id) = binding.agent_id.clone() else {
        return Err(ProductSurfaceFailure::BindingResolutionFailed {
            reason: "resolved binding missing agent_id required for thread scope".into(),
        });
    };
    Ok(ThreadScope {
        tenant_id: binding.tenant_id.clone(),
        agent_id,
        project_id: binding.project_id.clone(),
        owner_user_id: binding.subject_user_id.clone(),
        mission_id: None,
    })
}

fn product_source_binding_id(
    envelope: &ProductInboundEnvelope,
    binding: &ResolvedBinding,
) -> String {
    format!(
        "{}{}{}{}{}",
        segment("adapter", envelope.adapter_id().as_str()),
        segment("installation", envelope.installation_id().as_str()),
        segment(
            "agent",
            binding.agent_id.as_ref().map_or("", |id| id.as_str())
        ),
        segment(
            "project",
            binding.project_id.as_ref().map_or("", |id| id.as_str())
        ),
        envelope.source_binding_key()
    )
}

fn submit_idempotency_key(envelope: &ProductInboundEnvelope, binding: &ResolvedBinding) -> String {
    format!(
        "{}{}{}{}{}",
        segment("adapter", envelope.adapter_id().as_str()),
        segment("installation", envelope.installation_id().as_str()),
        segment(
            "agent",
            binding.agent_id.as_ref().map_or("", |id| id.as_str())
        ),
        segment(
            "project",
            binding.project_id.as_ref().map_or("", |id| id.as_str())
        ),
        segment("event", envelope.external_event_id().as_str())
    )
}

fn segment(name: &str, value: &str) -> String {
    format!("{name}:{}:{value};", value.len())
}

#[cfg(test)]
mod tests;
