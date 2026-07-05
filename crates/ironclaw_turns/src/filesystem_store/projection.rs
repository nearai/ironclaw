use crate::{
    GetLoopCheckpointRequest, LoopCheckpointRecord, TurnPersistenceSnapshot, TurnRunId,
    TurnRunRecord, TurnScope,
};

/// Project the children of a run directly from a snapshot without building
/// an `InMemoryTurnStateStore`. Mirrors `InMemoryTurnStateStore::children_of`
/// scope semantics: returns an empty list when the parent is missing or out of
/// scope, filters children by the parent's scope envelope (tenant/agent/project),
/// and sorts by `received_at`.
pub(super) fn children_of(
    snapshot: &TurnPersistenceSnapshot,
    scope: &TurnScope,
    run_id: TurnRunId,
) -> Vec<TurnRunRecord> {
    if !snapshot
        .runs
        .iter()
        .any(|record| record.run_id == run_id && record.scope == *scope)
    {
        return Vec::new();
    }
    let mut children: Vec<TurnRunRecord> = snapshot
        .runs
        .iter()
        .filter(|record| {
            record.parent_run_id == Some(run_id)
                && record.scope.tenant_id == scope.tenant_id
                && record.scope.agent_id == scope.agent_id
                && record.scope.project_id == scope.project_id
        })
        .cloned()
        .collect();
    children.sort_by_key(|record| record.received_at);
    children
}

/// Project a run record by id directly from a snapshot, scoped exactly to
/// `scope`. Mirrors `InMemoryTurnStateStore::get_run_record` semantics.
pub(super) fn run_record(
    snapshot: &TurnPersistenceSnapshot,
    scope: &TurnScope,
    run_id: TurnRunId,
) -> Option<TurnRunRecord> {
    snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id && record.scope == *scope)
        .cloned()
}

/// Project a loop checkpoint directly from a snapshot without rebuilding an
/// `InMemoryTurnStateStore`. Mirrors `InMemoryTurnStateStore::get_loop_checkpoint`.
pub(super) fn loop_checkpoint(
    snapshot: &TurnPersistenceSnapshot,
    request: &GetLoopCheckpointRequest,
) -> Option<LoopCheckpointRecord> {
    snapshot
        .loop_checkpoints
        .iter()
        .find(|record| {
            record.scope == request.scope
                && record.turn_id == request.turn_id
                && record.run_id == request.run_id
                && record.checkpoint_id == request.checkpoint_id
        })
        .cloned()
}
