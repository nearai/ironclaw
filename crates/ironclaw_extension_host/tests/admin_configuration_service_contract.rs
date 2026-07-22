use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_host::{
    AdminConfigurationIdempotencyKey, AdminConfigurationService, AdminConfigurationServiceError,
    AdminConfigurationSubmittedValue, FilesystemAdminConfigurationStore,
};
use ironclaw_extensions::{
    AdminConfigurationField, AdminConfigurationGroupId, ExtensionAdminConfigurationDescriptor,
};
use ironclaw_filesystem::{InMemoryBackend, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, SecretHandle,
    TenantId, Timestamp, UserId, VirtualPath,
};
use ironclaw_secrets::{
    FilesystemSecretStore, SecretLease, SecretLeaseId, SecretMaterial, SecretMetadata, SecretStore,
    SecretStoreError,
};

#[tokio::test]
async fn save_before_install_stages_secrets_and_returns_a_redacted_group_view() {
    let (service, secrets) = service();
    let scope = sample_scope("tenant-a", "operator-a");

    let state = service
        .replace(
            &scope,
            &group_id(),
            &idempotency_key("save-1"),
            0,
            submitted("client-a", "super-secret"),
        )
        .await
        .expect("manifest catalog alone must be enough to save before installation");

    assert_eq!(state.revision, 1);
    assert!(state.complete);
    assert_eq!(state.fields[0].value.as_deref(), Some("client-a"));
    assert!(state.fields[0].provided);
    assert_eq!(
        state.fields[1].value, None,
        "secret material must be redacted"
    );
    assert!(state.fields[1].provided);

    let queried = service.get(&scope, &group_id()).await.unwrap();
    assert_eq!(
        queried, state,
        "query must preserve revision and redact secrets"
    );

    let shared_scope = scope.tenant_shared_managed_scope();
    let metadata = secrets.metadata_for_scope(&shared_scope).await.unwrap();
    assert_eq!(metadata.len(), 1);
    assert!(metadata[0].handle.as_str().contains("-r1-"));
    let lease = secrets
        .lease_once(&shared_scope, &metadata[0].handle)
        .await
        .unwrap();
    let material = secrets.consume(&shared_scope, lease.id).await.unwrap();
    assert_eq!(
        secrecy::ExposeSecret::expose_secret(&material),
        "super-secret"
    );
}

#[tokio::test]
async fn catalog_folds_equal_groups_and_rejects_descriptor_drift() {
    let store =
        FilesystemAdminConfigurationStore::new(scoped_admin_fs(Arc::new(InMemoryBackend::new())));
    let secrets = Arc::new(FilesystemSecretStore::ephemeral());
    let service = AdminConfigurationService::new(
        store,
        Arc::clone(&secrets),
        vec![descriptor(), descriptor()],
    )
    .unwrap();
    let groups = service
        .list(&sample_scope("tenant-a", "operator-a"))
        .await
        .unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].revision, 0);

    let mut conflicting = descriptor();
    conflicting.display_name = "Drifted provider".to_string();
    let store =
        FilesystemAdminConfigurationStore::new(scoped_admin_fs(Arc::new(InMemoryBackend::new())));
    let result = AdminConfigurationService::new(
        store,
        Arc::new(FilesystemSecretStore::ephemeral()),
        vec![descriptor(), conflicting],
    );
    let Err(error) = result else {
        panic!("conflicting group descriptors must fail closed");
    };
    assert_eq!(error, AdminConfigurationServiceError::DescriptorConflict);
}

#[tokio::test]
async fn blank_secret_preserves_the_previous_revision_handle() {
    let (service, secrets) = service();
    let scope = sample_scope("tenant-a", "operator-a");
    service
        .replace(
            &scope,
            &group_id(),
            &idempotency_key("save-1"),
            0,
            submitted("client-a", "super-secret"),
        )
        .await
        .unwrap();
    let before = secrets
        .metadata_for_scope(&scope.tenant_shared_managed_scope())
        .await
        .unwrap();

    let state = service
        .replace(
            &scope,
            &group_id(),
            &idempotency_key("save-2"),
            1,
            submitted("client-b", ""),
        )
        .await
        .unwrap();
    let after = secrets
        .metadata_for_scope(&scope.tenant_shared_managed_scope())
        .await
        .unwrap();

    assert_eq!(state.revision, 2);
    assert_eq!(state.fields[0].value.as_deref(), Some("client-b"));
    assert!(state.fields[1].provided);
    assert_eq!(
        after, before,
        "blank secret must not create or delete a handle"
    );
}

