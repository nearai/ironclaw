//! Tenant-scoped secret service boundary for IronClaw Reborn.
//!
//! This crate stores and leases secret material behind opaque
//! [`SecretHandle`] values. It does not decide authorization, inject secrets into
//! runtimes, emit audit records, or expose raw values through metadata. Runtime
//! injection is not enforced until a higher-level obligation-handler/runtime
//! composition slice consumes these primitives.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use ironclaw_host_api::{
    AgentId, InvocationId, MissionId, ProjectId, ResourceScope, SecretHandle, TenantId, ThreadId,
    UserId,
};
pub use secrecy::SecretString as SecretMaterial;
use thiserror::Error;
use uuid::Uuid;

/// Opaque identifier for a one-shot secret lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SecretLeaseId(Uuid);

impl SecretLeaseId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SecretLeaseId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SecretLeaseId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Redacted metadata for a stored secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretMetadata {
    pub scope: ResourceScope,
    pub handle: SecretHandle,
}

/// Lease lifecycle for one secret access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretLeaseStatus {
    Active,
    Consumed,
    Revoked,
}

/// Metadata for a scoped one-shot secret lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretLease {
    pub id: SecretLeaseId,
    pub scope: ResourceScope,
    pub handle: SecretHandle,
    pub status: SecretLeaseStatus,
}

/// Secret service failures. Variants intentionally avoid secret material.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SecretStoreError {
    #[error("unknown secret {handle} for tenant/user scope")]
    UnknownSecret {
        scope: Box<ResourceScope>,
        handle: SecretHandle,
    },
    #[error("unknown secret lease {lease_id} for tenant/user scope")]
    UnknownLease {
        scope: Box<ResourceScope>,
        lease_id: SecretLeaseId,
    },
    #[error("secret lease {lease_id} was already consumed")]
    LeaseConsumed { lease_id: SecretLeaseId },
    #[error("secret lease {lease_id} was revoked")]
    LeaseRevoked { lease_id: SecretLeaseId },
    #[error("secret store state is unavailable: {reason}")]
    StoreUnavailable { reason: String },
}

impl SecretStoreError {
    pub fn is_unknown_secret(&self) -> bool {
        matches!(self, Self::UnknownSecret { .. })
    }

    pub fn is_unknown_lease(&self) -> bool {
        matches!(self, Self::UnknownLease { .. })
    }

    pub fn is_consumed(&self) -> bool {
        matches!(self, Self::LeaseConsumed { .. })
    }

    pub fn is_revoked(&self) -> bool {
        matches!(self, Self::LeaseRevoked { .. })
    }
}

/// Scoped secret store contract.
#[async_trait]
pub trait SecretStore: Send + Sync {
    /// Stores or replaces a secret under the caller's tenant/user/project scope and returns redacted metadata.
    ///
    /// Intended for trusted setup, composition, migration, or storage-code paths that are already
    /// allowed to manage secret material. This low-level primitive intentionally does not authorize
    /// arbitrary runtime/plugin callers.
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
    ) -> Result<SecretMetadata, SecretStoreError>;

    /// Returns redacted metadata for a secret without exposing material.
    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError>;

    /// Creates a one-shot lease for later secret consumption.
    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError>;

    /// Consumes an active one-shot lease and returns secret material exactly once.
    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError>;

    /// Revokes an active one-shot lease without returning material.
    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError>;

    /// Lists leases visible to the caller's tenant/user/project scope.
    async fn leases_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretLease>, SecretStoreError>;
}

/// In-memory secret store for contract tests and non-durable demos.
#[derive(Debug, Default)]
pub struct InMemorySecretStore {
    state: Mutex<SecretState>,
}

impl InMemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, SecretState>, SecretStoreError> {
        self.state
            .lock()
            .map_err(|error| SecretStoreError::StoreUnavailable {
                reason: error.to_string(),
            })
    }
}

#[derive(Debug, Default)]
struct SecretState {
    secrets: HashMap<SecretKey, SecretRecord>,
    leases: HashMap<SecretLeaseKey, LeaseRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SecretKey {
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    handle: SecretHandle,
}

impl SecretKey {
    fn new(scope: &ResourceScope, handle: &SecretHandle) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            handle: handle.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SecretLeaseKey {
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    mission_id: Option<MissionId>,
    thread_id: Option<ThreadId>,
    invocation_id: InvocationId,
    lease_id: SecretLeaseId,
}

impl SecretLeaseKey {
    fn new(scope: &ResourceScope, lease_id: SecretLeaseId) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            mission_id: scope.mission_id.clone(),
            thread_id: scope.thread_id.clone(),
            invocation_id: scope.invocation_id,
            lease_id,
        }
    }

    fn matches_scope(&self, scope: &ResourceScope) -> bool {
        self.tenant_id == scope.tenant_id
            && self.user_id == scope.user_id
            && self.agent_id == scope.agent_id
            && self.project_id == scope.project_id
            && self.mission_id == scope.mission_id
            && self.thread_id == scope.thread_id
            && self.invocation_id == scope.invocation_id
    }
}

