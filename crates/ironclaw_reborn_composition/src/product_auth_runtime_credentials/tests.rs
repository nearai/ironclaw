use ironclaw_auth::{
    CredentialAccountLabel, CredentialAccountService, CredentialOwnership,
    InMemoryAuthProductServices, NewCredentialAccount,
};
use ironclaw_host_api::{
    ExtensionId, InvocationId, MissionId, ResourceScope, RuntimeCredentialAccountProviderId,
    RuntimeCredentialAccountSetup, RuntimeCredentialAuthRequirement, SecretHandle, ThreadId,
    UserId,
};

use super::*;

mod duplicate_selection;

fn resolver_with_accounts(
    accounts: Arc<InMemoryAuthProductServices>,
) -> ProductAuthRuntimeCredentialResolver {
    ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new_with_visibility(
            accounts,
            Arc::new(crate::gsuite::GsuiteRuntimeCredentialAccountVisibilityPolicy),
        ),
    ))
}

fn resolver_with_refresh(
    accounts: Arc<InMemoryAuthProductServices>,
) -> ProductAuthRuntimeCredentialResolver {
    ProductAuthRuntimeCredentialResolver::new_with_refresh(
        Arc::new(
            ProductAuthRuntimeCredentialAccountSelector::new_with_visibility(
                accounts.clone(),
                Arc::new(crate::gsuite::GsuiteRuntimeCredentialAccountVisibilityPolicy),
            ),
        ),
        Arc::new(ProductAuthRuntimeCredentialAccountRefresher::new(Arc::new(
            TestRuntimeCredentialRefreshPort(accounts),
        ))),
    )
}

struct TestRuntimeCredentialRefreshPort(Arc<InMemoryAuthProductServices>);

#[async_trait::async_trait]
impl RuntimeCredentialAccountRefreshPort for TestRuntimeCredentialRefreshPort {
    async fn refresh_credential_account(
        &self,
        request: CredentialRefreshRequest,
    ) -> Result<CredentialRefreshReport, AuthProductError> {
        self.0.refresh_account(request).await
    }
}

#[tokio::test]
async fn resolver_returns_configured_product_auth_access_secret() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let access_secret = SecretHandle::new("github_manual_access").unwrap();
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("github").unwrap(),
            label: CredentialAccountLabel::new("work github").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(access_secret.clone()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let resolver = resolver_with_accounts(accounts);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .unwrap();

    assert_eq!(resolved.handle, access_secret);
    assert_eq!(resolved.scope, scope);
}

#[tokio::test]
async fn resolver_refreshes_oauth_account_before_staging_access_secret() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let stale_access = SecretHandle::new("google_stale_access").unwrap();
    let drive_scope = ProviderScope::new("https://www.googleapis.com/auth/drive.readonly").unwrap();
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(stale_access.clone()),
            refresh_secret: Some(SecretHandle::new("google_refresh").unwrap()),
            scopes: vec![drive_scope.clone()],
        })
        .await
        .unwrap();
    let resolver = resolver_with_refresh(accounts.clone());

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[drive_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("google-drive").unwrap(),
        })
        .await
        .expect("OAuth runtime credentials should refresh before staging");

    assert_eq!(resolved.scope, scope);
    assert_ne!(resolved.handle, stale_access);
    assert!(
        resolved
            .handle
            .as_str()
            .starts_with("oauth-refreshed-access")
    );
}

#[tokio::test]
async fn resolver_refreshes_gsuite_owned_account_with_owner_authority_for_sibling_requester() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let stale_access = SecretHandle::new("google_stale_gsuite_access").unwrap();
    let calendar_scope =
        ProviderScope::new("https://www.googleapis.com/auth/calendar.readonly").unwrap();
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(ExtensionId::new("google-drive").unwrap()),
            granted_extensions: Vec::new(),
            access_secret: Some(stale_access.clone()),
            refresh_secret: Some(SecretHandle::new("google_gsuite_refresh").unwrap()),
            scopes: vec![calendar_scope.clone()],
        })
        .await
        .unwrap();
    let resolver = resolver_with_refresh(accounts.clone());

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[calendar_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("google-calendar").unwrap(),
        })
        .await
        .expect("GSuite siblings should refresh through the selected account owner");

    assert_ne!(resolved.handle, stale_access);
    assert!(
        resolved
            .handle
            .as_str()
            .starts_with("oauth-refreshed-access")
    );
}

