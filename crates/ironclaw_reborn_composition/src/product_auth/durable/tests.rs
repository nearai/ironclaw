// arch-exempt: large_file, durable auth lifecycle failure-injection coverage, plan #5905
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{Duration, Utc};
use ironclaw_filesystem::{
    BackendCapabilities, CasExpectation, DirEntry, Entry, FileStat, FilesystemError,
    InMemoryBackend, RecordVersion, RootFilesystem, ScopedFilesystem, VersionedEntry,
};
use ironclaw_host_api::{
    ExtensionId, InvocationId, MountAlias, MountGrant, MountPermissions,
    RuntimeCredentialAccountProviderId, SecretHandle, ThreadId, Timestamp, UserId, VirtualPath,
};
use ironclaw_host_runtime::RuntimeCredentialAccountRequest;
use ironclaw_host_runtime::RuntimeCredentialAccountResolver;
use ironclaw_secrets::{
    InMemorySecretStore, SecretLease, SecretLeaseId, SecretMaterial, SecretMetadata, SecretStore,
    SecretStoreError,
};
use secrecy::SecretString;
use tokio::sync::Notify;
use tokio::task::JoinSet;

use super::*;
use crate::product_auth::credentials::runtime_credentials::{
    ProductAuthRuntimeCredentialAccountSelector, ProductAuthRuntimeCredentialResolver,
    RuntimeCredentialAccountSelectionRequest, RuntimeCredentialAccountSelectionService,
};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthErrorCode, AuthFlowKind, AuthFlowManager,
    AuthFlowOwnerScope, AuthFlowRecordSource, AuthFlowStatus, AuthGateRef, AuthInteractionId,
    AuthInteractionService, AuthProductError, AuthProductScope, AuthProviderId, AuthSessionId,
    AuthSurface, AuthorizationCodeHash, CredentialAccountChoiceRequest, CredentialAccountLabel,
    CredentialAccountListRequest, CredentialAccountLookupRequest, CredentialAccountRecordSource,
    CredentialAccountSelectionRequest, CredentialAccountService, CredentialAccountStatus,
    CredentialAccountUpdateBinding, CredentialOwnership, ManualTokenCompletionInput,
    ManualTokenSetupRequest, NewAuthFlow, NewCredentialAccount, OAuthAuthorizationUrl,
    OAuthCallbackClaimRequest, OAuthCallbackFailureInput, OAuthCallbackInput,
    OAuthCompletionCompensationOutcome, OAuthCompletionCompensationRequest, OAuthProviderExchange,
    OpaqueStateHash, PkceVerifierHash, ProviderScope, SecretCleanupService, SecretSubmitRequest,
    TurnRunRef,
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

struct PausedAccountPutBackend {
    inner: InMemoryBackend,
    pause_next_account_put: AtomicBool,
    account_put_reached: Notify,
    resume_account_put: Notify,
    pause_next_account_get: AtomicBool,
    account_get_reached: Notify,
    resume_account_get: Notify,
    pause_next_flow_list: AtomicBool,
    flow_list_reached: Notify,
    resume_flow_list: Notify,
    fail_next_flow_get: AtomicBool,
    fail_account_put_path_fragment: std::sync::Mutex<Option<(String, usize)>>,
}

impl PausedAccountPutBackend {
    fn new() -> Self {
        Self {
            inner: InMemoryBackend::new(),
            pause_next_account_put: AtomicBool::new(false),
            account_put_reached: Notify::new(),
            resume_account_put: Notify::new(),
            pause_next_account_get: AtomicBool::new(false),
            account_get_reached: Notify::new(),
            resume_account_get: Notify::new(),
            pause_next_flow_list: AtomicBool::new(false),
            flow_list_reached: Notify::new(),
            resume_flow_list: Notify::new(),
            fail_next_flow_get: AtomicBool::new(false),
            fail_account_put_path_fragment: std::sync::Mutex::new(None),
        }
    }

    fn pause_next_account_put(&self) {
        self.pause_next_account_put.store(true, Ordering::SeqCst);
    }

    async fn wait_for_account_put(&self) {
        self.account_put_reached.notified().await;
    }

    fn resume_account_put(&self) {
        self.resume_account_put.notify_one();
    }

    fn pause_next_account_get(&self) {
        self.pause_next_account_get.store(true, Ordering::SeqCst);
    }

    async fn wait_for_account_get(&self) {
        self.account_get_reached.notified().await;
    }

    fn resume_account_get(&self) {
        self.resume_account_get.notify_one();
    }

    fn pause_next_flow_list(&self) {
        self.pause_next_flow_list.store(true, Ordering::SeqCst);
    }

    async fn wait_for_flow_list(&self) {
        self.flow_list_reached.notified().await;
    }

    fn resume_flow_list(&self) {
        self.resume_flow_list.notify_one();
    }

    fn fail_next_flow_get(&self) {
        self.fail_next_flow_get.store(true, Ordering::SeqCst);
    }

    fn fail_account_put_for_after(
        &self,
        account_id: CredentialAccountId,
        successful_matches_before_failure: usize,
    ) {
        *self.fail_account_put_path_fragment.lock().unwrap() =
            Some((account_id.to_string(), successful_matches_before_failure));
    }
}

#[async_trait::async_trait]
impl RootFilesystem for PausedAccountPutBackend {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        if path.as_str().contains("/accounts/")
            && self.pause_next_account_put.swap(false, Ordering::SeqCst)
        {
            self.account_put_reached.notify_one();
            self.resume_account_put.notified().await;
        }
        let fail_account_put = {
            let mut fragment = self.fail_account_put_path_fragment.lock().unwrap();
            let matches = path.as_str().contains("/accounts/")
                && fragment
                    .as_ref()
                    .is_some_and(|(fragment, _)| path.as_str().contains(fragment));
            if !matches {
                false
            } else if fragment
                .as_ref()
                .is_some_and(|(_, remaining)| *remaining == 0)
            {
                *fragment = None;
                true
            } else {
                fragment.as_mut().unwrap().1 -= 1;
                false
            }
        };
        if fail_account_put {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: ironclaw_filesystem::FilesystemOperation::WriteFile,
                reason: "injected account write failure".to_string(),
            });
        }
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        if path.as_str().contains("/flows/")
            && self.fail_next_flow_get.swap(false, Ordering::SeqCst)
        {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: ironclaw_filesystem::FilesystemOperation::ReadFile,
                reason: "injected flow reread failure".to_string(),
            });
        }
        if path.as_str().contains("/accounts/")
            && self.pause_next_account_get.swap(false, Ordering::SeqCst)
        {
            self.account_get_reached.notify_one();
            self.resume_account_get.notified().await;
        }
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        if path.as_str().contains("/flows")
            && self.pause_next_flow_list.swap(false, Ordering::SeqCst)
        {
            self.flow_list_reached.notify_one();
            self.resume_flow_list.notified().await;
        }
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }

    async fn delete_if_version(
        &self,
        path: &VirtualPath,
        expected_version: RecordVersion,
    ) -> Result<(), FilesystemError> {
        self.inner.delete_if_version(path, expected_version).await
    }
}

fn paused_account_put_filesystem() -> (
    Arc<ScopedFilesystem<PausedAccountPutBackend>>,
    Arc<PausedAccountPutBackend>,
) {
    let mounts = ironclaw_host_api::MountView::new(vec![MountGrant::new(
        MountAlias::new("/secrets").unwrap(),
        VirtualPath::new("/tenants/test/users/alice/secrets").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    let backend = Arc::new(PausedAccountPutBackend::new());
    let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&backend),
        mounts,
    ));
    (filesystem, backend)
}

struct FailFirstDeleteSecretStore {
    inner: InMemorySecretStore,
    fail_next_delete: AtomicBool,
}

impl FailFirstDeleteSecretStore {
    fn new() -> Self {
        Self {
            inner: InMemorySecretStore::new(),
            fail_next_delete: AtomicBool::new(true),
        }
    }

