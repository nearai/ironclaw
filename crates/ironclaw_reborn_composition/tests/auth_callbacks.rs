use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationEvent, AuthContinuationRef, AuthErrorCode, AuthFlowId,
    AuthFlowKind, AuthProductError, AuthProductScope, AuthProviderClient, AuthProviderId,
    AuthSessionId, AuthSurface, AuthorizationCodeHash, CredentialAccountId, CredentialAccountLabel,
    InMemoryAuthProductServices, LifecyclePackageRef, NewAuthFlow, OAuthAuthorizationCode,
    OAuthAuthorizationUrl, OAuthProviderCallbackRequest, OAuthProviderExchange,
    OAuthProviderExchangeContext, OAuthProviderRefresh, OAuthProviderRefreshRequest,
    OpaqueStateHash, PkceVerifierHash, PkceVerifierSecret, ProviderScope,
};
use ironclaw_host_api::{InvocationId, ResourceScope, SecretHandle, UserId};
use ironclaw_reborn_composition::{
    RebornAuthContinuationDispatcher, RebornOAuthCallbackOutcome, RebornOAuthCallbackRequest,
    RebornOAuthCallbackResponse, RebornProductAuthServices,
};
use secrecy::SecretString;

#[derive(Default)]
struct RecordingContinuationDispatcher {
    events: Mutex<Vec<AuthContinuationEvent>>,
}

impl RecordingContinuationDispatcher {
    fn events(&self) -> Vec<AuthContinuationEvent> {
        self.events
            .lock()
            .expect("continuation event lock poisoned")
            .clone()
    }
}

#[async_trait]
impl RebornAuthContinuationDispatcher for RecordingContinuationDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        self.events
            .lock()
            .expect("continuation event lock poisoned")
            .push(event);
        Ok(())
    }
}

struct FailingProviderClient {
    error: AuthProductError,
}

#[async_trait]
impl AuthProviderClient for FailingProviderClient {
    async fn exchange_callback(
        &self,
        _context: OAuthProviderExchangeContext,
        _request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        Err(self.error.clone())
    }

    async fn refresh_token(
        &self,
        _request: OAuthProviderRefreshRequest,
    ) -> Result<OAuthProviderRefresh, AuthProductError> {
        Err(self.error.clone())
    }
}

#[derive(Default)]
struct CountingProviderClient {
    calls: AtomicUsize,
}

impl CountingProviderClient {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl AuthProviderClient for CountingProviderClient {
    async fn exchange_callback(
        &self,
        _context: OAuthProviderExchangeContext,
        _request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(AuthProductError::TokenExchangeFailed)
    }

    async fn refresh_token(
        &self,
        _request: OAuthProviderRefreshRequest,
    ) -> Result<OAuthProviderRefresh, AuthProductError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(AuthProductError::RefreshFailed)
    }
}

#[derive(Default)]
struct SuccessfulCountingProviderClient {
    calls: AtomicUsize,
    last_context: Mutex<Option<OAuthProviderExchangeContext>>,
}

impl SuccessfulCountingProviderClient {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn last_context(&self) -> Option<OAuthProviderExchangeContext> {
        self.last_context
            .lock()
            .expect("provider context lock poisoned")
            .clone()
    }
}

#[async_trait]
impl AuthProviderClient for SuccessfulCountingProviderClient {
    async fn exchange_callback(
        &self,
        context: OAuthProviderExchangeContext,
        request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self
            .last_context
            .lock()
            .expect("provider context lock poisoned") = Some(context);
        Ok(OAuthProviderExchange {
            provider: request.provider,
            account_label: request.account_label,
            authorization_code_hash: request.authorization_code_hash,
            pkce_verifier_hash: request.pkce_verifier_hash,
            access_secret: SecretHandle::new("oauth-access").unwrap(),
            refresh_secret: Some(SecretHandle::new("oauth-refresh").unwrap()),
            scopes: request.scopes,
            account_id: None,
        })
    }

