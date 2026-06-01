use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use ironclaw_auth::{
    AuthContinuationEvent, AuthProductError, AuthProductScope, AuthProviderId, AuthSessionId,
    AuthSurface, CredentialAccountLookupRequest, CredentialAccountService, CredentialAccountStatus,
    CredentialOwnership, CredentialRefreshRequest, InMemoryAuthProductServices,
    NewCredentialAccount, OAuthProviderCallbackRequest, OAuthProviderExchange,
    OAuthProviderExchangeContext, OAuthProviderRefresh, OAuthProviderRefreshRequest, ProviderScope,
    SecretCleanupAction, SecretCleanupQuarantineReason, SecretCleanupRequest,
};
use ironclaw_host_api::{ExtensionId, InvocationId, ResourceScope, SecretHandle, UserId};
use ironclaw_reborn_composition::{RebornAuthContinuationDispatcher, RebornProductAuthServices};

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
