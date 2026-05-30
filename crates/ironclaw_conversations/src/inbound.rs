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

        let (requested_agent_id, requested_project_id, binding_dispatch) =
            binding_policy.into_resolution_parts(requested_agent_id, requested_project_id);
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
        let resolution = match binding_dispatch {
            BindingResolutionDispatch::Untrusted => {
                self.binding_service
                    .resolve_or_create_binding(resolve_request)
                    .await?
            }
            BindingResolutionDispatch::Trusted {
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

enum BindingResolutionDispatch {
    Untrusted,
    Trusted {
        trusted_agent_id: Option<ironclaw_host_api::AgentId>,
        trusted_project_id: Option<ironclaw_host_api::ProjectId>,
    },
}

impl BindingResolutionPolicy {
    fn into_resolution_parts(
        self,
        requested_agent_id: Option<ironclaw_host_api::AgentId>,
        requested_project_id: Option<ironclaw_host_api::ProjectId>,
    ) -> (
        Option<ironclaw_host_api::AgentId>,
        Option<ironclaw_host_api::ProjectId>,
        BindingResolutionDispatch,
    ) {
        match self {
            Self::Untrusted => (
                requested_agent_id,
                requested_project_id,
                BindingResolutionDispatch::Untrusted,
            ),
            Self::Trusted {
                trusted_agent_id,
                trusted_project_id,
            } => (
                None,
                None,
                BindingResolutionDispatch::Trusted {
                    trusted_agent_id,
                    trusted_project_id,
                },
            ),
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
    use ironclaw_turns::{
        CancelRunRequest, CancelRunResponse, EventCursor, GetRunStateRequest, ResumeTurnRequest,
        ResumeTurnResponse, RunProfileId, RunProfileVersion, SubmitTurnRequest, SubmitTurnResponse,
        TurnCoordinator, TurnError, TurnId, TurnRunId, TurnRunState, TurnScope, TurnStatus,
    };

    use crate::{
        AdapterInstallationId, AdapterKind, ConversationBindingResolution,
        ConversationBindingService, ConversationRouteKind, ExternalActorRef,
        ExternalConversationRef, ExternalEventId, InMemoryConversationServices,
        InboundMessageContentRef, InboundTurnError, InboundTurnRequest, InboundTurnService,
        LinkConversationRequest, LinkedConversationBinding, MessageIdempotencyStatus,
        ReplyTargetBinding, TrustedInboundTurnRequest, ValidateReplyTargetRequest,
    };

    #[tokio::test]
    async fn trusted_inbound_with_real_services_creates_binding_records_message_and_replays_submission()
     {
        let services = InMemoryConversationServices::default();
        services
            .pair_external_actor(
                tenant(),
                telegram(),
                default_installation(),
                external_actor("telegram-user-1"),
                user("alice"),
            )
            .await;
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let inbound =
            InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());
        let request = trusted_inbound_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("trusted-chat-1", None),
            "trusted-event-1",
            Some(agent()),
            Some(project()),
        );

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
        assert_eq!(services.accepted_messages().await.len(), 1);
        assert_eq!(coordinator.submissions().len(), 1);
    }

    #[tokio::test]
    async fn trusted_inbound_uses_trusted_binding_resolution_and_replays_duplicate_submission() {
        let services = InMemoryConversationServices::default();
        services
            .pair_external_actor(
                tenant(),
                telegram(),
                default_installation(),
                external_actor("telegram-user-1"),
                user("alice"),
            )
            .await;
        let binding = TrustedOnlyBindingService::new(services.clone());
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let inbound =
            InboundTurnService::new(binding.clone(), services.clone(), coordinator.clone());
        let request = trusted_inbound_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("trusted-chat-1", None),
            "trusted-event-1",
            Some(agent()),
            Some(project()),
        );

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
            vec![(Some(agent()), Some(project()))]
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
    }

    #[tokio::test]
    async fn trusted_inbound_propagates_binding_resolution_failure_without_accepting_or_submitting()
    {
        let services = InMemoryConversationServices::default();
        let binding = RejectingTrustedBindingService::new();
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let inbound =
            InboundTurnService::new(binding.clone(), services.clone(), coordinator.clone());
        let request = trusted_inbound_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("trusted-chat-reject", None),
            "trusted-event-reject",
            Some(agent()),
            Some(project()),
        );

        let err = inbound
            .handle_inbound_turn_with_trusted_scope(request)
            .await
            .unwrap_err();

        assert!(matches!(err, InboundTurnError::BindingRequired { .. }));
        assert_eq!(
            binding.trusted_scopes(),
            vec![(Some(agent()), Some(project()))]
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
                telegram(),
                default_installation(),
                external_actor("telegram-user-1"),
                user("alice"),
            )
            .await;
        let binding = TrustedOnlyBindingService::new(services.clone());
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let inbound =
            InboundTurnService::new(binding.clone(), services.clone(), coordinator.clone());
        let request = trusted_inbound_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("trusted-chat-none", None),
            "trusted-event-none",
            None,
            None,
        );

        inbound
            .handle_inbound_turn_with_trusted_scope(request)
            .await
            .unwrap();

        assert_eq!(binding.trusted_scopes(), vec![(None, None)]);
        let resolve_requests = binding.resolve_requests();
        assert_eq!(resolve_requests.len(), 1);
        assert_eq!(resolve_requests[0].requested_agent_id, None);
        assert_eq!(resolve_requests[0].requested_project_id, None);
    }

    fn trusted_inbound_request(
        adapter_kind: AdapterKind,
        external_actor_ref: ExternalActorRef,
        external_conversation_ref: ExternalConversationRef,
        external_event_id: &str,
        trusted_agent_id: Option<AgentId>,
        trusted_project_id: Option<ProjectId>,
    ) -> TrustedInboundTurnRequest {
        TrustedInboundTurnRequest::new(
            crate::types::trusted_ingress::mint(),
            inbound_request(
                adapter_kind,
                external_actor_ref,
                external_conversation_ref,
                external_event_id,
            ),
            trusted_agent_id,
            trusted_project_id,
        )
    }

    fn inbound_request(
        adapter_kind: AdapterKind,
        external_actor_ref: ExternalActorRef,
        external_conversation_ref: ExternalConversationRef,
        external_event_id: &str,
    ) -> InboundTurnRequest {
        InboundTurnRequest {
            tenant_id: tenant(),
            adapter_kind,
            adapter_installation_id: default_installation(),
            external_actor_ref,
            external_conversation_ref,
            external_event_id: ExternalEventId::new(external_event_id).unwrap(),
            route_kind: ConversationRouteKind::Direct,
            content_ref: InboundMessageContentRef::new(format!("content:{external_event_id}"))
                .unwrap(),
            requested_agent_id: Some(agent()),
            requested_project_id: Some(project()),
            received_at: Utc.with_ymd_and_hms(2026, 5, 6, 12, 0, 0).unwrap(),
            requested_run_profile: None,
        }
    }

    fn tenant() -> TenantId {
        TenantId::new("tenant").unwrap()
    }

    fn telegram() -> AdapterKind {
        AdapterKind::new("telegram").unwrap()
    }

    fn default_installation() -> AdapterInstallationId {
        AdapterInstallationId::new("installation").unwrap()
    }

    fn external_actor(value: &str) -> ExternalActorRef {
        ExternalActorRef::new("user", value).unwrap()
    }

    fn external_conversation(value: &str, message_id: Option<&str>) -> ExternalConversationRef {
        ExternalConversationRef::new(None, value, None, message_id).unwrap()
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

    type TrustedScopeRecord = (Option<AgentId>, Option<ProjectId>);
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

        fn trusted_scopes(&self) -> Vec<(Option<AgentId>, Option<ProjectId>)> {
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
        ) -> Result<ConversationBindingResolution, InboundTurnError> {
            self.resolve_requests.lock().unwrap().push(request.clone());
            self.trusted_scopes
                .lock()
                .unwrap()
                .push((trusted_agent_id.clone(), trusted_project_id.clone()));
            self.inner
                .resolve_or_create_binding_with_trusted_scope(
                    request,
                    trusted_agent_id,
                    trusted_project_id,
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

        fn trusted_scopes(&self) -> Vec<(Option<AgentId>, Option<ProjectId>)> {
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
        ) -> Result<ConversationBindingResolution, InboundTurnError> {
            self.resolve_requests.lock().unwrap().push(request);
            self.trusted_scopes
                .lock()
                .unwrap()
                .push((trusted_agent_id, trusted_project_id));
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
}
