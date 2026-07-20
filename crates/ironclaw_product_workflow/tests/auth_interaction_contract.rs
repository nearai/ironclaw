// arch-exempt: large_file, cross-surface auth interaction contract suite, plan #5905
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthErrorCode, AuthFlowId, AuthFlowKind, AuthFlowManager,
    AuthFlowOutcome, AuthFlowRecord, AuthFlowState, AuthGateRef, AuthProductError,
    AuthProductScope, AuthResolved, AuthSurface, CredentialAccountId, CredentialAccountLabel,
    CredentialAccountProjection, CredentialAccountStatus, CredentialAccountUpdateBinding,
    CredentialOwnership, CredentialSelectionInput, ManualTokenCompletionInput, NewAuthFlow,
    OAuthAuthorizationUrl, OAuthCallbackClaimRequest, OAuthCallbackFailureInput,
    OAuthCallbackInput, TurnRunRef,
};
use ironclaw_host_api::{
    AgentId, ExtensionId, InvocationId, ProjectId, ResourceScope, TenantId, ThreadId, UserId,
};
use ironclaw_product_workflow::{
    AuthGateRecord, AuthInteractionChallengeView, AuthInteractionDecision,
    AuthInteractionReadModel, AuthInteractionRejectionKind, AuthInteractionScope,
    AuthInteractionService, AuthResolutionDispatchOutcome, DefaultAuthInteractionService,
    ListPendingAuthInteractionsRequest, ProductAuthTurnGateResumeDispatcher, ProductWorkflowError,
    ResolveAuthInteractionRequest, ResolveAuthInteractionResponse,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunPrecondition, CancelRunRequest, CancelRunResponse, EventCursor,
    GateRef, GetRunStateRequest, IdempotencyKey, ReplyTargetBindingRef, ResumeTurnPrecondition,
    ResumeTurnRequest, ResumeTurnResponse, RunProfileId, RunProfileVersion, SanitizedCancelReason,
    SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError,
    TurnId, TurnRunId, TurnRunState, TurnScope, TurnStatus,
};
use tokio::sync::Barrier;

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
    complete_before_cancel: Mutex<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestAuthFlowState {
    Open,
    Processing,
    ResolvedAuthorized,
    ResolvedFailed,
    ResolvedExpired,
    ResolvedUserAborted,
}

impl RecordingFlowManager {
    fn new(flow: AuthFlowRecord) -> Self {
        Self {
            flow: Mutex::new(Some(flow)),
            cancellations: Mutex::new(Vec::new()),
            complete_before_cancel: Mutex::new(false),
        }
    }

    fn cancellations(&self) -> Vec<AuthFlowId> {
        self.cancellations.lock().expect("lock").clone()
    }

    fn status(&self) -> TestAuthFlowState {
        let state = self
            .flow
            .lock()
            .expect("lock")
            .as_ref()
            .expect("flow")
            .state;
        match state {
            AuthFlowState::Open => TestAuthFlowState::Open,
            AuthFlowState::Processing => TestAuthFlowState::Processing,
            AuthFlowState::Resolved(AuthFlowOutcome::Authorized { .. }) => {
                TestAuthFlowState::ResolvedAuthorized
            }
            AuthFlowState::Resolved(
                AuthFlowOutcome::ProviderDenied | AuthFlowOutcome::Failed { .. },
            ) => TestAuthFlowState::ResolvedFailed,
            AuthFlowState::Resolved(AuthFlowOutcome::Expired) => TestAuthFlowState::ResolvedExpired,
            AuthFlowState::Resolved(AuthFlowOutcome::UserAborted) => {
                TestAuthFlowState::ResolvedUserAborted
            }
        }
    }

    fn resolution_was_delivered(&self) -> bool {
        self.flow
            .lock()
            .expect("lock")
            .as_ref()
            .is_some_and(|flow| flow.resolution_delivered_at.is_some())
    }

    fn complete_before_cancel(&self) {
        *self.complete_before_cancel.lock().expect("lock") = true;
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
        record.state = AuthFlowState::Resolved(AuthFlowOutcome::Authorized {
            account_id: input.credential_account_id,
        });
        record.updated_at = Utc::now();
        Ok(record.clone())
    }

    async fn complete_manual_token(
        &self,
        _scope: &AuthProductScope,
        _input: ManualTokenCompletionInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        Err(AuthProductError::BackendUnavailable)
    }

    async fn cancel_manual_token(
        &self,
        _scope: &AuthProductScope,
        _interaction_id: ironclaw_auth::AuthInteractionId,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
        Err(AuthProductError::BackendUnavailable)
    }

    async fn fail_oauth_callback(
        &self,
        _scope: &AuthProductScope,
        _input: OAuthCallbackFailureInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        Err(AuthProductError::BackendUnavailable)
    }

    async fn expire_flow(
        &self,
        _scope: &AuthProductScope,
        _flow_id: AuthFlowId,
        _observed_at: ironclaw_auth::Timestamp,
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
        if std::mem::take(&mut *self.complete_before_cancel.lock().expect("lock")) {
            record.state = AuthFlowState::Resolved(AuthFlowOutcome::Authorized {
                account_id: CredentialAccountId::new(),
            });
            record.updated_at = Utc::now();
            return Err(AuthProductError::FlowAlreadyTerminal);
        }
        if let AuthFlowState::Resolved(outcome) = record.state {
            return Err(match outcome {
                AuthFlowOutcome::UserAborted => AuthProductError::Canceled,
                _ => AuthProductError::FlowAlreadyTerminal,
            });
        }
        record.state = AuthFlowState::Resolved(AuthFlowOutcome::UserAborted);
        record.updated_at = Utc::now();
        self.cancellations.lock().expect("lock").push(flow_id);
        Ok(record.clone())
    }

