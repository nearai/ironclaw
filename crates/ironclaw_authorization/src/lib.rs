//! Capability authorization contracts for IronClaw Reborn.
//!
//! `ironclaw_authorization` evaluates authority-bearing host API contracts. It
//! does not execute capabilities, reserve resources, prompt users, or reach into
//! runtime internals. The first slices implement grant- and lease-backed gates
//! for capability dispatch.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{FileType, FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    AgentId, CapabilityDescriptor, CapabilityGrant, CapabilityGrantId, Decision, DenyReason,
    EffectKind, ExecutionContext, HostApiError, InvocationFingerprint, InvocationId, MissionId,
    NetworkPolicy, Obligation, Obligations, Principal, ProjectId, ResourceCeiling,
    ResourceEstimate, ResourceScope, SandboxQuota, TenantId, ThreadId, UserId, VirtualPath,
};
use ironclaw_trust::{AuthorityCeiling, TrustDecision};
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

/// Trust-aware capability dispatch authorizer.
///
/// This trait is the host-policy-aware counterpart to
/// [`CapabilityDispatchAuthorizer`]. Callers pass the policy-validated
/// [`TrustDecision`] alongside the serializable [`ExecutionContext`]. We keep
/// this separate because `ironclaw_trust::EffectiveTrustClass` deliberately
/// does not implement `Deserialize`; it should not be embedded directly in
/// wire-shaped execution contexts.
#[async_trait]
pub trait TrustAwareCapabilityDispatchAuthorizer: Send + Sync {
    /// Authorize a dispatch using both explicit grants/leases and the
    /// policy-derived authority ceiling.
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
    ) -> Decision;

    /// Authorize a background-process spawn using both explicit grants/leases
    /// and the policy-derived authority ceiling.
    async fn authorize_spawn_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
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
        estimate: &ResourceEstimate,
    ) -> Decision {
        authorize_from_grants(context, descriptor, estimate, context.grants.grants.iter())
    }

    async fn authorize_spawn(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
    ) -> Decision {
        authorize_from_grants(
            context,
            &spawn_descriptor(descriptor),
            estimate,
            context.grants.grants.iter(),
        )
    }
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for GrantAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
    ) -> Decision {
        authorize_from_grants_with_trust(
            context,
            descriptor,
            estimate,
            context.grants.grants.iter(),
            trust_decision,
        )
    }

    async fn authorize_spawn_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
    ) -> Decision {
        authorize_from_grants_with_trust(
            context,
            &spawn_descriptor(descriptor),
            estimate,
            context.grants.grants.iter(),
            trust_decision,
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
    #[error("capability lease {lease_id} has not been claimed with its fingerprint")]
    UnclaimedFingerprintLease { lease_id: CapabilityGrantId },
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

    /// Revokes a lease only within the exact resource-owner/invocation scope that owns it.
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

    /// Lists leases visible to the exact resource-owner scope without exposing cross-scope records.
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
        if lease.invocation_fingerprint.is_some() {
            if let Some(remaining) = lease.grant.constraints.max_invocations.as_mut() {
                *remaining = 0;
            }
            lease.status = CapabilityLeaseStatus::Consumed;
        } else if let Some(remaining) = lease.grant.constraints.max_invocations.as_mut() {
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
            .filter(|lease| same_scope_owner(&lease.scope, scope))
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

/// Filesystem-backed capability lease store under resource-owner/invocation-scoped `/engine` paths.
pub struct FilesystemCapabilityLeaseStore<'a, F>
where
    F: RootFilesystem,
{
    filesystem: &'a F,
    mutation_locks: Mutex<HashMap<CapabilityLeaseOwnerKey, Arc<tokio::sync::Mutex<()>>>>,
}

impl<'a, F> FilesystemCapabilityLeaseStore<'a, F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: &'a F) -> Self {
        Self {
            filesystem,
            mutation_locks: Mutex::new(HashMap::new()),
        }
    }

    fn mutation_lock(&self, scope: &ResourceScope) -> Arc<tokio::sync::Mutex<()>> {
        let key = CapabilityLeaseOwnerKey::new(scope);
        self.mutation_locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(key)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
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

    async fn read_lease_index(
        &self,
        scope: &ResourceScope,
    ) -> Result<Option<Vec<VirtualPath>>, CapabilityLeaseError> {
        let path = lease_index_path(scope)?;
        let bytes = match self.filesystem.read_file(&path).await {
            Ok(bytes) => bytes,
            Err(error) if is_not_found(&error) => return Ok(None),
            Err(error) => return Err(lease_persistence_error(error)),
        };
        let index: CapabilityLeaseIndex = deserialize(&bytes)?;
        Ok(Some(index.paths))
    }

    async fn write_lease_index(
        &self,
        scope: &ResourceScope,
        mut paths: Vec<VirtualPath>,
    ) -> Result<(), CapabilityLeaseError> {
        paths.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        paths.dedup_by(|left, right| left.as_str() == right.as_str());
        let path = lease_index_path(scope)?;
        let bytes = serialize_pretty(&CapabilityLeaseIndex { paths })?;
        self.filesystem
            .write_file(&path, &bytes)
            .await
            .map_err(lease_persistence_error)
    }

    async fn index_lease_path(
        &self,
        scope: &ResourceScope,
        path: VirtualPath,
    ) -> Result<(), CapabilityLeaseError> {
        let mut paths = self.read_lease_index(scope).await?.unwrap_or_default();
        if !paths.iter().any(|existing| existing == &path) {
            paths.push(path);
        }
        self.write_lease_index(scope, paths).await
    }

    async fn list_lease_paths_from_index_or_scan(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<VirtualPath>, CapabilityLeaseError> {
        if let Some(paths) = self.read_lease_index(scope).await? {
            return Ok(paths);
        }
        self.scan_lease_paths(scope).await
    }

    async fn scan_lease_paths(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<VirtualPath>, CapabilityLeaseError> {
        let roots = self.list_invocation_roots(scope).await?;
        let mut paths = Vec::new();
        for root in roots {
            paths.extend(self.list_lease_files(&root).await?);
        }
        Ok(paths)
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
        let lock = self.mutation_lock(&lease.scope);
        let _guard = lock.lock().await;
        self.index_lease_path(&lease.scope, lease_path(&lease.scope, lease.grant.id)?)
            .await?;
        self.write_lease(&lease).await?;
        Ok(lease)
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        let lock = self.mutation_lock(scope);
        let _guard = lock.lock().await;
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
        let lock = self.mutation_lock(scope);
        let _guard = lock.lock().await;
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
        let lock = self.mutation_lock(scope);
        let _guard = lock.lock().await;
        let mut lease = self
            .read_lease(scope, lease_id)
            .await?
            .ok_or(CapabilityLeaseError::UnknownLease { lease_id })?;
        let was_claimed = lease.status == CapabilityLeaseStatus::Claimed;
        ensure_consumable(&lease)?;
        if lease.invocation_fingerprint.is_some() {
            if let Some(remaining) = lease.grant.constraints.max_invocations.as_mut() {
                *remaining = 0;
            }
            lease.status = CapabilityLeaseStatus::Consumed;
        } else if let Some(remaining) = lease.grant.constraints.max_invocations.as_mut() {
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
        let Ok(paths) = self.list_lease_paths_from_index_or_scan(scope).await else {
            return Vec::new();
        };
        let mut leases = Vec::new();
        for path in paths {
            if let Ok(lease) = self.read_lease_file(&path).await {
                leases.push(lease);
            }
        }
        let mut leases = leases
            .into_iter()
            .filter(|lease| same_scope_owner(&lease.scope, scope))
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
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    mission_id: Option<MissionId>,
    thread_id: Option<ThreadId>,
    invocation_id: InvocationId,
    lease_id: CapabilityGrantId,
}

impl CapabilityLeaseKey {
    fn new(scope: &ResourceScope, lease_id: CapabilityGrantId) -> Self {
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
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CapabilityLeaseOwnerKey {
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    mission_id: Option<MissionId>,
    thread_id: Option<ThreadId>,
}

impl CapabilityLeaseOwnerKey {
    fn new(scope: &ResourceScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            mission_id: scope.mission_id.clone(),
            thread_id: scope.thread_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CapabilityLeaseIndex {
    paths: Vec<VirtualPath>,
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
        estimate: &ResourceEstimate,
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
            estimate,
            context.grants.grants.iter().chain(lease_grants.iter()),
        )
    }

    async fn authorize_spawn(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
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
            estimate,
            context.grants.grants.iter().chain(lease_grants.iter()),
        )
    }
}

#[async_trait]
impl<S> TrustAwareCapabilityDispatchAuthorizer for LeaseBackedAuthorizer<'_, S>
where
    S: CapabilityLeaseStore + ?Sized,
{
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
    ) -> Decision {
        if context.validate().is_err() {
            return Decision::Deny {
                reason: DenyReason::InternalInvariantViolation,
            };
        }

        let lease_grants = self.leases.active_grants_for_context(context).await;
        authorize_from_grants_with_trust(
            context,
            descriptor,
            estimate,
            context.grants.grants.iter().chain(lease_grants.iter()),
            trust_decision,
        )
    }

    async fn authorize_spawn_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
    ) -> Decision {
        if context.validate().is_err() {
            return Decision::Deny {
                reason: DenyReason::InternalInvariantViolation,
            };
        }

        let lease_grants = self.leases.active_grants_for_context(context).await;
        authorize_from_grants_with_trust(
            context,
            &spawn_descriptor(descriptor),
            estimate,
            context.grants.grants.iter().chain(lease_grants.iter()),
            trust_decision,
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
    estimate: &ResourceEstimate,
    grants: impl Iterator<Item = &'a CapabilityGrant>,
) -> Decision {
    authorize_from_grants_with_authority_ceiling(context, descriptor, estimate, grants, None)
}

fn authorize_from_grants_with_trust<'a>(
    context: &ExecutionContext,
    descriptor: &CapabilityDescriptor,
    estimate: &ResourceEstimate,
    grants: impl Iterator<Item = &'a CapabilityGrant>,
    trust_decision: &TrustDecision,
) -> Decision {
    if context.validate().is_err() {
        return Decision::Deny {
            reason: DenyReason::InternalInvariantViolation,
        };
    }
    if context.trust != trust_decision.effective_trust.class() {
        return Decision::Deny {
            reason: DenyReason::PolicyDenied,
        };
    }
    authorize_from_grants_with_authority_ceiling(
        context,
        descriptor,
        estimate,
        grants,
        Some(&trust_decision.authority_ceiling),
    )
}

fn authorize_from_grants_with_authority_ceiling<'a>(
    context: &ExecutionContext,
    descriptor: &CapabilityDescriptor,
    estimate: &ResourceEstimate,
    grants: impl Iterator<Item = &'a CapabilityGrant>,
    authority_ceiling: Option<&AuthorityCeiling>,
) -> Decision {
    if context.validate().is_err() {
        return Decision::Deny {
            reason: DenyReason::InternalInvariantViolation,
        };
    }

    let mut saw_active_matching_grant = false;
    for grant in grants
        .filter(|grant| grant.capability == descriptor.id)
        .filter(|grant| principal_matches_context(&grant.grantee, context))
        .filter(|grant| grant_is_active(grant))
    {
        saw_active_matching_grant = true;
        let effective_resource_ceiling = intersect_resource_ceilings(
            grant.constraints.resource_ceiling.as_ref(),
            authority_ceiling.and_then(|ceiling| ceiling.max_resource_ceiling.as_ref()),
        );
        let authority_effects_allow_descriptor = match authority_ceiling {
            Some(ceiling) => effects_are_covered(&descriptor.effects, &ceiling.allowed_effects),
            None => true,
        };
        if effects_are_covered(&descriptor.effects, &grant.constraints.allowed_effects)
            && authority_effects_allow_descriptor
            && resource_estimate_is_covered(estimate, effective_resource_ceiling.as_ref())
            && let Some(obligations) =
                obligations_for_grant(descriptor, grant, effective_resource_ceiling)
        {
            return Decision::Allow { obligations };
        }
    }

    if saw_active_matching_grant {
        Decision::Deny {
            reason: DenyReason::PolicyDenied,
        }
    } else {
        Decision::Deny {
            reason: DenyReason::MissingGrant,
        }
    }
}

fn obligations_for_grant(
    descriptor: &CapabilityDescriptor,
    grant: &CapabilityGrant,
    effective_resource_ceiling: Option<ResourceCeiling>,
) -> Option<Obligations> {
    let mut obligations = Vec::new();

    if descriptor_requires_mount_policy(descriptor) {
        obligations.push(Obligation::UseScopedMounts {
            mounts: grant.constraints.mounts.clone(),
        });
    }

    if descriptor.effects.contains(&EffectKind::Network)
        || network_policy_is_constrained(&grant.constraints.network)
    {
        obligations.push(Obligation::ApplyNetworkPolicy {
            policy: grant.constraints.network.clone(),
        });
    }

    if descriptor.effects.contains(&EffectKind::UseSecret) {
        match grant.constraints.secrets.as_slice() {
            [handle] => obligations.push(Obligation::InjectSecretOnce {
                handle: handle.clone(),
            }),
            _ => return None,
        }
    }

    if let Some(ceiling) = effective_resource_ceiling {
        obligations.push(Obligation::EnforceResourceCeiling {
            ceiling: ceiling.clone(),
        });
        if let Some(bytes) = ceiling.max_output_bytes {
            obligations.push(Obligation::EnforceOutputLimit { bytes });
        }
    }

    Obligations::new(obligations).ok()
}

fn descriptor_requires_mount_policy(descriptor: &CapabilityDescriptor) -> bool {
    descriptor.effects.iter().any(|effect| {
        matches!(
            effect,
            EffectKind::ReadFilesystem
                | EffectKind::WriteFilesystem
                | EffectKind::DeleteFilesystem
                | EffectKind::ExecuteCode
        )
    })
}

fn network_policy_is_constrained(policy: &NetworkPolicy) -> bool {
    !policy.allowed_targets.is_empty()
        || policy.deny_private_ip_ranges
        || policy.max_egress_bytes.is_some()
}

fn principal_matches_context(principal: &Principal, context: &ExecutionContext) -> bool {
    match principal {
        Principal::Tenant(id) => id == &context.tenant_id,
        Principal::User(id) => id == &context.user_id,
        Principal::Agent(id) => context.agent_id.as_ref() == Some(id),
        Principal::Project(id) => context.project_id.as_ref() == Some(id),
        Principal::Mission(id) => context.mission_id.as_ref() == Some(id),
        Principal::Thread(id) => context.thread_id.as_ref() == Some(id),
        Principal::Extension(id) => id == &context.extension_id,
        Principal::HostRuntime | Principal::System(_) => false,
    }
}

fn effects_are_covered(required: &[EffectKind], allowed: &[EffectKind]) -> bool {
    required.iter().all(|effect| allowed.contains(effect))
}

fn grant_is_active(grant: &CapabilityGrant) -> bool {
    let grant_not_expired = match grant.constraints.expires_at.as_ref() {
        Some(expires_at) => expires_at > &Utc::now(),
        None => true,
    };
    grant_not_expired && grant.constraints.max_invocations != Some(0)
}

/// Returns true when an existing grant exceeds the current policy-derived
/// authority ceiling and should be reissued or revoked by a trust-change
/// invalidation listener.
///
/// This helper is intentionally synchronous and store-agnostic: the
/// `ironclaw_trust::InvalidationBus` runs listeners synchronously, while this
/// crate's durable lease stores are async. Higher-level host wiring can use
/// this predicate inside whatever transactional store/reconciliation path it
/// owns, without introducing nested blocking executors or process/runtime dependencies here.
pub fn grant_exceeds_authority_ceiling(
    grant: &CapabilityGrant,
    authority_ceiling: &AuthorityCeiling,
) -> bool {
    !effects_are_covered(
        &grant.constraints.allowed_effects,
        &authority_ceiling.allowed_effects,
    ) || resource_ceiling_exceeds_authority(
        grant.constraints.resource_ceiling.as_ref(),
        authority_ceiling.max_resource_ceiling.as_ref(),
    )
}

fn resource_ceiling_exceeds_authority(
    grant_ceiling: Option<&ResourceCeiling>,
    authority_ceiling: Option<&ResourceCeiling>,
) -> bool {
    match (grant_ceiling, authority_ceiling) {
        (None, Some(_)) => true,
        (None, None) | (Some(_), None) => false,
        (Some(grant), Some(authority)) => {
            limit_exceeds(&grant.max_usd, &authority.max_usd)
                || limit_exceeds(&grant.max_input_tokens, &authority.max_input_tokens)
                || limit_exceeds(&grant.max_output_tokens, &authority.max_output_tokens)
                || limit_exceeds(&grant.max_wall_clock_ms, &authority.max_wall_clock_ms)
                || limit_exceeds(&grant.max_output_bytes, &authority.max_output_bytes)
                || sandbox_quota_exceeds_authority(
                    grant.sandbox.as_ref(),
                    authority.sandbox.as_ref(),
                )
        }
    }
}

fn sandbox_quota_exceeds_authority(
    grant_quota: Option<&SandboxQuota>,
    authority_quota: Option<&SandboxQuota>,
) -> bool {
    match (grant_quota, authority_quota) {
        (None, Some(_)) => true,
        (None, None) | (Some(_), None) => false,
        (Some(grant), Some(authority)) => {
            limit_exceeds(&grant.cpu_time_ms, &authority.cpu_time_ms)
                || limit_exceeds(&grant.memory_bytes, &authority.memory_bytes)
                || limit_exceeds(&grant.disk_bytes, &authority.disk_bytes)
                || limit_exceeds(&grant.network_egress_bytes, &authority.network_egress_bytes)
                || limit_exceeds(&grant.process_count, &authority.process_count)
        }
    }
}

fn limit_exceeds<T>(grant: &Option<T>, authority: &Option<T>) -> bool
where
    T: PartialOrd,
{
    match (grant, authority) {
        (Some(grant), Some(authority)) => grant > authority,
        (None, Some(_)) => true,
        (None, None) | (Some(_), None) => false,
    }
}

fn intersect_resource_ceilings(
    grant_ceiling: Option<&ResourceCeiling>,
    authority_ceiling: Option<&ResourceCeiling>,
) -> Option<ResourceCeiling> {
    match (grant_ceiling, authority_ceiling) {
        (None, None) => None,
        (Some(ceiling), None) | (None, Some(ceiling)) => Some(ceiling.clone()),
        (Some(grant), Some(authority)) => Some(ResourceCeiling {
            max_usd: stricter_limit(&grant.max_usd, &authority.max_usd),
            max_input_tokens: stricter_limit(&grant.max_input_tokens, &authority.max_input_tokens),
            max_output_tokens: stricter_limit(
                &grant.max_output_tokens,
                &authority.max_output_tokens,
            ),
            max_wall_clock_ms: stricter_limit(
                &grant.max_wall_clock_ms,
                &authority.max_wall_clock_ms,
            ),
            max_output_bytes: stricter_limit(&grant.max_output_bytes, &authority.max_output_bytes),
            sandbox: intersect_sandbox_quotas(grant.sandbox.as_ref(), authority.sandbox.as_ref()),
        }),
    }
}

fn intersect_sandbox_quotas(
    grant_quota: Option<&SandboxQuota>,
    authority_quota: Option<&SandboxQuota>,
) -> Option<SandboxQuota> {
    match (grant_quota, authority_quota) {
        (None, None) => None,
        (Some(quota), None) | (None, Some(quota)) => Some(quota.clone()),
        (Some(grant), Some(authority)) => Some(SandboxQuota {
            cpu_time_ms: stricter_limit(&grant.cpu_time_ms, &authority.cpu_time_ms),
            memory_bytes: stricter_limit(&grant.memory_bytes, &authority.memory_bytes),
            disk_bytes: stricter_limit(&grant.disk_bytes, &authority.disk_bytes),
            network_egress_bytes: stricter_limit(
                &grant.network_egress_bytes,
                &authority.network_egress_bytes,
            ),
            process_count: stricter_limit(&grant.process_count, &authority.process_count),
        }),
    }
}

fn stricter_limit<T>(left: &Option<T>, right: &Option<T>) -> Option<T>
where
    T: Clone + PartialOrd,
{
    match (left, right) {
        (Some(left), Some(right)) if right < left => Some(right.clone()),
        (Some(left), Some(_)) => Some(left.clone()),
        (Some(left), None) => Some(left.clone()),
        (None, Some(right)) => Some(right.clone()),
        (None, None) => None,
    }
}

fn resource_estimate_is_covered(
    estimate: &ResourceEstimate,
    ceiling: Option<&ResourceCeiling>,
) -> bool {
    let Some(ceiling) = ceiling else {
        return true;
    };
    options_within_ceiling(estimate.usd.as_ref(), ceiling.max_usd.as_ref())
        && options_within_ceiling(
            estimate.input_tokens.as_ref(),
            ceiling.max_input_tokens.as_ref(),
        )
        && options_within_ceiling(
            estimate.output_tokens.as_ref(),
            ceiling.max_output_tokens.as_ref(),
        )
        && options_within_ceiling(
            estimate.wall_clock_ms.as_ref(),
            ceiling.max_wall_clock_ms.as_ref(),
        )
        && options_within_ceiling(
            estimate.output_bytes.as_ref(),
            ceiling.max_output_bytes.as_ref(),
        )
        && match ceiling.sandbox.as_ref() {
            Some(sandbox) => {
                options_within_ceiling(
                    estimate.network_egress_bytes.as_ref(),
                    sandbox.network_egress_bytes.as_ref(),
                ) && options_within_ceiling(
                    estimate.process_count.as_ref(),
                    sandbox.process_count.as_ref(),
                )
            }
            None => true,
        }
}

fn options_within_ceiling<T>(estimate: Option<&T>, maximum: Option<&T>) -> bool
where
    T: PartialOrd,
{
    match (estimate, maximum) {
        (Some(estimate), Some(maximum)) => estimate <= maximum,
        (None, Some(_)) => false,
        _ => true,
    }
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

    if lease.invocation_fingerprint.is_some() && lease.status != CapabilityLeaseStatus::Claimed {
        return Err(CapabilityLeaseError::UnclaimedFingerprintLease { lease_id });
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

fn same_scope_owner(left: &ResourceScope, right: &ResourceScope) -> bool {
    left.tenant_id == right.tenant_id
        && left.user_id == right.user_id
        && left.agent_id == right.agent_id
        && left.project_id == right.project_id
        && left.mission_id == right.mission_id
        && left.thread_id == right.thread_id
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

fn lease_index_path(scope: &ResourceScope) -> Result<VirtualPath, CapabilityLeaseError> {
    VirtualPath::new(format!(
        "{}/_lease_index.json",
        lease_tenant_user_root(scope)?.as_str()
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
    VirtualPath::new(format!("{}/capability-leases", scoped_owner_root(scope)))
        .map_err(lease_host_api_error)
}

fn scoped_owner_root(scope: &ResourceScope) -> String {
    let mut base = format!(
        "/engine/tenants/{}/users/{}",
        scope.tenant_id, scope.user_id
    );
    if let Some(agent_id) = &scope.agent_id {
        base = format!("{base}/agents/{agent_id}");
    }
    if let Some(project_id) = &scope.project_id {
        base = format!("{base}/projects/{project_id}");
    }
    if let Some(mission_id) = &scope.mission_id {
        base = format!("{base}/missions/{mission_id}");
    }
    if let Some(thread_id) = &scope.thread_id {
        base = format!("{base}/threads/{thread_id}");
    }
    base
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
    matches!(error, FilesystemError::NotFound { .. })
}
