use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationEvent, AuthContinuationRef, AuthErrorCode, AuthFlowId,
    AuthFlowManager, AuthFlowRecord, AuthFlowStatus, AuthInteractionId, AuthInteractionService,
    AuthProductError, AuthProductScope, AuthProviderClient, AuthProviderId, AuthSessionId,
    AuthSurface, CredentialAccountLabel, CredentialAccountListRequest, CredentialAccountService,
    CredentialAccountStatus, CredentialAccountUpdateBinding, CredentialOwnership,
    CredentialSelectionInput, CredentialSetupService, InMemoryAuthProductServices,
    ManualTokenCompletionInput, ManualTokenSetupRequest, NewAuthFlow, NewCredentialAccount,
    OAuthAuthorizationUrl, OAuthCallbackClaimRequest, OAuthCallbackFailureInput,
    OAuthCallbackInput, SecretCleanupService, SecretSubmitRequest, SecretSubmitResult, Timestamp,
};
use ironclaw_host_api::{InvocationId, ResourceScope, SecretHandle, UserId};
use ironclaw_reborn_composition::{
    RebornAuthContinuationDispatcher, RebornManualTokenError, RebornManualTokenSetupRequest,
    RebornManualTokenSubmitRequest, RebornManualTokenSubmitResponse, RebornProductAuthServices,
};
use secrecy::SecretString;

const RAW_TOKEN: &str = "super-secret-manual-token";

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

#[derive(Debug, Default)]
struct RecordingContinuationDispatcher {
    events: Mutex<Vec<AuthContinuationEvent>>,
}

impl RecordingContinuationDispatcher {
    fn events(&self) -> Vec<AuthContinuationEvent> {
        self.events.lock().unwrap().clone()
    }
}

#[async_trait]
impl RebornAuthContinuationDispatcher for RecordingContinuationDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

#[derive(Debug, Default)]
struct FailsFirstContinuationDispatcher {
    attempts: Mutex<usize>,
    events: Mutex<Vec<AuthContinuationEvent>>,
}

impl FailsFirstContinuationDispatcher {
    fn events(&self) -> Vec<AuthContinuationEvent> {
        self.events.lock().unwrap().clone()
    }
}

#[async_trait]
impl RebornAuthContinuationDispatcher for FailsFirstContinuationDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        let mut attempts = self.attempts.lock().unwrap();
        *attempts += 1;
        if *attempts == 1 {
            return Err(AuthProductError::BackendUnavailable);
        }
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

#[derive(Debug)]
struct FailingInteractionService {
    error: AuthProductError,
}

#[async_trait]
impl AuthInteractionService for FailingInteractionService {
    async fn request_secret_input(
        &self,
        _request: ManualTokenSetupRequest,
    ) -> Result<AuthChallenge, AuthProductError> {
        Err(self.error.clone())
    }

    async fn submit_manual_token(
        &self,
        _scope: &AuthProductScope,
        _request: SecretSubmitRequest,
    ) -> Result<SecretSubmitResult, AuthProductError> {
        Err(self.error.clone())
    }

    async fn abandon_manual_token(
        &self,
        _scope: &AuthProductScope,
        _interaction_id: AuthInteractionId,
    ) -> Result<bool, AuthProductError> {
        Ok(false)
    }
}

#[derive(Debug, Default)]
struct UnexpectedChallengeInteractionService;

#[async_trait]
impl AuthInteractionService for UnexpectedChallengeInteractionService {
    async fn request_secret_input(
        &self,
        _request: ManualTokenSetupRequest,
    ) -> Result<AuthChallenge, AuthProductError> {
        Ok(AuthChallenge::OAuthUrl {
            authorization_url: OAuthAuthorizationUrl::new("https://provider.example/oauth")
                .unwrap(),
            expires_at: Utc::now() + Duration::minutes(5),
        })
    }

    async fn submit_manual_token(
        &self,
        _scope: &AuthProductScope,
        _request: SecretSubmitRequest,
    ) -> Result<SecretSubmitResult, AuthProductError> {
        unreachable!("unexpected-challenge test does not submit manual tokens")
    }

