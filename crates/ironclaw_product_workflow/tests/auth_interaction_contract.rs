use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthFlowId, AuthFlowKind, AuthFlowManager, AuthFlowRecord,
    AuthFlowStatus, AuthGateRef, AuthProductError, AuthProductScope, AuthSurface,
    CredentialAccountId, CredentialAccountLabel, CredentialAccountProjection,
    CredentialAccountStatus, CredentialAccountUpdateBinding, CredentialOwnership,
    CredentialSelectionInput, NewAuthFlow, OAuthAuthorizationUrl, OAuthCallbackClaimRequest,
    OAuthCallbackFailureInput, OAuthCallbackInput, Timestamp, TurnRunRef,
};
use ironclaw_host_api::{
    AgentId, ExtensionId, InvocationId, ProjectId, ResourceScope, TenantId, ThreadId, UserId,
};
use ironclaw_product_workflow::{
    AuthGateRecord, AuthInteractionChallengeView, AuthInteractionDecision,
    AuthInteractionReadModel, AuthInteractionRejectionKind, AuthInteractionScope,
    AuthInteractionService, DefaultAuthInteractionService, ListPendingAuthInteractionsRequest,
    ProductWorkflowError, ResolveAuthInteractionRequest, ResolveAuthInteractionResponse,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, EventCursor, GateRef,
    GetRunStateRequest, IdempotencyKey, ReplyTargetBindingRef, ResumeTurnPrecondition,
    ResumeTurnRequest, ResumeTurnResponse, RunProfileId, RunProfileVersion, SourceBindingRef,
    SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError, TurnId,
    TurnRunId, TurnRunState, TurnScope, TurnStatus,
};

#[derive(Default)]
struct FakeAuthReadModel {
    gates: Mutex<Vec<AuthGateRecord>>,
}

impl FakeAuthReadModel {
    fn with_gates(gates: Vec<AuthGateRecord>) -> Self {
        Self {
            gates: Mutex::new(gates),
        }
    }
}

#[async_trait]
impl AuthInteractionReadModel for FakeAuthReadModel {
    async fn auth_gates(
        &self,
        _scope: &AuthInteractionScope,
    ) -> Result<Vec<AuthGateRecord>, ProductWorkflowError> {
        Ok(self.gates.lock().expect("lock").clone())
    }

    async fn auth_gate(
        &self,
        _scope: &AuthInteractionScope,
        run_id_hint: Option<TurnRunId>,
        gate_ref: &GateRef,
    ) -> Result<Option<AuthGateRecord>, ProductWorkflowError> {
        Ok(self
            .gates
            .lock()
            .expect("lock")
            .iter()
            .find(|gate| {
                gate.gate_ref() == gate_ref
                    && run_id_hint.is_none_or(|run_id| gate.run_id() == run_id)
            })
            .cloned())
    }
}

struct RecordingFlowManager {
    flow: Mutex<Option<AuthFlowRecord>>,
    cancellations: Mutex<Vec<AuthFlowId>>,
}

impl RecordingFlowManager {
    fn new(flow: AuthFlowRecord) -> Self {
        Self {
            flow: Mutex::new(Some(flow)),
            cancellations: Mutex::new(Vec::new()),
        }
    }

    fn cancellations(&self) -> Vec<AuthFlowId> {
        self.cancellations.lock().expect("lock").clone()
    }
}

#[async_trait]
impl AuthFlowManager for RecordingFlowManager {
    async fn create_flow(&self, _request: NewAuthFlow) -> Result<AuthFlowRecord, AuthProductError> {
        Err(AuthProductError::BackendUnavailable)
    }