    async fn mark_resolution_delivered(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
        delivered_at: ironclaw_auth::Timestamp,
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
        if record.resolution_delivered_at.is_some() {
            return Ok(record.clone());
        }
        record.resolution_delivered_at = Some(delivered_at);
        record.updated_at = delivered_at;
        Ok(record.clone())
    }
}

struct RecordingTurnCoordinator {
    actor: TurnActor,
    status: Mutex<TurnStatus>,
    gate_ref: Mutex<Option<GateRef>>,
    resumes: Mutex<Vec<ResumeTurnRequest>>,
    cancellations: Mutex<Vec<CancelRunRequest>>,
    get_run_state_error: Mutex<Option<TurnError>>,
    resume_error: Mutex<Option<TurnError>>,
    transition_before_cancel: Mutex<Option<(TurnStatus, Option<GateRef>)>>,
    resume_barrier: Mutex<Option<Arc<Barrier>>>,
    resume_effect_count: Mutex<usize>,
    /// Idempotency cache: maps (run_id, idempotency_key) → cached ResumeTurnResponse.
    /// A second resume_turn call with the same key returns the cached response
    /// before any precondition or status check, mirroring real TurnCoordinator behaviour.
    resume_cache: Mutex<HashMap<(TurnRunId, IdempotencyKey), ResumeTurnResponse>>,
    cancel_cache: Mutex<HashMap<(TurnRunId, IdempotencyKey), CancelRunResponse>>,
}

impl RecordingTurnCoordinator {
    fn blocked_auth(actor: TurnActor, gate_ref: GateRef) -> Self {
        Self {
            actor,
            status: Mutex::new(TurnStatus::BlockedAuth),
            gate_ref: Mutex::new(Some(gate_ref)),
            resumes: Mutex::new(Vec::new()),
            cancellations: Mutex::new(Vec::new()),
            get_run_state_error: Mutex::new(None),
            resume_error: Mutex::new(None),
            transition_before_cancel: Mutex::new(None),
            resume_barrier: Mutex::new(None),
            resume_effect_count: Mutex::new(0),
            resume_cache: Mutex::new(HashMap::new()),
            cancel_cache: Mutex::new(HashMap::new()),
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

    fn set_get_run_state_error(&self, error: TurnError) {
        *self.get_run_state_error.lock().expect("lock") = Some(error);
    }

    fn set_resume_error(&self, error: TurnError) {
        *self.resume_error.lock().expect("lock") = Some(error);
    }

    fn hold_concurrent_resumes_after_reads(&self, barrier: Arc<Barrier>) {
        *self.resume_barrier.lock().expect("lock") = Some(barrier);
    }

    fn resume_effect_count(&self) -> usize {
        *self.resume_effect_count.lock().expect("lock")
    }

    fn transition_before_cancel(&self, status: TurnStatus, gate_ref: Option<GateRef>) {
        *self.transition_before_cancel.lock().expect("lock") = Some((status, gate_ref));
    }

    fn status(&self) -> TurnStatus {
        *self.status.lock().expect("lock")
    }

    fn seed_cancel_cache(
        &self,
        run_id: TurnRunId,
        key: IdempotencyKey,
        response: CancelRunResponse,
    ) {
        self.cancel_cache
            .lock()
            .expect("lock")
            .insert((run_id, key), response);
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
        let cache_key = (run_id, request.idempotency_key.clone());
        self.resumes.lock().expect("lock").push(request);
        let barrier = self.resume_barrier.lock().expect("lock").clone();
        if let Some(barrier) = barrier {
            barrier.wait().await;
        }
        if let Some(error) = self.resume_error.lock().expect("lock").clone() {
            return Err(error);
        }
        // Idempotency: return cached response for a repeated key before any
        // other check, matching real TurnCoordinator behaviour.
        let mut cache = self.resume_cache.lock().expect("lock");
        if let Some(cached) = cache.get(&cache_key).cloned() {
            return Ok(cached);
        }
        let response = ResumeTurnResponse {
            run_id,
            status: TurnStatus::Queued,
            event_cursor: EventCursor(41),
        };
        cache.insert(cache_key, response.clone());
        *self.resume_effect_count.lock().expect("lock") += 1;
        *self.status.lock().expect("lock") = TurnStatus::Queued;
        *self.gate_ref.lock().expect("lock") = None;
        Ok(response)
    }

    async fn retry_turn(
        &self,
        _request: ironclaw_turns::RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        panic!("auth interactions must not retry a turn")
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        let run_id = request.run_id;
        let cache_key = (run_id, request.idempotency_key.clone());
        if let Some(cached) = self
            .cancel_cache
            .lock()
            .expect("lock")
            .get(&cache_key)
            .cloned()
        {
            self.cancellations.lock().expect("lock").push(request);
            return Ok(cached);
        }
        if let Some((status, gate_ref)) = self.transition_before_cancel.lock().expect("lock").take()
        {
            *self.status.lock().expect("lock") = status;
            *self.gate_ref.lock().expect("lock") = gate_ref;
        }
        if let Some(precondition) = request.precondition.as_ref() {
            if self.status() != precondition.required_status() {
                self.cancellations.lock().expect("lock").push(request);
                return Err(TurnError::InvalidTransition {
                    from: self.status(),
                    to: TurnStatus::Cancelled,
                });
            }
            if self.gate_ref.lock().expect("lock").as_ref() != Some(precondition.gate_ref()) {
                self.cancellations.lock().expect("lock").push(request);
                return Err(TurnError::InvalidRequest {
                    reason: "gate cancellation reference mismatch".to_string(),
                });
            }
        }
        self.cancellations.lock().expect("lock").push(request);
        let response = CancelRunResponse {
            run_id,
            status: TurnStatus::Cancelled,
            event_cursor: EventCursor(43),
            already_terminal: false,
            actor: None,
        };
        *self.status.lock().expect("lock") = TurnStatus::Cancelled;
        *self.gate_ref.lock().expect("lock") = None;
        self.cancel_cache
            .lock()
            .expect("lock")
            .insert(cache_key, response.clone());
        Ok(response)
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        if let Some(error) = self.get_run_state_error.lock().expect("lock").clone() {
            return Err(error);
        }
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
            model_usage: None,
            received_at: Utc::now(),
            checkpoint_id: None,
            gate_ref: self.gate_ref.lock().expect("lock").clone(),
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: EventCursor(47),
            product_context: None,
            resume_disposition: None,
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
        TestAuthFlowState::Open,
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
        TestAuthFlowState::Open,
        &turn_scope("bob", "thread-b"),
        &TurnActor::new(UserId::new("bob").unwrap()),
        TurnRunId::new(),
        &make_gate_ref("gate:auth-other"),
        None,
        setup_challenge(),
    );
    let failed = auth_flow(
        TestAuthFlowState::ResolvedFailed,
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
            TestAuthFlowState::Open,
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
            TestAuthFlowState::Open,
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
            TestAuthFlowState::Open,
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
        TestAuthFlowState::ResolvedAuthorized,
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
            gate_ref: gate_ref.clone(),
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
    assert_eq!(resumes[0].source_binding_ref.as_str(), "src:auth");
    assert_eq!(resumes[0].reply_target_binding_ref.as_str(), "reply:auth");
}

#[tokio::test]
async fn exact_gate_invalid_request_is_retried_without_marking_resolution_delivered() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-invalid-request");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-invalid-request");
    let account_id = CredentialAccountId::new();
    let flow = auth_flow(
        TestAuthFlowState::ResolvedAuthorized,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        Some(account_id),
        setup_challenge(),
    );
    let (service, flow_manager, coordinator) =
        service_parts(flow.clone(), vec![flow], actor.clone(), gate_ref.clone());
    coordinator.set_resume_error(TurnError::InvalidRequest {
        reason: "coordinator rejected a non-gate request invariant".to_string(),
    });

    let error = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::CredentialProvided {
                credential_ref: account_id,
            },
            idempotency_key: IdempotencyKey::new("auth-invalid-request").unwrap(),
        })
        .await
        .expect_err("a non-stale coordinator rejection must remain retryable");

    assert!(matches!(
        error,
        ProductWorkflowError::TurnSubmissionFailed {
            error: TurnError::InvalidRequest { .. }
        }
    ));
    assert!(!flow_manager.resolution_was_delivered());
    assert_eq!(coordinator.resumes().len(), 1);
}