    async fn abandon_manual_token(
        &self,
        _scope: &AuthProductScope,
        _interaction_id: AuthInteractionId,
    ) -> Result<bool, AuthProductError> {
        unreachable!("unexpected-challenge test does not abandon manual tokens")
    }
}

#[derive(Debug)]
struct SuccessfulManualTokenInteractionService {
    interaction_id: AuthInteractionId,
    abandoned: Mutex<Vec<AuthInteractionId>>,
}

impl SuccessfulManualTokenInteractionService {
    fn new(interaction_id: AuthInteractionId) -> Self {
        Self {
            interaction_id,
            abandoned: Mutex::new(Vec::new()),
        }
    }

    fn abandoned(&self) -> Vec<AuthInteractionId> {
        self.abandoned.lock().unwrap().clone()
    }
}

#[async_trait]
impl AuthInteractionService for SuccessfulManualTokenInteractionService {
    async fn request_secret_input(
        &self,
        request: ManualTokenSetupRequest,
    ) -> Result<AuthChallenge, AuthProductError> {
        Ok(AuthChallenge::ManualTokenRequired {
            interaction_id: self.interaction_id,
            provider: request.provider,
            label: request.label,
            expires_at: request.expires_at,
        })
    }

    async fn submit_manual_token(
        &self,
        _scope: &AuthProductScope,
        _request: SecretSubmitRequest,
    ) -> Result<SecretSubmitResult, AuthProductError> {
        Ok(SecretSubmitResult {
            account_id: ironclaw_auth::CredentialAccountId::new(),
            status: CredentialAccountStatus::Configured,
            continuation: AuthContinuationRef::SetupOnly,
        })
    }

    async fn abandon_manual_token(
        &self,
        _scope: &AuthProductScope,
        interaction_id: AuthInteractionId,
    ) -> Result<bool, AuthProductError> {
        self.abandoned.lock().unwrap().push(interaction_id);
        Ok(true)
    }
}

#[derive(Debug)]
struct FailingManualTokenFlowManager {
    create_error: Option<AuthProductError>,
    complete_error: Option<AuthProductError>,
    canceled: Mutex<Vec<AuthInteractionId>>,
}

impl FailingManualTokenFlowManager {
    fn create_fails(error: AuthProductError) -> Self {
        Self {
            create_error: Some(error),
            complete_error: None,
            canceled: Mutex::new(Vec::new()),
        }
    }

    fn complete_fails(error: AuthProductError) -> Self {
        Self {
            create_error: None,
            complete_error: Some(error),
            canceled: Mutex::new(Vec::new()),
        }
    }

    fn canceled(&self) -> Vec<AuthInteractionId> {
        self.canceled.lock().unwrap().clone()
    }
}

#[derive(Debug, Default)]
struct RecordingCredentialAccountService {
    status_updates: Mutex<Vec<(ironclaw_auth::CredentialAccountId, CredentialAccountStatus)>>,
}

impl RecordingCredentialAccountService {
    fn status_updates(&self) -> Vec<(ironclaw_auth::CredentialAccountId, CredentialAccountStatus)> {
        self.status_updates.lock().unwrap().clone()
    }
}

#[async_trait]
impl CredentialAccountService for RecordingCredentialAccountService {
    async fn create_account(
        &self,
        _request: NewCredentialAccount,
    ) -> Result<ironclaw_auth::CredentialAccount, AuthProductError> {
        unreachable!("manual-token completion compensation test does not create accounts")
    }

    async fn get_account(
        &self,
        _request: ironclaw_auth::CredentialAccountLookupRequest,
    ) -> Result<Option<ironclaw_auth::CredentialAccount>, AuthProductError> {
        unreachable!("manual-token completion compensation test does not get accounts")
    }

    async fn list_accounts(
        &self,
        _request: CredentialAccountListRequest,
    ) -> Result<ironclaw_auth::CredentialAccountListPage, AuthProductError> {
        unreachable!("manual-token completion compensation test does not list accounts")
    }

