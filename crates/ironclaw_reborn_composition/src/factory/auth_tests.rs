use crate::OAuthClientConfig;
use chrono::{Duration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthErrorCode, AuthFlowId, AuthFlowKind, AuthGateRef,
    AuthProductScope, AuthProviderId, AuthSessionId, AuthSurface, AuthorizationCodeHash,
    CredentialAccountLabel, InMemoryAuthProductServices, LifecyclePackageRef, NewAuthFlow,
    OAuthAuthorizationCode, OAuthAuthorizationUrl, OAuthClientId, OAuthProviderCallbackRequest,
    OAuthRedirectUri, OpaqueStateHash, PkceVerifierHash, PkceVerifierSecret, ProviderScope,
    TurnRunRef,
};
use ironclaw_capabilities::{CapabilityObligationHandler, CapabilityObligationRequest};
use ironclaw_events::{InMemorySecurityAuditSink, SecurityBoundary, SecurityDecision};
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, RuntimeHttpEgress, RuntimeHttpEgressError,
    RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, TenantId, ThreadId, UserId,
};
use ironclaw_product_workflow::ProductAuthTurnGateResumeDispatcher;
use ironclaw_secrets::InMemorySecretStore;
use ironclaw_turns::{
    AcceptedMessageRef, BlockedReason, CancelRunRequest, CancelRunResponse, EventCursor, GateRef,
    GetRunStateRequest, IdempotencyKey, LoopCheckpointStateRef, ReplyTargetBindingRef,
    RunProfileId, RunProfileRequest, RunProfileVersion, SourceBindingRef, SubmitTurnRequest,
    SubmitTurnResponse, TurnActor, TurnCheckpointId, TurnCoordinator, TurnError, TurnId,
    TurnLeaseToken, TurnRunId, TurnRunState, TurnRunnerId, TurnScope, TurnStatus,
    runner::{BlockRunRequest, ClaimRunRequest, TurnRunTransitionPort},
};
use secrecy::SecretString;
use std::sync::Mutex;

use crate::auth::AUTH_CONTINUATION_DISPATCH_FAILED_CODE;

use super::*;
use crate::notion_oauth::{NOTION_PROVIDER_ID, notion_provider_spec};
use crate::oauth_provider_client::HostOAuthProviderClient;

#[derive(Clone)]
struct ErrorTurnCoordinator {
    resume_error: TurnError,
}

#[async_trait::async_trait]
impl TurnCoordinator for ErrorTurnCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Ok(TurnRunId::new())
    }

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

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        Ok(auth_error_mapping_run_state(&request))
    }
}

fn auth_error_mapping_run_state(request: &GetRunStateRequest) -> TurnRunState {
    TurnRunState {
        scope: request.scope.clone(),
        actor: Some(TurnActor::new(UserId::new("alice").unwrap())), // safety: fixed test user id literal is valid.
        turn_id: TurnId::new(),
        run_id: request.run_id,
        status: TurnStatus::BlockedAuth,
        accepted_message_ref: AcceptedMessageRef::new("message-auth-error").unwrap(), // safety: fixed test binding literal is valid.
        source_binding_ref: SourceBindingRef::new("source-auth-error").unwrap(), // safety: fixed test binding literal is valid.
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-auth-error").unwrap(), // safety: fixed test binding literal is valid.
        resolved_run_profile_id: RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        resolved_model_route: None,
        received_at: Utc::now(),
        checkpoint_id: None,
        gate_ref: Some(GateRef::new("gate:auth-error").unwrap()), // safety: fixed test gate literal is valid.
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: EventCursor::default(),
        product_context: None,
        resume_disposition: None,
    }
}

#[derive(Debug, Default)]
struct NoopContinuationDispatcher;

#[async_trait::async_trait]
impl RebornAuthContinuationDispatcher for NoopContinuationDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        _event: ironclaw_auth::AuthContinuationEvent,
    ) -> Result<(), ironclaw_auth::AuthProductError> {
        Ok(())
    }
}

#[derive(Debug, Default)]
struct NoopObligationHandler;

#[async_trait::async_trait]
impl CapabilityObligationHandler for NoopObligationHandler {
    async fn satisfy(
        &self,
        _request: CapabilityObligationRequest<'_>,
    ) -> Result<(), ironclaw_capabilities::CapabilityObligationError> {
        Ok(())
    }
}

