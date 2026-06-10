use super::*;

#[tokio::test]
async fn filesystem_accounts_survive_service_recreation() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

    let created = service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("google-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("google-refresh").unwrap()),
            scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
        })
        .await
        .unwrap();

    let recreated = test_service(Arc::clone(&filesystem), secret_store);
    let loaded = recreated
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            created.id,
        ))
        .await
        .unwrap()
        .expect("account should be durable");
    assert_eq!(loaded.id, created.id);
    assert_eq!(loaded.access_secret, created.access_secret);

    let page = recreated
        .list_accounts(CredentialAccountListRequest::new(scope, google_provider()))
        .await
        .unwrap();
    assert_eq!(page.accounts.len(), 1);
    assert_eq!(page.accounts[0].id, created.id);
}

#[tokio::test]
async fn filesystem_account_record_source_projects_session_scoped_accounts_for_runtime_owner() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let mut setup_scope = test_scope();
    setup_scope.surface = AuthSurface::Callback;
    setup_scope.resource.thread_id = Some(ThreadId::new("thread-auth-account").unwrap());
    setup_scope.session_id = Some(AuthSessionId::new("session-auth-account").unwrap());
    let service = FilesystemAuthProductServices::new(filesystem, secret_store);
    let account = service
        .create_account(NewCredentialAccount {
            scope: setup_scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("session-scoped-access").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let mut runtime_resource = setup_scope.resource.clone();
    runtime_resource.invocation_id = InvocationId::new();
    let runtime_scope = AuthProductScope::new(runtime_resource, AuthSurface::Api);

    let projected = service.accounts_for_owner(&runtime_scope).await.unwrap();
    let projected_account = projected
        .iter()
        .find(|candidate| candidate.id == account.id)
        .expect("runtime owner projection should include session-scoped setup account");

    assert_eq!(projected_account.scope.session_id, setup_scope.session_id);
    assert_eq!(projected_account.provider, google_provider());
}

#[tokio::test]
async fn filesystem_account_record_source_rejects_malformed_scan_records() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), secret_store);
    service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("valid-account-access").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();

    let malformed_account_id = ironclaw_auth::CredentialAccountId::new();
    let malformed_path = super::paths::account_path(&scope, malformed_account_id)
        .expect("account path derivation must succeed");
    let malformed = ironclaw_filesystem::Entry::bytes(b"{ malformed account json".to_vec())
        .with_content_type(ironclaw_filesystem::ContentType::json());
    filesystem
        .put(
            &scope.resource,
            &malformed_path,
            malformed,
            ironclaw_filesystem::CasExpectation::Absent,
        )
        .await
        .expect("malformed account fixture must write");

    assert!(
        matches!(
            service.accounts_for_owner(&scope).await,
            Err(AuthProductError::BackendUnavailable)
        ),
        "runtime owner scans should fail loudly on malformed account records"
    );

    assert!(
        matches!(
            service.read_account(&scope, malformed_account_id).await,
            Err(AuthProductError::BackendUnavailable)
        ),
        "exact account reads should remain strict"
    );
}

// ─── fix: durable SecretCleanupService purges secrets on Uninstall ───────────

#[tokio::test]
async fn filesystem_cleanup_for_lifecycle_deactivates_owner_and_revokes_on_uninstall() {
    use ironclaw_auth::{SecretCleanupAction, SecretCleanupRequest, SecretCleanupService};
    use ironclaw_host_api::ExtensionId;

    let filesystem = test_filesystem();
    let concrete_secret_store = Arc::new(InMemorySecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

    let ext_id = ExtensionId::new("test-ext").unwrap();
    let access = SecretHandle::new("ext-access").unwrap();
    let refresh = SecretHandle::new("ext-refresh").unwrap();

    // Seed secret material.
    use secrecy::SecretString;
    concrete_secret_store
        .put(
            scope.resource.clone(),
            access.clone(),
            SecretString::from("access-material"),
        )
        .await
        .unwrap();
    concrete_secret_store
        .put(
            scope.resource.clone(),
            refresh.clone(),
            SecretString::from("refresh-material"),
        )
        .await
        .unwrap();

    // Create an extension-owned account.
    let account = service
        .create_account(ironclaw_auth::NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(ext_id.clone()),
            granted_extensions: vec![],
            access_secret: Some(access.clone()),
            refresh_secret: Some(refresh.clone()),
            scopes: vec![],
        })
        .await
        .unwrap();

    // Deactivate: account should be Inactive; secrets retained.
    let deactivate_report = service
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: scope.clone(),
            extension_id: ext_id.clone(),
            action: SecretCleanupAction::Deactivate,
        })
        .await
        .unwrap();
    assert_eq!(deactivate_report.retained_accounts, vec![account.id]);
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access)
            .await
            .unwrap()
            .is_some(),
        "Deactivate must retain secret material"
    );

    // Uninstall: account revoked, secrets purged from SecretStore.
    let uninstall_report = service
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: scope.clone(),
            extension_id: ext_id.clone(),
            action: SecretCleanupAction::Uninstall,
        })
        .await
        .unwrap();
    assert_eq!(uninstall_report.revoked_accounts, vec![account.id]);
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access)
            .await
            .unwrap()
            .is_none(),
        "Uninstall must delete access secret from SecretStore"
    );
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &refresh)
            .await
            .unwrap()
            .is_none(),
        "Uninstall must delete refresh secret from SecretStore"
    );
}