    async fn update_status(
        &self,
        scope: &AuthProductScope,
        account_id: ironclaw_auth::CredentialAccountId,
        status: CredentialAccountStatus,
    ) -> Result<ironclaw_auth::CredentialAccount, AuthProductError> {
        self.status_updates
            .lock()
            .unwrap()
            .push((account_id, status));
        Ok(ironclaw_auth::CredentialAccount {
            id: account_id,
            scope: scope.clone(),
            provider: provider(),
            label: label(),
            status,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: None,
            refresh_secret: None,
            scopes: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }

    async fn select_unique_configured_account(
        &self,
        _request: ironclaw_auth::CredentialAccountSelectionRequest,
    ) -> Result<ironclaw_auth::CredentialAccountProjection, AuthProductError> {
        unreachable!("manual-token completion compensation test does not select accounts")
    }

    async fn project_credential_recovery(
        &self,
        _request: ironclaw_auth::CredentialRecoveryRequest,
    ) -> Result<ironclaw_auth::CredentialRecoveryProjection, AuthProductError> {
        unreachable!("manual-token completion compensation test does not project recovery")
    }

    async fn select_configured_account(
        &self,
        _request: ironclaw_auth::CredentialAccountChoiceRequest,
    ) -> Result<ironclaw_auth::CredentialAccountProjection, AuthProductError> {
        unreachable!("manual-token completion compensation test does not choose accounts")
    }

    async fn refresh_account(
        &self,
        _request: ironclaw_auth::CredentialRefreshRequest,
    ) -> Result<ironclaw_auth::CredentialRefreshReport, AuthProductError> {
        unreachable!("manual-token completion compensation test does not refresh accounts")
    }
}

#[async_trait]
impl AuthFlowManager for FailingManualTokenFlowManager {
    async fn create_flow(&self, _request: NewAuthFlow) -> Result<AuthFlowRecord, AuthProductError> {
        Err(self
            .create_error
            .clone()
            .unwrap_or(AuthProductError::BackendUnavailable))
    }

    async fn get_flow(
        &self,
        _scope: &AuthProductScope,
        _flow_id: AuthFlowId,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
        unreachable!("manual-token cleanup tests do not read flows")
    }

    async fn claim_oauth_callback(
        &self,
        _scope: &AuthProductScope,
        _request: OAuthCallbackClaimRequest,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        unreachable!("manual-token cleanup tests do not claim OAuth callbacks")
    }

    async fn complete_oauth_callback(
        &self,
        _scope: &AuthProductScope,
        _input: OAuthCallbackInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        unreachable!("manual-token cleanup tests do not complete OAuth callbacks")
    }

    async fn complete_credential_selection(
        &self,
        _scope: &AuthProductScope,
        _input: CredentialSelectionInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        unreachable!("manual-token cleanup tests do not complete account selection")
    }

    async fn complete_manual_token(
        &self,
        _scope: &AuthProductScope,
        _input: ManualTokenCompletionInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        Err(self
            .complete_error
            .clone()
            .unwrap_or(AuthProductError::BackendUnavailable))
    }

    async fn cancel_manual_token(
        &self,
        _scope: &AuthProductScope,
        interaction_id: AuthInteractionId,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
        self.canceled.lock().unwrap().push(interaction_id);
        Ok(None)
    }

    async fn fail_oauth_callback(
        &self,
        _scope: &AuthProductScope,
        _input: OAuthCallbackFailureInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        unreachable!("manual-token cleanup tests do not fail OAuth callbacks")
    }

    async fn mark_continuation_dispatched(
        &self,
        _scope: &AuthProductScope,
        _flow_id: AuthFlowId,
        _emitted_at: Timestamp,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        unreachable!("manual-token cleanup tests do not mark continuations")
    }

    async fn cancel_flow(
        &self,
        _scope: &AuthProductScope,
        _flow_id: AuthFlowId,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        unreachable!("manual-token cleanup tests do not cancel generic flows")
    }
}

fn auth_services() -> RebornProductAuthServices {
    RebornProductAuthServices::from_shared(
        Arc::new(InMemoryAuthProductServices::new()),
        Arc::new(NoopContinuationDispatcher),
    )
}

fn auth_services_with_interaction(
    interaction_service: Arc<dyn AuthInteractionService>,
) -> RebornProductAuthServices {
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let flow_manager: Arc<dyn AuthFlowManager> = shared.clone();
    let credential_setup_service: Arc<dyn CredentialSetupService> = shared.clone();
    let credential_account_service: Arc<dyn CredentialAccountService> = shared.clone();
    let provider_client: Arc<dyn AuthProviderClient> = shared.clone();
    let cleanup_service: Arc<dyn SecretCleanupService> = shared;

    RebornProductAuthServices::new(
        flow_manager,
        interaction_service,
        credential_setup_service,
        credential_account_service,
        provider_client,
        cleanup_service,
        Arc::new(NoopContinuationDispatcher),
    )
}

fn auth_services_with_flow_and_interaction(
    flow_manager: Arc<dyn AuthFlowManager>,
    interaction_service: Arc<dyn AuthInteractionService>,
) -> RebornProductAuthServices {
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let credential_setup_service: Arc<dyn CredentialSetupService> = shared.clone();
    let credential_account_service: Arc<dyn CredentialAccountService> = shared.clone();
    let provider_client: Arc<dyn AuthProviderClient> = shared.clone();
    let cleanup_service: Arc<dyn SecretCleanupService> = shared;

    RebornProductAuthServices::new(
        flow_manager,
        interaction_service,
        credential_setup_service,
        credential_account_service,
        provider_client,
        cleanup_service,
        Arc::new(NoopContinuationDispatcher),
    )
}

fn auth_services_with_flow_interaction_and_accounts(
    flow_manager: Arc<dyn AuthFlowManager>,
    interaction_service: Arc<dyn AuthInteractionService>,
    credential_account_service: Arc<dyn CredentialAccountService>,
) -> RebornProductAuthServices {
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let credential_setup_service: Arc<dyn CredentialSetupService> = shared.clone();
    let provider_client: Arc<dyn AuthProviderClient> = shared.clone();
    let cleanup_service: Arc<dyn SecretCleanupService> = shared;

    RebornProductAuthServices::new(
        flow_manager,
        interaction_service,
        credential_setup_service,
        credential_account_service,
        provider_client,
        cleanup_service,
        Arc::new(NoopContinuationDispatcher),
    )
}

fn scope(user: &str) -> AuthProductScope {
    AuthProductScope::new(
        ResourceScope::local_default(UserId::new(user).unwrap(), InvocationId::new()).unwrap(),
        AuthSurface::Web,
    )
    .with_session_id(AuthSessionId::new(format!("web-session-{user}")).unwrap())
}

fn provider() -> AuthProviderId {
    AuthProviderId::new("github").unwrap()
}

fn label() -> CredentialAccountLabel {
    CredentialAccountLabel::new("work github").unwrap()
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_string())
}

fn setup_request(scope: AuthProductScope) -> RebornManualTokenSetupRequest {
    RebornManualTokenSetupRequest::new(
        scope,
        provider(),
        label(),
        AuthContinuationRef::SetupOnly,
        Utc::now() + Duration::minutes(5),
    )
}

fn expired_setup_request(scope: AuthProductScope) -> RebornManualTokenSetupRequest {
    RebornManualTokenSetupRequest {
        expires_at: Utc::now() - Duration::seconds(1),
        ..setup_request(scope)
    }
}

async fn request_challenge(
    services: &RebornProductAuthServices,
    owner: AuthProductScope,
) -> AuthInteractionId {
    services
        .request_manual_token_setup(setup_request(owner))
        .await
        .expect("manual-token challenge")
        .interaction_id
}

fn update_binding(account: &ironclaw_auth::CredentialAccount) -> CredentialAccountUpdateBinding {
    CredentialAccountUpdateBinding {
        account_id: account.id,
        ownership: account.ownership,
        owner_extension: account.owner_extension.clone(),
        granted_extensions: account.granted_extensions.clone(),
    }
}

fn account_request(owner: AuthProductScope) -> NewCredentialAccount {
    NewCredentialAccount {
        scope: owner,
        provider: provider(),
        label: CredentialAccountLabel::new("old work github").unwrap(),
        status: CredentialAccountStatus::Expired,
        ownership: CredentialOwnership::UserReusable,
        owner_extension: None,
        granted_extensions: Vec::new(),
        access_secret: Some(SecretHandle::new("old-manual-access").unwrap()),
        refresh_secret: None,
        scopes: Vec::new(),
    }
}

fn assert_error_safe(error: &RebornManualTokenError) {
    let serialized = serde_json::to_string(error).unwrap();
    assert_eq!(
        serde_json::from_str::<RebornManualTokenError>(&serialized).unwrap(),
        *error
    );
    assert!(!serialized.contains(RAW_TOKEN));
    assert!(!format!("{error:?}").contains(RAW_TOKEN));
}

#[tokio::test]
async fn manual_token_facade_updates_bound_account_without_exposing_token() {
    let services = auth_services();
    let owner = scope("alice");
    let existing = services
        .credential_account_service()
        .create_account(account_request(owner.clone()))
        .await
        .expect("existing account");
    let challenge = services
        .request_manual_token_setup(RebornManualTokenSetupRequest {
            label: CredentialAccountLabel::new("updated work github").unwrap(),
            ..setup_request(owner.clone()).with_update_binding(update_binding(&existing))
        })
        .await
        .expect("manual-token update challenge");

    let response = services
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            owner.clone(),
            challenge.interaction_id,
            secret(RAW_TOKEN),
        ))
        .await
        .expect("manual token update succeeds");

    assert_eq!(response.account_id, existing.id);
    assert_eq!(response.status, CredentialAccountStatus::Configured);
    assert_response_safe(&response);

    let accounts = services
        .credential_account_service()
        .list_accounts(CredentialAccountListRequest::new(owner, provider()))
        .await
        .expect("account list");
    assert_eq!(accounts.accounts.len(), 1);
    assert_eq!(accounts.accounts[0].id, existing.id);
    assert_eq!(
        accounts.accounts[0].label,
        CredentialAccountLabel::new("updated work github").unwrap()
    );
    let serialized = serde_json::to_string(&accounts).unwrap();
    assert!(!serialized.contains(RAW_TOKEN));
    assert!(!serialized.contains("manual-access-"));
}