#[tokio::test]
async fn credential_selection_completes_pending_auth_gate_before_resume() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-account-selection");
    let account_id = CredentialAccountId::new();
    let flow = auth_flow(
        TestAuthFlowState::Open,
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
    assert_eq!(resumes[0].source_binding_ref.as_str(), "src:auth");
    assert_eq!(resumes[0].reply_target_binding_ref.as_str(), "reply:auth");
}

#[tokio::test]
async fn callback_completed_resumes_completed_auth_gate() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-callback");
    let account_id = CredentialAccountId::new();
    let flow = auth_flow(
        TestAuthFlowState::ResolvedAuthorized,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        Some(account_id),
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
    let account_id = CredentialAccountId::new();
    let flow = auth_flow(
        TestAuthFlowState::ResolvedAuthorized,
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
async fn credential_provided_rejects_completed_flow_with_a_different_account_id() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-missing-account");
    let completed_account_id = CredentialAccountId::new();
    let flow = auth_flow(
        TestAuthFlowState::ResolvedAuthorized,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        Some(completed_account_id),
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
            idempotency_key: IdempotencyKey::new("auth-action-different-account").unwrap(),
        })
        .await
        .expect_err("a different credential account must be rejected");

    assert!(matches!(
        error,
        ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::InvalidCredentialRef
        }
    ));
    assert!(coordinator.resumes().is_empty());
}

#[tokio::test]
async fn deny_on_completed_flow_converges_on_completion() {
    // Race: OAuth flow completed just as the user clicked Deny. Completion is
    // already terminal, so it wins and the parked run follows that path.
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-deny-completed");
    let account_id = CredentialAccountId::new();
    let flow = auth_flow(
        TestAuthFlowState::ResolvedAuthorized,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        Some(account_id),
        setup_challenge(),
    );
    let (service, _flow_manager, coordinator) =
        service_parts(flow.clone(), vec![flow], actor.clone(), gate_ref.clone());

    let response = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::Deny,
            idempotency_key: IdempotencyKey::new("auth-action-deny-completed").unwrap(),
        })
        .await
        .expect("completed flow wins a concurrent denial");

    assert!(matches!(
        response,
        ResolveAuthInteractionResponse::Resumed(_)
    ));
    assert_eq!(coordinator.resumes().len(), 1);
    assert!(coordinator.cancellations().is_empty());
}

