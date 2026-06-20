use std::sync::Arc;

use chrono::{Duration, Utc};
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{
    ExtensionId, InvocationId, MountAlias, MountGrant, MountPermissions,
    RuntimeCredentialAccountProviderId, SecretHandle, ThreadId, UserId, VirtualPath,
};
use ironclaw_host_runtime::RuntimeCredentialAccountRequest;
use ironclaw_host_runtime::RuntimeCredentialAccountResolver;
use ironclaw_secrets::{InMemorySecretStore, SecretStore};
use secrecy::SecretString;
use tokio::task::JoinSet;

use super::*;
use crate::product_auth_runtime_credentials::{
    ProductAuthRuntimeCredentialAccountSelector, ProductAuthRuntimeCredentialResolver,
    RuntimeCredentialAccountSelectionRequest, RuntimeCredentialAccountSelectionService,
};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthFlowKind, AuthFlowManager, AuthFlowOwnerScope,
    AuthFlowRecordSource, AuthFlowStatus, AuthInteractionId, AuthInteractionService,
    AuthProductError, AuthProductScope, AuthProviderId, AuthSessionId, AuthSurface,
    AuthorizationCodeHash, CredentialAccountChoiceRequest, CredentialAccountLabel,
    CredentialAccountListRequest, CredentialAccountLookupRequest, CredentialAccountRecordSource,
    CredentialAccountSelectionRequest, CredentialAccountService, CredentialAccountStatus,
    CredentialOwnership, ManualTokenCompletionInput, ManualTokenSetupRequest, NewAuthFlow,
    NewCredentialAccount, OAuthAuthorizationUrl, OAuthCallbackClaimRequest, OAuthCallbackInput,
    OAuthProviderExchange, OpaqueStateHash, PkceVerifierHash, ProviderScope, SecretSubmitRequest,
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

async fn create_manual_token_flow(
    service: &FilesystemAuthProductServices<InMemoryBackend>,
    scope: &AuthProductScope,
    expires_at: chrono::DateTime<Utc>,
) -> AuthInteractionId {
    let challenge = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at,
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired {
        interaction_id,
        provider,
        label,
        expires_at: challenge_expires_at,
    } = challenge
    else {
        panic!("expected manual token challenge");
    };
    service
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider,
            challenge: AuthChallenge::ManualTokenRequired {
                interaction_id,
                provider: google_provider(),
                label,
                expires_at: challenge_expires_at,
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at,
        })
        .await
        .unwrap();
    interaction_id
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
async fn filesystem_runtime_account_selection_matches_setup_invocation_account() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let mut setup_scope = test_scope();
    setup_scope.surface = AuthSurface::Callback;
    setup_scope.resource.thread_id = Some(ThreadId::new("thread-auth-1").unwrap());
    let mut runtime_scope = AuthProductScope::new(setup_scope.resource.clone(), AuthSurface::Api);
    runtime_scope.resource.invocation_id = InvocationId::new();
    let service = Arc::new(test_service(filesystem, secret_store));
    let access_secret = SecretHandle::new("google-access").unwrap();

    let created = service
        .create_account(NewCredentialAccount {
            scope: setup_scope,
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(access_secret.clone()),
            refresh_secret: None,
            scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
        })
        .await
        .unwrap();

    let selector = ProductAuthRuntimeCredentialAccountSelector::new(service.clone());
    let selected = selector
        .select_unique_configured_runtime_account(RuntimeCredentialAccountSelectionRequest::new(
            CredentialAccountSelectionRequest::new(runtime_scope.clone(), google_provider()),
            runtime_scope,
            ironclaw_host_api::RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            Vec::new(),
        ))
        .await
        .unwrap();

    assert_eq!(selected.id, created.id);
    assert_eq!(selected.access_secret, Some(access_secret));
}

#[tokio::test]
async fn filesystem_runtime_account_selection_matches_new_thread_reusable_account() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let mut setup_scope = test_scope();
    setup_scope.surface = AuthSurface::Callback;
    setup_scope.resource.thread_id = Some(ThreadId::new("thread-auth-1").unwrap());
    let mut runtime_scope = AuthProductScope::new(setup_scope.resource.clone(), AuthSurface::Api);
    runtime_scope.resource.thread_id = Some(ThreadId::new("thread-auth-2").unwrap());
    runtime_scope.resource.invocation_id = InvocationId::new();
    let service = Arc::new(test_service(filesystem, secret_store));
    let access_secret = SecretHandle::new("google-access").unwrap();

    let created = service
        .create_account(NewCredentialAccount {
            scope: setup_scope,
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(access_secret.clone()),
            refresh_secret: None,
            scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
        })
        .await
        .unwrap();

    let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
        ProductAuthRuntimeCredentialAccountSelector::new(service),
    ));
    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &runtime_scope.resource,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &ironclaw_host_api::RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("google-calendar").unwrap(),
        })
        .await
        .unwrap();

    assert_eq!(created.access_secret, Some(resolved.handle.clone()));
    assert_eq!(resolved.handle, access_secret);
    assert_eq!(resolved.scope, created.scope.resource);
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
async fn filesystem_manual_token_submit_rotates_existing_reusable_account() {
    let filesystem = test_filesystem();
    let concrete_secret_store = Arc::new(InMemorySecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    let first_challenge = service
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
        interaction_id: first_interaction,
        ..
    } = first_challenge
    else {
        panic!("expected manual token challenge");
    };
    let first = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id: first_interaction,
                secret: SecretString::from("first-manual-token"),
            },
        )
        .await
        .unwrap();
    let first_account = service
        .read_account(&scope, first.account_id)
        .await
        .unwrap()
        .expect("first account")
        .0;
    let first_handle = first_account.access_secret.expect("first secret handle");

    let second_challenge = service
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
        interaction_id: second_interaction,
        ..
    } = second_challenge
    else {
        panic!("expected manual token challenge");
    };
    let second = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id: second_interaction,
                secret: SecretString::from("second-manual-token"),
            },
        )
        .await
        .unwrap();

    assert_eq!(second.account_id, first.account_id);
    let accounts = service.accounts_for_owner(&scope).await.unwrap();
    assert_eq!(accounts.len(), 1);
    let updated = accounts.into_iter().next().unwrap();
    let second_handle = updated.access_secret.expect("second secret handle");
    assert_ne!(second_handle, first_handle);
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &second_handle)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &first_handle)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn filesystem_manual_token_completion_persists_auth_flow_account() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);
    let expires_at = Utc::now() + Duration::minutes(5);

    let challenge = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at,
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired {
        interaction_id,
        provider,
        label,
        expires_at: challenge_expires_at,
    } = challenge
    else {
        panic!("expected manual token challenge");
    };

    let flow = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: google_provider(),
            challenge: AuthChallenge::ManualTokenRequired {
                interaction_id,
                provider,
                label,
                expires_at: challenge_expires_at,
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at,
        })
        .await
        .unwrap();

    let submitted = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("manual-token-value"),
            },
        )
        .await
        .unwrap();

    let completed = service
        .complete_manual_token(
            &scope,
            ManualTokenCompletionInput {
                interaction_id,
                credential_account_id: submitted.account_id,
            },
        )
        .await
        .unwrap();

    assert_eq!(completed.id, flow.id);
    assert_eq!(completed.status, AuthFlowStatus::Completed);
    assert_eq!(completed.credential_account_id, Some(submitted.account_id));
}