fn assert_response_safe(response: &RebornManualTokenSubmitResponse) {
    let serialized = serde_json::to_string(response).unwrap();
    assert_eq!(
        serde_json::from_str::<RebornManualTokenSubmitResponse>(&serialized).unwrap(),
        *response
    );
    assert!(!serialized.contains(RAW_TOKEN));
    assert!(!serialized.contains("manual-access-"));
}

#[tokio::test]
async fn manual_token_facade_submits_secret_without_exposing_token() {
    let services = auth_services();
    let owner = scope("alice");
    let challenge = services
        .request_manual_token_setup(setup_request(owner.clone()))
        .await
        .expect("manual-token challenge");
    assert_eq!(challenge.provider, provider());
    assert_eq!(challenge.label, label());

    let request = RebornManualTokenSubmitRequest::new(
        owner.clone(),
        challenge.interaction_id,
        secret(RAW_TOKEN),
    );
    let debug = format!("{request:?}");
    assert!(!debug.contains(RAW_TOKEN));

    let response = services
        .submit_manual_token(request)
        .await
        .expect("manual token submit succeeds");

    assert_eq!(response.status, CredentialAccountStatus::Configured);
    assert_eq!(response.continuation, AuthContinuationRef::SetupOnly);
    assert_response_safe(&response);

    let accounts = services
        .credential_account_service()
        .list_accounts(CredentialAccountListRequest::new(owner, provider()))
        .await
        .expect("account list");
    assert_eq!(accounts.accounts.len(), 1);
    let accounts_json = serde_json::to_string(&accounts).unwrap();
    assert!(!accounts_json.contains(RAW_TOKEN));
    assert!(!accounts_json.contains("manual-access-"));
}

