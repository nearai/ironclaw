use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_safety::{
    InjectionScanner, PromptSafetyRejection, Sanitizer, validate_trusted_trigger_prompt,
};
use ironclaw_triggers::{
    TriggerError, TrustedTriggerFireSubmitOutcome, TrustedTriggerFireSubmitter,
    TrustedTriggerSubmitRequest,
};
use ironclaw_turns::{
    AdmissionRejectionReason, RunOriginAdapter, SubmitTurnRequest, TurnCoordinator, TurnError,
    TurnSurfaceType,
};

use crate::trusted_trigger::{TrustedTriggerInboundFailureKind, classify_inbound_error};
use crate::types::{TrustedInboundKind, TrustedInboundTurnRequest};
use crate::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageLookup,
    AdapterInstallationId, AdapterKind, ConversationBindingResolution, ConversationBindingService,
    ConversationRouteKind, ExternalActorRef, ExternalConversationRef, ExternalEventId,
    InboundMessageContentRef, InboundTurnError, InboundTurnRequest, InboundTurnResponse,
    MessageIdempotencyStatus, ResolveConversationRequest, SessionThreadService,
};

#[derive(Clone)]
pub struct InboundTurnService<B, S, C: ?Sized> {
    binding_service: B,
    session_thread_service: S,
    turn_coordinator: Arc<C>,
}

impl<B, S, C> InboundTurnService<B, S, C>
where
    B: ConversationBindingService,
    S: SessionThreadService,
    C: TurnCoordinator + ?Sized,
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

    async fn handle_inbound_turn_with_trusted_scope(
        &self,
        request: TrustedInboundTurnRequest,
    ) -> Result<InboundTurnResponse, InboundTurnError> {
        let TrustedInboundTurnRequest {
            request,
            trusted_agent_id,
            trusted_project_id,
            trusted_owner_user_id,
            kind,
        } = request;
        self.handle_inbound_turn_inner(
            request,
            BindingResolutionPolicy::Trusted {
                trusted_agent_id,
                trusted_project_id,
                trusted_owner_user_id,
                kind,
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

        // Origin classification is derived from the typed trust policy, never
        // re-derived from the adapter-kind string. `TrustedTrigger` is reachable
        // only when the trusted-trigger submit seam built this request with
        // `TrustedInboundKind::Trigger`; see `.claude/rules/types.md`.
        let classification = match &binding_policy {
            BindingResolutionPolicy::Trusted {
                kind: TrustedInboundKind::Trigger,
                ..
            } => ironclaw_product_context::InboundClassification::TrustedTrigger,
            BindingResolutionPolicy::Trusted { .. } => {
                ironclaw_product_context::InboundClassification::TrustedOther
            }
            BindingResolutionPolicy::Untrusted => {
                ironclaw_product_context::InboundClassification::Untrusted
            }
        };
        let surface_type = match &route_kind {
            ConversationRouteKind::Direct => Some(TurnSurfaceType::Direct),
            ConversationRouteKind::Shared => Some(TurnSurfaceType::Channel),
        };
        let run_adapter = RunOriginAdapter::new(adapter_kind.as_str()).map_err(|e| {
            InboundTurnError::InvalidCanonicalRef {
                reason: e.to_string(),
            }
        })?;

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
                .submit_or_replay(
                    replay.resolution,
                    replay.accepted_message,
                    classification,
                    run_adapter,
                    surface_type,
                )
                .await;
        }

        let (requested_agent_id, requested_project_id) = match &binding_policy {
            BindingResolutionPolicy::Untrusted => (requested_agent_id, requested_project_id),
            BindingResolutionPolicy::Trusted { .. } => (None, None),
        };
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
                trusted_owner_user_id,
                kind: _,
            } => {
                self.binding_service
                    .resolve_or_create_binding_with_trusted_scope(
                        resolve_request,
                        trusted_agent_id,
                        trusted_project_id,
                        trusted_owner_user_id,
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

        self.submit_or_replay(
            resolution,
            accepted_message,
            classification,
            run_adapter,
            surface_type,
        )
        .await
    }

    async fn submit_or_replay(
        &self,
        mut resolution: ConversationBindingResolution,
        accepted_message: AcceptedInboundMessage,
        classification: ironclaw_product_context::InboundClassification,
        run_adapter: RunOriginAdapter,
        surface_type: Option<TurnSurfaceType>,
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
                replayed_turn_submission: true,
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
                product_context: Some(ironclaw_product_context::resolve_inbound(
                    classification,
                    run_adapter,
                    surface_type,
                    resolution.turn_scope.product_owner(&accepted_message.actor),
                )),
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
            replayed_turn_submission: false,
        })
    }
}

#[derive(Clone)]
pub(crate) struct ConversationTrustedTriggerSubmitter<B, S, C: ?Sized> {
    inbound: InboundTurnService<B, S, C>,
    prompt_safety: Arc<dyn InjectionScanner>,
}

impl<B, S, C> ConversationTrustedTriggerSubmitter<B, S, C>
where
    B: ConversationBindingService,
    S: SessionThreadService,
    C: TurnCoordinator + ?Sized,
{
    pub(crate) fn new(
        binding_service: B,
        session_thread_service: S,
        turn_coordinator: Arc<C>,
    ) -> Self {
        Self {
            inbound: InboundTurnService::new(
                binding_service,
                session_thread_service,
                turn_coordinator,
            ),
            prompt_safety: Arc::new(Sanitizer::new()),
        }
    }
}

/// Build the conversation-owned submitter used by host composition for trusted
/// trigger fires.
///
/// This factory only wires the submitter. Trusted authority lives in the sealed
/// `TrustedTriggerSubmitRequest`, whose constructor is owned by the trigger
/// worker, not in this public function.
pub fn trusted_trigger_fire_submitter<B, S, C>(
    binding_service: B,
    session_thread_service: S,
    turn_coordinator: Arc<C>,
) -> Arc<dyn TrustedTriggerFireSubmitter>
where
    B: ConversationBindingService + 'static,
    S: SessionThreadService + 'static,
    C: TurnCoordinator + ?Sized + 'static,
{
    Arc::new(ConversationTrustedTriggerSubmitter::new(
        binding_service,
        session_thread_service,
        turn_coordinator,
    ))
}

#[async_trait]
impl<B, S, C> TrustedTriggerFireSubmitter for ConversationTrustedTriggerSubmitter<B, S, C>
where
    B: ConversationBindingService,
    S: SessionThreadService,
    C: TurnCoordinator + ?Sized,
{
    async fn submit_trusted_trigger_fire(
        &self,
        request: TrustedTriggerSubmitRequest,
    ) -> Result<TrustedTriggerFireSubmitOutcome, TriggerError> {
        let submitted_at = request.received_at();
        // Defense in depth: composition scans before materializing/recording the
        // prompt, and conversations scans again at the final trusted submission
        // boundary before converting into the private trusted inbound request.
        validate_trusted_trigger_prompt(&*self.prompt_safety, &request.fire().prompt)
            .map_err(trigger_prompt_safety_rejection)?;
        let response = self
            .inbound
            .handle_inbound_turn_with_trusted_scope(
                trusted_inbound_request_from_trigger(request)
                    .map_err(classify_trusted_trigger_inbound_error)?,
            )
            .await
            .map_err(classify_trusted_trigger_inbound_error)?;
        submit_trusted_trigger_outcome(&response, submitted_at)
    }
}

