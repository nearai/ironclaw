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
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));

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
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new_with_refresh(
            accounts.clone(),
            accounts.clone(),
        ),
    ));

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
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new_with_refresh(
            accounts.clone(),
            accounts.clone(),
        ),
    ));

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
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));
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
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));

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
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));

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
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));

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
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));

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
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));

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
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));
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
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));
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
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));
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
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));

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
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));

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
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));

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
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));

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
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));

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
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));

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

#[tokio::test]
async fn resolver_uses_latest_duplicate_user_reusable_account() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let first_secret = SecretHandle::new("old-token").unwrap();
    let latest_secret = SecretHandle::new("new-token").unwrap();
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope.clone(),
            provider: AuthProviderId::new("github").unwrap(),
            label: CredentialAccountLabel::new("GitHub").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(first_secret),
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
            label: CredentialAccountLabel::new("GitHub").unwrap(),
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
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));

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

    assert_eq!(resolved.handle, latest_secret);
}

/// Direct reproduction of the reported bug (#auth-gate-reuse): a single
/// Google login authenticated through different gate/setup surfaces ends up
/// stored as multiple reusable accounts under capability-derived labels
/// ("gmail google", "google-calendar google"). The runtime resolver must
/// pick the most-recent usable credential instead of returning
/// `AuthRequired`, which re-prompted the user on every gmail/calendar call.
#[tokio::test]
async fn resolver_resolves_google_capability_labeled_duplicates() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let gmail_scope = ProviderScope::new("https://www.googleapis.com/auth/gmail.modify").unwrap();
    let latest_secret = SecretHandle::new("calendar-surface-token").unwrap();
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope.clone(),
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("gmail google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("gmail-surface-token").unwrap()),
            refresh_secret: None,
            scopes: vec![gmail_scope.clone()],
        })
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("google-calendar google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(latest_secret.clone()),
            refresh_secret: None,
            scopes: vec![gmail_scope.clone()],
        })
        .await
        .unwrap();
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[gmail_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("gmail").unwrap(),
        })
        .await
        .expect("capability-labeled google duplicates must resolve, not re-prompt");

    assert_eq!(resolved.handle, latest_secret);
}

#[tokio::test]
async fn resolver_does_not_auto_select_mixed_reusable_and_extension_owned_accounts() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let requester = ExtensionId::new("gmail").unwrap();
    let google_scope =
        ProviderScope::new("https://www.googleapis.com/auth/gmail.readonly").unwrap();
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope.clone(),
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("reusable google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("reusable-token").unwrap()),
            refresh_secret: None,
            scopes: vec![google_scope.clone()],
        })
        .await
        .unwrap();
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("extension google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(requester.clone()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("extension-token").unwrap()),
            refresh_secret: None,
            scopes: vec![google_scope.clone()],
        })
        .await
        .unwrap();
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[google_scope.as_str().to_string()],
            requester_extension: &requester,
        })
        .await
        .expect_err("mixed ownership must require explicit account selection");

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_does_not_auto_select_mixed_reusable_and_shared_admin_accounts() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let requester = ExtensionId::new("gmail").unwrap();
    let google_scope =
        ProviderScope::new("https://www.googleapis.com/auth/gmail.readonly").unwrap();
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope.clone(),
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("reusable google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("reusable-token").unwrap()),
            refresh_secret: None,
            scopes: vec![google_scope.clone()],
        })
        .await
        .unwrap();
    accounts
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("shared google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::SharedAdminManaged,
            owner_extension: None,
            granted_extensions: vec![requester.clone()],
            access_secret: Some(SecretHandle::new("shared-token").unwrap()),
            refresh_secret: None,
            scopes: vec![google_scope.clone()],
        })
        .await
        .unwrap();
    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(accounts),
    ));

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[google_scope.as_str().to_string()],
            requester_extension: &requester,
        })
        .await
        .expect_err("mixed sharing semantics must require explicit account selection");

    assert_eq!(error, CredentialStageError::AuthRequired);
}