#[tokio::test]
async fn manual_token_facade_tracks_setup_and_submit_in_auth_flow() {
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let dispatcher = Arc::new(RecordingContinuationDispatcher::default());
    let services = RebornProductAuthServices::from_shared(shared.clone(), dispatcher.clone());
    let owner = scope("alice");

    let challenge = services
        .request_manual_token_setup(setup_request(owner.clone()))
        .await
        .expect("manual-token challenge");

    let flows = shared.flow_records_snapshot();
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].status, AuthFlowStatus::AwaitingUser);
    assert!(matches!(
        &flows[0].challenge,
        Some(AuthChallenge::ManualTokenRequired { interaction_id, .. })
            if interaction_id == &challenge.interaction_id
    ));

    let response = services
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            owner,
            challenge.interaction_id,
            secret(RAW_TOKEN),
        ))
        .await
        .expect("manual token submit succeeds");

    let flows = shared.flow_records_snapshot();
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].status, AuthFlowStatus::Completed);
    assert_eq!(flows[0].credential_account_id, Some(response.account_id));
    assert!(
        flows[0].continuation_emitted_at.is_some(),
        "manual-token submit should mark continuation dispatch"
    );
    let events = dispatcher.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].flow_id, flows[0].id);
    assert_eq!(events[0].credential_account_id, Some(response.account_id));
}