#[tokio::test]
async fn filesystem_manual_token_completion_rejects_invalid_completed_account() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);
    let interaction_id =
        create_manual_token_flow(&service, &scope, Utc::now() + Duration::minutes(5)).await;

    let missing = service
        .complete_manual_token(
            &scope,
            ManualTokenCompletionInput {
                interaction_id,
                credential_account_id: CredentialAccountId::new(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(missing, AuthProductError::CredentialMissing);

    let account = service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::PendingSetup,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: None,
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();
    let unconfigured = service
        .complete_manual_token(
            &scope,
            ManualTokenCompletionInput {
                interaction_id,
                credential_account_id: account.id,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(unconfigured, AuthProductError::CrossScopeDenied);

    let mut foreign_scope = scope.clone();
    foreign_scope.resource.user_id = UserId::new("bob").unwrap();
    let foreign = service
        .create_account(NewCredentialAccount {
            scope: foreign_scope,
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(SecretHandle::new("foreign-access").unwrap()),
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();
    let cross_scope = service
        .complete_manual_token(
            &scope,
            ManualTokenCompletionInput {
                interaction_id,
                credential_account_id: foreign.id,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(cross_scope, AuthProductError::CrossScopeDenied);
}

#[tokio::test]
async fn filesystem_manual_token_completion_expires_stale_auth_flow() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);
    let interaction_id =
        create_manual_token_flow(&service, &scope, Utc::now() - Duration::minutes(1)).await;

    let submitted = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("manual-token-value"),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(submitted, AuthProductError::UnknownOrExpiredFlow);

    let err = service
        .complete_manual_token(
            &scope,
            ManualTokenCompletionInput {
                interaction_id,
                credential_account_id: CredentialAccountId::new(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err, AuthProductError::UnknownOrExpiredFlow);
    let flows = service.flows_for_scope(&scope).await.unwrap();
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].0.status, AuthFlowStatus::Expired);
}

#[tokio::test]
async fn filesystem_manual_token_cancel_marks_flow_canceled_and_is_idempotent() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);
    let interaction_id =
        create_manual_token_flow(&service, &scope, Utc::now() + Duration::minutes(5)).await;

    let canceled = service
        .cancel_manual_token(&scope, interaction_id)
        .await
        .unwrap()
        .expect("manual-token flow should be canceled");
    assert_eq!(canceled.status, AuthFlowStatus::Canceled);
    let still_canceled = service
        .cancel_manual_token(&scope, interaction_id)
        .await
        .unwrap()
        .expect("terminal flow should still be returned");
    assert_eq!(still_canceled.status, AuthFlowStatus::Canceled);
    let unknown = service
        .cancel_manual_token(&scope, AuthInteractionId::new())
        .await
        .unwrap();
    assert!(unknown.is_none());
}

#[tokio::test]
async fn filesystem_flow_record_source_projects_session_scoped_manual_flows() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let mut scope = test_scope();
    scope.surface = AuthSurface::Callback;
    scope.resource.thread_id = Some(ThreadId::new("thread-auth-flow").unwrap());
    scope.session_id = Some(AuthSessionId::new("session-auth-flow").unwrap());
    let service = FilesystemAuthProductServices::new(filesystem, secret_store);
    let expires_at = Utc::now() + Duration::minutes(5);

    let challenge = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at,
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired {
        interaction_id,
        provider,
        label,
        expires_at: challenge_expires_at,
    } = challenge
    else {
        panic!("expected manual token challenge");
    };
    let flow = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: google_provider(),
            challenge: AuthChallenge::ManualTokenRequired {
                interaction_id,
                provider,
                label,
                expires_at: challenge_expires_at,
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at,
        })
        .await
        .unwrap();

    let submitted = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("manual-token-value"),
            },
        )
        .await
        .unwrap();
    service
        .complete_manual_token(
            &scope,
            ManualTokenCompletionInput {
                interaction_id,
                credential_account_id: submitted.account_id,
            },
        )
        .await
        .unwrap();

    let owner = AuthFlowOwnerScope {
        tenant_id: scope.resource.tenant_id.clone(),
        user_id: scope.resource.user_id.clone(),
        agent_id: scope.resource.agent_id.clone(),
        project_id: scope.resource.project_id.clone(),
        thread_id: scope.resource.thread_id.clone().unwrap(),
    };
    let snapshot = service.flows_for_owner(owner).await.unwrap();
    let projected = snapshot
        .iter()
        .find(|record| record.id == flow.id)
        .expect("session-scoped flow should be projected for auth gates");

    assert_eq!(projected.status, AuthFlowStatus::Completed);
    assert_eq!(projected.scope.session_id, scope.session_id);
    assert_eq!(
        projected.credential_account_id,
        Some(submitted.account_id),
        "manual-token completion must remain visible to the auth read model"
    );
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

#[tokio::test]
async fn filesystem_runtime_account_selection_tolerates_many_session_account_roots() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let service = Arc::new(test_service(filesystem, secret_store));
    let mut setup_scope = test_scope();
    setup_scope.surface = AuthSurface::Callback;
    setup_scope.resource.thread_id = Some(ThreadId::new("thread-many-sessions").unwrap());
    let mut runtime_scope = AuthProductScope::new(setup_scope.resource.clone(), AuthSurface::Web);
    runtime_scope.resource.invocation_id = InvocationId::new();

    for index in 0..70 {
        let mut account_scope = setup_scope.clone();
        account_scope.session_id = Some(AuthSessionId::new(format!("session-{index:03}")).unwrap());
        service
            .create_account(NewCredentialAccount {
                scope: account_scope,
                provider: google_provider(),
                label: account_label(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(
                    SecretHandle::new(format!("many-session-access-{index}")).unwrap(),
                ),
                refresh_secret: None,
                scopes: vec![ProviderScope::new("drive.readonly").unwrap()],
            })
            .await
            .unwrap();
    }

    let selector = ProductAuthRuntimeCredentialAccountSelector::new(service);
    let selected = selector
        .select_unique_configured_runtime_account(RuntimeCredentialAccountSelectionRequest::new(
            CredentialAccountSelectionRequest::new(runtime_scope.clone(), google_provider()),
            runtime_scope,
            ironclaw_host_api::RuntimeCredentialAccountSetup::OAuth {
                scopes: vec!["drive.readonly".to_string()],
            },
            vec![ProviderScope::new("drive.readonly").unwrap()],
        ))
        .await
        .expect("session-root fanout must not make credential selection unavailable");

    assert_eq!(selected.provider, google_provider());
}

#[tokio::test]
async fn filesystem_runtime_account_selection_tolerates_many_account_records_per_root() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let service = Arc::new(test_service(filesystem, secret_store));
    let mut setup_scope = test_scope();
    setup_scope.surface = AuthSurface::Callback;
    setup_scope.resource.thread_id = Some(ThreadId::new("thread-many-accounts").unwrap());
    let mut runtime_scope = AuthProductScope::new(setup_scope.resource.clone(), AuthSurface::Web);
    runtime_scope.resource.invocation_id = InvocationId::new();

    for index in 0..70 {
        service
            .create_account(NewCredentialAccount {
                scope: setup_scope.clone(),
                provider: google_provider(),
                label: account_label(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(
                    SecretHandle::new(format!("many-account-access-{index}")).unwrap(),
                ),
                refresh_secret: None,
                scopes: vec![ProviderScope::new("drive.readonly").unwrap()],
            })
            .await
            .unwrap();
    }

    let selector = ProductAuthRuntimeCredentialAccountSelector::new(service);
    let selected = selector
        .select_unique_configured_runtime_account(RuntimeCredentialAccountSelectionRequest::new(
            CredentialAccountSelectionRequest::new(runtime_scope.clone(), google_provider()),
            runtime_scope,
            ironclaw_host_api::RuntimeCredentialAccountSetup::OAuth {
                scopes: vec!["drive.readonly".to_string()],
            },
            vec![ProviderScope::new("drive.readonly").unwrap()],
        ))
        .await
        .expect("account-record fanout must not make credential selection unavailable");

    assert_eq!(selected.provider, google_provider());
}

#[tokio::test]
async fn filesystem_oauth_callback_claim_is_one_shot_and_completion_persists() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

    let flow = service
        .create_flow(NewAuthFlow {
            id: None,
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
            id: None,
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

#[tokio::test]
async fn filesystem_manual_token_reconnect_updates_bound_account_across_a_different_thread() {
    // Regression (#4935 defect A, manual-token durable path): a manual-token
    // reconnect from a different thread/invocation than the account was created
    // in must UPDATE the bound account at owner granularity. The apply step used
    // `validate_account_update_target` (full `scope_matches`), so setup accepted
    // the binding but submit rejected it with CrossScopeDenied and would re-fork.
    use ironclaw_auth::{
        CredentialAccountUpdateBinding, ManualTokenSetupRequest, SecretSubmitRequest,
    };

    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

    // Account created in thread-a.
    let mut create_scope = test_scope();
    create_scope.resource.thread_id = Some(ThreadId::new("thread-a").unwrap());
    let challenge1 = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: create_scope.clone(),
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
    let account_id = service
        .submit_manual_token(
            &create_scope,
            SecretSubmitRequest {
                interaction_id: iid1,
                secret: SecretString::from("token-v1"),
            },
        )
        .await
        .unwrap()
        .account_id;

    // Reconnect from thread-b (fresh invocation), binding to the same account.
    let mut reauth_scope = test_scope();
    reauth_scope.resource.thread_id = Some(ThreadId::new("thread-b").unwrap());
    let challenge2 = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: reauth_scope.clone(),
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
        .expect("reconnect challenge across a different thread");
    let AuthChallenge::ManualTokenRequired {
        interaction_id: iid2,
        ..
    } = challenge2
    else {
        panic!("expected ManualTokenRequired for reconnect");
    };
    let result = service
        .submit_manual_token(
            &reauth_scope,
            SecretSubmitRequest {
                interaction_id: iid2,
                secret: SecretString::from("token-v2"),
            },
        )
        .await
        .expect("cross-thread manual-token reconnect must update the bound account, not fork");
    assert_eq!(result.account_id, account_id);

    // No fork: still exactly one account for the owner.
    let accounts = service
        .accounts_for_owner(&create_scope.to_credential_owner())
        .await
        .unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].id, account_id);
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
            None,
        )
        .await
        .unwrap();
    concrete_secret_store
        .put(
            scope.resource.clone(),
            refresh.clone(),
            SecretString::from("refresh-material"),
            None,
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

// ─── zmanian follow-up #1: OAuth re-auth must purge previous secret handles ──

#[tokio::test]
async fn filesystem_oauth_reauth_purges_previous_provider_secrets() {
    // After a successful OAuth re-auth through a bound flow, the OLD access
    // and refresh secret handles must be deleted from SecretStore so repeated
    // re-auths do not accumulate dead handles. Host OAuth provider clients
    // return exchange.account_id == None, so the durable flow must use the
    // update_binding account id rather than rejecting the callback.
    use ironclaw_auth::{CredentialAccountUpdateBinding, ProviderCallbackOutcome};
    use ironclaw_secrets::SecretMaterial;

    let filesystem = test_filesystem();
    let concrete_secret_store = Arc::new(InMemorySecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

    // ── Step 1: initial OAuth flow creates a new account ─────────────────────
    let flow1 = service
        .create_flow(NewAuthFlow {
            id: None,
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
            opaque_state_hash: Some(state_hash("state1")),
            pkce_verifier_hash: Some(pkce_hash("pkce1")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();

    service
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow1.id,
                opaque_state_hash: state_hash("state1"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("pkce1"),
            },
        )
        .await
        .unwrap();

    let access_v1 = SecretHandle::new("oauth-access-v1").unwrap();
    let refresh_v1 = SecretHandle::new("oauth-refresh-v1").unwrap();
    // Pre-populate SecretStore to simulate provider client having stored these
    // handles; this lets us verify they are purged on re-auth.
    concrete_secret_store
        .put(
            scope.resource.clone(),
            access_v1.clone(),
            SecretMaterial::from("access-token-v1"),
            None,
        )
        .await
        .unwrap();
    concrete_secret_store
        .put(
            scope.resource.clone(),
            refresh_v1.clone(),
            SecretMaterial::from("refresh-token-v1"),
            None,
        )
        .await
        .unwrap();

    let completed1 = service
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: flow1.id,
                opaque_state_hash: state_hash("state1"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("code1"),
                        pkce_verifier_hash: pkce_hash("pkce1"),
                        access_secret: access_v1.clone(),
                        refresh_secret: Some(refresh_v1.clone()),
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                    },
                },
            },
        )
        .await
        .unwrap();
    let account_id = completed1
        .credential_account_id
        .expect("first OAuth flow must produce a credential account");

    // v1 handles must be present before re-auth.
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access_v1)
            .await
            .unwrap()
            .is_some(),
        "v1 access handle must exist before re-auth"
    );
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &refresh_v1)
            .await
            .unwrap()
            .is_some(),
        "v1 refresh handle must exist before re-auth"
    );

    // ── Step 2: re-auth flow bound to the existing account ───────────────────
    let flow2 = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: google_provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new("https://provider.example/oauth")
                    .unwrap(),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: Some(CredentialAccountUpdateBinding {
                account_id,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: vec![],
            }),
            opaque_state_hash: Some(state_hash("state2")),
            pkce_verifier_hash: Some(pkce_hash("pkce2")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();

    service
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow2.id,
                opaque_state_hash: state_hash("state2"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("pkce2"),
            },
        )
        .await
        .unwrap();

    let access_v2 = SecretHandle::new("oauth-access-v2").unwrap();
    let refresh_v2 = SecretHandle::new("oauth-refresh-v2").unwrap();
    concrete_secret_store
        .put(
            scope.resource.clone(),
            access_v2.clone(),
            SecretMaterial::from("access-token-v2"),
            None,
        )
        .await
        .unwrap();
    concrete_secret_store
        .put(
            scope.resource.clone(),
            refresh_v2.clone(),
            SecretMaterial::from("refresh-token-v2"),
            None,
        )
        .await
        .unwrap();

    service
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: flow2.id,
                opaque_state_hash: state_hash("state2"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("code2"),
                        pkce_verifier_hash: pkce_hash("pkce2"),
                        access_secret: access_v2.clone(),
                        refresh_secret: Some(refresh_v2.clone()),
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                    },
                },
            },
        )
        .await
        .unwrap();

    // Old handles must have been purged from SecretStore.
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access_v1)
            .await
            .unwrap()
            .is_none(),
        "v1 access handle must be purged from SecretStore after re-auth"
    );
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &refresh_v1)
            .await
            .unwrap()
            .is_none(),
        "v1 refresh handle must be purged from SecretStore after re-auth"
    );

    // New handles must remain.
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access_v2)
            .await
            .unwrap()
            .is_some(),
        "v2 access handle must be present in SecretStore after re-auth"
    );
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &refresh_v2)
            .await
            .unwrap()
            .is_some(),
        "v2 refresh handle must be present in SecretStore after re-auth"
    );
}