fn trusted_inbound_request_from_trigger(
    request: TrustedTriggerSubmitRequest,
) -> Result<TrustedInboundTurnRequest, InboundTurnError> {
    let (fire, materialized_prompt, received_at) = request.into_parts();
    let (content_ref, trusted_inbound_binding) = materialized_prompt.into_parts();
    Ok(TrustedInboundTurnRequest::new(
        InboundTurnRequest {
            tenant_id: fire.identity.tenant_id().clone(),
            adapter_kind: AdapterKind::new(trusted_inbound_binding.adapter_kind())?,
            adapter_installation_id: AdapterInstallationId::new(
                trusted_inbound_binding.adapter_installation_id(),
            )?,
            external_actor_ref: ExternalActorRef::new(
                trusted_inbound_binding.external_actor_namespace(),
                trusted_inbound_binding.external_actor_id(),
            )?,
            external_conversation_ref: ExternalConversationRef::new(
                None,
                trusted_inbound_binding.external_conversation_id(),
                Some(trusted_inbound_binding.route_thread_id()),
                None,
            )?,
            external_event_id: ExternalEventId::new(trusted_inbound_binding.external_event_id())?,
            route_kind: ConversationRouteKind::Direct,
            content_ref: InboundMessageContentRef::new(content_ref.as_str())?,
            requested_agent_id: None,
            requested_project_id: None,
            received_at,
            requested_run_profile: None,
        },
        fire.agent_id,
        fire.project_id,
        Some(fire.creator_user_id),
        TrustedInboundKind::Trigger,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BindingResolutionPolicy {
    Untrusted,
    Trusted {
        trusted_agent_id: Option<ironclaw_host_api::AgentId>,
        trusted_project_id: Option<ironclaw_host_api::ProjectId>,
        trusted_owner_user_id: Option<ironclaw_host_api::UserId>,
        kind: TrustedInboundKind,
    },
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
        | TurnError::LeaseMismatch
        | TurnError::InvalidRunOriginAdapter => false,
    }
}

fn submit_trusted_trigger_outcome(
    response: &InboundTurnResponse,
    submitted_at: chrono::DateTime<chrono::Utc>,
) -> Result<TrustedTriggerFireSubmitOutcome, TriggerError> {
    let run_id = match &response.turn_submission {
        Some(ironclaw_turns::SubmitTurnResponse::Accepted { run_id, .. }) => *run_id,
        None => {
            return Err(TriggerError::Backend {
                reason: "trusted trigger fire accepted no turn submission".to_string(),
            });
        }
    };
    if response.replayed_turn_submission {
        return Ok(TrustedTriggerFireSubmitOutcome::Replayed {
            original_run_id: run_id,
            replayed_at: submitted_at,
            thread_id: Some(response.resolution.turn_scope.thread_id.clone()),
        });
    }
    Ok(TrustedTriggerFireSubmitOutcome::Accepted {
        run_id,
        submitted_at,
        turn_scope: response.resolution.turn_scope.clone(),
    })
}

fn trigger_prompt_safety_rejection(error: PromptSafetyRejection) -> TriggerError {
    TriggerError::InvalidMaterialization {
        reason: error.to_string(),
    }
}

/// Classify conversation inbound failures for the trusted trigger submit path.
///
/// This helper is private submitter policy. Composition classifies its own
/// materialization failures before it mints a sealed submit request.
fn classify_trusted_trigger_inbound_error(error: InboundTurnError) -> TriggerError {
    match classify_inbound_error(&error) {
        TrustedTriggerInboundFailureKind::RetryableBackend => {
            retryable_trusted_trigger_backend_error(&error)
        }
        TrustedTriggerInboundFailureKind::SubmitRejected => {
            opaque_trusted_trigger_inbound_rejection("trusted trigger submit rejected", &error)
        }
        TrustedTriggerInboundFailureKind::InboundRequestRejected => {
            opaque_trusted_trigger_inbound_rejection(
                "trusted trigger inbound request rejected",
                &error,
            )
        }
    }
}

fn retryable_trusted_trigger_backend_error(_error: &InboundTurnError) -> TriggerError {
    tracing::debug!("trusted trigger submit retryable failure");
    TriggerError::Backend {
        reason: "trusted trigger submit retryable failure".to_string(),
    }
}

fn opaque_trusted_trigger_inbound_rejection(
    reason: &'static str,
    _error: &InboundTurnError,
) -> TriggerError {
    tracing::debug!(reason, "trusted trigger inbound rejection");
    TriggerError::InvalidMaterialization {
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
    use ironclaw_triggers::{
        TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
        TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerFire, TriggerFireIdentity, TriggerId,
        TriggerInboundContentRef, TriggerMaterializedPrompt, TrustedTriggerFireSubmitOutcome,
        TrustedTriggerSubmitRequest,
    };
    use ironclaw_turns::{
        AcceptedMessageRef, AdmissionRejection, AdmissionRejectionReason, CancelRunRequest,
        CancelRunResponse, EventCursor, GetRunStateRequest, ReplyTargetBindingRef,
        ResumeTurnRequest, ResumeTurnResponse, RunProfileId, RunProfileVersion, SourceBindingRef,
        SubmitTurnRequest, SubmitTurnResponse, ThreadBusy, TurnCapacityResource, TurnCoordinator,
        TurnError, TurnId, TurnOriginKind, TurnRunId, TurnRunState, TurnScope, TurnStatus,
        TurnSurfaceType,
    };

    use super::{
        classify_trusted_trigger_inbound_error, submit_trusted_trigger_outcome,
        trusted_trigger_fire_submitter,
    };
    use crate::types::{TrustedInboundKind, TrustedInboundTurnRequest};
    use crate::{
        AcceptedInboundMessage, AdapterInstallationId, AdapterKind, ConversationBindingResolution,
        ConversationBindingService, ConversationRouteKind, ExternalActorRef,
        ExternalConversationRef, ExternalEventId, InMemoryConversationServices,
        InboundMessageContentRef, InboundTurnError, InboundTurnRequest, InboundTurnResponse,
        InboundTurnService, LinkConversationRequest, LinkedConversationBinding,
        MessageIdempotencyStatus, ReplyTargetBinding, ThreadAccessDecision,
        ValidateReplyTargetRequest,
    };

    #[tokio::test]
    async fn trusted_inbound_with_real_services_creates_binding_records_message_and_replays_submission()
     {
        let (inbound, services, coordinator) = trusted_inbound_service().await;
        let request = trusted_inbound_request(Some(agent()), Some(project()));

        let first = inbound
            .handle_inbound_turn_with_trusted_scope(request.clone())
            .await
            .unwrap();
        let duplicate = inbound
            .handle_inbound_turn_with_trusted_scope(request)
            .await
            .unwrap();

        assert_eq!(first.resolution.turn_scope.agent_id, Some(agent()));
        assert_eq!(first.resolution.turn_scope.project_id, Some(project()));
        assert_eq!(
            first.accepted_message.idempotency,
            MessageIdempotencyStatus::Inserted
        );
        assert_eq!(duplicate.turn_submission, first.turn_submission);
        assert_eq!(
            duplicate.accepted_message.message_ref,
            first.accepted_message.message_ref
        );
        assert_eq!(
            duplicate.accepted_message.idempotency,
            MessageIdempotencyStatus::Duplicate
        );
        assert!(!first.replayed_turn_submission);
        assert!(duplicate.replayed_turn_submission);
        assert_eq!(services.accepted_messages().await.len(), 1);
        assert_eq!(coordinator.submissions().len(), 1);
        assert_eq!(
            coordinator.submissions()[0]
                .product_context
                .as_ref()
                .map(|c| c.origin),
            Some(TurnOriginKind::ScheduledTrigger)
        );
    }

    #[tokio::test]
    async fn trusted_inbound_uses_trusted_binding_resolution_and_replays_duplicate_submission() {
        let services = InMemoryConversationServices::default();
        services
            .pair_external_actor(
                tenant(),
                trigger_adapter(),
                trigger_installation(),
                external_actor("alice"),
                user("alice"),
            )
            .await;
        let binding = TrustedOnlyBindingService::new(services.clone());
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let inbound =
            InboundTurnService::new(binding.clone(), services.clone(), coordinator.clone());
        let request = trusted_inbound_request(Some(agent()), Some(project()));

        let first = inbound
            .handle_inbound_turn_with_trusted_scope(request.clone())
            .await
            .unwrap();
        let duplicate = inbound
            .handle_inbound_turn_with_trusted_scope(request)
            .await
            .unwrap();

        assert_eq!(binding.trusted_calls(), 1);
        assert_eq!(
            binding.trusted_scopes(),
            vec![(Some(agent()), Some(project()), None)]
        );
        let resolve_requests = binding.resolve_requests();
        assert_eq!(resolve_requests.len(), 1);
        assert_eq!(resolve_requests[0].requested_agent_id, None);
        assert_eq!(resolve_requests[0].requested_project_id, None);
        assert_eq!(coordinator.submissions().len(), 1);
        assert_eq!(duplicate.turn_submission, first.turn_submission);
        assert_eq!(
            duplicate.accepted_message.message_ref,
            first.accepted_message.message_ref
        );
        assert_eq!(
            duplicate.accepted_message.idempotency,
            MessageIdempotencyStatus::Duplicate
        );
        assert!(!first.replayed_turn_submission);
        assert!(duplicate.replayed_turn_submission);
    }

    #[tokio::test]
    async fn trusted_inbound_propagates_binding_resolution_failure_without_accepting_or_submitting()
    {
        let services = InMemoryConversationServices::default();
        let binding = RejectingTrustedBindingService::new();
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let inbound =
            InboundTurnService::new(binding.clone(), services.clone(), coordinator.clone());
        let request = trusted_inbound_request(Some(agent()), Some(project()));

        let err = inbound
            .handle_inbound_turn_with_trusted_scope(request)
            .await
            .unwrap_err();

        assert!(matches!(err, InboundTurnError::BindingRequired { .. }));
        assert_eq!(
            binding.trusted_scopes(),
            vec![(Some(agent()), Some(project()), None)]
        );
        let resolve_requests = binding.resolve_requests();
        assert_eq!(resolve_requests.len(), 1);
        assert_eq!(resolve_requests[0].requested_agent_id, None);
        assert_eq!(resolve_requests[0].requested_project_id, None);
        assert!(services.accepted_messages().await.is_empty());
    }

    #[tokio::test]
    async fn trusted_inbound_preserves_none_trusted_scope() {
        let services = InMemoryConversationServices::default();
        services
            .pair_external_actor(
                tenant(),
                trigger_adapter(),
                trigger_installation(),
                external_actor("alice"),
                user("alice"),
            )
            .await;
        let binding = TrustedOnlyBindingService::new(services.clone());
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let inbound =
            InboundTurnService::new(binding.clone(), services.clone(), coordinator.clone());
        let request = trusted_inbound_request(None, None);

        inbound
            .handle_inbound_turn_with_trusted_scope(request)
            .await
            .unwrap();

        assert_eq!(binding.trusted_scopes(), vec![(None, None, None)]);
        let resolve_requests = binding.resolve_requests();
        assert_eq!(resolve_requests.len(), 1);
        assert_eq!(resolve_requests[0].requested_agent_id, None);
        assert_eq!(resolve_requests[0].requested_project_id, None);
    }

    #[tokio::test]
    async fn trusted_inbound_with_owner_resolves_explicit_user_turn_scope() {
        let (facade, _services, _coordinator) = trusted_inbound_service().await;
        let creator = UserId::new("user-creator").expect("user id");

        let request = TrustedInboundTurnRequest::new(
            base_inbound_request(),
            Some(agent()),
            Some(project()),
            Some(creator.clone()),
            TrustedInboundKind::Trigger,
        );

        let response = facade
            .handle_inbound_turn_with_trusted_scope(request)
            .await
            .expect("trusted inbound turn succeeds");

        assert_eq!(
            response
                .resolution
                .turn_scope
                .explicit_owner_user_id()
                .map(|u| u.as_str()),
            Some("user-creator"),
            "trusted owner must surface as ExplicitUser on the resolved TurnScope"
        );
    }

    #[tokio::test]
    async fn trusted_inbound_does_not_backfill_owner_on_existing_direct_binding() {
        let (facade, _services, _coordinator) = trusted_inbound_service().await;
        let creator = UserId::new("user-creator").expect("user id");

        // First fire: no owner (legacy-shaped binding).
        let first = TrustedInboundTurnRequest::new(
            base_inbound_request(),
            Some(agent()),
            Some(project()),
            None,
            TrustedInboundKind::Trigger,
        );
        facade
            .handle_inbound_turn_with_trusted_scope(first)
            .await
            .expect("first trusted turn succeeds");

        // Second fire on the same external conversation: owner now supplied.
        // Must use a DIFFERENT external_event_id (same id replays the first
        // submission instead of re-resolving the binding).
        let mut second_request = base_inbound_request();
        second_request.external_event_id =
            ExternalEventId::new("trusted-event-2").expect("event id");
        let second = TrustedInboundTurnRequest::new(
            second_request,
            Some(agent()),
            Some(project()),
            Some(creator),
            TrustedInboundKind::Trigger,
        );
        let response = facade
            .handle_inbound_turn_with_trusted_scope(second)
            .await
            .expect("second trusted turn succeeds");

        assert_eq!(
            response.resolution.turn_scope.explicit_owner_user_id(),
            None,
            "Direct-route bindings must not retro-upgrade owner (legacy compat; recreate the trigger to fix delivery)"
        );
    }

    #[tokio::test]
    async fn submit_trusted_trigger_fire_surfaces_creator_as_explicit_turn_scope_owner() {
        let (_inbound, services, coordinator) = trusted_inbound_service().await;

        // Pair the trigger creator so the trusted binding resolution succeeds.
        let creator = UserId::new("user-trigger-creator").expect("user id");
        services
            .pair_external_actor(
                tenant(),
                trigger_adapter(),
                trigger_installation(),
                external_actor(creator.as_str()),
                creator.clone(),
            )
            .await;

        let submitter = trusted_trigger_fire_submitter(services.clone(), services, coordinator);

        let fire_slot = Utc.with_ymd_and_hms(2026, 6, 1, 9, 0, 0).unwrap();
        let identity = TriggerFireIdentity::new(tenant(), TriggerId::new(), fire_slot);
        let fire = TriggerFire {
            identity: identity.clone(),
            creator_user_id: creator.clone(),
            agent_id: Some(agent()),
            project_id: Some(project()),
            prompt: "test trigger prompt".to_string(),
        };
        let content_ref =
            TriggerInboundContentRef::new("content:test-trigger-creator").expect("content ref");
        let materialized_prompt = TriggerMaterializedPrompt::for_fire(&fire, content_ref);
        let request =
            TrustedTriggerSubmitRequest::new_for_test(fire, materialized_prompt, fire_slot);

        let outcome = submitter
            .submit_trusted_trigger_fire(request)
            .await
            .expect("submit_trusted_trigger_fire succeeds");

        let TrustedTriggerFireSubmitOutcome::Accepted { turn_scope, .. } = outcome else {
            panic!("expected accepted trigger fire");
        };
        assert_eq!(
            turn_scope.explicit_owner_user_id(),
            Some(&creator),
            "submit_trusted_trigger_fire must surface the creator as explicit turn-scope owner"
        );
    }

    #[test]
    fn submit_trusted_trigger_outcome_preserves_received_at_for_accepted_and_replayed_fires() {
        let submitted_at = Utc.with_ymd_and_hms(2026, 5, 6, 12, 30, 0).unwrap();
        let run_id = TurnRunId::new();

        let accepted = trusted_trigger_response(run_id, MessageIdempotencyStatus::Inserted, false);
        let accepted_outcome = submit_trusted_trigger_outcome(&accepted, submitted_at).unwrap();
        assert!(matches!(
            accepted_outcome,
            TrustedTriggerFireSubmitOutcome::Accepted {
                run_id: observed_run_id,
                submitted_at: observed_submitted_at,
                ..
            } if observed_run_id == run_id && observed_submitted_at == submitted_at
        ));

        let replayed = trusted_trigger_response(run_id, MessageIdempotencyStatus::Duplicate, true);
        let replayed_outcome = submit_trusted_trigger_outcome(&replayed, submitted_at).unwrap();
        assert!(matches!(
            replayed_outcome,
            TrustedTriggerFireSubmitOutcome::Replayed {
                original_run_id,
                replayed_at,
                ..
            } if original_run_id == run_id && replayed_at == submitted_at
        ));

        let fresh_retry =
            trusted_trigger_response(run_id, MessageIdempotencyStatus::Duplicate, false);
        let fresh_retry_outcome =
            submit_trusted_trigger_outcome(&fresh_retry, submitted_at).unwrap();
        assert!(matches!(
            fresh_retry_outcome,
            TrustedTriggerFireSubmitOutcome::Accepted {
                run_id: observed_run_id,
                submitted_at: observed_submitted_at,
                ..
            } if observed_run_id == run_id && observed_submitted_at == submitted_at
        ));
    }

    #[test]
    fn submit_trusted_trigger_outcome_rejects_missing_turn_submission() {
        let submitted_at = Utc.with_ymd_and_hms(2026, 5, 6, 12, 30, 0).unwrap();
        let run_id = TurnRunId::new();
        let mut response =
            trusted_trigger_response(run_id, MessageIdempotencyStatus::Inserted, false);
        response.turn_submission = None;

        let error = submit_trusted_trigger_outcome(&response, submitted_at).unwrap_err();

        assert!(matches!(
            error,
            ironclaw_triggers::TriggerError::Backend { reason }
                if reason.contains("no turn submission")
        ));
    }

    #[test]
    fn classify_trusted_trigger_inbound_error_maps_retryable_backend_cases_to_opaque_backend() {
        for error in [
            InboundTurnError::TurnSubmissionFailed {
                error: TurnError::ThreadBusy(ThreadBusy {
                    active_run_id: TurnRunId::new(),
                    status: TurnStatus::Running,
                    event_cursor: EventCursor(7),
                }),
            },
            InboundTurnError::TurnSubmissionFailed {
                error: TurnError::AdmissionRejected(AdmissionRejection::new(
                    AdmissionRejectionReason::TenantLimit,
                )),
            },
            InboundTurnError::TurnSubmissionFailed {
                error: TurnError::AdmissionRejected(AdmissionRejection::new(
                    AdmissionRejectionReason::Unavailable,
                )),
            },
            InboundTurnError::TurnSubmissionFailed {
                error: TurnError::Unavailable {
                    reason: "turn store unavailable".to_string(),
                },
            },
            InboundTurnError::TurnSubmissionFailed {
                error: TurnError::CapacityExceeded {
                    resource: TurnCapacityResource::SubmitTurn,
                    cap: 1,
                },
            },
            InboundTurnError::TurnSubmissionFailed {
                error: TurnError::Conflict {
                    reason: "cas mismatch".to_string(),
                },
            },
            InboundTurnError::DurableState {
                reason: "disk write failed".to_string(),
            },
        ] {
            let classified = classify_trusted_trigger_inbound_error(error);
            assert!(matches!(
                classified,
                ironclaw_triggers::TriggerError::Backend { reason }
                    if reason == "trusted trigger submit retryable failure"
            ));
        }

        for error in [
            InboundTurnError::TurnSubmissionFailed {
                error: TurnError::AdmissionRejected(AdmissionRejection::new(
                    AdmissionRejectionReason::ProfileRejected,
                )),
            },
            InboundTurnError::TurnSubmissionFailed {
                error: TurnError::AdmissionRejected(AdmissionRejection::new(
                    AdmissionRejectionReason::Policy,
                )),
            },
            InboundTurnError::TurnSubmissionFailed {
                error: TurnError::AdmissionRejected(AdmissionRejection::new(
                    AdmissionRejectionReason::Unauthorized,
                )),
            },
            InboundTurnError::TurnSubmissionFailed {
                error: TurnError::ScopeNotFound,
            },
            InboundTurnError::TurnSubmissionFailed {
                error: TurnError::Unauthorized,
            },
            InboundTurnError::TurnSubmissionFailed {
                error: TurnError::InvalidRequest {
                    reason: "bad request".to_string(),
                },
            },
            InboundTurnError::TurnSubmissionFailed {
                error: TurnError::InvalidTransition {
                    from: TurnStatus::Queued,
                    to: TurnStatus::Completed,
                },
            },
            InboundTurnError::TurnSubmissionFailed {
                error: TurnError::LeaseMismatch,
            },
        ] {
            let classified = classify_trusted_trigger_inbound_error(error);
            assert!(matches!(
                classified,
                ironclaw_triggers::TriggerError::InvalidMaterialization { reason }
                    if reason == "trusted trigger submit rejected"
            ));
        }

        for error in [
            InboundTurnError::InvalidExternalRef {
                kind: "adapter_kind",
                reason: "empty".to_string(),
            },
            InboundTurnError::BindingRequired {
                adapter_kind: TRIGGER_TRUSTED_ADAPTER_KIND.to_string(),
                external_actor_id: "actor".to_string(),
            },
            InboundTurnError::AccessDenied {
                actor_id: "actor".to_string(),
                thread_id: "thread".to_string(),
            },
            InboundTurnError::BindingConflict {
                thread_id: "conflicting-thread".to_string(),
            },
            InboundTurnError::ThreadNotFound {
                thread_id: "missing-thread".to_string(),
            },
            InboundTurnError::StatePoisoned,
            InboundTurnError::InvalidCanonicalRef {
                reason: "too long".to_string(),
            },
        ] {
            let classified = classify_trusted_trigger_inbound_error(error);
            assert!(matches!(
                classified,
                ironclaw_triggers::TriggerError::InvalidMaterialization { reason }
                    if reason == "trusted trigger inbound request rejected"
            ));
        }
    }

    fn trusted_trigger_response(
        run_id: TurnRunId,
        idempotency: MessageIdempotencyStatus,
        replayed_turn_submission: bool,
    ) -> InboundTurnResponse {
        let tenant_id = tenant();
        let actor_user_id = user("alice");
        let actor = ironclaw_turns::TurnActor::new(actor_user_id);
        let thread_id = ThreadId::new("trusted-trigger-outcome-thread").unwrap();
        let source_binding_ref = SourceBindingRef::new("trusted-trigger-outcome-source").unwrap();
        let reply_target_binding_ref =
            ReplyTargetBindingRef::new("trusted-trigger-outcome-reply").unwrap();
        let accepted_message_ref =
            AcceptedMessageRef::new("message:trusted-trigger-outcome").unwrap();
        let received_at = Utc.with_ymd_and_hms(2026, 5, 6, 12, 0, 0).unwrap();
        InboundTurnResponse {
            resolution: ConversationBindingResolution {
                tenant_id: tenant_id.clone(),
                actor: actor.clone(),
                turn_scope: TurnScope::new(
                    tenant_id.clone(),
                    Some(agent()),
                    Some(project()),
                    thread_id.clone(),
                ),
                source_binding_ref: source_binding_ref.clone(),
                reply_target_binding_ref: reply_target_binding_ref.clone(),
                access: ThreadAccessDecision::Allowed,
            },
            accepted_message: AcceptedInboundMessage {
                tenant_id,
                thread_id,
                actor,
                message_ref: accepted_message_ref.clone(),
                source_binding_ref,
                reply_target_binding_ref: reply_target_binding_ref.clone(),
                received_at,
                requested_run_profile: None,
                idempotency,
            },
            turn_submission: Some(SubmitTurnResponse::Accepted {
                turn_id: TurnId::new(),
                run_id,
                status: TurnStatus::Completed,
                resolved_run_profile_id: RunProfileId::default_profile(),
                resolved_run_profile_version: RunProfileVersion::new(1),
                event_cursor: EventCursor(0),
                accepted_message_ref,
                reply_target_binding_ref,
            }),
            replayed_turn_submission,
        }
    }

    fn trusted_inbound_request(
        trusted_agent_id: Option<AgentId>,
        trusted_project_id: Option<ProjectId>,
    ) -> TrustedInboundTurnRequest {
        TrustedInboundTurnRequest::new(
            base_inbound_request(),
            trusted_agent_id,
            trusted_project_id,
            None,
            TrustedInboundKind::Trigger,
        )
    }

    fn base_inbound_request() -> InboundTurnRequest {
        let fire_slot = Utc.with_ymd_and_hms(2026, 5, 6, 12, 0, 0).unwrap();
        InboundTurnRequest {
            tenant_id: tenant(),
            adapter_kind: trigger_adapter(),
            adapter_installation_id: trigger_installation(),
            external_actor_ref: external_actor("alice"),
            external_conversation_ref: ExternalConversationRef::new(
                None,
                "trigger-test",
                Some("route-trigger-test"),
                None,
            )
            .unwrap(),
            external_event_id: ExternalEventId::new("external-event-trigger-test").unwrap(),
            route_kind: ConversationRouteKind::Direct,
            content_ref: InboundMessageContentRef::new("content:trigger-test").unwrap(),
            requested_agent_id: None,
            requested_project_id: None,
            received_at: fire_slot,
            requested_run_profile: None,
        }
    }

    /// Returns `(facade, services, coordinator)` — a paired `InboundTurnService`
    /// backed by `InMemoryConversationServices` with "alice" already paired so
    /// trusted binding resolution succeeds, plus the underlying services and
    /// coordinator for post-call inspection.
    async fn trusted_inbound_service() -> (
        InboundTurnService<
            InMemoryConversationServices,
            InMemoryConversationServices,
            RecordingTurnCoordinator,
        >,
        InMemoryConversationServices,
        Arc<RecordingTurnCoordinator>,
    ) {
        let services = InMemoryConversationServices::default();
        services
            .pair_external_actor(
                tenant(),
                trigger_adapter(),
                trigger_installation(),
                external_actor("alice"),
                user("alice"),
            )
            .await;
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let facade =
            InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());
        (facade, services, coordinator)
    }

    fn tenant() -> TenantId {
        TenantId::new("tenant").unwrap()
    }

    fn trigger_adapter() -> AdapterKind {
        AdapterKind::new(TRIGGER_TRUSTED_ADAPTER_KIND).unwrap()
    }

    fn trigger_installation() -> AdapterInstallationId {
        AdapterInstallationId::new(TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID).unwrap()
    }

    fn external_actor(value: &str) -> ExternalActorRef {
        ExternalActorRef::new(TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, value).unwrap()
    }

    fn user(value: &str) -> UserId {
        UserId::new(value).unwrap()
    }

    fn agent() -> AgentId {
        AgentId::new("agent").unwrap()
    }

    fn project() -> ProjectId {
        ProjectId::new("project").unwrap()
    }

    type TrustedScopeRecord = (Option<AgentId>, Option<ProjectId>, Option<UserId>);
    type TrustedScopeRecords = Arc<Mutex<Vec<TrustedScopeRecord>>>;

    #[derive(Clone)]
    struct TrustedOnlyBindingService {
        inner: InMemoryConversationServices,
        resolve_requests: Arc<Mutex<Vec<crate::ResolveConversationRequest>>>,
        trusted_scopes: TrustedScopeRecords,
    }

    impl TrustedOnlyBindingService {
        fn new(inner: InMemoryConversationServices) -> Self {
            Self {
                inner,
                resolve_requests: Arc::new(Mutex::new(Vec::new())),
                trusted_scopes: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn trusted_calls(&self) -> usize {
            self.trusted_scopes.lock().unwrap().len()
        }

        fn resolve_requests(&self) -> Vec<crate::ResolveConversationRequest> {
            self.resolve_requests.lock().unwrap().clone()
        }

        fn trusted_scopes(&self) -> Vec<(Option<AgentId>, Option<ProjectId>, Option<UserId>)> {
            self.trusted_scopes.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ConversationBindingService for TrustedOnlyBindingService {
        async fn resolve_or_create_binding(
            &self,
            _request: crate::ResolveConversationRequest,
        ) -> Result<ConversationBindingResolution, InboundTurnError> {
            panic!("trusted inbound must call resolve_or_create_binding_with_trusted_scope")
        }

        async fn resolve_or_create_binding_with_trusted_scope(
            &self,
            request: crate::ResolveConversationRequest,
            trusted_agent_id: Option<AgentId>,
            trusted_project_id: Option<ProjectId>,
            trusted_owner_user_id: Option<UserId>,
        ) -> Result<ConversationBindingResolution, InboundTurnError> {
            self.resolve_requests.lock().unwrap().push(request.clone());
            self.trusted_scopes.lock().unwrap().push((
                trusted_agent_id.clone(),
                trusted_project_id.clone(),
                trusted_owner_user_id.clone(),
            ));
            self.inner
                .resolve_or_create_binding_with_trusted_scope(
                    request,
                    trusted_agent_id,
                    trusted_project_id,
                    trusted_owner_user_id,
                )
                .await
        }

        async fn lookup_binding(
            &self,
            request: crate::ResolveConversationRequest,
        ) -> Result<ConversationBindingResolution, InboundTurnError> {
            self.inner.lookup_binding(request).await
        }

        async fn link_conversation_to_thread(
            &self,
            request: LinkConversationRequest,
        ) -> Result<LinkedConversationBinding, InboundTurnError> {
            self.inner.link_conversation_to_thread(request).await
        }

        async fn validate_reply_target(
            &self,
            request: ValidateReplyTargetRequest,
        ) -> Result<ReplyTargetBinding, InboundTurnError> {
            self.inner.validate_reply_target(request).await
        }
    }

    #[derive(Clone)]
    struct RejectingTrustedBindingService {
        resolve_requests: Arc<Mutex<Vec<crate::ResolveConversationRequest>>>,
        trusted_scopes: TrustedScopeRecords,
    }

    impl RejectingTrustedBindingService {
        fn new() -> Self {
            Self {
                resolve_requests: Arc::new(Mutex::new(Vec::new())),
                trusted_scopes: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn trusted_scopes(&self) -> Vec<(Option<AgentId>, Option<ProjectId>, Option<UserId>)> {
            self.trusted_scopes.lock().unwrap().clone()
        }

        fn resolve_requests(&self) -> Vec<crate::ResolveConversationRequest> {
            self.resolve_requests.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ConversationBindingService for RejectingTrustedBindingService {
        async fn resolve_or_create_binding(
            &self,
            _request: crate::ResolveConversationRequest,
        ) -> Result<ConversationBindingResolution, InboundTurnError> {
            panic!("trusted inbound must call resolve_or_create_binding_with_trusted_scope")
        }

        async fn resolve_or_create_binding_with_trusted_scope(
            &self,
            request: crate::ResolveConversationRequest,
            trusted_agent_id: Option<AgentId>,
            trusted_project_id: Option<ProjectId>,
            trusted_owner_user_id: Option<UserId>,
        ) -> Result<ConversationBindingResolution, InboundTurnError> {
            self.resolve_requests.lock().unwrap().push(request);
            self.trusted_scopes.lock().unwrap().push((
                trusted_agent_id,
                trusted_project_id,
                trusted_owner_user_id,
            ));
            Err(InboundTurnError::BindingRequired {
                adapter_kind: "trusted".to_string(),
                external_actor_id: "trusted".to_string(),
            })
        }

        async fn lookup_binding(
            &self,
            _request: crate::ResolveConversationRequest,
        ) -> Result<ConversationBindingResolution, InboundTurnError> {
            unimplemented!("not used by inbound facade tests")
        }

        async fn link_conversation_to_thread(
            &self,
            _request: LinkConversationRequest,
        ) -> Result<LinkedConversationBinding, InboundTurnError> {
            unimplemented!("not used by inbound facade tests")
        }

        async fn validate_reply_target(
            &self,
            _request: ValidateReplyTargetRequest,
        ) -> Result<ReplyTargetBinding, InboundTurnError> {
            unimplemented!("not used by inbound facade tests")
        }
    }

    #[derive(Default)]
    struct RecordingTurnCoordinator {
        submissions: Mutex<Vec<SubmitTurnRequest>>,
    }

    impl RecordingTurnCoordinator {
        fn submissions(&self) -> Vec<SubmitTurnRequest> {
            self.submissions.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl TurnCoordinator for RecordingTurnCoordinator {
        async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
            Ok(TurnRunId::new())
        }

        async fn submit_turn(
            &self,
            request: SubmitTurnRequest,
        ) -> Result<SubmitTurnResponse, TurnError> {
            self.submissions.lock().unwrap().push(request.clone());
            Ok(SubmitTurnResponse::Accepted {
                turn_id: TurnId::new(),
                run_id: TurnRunId::new(),
                status: TurnStatus::Completed,
                resolved_run_profile_id: RunProfileId::default_profile(),
                resolved_run_profile_version: RunProfileVersion::new(1),
                event_cursor: EventCursor(0),
                accepted_message_ref: request.accepted_message_ref,
                reply_target_binding_ref: request.reply_target_binding_ref,
            })
        }

        async fn resume_turn(
            &self,
            _request: ResumeTurnRequest,
        ) -> Result<ResumeTurnResponse, TurnError> {
            unimplemented!("not used by inbound facade tests")
        }

        async fn cancel_run(
            &self,
            _request: CancelRunRequest,
        ) -> Result<CancelRunResponse, TurnError> {
            unimplemented!("not used by inbound facade tests")
        }

        async fn get_run_state(
            &self,
            _request: GetRunStateRequest,
        ) -> Result<TurnRunState, TurnError> {
            unimplemented!("not used by inbound facade tests")
        }
    }

    // --- Tests: error classification ---

    #[test]
    fn classify_trusted_trigger_inbound_error_maps_invalid_run_origin_adapter_to_submit_rejected() {
        let error = InboundTurnError::TurnSubmissionFailed {
            error: TurnError::InvalidRunOriginAdapter,
        };
        let classified = classify_trusted_trigger_inbound_error(error);
        assert!(
            matches!(
                classified,
                ironclaw_triggers::TriggerError::InvalidMaterialization { reason }
                    if reason == "trusted trigger submit rejected"
            ),
            "InvalidRunOriginAdapter must be classified as SubmitRejected → InvalidMaterialization"
        );
    }

    // --- Tests: submit-key rotation ---

    #[tokio::test]
    async fn invalid_run_origin_adapter_does_not_rotate_submit_idempotency_key() {
        let services = InMemoryConversationServices::default();
        services
            .pair_external_actor(
                tenant(),
                trigger_adapter(),
                trigger_installation(),
                external_actor("alice"),
                user("alice"),
            )
            .await;
        let coordinator = Arc::new(FailingOnFirstTurnCoordinator::default());
        let inbound =
            InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());
        let request = trusted_inbound_request(Some(agent()), Some(project()));

        // First call: coordinator returns InvalidRunOriginAdapter — inbound returns an error.
        let err = inbound
            .handle_inbound_turn_with_trusted_scope(request.clone())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            InboundTurnError::TurnSubmissionFailed {
                error: TurnError::InvalidRunOriginAdapter
            }
        ));

        // Second call: same request (same external_event_id → same accepted_message_ref).
        // The first attempt never called mark_inbound_message_turn_submitted, so the
        // duplicate idempotency path falls through to a fresh submit_turn call.
        // Coordinator now succeeds and records the second key.
        let _ = inbound
            .handle_inbound_turn_with_trusted_scope(request)
            .await
            .expect("second inbound attempt succeeds");

        let submissions = coordinator.submissions();
        assert_eq!(
            submissions.len(),
            2,
            "coordinator must have been called twice"
        );

        // Both calls must have received the same idempotency key: not rotating on
        // InvalidRunOriginAdapter preserves the original key so the turn store can
        // deduplicate duplicate retries.
        assert_eq!(
            submissions[0].idempotency_key, submissions[1].idempotency_key,
            "submit key must not rotate after InvalidRunOriginAdapter — duplicate retries must share the same idempotency key"
        );
    }

    /// A `TurnCoordinator` that returns `TurnError::InvalidRunOriginAdapter` on the
    /// first `submit_turn` call and succeeds on all subsequent calls.
    #[derive(Default)]
    struct FailingOnFirstTurnCoordinator {
        submissions: Mutex<Vec<SubmitTurnRequest>>,
    }

    impl FailingOnFirstTurnCoordinator {
        fn submissions(&self) -> Vec<SubmitTurnRequest> {
            self.submissions.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl TurnCoordinator for FailingOnFirstTurnCoordinator {
        async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
            Ok(TurnRunId::new())
        }

        async fn submit_turn(
            &self,
            request: SubmitTurnRequest,
        ) -> Result<SubmitTurnResponse, TurnError> {
            let mut submissions = self.submissions.lock().unwrap();
            submissions.push(request.clone());
            if submissions.len() == 1 {
                return Err(TurnError::InvalidRunOriginAdapter);
            }
            Ok(SubmitTurnResponse::Accepted {
                turn_id: TurnId::new(),
                run_id: TurnRunId::new(),
                status: TurnStatus::Completed,
                resolved_run_profile_id: RunProfileId::default_profile(),
                resolved_run_profile_version: RunProfileVersion::new(1),
                event_cursor: EventCursor(0),
                accepted_message_ref: request.accepted_message_ref,
                reply_target_binding_ref: request.reply_target_binding_ref,
            })
        }

        async fn resume_turn(
            &self,
            _request: ResumeTurnRequest,
        ) -> Result<ResumeTurnResponse, TurnError> {
            unimplemented!("not used by submit-key rotation tests")
        }

        async fn cancel_run(
            &self,
            _request: CancelRunRequest,
        ) -> Result<CancelRunResponse, TurnError> {
            unimplemented!("not used by submit-key rotation tests")
        }

        async fn get_run_state(
            &self,
            _request: GetRunStateRequest,
        ) -> Result<TurnRunState, TurnError> {
            unimplemented!("not used by submit-key rotation tests")
        }
    }

    // --- Tests: run_origin integrity ---

    /// A trusted inbound request whose adapter_kind is NOT a trusted-trigger
    /// adapter (e.g. "slack") must record `TurnOriginKind::Inbound`, not
    /// `ScheduledTrigger`.  This exercises the `TrustedOther` classification
    /// branch that sits between `TrustedTrigger` and `Untrusted`.
    #[tokio::test]
    async fn trusted_non_trigger_adapter_records_inbound_origin() {
        let slack = AdapterKind::new("slack").unwrap();
        let slack_install = AdapterInstallationId::new("slack-install").unwrap();
        let slack_actor = ExternalActorRef::new("slack", "alice").unwrap();
        let services = InMemoryConversationServices::default();
        services
            .pair_external_actor(
                tenant(),
                slack.clone(),
                slack_install.clone(),
                slack_actor.clone(),
                user("alice"),
            )
            .await;
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let inbound =
            InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());

        // Build a trusted request using a non-trigger adapter ("slack").
        let request = TrustedInboundTurnRequest::new(
            InboundTurnRequest {
                tenant_id: tenant(),
                adapter_kind: slack,
                adapter_installation_id: slack_install,
                external_actor_ref: slack_actor,
                external_conversation_ref: ExternalConversationRef::new(
                    None,
                    "slack-trusted-conv",
                    Some("slack-trusted-thread"),
                    None,
                )
                .unwrap(),
                external_event_id: ExternalEventId::new("slack-trusted-event-1").unwrap(),
                route_kind: ConversationRouteKind::Direct,
                content_ref: InboundMessageContentRef::new("content:slack-trusted-1").unwrap(),
                requested_agent_id: None,
                requested_project_id: None,
                received_at: Utc.with_ymd_and_hms(2026, 6, 13, 10, 0, 0).unwrap(),
                requested_run_profile: None,
            },
            Some(agent()),
            Some(project()),
            None,
            TrustedInboundKind::Other,
        );

        inbound
            .handle_inbound_turn_with_trusted_scope(request)
            .await
            .expect("trusted non-trigger inbound succeeds");

        let submissions = coordinator.submissions();
        assert_eq!(submissions.len(), 1);
        assert_eq!(
            submissions[0].product_context.as_ref().map(|c| c.origin),
            Some(TurnOriginKind::Inbound),
            "trusted binding with non-trigger adapter 'slack' must record Inbound origin, not ScheduledTrigger"
        );
        assert_ne!(
            submissions[0].product_context.as_ref().map(|c| c.origin),
            Some(TurnOriginKind::ScheduledTrigger),
            "trusted non-trigger adapter must NOT be labelled ScheduledTrigger"
        );
    }

    /// An untrusted inbound request with adapter_kind "trigger" must NOT be
    /// labelled ScheduledTrigger — only a Trusted binding policy + trusted-trigger
    /// adapter qualifies.
    #[tokio::test]
    async fn untrusted_trigger_adapter_records_product_inbound_not_scheduled_trigger() {
        let services = InMemoryConversationServices::default();
        services
            .pair_external_actor(
                tenant(),
                trigger_adapter(),
                trigger_installation(),
                external_actor("alice"),
                user("alice"),
            )
            .await;
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let inbound =
            InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());

        // Untrusted path: handle_inbound_turn uses BindingResolutionPolicy::Untrusted.
        let request = InboundTurnRequest {
            tenant_id: tenant(),
            adapter_kind: trigger_adapter(),
            adapter_installation_id: trigger_installation(),
            external_actor_ref: external_actor("alice"),
            external_conversation_ref: ExternalConversationRef::new(
                None,
                "untrusted-trigger-conv",
                Some("untrusted-trigger-thread"),
                None,
            )
            .unwrap(),
            external_event_id: ExternalEventId::new("untrusted-trigger-event").unwrap(),
            route_kind: ConversationRouteKind::Direct,
            content_ref: InboundMessageContentRef::new("content:untrusted-trigger").unwrap(),
            requested_agent_id: None,
            requested_project_id: None,
            received_at: Utc.with_ymd_and_hms(2026, 6, 13, 10, 0, 0).unwrap(),
            requested_run_profile: None,
        };

        inbound
            .handle_inbound_turn(request)
            .await
            .expect("untrusted inbound succeeds");

        let submissions = coordinator.submissions();
        assert_eq!(submissions.len(), 1);
        assert_eq!(
            submissions[0].product_context.as_ref().map(|c| c.origin),
            Some(TurnOriginKind::Inbound),
            "untrusted adapter_kind='trigger' must record Inbound origin, not ScheduledTrigger"
        );
        assert_eq!(
            submissions[0]
                .product_context
                .as_ref()
                .and_then(|c| c.adapter.as_ref())
                .map(|a| a.as_str()),
            Some("trigger"),
            "untrusted adapter_kind='trigger' must carry adapter name 'trigger'"
        );
    }

    /// An untrusted inbound request with a Shared route kind must record
    /// `surface_type == Some(TurnSurfaceType::Channel)` on the submitted product context.
    #[tokio::test]
    async fn shared_route_kind_records_channel_surface_type() {
        let slack = AdapterKind::new("slack").unwrap();
        let slack_install = AdapterInstallationId::new("slack-install").unwrap();
        let slack_actor = ExternalActorRef::new("slack", "user-alice").unwrap();
        let services = InMemoryConversationServices::default();
        services
            .pair_external_actor(
                tenant(),
                slack.clone(),
                slack_install.clone(),
                slack_actor.clone(),
                user("alice"),
            )
            .await;
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let inbound =
            InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());

        let request = InboundTurnRequest {
            tenant_id: tenant(),
            adapter_kind: slack,
            adapter_installation_id: slack_install,
            external_actor_ref: slack_actor,
            external_conversation_ref: ExternalConversationRef::new(
                None,
                "slack-channel-conv",
                Some("slack-channel-thread"),
                None,
            )
            .unwrap(),
            external_event_id: ExternalEventId::new("slack-channel-event-1").unwrap(),
            route_kind: ConversationRouteKind::Shared,
            content_ref: InboundMessageContentRef::new("content:slack-channel-1").unwrap(),
            requested_agent_id: None,
            requested_project_id: None,
            received_at: Utc.with_ymd_and_hms(2026, 6, 13, 10, 0, 0).unwrap(),
            requested_run_profile: None,
        };

        inbound
            .handle_inbound_turn(request)
            .await
            .expect("shared-route inbound succeeds");

        let submissions = coordinator.submissions();
        assert_eq!(submissions.len(), 1);
        assert_eq!(
            submissions[0].product_context.as_ref().map(|c| c.origin),
            Some(TurnOriginKind::Inbound),
            "shared-route inbound must record Inbound origin"
        );
        assert_eq!(
            submissions[0]
                .product_context
                .as_ref()
                .and_then(|c| c.surface_type),
            Some(TurnSurfaceType::Channel),
            "Shared route kind must record Channel surface type"
        );
    }

    /// A normal (non-trigger) inbound adapter through the standard untrusted path
    /// must record ProductInbound with the adapter name.
    #[tokio::test]
    async fn ordinary_inbound_adapter_records_product_inbound_with_adapter_name() {
        let telegram = AdapterKind::new("telegram").unwrap();
        let telegram_install = AdapterInstallationId::new("telegram-install").unwrap();
        let telegram_actor = ExternalActorRef::new("telegram", "user-alice").unwrap();
        let services = InMemoryConversationServices::default();
        services
            .pair_external_actor(
                tenant(),
                telegram.clone(),
                telegram_install.clone(),
                telegram_actor.clone(),
                user("alice"),
            )
            .await;
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let inbound =
            InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());

        let request = InboundTurnRequest {
            tenant_id: tenant(),
            adapter_kind: telegram,
            adapter_installation_id: telegram_install,
            external_actor_ref: telegram_actor,
            external_conversation_ref: ExternalConversationRef::new(
                None,
                "telegram-conv",
                Some("telegram-thread"),
                None,
            )
            .unwrap(),
            external_event_id: ExternalEventId::new("telegram-event-1").unwrap(),
            route_kind: ConversationRouteKind::Direct,
            content_ref: InboundMessageContentRef::new("content:telegram-1").unwrap(),
            requested_agent_id: None,
            requested_project_id: None,
            received_at: Utc.with_ymd_and_hms(2026, 6, 13, 10, 0, 0).unwrap(),
            requested_run_profile: None,
        };

        inbound
            .handle_inbound_turn(request)
            .await
            .expect("telegram inbound succeeds");

        let submissions = coordinator.submissions();
        assert_eq!(submissions.len(), 1);
        assert_eq!(
            submissions[0].product_context.as_ref().map(|c| c.origin),
            Some(TurnOriginKind::Inbound),
            "ordinary inbound adapter must record Inbound origin"
        );
        assert_eq!(
            submissions[0]
                .product_context
                .as_ref()
                .and_then(|c| c.adapter.as_ref())
                .map(|a| a.as_str()),
            Some("telegram"),
            "ordinary inbound adapter must carry adapter name"
        );
    }

    /// A long but valid adapter kind (300 bytes, well within `AdapterKind`'s 512-byte
    /// cap) must NOT be rejected by the `AdapterKind` → `RunOriginAdapter` conversion
    /// inside `handle_inbound_turn`. Before the bound alignment fix, `RunOriginAdapter`
    /// capped at 256 bytes and would return `InvalidCanonicalRef` for any adapter kind
    /// between 257–512 bytes — a silent narrowing below `AdapterKind`'s own limit.
    #[tokio::test]
    async fn long_valid_adapter_kind_is_not_rejected_by_run_origin_conversion() {
        // 300-byte adapter kind: valid for both AdapterKind (≤ 512) and the now-aligned
        // RunOriginAdapter (≤ 512). Must reach accept/submit normally.
        let long_name = "a".repeat(300);
        let long_adapter = AdapterKind::new(&long_name)
            .expect("300-byte adapter kind must be valid — AdapterKind allows up to 512 bytes");
        let long_install = AdapterInstallationId::new("long-adapter-install").unwrap();
        let long_actor = ExternalActorRef::new("long", "user-alice").unwrap();
        let services = InMemoryConversationServices::default();
        services
            .pair_external_actor(
                tenant(),
                long_adapter.clone(),
                long_install.clone(),
                long_actor.clone(),
                user("alice"),
            )
            .await;
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let inbound =
            InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());

        let request = InboundTurnRequest {
            tenant_id: tenant(),
            adapter_kind: long_adapter,
            adapter_installation_id: long_install,
            external_actor_ref: long_actor,
            external_conversation_ref: ExternalConversationRef::new(
                None,
                "long-adapter-conv",
                Some("long-adapter-thread"),
                None,
            )
            .unwrap(),
            external_event_id: ExternalEventId::new("long-adapter-event-1").unwrap(),
            route_kind: ConversationRouteKind::Direct,
            content_ref: InboundMessageContentRef::new("content:long-adapter-1").unwrap(),
            requested_agent_id: None,
            requested_project_id: None,
            received_at: Utc.with_ymd_and_hms(2026, 6, 14, 10, 0, 0).unwrap(),
            requested_run_profile: None,
        };

        let result = inbound.handle_inbound_turn(request).await;

        // Must NOT return InvalidCanonicalRef — the conversion must not narrow below
        // AdapterKind's own limit.
        assert!(
            !matches!(result, Err(InboundTurnError::InvalidCanonicalRef { .. })),
            "a 300-byte adapter kind must not be rejected by the RunOriginAdapter conversion; \
             got: {result:?}"
        );
        // Should reach the submit path successfully.
        assert!(
            result.is_ok(),
            "a 300-byte adapter kind must succeed end-to-end; got: {result:?}"
        );
        assert_eq!(
            coordinator.submissions().len(),
            1,
            "exactly one turn submission must have been recorded"
        );
    }
}
