use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use crate::common::*;
use async_trait::async_trait;
use ironclaw_auth::{OAuthProviderRefresh, ProviderBackedCredentialAccountService};
use tokio::sync::Notify;

struct BlockingRefreshProvider {
    inner: Arc<InMemoryAuthProductServices>,
    refresh_calls: AtomicUsize,
    refresh_started: Notify,
    release_refresh: Notify,
    refresh_released: AtomicBool,
}

impl BlockingRefreshProvider {
    fn new(inner: Arc<InMemoryAuthProductServices>) -> Self {
        Self {
            inner,
            refresh_calls: AtomicUsize::new(0),
            refresh_started: Notify::new(),
            release_refresh: Notify::new(),
            refresh_released: AtomicBool::new(false),
        }
    }

    fn refresh_call_count(&self) -> usize {
        self.refresh_calls.load(Ordering::SeqCst)
    }

    async fn wait_for_first_refresh_start(&self) {
        self.refresh_started.notified().await;
    }

    fn release_refresh(&self) {
        self.refresh_released.store(true, Ordering::SeqCst);
        self.release_refresh.notify_waiters();
    }
}

fn provider_backed_auth(
    services: Arc<InMemoryAuthProductServices>,
) -> Arc<ProviderBackedCredentialAccountService> {
    Arc::new(ProviderBackedCredentialAccountService::new(
        services.clone(),
        services.clone(),
        services,
    ))
}

#[async_trait]
impl AuthProviderClient for BlockingRefreshProvider {
    async fn exchange_callback(
        &self,
        context: ironclaw_auth::OAuthProviderExchangeContext,
        request: ironclaw_auth::OAuthProviderCallbackRequest,
    ) -> Result<ironclaw_auth::OAuthProviderExchange, AuthProductError> {
        self.inner.exchange_callback(context, request).await
    }

    async fn refresh_token(
        &self,
        request: OAuthProviderRefreshRequest,
    ) -> Result<OAuthProviderRefresh, AuthProductError> {
        self.refresh_calls.fetch_add(1, Ordering::SeqCst);
        self.refresh_started.notify_one();
        while !self.refresh_released.load(Ordering::SeqCst) {
            self.release_refresh.notified().await;
        }
        self.inner.refresh_token(request).await
    }
}

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
        .get_account(CredentialAccountLookupRequest::new(
            owner.clone(),
            account.id,
        ))
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
    let services = Arc::new(InMemoryAuthProductServices::new());
    let auth = provider_backed_auth(services.clone());
    let owner = scope("alice");
    let account = auth
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

    let report = auth
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
        .get_account(CredentialAccountLookupRequest::new(
            owner.clone(),
            account.id,
        ))
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
async fn concurrent_refreshes_for_same_account_are_single_flight() {
    let services = Arc::new(InMemoryAuthProductServices::new());
    let provider_client = Arc::new(BlockingRefreshProvider::new(services.clone()));
    let auth = Arc::new(ProviderBackedCredentialAccountService::new(
        services.clone(),
        services.clone(),
        provider_client.clone(),
    ));

    let owner = scope("alice");
    let account = auth
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("work"),
            status: CredentialAccountStatus::Expired,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-concurrent-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("github-concurrent-refresh").unwrap()),
            scopes: provider_scopes(&["repo"]),
        })
        .await
        .expect("expired account");

    let first_refresh = {
        let auth = auth.clone();
        let owner = owner.clone();
        tokio::spawn(async move {
            auth.refresh_account(CredentialRefreshRequest::new(owner, provider(), account.id))
                .await
        })
    };

    provider_client.wait_for_first_refresh_start().await;

    let second_refresh = {
        let auth = auth.clone();
        let owner = owner.clone();
        tokio::spawn(async move {
            auth.refresh_account(CredentialRefreshRequest::new(owner, provider(), account.id))
                .await
        })
    };

    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    assert_eq!(provider_client.refresh_call_count(), 1);

    provider_client.release_refresh();

    let first_report = first_refresh
        .await
        .expect("first refresh task")
        .expect("first refresh");
    let second_report = second_refresh
        .await
        .expect("second refresh task")
        .expect("second refresh");

    assert!(first_report.refreshed);
    assert!(!second_report.refreshed);
    assert_eq!(provider_client.refresh_call_count(), 1);

    let stored = auth
        .get_account(CredentialAccountLookupRequest::new(
            owner.clone(),
            account.id,
        ))
        .await
        .expect("lookup")
        .expect("refreshed account");
    assert_eq!(stored.status, CredentialAccountStatus::Configured);
    assert_eq!(
        second_report.account.status,
        CredentialAccountStatus::Configured
    );
}

