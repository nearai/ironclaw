//! Capability authorization contracts for IronClaw Reborn.
//!
//! `ironclaw_authorization` evaluates authority-bearing host API contracts. It
//! does not execute capabilities, reserve resources, prompt users, or reach into
//! runtime internals. The first slices implement grant- and lease-backed gates
//! for capability dispatch.

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{FileType, FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityGrant, CapabilityGrantId, Decision, DenyReason, EffectKind,
    ExecutionContext, HostApiError, InvocationFingerprint, InvocationId, Principal,
    ResourceEstimate, ResourceScope, TenantId, UserId, VirtualPath,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Authorizes a capability dispatch request against an execution context.
#[async_trait]
pub trait CapabilityDispatchAuthorizer: Send + Sync {
    /// Returns `Allow` only when the context has matching authority for the capability and declared effects; otherwise fails closed.
    async fn authorize_dispatch(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
    ) -> Decision;

    /// Returns `Allow` only when dispatch authority and `SpawnProcess` authority are both present for the target capability.
    async fn authorize_spawn(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        Decision::Deny {
            reason: DenyReason::MissingGrant,
        }
    }
}

/// Grant-backed capability dispatch authorizer.
#[derive(Debug, Clone, Copy, Default)]
pub struct GrantAuthorizer;

impl GrantAuthorizer {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl CapabilityDispatchAuthorizer for GrantAuthorizer {
    async fn authorize_dispatch(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        authorize_from_grants(context, descriptor, context.grants.grants.iter())
    }

    async fn authorize_spawn(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        authorize_from_grants(
            context,
            &spawn_descriptor(descriptor),
            context.grants.grants.iter(),
        )
    }
}

/// Capability lease issued from an approved request or policy workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityLease {
    pub scope: ResourceScope,
    pub grant: CapabilityGrant,
    pub invocation_fingerprint: Option<InvocationFingerprint>,
    pub status: CapabilityLeaseStatus,
}

impl CapabilityLease {
    pub fn new(scope: ResourceScope, grant: CapabilityGrant) -> Self {
        Self {
            scope,
            grant,
            invocation_fingerprint: None,
            status: CapabilityLeaseStatus::Active,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityLeaseStatus {
    Active,
    Claimed,
    Consumed,
    Revoked,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CapabilityLeaseError {
    #[error("unknown capability lease {lease_id}")]
    UnknownLease { lease_id: CapabilityGrantId },
    #[error("capability lease {lease_id} is expired")]
    ExpiredLease { lease_id: CapabilityGrantId },
    #[error("capability lease {lease_id} has no remaining invocations")]
    ExhaustedLease { lease_id: CapabilityGrantId },
    #[error("capability lease {lease_id} fingerprint does not match")]
    FingerprintMismatch { lease_id: CapabilityGrantId },
    #[error("capability lease {lease_id} is not active: {status:?}")]
    InactiveLease {
        lease_id: CapabilityGrantId,
        status: CapabilityLeaseStatus,
    },
    #[error("capability lease persistence error: {reason}")]
    Persistence { reason: String },
}

/// Store of active/revoked capability leases.
#[async_trait]
pub trait CapabilityLeaseStore: Send + Sync {
    /// Persists a scoped lease before any approval record is marked approved.
    async fn issue(&self, lease: CapabilityLease) -> Result<CapabilityLease, CapabilityLeaseError>;

    /// Revokes a lease only within the exact tenant/user/invocation scope that owns it.
    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError>;

    /// Loads a lease by exact scope and ID; wrong-scope lookups must behave as unknown.
    async fn get(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Option<CapabilityLease>;

    /// Atomically marks an active fingerprinted lease as claimed after matching the replay fingerprint.
    async fn claim(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
        invocation_fingerprint: &InvocationFingerprint,
    ) -> Result<CapabilityLease, CapabilityLeaseError>;

    /// Consumes or decrements an active/claimed lease after successful dispatch.
    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError>;

    /// Lists leases visible to the tenant/user scope without exposing cross-tenant records.
    async fn leases_for_scope(&self, scope: &ResourceScope) -> Vec<CapabilityLease>;

    /// Returns active, unexpired, unexhausted leases for the exact invocation context.
    async fn active_leases_for_context(&self, context: &ExecutionContext) -> Vec<CapabilityLease>;

    /// Converts only non-fingerprinted active leases into ambient grants for authorization.
    async fn active_grants_for_context(&self, context: &ExecutionContext) -> Vec<CapabilityGrant> {
        self.active_leases_for_context(context)
            .await
            .into_iter()
            .filter(|lease| lease.invocation_fingerprint.is_none())
            .map(|lease| lease.grant)
            .collect()
    }
}

/// In-memory lease store for early Reborn flows and tests.
#[derive(Debug, Default)]
pub struct InMemoryCapabilityLeaseStore {
    leases: Mutex<HashMap<CapabilityLeaseKey, CapabilityLease>>,
}

impl InMemoryCapabilityLeaseStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn leases_guard(&self) -> MutexGuard<'_, HashMap<CapabilityLeaseKey, CapabilityLease>> {
        self.leases
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl CapabilityLeaseStore for InMemoryCapabilityLeaseStore {
    async fn issue(&self, lease: CapabilityLease) -> Result<CapabilityLease, CapabilityLeaseError> {
        self.leases_guard().insert(
            CapabilityLeaseKey::new(&lease.scope, lease.grant.id),
            lease.clone(),
        );
        Ok(lease)
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        let mut leases = self.leases_guard();
        let lease = leases
            .get_mut(&CapabilityLeaseKey::new(scope, lease_id))
            .ok_or(CapabilityLeaseError::UnknownLease { lease_id })?;
        lease.status = CapabilityLeaseStatus::Revoked;
        Ok(lease.clone())
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Option<CapabilityLease> {
        self.leases_guard()
            .get(&CapabilityLeaseKey::new(scope, lease_id))
            .cloned()
    }

    async fn claim(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
        invocation_fingerprint: &InvocationFingerprint,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        let mut leases = self.leases_guard();
        let lease = leases
            .get_mut(&CapabilityLeaseKey::new(scope, lease_id))
            .ok_or(CapabilityLeaseError::UnknownLease { lease_id })?;

        ensure_claimable(lease, invocation_fingerprint)?;
        lease.status = CapabilityLeaseStatus::Claimed;
        Ok(lease.clone())
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        let mut leases = self.leases_guard();
        let lease = leases
            .get_mut(&CapabilityLeaseKey::new(scope, lease_id))
            .ok_or(CapabilityLeaseError::UnknownLease { lease_id })?;

        let was_claimed = lease.status == CapabilityLeaseStatus::Claimed;
        ensure_consumable(lease)?;
        if let Some(remaining) = lease.grant.constraints.max_invocations.as_mut() {
            *remaining -= 1;
            if *remaining == 0 {
                lease.status = CapabilityLeaseStatus::Consumed;
            } else if was_claimed {
                lease.status = CapabilityLeaseStatus::Active;
            }
        } else if was_claimed {
            lease.status = CapabilityLeaseStatus::Active;
        }
        Ok(lease.clone())
    }

    async fn leases_for_scope(&self, scope: &ResourceScope) -> Vec<CapabilityLease> {
        let mut leases = self
            .leases_guard()
            .values()
            .filter(|lease| same_tenant_user(&lease.scope, scope))
            .cloned()
            .collect::<Vec<_>>();
        leases.sort_by_key(|lease| lease.grant.id.as_uuid());
        leases
    }

    async fn active_leases_for_context(&self, context: &ExecutionContext) -> Vec<CapabilityLease> {
        self.leases_for_scope(&context.resource_scope)
            .await
            .into_iter()
            .filter(|lease| lease_is_authorizing(lease, context))
            .collect()
    }
}

/// Filesystem-backed capability lease store under tenant/user/invocation-scoped `/engine` paths.
pub struct FilesystemCapabilityLeaseStore<'a, F>
where
    F: RootFilesystem,
{
    filesystem: &'a F,
    lock: tokio::sync::Mutex<()>,
}

impl<'a, F> FilesystemCapabilityLeaseStore<'a, F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: &'a F) -> Self {
        Self {
            filesystem,
            lock: tokio::sync::Mutex::new(()),
        }
    }

    async fn read_lease(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<Option<CapabilityLease>, CapabilityLeaseError> {
        let path = lease_path(scope, lease_id)?;
        let bytes = match self.filesystem.read_file(&path).await {
            Ok(bytes) => bytes,
            Err(error) if is_not_found(&error) => return Ok(None),
            Err(error) => return Err(lease_persistence_error(error)),
        };
        deserialize(&bytes).map(Some)
    }

    async fn write_lease(&self, lease: &CapabilityLease) -> Result<(), CapabilityLeaseError> {
        let path = lease_path(&lease.scope, lease.grant.id)?;
        let bytes = serialize_pretty(lease)?;
        self.filesystem
            .write_file(&path, &bytes)
            .await
            .map_err(lease_persistence_error)
    }

    async fn list_invocation_roots(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<VirtualPath>, CapabilityLeaseError> {
        let root = lease_tenant_user_root(scope)?;
        let entries = match self.filesystem.list_dir(&root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(lease_persistence_error(error)),
        };
        Ok(entries
            .into_iter()
            .filter(|entry| entry.file_type == FileType::Directory)
            .map(|entry| entry.path)
            .collect())
    }

    async fn list_lease_files(
        &self,
        root: &VirtualPath,
    ) -> Result<Vec<VirtualPath>, CapabilityLeaseError> {
        let entries = match self.filesystem.list_dir(root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(lease_persistence_error(error)),
        };
        Ok(entries
            .into_iter()
            .filter(|entry| entry.file_type == FileType::File)
            .map(|entry| entry.path)
            .collect())
    }

    async fn read_lease_file(
        &self,
        path: &VirtualPath,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        let bytes = self
            .filesystem
            .read_file(path)
            .await
            .map_err(lease_persistence_error)?;
        deserialize(&bytes)
    }
}

#[async_trait]
impl<F> CapabilityLeaseStore for FilesystemCapabilityLeaseStore<'_, F>
where
    F: RootFilesystem,
{
    async fn issue(&self, lease: CapabilityLease) -> Result<CapabilityLease, CapabilityLeaseError> {
        let _guard = self.lock.lock().await;
        self.write_lease(&lease).await?;
        Ok(lease)
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        let _guard = self.lock.lock().await;
        let mut lease = self
            .read_lease(scope, lease_id)
            .await?
            .ok_or(CapabilityLeaseError::UnknownLease { lease_id })?;
        lease.status = CapabilityLeaseStatus::Revoked;
        self.write_lease(&lease).await?;
        Ok(lease)
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Option<CapabilityLease> {
        self.read_lease(scope, lease_id).await.ok().flatten()
    }

    async fn claim(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
        invocation_fingerprint: &InvocationFingerprint,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        let _guard = self.lock.lock().await;
        let mut lease = self
            .read_lease(scope, lease_id)
            .await?
            .ok_or(CapabilityLeaseError::UnknownLease { lease_id })?;
        ensure_claimable(&lease, invocation_fingerprint)?;
        lease.status = CapabilityLeaseStatus::Claimed;
        self.write_lease(&lease).await?;
        Ok(lease)
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        let _guard = self.lock.lock().await;
        let mut lease = self
            .read_lease(scope, lease_id)
            .await?
            .ok_or(CapabilityLeaseError::UnknownLease { lease_id })?;
        let was_claimed = lease.status == CapabilityLeaseStatus::Claimed;
        ensure_consumable(&lease)?;
        if let Some(remaining) = lease.grant.constraints.max_invocations.as_mut() {
            *remaining -= 1;
            if *remaining == 0 {
                lease.status = CapabilityLeaseStatus::Consumed;
            } else if was_claimed {
                lease.status = CapabilityLeaseStatus::Active;
            }
        } else if was_claimed {
            lease.status = CapabilityLeaseStatus::Active;
        }
        self.write_lease(&lease).await?;
        Ok(lease)
    }

    async fn leases_for_scope(&self, scope: &ResourceScope) -> Vec<CapabilityLease> {
        let Ok(roots) = self.list_invocation_roots(scope).await else {
            return Vec::new();
        };
        let mut leases = Vec::new();
        for root in roots {
            let Ok(files) = self.list_lease_files(&root).await else {
                continue;
            };
            for path in files {
                if let Ok(lease) = self.read_lease_file(&path).await {
                    leases.push(lease);
                }
            }
        }
        let mut leases = leases
            .into_iter()
            .filter(|lease| same_tenant_user(&lease.scope, scope))
            .collect::<Vec<_>>();
        leases.sort_by_key(|lease| lease.grant.id.as_uuid());
        leases
    }

    async fn active_leases_for_context(&self, context: &ExecutionContext) -> Vec<CapabilityLease> {
        self.leases_for_scope(&context.resource_scope)
            .await
            .into_iter()
            .filter(|lease| lease_is_authorizing(lease, context))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CapabilityLeaseKey {
    tenant_id: TenantId,
    user_id: UserId,
    invocation_id: InvocationId,
    lease_id: CapabilityGrantId,
}

impl CapabilityLeaseKey {
    fn new(scope: &ResourceScope, lease_id: CapabilityGrantId) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            invocation_id: scope.invocation_id,
            lease_id,
        }
    }
}

/// Authorizer that combines request-scoped grants with active capability leases.
pub struct LeaseBackedAuthorizer<'a, S>
where
    S: CapabilityLeaseStore + ?Sized,
{
    leases: &'a S,
}

impl<'a, S> LeaseBackedAuthorizer<'a, S>
where
    S: CapabilityLeaseStore + ?Sized,
{
    pub fn new(leases: &'a S) -> Self {
        Self { leases }
    }
}

#[async_trait]
impl<S> CapabilityDispatchAuthorizer for LeaseBackedAuthorizer<'_, S>
where
    S: CapabilityLeaseStore + ?Sized,
{
    async fn authorize_dispatch(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        if context.validate().is_err() {
            return Decision::Deny {
                reason: DenyReason::InternalInvariantViolation,
            };
        }

        let lease_grants = self.leases.active_grants_for_context(context).await;
        authorize_from_grants(
            context,
            descriptor,
            context.grants.grants.iter().chain(lease_grants.iter()),
        )
    }

    async fn authorize_spawn(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        if context.validate().is_err() {
            return Decision::Deny {
                reason: DenyReason::InternalInvariantViolation,
            };
        }

        let lease_grants = self.leases.active_grants_for_context(context).await;
        authorize_from_grants(
            context,
            &spawn_descriptor(descriptor),
            context.grants.grants.iter().chain(lease_grants.iter()),
        )
    }
}

fn spawn_descriptor(descriptor: &CapabilityDescriptor) -> CapabilityDescriptor {
    let mut descriptor = descriptor.clone();
    if !descriptor.effects.contains(&EffectKind::SpawnProcess) {
        descriptor.effects.push(EffectKind::SpawnProcess);
    }
    descriptor
}

fn authorize_from_grants<'a>(
    context: &ExecutionContext,
    descriptor: &CapabilityDescriptor,
    grants: impl Iterator<Item = &'a CapabilityGrant>,
) -> Decision {
    if context.validate().is_err() {
        return Decision::Deny {
            reason: DenyReason::InternalInvariantViolation,
        };
    }

    let Some(grant) = grants
        .filter(|grant| grant.capability == descriptor.id)
        .find(|grant| principal_matches_context(&grant.grantee, context))
    else {
        return Decision::Deny {
            reason: DenyReason::MissingGrant,
        };
    };

    if !effects_are_covered(&descriptor.effects, &grant.constraints.allowed_effects) {
        return Decision::Deny {
            reason: DenyReason::PolicyDenied,
        };
    }

    Decision::Allow {
        obligations: Vec::new(),
    }
}

fn principal_matches_context(principal: &Principal, context: &ExecutionContext) -> bool {
    match principal {
        Principal::Tenant(id) => id == &context.tenant_id,
        Principal::User(id) => id == &context.user_id,
        Principal::Project(id) => context.project_id.as_ref() == Some(id),
        Principal::Mission(id) => context.mission_id.as_ref() == Some(id),
        Principal::Thread(id) => context.thread_id.as_ref() == Some(id),
        Principal::Extension(id) => id == &context.extension_id,
        Principal::System => false,
    }
}

fn effects_are_covered(required: &[EffectKind], allowed: &[EffectKind]) -> bool {
    required.iter().all(|effect| allowed.contains(effect))
}

fn lease_is_authorizing(lease: &CapabilityLease, context: &ExecutionContext) -> bool {
    lease.status == CapabilityLeaseStatus::Active
        && lease.scope.invocation_id == context.invocation_id
        && !lease_is_expired(lease)
        && lease.grant.constraints.max_invocations != Some(0)
}

fn ensure_claimable(
    lease: &CapabilityLease,
    invocation_fingerprint: &InvocationFingerprint,
) -> Result<(), CapabilityLeaseError> {
    let lease_id = lease.grant.id;
    if lease.status != CapabilityLeaseStatus::Active {
        return Err(CapabilityLeaseError::InactiveLease {
            lease_id,
            status: lease.status,
        });
    }
    if lease.invocation_fingerprint.as_ref() != Some(invocation_fingerprint) {
        return Err(CapabilityLeaseError::FingerprintMismatch { lease_id });
    }
    ensure_not_expired_or_exhausted(lease)
}

fn ensure_consumable(lease: &CapabilityLease) -> Result<(), CapabilityLeaseError> {
    let lease_id = lease.grant.id;
    match lease.status {
        CapabilityLeaseStatus::Active | CapabilityLeaseStatus::Claimed => {}
        CapabilityLeaseStatus::Consumed => {
            return Err(CapabilityLeaseError::ExhaustedLease { lease_id });
        }
        CapabilityLeaseStatus::Revoked => {
            return Err(CapabilityLeaseError::InactiveLease {
                lease_id,
                status: lease.status,
            });
        }
    }

    ensure_not_expired_or_exhausted(lease)
}

fn ensure_not_expired_or_exhausted(lease: &CapabilityLease) -> Result<(), CapabilityLeaseError> {
    let lease_id = lease.grant.id;
    if lease_is_expired(lease) {
        return Err(CapabilityLeaseError::ExpiredLease { lease_id });
    }

    if lease.grant.constraints.max_invocations == Some(0) {
        return Err(CapabilityLeaseError::ExhaustedLease { lease_id });
    }

    Ok(())
}

fn lease_is_expired(lease: &CapabilityLease) -> bool {
    lease
        .grant
        .constraints
        .expires_at
        .is_some_and(|expires_at| expires_at <= Utc::now())
}

fn same_tenant_user(left: &ResourceScope, right: &ResourceScope) -> bool {
    left.tenant_id == right.tenant_id && left.user_id == right.user_id
}

fn lease_path(
    scope: &ResourceScope,
    lease_id: CapabilityGrantId,
) -> Result<VirtualPath, CapabilityLeaseError> {
    VirtualPath::new(format!(
        "{}/{lease_id}.json",
        lease_invocation_root(scope)?.as_str()
    ))
    .map_err(lease_host_api_error)
}

fn lease_invocation_root(scope: &ResourceScope) -> Result<VirtualPath, CapabilityLeaseError> {
    VirtualPath::new(format!(
        "{}/{}",
        lease_tenant_user_root(scope)?.as_str(),
        scope.invocation_id
    ))
    .map_err(lease_host_api_error)
}

fn lease_tenant_user_root(scope: &ResourceScope) -> Result<VirtualPath, CapabilityLeaseError> {
    VirtualPath::new(format!(
        "/engine/tenants/{}/users/{}/capability-leases",
        scope.tenant_id, scope.user_id
    ))
    .map_err(lease_host_api_error)
}

fn serialize_pretty<T>(value: &T) -> Result<Vec<u8>, CapabilityLeaseError>
where
    T: Serialize,
{
    serde_json::to_vec_pretty(value).map_err(|error| CapabilityLeaseError::Persistence {
        reason: error.to_string(),
    })
}

fn deserialize<T>(bytes: &[u8]) -> Result<T, CapabilityLeaseError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_slice(bytes).map_err(|error| CapabilityLeaseError::Persistence {
        reason: error.to_string(),
    })
}

fn lease_host_api_error(error: HostApiError) -> CapabilityLeaseError {
    CapabilityLeaseError::Persistence {
        reason: error.to_string(),
    }
}

fn lease_persistence_error(error: FilesystemError) -> CapabilityLeaseError {
    CapabilityLeaseError::Persistence {
        reason: error.to_string(),
    }
}

fn is_not_found(error: &FilesystemError) -> bool {
    match error {
        FilesystemError::Backend { reason, .. } => {
            reason.contains("No such file")
                || reason.contains("not found")
                || reason.contains("entity not found")
        }
        _ => false,
    }
}