#[tokio::test]
async fn manual_token_facade_retries_completed_flow_when_continuation_dispatch_fails() {
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let dispatcher = Arc::new(FailsFirstContinuationDispatcher::default());
    let services = RebornProductAuthServices::from_shared(shared.clone(), dispatcher.clone())
        .with_flow_record_source(shared.clone());
    let mut owner = scope("alice");
    owner.resource.thread_id = Some(ironclaw_host_api::ThreadId::new("thread-retry").unwrap());

    let challenge = services
        .request_manual_token_setup(setup_request(owner.clone()))
        .await
        .expect("manual-token challenge");

    let error = services
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            owner.clone(),
            challenge.interaction_id,
            secret(RAW_TOKEN),
        ))
        .await
        .expect_err("first dispatch fails after flow completion");
    assert_eq!(error.code, AuthErrorCode::BackendUnavailable);
    assert!(error.retryable);

    let response = services
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            owner,
            challenge.interaction_id,
            secret(RAW_TOKEN),
        ))
        .await
        .expect("retry dispatches the completed manual-token flow");
    assert_eq!(response.status, CredentialAccountStatus::Configured);
    let flows = shared.flow_records_snapshot();
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].status, AuthFlowStatus::Completed);
    assert_eq!(flows[0].credential_account_id, Some(response.account_id));
    assert!(flows[0].continuation_emitted_at.is_some());
    let events = dispatcher.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].flow_id, flows[0].id);
}

#[tokio::test]
async fn manual_token_facade_denies_cross_scope_submit_without_consuming_interaction() {
    let services = auth_services();
    let owner = scope("alice");
    let interaction_id = request_challenge(&services, owner.clone()).await;

    let error = services
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            scope("bob"),
            interaction_id,
            secret(RAW_TOKEN),
        ))
        .await
        .expect_err("cross-scope submit is denied");
    assert_eq!(error.code, AuthErrorCode::CrossScopeDenied);
    assert!(!error.retryable);
    assert_error_safe(&error);

    let response = services
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            owner,
            interaction_id,
            secret(RAW_TOKEN),
        ))
        .await
        .expect("owner can still submit after denied foreign attempt");
    assert_eq!(response.status, CredentialAccountStatus::Configured);
}

#[tokio::test]
async fn manual_token_facade_fails_closed_for_stale_duplicate_and_malformed_submit() {
    let services = auth_services();
    let owner = scope("alice");

    let expired = services
        .request_manual_token_setup(expired_setup_request(owner.clone()))
        .await
        .expect("expired challenge is still typed");
    let stale = services
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            owner.clone(),
            expired.interaction_id,
            secret(RAW_TOKEN),
        ))
        .await
        .expect_err("expired interaction fails closed");
    assert_eq!(stale.code, AuthErrorCode::UnknownOrExpiredFlow);
    assert_error_safe(&stale);

    let malformed_interaction = request_challenge(&services, owner.clone()).await;
    let malformed = services
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            owner.clone(),
            malformed_interaction,
            secret(""),
        ))
        .await
        .expect_err("empty secret is rejected");
    assert_eq!(malformed.code, AuthErrorCode::InvalidRequest);
    assert!(!malformed.retryable);

    services
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            owner.clone(),
            malformed_interaction,
            secret(RAW_TOKEN),
        ))
        .await
        .expect("malformed submit does not consume the interaction");

    let one_shot_interaction = request_challenge(&services, owner.clone()).await;
    services
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            owner.clone(),
            one_shot_interaction,
            secret(RAW_TOKEN),
        ))
        .await
        .expect("first submit succeeds");
    let duplicate = services
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            owner,
            one_shot_interaction,
            secret(RAW_TOKEN),
        ))
        .await
        .expect_err("duplicate submit is stale");
    assert_eq!(duplicate.code, AuthErrorCode::UnknownOrExpiredFlow);
    assert_error_safe(&duplicate);
}

