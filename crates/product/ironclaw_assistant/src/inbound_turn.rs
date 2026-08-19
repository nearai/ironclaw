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

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS;
use ironclaw_extension_contracts::channel_adapter::ProductTriggerReason;
use ironclaw_extension_contracts::external::ProductAttachmentDescriptor;
use ironclaw_host_api::attachment::InboundAttachment;
use ironclaw_host_api::ids::ThreadId;
#[cfg(test)]
use ironclaw_host_api::ids::UserId;
use ironclaw_host_api::product_adapter::ProductAdapterId;
use ironclaw_loop_host::HostInputEnqueuePort;
#[cfg(doc)]
use ironclaw_loop_host::RejectingInputEnqueue;
use ironclaw_product_contracts::inbound::{
    AcceptedTurnSubmission, BusyRunSnapshot, ProductInboundAck, ProductInboundBindingDirective,
    ProductInboundEnvelope, ProductInboundPayload, ProductRejection, ProductSourceChannel,
};
use ironclaw_product_contracts::operator_llm::{LlmConfigService, LlmConfigServiceError};
use ironclaw_product_contracts::surface::ProductSurfaceCaller;
use ironclaw_product_contracts::surface::ProductSurfaceError;
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
use ironclaw_product_contracts::binding::{
    ProductBindingResolver, ProductConversationBindingCreationPolicy, ProductConversationRouteKind,
    ResolveBindingRequest, ResolvedBinding, binding_profile_for_trigger,
};

#[cfg(not(any(test, feature = "test-support")))]
const BEFORE_INBOUND_POLICY_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(any(test, feature = "test-support"))]
const BEFORE_INBOUND_POLICY_TIMEOUT: Duration = Duration::from_millis(10);
const ATTACHMENT_CLEANUP_TIMEOUT: Duration = Duration::from_millis(250);
const ATTACHMENT_CLEANUP_MAX_THREADS: usize = 50;
const ATTACHMENT_CLEANUP_MAX_MESSAGES: usize = 10_000;
/// Persisted session-lane binding-ref prefixes — byte-identical to what the
/// dedicated browser path always wrote. Changing either breaks replay of
/// already-accepted messages; they are defined once so the two construction
/// sites cannot drift.
const SESSION_SOURCE_BINDING_PREFIX: &str = "webui-src";
const SESSION_REPLY_BINDING_PREFIX: &str = "webui-reply";

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
        /// Submit-time coordinator metadata. `Some` on a fresh submission;
        /// `None` on an idempotent replay of an already-submitted message
        /// (replays report the run's *current* state, read separately).
        submission: Option<AcceptedTurnSubmission>,
    },
    /// Turn submission was busy (thread already has an active run). The message
    /// was recorded as RejectedBusy — it will NOT be auto-resubmitted; the user
    /// must resend once the current task finishes.
    RejectedBusy {
        accepted_message_ref: AcceptedMessageRef,
        active_run_id: Option<TurnRunId>,
        binding: ResolvedBinding,
        /// Blocking-run snapshot at decision time. `None` on replays of a
        /// stored busy outcome.
        busy: Option<BusyRunSnapshot>,
    },
    DeferredBusy {
        accepted_message_ref: AcceptedMessageRef,
        active_run_id: TurnRunId,
        binding: ResolvedBinding,
        busy: Option<BusyRunSnapshot>,
    },
}

impl InboundTurnOutcome {
    /// Convert to a product-safe acknowledgement for the adapter.
    pub fn to_ack(&self) -> ProductInboundAck {
        match self {
            Self::Submitted {
                accepted_message_ref,
                submitted_run_id,
                submission,
                ..
            } => ProductInboundAck::Accepted {
                accepted_message_ref: accepted_message_ref.clone(),
                submitted_run_id: *submitted_run_id,
                submission: submission.clone().map(Box::new),
            },
            Self::RejectedBusy {
                accepted_message_ref,
                active_run_id,
                busy,
                ..
            } => ProductInboundAck::RejectedBusy {
                accepted_message_ref: accepted_message_ref.clone(),
                active_run_id: *active_run_id,
                busy: busy.clone().map(Box::new),
            },
            Self::DeferredBusy {
                accepted_message_ref,
                active_run_id,
                busy,
                ..
            } => ProductInboundAck::DeferredBusy {
                accepted_message_ref: accepted_message_ref.clone(),
                active_run_id: *active_run_id,
                busy: busy.clone().map(Box::new),
            },
        }
    }
}

/// Result of running replay, before-inbound policy, and fresh user-message acceptance.
/// The accepted arm is boxed: `InboundTurnOutcome` carries the full ack +
/// resolution payload (~280 bytes) while a rejection is a slim reason.
pub enum InboundUserMessageDispatch {
    Accepted(Box<InboundTurnOutcome>),
    Rejected(ProductRejection),
}

/// The two submission lanes of the one inbound core, selected by the
/// envelope's binding directive. Everything below `submit_turn` is shared;
/// the lane decides binding resolution, the persisted binding-id schemes,
/// the turn-ref prefixes, the idempotency-key shape, and the resolved
/// product context — enum arms, never a second pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubmissionLane {
    /// Webhook-verified channel inbound: external refs resolved through the
    /// binding resolver, untrusted-inbound product context.
    Webhook,
    /// Authenticated-session inbound: the caller owns the thread; trusted
    /// first-party product context.
    Session,
}

/// Session skill-activation recorder: mirrors the product-layer closure shape
/// so the same composition-wired hooks serve both. Failures surface as
/// internal errors — the message was accepted but the turn must not submit
/// with stale skill bookkeeping.
pub type SessionSkillActivationRecorder =
    dyn Fn(&TurnScope, &AcceptedMessageRef, &str) -> Result<(), ProductSurfaceError> + Send + Sync;
pub type SessionSkillActivationClearer =
    dyn Fn(&TurnScope, &AcceptedMessageRef) -> Result<(), ProductSurfaceError> + Send + Sync;

