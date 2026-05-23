use chrono::{Duration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthErrorCode, AuthFlowId, AuthFlowKind, AuthGateRef,
    AuthProductScope, AuthProviderId, AuthSessionId, AuthSurface, AuthorizationCodeHash,
    CredentialAccountLabel, InMemoryAuthProductServices, LifecyclePackageRef, NewAuthFlow,
    OAuthAuthorizationCode, OAuthAuthorizationUrl, OAuthProviderCallbackRequest, OpaqueStateHash,
    PkceVerifierHash, PkceVerifierSecret, ProviderScope, TurnRunRef,
};
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, TenantId, ThreadId, UserId,
};
use ironclaw_turns::{
    AcceptedMessageRef, BlockedReason, CancelRunRequest, CancelRunResponse, GetRunStateRequest,
    IdempotencyKey, LoopCheckpointStateRef, ReplyTargetBindingRef, RunProfileRequest,
    SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCheckpointId,
    TurnCoordinator, TurnError, TurnLeaseToken, TurnRunId, TurnRunState, TurnRunnerId, TurnScope,
    TurnStatus,
    runner::{BlockRunRequest, ClaimRunRequest, TurnRunTransitionPort},
};
use secrecy::SecretString;

use crate::auth::RebornProductWorkflowAuthContinuationDispatcher;

use super::*;

#[derive(Clone)]
struct ErrorTurnCoordinator {
    resume_error: TurnError,
}

#[async_trait::async_trait]
impl TurnCoordinator for ErrorTurnCoordinator {
    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        panic!("submit_turn is not used by auth continuation error mapping tests");
    }

    async fn resume_turn(
        &self,
        _request: ironclaw_turns::ResumeTurnRequest,
    ) -> Result<ironclaw_turns::ResumeTurnResponse, TurnError> {
        Err(self.resume_error.clone())
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        panic!("cancel_run is not used by auth continuation error mapping tests");
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        panic!("get_run_state is not used by auth continuation error mapping tests");
    }
}

#[tokio::test]
async fn local_dev_oauth_turn_gate_callback_resumes_default_turn_coordinator() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "local-dev-auth-owner",
        dir.path().join("local-dev"),
    ))
    .await
    .expect("local-dev services build");
    let product_auth = services.product_auth.as_ref().expect("product auth");
    let turn_coordinator = services
        .turn_coordinator
        .as_ref()
        .expect("turn coordinator");
    let local_runtime = services.local_runtime.as_ref().expect("local runtime");
    let scope = turn_scope();
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let submit = turn_coordinator
        .submit_turn(SubmitTurnRequest {
            scope: scope.clone(),
            actor: actor.clone(),
            accepted_message_ref: AcceptedMessageRef::new("message-auth-callback").unwrap(),
            source_binding_ref: SourceBindingRef::new("source-auth-callback").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-auth-callback").unwrap(),
            requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
            idempotency_key: IdempotencyKey::new("submit-auth-callback").unwrap(),
            received_at: Utc::now(),
        })
        .await
        .expect("submit turn");
    let SubmitTurnResponse::Accepted { run_id, .. } = submit;
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    local_runtime
        .turn_state
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(scope.clone()),
        })
        .await
        .expect("claim run")
        .expect("queued run exists");
    let gate_ref = ironclaw_turns::GateRef::new("gate:auth-callback").unwrap();
    local_runtime
        .turn_state
        .block_run(BlockRunRequest {
            run_id,
            runner_id,
            lease_token,
            checkpoint_id: TurnCheckpointId::new(),
            state_ref: LoopCheckpointStateRef::new("checkpoint:auth-callback").unwrap(),
            reason: BlockedReason::Auth {
                gate_ref: gate_ref.clone(),
            },
        })
        .await
        .expect("block auth gate");
    let auth_scope = auth_scope_for_turn(&scope, &actor);
    let flow = product_auth
        .flow_manager()
        .create_flow(NewAuthFlow {
            scope: auth_scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: authorization_url("https://provider.example/oauth"),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
                gate_ref: AuthGateRef::new(gate_ref.as_str()).unwrap(),
            },
            update_binding: None,
            opaque_state_hash: Some(state_hash()),
            pkce_verifier_hash: Some(pkce_hash()),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect("auth flow");

    let response = product_auth
        .handle_oauth_callback(crate::RebornOAuthCallbackRequest {
            scope: auth_scope,
            flow_id: flow.id,
            opaque_state_hash: state_hash(),
            outcome: crate::RebornOAuthCallbackOutcome::Authorized {
                provider_request: OAuthProviderCallbackRequest {
                    provider: provider(),
                    account_label: label(),
                    authorization_code: OAuthAuthorizationCode::new(SecretString::from(
                        "raw-auth-code".to_string(),
                    ))
                    .unwrap(),
                    authorization_code_hash: code_hash(),
                    pkce_verifier: PkceVerifierSecret::new(SecretString::from(
                        "raw-pkce-verifier".to_string(),
                    ))
                    .unwrap(),
                    pkce_verifier_hash: pkce_hash(),
                    scopes: vec![provider_scope("repo")],
                },
            },
        })
        .await
        .expect("oauth callback succeeds");

    assert_eq!(response.flow_id, flow.id);
    let state = turn_coordinator
        .get_run_state(GetRunStateRequest { scope, run_id })
        .await
        .expect("run state");
    assert_eq!(state.status, TurnStatus::Queued);
    assert_eq!(state.gate_ref, None);
    assert!(
        state
            .source_binding_ref
            .as_str()
            .starts_with("auth-continuation-src:")
    );
}