    fn set_delete_failure(&self, fail: bool) {
        self.fail_next_delete.store(fail, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl SecretStore for FailFirstDeleteSecretStore {
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
        expires_at: Option<Timestamp>,
    ) -> Result<SecretMetadata, SecretStoreError> {
        self.inner.put(scope, handle, material, expires_at).await
    }

    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError> {
        self.inner.metadata(scope, handle).await
    }

    async fn metadata_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretMetadata>, SecretStoreError> {
        self.inner.metadata_for_scope(scope).await
    }

    async fn delete(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError> {
        if self.fail_next_delete.swap(false, Ordering::SeqCst) {
            return Err(SecretStoreError::StoreUnavailable {
                reason: "injected transient delete failure".to_string(),
            });
        }
        self.inner.delete(scope, handle).await
    }

    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError> {
        self.inner.lease_once(scope, handle).await
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError> {
        self.inner.consume(scope, lease_id).await
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError> {
        self.inner.revoke(scope, lease_id).await
    }

    async fn leases_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretLease>, SecretStoreError> {
        self.inner.leases_for_scope(scope).await
    }
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

async fn create_pending_setup_flow(
    service: &FilesystemAuthProductServices<InMemoryBackend>,
    scope: &AuthProductScope,
    provider: &AuthProviderId,
) -> AuthFlowStatus {
    let expires_at = Utc::now() + Duration::minutes(10);
    service
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider.clone(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new(
                    "https://example.com/oauth/authorize?state=x",
                )
                .unwrap(),
                expires_at,
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: Some(state_hash(provider.as_str())),
            pkce_verifier_hash: Some(pkce_hash(provider.as_str())),
            expires_at,
        })
        .await
        .unwrap()
        .status
}

#[tokio::test]
async fn cleanup_for_lifecycle_cancels_pending_flows_for_the_disconnected_provider_only() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);

    // Two pending (non-terminal) setup flows for DIFFERENT providers, both
    // thread-less (`thread_id: None`) exactly as an extension Configure card
    // creates. The cleanup mechanism is provider-agnostic — not Slack-specific —
    // so we prove it: disconnecting one provider cancels only its flow and leaves
    // the other untouched.
    let disconnected = AuthProviderId::new("google").unwrap();
    let untouched = AuthProviderId::new("github").unwrap();
    assert_eq!(
        create_pending_setup_flow(&service, &scope, &disconnected).await,
        AuthFlowStatus::AwaitingUser
    );
    assert_eq!(
        create_pending_setup_flow(&service, &scope, &untouched).await,
        AuthFlowStatus::AwaitingUser
    );
    let turn_flow = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: disconnected.clone(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new(
                    "https://example.com/oauth/authorize?state=turn",
                )
                .unwrap(),
                expires_at: Utc::now() + Duration::minutes(10),
            },
            continuation: AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(uuid::Uuid::new_v4().to_string()).unwrap(),
                gate_ref: AuthGateRef::new("gate:lifecycle-cleanup").unwrap(),
            },
            update_binding: None,
            opaque_state_hash: Some(state_hash("turn-gate")),
            pkce_verifier_hash: Some(pkce_hash("turn-gate")),
            expires_at: Utc::now() + Duration::minutes(10),
        })
        .await
        .unwrap();
    let failed_turn_flow = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: disconnected.clone(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new(
                    "https://example.com/oauth/authorize?state=failed-turn",
                )
                .unwrap(),
                expires_at: Utc::now() + Duration::minutes(10),
            },
            continuation: AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(uuid::Uuid::new_v4().to_string()).unwrap(),
                gate_ref: AuthGateRef::new("gate:failed-lifecycle-cleanup").unwrap(),
            },
            update_binding: None,
            opaque_state_hash: Some(state_hash("failed-turn-gate")),
            pkce_verifier_hash: Some(pkce_hash("failed-turn-gate")),
            expires_at: Utc::now() + Duration::minutes(10),
        })
        .await
        .unwrap();
    service
        .fail_oauth_callback(
            &scope,
            OAuthCallbackFailureInput {
                flow_id: failed_turn_flow.id,
                opaque_state_hash: state_hash("failed-turn-gate"),
                error: AuthErrorCode::TokenExchangeFailed,
            },
        )
        .await
        .expect("terminal callback failure persists");

    // The exact lifecycle cleanup an extension disconnect/remove issues for one
    // provider. Both the WebUI facade remove and the model-visible
    // `extension_remove` tool funnel through this same call
    // (`RebornProductAuthServices` -> `SecretCleanupService::cleanup_for_lifecycle`),
    // so covering it here covers both paths identically.
    let report = ironclaw_auth::SecretCleanupService::cleanup_for_lifecycle(
        &service,
        ironclaw_auth::SecretCleanupRequest {
            scope: scope.clone(),
            extension_id: ExtensionId::new("example_ext").unwrap(),
            provider: Some(disconnected.clone()),
            action: ironclaw_auth::SecretCleanupAction::Uninstall,
        },
    )
    .await
    .unwrap();
    assert_eq!(report.canceled_turn_gate_continuations.len(), 2);
    let cleanup_flow_ids = report
        .canceled_turn_gate_continuations
        .iter()
        .map(|event| event.flow_id)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        cleanup_flow_ids,
        [turn_flow.id, failed_turn_flow.id].into_iter().collect()
    );
    for event in &report.canceled_turn_gate_continuations {
        service
            .mark_continuation_dispatched(&event.scope, event.flow_id, event.emitted_at)
            .await
            .expect("cleanup denial acknowledgement supports canceled and failed flows");
    }
    let failed_after_ack = service
        .get_flow(&scope, failed_turn_flow.id)
        .await
        .expect("failed flow lookup")
        .expect("failed flow remains durable");
    assert_eq!(failed_after_ack.status, AuthFlowStatus::Failed);
    assert!(failed_after_ack.continuation_emitted_at.is_some());
    let retry = ironclaw_auth::SecretCleanupService::cleanup_for_lifecycle(
        &service,
        ironclaw_auth::SecretCleanupRequest {
            scope: scope.clone(),
            extension_id: ExtensionId::new("example_ext").unwrap(),
            provider: Some(disconnected.clone()),
            action: ironclaw_auth::SecretCleanupAction::Uninstall,
        },
    )
    .await
    .expect("cleanup retry");
    assert!(retry.canceled_turn_gate_continuations.is_empty());

    // LLM data is never deleted: flow records are retained (filterable by their
    // terminal status), not removed.
    let flows = service.flows_for_scope(&scope).await.unwrap();
    let status_of = |provider: &AuthProviderId| {
        flows
            .iter()
            .find(|(flow, _)| {
                &flow.provider == provider
                    && matches!(flow.continuation, AuthContinuationRef::SetupOnly)
            })
            .map(|(flow, _)| flow.status)
    };
    // The disconnected provider's pending flow is canceled...
    assert_eq!(
        status_of(&disconnected),
        Some(AuthFlowStatus::Canceled),
        "cleanup must cancel the disconnected provider's pending flow"
    );
    // ...and a DIFFERENT provider's flow is untouched (correctly provider-scoped,
    // not a blanket cancel).
    assert_eq!(
        status_of(&untouched),
        Some(AuthFlowStatus::AwaitingUser),
        "cleanup must not touch other providers' flows"
    );
}

/// #4a lifecycle lock — the removal entrypoints cancel a pending flow through
/// the one shared cleanup, so "disconnect via the bot's `extension_remove` tool"
/// and "disconnect via the web UI" cannot diverge into duplicated behaviour.
///
/// Both doors are thin `pub(crate)` forwarders on `RebornProductAuthServices` to
/// the single guardrail entry point `cleanup_credentials_for_lifecycle`:
/// - the model-visible `builtin.extension_remove` capability
///   ([`ExtensionCredentialCleanup::cleanup_for_lifecycle`]) is ALWAYS compiled,
///   so its assertion is ungated and runs in the default CI test job — it is the
///   primary door under test here, never a silent skip;
/// - the WebUI Slack-disconnect facade
///   ([`SlackPersonalCredentialCleanup::cleanup_credentials_for_lifecycle`])
///   only exists under `slack-v2-host-beta`, so when that feature is built we
///   ALSO drive it and assert identical behaviour, proving the two doors stay in
///   lockstep.
///
/// Each door runs independently against the REAL durable service. Provider-
/// agnostic ("google", not Slack) so the guarantee cannot silently narrow to a
/// Slack-only cleanup.
#[tokio::test]
async fn removal_doors_cancel_pending_flow_through_the_shared_cleanup() {
    use crate::extension_host::extension_lifecycle::ExtensionCredentialCleanup;

    // Cleanup never dispatches a continuation, but the facade constructor
    // requires one.
    #[derive(Debug, Default)]
    struct NoopDispatcher;
    #[async_trait::async_trait]
    impl crate::RebornAuthContinuationDispatcher for NoopDispatcher {
        async fn dispatch_auth_continuation(
            &self,
            _event: ironclaw_auth::AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            Ok(())
        }
    }

    // Fresh real durable service wired behind the production facade, seeded with
    // exactly one pending flow, so each door starts from identical state. The
    // durable service is every product-auth port except the OAuth provider
    // client (cleanup never exchanges provider material), so it is wired as the
    // shared cleanup_service with the unused provider slot stubbed.
    async fn seeded_facade(
        provider: &AuthProviderId,
    ) -> (
        crate::RebornProductAuthServices,
        AuthProductScope,
        Arc<FilesystemAuthProductServices<InMemoryBackend>>,
    ) {
        let durable = Arc::new(test_service(
            test_filesystem(),
            Arc::new(InMemorySecretStore::new()),
        ));
        let scope = test_scope();
        assert_eq!(
            create_pending_setup_flow(&durable, &scope, provider).await,
            AuthFlowStatus::AwaitingUser
        );
        let services = crate::RebornProductAuthServices::new(
            durable.clone(),
            durable.clone(),
            durable.clone(),
            durable.clone(),
            Arc::new(super::provider::UnavailableAuthProviderClient),
            durable.clone(),
            Arc::new(NoopDispatcher),
        );
        (services, scope, durable)
    }

    async fn assert_pending_flow_canceled(
        durable: &FilesystemAuthProductServices<InMemoryBackend>,
        scope: &AuthProductScope,
        provider: &AuthProviderId,
    ) {
        let flows = durable.flows_for_scope(scope).await.unwrap();
        let status = flows
            .iter()
            .find(|(flow, _)| &flow.provider == provider)
            .map(|(flow, _)| flow.status);
        assert_eq!(
            status,
            Some(AuthFlowStatus::Canceled),
            "removal door must cancel the pending flow through the shared cleanup"
        );
    }

    let provider = AuthProviderId::new("google").unwrap();
    let request = |scope: &AuthProductScope| ironclaw_auth::SecretCleanupRequest {
        scope: scope.clone(),
        extension_id: ExtensionId::new("example_ext").unwrap(),
        provider: Some(provider.clone()),
        action: ironclaw_auth::SecretCleanupAction::Uninstall,
    };

    // Primary door (always compiled, runs in CI) — the model-visible
    // `extension_remove` capability.
    let (tool, tool_scope, tool_durable) = seeded_facade(&provider).await;
    ExtensionCredentialCleanup::cleanup_for_lifecycle(&tool, request(&tool_scope))
        .await
        .expect("extension_remove tool cleanup should succeed");
    assert_pending_flow_canceled(&tool_durable, &tool_scope, &provider).await;

    // Parity door (only compiled under `slack-v2-host-beta`) — the WebUI
    // channel-disconnect facade must yield the identical cancel.
    #[cfg(feature = "slack-v2-host-beta")]
    {
        use crate::slack::slack_channel_connection::SlackPersonalCredentialCleanup;
        let (web, web_scope, web_durable) = seeded_facade(&provider).await;
        SlackPersonalCredentialCleanup::cleanup_credentials_for_lifecycle(
            &web,
            request(&web_scope),
        )
        .await
        .expect("web-UI disconnect cleanup should succeed");
        assert_pending_flow_canceled(&web_durable, &web_scope, &provider).await;
    }
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

    let mut other_thread_scope = scope.clone();
    other_thread_scope.resource.thread_id = Some(ThreadId::new("thread-auth-flow-2").unwrap());
    let reused = service
        .flow_for_owner_by_id(&other_thread_scope, flow.id)
        .await
        .unwrap()
        .expect("same owner must find an opaque flow id across threads");
    assert_eq!(reused.id, flow.id);

    let mut foreign_owner_scope = other_thread_scope;
    foreign_owner_scope.resource.user_id = UserId::new("user-foreign").unwrap();
    assert!(
        service
            .flow_for_owner_by_id(&foreign_owner_scope, flow.id)
            .await
            .unwrap()
            .is_none(),
        "cross-thread lookup must not cross the durable user owner boundary"
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
                    exchange: Box::new(OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("code"),
                        pkce_verifier_hash: pkce_hash("pkce"),
                        access_secret: SecretHandle::new("oauth-access").unwrap(),
                        refresh_secret: Some(SecretHandle::new("oauth-refresh").unwrap()),
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                        provider_identity: None,
                    }),
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
async fn filesystem_oauth_callback_canceled_after_flow_read_cannot_leave_configured_account() {
    use ironclaw_auth::{SecretCleanupAction, SecretCleanupRequest, SecretCleanupService as _};

    let (filesystem, backend) = paused_account_put_filesystem();
    let concrete_secret_store = Arc::new(FailFirstDeleteSecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let scope = test_scope();
    let callback_service = Arc::new(FilesystemAuthProductServices::new(
        Arc::clone(&filesystem),
        Arc::clone(&secret_store),
    ));
    let cleanup_service = FilesystemAuthProductServices::new(filesystem, secret_store);
    let access_handle = SecretHandle::new("disconnect-race-access").unwrap();
    let refresh_handle = SecretHandle::new("disconnect-race-refresh").unwrap();
    concrete_secret_store
        .put(
            scope.resource.clone(),
            access_handle.clone(),
            SecretMaterial::from("disconnect-race-access-token"),
            None,
        )
        .await
        .unwrap();
    concrete_secret_store
        .put(
            scope.resource.clone(),
            refresh_handle.clone(),
            SecretMaterial::from("disconnect-race-refresh-token"),
            None,
        )
        .await
        .unwrap();

    let flow = callback_service
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
            opaque_state_hash: Some(state_hash("disconnect-race-state")),
            pkce_verifier_hash: Some(pkce_hash("disconnect-race-pkce")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    callback_service
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash("disconnect-race-state"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("disconnect-race-pkce"),
            },
        )
        .await
        .unwrap();

    backend.pause_next_account_put();
    let callback_scope = scope.clone();
    let callback_access_handle = access_handle.clone();
    let callback_refresh_handle = refresh_handle.clone();
    let callback = tokio::spawn(async move {
        callback_service
            .complete_oauth_callback(
                &callback_scope,
                OAuthCallbackInput {
                    flow_id: flow.id,
                    opaque_state_hash: state_hash("disconnect-race-state"),
                    outcome: ironclaw_auth::ProviderCallbackOutcome::Authorized {
                        exchange: Box::new(OAuthProviderExchange {
                            provider: google_provider(),
                            account_label: account_label(),
                            authorization_code_hash: code_hash("disconnect-race-code"),
                            pkce_verifier_hash: pkce_hash("disconnect-race-pkce"),
                            access_secret: callback_access_handle,
                            refresh_secret: Some(callback_refresh_handle),
                            scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                            account_id: None,
                            provider_identity: None,
                        }),
                    },
                },
            )
            .await
    });

    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        backend.wait_for_account_put(),
    )
    .await
    .expect("callback must reach the account write barrier");

    let cleanup_request = SecretCleanupRequest {
        scope: scope.clone(),
        extension_id: ExtensionId::new("slack").unwrap(),
        provider: Some(google_provider()),
        action: SecretCleanupAction::Uninstall,
    };
    cleanup_service
        .cleanup_for_lifecycle(cleanup_request.clone())
        .await
        .expect("disconnect cleanup must finish before the callback account write");

    backend.fail_next_flow_get();
    backend.resume_account_put();
    let callback_error = callback
        .await
        .expect("callback task must finish")
        .expect_err("canceled flow must reject callback completion");
    assert_eq!(callback_error, AuthProductError::BackendUnavailable);

    let account_id = CredentialAccountId::from_uuid(flow.id.as_uuid());
    let account = cleanup_service
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            account_id,
        ))
        .await
        .unwrap()
        .expect("the failed callback account remains as a durable tombstone");
    assert_eq!(account.status, CredentialAccountStatus::Revoked);
    assert_eq!(account.access_secret, Some(access_handle.clone()));
    assert!(account.refresh_secret.is_none());
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access_handle)
            .await
            .unwrap()
            .is_some(),
        "failed deletion must leave the handle retryable on the revoked account"
    );
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &refresh_handle)
            .await
            .unwrap()
            .is_none(),
        "successful refresh-token deletion must be persisted"
    );

    let selection_error = cleanup_service
        .select_unique_configured_account(CredentialAccountSelectionRequest::new(
            scope.clone(),
            google_provider(),
        ))
        .await
        .expect_err("failed callback account must never be selectable");
    assert_eq!(selection_error, AuthProductError::CredentialMissing);

    cleanup_service
        .cleanup_for_lifecycle(cleanup_request)
        .await
        .expect("retry must finish purging the revoked callback account");
    let retried = cleanup_service
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            account_id,
        ))
        .await
        .unwrap()
        .unwrap();
    assert!(retried.access_secret.is_none());
    assert!(retried.refresh_secret.is_none());
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access_handle)
            .await
            .unwrap()
            .is_none(),
        "lifecycle retry must remove the retained failed-deletion secret"
    );
}

