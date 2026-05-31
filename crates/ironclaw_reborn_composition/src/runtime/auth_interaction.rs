use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{AuthContinuationRef, AuthFlowRecord, AuthFlowRecordSource};
use ironclaw_product_workflow::{
    AuthGateRecord, AuthInteractionReadModel, AuthInteractionRejectionKind, AuthInteractionScope,
    AuthInteractionService, ListPendingAuthInteractionsRequest,
    ListPendingAuthInteractionsResponse, ProductWorkflowError, ResolveAuthInteractionRequest,
    ResolveAuthInteractionResponse,
};
use ironclaw_turns::{
    GateRef, TurnActor, TurnPersistenceSnapshot, TurnRunId, TurnRunRecord, TurnScope, TurnStatus,
};

use crate::factory::LocalDevTurnStateStore;

#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockedAuthRun {
    run_id: TurnRunId,
    gate_ref: GateRef,
}

pub(super) struct LocalDevAuthInteractionReadModel {
    turn_state: Arc<LocalDevTurnStateStore>,
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

impl LocalDevAuthInteractionReadModel {
    pub(super) fn new(
        turn_state: Arc<LocalDevTurnStateStore>,
        flow_records: Arc<dyn AuthFlowRecordSource>,
    ) -> Self {
        Self {
            turn_state,
            flow_records,
        }
    }

    async fn snapshot(&self) -> Result<TurnPersistenceSnapshot, ProductWorkflowError> {
        #[cfg(feature = "libsql")]
        {
            self.turn_state
                .persistence_snapshot()
                .await
                .map_err(|_| auth_read_model_unavailable())
        }
        #[cfg(not(feature = "libsql"))]
        {
            Ok(self.turn_state.persistence_snapshot())
        }
    }

    async fn blocked_auth_runs(
        &self,
        scope: &AuthInteractionScope,
    ) -> Result<Vec<BlockedAuthRun>, ProductWorkflowError> {
        let turn_scope = turn_scope_for_interaction(scope);
        let actor = TurnActor::new(scope.user_id.clone());
        let snapshot = self.snapshot().await?;
        let mut runs = snapshot
            .runs
            .iter()
            .filter(|run| {
                run.scope == turn_scope
                    && run.status == TurnStatus::BlockedAuth
                    && run.gate_ref.is_some()
                    && snapshot_run_actor_matches(&snapshot, run, &actor)
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
        let actor = TurnActor::new(scope.user_id.clone());
        let snapshot = self.snapshot().await?;
        let active = snapshot
            .runs
            .iter()
            .find(|run| {
                run.scope == turn_scope
                    && run.status == TurnStatus::BlockedAuth
                    && run.gate_ref.as_ref() == Some(gate_ref)
                    && snapshot_run_actor_matches(&snapshot, run, &actor)
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
                    .find(|run| {
                        run.run_id == checkpoint.run_id
                            && run.scope == turn_scope
                            && snapshot_run_actor_matches(&snapshot, run, &actor)
                    })
                    .map(|run| run.run_id)
            })
            .collect::<Vec<_>>();
        historical.sort_by_key(|run_id| run_id.as_uuid());
        historical.dedup();
        Ok(historical.into_iter().next())
    }

    fn flow_for_gate(
        &self,
        scope: &AuthInteractionScope,
        run_id: TurnRunId,
        gate_ref: &GateRef,
    ) -> Option<AuthFlowRecord> {
        self.flow_records
            .flow_records_snapshot()
            .into_iter()
            .find(|flow| same_auth_owner(flow, scope) && flow_matches_gate(flow, run_id, gate_ref))
    }
}

#[async_trait]
impl AuthInteractionReadModel for LocalDevAuthInteractionReadModel {
    async fn auth_gates(
        &self,
        scope: &AuthInteractionScope,
    ) -> Result<Vec<AuthGateRecord>, ProductWorkflowError> {
        let mut gates = Vec::new();
        for run in self.blocked_auth_runs(scope).await? {
            if let Some(flow) = self.flow_for_gate(scope, run.run_id, &run.gate_ref) {
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
        let Some(flow) = self.flow_for_gate(scope, run_id, gate_ref) else {
            return Ok(None);
        };
        Ok(Some(AuthGateRecord::new(run_id, gate_ref.clone(), flow)?))
    }
}

fn turn_scope_for_interaction(scope: &AuthInteractionScope) -> TurnScope {
    TurnScope::new(
        scope.tenant_id.clone(),
        scope.agent_id.clone(),
        scope.project_id.clone(),
        scope.thread_id.clone(),
    )
}

fn same_auth_owner(flow: &AuthFlowRecord, scope: &AuthInteractionScope) -> bool {
    let resource = &flow.scope.resource;
    // Surface/session/invocation are not authority for this UI bridge; the
    // caller must own the tenant/user/agent/project/thread that is blocked.
    resource.tenant_id == scope.tenant_id
        && resource.user_id == scope.user_id
        && resource.agent_id == scope.agent_id
        && resource.project_id == scope.project_id
        && resource.mission_id.is_none()
        && resource.thread_id.as_ref() == Some(&scope.thread_id)
}

fn flow_matches_gate(flow: &AuthFlowRecord, run_id: TurnRunId, gate_ref: &GateRef) -> bool {
    let AuthContinuationRef::TurnGateResume {
        turn_run_ref,
        gate_ref: continuation_gate_ref,
    } = &flow.continuation
    else {
        return false;
    };
    turn_run_ref.as_str() == run_id.to_string()
        && continuation_gate_ref.as_str() == gate_ref.as_str()
}

fn snapshot_run_actor_matches(
    snapshot: &TurnPersistenceSnapshot,
    run: &TurnRunRecord,
    actor: &TurnActor,
) -> bool {
    snapshot
        .turns
        .iter()
        .any(|turn| turn.turn_id == run.turn_id && turn.scope == run.scope && turn.actor == *actor)
}

fn auth_read_model_unavailable() -> ProductWorkflowError {
    ProductWorkflowError::AuthInteractionRejected {
        kind: AuthInteractionRejectionKind::FlowUnavailable,
    }
}
