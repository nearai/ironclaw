use std::sync::Arc;

use ironclaw_turns::{AdmissionRejectionReason, SubmitTurnRequest, TurnCoordinator, TurnError};

use crate::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageLookup,
    ConversationBindingResolution, ConversationBindingService, InboundTurnError,
    InboundTurnRequest, InboundTurnResponse, MessageIdempotencyStatus, ResolveConversationRequest,
    SessionThreadService, TrustedInboundTurnRequest,
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
        self.handle_inbound_turn_inner(request, BindingResolutionPolicy::Untrusted)
            .await
    }

    pub async fn handle_inbound_turn_with_trusted_scope(
        &self,
        request: TrustedInboundTurnRequest,
    ) -> Result<InboundTurnResponse, InboundTurnError> {
        let (request, trusted_agent_id, trusted_project_id) = request.into_parts();
        self.handle_inbound_turn_inner(
            request,
            BindingResolutionPolicy::Trusted {
                trusted_agent_id,
                trusted_project_id,
            },
        )
        .await
    }

    async fn handle_inbound_turn_inner(
        &self,
        request: InboundTurnRequest,
        binding_policy: BindingResolutionPolicy,
    ) -> Result<InboundTurnResponse, InboundTurnError> {
        let InboundTurnRequest {
            tenant_id,
            adapter_kind,
            adapter_installation_id,
            external_actor_ref,
            external_conversation_ref,
            external_event_id,
            route_kind,
            content_ref,
            requested_agent_id,
            requested_project_id,
            received_at,
            requested_run_profile,
        } = request;

        let replay_lookup = AcceptedInboundMessageLookup {
            tenant_id: tenant_id.clone(),
            adapter_kind: adapter_kind.clone(),
            adapter_installation_id: adapter_installation_id.clone(),
            external_actor_ref: external_actor_ref.clone(),
            external_conversation_ref: external_conversation_ref.clone(),
            external_event_id: external_event_id.clone(),
        };
        if let Some(replay) = self
            .session_thread_service
            .replay_accepted_inbound_message(replay_lookup)
            .await?
        {
            return self
                .submit_or_replay(replay.resolution, replay.accepted_message)
                .await;
        }

        let (requested_agent_id, requested_project_id) =
            binding_policy.requested_scope(requested_agent_id, requested_project_id);
        let resolve_request = ResolveConversationRequest {
            tenant_id: tenant_id.clone(),
            adapter_kind: adapter_kind.clone(),
            adapter_installation_id: adapter_installation_id.clone(),
            external_actor_ref: external_actor_ref.clone(),
            external_conversation_ref: external_conversation_ref.clone(),
            external_event_id: external_event_id.clone(),
            route_kind,
            requested_agent_id,
            requested_project_id,
        };
        let resolution = match binding_policy {
            BindingResolutionPolicy::Untrusted => {
                self.binding_service
                    .resolve_or_create_binding(resolve_request)
                    .await?
            }
            BindingResolutionPolicy::Trusted {
                trusted_agent_id,
                trusted_project_id,
            } => {
                self.binding_service
                    .resolve_or_create_binding_with_trusted_scope(
                        resolve_request,
                        trusted_agent_id,
                        trusted_project_id,
                    )
                    .await?
            }
        };
        let accepted_message = self
            .session_thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                tenant_id: resolution.tenant_id.clone(),
                thread_id: resolution.turn_scope.thread_id.clone(),
                actor: resolution.actor.clone(),
                adapter_kind,
                adapter_installation_id,
                external_actor_ref,
                source_binding_ref: resolution.source_binding_ref.clone(),
                reply_target_binding_ref: resolution.reply_target_binding_ref.clone(),
                external_conversation_ref,
                external_event_id,
                route_kind,
                content_ref,
                received_at,
                requested_run_profile,
            })
            .await?;

        self.submit_or_replay(resolution, accepted_message).await
    }

    async fn submit_or_replay(
        &self,
        mut resolution: ConversationBindingResolution,
        accepted_message: AcceptedInboundMessage,
    ) -> Result<InboundTurnResponse, InboundTurnError> {
        resolution.actor = accepted_message.actor.clone();

        if accepted_message.idempotency == MessageIdempotencyStatus::Duplicate
            && let Some(turn_submission) = self
                .session_thread_service
                .inbound_message_turn_submission(&accepted_message.message_ref)
                .await?
        {
            return Ok(InboundTurnResponse {
                resolution,
                accepted_message,
                turn_submission: Some(turn_submission),
            });
        }

        let idempotency_key = self
            .session_thread_service
            .inbound_message_turn_submission_key(&accepted_message.message_ref)
            .await?;
        let turn_submission_result = self
            .turn_coordinator
            .submit_turn(SubmitTurnRequest {
                scope: resolution.turn_scope.clone(),
                actor: accepted_message.actor.clone(),
                accepted_message_ref: accepted_message.message_ref.clone(),
                source_binding_ref: accepted_message.source_binding_ref.clone(),
                reply_target_binding_ref: accepted_message.reply_target_binding_ref.clone(),
                requested_run_profile: accepted_message.requested_run_profile.clone(),
                idempotency_key,
                received_at: accepted_message.received_at,
                requested_run_id: None,
                parent_run_id: None,
                subagent_depth: 0,
                spawn_tree_root_run_id: None,
            })
            .await;
        let turn_submission = match turn_submission_result {
            Ok(response) => response,
            Err(error) => {
                if should_rotate_submit_key(&error) {
                    self.session_thread_service
                        .rotate_inbound_message_turn_submission_key(&accepted_message.message_ref)
                        .await?;
                }
                return Err(InboundTurnError::TurnSubmissionFailed { error });
            }
        };
        self.session_thread_service
            .mark_inbound_message_turn_submitted(
                &accepted_message.message_ref,
                turn_submission.clone(),
            )
            .await?;

        Ok(InboundTurnResponse {
            resolution,
            accepted_message,
            turn_submission: Some(turn_submission),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BindingResolutionPolicy {
    Untrusted,
    Trusted {
        trusted_agent_id: Option<ironclaw_host_api::AgentId>,
        trusted_project_id: Option<ironclaw_host_api::ProjectId>,
    },
}

impl BindingResolutionPolicy {
    fn requested_scope(
        &self,
        requested_agent_id: Option<ironclaw_host_api::AgentId>,
        requested_project_id: Option<ironclaw_host_api::ProjectId>,
    ) -> (
        Option<ironclaw_host_api::AgentId>,
        Option<ironclaw_host_api::ProjectId>,
    ) {
        match self {
            Self::Untrusted => (requested_agent_id, requested_project_id),
            Self::Trusted { .. } => (None, None),
        }
    }
}

fn should_rotate_submit_key(error: &TurnError) -> bool {
    match error {
        TurnError::ThreadBusy(_) | TurnError::Unavailable { .. } => true,
        TurnError::AdmissionRejected(rejection) => matches!(
            rejection.reason,
            AdmissionRejectionReason::TenantLimit | AdmissionRejectionReason::Unavailable
        ),
        TurnError::ScopeNotFound
        | TurnError::Unauthorized
        | TurnError::InvalidRequest { .. }
        | TurnError::CapacityExceeded { .. }
        | TurnError::Conflict { .. }
        | TurnError::InvalidTransition { .. }
        | TurnError::LeaseMismatch => false,
    }
}