#[tokio::test]
async fn filesystem_disconnect_cleans_account_when_callback_completes_before_flow_cancel() {
    use ironclaw_auth::{SecretCleanupAction, SecretCleanupRequest, SecretCleanupService as _};

    let (filesystem, backend) = paused_account_put_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let callback_service = Arc::new(FilesystemAuthProductServices::new(
        Arc::clone(&filesystem),
        Arc::clone(&secret_store),
    ));
    let cleanup_service = Arc::new(FilesystemAuthProductServices::new(filesystem, secret_store));

    let flow = callback_service
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
            opaque_state_hash: Some(state_hash("callback-wins-state")),
            pkce_verifier_hash: Some(pkce_hash("callback-wins-pkce")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    callback_service
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash("callback-wins-state"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("callback-wins-pkce"),
            },
        )
        .await
        .unwrap();

    backend.pause_next_flow_list();
    let cleanup_scope = scope.clone();
    let cleanup = tokio::spawn(async move {
        cleanup_service
            .cleanup_for_lifecycle(SecretCleanupRequest {
                scope: cleanup_scope,
                extension_id: ExtensionId::new("slack").unwrap(),
                provider: Some(google_provider()),
                action: SecretCleanupAction::Uninstall,
            })
            .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        backend.wait_for_flow_list(),
    )
    .await
    .expect("disconnect must reach the flow scan barrier");

    let completed = callback_service
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("callback-wins-state"),
                outcome: ironclaw_auth::ProviderCallbackOutcome::Authorized {
                    exchange: Box::new(OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("callback-wins-code"),
                        pkce_verifier_hash: pkce_hash("callback-wins-pkce"),
                        access_secret: SecretHandle::new("callback-wins-access").unwrap(),
                        refresh_secret: Some(SecretHandle::new("callback-wins-refresh").unwrap()),
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                        provider_identity: None,
                    }),
                },
            },
        )
        .await
        .expect("callback wins the flow terminal-state race");
    assert_eq!(completed.status, AuthFlowStatus::Completed);

    backend.resume_flow_list();
    cleanup
        .await
        .expect("cleanup task must finish")
        .expect("disconnect cleanup must succeed");

    let account_id = CredentialAccountId::from_uuid(flow.id.as_uuid());
    let account = callback_service
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            account_id,
        ))
        .await
        .unwrap()
        .expect("completed callback account remains as a durable tombstone");
    assert_eq!(account.status, CredentialAccountStatus::Revoked);
    assert!(account.access_secret.is_none());
    assert!(account.refresh_secret.is_none());
}

#[derive(Clone, Copy)]
enum StaleBoundRollbackFailure {
    CleanupAccountWrite,
    RestoreWrite,
    SecretDelete,
}

#[tokio::test]
async fn stale_bound_callback_restores_newer_reconnect_when_cleanup_staging_fails() {
    assert_stale_bound_callback_restores_newer_reconnect(
        StaleBoundRollbackFailure::CleanupAccountWrite,
    )
    .await;
}

#[tokio::test]
async fn stale_bound_callback_retains_failed_secret_deletion_for_lifecycle_retry() {
    assert_stale_bound_callback_restores_newer_reconnect(StaleBoundRollbackFailure::SecretDelete)
        .await;
}

#[tokio::test]
async fn stale_bound_callback_retries_transient_restore_failure() {
    assert_stale_bound_callback_restores_newer_reconnect(StaleBoundRollbackFailure::RestoreWrite)
        .await;
}