#[derive(Debug)]
struct RecordingOAuthEgress {
    response_body: Vec<u8>,
    requests: Mutex<Vec<RuntimeHttpEgressRequest>>,
}

impl RecordingOAuthEgress {
    fn ok(response_body: Vec<u8>) -> Self {
        Self {
            response_body,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn single_request(&self) -> RuntimeHttpEgressRequest {
        let requests = self
            .requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if requests.len() != 1 {
            panic!("expected exactly one OAuth egress request");
        }
        requests[0].clone()
    }
}

#[async_trait::async_trait]
impl RuntimeHttpEgress for RecordingOAuthEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        Ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: self.response_body.clone(),
            saved_body: None,
            request_bytes: 0,
            response_bytes: 0,
            redaction_applied: true,
        })
    }
}

#[tokio::test]
async fn local_dev_oauth_turn_gate_callback_resumes_default_turn_coordinator() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_reborn_services(
        RebornBuildInput::local_dev("local-dev-auth-owner", dir.path().join("local-dev"))
            .with_product_auth_ports(in_memory_product_auth_ports()),
    )
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
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
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
                credential_requirements: Vec::new(),
            },
        })
        .await
        .expect("block auth gate");
    let auth_scope = auth_scope_for_turn(&scope, &actor);
    let flow = product_auth
        .flow_manager()
        .create_flow(NewAuthFlow {
            id: None,
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
            scope: auth_scope.clone(),
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
    assert_eq!(state.source_binding_ref.as_str(), "source-auth-callback"); // safety: verifies fixed test binding literal is preserved.
    let reply_binding_ref = state.reply_target_binding_ref.as_str();
    assert_eq!(reply_binding_ref, "reply-auth-callback"); // safety: verifies fixed test binding literal is preserved.
}

#[tokio::test]
async fn local_dev_google_oauth_backend_builds_with_host_provider_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_reborn_services(
        RebornBuildInput::local_dev("local-dev-google-oauth-owner", dir.path().join("local-dev"))
            .with_google_oauth_backend(OAuthClientConfig {
                client_id: OAuthClientId::new("google-client-123").expect("client id"),
                client_secret: None,
                redirect_uri: OAuthRedirectUri::new("https://app.example/oauth/google/callback")
                    .expect("redirect uri"),
                hosted_domain_hint: None,
            }),
    )
    .await
    .expect("local-dev services build");
    assert!(services.product_auth.is_some());
}

#[tokio::test]
async fn local_dev_notion_oauth_backend_builds_with_host_provider_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_reborn_services(
        RebornBuildInput::local_dev("local-dev-notion-oauth-owner", dir.path().join("local-dev"))
            .with_google_oauth_backend(OAuthClientConfig {
                client_id: OAuthClientId::new("google-client-123").expect("client id"),
                client_secret: None,
                redirect_uri: OAuthRedirectUri::new("https://app.example/oauth/google/callback")
                    .expect("redirect uri"),
                hosted_domain_hint: None,
            })
            .with_notion_oauth_backend(OAuthClientConfig {
                client_id: OAuthClientId::new("notion-client-123").expect("client id"),
                client_secret: None,
                redirect_uri: OAuthRedirectUri::new("https://app.example/oauth/notion/callback")
                    .expect("redirect uri"),
                hosted_domain_hint: None,
            }),
    )
    .await
    .expect("local-dev services build");
    assert!(services.product_auth.is_some());
}

#[tokio::test]
async fn local_dev_notion_dcr_oauth_backend_builds_and_wires_registry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_reborn_services(
        RebornBuildInput::local_dev(
            "local-dev-notion-dcr-oauth-owner",
            dir.path().join("local-dev"),
        )
        .with_notion_dcr_oauth_backend("http://127.0.0.1:3000", "Ironclaw")
        .expect("notion dcr config"),
    )
    .await
    .expect("local-dev services build");

    assert!(services.product_auth.is_some());
    assert!(
        services
            .product_auth
            .as_ref()
            .and_then(|product_auth| product_auth.as_auth_challenge_provider())
            .is_some(),
        "DCR-backed product auth must expose the challenge provider projection path"
    );
}