/// The pair of session skill-activation hooks, wired together or not at all.
#[derive(Clone)]
pub struct SessionSkillActivationPorts {
    pub recorder: Arc<SessionSkillActivationRecorder>,
    pub clearer: Arc<SessionSkillActivationClearer>,
}

struct PreparedUserMessage {
    binding: ResolvedBinding,
    thread_scope: ThreadScope,
    source_binding_id: String,
    reply_target_binding_id: String,
    submit_idempotency_key: String,
    adapter_id: ProductAdapterId,
    source_channel: ProductSourceChannel,
    surface_type: TurnSurfaceType,
    lane: SubmissionLane,
    /// The user-message text, carried so the session lane can record skill
    /// activation between acceptance and submission. `None` on the webhook
    /// lane, which has no skill-activation hook today.
    skill_activation_text: Option<String>,
}

struct ReplaySubmissionContext {
    binding: ResolvedBinding,
    thread_scope: ThreadScope,
    adapter_id: ProductAdapterId,
    source_channel: ProductSourceChannel,
    surface_type: TurnSurfaceType,
    lane: SubmissionLane,
    skill_activation_text: Option<String>,
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
}

/// Default implementation that composes a [`ProductBindingResolver`] with a
/// [`SessionThreadService`] and [`TurnCoordinator`].
pub struct DefaultInboundTurnService<B, T, C> {
    binding_service: B,
    thread_service: T,
    turn_coordinator: C,
    inbound_attachments: Option<Arc<dyn InboundAttachmentLander>>,
    llm_config: Option<Arc<dyn LlmConfigService>>,
    input_enqueue: Arc<dyn HostInputEnqueuePort>,
    session_skill_activation: Option<SessionSkillActivationPorts>,
}

impl<B, T, C> DefaultInboundTurnService<B, T, C>
where
    B: ProductBindingResolver,
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
            llm_config: None,
            input_enqueue,
            session_skill_activation: None,
        }
    }

    /// Resolve explicit model hints and caller-scoped saved preferences before
    /// a message crosses the durable acceptance boundary.
    pub fn with_llm_config_service(mut self, llm_config: Arc<dyn LlmConfigService>) -> Self {
        self.llm_config = Some(llm_config);
        self
    }

    async fn resolve_user_model(
        &self,
        binding: &ResolvedBinding,
        requested_model: Option<String>,
    ) -> Result<Option<String>, ProductSurfaceFailure> {
        let Some(llm_config) = self.llm_config.as_ref() else {
            return Ok(requested_model);
        };
        llm_config
            .resolve_user_model(
                ProductSurfaceCaller::new(
                    binding.tenant_id.clone(),
                    binding.actor_user_id.clone(),
                    binding.agent_id.clone(),
                    binding.project_id.clone(),
                ),
                requested_model,
            )
            .await
            .map_err(inbound_model_resolution_failure)
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

    /// Wire the session skill-activation hooks recorded between message
    /// acceptance and turn submission (and cleared on busy/error). Session
    /// lane only; the webhook lane has no skill-activation hook today.
    pub fn with_session_skill_activation(
        mut self,
        session_skill_activation: SessionSkillActivationPorts,
    ) -> Self {
        self.session_skill_activation = Some(session_skill_activation);
        self
    }

    // Deliberate blind spot: `list_threads_for_scope` excludes
    // prepared-context (unbound/subagent) threads, so their attachments —
    // none exist today, the accept door seeds text and tool history only —
    // would not be visible to this sweep. If prepared submissions ever gain
    // attachment landing, reconcile them against the unbound scope
    // explicitly rather than widening the listing.
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
    B: ProductBindingResolver,
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
            InboundUserMessageDispatch::Accepted(outcome) => Ok(*outcome),
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
        self.accept_with_before_policy_inner(envelope, before_inbound_policy, Vec::new())
            .await
    }

    async fn accept_user_message_with_before_policy_and_attachments(
        &self,
        envelope: &ProductInboundEnvelope,
        before_inbound_policy: &dyn BeforeInboundPolicy,
        attachments: Vec<InboundAttachment>,
    ) -> Result<InboundUserMessageDispatch, ProductSurfaceFailure> {
        self.accept_with_before_policy_inner(envelope, before_inbound_policy, attachments)
            .await
    }
}