    async fn refresh_token(
        &self,
        request: OAuthProviderRefreshRequest,
    ) -> Result<OAuthProviderRefresh, AuthProductError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(OAuthProviderRefresh {
            provider: request.provider,
            access_secret: SecretHandle::new("oauth-refreshed-access").unwrap(),
            refresh_secret: Some(SecretHandle::new("oauth-refreshed-refresh").unwrap()),
            scopes: request.scopes,
        })
    }
}

struct FailingContinuationDispatcher {
    error: AuthProductError,
}

#[derive(Default)]
struct CleanupRecordingProviderClient {
    cleaned: Mutex<Vec<SecretHandle>>,
}

impl CleanupRecordingProviderClient {
    fn cleaned(&self) -> Vec<SecretHandle> {
        self.cleaned
            .lock()
            .expect("cleaned handles lock poisoned")
            .clone()
    }
}

#[async_trait]
impl AuthProviderClient for CleanupRecordingProviderClient {
    async fn exchange_callback(
        &self,
        _context: OAuthProviderExchangeContext,
        request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        Ok(OAuthProviderExchange {
            provider: request.provider,
            account_label: request.account_label,
            authorization_code_hash: request.authorization_code_hash,
            pkce_verifier_hash: request.pkce_verifier_hash,
            access_secret: SecretHandle::new("orphan-access").unwrap(),
            refresh_secret: Some(SecretHandle::new("orphan-refresh").unwrap()),
            scopes: request.scopes,
            account_id: Some(CredentialAccountId::new()),
        })
    }

    async fn refresh_token(
        &self,
        _request: OAuthProviderRefreshRequest,
    ) -> Result<OAuthProviderRefresh, AuthProductError> {
        Err(AuthProductError::RefreshFailed)
    }

    async fn cleanup_exchange(
        &self,
        _context: OAuthProviderExchangeContext,
        exchange: &OAuthProviderExchange,
    ) -> Result<(), AuthProductError> {
        let mut cleaned = self.cleaned.lock().expect("cleaned handles lock poisoned");
        cleaned.push(exchange.access_secret.clone());
        cleaned.extend(exchange.refresh_secret.clone());
        Ok(())
    }
}

#[async_trait]
impl RebornAuthContinuationDispatcher for FailingContinuationDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        _event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        Err(self.error.clone())
    }
}

fn auth_services(dispatcher: Arc<RecordingContinuationDispatcher>) -> RebornProductAuthServices {
    RebornProductAuthServices::from_shared(Arc::new(InMemoryAuthProductServices::new()), dispatcher)
}

fn scope(user: &str) -> AuthProductScope {
    AuthProductScope::new(
        ResourceScope::local_default(UserId::new(user).unwrap(), InvocationId::new()).unwrap(),
        AuthSurface::Callback,
    )
    .with_session_id(AuthSessionId::new(format!("callback-session-{user}")).unwrap())
}

fn provider() -> AuthProviderId {
    AuthProviderId::new("github").unwrap()
}

fn label() -> CredentialAccountLabel {
    CredentialAccountLabel::new("work github").unwrap()
}

fn state_hash(value: &str) -> OpaqueStateHash {
    OpaqueStateHash::new(fake_digest(value)).unwrap()
}

fn pkce_hash(value: &str) -> PkceVerifierHash {
    PkceVerifierHash::new(fake_digest(value)).unwrap()
}

fn code_hash(value: &str) -> AuthorizationCodeHash {
    AuthorizationCodeHash::new(fake_digest(value)).unwrap()
}

fn fake_digest(value: &str) -> String {
    format!(
        "{:064x}",
        value.bytes().fold(0_u64, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(u64::from(byte))
        })
    )
}

fn authorization_url(value: &str) -> OAuthAuthorizationUrl {
    OAuthAuthorizationUrl::new(value).unwrap()
}