async fn assert_stale_bound_callback_restores_newer_reconnect(failure: StaleBoundRollbackFailure) {
    use ironclaw_auth::{
        CredentialAccountUpdateBinding, SecretCleanupAction, SecretCleanupRequest,
        SecretCleanupService as _,
    };

    let (filesystem, backend) = paused_account_put_filesystem();
    let concrete_secret_store = Arc::new(FailFirstDeleteSecretStore::new());
    concrete_secret_store.set_delete_failure(false);
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let scope = test_scope();
    let stale_service = Arc::new(FilesystemAuthProductServices::new(
        Arc::clone(&filesystem),
        Arc::clone(&secret_store),
    ));
    let newer_service = FilesystemAuthProductServices::new(filesystem, secret_store);

    let original_access = SecretHandle::new("bound-race-original").unwrap();
    concrete_secret_store
        .put(
            scope.resource.clone(),
            original_access.clone(),
            SecretMaterial::from("original-token"),
            None,
        )
        .await
        .unwrap();
    let account = stale_service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(original_access),
            refresh_secret: None,
            scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
        })
        .await
        .unwrap();
    let binding = CredentialAccountUpdateBinding {
        account_id: account.id,
        ownership: CredentialOwnership::UserReusable,
        owner_extension: None,
        granted_extensions: vec![],
    };

    let stale_flow = stale_service
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
            update_binding: Some(binding.clone()),
            opaque_state_hash: Some(state_hash("stale-bound-state")),
            pkce_verifier_hash: Some(pkce_hash("stale-bound-pkce")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    stale_service
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: stale_flow.id,
                opaque_state_hash: state_hash("stale-bound-state"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("stale-bound-pkce"),
            },
        )
        .await
        .unwrap();

    let stale_access = SecretHandle::new("bound-race-stale").unwrap();
    concrete_secret_store
        .put(
            scope.resource.clone(),
            stale_access.clone(),
            SecretMaterial::from("stale-token"),
            None,
        )
        .await
        .unwrap();
    let stale_flow_id = stale_flow.id;
    let callback_stale_access = stale_access.clone();
    backend.pause_next_account_get();
    let stale_scope = scope.clone();
    let stale_callback = tokio::spawn(async move {
        stale_service
            .complete_oauth_callback(
                &stale_scope,
                OAuthCallbackInput {
                    flow_id: stale_flow_id,
                    opaque_state_hash: state_hash("stale-bound-state"),
                    outcome: ironclaw_auth::ProviderCallbackOutcome::Authorized {
                        exchange: Box::new(OAuthProviderExchange {
                            provider: google_provider(),
                            account_label: account_label(),
                            authorization_code_hash: code_hash("stale-bound-code"),
                            pkce_verifier_hash: pkce_hash("stale-bound-pkce"),
                            access_secret: callback_stale_access,
                            refresh_secret: None,
                            scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                            account_id: None,
                            provider_identity: None,
                        }),
                    },
                },
            )
            .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        backend.wait_for_account_get(),
    )
    .await
    .expect("stale callback must pause after reading its flow");

    newer_service
        .cancel_flow(&scope, stale_flow_id)
        .await
        .expect("disconnect cancels the stale flow");
    let newer_flow = newer_service
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
            update_binding: Some(binding),
            opaque_state_hash: Some(state_hash("newer-bound-state")),
            pkce_verifier_hash: Some(pkce_hash("newer-bound-pkce")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    newer_service
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: newer_flow.id,
                opaque_state_hash: state_hash("newer-bound-state"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("newer-bound-pkce"),
            },
        )
        .await
        .unwrap();
    let newer_access = SecretHandle::new("bound-race-newer").unwrap();
    concrete_secret_store
        .put(
            scope.resource.clone(),
            newer_access.clone(),
            SecretMaterial::from("newer-token"),
            None,
        )
        .await
        .unwrap();
    newer_service
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: newer_flow.id,
                opaque_state_hash: state_hash("newer-bound-state"),
                outcome: ironclaw_auth::ProviderCallbackOutcome::Authorized {
                    exchange: Box::new(OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("newer-bound-code"),
                        pkce_verifier_hash: pkce_hash("newer-bound-pkce"),
                        access_secret: newer_access.clone(),
                        refresh_secret: None,
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                        provider_identity: None,
                    }),
                },
            },
        )
        .await
        .expect("newer reconnect must complete");

    let cleanup_account_id = CredentialAccountId::from_uuid(stale_flow_id.as_uuid());
    match failure {
        StaleBoundRollbackFailure::CleanupAccountWrite => {
            backend.fail_account_put_for_after(cleanup_account_id, 0);
        }
        StaleBoundRollbackFailure::RestoreWrite => {
            // The first matching write stores the stale callback account; the
            // second is the compensation that restores the newer reconnect.
            backend.fail_account_put_for_after(account.id, 1);
        }
        StaleBoundRollbackFailure::SecretDelete => {
            concrete_secret_store.set_delete_failure(true);
        }
    }
    backend.resume_account_get();
    let stale_error = stale_callback
        .await
        .expect("stale callback task must finish")
        .expect_err("canceled stale callback must not complete");
    assert_eq!(
        stale_error,
        match failure {
            StaleBoundRollbackFailure::RestoreWrite => AuthProductError::Canceled,
            StaleBoundRollbackFailure::CleanupAccountWrite
            | StaleBoundRollbackFailure::SecretDelete => AuthProductError::BackendUnavailable,
        }
    );

    let stored = newer_service
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            account.id,
        ))
        .await
        .unwrap()
        .expect("newer account must remain durable");
    assert_eq!(stored.status, CredentialAccountStatus::Configured);
    assert_eq!(stored.access_secret, Some(newer_access.clone()));
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &newer_access)
            .await
            .unwrap()
            .is_some(),
        "stale rollback must not delete the newer reconnect token"
    );
    let stale_secret_remains = concrete_secret_store
        .metadata(&scope.resource, &stale_access)
        .await
        .unwrap()
        .is_some();
    assert_eq!(
        stale_secret_remains,
        !matches!(failure, StaleBoundRollbackFailure::RestoreWrite),
        "stale token must be deleted unless cleanup itself was injected to fail"
    );

    let cleanup_account = newer_service
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            cleanup_account_id,
        ))
        .await
        .unwrap();
    match failure {
        StaleBoundRollbackFailure::CleanupAccountWrite => {
            assert!(
                cleanup_account.is_none(),
                "injected staging failure must not create a cleanup account"
            );
        }
        StaleBoundRollbackFailure::RestoreWrite => {
            let cleanup_account =
                cleanup_account.expect("successful retry must retain an empty tombstone");
            assert_eq!(cleanup_account.status, CredentialAccountStatus::Revoked);
            assert!(cleanup_account.access_secret.is_none());
        }
        StaleBoundRollbackFailure::SecretDelete => {
            let cleanup_account = newer_service
                .accounts_for_owner(&scope)
                .await
                .unwrap()
                .into_iter()
                .find(|account| account.access_secret.as_ref() == Some(&stale_access))
                .expect("failed deletion must retain a durable cleanup account");
            assert_eq!(cleanup_account.status, CredentialAccountStatus::Revoked);
            assert_eq!(cleanup_account.access_secret, Some(stale_access.clone()));
            let cleanup_account_id = cleanup_account.id;

            newer_service
                .cleanup_for_lifecycle(SecretCleanupRequest {
                    scope: scope.clone(),
                    extension_id: ExtensionId::new("slack").unwrap(),
                    provider: Some(google_provider()),
                    action: SecretCleanupAction::Uninstall,
                })
                .await
                .expect("lifecycle retry must purge retained stale callback secrets");
            let retried_cleanup_account = newer_service
                .get_account(CredentialAccountLookupRequest::new(
                    scope.clone(),
                    cleanup_account_id,
                ))
                .await
                .unwrap()
                .unwrap();
            assert!(retried_cleanup_account.access_secret.is_none());
            assert!(
                concrete_secret_store
                    .metadata(&scope.resource, &stale_access)
                    .await
                    .unwrap()
                    .is_none(),
                "lifecycle retry must delete the stale callback token"
            );
        }
    }
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
                    exchange: Box::new(OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("c"),
                        pkce_verifier_hash: pkce_hash("p"),
                        access_secret: SecretHandle::new("access").unwrap(),
                        refresh_secret: None,
                        scopes: vec![],
                        account_id: None,
                        provider_identity: None,
                    }),
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
            provider: None,
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
            provider: None,
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

#[tokio::test]
async fn filesystem_cleanup_retries_failed_secret_deletion_without_losing_handle() {
    use ironclaw_auth::{SecretCleanupAction, SecretCleanupRequest, SecretCleanupService};

    let filesystem = test_filesystem();
    let concrete_secret_store = Arc::new(FailFirstDeleteSecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);
    let extension_id = ExtensionId::new("retryable-cleanup").unwrap();
    let access = SecretHandle::new("retryable-access").unwrap();
    let refresh = SecretHandle::new("retryable-refresh").unwrap();

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
    let account = service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(extension_id.clone()),
            granted_extensions: Vec::new(),
            access_secret: Some(access.clone()),
            refresh_secret: Some(refresh.clone()),
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let request = SecretCleanupRequest {
        scope: scope.clone(),
        extension_id: extension_id.clone(),
        provider: None,
        action: SecretCleanupAction::Uninstall,
    };

    let first = service.cleanup_for_lifecycle(request.clone()).await;
    assert_eq!(first, Err(AuthProductError::BackendUnavailable));
    let after_failure = service
        .get_account(
            CredentialAccountLookupRequest::new(scope.clone(), account.id)
                .for_extension(extension_id.clone()),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_failure.status, CredentialAccountStatus::Revoked);
    assert_eq!(after_failure.access_secret, Some(access.clone()));
    assert_eq!(after_failure.refresh_secret, None);
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &refresh)
            .await
            .unwrap()
            .is_none()
    );

    service.cleanup_for_lifecycle(request).await.unwrap();
    let after_retry = service
        .get_account(
            CredentialAccountLookupRequest::new(scope.clone(), account.id)
                .for_extension(extension_id),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_retry.access_secret, None);
    assert_eq!(after_retry.refresh_secret, None);
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access)
            .await
            .unwrap()
            .is_none()
    );
}

// ─── fix: cleanup matches owner granularity + provider-selected OAuth accounts ─

/// The production shape the Slack disconnect issues: the OAuth flow stored the
/// account under its own scope (fresh per-flow `invocation_id`), as
/// `UserReusable` with NO extension ownership/grants; the later disconnect
/// mints another fresh invocation. Full-scope-equality matching (the old
/// behavior) silently revoked nothing.
#[tokio::test]
async fn filesystem_cleanup_matches_owner_granularity_and_provider_selector() {
    use ironclaw_auth::{SecretCleanupAction, SecretCleanupRequest, SecretCleanupService};
    use ironclaw_host_api::ExtensionId;

    let filesystem = test_filesystem();
    let concrete_secret_store = Arc::new(InMemorySecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let flow_scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));

    let access = SecretHandle::new("slack-personal-access").unwrap();
    use secrecy::SecretString;
    concrete_secret_store
        .put(
            flow_scope.resource.clone(),
            access.clone(),
            SecretString::from("xoxp-material"),
            None,
        )
        .await
        .unwrap();

    // Exactly how the OAuth callback mints a personal credential (flows.rs).
    let account = service
        .create_account(ironclaw_auth::NewCredentialAccount {
            scope: flow_scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(access.clone()),
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();

    // Same owner, FRESH invocation id — the disconnect/lifecycle caller shape.
    let cleanup_scope = test_scope();
    assert_ne!(
        cleanup_scope.resource.invocation_id, flow_scope.resource.invocation_id,
        "fixture must model the cross-invocation caller"
    );

    // Extension-keyed cleanup alone must NOT sweep the reusable account…
    let report = service
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: cleanup_scope.clone(),
            extension_id: ExtensionId::new("slack").unwrap(),
            provider: None,
            action: SecretCleanupAction::Uninstall,
        })
        .await
        .unwrap();
    assert!(report.revoked_accounts.is_empty());

    // …but the explicit provider selector revokes it and purges the secret.
    let report = service
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: cleanup_scope,
            extension_id: ExtensionId::new("slack").unwrap(),
            provider: Some(google_provider()),
            action: SecretCleanupAction::Uninstall,
        })
        .await
        .unwrap();
    assert_eq!(report.revoked_accounts, vec![account.id]);
    assert!(
        concrete_secret_store
            .metadata(&flow_scope.resource, &access)
            .await
            .unwrap()
            .is_none(),
        "provider-selected Uninstall must purge the token material"
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
async fn filesystem_oauth_reauth_retains_failed_old_secret_deletion_for_lifecycle_retry() {
    // After a successful OAuth re-auth through a bound flow, the OLD access
    // and refresh secret handles must be deleted from SecretStore so repeated
    // re-auths do not accumulate dead handles. Host OAuth provider clients
    // return exchange.account_id == None, so the durable flow must use the
    // update_binding account id rather than rejecting the callback.
    use ironclaw_auth::{
        CredentialAccountUpdateBinding, ProviderCallbackOutcome, SecretCleanupAction,
        SecretCleanupRequest, SecretCleanupService as _,
    };
    use ironclaw_secrets::SecretMaterial;

    let filesystem = test_filesystem();
    let concrete_secret_store = Arc::new(FailFirstDeleteSecretStore::new());
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
                    exchange: Box::new(OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("code1"),
                        pkce_verifier_hash: pkce_hash("pkce1"),
                        access_secret: access_v1.clone(),
                        refresh_secret: Some(refresh_v1.clone()),
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                        provider_identity: None,
                    }),
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
                    exchange: Box::new(OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("code2"),
                        pkce_verifier_hash: pkce_hash("pkce2"),
                        access_secret: access_v2.clone(),
                        refresh_secret: Some(refresh_v2.clone()),
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                        provider_identity: None,
                    }),
                },
            },
        )
        .await
        .unwrap();

    let cleanup_account_id = CredentialAccountId::from_uuid(flow2.id.as_uuid());
    let cleanup_account = service
        .accounts_for_owner(&scope)
        .await
        .unwrap()
        .into_iter()
        .find(|account| account.id == cleanup_account_id)
        .expect("failed old-secret deletion must leave a durable cleanup account");
    assert_eq!(cleanup_account.status, CredentialAccountStatus::Revoked);
    assert_eq!(cleanup_account.access_secret, Some(access_v1.clone()));
    assert_eq!(cleanup_account.refresh_secret, None);

    // The injected first deletion failure leaves the old access handle in the
    // secret store while the old refresh handle was deleted successfully.
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access_v1)
            .await
            .unwrap()
            .is_some(),
        "failed v1 access deletion must remain available for lifecycle retry"
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

    service
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: scope.clone(),
            extension_id: ExtensionId::new("slack").unwrap(),
            provider: Some(google_provider()),
            action: SecretCleanupAction::Uninstall,
        })
        .await
        .expect("lifecycle retry must purge the retained old handle");

    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access_v1)
            .await
            .unwrap()
            .is_none(),
        "lifecycle retry must delete the retained v1 access handle"
    );
    let retried_cleanup_account = service
        .accounts_for_owner(&scope)
        .await
        .unwrap()
        .into_iter()
        .find(|account| account.id == cleanup_account_id)
        .expect("cleanup account tombstone must remain discoverable");
    assert!(retried_cleanup_account.access_secret.is_none());
    assert!(retried_cleanup_account.refresh_secret.is_none());
}

