use ironclaw_host_api::turn::{TurnGateRef, TurnRunId, TurnScope};
use ironclaw_host_api::{
    ids::ProcessId,
    resource::{ResourceScope, SYSTEM_RESERVED_ID},
};
use ironclaw_processes::ProcessGateRecord;

pub(crate) fn current_turn_gate_runs(
    records: impl IntoIterator<Item = ProcessGateRecord>,
) -> Vec<(TurnRunId, TurnGateRef)> {
    let mut runs = records
        .into_iter()
        .filter_map(|record| {
            record
                .suspension
                .gate_ref
                .map(|gate_ref| (turn_run_id_from_process_id(record.process_id), gate_ref))
        })
        .collect::<Vec<_>>();
    runs.sort_by_key(|(run_id, _)| run_id.as_uuid());
    runs
}

pub(crate) fn first_turn_run_for_gate(
    records: impl IntoIterator<Item = ProcessGateRecord>,
) -> Option<TurnRunId> {
    let mut runs = records
        .into_iter()
        .map(|record| {
            (
                record.historical,
                turn_run_id_from_process_id(record.process_id),
            )
        })
        .collect::<Vec<_>>();
    runs.sort_by_key(|(historical, run_id)| (*historical, run_id.as_uuid()));
    runs.dedup_by_key(|(_, run_id)| *run_id);
    runs.into_iter().map(|(_, run_id)| run_id).next()
}

pub(crate) fn turn_run_id_from_process_id(process_id: ProcessId) -> TurnRunId {
    TurnRunId::from_uuid(process_id.as_uuid())
}

pub(crate) fn turn_scope_from_process_gate(scope: &ResourceScope) -> Option<TurnScope> {
    let thread_id = scope.thread_id.clone()?;
    let owner = if scope.user_id.as_str() == SYSTEM_RESERVED_ID {
        None
    } else {
        Some(scope.user_id.clone())
    };
    Some(TurnScope::new_with_owner(
        scope.tenant_id.clone(),
        scope.agent_id.clone(),
        scope.project_id.clone(),
        thread_id,
        owner,
    ))
}
