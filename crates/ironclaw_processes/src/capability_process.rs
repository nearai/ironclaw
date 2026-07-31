//! Capability process submission and views over the authoritative journal.

use ironclaw_events::sanitize_error_kind;
use ironclaw_host_api::{
    authorized::ProcessAuthorizedContinuation,
    capability::CapabilitySet,
    ids::{CapabilityId, ExtensionId, InvocationId, ProcessId, ResourceReservationId, UserId},
    mount::MountView,
    resource::{ResourceEstimate, ResourceScope},
    runtime::RuntimeKind,
    turn::SanitizedFailure,
};
use serde::{Deserialize, Serialize};

use crate::types::{ProcessError, ProcessRecord, ProcessStart, ProcessStatus};
use crate::{
    ClaimProcessesRequest, FailProcessRequest, GetProcessSnapshotRequest, ProcessInputPayload,
    ProcessInputRef, ProcessInputSubmission, ProcessJournalStoreError, ProcessKind,
    ProcessLeaseRequest, ProcessLifecycleStatus, ProcessRuntimePort, ProcessStateTransitionRequest,
    ProcessWorkerId, SubmitProcessRequest,
};

const CAPABILITY_INPUT_REF: &str = "capability-invocation:v1";
const CAPABILITY_PROJECTION_WORKER_ID: &str = "capability-projection";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CapabilityProcessMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) parent_process_id: Option<ProcessId>,
    pub(crate) invocation_id: InvocationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) authenticated_actor_user_id: Option<UserId>,
    pub(crate) extension_id: ExtensionId,
    pub(crate) capability_id: CapabilityId,
    #[serde(deserialize_with = "ironclaw_host_api::runtime::deserialize_trusted_runtime_kind")]
    pub(crate) runtime: RuntimeKind,
    pub(crate) grants: CapabilitySet,
    pub(crate) mounts: MountView,
    pub(crate) estimated_resources: ResourceEstimate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) resource_reservation_id: Option<ResourceReservationId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) authorized_continuation: Option<ProcessAuthorizedContinuation>,
}

pub fn process_record_from_snapshot(
    snapshot: crate::JournaledProcessSnapshot,
) -> Result<ProcessRecord, ProcessError> {
    let metadata: CapabilityProcessMetadata = serde_json::from_value(snapshot.metadata)
        .map_err(|error| ProcessError::Deserialization(error.to_string()))?;
    Ok(ProcessRecord {
        process_id: snapshot.process_id,
        parent_process_id: metadata.parent_process_id.or(snapshot.parent_process_id),
        invocation_id: metadata.invocation_id,
        scope: snapshot.scope,
        authenticated_actor_user_id: metadata.authenticated_actor_user_id,
        extension_id: metadata.extension_id,
        capability_id: metadata.capability_id,
        runtime: metadata.runtime,
        status: process_status(snapshot.status),
        grants: metadata.grants,
        mounts: metadata.mounts,
        estimated_resources: metadata.estimated_resources,
        resource_reservation_id: metadata.resource_reservation_id,
        authorized_continuation: metadata.authorized_continuation,
        error_kind: snapshot.failure.map(SanitizedFailure::into_category),
    })
}

pub async fn submit_capability_process(
    runtime: &dyn ProcessRuntimePort,
    start: ProcessStart,
) -> Result<ProcessRecord, ProcessError> {
    let process_id = start.process_id;
    let scope = start.scope.clone();
    let input = serde_json::to_vec(&start.input)
        .map_err(|error| ProcessError::Serialization(error.to_string()))
        .and_then(|payload| {
            ProcessInputPayload::new(payload)
                .map_err(|error| ProcessError::Serialization(error.to_string()))
        })?;
    let metadata = serde_json::to_value(CapabilityProcessMetadata {
        parent_process_id: start.parent_process_id,
        invocation_id: start.invocation_id,
        authenticated_actor_user_id: start.authenticated_actor_user_id,
        extension_id: start.extension_id,
        capability_id: start.capability_id,
        runtime: start.runtime,
        grants: start.grants,
        mounts: start.mounts,
        estimated_resources: start.estimated_resources,
        resource_reservation_id: start.resource_reservation_id,
        authorized_continuation: start.authorized_continuation,
    })
    .map_err(|error| ProcessError::Serialization(error.to_string()))?;
    let snapshot = runtime
        .submit_process(SubmitProcessRequest {
            process_id,
            process_kind: ProcessKind::CapabilityInvocation,
            scope,
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: None,
            concurrency_class: None,
            // Capability causality is persisted in typed capability metadata,
            // not in the spawn-tree reservation relation. Capability work does
            // not reserve a subagent descendant slot.
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: Some(ProcessInputSubmission {
                input_ref: ProcessInputRef::from_trusted(CAPABILITY_INPUT_REF),
                payload: input,
            }),
            created_at: chrono::Utc::now(),
            metadata,
        })
        .await
        .map_err(map_process_journal_error)?;
    process_record_from_snapshot(snapshot)
}

pub async fn capability_process_record(
    runtime: &dyn ProcessRuntimePort,
    scope: &ResourceScope,
    process_id: ProcessId,
) -> Result<Option<ProcessRecord>, ProcessError> {
    match runtime
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: scope.clone(),
            process_id,
        })
        .await
    {
        Ok(snapshot) => process_record_from_snapshot(snapshot).map(Some),
        Err(error) => match map_process_journal_error(error) {
            ProcessError::UnknownProcess { .. } => Ok(None),
            error => Err(error),
        },
    }
}

