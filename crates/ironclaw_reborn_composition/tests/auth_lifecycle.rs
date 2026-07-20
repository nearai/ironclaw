use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationEvent, AuthContinuationRef, AuthErrorCode, AuthFlowKind,
    AuthFlowManager, AuthFlowStatus, AuthGateRef, AuthProductError, AuthProductScope,
    AuthProviderId, AuthSessionId, AuthSurface, CredentialAccountLookupRequest,
    CredentialAccountService, CredentialAccountStatus, CredentialOwnership,
    CredentialRefreshRequest, InMemoryAuthProductServices, NewAuthFlow, NewCredentialAccount,
    OAuthAuthorizationUrl, OAuthCallbackFailureInput, OAuthProviderCallbackRequest,
    OAuthProviderExchange, OAuthProviderExchangeContext, OAuthProviderRefresh,
    OAuthProviderRefreshRequest, ProviderScope, SecretCleanupAction, SecretCleanupQuarantineReason,
    SecretCleanupRequest, TurnRunRef, opaque_state_hash,
};
use ironclaw_host_api::{ExtensionId, InvocationId, ResourceScope, SecretHandle, ThreadId, UserId};
use ironclaw_product_workflow::ProductAuthTurnGateResumeDispatcher;
use ironclaw_reborn_composition::{RebornAuthContinuationDispatcher, RebornProductAuthServices};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, GateRef, GateResumeDisposition,
    GetRunStateRequest, ReplyTargetBindingRef, ResumeTurnPrecondition, ResumeTurnRequest,
    ResumeTurnResponse, RetryTurnRequest, RetryTurnResponse, RunProfileId, RunProfileVersion,
    SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError,
    TurnId, TurnRunId, TurnRunState, TurnScope, TurnStatus, events::EventCursor,
};

#[derive(Debug, Default)]
struct NoopContinuationDispatcher;

#[async_trait]
impl RebornAuthContinuationDispatcher for NoopContinuationDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        _event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        Ok(())
    }
}

struct AccessOnlyRefreshProvider;

#[async_trait]
impl ironclaw_auth::AuthProviderClient for AccessOnlyRefreshProvider {
    async fn exchange_callback(
        &self,
        _context: OAuthProviderExchangeContext,
        _request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        Err(AuthProductError::TokenExchangeFailed)
    }

    async fn refresh_token(
        &self,
        request: OAuthProviderRefreshRequest,
    ) -> Result<OAuthProviderRefresh, AuthProductError> {
        Ok(OAuthProviderRefresh {
            provider: request.provider,
            access_secret: SecretHandle::new("google-new-access").unwrap(),
            refresh_secret: None,
            scopes: request.scopes,
        })
    }
}

struct CountingRefreshProvider {
    calls: AtomicUsize,
}

#[async_trait]
impl ironclaw_auth::AuthProviderClient for CountingRefreshProvider {
    async fn exchange_callback(
        &self,
        _context: OAuthProviderExchangeContext,
        _request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        Err(AuthProductError::TokenExchangeFailed)
    }

    async fn refresh_token(
        &self,
        request: OAuthProviderRefreshRequest,
    ) -> Result<OAuthProviderRefresh, AuthProductError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(OAuthProviderRefresh {
            provider: request.provider,
            access_secret: SecretHandle::new("google-counted-access").unwrap(),
            refresh_secret: Some(SecretHandle::new("google-counted-refresh").unwrap()),
            scopes: request.scopes,
        })
    }
}

fn scope(user: &str) -> AuthProductScope {
    AuthProductScope::new(
        ResourceScope::local_default(UserId::new(user).unwrap(), InvocationId::new()).unwrap(),
        AuthSurface::Web,
    )
    .with_session_id(AuthSessionId::new(format!("session-{user}")).unwrap())
}

fn provider() -> AuthProviderId {
    AuthProviderId::new("github").unwrap()
}

fn provider_scope(value: &str) -> ProviderScope {
    ProviderScope::new(value).unwrap()
}

fn auth_services(services: Arc<InMemoryAuthProductServices>) -> RebornProductAuthServices {
    RebornProductAuthServices::from_shared(services, Arc::new(NoopContinuationDispatcher))
}

struct LifecycleTurnCoordinator {
    actor: TurnActor,
    scope: TurnScope,
    run_id: TurnRunId,
    gate_ref: GateRef,
    status: Mutex<TurnStatus>,
    resumes: Mutex<Vec<ResumeTurnRequest>>,
}