#[tokio::test]
async fn completion_wins_when_it_interleaves_before_denial_reservation() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-completion-wins");
    let flow = auth_flow(
        TestAuthFlowState::Open,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        None,
        setup_challenge(),
    );
    let (service, flow_manager, coordinator) =
        service_parts(flow.clone(), vec![flow], actor.clone(), gate_ref.clone());
    flow_manager.complete_before_cancel();

    let response = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::Deny,
            idempotency_key: IdempotencyKey::new("auth-completion-wins").unwrap(),
        })
        .await
        .expect("the completed OAuth flow wins the race");

    assert!(matches!(
        response,
        ResolveAuthInteractionResponse::Resumed(_)
    ));
    assert!(flow_manager.cancellations().is_empty());
    assert!(coordinator.cancellations().is_empty());
    assert_eq!(coordinator.resumes().len(), 1);
}

#[tokio::test]
async fn denied_auth_on_parked_gate_cancels_flow_and_run() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-deny");
    let flow = auth_flow(
        TestAuthFlowState::Open,
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
            gate_ref: gate_ref.clone(),
            decision: AuthInteractionDecision::Deny,
            idempotency_key: IdempotencyKey::new("auth-action-deny").unwrap(),
        })
        .await
        .expect("deny auth on parked gate");

    // Explicit product denial abandons the current request instead of
    // resuming the model, which could immediately request a sibling credential.
    assert!(matches!(
        response,
        ResolveAuthInteractionResponse::Canceled(_)
    ));
    assert_eq!(flow_manager.cancellations().len(), 1);
    assert!(flow_manager.resolution_was_delivered());
    assert!(coordinator.resumes().is_empty());
    let cancellations = coordinator.cancellations();
    assert_eq!(
        cancellations.len(),
        1,
        "explicit denial must cancel the exact blocked run"
    );
    assert_eq!(
        cancellations[0].reason,
        SanitizedCancelReason::UserRequested
    );
    assert_eq!(
        cancellations[0].precondition,
        Some(CancelRunPrecondition::BlockedAuthGate { gate_ref })
    );
}

#[tokio::test]
async fn denied_auth_does_not_cancel_run_that_left_the_gate_before_store_transition() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-race");
    let flow = auth_flow(
        TestAuthFlowState::Open,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        None,
        setup_challenge(),
    );
    let (service, flow_manager, coordinator) =
        service_parts(flow.clone(), vec![flow], actor.clone(), gate_ref);
    coordinator.transition_before_cancel(TurnStatus::Queued, None);

    let error = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref: make_gate_ref("gate:auth-race"),
            decision: AuthInteractionDecision::Deny,
            idempotency_key: IdempotencyKey::new("auth-deny-race").unwrap(),
        })
        .await
        .expect_err("a stale gate decision must not cancel a resumed run");

    assert!(matches!(
        error,
        ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::StaleAuth
        }
    ));
    assert_eq!(flow_manager.cancellations().len(), 1);
    assert_eq!(
        flow_manager.status(),
        TestAuthFlowState::ResolvedUserAborted
    );
    assert!(flow_manager.resolution_was_delivered());
    assert_eq!(coordinator.status(), TurnStatus::Queued);
    assert_eq!(coordinator.cancellations().len(), 1);
}

#[tokio::test]
async fn idempotent_auth_deny_replay_returns_same_canceled_response_as_first_deny() {
    // First Deny (ParkedOnGate + Open) produces UserAborted(R).
    // A second resolve() with the SAME idempotency key (NotParkedOnGate + UserAborted)
    // must return the SAME UserAborted(R) via cancel_run idempotency caching — even
    // though the run is no longer parked.
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-idem-deny-replay");
    let flow = auth_flow(
        TestAuthFlowState::Open,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        None,
        setup_challenge(),
    );
    let (service, flow_manager, coordinator) =
        service_parts(flow.clone(), vec![flow], actor.clone(), gate_ref.clone());

    // ── First call: Deny on a parked gate ─────────────────────────────────────
    let first_response = service
        .resolve(ResolveAuthInteractionRequest {
            scope: scope.clone(),
            actor: actor.clone(),
            run_id_hint: Some(run_id),
            gate_ref: gate_ref.clone(),
            decision: AuthInteractionDecision::Deny,
            idempotency_key: IdempotencyKey::new("idem-auth-deny").unwrap(),
        })
        .await
        .expect("first deny");

    let first_canceled = match &first_response {
        ResolveAuthInteractionResponse::Canceled(r) => r.clone(),
        other => panic!("expected canceled response, got {other:?}"),
    };
    assert_eq!(flow_manager.cancellations().len(), 1);
    assert_eq!(coordinator.cancellations().len(), 1);
    assert!(coordinator.resumes().is_empty());

    // Simulate transition: the run and flow are now canceled.
    coordinator.set_status(TurnStatus::Cancelled);

    // ── Second call: replay with SAME idempotency key ─────────────────────────
    // Use a separate fixture with the gate pre-set to UserAborted flow status.
    let canceled_flow = auth_flow(
        TestAuthFlowState::ResolvedUserAborted,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        None,
        setup_challenge(),
    );
    let (service2, _flow_manager2, coordinator2) = service_parts(
        canceled_flow.clone(),
        vec![canceled_flow],
        actor.clone(),
        gate_ref.clone(),
    );
    coordinator2.set_status(TurnStatus::Cancelled);
    // Seed the cache with the first Deny's response.
    coordinator2.seed_cancel_cache(
        run_id,
        IdempotencyKey::new("idem-auth-deny").unwrap(),
        CancelRunResponse {
            run_id,
            status: TurnStatus::Cancelled,
            event_cursor: EventCursor(43),
            already_terminal: false,
            actor: None,
        },
    );

    let second_error = service2
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::Deny,
            idempotency_key: IdempotencyKey::new("idem-auth-deny").unwrap(),
        })
        .await
        .expect_err("a duplicate terminal delivery is stale at this service layer");

    assert!(matches!(
        second_error,
        ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::StaleAuth
        }
    ));
    assert_eq!(first_canceled.status, TurnStatus::Cancelled);
    assert!(coordinator2.resumes().is_empty());
    assert!(coordinator2.cancellations().is_empty());
}