impl<B, T, C> DefaultInboundTurnService<B, T, C>
where
    B: ProductBindingResolver,
    T: SessionThreadService,
    C: TurnCoordinator,
{
    async fn accept_with_before_policy_inner(
        &self,
        envelope: &ProductInboundEnvelope,
        before_inbound_policy: &dyn BeforeInboundPolicy,
        attachments: Vec<InboundAttachment>,
    ) -> Result<InboundUserMessageDispatch, ProductSurfaceFailure> {
        let ProductInboundPayload::UserMessage(payload) = envelope.payload() else {
            return Err(ProductSurfaceFailure::UnsupportedActionKind {
                kind: "non_user_message".into(),
            });
        };
        let original_trigger = payload.trigger;
        let original_descriptors = payload.attachments.clone();
        let prepared = self.prepare_user_message(envelope).await?;
        if let Some(outcome) = self
            .replay_prepared_user_message(envelope, &prepared)
            .await?
        {
            return Ok(InboundUserMessageDispatch::Accepted(Box::new(outcome)));
        }

        // The adapter has already completed vendor transfer. Validate the
        // descriptor/byte pairing and canonicalize media types before either
        // the policy backend or durable message acceptance sees it.
        let attachments = validate_and_normalize_inbound_attachments(
            original_descriptors.as_slice(),
            attachments,
        )?;

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

        let ProductInboundPayload::UserMessage(rewritten_payload) = envelope_for_turn.payload()
        else {
            return Err(ProductSurfaceFailure::UnsupportedActionKind {
                kind: "non_user_message".into(),
            });
        };
        let attachments = reconcile_inbound_attachments_after_policy(
            original_descriptors.as_slice(),
            rewritten_payload.attachments.as_slice(),
            attachments,
        )?;

        let requested_model = self
            .resolve_user_model(
                &prepared_for_turn.binding,
                rewritten_payload.requested_model.clone(),
            )
            .await?;

        self.accept_prepared_user_message(
            prepared_for_turn,
            envelope_for_turn,
            attachments,
            requested_model,
        )
        .await
        .map(|outcome| InboundUserMessageDispatch::Accepted(Box::new(outcome)))
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
        match envelope.binding_directive() {
            ProductInboundBindingDirective::ExternalRef => {
                self.prepare_external_ref_message(envelope, payload.trigger)
                    .await
            }
            ProductInboundBindingDirective::OwnedThread { thread_id } => {
                self.prepare_owned_thread_message(envelope, thread_id, &payload.text)
                    .await
            }
        }
    }

    async fn prepare_external_ref_message(
        &self,
        envelope: &ProductInboundEnvelope,
        trigger: ProductTriggerReason,
    ) -> Result<PreparedUserMessage, ProductSurfaceFailure> {
        let (route_kind, creation_policy) = binding_profile_for_trigger(trigger);
        let surface_type = match route_kind {
            ProductConversationRouteKind::Direct => TurnSurfaceType::Direct,
            ProductConversationRouteKind::Shared => TurnSurfaceType::Channel,
        };
        let auth_claim = envelope
            .require_verified_auth_claim()
            .map_err(|error| ProductSurfaceFailure::BindingResolutionFailed {
                reason: error.to_string(),
            })?
            .clone();
        let binding_request = ResolveBindingRequest {
            adapter_id: envelope.adapter_id().clone(),
            installation_id: envelope.installation_id().clone(),
            external_actor_ref: envelope.external_actor_ref().clone(),
            external_conversation_ref: envelope.external_conversation_ref().clone(),
            external_event_id: envelope.external_event_id().clone(),
            route_kind,
            auth_claim,
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
        // The conversation resolution mints a per-event source/reply binding
        // pair anchored to this event's own (per-ping ephemeral, for shared
        // routes) thread. Carry both refs verbatim — do NOT re-derive a
        // per-conversation id — so the accepted message and the submitted run
        // stay anchored to this event's thread, and a second event in the same
        // external conversation is not pinned to the first event's thread.
        let source_binding_id = binding.source_binding_ref.as_str().to_string();
        let reply_target_binding_id = binding.reply_target_binding_ref.as_str().to_string();
        let submit_idempotency_key = submit_idempotency_key(envelope, &binding);
        let thread_scope = thread_scope_from_binding(&binding)?;
        Ok(PreparedUserMessage {
            binding,
            thread_scope,
            source_binding_id,
            reply_target_binding_id,
            submit_idempotency_key,
            adapter_id: envelope.adapter_id().clone(),
            source_channel: envelope.source_channel().clone(),
            surface_type,
            lane: SubmissionLane::Webhook,
            skill_activation_text: None,
        })
    }

    /// The session lane's binding step: the authenticated caller *is* the
    /// binding authority. The thread must already exist and be owned by the
    /// caller — never created implicitly — and the ownership probe collapses
    /// "missing" and "someone else's" into one indistinguishable failure so
    /// the response is not an existence oracle. The external binding
    /// resolver and the webhook pairing machinery never run on this lane.
    async fn prepare_owned_thread_message(
        &self,
        envelope: &ProductInboundEnvelope,
        thread_id: &ThreadId,
        message_text: &str,
    ) -> Result<PreparedUserMessage, ProductSurfaceFailure> {
        let Some(caller) = envelope.session_caller() else {
            return Err(ProductSurfaceFailure::BindingResolutionFailed {
                reason: "owned-thread binding requires a session caller".into(),
            });
        };
        let scope = caller.turn_scope(thread_id.clone());
        let Some(agent_id) = scope.agent_id.clone() else {
            return Err(ProductSurfaceFailure::BindingResolutionFailed {
                reason: "session caller is missing an agent scope".into(),
            });
        };
        let thread_scope = ThreadScope {
            tenant_id: scope.tenant_id.clone(),
            agent_id,
            project_id: scope.project_id.clone(),
            owner_user_id: Some(caller.user_id.clone()),
            mission_id: None,
        };
        self.thread_service
            .read_thread(ThreadHistoryRequest {
                scope: thread_scope.clone(),
                thread_id: thread_id.clone(),
            })
            .await
            .map_err(owned_thread_probe_failure)?;
        let actor = TurnActor::new(caller.user_id.clone());
        // One caller-scoped id backs BOTH binding halves on the session lane
        // (the browser transport's historical scheme): the source and reply
        // refs are the same raw id under the "webui-src"/"webui-reply"
        // prefixes, kept byte-identical so persisted records and replays
        // written by the dedicated browser path keep matching.
        let session_binding_id = session_source_binding_id(&scope, &actor);
        let source_binding_ref = bounded_source_binding_ref(
            SESSION_SOURCE_BINDING_PREFIX,
            &session_binding_id,
            DEFAULT_BINDING_REF_RAW_MAX_BYTES,
        )
        .map_err(|e| ProductSurfaceFailure::BindingResolutionFailed {
            reason: format!("invalid session src ref: {e}"),
        })?;
        let reply_target_binding_ref = bounded_reply_target_binding_ref(
            SESSION_REPLY_BINDING_PREFIX,
            &session_binding_id,
            DEFAULT_BINDING_REF_RAW_MAX_BYTES,
        )
        .map_err(|e| ProductSurfaceFailure::BindingResolutionFailed {
            reason: format!("invalid session reply ref: {e}"),
        })?;
        let binding = ResolvedBinding {
            tenant_id: scope.tenant_id.clone(),
            actor_user_id: caller.user_id.clone(),
            thread_id: thread_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            source_binding_ref,
            reply_target_binding_ref,
        };
        Ok(PreparedUserMessage {
            source_binding_id: session_binding_id.clone(),
            reply_target_binding_id: session_binding_id,
            // The session submit idempotency key is the caller's client
            // action id verbatim — the same value the transport has always
            // handed the coordinator.
            submit_idempotency_key: envelope.external_event_id().as_str().to_string(),
            binding,
            thread_scope,
            adapter_id: envelope.adapter_id().clone(),
            source_channel: envelope.source_channel().clone(),
            surface_type: TurnSurfaceType::Direct,
            lane: SubmissionLane::Session,
            skill_activation_text: Some(message_text.to_string()),
        })
    }

    async fn replay_prepared_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
        prepared: &PreparedUserMessage,
    ) -> Result<Option<InboundTurnOutcome>, ProductSurfaceFailure> {
        let Some(replay) = self.lookup_accepted_replay(envelope, prepared).await? else {
            return Ok(None);
        };

        // A session client action id is caller-scoped, not thread-scoped: the
        // same id replayed against a different thread is a duplicate action,
        // not a fresh submission for the new thread.
        if prepared.lane == SubmissionLane::Session
            && replay.thread_id != prepared.binding.thread_id
        {
            return Err(ProductSurfaceFailure::ClientActionReplayMismatch);
        }

        submit_or_replay_accepted_message(
            &self.thread_service,
            &self.turn_coordinator,
            self.input_enqueue.as_ref(),
            self.session_skill_activation.as_ref(),
            replay,
            prepared.submit_idempotency_key.clone(),
            envelope.received_at(),
            prepared,
        )
        .await
        .map(Some)
    }

    /// Replay lookup for one prepared message. The session lane additionally
    /// probes the legacy persisted binding-id schemes so messages accepted by
    /// earlier builds still replay instead of double-accepting.
    async fn lookup_accepted_replay(
        &self,
        envelope: &ProductInboundEnvelope,
        prepared: &PreparedUserMessage,
    ) -> Result<Option<AcceptedInboundMessageReplay>, ProductSurfaceFailure> {
        let mut candidate_binding_ids = vec![prepared.source_binding_id.clone()];
        if prepared.lane == SubmissionLane::Session {
            let scope = TurnScope::new_with_owner(
                prepared.binding.tenant_id.clone(),
                prepared.binding.agent_id.clone(),
                prepared.binding.project_id.clone(),
                prepared.binding.thread_id.clone(),
                prepared.thread_scope.owner_user_id.clone(),
            );
            let actor = TurnActor::new(prepared.binding.actor_user_id.clone());
            let legacy = legacy_session_source_binding_id(&scope, &actor);
            if !candidate_binding_ids.contains(&legacy) {
                candidate_binding_ids.push(legacy);
            }
        }
        for source_binding_id in candidate_binding_ids {
            let replay = self
                .thread_service
                .replay_accepted_inbound_message(ReplayAcceptedInboundMessageRequest {
                    scope: prepared.thread_scope.clone(),
                    actor_id: prepared.binding.actor_user_id.as_str().to_string(),
                    source_binding_id,
                    external_event_id: envelope.external_event_id().as_str().to_string(),
                })
                .await
                .map_err(|e| ProductSurfaceFailure::Transient {
                    reason: format!("failed to replay accepted inbound message: {e}"),
                })?;
            if replay.is_some() {
                return Ok(replay);
            }
        }
        Ok(None)
    }

    async fn accept_prepared_user_message(
        &self,
        prepared: PreparedUserMessage,
        envelope: &ProductInboundEnvelope,
        attachments: Vec<InboundAttachment>,
        requested_model: Option<String>,
    ) -> Result<InboundTurnOutcome, ProductSurfaceFailure> {
        let ProductInboundPayload::UserMessage(payload) = envelope.payload() else {
            return Err(ProductSurfaceFailure::UnsupportedActionKind {
                kind: "non_user_message".into(),
            });
        };
        // The session lane never creates threads: the caller-owned thread was
        // ownership-probed during prepare, and send-message must not
        // implicitly mint one. Only the webhook lane, whose binding resolver
        // decided a thread identity, ensures it exists.
        if prepared.lane == SubmissionLane::Webhook {
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
        }

        // Inbound attachment bytes (inline or fetched after channel policy)
        // are landed into project storage through the same authority
        // the agent's file tools resolve through, then carried on the message as
        // refs — never as raw bytes through the bytes-free product envelope.
        let (content, landed_refs) = if attachments.is_empty() {
            (MessageContent::text(payload.text.clone()), None)
        } else {
            let lander = self
                .inbound_attachments
                .as_ref()
                .ok_or(ProductSurfaceFailure::AttachmentLanderUnavailable)?;
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

        let reply_target_binding_id = prepared.reply_target_binding_id.clone();
        let accepted = match self
            .thread_service
            .accept_inbound_message_with_replay_metadata(
                AcceptInboundMessageRequest {
                    scope: prepared.thread_scope.clone(),
                    thread_id: prepared.binding.thread_id.clone(),
                    actor_id: prepared.binding.actor_user_id.as_str().to_string(),
                    source_binding_id: Some(prepared.source_binding_id.clone()),
                    reply_target_binding_id: Some(reply_target_binding_id.clone()),
                    external_event_id: Some(envelope.external_event_id().as_str().to_string()),
                    content,
                },
                ironclaw_threads::InboundMessageReplayMetadata {
                    resolved_model: requested_model,
                },
            )
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
                idempotency_key_raw: prepared.submit_idempotency_key,
                received_at: envelope.received_at(),
                adapter_id: prepared.adapter_id,
                source_channel: prepared.source_channel,
                surface_type: prepared.surface_type,
                requested_model: accepted.replay_metadata.resolved_model,
                lane: prepared.lane,
                skill_activation_text: prepared.skill_activation_text,
                channel_context: payload.channel_context.clone(),
            }))
            .submit_or_replay(
                &self.thread_service,
                &self.turn_coordinator,
                self.input_enqueue.as_ref(),
                self.session_skill_activation.as_ref(),
            )
            .await?;
        if cleanup_needed {
            self.reconcile_stale_attachment_batches(&cleanup_scope)
                .await;
        }
        Ok(outcome)
    }
}