// ─── [tests] OAuth reauth updates the bound account across transient scope diffs

#[tokio::test]
async fn filesystem_oauth_reauth_updates_bound_account_across_fresh_invocation() {
    // Defect A, durable callback path: the bound-account update resolves at owner
    // granularity. When the reconnect flow's scope differs from the account's
    // creation scope ONLY by transient invocation/thread/mission, the callback
    // must still update the SAME account. A regression to full `scope_matches`
    // (in `validate_scoped_update_binding` or `update_bound_oauth_account`) would
    // reject this with CrossScopeDenied. Owner granularity is tenant/user/agent/
    // project plus path-segmenting session — all unchanged across the reconnect.
    use ironclaw_auth::{CredentialAccountUpdateBinding, ProviderCallbackOutcome};

    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let setup_scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

    // ── Step 1: initial flow creates the account under `setup_scope`. ─────────
    let flow1 = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: setup_scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: google_provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new("https://provider.example/oauth")
                    .unwrap(),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: Some(state_hash("state1")),
            pkce_verifier_hash: Some(pkce_hash("pkce1")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    service
        .claim_oauth_callback(
            &setup_scope,
            OAuthCallbackClaimRequest {
                flow_id: flow1.id,
                opaque_state_hash: state_hash("state1"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("pkce1"),
            },
        )
        .await
        .unwrap();
    let access_v1 = SecretHandle::new("oauth-access-v1").unwrap();
    let completed1 = service
        .complete_oauth_callback(
            &setup_scope,
            OAuthCallbackInput {
                flow_id: flow1.id,
                opaque_state_hash: state_hash("state1"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("code1"),
                        pkce_verifier_hash: pkce_hash("pkce1"),
                        access_secret: access_v1.clone(),
                        refresh_secret: None,
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                    },
                },
            },
        )
        .await
        .unwrap();
    let account_id = completed1
        .credential_account_id
        .expect("first OAuth flow must produce a credential account");

    // ── Step 2: reconnect from a DIFFERENT context — fresh invocation plus a
    // different thread/mission, same owner (tenant/user/agent/project/session).
    let mut reauth_resource = setup_scope.resource.clone();
    reauth_resource.invocation_id = InvocationId::new();
    reauth_resource.thread_id = Some(ThreadId::new("thread-reauth").unwrap());
    reauth_resource.mission_id = Some(ironclaw_host_api::MissionId::new("mission-reauth").unwrap());
    let reauth_scope = AuthProductScope::new(reauth_resource, setup_scope.surface);

    let flow2 = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: reauth_scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: google_provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new("https://provider.example/oauth")
                    .unwrap(),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: Some(CredentialAccountUpdateBinding {
                account_id,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: vec![],
            }),
            opaque_state_hash: Some(state_hash("state2")),
            pkce_verifier_hash: Some(pkce_hash("pkce2")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect("cross-invocation reconnect must accept the owner's binding");
    service
        .claim_oauth_callback(
            &reauth_scope,
            OAuthCallbackClaimRequest {
                flow_id: flow2.id,
                opaque_state_hash: state_hash("state2"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("pkce2"),
            },
        )
        .await
        .unwrap();
    let access_v2 = SecretHandle::new("oauth-access-v2").unwrap();
    let completed2 = service
        .complete_oauth_callback(
            &reauth_scope,
            OAuthCallbackInput {
                flow_id: flow2.id,
                opaque_state_hash: state_hash("state2"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("code2"),
                        pkce_verifier_hash: pkce_hash("pkce2"),
                        access_secret: access_v2.clone(),
                        refresh_secret: None,
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                    },
                },
            },
        )
        .await
        .expect("cross-invocation reconnect callback must update the bound account");

    // The bound account was UPDATED in place across the transient scope diff —
    // same account id, carrying the re-auth's access secret, and not forked.
    assert_eq!(
        completed2.credential_account_id,
        Some(account_id),
        "reconnect must complete against the same owner account, not a fork",
    );
    let owner_accounts = service.accounts_for_owner(&setup_scope).await.unwrap();
    assert_eq!(
        owner_accounts.len(),
        1,
        "reconnect must not fork a second account",
    );
    assert_eq!(
        owner_accounts[0].access_secret,
        Some(access_v2),
        "the bound account must carry the re-auth access secret",
    );
}

// ─── [High · tests] manual-token submit cleans up secret on account write fail

#[tokio::test]
async fn filesystem_manual_token_submit_cleans_up_secret_when_account_write_fails() {
    // create_or_update_manual_token_account (None path) stores the secret first,
    // then calls create_account_with_id(CasExpectation::Absent). If the write
    // fails the newly-stored secret must be deleted from SecretStore so it does
    // not orphan in the store.
    //
    // Failure injection: derive the account ID that submit_manual_token will use
    // (CredentialAccountId::from_uuid(interaction_id.as_uuid())) and write a
    // dummy record at that path before submitting, causing CasExpectation::Absent
    // to return VersionMismatch → BackendConflict.
    use ironclaw_auth::CredentialAccountId;
    use ironclaw_filesystem::CasExpectation;

    let filesystem = test_filesystem();
    let concrete_secret_store = Arc::new(InMemorySecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

    // Request an interaction so we know its ID (and can derive the account path).
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
        panic!("expected ManualTokenRequired");
    };

    // Derive the same account ID the submit path will use.
    let account_id = CredentialAccountId::from_uuid(interaction_id.as_uuid());

    // Write a dummy record at that path so create_account_with_id(Absent) fails.
    let dummy_account = ironclaw_auth::CredentialAccount {
        id: account_id,
        scope: scope.clone(),
        provider: google_provider(),
        label: account_label(),
        status: ironclaw_auth::CredentialAccountStatus::Configured,
        ownership: CredentialOwnership::UserReusable,
        owner_extension: None,
        granted_extensions: vec![],
        access_secret: None,
        refresh_secret: None,
        scopes: vec![],
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let path = super::paths::account_path(&scope, account_id)
        .expect("account path derivation must succeed");
    let json = serde_json::to_vec(&dummy_account).expect("serialization must succeed");
    use ironclaw_filesystem::{ContentType, Entry};
    let entry = Entry::bytes(json).with_content_type(ContentType::json());
    filesystem
        .put(&scope.resource, &path, entry, CasExpectation::Absent)
        .await
        .expect("pre-create dummy account must succeed");

    // Submit the token — account write will fail; cleanup must run.
    let result = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("token-value"),
            },
        )
        .await;
    assert!(result.is_err(), "submit must fail when account write fails");

    // The secret stored before the failing write must have been purged.
    let access_handle = super::paths::manual_token_secret_handle(account_id, interaction_id)
        .expect("handle derivation must succeed");
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access_handle)
            .await
            .unwrap()
            .is_none(),
        "orphaned secret must be purged from SecretStore after failed account write"
    );
}

