use chrono::{Duration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthFlowKind, AuthGateRef, AuthProductScope,
    AuthProviderId, AuthSessionId, AuthSurface, AuthorizationCodeHash, CredentialAccountLabel,
    NewAuthFlow, OAuthAuthorizationCode, OAuthAuthorizationUrl, OAuthProviderCallbackRequest,
    OpaqueStateHash, PkceVerifierHash, PkceVerifierSecret, ProviderScope, TurnRunRef,
};
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, TenantId, ThreadId, UserId,
};
use ironclaw_turns::{
    AcceptedMessageRef, BlockedReason, GetRunStateRequest, IdempotencyKey, LoopCheckpointStateRef,
    ReplyTargetBindingRef, RunProfileRequest, SourceBindingRef, SubmitTurnRequest,
    SubmitTurnResponse, TurnActor, TurnCheckpointId, TurnLeaseToken, TurnRunnerId, TurnScope,
    TurnStatus,
    runner::{BlockRunRequest, ClaimRunRequest, TurnRunTransitionPort},
};
use secrecy::SecretString;

use super::*;

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