fn validate_and_normalize_inbound_attachments(
    descriptors: &[ProductAttachmentDescriptor],
    attachments: Vec<InboundAttachment>,
) -> Result<Vec<InboundAttachment>, ProductSurfaceFailure> {
    if descriptors.len() != attachments.len() {
        return Err(permanent_attachment_failure(
            "attachment bytes do not match message descriptors",
        ));
    }
    if attachments.len() > DEFAULT_ATTACHMENT_BUDGETS.max_count {
        return Err(permanent_attachment_failure(
            "attachments exceed the count limit",
        ));
    }
    let mut external_file_ids = BTreeSet::new();
    let mut total_bytes = 0usize;
    let mut normalized = Vec::with_capacity(attachments.len());
    for (descriptor, mut attachment) in descriptors.iter().zip(attachments) {
        if !external_file_ids.insert(descriptor.external_file_id.clone()) {
            return Err(permanent_attachment_failure(
                "attachment descriptors contain duplicate external file ids",
            ));
        }
        if attachment.id != descriptor.external_file_id {
            return Err(permanent_attachment_failure(
                "fetched attachment id does not match its descriptor",
            ));
        }
        let mime_type = ironclaw_common::normalize_mime_type(&attachment.mime_type);
        let declared_mime_type = ironclaw_common::normalize_mime_type(&descriptor.mime_type);
        if mime_type != declared_mime_type || !ironclaw_common::is_supported_mime(&mime_type) {
            return Err(permanent_attachment_failure(
                "fetched attachment MIME type does not match its descriptor",
            ));
        }
        attachment.mime_type = mime_type;
        if let Some(descriptor_filename) = descriptor.filename.clone() {
            attachment.filename = Some(descriptor_filename);
        }
        if attachment.bytes.len() > DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes {
            return Err(permanent_attachment_failure(
                "attachment exceeds the per-file byte limit",
            ));
        }
        if let Some(declared_size) = descriptor.size_bytes
            && declared_size != attachment.bytes.len() as u64
        {
            return Err(permanent_attachment_failure(
                "fetched attachment size does not match its descriptor",
            ));
        }
        total_bytes = total_bytes.saturating_add(attachment.bytes.len());
        if total_bytes > DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes {
            return Err(permanent_attachment_failure(
                "attachments exceed the total byte limit",
            ));
        }
        normalized.push(attachment);
    }
    Ok(normalized)
}