#[tokio::test]
async fn denied_auth_without_flow_record_cancels_parked_auth_run() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-deny-no-flow");
    let flow = auth_flow(
        TestAuthFlowState::Open,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        None,
        setup_challenge(),
    );
    let (service, flow_manager, coordinator) =
        service_parts(flow, Vec::new(), actor.clone(), gate_ref.clone());

    let error = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::Deny,
            idempotency_key: IdempotencyKey::new("auth-action-deny-no-flow").unwrap(),
        })
        .await
        .expect_err("deny requires a durable auth flow record");

    assert!(matches!(
        error,
        ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::MissingAuth
        }
    ));
    assert!(
        flow_manager.cancellations().is_empty(),
        "no auth flow record should mean there is no flow to cancel"
    );
    assert!(coordinator.resumes().is_empty());
    assert!(coordinator.cancellations().is_empty());
}

#[tokio::test]
async fn denied_auth_without_flow_record_requires_current_parked_auth_gate() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-deny-no-flow-stale");
    let flow = auth_flow(
        TestAuthFlowState::Open,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        None,
        setup_challenge(),
    );
    let (service, _flow_manager, coordinator) =
        service_parts(flow, Vec::new(), actor.clone(), gate_ref.clone());
    coordinator.set_status(TurnStatus::Queued);

    let error = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::Deny,
            idempotency_key: IdempotencyKey::new("auth-action-deny-no-flow-stale").unwrap(),
        })
        .await
        .expect_err("missing auth flow must not cancel a non-parked run");

    assert!(matches!(
        error,
        ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::MissingAuth
        }
    ));
    assert!(coordinator.cancellations().is_empty());
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
        TestAuthFlowState::ResolvedAuthorized,
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

    let error = service
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
        .expect_err("duplicate completed auth resolution is stale");

    assert!(matches!(
        error,
        ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::StaleAuth
        }
    ));
    assert!(coordinator.resumes().is_empty());
    assert_eq!(coordinator.cancellations().len(), 0);
}

#[tokio::test]
async fn denied_auth_on_nonblocked_live_run_is_stale() {
    // A canceled flow alone does not authorize replay cancellation. Live denial
    // uses the atomic BlockedAuthGate precondition; this replay path first
    // requires the exact run to be terminally Cancelled, so a stale Deny can
    // never cancel work that another path already resumed.
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-replay-denied");
    let flow = auth_flow(
        TestAuthFlowState::ResolvedUserAborted,
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

    let error = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::Deny,
            idempotency_key: IdempotencyKey::new("auth-action-replay-denied").unwrap(),
        })
        .await
        .expect_err("stale denial must not cancel a live run");

    assert!(
        matches!(
            error,
            ProductWorkflowError::AuthInteractionRejected {
                kind: AuthInteractionRejectionKind::StaleAuth
            }
        ),
        "expected StaleAuth, got: {error:?}"
    );
    assert!(coordinator.cancellations().is_empty());
    assert!(coordinator.resumes().is_empty());
}

#[tokio::test]
async fn deny_on_canceled_flow_without_deny_marker_returns_stale_auth() {
    // Scenario: NotParkedOnGate + Deny, flow=UserAborted, run is non-terminal,
    // but the flow was canceled by some other path. The live run is protected.
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-cancel-other-path");
    let flow = auth_flow(
        TestAuthFlowState::ResolvedUserAborted,
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

    let error = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::Deny,
            idempotency_key: IdempotencyKey::new("auth-action-cancel-other-path").unwrap(),
        })
        .await
        .expect_err("deny on other-path-canceled flow must be stale");

    assert!(
        matches!(
            error,
            ProductWorkflowError::AuthInteractionRejected {
                kind: AuthInteractionRejectionKind::StaleAuth
            }
        ),
        "expected StaleAuth, got: {error:?}"
    );
    // Must not issue a cancel_run — the run was not parked by us.
    assert_eq!(
        coordinator.cancellations().len(),
        0,
        "must not call cancel_run when the flow was canceled by another path"
    );
    assert!(coordinator.resumes().is_empty());
}

#[tokio::test]
async fn duplicate_denied_auth_on_cancelled_run_with_same_key_returns_canceled() {
    // The first explicit Deny canceled both the flow and run. A retry with the
    // same idempotency key returns the cached cancellation.
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-replay-denied-terminal");
    let flow = auth_flow(
        TestAuthFlowState::ResolvedUserAborted,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        None,
        setup_challenge(),
    );
    let (service, _flow_manager, coordinator) =
        service_parts(flow.clone(), vec![flow], actor.clone(), gate_ref.clone());
    // Run is already in terminal Cancelled state.
    coordinator.set_status(TurnStatus::Cancelled);
    coordinator.seed_cancel_cache(
        run_id,
        IdempotencyKey::new("auth-action-replay-denied-terminal").unwrap(),
        CancelRunResponse {
            run_id,
            status: TurnStatus::Cancelled,
            event_cursor: EventCursor(43),
            already_terminal: false,
            actor: None,
        },
    );

    let error = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::Deny,
            idempotency_key: IdempotencyKey::new("auth-action-replay-denied-terminal").unwrap(),
        })
        .await
        .expect_err("duplicate terminal denial is stale before coordinator dispatch");

    assert!(matches!(
        error,
        ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::StaleAuth
        }
    ));
    assert!(coordinator.cancellations().is_empty());
    assert!(coordinator.resumes().is_empty());
}