#[tokio::test]
async fn oauth_callback_exchanges_notion_through_reborn_product_auth_boundary() {
    let egress = Arc::new(RecordingOAuthEgress::ok(
        br#"{"access_token":"notion-access","refresh_token":"notion-refresh","expires_in":3600,"token_type":"Bearer"}"#.to_vec(),
    ));
    let provider_client = HostOAuthProviderClient::new(
        notion_provider_spec(),
        egress.clone(),
        Arc::new(InMemorySecretStore::new()),
        Arc::new(NoopObligationHandler),
        OAuthClientId::new("notion-client-123").expect("client id"),
        OAuthRedirectUri::new("https://app.example/oauth/notion/callback").expect("redirect uri"),
    )
    .expect("notion provider client");
    let services = RebornProductAuthServices::from_shared(
        Arc::new(InMemoryAuthProductServices::new()),
        Arc::new(NoopContinuationDispatcher),
    )
    .with_provider_client(Arc::new(provider_client));
    let auth_scope = auth_scope_for_turn(
        &turn_scope(),
        &TurnActor::new(UserId::new("alice").unwrap()),
    );
    let flow_id = create_notion_flow(
        &services,
        auth_scope.clone(),
        AuthContinuationRef::SetupOnly,
    )
    .await;

    let response = services
        .handle_oauth_callback(notion_authorized_request(auth_scope, flow_id))
        .await
        .expect("notion callback succeeds through product auth");

    assert_eq!(response.flow_id, flow_id);
    assert!(response.credential_account_id.is_some());
    let request = egress.single_request();
    assert_eq!(request.url, "https://mcp.notion.com/token");
    let body = form_params(&request.body);
    assert_eq!(
        body.get("grant_type").map(String::as_str),
        Some("authorization_code")
    );
    assert_eq!(
        body.get("resource").map(String::as_str),
        Some("https://mcp.notion.com/mcp")
    );
}

#[tokio::test]
async fn local_dev_google_oauth_backend_accepts_optional_client_secret_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_reborn_services(
        RebornBuildInput::local_dev(
            "local-dev-google-oauth-secret-owner",
            dir.path().join("local-dev"),
        )
        .with_google_oauth_backend(OAuthClientConfig {
            client_id: OAuthClientId::new("google-client-123").expect("client id"),
            client_secret: Some(SecretString::from("raw-client-secret".to_string())),
            redirect_uri: OAuthRedirectUri::new("https://app.example/oauth/google/callback")
                .expect("redirect uri"),
            hosted_domain_hint: None,
        }),
    )
    .await
    .expect("local-dev services build");
    assert!(services.product_auth.is_some());
}

#[tokio::test]
async fn oauth_callback_with_stale_gate_maps_to_terminal_invalid_request() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_reborn_services(
        RebornBuildInput::local_dev("local-dev-auth-stale-owner", dir.path().join("local-dev"))
            .with_product_auth_ports(in_memory_product_auth_ports()),
    )
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
    let services = build_reborn_services(
        RebornBuildInput::local_dev(
            "local-dev-auth-lifecycle-owner",
            dir.path().join("local-dev"),
        )
        .with_product_auth_ports(in_memory_product_auth_ports()),
    )
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
            Arc::new(ProductAuthTurnGateResumeDispatcher::new(coordinator)),
        );
        let security_audit_sink = Arc::new(InMemorySecurityAuditSink::new());
        let services = services.with_security_audit_sink(security_audit_sink.clone());
        let scope = turn_scope();
        let actor = TurnActor::new(UserId::new("alice").unwrap());
        let auth_scope = auth_scope_for_turn(&scope, &actor);
        let expected_scope = auth_scope.resource.clone();
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

        let events = security_audit_sink.snapshot();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].boundary, SecurityBoundary::AuthContinuation);
        assert_eq!(events[0].decision, SecurityDecision::Blocked);
        assert_eq!(events[0].code, AUTH_CONTINUATION_DISPATCH_FAILED_CODE);
        assert_eq!(events[0].scope.as_ref(), Some(&expected_scope));
    }
}