fn reconcile_inbound_attachments_after_policy(
    original_descriptors: &[ProductAttachmentDescriptor],
    rewritten_descriptors: &[ProductAttachmentDescriptor],
    attachments: Vec<InboundAttachment>,
) -> Result<Vec<InboundAttachment>, ProductSurfaceFailure> {
    if original_descriptors.len() != attachments.len() {
        return Err(permanent_attachment_failure(
            "attachment bytes do not match original message descriptors",
        ));
    }
    let mut used = BTreeSet::new();
    let mut reconciled = Vec::with_capacity(rewritten_descriptors.len());
    for rewritten in rewritten_descriptors {
        let Some((index, _)) = original_descriptors
            .iter()
            .enumerate()
            .find(|(index, original)| !used.contains(index) && *original == rewritten)
        else {
            return Err(permanent_attachment_failure(
                "policy rewrite changed or invented an attachment descriptor",
            ));
        };
        used.insert(index);
        let attachment = attachments.get(index).cloned().ok_or_else(|| {
            permanent_attachment_failure(
                "attachment bytes do not match original message descriptors",
            )
        })?;
        reconciled.push(attachment);
    }
    Ok(reconciled)
}

fn permanent_attachment_failure(reason: impl Into<String>) -> ProductSurfaceFailure {
    ProductSurfaceFailure::InboundAttachmentFailed {
        reason: reason.into(),
        retryable: false,
    }
}

fn inbound_model_resolution_failure(error: LlmConfigServiceError) -> ProductSurfaceFailure {
    match error {
        LlmConfigServiceError::InvalidRequest { reason, .. } => {
            ProductSurfaceFailure::InboundModelResolutionFailed {
                reason,
                retryable: false,
            }
        }
        LlmConfigServiceError::NotFound => ProductSurfaceFailure::InboundModelResolutionFailed {
            reason: "requested model was not found".into(),
            retryable: false,
        },
        LlmConfigServiceError::Unavailable => ProductSurfaceFailure::InboundModelResolutionFailed {
            reason: "model selection is temporarily unavailable".into(),
            retryable: true,
        },
        LlmConfigServiceError::Internal => ProductSurfaceFailure::InboundModelResolutionFailed {
            reason: "model selection failed".into(),
            retryable: true,
        },
    }
}

