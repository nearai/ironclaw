use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_approvals::{ApprovalResolutionError, ApprovalResolver, DenyApproval, LeaseApproval};
use ironclaw_authorization::{CapabilityLeaseStatus, CapabilityLeaseStore};
use ironclaw_events::AuditSink;
use ironclaw_host_api::{Action, ApprovalRequestId, CapabilityId, ResourceScope};
use ironclaw_run_state::{ApprovalRequestStore, ApprovalStatus, RunStateError};

use super::{ApprovalGateRecord, ApprovalInteractionRejectionKind, approval_rejected};
use crate::error::ProductWorkflowError;

#[async_trait]
pub trait ApprovalLeaseTermsProvider: Send + Sync {
    async fn lease_terms_for(
        &self,
        gate: &ApprovalGateRecord,
    ) -> Result<LeaseApproval, ProductWorkflowError>;
}

#[async_trait]
pub trait ApprovalResolutionPort: Send + Sync {
    async fn approve_dispatch(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<(), ProductWorkflowError>;

    async fn approve_spawn(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<(), ProductWorkflowError>;

    async fn ensure_dispatch_lease(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<(), ProductWorkflowError>;

    async fn ensure_spawn_lease(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<(), ProductWorkflowError>;

    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        denial: DenyApproval,
    ) -> Result<(), ProductWorkflowError>;
}

pub struct ApprovalResolverPort {
    approvals: Arc<dyn ApprovalRequestStore>,
    leases: Arc<dyn CapabilityLeaseStore>,
    audit_sink: Option<Arc<dyn AuditSink>>,
}

impl ApprovalResolverPort {
    pub fn new(
        approvals: Arc<dyn ApprovalRequestStore>,
        leases: Arc<dyn CapabilityLeaseStore>,
    ) -> Self {
        Self {
            approvals,
            leases,
            audit_sink: None,
        }
    }

    pub fn with_audit_sink(mut self, audit_sink: Arc<dyn AuditSink>) -> Self {
        self.audit_sink = Some(audit_sink);
        self
    }

    fn resolver(&self) -> ApprovalResolver<'_, dyn ApprovalRequestStore, dyn CapabilityLeaseStore> {
        let mut resolver = ApprovalResolver::new(self.approvals.as_ref(), self.leases.as_ref());
        if let Some(audit_sink) = &self.audit_sink {
            resolver = resolver.with_audit_sink(audit_sink.as_ref());
        }
        resolver
    }

    async fn matching_lease_exists(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        expected_action: ApprovedApprovalAction,
    ) -> Result<bool, ProductWorkflowError> {
        let record = self
            .approvals
            .get(scope, request_id)
            .await
            .map_err(|error| {
                map_approval_resolution_error(ApprovalResolutionError::RunState(error))
            })?
            .ok_or_else(|| approval_rejected(ApprovalInteractionRejectionKind::MissingGate))?;
        if record.status != ApprovalStatus::Approved {
            return Err(approval_rejected(
                ApprovalInteractionRejectionKind::StaleGate,
            ));
        }
        let capability = capability_for_action(record.request.action.as_ref(), expected_action)?;
        let Some(fingerprint) = record.request.invocation_fingerprint.as_ref() else {
            return Err(approval_rejected(
                ApprovalInteractionRejectionKind::StaleGate,
            ));
        };
        Ok(self
            .leases
            .leases_for_scope(scope)
            .await
            .into_iter()
            .any(|lease| {
                lease.status == CapabilityLeaseStatus::Active
                    && lease.grant.capability == *capability
                    && lease.grant.grantee == record.request.requested_by
                    && lease.invocation_fingerprint.as_ref() == Some(fingerprint)
            }))
    }
}

#[async_trait]
impl ApprovalResolutionPort for ApprovalResolverPort {
    async fn approve_dispatch(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<(), ProductWorkflowError> {
        self.resolver()
            .approve_dispatch(scope, request_id, approval)
            .await
            .map(|_| ())
            .map_err(map_approval_resolution_error)
    }

    async fn approve_spawn(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<(), ProductWorkflowError> {
        self.resolver()
            .approve_spawn(scope, request_id, approval)
            .await
            .map(|_| ())
            .map_err(map_approval_resolution_error)
    }

    async fn ensure_dispatch_lease(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<(), ProductWorkflowError> {
        if self
            .matching_lease_exists(scope, request_id, ApprovedApprovalAction::Dispatch)
            .await?
        {
            return Ok(());
        }
        self.resolver()
            .retry_lease_issue_for_dispatch(scope, request_id, approval)
            .await
            .map(|_| ())
            .map_err(map_approval_resolution_error)
    }

    async fn ensure_spawn_lease(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<(), ProductWorkflowError> {
        if self
            .matching_lease_exists(scope, request_id, ApprovedApprovalAction::Spawn)
            .await?
        {
            return Ok(());
        }
        self.resolver()
            .retry_lease_issue_for_spawn(scope, request_id, approval)
            .await
            .map(|_| ())
            .map_err(map_approval_resolution_error)
    }

    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        denial: DenyApproval,
    ) -> Result<(), ProductWorkflowError> {
        self.resolver()
            .deny(scope, request_id, denial)
            .await
            .map(|_| ())
            .map_err(map_approval_resolution_error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovedApprovalAction {
    Dispatch,
    Spawn,
}

fn capability_for_action(
    action: &Action,
    expected_action: ApprovedApprovalAction,
) -> Result<&CapabilityId, ProductWorkflowError> {
    match (action, expected_action) {
        (Action::Dispatch { capability, .. }, ApprovedApprovalAction::Dispatch)
        | (Action::SpawnCapability { capability, .. }, ApprovedApprovalAction::Spawn) => {
            Ok(capability)
        }
        _ => Err(approval_rejected(
            ApprovalInteractionRejectionKind::UnsupportedAction,
        )),
    }
}

fn map_approval_resolution_error(error: ApprovalResolutionError) -> ProductWorkflowError {
    match error {
        ApprovalResolutionError::RunState(RunStateError::UnknownApprovalRequest { .. }) => {
            approval_rejected(ApprovalInteractionRejectionKind::MissingGate)
        }
        ApprovalResolutionError::RunState(RunStateError::ApprovalNotPending { .. })
        | ApprovalResolutionError::NotPending { .. }
        | ApprovalResolutionError::NotApproved { .. } => {
            approval_rejected(ApprovalInteractionRejectionKind::StaleGate)
        }
        ApprovalResolutionError::UnsupportedAction => {
            approval_rejected(ApprovalInteractionRejectionKind::UnsupportedAction)
        }
        ApprovalResolutionError::MissingInvocationFingerprint => {
            approval_rejected(ApprovalInteractionRejectionKind::StaleGate)
        }
        ApprovalResolutionError::RunState(_) | ApprovalResolutionError::Lease(_) => {
            ProductWorkflowError::Transient {
                reason: "approval resolver unavailable".to_string(),
            }
        }
    }
}
