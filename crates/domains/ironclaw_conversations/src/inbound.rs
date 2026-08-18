use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_contracts::external::{ExternalActorRef, ExternalConversationRef};
use ironclaw_host_api::turn::{RunOriginAdapter, RunProfileId, RunProfileRequest, TurnSurfaceType};
use ironclaw_triggers::{
    TriggerError, TrustedTriggerFireSubmitOutcome, TrustedTriggerFireSubmitter,
    TrustedTriggerSubmitRequest,
};

use crate::ids::map_external_ref_error;
use crate::trusted_trigger::{TrustedTriggerInboundFailureKind, classify_inbound_error};
use crate::turn_submission::{
    ConversationInboundClassification, ConversationTurnSubmission, ConversationTurnSubmitter,
    TurnSubmissionError, TurnSubmissionRetry,
};
use crate::types::{TrustedInboundKind, TrustedInboundTurnRequest};
use crate::{
    AcceptConversationMessageRequest, AcceptedConversationMessage,
    AcceptedConversationMessageLookup, AdapterInstallationId, AdapterKind,
    ConversationBindingResolution, ConversationBindingService, ConversationRouteKind,
    ExternalEventId, InboundConversationService, InboundMessageContentRef, InboundTurnError,
    InboundTurnRequest, InboundTurnResponse, MessageIdempotencyStatus, ResolveConversationRequest,
};

#[derive(Clone)]
pub struct InboundTurnService<B, S, C: ?Sized> {
    binding_service: B,
    conversation_service: S,
    turn_submitter: Arc<C>,
}

