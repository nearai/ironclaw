//! Approval resolution service for IronClaw Reborn.
//!
//! `ironclaw_approvals` resolves durable approval requests and issues scoped
//! authorization leases. It does not prompt users, execute capabilities, or
//! dispatch runtime work.

use ironclaw_authorization::{CapabilityLease, CapabilityLeaseError, CapabilityLeaseStore};
use ironclaw_events::AuditSink;
use ironclaw_host_api::{
    Action, ApprovalRequestId, CapabilityGrant, CapabilityGrantId, CapabilityId, EffectKind,
    GrantConstraints, MountView, NetworkPolicy, Principal, ResourceCeiling, ResourceScope,
    SecretHandle, Timestamp,
};
use ironclaw_run_state::{ApprovalRecord, ApprovalRequestStore, ApprovalStatus, RunStateError};
use thiserror::Error;

pub struct ApprovalResolver<'a, A, L>
where
    A: ApprovalRequestStore + ?Sized,
    L: CapabilityLeaseStore + ?Sized,
{
    approvals: &'a A,
    leases: &'a L,
    audit_sink: Option<&'a dyn AuditSink>,
}

impl<'a, A, L> ApprovalResolver<'a, A, L>
where
    A: ApprovalRequestStore + ?Sized,
    L: CapabilityLeaseStore + ?Sized,
{
    pub fn new(approvals: &'a A, leases: &'a L) -> Self {
        Self {
            approvals,
            leases,
            audit_sink: None,
        }
    }

    pub fn with_audit_sink(mut self, audit_sink: &'a dyn AuditSink) -> Self {
        self.audit_sink = Some(audit_sink);
        self
    }

    pub async fn approve_dispatch(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<CapabilityLease, ApprovalResolutionError> {
        self.approve_capability_action(
            scope,
            request_id,
            approval,
            ApprovedCapabilityAction::Dispatch,
        )
        .await
    }

    pub async fn approve_spawn(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<CapabilityLease, ApprovalResolutionError> {
        self.approve_capability_action(scope, request_id, approval, ApprovedCapabilityAction::Spawn)
            .await
    }

    async fn approve_capability_action(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
        expected_action: ApprovedCapabilityAction,
    ) -> Result<CapabilityLease, ApprovalResolutionError> {
        let record = self
            .approvals
            .get(scope, request_id)
            .await?
            .ok_or(RunStateError::UnknownApprovalRequest { request_id })?;
        if record.status != ApprovalStatus::Pending {
            return Err(ApprovalResolutionError::NotPending {
                status: record.status,
            });
        }

        let capability = capability_for_action(record.request.action.as_ref(), expected_action)
            .ok_or(ApprovalResolutionError::UnsupportedAction)?;

        let invocation_fingerprint = record
            .request
            .invocation_fingerprint
            .clone()
            .ok_or(ApprovalResolutionError::MissingInvocationFingerprint)?;
        let resolved_by = approval.issued_by.clone();
        let grant = CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: capability.clone(),
            grantee: record.request.requested_by.clone(),
            issued_by: approval.issued_by,
            constraints: GrantConstraints {
                allowed_effects: approval.allowed_effects,
                mounts: approval.mounts,
                network: approval.network,
                secrets: approval.secrets,
                resource_ceiling: approval.resource_ceiling,
                expires_at: approval.expires_at,
                max_invocations: approval.max_invocations,
            },
        };
        let mut lease = CapabilityLease::new(record.scope.clone(), grant);
        lease.invocation_fingerprint = Some(invocation_fingerprint);
        let lease = self.leases.issue(lease).await?;
        if let Err(error) = self.approvals.approve(scope, request_id).await {
            let _ = self.leases.revoke(&lease.scope, lease.grant.id).await;
            return match error {
                RunStateError::ApprovalNotPending { status, .. } => {
                    Err(ApprovalResolutionError::NotPending { status })
                }
                error => Err(error.into()),
            };
        }
        self.emit_audit_best_effort(ironclaw_host_api::AuditEnvelope::approval_resolved(
            &record.scope,
            &record.request,
            resolved_by,
            "approved",
        ))
        .await;
        Ok(lease)
    }

    pub async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ironclaw_host_api::ApprovalRequestId,
        denial: DenyApproval,
    ) -> Result<ApprovalRecord, ApprovalResolutionError> {
        let record = self
            .approvals
            .get(scope, request_id)
            .await?
            .ok_or(RunStateError::UnknownApprovalRequest { request_id })?;
        if record.status != ApprovalStatus::Pending {
            return Err(ApprovalResolutionError::NotPending {
                status: record.status,
            });
        }

        let denied = match self.approvals.deny(scope, request_id).await {
            Ok(denied) => denied,
            Err(RunStateError::ApprovalNotPending { status, .. }) => {
                return Err(ApprovalResolutionError::NotPending { status });
            }
            Err(error) => return Err(error.into()),
        };
        self.emit_audit_best_effort(ironclaw_host_api::AuditEnvelope::approval_resolved(
            &denied.scope,
            &denied.request,
            denial.denied_by,
            "denied",
        ))
        .await;
        Ok(denied)
    }

    async fn emit_audit_best_effort(&self, record: ironclaw_host_api::AuditEnvelope) {
        if let Some(sink) = self.audit_sink {
            let _ = sink.emit_audit(record).await;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovedCapabilityAction {
    Dispatch,
    Spawn,
}

fn capability_for_action(
    action: &Action,
    expected: ApprovedCapabilityAction,
) -> Option<&CapabilityId> {
    match (expected, action) {
        (ApprovedCapabilityAction::Dispatch, Action::Dispatch { capability, .. })
        | (ApprovedCapabilityAction::Spawn, Action::SpawnCapability { capability, .. }) => {
            Some(capability)
        }
        _ => None,
    }
}

/// Approval resolution input supplied by a trusted human/admin policy surface.
///
/// `allowed_effects` and the constraint fields are the final attenuated grant
/// shape that the resolver stamps onto the resume-only lease. The current
/// [`ApprovalRequest`] shape does not carry the originating capability
/// descriptor's full grant constraints, so callers must derive these values from
/// the same reviewed descriptor/request they presented to the approver rather
/// than widening them in the UI layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseApproval {
    pub issued_by: Principal,
    pub allowed_effects: Vec<EffectKind>,
    pub mounts: MountView,
    pub network: NetworkPolicy,
    pub secrets: Vec<SecretHandle>,
    pub resource_ceiling: Option<ResourceCeiling>,
    pub expires_at: Option<Timestamp>,
    pub max_invocations: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenyApproval {
    pub denied_by: Principal,
}

#[derive(Debug, Error)]
pub enum ApprovalResolutionError {
    #[error("approval store failed: {0}")]
    RunState(#[from] RunStateError),
    #[error("approval request is not pending: {status:?}")]
    NotPending { status: ApprovalStatus },
    #[error("approval request is missing an invocation fingerprint")]
    MissingInvocationFingerprint,
    #[error("approval action cannot issue a dispatch lease")]
    UnsupportedAction,
    #[error("capability lease failed: {0}")]
    Lease(#[from] CapabilityLeaseError),
}