#[tokio::test]
async fn manual_token_facade_returns_sanitized_backend_and_canceled_failures() {
    let backend = auth_services_with_interaction(Arc::new(FailingInteractionService {
        error: AuthProductError::BackendUnavailable,
    }));
    let backend_error = backend
        .request_manual_token_setup(setup_request(scope("alice")))
        .await
        .expect_err("backend failures are sanitized");
    assert_eq!(backend_error.code, AuthErrorCode::BackendUnavailable);
    assert!(backend_error.retryable);
    assert_error_safe(&backend_error);

    let canceled = auth_services_with_interaction(Arc::new(FailingInteractionService {
        error: AuthProductError::Canceled,
    }));
    let canceled_error = canceled
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            scope("alice"),
            AuthInteractionId::new(),
            secret(RAW_TOKEN),
        ))
        .await
        .expect_err("canceled interactions are sanitized");
    assert_eq!(canceled_error.code, AuthErrorCode::Canceled);
    assert!(!canceled_error.retryable);
    assert_error_safe(&canceled_error);
}

#[tokio::test]
async fn manual_token_facade_abandons_interaction_when_flow_creation_fails() {
    let interaction_id = AuthInteractionId::new();
    let interaction = Arc::new(SuccessfulManualTokenInteractionService::new(interaction_id));
    let services = auth_services_with_flow_and_interaction(
        Arc::new(FailingManualTokenFlowManager::create_fails(
            AuthProductError::BackendUnavailable,
        )),
        interaction.clone(),
    );

    let error = services
        .request_manual_token_setup(setup_request(scope("alice")))
        .await
        .expect_err("flow creation failure should surface");

    assert_eq!(error.code, AuthErrorCode::BackendUnavailable);
    assert_eq!(interaction.abandoned(), vec![interaction_id]);
}

#[tokio::test]
async fn manual_token_facade_cancels_flow_when_completion_fails() {
    let interaction_id = AuthInteractionId::new();
    let flow_manager = Arc::new(FailingManualTokenFlowManager::complete_fails(
        AuthProductError::BackendUnavailable,
    ));
    let account_service = Arc::new(RecordingCredentialAccountService::default());
    let services = auth_services_with_flow_interaction_and_accounts(
        flow_manager.clone(),
        Arc::new(SuccessfulManualTokenInteractionService::new(interaction_id)),
        account_service.clone(),
    );

    let error = services
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            scope("alice"),
            interaction_id,
            secret(RAW_TOKEN),
        ))
        .await
        .expect_err("flow completion failure should surface");

    assert_eq!(error.code, AuthErrorCode::BackendUnavailable);
    assert_eq!(flow_manager.canceled(), vec![interaction_id]);
    assert_eq!(
        account_service
            .status_updates()
            .into_iter()
            .map(|(_, status)| status)
            .collect::<Vec<_>>(),
        vec![CredentialAccountStatus::Revoked]
    );
}

#[tokio::test]
async fn request_manual_token_setup_returns_error_on_unexpected_challenge() {
    let services = auth_services_with_interaction(Arc::new(UnexpectedChallengeInteractionService));

    let error = services
        .request_manual_token_setup(setup_request(scope("alice")))
        .await
        .expect_err("unexpected challenge is rejected");

    assert_eq!(error.code, AuthErrorCode::InvalidRequest);
    assert!(!error.retryable);
    assert_error_safe(&error);
}