    async fn get_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
        let flow = self.flow.lock().expect("lock").clone();
        let Some(flow) = flow else {
            return Ok(None);
        };
        if flow.id != flow_id {
            return Ok(None);
        }
        if &flow.scope != scope {
            return Err(AuthProductError::CrossScopeDenied);
        }
        Ok(Some(flow))
    }

    async fn claim_oauth_callback(
        &self,
        _scope: &AuthProductScope,
        _request: OAuthCallbackClaimRequest,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        Err(AuthProductError::BackendUnavailable)
    }

    async fn complete_oauth_callback(
        &self,
        _scope: &AuthProductScope,
        _input: OAuthCallbackInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        Err(AuthProductError::BackendUnavailable)
    }

    async fn complete_credential_selection(
        &self,
        scope: &AuthProductScope,
        input: CredentialSelectionInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let mut flow = self.flow.lock().expect("lock");
        let Some(record) = flow.as_mut() else {
            return Err(AuthProductError::UnknownOrExpiredFlow);
        };
        if record.id != input.flow_id {
            return Err(AuthProductError::UnknownOrExpiredFlow);
        }
        if &record.scope != scope {
            return Err(AuthProductError::CrossScopeDenied);
        }
        let Some(AuthChallenge::AccountSelectionRequired { accounts, .. }) = &record.challenge
        else {
            return Err(AuthProductError::AccountSelectionRequired);
        };
        if !accounts
            .iter()
            .any(|account| account.id == input.credential_account_id)
        {
            return Err(AuthProductError::CredentialMissing);
        }
        record.status = AuthFlowStatus::Completed;
        record.credential_account_id = Some(input.credential_account_id);
        record.updated_at = Utc::now();
        Ok(record.clone())
    }

    async fn fail_oauth_callback(
        &self,
        _scope: &AuthProductScope,
        _input: OAuthCallbackFailureInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        Err(AuthProductError::BackendUnavailable)
    }

    async fn cancel_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let mut flow = self.flow.lock().expect("lock");
        let Some(record) = flow.as_mut() else {
            return Err(AuthProductError::UnknownOrExpiredFlow);
        };
        if record.id != flow_id {
            return Err(AuthProductError::UnknownOrExpiredFlow);
        }
        if &record.scope != scope {
            return Err(AuthProductError::CrossScopeDenied);
        }
        record.status = AuthFlowStatus::Canceled;
        record.updated_at = Utc::now();
        self.cancellations.lock().expect("lock").push(flow_id);
        Ok(record.clone())
    }

    async fn mark_continuation_dispatched(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
        emitted_at: Timestamp,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let mut flow = self.flow.lock().expect("lock");
        let Some(record) = flow.as_mut() else {
            return Err(AuthProductError::UnknownOrExpiredFlow);
        };
        if record.id != flow_id {
            return Err(AuthProductError::UnknownOrExpiredFlow);
        }
        if &record.scope != scope {
            return Err(AuthProductError::CrossScopeDenied);
        }
        if record.continuation_emitted_at.is_some() {
            return Ok(record.clone());
        }
        record.continuation_emitted_at = Some(emitted_at);
        record.updated_at = emitted_at;
        Ok(record.clone())
    }
}

struct RecordingTurnCoordinator {
    actor: TurnActor,
    status: Mutex<TurnStatus>,
    gate_ref: Mutex<Option<GateRef>>,
    resumes: Mutex<Vec<ResumeTurnRequest>>,
    cancellations: Mutex<Vec<CancelRunRequest>>,
}

impl RecordingTurnCoordinator {
    fn blocked_auth(actor: TurnActor, gate_ref: GateRef) -> Self {
        Self {
            actor,
            status: Mutex::new(TurnStatus::BlockedAuth),
            gate_ref: Mutex::new(Some(gate_ref)),
            resumes: Mutex::new(Vec::new()),
            cancellations: Mutex::new(Vec::new()),
        }
    }

    fn resumes(&self) -> Vec<ResumeTurnRequest> {
        self.resumes.lock().expect("lock").clone()
    }

    fn cancellations(&self) -> Vec<CancelRunRequest> {
        self.cancellations.lock().expect("lock").clone()
    }

    fn set_status(&self, status: TurnStatus) {
        *self.status.lock().expect("lock") = status;
    }
}