#[tokio::test]
async fn deny_on_cancelled_run_with_fresh_key_converges_as_canceled() {
    // A fresh action key is safe once the exact run is already terminally
    // Cancelled. The coordinator can converge on the existing terminal state.
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-replay-cancelled-fresh-key");
    let flow = auth_flow(
        TestAuthFlowState::ResolvedUserAborted,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        None,
        setup_challenge(),
    );
    let (service, _flow_manager, coordinator) =
        service_parts(flow.clone(), vec![flow], actor.clone(), gate_ref.clone());
    coordinator.set_status(TurnStatus::Cancelled);
    let error = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::Deny,
            idempotency_key: IdempotencyKey::new("auth-action-cancelled-fresh-key").unwrap(),
        })
        .await
        .expect_err("fresh-key terminal denial is still stale");

    assert!(matches!(
        error,
        ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::StaleAuth
        }
    ));
    assert!(coordinator.cancellations().is_empty());
    assert!(coordinator.resumes().is_empty());
}

#[tokio::test]
async fn credential_resolution_requires_completed_flow() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-stale");
    let account_id = CredentialAccountId::new();
    let flow = auth_flow(
        TestAuthFlowState::Open,
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
        TestAuthFlowState::ResolvedAuthorized,
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
async fn auth_resolution_uses_the_authoritative_run_state_actor() {
    let caller = TurnActor::new(UserId::new("alice").unwrap());
    let state_actor = TurnActor::new(UserId::new("bob").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-actor-mismatch");
    let account_id = CredentialAccountId::new();
    let flow = auth_flow(
        TestAuthFlowState::ResolvedAuthorized,
        &scope,
        &caller,
        run_id,
        &gate_ref,
        Some(account_id),
        setup_challenge(),
    );
    let (service, _flow_manager, coordinator) = service_parts(
        flow.clone(),
        vec![flow],
        state_actor.clone(),
        gate_ref.clone(),
    );

    let response = service
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
        .expect("the scoped run-state actor is authoritative");

    assert!(matches!(
        response,
        ResolveAuthInteractionResponse::Resumed(_)
    ));
    let resumes = coordinator.resumes();
    assert_eq!(resumes.len(), 1);
    assert_eq!(resumes[0].actor, state_actor);
    assert!(coordinator.cancellations().is_empty());
}

/// WebUI-path parity test (deliverable 3).
///
/// After a capability round-trips through approval and then hits an auth gate,
/// the `AuthInteractionService` must resume with `BlockedAuthGate` — not
/// `BlockedApproval`. Using the wrong precondition would bypass the
/// executor-level guard and could allow a second re-dispatch loop.
///
/// Harness limitation: `auth_interaction_contract.rs` and
/// `approval_interaction_contract.rs` are separate Rust test binaries and
/// cannot share fakes. This test therefore cannot drive the full
/// approval → auth two-service flow in a single binary. The cross-service
/// loop is validated at the executor tier in
/// `ironclaw_agent_loop::executor::tests::
///  auth_resume_after_approval_carries_resume_token_and_approval_request_id`.
///
/// The strongest assertion available at THIS tier: verify that
/// `AuthInteractionService` always emits `BlockedAuthGate` — regardless of
/// what the run's prior approval state was — and calls the coordinator
/// exactly once with that precondition.
#[tokio::test]
async fn auth_resume_after_approval_uses_blocked_auth_gate_precondition() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-approval-auth");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-after-approval");
    let account_id = CredentialAccountId::new();

    // Simulate the state AFTER approval resolved: the run is now BlockedAuth.
    // `service_parts` wires `RecordingTurnCoordinator::blocked_auth`, which
    // returns `TurnStatus::BlockedAuth` from `get_run_state`.
    let flow = auth_flow(
        TestAuthFlowState::ResolvedAuthorized,
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
            idempotency_key: IdempotencyKey::new("auth-after-approval-1").unwrap(),
        })
        .await
        .expect("auth resolve after approval");

    // Auth resolution must succeed and resume the run.
    assert!(matches!(
        response,
        ResolveAuthInteractionResponse::Resumed(_)
    ));

    // Coordinator must be called exactly ONCE with BlockedAuthGate.
    // Any other precondition (e.g. BlockedApproval) would bypass the
    // executor guard that breaks the re-approval loop.
    let resumes = coordinator.resumes();
    assert_eq!(resumes.len(), 1, "resume must be called exactly once");
    assert_eq!(
        resumes[0].precondition,
        ResumeTurnPrecondition::BlockedAuthGate,
        "auth resume must use BlockedAuthGate, not BlockedApproval"
    );

    // Auth resolution must not cancel the run.
    assert!(
        coordinator.cancellations().is_empty(),
        "auth resume must not cancel the run"
    );

    // Auth resolution must not touch the flow's cancel path.
    assert!(
        flow_manager.cancellations().is_empty(),
        "auth resume must not cancel the auth flow"
    );
}