pub async fn complete_capability_process(
    runtime: &dyn ProcessRuntimePort,
    scope: &ResourceScope,
    process_id: ProcessId,
) -> Result<ProcessRecord, ProcessError> {
    let snapshot = claim_capability_process(
        runtime,
        capability_process_snapshot(runtime, scope, process_id).await?,
    )
    .await?;
    if snapshot.status != ProcessLifecycleStatus::Running {
        return Err(ProcessError::InvalidTransition {
            process_id,
            from: process_status(snapshot.status),
            to: ProcessStatus::Completed,
        });
    }
    let snapshot = runtime
        .complete_process(ProcessStateTransitionRequest {
            lease: lease_request(&snapshot)?,
            metadata: None,
        })
        .await
        .map_err(map_process_journal_error)?;
    process_record_from_snapshot(snapshot)
}

pub async fn fail_capability_process(
    runtime: &dyn ProcessRuntimePort,
    scope: &ResourceScope,
    process_id: ProcessId,
    error_kind: String,
) -> Result<ProcessRecord, ProcessError> {
    let snapshot = claim_capability_process(
        runtime,
        capability_process_snapshot(runtime, scope, process_id).await?,
    )
    .await?;
    if snapshot.status != ProcessLifecycleStatus::Running {
        return Err(ProcessError::InvalidTransition {
            process_id,
            from: process_status(snapshot.status),
            to: ProcessStatus::Failed,
        });
    }
    let lease = lease_request(&snapshot)?;
    let sanitized = sanitize_error_kind(error_kind);
    let failure = SanitizedFailure::new(sanitized)
        .unwrap_or_else(|_| SanitizedFailure::from_trusted_static("unknown_failure"));
    let snapshot = runtime
        .fail_process(FailProcessRequest {
            process_id,
            worker_id: lease.worker_id,
            lease_token: lease.lease_token,
            failure,
            recovery: crate::ProcessFailureRecovery::Terminal,
            checkpoint_ref: None,
            metadata: None,
        })
        .await
        .map_err(map_process_journal_error)?;
    process_record_from_snapshot(snapshot)
}

async fn capability_process_snapshot(
    runtime: &dyn ProcessRuntimePort,
    scope: &ResourceScope,
    process_id: ProcessId,
) -> Result<crate::JournaledProcessSnapshot, ProcessError> {
    runtime
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: scope.clone(),
            process_id,
        })
        .await
        .map_err(map_process_journal_error)
}

async fn claim_capability_process(
    runtime: &dyn ProcessRuntimePort,
    snapshot: crate::JournaledProcessSnapshot,
) -> Result<crate::JournaledProcessSnapshot, ProcessError> {
    if snapshot.status != ProcessLifecycleStatus::Queued {
        return Ok(snapshot);
    }
    let process_id = snapshot.process_id;
    runtime
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: ProcessWorkerId::from_trusted(CAPABILITY_PROJECTION_WORKER_ID),
            scope_filter: Some(snapshot.scope),
            process_id_filter: Some(process_id),
            process_kind_filter: Some(ProcessKind::CapabilityInvocation),
            max_processes: 1,
        })
        .await
        .map_err(map_process_journal_error)?
        .into_iter()
        .next()
        .map(|claimed| claimed.state)
        .ok_or_else(|| ProcessError::InvalidStoredRecord {
            reason: format!("submitted process {process_id} was not claimable"),
        })
}

fn lease_request(
    snapshot: &crate::JournaledProcessSnapshot,
) -> Result<ProcessLeaseRequest, ProcessError> {
    let lease = snapshot
        .lease
        .as_ref()
        .ok_or_else(|| ProcessError::InvalidStoredRecord {
            reason: format!(
                "running process {} has no journal lease",
                snapshot.process_id
            ),
        })?;
    Ok(ProcessLeaseRequest {
        process_id: snapshot.process_id,
        worker_id: lease.worker_id.clone(),
        lease_token: lease.lease_token.clone(),
    })
}

pub(crate) fn process_status(status: ProcessLifecycleStatus) -> ProcessStatus {
    match status {
        ProcessLifecycleStatus::Queued
        | ProcessLifecycleStatus::Running
        | ProcessLifecycleStatus::Suspended
        | ProcessLifecycleStatus::StopRequested
        | ProcessLifecycleStatus::CancelRequested => ProcessStatus::Running,
        ProcessLifecycleStatus::Completed => ProcessStatus::Completed,
        ProcessLifecycleStatus::Failed | ProcessLifecycleStatus::RecoveryRequired => {
            ProcessStatus::Failed
        }
        ProcessLifecycleStatus::Stopped
        | ProcessLifecycleStatus::Cancelled
        | ProcessLifecycleStatus::Killed => ProcessStatus::Killed,
    }
}

pub fn map_process_journal_error(error: ProcessJournalStoreError) -> ProcessError {
    match error {
        ProcessJournalStoreError::UnknownProcess { process_id } => {
            ProcessError::UnknownProcess { process_id }
        }
        ProcessJournalStoreError::ProcessAlreadyExists { process_id } => {
            ProcessError::ProcessAlreadyExists { process_id }
        }
        ProcessJournalStoreError::InvalidTransition {
            process_id,
            from,
            to,
        } => ProcessError::InvalidTransition {
            process_id,
            from: process_status(from),
            to: process_status(to),
        },
        ProcessJournalStoreError::Filesystem(error) => ProcessError::Filesystem(error),
        ProcessJournalStoreError::Serialization(reason) => ProcessError::Serialization(reason),
        ProcessJournalStoreError::Deserialization(reason) => ProcessError::Deserialization(reason),
        other => ProcessError::InvalidStoredRecord {
            reason: other.to_string(),
        },
    }
}