// ─── validate_account_list_request boundary cases ────────────────────────────

#[tokio::test]
async fn filesystem_list_accounts_rejects_zero_and_oversized_limit() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    // limit = 0.
    let err = service
        .list_accounts(
            CredentialAccountListRequest::new(scope.clone(), google_provider()).with_limit(0),
        )
        .await
        .expect_err("limit=0 must be rejected");
    assert!(matches!(err, AuthProductError::InvalidRequest { .. }));

    // limit = MAX + 1.
    let err = service
        .list_accounts(
            CredentialAccountListRequest::new(scope.clone(), google_provider())
                .with_limit(CredentialAccountListRequest::MAX_LIMIT + 1),
        )
        .await
        .expect_err("limit > MAX must be rejected");
    assert!(matches!(err, AuthProductError::InvalidRequest { .. }));

    // Cursor + pagination: 2 accounts, limit=1 → next_cursor present.
    for i in 0..2u8 {
        service
            .create_account(ironclaw_auth::NewCredentialAccount {
                scope: scope.clone(),
                provider: google_provider(),
                label: CredentialAccountLabel::new(format!("User {i}")).unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: vec![],
                access_secret: None,
                refresh_secret: None,
                scopes: vec![],
            })
            .await
            .unwrap();
    }
    let page = service
        .list_accounts(
            CredentialAccountListRequest::new(scope.clone(), google_provider()).with_limit(1),
        )
        .await
        .unwrap();
    assert_eq!(page.accounts.len(), 1);
    assert!(
        page.next_cursor.is_some(),
        "second page must have next_cursor"
    );
}

// ─── fix: grant-removal on non-owner account in cleanup_for_lifecycle ─────────

#[tokio::test]
async fn filesystem_cleanup_removes_grant_from_non_owner_account() {
    use ironclaw_auth::{SecretCleanupAction, SecretCleanupRequest, SecretCleanupService};
    use ironclaw_host_api::ExtensionId;

    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    let ext_id = ExtensionId::new("granted-ext").unwrap();

    // Create user-reusable account with a grant to ext_id (not owner).
    let account = service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![ext_id.clone()],
            access_secret: None,
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();

    let report = service
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: scope.clone(),
            extension_id: ext_id.clone(),
            action: SecretCleanupAction::Uninstall,
        })
        .await
        .unwrap();

    assert_eq!(
        report.removed_grants,
        vec![account.id],
        "grant must be reported removed"
    );
    assert!(
        report.revoked_accounts.is_empty(),
        "non-owner account must not be revoked"
    );

    let updated = service
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            account.id,
        ))
        .await
        .unwrap()
        .expect("account must still exist");
    assert!(
        !updated.granted_extensions.contains(&ext_id),
        "grant must be removed from account record"
    );
    assert_eq!(
        updated.status,
        CredentialAccountStatus::Configured,
        "status must be unchanged"
    );
}

// ─── fix: select_unique_configured_account and select_configured_account ──────

#[tokio::test]
async fn filesystem_select_unique_configured_account_single_and_multi() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    // No accounts — CredentialMissing.
    let err = service
        .select_unique_configured_account(CredentialAccountSelectionRequest::new(
            scope.clone(),
            google_provider(),
        ))
        .await
        .expect_err("no accounts must return CredentialMissing");
    assert_eq!(err, AuthProductError::CredentialMissing);

    let a1 = service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: None,
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();

    // One configured — returns it.
    let selected = service
        .select_unique_configured_account(CredentialAccountSelectionRequest::new(
            scope.clone(),
            google_provider(),
        ))
        .await
        .unwrap();
    assert_eq!(selected.id, a1.id);

    // Second configured — AccountSelectionRequired.
    service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: CredentialAccountLabel::new("Alice Google 2").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: None,
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();

    let err = service
        .select_unique_configured_account(CredentialAccountSelectionRequest::new(
            scope.clone(),
            google_provider(),
        ))
        .await
        .expect_err("two configured must require selection");
    assert_eq!(err, AuthProductError::AccountSelectionRequired);
}

