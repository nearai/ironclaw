use async_trait::async_trait;
use ironclaw_host_api::{ResourceScope, SecretHandle};
use ironclaw_secrets::{
    SecretLease, SecretLeaseId, SecretLeaseStatus, SecretMaterial, SecretMetadata,
    SecretStoreError, SecretStorePort,
};

pub(crate) struct StaticSecretStore {
    handle: SecretHandle,
    material: SecretMaterial,
}

impl StaticSecretStore {
    pub(crate) fn new(handle: SecretHandle, material: SecretMaterial) -> Self {
        Self { handle, material }
    }
}

#[async_trait]
impl SecretStorePort for StaticSecretStore {
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        _material: SecretMaterial,
        _expires_at: Option<ironclaw_host_api::Timestamp>,
    ) -> Result<SecretMetadata, SecretStoreError> {
        Ok(SecretMetadata {
            scope,
            handle,
            expires_at: None,
        })
    }

    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError> {
        Ok((handle == &self.handle).then(|| SecretMetadata {
            scope: scope.clone(),
            handle: handle.clone(),
            expires_at: None,
        }))
    }

    async fn metadata_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretMetadata>, SecretStoreError> {
        Ok(vec![SecretMetadata {
            scope: scope.clone(),
            handle: self.handle.clone(),
            expires_at: None,
        }])
    }

    async fn delete(
        &self,
        _scope: &ResourceScope,
        _handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError> {
        Ok(false)
    }

    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError> {
        if handle != &self.handle {
            return Err(SecretStoreError::UnknownSecret {
                scope: Box::new(scope.clone()),
                handle: handle.clone(),
            });
        }
        Ok(SecretLease {
            id: SecretLeaseId::new(),
            scope: scope.clone(),
            handle: handle.clone(),
            status: SecretLeaseStatus::Active,
        })
    }

    async fn consume(
        &self,
        _scope: &ResourceScope,
        _lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError> {
        Ok(self.material.clone())
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError> {
        Ok(SecretLease {
            id: lease_id,
            scope: scope.clone(),
            handle: self.handle.clone(),
            status: SecretLeaseStatus::Revoked,
        })
    }

    async fn leases_for_scope(
        &self,
        _scope: &ResourceScope,
    ) -> Result<Vec<SecretLease>, SecretStoreError> {
        Ok(Vec::new())
    }
}