#[async_trait]
impl TurnCoordinator for RecordingTurnCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Ok(TurnRunId::new())
    }

    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        panic!("auth interactions must not submit a turn")
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        let run_id = request.run_id;
        self.resumes.lock().expect("lock").push(request);
        Ok(ResumeTurnResponse {
            run_id,
            status: TurnStatus::Queued,
            event_cursor: EventCursor(41),
        })
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        let run_id = request.run_id;
        self.cancellations.lock().expect("lock").push(request);
        Ok(CancelRunResponse {
            run_id,
            status: TurnStatus::Cancelled,
            event_cursor: EventCursor(43),
            already_terminal: false,
            actor: None,
        })
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        Ok(TurnRunState {
            scope: request.scope,
            actor: Some(self.actor.clone()),
            turn_id: TurnId::new(),
            run_id: request.run_id,
            status: *self.status.lock().expect("lock"),
            accepted_message_ref: AcceptedMessageRef::new("msg:auth").expect("valid"),
            source_binding_ref: SourceBindingRef::new("src:auth").expect("valid"),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:auth").expect("valid"),
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
            received_at: Utc::now(),
            checkpoint_id: None,
            gate_ref: self.gate_ref.lock().expect("lock").clone(),
            failure: None,
            event_cursor: EventCursor(47),
        })
    }
}

#[tokio::test]
async fn list_pending_auth_redacts_setup_message_and_filters_scope() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-setup");
    let flow = auth_flow(
        AuthFlowStatus::AwaitingUser,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        None,
        AuthChallenge::SetupRequired {
            provider: provider(),
            message: "RAW_PROMPT_SENTINEL_3094 /tmp/private-auth-path sk-live".to_string(),
        },
    );
    let other = auth_flow(
        AuthFlowStatus::AwaitingUser,
        &turn_scope("bob", "thread-b"),
        &TurnActor::new(UserId::new("bob").unwrap()),
        TurnRunId::new(),
        &make_gate_ref("gate:auth-other"),
        None,
        setup_challenge(),
    );
    let failed = auth_flow(
        AuthFlowStatus::Failed,
        &scope,
        &actor,
        TurnRunId::new(),
        &make_gate_ref("gate:auth-failed"),
        None,
        setup_challenge(),
    );
    let service = service(
        flow.clone(),
        vec![flow, other, failed],
        actor.clone(),
        gate_ref,
    );

    let response = service
        .list_pending(ListPendingAuthInteractionsRequest { scope, actor })
        .await
        .expect("list pending auth");

    assert_eq!(response.auth_interactions.len(), 1);
    let serialized = serde_json::to_string(&response).expect("serialize");
    assert!(!serialized.contains("RAW_PROMPT_SENTINEL_3094"));
    assert!(!serialized.contains("/tmp/private-auth-path"));
    assert!(!serialized.contains("sk-live"));
    assert!(!serialized.contains("gate:auth-failed"));
}

#[tokio::test]
async fn list_pending_auth_projects_challenges_to_minimal_safe_views() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let oauth_gate = make_gate_ref("gate:auth-oauth");
    let manual_gate = make_gate_ref("gate:auth-manual");
    let account_gate = make_gate_ref("gate:auth-account");
    let now = Utc::now();
    let account_id = CredentialAccountId::new();
    let flows = vec![
        auth_flow(
            AuthFlowStatus::AwaitingUser,
            &scope,
            &actor,
            TurnRunId::new(),
            &oauth_gate,
            None,
            AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new(
                    "https://auth.example.test/authorize?state=secret-state&code_challenge=pkce"
                        .to_string(),
                )
                .expect("oauth url"),
                expires_at: now + Duration::minutes(5),
            },
        ),
        auth_flow(
            AuthFlowStatus::AwaitingUser,
            &scope,
            &actor,
            TurnRunId::new(),
            &manual_gate,
            None,
            AuthChallenge::ManualTokenRequired {
                interaction_id: ironclaw_auth::AuthInteractionId::new(),
                provider: provider(),
                label: CredentialAccountLabel::new("private user token label").expect("label"),
                expires_at: now + Duration::minutes(5),
            },
        ),
        auth_flow(
            AuthFlowStatus::AwaitingUser,
            &scope,
            &actor,
            TurnRunId::new(),
            &account_gate,
            None,
            AuthChallenge::AccountSelectionRequired {
                provider: provider(),
                accounts: vec![CredentialAccountProjection {
                    id: account_id,
                    provider: provider(),
                    label: CredentialAccountLabel::new("alice@example.test").expect("label"),
                    status: CredentialAccountStatus::Configured,
                    ownership: CredentialOwnership::UserReusable,
                    owner_extension: Some(ExtensionId::new("private.extension").unwrap()),
                    granted_extensions: vec![ExtensionId::new("granted.extension").unwrap()],
                    secret_handle_count: 2,
                }],
            },
        ),
    ];
    let service = service(
        flows[0].clone(),
        flows.clone(),
        actor.clone(),
        oauth_gate.clone(),
    );

    let response = service
        .list_pending(ListPendingAuthInteractionsRequest { scope, actor })
        .await
        .expect("list pending auth");

    assert_eq!(response.auth_interactions.len(), 3);
    assert!(response.auth_interactions.iter().any(|pending| matches!(
        pending.challenge,
        Some(AuthInteractionChallengeView::OAuthRedirectRequired { .. })
    )));
    let account_view = response
        .auth_interactions
        .iter()
        .find_map(|pending| match &pending.challenge {
            Some(AuthInteractionChallengeView::AccountSelectionRequired { accounts, .. }) => {
                Some(accounts)
            }
            _ => None,
        })
        .expect("account choices");
    assert_eq!(account_view.len(), 1);
    assert_eq!(account_view[0].credential_ref, account_id.to_string());
    assert_eq!(account_view[0].status, CredentialAccountStatus::Configured);
    let serialized = serde_json::to_string(&response).expect("serialize");
    assert!(!serialized.contains("secret-state"));
    assert!(!serialized.contains("code_challenge"));
    assert!(!serialized.contains("private user token label"));
    assert!(!serialized.contains("alice@example.test"));
    assert!(!serialized.contains("private.extension"));
    assert!(!serialized.contains("granted.extension"));
    assert!(!serialized.contains("secret_handle_count"));
}