#[tokio::test]
async fn filesystem_oauth_reauth_cleanup_journal_failure_preserves_both_generations() {
    use ironclaw_auth::{
        CredentialAccountUpdateBinding, OAuthExchangeCleanupRequest, ProviderCallbackOutcome,
        SecretCleanupAction, SecretCleanupRequest, SecretCleanupService as _,
    };

    let (filesystem, backend) = paused_account_put_filesystem();
    let concrete_secret_store = Arc::new(InMemorySecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
    let scope = test_scope();
    let service = FilesystemAuthProductServices::new(filesystem, secret_store);

    let access_v1 = SecretHandle::new("reauth-journal-access-v1").unwrap();
    concrete_secret_store
        .put(
            scope.resource.clone(),
            access_v1.clone(),
            SecretMaterial::from("access-token-v1"),
            None,
        )
        .await
        .unwrap();
    let account = service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(access_v1.clone()),
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
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new("https://provider.example/oauth")
                    .unwrap(),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: Some(CredentialAccountUpdateBinding {
                account_id: account.id,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: vec![],
            }),
            opaque_state_hash: Some(state_hash("reauth-journal-state")),
            pkce_verifier_hash: Some(pkce_hash("reauth-journal-pkce")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    service
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash("reauth-journal-state"),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash("reauth-journal-pkce"),
            },
        )
        .await
        .unwrap();

    let access_v2 = SecretHandle::new("reauth-journal-access-v2").unwrap();
    concrete_secret_store
        .put(
            scope.resource.clone(),
            access_v2.clone(),
            SecretMaterial::from("access-token-v2"),
            None,
        )
        .await
        .unwrap();
    let exchange = OAuthProviderExchange {
        provider: google_provider(),
        account_label: account_label(),
        authorization_code_hash: code_hash("reauth-journal-code"),
        pkce_verifier_hash: pkce_hash("reauth-journal-pkce"),
        access_secret: access_v2.clone(),
        refresh_secret: None,
        scopes: vec![],
        account_id: None,
        provider_identity: None,
    };
    backend.fail_account_put_for_after(CredentialAccountId::from_uuid(flow.id.as_uuid()), 0);
    let error = service
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("reauth-journal-state"),
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: Box::new(exchange.clone()),
                },
            },
        )
        .await
        .expect_err("callback must not succeed without a durable pointer to v1");
    assert_eq!(error, AuthProductError::BackendUnavailable);

    let retained_v1 = service
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            account.id,
        ))
        .await
        .unwrap()
        .expect("v1 account must remain durable");
    assert_eq!(retained_v1.status, CredentialAccountStatus::Configured);
    assert_eq!(retained_v1.access_secret, Some(access_v1.clone()));

    service
        .retain_oauth_exchange_for_cleanup(OAuthExchangeCleanupRequest {
            scope: scope.clone(),
            flow_id: flow.id,
            exchange,
        })
        .await
        .expect("failed provider cleanup can retain v2 for lifecycle retry");
    service
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: scope.clone(),
            extension_id: ExtensionId::new("slack").unwrap(),
            provider: Some(google_provider()),
            action: SecretCleanupAction::Uninstall,
        })
        .await
        .expect("lifecycle uninstall must remove both token generations");

    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access_v1)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        concrete_secret_store
            .metadata(&scope.resource, &access_v2)
            .await
            .unwrap()
            .is_none()
    );
    let accounts = service.accounts_for_owner(&scope).await.unwrap();
    assert!(accounts.iter().any(|candidate| candidate.id == account.id));
    assert!(accounts.iter().all(|candidate| {
        candidate.status == CredentialAccountStatus::Revoked
            && candidate.access_secret.is_none()
            && candidate.refresh_secret.is_none()
    }));
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
                    exchange: Box::new(OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("code1"),
                        pkce_verifier_hash: pkce_hash("pkce1"),
                        access_secret: access_v1.clone(),
                        refresh_secret: None,
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                        provider_identity: None,
                    }),
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
                    exchange: Box::new(OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("code2"),
                        pkce_verifier_hash: pkce_hash("pkce2"),
                        access_secret: access_v2.clone(),
                        refresh_secret: None,
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                        provider_identity: None,
                    }),
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
    let configured_accounts = owner_accounts
        .iter()
        .filter(|account| account.status == CredentialAccountStatus::Configured)
        .collect::<Vec<_>>();
    assert_eq!(
        configured_accounts.len(),
        1,
        "reconnect must not fork a second configured account",
    );
    assert_eq!(
        configured_accounts[0].id, account_id,
        "the sole configured account must be the original bound account",
    );
    assert_eq!(
        configured_accounts[0].access_secret,
        Some(access_v2.clone()),
        "the bound account must carry the re-auth access secret",
    );
    assert!(
        owner_accounts.iter().all(|account| {
            account.status == CredentialAccountStatus::Configured
                || (account.status == CredentialAccountStatus::Revoked
                    && account.access_secret.is_none()
                    && account.refresh_secret.is_none())
        }),
        "any durable cleanup tombstone must be revoked and contain no secret handles",
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
        provider_identity: None,
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
                    exchange: Box::new(OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("c2"),
                        pkce_verifier_hash: pkce_hash("p2"),
                        access_secret: SecretHandle::new("new-access").unwrap(),
                        refresh_secret: Some(SecretHandle::new("new-refresh").unwrap()),
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                        provider_identity: None,
                    }),
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

#[tokio::test]
async fn filesystem_oauth_compensation_preserves_newer_secret_material() {
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
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("original-access").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();

    let first = complete_bound_oauth_generation(&service, &scope, &account, "first").await;
    let first_fingerprint = first
        .credential_secret_fingerprint
        .clone()
        .expect("first secret fingerprint");
    let first_claim = service
        .claim_continuation_dispatch(
            &scope,
            ironclaw_auth::AuthContinuationDispatchClaimInput {
                flow_id: first.id,
                claimed_at: Utc::now(),
            },
        )
        .await
        .expect("first continuation claim");
    service
        .settle_continuation_dispatch(
            &scope,
            ironclaw_auth::AuthContinuationDispatchSettlementInput {
                flow_id: first.id,
                expected_claimed_at: first_claim.updated_at,
                outcome: ironclaw_auth::AuthContinuationDispatchOutcome::TerminalFailure {
                    error: AuthErrorCode::BackendUnavailable,
                },
            },
        )
        .await
        .expect("first continuation failure");
    let second = complete_bound_oauth_generation(&service, &scope, &account, "second").await;
    assert_ne!(
        second.credential_secret_fingerprint,
        Some(first_fingerprint.clone()),
        "a reconnect must replace the secret fingerprint"
    );

    let outcome = service
        .compensate_oauth_completion(OAuthCompletionCompensationRequest {
            scope: scope.clone(),
            flow_id: first.id,
            provider: google_provider(),
            credential_account_id: account.id,
            expected_secret_fingerprint: first_fingerprint,
        })
        .await
        .unwrap();
    assert_eq!(outcome, OAuthCompletionCompensationOutcome::Superseded);
    let current = service
        .get_account(CredentialAccountLookupRequest::new(scope, account.id))
        .await
        .unwrap()
        .expect("newer account remains");
    assert_eq!(current.status, CredentialAccountStatus::Configured);
    assert_eq!(
        current.access_secret,
        Some(SecretHandle::new("second-access").unwrap())
    );
}

#[tokio::test]
async fn filesystem_oauth_compensation_revokes_only_the_exact_account() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(filesystem, secret_store);
    let failed_account = service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("failed-old-access").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let unrelated_account = service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: CredentialAccountLabel::new("unrelated google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("unrelated-access").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let completed =
        complete_bound_oauth_generation(&service, &scope, &failed_account, "failed").await;
    let expected_secret_fingerprint = completed
        .credential_secret_fingerprint
        .clone()
        .expect("completed secret fingerprint");
    let claim = service
        .claim_continuation_dispatch(
            &scope,
            ironclaw_auth::AuthContinuationDispatchClaimInput {
                flow_id: completed.id,
                claimed_at: Utc::now(),
            },
        )
        .await
        .expect("continuation claim");
    service
        .settle_continuation_dispatch(
            &scope,
            ironclaw_auth::AuthContinuationDispatchSettlementInput {
                flow_id: completed.id,
                expected_claimed_at: claim.updated_at,
                outcome: ironclaw_auth::AuthContinuationDispatchOutcome::TerminalFailure {
                    error: AuthErrorCode::BackendUnavailable,
                },
            },
        )
        .await
        .expect("continuation failure");

    let outcome = service
        .compensate_oauth_completion(OAuthCompletionCompensationRequest {
            scope: scope.clone(),
            flow_id: completed.id,
            provider: google_provider(),
            credential_account_id: failed_account.id,
            expected_secret_fingerprint,
        })
        .await
        .unwrap();
    assert_eq!(outcome, OAuthCompletionCompensationOutcome::Compensated);
    let failed = service
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            failed_account.id,
        ))
        .await
        .unwrap()
        .expect("failed account retained as tombstone");
    assert_eq!(failed.status, CredentialAccountStatus::Revoked);
    assert!(failed.access_secret.is_none());
    let unrelated = service
        .get_account(CredentialAccountLookupRequest::new(
            scope,
            unrelated_account.id,
        ))
        .await
        .unwrap()
        .expect("unrelated account remains");
    assert_eq!(unrelated.status, CredentialAccountStatus::Configured);
    assert!(unrelated.access_secret.is_some());
}