#[tokio::test]
async fn oauth_callback_with_stale_gate_maps_to_terminal_invalid_request() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "local-dev-auth-stale-owner",
        dir.path().join("local-dev"),
    ))
    .await
    .expect("local-dev services build");
    let product_auth = services.product_auth.as_ref().expect("product auth");
    let turn_coordinator = services
        .turn_coordinator
        .as_ref()
        .expect("turn coordinator");
    let local_runtime = services.local_runtime.as_ref().expect("local runtime");
    let scope = turn_scope();
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let run_id = submit_and_block_auth_run(
        turn_coordinator.as_ref(),
        local_runtime.as_ref(),
        scope.clone(),
        actor.clone(),
        "gate:current-auth",
    )
    .await;
    let auth_scope = auth_scope_for_turn(&scope, &actor);
    let flow_id = create_flow(
        product_auth,
        auth_scope.clone(),
        AuthContinuationRef::TurnGateResume {
            turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
            gate_ref: AuthGateRef::new("gate:stale-auth").unwrap(),
        },
    )
    .await;

    let error = product_auth
        .handle_oauth_callback(authorized_request(auth_scope, flow_id))
        .await
        .expect_err("stale auth gate should not resume");

    assert_eq!(error.code, AuthErrorCode::InvalidRequest);
    assert!(!error.retryable);
}

#[tokio::test]
async fn oauth_callback_with_lifecycle_activation_returns_ok_without_resume() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "local-dev-auth-lifecycle-owner",
        dir.path().join("local-dev"),
    ))
    .await
    .expect("local-dev services build");
    let product_auth = services.product_auth.as_ref().expect("product auth");
    let auth_scope = auth_scope_for_turn(
        &turn_scope(),
        &TurnActor::new(UserId::new("alice").unwrap()),
    );
    let continuation = AuthContinuationRef::LifecycleActivation {
        package_ref: LifecyclePackageRef::new("github-extension").unwrap(),
    };
    let flow_id = create_flow(product_auth, auth_scope.clone(), continuation.clone()).await;

    let response = product_auth
        .handle_oauth_callback(authorized_request(auth_scope, flow_id))
        .await
        .expect("lifecycle continuation is deferred");

    assert_eq!(response.flow_id, flow_id);
    assert_eq!(response.continuation, continuation);
}

#[tokio::test]
async fn oauth_callback_continuation_dispatch_maps_turn_error_categories() {
    for (turn_error, expected_code, expected_retryable) in [
        (
            TurnError::Unavailable {
                reason: "turn coordinator offline".to_string(),
            },
            AuthErrorCode::BackendUnavailable,
            true,
        ),
        (
            TurnError::Unauthorized,
            AuthErrorCode::CrossScopeDenied,
            false,
        ),
        (
            TurnError::ScopeNotFound,
            AuthErrorCode::UnknownOrExpiredFlow,
            false,
        ),
    ] {
        let coordinator = Arc::new(ErrorTurnCoordinator {
            resume_error: turn_error,
        });
        let services = RebornProductAuthServices::from_shared(
            Arc::new(InMemoryAuthProductServices::new()),
            Arc::new(RebornProductWorkflowAuthContinuationDispatcher::new(
                coordinator,
            )),
        );
        let scope = turn_scope();
        let actor = TurnActor::new(UserId::new("alice").unwrap());
        let auth_scope = auth_scope_for_turn(&scope, &actor);
        let flow_id = create_flow(
            &services,
            auth_scope.clone(),
            AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(TurnRunId::new().to_string()).unwrap(),
                gate_ref: AuthGateRef::new("gate:auth-error").unwrap(),
            },
        )
        .await;

        let error = services
            .handle_oauth_callback(authorized_request(auth_scope, flow_id))
            .await
            .expect_err("continuation dispatch error should surface");

        assert_eq!(error.code, expected_code);
        assert_eq!(error.retryable, expected_retryable);
    }
}