impl LifecycleTurnCoordinator {
    fn blocked_auth(
        actor: TurnActor,
        scope: TurnScope,
        run_id: TurnRunId,
        gate_ref: GateRef,
    ) -> Self {
        Self {
            actor,
            scope,
            run_id,
            gate_ref,
            status: Mutex::new(TurnStatus::BlockedAuth),
            resumes: Mutex::new(Vec::new()),
        }
    }

    fn status(&self) -> TurnStatus {
        *self.status.lock().expect("status lock")
    }

    fn resumes(&self) -> Vec<ResumeTurnRequest> {
        self.resumes.lock().expect("resumes lock").clone()
    }
}

#[async_trait]
impl TurnCoordinator for LifecycleTurnCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Ok(TurnRunId::new())
    }

    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        panic!("lifecycle cleanup must not submit turns")
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        self.resumes
            .lock()
            .expect("resumes lock")
            .push(request.clone());
        *self.status.lock().expect("status lock") = TurnStatus::Queued;
        Ok(ResumeTurnResponse {
            run_id: request.run_id,
            status: TurnStatus::Queued,
            event_cursor: EventCursor(2),
        })
    }

    async fn retry_turn(&self, _request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        panic!("lifecycle cleanup must not retry turns")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        panic!("lifecycle cleanup must deny the auth gate, not cancel the run")
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        assert_eq!(request.scope, self.scope);
        assert_eq!(request.run_id, self.run_id);
        Ok(TurnRunState {
            scope: self.scope.clone(),
            actor: Some(self.actor.clone()),
            turn_id: TurnId::new(),
            run_id: self.run_id,
            status: self.status(),
            accepted_message_ref: AcceptedMessageRef::new("msg:lifecycle").unwrap(),
            source_binding_ref: SourceBindingRef::new("src:lifecycle").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:lifecycle").unwrap(),
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
            model_usage: None,
            received_at: Utc::now(),
            checkpoint_id: None,
            gate_ref: Some(self.gate_ref.clone()),
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: EventCursor(1),
            product_context: None,
            resume_disposition: None,
        })
    }
}

#[tokio::test]
async fn lifecycle_uninstall_cancels_turn_flow_and_denies_blocked_auth_gate() {
    assert_lifecycle_uninstall_denies_blocked_auth_gate(false).await;
}

#[tokio::test]
async fn lifecycle_uninstall_denies_failed_turn_flow_and_is_idempotent() {
    assert_lifecycle_uninstall_denies_blocked_auth_gate(true).await;
}

async fn assert_lifecycle_uninstall_denies_blocked_auth_gate(fail_flow_before_uninstall: bool) {
    let auth = Arc::new(InMemoryAuthProductServices::new());
    let actor = TurnActor::new(UserId::new("alice").unwrap());
    let run_id = TurnRunId::new();
    let gate_ref = GateRef::new("gate:lifecycle").unwrap();
    let mut flow_scope = scope("alice");
    let thread_id = ThreadId::new("thread-lifecycle").unwrap();
    flow_scope.resource.thread_id = Some(thread_id.clone());
    let turn_scope = TurnScope::new_with_owner(
        flow_scope.resource.tenant_id.clone(),
        flow_scope.resource.agent_id.clone(),
        flow_scope.resource.project_id.clone(),
        thread_id,
        Some(actor.user_id.clone()),
    );
    let state_hash = opaque_state_hash("lifecycle-state").unwrap();
    let flow = auth
        .create_flow(NewAuthFlow {
            id: None,
            scope: flow_scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new(
                    "https://example.com/oauth/authorize",
                )
                .unwrap(),
                expires_at: Utc::now() + Duration::minutes(10),
            },
            continuation: AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
                gate_ref: AuthGateRef::new(gate_ref.as_str()).unwrap(),
            },
            update_binding: None,
            opaque_state_hash: Some(state_hash.clone()),
            pkce_verifier_hash: None,
            expires_at: Utc::now() + Duration::minutes(10),
        })
        .await
        .unwrap();
    if fail_flow_before_uninstall {
        auth.fail_oauth_callback(
            &flow_scope,
            OAuthCallbackFailureInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash,
                error: AuthErrorCode::TokenExchangeFailed,
            },
        )
        .await
        .expect("mark callback failed");
    }
    let coordinator = Arc::new(LifecycleTurnCoordinator::blocked_auth(
        actor.clone(),
        turn_scope,
        run_id,
        gate_ref,
    ));
    let dispatcher = Arc::new(ProductAuthTurnGateResumeDispatcher::new(
        coordinator.clone(),
    ));
    let services = RebornProductAuthServices::from_shared(auth.clone(), dispatcher);

    let report = services
        .cleanup_credentials_for_lifecycle(SecretCleanupRequest {
            scope: flow_scope.clone(),
            extension_id: ExtensionId::new("github").unwrap(),
            provider: Some(provider()),
            action: SecretCleanupAction::Uninstall,
        })
        .await
        .unwrap();

    let canceled = auth.get_flow(&flow_scope, flow.id).await.unwrap().unwrap();
    assert_eq!(
        canceled.status,
        if fail_flow_before_uninstall {
            AuthFlowStatus::Failed
        } else {
            AuthFlowStatus::Canceled
        }
    );
    assert!(canceled.continuation_emitted_at.is_some());
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("canceled_turn_gate_continuations")
    );
    assert_eq!(coordinator.status(), TurnStatus::Queued);
    let resumes = coordinator.resumes();
    assert_eq!(resumes.len(), 1);
    assert_eq!(resumes[0].actor, actor);
    assert_eq!(
        resumes[0].precondition,
        ResumeTurnPrecondition::BlockedAuthGate
    );
    assert_eq!(
        resumes[0].resume_disposition,
        Some(GateResumeDisposition::Denied)
    );

    services
        .cleanup_credentials_for_lifecycle(SecretCleanupRequest {
            scope: flow_scope,
            extension_id: ExtensionId::new("github").unwrap(),
            provider: Some(provider()),
            action: SecretCleanupAction::Uninstall,
        })
        .await
        .expect("lifecycle cleanup retry");
    assert_eq!(
        coordinator.resumes().len(),
        1,
        "continuation marking must make lifecycle cleanup idempotent"
    );
}