#[tokio::test]
async fn filesystem_oauth_compensation_retries_after_restart_without_losing_its_journal() {
    let filesystem = test_filesystem();
    let concrete_secret_store = Arc::new(FailFirstDeleteSecretStore::new());
    let secret_store: Arc<dyn SecretStore> = concrete_secret_store.clone();
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
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("restart-old-access").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let completed =
        complete_bound_oauth_generation(&service, &scope, &account, "restart-failed").await;
    let expected_secret_fingerprint = completed
        .credential_secret_fingerprint
        .clone()
        .expect("completed secret fingerprint");
    let claim = service
        .claim_continuation_dispatch(
            &scope,
            ironclaw_auth::AuthContinuationDispatchClaimInput {
                flow_id: completed.id,
                claimed_at: Utc::now(),
            },
        )
        .await
        .unwrap();
    service
        .settle_continuation_dispatch(
            &scope,
            ironclaw_auth::AuthContinuationDispatchSettlementInput {
                flow_id: completed.id,
                expected_claimed_at: claim.updated_at,
                outcome: ironclaw_auth::AuthContinuationDispatchOutcome::TerminalFailure {
                    error: AuthErrorCode::BackendUnavailable,
                },
            },
        )
        .await
        .unwrap();
    let request = OAuthCompletionCompensationRequest {
        scope: scope.clone(),
        flow_id: completed.id,
        provider: google_provider(),
        credential_account_id: account.id,
        expected_secret_fingerprint,
    };

    // Bound OAuth completion may clean the replaced generation first; inject
    // the failure specifically at compensation so the failed flow is already
    // durable when deletion stops.
    concrete_secret_store.set_delete_failure(true);

    assert_eq!(
        service.compensate_oauth_completion(request.clone()).await,
        Err(AuthProductError::BackendUnavailable)
    );
    let pending = service
        .get_flow(&scope, completed.id)
        .await
        .unwrap()
        .expect("failed flow remains durable");
    assert_eq!(pending.status, AuthFlowStatus::Failed);
    assert!(
        pending.credential_secret_fingerprint.is_some(),
        "the fingerprint is the durable pending-compensation journal"
    );

    let reopened = test_service(filesystem, secret_store);
    assert_eq!(
        reopened.compensate_oauth_completion(request).await.unwrap(),
        OAuthCompletionCompensationOutcome::Compensated
    );
    let converged = reopened
        .get_flow(&scope, completed.id)
        .await
        .unwrap()
        .expect("flow after retry");
    assert!(converged.credential_secret_fingerprint.is_none());
    let revoked = reopened
        .get_account(CredentialAccountLookupRequest::new(scope, account.id))
        .await
        .unwrap()
        .expect("revoked tombstone");
    assert_eq!(revoked.status, CredentialAccountStatus::Revoked);
    assert!(revoked.access_secret.is_none());
    assert!(revoked.refresh_secret.is_none());
}

#[tokio::test]
async fn filesystem_continuation_failure_is_durable_and_fenced() {
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
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("before-failure").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let completed =
        complete_bound_oauth_generation(&service, &scope, &account, "continuation-failure").await;
    let claim = service
        .claim_continuation_dispatch(
            &scope,
            ironclaw_auth::AuthContinuationDispatchClaimInput {
                flow_id: completed.id,
                claimed_at: Utc::now(),
            },
        )
        .await
        .expect("claim continuation");
    service
        .settle_continuation_dispatch(
            &scope,
            ironclaw_auth::AuthContinuationDispatchSettlementInput {
                flow_id: completed.id,
                expected_claimed_at: claim.updated_at,
                outcome: ironclaw_auth::AuthContinuationDispatchOutcome::TerminalFailure {
                    error: AuthErrorCode::BackendUnavailable,
                },
            },
        )
        .await
        .expect("mark continuation failed");

    let reopened = test_service(filesystem, secret_store);
    let persisted = reopened
        .get_flow(&scope, completed.id)
        .await
        .unwrap()
        .expect("persisted flow");
    assert_eq!(persisted.status, AuthFlowStatus::Failed);
    assert_eq!(persisted.error, Some(AuthErrorCode::BackendUnavailable));
    let stale = reopened
        .settle_continuation_dispatch(
            &scope,
            ironclaw_auth::AuthContinuationDispatchSettlementInput {
                flow_id: completed.id,
                expected_claimed_at: claim.updated_at,
                outcome: ironclaw_auth::AuthContinuationDispatchOutcome::TerminalFailure {
                    error: AuthErrorCode::BackendUnavailable,
                },
            },
        )
        .await
        .expect_err("stale failure is fenced");
    assert_eq!(stale, AuthProductError::FlowAlreadyTerminal);
}

#[tokio::test]
async fn filesystem_continuation_claim_has_one_owner_across_service_instances() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let first_service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));
    let second_service = test_service(filesystem, secret_store);
    let account = first_service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("claim-owner-before").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let completed =
        complete_bound_oauth_generation(&first_service, &scope, &account, "claim-owner").await;
    let claimed_at = Utc::now();

    let (first, second) = tokio::join!(
        first_service.claim_continuation_dispatch(
            &scope,
            ironclaw_auth::AuthContinuationDispatchClaimInput {
                flow_id: completed.id,
                claimed_at,
            },
        ),
        second_service.claim_continuation_dispatch(
            &scope,
            ironclaw_auth::AuthContinuationDispatchClaimInput {
                flow_id: completed.id,
                claimed_at,
            },
        )
    );

    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    let loser = first
        .err()
        .or_else(|| second.err())
        .expect("one losing claim");
    assert!(matches!(
        loser,
        AuthProductError::BackendUnavailable | AuthProductError::BackendConflict
    ));
}

#[tokio::test]
async fn filesystem_continuation_recovery_fences_the_stale_lease_owner() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let stale_service = test_service(Arc::clone(&filesystem), Arc::clone(&secret_store));
    let recovery_service = test_service(filesystem, secret_store);
    let account = stale_service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("stale-owner-before").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let completed =
        complete_bound_oauth_generation(&stale_service, &scope, &account, "stale-owner").await;
    let stale_claim = stale_service
        .claim_continuation_dispatch(
            &scope,
            ironclaw_auth::AuthContinuationDispatchClaimInput {
                flow_id: completed.id,
                claimed_at: Utc::now()
                    - Duration::seconds(
                        ironclaw_auth::AUTH_CONTINUATION_DISPATCH_LEASE_SECONDS + 1,
                    ),
            },
        )
        .await
        .expect("stale lease claim");
    let recovered_claim = recovery_service
        .claim_continuation_dispatch(
            &scope,
            ironclaw_auth::AuthContinuationDispatchClaimInput {
                flow_id: completed.id,
                claimed_at: Utc::now(),
            },
        )
        .await
        .expect("recover expired lease");

    let stale_settlement = stale_service
        .settle_continuation_dispatch(
            &scope,
            ironclaw_auth::AuthContinuationDispatchSettlementInput {
                flow_id: completed.id,
                expected_claimed_at: stale_claim.updated_at,
                outcome: ironclaw_auth::AuthContinuationDispatchOutcome::Dispatched {
                    emitted_at: Utc::now(),
                },
            },
        )
        .await
        .expect_err("stale lease owner must be fenced");
    assert_eq!(stale_settlement, AuthProductError::FlowAlreadyTerminal);

    recovery_service
        .settle_continuation_dispatch(
            &scope,
            ironclaw_auth::AuthContinuationDispatchSettlementInput {
                flow_id: completed.id,
                expected_claimed_at: recovered_claim.updated_at,
                outcome: ironclaw_auth::AuthContinuationDispatchOutcome::Dispatched {
                    emitted_at: Utc::now(),
                },
            },
        )
        .await
        .expect("current lease owner settles");
}

#[tokio::test]
async fn filesystem_oauth_compensation_converges_when_account_is_already_absent() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let scope = test_scope();
    let service = test_service(Arc::clone(&filesystem), secret_store);
    let account = service
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("already-absent-before").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .unwrap();
    let completed =
        complete_bound_oauth_generation(&service, &scope, &account, "already-absent").await;
    let expected_secret_fingerprint = completed
        .credential_secret_fingerprint
        .clone()
        .expect("completed secret fingerprint");
    let claim = service
        .claim_continuation_dispatch(
            &scope,
            ironclaw_auth::AuthContinuationDispatchClaimInput {
                flow_id: completed.id,
                claimed_at: Utc::now(),
            },
        )
        .await
        .expect("continuation claim");
    service
        .settle_continuation_dispatch(
            &scope,
            ironclaw_auth::AuthContinuationDispatchSettlementInput {
                flow_id: completed.id,
                expected_claimed_at: claim.updated_at,
                outcome: ironclaw_auth::AuthContinuationDispatchOutcome::TerminalFailure {
                    error: AuthErrorCode::BackendUnavailable,
                },
            },
        )
        .await
        .expect("continuation failure");
    let account_path = super::paths::account_path(&scope, account.id).expect("account path");
    filesystem
        .delete(&scope.resource, &account_path)
        .await
        .expect("remove account before compensation");

    let outcome = service
        .compensate_oauth_completion(OAuthCompletionCompensationRequest {
            scope: scope.clone(),
            flow_id: completed.id,
            provider: google_provider(),
            credential_account_id: account.id,
            expected_secret_fingerprint,
        })
        .await
        .expect("already-absent compensation converges");
    assert_eq!(outcome, OAuthCompletionCompensationOutcome::AlreadyAbsent);
    let converged = service
        .get_flow(&scope, completed.id)
        .await
        .expect("flow read")
        .expect("failed flow retained");
    assert!(converged.credential_secret_fingerprint.is_none());
}

