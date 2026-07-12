//! Admin-scoped per-user secret provisioning.
//!
//! The `ironclaw_secrets` store isolates tenant/user by the caller's
//! `MountView`, not the `ResourceScope` argument (`secret_owner_alias` only
//! encodes agent/project into the path). So provisioning a secret for an
//! *arbitrary target user* — the admin use case — requires a store whose
//! `MountView` points at that target user's `/secrets` subtree. This is the
//! "explicit admin-scoped API" the `ironclaw_secrets` guardrails anticipate
//! ("no global handle lookup unless an explicit admin-scoped API is introduced
//! later").
//!
//! We build a fresh per-target-user `FilesystemSecretStore` on demand from the
//! shared root filesystem + the SAME `SecretsCrypto` the runtime's own store
//! uses (so material written here decrypts under the user's own store and vice
//! versa). Construction is cheap (an `Arc` clone + a fixed `MountView`).

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{InvocationId, ResourceScope, SecretHandle};
use ironclaw_product_workflow::AdminUserSecretScope;
use ironclaw_secrets::{
    FilesystemSecretStore, SecretMaterial, SecretMetadata, SecretStore, SecretStoreError,
    SecretsCrypto,
};

/// Admin provisioning of per-user secrets for an arbitrary target `(tenant,
/// user)`. Implemented over the filesystem secret substrate; a `dyn` port so
/// the runtime can retain it without carrying the backend generic.
#[async_trait]
pub(crate) trait AdminSecretProvisioner: Send + Sync {
    async fn list(
        &self,
        scope: &AdminUserSecretScope,
    ) -> Result<Vec<SecretMetadata>, SecretStoreError>;

    async fn put(
        &self,
        scope: &AdminUserSecretScope,
        handle: SecretHandle,
        material: SecretMaterial,
    ) -> Result<SecretMetadata, SecretStoreError>;

    async fn delete(
        &self,
        scope: &AdminUserSecretScope,
        handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError>;
}

/// Filesystem-backed admin secret provisioner: holds the shared raw root + the
/// shared crypto and mints a per-target-user store per call.
pub(crate) struct FilesystemAdminSecretProvisioner<F>
where
    F: RootFilesystem + 'static,
{
    root: Arc<F>,
    crypto: Arc<SecretsCrypto>,
    operation_lock: tokio::sync::Mutex<()>,
}

impl<F> FilesystemAdminSecretProvisioner<F>
where
    F: RootFilesystem + 'static,
{
    pub(crate) fn new(root: Arc<F>, crypto: Arc<SecretsCrypto>) -> Self {
        Self {
            root,
            crypto,
            operation_lock: tokio::sync::Mutex::new(()),
        }
    }

    fn resource_scope(admin_scope: &AdminUserSecretScope) -> ResourceScope {
        ResourceScope {
            tenant_id: admin_scope.tenant_id.clone(),
            user_id: admin_scope.user_id.clone(),
            agent_id: admin_scope.agent_id.clone(),
            project_id: admin_scope.project_id.clone(),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn legacy_resource_scope(admin_scope: &AdminUserSecretScope) -> ResourceScope {
        ResourceScope {
            agent_id: None,
            project_id: None,
            ..Self::resource_scope(admin_scope)
        }
    }

    /// Build a target-user secret store plus the matching `ResourceScope`. The
    /// `MountView` (via [`invocation_mount_view`](crate::invocation_mount_view))
    /// resolves `/secrets` → `/tenants/{tenant}/users/{user}/secrets` for path
    /// isolation; the scope carries the same `(tenant, user)` so the store's
    /// `same_scope_owner` check matches between put and list/delete. The
    /// optional agent/project pair is the trusted runtime owner scope stamped
    /// onto the authenticated WebUI caller, so admin writes land where
    /// capability preflight reads for that same deployment.
    fn store_for(
        &self,
        admin_scope: &AdminUserSecretScope,
    ) -> Result<(FilesystemSecretStore<F>, ResourceScope), SecretStoreError> {
        self.store_for_resource_scope(Self::resource_scope(admin_scope))
    }

    fn legacy_store_for(
        &self,
        admin_scope: &AdminUserSecretScope,
    ) -> Result<(FilesystemSecretStore<F>, ResourceScope), SecretStoreError> {
        self.store_for_resource_scope(Self::legacy_resource_scope(admin_scope))
    }

    fn store_for_resource_scope(
        &self,
        scope: ResourceScope,
    ) -> Result<(FilesystemSecretStore<F>, ResourceScope), SecretStoreError> {
        let view = crate::invocation_mount_view(&scope).map_err(|error| {
            SecretStoreError::StoreUnavailable {
                reason: format!("admin secret mount view: {error}"),
            }
        })?;
        let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::clone(&self.root),
            view,
        ));
        Ok((
            FilesystemSecretStore::new(filesystem, Arc::clone(&self.crypto)),
            scope,
        ))
    }

    async fn metadata_with_legacy(
        &self,
        admin_scope: &AdminUserSecretScope,
        mut metadata: Vec<SecretMetadata>,
    ) -> Result<Vec<SecretMetadata>, SecretStoreError> {
        if admin_scope.agent_id.is_none() && admin_scope.project_id.is_none() {
            return Ok(metadata);
        }
        let (legacy_store, legacy_scope) = self.legacy_store_for(admin_scope)?;
        for legacy_metadata in legacy_store.metadata_for_scope(&legacy_scope).await? {
            if !metadata
                .iter()
                .any(|current| current.handle == legacy_metadata.handle)
            {
                metadata.push(legacy_metadata);
            }
        }
        Ok(metadata)
    }
}

#[async_trait]
impl<F> AdminSecretProvisioner for FilesystemAdminSecretProvisioner<F>
where
    F: RootFilesystem + 'static,
{
    async fn list(
        &self,
        scope: &AdminUserSecretScope,
    ) -> Result<Vec<SecretMetadata>, SecretStoreError> {
        let _guard = self.operation_lock.lock().await;
        let (store, resource_scope) = self.store_for(scope)?;
        let metadata = store.metadata_for_scope(&resource_scope).await?;
        self.metadata_with_legacy(scope, metadata).await
    }

    async fn put(
        &self,
        scope: &AdminUserSecretScope,
        handle: SecretHandle,
        material: SecretMaterial,
    ) -> Result<SecretMetadata, SecretStoreError> {
        let _guard = self.operation_lock.lock().await;
        let (store, resource_scope) = self.store_for(scope)?;
        let metadata = store
            .put(resource_scope, handle.clone(), material, None)
            .await?;
        if scope.agent_id.is_some() || scope.project_id.is_some() {
            let (legacy_store, legacy_scope) = self.legacy_store_for(scope)?;
            legacy_store.delete(&legacy_scope, &handle).await?;
        }
        Ok(metadata)
    }

    async fn delete(
        &self,
        scope: &AdminUserSecretScope,
        handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError> {
        let _guard = self.operation_lock.lock().await;
        let (store, resource_scope) = self.store_for(scope)?;
        let deleted = store.delete(&resource_scope, handle).await?;
        if scope.agent_id.is_none() && scope.project_id.is_none() {
            return Ok(deleted);
        }
        let (legacy_store, legacy_scope) = self.legacy_store_for(scope)?;
        let legacy_deleted = legacy_store.delete(&legacy_scope, handle).await?;
        Ok(deleted || legacy_deleted)
    }
}
