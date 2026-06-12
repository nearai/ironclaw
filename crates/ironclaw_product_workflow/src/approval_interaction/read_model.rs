use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::ResourceScope;
use ironclaw_run_state::{ApprovalRequestStore, RunStateError};
use ironclaw_turns::{GateRef, TurnRunId};

use super::gate_ref::{approval_gate_ref, approval_request_id_from_gate_ref};
use super::{ApprovalGateRecord, ApprovalInteractionScope};
use crate::error::ProductWorkflowError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalBlockedTurnRun {
    pub run_id: TurnRunId,
    pub gate_ref: GateRef,
}

#[async_trait]
pub trait ApprovalTurnRunLocator: Send + Sync {
    async fn blocked_approval_runs(
        &self,
        scope: &ApprovalInteractionScope,
    ) -> Result<Vec<ApprovalBlockedTurnRun>, ProductWorkflowError>;

    async fn blocked_approval_run(
        &self,
        scope: &ApprovalInteractionScope,
        gate_ref: &GateRef,
    ) -> Result<Option<TurnRunId>, ProductWorkflowError> {
        Ok(self
            .blocked_approval_runs(scope)
            .await?
            .into_iter()
            .find(|run| &run.gate_ref == gate_ref)
            .map(|run| run.run_id))
    }

    async fn approval_run_for_gate(
        &self,
        scope: &ApprovalInteractionScope,
        gate_ref: &GateRef,
    ) -> Result<Option<TurnRunId>, ProductWorkflowError> {
        self.blocked_approval_run(scope, gate_ref).await
    }
}

#[async_trait]
pub trait ApprovalInteractionReadModel: Send + Sync {
    async fn approval_gates(
        &self,
        scope: &ApprovalInteractionScope,
    ) -> Result<Vec<ApprovalGateRecord>, ProductWorkflowError>;

    async fn approval_gate(
        &self,
        scope: &ApprovalInteractionScope,
        run_id_hint: Option<TurnRunId>,
        gate_ref: &GateRef,
    ) -> Result<Option<ApprovalGateRecord>, ProductWorkflowError>;
}

/// Read-model backed by canonical approval records and parked turn state.
pub struct RunStateApprovalInteractionReadModel {
    approval_requests: Arc<dyn ApprovalRequestStore>,
    turn_runs: Arc<dyn ApprovalTurnRunLocator>,
}

impl RunStateApprovalInteractionReadModel {
    pub fn new(
        approval_requests: Arc<dyn ApprovalRequestStore>,
        turn_runs: Arc<dyn ApprovalTurnRunLocator>,
    ) -> Self {
        Self {
            approval_requests,
            turn_runs,
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
        let mut gates = Vec::new();
        for run in self.turn_runs.blocked_approval_runs(scope).await? {
            let request_id = approval_request_id_from_gate_ref(&run.gate_ref)?;
            let Some(approval) = self
                .approval_requests
                .get(&owner_scope, request_id)
                .await
                .map_err(map_approval_read_error)?
            else {
                continue;
            };
            if !same_interaction_owner(&approval.scope, &owner_scope) {
                continue;
            }
            gates.push(ApprovalGateRecord::with_status(
                approval.scope,
                run.run_id,
                run.gate_ref,
                approval.request,
                approval.status,
            )?);
        }
        Ok(gates)
    }

    async fn approval_gate(
        &self,
        scope: &ApprovalInteractionScope,
        run_id_hint: Option<TurnRunId>,
        gate_ref: &GateRef,
    ) -> Result<Option<ApprovalGateRecord>, ProductWorkflowError> {
        let request_id = approval_request_id_from_gate_ref(gate_ref)?;
        let owner_scope = resource_scope_for_interaction(scope);
        let Some(approval) = self
            .approval_requests
            .get(&owner_scope, request_id)
            .await
            .map_err(map_approval_read_error)?
        else {
            return Ok(None);
        };
        if !same_interaction_owner(&approval.scope, &owner_scope) {
            return Ok(None);
        }
        let run_id = match run_id_hint {
            Some(run_id) => run_id,
            None => {
                let Some(run_id) = self
                    .turn_runs
                    .approval_run_for_gate(scope, gate_ref)
                    .await?
                else {
                    return Ok(None);
                };
                run_id
            }
        };
        Ok(Some(ApprovalGateRecord::with_status(
            approval.scope,
            run_id,
            approval_gate_ref(request_id)?,
            approval.request,
            approval.status,
        )?))
    }
}

fn resource_scope_for_interaction(scope: &ApprovalInteractionScope) -> ResourceScope {
    scope.to_resource_scope()
}

fn same_interaction_owner(left: &ResourceScope, right: &ResourceScope) -> bool {
    left.tenant_id == right.tenant_id
        && left.user_id == right.user_id
        && left.agent_id == right.agent_id
        && left.project_id == right.project_id
        && left.mission_id == right.mission_id
        && left.thread_id == right.thread_id
}

fn map_approval_read_error(_error: RunStateError) -> ProductWorkflowError {
    ProductWorkflowError::Transient {
        reason: "approval read model unavailable".to_string(),
    }
}