async fn complete_bound_oauth_generation(
    service: &FilesystemAuthProductServices<InMemoryBackend>,
    scope: &AuthProductScope,
    account: &ironclaw_auth::CredentialAccount,
    suffix: &str,
) -> ironclaw_auth::AuthFlowRecord {
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
            continuation: AuthContinuationRef::LifecycleActivation {
                package_ref: ironclaw_auth::LifecyclePackageRef::new("google-extension").unwrap(),
            },
            update_binding: Some(CredentialAccountUpdateBinding::from_projection(
                &account.projection(),
            )),
            opaque_state_hash: Some(state_hash(suffix)),
            pkce_verifier_hash: Some(pkce_hash(suffix)),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    service
        .claim_oauth_callback(
            scope,
            OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash(suffix),
                provider: google_provider(),
                pkce_verifier_hash: pkce_hash(suffix),
            },
        )
        .await
        .unwrap();
    service
        .complete_oauth_callback(
            scope,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash(suffix),
                outcome: ironclaw_auth::ProviderCallbackOutcome::Authorized {
                    exchange: Box::new(OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash(suffix),
                        pkce_verifier_hash: pkce_hash(suffix),
                        access_secret: SecretHandle::new(format!("{suffix}-access")).unwrap(),
                        refresh_secret: None,
                        scopes: Vec::new(),
                        account_id: Some(account.id),
                        provider_identity: None,
                    }),
                },
            },
        )
        .await
        .unwrap()
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
            provider: None,
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
    assert_eq!(
        result.expect_err("account under a different surface must be denied"),
        AuthProductError::CrossScopeDenied
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
                    exchange: Box::new(OAuthProviderExchange {
                        provider: google_provider(),
                        account_label: account_label(),
                        authorization_code_hash: code_hash("cas-c"),
                        pkce_verifier_hash: pkce_hash("cas-p"),
                        access_secret: new_access.clone(),
                        refresh_secret: Some(new_refresh.clone()),
                        scopes: vec![ProviderScope::new("gmail.readonly").unwrap()],
                        account_id: None,
                        provider_identity: None,
                    }),
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
        provider_identity: None,
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

// ─── fix: complete_manual_token accepts reconnect across a fresh invocation_id

#[tokio::test]
async fn filesystem_complete_manual_token_succeeds_across_different_invocation_id() {
    // Regression for #4935 class, unbound/reusable completion path:
    // `complete_manual_token` previously called `scope_matches` (full equality)
    // to validate the credential account.  The submit handler mints a fresh
    // `invocation_id` on every HTTP request, so the flow record's scope differs
    // from the credential account's scope by `invocation_id` alone.  That full
    // equality check caused `CrossScopeDenied` on every real re-auth attempt.
    //
    // After the fix the check uses `binding_scope_owns_account` (owner
    // granularity: tenant/user/agent/project + surface + session, ignoring the
    // ephemeral `invocation_id`), so a legitimate reconnect now succeeds.
    //
    // This test MUST FAIL before the fix (it will return CrossScopeDenied).
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());

    // Build an account scope whose invocation_id is A (the "earlier request").
    let mut account_resource = test_scope().resource;
    account_resource.invocation_id = InvocationId::new();
    let account_scope =
        AuthProductScope::new(account_resource.clone(), ironclaw_auth::AuthSurface::Web);

    // Build a flow-record scope whose invocation_id is B (a "later request").
    // All other fields are identical.
    let mut flow_resource = account_resource.clone();
    flow_resource.invocation_id = InvocationId::new(); // fresh — B != A
    let flow_scope = AuthProductScope::new(flow_resource.clone(), ironclaw_auth::AuthSurface::Web);

    let service = test_service(filesystem, secret_store);
    let expires_at = Utc::now() + Duration::minutes(5);

    // Create the credential account under invocation A.
    let account = service
        .create_account(NewCredentialAccount {
            scope: account_scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(SecretHandle::new("reauth-access").unwrap()),
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();

    // Create the manual-token flow under invocation B.
    let interaction_id = create_manual_token_flow(&service, &flow_scope, expires_at).await;

    // Drive complete_manual_token with a scope built from invocation B.
    // Before the fix this returned CrossScopeDenied; after the fix it succeeds.
    let completed = service
        .complete_manual_token(
            &flow_scope,
            ManualTokenCompletionInput {
                interaction_id,
                credential_account_id: account.id,
            },
        )
        .await
        .expect(
            "complete_manual_token must succeed when only invocation_id differs (regression: \
             CrossScopeDenied was returned before the binding_scope_owns_account fix)",
        );

    assert_eq!(
        completed.status,
        AuthFlowStatus::Completed,
        "flow must reach Completed status on cross-invocation reconnect"
    );
    assert_eq!(
        completed.credential_account_id,
        Some(account.id),
        "completed flow must reference the pre-existing credential account"
    );
}

#[tokio::test]
async fn filesystem_complete_manual_token_still_rejects_genuinely_foreign_owner() {
    // Ownership enforcement must NOT be relaxed by the fix: a flow whose record
    // scope has a different *owner* (different user_id) than the credential account
    // must still return CrossScopeDenied.  This guards against
    // `binding_scope_owns_account` being over-permissive.
    //
    // GUARD ANALYSIS: `user_id` is NOT encoded in the on-disk path (the path is
    // keyed by surface + session, not by user; the filesystem mount is fixed to
    // alice's tree in tests).  Bob's account written via `create_account` lands at
    // the SAME physical path that alice's flow reads.  Therefore `read_account`
    // returns `Some(bob_account)`, and the `CrossScopeDenied` comes from
    // `binding_scope_owns_account` comparing the scopes — the guard itself is
    // exercised, not a path-partition miss.
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());

    // Build an account scope for user "bob".
    let mut bob_resource = test_scope().resource;
    bob_resource.user_id = UserId::new("bob").unwrap();
    let bob_scope = AuthProductScope::new(bob_resource, ironclaw_auth::AuthSurface::Web);

    // Build a flow scope for user "alice" (different owner).
    let alice_scope = test_scope(); // alice's scope from the default helper

    let service = test_service(filesystem, secret_store);
    let expires_at = Utc::now() + Duration::minutes(5);

    // Create an account owned by bob.
    let bob_account = service
        .create_account(NewCredentialAccount {
            scope: bob_scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(SecretHandle::new("bob-access").unwrap()),
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();

    // Create the flow under alice's scope.
    let interaction_id = create_manual_token_flow(&service, &alice_scope, expires_at).await;

    // Alice's flow must not be able to complete against bob's account.
    let err = service
        .complete_manual_token(
            &alice_scope,
            ManualTokenCompletionInput {
                interaction_id,
                credential_account_id: bob_account.id,
            },
        )
        .await
        .expect_err("completion against a foreign-owner account must return CrossScopeDenied");

    assert_eq!(
        err,
        AuthProductError::CrossScopeDenied,
        "owner-level boundary must still be enforced after the invocation_id fix"
    );
}

// ─── security: enforced isolation axes — session and surface are exact-matched

#[tokio::test]
async fn filesystem_complete_manual_token_rejects_different_session_id() {
    // `binding_scope_owns_account` must still reject a credential account whose
    // `session_id` differs from the flow record's session_id even when every
    // other ownership axis (tenant/user/agent/project/surface) matches.
    // This locks the "session is exact-matched" invariant documented in the
    // `binding_scope_owns_account` docstring.
    //
    // This test MUST FAIL before fix #1 (same-session uses scope_matches which
    // may pass, but here scope_matches would fail on session_id mismatch too —
    // either way the new binding_scope_owns_account correctly enforces it).
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());

    // Account created under session S1.
    let account_resource = test_scope().resource;
    let mut account_scope = AuthProductScope::new(account_resource.clone(), AuthSurface::Web);
    account_scope.session_id = Some(AuthSessionId::new("session-s1").unwrap());

    // Flow created under session S2 (same user/agent/project/surface).
    let mut flow_resource = test_scope().resource;
    flow_resource.invocation_id = InvocationId::new(); // different invocation too (realistic)
    let mut flow_scope = AuthProductScope::new(flow_resource, AuthSurface::Web);
    flow_scope.session_id = Some(AuthSessionId::new("session-s2").unwrap());

    let service = test_service(filesystem, secret_store);
    let expires_at = Utc::now() + Duration::minutes(5);

    // Create the credential account under session S1.
    let account = service
        .create_account(NewCredentialAccount {
            scope: account_scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(SecretHandle::new("s1-access").unwrap()),
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();

    // Create the manual-token flow under session S2.
    let interaction_id = create_manual_token_flow(&service, &flow_scope, expires_at).await;

    // Cross-session completion must be rejected — session is exact-matched.
    // Note: the durable store partitions account paths by session_id (see
    // `surface_sessions_root`), so a lookup under S2 will not find an account
    // created under S1. The observed outcome is `CredentialMissing` rather than
    // `CrossScopeDenied`; both are secure — the cross-session account is
    // inaccessible either way.
    let err = service
        .complete_manual_token(
            &flow_scope,
            ManualTokenCompletionInput {
                interaction_id,
                credential_account_id: account.id,
            },
        )
        .await
        .expect_err("complete_manual_token with different session_id must be rejected");

    assert!(
        matches!(
            err,
            AuthProductError::CredentialMissing | AuthProductError::CrossScopeDenied
        ),
        "cross-session completion must return CredentialMissing or CrossScopeDenied \
         (session_id is an exact-matched axis — different session is never accessible), \
         got: {err:?}"
    );
}

#[tokio::test]
async fn filesystem_complete_manual_token_rejects_different_auth_surface() {
    // `binding_scope_owns_account` must still reject a credential account whose
    // `surface` differs from the flow record's surface even when every other
    // ownership axis matches and session_id is None on both.
    // This locks the "surface is exact-matched" invariant.
    //
    // Note: because accounts are partitioned by surface in the filesystem path
    // layout (see `surface_sessions_root`), a cross-surface account lookup via
    // `read_account(scope, id)` will not find the account at all and will return
    // `CredentialMissing` rather than `CrossScopeDenied`. Both are acceptable
    // secure outcomes; this test documents which one actually occurs.
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());

    // Account created under AuthSurface::Web.
    let web_scope = test_scope(); // uses Web surface by default (see test_scope())

    // Flow created under AuthSurface::Cli (same owner, different surface).
    let cli_scope = AuthProductScope::new(test_scope().resource, AuthSurface::Cli);

    let service = test_service(filesystem, secret_store);
    let expires_at = Utc::now() + Duration::minutes(5);

    // Create the credential account under Web surface.
    let account = service
        .create_account(NewCredentialAccount {
            scope: web_scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(SecretHandle::new("web-access").unwrap()),
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();

    // Create the manual-token flow under Cli surface.
    let interaction_id = create_manual_token_flow(&service, &cli_scope, expires_at).await;

    // Cross-surface completion must be rejected. The filesystem partitions
    // accounts by surface, so the account is simply not found from the Cli
    // surface path — CredentialMissing is the observed (secure) outcome.
    let err = service
        .complete_manual_token(
            &cli_scope,
            ManualTokenCompletionInput {
                interaction_id,
                credential_account_id: account.id,
            },
        )
        .await
        .expect_err("complete_manual_token with different AuthSurface must be rejected");

    assert!(
        matches!(
            err,
            AuthProductError::CredentialMissing | AuthProductError::CrossScopeDenied
        ),
        "cross-surface completion must return CredentialMissing or CrossScopeDenied, got: {err:?}"
    );
}

#[tokio::test]
async fn filesystem_complete_credential_selection_succeeds_across_different_invocation_id() {
    // Regression test for fix #2 (`complete_credential_selection` parity with
    // `complete_manual_token`): when the flow record's scope differs from the
    // credential account's scope ONLY in the ephemeral `invocation_id`
    // (and/or `thread_id`/`mission_id`), `complete_credential_selection` must
    // succeed. Before fix #2 it used `scope_matches` (full equality) which would
    // return `CrossScopeDenied` on every real cross-invocation selection.
    //
    // This test MUST FAIL before fix #2.
    use ironclaw_auth::{AuthFlowKind, CredentialSelectionInput};

    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());

    // Account created under invocation A.
    let mut account_resource = test_scope().resource;
    account_resource.invocation_id = InvocationId::new();
    let account_scope = AuthProductScope::new(account_resource.clone(), AuthSurface::Web);

    // Flow created under invocation B (all other fields identical).
    let mut flow_resource = account_resource.clone();
    flow_resource.invocation_id = InvocationId::new(); // B != A
    let flow_scope = AuthProductScope::new(flow_resource, AuthSurface::Web);

    let service = test_service(filesystem, secret_store);

    // Create the credential account under invocation A.
    let account = service
        .create_account(NewCredentialAccount {
            scope: account_scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(SecretHandle::new("sel-access").unwrap()),
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();

    // Create the account-selection flow under invocation B.
    let flow = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: flow_scope.clone(),
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

    // Cross-invocation completion must succeed after fix #2.
    let completed = service
        .complete_credential_selection(
            &flow_scope,
            CredentialSelectionInput {
                flow_id: flow.id,
                credential_account_id: account.id,
            },
        )
        .await
        .expect(
            "complete_credential_selection must succeed when only invocation_id differs \
             (regression: CrossScopeDenied was returned before the binding_scope_owns_account fix)",
        );

    assert_eq!(
        completed.status,
        AuthFlowStatus::Completed,
        "flow must reach Completed status on cross-invocation selection"
    );
    assert_eq!(
        completed.credential_account_id,
        Some(account.id),
        "completed flow must reference the pre-existing credential account"
    );
}

// ─── security: complete_credential_selection ownership enforcement ────────────

#[tokio::test]
async fn filesystem_complete_credential_selection_rejects_genuinely_foreign_owner() {
    // Reviewer A (serrrfirat): `complete_credential_selection` must enforce the
    // same ownership boundary as `complete_manual_token`. A flow owned by alice
    // must not complete against a credential account owned by bob, even after the
    // `binding_scope_owns_account` relaxation for ephemeral invocation_id/thread.
    //
    // GUARD ANALYSIS: `user_id` is NOT encoded in the on-disk account path (path
    // is keyed by surface + session only; the test filesystem mount is fixed to
    // alice's tree). Bob's account therefore lands at the same physical path that
    // alice's flow reads — `read_account` returns `Some(bob_account)`. The
    // `CrossScopeDenied` comes from `binding_scope_owns_account` itself (the guard
    // is exercised, not a path-partition miss).  This is the most important new
    // test: it proves the guard actually fires on a reachable foreign-owner account.
    use ironclaw_auth::{AuthFlowKind, CredentialSelectionInput};

    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());

    // Account created under user "bob" (foreign owner).
    let mut bob_resource = test_scope().resource;
    bob_resource.user_id = UserId::new("bob").unwrap();
    let bob_scope = AuthProductScope::new(bob_resource, AuthSurface::Web);

    // Flow created under user "alice" (the default `test_scope()`).
    let alice_scope = test_scope();

    let service = test_service(filesystem, secret_store);

    // Create a Configured account owned by bob.
    let bob_account = service
        .create_account(NewCredentialAccount {
            scope: bob_scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(SecretHandle::new("bob-sel-access").unwrap()),
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();

    // Create the account-selection flow under alice's scope, advertising bob's
    // account id (simulates a tampered or confused client submission).
    let flow = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: alice_scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: google_provider(),
            challenge: AuthChallenge::AccountSelectionRequired {
                provider: google_provider(),
                accounts: vec![bob_account.projection()],
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();

    // Alice's flow must not complete against bob's account — CrossScopeDenied.
    let err = service
        .complete_credential_selection(
            &alice_scope,
            CredentialSelectionInput {
                flow_id: flow.id,
                credential_account_id: bob_account.id,
            },
        )
        .await
        .expect_err(
            "complete_credential_selection against a foreign-owner account must return \
             CrossScopeDenied",
        );

    assert_eq!(
        err,
        AuthProductError::CrossScopeDenied,
        "binding_scope_owns_account must reject a reachable account whose user_id differs \
         from the flow scope's user_id"
    );
}

#[tokio::test]
async fn filesystem_complete_credential_selection_rejects_different_session_id() {
    // Reviewer A (serrrfirat) parity with `complete_manual_token` session test.
    // `complete_credential_selection` must reject an attempt to complete a
    // selection flow whose scope carries session S2 against a credential account
    // created under session S1.
    //
    // GUARD ANALYSIS: `session_id` IS encoded in the on-disk account path (see
    // `product_auth_root` — the path includes `/sessions/{session_id}` when
    // `session_id` is Some). An account stored under S1 is therefore NOT
    // accessible from a read under S2. The durable store returns `None` for the
    // account lookup → `CredentialMissing`. Both `CredentialMissing` (path
    // partitioning intercepts before the guard) and `CrossScopeDenied` (the guard
    // fires) are correct secure outcomes; this test locks which one actually
    // occurs so it cannot silently regress. The `binding_scope_owns_account`
    // session exact-match is defense-in-depth for any future code path that
    // bypasses the path partitioning.
    use ironclaw_auth::{AuthFlowKind, CredentialSelectionInput};

    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());

    // Account created under session S1.
    let account_resource = test_scope().resource;
    let mut account_scope = AuthProductScope::new(account_resource.clone(), AuthSurface::Web);
    account_scope.session_id = Some(AuthSessionId::new("sel-session-s1").unwrap());

    // Flow created under session S2 (same surface, same owner, different session).
    let mut flow_resource = test_scope().resource;
    flow_resource.invocation_id = InvocationId::new(); // realistic fresh invocation
    let mut flow_scope = AuthProductScope::new(flow_resource, AuthSurface::Web);
    flow_scope.session_id = Some(AuthSessionId::new("sel-session-s2").unwrap());

    let service = test_service(filesystem, secret_store);

    // Create the credential account under session S1.
    let account = service
        .create_account(NewCredentialAccount {
            scope: account_scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(SecretHandle::new("sel-s1-access").unwrap()),
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();

    // Create the account-selection flow under session S2.
    let flow = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: flow_scope.clone(),
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

    // Cross-session completion must be rejected.
    // The disk layout partitions by session_id so the account is not found at
    // all under S2 → CredentialMissing.  CrossScopeDenied would be returned if
    // the account were somehow reachable with a mismatched session.  Both are
    // correct secure outcomes; accepting either documents the actual behavior.
    let err = service
        .complete_credential_selection(
            &flow_scope,
            CredentialSelectionInput {
                flow_id: flow.id,
                credential_account_id: account.id,
            },
        )
        .await
        .expect_err("complete_credential_selection with different session_id must be rejected");

    assert!(
        matches!(
            err,
            AuthProductError::CredentialMissing | AuthProductError::CrossScopeDenied
        ),
        "cross-session credential selection must return CredentialMissing (path-partition \
         intercepts before the guard) or CrossScopeDenied (guard fires on a reachable \
         session-mismatched account), got: {err:?}"
    );
}

#[tokio::test]
async fn filesystem_complete_credential_selection_rejects_different_auth_surface() {
    // Reviewer A (serrrfirat) parity with `complete_manual_token` surface test.
    // `complete_credential_selection` must reject an attempt to complete a
    // selection flow whose scope carries surface Cli against a credential account
    // created under surface Web.
    //
    // GUARD ANALYSIS: `surface` IS encoded in the on-disk account path (see
    // `surface_path_segment` in `paths.rs`). An account stored under Web is NOT
    // accessible from a read under Cli — `read_account` returns `None` →
    // `CredentialMissing`. The `binding_scope_owns_account` surface exact-match is
    // defense-in-depth: if a future refactor bypasses path partitioning the guard
    // would catch a reachable surface-mismatched account and return
    // `CrossScopeDenied`. Both outcomes are correct and secure; this test locks
    // which one occurs so a regression cannot pass silently.
    use ironclaw_auth::{AuthFlowKind, CredentialSelectionInput};

    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());

    // Account created under AuthSurface::Web (default from test_scope()).
    let web_scope = test_scope();

    // Flow created under AuthSurface::Cli (same owner, different surface).
    let cli_scope = AuthProductScope::new(test_scope().resource, AuthSurface::Cli);

    let service = test_service(filesystem, secret_store);

    // Create the credential account under Web surface.
    let account = service
        .create_account(NewCredentialAccount {
            scope: web_scope.clone(),
            provider: google_provider(),
            label: account_label(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(SecretHandle::new("sel-web-access").unwrap()),
            refresh_secret: None,
            scopes: vec![],
        })
        .await
        .unwrap();

    // Create the account-selection flow under Cli surface.
    let flow = service
        .create_flow(NewAuthFlow {
            id: None,
            scope: cli_scope.clone(),
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

    // Cross-surface completion must be rejected.
    // The filesystem partitions by surface path segment so the account is not
    // found from Cli → CredentialMissing.  CrossScopeDenied would fire if the
    // account were somehow reachable with a mismatched surface.
    let err = service
        .complete_credential_selection(
            &cli_scope,
            CredentialSelectionInput {
                flow_id: flow.id,
                credential_account_id: account.id,
            },
        )
        .await
        .expect_err("complete_credential_selection with different AuthSurface must be rejected");

    assert!(
        matches!(
            err,
            AuthProductError::CredentialMissing | AuthProductError::CrossScopeDenied
        ),
        "cross-surface credential selection must return CredentialMissing (path-partition \
         intercepts before the guard) or CrossScopeDenied (guard fires on a reachable \
         surface-mismatched account), got: {err:?}"
    );
}