// arch-exempt: too_many_args, replay tail wants a SubmissionPorts bundle once the session lane settles, plan docs/internal/design/2026-08-10-unified-channel-model.md
#[allow(clippy::too_many_arguments)]
async fn submit_or_replay_accepted_message<T, C>(
    thread_service: &T,
    turn_coordinator: &C,
    input_enqueue: &dyn HostInputEnqueuePort,
    session_skill_activation: Option<&SessionSkillActivationPorts>,
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
    .submit_or_replay(
        thread_service,
        turn_coordinator,
        input_enqueue,
        session_skill_activation,
    )
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
                lane: SubmissionLane::Webhook,
                skill_activation_text: None,
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
                lane: prepared.lane,
                skill_activation_text: prepared.skill_activation_text.clone(),
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
            lane,
            skill_activation_text,
        } = context;
        let accepted_message_ref = accepted_message_ref(replay.message_id)?;

        if replay.status == MessageStatus::Submitted {
            let Some(turn_run_id) = replay.turn_run_id.as_deref() else {
                return Err(match lane {
                    SubmissionLane::Session => ProductSurfaceFailure::ReplayUnavailable {
                        reason: "submitted replay missing turn_run_id".into(),
                    },
                    SubmissionLane::Webhook => ProductSurfaceFailure::TurnSubmissionRejected {
                        reason: "submitted replay missing turn_run_id".into(),
                    },
                });
            };
            let submitted_run_id = Uuid::parse_str(turn_run_id)
                .map(TurnRunId::from_uuid)
                .map_err(|e| match lane {
                    SubmissionLane::Session => ProductSurfaceFailure::ReplayUnavailable {
                        reason: format!("invalid submitted turn_run_id: {e}"),
                    },
                    SubmissionLane::Webhook => ProductSurfaceFailure::TurnSubmissionRejected {
                        reason: format!("invalid submitted turn_run_id: {e}"),
                    },
                })?;
            return Ok(Self::AlreadySubmitted {
                accepted_message_ref,
                submitted_run_id,
                binding,
            });
        }

        if replay.status == MessageStatus::RejectedBusy {
            let active_run_id = match lane {
                // The session surface has always reported a replayed busy
                // rejection with no run metadata: the original blocking run
                // may be long gone, and handing the client a reference it
                // cannot query invites dead lookups.
                SubmissionLane::Session => None,
                SubmissionLane::Webhook => replay
                    .turn_run_id
                    .as_deref()
                    .map(|s| {
                        Uuid::parse_str(s).map(TurnRunId::from_uuid).map_err(|e| {
                            ProductSurfaceFailure::TurnSubmissionRejected {
                                reason: format!("invalid rejected busy turn_run_id: {e}"),
                            }
                        })
                    })
                    .transpose()?,
            };
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
        Ok(Self::NeedsSubmission(Box::new(
            AcceptedProductInboundTurn {
                binding,
                thread_scope,
                message_id: replay.message_id,
                source_binding_id,
                idempotency_key_raw: submit_idempotency_key,
                received_at,
                adapter_id,
                source_channel,
                surface_type,
                requested_model: replay.replay_metadata.resolved_model,
                lane,
                skill_activation_text,
                // Channel conversation context is likewise not persisted in the
                // message store; an idempotent resubmission degrades to no
                // context (it is advisory).
                channel_context: None,
            },
        )))
    }

    async fn submit_or_replay<T, C>(
        self,
        thread_service: &T,
        turn_coordinator: &C,
        input_enqueue: &dyn HostInputEnqueuePort,
        session_skill_activation: Option<&SessionSkillActivationPorts>,
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
                submission: None,
            }),
            Self::AlreadyRejected {
                accepted_message_ref,
                binding,
                active_run_id,
            } => Ok(InboundTurnOutcome::RejectedBusy {
                accepted_message_ref,
                active_run_id,
                binding,
                busy: None,
            }),
            Self::AlreadyDeferred {
                accepted_message_ref,
                binding,
                active_run_id,
                thread_scope,
                message_id,
            } => {
                // Same rule as the submit path: this message's run is scoped to
                // the pinger who sent it (owner == actor). The shared transcript
                // still lives under `thread_scope`.
                let turn_scope = TurnScope::new_with_owner(
                    binding.tenant_id.clone(),
                    binding.agent_id.clone(),
                    binding.project_id.clone(),
                    binding.thread_id.clone(),
                    Some(binding.actor_user_id.clone()),
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
                    Ok(crate::steering::SteeringAdmission::Deferred { run }) => {
                        Ok(InboundTurnOutcome::DeferredBusy {
                            accepted_message_ref,
                            active_run_id,
                            binding,
                            busy: Some(BusyRunSnapshot {
                                status: run.status,
                                event_cursor: run.event_cursor,
                            }),
                        })
                    }
                    Ok(crate::steering::SteeringAdmission::Rejected) => {
                        Ok(InboundTurnOutcome::RejectedBusy {
                            accepted_message_ref,
                            active_run_id: Some(active_run_id),
                            binding,
                            busy: None,
                        })
                    }
                    Err(error) => Err(steering_admission_failure(error)),
                }
            }
            Self::NeedsSubmission(submission) => {
                submission
                    .submit(
                        thread_service,
                        turn_coordinator,
                        input_enqueue,
                        session_skill_activation,
                    )
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
    idempotency_key_raw: String,
    received_at: DateTime<Utc>,
    adapter_id: ProductAdapterId,
    source_channel: ProductSourceChannel,
    surface_type: TurnSurfaceType,
    requested_model: Option<String>,
    lane: SubmissionLane,
    skill_activation_text: Option<String>,
    channel_context: Option<String>,
}

impl AcceptedProductInboundTurn {
    async fn submit<T, C>(
        self,
        thread_service: &T,
        turn_coordinator: &C,
        input_enqueue: &dyn HostInputEnqueuePort,
        session_skill_activation: Option<&SessionSkillActivationPorts>,
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
            idempotency_key_raw,
            received_at,
            adapter_id,
            source_channel,
            surface_type,
            requested_model,
            lane,
            skill_activation_text,
            channel_context,
        } = self;
        // The run is scoped to the person who pinged (its actor); owner ==
        // actor. Each run's gates, approvals, auth, settings, and mounts are
        // that user's own, while the shared channel transcript lives under
        // `thread_scope` (below, in `mark_message_submitted`) so the
        // conversation stays shared.
        let turn_scope = TurnScope::new_with_owner(
            binding.tenant_id.clone(),
            binding.agent_id.clone(),
            binding.project_id.clone(),
            binding.thread_id.clone(),
            Some(binding.actor_user_id.clone()),
        );
        let actor = TurnActor::new(binding.actor_user_id.clone());
        let accepted_message_ref = accepted_message_ref(message_id)?;
        let idempotency_key = match lane {
            SubmissionLane::Webhook => bounded_idempotency_key(
                "turn",
                &idempotency_key_raw,
                DEFAULT_BINDING_REF_RAW_MAX_BYTES,
            )
            .map_err(|e| ProductSurfaceFailure::TurnSubmissionRejected {
                reason: format!("invalid turn ref: {e}"),
            })?,
            SubmissionLane::Session => {
                ironclaw_turns::IdempotencyKey::new(idempotency_key_raw.clone()).map_err(|e| {
                    ProductSurfaceFailure::TurnSubmissionRejected {
                        reason: format!("invalid client action id: {e}"),
                    }
                })?
            }
        };