#[tokio::test]
async fn stale_refresh_success_does_not_overwrite_concurrent_refresh() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let concurrent_access = SecretHandle::new("github-concurrent-access").unwrap();
    let concurrent_refresh = SecretHandle::new("github-concurrent-refresh").unwrap();
    let account = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("work"),
            status: CredentialAccountStatus::Expired,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-stale-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("github-stale-refresh").unwrap()),
            scopes: provider_scopes(&["repo"]),
        })
        .await
        .expect("expired account");
    services.complete_refresh_during_next_provider_call_for_tests(
        account.id,
        concurrent_access.clone(),
        concurrent_refresh.clone(),
    );

    let error = services
        .refresh_account(CredentialRefreshRequest::new(
            owner.clone(),
            provider(),
            account.id,
        ))
        .await
        .expect_err("stale refresh result cannot overwrite newer credentials");

    assert_eq!(error, AuthProductError::RefreshFailed);
    let stored = services
        .get_account(CredentialAccountLookupRequest::new(
            owner.clone(),
            account.id,
        ))
        .await
        .expect("lookup")
        .expect("account");
    assert_eq!(stored.status, CredentialAccountStatus::Configured);
    assert_eq!(stored.access_secret, Some(concurrent_access));
    assert_eq!(stored.refresh_secret, Some(concurrent_refresh));
}

#[tokio::test]
async fn stale_refresh_failure_does_not_mark_concurrent_refresh_failed() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let concurrent_access = SecretHandle::new("github-concurrent-failed-access").unwrap();
    let concurrent_refresh = SecretHandle::new("github-concurrent-failed-refresh").unwrap();
    let account = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("work"),
            status: CredentialAccountStatus::Expired,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-old-failed-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("github-old-failed-refresh").unwrap()),
            scopes: provider_scopes(&["repo"]),
        })
        .await
        .expect("expired account");
    services.complete_refresh_during_next_provider_call_for_tests(
        account.id,
        concurrent_access.clone(),
        concurrent_refresh.clone(),
    );
    services.fail_next_refresh_for_tests(account.id);

    let report = services
        .refresh_account(CredentialRefreshRequest::new(
            owner.clone(),
            provider(),
            account.id,
        ))
        .await
        .expect("stale failure reports current account state");

    assert!(!report.refreshed);
    assert_eq!(report.account.status, CredentialAccountStatus::Configured);
    assert_eq!(report.recovery.kind(), CredentialRecoveryKind::Configured);
    let stored = services
        .get_account(CredentialAccountLookupRequest::new(
            owner.clone(),
            account.id,
        ))
        .await
        .expect("lookup")
        .expect("account");
    assert_eq!(stored.status, CredentialAccountStatus::Configured);
    assert_eq!(stored.access_secret, Some(concurrent_access));
    assert_eq!(stored.refresh_secret, Some(concurrent_refresh));
}

#[tokio::test]
async fn credential_refresh_without_refresh_secret_becomes_recoverable_status() {
    let services = Arc::new(InMemoryAuthProductServices::new());
    let auth = provider_backed_auth(services.clone());
    let owner = scope("alice");
    let account = auth
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

    let report = auth
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
        .get_account(CredentialAccountLookupRequest::new(
            owner.clone(),
            account.id,
        ))
        .await
        .expect("lookup")
        .expect("failed account");
    assert_eq!(failed.status, CredentialAccountStatus::RefreshFailed);
}

#[tokio::test]
async fn provider_backed_refresh_preserves_requester_for_authorized_extensions() {
    let services = Arc::new(InMemoryAuthProductServices::new());
    let auth = provider_backed_auth(services.clone());
    let owner = scope("alice");
    let extension_owned = ExtensionId::new("github-extension-owned").unwrap();
    let shared_admin = ExtensionId::new("github-shared-admin").unwrap();

    let extension_account = auth
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("extension owned"),
            status: CredentialAccountStatus::Expired,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(extension_owned.clone()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-extension-owned-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("github-extension-owned-refresh").unwrap()),
            scopes: provider_scopes(&["repo"]),
        })
        .await
        .expect("extension-owned account");

    let extension_report = auth
        .refresh_account(
            CredentialRefreshRequest::new(owner.clone(), provider(), extension_account.id)
                .for_extension(extension_owned.clone()),
        )
        .await
        .expect("extension-owned refresh");
    assert!(extension_report.refreshed);
    assert_eq!(
        extension_report.account.status,
        CredentialAccountStatus::Configured
    );
    assert_eq!(
        extension_report.recovery.kind(),
        CredentialRecoveryKind::Configured
    );
    assert_eq!(
        extension_report
            .recovery
            .selected_account()
            .map(|account| account.id),
        Some(extension_account.id)
    );

    let shared_account = auth
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("shared admin"),
            status: CredentialAccountStatus::Expired,
            ownership: CredentialOwnership::SharedAdminManaged,
            owner_extension: None,
            granted_extensions: vec![shared_admin.clone()],
            access_secret: Some(SecretHandle::new("github-shared-admin-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("github-shared-admin-refresh").unwrap()),
            scopes: provider_scopes(&["repo"]),
        })
        .await
        .expect("shared-admin account");

    let shared_report = auth
        .refresh_account(
            CredentialRefreshRequest::new(owner, provider(), shared_account.id)
                .for_extension(shared_admin),
        )
        .await
        .expect("shared-admin refresh");
    assert!(shared_report.refreshed);
    assert_eq!(
        shared_report.account.status,
        CredentialAccountStatus::Configured
    );
    assert_eq!(
        shared_report.recovery.kind(),
        CredentialRecoveryKind::Configured
    );
    assert_eq!(
        shared_report
            .recovery
            .selected_account()
            .map(|account| account.id),
        Some(shared_account.id)
    );
}

