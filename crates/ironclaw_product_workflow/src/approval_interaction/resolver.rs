use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_approvals::{ApprovalResolutionError, ApprovalResolver, DenyApproval, LeaseApproval};
use ironclaw_authorization::CapabilityLeaseStore;
use ironclaw_events::AuditSink;
use ironclaw_host_api::{ApprovalRequestId, ResourceScope};
use ironclaw_run_state::{ApprovalRequestStore, RunStateError};

use super::{ApprovalInteractionRejectionKind, PendingApprovalGateRecord, approval_rejected};
use crate::error::ProductWorkflowError;

#[async_trait]
pub trait ApprovalLeaseTermsProvider: Send + Sync {
    async fn lease_terms_for(
        &self,
        gate: &PendingApprovalGateRecord,
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