fn turn_scope() -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-auth").unwrap(),
        Some(AgentId::new("agent-auth").unwrap()),
        Some(ProjectId::new("project-auth").unwrap()),
        ThreadId::new("thread-auth").unwrap(),
    )
}

fn auth_scope_for_turn(scope: &TurnScope, actor: &TurnActor) -> AuthProductScope {
    AuthProductScope::new(
        ResourceScope {
            tenant_id: scope.tenant_id.clone(),
            user_id: actor.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            mission_id: None,
            thread_id: Some(scope.thread_id.clone()),
            invocation_id: InvocationId::new(),
        },
        AuthSurface::Callback,
    )
    .with_session_id(AuthSessionId::new("session-auth-callback").unwrap())
}

fn provider() -> AuthProviderId {
    AuthProviderId::new("github").unwrap()
}

fn label() -> CredentialAccountLabel {
    CredentialAccountLabel::new("work github").unwrap()
}

fn authorization_url(value: &str) -> OAuthAuthorizationUrl {
    OAuthAuthorizationUrl::new(value).unwrap()
}

fn provider_scope(value: &str) -> ProviderScope {
    ProviderScope::new(value).unwrap()
}

async fn submit_and_block_auth_run(
    turn_coordinator: &dyn ironclaw_turns::TurnCoordinator,
    local_runtime: &RebornLocalRuntimeServices,
    scope: TurnScope,
    actor: TurnActor,
    gate_ref: &str,
) -> ironclaw_turns::TurnRunId {
    let submit = turn_coordinator
        .submit_turn(SubmitTurnRequest {
            scope: scope.clone(),
            actor,
            accepted_message_ref: AcceptedMessageRef::new("message-auth-callback-2").unwrap(),
            source_binding_ref: SourceBindingRef::new("source-auth-callback-2").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-auth-callback-2").unwrap(),
            requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
            idempotency_key: IdempotencyKey::new("submit-auth-callback-2").unwrap(),
            received_at: Utc::now(),
        })
        .await
        .expect("submit turn");
    let SubmitTurnResponse::Accepted { run_id, .. } = submit;
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    local_runtime
        .turn_state
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(scope),
        })
        .await
        .expect("claim run")
        .expect("queued run exists");
    local_runtime
        .turn_state
        .block_run(BlockRunRequest {
            run_id,
            runner_id,
            lease_token,
            checkpoint_id: TurnCheckpointId::new(),
            state_ref: LoopCheckpointStateRef::new("checkpoint:auth-callback-2").unwrap(),
            reason: BlockedReason::Auth {
                gate_ref: ironclaw_turns::GateRef::new(gate_ref).unwrap(),
            },
        })
        .await
        .expect("block auth gate");
    run_id
}

async fn create_flow(
    product_auth: &RebornProductAuthServices,
    scope: AuthProductScope,
    continuation: AuthContinuationRef,
) -> AuthFlowId {
    product_auth
        .flow_manager()
        .create_flow(NewAuthFlow {
            scope,
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: authorization_url("https://provider.example/oauth"),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation,
            update_binding: None,
            opaque_state_hash: Some(state_hash()),
            pkce_verifier_hash: Some(pkce_hash()),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect("auth flow")
        .id
}

fn authorized_request(
    scope: AuthProductScope,
    flow_id: AuthFlowId,
) -> crate::RebornOAuthCallbackRequest {
    crate::RebornOAuthCallbackRequest {
        scope,
        flow_id,
        opaque_state_hash: state_hash(),
        outcome: crate::RebornOAuthCallbackOutcome::Authorized {
            provider_request: OAuthProviderCallbackRequest {
                provider: provider(),
                account_label: label(),
                authorization_code: OAuthAuthorizationCode::new(SecretString::from(
                    "raw-auth-code".to_string(),
                ))
                .unwrap(),
                authorization_code_hash: code_hash(),
                pkce_verifier: PkceVerifierSecret::new(SecretString::from(
                    "raw-pkce-verifier".to_string(),
                ))
                .unwrap(),
                pkce_verifier_hash: pkce_hash(),
                scopes: vec![provider_scope("repo")],
            },
        },
    }
}

fn state_hash() -> OpaqueStateHash {
    OpaqueStateHash::new(fake_digest("state-hash")).unwrap()
}

fn pkce_hash() -> PkceVerifierHash {
    PkceVerifierHash::new(fake_digest("pkce-hash")).unwrap()
}

fn code_hash() -> AuthorizationCodeHash {
    AuthorizationCodeHash::new(fake_digest("code-hash")).unwrap()
}

fn fake_digest(value: &str) -> String {
    format!(
        "{:064x}",
        value.bytes().fold(0_u64, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(u64::from(byte))
        })
    )
}