#[tokio::test]
async fn credential_provided_resumes_completed_auth_gate() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-manual");
    let account_id = CredentialAccountId::new();
    let flow = auth_flow(
        AuthFlowStatus::Completed,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        Some(account_id),
        setup_challenge(),
    );
    let (service, flow_manager, coordinator) =
        service_parts(flow.clone(), vec![flow], actor.clone(), gate_ref.clone());

    let response = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::CredentialProvided {
                credential_ref: account_id,
            },
            idempotency_key: IdempotencyKey::new("auth-action-1").unwrap(),
        })
        .await
        .expect("resolve auth");

    assert!(matches!(
        response,
        ResolveAuthInteractionResponse::Resumed(_)
    ));
    assert!(flow_manager.cancellations().is_empty());
    let resumes = coordinator.resumes();
    assert_eq!(resumes.len(), 1);
    assert_eq!(
        resumes[0].precondition,
        ResumeTurnPrecondition::BlockedAuthGate
    );
}

#[tokio::test]
async fn credential_selection_completes_pending_auth_gate_before_resume() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-account-selection");
    let account_id = CredentialAccountId::new();
    let flow = auth_flow(
        AuthFlowStatus::AwaitingUser,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        None,
        AuthChallenge::AccountSelectionRequired {
            provider: provider(),
            accounts: vec![CredentialAccountProjection {
                id: account_id,
                provider: provider(),
                label: CredentialAccountLabel::new("alice@example.test").expect("label"),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: vec![],
                secret_handle_count: 1,
            }],
        },
    );
    let (service, flow_manager, coordinator) =
        service_parts(flow.clone(), vec![flow], actor.clone(), gate_ref.clone());

    let response = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::CredentialProvided {
                credential_ref: account_id,
            },
            idempotency_key: IdempotencyKey::new("auth-action-selection").unwrap(),
        })
        .await
        .expect("credential selection resumes auth");

    assert!(matches!(
        response,
        ResolveAuthInteractionResponse::Resumed(_)
    ));
    assert!(flow_manager.cancellations().is_empty());
    let resumes = coordinator.resumes();
    assert_eq!(resumes.len(), 1);
    assert_eq!(
        resumes[0].precondition,
        ResumeTurnPrecondition::BlockedAuthGate
    );
}

