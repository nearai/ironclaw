use crate::{
    GetLoopCheckpointRequest, GetRunStateRequest, LoopCheckpointRecord, TurnActor, TurnError,
    TurnPersistenceSnapshot, TurnRunId, TurnRunRecord, TurnRunState, TurnScope,
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

pub(super) fn run_state_parts(
    snapshot: &TurnPersistenceSnapshot,
    request: &GetRunStateRequest,
) -> Result<Option<(TurnRunRecord, TurnActor)>, TurnError> {
    let Some(run) = snapshot
        .runs
        .iter()
        .find(|record| record.run_id == request.run_id && record.scope == request.scope)
        .cloned()
    else {
        return Ok(None);
    };
    let actor = snapshot
        .turns
        .iter()
        .find(|record| record.turn_id == run.turn_id)
        .map(|record| record.actor.clone())
        .ok_or_else(|| TurnError::Unavailable {
            reason: "turn run references missing turn record".to_string(),
        })?;
    Ok(Some((run, actor)))
}

pub(super) fn run_state_from_record(run: TurnRunRecord, actor: TurnActor) -> TurnRunState {
    TurnRunState {
        scope: run.scope,
        actor: Some(actor),
        turn_id: run.turn_id,
        run_id: run.run_id,
        status: run.status,
        accepted_message_ref: run.accepted_message_ref,
        source_binding_ref: run.source_binding_ref,
        reply_target_binding_ref: run.reply_target_binding_ref,
        resolved_run_profile_id: run.profile.id,
        resolved_run_profile_version: run.profile.version,
        resolved_model_route: run.resolved_model_route,
        received_at: run.received_at,
        checkpoint_id: run.checkpoint_id,
        gate_ref: run.gate_ref,
        blocked_activity_id: run.blocked_activity_id,
        credential_requirements: run.credential_requirements,
        failure: run.failure,
        event_cursor: run.event_cursor,
        product_context: run.product_context,
        resume_disposition: run.resume_disposition,
    }
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
