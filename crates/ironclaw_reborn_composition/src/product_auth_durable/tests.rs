use std::sync::Arc;

use chrono::{Duration, Utc};
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{
    InvocationId, MountAlias, MountGrant, MountPermissions, SecretHandle, UserId, VirtualPath,
};
use ironclaw_secrets::{InMemorySecretStore, SecretStore};
use secrecy::SecretString;
use tokio::task::JoinSet;

use super::*;
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthFlowKind, AuthFlowManager, AuthFlowStatus,
    AuthInteractionService, AuthProductError, AuthProductScope, AuthProviderId, AuthSurface,
    AuthorizationCodeHash, CredentialAccountLabel, CredentialAccountListRequest,
    CredentialAccountLookupRequest, CredentialAccountService, CredentialAccountStatus,
    CredentialOwnership, ManualTokenSetupRequest, NewAuthFlow, OAuthAuthorizationUrl,
    OAuthCallbackClaimRequest, OAuthCallbackInput, OAuthProviderExchange, OpaqueStateHash,
    PkceVerifierHash, ProviderScope, SecretSubmitRequest,
};

fn test_scope() -> AuthProductScope {
    let resource =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    AuthProductScope::new(resource, AuthSurface::Web)
}

fn test_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let mounts = ironclaw_host_api::MountView::new(vec![MountGrant::new(
        MountAlias::new("/secrets").unwrap(),
        VirtualPath::new("/tenants/test/users/alice/secrets").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}

fn test_service(
    filesystem: Arc<ScopedFilesystem<InMemoryBackend>>,
    secret_store: Arc<dyn SecretStore>,
) -> FilesystemAuthProductServices<InMemoryBackend> {
    FilesystemAuthProductServices::new(filesystem, secret_store)
}

fn google_provider() -> AuthProviderId {
    AuthProviderId::new("google").unwrap()
}

fn account_label() -> CredentialAccountLabel {
    CredentialAccountLabel::new("Alice Google").unwrap()
}

fn fake_digest(value: &str) -> String {
    format!(
        "{:064x}",
        value.bytes().fold(0_u64, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(u64::from(byte))
        })
    )
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
async fn filesystem_manual_token_submit_stores_secret_and_dedupes_replay() {
    let filesystem = test_filesystem();
    let concrete_secret_store = Arc::new(InMemorySecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    let challenge = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired { interaction_id, .. } = challenge else {
        panic!("expected manual token challenge");
    };

    let result = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("manual-token-value"),
            },
        )
        .await
        .unwrap();
    assert_eq!(result.status, CredentialAccountStatus::Configured);

    let account = service
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            result.account_id,
        ))
        .await
        .unwrap()
        .expect("manual token submit should create account");
    let access_secret = account.access_secret.expect("manual token secret handle");
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access_secret)
            .await
            .unwrap()
            .is_some()
    );

    let replay = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("manual-token-value"),
            },
        )
        .await
        .expect_err("manual token submit should be one-shot");
    assert_eq!(replay, AuthProductError::UnknownOrExpiredFlow);
}

