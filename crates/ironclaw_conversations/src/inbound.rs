use std::sync::Arc;

use ironclaw_turns::{IdempotencyKey, SubmitTurnRequest, TurnCoordinator};

use crate::{
    AcceptInboundMessageRequest, ConversationBindingService, InboundTurnError, InboundTurnRequest,
    InboundTurnResponse, MessageIdempotencyStatus, ResolveConversationRequest,
    SessionThreadService,
};

#[derive(Clone)]
pub struct InboundTurnService<B, S, C> {
    binding_service: B,
    session_thread_service: S,
    turn_coordinator: Arc<C>,
}

impl<B, S, C> InboundTurnService<B, S, C>
where
    B: ConversationBindingService,
    S: SessionThreadService,
    C: TurnCoordinator,
{
    pub fn new(binding_service: B, session_thread_service: S, turn_coordinator: Arc<C>) -> Self {
        Self {
            binding_service,
            session_thread_service,
            turn_coordinator,
        }
    }

    pub async fn handle_inbound_turn(
        &self,
        request: InboundTurnRequest,
    ) -> Result<InboundTurnResponse, InboundTurnError> {
        let resolution = self
            .binding_service
            .resolve_or_create_binding(ResolveConversationRequest {
                tenant_id: request.tenant_id.clone(),
                adapter_kind: request.adapter_kind,
                adapter_installation_id: request.adapter_installation_id,
                external_actor_ref: request.external_actor_ref,
                external_conversation_ref: request.external_conversation_ref,
                external_event_id: request.external_event_id.clone(),
                requested_agent_id: request.requested_agent_id,
                requested_project_id: request.requested_project_id,
            })
            .await?;
        let accepted_message = self
            .session_thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                tenant_id: resolution.tenant_id.clone(),
                thread_id: resolution.turn_scope.thread_id.clone(),
                actor: resolution.actor.clone(),
                source_binding_ref: resolution.source_binding_ref.clone(),
                reply_target_binding_ref: resolution.reply_target_binding_ref.clone(),
                external_event_id: request.external_event_id,
                content_ref: request.content_ref,
                received_at: request.received_at,
            })
            .await?;

        if accepted_message.idempotency == MessageIdempotencyStatus::Duplicate
            && self
                .session_thread_service
                .inbound_message_turn_submitted(&accepted_message.message_ref)
                .await?
        {
            return Ok(InboundTurnResponse {
                resolution,
                accepted_message,
                turn_submission: None,
            });
        }

        let idempotency_key =
            IdempotencyKey::new(accepted_message.message_ref.as_str().to_string())
                .map_err(|reason| InboundTurnError::InvalidCanonicalRef { reason })?;
        let turn_submission = self
            .turn_coordinator
            .submit_turn(SubmitTurnRequest {
                scope: resolution.turn_scope.clone(),
                actor: resolution.actor.clone(),
                accepted_message_ref: accepted_message.message_ref.clone(),
                source_binding_ref: accepted_message.source_binding_ref.clone(),
                reply_target_binding_ref: accepted_message.reply_target_binding_ref.clone(),
                requested_run_profile: request.requested_run_profile,
                idempotency_key,
                received_at: request.received_at,
            })
            .await
            .map_err(|error| InboundTurnError::TurnSubmissionFailed {
                reason: error.to_string(),
            })?;
        self.session_thread_service
            .mark_inbound_message_turn_submitted(&accepted_message.message_ref)
            .await?;

        Ok(InboundTurnResponse {
            resolution,
            accepted_message,
            turn_submission: Some(turn_submission),
        })
    }
}