#[tokio::test]
async fn callback_completed_resumes_completed_auth_gate() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-callback");
    let flow = auth_flow(
        AuthFlowStatus::Completed,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        None,
        setup_challenge(),
    );
    let callback_ref = flow.id;
    let (service, flow_manager, coordinator) =
        service_parts(flow.clone(), vec![flow], actor.clone(), gate_ref.clone());

    let response = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::CallbackCompleted { callback_ref },
            idempotency_key: IdempotencyKey::new("auth-action-callback").unwrap(),
        })
        .await
        .expect("resolve callback auth");

    assert!(matches!(
        response,
        ResolveAuthInteractionResponse::Resumed(_)
    ));
    assert!(flow_manager.cancellations().is_empty());
    assert_eq!(coordinator.resumes().len(), 1);
}

#[tokio::test]
async fn callback_completed_rejects_mismatched_callback_ref() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-callback-mismatch");
    let flow = auth_flow(
        AuthFlowStatus::Completed,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        None,
        setup_challenge(),
    );
    let (service, _flow_manager, coordinator) =
        service_parts(flow.clone(), vec![flow], actor.clone(), gate_ref.clone());

    let error = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::CallbackCompleted {
                callback_ref: AuthFlowId::new(),
            },
            idempotency_key: IdempotencyKey::new("auth-action-callback-wrong").unwrap(),
        })
        .await
        .expect_err("wrong callback ref must be rejected");

    assert!(matches!(
        error,
        ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::InvalidCallbackRef
        }
    ));
    assert!(coordinator.resumes().is_empty());
}

#[tokio::test]
async fn credential_provided_rejects_completed_flow_without_account_id() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-missing-account");
    let flow = auth_flow(
        AuthFlowStatus::Completed,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        None,
        setup_challenge(),
    );
    let (service, _flow_manager, coordinator) =
        service_parts(flow.clone(), vec![flow], actor.clone(), gate_ref.clone());

    let error = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::CredentialProvided {
                credential_ref: CredentialAccountId::new(),
            },
            idempotency_key: IdempotencyKey::new("auth-action-missing-account").unwrap(),
        })
        .await
        .expect_err("missing account id must be stale");

    assert!(matches!(
        error,
        ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::StaleAuth
        }
    ));
    assert!(coordinator.resumes().is_empty());
}

#[tokio::test]
async fn denied_auth_cancels_flow_and_run() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-deny");
    let flow = auth_flow(
        AuthFlowStatus::AwaitingUser,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        None,
        setup_challenge(),
    );
    let (service, flow_manager, coordinator) =
        service_parts(flow.clone(), vec![flow], actor.clone(), gate_ref.clone());

    let response = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::Deny,
            idempotency_key: IdempotencyKey::new("auth-action-deny").unwrap(),
        })
        .await
        .expect("deny auth");

    assert!(matches!(
        response,
        ResolveAuthInteractionResponse::Canceled(_)
    ));
    assert_eq!(flow_manager.cancellations().len(), 1);
    assert_eq!(coordinator.cancellations().len(), 1);
    assert!(coordinator.resumes().is_empty());
}

#[tokio::test]
async fn duplicate_completed_auth_resolution_replays_through_turn_coordinator() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-replay-completed");
    let account_id = CredentialAccountId::new();
    let flow = auth_flow(
        AuthFlowStatus::Completed,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        Some(account_id),
        setup_challenge(),
    );
    let (service, _flow_manager, coordinator) =
        service_parts(flow.clone(), vec![flow], actor.clone(), gate_ref.clone());
    coordinator.set_status(TurnStatus::Queued);

    let response = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::CredentialProvided {
                credential_ref: account_id,
            },
            idempotency_key: IdempotencyKey::new("auth-action-replay-completed").unwrap(),
        })
        .await
        .expect("duplicate completed auth resolution replays");

    assert!(matches!(
        response,
        ResolveAuthInteractionResponse::Resumed(_)
    ));
    assert_eq!(coordinator.resumes().len(), 1);
    assert_eq!(coordinator.cancellations().len(), 0);
}

#[tokio::test]
async fn duplicate_denied_auth_resolution_replays_through_turn_coordinator() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-replay-denied");
    let flow = auth_flow(
        AuthFlowStatus::Canceled,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        None,
        setup_challenge(),
    );
    let (service, _flow_manager, coordinator) =
        service_parts(flow.clone(), vec![flow], actor.clone(), gate_ref.clone());
    coordinator.set_status(TurnStatus::Queued);

    let response = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::Deny,
            idempotency_key: IdempotencyKey::new("auth-action-replay-denied").unwrap(),
        })
        .await
        .expect("duplicate denied auth resolution replays");

    assert!(matches!(
        response,
        ResolveAuthInteractionResponse::Canceled(_)
    ));
    assert_eq!(coordinator.cancellations().len(), 1);
    assert_eq!(coordinator.resumes().len(), 0);
}

