//! InboundTurnService — the user-message turn submission path.
//!
//! This is the narrower user-message subset of [`ProductWorkflow`]. It
//! resolves product adapter envelopes into a thread-bound accepted message, then
//! hands off to the accepted-message turn submission seam. Keeping replay and
//! submit/deferred handling behind that seam prevents adapter-specific binding
//! code from owning the whole inbound turn pipeline.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::UserId;
use ironclaw_product_adapters::{
    ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload, ProductRejection,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AcceptedInboundMessageReplay, EnsureThreadRequest, MessageContent,
    MessageStatus, ReplayAcceptedInboundMessageRequest, SessionThreadService, ThreadMessageId,
    ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCoordinator,
    TurnError, TurnRunId, TurnScope,
};
use uuid::Uuid;

use crate::binding::{
    ConversationBindingService, ProductConversationRouteKind, ResolveBindingRequest,
    ResolvedBinding,
};
use crate::binding_ref::{
    DEFAULT_BINDING_REF_RAW_MAX_BYTES, bounded_idempotency_key, bounded_reply_target_binding_ref,
    bounded_source_binding_ref,
};
use crate::error::ProductWorkflowError;
use crate::policy::{
    BeforeInboundPolicy, BeforeInboundPolicyOutcome, BeforeInboundPolicyRequest,
    NoopBeforeInboundPolicy,
};

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
    /// was accepted but deferred.
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
}

/// Default implementation that composes a [`ConversationBindingService`] with a
/// [`SessionThreadService`] and [`TurnCoordinator`].
pub struct DefaultInboundTurnService<B, T, C> {
    binding_service: B,
    thread_service: T,
    turn_coordinator: C,
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
            InboundUserMessageDispatch::Accepted(outcome) => Ok(outcome),
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
            return Ok(InboundUserMessageDispatch::Accepted(outcome));
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

        self.accept_prepared_user_message(prepared_for_turn, envelope_for_turn)
            .await
            .map(InboundUserMessageDispatch::Accepted)
    }
}

impl<B, T, C> DefaultInboundTurnService<B, T, C>
where
    B: ConversationBindingService,
    T: SessionThreadService,
    C: TurnCoordinator,
{
    async fn prepare_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<PreparedUserMessage, ProductWorkflowError> {
        let ProductInboundPayload::UserMessage(payload) = envelope.payload() else {
            return Err(ProductWorkflowError::UnsupportedActionKind {
                kind: "non_user_message".into(),
            });
        };
        let route_kind = route_kind_for_user_message(payload.trigger);
        let binding = self
            .binding_service
            .resolve_binding(ResolveBindingRequest {
                adapter_id: envelope.adapter_id().clone(),
                installation_id: envelope.installation_id().clone(),
                external_actor_ref: envelope.external_actor_ref().clone(),
                external_conversation_ref: envelope.external_conversation_ref().clone(),
                external_event_id: envelope.external_event_id().clone(),
                route_kind,
                auth_claim: envelope.auth_claim().clone(),
            })
            .await?;
        let source_binding_id = product_source_binding_id(envelope, &binding);
        let submit_idempotency_key = submit_idempotency_key(envelope, &binding);
        let thread_scope = thread_scope_from_binding(&binding, route_kind)?;
        Ok(PreparedUserMessage {
            binding,
            thread_scope,
            source_binding_id,
            submit_idempotency_key,
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
                actor_id: prepared.binding.user_id.as_str().to_string(),
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
        )
        .await
        .map(Some)
    }

    async fn accept_prepared_user_message(
        &self,
        prepared: PreparedUserMessage,
        envelope: &ProductInboundEnvelope,
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
                created_by_actor_id: prepared.binding.user_id.as_str().to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .map_err(|e| ProductWorkflowError::Transient {
                reason: format!("failed to ensure thread: {e}"),
            })?;

        let reply_target_binding_id = prepared.source_binding_id.clone();
        let accepted = self
            .thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: prepared.thread_scope.clone(),
                thread_id: prepared.binding.thread_id.clone(),
                actor_id: prepared.binding.user_id.as_str().to_string(),
                source_binding_id: Some(prepared.source_binding_id.clone()),
                reply_target_binding_id: Some(reply_target_binding_id.clone()),
                external_event_id: Some(envelope.external_event_id().as_str().to_string()),
                content: MessageContent::text(payload.text.clone()),
            })
            .await
            .map_err(|e| ProductWorkflowError::Transient {
                reason: format!("failed to accept inbound message: {e}"),
            })?;

        ProductInboundTurnHandoff::NeedsSubmission(AcceptedProductInboundTurn {
            binding: prepared.binding,
            thread_scope: prepared.thread_scope,
            message_id: accepted.message_id,
            source_binding_id: prepared.source_binding_id,
            reply_target_binding_id,
            idempotency_key_raw: prepared.submit_idempotency_key,
            received_at: envelope.received_at(),
        })
        .submit_or_replay(&self.thread_service, &self.turn_coordinator)
        .await
    }
}

