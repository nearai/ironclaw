use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_host_api::{ids::SecretHandle, resource::ResourceScope};
use ironclaw_secrets::{
    SecretLease, SecretLeaseId, SecretLeaseStatus, SecretMaterial, SecretMetadata,
    SecretStoreError, SecretStorePort,
};

/// One pre-seeded static secret plus honest `put_if_absent` create semantics.
///
/// Regression: `put_if_absent` used to answer `Ok(true)` ("created") for any
/// unknown handle while discarding the material, so a first-time
/// create-then-read flow (the web-app VAPID bootstrap) claimed success and
/// then failed the immediate read-back. Created secrets are now stored, and
/// leases remember which handle they were minted for.
pub(crate) struct StaticSecretStore {
    handle: SecretHandle,
    material: SecretMaterial,
    created: Mutex<HashMap<SecretHandle, SecretMaterial>>,
    leases: Mutex<HashMap<SecretLeaseId, SecretHandle>>,
}

impl StaticSecretStore {
    pub(crate) fn new(handle: SecretHandle, material: SecretMaterial) -> Self {
        Self {
            handle,
            material,
            created: Mutex::new(HashMap::new()),
            leases: Mutex::new(HashMap::new()),
        }
    }

    fn material_for(&self, handle: &SecretHandle) -> Option<SecretMaterial> {
        if handle == &self.handle {
            return Some(self.material.clone());
        }
        self.created
            .lock()
            .expect("created lock")
            .get(handle)
            .cloned()
    }
}

#[async_trait]
impl SecretStorePort for StaticSecretStore {
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
        _expires_at: Option<ironclaw_host_api::Timestamp>,
    ) -> Result<SecretMetadata, SecretStoreError> {
        if handle != self.handle {
            self.created
                .lock()
                .expect("created lock")
                .insert(handle.clone(), material);
        }
        Ok(SecretMetadata {
            scope,
            handle,
            expires_at: None,
        })
    }

    async fn put_versioned(
        &self,
        _scope: ResourceScope,
        _handle: SecretHandle,
        _material: SecretMaterial,
        _expires_at: Option<ironclaw_host_api::Timestamp>,
        _expected: ironclaw_secrets::SecretCasExpectation,
    ) -> Result<ironclaw_secrets::SecretCasWriteOutcome, SecretStoreError> {
        // Fail loud: this double serves one immutable secret for injection
        // tests; a compare-and-swap write reaching it is a wiring bug.
        Err(SecretStoreError::StoreUnavailable {
            reason: "static secret store double does not support versioned writes".to_string(),
        })
    }

    async fn read_versioned(
        &self,
        _scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<ironclaw_secrets::VersionedSecretMaterial>, SecretStoreError> {
        Ok(
            (handle == &self.handle).then(|| ironclaw_secrets::VersionedSecretMaterial {
                material: self.material.clone(),
                version: ironclaw_secrets::SecretVersion::from_backend(1),
            }),
        )
    }

    async fn put_if_absent(
        &self,
        _scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
        _expires_at: Option<ironclaw_host_api::Timestamp>,
    ) -> Result<bool, SecretStoreError> {
        if handle == self.handle {
            return Ok(false);
        }
        let mut created = self.created.lock().expect("created lock");
        if created.contains_key(&handle) {
            return Ok(false);
        }
        created.insert(handle, material);
        Ok(true)
    }

    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError> {
        Ok(self.material_for(handle).map(|_| SecretMetadata {
            scope: scope.clone(),
            handle: handle.clone(),
            expires_at: None,
        }))
    }

    async fn metadata_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretMetadata>, SecretStoreError> {
        let mut all = vec![SecretMetadata {
            scope: scope.clone(),
            handle: self.handle.clone(),
            expires_at: None,
        }];
        all.extend(
            self.created
                .lock()
                .expect("created lock")
                .keys()
                .map(|handle| SecretMetadata {
                    scope: scope.clone(),
                    handle: handle.clone(),
                    expires_at: None,
                }),
        );
        Ok(all)
    }

    async fn delete(
        &self,
        _scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError> {
        Ok(self
            .created
            .lock()
            .expect("created lock")
            .remove(handle)
            .is_some())
    }

    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError> {
        if self.material_for(handle).is_none() {
            return Err(SecretStoreError::UnknownSecret {
                scope: Box::new(scope.clone()),
                handle: handle.clone(),
            });
        }
        let lease_id = SecretLeaseId::new();
        self.leases
            .lock()
            .expect("leases lock")
            .insert(lease_id, handle.clone());
        Ok(SecretLease {
            id: lease_id,
            scope: scope.clone(),
            handle: handle.clone(),
            status: SecretLeaseStatus::Active,
        })
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError> {
        let handle = self
            .leases
            .lock()
            .expect("leases lock")
            .remove(&lease_id)
            // Pre-lease-tracking callers consumed the static secret only.
            .unwrap_or_else(|| self.handle.clone());
        self.material_for(&handle)
            .ok_or_else(|| SecretStoreError::UnknownSecret {
                scope: Box::new(scope.clone()),
                handle,
            })
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError> {
        let handle = self
            .leases
            .lock()
            .expect("leases lock")
            .remove(&lease_id)
            .unwrap_or_else(|| self.handle.clone());
        Ok(SecretLease {
            id: lease_id,
            scope: scope.clone(),
            handle,
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
