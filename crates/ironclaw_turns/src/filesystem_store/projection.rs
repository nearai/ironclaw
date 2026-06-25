use crate::{TurnPersistenceSnapshot, TurnRunId, TurnRunRecord, TurnScope};

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
    let Some(parent) = snapshot.runs.iter().find(|record| record.run_id == run_id) else {
        return Vec::new();
    };
    if parent.scope != *scope {
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