#[tokio::test]
async fn filesystem_select_configured_account_validates_provider_and_rejects_missing() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    let account = service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: None,
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();

    // Happy path.
    let selected = service
        .select_configured_account(CredentialAccountChoiceRequest::new(
            scope.clone(),
            google_provider(),
            account.id,
        ))
        .await
        .unwrap();
    assert_eq!(selected.id, account.id);

    // Non-existent account.
    let err = service
        .select_configured_account(CredentialAccountChoiceRequest::new(
            scope.clone(),
            google_provider(),
            CredentialAccountId::new(),
        ))
        .await
        .expect_err("missing account must return CredentialMissing");
    assert_eq!(err, AuthProductError::CredentialMissing);

    // Wrong provider is intentionally indistinguishable from a missing account
    // at the public boundary, so account ids cannot be used as provider oracles.
    let err = service
        .select_configured_account(CredentialAccountChoiceRequest::new(
            scope.clone(),
            AuthProviderId::new("github").unwrap(),
            account.id,
        ))
        .await
        .expect_err("wrong provider must return CredentialMissing");
    assert_eq!(err, AuthProductError::CredentialMissing);
}

// ─── tests: update_status, project_credential_recovery, CredentialSetupService update ───

#[tokio::test]
async fn filesystem_update_status_and_cross_scope_rejection() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    let account = service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: None,
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();

    let updated = service
        .update_status(&scope, account.id, CredentialAccountStatus::Inactive)
        .await
        .unwrap();
    assert_eq!(updated.status, CredentialAccountStatus::Inactive);

    // Non-existent account.
    let err = service
        .update_status(
            &scope,
            CredentialAccountId::new(),
            CredentialAccountStatus::Inactive,
        )
        .await
        .expect_err("missing account must return CredentialMissing");
    assert_eq!(err, AuthProductError::CredentialMissing);
}

#[tokio::test]
async fn filesystem_project_credential_recovery_returns_setup_required_when_empty() {
    use ironclaw_auth::CredentialRecoveryRequest;
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    // No accounts → setup_required.
    let recovery = service
        .project_credential_recovery(CredentialRecoveryRequest::new(
            scope.clone(),
            google_provider(),
        ))
        .await
        .unwrap();
    use ironclaw_auth::CredentialRecoveryState;
    assert!(
        matches!(
            recovery.state,
            CredentialRecoveryState::SetupRequired { .. }
        ),
        "no accounts must return setup_required"
    );

    // One configured account → configured.
    let account = service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: None,
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();

    let recovery = service
        .project_credential_recovery(CredentialRecoveryRequest::new(
            scope.clone(),
            google_provider(),
        ))
        .await
        .unwrap();
    let CredentialRecoveryState::Configured { selected_account } = &recovery.state else {
        panic!(
            "single configured account must return Configured state, got: {:?}",
            recovery.state
        );
    };
    assert_eq!(selected_account.id, account.id);
}

#[tokio::test]
async fn filesystem_credential_setup_service_update_path() {
    use ironclaw_auth::{
        CredentialAccountMutation, CredentialAccountUpdate, CredentialSetupService,
    };
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    let account = service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(SecretHandle::new("old-access").unwrap()),
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();

    let new_handle = SecretHandle::new("new-access").unwrap();
    let updated = service
        .create_or_update_account(CredentialAccountMutation::Update(CredentialAccountUpdate {
            account_id: account.id,
            account: NewCredentialAccount {
                scope: scope.clone(),
                provider: google_provider(),
                label: account_label(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: vec![],
                access_secret: Some(new_handle.clone()),
                refresh_secret: None,
                scopes: vec![],
            },
        }))
        .await
        .unwrap();
    assert_eq!(updated.access_secret, Some(new_handle));
}

// ─── tests: get_account cross-scope rejection ─────────────────────────────────

#[tokio::test]
async fn filesystem_get_account_cross_scope_returns_cross_scope_denied() {
    use ironclaw_auth::AuthSurface;
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

    let account = service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: None,
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();

    // Same resource but different surface → CrossScopeDenied.
    let cli_scope = AuthProductScope::new(scope.resource.clone(), AuthSurface::Cli);
    let service2 = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));
    let result = service2
        .get_account(CredentialAccountLookupRequest::new(cli_scope, account.id))
        .await;
    // The account doesn't exist in the CLI path (different path on filesystem), so None.
    assert!(
        result.unwrap().is_none(),
        "account written under web scope must not be visible under cli scope"
    );
}