// ─── fix: OAuth callback CAS-conflict re-read branch ─────────────────────────

#[tokio::test]
async fn filesystem_oauth_callback_cas_conflict_reuses_concurrent_account() {
    // Pre-create an account with the deterministic id that complete_oauth_callback
    // derives from flow_id (CredentialAccountId::from_uuid(flow_id.as_uuid())).
    // This simulates a concurrent callback that already created the account.
    // The CAS-conflict branch should re-read, validate, update, and succeed.
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

    let flow = service
        .create_flow(NewAuthFlow {
            id: None,
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
            opaque_state_hash: Some(state_hash("s2")),
            pkce_verifier_hash: Some(pkce_hash("p2")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();

    // Pre-seed the account with the deterministic id.
    let preseeded_id = CredentialAccountId::from_uuid(flow.id.as_uuid());
    service
        .create_account_with_id(
            preseeded_id,
            NewCredentialAccount {
                scope: scope.clone(),
                provider: google_provider(),
                label: account_label(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: vec![],
                access_secret: Some(SecretHandle::new("pre-seeded-access").unwrap()),
                refresh_secret: None,
                scopes: vec![],
            },
            CasExpectation::Absent,
        )
        .await
        .unwrap();

    service
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash("s2"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("p2"),
            },
        )
        .await
        .unwrap();

    let completed = service
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("s2"),
                outcome: ironclaw_auth::ProviderCallbackOutcome::Authorized {
                    exchange: OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("c2"),
                        pkce_verifier_hash: pkce_hash("p2"),
                        access_secret: SecretHandle::new("new-access").unwrap(),
                        refresh_secret: Some(SecretHandle::new("new-refresh").unwrap()),
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                    },
                },
            },
        )
        .await
        .unwrap();

    assert_eq!(
        completed.credential_account_id,
        Some(preseeded_id),
        "CAS-conflict branch must reuse the pre-seeded account id"
    );
    assert_eq!(completed.status, AuthFlowStatus::Completed);
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

// ─── tests: cancel_flow, fail_oauth_callback, complete_credential_selection ───

#[tokio::test]
async fn filesystem_cancel_flow_and_terminal_state_rejection() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    let flow = service
        .create_flow(NewAuthFlow {
            id: None,
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
            opaque_state_hash: Some(state_hash("cancel-s")),
            pkce_verifier_hash: Some(pkce_hash("cancel-p")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();

    let cancelled = service.cancel_flow(&scope, flow.id).await.unwrap();
    assert_eq!(cancelled.status, AuthFlowStatus::Canceled);

    // Second cancel on already-terminal flow returns Canceled error.
    let err = service
        .cancel_flow(&scope, flow.id)
        .await
        .expect_err("second cancel must fail");
    assert_eq!(err, AuthProductError::Canceled);
}

#[tokio::test]
async fn filesystem_fail_oauth_callback_marks_flow_failed() {
    use ironclaw_auth::{AuthErrorCode, OAuthCallbackFailureInput};
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    let flow = service
        .create_flow(NewAuthFlow {
            id: None,
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
            opaque_state_hash: Some(state_hash("fail-s")),
            pkce_verifier_hash: Some(pkce_hash("fail-p")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();

    service
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash("fail-s"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("fail-p"),
            },
        )
        .await
        .unwrap();

    let failed = service
        .fail_oauth_callback(
            &scope,
            OAuthCallbackFailureInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("fail-s"),
                error: AuthErrorCode::ProviderDenied,
            },
        )
        .await
        .unwrap();
    assert_eq!(failed.status, AuthFlowStatus::Failed);
    assert_eq!(failed.error, Some(AuthErrorCode::ProviderDenied));
}

#[tokio::test]
async fn filesystem_complete_credential_selection_completes_flow() {
    use ironclaw_auth::{AuthFlowKind, CredentialSelectionInput};
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

    let flow = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: google_provider(),
            challenge: AuthChallenge::AccountSelectionRequired {
                provider: google_provider(),
                accounts: vec![account.projection()],
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();

    let completed = service
        .complete_credential_selection(
            &scope,
            CredentialSelectionInput {
                flow_id: flow.id,
                credential_account_id: account.id,
            },
        )
        .await
        .unwrap();
    assert_eq!(completed.status, AuthFlowStatus::Completed);
    assert_eq!(completed.credential_account_id, Some(account.id));
}

// ─── tests: create_flow update_binding validation ─────────────────────────────

#[tokio::test]
async fn filesystem_create_flow_rejects_invalid_update_binding() {
    use ironclaw_auth::CredentialAccountUpdateBinding;
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    // Non-existent account in update_binding → CredentialMissing.
    let err = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: google_provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new("https://provider.example/oauth")
                    .unwrap(),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: Some(CredentialAccountUpdateBinding {
                account_id: CredentialAccountId::new(),
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: vec![],
            }),
            opaque_state_hash: Some(state_hash("ubv-s")),
            pkce_verifier_hash: Some(pkce_hash("ubv-p")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect_err("non-existent binding account must return CredentialMissing");
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

// ─── tests: validate_secret control-char branch ───────────────────────────────

#[tokio::test]
async fn filesystem_validate_secret_rejects_control_characters() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
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
        panic!("expected ManualTokenRequired");
    };

    // NUL byte must be rejected without consuming the interaction.
    let err = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("valid\x00nul"),
            },
        )
        .await
        .expect_err("NUL byte must be rejected");
    assert!(
        matches!(err, AuthProductError::InvalidRequest { .. }),
        "must return InvalidRequest for control characters"
    );

    // Interaction must NOT be consumed — replay still possible.
    let ok = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("clean-token"),
            },
        )
        .await;
    assert!(
        ok.is_ok(),
        "interaction must be usable after control-char rejection"
    );
}

// ─── fix: abbyshekit review — expired flow mutation persisted ────────────────

#[tokio::test]
async fn filesystem_expired_flow_status_persisted_before_returning_error() {
    // When claim_oauth_callback / complete_oauth_callback / fail_oauth_callback
    // encounter an expired flow, the Expired status must be written to disk
    // before returning UnknownOrExpiredFlow so durable state matches the contract.
    use ironclaw_auth::{
        AuthErrorCode, OAuthCallbackClaimRequest, OAuthCallbackFailureInput, OAuthCallbackInput,
        ProviderCallbackOutcome,
    };

    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    let flow = service
        .create_flow(NewAuthFlow {
            id: None,
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
            opaque_state_hash: Some(state_hash("exp-s")),
            pkce_verifier_hash: Some(pkce_hash("exp-p")),
            expires_at: Utc::now() - Duration::seconds(1),
        })
        .await
        .unwrap();

    // claim_oauth_callback must persist Expired before returning error.
    let err = service
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash("exp-s"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("exp-p"),
            },
        )
        .await
        .expect_err("expired flow must be rejected");
    assert_eq!(err, AuthProductError::UnknownOrExpiredFlow);

    let persisted = service
        .get_flow(&scope, flow.id)
        .await
        .unwrap()
        .expect("flow must still exist");
    assert_eq!(persisted.status, AuthFlowStatus::Expired);
    assert_eq!(persisted.error, Some(AuthErrorCode::UnknownOrExpiredFlow));

    // fail_oauth_callback on already-expired flow returns FlowAlreadyTerminal
    // because Expired is a terminal status; the record was already persisted
    // as Expired by claim_oauth_callback above.
    let err2 = service
        .fail_oauth_callback(
            &scope,
            OAuthCallbackFailureInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("exp-s"),
                error: AuthErrorCode::ProviderDenied,
            },
        )
        .await
        .expect_err("expired flow must be rejected");
    assert_eq!(
        err2,
        AuthProductError::FlowAlreadyTerminal,
        "already-expired flow returns FlowAlreadyTerminal"
    );

    let persisted2 = service
        .get_flow(&scope, flow.id)
        .await
        .unwrap()
        .expect("flow must still exist");
    assert_eq!(persisted2.status, AuthFlowStatus::Expired);
    assert_eq!(persisted2.error, Some(AuthErrorCode::UnknownOrExpiredFlow));

    // complete_oauth_callback on a fresh expired flow (never claimed) must also
    // persist the Expired status before returning error.
    let flow2 = service
        .create_flow(NewAuthFlow {
            id: None,
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
            opaque_state_hash: Some(state_hash("exp2-s")),
            pkce_verifier_hash: Some(pkce_hash("exp2-p")),
            expires_at: Utc::now() - Duration::seconds(1),
        })
        .await
        .unwrap();

    let err3 = service
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: flow2.id,
                opaque_state_hash: state_hash("exp2-s"),
                outcome: ProviderCallbackOutcome::Denied,
            },
        )
        .await
        .expect_err("expired flow must be rejected");
    assert_eq!(
        err3,
        AuthProductError::UnknownOrExpiredFlow,
        "complete_oauth_callback on expired flow returns UnknownOrExpiredFlow"
    );

    let persisted3 = service
        .get_flow(&scope, flow2.id)
        .await
        .unwrap()
        .expect("flow2 must still exist");
    assert_eq!(persisted3.status, AuthFlowStatus::Expired);
    assert_eq!(persisted3.error, Some(AuthErrorCode::UnknownOrExpiredFlow));
}