#[tokio::test]
async fn refresh_credential_account_uses_product_auth_facade_and_redacts_response() {
    let auth = Arc::new(InMemoryAuthProductServices::new());
    let owner = scope("alice");
    let old_access = SecretHandle::new("github-facade-old-access").unwrap();
    let old_refresh = SecretHandle::new("github-facade-old-refresh").unwrap();
    let account = auth
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: ironclaw_auth::CredentialAccountLabel::new("work").unwrap(),
            status: CredentialAccountStatus::Expired,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(old_access.clone()),
            refresh_secret: Some(old_refresh.clone()),
            scopes: vec![provider_scope("repo")],
        })
        .await
        .unwrap();
    let services = auth_services(Arc::clone(&auth));

    let report = services
        .refresh_credential_account(CredentialRefreshRequest::new(
            owner.clone(),
            provider(),
            account.id,
        ))
        .await
        .unwrap();

    assert!(report.refreshed);
    assert_eq!(report.account.id, account.id);
    assert_eq!(report.account.status, CredentialAccountStatus::Configured);
    let stored = auth
        .get_account(CredentialAccountLookupRequest::new(
            owner.clone(),
            account.id,
        ))
        .await
        .unwrap()
        .expect("refreshed account");
    assert_eq!(stored.status, CredentialAccountStatus::Configured);
    assert_ne!(stored.access_secret, Some(old_access));
    assert_ne!(stored.refresh_secret, Some(old_refresh));

    let serialized = serde_json::to_string(&report).unwrap();
    assert!(!serialized.contains("github-facade-old-access"));
    assert!(!serialized.contains("github-facade-old-refresh"));
    assert!(!serialized.contains("oauth-refreshed"));
}

#[tokio::test]
async fn refresh_credential_account_maps_facade_errors_to_stable_codes() {
    let auth = Arc::new(InMemoryAuthProductServices::new());
    let owner = scope("alice");
    let account = auth
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: ironclaw_auth::CredentialAccountLabel::new("shared").unwrap(),
            status: CredentialAccountStatus::Expired,
            ownership: CredentialOwnership::SharedAdminManaged,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-shared-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("github-shared-refresh").unwrap()),
            scopes: vec![provider_scope("repo")],
        })
        .await
        .unwrap();
    let services = auth_services(auth);

    let error = services
        .refresh_credential_account(CredentialRefreshRequest::new(owner, provider(), account.id))
        .await
        .unwrap_err();

    assert_eq!(error.code, ironclaw_auth::AuthErrorCode::CrossScopeDenied);
    assert!(!error.retryable);
    let serialized = serde_json::to_string(&error).unwrap();
    assert!(!serialized.contains("github-shared-access"));
    assert!(!serialized.contains("github-shared-refresh"));
}