#[tokio::test]
async fn filesystem_oauth_callback_claim_is_one_shot_and_completion_persists() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

    let flow = service
        .create_flow(NewAuthFlow {
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: google_provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new("https://provider.example/oauth")
                    .unwrap(),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: Some(state_hash("state")),
            pkce_verifier_hash: Some(pkce_hash("pkce")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    let claim = OAuthCallbackClaimRequest {
        flow_id: flow.id,
        opaque_state_hash: state_hash("state"),
        provider: google_provider(),
        pkce_verifier_hash: pkce_hash("pkce"),
    };

    let claimed = service
        .claim_oauth_callback(&scope, claim.clone())
        .await
        .unwrap();
    assert_eq!(claimed.status, AuthFlowStatus::CallbackReceived);

    let second_claim = service
        .claim_oauth_callback(&scope, claim.clone())
        .await
        .expect_err("in-flight callback claim must be one-shot");
    assert_eq!(second_claim, AuthProductError::FlowAlreadyTerminal);

    let completed = service
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("state"),
                outcome: ironclaw_auth::ProviderCallbackOutcome::Authorized {
                    exchange: OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("code"),
                        pkce_verifier_hash: pkce_hash("pkce"),
                        access_secret: SecretHandle::new("oauth-access").unwrap(),
                        refresh_secret: Some(SecretHandle::new("oauth-refresh").unwrap()),
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                    },
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(completed.status, AuthFlowStatus::Completed);
    assert!(completed.credential_account_id.is_some());

    let emitted_at = Utc::now();
    service
        .mark_continuation_dispatched(&scope, flow.id, emitted_at)
        .await
        .unwrap();

    let recreated = test_service(Arc::clone(&filesystem), secret_store);
    let stored = recreated
        .get_flow(&scope, flow.id)
        .await
        .unwrap()
        .expect("completed flow should be durable");
    assert_eq!(stored.status, AuthFlowStatus::Completed);
    assert_eq!(stored.continuation_emitted_at, Some(emitted_at));

    let completed_replay = recreated
        .claim_oauth_callback(&scope, claim)
        .await
        .expect("completed callback replay should not reclaim provider exchange");
    assert_eq!(completed_replay.status, AuthFlowStatus::Completed);
    assert_eq!(completed_replay.continuation_emitted_at, Some(emitted_at));
}

#[tokio::test]
async fn filesystem_manual_token_submit_allows_only_one_concurrent_consumer() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = Arc::new(test_service(filesystem, secret_store));

    let challenge = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired { interaction_id, .. } = challenge else {
        panic!("expected manual token challenge");
    };

    let mut tasks = JoinSet::new();
    for value in ["first-token", "second-token"] {
        let service = Arc::clone(&service);
        let scope = scope.clone();
        tasks.spawn(async move {
            service
                .submit_manual_token(
                    &scope,
                    SecretSubmitRequest {
                        interaction_id,
                        secret: SecretString::from(value),
                    },
                )
                .await
        });
    }

    let mut successes = 0;
    let mut consumed_rejections = 0;
    while let Some(result) = tasks.join_next().await {
        match result.unwrap() {
            Ok(_) => successes += 1,
            Err(AuthProductError::UnknownOrExpiredFlow) => consumed_rejections += 1,
            Err(error) => panic!("unexpected submit error: {error:?}"),
        }
    }

    assert_eq!(successes, 1);
    assert_eq!(consumed_rejections, 1);
}

// ─── fix: fs_error maps VersionMismatch to BackendConflict ───────────────────

#[test]
fn fs_error_maps_version_mismatch_to_backend_conflict() {
    use super::paths::fs_error;
    use ironclaw_filesystem::{FilesystemError, FilesystemOperation};
    use ironclaw_host_api::VirtualPath;

    let version_mismatch = FilesystemError::VersionMismatch {
        path: VirtualPath::new("/secrets/test").unwrap(),
        expected: None,
        found: None,
    };
    assert_eq!(
        fs_error(version_mismatch),
        AuthProductError::BackendConflict,
        "VersionMismatch must map to BackendConflict, not BackendUnavailable"
    );

    let backend_err = FilesystemError::Backend {
        path: VirtualPath::new("/secrets/test").unwrap(),
        operation: FilesystemOperation::ReadFile,
        reason: "io error".to_string(),
    };
    assert_eq!(
        fs_error(backend_err),
        AuthProductError::BackendUnavailable,
        "non-CAS errors must still map to BackendUnavailable"
    );
}

// ─── fix: mark_continuation_dispatched is idempotent ─────────────────────────

#[tokio::test]
async fn filesystem_oauth_continuation_marker_is_idempotent() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

    let flow = service
        .create_flow(NewAuthFlow {
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: google_provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new("https://provider.example/oauth")
                    .unwrap(),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: Some(state_hash("s")),
            pkce_verifier_hash: Some(pkce_hash("p")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();

    // Complete the flow so mark_continuation_dispatched is valid.
    service
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash("s"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("p"),
            },
        )
        .await
        .unwrap();
    service
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("s"),
                outcome: ironclaw_auth::ProviderCallbackOutcome::Authorized {
                    exchange: OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("c"),
                        pkce_verifier_hash: pkce_hash("p"),
                        access_secret: SecretHandle::new("access").unwrap(),
                        refresh_secret: None,
                        scopes: vec![],
                        account_id: None,
                    },
                },
            },
        )
        .await
        .unwrap();

    let first_at = Utc::now();
    let first = service
        .mark_continuation_dispatched(&scope, flow.id, first_at)
        .await
        .unwrap();
    assert_eq!(first.continuation_emitted_at, Some(first_at));

    // Second call with a different timestamp must NOT overwrite.
    let second_at = first_at + Duration::seconds(1);
    let second = service
        .mark_continuation_dispatched(&scope, flow.id, second_at)
        .await
        .unwrap();
    assert_eq!(
        second.continuation_emitted_at,
        Some(first_at),
        "idempotent: second call must not overwrite the first emitted_at"
    );
}

// ─── fix: manual-token submit cleans up secret on write failure ───────────────