fn route_kind_for_user_message(
    trigger: ironclaw_product_adapters::ProductTriggerReason,
) -> ProductConversationRouteKind {
    match trigger {
        ironclaw_product_adapters::ProductTriggerReason::DirectChat => {
            ProductConversationRouteKind::Direct
        }
        ironclaw_product_adapters::ProductTriggerReason::BotMention
        | ironclaw_product_adapters::ProductTriggerReason::ReplyToBot
        | ironclaw_product_adapters::ProductTriggerReason::BotCommand
        | ironclaw_product_adapters::ProductTriggerReason::LinkedThreadAction => {
            ProductConversationRouteKind::Shared
        }
    }
}

async fn submit_or_replay_accepted_message<T, C>(
    thread_service: &T,
    turn_coordinator: &C,
    replay: AcceptedInboundMessageReplay,
    submit_idempotency_key: String,
    received_at: DateTime<Utc>,
) -> Result<InboundTurnOutcome, ProductWorkflowError>
where
    T: SessionThreadService,
    C: TurnCoordinator,
{
    ProductInboundTurnHandoff::from_replay(replay, submit_idempotency_key, received_at)?
        .submit_or_replay(thread_service, turn_coordinator)
        .await
}

enum ProductInboundTurnHandoff {
    AlreadySubmitted {
        accepted_message_ref: AcceptedMessageRef,
        submitted_run_id: TurnRunId,
        binding: ResolvedBinding,
    },
    NeedsSubmission(AcceptedProductInboundTurn),
}

impl ProductInboundTurnHandoff {
    fn from_replay(
        replay: AcceptedInboundMessageReplay,
        submit_idempotency_key: String,
        received_at: DateTime<Utc>,
    ) -> Result<Self, ProductWorkflowError> {
        let binding = binding_from_replay(&replay)?;
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

        let source_binding_id = replay.source_binding_id.clone().ok_or_else(|| {
            ProductWorkflowError::TurnSubmissionRejected {
                reason: "accepted replay missing source_binding_id".into(),
            }
        })?;
        let reply_target_binding_id = replay.reply_target_binding_id.clone().ok_or_else(|| {
            ProductWorkflowError::TurnSubmissionRejected {
                reason: "accepted replay missing reply_target_binding_id".into(),
            }
        })?;

        Ok(Self::NeedsSubmission(AcceptedProductInboundTurn {
            binding,
            thread_scope: replay.scope,
            message_id: replay.message_id,
            source_binding_id,
            reply_target_binding_id,
            idempotency_key_raw: submit_idempotency_key,
            received_at,
        }))
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
    source_binding_id: String,
    reply_target_binding_id: String,
    idempotency_key_raw: String,
    received_at: DateTime<Utc>,
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
            source_binding_id,
            reply_target_binding_id,
            idempotency_key_raw,
            received_at,
        } = self;
        let turn_scope = TurnScope::new(
            binding.tenant_id.clone(),
            binding.agent_id.clone(),
            binding.project_id.clone(),
            binding.thread_id.clone(),
        );
        let actor = TurnActor::new(binding.user_id.clone());
        let source_binding_ref = bounded_source_binding_ref(
            "src",
            &source_binding_id,
            DEFAULT_BINDING_REF_RAW_MAX_BYTES,
        )
        .map_err(|e| ProductWorkflowError::TurnSubmissionRejected {
            reason: format!("invalid src ref: {e}"),
        })?;
        let accepted_message_ref = accepted_message_ref(message_id)?;
        let reply_target_binding_ref = bounded_reply_target_binding_ref(
            "reply",
            &reply_target_binding_id,
            DEFAULT_BINDING_REF_RAW_MAX_BYTES,
        )
        .map_err(|e| ProductWorkflowError::TurnSubmissionRejected {
            reason: format!("invalid reply ref: {e}"),
        })?;
        let idempotency_key = bounded_idempotency_key(
            "turn",
            &idempotency_key_raw,
            DEFAULT_BINDING_REF_RAW_MAX_BYTES,
        )
        .map_err(|e| ProductWorkflowError::TurnSubmissionRejected {
            reason: format!("invalid turn ref: {e}"),
        })?;

