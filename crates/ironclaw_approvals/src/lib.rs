//! Approval resolution service for IronClaw Reborn.
//!
//! `ironclaw_approvals` resolves durable approval requests and issues scoped
//! authorization leases. It does not prompt users, execute capabilities, or
//! dispatch runtime work.

mod policy;

use ironclaw_authorization::{CapabilityLease, CapabilityLeaseError, CapabilityLeaseStore};
use ironclaw_events::AuditSink;
use ironclaw_host_api::{
    Action, ApprovalDecisionKind, ApprovalRequestId, CapabilityGrant, CapabilityGrantId,
    CapabilityId, EffectKind, GrantConstraints, InvocationFingerprint, MountView, NetworkPolicy,
    Principal, ResourceCeiling, ResourceScope, SecretHandle, Timestamp,
};
use ironclaw_run_state::{ApprovalRecord, ApprovalRequestStore, ApprovalStatus, RunStateError};
use thiserror::Error;

pub use policy::{
    FilesystemPersistentApprovalPolicyStore, InMemoryPersistentApprovalPolicyStore,
    PersistentApprovalAction, PersistentApprovalPolicy, PersistentApprovalPolicyError,
    PersistentApprovalPolicyInput, PersistentApprovalPolicyKey, PersistentApprovalPolicyStore,
    PersistentApprovalScope, permission_mode_allows_persistent_approval,
    persistent_approval_grant_issuer,
};

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

    /// Retry lease issuance for a request that is already `Approved`.
    ///
    /// Closes the "approved but no lease" recovery window: if
    /// [`approve_dispatch`] / [`approve_spawn`] persisted the approval
    /// record but the subsequent `leases.issue(...)` call failed with a
    /// transient store error, the request status stays `Approved` and
    /// the caller can recover by calling this method with the same
    /// [`LeaseApproval`] terms. Idempotent on the approval record (no
    /// status flip); the underlying lease store generates a fresh
    /// `CapabilityGrantId` per call, so callers retrying after a partial
    /// success must compare against `leases_for_scope` if duplicate
    /// leases are unacceptable.
    pub async fn retry_lease_issue_for_dispatch(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<CapabilityLease, ApprovalResolutionError> {
        self.retry_lease_issue_for_action(
            scope,
            request_id,
            approval,
            ApprovedCapabilityAction::Dispatch,
        )
        .await
    }

    /// See [`retry_lease_issue_for_dispatch`] — same recovery path for
    /// spawn-action approvals.
    pub async fn retry_lease_issue_for_spawn(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<CapabilityLease, ApprovalResolutionError> {
        self.retry_lease_issue_for_action(
            scope,
            request_id,
            approval,
            ApprovedCapabilityAction::Spawn,
        )
        .await
    }

    async fn retry_lease_issue_for_action(
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
        if record.status != ApprovalStatus::Approved {
            return Err(ApprovalResolutionError::NotApproved {
                status: record.status,
            });
        }
        let capability = capability_for_action(record.request.action.as_ref(), expected_action)
            .ok_or(ApprovalResolutionError::UnsupportedAction)?
            .clone();
        let invocation_fingerprint = record
            .request
            .invocation_fingerprint
            .clone()
            .ok_or(ApprovalResolutionError::MissingInvocationFingerprint)?;
        self.issue_lease_for_approved(record, capability, approval, invocation_fingerprint)
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
            .ok_or(ApprovalResolutionError::UnsupportedAction)?
            .clone();

        let invocation_fingerprint = record
            .request
            .invocation_fingerprint
            .clone()
            .ok_or(ApprovalResolutionError::MissingInvocationFingerprint)?;

        // F2: persist the approval state *before* issuing the lease. The
        // approval record is the authority of record — once it flips to
        // `Approved`, any subsequent lease re-issue is a recoverable
        // operation against an already-decided request. The previous
        // order (issue lease, then approve, best-effort revoke on
        // failure) left a window where a transient store error could
        // produce a live lease whose approval status was still
        // `Pending`. See audit finding F2.
        //
        // Recovery path for the "approved but no lease" window when
        // `leases.issue(...)` fails after `approvals.approve(...)`
        // succeeded: callers can invoke [`retry_lease_issue_for_dispatch`]
        // / [`retry_lease_issue_for_spawn`] against the now-Approved
        // record.
        let approved_record = match self.approvals.approve(scope, request_id).await {
            Ok(record) => record,
            Err(RunStateError::ApprovalNotPending { status, .. }) => {
                return Err(ApprovalResolutionError::NotPending { status });
            }
            Err(error) => return Err(error.into()),
        };

        self.issue_lease_for_approved(
            approved_record,
            capability,
            approval,
            invocation_fingerprint,
        )
        .await
    }

    /// Shared lease-issuance path used by both the first-time approve and
    /// the retry-after-transient-failure recovery path. The approval
    /// record must already be in `Approved` status; the caller is
    /// responsible for that precondition.
    async fn issue_lease_for_approved(
        &self,
        approved_record: ApprovalRecord,
        capability: CapabilityId,
        approval: LeaseApproval,
        invocation_fingerprint: InvocationFingerprint,
    ) -> Result<CapabilityLease, ApprovalResolutionError> {
        let resolved_by = approval.issued_by.clone();
        let grant = CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability,
            grantee: approved_record.request.requested_by.clone(),
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
        let mut lease = CapabilityLease::new(approved_record.scope.clone(), grant);
        lease.invocation_fingerprint = Some(invocation_fingerprint);
        let lease = self.leases.issue(lease).await?;
        self.emit_approval_resolved(
            &approved_record.scope,
            &approved_record.request,
            resolved_by,
            ApprovalDecisionKind::Approved,
        )
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
        self.emit_approval_resolved(
            &denied.scope,
            &denied.request,
            denial.denied_by,
            ApprovalDecisionKind::Denied,
        )
        .await;
        Ok(denied)
    }

    /// Single emission path for approval-resolution audit events. Both
    /// `approve_*` and `deny` go through this so the audit envelope's
    /// scope/request/decision shape cannot diverge between resolution
    /// kinds. See audit finding F3.
    async fn emit_approval_resolved(
        &self,
        scope: &ResourceScope,
        request: &ironclaw_host_api::ApprovalRequest,
        resolved_by: Principal,
        decision: ApprovalDecisionKind,
    ) {
        self.emit_audit_best_effort(ironclaw_host_api::AuditEnvelope::approval_resolved(
            scope,
            request,
            resolved_by,
            decision,
        ))
        .await;
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
    /// Surfaced by [`retry_lease_issue_for_dispatch`] /
    /// [`retry_lease_issue_for_spawn`] when the approval record is not in
    /// `Approved` status — the retry path is only valid for requests that
    /// already cleared the first approve call.
    #[error("approval request is not approved: {status:?}")]
    NotApproved { status: ApprovalStatus },
    #[error("approval request is missing an invocation fingerprint")]
    MissingInvocationFingerprint,
    #[error("approval action cannot issue a dispatch lease")]
    UnsupportedAction,
    #[error("capability lease failed: {0}")]
    Lease(#[from] CapabilityLeaseError),
}
