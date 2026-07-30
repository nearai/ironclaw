use ironclaw_host_api::{ProcessId, ResourceScope};

use super::{ProcessJournalMaterializedState, ProcessJournalStoreError};
use crate::{
    JournaledProcessSnapshot, ProcessConcurrencyLimits, ProcessGateOwnerMatch, ProcessGateQuery,
    ProcessLeaseToken, ProcessLifecycleStatus, ProcessWorkerId,
};

pub(super) fn ensure_transition(
    snapshot: &JournaledProcessSnapshot,
    to: ProcessLifecycleStatus,
) -> Result<(), ProcessJournalStoreError> {
    let valid = match (snapshot.status, to) {
        (ProcessLifecycleStatus::Queued, ProcessLifecycleStatus::Running)
        | (ProcessLifecycleStatus::Suspended, ProcessLifecycleStatus::Queued)
        | (ProcessLifecycleStatus::Running, ProcessLifecycleStatus::Running)
        | (ProcessLifecycleStatus::Running, ProcessLifecycleStatus::Suspended)
        | (ProcessLifecycleStatus::Running, ProcessLifecycleStatus::Completed)
        | (ProcessLifecycleStatus::Running, ProcessLifecycleStatus::Cancelled)
        | (ProcessLifecycleStatus::Running, ProcessLifecycleStatus::Failed)
        | (ProcessLifecycleStatus::Running, ProcessLifecycleStatus::Queued)
        | (ProcessLifecycleStatus::CancelRequested, ProcessLifecycleStatus::Cancelled) => true,
        (from, _) if from == to => true,
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(ProcessJournalStoreError::InvalidTransition {
            process_id: snapshot.process_id,
            from: snapshot.status,
            to,
        })
    }
}

pub(super) fn ensure_lease(
    snapshot: &JournaledProcessSnapshot,
    worker_id: &ProcessWorkerId,
    lease_token: &ProcessLeaseToken,
) -> Result<(), ProcessJournalStoreError> {
    let Some(lease) = &snapshot.lease else {
        return Err(ProcessJournalStoreError::InvalidLease {
            process_id: snapshot.process_id,
        });
    };
    if &lease.worker_id == worker_id && &lease.lease_token == lease_token {
        Ok(())
    } else {
        Err(ProcessJournalStoreError::InvalidLease {
            process_id: snapshot.process_id,
        })
    }
}

pub(super) fn process_gate_snapshot_matches(
    snapshot: &JournaledProcessSnapshot,
    request: &ProcessGateQuery,
) -> bool {
    let Some(suspension) = &snapshot.suspension else {
        return false;
    };
    let scope_matches = match request
        .scope_match
        .unwrap_or(crate::ProcessGateScopeMatch::Exact)
    {
        crate::ProcessGateScopeMatch::Exact => {
            crate::types::same_scope_owner(&snapshot.scope, &request.scope)
        }
        crate::ProcessGateScopeMatch::Owner => {
            snapshot.scope.tenant_id == request.scope.tenant_id
                && snapshot.scope.user_id == request.scope.user_id
        }
    };
    snapshot.status == ProcessLifecycleStatus::Suspended
        && suspension.kind == request.gate_kind
        && scope_matches
        && request
            .gate_ref
            .as_ref()
            .is_none_or(|gate_ref| suspension.gate_ref.as_ref() == Some(gate_ref))
        && request.owner_user_id.as_ref().is_none_or(|owner| {
            match request
                .owner_match
                .unwrap_or(ProcessGateOwnerMatch::Explicit)
            {
                ProcessGateOwnerMatch::Explicit | ProcessGateOwnerMatch::ExplicitOrActor => {
                    snapshot.owner_user_id.as_ref() == Some(owner)
                }
            }
        })
}

pub(super) fn process_claim_within_limits(
    state: &ProcessJournalMaterializedState,
    process_id: ProcessId,
    limits: &ProcessConcurrencyLimits,
) -> bool {
    let Some(candidate) = state.processes.get(&process_id) else {
        return false;
    };
    if let (Some(cap), Some(owner)) = (limits.max_running_per_owner, &candidate.owner_user_id) {
        let running_for_owner = state
            .processes
            .values()
            .filter(|snapshot| {
                snapshot.status == ProcessLifecycleStatus::Running
                    && snapshot.scope.tenant_id == candidate.scope.tenant_id
                    && snapshot.owner_user_id.as_ref() == Some(owner)
            })
            .count();
        if running_for_owner >= cap as usize {
            return false;
        }
    }
    let Some(class) = &candidate.concurrency_class else {
        return true;
    };
    let Some(cap) = limits.max_running_by_class.get(class) else {
        return true;
    };
    state
        .processes
        .values()
        .filter(|snapshot| {
            snapshot.status == ProcessLifecycleStatus::Running
                && snapshot.scope.tenant_id == candidate.scope.tenant_id
                && snapshot.concurrency_class.as_ref() == Some(class)
        })
        .count()
        < *cap as usize
}

pub(super) fn process_scope_visible(stored: &ResourceScope, requested: &ResourceScope) -> bool {
    crate::types::same_scope_owner(stored, requested)
}

pub(super) fn same_lineage_scope(left: &ResourceScope, right: &ResourceScope) -> bool {
    left.tenant_id == right.tenant_id
        && left.user_id == right.user_id
        && left.agent_id == right.agent_id
        && left.project_id == right.project_id
        && left.mission_id == right.mission_id
}

pub(super) fn validate_tree_root(
    state: &ProcessJournalMaterializedState,
    scope: &ResourceScope,
    root_process_id: ProcessId,
) -> Result<(), ProcessJournalStoreError> {
    let root =
        state
            .processes
            .get(&root_process_id)
            .ok_or(ProcessJournalStoreError::UnknownProcess {
                process_id: root_process_id,
            })?;
    if !same_lineage_scope(&root.scope, scope) {
        return Err(ProcessJournalStoreError::UnauthorizedScope);
    }
    if root.root_process_id.unwrap_or(root.process_id) != root.process_id {
        return Err(ProcessJournalStoreError::InvalidRequest(
            "root_process_id must identify the process tree root".to_string(),
        ));
    }
    Ok(())
}