#[tokio::test]
async fn resolver_does_not_refresh_same_oauth_account_twice_during_runtime_staging() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let drive_scope = ProviderScope::new("https://www.googleapis.com/auth/drive.readonly").unwrap();
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("google_stale_access_once").unwrap()),
            refresh_secret: Some(SecretHandle::new("google_refresh_once").unwrap()),
            scopes: vec![drive_scope.clone()],
        })
        .await
        .unwrap();
    let resolver = resolver_with_refresh(accounts.clone());
    let provider = RuntimeCredentialAccountProviderId::new("google").unwrap();
    let setup = RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() };
    let provider_scopes = vec![drive_scope.as_str().to_string()];
    let requester_extension = ExtensionId::new("google-drive").unwrap();

    let first = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &provider,
            setup: &setup,
            provider_scopes: &provider_scopes,
            requester_extension: &requester_extension,
        })
        .await
        .expect("first OAuth staging refreshes");
    let second = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &provider,
            setup: &setup,
            provider_scopes: &provider_scopes,
            requester_extension: &requester_extension,
        })
        .await
        .expect("second OAuth staging reuses refreshed account");

    assert_eq!(second.handle, first.handle);
}

#[tokio::test]
async fn resolver_stages_oauth_access_secret_when_refresh_secret_is_absent() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let access_secret = SecretHandle::new("google_access_without_refresh").unwrap();
    let drive_scope = ProviderScope::new("https://www.googleapis.com/auth/drive.readonly").unwrap();
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(access_secret.clone()),
            refresh_secret: None,
            scopes: vec![drive_scope.clone()],
        })
        .await
        .unwrap();
    let resolver = resolver_with_refresh(accounts.clone());

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[drive_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("google-drive").unwrap(),
        })
        .await
        .expect("configured OAuth access token should still stage without a refresh token");

    assert_eq!(resolved.handle, access_secret);
    assert_eq!(resolved.scope, scope);
}

#[tokio::test]
async fn resolver_stages_oauth_access_secret_when_proactive_refresh_backend_is_unavailable() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let access_secret = SecretHandle::new("google_access_refresh_backend_down").unwrap();
    let drive_scope = ProviderScope::new("https://www.googleapis.com/auth/drive.readonly").unwrap();
    let account = accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(access_secret.clone()),
            refresh_secret: Some(SecretHandle::new("google_refresh").unwrap()),
            scopes: vec![drive_scope.clone()],
        })
        .await
        .unwrap();
    accounts.fail_next_refresh_backend_for_tests(account.id);
    let resolver = resolver_with_refresh(accounts.clone());

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[drive_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("google-drive").unwrap(),
        })
        .await
        .expect("proactive refresh backend outage should not fail configured token staging");

    assert_eq!(resolved.handle, access_secret);
    assert_eq!(resolved.scope, scope);
}

#[tokio::test]
async fn resolver_maps_oauth_refresh_failure_to_auth_required() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let drive_scope = ProviderScope::new("https://www.googleapis.com/auth/drive.readonly").unwrap();
    let account = accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("google_stale_access").unwrap()),
            refresh_secret: Some(SecretHandle::new("google_refresh").unwrap()),
            scopes: vec![drive_scope.clone()],
        })
        .await
        .unwrap();
    accounts.fail_next_refresh_for_tests(account.id);
    let resolver = resolver_with_refresh(accounts.clone());

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[drive_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("google-drive").unwrap(),
        })
        .await
        .expect_err("stale OAuth access token must not be staged after refresh failure");

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_accepts_unscoped_github_manual_token_for_scoped_runtime_request() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let access_secret = SecretHandle::new("github_manual_access").unwrap();
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("github").unwrap(),
            label: CredentialAccountLabel::new("work github").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(access_secret.clone()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let resolver = resolver_with_accounts(accounts);
    let required_scopes = vec!["repo".to_string()];

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &required_scopes,
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .expect("GitHub PAT scopes are encoded in the token and cannot be introspected");

    assert_eq!(resolved.handle, access_secret);
    assert_eq!(resolved.scope, scope);
}