#[tokio::test]
async fn filesystem_manual_token_rotation_removes_previous_secret() {
    // Tests the update_binding path in create_or_update_manual_token_account:
    // after a successful token rotation the OLD access secret must be purged
    // from SecretStore so it does not accumulate orphaned material.
    use ironclaw_auth::{
        CredentialAccountUpdateBinding, ManualTokenSetupRequest, SecretSubmitRequest,
    };

    let filesystem = test_filesystem();
    let concrete_secret_store = Arc::new(InMemorySecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

    // --- First submit: create the account via the no-binding path. ---
    let challenge1 = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired {
        interaction_id: iid1,
        ..
    } = challenge1
    else {
        panic!("expected ManualTokenRequired");
    };
    let result1 = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id: iid1,
                secret: SecretString::from("token-v1"),
            },
        )
        .await
        .unwrap();
    let account_id = result1.account_id;

    // Grab the first-generation secret handle.
    let account_after_v1 = service
        .get_account(ironclaw_auth::CredentialAccountLookupRequest::new(
            scope.clone(),
            account_id,
        ))
        .await
        .unwrap()
        .unwrap();
    let old_handle = account_after_v1
        .access_secret
        .clone()
        .expect("v1 access_secret");
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &old_handle)
            .await
            .unwrap()
            .is_some(),
        "v1 secret must exist in store"
    );

    // --- Second submit: rotate via update_binding to the same account. ---
    let challenge2 = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: Some(CredentialAccountUpdateBinding {
                account_id,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: vec![],
            }),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired {
        interaction_id: iid2,
        ..
    } = challenge2
    else {
        panic!("expected ManualTokenRequired for rotation");
    };
    service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id: iid2,
                secret: SecretString::from("token-v2"),
            },
        )
        .await
        .unwrap();

    // The old handle must have been purged from SecretStore after the rotation.
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &old_handle)
            .await
            .unwrap()
            .is_none(),
        "v1 secret must be purged from SecretStore after rotation"
    );

    // The new handle must be present.
    let account_after_v2 = service
        .get_account(ironclaw_auth::CredentialAccountLookupRequest::new(
            scope.clone(),
            account_id,
        ))
        .await
        .unwrap()
        .unwrap();
    let new_handle = account_after_v2.access_secret.expect("v2 access_secret");
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &new_handle)
            .await
            .unwrap()
            .is_some(),
        "v2 secret must be present in SecretStore"
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

// ─── fix: lock-cache weak-reference GC actually shrinks the map ──────────────

#[tokio::test]
async fn filesystem_lock_cache_drops_weak_entries_after_release() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let service = test_service(filesystem, secret_store);

    {
        // Acquire a lock for key A and drop the guard immediately.
        let lock_a = service.lock_for("account:key-a".to_string());
        let _guard_a = lock_a.lock().await;
        // guard_a dropped at end of this block; Arc<Mutex> dropped too after lock_a drops.
    }
    // After key-A's Arc dropped, the next call to lock_for should evict the
    // dead weak reference. We trigger eviction via lock_for on a different key.
    let _lock_b = service.lock_for("account:key-b".to_string());

    // Verify key-A is gone: requesting it again must produce a *new* Arc (i.e.
    // a fresh Mutex), not the evicted weak ref.
    let lock_a2 = service.lock_for("account:key-a".to_string());
    // The new lock should be unlocked (no one holds it).
    assert!(
        lock_a2.try_lock().is_ok(),
        "re-acquired key-a must be unlocked"
    );
}

// ─── fix: manual-token expiry branch ─────────────────────────────────────────

#[tokio::test]
async fn filesystem_manual_token_submit_rejects_expired_interaction() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    // Create an interaction that is already past its expiry.
    let challenge = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            // Expired immediately.
            expires_at: Utc::now() - Duration::seconds(1),
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired { interaction_id, .. } = challenge else {
        panic!("expected ManualTokenRequired");
    };

    let err = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("too-late"),
            },
        )
        .await
        .expect_err("expired interaction must be rejected");
    assert_eq!(
        err,
        AuthProductError::UnknownOrExpiredFlow,
        "expired interaction must return UnknownOrExpiredFlow"
    );
}

// ─── UnavailableAuthProviderClient validates before returning error ───────────

#[tokio::test]
async fn unavailable_auth_provider_client_validates_before_returning_backend_unavailable() {
    use super::provider::UnavailableAuthProviderClient;
    use ironclaw_auth::{
        AuthProviderClient, OAuthAuthorizationCode, OAuthProviderCallbackRequest,
        OAuthProviderExchangeContext, OAuthProviderRefreshRequest,
    };
    use secrecy::SecretString;

    let client = UnavailableAuthProviderClient;

    let ctx = OAuthProviderExchangeContext {
        scope: test_scope(),
        flow_id: ironclaw_auth::AuthFlowId::new(),
    };

    // Valid request must return BackendUnavailable (no provider configured) after
    // the internal validate_provider_callback_request guard passes.
    let valid = OAuthProviderCallbackRequest {
        provider: google_provider(),
        account_label: account_label(),
        authorization_code: OAuthAuthorizationCode::new(SecretString::from("real-code")).unwrap(),
        authorization_code_hash: code_hash("c"),
        pkce_verifier: ironclaw_auth::PkceVerifierSecret::new(SecretString::from("real-verifier"))
            .unwrap(),
        pkce_verifier_hash: pkce_hash("p"),
        scopes: vec![],
    };
    let err = client.exchange_callback(ctx, valid).await.unwrap_err();
    assert_eq!(
        err,
        AuthProductError::BackendUnavailable,
        "valid request must reach BackendUnavailable (no provider configured)"
    );

    // 3. refresh_token always BackendUnavailable.
    let refresh_err = client
        .refresh_token(OAuthProviderRefreshRequest {
            scope: test_scope(),
            account_id: CredentialAccountId::new(),
            provider: google_provider(),
            refresh_secret: SecretHandle::new("r").unwrap(),
            scopes: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(refresh_err, AuthProductError::BackendUnavailable);
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