        let product_context = match lane {
            SubmissionLane::Webhook => {
                let run_adapter = ironclaw_turns::RunOriginAdapter::new(adapter_id.as_str())
                    .map_err(|e| ProductSurfaceFailure::TurnSubmissionRejected {
                        reason: e.to_string(),
                    })?;
                let run_source_channel = ironclaw_turns::RunOriginAdapter::new(
                    source_channel.as_str(),
                )
                .map_err(|e| ProductSurfaceFailure::TurnSubmissionRejected {
                    reason: e.to_string(),
                })?;
                ironclaw_turns::product_context::resolve_inbound_with_source_channel(
                    ironclaw_turns::product_context::InboundClassification::Untrusted,
                    run_adapter,
                    Some(run_source_channel),
                    Some(surface_type),
                    turn_scope.product_owner(&actor),
                )
                .with_channel_context(channel_context)
            }
            // A session submission is the trusted first-party chat surface;
            // its product context is the WebUi origin, exactly as the
            // dedicated browser path always resolved it.
            SubmissionLane::Session => {
                ironclaw_turns::product_context::resolve_web_ui(turn_scope.product_owner(&actor))
            }
        };
        let request = SubmitTurnRequest {
            scope: turn_scope.clone(),
            actor,
            accepted_message_ref: accepted_message_ref.clone(),
            requested_run_profile: None,
            output_contract: None,
            requested_model,
            idempotency_key,
            received_at,
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: Some(product_context),
        };

        record_session_skill_activation(
            lane,
            session_skill_activation,
            skill_activation_text.as_deref(),
            &turn_scope,
            &accepted_message_ref,
        )?;

        match turn_coordinator.submit_turn(request).await {
            Ok(SubmitTurnResponse::Accepted {
                turn_id,
                run_id,
                status,
                resolved_run_profile_id,
                resolved_run_profile_version,
                event_cursor,
                ..
            }) => {
                mark_message_submitted_or_reconcile(
                    thread_service,
                    &thread_scope,
                    &binding,
                    message_id,
                    lane,
                    &source_binding_id,
                    &idempotency_key_raw,
                    turn_id.to_string(),
                    run_id.to_string(),
                )
                .await?;
                Ok(InboundTurnOutcome::Submitted {
                    accepted_message_ref,
                    submitted_run_id: run_id,
                    binding,
                    submission: Some(AcceptedTurnSubmission {
                        turn_id: turn_id.to_string(),
                        status,
                        resolved_run_profile_id: resolved_run_profile_id.as_str().to_string(),
                        resolved_run_profile_version: resolved_run_profile_version.as_u64(),
                        event_cursor,
                    }),
                })
            }
            Err(TurnError::ThreadBusy(busy)) => {
                clear_session_skill_activation(
                    lane,
                    session_skill_activation,
                    &turn_scope,
                    &accepted_message_ref,
                )?;
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
                    Ok(crate::steering::SteeringAdmission::Deferred { run }) => {
                        Ok(InboundTurnOutcome::DeferredBusy {
                            accepted_message_ref,
                            active_run_id: busy.active_run_id,
                            binding,
                            busy: Some(BusyRunSnapshot {
                                status: run.status,
                                event_cursor: run.event_cursor,
                            }),
                        })
                    }
                    Ok(crate::steering::SteeringAdmission::Rejected) => {
                        Ok(InboundTurnOutcome::RejectedBusy {
                            accepted_message_ref,
                            active_run_id: Some(busy.active_run_id),
                            binding,
                            busy: Some(BusyRunSnapshot {
                                status: busy.status,
                                event_cursor: busy.event_cursor,
                            }),
                        })
                    }
                    Err(error) => Err(steering_admission_failure(error)),
                }
            }
            Err(error) => {
                clear_session_skill_activation(
                    lane,
                    session_skill_activation,
                    &turn_scope,
                    &accepted_message_ref,
                )?;
                Err(ProductSurfaceFailure::TurnSubmissionFailed { error })
            }
        }
    }
}

/// Record the session skill-activation bookkeeping between message acceptance
/// and turn submission. Webhook-lane submissions skip it entirely.
fn record_session_skill_activation(
    lane: SubmissionLane,
    ports: Option<&SessionSkillActivationPorts>,
    text: Option<&str>,
    scope: &TurnScope,
    accepted_message_ref: &AcceptedMessageRef,
) -> Result<(), ProductSurfaceFailure> {
    if lane != SubmissionLane::Session {
        return Ok(());
    }
    let (Some(ports), Some(text)) = (ports, text) else {
        return Ok(());
    };
    (ports.recorder)(scope, accepted_message_ref, text).map_err(|error| {
        ProductSurfaceFailure::SkillActivationFailed {
            reason: format!("skill activation recorder failed: {:?}", error.code),
        }
    })
}