#[tokio::test]
async fn resolver_does_not_use_reusable_account_from_different_user() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let alice_scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let admin_scope =
        ResourceScope::local_default(UserId::new("admin").unwrap(), InvocationId::new()).unwrap();
    accounts
        .create_account(NewCredentialAccount {
            scope: AuthProductScope::new(alice_scope, AuthSurface::Api),
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("alice google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("alice-google-access").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let resolver = resolver_with_accounts(accounts);

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &admin_scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("gmail").unwrap(),
        })
        .await
        .expect_err("admin must not resolve alice's reusable account");

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_matches_callback_setup_account_from_runtime_invocation() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let mut setup_scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    setup_scope.thread_id = Some(ThreadId::new("thread-auth-1").unwrap());
    let mut runtime_scope = setup_scope.clone();
    runtime_scope.invocation_id = InvocationId::new();
    let access_secret = SecretHandle::new("github_manual_access").unwrap();
    accounts
        .create_account(NewCredentialAccount {
            scope: AuthProductScope::new(setup_scope.clone(), AuthSurface::Callback),
            provider: AuthProviderId::new("github").unwrap(),
            label: CredentialAccountLabel::new("work github").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(access_secret.clone()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let resolver = resolver_with_accounts(accounts);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &runtime_scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .unwrap();

    assert_eq!(resolved.handle, access_secret);
    assert_eq!(resolved.scope, setup_scope);
}

#[tokio::test]
async fn resolver_matches_reusable_setup_account_from_new_thread() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let mut setup_scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    setup_scope.thread_id = Some(ThreadId::new("thread-auth-1").unwrap());
    let mut runtime_scope = setup_scope.clone();
    runtime_scope.thread_id = Some(ThreadId::new("thread-auth-2").unwrap());
    runtime_scope.invocation_id = InvocationId::new();
    let access_secret = SecretHandle::new("github_manual_access").unwrap();
    accounts
        .create_account(NewCredentialAccount {
            scope: AuthProductScope::new(setup_scope.clone(), AuthSurface::Callback),
            provider: AuthProviderId::new("github").unwrap(),
            label: CredentialAccountLabel::new("work github").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(access_secret.clone()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let resolver = resolver_with_accounts(accounts);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &runtime_scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .unwrap();

    assert_eq!(resolved.handle, access_secret);
    assert_eq!(resolved.scope, setup_scope);
}

#[tokio::test]
async fn resolver_matches_reusable_setup_account_from_new_mission() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let mut setup_scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    setup_scope.mission_id = Some(MissionId::new("mission-auth-1").unwrap());
    let mut runtime_scope = setup_scope.clone();
    runtime_scope.mission_id = Some(MissionId::new("mission-auth-2").unwrap());
    runtime_scope.invocation_id = InvocationId::new();
    let access_secret = SecretHandle::new("github_manual_access").unwrap();
    accounts
        .create_account(NewCredentialAccount {
            scope: AuthProductScope::new(setup_scope.clone(), AuthSurface::Callback),
            provider: AuthProviderId::new("github").unwrap(),
            label: CredentialAccountLabel::new("work github").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(access_secret.clone()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let resolver = resolver_with_accounts(accounts);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &runtime_scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .unwrap();

    assert_eq!(resolved.handle, access_secret);
    assert_eq!(resolved.scope, setup_scope);
}

#[tokio::test]
async fn resolver_rejects_extension_owned_account_from_new_thread() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let mut setup_scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    setup_scope.thread_id = Some(ThreadId::new("thread-auth-1").unwrap());
    let mut runtime_scope = setup_scope.clone();
    runtime_scope.thread_id = Some(ThreadId::new("thread-auth-2").unwrap());
    runtime_scope.invocation_id = InvocationId::new();
    accounts
        .create_account(NewCredentialAccount {
            scope: AuthProductScope::new(setup_scope, AuthSurface::Callback),
            provider: AuthProviderId::new("github").unwrap(),
            label: CredentialAccountLabel::new("work github").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(ExtensionId::new("github").unwrap()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github_manual_access").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let resolver = resolver_with_accounts(accounts);

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &runtime_scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .unwrap_err();

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_maps_missing_account_to_auth_required() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let resolver = resolver_with_accounts(accounts);
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .unwrap_err();

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_requires_requested_provider_scopes() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("work google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("google_manual_access").unwrap()),
            refresh_secret: None,
            scopes: vec![ProviderScope::new("https://www.googleapis.com/auth/gmail.send").unwrap()],
        })
        .await
        .unwrap();
    let resolver = resolver_with_accounts(accounts);
    let required_scopes = vec!["https://www.googleapis.com/auth/drive".to_string()];

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &required_scopes,
            requester_extension: &ExtensionId::new("google-drive").unwrap(),
        })
        .await
        .unwrap_err();

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_does_not_treat_unscoped_google_account_as_scoped() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("work google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("google_manual_access").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let resolver = resolver_with_accounts(accounts);
    let required_scopes = vec!["https://www.googleapis.com/auth/drive".to_string()];

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth {
                scopes: required_scopes.clone(),
            },
            provider_scopes: &required_scopes,
            requester_extension: &ExtensionId::new("google-drive").unwrap(),
        })
        .await
        .expect_err("unscoped OAuth accounts must not satisfy scoped Google requirements");

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_reuses_gsuite_owned_google_account_for_gsuite_requester() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let access_secret = SecretHandle::new("google-drive-access").unwrap();
    let calendar_scope =
        ProviderScope::new("https://www.googleapis.com/auth/calendar.readonly").unwrap();
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("drive google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(ExtensionId::new("google-drive").unwrap()),
            granted_extensions: Vec::new(),
            access_secret: Some(access_secret.clone()),
            refresh_secret: None,
            scopes: vec![calendar_scope.clone()],
        })
        .await
        .unwrap();
    let resolver = resolver_with_accounts(accounts);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[calendar_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("google-calendar").unwrap(),
        })
        .await
        .unwrap();

    assert_eq!(resolved.handle, access_secret);
}

#[tokio::test]
async fn resolver_does_not_share_unbound_google_account_with_third_party_requester() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let google_scope =
        ProviderScope::new("https://www.googleapis.com/auth/gmail.readonly").unwrap();
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("work google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("google-access").unwrap()),
            refresh_secret: None,
            scopes: vec![google_scope.clone()],
        })
        .await
        .unwrap();
    let resolver = resolver_with_accounts(accounts);

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[google_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("third-party").unwrap(),
        })
        .await
        .expect_err("third-party requesters need an explicit Google account grant");

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_allows_google_account_explicitly_granted_to_third_party_requester() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let requester = ExtensionId::new("third-party").unwrap();
    let access_secret = SecretHandle::new("granted-google-access").unwrap();
    let google_scope =
        ProviderScope::new("https://www.googleapis.com/auth/gmail.readonly").unwrap();
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("shared google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::SharedAdminManaged,
            owner_extension: None,
            granted_extensions: vec![requester.clone()],
            access_secret: Some(access_secret.clone()),
            refresh_secret: None,
            scopes: vec![google_scope.clone()],
        })
        .await
        .unwrap();
    let resolver = resolver_with_accounts(accounts);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[google_scope.as_str().to_string()],
            requester_extension: &requester,
        })
        .await
        .expect("explicit grants should still authorize third-party requesters");

    assert_eq!(resolved.handle, access_secret);
}

