use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use ironclaw_host_api::{InvocationId, ResourceScope};
use ironclaw_run_state::{ApprovalRequestStore, RunStateError, RunStateStore, RunStatus};
use ironclaw_turns::TurnRunId;

use super::{ApprovalGateRecord, ApprovalInteractionScope, approval_gate_ref};
use crate::error::ProductWorkflowError;

#[async_trait]
pub trait ApprovalInteractionReadModel: Send + Sync {
    async fn approval_gates(
        &self,
        scope: &ApprovalInteractionScope,
    ) -> Result<Vec<ApprovalGateRecord>, ProductWorkflowError>;
}

/// Read-model backed directly by canonical run-state and approval records.
pub struct RunStateApprovalInteractionReadModel {
    run_state: Arc<dyn RunStateStore>,
    approval_requests: Arc<dyn ApprovalRequestStore>,
}

impl RunStateApprovalInteractionReadModel {
    pub fn new(
        run_state: Arc<dyn RunStateStore>,
        approval_requests: Arc<dyn ApprovalRequestStore>,
    ) -> Self {
        Self {
            run_state,
            approval_requests,
        }
    }
}

#[async_trait]
impl ApprovalInteractionReadModel for RunStateApprovalInteractionReadModel {
    async fn approval_gates(
        &self,
        scope: &ApprovalInteractionScope,
    ) -> Result<Vec<ApprovalGateRecord>, ProductWorkflowError> {
        let owner_scope = resource_scope_for_interaction(scope);
        let approvals = self
            .approval_requests
            .records_for_scope(&owner_scope)
            .await
            .map_err(map_approval_read_error)?
            .into_iter()
            .map(|record| (record.request.id, record))
            .collect::<HashMap<_, _>>();
        if approvals.is_empty() {
            return Ok(Vec::new());
        }

        let mut gates = Vec::new();
        for run in self
            .run_state
            .records_for_scope(&owner_scope)
            .await
            .map_err(map_approval_read_error)?
        {
            if run.status != RunStatus::BlockedApproval {
                continue;
            }
            let Some(request_id) = run.approval_request_id else {
                continue;
            };
            let Some(approval) = approvals.get(&request_id) else {
                continue;
            };
            if approval.scope != run.scope {
                continue;
            }
            gates.push(ApprovalGateRecord::with_status(
                approval.scope.clone(),
                TurnRunId::from_uuid(run.invocation_id.as_uuid()),
                approval_gate_ref(request_id)?,
                approval.request.clone(),
                approval.status,
            )?);
        }
        Ok(gates)
    }
}

fn resource_scope_for_interaction(scope: &ApprovalInteractionScope) -> ResourceScope {
    ResourceScope {
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        mission_id: None,
        thread_id: Some(scope.thread_id.clone()),
        invocation_id: InvocationId::new(),
    }
}

fn map_approval_read_error(_error: RunStateError) -> ProductWorkflowError {
    ProductWorkflowError::Transient {
        reason: "approval read model unavailable".to_string(),
    }
}
