use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthFlowOwnerScope, AuthFlowRecord, AuthFlowRecordSource, AuthGateRef, TurnGateAuthFlowQuery,
    TurnRunRef, flow_matches_turn_gate_query,
};
use ironclaw_product_workflow::{
    AuthGateRecord, AuthInteractionReadModel, AuthInteractionRejectionKind, AuthInteractionScope,
    AuthInteractionService, ListPendingAuthInteractionsRequest,
    ListPendingAuthInteractionsResponse, ProductWorkflowError, ResolveAuthInteractionRequest,
    ResolveAuthInteractionResponse,
};
use ironclaw_turns::{GateRef, TurnPersistenceSnapshot, TurnRunId, TurnScope, TurnStatus};

use crate::turn_run_snapshot::TurnRunSnapshotSource;

#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockedAuthRun {
    run_id: TurnRunId,
    gate_ref: GateRef,
}

pub(super) struct SnapshotAuthInteractionReadModel {
    /// A trait object (not the concrete `ComposedTurnStateStore`) so a
    /// caller can substitute a different turn-state store's snapshot view —
    /// see `turn_run_snapshot::TurnRunSnapshotSource` and
    /// `build_webui_auth_interaction_service_with_turn_run_source`.
    turn_state: Arc<dyn TurnRunSnapshotSource>,
    flow_records: Arc<dyn AuthFlowRecordSource>,
}

pub(super) struct UnavailableAuthInteractionService;

#[async_trait]
impl AuthInteractionService for UnavailableAuthInteractionService {
    async fn list_pending(
        &self,
        _request: ListPendingAuthInteractionsRequest,
    ) -> Result<ListPendingAuthInteractionsResponse, ProductWorkflowError> {
        Err(auth_read_model_unavailable())
    }

    async fn resolve(
        &self,
        _request: ResolveAuthInteractionRequest,
    ) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError> {
        Err(auth_read_model_unavailable())
    }
}

impl SnapshotAuthInteractionReadModel {
    pub(super) fn new(
        turn_state: Arc<dyn TurnRunSnapshotSource>,
        flow_records: Arc<dyn AuthFlowRecordSource>,
    ) -> Self {
        Self {
            turn_state,
            flow_records,
        }
    }

    async fn snapshot(&self) -> Result<TurnPersistenceSnapshot, ProductWorkflowError> {
        self.turn_state.turn_run_snapshot().await.map_err(|error| {
            tracing::debug!(
                %error,
                "auth interaction read model could not read turn persistence snapshot"
            );
            auth_read_model_unavailable()
        })
    }

    async fn blocked_auth_runs(
        &self,
        scope: &AuthInteractionScope,
    ) -> Result<Vec<BlockedAuthRun>, ProductWorkflowError> {
        let turn_scope = turn_scope_for_interaction(scope);
        let snapshot = self.snapshot().await?;
        let mut runs = snapshot
            .runs
            .iter()
            .filter(|run| {
                run.scope == turn_scope
                    && run.status == TurnStatus::BlockedAuth
                    && run.gate_ref.is_some()
            })
            .filter_map(|run| {
                run.gate_ref.clone().map(|gate_ref| BlockedAuthRun {
                    run_id: run.run_id,
                    gate_ref,
                })
            })
            .collect::<Vec<_>>();
        runs.sort_by_key(|run| run.run_id.as_uuid());
        Ok(runs)
    }

    async fn auth_run_for_gate(
        &self,
        scope: &AuthInteractionScope,
        gate_ref: &GateRef,
    ) -> Result<Option<TurnRunId>, ProductWorkflowError> {
        let turn_scope = turn_scope_for_interaction(scope);
        let snapshot = self.snapshot().await?;
        let active = snapshot
            .runs
            .iter()
            .find(|run| {
                run.scope == turn_scope
                    && run.status == TurnStatus::BlockedAuth
                    && run.gate_ref.as_ref() == Some(gate_ref)
            })
            .map(|run| run.run_id);
        if active.is_some() {
            return Ok(active);
        }

        let mut historical = snapshot
            .checkpoints
            .iter()
            .filter(|checkpoint| {
                checkpoint.status == TurnStatus::BlockedAuth
                    && &checkpoint.gate_ref == gate_ref
                    && checkpoint
                        .scope
                        .as_ref()
                        .is_none_or(|stored| stored == &turn_scope)
            })
            .filter_map(|checkpoint| {
                snapshot
                    .runs
                    .iter()
                    .find(|run| run.run_id == checkpoint.run_id && run.scope == turn_scope)
                    .map(|run| run.run_id)
            })
            .collect::<Vec<_>>();
        historical.sort_by_key(|run_id| run_id.as_uuid());
        historical.dedup();
        Ok(historical.into_iter().next())
    }

