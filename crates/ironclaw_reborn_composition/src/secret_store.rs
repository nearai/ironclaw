use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::EncryptedBackend;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
use ironclaw_host_api::{ResourceScope, SecretHandle};
use ironclaw_secrets::{
    FilesystemSecretsStore, ScopedSecretsStoreAdapter, SecretError, SecretLease, SecretLeaseId,
    SecretMaterial, SecretMetadata, SecretStore, SecretStoreError, SecretsCrypto,
};

#[cfg(feature = "libsql")]
pub(crate) async fn build_libsql_secret_store(
    filesystem: Arc<LibSqlRootFilesystem>,
    master_key: SecretMaterial,
) -> Result<Arc<SharedSecretStore>, SecretError> {
    let crypto = Arc::new(SecretsCrypto::new(master_key)?);
    let filesystem = Arc::new(EncryptedBackend::new(filesystem, crypto));
    let backend = Arc::new(FilesystemSecretsStore::over_root(filesystem)?);
    backend.verify_can_decrypt_existing_secrets().await?;
    let store: Arc<dyn SecretStore> = Arc::new(ScopedSecretsStoreAdapter::new(backend));
    Ok(Arc::new(SharedSecretStore::new(store)))
}

#[cfg(feature = "postgres")]
pub(crate) async fn build_postgres_secret_store(
    filesystem: Arc<PostgresRootFilesystem>,
    master_key: SecretMaterial,
) -> Result<Arc<SharedSecretStore>, SecretError> {
    let crypto = Arc::new(SecretsCrypto::new(master_key)?);
    let filesystem = Arc::new(EncryptedBackend::new(filesystem, crypto));
    let backend = Arc::new(FilesystemSecretsStore::over_root(filesystem)?);
    backend.verify_can_decrypt_existing_secrets().await?;
    let store: Arc<dyn SecretStore> = Arc::new(ScopedSecretsStoreAdapter::new(backend));
    Ok(Arc::new(SharedSecretStore::new(store)))
}

// TODO(#3571): remove this adapter when the host-runtime services builder
// accepts `Arc<dyn SecretStore>` directly. Until then, this crate-private
// newtype keeps low-level `ironclaw_secrets` construction details out of the
// higher Reborn composition entrypoints.
#[derive(Clone)]
pub(crate) struct SharedSecretStore {
    inner: Arc<dyn SecretStore>,
}

impl SharedSecretStore {
    fn new(inner: Arc<dyn SecretStore>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl SecretStore for SharedSecretStore {
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
    ) -> Result<SecretMetadata, SecretStoreError> {
        self.inner.put(scope, handle, material).await
    }

    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError> {
        self.inner.metadata(scope, handle).await
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