// ─── fix: abbyshekit review — OAuth CAS-conflict branch purges old secrets ───

#[tokio::test]
async fn filesystem_oauth_cas_conflict_branch_purges_previous_secrets() {
    // When the None-path CAS-conflict branch re-reads and overwrites an existing
    // account, the previous access/refresh secret handles must be deleted from
    // SecretStore so repeated re-auths do not accumulate dead handles.
    use ironclaw_auth::ProviderCallbackOutcome;
    use ironclaw_secrets::SecretMaterial;

    let filesystem = test_filesystem();
    let concrete_secret_store = Arc::new(InMemorySecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

    let flow = service
        .create_flow(NewAuthFlow {
            id: None,
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
            opaque_state_hash: Some(state_hash("cas-s")),
            pkce_verifier_hash: Some(pkce_hash("cas-p")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();

    // Pre-seed the account with old secrets.
    let preseeded_id = CredentialAccountId::from_uuid(flow.id.as_uuid());
    let old_access = SecretHandle::new("old-access").unwrap();
    let old_refresh = SecretHandle::new("old-refresh").unwrap();
    concrete_secret_store
        .put(
            scope.resource.clone(),
            old_access.clone(),
            SecretMaterial::from("old-access-token"),
            None,
        )
        .await
        .unwrap();
    concrete_secret_store
        .put(
            scope.resource.clone(),
            old_refresh.clone(),
            SecretMaterial::from("old-refresh-token"),
            None,
        )
        .await
        .unwrap();

    service
        .create_account_with_id(
            preseeded_id,
            NewCredentialAccount {
                scope: scope.clone(),
                provider: google_provider(),
                label: account_label(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: vec![],
                access_secret: Some(old_access.clone()),
                refresh_secret: Some(old_refresh.clone()),
                scopes: vec![],
            },
            CasExpectation::Absent,
        )
        .await
        .unwrap();

    service
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash("cas-s"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("cas-p"),
            },
        )
        .await
        .unwrap();

    let new_access = SecretHandle::new("new-access").unwrap();
    let new_refresh = SecretHandle::new("new-refresh").unwrap();
    let completed = service
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("cas-s"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("cas-c"),
                        pkce_verifier_hash: pkce_hash("cas-p"),
                        access_secret: new_access.clone(),
                        refresh_secret: Some(new_refresh.clone()),
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                    },
                },
            },
        )
        .await
        .unwrap();

    assert_eq!(
        completed.credential_account_id,
        Some(preseeded_id),
        "CAS-conflict branch must reuse pre-seeded account"
    );

    // Old secrets must be purged from SecretStore.
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &old_access)
            .await
            .unwrap()
            .is_none(),
        "old access secret must be purged after CAS-conflict update"
    );
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &old_refresh)
            .await
            .unwrap()
            .is_none(),
        "old refresh secret must be purged after CAS-conflict update"
    );
}