#[tokio::test]
async fn credential_resolution_requires_completed_flow() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-stale");
    let account_id = CredentialAccountId::new();
    let flow = auth_flow(
        AuthFlowStatus::AwaitingUser,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        Some(account_id),
        setup_challenge(),
    );
    let (service, _flow_manager, coordinator) =
        service_parts(flow.clone(), vec![flow], actor.clone(), gate_ref.clone());

    let error = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::CredentialProvided {
                credential_ref: account_id,
            },
            idempotency_key: IdempotencyKey::new("auth-action-stale").unwrap(),
        })
        .await
        .expect_err("pending auth must not resume");

    assert!(matches!(
        error,
        ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::StaleAuth
        }
    ));
    assert!(coordinator.resumes().is_empty());
}

#[tokio::test]
async fn cross_scope_auth_gate_is_denied_before_resume() {
    let owner = TurnActor::new(UserId::new("alice").unwrap());
    let owner_scope = turn_scope("alice", "thread-a");
    let caller = TurnActor::new(UserId::new("bob").unwrap());
    let caller_scope = turn_scope("bob", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-cross-scope");
    let account_id = CredentialAccountId::new();
    let flow = auth_flow(
        AuthFlowStatus::Completed,
        &owner_scope,
        &owner,
        run_id,
        &gate_ref,
        Some(account_id),
        setup_challenge(),
    );
    let (service, _flow_manager, coordinator) =
        service_parts(flow.clone(), vec![flow], caller.clone(), gate_ref.clone());

    let error = service
        .resolve(ResolveAuthInteractionRequest {
            scope: caller_scope,
            actor: caller,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::CredentialProvided {
                credential_ref: account_id,
            },
            idempotency_key: IdempotencyKey::new("auth-action-cross-scope").unwrap(),
        })
        .await
        .expect_err("cross-scope auth must be denied");

    assert!(matches!(
        error,
        ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::CrossScopeDenied
        }
    ));
    assert!(coordinator.resumes().is_empty());
}

#[tokio::test]
async fn auth_resolution_rejects_run_state_actor_mismatch() {
    let caller = TurnActor::new(UserId::new("alice").unwrap());
    let state_actor = TurnActor::new(UserId::new("bob").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-actor-mismatch");
    let account_id = CredentialAccountId::new();
    let flow = auth_flow(
        AuthFlowStatus::Completed,
        &scope,
        &caller,
        run_id,
        &gate_ref,
        Some(account_id),
        setup_challenge(),
    );
    let (service, _flow_manager, coordinator) =
        service_parts(flow.clone(), vec![flow], state_actor, gate_ref.clone());

    let error = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor: caller,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::CredentialProvided {
                credential_ref: account_id,
            },
            idempotency_key: IdempotencyKey::new("auth-action-actor-mismatch").unwrap(),
        })
        .await
        .expect_err("run-state actor mismatch must be denied");

    assert!(matches!(
        error,
        ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::CrossScopeDenied
        }
    ));
    assert!(coordinator.resumes().is_empty());
    assert!(coordinator.cancellations().is_empty());
}

#[test]
fn auth_gate_record_new_rejects_invalid_continuation_run_and_gate() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-record");
    let valid = auth_flow(
        AuthFlowStatus::AwaitingUser,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        None,
        setup_challenge(),
    );

    let mut wrong_continuation = valid.clone();
    wrong_continuation.continuation = AuthContinuationRef::SetupOnly;
    let error = AuthGateRecord::new(run_id, gate_ref.clone(), wrong_continuation)
        .expect_err("non turn-gate continuation rejected");
    assert!(matches!(
        error,
        ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::UnsupportedResult
        }
    ));

    let error = AuthGateRecord::new(TurnRunId::new(), gate_ref.clone(), valid.clone())
        .expect_err("mismatched run rejected");
    assert!(matches!(
        error,
        ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::StaleAuth
        }
    ));

    let error = AuthGateRecord::new(run_id, make_gate_ref("gate:auth-wrong"), valid)
        .expect_err("mismatched gate rejected");
    assert!(matches!(
        error,
        ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::InvalidGateRef
        }
    ));
}