        let request = SubmitTurnRequest {
            scope: turn_scope,
            actor,
            accepted_message_ref: accepted_message_ref.clone(),
            source_binding_ref,
            reply_target_binding_ref,
            requested_run_profile: None,
            idempotency_key,
            received_at,
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
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
                    .mark_message_deferred_busy(&thread_scope, &binding.thread_id, message_id)
                    .await
                    .map_err(|e| ProductWorkflowError::Transient {
                        reason: format!("failed to mark message deferred: {e}"),
                    })?;
                Ok(InboundTurnOutcome::DeferredBusy {
                    accepted_message_ref,
                    active_run_id: busy.active_run_id,
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

fn binding_from_replay(
    replay: &AcceptedInboundMessageReplay,
) -> Result<ResolvedBinding, ProductWorkflowError> {
    let user_id = match replay.scope.owner_user_id.clone() {
        Some(user_id) => user_id,
        None => UserId::new(replay.actor_id.as_deref().ok_or_else(|| {
            ProductWorkflowError::BindingResolutionFailed {
                reason: "accepted replay missing user id".into(),
            }
        })?)
        .map_err(|e| ProductWorkflowError::BindingResolutionFailed {
            reason: format!("invalid replay user id: {e}"),
        })?,
    };
    Ok(ResolvedBinding {
        tenant_id: replay.scope.tenant_id.clone(),
        user_id,
        thread_id: replay.thread_id.clone(),
        agent_id: Some(replay.scope.agent_id.clone()),
        project_id: replay.scope.project_id.clone(),
    })
}

fn thread_scope_from_binding(
    binding: &ResolvedBinding,
    route_kind: ProductConversationRouteKind,
) -> Result<ThreadScope, ProductWorkflowError> {
    let Some(agent_id) = binding.agent_id.clone() else {
        return Err(ProductWorkflowError::BindingResolutionFailed {
            reason: "resolved binding missing agent_id required for thread scope".into(),
        });
    };
    let owner_user_id = match route_kind {
        ProductConversationRouteKind::Direct => Some(binding.user_id.clone()),
        ProductConversationRouteKind::Shared => None,
    };
    Ok(ThreadScope {
        tenant_id: binding.tenant_id.clone(),
        agent_id,
        project_id: binding.project_id.clone(),
        owner_user_id,
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
    use std::future::pending;

    use async_trait::async_trait;
    use chrono::TimeZone;
    use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
    use ironclaw_product_adapters::{
        AdapterInstallationId, ExternalActorRef, ExternalConversationRef, ProductAdapterId,
        ProductTriggerReason, UserMessagePayload,
    };
    use ironclaw_threads::ThreadScope;

    use crate::action::SourceBindingKey;

    use super::*;

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
    fn deferred_replay_becomes_needs_submission_handoff() {
        let message_id = ThreadMessageId::new();
        let handoff = ProductInboundTurnHandoff::from_replay(
            replay(
                message_id,
                MessageStatus::DeferredBusy,
                Some("src:alpha"),
                Some("reply:alpha"),
                None,
            ),
            "turn-key".to_string(),
            received_at(),
        )
        .expect("deferred replay handoff");

        let ProductInboundTurnHandoff::NeedsSubmission(submission) = handoff else {
            panic!("expected deferred replay to require a new turn submission")
        };

        assert_eq!(submission.message_id, message_id);
        assert_eq!(submission.source_binding_id, "src:alpha");
        assert_eq!(submission.reply_target_binding_id, "reply:alpha");
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
            actor_id: None,
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
}