#[cfg(test)]
fn turn_scope() -> TurnScope {
    TurnScope::new_with_owner(
        TenantId::new("tenant-auth").unwrap(),
        Some(AgentId::new("agent-auth").unwrap()),
        Some(ProjectId::new("project-auth").unwrap()),
        ThreadId::new("thread-auth").unwrap(),
        Some(UserId::new("alice").unwrap()), // safety: fixed test user id literal is valid.
    )
}

#[cfg(test)]
fn in_memory_product_auth_ports() -> RebornProductAuthServicePorts {
    RebornProductAuthServicePorts::from_shared(Arc::new(InMemoryAuthProductServices::new()))
}

#[cfg(test)]
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

#[cfg(test)]
fn provider() -> AuthProviderId {
    AuthProviderId::new("github").unwrap()
}

#[cfg(test)]
fn notion_provider() -> AuthProviderId {
    AuthProviderId::new(NOTION_PROVIDER_ID).unwrap()
}

#[cfg(test)]
fn label() -> CredentialAccountLabel {
    CredentialAccountLabel::new("work github").unwrap()
}

#[cfg(test)]
fn notion_label() -> CredentialAccountLabel {
    CredentialAccountLabel::new("work notion").unwrap()
}

#[cfg(test)]
fn authorization_url(value: &str) -> OAuthAuthorizationUrl {
    OAuthAuthorizationUrl::new(value).unwrap()
}

#[cfg(test)]
fn provider_scope(value: &str) -> ProviderScope {
    ProviderScope::new(value).unwrap()
}

#[cfg(test)]
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
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
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
                credential_requirements: Vec::new(),
            },
        })
        .await
        .expect("block auth gate");
    run_id
}

#[cfg(test)]
async fn create_flow(
    product_auth: &RebornProductAuthServices,
    scope: AuthProductScope,
    continuation: AuthContinuationRef,
) -> AuthFlowId {
    product_auth
        .flow_manager()
        .create_flow(NewAuthFlow {
            id: None,
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

async fn create_notion_flow(
    product_auth: &RebornProductAuthServices,
    scope: AuthProductScope,
    continuation: AuthContinuationRef,
) -> AuthFlowId {
    match product_auth
        .flow_manager()
        .create_flow(NewAuthFlow {
            id: None,
            scope,
            kind: AuthFlowKind::IntegrationCredential,
            provider: notion_provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: authorization_url("https://mcp.notion.com/oauth/authorize"),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation,
            update_binding: None,
            opaque_state_hash: Some(state_hash()),
            pkce_verifier_hash: Some(pkce_hash()),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
    {
        Ok(flow) => flow.id,
        Err(error) => panic!("notion auth flow failed: {error:?}"),
    }
}

#[cfg(test)]
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

fn notion_authorized_request(
    scope: AuthProductScope,
    flow_id: AuthFlowId,
) -> crate::RebornOAuthCallbackRequest {
    crate::RebornOAuthCallbackRequest {
        scope,
        flow_id,
        opaque_state_hash: state_hash(),
        outcome: crate::RebornOAuthCallbackOutcome::Authorized {
            provider_request: OAuthProviderCallbackRequest {
                provider: notion_provider(),
                account_label: notion_label(),
                authorization_code: OAuthAuthorizationCode::new(SecretString::from(
                    "raw-notion-auth-code".to_string(),
                ))
                .unwrap(), // safety: test-only fixture literal is valid by construction.
                authorization_code_hash: code_hash(),
                pkce_verifier: PkceVerifierSecret::new(SecretString::from(
                    "raw-notion-pkce-verifier".to_string(),
                ))
                .unwrap(), // safety: test-only fixture literal is valid by construction.
                pkce_verifier_hash: pkce_hash(),
                scopes: vec![provider_scope("workspace")],
            },
        },
    }
}

#[cfg(test)]
fn state_hash() -> OpaqueStateHash {
    OpaqueStateHash::new(fake_digest("state-hash")).unwrap()
}

#[cfg(test)]
fn pkce_hash() -> PkceVerifierHash {
    PkceVerifierHash::new(fake_digest("pkce-hash")).unwrap()
}

#[cfg(test)]
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

fn form_params(body: &[u8]) -> std::collections::BTreeMap<String, String> {
    url::form_urlencoded::parse(body).into_owned().collect()
}