#[tokio::test]
async fn runtime_resolvers_follow_descriptor_kinds_and_return_effective_values() {
    let (service, _) = service();
    let scope = sample_scope("tenant-a", "operator-a");
    service
        .replace(
            &scope,
            &group_id(),
            &idempotency_key("save-1"),
            0,
            submitted("client-a", "super-secret"),
        )
        .await
        .unwrap();

    assert_eq!(
        service
            .non_secret_value(
                &scope,
                &group_id(),
                &SecretHandle::new("client_id").unwrap(),
            )
            .await
            .unwrap()
            .as_deref(),
        Some("client-a"),
    );
    let secret = service
        .secret_material(
            &scope,
            &group_id(),
            &SecretHandle::new("client_secret").unwrap(),
        )
        .await
        .unwrap()
        .expect("configured secret resolves");
    assert_eq!(
        secrecy::ExposeSecret::expose_secret(&secret),
        "super-secret",
    );

    assert_eq!(
        service
            .secret_material(
                &scope,
                &group_id(),
                &SecretHandle::new("client_id").unwrap(),
            )
            .await
            .unwrap_err(),
        AdminConfigurationServiceError::UnknownField,
    );
    assert_eq!(
        service
            .non_secret_value(
                &scope,
                &group_id(),
                &SecretHandle::new("client_secret").unwrap(),
            )
            .await
            .unwrap_err(),
        AdminConfigurationServiceError::UnknownField,
    );
}

#[tokio::test]
async fn exact_replay_does_not_stage_again_after_a_later_revision() {
    let (service, secrets) = service();
    let scope = sample_scope("tenant-a", "operator-a");
    let first_key = idempotency_key("save-1");
    let first = service
        .replace(
            &scope,
            &group_id(),
            &first_key,
            0,
            submitted("client-a", "secret-a"),
        )
        .await
        .unwrap();
    service
        .replace(
            &scope,
            &group_id(),
            &idempotency_key("save-2"),
            1,
            submitted("client-b", "secret-b"),
        )
        .await
        .unwrap();
    let before_replay = secrets
        .metadata_for_scope(&scope.tenant_shared_managed_scope())
        .await
        .unwrap();

    let replay = service
        .replace(
            &scope,
            &group_id(),
            &first_key,
            0,
            submitted("client-a", "secret-a"),
        )
        .await
        .unwrap();

    assert_eq!(replay, first);
    assert_eq!(
        secrets
            .metadata_for_scope(&scope.tenant_shared_managed_scope())
            .await
            .unwrap(),
        before_replay,
    );
}

#[tokio::test]
async fn idempotency_is_key_dominant_for_secrets_but_conflicts_on_nonsecret_changes() {
    let (service, secrets) = service();
    let scope = sample_scope("tenant-a", "operator-a");
    let key = idempotency_key("save-1");
    let first = service
        .replace(
            &scope,
            &group_id(),
            &key,
            0,
            submitted("client-a", "original-secret"),
        )
        .await
        .unwrap();

    let replay = service
        .replace(
            &scope,
            &group_id(),
            &key,
            0,
            submitted("client-a", "different-secret"),
        )
        .await
        .unwrap();
    assert_eq!(replay, first);

    let metadata = secrets
        .metadata_for_scope(&scope.tenant_shared_managed_scope())
        .await
        .unwrap();
    assert_eq!(metadata.len(), 1);
    let lease = secrets
        .lease_once(&scope.tenant_shared_managed_scope(), &metadata[0].handle)
        .await
        .unwrap();
    let material = secrets
        .consume(&scope.tenant_shared_managed_scope(), lease.id)
        .await
        .unwrap();
    assert_eq!(
        secrecy::ExposeSecret::expose_secret(&material),
        "original-secret",
        "reusing an action key must never apply a replacement secret body",
    );

    let conflict = service
        .replace(
            &scope,
            &group_id(),
            &key,
            0,
            submitted("client-b", "another-secret"),
        )
        .await
        .unwrap_err();
    assert_eq!(
        conflict,
        AdminConfigurationServiceError::IdempotencyConflict
    );
}

#[tokio::test]
async fn ambiguous_secret_put_is_cleaned_when_the_write_landed() {
    let store =
        FilesystemAdminConfigurationStore::new(scoped_admin_fs(Arc::new(InMemoryBackend::new())));
    let secrets = Arc::new(WriteThenFailSecretStore {
        inner: FilesystemSecretStore::ephemeral(),
    });
    let service =
        AdminConfigurationService::new(store, Arc::clone(&secrets), vec![descriptor()]).unwrap();
    let scope = sample_scope("tenant-a", "operator-a");

    let error = service
        .replace(
            &scope,
            &group_id(),
            &idempotency_key("ambiguous-put"),
            0,
            submitted("client-a", "secret-that-landed"),
        )
        .await
        .unwrap_err();
    assert_eq!(error, AdminConfigurationServiceError::Unavailable);
    assert!(
        secrets
            .metadata_for_scope(&scope.tenant_shared_managed_scope())
            .await
            .unwrap()
            .is_empty(),
        "a write-then-error secret must not remain unreferenced",
    );
}