#[tokio::test]
async fn denied_auth_replay_propagates_get_run_state_error_without_canceling() {
    // Scenario: first Deny already canceled the flow (flow=UserAborted, run no
    // longer parked), but when we fetch the run state to replay the outcome
    // the TurnCoordinator returns an error (e.g. backend unavailable).
    // The service must propagate the error and must not cancel the run.
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-deny-get-state-error");
    let flow = auth_flow(
        TestAuthFlowState::ResolvedUserAborted,
        &scope,
        &actor,
        run_id,
        &gate_ref,
        None,
        setup_challenge(),
    );
    // Run is NotParkedOnGate (not BlockedAuth) so the replay arm is entered.
    let (service, _flow_manager, coordinator) =
        service_parts(flow.clone(), vec![flow], actor.clone(), gate_ref.clone());
    // Inject a get_run_state error so the coordinator fails when the service
    // tries to fetch current run state for the idempotent replay.
    coordinator.set_get_run_state_error(TurnError::ScopeNotFound);
    // The run status and disposition do not matter — the error fires first.
    coordinator.set_status(TurnStatus::Queued);

    let result = service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: AuthInteractionDecision::Deny,
            idempotency_key: IdempotencyKey::new("auth-action-deny-get-state-error").unwrap(),
        })
        .await;

    // Must propagate as an Err — not a spurious terminal outcome.
    assert!(
        result.is_err(),
        "get_run_state error must propagate as Err, got: {result:?}"
    );
    assert!(
        !matches!(result, Ok(ResolveAuthInteractionResponse::Canceled(_))),
        "get_run_state error must not produce a spurious cancellation"
    );
    // resume_turn must NOT have been called — no live resume should happen.
    assert_eq!(
        coordinator.resumes().len(),
        0,
        "resume_turn must not be called when get_run_state fails"
    );
    assert!(
        coordinator.cancellations().is_empty(),
        "cancel_run must not be called when get_run_state fails"
    );
}

#[test]
fn auth_gate_record_new_rejects_invalid_continuation_run_and_gate() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:auth-record");
    let valid = auth_flow(
        TestAuthFlowState::Open,
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
    status: TestAuthFlowState,
    scope: &TurnScope,
    actor: &TurnActor,
    run_id: TurnRunId,
    gate_ref: &GateRef,
    credential_account_id: Option<CredentialAccountId>,
    challenge: AuthChallenge,
) -> AuthFlowRecord {
    let now = Utc::now();
    let state = match status {
        TestAuthFlowState::Open => AuthFlowState::Open,
        TestAuthFlowState::Processing => AuthFlowState::Processing,
        TestAuthFlowState::ResolvedAuthorized => {
            AuthFlowState::Resolved(AuthFlowOutcome::Authorized {
                account_id: credential_account_id.expect("authorized fixture needs an account"),
            })
        }
        TestAuthFlowState::ResolvedFailed => AuthFlowState::Resolved(AuthFlowOutcome::Failed {
            error: AuthErrorCode::TokenExchangeFailed,
        }),
        TestAuthFlowState::ResolvedExpired => AuthFlowState::Resolved(AuthFlowOutcome::Expired),
        TestAuthFlowState::ResolvedUserAborted => {
            AuthFlowState::Resolved(AuthFlowOutcome::UserAborted)
        }
    };
    AuthFlowRecord {
        id: AuthFlowId::new(),
        scope: auth_scope(scope, actor),
        kind: AuthFlowKind::IntegrationCredential,
        state,
        provider: provider(),
        challenge: Some(challenge),
        continuation: AuthContinuationRef::TurnGateResume {
            turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
            gate_ref: AuthGateRef::new(gate_ref.as_str()).unwrap(),
        },
        credential_secret_fingerprint: None,
        update_binding: Option::<CredentialAccountUpdateBinding>::None,
        opaque_state_hash: None,
        pkce_verifier_hash: None,
        authorization_code_hash: None,
        resolution_delivered_at: None,
        created_at: now,
        updated_at: now,
        expires_at: now + Duration::minutes(10),
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

fn auth_resolution(
    scope: &TurnScope,
    actor: &TurnActor,
    run_id: TurnRunId,
    gate_ref: &GateRef,
    outcome: AuthFlowOutcome,
) -> AuthResolved {
    AuthResolved {
        flow_id: AuthFlowId::new(),
        scope: auth_scope(scope, actor),
        continuation: AuthContinuationRef::TurnGateResume {
            turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
            gate_ref: AuthGateRef::new(gate_ref.as_str()).unwrap(),
        },
        provider: provider(),
        outcome,
        resolved_at: Utc::now(),
    }
}

#[tokio::test]
async fn exact_auth_resolution_dispatcher_maps_each_terminal_outcome() {
    for (case, outcome, expected_disposition) in [
        (
            "authorized",
            AuthFlowOutcome::Authorized {
                account_id: CredentialAccountId::new(),
            },
            None,
        ),
        (
            "provider_denied",
            AuthFlowOutcome::ProviderDenied,
            Some(ironclaw_turns::GateResumeDisposition::Denied),
        ),
        (
            "expired",
            AuthFlowOutcome::Expired,
            Some(ironclaw_turns::GateResumeDisposition::Error),
        ),
        (
            "failed",
            AuthFlowOutcome::Failed {
                error: AuthErrorCode::TokenExchangeFailed,
            },
            Some(ironclaw_turns::GateResumeDisposition::Error),
        ),
    ] {
        let actor = TurnActor::new(UserId::new("alice").unwrap());
        let scope = turn_scope("alice", &format!("dispatcher-{case}"));
        let run_id = TurnRunId::new();
        let gate_ref = make_gate_ref(&format!("gate:dispatcher-{case}"));
        let coordinator = Arc::new(RecordingTurnCoordinator::blocked_auth(
            actor.clone(),
            gate_ref.clone(),
        ));
        let result = ProductAuthTurnGateResumeDispatcher::new(coordinator.clone())
            .dispatch_auth_resolved(auth_resolution(&scope, &actor, run_id, &gate_ref, outcome))
            .await
            .expect("terminal auth resolution dispatches");

        assert!(matches!(result, AuthResolutionDispatchOutcome::Resumed(_)));
        let resumes = coordinator.resumes();
        assert_eq!(resumes.len(), 1, "{case}");
        assert_eq!(resumes[0].run_id, run_id, "{case}");
        assert_eq!(resumes[0].gate_resolution_ref, gate_ref, "{case}");
        assert_eq!(
            resumes[0].precondition,
            ResumeTurnPrecondition::BlockedAuthGate,
            "{case}"
        );
        assert_eq!(
            resumes[0].resume_disposition, expected_disposition,
            "{case}"
        );
        assert!(coordinator.cancellations().is_empty(), "{case}");
    }
}

#[tokio::test]
async fn user_aborted_dispatches_one_exact_compare_and_cancel() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "dispatcher-user-aborted");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:dispatcher-user-aborted");
    let coordinator = Arc::new(RecordingTurnCoordinator::blocked_auth(
        actor.clone(),
        gate_ref.clone(),
    ));

    let result = ProductAuthTurnGateResumeDispatcher::new(coordinator.clone())
        .dispatch_auth_resolved(auth_resolution(
            &scope,
            &actor,
            run_id,
            &gate_ref,
            AuthFlowOutcome::UserAborted,
        ))
        .await
        .expect("user abort dispatches");

    assert!(matches!(result, AuthResolutionDispatchOutcome::Canceled(_)));
    assert!(coordinator.resumes().is_empty());
    let cancellations = coordinator.cancellations();
    assert_eq!(cancellations.len(), 1);
    assert_eq!(
        cancellations[0].precondition,
        Some(CancelRunPrecondition::BlockedAuthGate { gate_ref })
    );
}