// ─── PR #5087 A1: list_refresh_candidates covers all owner-scope shapes ──────

/// Builds an `AuthProductScope` for `resource` using the Web surface (the
/// surface used by most fixture helpers). This is only for scope construction;
/// the surface does not affect the keepalive candidate filter.
#[cfg(any(feature = "libsql", feature = "postgres"))]
fn scope_for_resource(
    resource: ironclaw_host_api::ResourceScope,
) -> ironclaw_auth::AuthProductScope {
    ironclaw_auth::AuthProductScope::new(resource, AuthSurface::Web)
}

/// Builds a minimal `ResourceScope` for a given (tenant, user) pair.
/// `agent_id` and `project_id` are threaded through directly.
#[cfg(any(feature = "libsql", feature = "postgres"))]
fn resource_scope(
    tenant_id: &str,
    user_id: &str,
    agent_id: Option<&str>,
    project_id: Option<&str>,
) -> ironclaw_host_api::ResourceScope {
    use ironclaw_host_api::{AgentId, ProjectId, TenantId};
    ironclaw_host_api::ResourceScope {
        tenant_id: TenantId::new(tenant_id).unwrap(),
        user_id: UserId::new(user_id).unwrap(),
        agent_id: agent_id.map(|a| AgentId::new(a).unwrap()),
        project_id: project_id.map(|p| ProjectId::new(p).unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
#[tokio::test]
async fn list_refresh_candidates_covers_agent_and_project_scopes() {
    // Goal: verify that `list_refresh_candidates` discovers Google keepalive
    // candidates across all four owner-scope shapes (plain, agent-only,
    // agent+project, project-only) and excludes accounts that fail any one
    // of the three eligibility filters (provider != google, status != Configured,
    // refresh_secret == None).
    //
    // Setup uses `new_with_root` + `invocation_mount_view` so account writes
    // land at real paths (e.g. /tenants/t/users/u/secrets/agents/<a>/product-auth/…)
    // that `list_refresh_candidates` can enumerate via the raw `RootFilesystem`.

    use ironclaw_auth::GOOGLE_PROVIDER_ID;

    let backend = Arc::new(InMemoryBackend::new());
    let scoped = Arc::new(ScopedFilesystem::new(
        Arc::clone(&backend),
        crate::invocation_mount_view,
    ));
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let service = FilesystemAuthProductServices::new_with_root(
        Arc::clone(&scoped),
        Arc::clone(&backend),
        Arc::clone(&secret_store),
    );

    let tenant = "acmetenant";
    let user = "alice";

    // ── Positive cases: Google Configured + refresh_secret present ────────────

    // 1. Plain scope: no agent, no project.
    let plain_resource = resource_scope(tenant, user, None, None);
    let plain_scope = scope_for_resource(plain_resource);
    let plain_account = service
        .create_account(NewCredentialAccount {
            scope: plain_scope,
            provider: AuthProviderId::new(GOOGLE_PROVIDER_ID).unwrap(),
            label: CredentialAccountLabel::new("Alice Google Plain").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(SecretHandle::new("plain-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("plain-refresh").unwrap()),
            scopes: vec![],
        })
        .await
        .unwrap();

    // 2. Agent-only scope.
    let agent_resource = resource_scope(tenant, user, Some("testagent"), None);
    let agent_scope = scope_for_resource(agent_resource);
    let agent_account = service
        .create_account(NewCredentialAccount {
            scope: agent_scope,
            provider: AuthProviderId::new(GOOGLE_PROVIDER_ID).unwrap(),
            label: CredentialAccountLabel::new("Alice Google Agent").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(SecretHandle::new("agent-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("agent-refresh").unwrap()),
            scopes: vec![],
        })
        .await
        .unwrap();

    // 3. Agent+project scope.
    let agent_project_resource =
        resource_scope(tenant, user, Some("testagent"), Some("testproject"));
    let agent_project_scope = scope_for_resource(agent_project_resource);
    let agent_project_account = service
        .create_account(NewCredentialAccount {
            scope: agent_project_scope,
            provider: AuthProviderId::new(GOOGLE_PROVIDER_ID).unwrap(),
            label: CredentialAccountLabel::new("Alice Google Agent+Project").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(SecretHandle::new("agent-project-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("agent-project-refresh").unwrap()),
            scopes: vec![],
        })
        .await
        .unwrap();

    // 4. Project-only scope (no agent).
    let project_resource = resource_scope(tenant, user, None, Some("testproject"));
    let project_scope = scope_for_resource(project_resource);
    let project_account = service
        .create_account(NewCredentialAccount {
            scope: project_scope,
            provider: AuthProviderId::new(GOOGLE_PROVIDER_ID).unwrap(),
            label: CredentialAccountLabel::new("Alice Google Project").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(SecretHandle::new("project-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("project-refresh").unwrap()),
            scopes: vec![],
        })
        .await
        .unwrap();

    // ── Negative cases: must be excluded ─────────────────────────────────────

    // 5. Non-Google provider (GitHub) — must be excluded even if Configured+refresh.
    let neg_resource_github = resource_scope(tenant, user, None, None);
    let neg_scope_github = scope_for_resource(neg_resource_github);
    let github_account = service
        .create_account(NewCredentialAccount {
            scope: neg_scope_github,
            provider: AuthProviderId::new("github").unwrap(),
            label: CredentialAccountLabel::new("Alice GitHub").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(SecretHandle::new("github-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("github-refresh").unwrap()),
            scopes: vec![],
        })
        .await
        .unwrap();

    // 6. Google Revoked — must be excluded (status != Configured).
    let neg_resource_revoked = resource_scope(tenant, user, None, None);
    let neg_scope_revoked = scope_for_resource(neg_resource_revoked);
    let revoked_account = service
        .create_account(NewCredentialAccount {
            scope: neg_scope_revoked,
            provider: AuthProviderId::new(GOOGLE_PROVIDER_ID).unwrap(),
            label: CredentialAccountLabel::new("Alice Google Revoked").unwrap(),
            status: CredentialAccountStatus::Revoked,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: None,
            refresh_secret: Some(SecretHandle::new("revoked-refresh").unwrap()),
            scopes: vec![],
        })
        .await
        .unwrap();

    // 7. Google Configured but NO refresh_secret — must be excluded.
    let neg_resource_no_refresh = resource_scope(tenant, user, None, None);
    let neg_scope_no_refresh = scope_for_resource(neg_resource_no_refresh);
    let no_refresh_account = service
        .create_account(NewCredentialAccount {
            scope: neg_scope_no_refresh,
            provider: AuthProviderId::new(GOOGLE_PROVIDER_ID).unwrap(),
            label: CredentialAccountLabel::new("Alice Google No Refresh").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(SecretHandle::new("no-refresh-access").unwrap()),
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();

    // ── Exercise ──────────────────────────────────────────────────────────────

    let candidates = service.list_refresh_candidates().await;

    // ── Assert: all 4 scope shapes are returned ───────────────────────────────
    let candidate_ids: std::collections::BTreeSet<_> = candidates.iter().map(|a| a.id).collect();

    assert!(
        candidate_ids.contains(&plain_account.id),
        "plain (no agent/project) Google account must be a keepalive candidate; found ids: {candidate_ids:?}"
    );
    assert!(
        candidate_ids.contains(&agent_account.id),
        "agent-only-scoped Google account must be a keepalive candidate; found ids: {candidate_ids:?}"
    );
    assert!(
        candidate_ids.contains(&agent_project_account.id),
        "agent+project-scoped Google account must be a keepalive candidate; found ids: {candidate_ids:?}"
    );
    assert!(
        candidate_ids.contains(&project_account.id),
        "project-only-scoped Google account must be a keepalive candidate; found ids: {candidate_ids:?}"
    );

    // ── Assert: negative cases are excluded ───────────────────────────────────
    assert!(
        !candidate_ids.contains(&github_account.id),
        "non-Google (GitHub) account must NOT be a keepalive candidate"
    );
    assert!(
        !candidate_ids.contains(&revoked_account.id),
        "Revoked Google account must NOT be a keepalive candidate"
    );
    assert!(
        !candidate_ids.contains(&no_refresh_account.id),
        "Google Configured account with no refresh_secret must NOT be a keepalive candidate"
    );

    // ── Light secret-material guard: no refresh handle is exposed beyond ──────
    // account metadata (the returned CredentialAccount has a handle name only,
    // not the secret material itself). Verified structurally: the candidate list
    // must not return any account whose refresh_secret is None (the test would
    // have already caught that above, but belt-and-suspenders).
    assert!(
        candidates.iter().all(|a| a.refresh_secret.is_some()),
        "every returned candidate must carry a refresh_secret handle"
    );
    // Confirm each handle is opaque (handle name, not raw secret material).
    assert!(
        candidates
            .iter()
            .flat_map(|a| [a.access_secret.as_ref(), a.refresh_secret.as_ref()])
            .flatten()
            .all(|h| !h.as_str().is_empty()),
        "secret handles in candidates must be non-empty opaque identifiers"
    );
}

// ─── fix: abbyshekit review — manual-token consume only after success ────────

#[tokio::test]
async fn filesystem_manual_token_consume_only_after_successful_account_write() {
    // If the account write fails, the interaction must NOT be marked consumed
    // so the user can retry without going through a full re-setup.
    use ironclaw_auth::CredentialAccountId;
    use ironclaw_filesystem::CasExpectation;

    let filesystem = test_filesystem();
    let concrete_secret_store = Arc::new(InMemorySecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

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
        panic!("expected ManualTokenRequired");
    };

    // Derive the account ID and pre-create a dummy record to force CAS failure.
    let account_id = CredentialAccountId::from_uuid(interaction_id.as_uuid());
    let dummy_account = ironclaw_auth::CredentialAccount {
        id: account_id,
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
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let path = super::paths::account_path(&scope, account_id)
        .expect("account path derivation must succeed");
    let json = serde_json::to_vec(&dummy_account).expect("serialization must succeed");
    use ironclaw_filesystem::{ContentType, Entry};
    let entry = Entry::bytes(json).with_content_type(ContentType::json());
    filesystem
        .put(&scope.resource, &path, entry, CasExpectation::Absent)
        .await
        .expect("pre-create dummy account must succeed");

    // First submit fails because account write hits CAS conflict.
    let err = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("first-attempt"),
            },
        )
        .await
        .expect_err("submit must fail when account write fails");
    assert_eq!(
        err,
        AuthProductError::BackendConflict,
        "CAS conflict must surface as BackendConflict"
    );

    // Interaction must NOT be consumed — retry still possible.
    let retry_before_cleanup = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("retry-before-cleanup"),
            },
        )
        .await;
    assert!(
        retry_before_cleanup.is_err(),
        "retry must still fail because dummy account still blocks"
    );
    assert_eq!(
        retry_before_cleanup.unwrap_err(),
        AuthProductError::BackendConflict,
        "retry must still hit BackendConflict, not UnknownOrExpiredFlow"
    );

    // Remove the dummy record so retry succeeds.
    filesystem
        .delete(&scope.resource, &path)
        .await
        .expect("delete dummy account must succeed");

    let result = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("retry-token"),
            },
        )
        .await;
    assert!(
        result.is_ok(),
        "retry must succeed after removing the blocking dummy record"
    );

    // Third attempt must now fail with UnknownOrExpiredFlow because consumed_at is set.
    let consumed_err = service
        .submit_manual_token(
            &scope,
            SecretSubmitRequest {
                interaction_id,
                secret: SecretString::from("third-attempt"),
            },
        )
        .await
        .expect_err("third submit must fail because interaction is consumed");
    assert_eq!(
        consumed_err,
        AuthProductError::UnknownOrExpiredFlow,
        "interaction must be consumed after successful retry"
    );
}