#[tokio::test]
async fn unknown_duplicate_missing_and_oversized_values_fail_closed() {
    let (service, _) = service();
    let scope = sample_scope("tenant-a", "operator-a");

    let unknown_group = service
        .replace(
            &scope,
            &AdminConfigurationGroupId::new("unknown.group").unwrap(),
            &idempotency_key("unknown-group"),
            0,
            submitted("client-a", "secret"),
        )
        .await
        .unwrap_err();
    assert_eq!(unknown_group, AdminConfigurationServiceError::UnknownGroup);

    let unknown_handle = service
        .replace(
            &scope,
            &group_id(),
            &idempotency_key("unknown-handle"),
            0,
            vec![submitted_value("not_declared", "value")],
        )
        .await
        .unwrap_err();
    assert_eq!(unknown_handle, AdminConfigurationServiceError::UnknownField);

    let duplicate = service
        .replace(
            &scope,
            &group_id(),
            &idempotency_key("duplicate"),
            0,
            vec![
                submitted_value("client_id", "one"),
                submitted_value("client_id", "two"),
            ],
        )
        .await
        .unwrap_err();
    assert_eq!(duplicate, AdminConfigurationServiceError::DuplicateField);

    let missing = service
        .replace(
            &scope,
            &group_id(),
            &idempotency_key("missing"),
            0,
            Vec::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        missing,
        AdminConfigurationServiceError::MissingRequiredField
    );

    let oversized = service
        .replace(
            &scope,
            &group_id(),
            &idempotency_key("oversized"),
            0,
            submitted(&"x".repeat(16 * 1024 + 1), "secret"),
        )
        .await
        .unwrap_err();
    assert_eq!(oversized, AdminConfigurationServiceError::ValueTooLarge);
}

fn service() -> (
    AdminConfigurationService<InMemoryBackend, FilesystemSecretStore<InMemoryBackend>>,
    Arc<FilesystemSecretStore<InMemoryBackend>>,
) {
    let store =
        FilesystemAdminConfigurationStore::new(scoped_admin_fs(Arc::new(InMemoryBackend::new())));
    let secrets = Arc::new(FilesystemSecretStore::ephemeral());
    let service = AdminConfigurationService::new(store, Arc::clone(&secrets), vec![descriptor()])
        .expect("descriptor catalog");
    (service, secrets)
}

fn descriptor() -> ExtensionAdminConfigurationDescriptor {
    ExtensionAdminConfigurationDescriptor {
        group_id: group_id(),
        display_name: "Example provider".to_string(),
        description: "Deployment-owned provider credentials".to_string(),
        fields: vec![
            AdminConfigurationField {
                handle: SecretHandle::new("client_id").unwrap(),
                label: "Client ID".to_string(),
                secret: false,
                required: true,
            },
            AdminConfigurationField {
                handle: SecretHandle::new("client_secret").unwrap(),
                label: "Client secret".to_string(),
                secret: true,
                required: true,
            },
        ],
    }
}

fn submitted(client_id: &str, client_secret: &str) -> Vec<AdminConfigurationSubmittedValue> {
    vec![
        submitted_value("client_id", client_id),
        submitted_value("client_secret", client_secret),
    ]
}

fn submitted_value(handle: &str, value: &str) -> AdminConfigurationSubmittedValue {
    AdminConfigurationSubmittedValue {
        handle: SecretHandle::new(handle).unwrap(),
        value: SecretMaterial::from(value.to_string()),
    }
}

fn scoped_admin_fs<F>(backend: Arc<F>) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    let view = MountView::new(vec![MountGrant::new(
        MountAlias::new("/extension-admin-configuration").unwrap(),
        VirtualPath::new("/engine/tenants/test/admin-configuration").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    Arc::new(ScopedFilesystem::with_fixed_view(backend, view))
}

fn sample_scope(tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn group_id() -> AdminConfigurationGroupId {
    AdminConfigurationGroupId::new("vendor.example").unwrap()
}

fn idempotency_key(value: &str) -> AdminConfigurationIdempotencyKey {
    AdminConfigurationIdempotencyKey::new(value).unwrap()
}

struct WriteThenFailSecretStore {
    inner: FilesystemSecretStore<InMemoryBackend>,
}

#[async_trait]
impl SecretStore for WriteThenFailSecretStore {
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
        expires_at: Option<Timestamp>,
    ) -> Result<SecretMetadata, SecretStoreError> {
        self.inner.put(scope, handle, material, expires_at).await?;
        Err(SecretStoreError::StoreUnavailable {
            reason: "injected ambiguous failure".to_string(),
        })
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