fn service(
    flow: AuthFlowRecord,
    gates: Vec<AuthFlowRecord>,
    actor: TurnActor,
    gate_ref: GateRef,
) -> DefaultAuthInteractionService {
    service_parts(flow, gates, actor, gate_ref).0
}

fn service_parts(
    flow: AuthFlowRecord,
    gates: Vec<AuthFlowRecord>,
    actor: TurnActor,
    gate_ref: GateRef,
) -> (
    DefaultAuthInteractionService,
    Arc<RecordingFlowManager>,
    Arc<RecordingTurnCoordinator>,
) {
    let read_model = Arc::new(FakeAuthReadModel::with_gates(
        gates
            .into_iter()
            .map(|flow| {
                let AuthContinuationRef::TurnGateResume { turn_run_ref, .. } = &flow.continuation
                else {
                    panic!("test flow must be turn-gate resume");
                };
                let run_id = uuid::Uuid::parse_str(turn_run_ref.as_str())
                    .map(TurnRunId::from_uuid)
                    .expect("run ref");
                let AuthContinuationRef::TurnGateResume { gate_ref, .. } = &flow.continuation
                else {
                    panic!("test flow must be turn-gate resume");
                };
                AuthGateRecord::new(run_id, GateRef::new(gate_ref.as_str()).unwrap(), flow)
                    .expect("auth gate")
            })
            .collect(),
    ));
    let flow_manager = Arc::new(RecordingFlowManager::new(flow));
    let coordinator = Arc::new(RecordingTurnCoordinator::blocked_auth(actor, gate_ref));
    (
        DefaultAuthInteractionService::new(read_model, flow_manager.clone(), coordinator.clone()),
        flow_manager,
        coordinator,
    )
}

fn auth_flow(
    status: AuthFlowStatus,
    scope: &TurnScope,
    actor: &TurnActor,
    run_id: TurnRunId,
    gate_ref: &GateRef,
    credential_account_id: Option<CredentialAccountId>,
    challenge: AuthChallenge,
) -> AuthFlowRecord {
    let now = Utc::now();
    AuthFlowRecord {
        id: AuthFlowId::new(),
        scope: auth_scope(scope, actor),
        kind: AuthFlowKind::IntegrationCredential,
        status,
        provider: provider(),
        challenge: Some(challenge),
        continuation: AuthContinuationRef::TurnGateResume {
            turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
            gate_ref: AuthGateRef::new(gate_ref.as_str()).unwrap(),
        },
        credential_account_id,
        update_binding: Option::<CredentialAccountUpdateBinding>::None,
        opaque_state_hash: None,
        pkce_verifier_hash: None,
        authorization_code_hash: None,
        error: None,
        created_at: now,
        updated_at: now,
        expires_at: now + Duration::minutes(10),
        continuation_emitted_at: None,
    }
}

fn auth_scope(scope: &TurnScope, actor: &TurnActor) -> AuthProductScope {
    let resource = ResourceScope {
        tenant_id: scope.tenant_id.clone(),
        user_id: actor.user_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        mission_id: None,
        thread_id: Some(scope.thread_id.clone()),
        invocation_id: InvocationId::new(),
    };
    AuthProductScope::new(resource, AuthSurface::Web)
}

fn turn_scope(user: &str, thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-1").unwrap(),
        Some(AgentId::new("agent-1").unwrap()),
        Some(ProjectId::new("project-1").unwrap()),
        ThreadId::new(format!("{thread}-{user}")).unwrap(),
    )
}

fn make_gate_ref(value: &str) -> GateRef {
    GateRef::new(value).unwrap()
}

fn provider() -> ironclaw_auth::AuthProviderId {
    ironclaw_auth::AuthProviderId::new("gmail").unwrap()
}

fn setup_challenge() -> AuthChallenge {
    AuthChallenge::SetupRequired {
        provider: provider(),
        message: "Authenticate to continue".to_string(),
    }
}
