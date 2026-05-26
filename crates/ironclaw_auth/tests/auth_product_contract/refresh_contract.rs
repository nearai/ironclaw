use crate::common::*;

#[tokio::test]
async fn credential_refresh_updates_account_through_provider_boundary() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let old_access = SecretHandle::new("github-refresh-old-access").unwrap();
    let old_refresh = SecretHandle::new("github-refresh-old-refresh").unwrap();
    let account = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("work"),
            status: CredentialAccountStatus::Expired,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(old_access.clone()),
            refresh_secret: Some(old_refresh.clone()),
            scopes: provider_scopes(&["repo"]),
        })
        .await
        .expect("expired account");

    let report = services
        .refresh_account(CredentialRefreshRequest::new(
            owner.clone(),
            provider(),
            account.id,
        ))
        .await
        .expect("refresh account");

    assert!(report.refreshed);
    assert_eq!(report.account.id, account.id);
    assert_eq!(report.account.status, CredentialAccountStatus::Configured);
    assert_eq!(report.recovery.kind(), CredentialRecoveryKind::Configured);
    assert_eq!(
        report.recovery.selected_account().map(|account| account.id),
        Some(account.id)
    );

    let refreshed = services
        .get_account(&owner, account.id)
        .await
        .expect("lookup")
        .expect("refreshed account");
    assert_eq!(refreshed.status, CredentialAccountStatus::Configured);
    assert_ne!(refreshed.access_secret, Some(old_access));
    assert_ne!(refreshed.refresh_secret, Some(old_refresh));
    assert_eq!(refreshed.scopes, provider_scopes(&["repo"]));

    let serialized = serde_json::to_string(&report).expect("serialize report");
    assert!(!serialized.contains("github-refresh-old-access"));
    assert!(!serialized.contains("github-refresh-old-refresh"));
    assert!(!serialized.contains("oauth-refreshed"));
    assert!(!serialized.contains("RAW_PROVIDER_ERROR_SENTINEL"));
}

#[tokio::test]
async fn credential_refresh_failure_becomes_recoverable_status() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let account = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("work"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-refresh-fail-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("github-refresh-fail-refresh").unwrap()),
            scopes: provider_scopes(&["repo"]),
        })
        .await
        .expect("configured account");
    services.fail_next_refresh_for_tests(account.id);

    let report = services
        .refresh_account(CredentialRefreshRequest::new(
            owner.clone(),
            provider(),
            account.id,
        ))
        .await
        .expect("refresh failure is projected");

    assert!(!report.refreshed);
    assert_eq!(
        report.account.status,
        CredentialAccountStatus::RefreshFailed
    );
    assert_eq!(
        report.recovery.kind(),
        CredentialRecoveryKind::ReauthorizeRequired
    );
    assert_eq!(
        report.recovery.reason,
        CredentialRecoveryReason::RefreshFailed
    );
    assert_eq!(report.recovery.choices().len(), 1);
    assert_eq!(report.recovery.choices()[0].id, account.id);

    let failed = services
        .get_account(&owner, account.id)
        .await
        .expect("lookup")
        .expect("failed account");
    assert_eq!(failed.status, CredentialAccountStatus::RefreshFailed);

    let serialized = serde_json::to_string(&report).expect("serialize report");
    assert!(!serialized.contains("github-refresh-fail-access"));
    assert!(!serialized.contains("github-refresh-fail-refresh"));
    assert!(!serialized.contains("RAW_PROVIDER_ERROR_SENTINEL"));
    assert!(!serialized.contains("/host/path"));
}

#[tokio::test]
async fn credential_refresh_without_refresh_secret_becomes_recoverable_status() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let account = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("work"),
            status: CredentialAccountStatus::Expired,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-refresh-no-refresh").unwrap()),
            refresh_secret: None,
            scopes: provider_scopes(&["repo"]),
        })
        .await
        .expect("expired account");

    let report = services
        .refresh_account(CredentialRefreshRequest::new(
            owner.clone(),
            provider(),
            account.id,
        ))
        .await
        .expect("missing refresh secret is projected");

    assert!(!report.refreshed);
    assert_eq!(
        report.account.status,
        CredentialAccountStatus::RefreshFailed
    );
    assert_eq!(
        report.recovery.kind(),
        CredentialRecoveryKind::ReauthorizeRequired
    );
    assert_eq!(
        report.recovery.reason,
        CredentialRecoveryReason::RefreshFailed
    );

    let failed = services
        .get_account(&owner, account.id)
        .await
        .expect("lookup")
        .expect("failed account");
    assert_eq!(failed.status, CredentialAccountStatus::RefreshFailed);
}

#[tokio::test]
async fn credential_refresh_revalidates_scope_provider_and_grants() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let granted_extension = ExtensionId::new("github-extension").unwrap();
    let other_extension = ExtensionId::new("other-extension").unwrap();
    let account = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("shared"),
            status: CredentialAccountStatus::Expired,
            ownership: CredentialOwnership::SharedAdminManaged,
            owner_extension: None,
            granted_extensions: vec![granted_extension.clone()],
            access_secret: Some(SecretHandle::new("github-shared-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("github-shared-refresh").unwrap()),
            scopes: provider_scopes(&["repo"]),
        })
        .await
        .expect("shared account");

    let no_requester = services
        .refresh_account(CredentialRefreshRequest::new(
            owner.clone(),
            provider(),
            account.id,
        ))
        .await
        .expect_err("shared account requires explicit grant");
    assert_eq!(no_requester, AuthProductError::CrossScopeDenied);

    let wrong_requester = services
        .refresh_account(
            CredentialRefreshRequest::new(owner.clone(), provider(), account.id)
                .for_extension(other_extension),
        )
        .await
        .expect_err("wrong extension cannot refresh shared account");
    assert_eq!(wrong_requester, AuthProductError::CrossScopeDenied);

    let provider_mismatch = services
        .refresh_account(
            CredentialRefreshRequest::new(
                owner.clone(),
                AuthProviderId::new("gitlab").unwrap(),
                account.id,
            )
            .for_extension(granted_extension.clone()),
        )
        .await
        .expect_err("provider mismatch rejected");
    assert_eq!(provider_mismatch, AuthProductError::CrossScopeDenied);

    let cross_scope = services
        .refresh_account(
            CredentialRefreshRequest::new(scope("bob"), provider(), account.id)
                .for_extension(granted_extension.clone()),
        )
        .await
        .expect_err("cross-scope refresh rejected");
    assert_eq!(cross_scope, AuthProductError::CrossScopeDenied);

    let report = services
        .refresh_account(
            CredentialRefreshRequest::new(owner, provider(), account.id)
                .for_extension(granted_extension),
        )
        .await
        .expect("granted extension can refresh");
    assert!(report.refreshed);
}

#[test]
fn provider_refresh_request_debug_redacts_secret_handle() {
    let request = OAuthProviderRefreshRequest {
        provider: provider(),
        account_id: ironclaw_auth::CredentialAccountId::new(),
        refresh_secret: SecretHandle::new("github-debug-refresh-secret").unwrap(),
        scopes: provider_scopes(&["repo"]),
    };
    let rendered = format!("{request:?}");
    assert!(rendered.contains("[REDACTED]"));
    assert!(!rendered.contains("github-debug-refresh-secret"));
}
