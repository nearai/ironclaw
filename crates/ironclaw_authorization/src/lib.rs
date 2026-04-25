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

use chrono::Utc;
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityGrant, CapabilityGrantId, Decision, DenyReason, EffectKind,
    ExecutionContext, InvocationFingerprint, InvocationId, Principal, ResourceEstimate,
    ResourceScope, TenantId, UserId,
};
use thiserror::Error;

/// Authorizes a capability dispatch request against an execution context.
pub trait CapabilityDispatchAuthorizer: Send + Sync {
    fn authorize_dispatch(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
    ) -> Decision;

    fn authorize_spawn(
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

impl CapabilityDispatchAuthorizer for GrantAuthorizer {
    fn authorize_dispatch(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        authorize_from_grants(context, descriptor, context.grants.grants.iter())
    }

    fn authorize_spawn(
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
#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

/// Store of active/revoked capability leases.
pub trait CapabilityLeaseStore: Send + Sync {
    fn issue(&self, lease: CapabilityLease) -> CapabilityLease;
    fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError>;
    fn get(&self, scope: &ResourceScope, lease_id: CapabilityGrantId) -> Option<CapabilityLease>;
    fn claim(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
        invocation_fingerprint: &InvocationFingerprint,
    ) -> Result<CapabilityLease, CapabilityLeaseError>;
    fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError>;
    fn leases_for_scope(&self, scope: &ResourceScope) -> Vec<CapabilityLease>;
    fn active_leases_for_context(&self, context: &ExecutionContext) -> Vec<CapabilityLease>;
    fn active_grants_for_context(&self, context: &ExecutionContext) -> Vec<CapabilityGrant> {
        self.active_leases_for_context(context)
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

impl CapabilityLeaseStore for InMemoryCapabilityLeaseStore {
    fn issue(&self, lease: CapabilityLease) -> CapabilityLease {
        self.leases_guard().insert(
            CapabilityLeaseKey::new(&lease.scope, lease.grant.id),
            lease.clone(),
        );
        lease
    }

    fn revoke(
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

    fn get(&self, scope: &ResourceScope, lease_id: CapabilityGrantId) -> Option<CapabilityLease> {
        self.leases_guard()
            .get(&CapabilityLeaseKey::new(scope, lease_id))
            .cloned()
    }

    fn claim(
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

    fn consume(
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

    fn leases_for_scope(&self, scope: &ResourceScope) -> Vec<CapabilityLease> {
        let mut leases = self
            .leases_guard()
            .values()
            .filter(|lease| same_tenant_user(&lease.scope, scope))
            .cloned()
            .collect::<Vec<_>>();
        leases.sort_by_key(|lease| lease.grant.id.as_uuid());
        leases
    }

    fn active_leases_for_context(&self, context: &ExecutionContext) -> Vec<CapabilityLease> {
        self.leases_for_scope(&context.resource_scope)
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

impl<S> CapabilityDispatchAuthorizer for LeaseBackedAuthorizer<'_, S>
where
    S: CapabilityLeaseStore + ?Sized,
{
    fn authorize_dispatch(
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

        let lease_grants = self.leases.active_grants_for_context(context);
        authorize_from_grants(
            context,
            descriptor,
            context.grants.grants.iter().chain(lease_grants.iter()),
        )
    }

    fn authorize_spawn(
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

        let lease_grants = self.leases.active_grants_for_context(context);
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