/// Clear session skill-activation bookkeeping when the accepted message did
/// not become the submitted turn (busy or submission failure).
fn clear_session_skill_activation(
    lane: SubmissionLane,
    ports: Option<&SessionSkillActivationPorts>,
    scope: &TurnScope,
    accepted_message_ref: &AcceptedMessageRef,
) -> Result<(), ProductSurfaceFailure> {
    if lane != SubmissionLane::Session {
        return Ok(());
    }
    let Some(ports) = ports else {
        return Ok(());
    };
    (ports.clearer)(scope, accepted_message_ref).map_err(|error| {
        ProductSurfaceFailure::SkillActivationFailed {
            reason: format!("skill activation clearer failed: {:?}", error.code),
        }
    })
}

/// Mark the accepted message submitted, reconciling a session-lane duplicate:
/// when a concurrent retry already marked the same message with the same run,
/// the mark failure is benign and the submission stands.
// arch-exempt: too_many_args, wants a SubmittedMarkContext bundle once the session lane settles, plan docs/internal/design/2026-08-10-unified-channel-model.md
#[allow(clippy::too_many_arguments)]
async fn mark_message_submitted_or_reconcile<T>(
    thread_service: &T,
    thread_scope: &ThreadScope,
    binding: &ResolvedBinding,
    message_id: ThreadMessageId,
    lane: SubmissionLane,
    source_binding_id: &str,
    external_event_id: &str,
    turn_id: String,
    run_id: String,
) -> Result<(), ProductSurfaceFailure>
where
    T: SessionThreadService,
{
    let mark_error = match thread_service
        .mark_message_submitted(
            thread_scope,
            &binding.thread_id,
            message_id,
            turn_id,
            run_id.clone(),
        )
        .await
    {
        Ok(_) => return Ok(()),
        Err(error) => error,
    };
    if lane != SubmissionLane::Session {
        return Err(ProductSurfaceFailure::Transient {
            reason: format!("failed to mark message submitted: {mark_error}"),
        });
    }
    let replay = thread_service
        .replay_accepted_inbound_message(ReplayAcceptedInboundMessageRequest {
            scope: thread_scope.clone(),
            actor_id: binding.actor_user_id.as_str().to_string(),
            source_binding_id: source_binding_id.to_string(),
            external_event_id: external_event_id.to_string(),
        })
        .await
        .map_err(|error| ProductSurfaceFailure::Transient {
            reason: format!("failed to reconcile submitted mark: {error}"),
        })?;
    match replay {
        Some(replay)
            if replay.thread_id == binding.thread_id
                && replay.message_id == message_id
                && replay.status == MessageStatus::Submitted
                && replay.turn_run_id == Some(run_id) =>
        {
            Ok(())
        }
        _ => Err(ProductSurfaceFailure::Transient {
            reason: format!("failed to mark message submitted: {mark_error}"),
        }),
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
    use ironclaw_host_api::turn::{ReplyTargetBindingRef, SourceBindingRef};
    let source_binding_ref = replay
        .source_binding_id
        .as_deref()
        .and_then(|id| SourceBindingRef::new(id).ok())
        .unwrap_or_else(|| SourceBindingRef::new("source:replay").expect("valid placeholder ref"));
    let reply_target_binding_ref = replay
        .reply_target_binding_id
        .as_deref()
        .and_then(|id| ReplyTargetBindingRef::new(id).ok())
        .unwrap_or_else(|| {
            ReplyTargetBindingRef::new("reply:replay").expect("valid placeholder ref")
        });
    Ok(ResolvedBinding {
        tenant_id: replay.scope.tenant_id.clone(),
        actor_user_id,
        thread_id: replay.thread_id.clone(),
        agent_id: Some(replay.scope.agent_id.clone()),
        project_id: replay.scope.project_id.clone(),
        source_binding_ref,
        reply_target_binding_ref,
    })
}

use crate::run_delivery::thread_scope_from_binding;

/// Map an owned-thread ownership probe failure. "Does not exist" and
/// "owned by another caller" collapse into one indistinguishable failure
/// (no existence oracle); anything else is a transient store fault.
fn owned_thread_probe_failure(
    error: ironclaw_threads::SessionThreadError,
) -> ProductSurfaceFailure {
    match error {
        ironclaw_threads::SessionThreadError::UnknownThread { .. }
        | ironclaw_threads::SessionThreadError::ThreadScopeMismatch { .. } => {
            ProductSurfaceFailure::OwnedThreadUnavailable
        }
        other => ProductSurfaceFailure::Transient {
            reason: format!("owned-thread probe failed: {other}"),
        },
    }
}

/// The session lane's persisted source-binding-id scheme. Byte-identical to
/// what the dedicated browser path always wrote (caller-scoped, deliberately
/// thread-free so a retry replays across the caller context): changing it
/// breaks replay of already-accepted messages.
fn session_source_binding_id(scope: &TurnScope, actor: &TurnActor) -> String {
    format!(
        "{}{}{}{}{}{}",
        segment("surface", "webui"),
        segment("tenant", scope.tenant_id.as_str()),
        segment(
            "agent",
            scope
                .agent_id
                .as_ref()
                .map(ironclaw_host_api::ids::AgentId::as_str)
                .unwrap_or("")
        ),
        segment(
            "project_scope",
            if scope.project_id.is_some() {
                "bound"
            } else {
                "none"
            }
        ),
        scope
            .project_id
            .as_ref()
            .map(|project_id| segment("project", project_id.as_str()))
            .unwrap_or_default(),
        segment("actor", actor.user_id.as_str())
    )
}

/// The session lane's pre-migration thread-scoped binding-id scheme, probed
/// on replay so messages accepted by older builds do not double-accept.
fn legacy_session_source_binding_id(scope: &TurnScope, actor: &TurnActor) -> String {
    format!(
        "{}{}{}{}{}",
        segment("surface", "webui"),
        segment("tenant", scope.tenant_id.as_str()),
        segment(
            "agent",
            scope
                .agent_id
                .as_ref()
                .map(ironclaw_host_api::ids::AgentId::as_str)
                .unwrap_or("")
        ),
        segment("thread", scope.thread_id.as_str()),
        segment("actor", actor.user_id.as_str())
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
