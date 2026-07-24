//! InboundTurnService — the user-message turn submission path.
//!
//! This is the narrower user-message subset of [`ProductWorkflow`]. It
//! resolves product adapter envelopes into a thread-bound accepted message, then
//! hands off to the accepted-message turn submission seam. Keeping replay and
//! submit/deferred handling behind that seam prevents adapter-specific binding
//! code from owning the whole inbound turn pipeline.

use std::sync::Arc;
use std::time::Duration;

use crate::{
    ProductAdapterId, ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload,
    ProductRejection, ProductSourceChannel,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_attachments::InboundAttachment;
#[cfg(test)]
use ironclaw_host_api::UserId;
use ironclaw_threads::{
    AcceptInboundMessageRequest, AcceptedInboundMessageReplay, EnsureThreadRequest, MessageContent,
    MessageStatus, ReplayAcceptedInboundMessageRequest, SessionThreadService, ThreadMessageId,
    ThreadScope,
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
use crate::binding_ref::{DEFAULT_BINDING_REF_RAW_MAX_BYTES, bounded_idempotency_key};
use crate::error::ProductWorkflowError;
use crate::policy::{
    BeforeInboundPolicy, BeforeInboundPolicyOutcome, BeforeInboundPolicyRequest,
    NoopBeforeInboundPolicy,
};
use crate::reborn_services::InboundAttachmentLander;

#[cfg(not(any(test, feature = "test-support")))]
const BEFORE_INBOUND_POLICY_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(any(test, feature = "test-support"))]
const BEFORE_INBOUND_POLICY_TIMEOUT: Duration = Duration::from_millis(10);

/// Run a before-inbound policy with the workflow-owned wall-clock budget.
///
/// The timeout keeps slow policy backends from holding an idempotency
/// fingerprint in-flight indefinitely. A timed-out policy maps to a transient,
/// non-permanent [`ProductWorkflowError::BeforeInboundPolicyFailed`] so the
/// workflow releases the fingerprint and lets the same inbound action retry.
pub(crate) async fn check_before_inbound_policy(
    before_inbound_policy: &dyn BeforeInboundPolicy,
    request: BeforeInboundPolicyRequest,
) -> Result<BeforeInboundPolicyOutcome, ProductWorkflowError> {
    tokio::time::timeout(
        BEFORE_INBOUND_POLICY_TIMEOUT,
        before_inbound_policy.check_user_message(request),
    )
    .await
    .map_err(|_| ProductWorkflowError::BeforeInboundPolicyFailed {
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
        }
    }
}

/// Result of running replay, before-inbound policy, and fresh user-message acceptance.
pub enum InboundUserMessageDispatch {
    Accepted(Box<InboundTurnOutcome>),
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
    ) -> Result<Option<InboundTurnOutcome>, ProductWorkflowError>;

    /// Accept a user message envelope: resolve binding, stage message, submit turn.
    async fn accept_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<InboundTurnOutcome, ProductWorkflowError>;

    /// Accept a user message while preserving the replay-before-policy ordering.
    async fn accept_user_message_with_before_policy(
        &self,
        envelope: &ProductInboundEnvelope,
        before_inbound_policy: &dyn BeforeInboundPolicy,
    ) -> Result<InboundUserMessageDispatch, ProductWorkflowError>;

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
    ) -> Result<InboundUserMessageDispatch, ProductWorkflowError> {
        if !attachments.is_empty() {
            return Err(ProductWorkflowError::TurnSubmissionRejected {
                reason: "inbound attachments are not supported by this turn service".into(),
            });
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
}

impl<B, T, C> DefaultInboundTurnService<B, T, C>
where
    B: ConversationBindingService,
    T: SessionThreadService,
    C: TurnCoordinator,
{
    pub fn new(binding_service: B, thread_service: T, turn_coordinator: C) -> Self {
        Self {
            binding_service,
            thread_service,
            turn_coordinator,
            inbound_attachments: None,
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
    ) -> Result<Option<InboundTurnOutcome>, ProductWorkflowError> {
        let prepared = self.prepare_user_message(envelope).await?;
        self.replay_prepared_user_message(envelope, &prepared).await
    }

    async fn accept_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<InboundTurnOutcome, ProductWorkflowError> {
        let policy = NoopBeforeInboundPolicy;
        match self
            .accept_user_message_with_before_policy(envelope, &policy)
            .await?
        {
            InboundUserMessageDispatch::Accepted(outcome) => Ok(*outcome),
            InboundUserMessageDispatch::Rejected(_) => {
                Err(ProductWorkflowError::TurnSubmissionRejected {
                    reason: "noop before-inbound policy unexpectedly rejected message".into(),
                })
            }
        }
    }

    async fn accept_user_message_with_before_policy(
        &self,
        envelope: &ProductInboundEnvelope,
        before_inbound_policy: &dyn BeforeInboundPolicy,
    ) -> Result<InboundUserMessageDispatch, ProductWorkflowError> {
        self.accept_with_before_policy_inner(envelope, before_inbound_policy, Vec::new())
            .await
    }

    async fn accept_user_message_with_before_policy_and_attachments(
        &self,
        envelope: &ProductInboundEnvelope,
        before_inbound_policy: &dyn BeforeInboundPolicy,
        attachments: Vec<InboundAttachment>,
    ) -> Result<InboundUserMessageDispatch, ProductWorkflowError> {
        self.accept_with_before_policy_inner(envelope, before_inbound_policy, attachments)
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
    ) -> Result<InboundUserMessageDispatch, ProductWorkflowError> {
        let ProductInboundPayload::UserMessage(payload) = envelope.payload() else {
            return Err(ProductWorkflowError::UnsupportedActionKind {
                kind: "non_user_message".into(),
            });
        };
        let original_trigger = payload.trigger;
        let prepared = self.prepare_user_message(envelope).await?;
        if let Some(outcome) = self
            .replay_prepared_user_message(envelope, &prepared)
            .await?
        {
            return Ok(InboundUserMessageDispatch::Accepted(Box::new(outcome)));
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
                        ProductWorkflowError::TurnSubmissionRejected {
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

        self.accept_prepared_user_message(prepared_for_turn, envelope_for_turn, attachments)
            .await
            .map(Box::new)
            .map(InboundUserMessageDispatch::Accepted)
    }

    async fn prepare_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<PreparedUserMessage, ProductWorkflowError> {
        let ProductInboundPayload::UserMessage(payload) = envelope.payload() else {
            return Err(ProductWorkflowError::UnsupportedActionKind {
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
    ) -> Result<Option<InboundTurnOutcome>, ProductWorkflowError> {
        let Some(replay) = self
            .thread_service
            .replay_accepted_inbound_message(ReplayAcceptedInboundMessageRequest {
                scope: prepared.thread_scope.clone(),
                actor_id: prepared.binding.actor_user_id.as_str().to_string(),
                source_binding_id: prepared.source_binding_id.clone(),
                external_event_id: envelope.external_event_id().as_str().to_string(),
            })
            .await
            .map_err(|e| ProductWorkflowError::Transient {
                reason: format!("failed to replay accepted inbound message: {e}"),
            })?
        else {
            return Ok(None);
        };

        submit_or_replay_accepted_message(
            &self.thread_service,
            &self.turn_coordinator,
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
    ) -> Result<InboundTurnOutcome, ProductWorkflowError> {
        let ProductInboundPayload::UserMessage(payload) = envelope.payload() else {
            return Err(ProductWorkflowError::UnsupportedActionKind {
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
            .map_err(|e| ProductWorkflowError::Transient {
                reason: format!("failed to ensure thread: {e}"),
            })?;

        // Inline attachment bytes (e.g. images on the OpenAI-compatible
        // surface) are landed into project storage through the same authority
        // the agent's file tools resolve through, then carried on the message as
        // refs — never as raw bytes through the bytes-free product envelope.
        let content = if attachments.is_empty() {
            MessageContent::text(payload.text.clone())
        } else {
            let lander = self.inbound_attachments.as_ref().ok_or_else(|| {
                ProductWorkflowError::TurnSubmissionRejected {
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
                .map_err(|e| ProductWorkflowError::Transient {
                    reason: format!("failed to land inbound attachments: {e}"),
                })?;
            MessageContent::with_attachments(payload.text.clone(), refs)
        };

        let reply_target_binding_id = prepared.source_binding_id.clone();
        let accepted = self
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
            .map_err(|e| ProductWorkflowError::Transient {
                reason: format!("failed to accept inbound message: {e}"),
            })?;

        ProductInboundTurnHandoff::NeedsSubmission(Box::new(AcceptedProductInboundTurn {
            binding: prepared.binding,
            thread_scope: prepared.thread_scope,
            message_id: accepted.message_id,
            idempotency_key_raw: prepared.submit_idempotency_key,
            received_at: envelope.received_at(),
            adapter_id: prepared.adapter_id,
            source_channel: prepared.source_channel,
            surface_type: prepared.surface_type,
            requested_model: payload.requested_model.clone(),
        }))
        .submit_or_replay(&self.thread_service, &self.turn_coordinator)
        .await
    }
}

async fn submit_or_replay_accepted_message<T, C>(
    thread_service: &T,
    turn_coordinator: &C,
    replay: AcceptedInboundMessageReplay,
    submit_idempotency_key: String,
    received_at: DateTime<Utc>,
    prepared: &PreparedUserMessage,
) -> Result<InboundTurnOutcome, ProductWorkflowError>
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
    .submit_or_replay(thread_service, turn_coordinator)
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
    NeedsSubmission(Box<AcceptedProductInboundTurn>),
}

impl ProductInboundTurnHandoff {
    #[cfg(test)]
    fn from_replay(
        replay: AcceptedInboundMessageReplay,
        submit_idempotency_key: String,
        received_at: DateTime<Utc>,
        adapter_id: ProductAdapterId,
    ) -> Result<Self, ProductWorkflowError> {
        let binding = binding_from_replay(&replay)?;
        let thread_scope = replay.scope.clone();
        let source_channel = ProductSourceChannel::new(adapter_id.as_str()).map_err(|e| {
            ProductWorkflowError::TurnSubmissionRejected {
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
    ) -> Result<Self, ProductWorkflowError> {
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
    ) -> Result<Self, ProductWorkflowError> {
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
                return Err(ProductWorkflowError::TurnSubmissionRejected {
                    reason: "submitted replay missing turn_run_id".into(),
                });
            };
            let submitted_run_id = Uuid::parse_str(turn_run_id)
                .map(TurnRunId::from_uuid)
                .map_err(|e| ProductWorkflowError::TurnSubmissionRejected {
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
                        ProductWorkflowError::TurnSubmissionRejected {
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

        if !matches!(
            replay.status,
            MessageStatus::Accepted | MessageStatus::DeferredBusy
        ) {
            return Err(ProductWorkflowError::TurnSubmissionRejected {
                reason: format!(
                    "cannot resubmit inbound message replay in {:?} status",
                    replay.status
                ),
            });
        }

        Ok(Self::NeedsSubmission(Box::new(
            AcceptedProductInboundTurn {
                binding,
                thread_scope,
                message_id: replay.message_id,
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
    ) -> Result<InboundTurnOutcome, ProductWorkflowError>
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
            Self::NeedsSubmission(submission) => {
                submission.submit(thread_service, turn_coordinator).await
            }
        }
    }
}

struct AcceptedProductInboundTurn {
    binding: ResolvedBinding,
    thread_scope: ThreadScope,
    message_id: ThreadMessageId,
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
    ) -> Result<InboundTurnOutcome, ProductWorkflowError>
    where
        T: SessionThreadService,
        C: TurnCoordinator,
    {
        let Self {
            binding,
            thread_scope,
            message_id,
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
        let source_binding_ref = binding.source_binding_ref.clone();
        let accepted_message_ref = accepted_message_ref(message_id)?;
        let reply_target_binding_ref = binding.reply_target_binding_ref.clone();
        let idempotency_key = bounded_idempotency_key(
            "turn",
            &idempotency_key_raw,
            DEFAULT_BINDING_REF_RAW_MAX_BYTES,
        )
        .map_err(|e| ProductWorkflowError::TurnSubmissionRejected {
            reason: format!("invalid turn ref: {e}"),
        })?;

        let run_adapter =
            ironclaw_turns::RunOriginAdapter::new(adapter_id.as_str()).map_err(|e| {
                ProductWorkflowError::TurnSubmissionRejected {
                    reason: e.to_string(),
                }
            })?;
        let run_source_channel = ironclaw_turns::RunOriginAdapter::new(source_channel.as_str())
            .map_err(|e| ProductWorkflowError::TurnSubmissionRejected {
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
            scope: turn_scope,
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
                    .map_err(|e| ProductWorkflowError::Transient {
                        reason: format!("failed to mark message submitted: {e}"),
                    })?;
                Ok(InboundTurnOutcome::Submitted {
                    accepted_message_ref,
                    submitted_run_id: run_id,
                    binding,
                })
            }
            Err(TurnError::ThreadBusy(busy)) => {
                thread_service
                    .mark_message_rejected_busy(&thread_scope, &binding.thread_id, message_id)
                    .await
                    .map_err(|e| ProductWorkflowError::Transient {
                        reason: format!("failed to mark message rejected: {e}"),
                    })?;
                Ok(InboundTurnOutcome::RejectedBusy {
                    accepted_message_ref,
                    active_run_id: Some(busy.active_run_id),
                    binding,
                })
            }
            Err(error) => Err(ProductWorkflowError::TurnSubmissionFailed { error }),
        }
    }
}

fn accepted_message_ref(
    message_id: ThreadMessageId,
) -> Result<AcceptedMessageRef, ProductWorkflowError> {
    AcceptedMessageRef::new(format!("msg:{message_id}")).map_err(|e| {
        ProductWorkflowError::TurnSubmissionRejected {
            reason: format!("invalid accepted message ref: {e}"),
        }
    })
}

#[cfg(test)]
fn binding_from_replay(
    replay: &AcceptedInboundMessageReplay,
) -> Result<ResolvedBinding, ProductWorkflowError> {
    let actor_user_id = match replay.actor_id.as_deref() {
        Some(actor_id) => {
            UserId::new(actor_id).map_err(|e| ProductWorkflowError::BindingResolutionFailed {
                reason: format!("invalid replay actor user id: {e}"),
            })?
        }
        None => replay.scope.owner_user_id.clone().ok_or_else(|| {
            ProductWorkflowError::BindingResolutionFailed {
                reason: "accepted replay missing actor user id and owner user id".into(),
            }
        })?,
    };
    Ok(ResolvedBinding {
        tenant_id: replay.scope.tenant_id.clone(),
        actor_user_id,
        subject_user_id: replay.scope.owner_user_id.clone(),
        source_binding_ref: ironclaw_turns::SourceBindingRef::new("source:test-replay").map_err(
            |e| ProductWorkflowError::BindingResolutionFailed {
                reason: e.to_string(),
            },
        )?,
        reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new("reply:test-replay")
            .map_err(|e| ProductWorkflowError::BindingResolutionFailed {
            reason: e.to_string(),
        })?,
        thread_id: replay.thread_id.clone(),
        agent_id: Some(replay.scope.agent_id.clone()),
        project_id: replay.scope.project_id.clone(),
    })
}

fn thread_scope_from_binding(
    binding: &ResolvedBinding,
) -> Result<ThreadScope, ProductWorkflowError> {
    let Some(agent_id) = binding.agent_id.clone() else {
        return Err(ProductWorkflowError::BindingResolutionFailed {
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
mod tests {
    use std::{future::pending, sync::Mutex};

    use crate::{
        AdapterInstallationId, ExternalActorRef, ExternalConversationRef, ProductAdapterId,
        ProductTriggerReason, UserMessagePayload,
    };
    use async_trait::async_trait;
    use chrono::TimeZone;
    use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
    use ironclaw_threads::{
        AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
        AppendAssistantDraftRequest, AppendCapabilityDisplayPreviewRequest,
        AppendToolResultReferenceRequest, ContextMessages, ContextWindow,
        CreateSummaryArtifactRequest, EnsureThreadRequest, ListThreadsForScopeRequest,
        ListThreadsForScopeResponse, LoadContextMessagesRequest, LoadContextWindowRequest,
        MessageContent, RedactMessageRequest, ReplayAcceptedInboundMessageRequest,
        SessionThreadError, SessionThreadRecord, SummaryArtifact, ThreadHistory,
        ThreadHistoryRequest, ThreadMessageId, ThreadMessageRecord, ThreadScope,
        UpdateAssistantDraftRequest, UpdateToolResultReferenceRequest,
    };
    use ironclaw_turns::{
        CancelRunRequest, CancelRunResponse, GetRunStateRequest, ResumeTurnRequest,
        ResumeTurnResponse, RetryTurnRequest, RetryTurnResponse, RunProfileId, RunProfileVersion,
        SubmitTurnRequest, SubmitTurnResponse, TurnCoordinator, TurnError, TurnId, TurnOriginKind,
        TurnRunId, TurnRunState, TurnScope, TurnStatus, TurnSurfaceType, events::EventCursor,
    };

    use crate::action::SourceBindingKey;

    use super::*;

    // --- Minimal stubs for submit path tests ---

    #[derive(Default)]
    struct CapturingTurnCoordinator {
        submissions: Mutex<Vec<SubmitTurnRequest>>,
    }

    impl CapturingTurnCoordinator {
        fn submissions(&self) -> Vec<SubmitTurnRequest> {
            self.submissions.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl TurnCoordinator for CapturingTurnCoordinator {
        async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
            Ok(TurnRunId::new())
        }

        async fn submit_turn(
            &self,
            request: SubmitTurnRequest,
        ) -> Result<SubmitTurnResponse, TurnError> {
            let run_id = TurnRunId::new();
            let message_ref = request.accepted_message_ref.clone();
            let reply_ref = request.reply_target_binding_ref.clone();
            self.submissions.lock().unwrap().push(request);
            Ok(SubmitTurnResponse::Accepted {
                turn_id: TurnId::new(),
                run_id,
                status: TurnStatus::Completed,
                resolved_run_profile_id: RunProfileId::default_profile(),
                resolved_run_profile_version: RunProfileVersion::new(1),
                event_cursor: EventCursor(0),
                accepted_message_ref: message_ref,
                reply_target_binding_ref: reply_ref,
            })
        }

        async fn resume_turn(
            &self,
            _request: ResumeTurnRequest,
        ) -> Result<ResumeTurnResponse, TurnError> {
            unimplemented!("not used in submit path tests")
        }

        async fn retry_turn(
            &self,
            _request: RetryTurnRequest,
        ) -> Result<RetryTurnResponse, TurnError> {
            unimplemented!("not used in submit path tests")
        }

        async fn cancel_run(
            &self,
            _request: CancelRunRequest,
        ) -> Result<CancelRunResponse, TurnError> {
            unimplemented!("not used in submit path tests")
        }

        async fn get_run_state(
            &self,
            _request: GetRunStateRequest,
        ) -> Result<TurnRunState, TurnError> {
            unimplemented!("not used in submit path tests")
        }
    }

    struct StubSessionThreadService;

    #[async_trait]
    impl ironclaw_threads::SessionThreadService for StubSessionThreadService {
        async fn ensure_thread(
            &self,
            _request: EnsureThreadRequest,
        ) -> Result<SessionThreadRecord, SessionThreadError> {
            unimplemented!("not used in submit path tests")
        }

        async fn accept_inbound_message(
            &self,
            _request: AcceptInboundMessageRequest,
        ) -> Result<AcceptedInboundMessage, SessionThreadError> {
            unimplemented!("not used in submit path tests")
        }

        async fn replay_accepted_inbound_message(
            &self,
            _request: ReplayAcceptedInboundMessageRequest,
        ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
            Ok(None)
        }

        async fn mark_message_submitted(
            &self,
            _scope: &ThreadScope,
            _thread_id: &ironclaw_host_api::ThreadId,
            _message_id: ThreadMessageId,
            _turn_id: String,
            _turn_run_id: String,
        ) -> Result<ThreadMessageRecord, SessionThreadError> {
            Ok(stub_message_record(_message_id))
        }

        async fn mark_message_rejected_busy(
            &self,
            _scope: &ThreadScope,
            _thread_id: &ironclaw_host_api::ThreadId,
            _message_id: ThreadMessageId,
        ) -> Result<ThreadMessageRecord, SessionThreadError> {
            unimplemented!("not used in submit path tests")
        }

        async fn append_assistant_draft(
            &self,
            _request: AppendAssistantDraftRequest,
        ) -> Result<ThreadMessageRecord, SessionThreadError> {
            unimplemented!("not used in submit path tests")
        }

        async fn append_tool_result_reference(
            &self,
            _request: AppendToolResultReferenceRequest,
        ) -> Result<ThreadMessageRecord, SessionThreadError> {
            unimplemented!("not used in submit path tests")
        }

        async fn append_capability_display_preview(
            &self,
            _request: AppendCapabilityDisplayPreviewRequest,
        ) -> Result<ThreadMessageRecord, SessionThreadError> {
            unimplemented!("not used in submit path tests")
        }

        async fn update_tool_result_reference(
            &self,
            _request: UpdateToolResultReferenceRequest,
        ) -> Result<ThreadMessageRecord, SessionThreadError> {
            unimplemented!("not used in submit path tests")
        }

        async fn update_assistant_draft(
            &self,
            _request: UpdateAssistantDraftRequest,
        ) -> Result<ThreadMessageRecord, SessionThreadError> {
            unimplemented!("not used in submit path tests")
        }

        async fn finalize_assistant_message(
            &self,
            _scope: &ThreadScope,
            _thread_id: &ironclaw_host_api::ThreadId,
            _message_id: ThreadMessageId,
            _content: MessageContent,
        ) -> Result<ThreadMessageRecord, SessionThreadError> {
            unimplemented!("not used in submit path tests")
        }

        async fn redact_message(
            &self,
            _request: RedactMessageRequest,
        ) -> Result<ThreadMessageRecord, SessionThreadError> {
            unimplemented!("not used in submit path tests")
        }

        async fn load_context_window(
            &self,
            _request: LoadContextWindowRequest,
        ) -> Result<ContextWindow, SessionThreadError> {
            unimplemented!("not used in submit path tests")
        }

        async fn load_context_messages(
            &self,
            _request: LoadContextMessagesRequest,
        ) -> Result<ContextMessages, SessionThreadError> {
            unimplemented!("not used in submit path tests")
        }

        async fn list_thread_history(
            &self,
            _request: ThreadHistoryRequest,
        ) -> Result<ThreadHistory, SessionThreadError> {
            unimplemented!("not used in submit path tests")
        }

        async fn create_summary_artifact(
            &self,
            _request: CreateSummaryArtifactRequest,
        ) -> Result<SummaryArtifact, SessionThreadError> {
            unimplemented!("not used in submit path tests")
        }

        async fn list_threads_for_scope(
            &self,
            _request: ListThreadsForScopeRequest,
        ) -> Result<ListThreadsForScopeResponse, SessionThreadError> {
            unimplemented!("not used in submit path tests")
        }
    }

    fn stub_message_record(message_id: ThreadMessageId) -> ThreadMessageRecord {
        ThreadMessageRecord {
            message_id,
            thread_id: thread_id(),
            sequence: 1,
            kind: ironclaw_threads::MessageKind::User,
            status: ironclaw_threads::MessageStatus::Submitted,
            created_at: None,
            updated_at: None,
            actor_id: None,
            source_binding_id: None,
            reply_target_binding_id: None,
            turn_id: None,
            turn_run_id: None,
            tool_result_ref: None,
            tool_result_provider_call: None,
            content: None,
            attachments: Vec::new(),
            redaction_ref: None,
        }
    }

    /// The legacy `from_replay` path hard-codes `TurnSurfaceType::Direct` and injects the
    /// adapter id. This test drives the handoff through `submit_or_replay` and asserts
    /// that the submitted `SubmitTurnRequest.product_context` carries `Direct` surface and
    /// the adapter from the replay call.
    #[tokio::test]
    async fn replay_submit_carries_direct_surface_type_and_adapter_id() {
        let adapter_id = ProductAdapterId::new("telegram").unwrap();
        let message_id = ThreadMessageId::new();
        let handoff = ProductInboundTurnHandoff::from_replay(
            replay(
                message_id,
                MessageStatus::DeferredBusy,
                Some("src:replay"),
                Some("reply:replay"),
                None,
            ),
            "turn-key-replay".to_string(),
            received_at(),
            adapter_id.clone(),
        )
        .expect("replay handoff");

        let coordinator = CapturingTurnCoordinator::default();
        let thread_service = StubSessionThreadService;

        handoff
            .submit_or_replay(&thread_service, &coordinator)
            .await
            .expect("submit_or_replay succeeds");

        let submissions = coordinator.submissions();
        assert_eq!(submissions.len(), 1, "one turn must be submitted");
        let ctx = submissions[0]
            .product_context
            .as_ref()
            .expect("product_context must be set");
        assert_eq!(
            ctx.surface_type,
            Some(TurnSurfaceType::Direct),
            "replay path must carry Direct surface type"
        );
        assert_eq!(
            ctx.adapter.as_ref().map(|a| a.as_str()),
            Some(adapter_id.as_str()),
            "replay path must carry the adapter id"
        );
        assert_eq!(
            ctx.origin,
            TurnOriginKind::Inbound,
            "replay path must record Inbound origin (Untrusted classification)"
        );
    }

    struct PendingBeforeInboundPolicy;

    #[async_trait]
    impl BeforeInboundPolicy for PendingBeforeInboundPolicy {
        async fn check_user_message(
            &self,
            _request: BeforeInboundPolicyRequest,
        ) -> Result<BeforeInboundPolicyOutcome, ProductWorkflowError> {
            pending().await
        }
    }

    #[tokio::test]
    async fn check_before_inbound_policy_times_out_as_retryable_failure() {
        let err = check_before_inbound_policy(&PendingBeforeInboundPolicy, policy_request())
            .await
            .expect_err("pending policy should time out");

        assert!(matches!(
            err,
            ProductWorkflowError::BeforeInboundPolicyFailed {
                permanent: false,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn noop_before_inbound_policy_allows_user_messages() {
        let outcome = NoopBeforeInboundPolicy
            .check_user_message(policy_request())
            .await
            .expect("noop policy should not fail");

        assert_eq!(outcome, BeforeInboundPolicyOutcome::Allow);
    }

    #[test]
    fn submitted_replay_becomes_already_submitted_handoff() {
        let submitted_run_id = TurnRunId::new();
        let message_id = ThreadMessageId::new();
        let handoff = ProductInboundTurnHandoff::from_replay(
            replay(
                message_id,
                MessageStatus::Submitted,
                Some("src:alpha"),
                Some("reply:alpha"),
                Some(submitted_run_id.to_string()),
            ),
            "turn-key".to_string(),
            received_at(),
            ProductAdapterId::new("test_adapter").unwrap(),
        )
        .expect("submitted replay handoff");

        let ProductInboundTurnHandoff::AlreadySubmitted {
            accepted_message_ref: actual_message_ref,
            submitted_run_id: actual_run_id,
            binding,
        } = handoff
        else {
            panic!("expected submitted replay to short-circuit turn submission")
        };

        assert_eq!(actual_run_id, submitted_run_id);
        assert_eq!(
            actual_message_ref,
            accepted_message_ref(message_id).unwrap()
        );
        assert_eq!(binding.thread_id, thread_id());
    }

    #[test]
    fn rejected_busy_replay_becomes_already_rejected_handoff() {
        let message_id = ThreadMessageId::new();
        let handoff = ProductInboundTurnHandoff::from_replay(
            replay(
                message_id,
                MessageStatus::RejectedBusy,
                Some("src:alpha"),
                Some("reply:alpha"),
                None,
            ),
            "turn-key".to_string(),
            received_at(),
            ProductAdapterId::new("test_adapter").unwrap(),
        )
        .expect("rejected busy replay handoff");

        let ProductInboundTurnHandoff::AlreadyRejected {
            accepted_message_ref: actual_message_ref,
            active_run_id,
            ..
        } = handoff
        else {
            panic!("expected rejected busy replay to be terminal, not resubmitted")
        };

        assert_eq!(
            actual_message_ref,
            accepted_message_ref(message_id).unwrap()
        );
        assert!(active_run_id.is_none());
    }

    #[test]
    fn legacy_replay_without_actor_id_uses_owner_as_actor() {
        let message_id = ThreadMessageId::new();
        let mut replay = replay(
            message_id,
            MessageStatus::DeferredBusy,
            Some("src:alpha"),
            Some("reply:alpha"),
            None,
        );
        replay.actor_id = None;

        let handoff = ProductInboundTurnHandoff::from_replay(
            replay,
            "turn-key".to_string(),
            received_at(),
            ProductAdapterId::new("test_adapter").unwrap(),
        )
        .expect("legacy replay handoff");

        let ProductInboundTurnHandoff::NeedsSubmission(submission) = handoff else {
            panic!("expected legacy replay to require a new turn submission")
        };

        assert_eq!(submission.binding.actor_user_id, user_id());
        assert_eq!(submission.binding.subject_user_id, Some(user_id()));
        assert_eq!(submission.message_id, message_id);
    }

    #[test]
    fn prepared_replay_uses_fresh_binding_scope_over_persisted_scope() {
        let message_id = ThreadMessageId::new();
        let mut replay = replay(
            message_id,
            MessageStatus::DeferredBusy,
            Some("src:alpha"),
            Some("reply:alpha"),
            None,
        );
        replay.scope.owner_user_id = None;
        let subject_user_id = UserId::new("user:team-subject").unwrap();
        let prepared = PreparedUserMessage {
            binding: ResolvedBinding {
                tenant_id: tenant_id(),
                actor_user_id: user_id(),
                subject_user_id: Some(subject_user_id.clone()),
                source_binding_ref: ironclaw_turns::SourceBindingRef::new("source:test-prepared")
                    .unwrap(),
                reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new(
                    "reply:test-prepared",
                )
                .unwrap(),
                thread_id: thread_id(),
                agent_id: Some(AgentId::new("agent:alpha").unwrap()),
                project_id: None,
            },
            thread_scope: ThreadScope {
                tenant_id: tenant_id(),
                agent_id: AgentId::new("agent:alpha").unwrap(),
                project_id: None,
                owner_user_id: Some(subject_user_id.clone()),
                mission_id: None,
            },
            source_binding_id: "src:alpha".to_string(),
            submit_idempotency_key: "turn-key".to_string(),
            adapter_id: ProductAdapterId::new("test_adapter").unwrap(),
            source_channel: ProductSourceChannel::new("test_adapter").unwrap(),
            surface_type: TurnSurfaceType::Direct,
        };

        let handoff = ProductInboundTurnHandoff::from_replay_with_prepared(
            replay,
            "turn-key".to_string(),
            received_at(),
            &prepared,
        )
        .expect("prepared replay handoff");

        let ProductInboundTurnHandoff::NeedsSubmission(submission) = handoff else {
            panic!("expected prepared replay to require a new turn submission")
        };

        assert_eq!(
            submission.binding.subject_user_id,
            Some(subject_user_id.clone())
        );
        assert_eq!(submission.thread_scope.owner_user_id, Some(subject_user_id));
        assert_eq!(submission.message_id, message_id);
    }

    /// A BotMention shared route must produce `TurnSurfaceType::Channel` in the
    /// submitted `SubmitTurnRequest.product_context`. This exercises the
    /// `ProductConversationRouteKind::Shared => TurnSurfaceType::Channel` branch
    /// in `prepare_user_message` through the replay-with-prepared handoff path,
    /// which is the same submission seam the full inbound-turn pipeline uses.
    #[tokio::test]
    async fn shared_user_message_records_channel_surface_type() {
        let message_id = ThreadMessageId::new();
        let prepared = PreparedUserMessage {
            binding: ResolvedBinding {
                tenant_id: tenant_id(),
                actor_user_id: user_id(),
                subject_user_id: Some(user_id()),
                source_binding_ref: ironclaw_turns::SourceBindingRef::new("source:test-shared")
                    .unwrap(),
                reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new(
                    "reply:test-shared",
                )
                .unwrap(),
                thread_id: thread_id(),
                agent_id: Some(AgentId::new("agent:alpha").unwrap()),
                project_id: None,
            },
            thread_scope: ThreadScope {
                tenant_id: tenant_id(),
                agent_id: AgentId::new("agent:alpha").unwrap(),
                project_id: None,
                owner_user_id: Some(user_id()),
                mission_id: None,
            },
            source_binding_id: "src:shared".to_string(),
            submit_idempotency_key: "turn-key-shared".to_string(),
            adapter_id: ProductAdapterId::new("slack").unwrap(),
            source_channel: ProductSourceChannel::new("slack").unwrap(),
            // BotMention shared route maps to Channel surface type.
            surface_type: TurnSurfaceType::Channel,
        };

        let handoff = ProductInboundTurnHandoff::from_replay_with_prepared(
            replay(
                message_id,
                MessageStatus::DeferredBusy,
                Some("src:shared"),
                Some("reply:shared"),
                None,
            ),
            "turn-key-shared".to_string(),
            received_at(),
            &prepared,
        )
        .expect("shared route replay handoff");

        let coordinator = CapturingTurnCoordinator::default();
        let thread_service = StubSessionThreadService;

        handoff
            .submit_or_replay(&thread_service, &coordinator)
            .await
            .expect("submit_or_replay succeeds");

        let submissions = coordinator.submissions();
        assert_eq!(submissions.len(), 1, "one turn must be submitted");
        let ctx = submissions[0]
            .product_context
            .as_ref()
            .expect("product_context must be set");
        assert_eq!(
            ctx.surface_type,
            Some(TurnSurfaceType::Channel),
            "BotMention shared route must carry Channel surface type"
        );
        assert_eq!(
            ctx.source_channel
                .as_ref()
                .map(ironclaw_turns::RunOriginAdapter::as_str),
            Some("slack"),
            "shared route must preserve source channel"
        );
    }

    fn policy_request() -> BeforeInboundPolicyRequest {
        BeforeInboundPolicyRequest {
            adapter_id: ProductAdapterId::new("test_adapter").expect("adapter"),
            installation_id: AdapterInstallationId::new("install_alpha").expect("installation"),
            external_actor_ref: ExternalActorRef::new("test", "user1", Option::<String>::None)
                .expect("actor"),
            external_conversation_ref: ExternalConversationRef::new(None, "conv1", None, None)
                .expect("conversation"),
            source_binding_key: SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
                .expect("source binding key"),
            rate_limit_key: SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
                .expect("rate limit key"),
            user_message: UserMessagePayload::new(
                "hello",
                vec![],
                ProductTriggerReason::DirectChat,
            )
            .expect("message"),
        }
    }

    fn replay(
        message_id: ThreadMessageId,
        status: MessageStatus,
        source_binding_id: Option<&str>,
        reply_target_binding_id: Option<&str>,
        turn_run_id: Option<String>,
    ) -> AcceptedInboundMessageReplay {
        AcceptedInboundMessageReplay {
            scope: ThreadScope {
                tenant_id: tenant_id(),
                agent_id: AgentId::new("agent:alpha").unwrap(),
                project_id: None,
                owner_user_id: Some(user_id()),
                mission_id: None,
            },
            thread_id: thread_id(),
            message_id,
            sequence: 1,
            status,
            actor_id: Some(user_id().as_str().to_string()),
            source_binding_id: source_binding_id.map(str::to_string),
            reply_target_binding_id: reply_target_binding_id.map(str::to_string),
            turn_run_id,
        }
    }

    fn received_at() -> DateTime<Utc> {
        Utc.timestamp_opt(0, 0).single().unwrap()
    }

    fn tenant_id() -> TenantId {
        TenantId::new("tenant:alpha").unwrap()
    }

    fn user_id() -> UserId {
        UserId::new("user:alpha").unwrap()
    }

    fn thread_id() -> ThreadId {
        ThreadId::new("thread:alpha").unwrap()
    }

    // --- Inline-attachment landing (vision, #4644) ---

    use crate::{
        AuthRequirement, ExternalEventId, ParsedProductInbound, ProductInboundEnvelope,
        ProductInboundPayload, ProtocolAuthEvidence, TrustedInboundContext,
    };
    use ironclaw_threads::{AttachmentKind, AttachmentRef, InMemorySessionThreadService};

    use crate::binding::ResolveBindingRequest;
    use ironclaw_host_api::ProductSurfaceError;

    struct LandingBindingStub;

    #[async_trait]
    impl ConversationBindingService for LandingBindingStub {
        async fn resolve_binding(
            &self,
            _request: ResolveBindingRequest,
        ) -> Result<ResolvedBinding, ProductWorkflowError> {
            Ok(ResolvedBinding {
                tenant_id: tenant_id(),
                actor_user_id: user_id(),
                subject_user_id: Some(user_id()),
                source_binding_ref: ironclaw_turns::SourceBindingRef::new("source:test-landing")
                    .unwrap(),
                reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new(
                    "reply:test-landing",
                )
                .unwrap(),
                thread_id: thread_id(),
                agent_id: Some(AgentId::new("agent:alpha").unwrap()),
                project_id: None,
            })
        }

        async fn lookup_binding(
            &self,
            request: ResolveBindingRequest,
        ) -> Result<ResolvedBinding, ProductWorkflowError> {
            self.resolve_binding(request).await
        }
    }

    #[derive(Default)]
    struct CapturingLander {
        landed: Mutex<Vec<InboundAttachment>>,
    }

    #[async_trait]
    impl InboundAttachmentLander for CapturingLander {
        async fn land(
            &self,
            _thread_scope: &ThreadScope,
            message_id: &str,
            attachments: Vec<InboundAttachment>,
        ) -> Result<Vec<AttachmentRef>, ProductSurfaceError> {
            let refs = attachments
                .iter()
                .enumerate()
                .map(|(index, attachment)| AttachmentRef {
                    id: attachment.id.clone(),
                    kind: AttachmentKind::Image,
                    mime_type: attachment.mime_type.clone(),
                    filename: attachment.filename.clone(),
                    size_bytes: Some(attachment.bytes.len() as u64),
                    storage_key: Some(format!(
                        "/workspace/attachments/test/{message_id}-{index}-img"
                    )),
                    extracted_text: None,
                })
                .collect();
            self.landed.lock().unwrap().extend(attachments);
            Ok(refs)
        }
    }

    fn user_message_envelope() -> ProductInboundEnvelope {
        let installation_id = AdapterInstallationId::new("install_alpha").expect("install");
        let evidence = ProtocolAuthEvidence::test_verified(
            AuthRequirement::SharedSecretHeader {
                header_name: "X-Secret".into(),
            },
            installation_id.as_str(),
        );
        let context = TrustedInboundContext::from_verified_evidence(
            ProductAdapterId::new("test_adapter").expect("adapter"),
            installation_id,
            received_at(),
            &evidence,
        )
        .expect("trusted context");
        let parsed = ParsedProductInbound::new(
            ExternalEventId::new("evt:image-1").expect("event"),
            ExternalActorRef::new("test", "user1", None::<String>).expect("actor"),
            ExternalConversationRef::new(None, "conv1", None, None).expect("conversation"),
            ProductInboundPayload::UserMessage(
                UserMessagePayload::new("look at this", vec![], ProductTriggerReason::DirectChat)
                    .expect("payload"),
            ),
        )
        .expect("parsed inbound");
        ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope")
    }

    /// Caller-level coverage for the native vision door: a user message carrying
    /// host-staged inline bytes must route those bytes through the
    /// [`InboundAttachmentLander`] before message acceptance (the bytes never
    /// touch the bytes-free product envelope). Mirrors the WebChat landing path.
    #[tokio::test]
    async fn native_attachment_path_lands_inline_bytes_before_acceptance() {
        let thread_service = std::sync::Arc::new(InMemorySessionThreadService::default());
        let lander = std::sync::Arc::new(CapturingLander::default());
        let service = DefaultInboundTurnService::new(
            LandingBindingStub,
            thread_service,
            CapturingTurnCoordinator::default(),
        )
        .with_inbound_attachments(lander.clone());

        let envelope = user_message_envelope();
        let bytes = vec![0x89, b'P', b'N', b'G'];
        let attachment = InboundAttachment {
            id: "openai-image-0".to_string(),
            mime_type: "image/png".to_string(),
            filename: Some("image-0.png".to_string()),
            bytes: bytes.clone(),
        };

        let dispatch = service
            .accept_user_message_with_before_policy_and_attachments(
                &envelope,
                &NoopBeforeInboundPolicy,
                vec![attachment],
            )
            .await
            .expect("accepting a user message with inline attachments succeeds");

        assert!(matches!(dispatch, InboundUserMessageDispatch::Accepted(_)));
        let landed = lander.landed.lock().unwrap();
        assert_eq!(landed.len(), 1, "the inline image is landed exactly once");
        assert_eq!(landed[0].mime_type, "image/png");
        assert_eq!(landed[0].bytes, bytes);
    }

    /// Without a lander wired, a user message carrying inline bytes must fail
    /// closed (rejected), never silently dropping the attachment.
    #[tokio::test]
    async fn native_attachment_path_without_lander_fails_closed() {
        let thread_service = std::sync::Arc::new(InMemorySessionThreadService::default());
        let service = DefaultInboundTurnService::new(
            LandingBindingStub,
            thread_service,
            CapturingTurnCoordinator::default(),
        );

        let envelope = user_message_envelope();
        let attachment = InboundAttachment {
            id: "openai-image-0".to_string(),
            mime_type: "image/png".to_string(),
            filename: Some("image-0.png".to_string()),
            bytes: vec![0x89, b'P', b'N', b'G'],
        };

        let result = service
            .accept_user_message_with_before_policy_and_attachments(
                &envelope,
                &NoopBeforeInboundPolicy,
                vec![attachment],
            )
            .await;

        assert!(
            matches!(
                result,
                Err(ProductWorkflowError::TurnSubmissionRejected { .. })
            ),
            "a missing lander must reject the turn, never silently drop the attachment"
        );
    }

    /// A turn service that does not override the attachments method, exercising
    /// the trait default. Its `accept_user_message_with_before_policy` returns a
    /// distinct `Transient` error so a test can tell "the default delegated"
    /// (Transient) apart from "the default rejected" (TurnSubmissionRejected).
    struct DefaultAttachmentsTurnService;

    #[async_trait]
    impl InboundTurnService for DefaultAttachmentsTurnService {
        async fn replay_accepted_user_message(
            &self,
            _envelope: &ProductInboundEnvelope,
        ) -> Result<Option<InboundTurnOutcome>, ProductWorkflowError> {
            Ok(None)
        }

        async fn accept_user_message(
            &self,
            _envelope: &ProductInboundEnvelope,
        ) -> Result<InboundTurnOutcome, ProductWorkflowError> {
            Err(ProductWorkflowError::Transient {
                reason: "delegated".into(),
            })
        }

        async fn accept_user_message_with_before_policy(
            &self,
            _envelope: &ProductInboundEnvelope,
            _before_inbound_policy: &dyn BeforeInboundPolicy,
        ) -> Result<InboundUserMessageDispatch, ProductWorkflowError> {
            Err(ProductWorkflowError::Transient {
                reason: "delegated".into(),
            })
        }
    }

    /// The trait default must reject a turn carrying inline bytes rather than
    /// silently dropping them, but still pass an attachment-free turn straight
    /// through to the underlying acceptance path.
    #[tokio::test]
    async fn default_attachments_impl_rejects_bytes_but_passes_empty_through() {
        let service = DefaultAttachmentsTurnService;
        let envelope = user_message_envelope();

        let rejected = service
            .accept_user_message_with_before_policy_and_attachments(
                &envelope,
                &NoopBeforeInboundPolicy,
                vec![InboundAttachment {
                    id: "openai-image-0".to_string(),
                    mime_type: "image/png".to_string(),
                    filename: Some("image-0.png".to_string()),
                    bytes: vec![0x89, b'P', b'N', b'G'],
                }],
            )
            .await;
        assert!(
            matches!(
                rejected,
                Err(ProductWorkflowError::TurnSubmissionRejected { .. })
            ),
            "the default must fail closed on inline bytes, never silently drop them"
        );

        let delegated = service
            .accept_user_message_with_before_policy_and_attachments(
                &envelope,
                &NoopBeforeInboundPolicy,
                Vec::new(),
            )
            .await;
        assert!(
            matches!(delegated, Err(ProductWorkflowError::Transient { .. })),
            "with no attachments the default must delegate to the normal path"
        );
    }

    #[test]
    fn rejected_busy_replay_with_invalid_turn_run_id_fails_loudly() {
        let message_id = ThreadMessageId::new();
        let result = ProductInboundTurnHandoff::from_replay(
            replay(
                message_id,
                MessageStatus::RejectedBusy,
                Some("src:alpha"),
                Some("reply:alpha"),
                Some("not-a-uuid".to_string()),
            ),
            "turn-key".to_string(),
            received_at(),
            ProductAdapterId::new("test_adapter").unwrap(),
        );
        let err = match result {
            Err(e) => e,
            Ok(_) => panic!("expected Err for malformed turn_run_id, got Ok"),
        };

        match err {
            ProductWorkflowError::TurnSubmissionRejected { reason } => {
                assert!(
                    reason.contains("invalid rejected busy turn_run_id"),
                    "expected reason to contain 'invalid rejected busy turn_run_id', got: {reason}"
                );
            }
            other => panic!("expected TurnSubmissionRejected, got: {other:?}"),
        }
    }
}