impl<B, S, C> InboundTurnService<B, S, C>
where
    B: ConversationBindingService,
    S: InboundConversationService,
    C: ConversationTurnSubmitter + ?Sized,
{
    pub fn new(binding_service: B, conversation_service: S, turn_submitter: Arc<C>) -> Self {
        Self {
            binding_service,
            conversation_service,
            turn_submitter,
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
            execution_policy,
        } = request;
        self.handle_inbound_turn_inner(
            request,
            BindingResolutionPolicy::Trusted {
                trusted_agent_id,
                trusted_project_id,
                trusted_owner_user_id,
                kind,
                execution_policy,
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
            } => ConversationInboundClassification::TrustedTrigger,
            BindingResolutionPolicy::Trusted { .. } => {
                ConversationInboundClassification::TrustedOther
            }
            BindingResolutionPolicy::Untrusted => ConversationInboundClassification::Untrusted,
        };
        let execution_policy = match &binding_policy {
            BindingResolutionPolicy::Trusted {
                kind: TrustedInboundKind::Trigger,
                execution_policy,
                ..
            } => execution_policy.clone(),
            BindingResolutionPolicy::Trusted { .. } | BindingResolutionPolicy::Untrusted => None,
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

        let replay_lookup = AcceptedConversationMessageLookup {
            tenant_id: tenant_id.clone(),
            adapter_kind: adapter_kind.clone(),
            adapter_installation_id: adapter_installation_id.clone(),
            external_actor_ref: external_actor_ref.clone(),
            external_conversation_ref: external_conversation_ref.clone(),
            external_event_id: external_event_id.clone(),
        };
        if let Some(replay) = self
            .conversation_service
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
                    execution_policy.clone(),
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
                execution_policy: _,
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
            .conversation_service
            .accept_inbound_message(AcceptConversationMessageRequest {
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
            execution_policy,
        )
        .await
    }

    async fn submit_or_replay(
        &self,
        mut resolution: ConversationBindingResolution,
        accepted_message: AcceptedConversationMessage,
        classification: ConversationInboundClassification,
        run_adapter: RunOriginAdapter,
        surface_type: Option<TurnSurfaceType>,
        execution_policy: Option<ironclaw_host_api::execution_policy::TurnExecutionPolicy>,
    ) -> Result<InboundTurnResponse, InboundTurnError> {
        resolution.actor = accepted_message.actor.clone();

        if accepted_message.idempotency == MessageIdempotencyStatus::Duplicate
            && let Some(turn_submission) = self
                .conversation_service
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
            .conversation_service
            .inbound_message_turn_submission_key(&accepted_message.message_ref)
            .await?;
        let turn_submission_result = self
            .turn_submitter
            .submit_conversation_turn(ConversationTurnSubmission {
                scope: resolution.turn_scope.clone(),
                actor: accepted_message.actor.clone(),
                accepted_message_ref: accepted_message.message_ref.clone(),
                requested_run_profile: accepted_message.requested_run_profile.clone(),
                idempotency_key,
                received_at: accepted_message.received_at,
                classification,
                origin_adapter: run_adapter,
                surface_type,
                execution_policy,
            })
            .await;
        let turn_submission = match turn_submission_result {
            Ok(response) => response,
            Err(error) => {
                if should_rotate_submit_key(&error) {
                    self.conversation_service
                        .rotate_inbound_message_turn_submission_key(&accepted_message.message_ref)
                        .await?;
                }
                return Err(InboundTurnError::TurnSubmissionFailed { error });
            }
        };
        self.conversation_service
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

/// Conversations-owned implementation of the trusted-trigger submit seam.
///
/// It converts an already-validated `TrustedTriggerSubmitRequest` into this
/// crate's private trusted inbound request and submits it. Prompt safety is
/// *not* checked here: PROPOSAL §6.4.2 moved that scan behind the seam, into
/// `ironclaw_triggers`' mint of the sealed request, so it applies to every
/// submitter rather than only to this one.
#[derive(Clone)]
pub(crate) struct ConversationTrustedTriggerSubmitter<B, S, C: ?Sized> {
    inbound: InboundTurnService<B, S, C>,
}

impl<B, S, C> ConversationTrustedTriggerSubmitter<B, S, C>
where
    B: ConversationBindingService,
    S: InboundConversationService,
    C: ConversationTurnSubmitter + ?Sized,
{
    pub(crate) fn new(binding_service: B, conversation_service: S, turn_submitter: Arc<C>) -> Self {
        Self {
            inbound: InboundTurnService::new(binding_service, conversation_service, turn_submitter),
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
    conversation_service: S,
    turn_submitter: Arc<C>,
) -> Arc<dyn TrustedTriggerFireSubmitter>
where
    B: ConversationBindingService + 'static,
    S: InboundConversationService + 'static,
    C: ConversationTurnSubmitter + ?Sized + 'static,
{
    Arc::new(ConversationTrustedTriggerSubmitter::new(
        binding_service,
        conversation_service,
        turn_submitter,
    ))
}

#[async_trait]
impl<B, S, C> TrustedTriggerFireSubmitter for ConversationTrustedTriggerSubmitter<B, S, C>
where
    B: ConversationBindingService,
    S: InboundConversationService,
    C: ConversationTurnSubmitter + ?Sized,
{
    async fn submit_trusted_trigger_fire(
        &self,
        request: TrustedTriggerSubmitRequest,
    ) -> Result<TrustedTriggerFireSubmitOutcome, TriggerError> {
        let submitted_at = request.received_at();
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
                // A trigger fire has no human display name to carry.
                None::<String>,
            )
            .map_err(map_external_ref_error)?,
            external_conversation_ref: ExternalConversationRef::new(
                None,
                trusted_inbound_binding.external_conversation_id(),
                Some(trusted_inbound_binding.route_thread_id()),
                None,
            )
            .map_err(map_external_ref_error)?,
            external_event_id: ExternalEventId::new(trusted_inbound_binding.external_event_id())?,
            route_kind: ConversationRouteKind::Direct,
            content_ref: InboundMessageContentRef::new(content_ref.as_str())?,
            requested_agent_id: None,
            requested_project_id: None,
            received_at,
            // Issue #5505: a trusted trigger fire must run under the
            // dedicated scheduled_trigger profile so the host deny-map
            // (ironclaw_turn_runner runtime.rs) strips the trigger mutator
            // capabilities from the fire's model-visible surface — a fire
            // must not be able to create/remove/pause/resume triggers.
            requested_run_profile: Some(
                RunProfileRequest::new(RunProfileId::scheduled_trigger().as_str()).map_err(
                    |reason| InboundTurnError::InvalidExternalRef {
                        kind: "run_profile_request",
                        reason,
                    },
                )?,
            ),
        },
        fire.agent_id,
        fire.project_id,
        Some(fire.creator_user_id),
        TrustedInboundKind::Trigger,
        fire.execution_policy,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BindingResolutionPolicy {
    Untrusted,
    Trusted {
        trusted_agent_id: Option<ironclaw_host_api::ids::AgentId>,
        trusted_project_id: Option<ironclaw_host_api::ids::ProjectId>,
        trusted_owner_user_id: Option<ironclaw_host_api::ids::UserId>,
        kind: TrustedInboundKind,
        execution_policy: Option<ironclaw_host_api::execution_policy::TurnExecutionPolicy>,
    },
}

fn should_rotate_submit_key(error: &TurnSubmissionError) -> bool {
    match error.retry() {
        TurnSubmissionRetry::RetryableAfterKeyRotation => true,
        TurnSubmissionRetry::RetryableWithSameKey | TurnSubmissionRetry::Permanent => false,
    }
}

fn submit_trusted_trigger_outcome(
    response: &InboundTurnResponse,
    submitted_at: chrono::DateTime<chrono::Utc>,
) -> Result<TrustedTriggerFireSubmitOutcome, TriggerError> {
    let run_id = match &response.turn_submission {
        Some(ironclaw_host_api::turn::SubmitTurnResponse::Accepted { run_id, .. }) => *run_id,
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
    error: &InboundTurnError,
) -> TriggerError {
    tracing::debug!(reason, "trusted trigger inbound rejection");
    if matches!(
        error,
        InboundTurnError::BindingRequired { .. } | InboundTurnError::AccessDenied { .. }
    ) {
        return TriggerError::BlockedMaterialization {
            reason: "trusted trigger inbound request blocked".to_string(),
        };
    }
    TriggerError::InvalidMaterialization {
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};
    use ironclaw_host_api::{
        execution_policy::TurnExecutionPolicy,
        ids::{AgentId, CapabilityId, ProjectId, TenantId, ThreadId, UserId},
    };
    use ironclaw_triggers::{
        TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
        TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerFire, TriggerFireIdentity, TriggerId,
        TriggerInboundContentRef, TriggerMaterializedPrompt, TrustedTriggerFireSubmitOutcome,
        TrustedTriggerSubmitRequest,
    };
    // Dev-only: `ironclaw_turns` is a dev-dependency here, never a normal one.
    // These fakes stand in for the composition adapter that implements the
    // submission port, so they speak the kernel request the real adapter mints
    // — see `submit_turn_request` below and the manifest comment.
    use ironclaw_turns::{
        AcceptedMessageRef, ReplyTargetBindingRef, RunProfileId, RunProfileRequest,
        RunProfileVersion, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnId,
        TurnOriginKind, TurnRunId, TurnScope, TurnStatus, TurnSurfaceType, product_context,
    };

    use super::{
        classify_trusted_trigger_inbound_error, submit_trusted_trigger_outcome,
        trusted_trigger_fire_submitter,
    };
    use crate::turn_submission::{
        ConversationInboundClassification, ConversationTurnSubmission, ConversationTurnSubmitter,
        TurnSubmissionError, TurnSubmissionErrorCategory, TurnSubmissionRetry,
    };
    use crate::types::{TrustedInboundKind, TrustedInboundTurnRequest};
    use crate::{
        AcceptedConversationMessage, AdapterInstallationId, AdapterKind,
        ConversationBindingResolution, ConversationBindingService, ConversationRouteKind,
        ExternalEventId, InMemoryConversationServices, InboundMessageContentRef, InboundTurnError,
        InboundTurnRequest, InboundTurnResponse, InboundTurnService, LinkConversationRequest,
        LinkedConversationBinding, MessageIdempotencyStatus, ReplyTargetBinding,
        ThreadAccessDecision, ValidateReplyTargetRequest,
    };
    use ironclaw_extension_contracts::external::{ExternalActorRef, ExternalConversationRef};

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
        let (service, _services, _coordinator) = trusted_inbound_service().await;
        let creator = UserId::new("user-creator").expect("user id");

        let request = TrustedInboundTurnRequest::new(
            base_inbound_request(),
            Some(agent()),
            Some(project()),
            Some(creator.clone()),
            TrustedInboundKind::Trigger,
            None,
        );

        let response = service
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
        let (service, _services, _coordinator) = trusted_inbound_service().await;
        let creator = UserId::new("user-creator").expect("user id");

        // First fire: no owner (legacy-shaped binding).
        let first = TrustedInboundTurnRequest::new(
            base_inbound_request(),
            Some(agent()),
            Some(project()),
            None,
            TrustedInboundKind::Trigger,
            None,
        );
        service
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
            None,
        );
        let response = service
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

        let submitter =
            trusted_trigger_fire_submitter(services.clone(), services, coordinator.clone());

        let fire_slot = Utc.with_ymd_and_hms(2026, 6, 1, 9, 0, 0).unwrap();
        let identity = TriggerFireIdentity::new(tenant(), TriggerId::new(), fire_slot);
        let fire = TriggerFire {
            identity: identity.clone(),
            creator_user_id: creator.clone(),
            agent_id: Some(agent()),
            project_id: Some(project()),
            prompt: "test trigger prompt".to_string(),
            execution_policy: Some(TurnExecutionPolicy {
                allowed_capability_ids: Some(vec![
                    CapabilityId::new("mail.list_messages").expect("capability id"),
                ]),
                required_skills: Vec::new(),
                ..TurnExecutionPolicy::default()
            }),
        };
        let content_ref =
            TriggerInboundContentRef::new("content:test-trigger-creator").expect("content ref");
        let materialized_prompt = TriggerMaterializedPrompt::for_fire(&fire, content_ref);
        let request =
            TrustedTriggerSubmitRequest::new_for_test(fire, materialized_prompt, fire_slot)
                .expect("a clean trigger prompt mints a trusted submit request");

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
        // Issue #5505: a trusted trigger fire must request the dedicated
        // scheduled_trigger run profile so the host deny-map (ironclaw_turn_runner
        // runtime.rs) strips the trigger mutator capabilities from the fire's
        // model-visible surface. Assert through the same recording
        // coordinator already used above, on the SubmitTurnRequest that
        // actually reached the coordinator.
        let submissions = coordinator.submissions();
        assert_eq!(submissions.len(), 1);
        assert_eq!(
            submissions[0].requested_run_profile,
            Some(RunProfileRequest::new(RunProfileId::scheduled_trigger().as_str()).unwrap()),
            "trigger fire must request the scheduled_trigger run profile"
        );
        assert_eq!(
            submissions[0]
                .product_context
                .as_ref()
                .and_then(|context| context.execution_policy.as_ref())
                .and_then(|policy| policy.allowed_capability_ids.as_ref())
                .and_then(|ids| ids.first())
                .map(CapabilityId::as_str),
            Some("mail.list_messages"),
            "trusted trigger execution policy must reach the persisted turn request"
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
        // Both retryable port classes, across every category the production
        // adapter can put in them, plus the durable-state failure that is not a
        // submission failure at all. Which `TurnError` lands in which class is
        // the adapter's total mapping, pinned at that seam.
        for error in [
            submission_failure(
                TurnSubmissionErrorCategory::ThreadBusy,
                TurnSubmissionRetry::RetryableAfterKeyRotation,
            ),
            submission_failure(
                TurnSubmissionErrorCategory::AdmissionRejected,
                TurnSubmissionRetry::RetryableAfterKeyRotation,
            ),
            submission_failure(
                TurnSubmissionErrorCategory::Unavailable,
                TurnSubmissionRetry::RetryableAfterKeyRotation,
            ),
            submission_failure(
                TurnSubmissionErrorCategory::CapacityExceeded,
                TurnSubmissionRetry::RetryableWithSameKey,
            ),
            submission_failure(
                TurnSubmissionErrorCategory::Conflict,
                TurnSubmissionRetry::RetryableWithSameKey,
            ),
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

        // The permanent port class, across every category the production
        // adapter can put in it — including `Conflict`, which straddles the two
        // classes (`TurnError::Conflict` is retryable, `LeaseMismatch` and
        // `InvalidTransition` are not) and so proves this classifier reads the
        // retry class rather than the category.
        for error in [
            submission_failure(
                TurnSubmissionErrorCategory::InvalidRequest,
                TurnSubmissionRetry::Permanent,
            ),
            submission_failure(
                TurnSubmissionErrorCategory::Unauthorized,
                TurnSubmissionRetry::Permanent,
            ),
            submission_failure(
                TurnSubmissionErrorCategory::ScopeNotFound,
                TurnSubmissionRetry::Permanent,
            ),
            submission_failure(
                TurnSubmissionErrorCategory::Conflict,
                TurnSubmissionRetry::Permanent,
            ),
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

        for error in [
            InboundTurnError::BindingRequired {
                adapter_kind: TRIGGER_TRUSTED_ADAPTER_KIND.to_string(),
                external_actor_id: "actor".to_string(),
            },
            InboundTurnError::AccessDenied {
                actor_id: "actor".to_string(),
                thread_id: "thread".to_string(),
            },
        ] {
            let classified = classify_trusted_trigger_inbound_error(error);
            assert!(matches!(
                classified,
                ironclaw_triggers::TriggerError::BlockedMaterialization { reason }
                    if reason == "trusted trigger inbound request blocked"
            ));
        }
    }

    /// A submission failure in the given `(category, retry)` class, carrying a
    /// rendered cause the way the production adapter carries the coordinator's.
    fn submission_failure(
        category: TurnSubmissionErrorCategory,
        retry: TurnSubmissionRetry,
    ) -> InboundTurnError {
        InboundTurnError::TurnSubmissionFailed {
            error: TurnSubmissionError::new(category, retry, format!("{category:?} from the host")),
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
                binding_epoch: None,
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
            accepted_message: AcceptedConversationMessage {
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
                event_cursor: ironclaw_host_api::turn::EventCursor(0),
                accepted_message_ref,
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
            None,
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

    /// Returns `(service, services, coordinator)` — a paired `InboundTurnService`
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
        let service =
            InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());
        (service, services, coordinator)
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
        ExternalActorRef::new(
            TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE,
            value,
            None::<String>,
        )
        .unwrap()
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
            unimplemented!("not used by inbound service tests")
        }

        async fn link_conversation_to_thread(
            &self,
            _request: LinkConversationRequest,
        ) -> Result<LinkedConversationBinding, InboundTurnError> {
            unimplemented!("not used by inbound service tests")
        }

        async fn validate_reply_target(
            &self,
            _request: ValidateReplyTargetRequest,
        ) -> Result<ReplyTargetBinding, InboundTurnError> {
            unimplemented!("not used by inbound service tests")
        }
    }

    /// Mirror of the production port adapter
    /// (`ironclaw_composition::automation::conversation_turn_submitter`):
    /// it derives the owner and resolves the classification through the same
    /// `product_context::resolve_inbound` call, producing the same
    /// `SubmitTurnRequest` the real adapter hands the coordinator. The fakes
    /// below record that request, so every run-origin, run-profile and
    /// idempotency-key assertion in this module keeps asserting on the exact
    /// value a coordinator receives rather than a paraphrase of it. The
    /// production copy is pinned by that adapter's own seam tests.
    fn submit_turn_request(submission: ConversationTurnSubmission) -> SubmitTurnRequest {
        let is_trusted_trigger = matches!(
            submission.classification,
            ConversationInboundClassification::TrustedTrigger
        );
        let mut product_context = product_context::resolve_inbound(
            inbound_classification(submission.classification),
            submission.origin_adapter,
            submission.surface_type,
            submission.scope.product_owner(&submission.actor),
        );
        if is_trusted_trigger {
            product_context.execution_policy = submission.execution_policy;
        }
        SubmitTurnRequest {
            requested_model: None,
            scope: submission.scope,
            actor: submission.actor,
            accepted_message_ref: submission.accepted_message_ref,
            requested_run_profile: submission.requested_run_profile,
            output_contract: None,
            idempotency_key: submission.idempotency_key,
            received_at: submission.received_at,
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: Some(product_context),
        }
    }

    fn inbound_classification(
        classification: ConversationInboundClassification,
    ) -> product_context::InboundClassification {
        match classification {
            ConversationInboundClassification::TrustedTrigger => {
                product_context::InboundClassification::TrustedTrigger
            }
            ConversationInboundClassification::TrustedOther => {
                product_context::InboundClassification::TrustedOther
            }
            ConversationInboundClassification::Untrusted => {
                product_context::InboundClassification::Untrusted
            }
        }
    }

    fn accepted_response(request: &SubmitTurnRequest) -> SubmitTurnResponse {
        SubmitTurnResponse::Accepted {
            turn_id: TurnId::new(),
            run_id: TurnRunId::new(),
            status: TurnStatus::Completed,
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: ironclaw_host_api::turn::RunProfileVersion::new(1),
            event_cursor: ironclaw_host_api::turn::EventCursor(0),
            accepted_message_ref: request.accepted_message_ref.clone(),
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
    impl ConversationTurnSubmitter for RecordingTurnCoordinator {
        async fn submit_conversation_turn(
            &self,
            submission: ConversationTurnSubmission,
        ) -> Result<SubmitTurnResponse, TurnSubmissionError> {
            let request = submit_turn_request(submission);
            let response = accepted_response(&request);
            self.submissions.lock().unwrap().push(request);
            Ok(response)
        }
    }

    // --- Tests: error classification ---

    #[test]
    fn classify_trusted_trigger_inbound_error_maps_invalid_run_origin_adapter_to_submit_rejected() {
        // `TurnError::InvalidRunOriginAdapter` arrives through the port in its
        // permanent/invalid-request class (pinned at the adapter seam by
        // `conversation_turn_submitter_maps_every_turn_error_to_its_class`).
        let error = InboundTurnError::TurnSubmissionFailed {
            error: invalid_run_origin_adapter_failure(),
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

        // First call: the port rejects with InvalidRunOriginAdapter's class —
        // inbound returns an error, and the typed port error survives.
        let err = inbound
            .handle_inbound_turn_with_trusted_scope(request.clone())
            .await
            .unwrap_err();
        let InboundTurnError::TurnSubmissionFailed { error } = &err else {
            panic!("expected structured turn submission failure, got: {err:?}");
        };
        assert_eq!(
            error.category(),
            TurnSubmissionErrorCategory::InvalidRequest
        );
        assert_eq!(error.retry(), TurnSubmissionRetry::Permanent);
        assert_eq!(
            error.to_string(),
            "invalid run-origin adapter: must be 1..=512 bytes",
            "the host's rendered cause must survive the port boundary"
        );

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

    /// The submission port's rendering of `TurnError::InvalidRunOriginAdapter`
    /// — the exact `(category, retry, detail)` triple the production adapter
    /// maps that failure onto, pinned there by
    /// `conversation_turn_submitter_maps_every_turn_error_to_its_class`.
    fn invalid_run_origin_adapter_failure() -> TurnSubmissionError {
        TurnSubmissionError::new(
            TurnSubmissionErrorCategory::InvalidRequest,
            TurnSubmissionRetry::Permanent,
            "invalid run-origin adapter: must be 1..=512 bytes",
        )
    }

    /// A submitter that rejects the first submission with the port's rendering
    /// of `TurnError::InvalidRunOriginAdapter` and accepts every later one.
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
    impl ConversationTurnSubmitter for FailingOnFirstTurnCoordinator {
        async fn submit_conversation_turn(
            &self,
            submission: ConversationTurnSubmission,
        ) -> Result<SubmitTurnResponse, TurnSubmissionError> {
            let request = submit_turn_request(submission);
            let mut submissions = self.submissions.lock().unwrap();
            submissions.push(request.clone());
            if submissions.len() == 1 {
                return Err(invalid_run_origin_adapter_failure());
            }
            Ok(accepted_response(&request))
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
        let slack_actor = ExternalActorRef::new("slack", "alice", None::<String>).unwrap();
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
            None,
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
        let slack_actor = ExternalActorRef::new("slack", "user-alice", None::<String>).unwrap();
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
        let telegram_actor =
            ExternalActorRef::new("telegram", "user-alice", None::<String>).unwrap();
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
        let long_actor = ExternalActorRef::new("long", "user-alice", None::<String>).unwrap();
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