#[tokio::test]
async fn credential_refresh_rejects_terminal_statuses_even_with_refresh_secret() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");

    for (status, label_value, secret_suffix) in [
        (
            CredentialAccountStatus::Revoked,
            "terminal revoked",
            "revoked",
        ),
        (
            CredentialAccountStatus::Inactive,
            "terminal inactive",
            "inactive",
        ),
        (
            CredentialAccountStatus::PendingSetup,
            "terminal pending",
            "pending",
        ),
    ] {
        let account = services
            .create_account(NewCredentialAccount {
                scope: owner.clone(),
                provider: provider(),
                label: label(label_value),
                status,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(
                    SecretHandle::new(format!("github-terminal-access-{secret_suffix}")).unwrap(),
                ),
                refresh_secret: Some(
                    SecretHandle::new(format!("github-terminal-refresh-{secret_suffix}")).unwrap(),
                ),
                scopes: provider_scopes(&["repo"]),
            })
            .await
            .expect("terminal account");

        let error = services
            .refresh_account(CredentialRefreshRequest::new(
                owner.clone(),
                provider(),
                account.id,
            ))
            .await
            .expect_err("terminal account refresh is rejected");
        assert_eq!(error, AuthProductError::CredentialMissing);

        let stored = services
            .get_account(CredentialAccountLookupRequest::new(
                owner.clone(),
                account.id,
            ))
            .await
            .expect("lookup")
            .expect("terminal account remains");
        assert_eq!(stored.status, status);
    }
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

#[tokio::test]
async fn credential_refresh_invalid_grant_marks_account_revoked() {
    let services = Arc::new(InMemoryAuthProductServices::new());
    let auth = provider_backed_auth(services.clone());
    let owner = scope("alice");
    let account = auth
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("work"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-invalid-grant-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("github-invalid-grant-refresh").unwrap()),
            scopes: provider_scopes(&["repo"]),
        })
        .await
        .expect("configured account");
    services.invalid_grant_next_refresh_for_tests(account.id);

    let report = auth
        .refresh_account(CredentialRefreshRequest::new(
            owner.clone(),
            provider(),
            account.id,
        ))
        .await
        .expect("invalid grant is projected, not propagated");

    assert!(!report.refreshed);
    assert_eq!(report.account.status, CredentialAccountStatus::Revoked);
    assert_eq!(
        report.recovery.kind(),
        CredentialRecoveryKind::ReauthorizeRequired
    );
    assert_eq!(
        report.recovery.reason,
        CredentialRecoveryReason::AccountRevoked
    );
    assert_eq!(report.recovery.choices().len(), 1);
    assert_eq!(report.recovery.choices()[0].id, account.id);

    let revoked = services
        .get_account(CredentialAccountLookupRequest::new(
            owner.clone(),
            account.id,
        ))
        .await
        .expect("lookup")
        .expect("revoked account");
    assert_eq!(revoked.status, CredentialAccountStatus::Revoked);

    let serialized = serde_json::to_string(&report).expect("serialize report");
    assert!(!serialized.contains("github-invalid-grant-access"));
    assert!(!serialized.contains("github-invalid-grant-refresh"));
    assert!(!serialized.contains("RAW_PROVIDER_ERROR_SENTINEL"));
}

#[test]
fn provider_refresh_request_debug_redacts_secret_handle() {
    let request = OAuthProviderRefreshRequest {
        provider: provider(),
        scope: scope("alice"),
        account_id: ironclaw_auth::CredentialAccountId::new(),
        refresh_secret: SecretHandle::new("github-debug-refresh-secret").unwrap(),
        scopes: provider_scopes(&["repo"]),
    };
    let rendered = format!("{request:?}");
    assert!(rendered.contains("[REDACTED]"));
    assert!(!rendered.contains("github-debug-refresh-secret"));
}