#[tokio::test]
async fn refresh_credential_account_rejects_system_owned_accounts_before_provider_call() {
    let auth = Arc::new(InMemoryAuthProductServices::new());
    let owner = scope("alice");
    let account = auth
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: ironclaw_auth::CredentialAccountLabel::new("system").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::System,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-system-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("github-system-refresh").unwrap()),
            scopes: vec![provider_scope("repo")],
        })
        .await
        .unwrap();
    let refresh_provider = Arc::new(CountingRefreshProvider {
        calls: AtomicUsize::new(0),
    });
    let services = auth_services(Arc::clone(&auth)).with_provider_client(refresh_provider.clone());

    let error = services
        .refresh_credential_account(CredentialRefreshRequest::new(
            owner.clone(),
            provider(),
            account.id,
        ))
        .await
        .expect_err("system-owned accounts cannot refresh");

    assert_eq!(error.code, ironclaw_auth::AuthErrorCode::CrossScopeDenied);
    assert_eq!(refresh_provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn refresh_credential_account_with_provider_keeps_existing_refresh_handle_when_omitted() {
    let auth = Arc::new(InMemoryAuthProductServices::new());
    let owner = scope("alice");
    let old_refresh = SecretHandle::new("google-existing-refresh").unwrap();
    let account = auth
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: ironclaw_auth::CredentialAccountLabel::new("work").unwrap(),
            status: CredentialAccountStatus::Expired,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("google-old-access").unwrap()),
            refresh_secret: Some(old_refresh.clone()),
            scopes: vec![provider_scope("repo")],
        })
        .await
        .unwrap();
    let services =
        auth_services(Arc::clone(&auth)).with_provider_client(Arc::new(AccessOnlyRefreshProvider));

    let report = services
        .refresh_credential_account(CredentialRefreshRequest::new(
            owner.clone(),
            provider(),
            account.id,
        ))
        .await
        .unwrap();

    assert!(report.refreshed);
    assert_eq!(report.account.status, CredentialAccountStatus::Configured);
    let stored = auth
        .get_account(CredentialAccountLookupRequest::new(owner, account.id))
        .await
        .unwrap()
        .expect("refreshed account");
    assert_eq!(
        stored.access_secret,
        Some(SecretHandle::new("google-new-access").unwrap())
    );
    assert_eq!(stored.refresh_secret, Some(old_refresh));
}

#[tokio::test]
async fn cleanup_credentials_for_lifecycle_uses_facade_and_quarantine_report() {
    let auth = Arc::new(InMemoryAuthProductServices::new());
    let owner = scope("alice");
    let extension = ExtensionId::new("github").unwrap();
    let owned = auth
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: ironclaw_auth::CredentialAccountLabel::new("owned").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(extension.clone()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-owned-facade").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let quarantined = auth
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: ironclaw_auth::CredentialAccountLabel::new("quarantine").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(extension.clone()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-quarantined-facade").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    auth.quarantine_cleanup_for_tests(
        quarantined.id,
        SecretCleanupQuarantineReason::TombstoneFailed,
    );
    let services = auth_services(Arc::clone(&auth));

    let report = services
        .cleanup_credentials_for_lifecycle(SecretCleanupRequest {
            scope: owner.clone(),
            extension_id: extension.clone(),
            provider: None,
            action: SecretCleanupAction::Uninstall,
        })
        .await
        .unwrap();

    assert_eq!(report.revoked_accounts, vec![owned.id]);
    assert_eq!(report.quarantined_accounts.len(), 1);
    assert_eq!(report.quarantined_accounts[0].account_id, quarantined.id);
    assert_eq!(
        report.quarantined_accounts[0].reason,
        SecretCleanupQuarantineReason::TombstoneFailed
    );
    let owned_after = auth
        .get_account(
            CredentialAccountLookupRequest::new(owner.clone(), owned.id)
                .for_extension(extension.clone()),
        )
        .await
        .unwrap()
        .expect("owned account");
    assert_eq!(owned_after.status, CredentialAccountStatus::Revoked);
    let quarantined_after = auth
        .get_account(
            CredentialAccountLookupRequest::new(owner.clone(), quarantined.id)
                .for_extension(extension),
        )
        .await
        .unwrap()
        .expect("quarantined account");
    assert_eq!(
        quarantined_after.status,
        CredentialAccountStatus::Configured
    );

    let serialized = serde_json::to_string(&report).unwrap();
    assert!(!serialized.contains("github-owned-facade"));
    assert!(!serialized.contains("github-quarantined-facade"));
}
