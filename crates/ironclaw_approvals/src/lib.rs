//! Approval resolution service for IronClaw Reborn.
//!
//! `ironclaw_approvals` resolves durable approval requests and issues scoped
//! authorization leases. It does not prompt users, execute capabilities, or
//! dispatch runtime work.

use ironclaw_authorization::{CapabilityLease, CapabilityLeaseStore};
use ironclaw_host_api::{
    Action, CapabilityGrant, CapabilityGrantId, EffectKind, GrantConstraints, Principal,
    ResourceScope, Timestamp,
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
}

impl<'a, A, L> ApprovalResolver<'a, A, L>
where
    A: ApprovalRequestStore + ?Sized,
    L: CapabilityLeaseStore + ?Sized,
{
    pub fn new(approvals: &'a A, leases: &'a L) -> Self {
        Self { approvals, leases }
    }

    pub async fn approve_dispatch(
        &self,
        scope: &ResourceScope,
        request_id: ironclaw_host_api::ApprovalRequestId,
        approval: LeaseApproval,
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

        let Action::Dispatch { capability, .. } = record.request.action.as_ref() else {
            return Err(ApprovalResolutionError::UnsupportedAction);
        };

        let grant = CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: capability.clone(),
            grantee: record.request.requested_by.clone(),
            issued_by: approval.issued_by,
            constraints: GrantConstraints {
                allowed_effects: approval.allowed_effects,
                mounts: Default::default(),
                network: Default::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: approval.expires_at,
                max_invocations: approval.max_invocations,
            },
        };
        self.approvals.approve(scope, request_id).await?;
        Ok(self
            .leases
            .issue(CapabilityLease::new(record.scope.clone(), grant)))
    }

    pub async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ironclaw_host_api::ApprovalRequestId,
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

        self.approvals
            .deny(scope, request_id)
            .await
            .map_err(Into::into)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseApproval {
    pub issued_by: Principal,
    pub allowed_effects: Vec<EffectKind>,
    pub expires_at: Option<Timestamp>,
    pub max_invocations: Option<u64>,
}

#[derive(Debug, Error)]
pub enum ApprovalResolutionError {
    #[error("approval store failed: {0}")]
    RunState(#[from] RunStateError),
    #[error("approval request is not pending: {status:?}")]
    NotPending { status: ApprovalStatus },
    #[error("approval action cannot issue a dispatch lease")]
    UnsupportedAction,
}