    async fn flow_for_gate(
        &self,
        scope: &AuthInteractionScope,
        run_id: TurnRunId,
        gate_ref: &GateRef,
    ) -> Result<Option<AuthFlowRecord>, ProductWorkflowError> {
        self.flow_records
            .flow_for_turn_gate(turn_gate_query(scope, run_id, gate_ref)?)
            .await
            .map_err(|error| {
                tracing::warn!(
                    %error,
                    %run_id,
                    gate_ref = %gate_ref.as_str(),
                    "local-dev auth read model failed to query flow for turn gate"
                );
                auth_read_model_unavailable()
            })
    }
}

fn owner_scope_for_interaction(scope: &AuthInteractionScope) -> AuthFlowOwnerScope {
    AuthFlowOwnerScope {
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        thread_id: scope.thread_id.clone(),
    }
}

fn turn_gate_query(
    scope: &AuthInteractionScope,
    run_id: TurnRunId,
    gate_ref: &GateRef,
) -> Result<TurnGateAuthFlowQuery, ProductWorkflowError> {
    Ok(TurnGateAuthFlowQuery {
        owner: owner_scope_for_interaction(scope),
        turn_run_ref: TurnRunRef::new(run_id.to_string())
            .map_err(|_| auth_read_model_unavailable())?,
        gate_ref: AuthGateRef::new(gate_ref.as_str()).map_err(|_| auth_read_model_unavailable())?,
        include_terminal: false,
    })
}

async fn flows_for_owner(
    source: &Arc<dyn AuthFlowRecordSource>,
    scope: &AuthInteractionScope,
) -> Result<Vec<AuthFlowRecord>, ProductWorkflowError> {
    source
        .flows_for_owner(owner_scope_for_interaction(scope))
        .await
        .map_err(|error| {
            tracing::warn!(
                %error,
                tenant_id = %scope.tenant_id.as_str(),
                user_id = %scope.user_id.as_str(),
                thread_id = %scope.thread_id.as_str(),
                "local-dev auth read model failed to query flows for owner"
            );
            auth_read_model_unavailable()
        })
}

fn matching_flow_for_run(
    flows: &[AuthFlowRecord],
    scope: &AuthInteractionScope,
    run_id: TurnRunId,
    gate_ref: &GateRef,
) -> Result<Option<AuthFlowRecord>, ProductWorkflowError> {
    let query = turn_gate_query(scope, run_id, gate_ref)?;
    Ok(flows
        .iter()
        .find(|flow| flow_matches_turn_gate_query(flow, &query))
        .cloned())
}

impl SnapshotAuthInteractionReadModel {
    async fn owner_flows(
        &self,
        scope: &AuthInteractionScope,
    ) -> Result<Vec<AuthFlowRecord>, ProductWorkflowError> {
        flows_for_owner(&self.flow_records, scope).await
    }
}

#[async_trait]
impl AuthInteractionReadModel for SnapshotAuthInteractionReadModel {
    async fn auth_gates(
        &self,
        scope: &AuthInteractionScope,
    ) -> Result<Vec<AuthGateRecord>, ProductWorkflowError> {
        let mut gates = Vec::new();
        let flows = self.owner_flows(scope).await?;
        for run in self.blocked_auth_runs(scope).await? {
            if let Some(flow) = matching_flow_for_run(&flows, scope, run.run_id, &run.gate_ref)? {
                gates.push(AuthGateRecord::new(run.run_id, run.gate_ref, flow)?);
            }
        }
        Ok(gates)
    }

    async fn auth_gate(
        &self,
        scope: &AuthInteractionScope,
        run_id_hint: Option<TurnRunId>,
        gate_ref: &GateRef,
    ) -> Result<Option<AuthGateRecord>, ProductWorkflowError> {
        let run_id = match run_id_hint {
            Some(run_id) => run_id,
            None => {
                let Some(run_id) = self.auth_run_for_gate(scope, gate_ref).await? else {
                    return Ok(None);
                };
                run_id
            }
        };
        let Some(flow) = self.flow_for_gate(scope, run_id, gate_ref).await? else {
            return Ok(None);
        };
        Ok(Some(AuthGateRecord::new(run_id, gate_ref.clone(), flow)?))
    }
}

fn turn_scope_for_interaction(scope: &AuthInteractionScope) -> TurnScope {
    TurnScope::new_with_owner(
        scope.tenant_id.clone(),
        scope.agent_id.clone(),
        scope.project_id.clone(),
        scope.thread_id.clone(),
        Some(scope.user_id.clone()),
    )
}

fn auth_read_model_unavailable() -> ProductWorkflowError {
    ProductWorkflowError::AuthInteractionRejected {
        kind: AuthInteractionRejectionKind::FlowUnavailable,
    }
}
