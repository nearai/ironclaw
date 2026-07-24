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
//! We build a fresh per-target-user `SecretStore` on demand from the
//! shared root filesystem + the SAME `SecretsCrypto` the runtime's own store
//! uses (so material written here decrypts under the user's own store and vice
//! versa). Construction is cheap (an `Arc` clone + a fixed `MountView`).

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{InvocationId, ResourceScope, SecretHandle, TenantId, UserId};
use ironclaw_secrets::{
    SecretMaterial, SecretMetadata, SecretStore, SecretStoreError, SecretStorePort, SecretsCrypto,
};

/// Admin provisioning of per-user secrets for an arbitrary target `(tenant,
/// user)`. Implemented over the filesystem secret substrate; a `dyn` port so
/// the runtime can retain it without carrying the backend generic.
#[async_trait]
pub(crate) trait AdminSecretProvisioner: Send + Sync {
    async fn list(
        &self,
        tenant: &TenantId,
        user: &UserId,
    ) -> Result<Vec<SecretMetadata>, SecretStoreError>;

    async fn put(
        &self,
        tenant: &TenantId,
        user: &UserId,
        handle: SecretHandle,
        material: SecretMaterial,
    ) -> Result<SecretMetadata, SecretStoreError>;

    async fn delete(
        &self,
        tenant: &TenantId,
        user: &UserId,
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
}

impl<F> FilesystemAdminSecretProvisioner<F>
where
    F: RootFilesystem + 'static,
{
    pub(crate) fn new(root: Arc<F>, crypto: Arc<SecretsCrypto>) -> Self {
        Self { root, crypto }
    }

    /// Build a target-user secret store plus the matching `ResourceScope`. The
    /// `MountView` (via [`invocation_mount_view`](crate::invocation_mount_view))
    /// resolves `/secrets` → `/tenants/{tenant}/users/{user}/secrets` for path
    /// isolation; the scope carries the same `(tenant, user)` so the store's
    /// `same_scope_owner` check matches between put and list/delete.
    fn store_for(
        &self,
        tenant: &TenantId,
        user: &UserId,
    ) -> Result<(SecretStore<F>, ResourceScope), SecretStoreError> {
        let scope = ResourceScope {
            tenant_id: tenant.clone(),
            user_id: user.clone(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
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
            SecretStore::new(filesystem, Arc::clone(&self.crypto)),
            scope,
        ))
    }
}

#[async_trait]
impl<F> AdminSecretProvisioner for FilesystemAdminSecretProvisioner<F>
where
    F: RootFilesystem + 'static,
{
    async fn list(
        &self,
        tenant: &TenantId,
        user: &UserId,
    ) -> Result<Vec<SecretMetadata>, SecretStoreError> {
        let (store, scope) = self.store_for(tenant, user)?;
        store.metadata_for_scope(&scope).await
    }

    async fn put(
        &self,
        tenant: &TenantId,
        user: &UserId,
        handle: SecretHandle,
        material: SecretMaterial,
    ) -> Result<SecretMetadata, SecretStoreError> {
        let (store, scope) = self.store_for(tenant, user)?;
        store.put(scope, handle, material, None).await
    }

    async fn delete(
        &self,
        tenant: &TenantId,
        user: &UserId,
        handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError> {
        let (store, scope) = self.store_for(tenant, user)?;
        store.delete(&scope, handle).await
    }
}