fn provider_scope(value: &str) -> ProviderScope {
    ProviderScope::new(value).unwrap()
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_string())
}

async fn create_flow(services: &RebornProductAuthServices, scope: AuthProductScope) -> AuthFlowId {
    services
        .flow_manager()
        .create_flow(NewAuthFlow {
            scope,
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: authorization_url("https://provider.example/oauth"),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::LifecycleActivation {
                package_ref: LifecyclePackageRef::new("github-extension").unwrap(),
            },
            update_binding: None,
            opaque_state_hash: Some(state_hash("state-hash")),
            pkce_verifier_hash: Some(pkce_hash("pkce-hash")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect("flow")
        .id
}

fn authorized_request(scope: AuthProductScope, flow_id: AuthFlowId) -> RebornOAuthCallbackRequest {
    RebornOAuthCallbackRequest {
        scope,
        flow_id,
        opaque_state_hash: state_hash("state-hash"),
        outcome: RebornOAuthCallbackOutcome::Authorized {
            provider_request: OAuthProviderCallbackRequest {
                provider: provider(),
                account_label: label(),
                authorization_code: OAuthAuthorizationCode::new(secret("raw-auth-code")).unwrap(),
                authorization_code_hash: code_hash("code-hash"),
                pkce_verifier: PkceVerifierSecret::new(secret("raw-pkce-verifier")).unwrap(),
                pkce_verifier_hash: pkce_hash("pkce-hash"),
                scopes: vec![provider_scope("repo")],
            },
        },
    }
}

#[tokio::test]
async fn oauth_callback_handler_completes_flow_and_dispatches_typed_continuation() {
    let dispatcher = Arc::new(RecordingContinuationDispatcher::default());
    let provider_client = Arc::new(SuccessfulCountingProviderClient::default());
    let services = auth_services(dispatcher.clone()).with_provider_client(provider_client.clone());
    let owner = scope("alice");
    let flow_id = create_flow(&services, owner.clone()).await;
    let request = authorized_request(owner.clone(), flow_id);
    let debug = format!("{request:?}");
    assert!(!debug.contains("raw-auth-code"));
    assert!(!debug.contains("raw-pkce-verifier"));

    let response = services
        .handle_oauth_callback(request)
        .await
        .expect("callback completes");

    assert_eq!(response.flow_id, flow_id);
    assert!(response.credential_account_id.is_some());
    assert_eq!(
        provider_client.last_context(),
        Some(OAuthProviderExchangeContext {
            scope: owner.clone(),
            flow_id,
        })
    );
    assert_eq!(
        response.continuation,
        AuthContinuationRef::LifecycleActivation {
            package_ref: LifecyclePackageRef::new("github-extension").unwrap()
        }
    );

    let events = dispatcher.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].flow_id, flow_id);
    assert_eq!(events[0].scope, owner);
    assert_eq!(events[0].continuation, response.continuation);
    assert_eq!(
        events[0].credential_account_id,
        response.credential_account_id
    );

    let serialized = serde_json::to_string(&response).unwrap();
    let parsed: RebornOAuthCallbackResponse = serde_json::from_str(&serialized).unwrap();
    assert_eq!(parsed, response);
    assert!(!serialized.contains("raw-auth-code"));
    assert!(!serialized.contains("raw-pkce-verifier"));
    assert!(!serialized.contains("oauth-access-"));
    assert!(!serialized.contains("oauth-refresh-"));
}

#[tokio::test]
async fn oauth_callback_handler_reports_completed_continuation_dispatch_failure_as_retryable() {
    let shared_auth = Arc::new(InMemoryAuthProductServices::new());
    let services = RebornProductAuthServices::from_shared(
        shared_auth.clone(),
        Arc::new(FailingContinuationDispatcher {
            error: AuthProductError::TokenExchangeFailed,
        }),
    );
    let owner = scope("alice");
    let flow_id = create_flow(&services, owner.clone()).await;

    let error = services
        .handle_oauth_callback(authorized_request(owner.clone(), flow_id))
        .await
        .expect_err("dispatch failure is reported to caller");

    assert_eq!(error.code, AuthErrorCode::BackendUnavailable);
    assert!(error.retryable);

    let dispatcher = Arc::new(RecordingContinuationDispatcher::default());
    let provider_client = Arc::new(SuccessfulCountingProviderClient::default());
    let retry_services = RebornProductAuthServices::new(
        shared_auth.clone(),
        shared_auth.clone(),
        shared_auth.clone(),
        shared_auth.clone(),
        provider_client.clone(),
        shared_auth,
        dispatcher.clone(),
    );
    let retry = retry_services
        .handle_oauth_callback(authorized_request(owner.clone(), flow_id))
        .await
        .expect("completed callback can be retried for continuation dispatch");

    assert_eq!(retry.flow_id, flow_id);
    assert_eq!(retry.status, ironclaw_auth::AuthFlowStatus::Completed);
    assert_eq!(provider_client.calls(), 0);
    assert_eq!(dispatcher.events().len(), 1);
}

#[tokio::test]
async fn oauth_callback_handler_cleans_provider_tokens_when_completion_rejects_exchange() {
    let dispatcher = Arc::new(RecordingContinuationDispatcher::default());
    let provider_client = Arc::new(CleanupRecordingProviderClient::default());
    let services = auth_services(dispatcher).with_provider_client(provider_client.clone());
    let owner = scope("alice");
    let flow_id = create_flow(&services, owner.clone()).await;

    let error = services
        .handle_oauth_callback(authorized_request(owner, flow_id))
        .await
        .expect_err("invalid exchange account id is rejected");

    assert_eq!(error.code, AuthErrorCode::CrossScopeDenied);
    assert_eq!(
        provider_client.cleaned(),
        vec![
            SecretHandle::new("orphan-access").unwrap(),
            SecretHandle::new("orphan-refresh").unwrap(),
        ]
    );
}

#[tokio::test]
async fn oauth_callback_handler_rejects_wrong_state_without_provider_exchange_or_dispatch() {
    let dispatcher = Arc::new(RecordingContinuationDispatcher::default());
    let provider_client = Arc::new(CountingProviderClient::default());
    let services = auth_services(dispatcher.clone()).with_provider_client(provider_client.clone());
    let owner = scope("alice");
    let flow_id = create_flow(&services, owner.clone()).await;
    let mut request = authorized_request(owner, flow_id);
    request.opaque_state_hash = state_hash("wrong-state");

    let error = services
        .handle_oauth_callback(request)
        .await
        .expect_err("wrong state is rejected before provider exchange");

    assert_eq!(error.code, AuthErrorCode::CrossScopeDenied);
    assert_eq!(provider_client.calls(), 0);
    assert!(dispatcher.events().is_empty());
}

#[tokio::test]
async fn oauth_callback_handler_rejects_wrong_pkce_without_provider_exchange_or_dispatch() {
    let dispatcher = Arc::new(RecordingContinuationDispatcher::default());
    let provider_client = Arc::new(CountingProviderClient::default());
    let services = auth_services(dispatcher.clone()).with_provider_client(provider_client.clone());
    let owner = scope("alice");
    let flow_id = create_flow(&services, owner.clone()).await;
    let mut request = authorized_request(owner, flow_id);
    let RebornOAuthCallbackOutcome::Authorized { provider_request } = &mut request.outcome else {
        panic!("authorized request expected");
    };
    provider_request.pkce_verifier_hash = pkce_hash("wrong-pkce");

    let error = services
        .handle_oauth_callback(request)
        .await
        .expect_err("wrong pkce is rejected before provider exchange");

    assert_eq!(error.code, AuthErrorCode::CrossScopeDenied);
    assert_eq!(provider_client.calls(), 0);
    assert!(dispatcher.events().is_empty());
}

#[tokio::test]
async fn oauth_callback_handler_returns_sanitized_failures_without_dispatch() {
    let dispatcher = Arc::new(RecordingContinuationDispatcher::default());
    let services = auth_services(dispatcher.clone());
    let owner = scope("alice");

    let stale = services
        .handle_oauth_callback(RebornOAuthCallbackRequest {
            scope: owner.clone(),
            flow_id: AuthFlowId::new(),
            opaque_state_hash: state_hash("state-hash"),
            outcome: RebornOAuthCallbackOutcome::ProviderDenied,
        })
        .await
        .expect_err("unknown flow fails");
    assert_eq!(stale.code, AuthErrorCode::UnknownOrExpiredFlow);

    let malformed = services
        .handle_oauth_callback(RebornOAuthCallbackRequest {
            scope: owner.clone(),
            flow_id: AuthFlowId::new(),
            opaque_state_hash: state_hash("state-hash"),
            outcome: RebornOAuthCallbackOutcome::Malformed,
        })
        .await
        .expect_err("malformed callback fails");
    assert_eq!(malformed.code, AuthErrorCode::MalformedCallback);

    let denied_flow = create_flow(&services, owner.clone()).await;
    let provider_denied = services
        .handle_oauth_callback(RebornOAuthCallbackRequest {
            scope: owner.clone(),
            flow_id: denied_flow,
            opaque_state_hash: state_hash("state-hash"),
            outcome: RebornOAuthCallbackOutcome::ProviderDenied,
        })
        .await
        .expect_err("provider denial fails");
    assert_eq!(provider_denied.code, AuthErrorCode::ProviderDenied);

    let cross_scope_flow = create_flow(&services, owner.clone()).await;
    let cross_scope = services
        .handle_oauth_callback(RebornOAuthCallbackRequest {
            scope: scope("bob"),
            flow_id: cross_scope_flow,
            opaque_state_hash: state_hash("state-hash"),
            outcome: RebornOAuthCallbackOutcome::ProviderDenied,
        })
        .await
        .expect_err("foreign callback denied");
    assert_eq!(cross_scope.code, AuthErrorCode::CrossScopeDenied);

    assert!(dispatcher.events().is_empty());
}

#[tokio::test]
async fn oauth_callback_handler_routes_exchange_failures_through_provider_boundary() {
    let dispatcher = Arc::new(RecordingContinuationDispatcher::default());
    let services =
        auth_services(dispatcher.clone()).with_provider_client(Arc::new(FailingProviderClient {
            error: AuthProductError::TokenExchangeFailed,
        }));
    let owner = scope("alice");
    let flow_id = create_flow(&services, owner.clone()).await;

    let error = services
        .handle_oauth_callback(authorized_request(owner.clone(), flow_id))
        .await
        .expect_err("provider exchange failure surfaces sanitized error");

    assert_eq!(error.code, AuthErrorCode::TokenExchangeFailed);
    assert!(!error.retryable);
    assert!(dispatcher.events().is_empty());
    let serialized = serde_json::to_string(&error).unwrap();
    let parsed: ironclaw_reborn_composition::RebornOAuthCallbackError =
        serde_json::from_str(&serialized).unwrap();
    assert_eq!(parsed, error);
    assert!(!serialized.contains("raw-auth-code"));
    assert!(!serialized.contains("raw-pkce-verifier"));

    let flow = services
        .flow_manager()
        .get_flow(&owner, flow_id)
        .await
        .expect("flow lookup")
        .expect("flow record");
    assert_eq!(flow.status, ironclaw_auth::AuthFlowStatus::Failed);
    assert_eq!(flow.error, Some(AuthErrorCode::TokenExchangeFailed));

    let retry = services
        .handle_oauth_callback(authorized_request(owner, flow_id))
        .await
        .expect_err("failed flow rejects retry");
    assert_eq!(retry.code, AuthErrorCode::FlowAlreadyTerminal);
}