#[tokio::test]
async fn stale_missing_terminal_and_newer_auth_resolution_deliveries_are_ignored() {
    for (case, status, parked_gate, missing) in [
        ("missing", TurnStatus::BlockedAuth, "gate:expected", true),
        ("completed", TurnStatus::Completed, "gate:expected", false),
        ("cancelled", TurnStatus::Cancelled, "gate:expected", false),
        (
            "differently_blocked",
            TurnStatus::BlockedResource,
            "gate:expected",
            false,
        ),
        ("newer_gate", TurnStatus::BlockedAuth, "gate:newer", false),
    ] {
        let actor = TurnActor::new(UserId::new("alice").unwrap());
        let scope = turn_scope("alice", &format!("dispatcher-ignore-{case}"));
        let run_id = TurnRunId::new();
        let expected_gate = make_gate_ref("gate:expected");
        let coordinator = Arc::new(RecordingTurnCoordinator::blocked_auth(
            actor.clone(),
            make_gate_ref(parked_gate),
        ));
        coordinator.set_status(status);
        if missing {
            coordinator.set_get_run_state_error(TurnError::ScopeNotFound);
        }

        let result = ProductAuthTurnGateResumeDispatcher::new(coordinator.clone())
            .dispatch_auth_resolved(auth_resolution(
                &scope,
                &actor,
                run_id,
                &expected_gate,
                AuthFlowOutcome::ProviderDenied,
            ))
            .await
            .expect("stale delivery is a successful no-op");

        assert_eq!(result, AuthResolutionDispatchOutcome::Ignored, "{case}");
        assert!(coordinator.resumes().is_empty(), "{case}");
        assert!(coordinator.cancellations().is_empty(), "{case}");
    }
}

#[tokio::test]
async fn concurrent_duplicate_delivery_replays_one_idempotent_resume_effect() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "dispatcher-duplicate");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:dispatcher-duplicate");
    let coordinator = Arc::new(RecordingTurnCoordinator::blocked_auth(
        actor.clone(),
        gate_ref.clone(),
    ));
    coordinator.hold_concurrent_resumes_after_reads(Arc::new(Barrier::new(2)));
    let dispatcher = ProductAuthTurnGateResumeDispatcher::new(coordinator.clone());
    let event = auth_resolution(
        &scope,
        &actor,
        run_id,
        &gate_ref,
        AuthFlowOutcome::ProviderDenied,
    );
    let (first, second) = tokio::join!(
        dispatcher.dispatch_auth_resolved(event.clone()),
        dispatcher.dispatch_auth_resolved(event),
    );
    let first = first.expect("first delivery returns the operation result");
    let second = second.expect("coordinator replay returns the operation result");

    assert!(matches!(first, AuthResolutionDispatchOutcome::Resumed(_)));
    assert!(matches!(second, AuthResolutionDispatchOutcome::Resumed(_)));
    assert_eq!(coordinator.resumes().len(), 2);
    assert_eq!(coordinator.resume_effect_count(), 1);
    assert_eq!(coordinator.status(), TurnStatus::Queued);
}

#[tokio::test]
async fn final_cancel_race_has_no_effect() {
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let scope = turn_scope("alice", "dispatcher-cancel-race");
    let run_id = TurnRunId::new();
    let gate_ref = make_gate_ref("gate:dispatcher-cancel-race");
    let abort_coordinator = Arc::new(RecordingTurnCoordinator::blocked_auth(
        actor.clone(),
        gate_ref.clone(),
    ));
    abort_coordinator.transition_before_cancel(TurnStatus::Queued, None);
    let raced = ProductAuthTurnGateResumeDispatcher::new(abort_coordinator.clone())
        .dispatch_auth_resolved(auth_resolution(
            &scope,
            &actor,
            run_id,
            &gate_ref,
            AuthFlowOutcome::UserAborted,
        ))
        .await
        .expect("a winning concurrent transition is observed as a no-op");
    assert_eq!(raced, AuthResolutionDispatchOutcome::Ignored);
    assert_eq!(abort_coordinator.cancellations().len(), 1);
}