#[derive(Debug, Clone)]
struct SecretRecord {
    metadata: SecretMetadata,
    material: SecretMaterial,
}

#[derive(Debug, Clone)]
struct LeaseRecord {
    lease: SecretLease,
    material: Option<SecretMaterial>,
}

#[async_trait]
impl SecretStore for InMemorySecretStore {
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
    ) -> Result<SecretMetadata, SecretStoreError> {
        let metadata = SecretMetadata {
            scope: scope.clone(),
            handle: handle.clone(),
        };
        let record = SecretRecord {
            metadata: metadata.clone(),
            material,
        };
        self.lock_state()?
            .secrets
            .insert(SecretKey::new(&scope, &handle), record);
        Ok(metadata)
    }

    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError> {
        Ok(self
            .lock_state()?
            .secrets
            .get(&SecretKey::new(scope, handle))
            .map(|record| record.metadata.clone()))
    }

    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError> {
        let mut state = self.lock_state()?;
        let secret = state
            .secrets
            .get(&SecretKey::new(scope, handle))
            .ok_or_else(|| SecretStoreError::UnknownSecret {
                scope: Box::new(scope.clone()),
                handle: handle.clone(),
            })?;
        let lease = SecretLease {
            id: SecretLeaseId::new(),
            scope: scope.clone(),
            handle: handle.clone(),
            status: SecretLeaseStatus::Active,
        };
        let record = LeaseRecord {
            lease: lease.clone(),
            material: Some(secret.material.clone()),
        };
        state
            .leases
            .insert(SecretLeaseKey::new(scope, lease.id), record);
        Ok(lease)
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError> {
        let mut state = self.lock_state()?;
        let key = SecretLeaseKey::new(scope, lease_id);
        let record = state
            .leases
            .get_mut(&key)
            .ok_or_else(|| SecretStoreError::UnknownLease {
                scope: Box::new(scope.clone()),
                lease_id,
            })?;
        match record.lease.status {
            SecretLeaseStatus::Active => {
                let Some(material) = record.material.take() else {
                    record.lease.status = SecretLeaseStatus::Consumed;
                    return Err(SecretStoreError::StoreUnavailable {
                        reason: "active lease material unavailable".to_string(),
                    });
                };
                record.lease.status = SecretLeaseStatus::Consumed;
                Ok(material)
            }
            SecretLeaseStatus::Consumed => Err(SecretStoreError::LeaseConsumed { lease_id }),
            SecretLeaseStatus::Revoked => Err(SecretStoreError::LeaseRevoked { lease_id }),
        }
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError> {
        let mut state = self.lock_state()?;
        let key = SecretLeaseKey::new(scope, lease_id);
        let record = state
            .leases
            .get_mut(&key)
            .ok_or_else(|| SecretStoreError::UnknownLease {
                scope: Box::new(scope.clone()),
                lease_id,
            })?;
        if record.lease.status == SecretLeaseStatus::Active {
            record.material = None;
            record.lease.status = SecretLeaseStatus::Revoked;
        }
        Ok(record.lease.clone())
    }

    async fn leases_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretLease>, SecretStoreError> {
        Ok(self
            .lock_state()?
            .leases
            .iter()
            .filter(|(key, _)| key.matches_scope(scope))
            .map(|(_, record)| record.lease.clone())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{
        InvocationId, MissionId, ProjectId, ResourceScope, SecretHandle, TenantId, ThreadId, UserId,
    };

    use crate::{InMemorySecretStore, SecretMaterial, SecretStore};

    #[tokio::test]
    async fn consumed_lease_record_drops_retained_material() {
        let store = InMemorySecretStore::new();
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();
        store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("super-secret"),
            )
            .await
            .unwrap();

        let lease = store.lease_once(&scope, &handle).await.unwrap();
        store.consume(&scope, lease.id).await.unwrap();

        let state = store.state.lock().unwrap();
        let leases_debug = format!("{:?}", state.leases);
        assert!(
            !leases_debug.contains("SecretBox"),
            "consumed lease records must not retain cloned secret material: {leases_debug}"
        );
    }

    #[tokio::test]
    async fn revoked_lease_record_drops_retained_material() {
        let store = InMemorySecretStore::new();
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();
        store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("super-secret"),
            )
            .await
            .unwrap();

        let lease = store.lease_once(&scope, &handle).await.unwrap();
        store.revoke(&scope, lease.id).await.unwrap();

        let state = store.state.lock().unwrap();
        let leases_debug = format!("{:?}", state.leases);
        assert!(
            !leases_debug.contains("SecretBox"),
            "revoked lease records must not retain cloned secret material: {leases_debug}"
        );
    }

    fn sample_scope(tenant: &str, user: &str) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(tenant).unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: None,
            project_id: Some(ProjectId::new("project-a").unwrap()),
            mission_id: Some(MissionId::new("mission-a").unwrap()),
            thread_id: Some(ThreadId::new("thread-a").unwrap()),
            invocation_id: InvocationId::new(),
        }
    }
}
