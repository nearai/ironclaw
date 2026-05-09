//! InboundTurnService — the user-message turn submission path.
//!
//! This is the narrower user-message subset of [`ProductWorkflow`]. It
//! resolves the conversation binding, accepts the inbound message into the
//! session thread, and submits the turn to the coordinator.

use async_trait::async_trait;
use ironclaw_product_adapters::{ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, MessageContent, SessionThreadService,
    ThreadScope,
};
use ironclaw_turns::{AcceptedMessageRef, TurnError, TurnErrorCategory, TurnRunId};
use ironclaw_turns::{
    IdempotencyKey, ReplyTargetBindingRef, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse,
    TurnActor, TurnCoordinator, TurnScope,
};
use uuid::Uuid;

use crate::binding::{ConversationBindingService, ResolveBindingRequest, ResolvedBinding};
use crate::error::ProductWorkflowError;

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

/// Port for the inbound turn submission path.
///
/// Implementations coordinate binding resolution, message acceptance into the
/// session thread service, and turn submission to the coordinator.
#[async_trait]
pub trait InboundTurnService: Send + Sync {
    /// Accept a user message envelope: resolve binding, stage message, submit turn.
    async fn accept_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<InboundTurnOutcome, ProductWorkflowError>;
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
    async fn accept_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<InboundTurnOutcome, ProductWorkflowError> {
        let binding = self
            .binding_service
            .resolve_binding(ResolveBindingRequest {
                adapter_id: envelope.adapter_id().clone(),
                installation_id: envelope.installation_id().clone(),
                external_actor_ref: envelope.external_actor_ref().clone(),
                external_conversation_ref: envelope.external_conversation_ref().clone(),
                auth_claim: envelope.auth_claim().clone(),
            })
            .await?;

        let thread_scope = thread_scope_from_binding(&binding)?;
        self.thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(binding.thread_id.clone()),
                created_by_actor_id: binding.user_id.as_str().to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .map_err(|e| ProductWorkflowError::Transient {
                reason: format!("failed to ensure thread: {e}"),
            })?;

        let ProductInboundPayload::UserMessage(payload) = envelope.payload() else {
            return Err(ProductWorkflowError::UnsupportedActionKind {
                kind: "non_user_message".into(),
            });
        };
        let source_binding_id = envelope.source_binding_key();
        let reply_target_binding_id = format!("reply:{source_binding_id}");
        let accepted = self
            .thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: thread_scope.clone(),
                thread_id: binding.thread_id.clone(),
                actor_id: binding.user_id.as_str().to_string(),
                source_binding_id: Some(source_binding_id.clone()),
                reply_target_binding_id: Some(reply_target_binding_id.clone()),
                external_event_id: Some(envelope.external_event_id().as_str().to_string()),
                content: MessageContent::text(payload.text.clone()),
            })
            .await
            .map_err(|e| ProductWorkflowError::Transient {
                reason: format!("failed to accept inbound message: {e}"),
            })?;

        let turn_scope = TurnScope::new(
            binding.tenant_id.clone(),
            binding.agent_id.clone(),
            binding.project_id.clone(),
            binding.thread_id.clone(),
        );
        let actor = TurnActor::new(binding.user_id.clone());
        let source_binding_ref = bounded_ref::<SourceBindingRef>("src", &source_binding_id)?;
        let accepted_message_ref = AcceptedMessageRef::new(format!("msg:{}", accepted.message_id))
            .map_err(|e| ProductWorkflowError::TurnSubmissionRejected {
                reason: format!("invalid accepted message ref: {e}"),
            })?;
        let reply_target_binding_ref =
            bounded_ref::<ReplyTargetBindingRef>("reply", &reply_target_binding_id)?;
        let idempotency_key = bounded_ref::<IdempotencyKey>(
            "turn",
            &format!(
                "{}:{}:{}",
                envelope.adapter_id(),
                envelope.installation_id(),
                envelope.external_event_id()
            ),
        )?;

        let request = SubmitTurnRequest {
            scope: turn_scope,
            actor,
            accepted_message_ref: accepted_message_ref.clone(),
            source_binding_ref,
            reply_target_binding_ref,
            requested_run_profile: None,
            idempotency_key,
            received_at: envelope.received_at(),
        };

        match self.turn_coordinator.submit_turn(request).await {
            Ok(SubmitTurnResponse::Accepted {
                turn_id, run_id, ..
            }) => {
                self.thread_service
                    .mark_message_submitted(
                        &thread_scope,
                        &binding.thread_id,
                        accepted.message_id,
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
                self.thread_service
                    .mark_message_deferred_busy(
                        &thread_scope,
                        &binding.thread_id,
                        accepted.message_id,
                    )
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
            Err(error) if error.category() == TurnErrorCategory::Unavailable => {
                Err(ProductWorkflowError::Transient {
                    reason: error.to_string(),
                })
            }
            Err(error) => Err(ProductWorkflowError::TurnSubmissionRejected {
                reason: error.to_string(),
            }),
        }
    }
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
        owner_user_id: Some(binding.user_id.clone()),
        mission_id: None,
    })
}

trait RefFactory: Sized {
    fn build(value: String) -> Result<Self, String>;
}

impl RefFactory for SourceBindingRef {
    fn build(value: String) -> Result<Self, String> {
        Self::new(value)
    }
}

impl RefFactory for ReplyTargetBindingRef {
    fn build(value: String) -> Result<Self, String> {
        Self::new(value)
    }
}

impl RefFactory for IdempotencyKey {
    fn build(value: String) -> Result<Self, String> {
        Self::new(value)
    }
}

fn bounded_ref<T: RefFactory>(prefix: &str, raw: &str) -> Result<T, ProductWorkflowError> {
    let value = if raw.len() <= 240 && !raw.chars().any(|c| c == '\0' || c.is_control()) {
        format!("{prefix}:{raw}")
    } else {
        let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, raw.as_bytes());
        format!("{prefix}:{id}")
    };
    T::build(value).map_err(|e| ProductWorkflowError::TurnSubmissionRejected {
        reason: format!("invalid {prefix} ref: {e}"),
    })
}
