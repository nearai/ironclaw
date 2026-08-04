use ironclaw_host_api::ids::ProcessId;
use serde::{Deserialize, Serialize};

use super::{
    ProcessControlMutation, ProcessJournalMaterializedState, ProcessTransitionMutation, rows,
};
use crate::{
    ClaimProcessesRequest, CloseProcessDependencyRequest, OpenProcessDependencyRequest,
    ProcessConcurrencyLimits, ProcessLeaseRequest, PruneReleasedProcessRequest,
    RecordProcessCheckpointRequest, RecoverExpiredProcessLeasesRequest, ReleaseProcessTreeRequest,
    ReserveProcessTreeRequest, SettleProcessDependencyRequest, SubmitProcessRequest,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum StoredProcessCommand {
    ImportLegacyState(Box<ProcessJournalMaterializedState>),
    Submit(Box<SubmitProcessRequest>),
    SubmitWithCheckpoint {
        request: Box<SubmitProcessRequest>,
        checkpoint: Box<RecordProcessCheckpointRequest>,
    },
    Claim {
        request: ClaimProcessesRequest,
        now: ironclaw_host_api::Timestamp,
        lease_duration_millis: u64,
        lease_nonce: ProcessId,
        limits: ProcessConcurrencyLimits,
    },
    Heartbeat {
        request: ProcessLeaseRequest,
        now: ironclaw_host_api::Timestamp,
        lease_duration_millis: u64,
    },
    RecoverExpired(RecoverExpiredProcessLeasesRequest),
    LeasedTransition {
        request: ProcessLeaseRequest,
        mutation: ProcessTransitionMutation,
    },
    Control(ProcessControlMutation),
    ReserveTree(ReserveProcessTreeRequest),
    ReleaseTree(ReleaseProcessTreeRequest),
    PruneTree(PruneReleasedProcessRequest),
    OpenDependency(OpenProcessDependencyRequest),
    SettleDependency(SettleProcessDependencyRequest),
    ConsumeDependency(CloseProcessDependencyRequest),
    AbandonDependency(CloseProcessDependencyRequest),
    RecordCheckpoint(RecordProcessCheckpointRequest),
}

impl StoredProcessCommand {
    pub(super) fn cursor_reservation_count(&self, generated_entry_count: usize) -> usize {
        match self {
            // Legacy import preserves its own cursor assignments, but reserves
            // once so a fresh backend's sequence allocator is initialized for
            // the first row-native command.
            Self::ImportLegacyState(_) => 1,
            _ => generated_entry_count,
        }
    }

    /// Legacy import rewrites the cursor allocator wholesale and reserves a
    /// range the batch preview cannot account for, so it never shares a
    /// group-commit transaction with ordinary lifecycle commands.
    pub(super) fn requires_solo_transaction(&self) -> bool {
        matches!(self, Self::ImportLegacyState(_))
    }

    pub(super) fn load_references(
        &self,
    ) -> Result<rows::LoadReferences, super::ProcessJournalStoreError> {
        let mut references = rows::LoadReferences::default();
        match self {
            Self::ImportLegacyState(_) => {}
            Self::Submit(request) | Self::SubmitWithCheckpoint { request, .. } => {
                references.process_ids.push(request.process_id);
                references.process_ids.extend(request.parent_process_id);
                references.process_ids.extend(request.root_process_id);
                references.tree_roots.extend(request.root_process_id);
                references
                    .submission_idempotency_keys
                    .extend(submission_replay_key(request)?);
                references.active_conflicts.extend(
                    request
                        .exclusive_within_scope
                        .then(|| (request.scope.clone(), request.process_kind.clone())),
                );
                if let Some(dependency) = &request.dependency {
                    references.process_ids.push(dependency.dependent_process_id);
                    references.process_ids.push(dependency.root_process_id);
                    references.tree_roots.push(dependency.root_process_id);
                    references
                        .dependencies
                        .push((dependency.dependent_process_id, request.process_id));
                }
                if let Self::SubmitWithCheckpoint { checkpoint, .. } = self {
                    references
                        .checkpoints
                        .push(checkpoint.checkpoint_id.clone());
                }
            }
            Self::Claim {
                request, limits, ..
            } => {
                references.process_ids.extend(request.process_id_filter);
                references.claims.push((request.clone(), limits.clone()));
            }
            Self::RecoverExpired(request) => {
                references.recover_expired.push(request.clone());
            }
            Self::Heartbeat { request, .. } | Self::LeasedTransition { request, .. } => {
                references.process_ids.push(request.process_id);
            }
            Self::Control(mutation) => {
                references.process_ids.push(mutation.process_id);
                references
                    .control_idempotency_keys
                    .extend(control_replay_key(mutation));
            }
            Self::ReserveTree(request) => {
                references.process_ids.push(request.root_process_id);
                references.tree_roots.push(request.root_process_id);
            }
            Self::ReleaseTree(request) => {
                references.process_ids.push(request.root_process_id);
                references.process_ids.push(request.idempotency_process_id);
                references.tree_roots.push(request.root_process_id);
            }
            Self::PruneTree(request) => {
                references.process_ids.push(request.root_process_id);
                references.process_ids.push(request.process_id);
                references.tree_roots.push(request.root_process_id);
            }
            Self::OpenDependency(request) => {
                references.process_ids.push(request.dependent_process_id);
                references.process_ids.push(request.dependency_process_id);
                references.process_ids.push(request.root_process_id);
                references.tree_roots.push(request.root_process_id);
                references
                    .dependencies
                    .push((request.dependent_process_id, request.dependency_process_id));
            }
            Self::SettleDependency(request) => {
                add_dependency_references(
                    &mut references,
                    request.dependent_process_id,
                    request.dependency_process_id,
                );
            }
            Self::ConsumeDependency(request) | Self::AbandonDependency(request) => {
                add_dependency_references(
                    &mut references,
                    request.dependent_process_id,
                    request.dependency_process_id,
                );
            }
            Self::RecordCheckpoint(request) => {
                references.process_ids.push(request.process_id);
                references.checkpoints.push(request.checkpoint_id.clone());
            }
        }
        references.normalize();
        Ok(references)
    }
}

pub(super) fn submission_replay_key(
    request: &SubmitProcessRequest,
) -> Result<Option<String>, super::ProcessJournalStoreError> {
    request
        .operation_id
        .as_ref()
        .map(|operation_id| {
            let scope = &request.scope;
            serde_json::to_string(&(
                &scope.tenant_id,
                &scope.user_id,
                &scope.agent_id,
                &scope.project_id,
                &scope.mission_id,
                &scope.thread_id,
            ))
            .map(|scope| {
                format!(
                    "submit:{scope}:{:?}:{}",
                    request.process_kind,
                    operation_id.as_str()
                )
            })
            .map_err(|error| super::ProcessJournalStoreError::Serialization(error.to_string()))
        })
        .transpose()
}

pub(super) fn control_replay_key(mutation: &super::ProcessControlMutation) -> Option<String> {
    mutation.operation_id.as_ref().map(|id| {
        format!(
            "{}:{}:{}",
            mutation.action.as_str(),
            mutation.process_id,
            id.as_str()
        )
    })
}

fn add_dependency_references(
    references: &mut rows::LoadReferences,
    dependent_process_id: ProcessId,
    dependency_process_id: ProcessId,
) {
    references.process_ids.push(dependent_process_id);
    references.process_ids.push(dependency_process_id);
    references
        .dependencies
        .push((dependent_process_id, dependency_process_id));
}
