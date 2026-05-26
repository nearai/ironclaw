use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthContinuationEvent, AuthProductError, AuthProductScope, AuthProviderId, AuthSessionId,
    AuthSurface, CredentialAccountService, CredentialAccountStatus, CredentialOwnership,
    CredentialRefreshRequest, InMemoryAuthProductServices, NewCredentialAccount, ProviderScope,
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
        .get_account(&owner, account.id)
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
            extension_id: extension,
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
        .get_account(&owner, owned.id)
        .await
        .unwrap()
        .expect("owned account");
    assert_eq!(owned_after.status, CredentialAccountStatus::Revoked);
    let quarantined_after = auth
        .get_account(&owner, quarantined.id)
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