#[tokio::test]
async fn resolver_maps_unconfigured_account_status_to_auth_required() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("github").unwrap(),
            label: CredentialAccountLabel::new("work github").unwrap(),
            status: CredentialAccountStatus::PendingSetup,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: None,
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let resolver = resolver_with_accounts(accounts);

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .unwrap_err();

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_maps_configured_account_without_access_secret_to_backend() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("github").unwrap(),
            label: CredentialAccountLabel::new("work github").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: None, // Configured but missing secret — data corruption
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let resolver = resolver_with_accounts(accounts);

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .unwrap_err();

    // Data corruption: should be Backend, not AuthRequired (re-auth would not fix it).
    // The durable product-auth store preserves Configured ↔ access_secret=Some,
    // so this state cannot arise from legitimate cleanup or rotation paths.
    assert_eq!(error, CredentialStageError::Backend);
}

#[tokio::test]
async fn activation_preflight_maps_configured_account_without_access_secret_to_backend() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("github").unwrap(),
            label: CredentialAccountLabel::new("corrupt github").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: None,
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let selector = ProductAuthRuntimeCredentialAccountSelector::new(accounts);

    let error = missing_runtime_credential_auth_requirements(
        &selector,
        &scope,
        vec![RuntimeCredentialAuthRequirement {
            provider: RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: Default::default(),
            requester_extension: ExtensionId::new("github").unwrap(),
            provider_scopes: Vec::new(),
        }],
    )
    .await
    .unwrap_err();

    assert_eq!(error, CredentialStageError::Backend);
}

#[tokio::test]
async fn resolver_uses_most_recent_account_across_multiple_reusable_logins() {
    // Runtime default rule (#auth-gate-reuse): when several reusable,
    // unbound accounts match the same provider — even under different
    // labels — the gate has no interactive picker, so the resolver selects
    // the most-recently-used account rather than failing with
    // `AccountSelectionRequired` (which re-prompted on every call). The
    // setup-time picker controls which one wins by bumping its recency.
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let latest_secret = SecretHandle::new("work-token").unwrap();
    // Two reusable accounts for the same provider under distinct labels.
    // The second one is created later, so it is the most-recently-used.
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope.clone(),
            provider: AuthProviderId::new("github").unwrap(),
            label: CredentialAccountLabel::new("personal github").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("personal-token").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("github").unwrap(),
            label: CredentialAccountLabel::new("work github").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(latest_secret.clone()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let resolver = resolver_with_accounts(accounts);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .expect("runtime must resolve to the most-recent reusable account, not re-prompt");

    assert_eq!(resolved.handle, latest_secret);
}
