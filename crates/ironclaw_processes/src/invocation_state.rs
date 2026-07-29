use std::sync::Arc;

use crate::{
    ClaimProcessesRequest, FailProcessRequest, GetProcessSnapshotRequest, JournaledProcessSnapshot,
    ProcessCheckpointRef, ProcessJournalStoreError, ProcessKind, ProcessLeaseRequest,
    ProcessLifecycleStatus, ProcessRuntimePort, ProcessStateTransitionRequest, ProcessSuspension,
    ProcessSuspensionKind, ProcessWorkerId, ResumeProcessRequest, SubmitProcessRequest,
    SuspendProcessRequest,
};
use async_trait::async_trait;
use ironclaw_events::sanitize_error_kind;
use ironclaw_host_api::{
    ApprovalRequest, ApprovalRequestId, CapabilityId, InvocationId, ProcessId, ResourceScope,
    SanitizedFailure, TurnGateRef, UserId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const CAPABILITY_RUN_WORKER: &str = "capability-invocation";
const CAPABILITY_RUN_RECORD: &str = "capability_run";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessInvocationStatus {
    Running,
    BlockedApproval,
    BlockedAuth,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessInvocationRecord {
    pub invocation_id: InvocationId,
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authenticated_actor_user_id: Option<UserId>,
    pub status: ProcessInvocationStatus,
    pub approval_request_id: Option<ApprovalRequestId>,
    pub error_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInvocationStart {
    pub invocation_id: InvocationId,
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub authenticated_actor_user_id: Option<UserId>,
}

#[derive(Debug, Error)]
pub enum ProcessInvocationError {
    #[error("unknown invocation {invocation_id}")]
    UnknownInvocation { invocation_id: InvocationId },
    #[error("invocation {invocation_id} already exists")]
    InvocationAlreadyExists { invocation_id: InvocationId },
    #[error("process invocation serialization failed: {0}")]
    Serialization(String),
    #[error("process invocation deserialization failed: {0}")]
    Deserialization(String),
    #[error("process invocation backend failed: {0}")]
    Backend(String),
}

#[async_trait]
pub trait ProcessInvocationStatePort: Send + Sync {
    async fn start(
        &self,
        start: ProcessInvocationStart,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError>;

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError>;

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError>;

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError>;

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError>;

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<ProcessInvocationRecord>, ProcessInvocationError>;

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessInvocationRecord>, ProcessInvocationError>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CapabilityRunMetadata {
    record_type: String,
    invocation_id: InvocationId,
    capability_id: CapabilityId,
    authenticated_actor_user_id: Option<UserId>,
    approval_request_id: Option<ApprovalRequestId>,
    error_kind: Option<String>,
}

pub struct ProcessInvocationStore {
    processes: Arc<dyn ProcessRuntimePort>,
}

impl ProcessInvocationStore {
    pub fn new(processes: Arc<dyn ProcessRuntimePort>) -> Self {
        Self { processes }
    }

    fn process_id(invocation_id: InvocationId) -> ProcessId {
        ProcessId::from_uuid(invocation_id.as_uuid())
    }

    fn checkpoint_ref(invocation_id: InvocationId) -> ProcessCheckpointRef {
        ProcessCheckpointRef::from_trusted(format!("capability-run-{invocation_id}"))
    }

    fn metadata(start: ProcessInvocationStart) -> CapabilityRunMetadata {
        CapabilityRunMetadata {
            record_type: CAPABILITY_RUN_RECORD.to_string(),
            invocation_id: start.invocation_id,
            capability_id: start.capability_id,
            authenticated_actor_user_id: start.authenticated_actor_user_id,
            approval_request_id: None,
            error_kind: None,
        }
    }

    fn encode_metadata(
        metadata: &CapabilityRunMetadata,
    ) -> Result<serde_json::Value, ProcessInvocationError> {
        serde_json::to_value(metadata)
            .map_err(|error| ProcessInvocationError::Serialization(error.to_string()))
    }

    fn decode_metadata(
        snapshot: &JournaledProcessSnapshot,
    ) -> Result<Option<CapabilityRunMetadata>, ProcessInvocationError> {
        if snapshot
            .metadata
            .get("record_type")
            .and_then(serde_json::Value::as_str)
            != Some(CAPABILITY_RUN_RECORD)
        {
            return Ok(None);
        }
        let metadata =
            serde_json::from_value::<CapabilityRunMetadata>(snapshot.metadata.clone())
                .map_err(|error| ProcessInvocationError::Deserialization(error.to_string()))?;
        Ok(Some(metadata))
    }

    fn record(
        snapshot: JournaledProcessSnapshot,
    ) -> Result<Option<ProcessInvocationRecord>, ProcessInvocationError> {
        let Some(metadata) = Self::decode_metadata(&snapshot)? else {
            return Ok(None);
        };
        let status = match snapshot.status {
            ProcessLifecycleStatus::Queued | ProcessLifecycleStatus::Running => {
                ProcessInvocationStatus::Running
            }
            ProcessLifecycleStatus::Suspended => match snapshot.suspension.as_ref().map(|s| s.kind)
            {
                Some(ProcessSuspensionKind::Approval) => ProcessInvocationStatus::BlockedApproval,
                Some(ProcessSuspensionKind::Authorization) => ProcessInvocationStatus::BlockedAuth,
                _ => ProcessInvocationStatus::Failed,
            },
            ProcessLifecycleStatus::Completed => ProcessInvocationStatus::Completed,
            ProcessLifecycleStatus::StopRequested
            | ProcessLifecycleStatus::CancelRequested
            | ProcessLifecycleStatus::Stopped
            | ProcessLifecycleStatus::Cancelled
            | ProcessLifecycleStatus::Failed
            | ProcessLifecycleStatus::Killed
            | ProcessLifecycleStatus::RecoveryRequired => ProcessInvocationStatus::Failed,
        };
        Ok(Some(ProcessInvocationRecord {
            invocation_id: metadata.invocation_id,
            capability_id: metadata.capability_id,
            scope: snapshot.scope,
            authenticated_actor_user_id: metadata.authenticated_actor_user_id,
            status,
            approval_request_id: metadata.approval_request_id,
            error_kind: metadata.error_kind,
        }))
    }

    async fn snapshot(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<JournaledProcessSnapshot, ProcessInvocationError> {
        self.processes
            .get_process_snapshot(GetProcessSnapshotRequest {
                scope: scope.clone(),
                process_id: Self::process_id(invocation_id),
            })
            .await
            .map_err(|error| map_process_error(error, invocation_id))
    }

    fn lease(
        snapshot: &JournaledProcessSnapshot,
    ) -> Result<ProcessLeaseRequest, ProcessInvocationError> {
        let lease = snapshot
            .lease
            .as_ref()
            .ok_or(ProcessInvocationError::Backend(
                "running capability process has no lease".to_string(),
            ))?;
        Ok(ProcessLeaseRequest {
            process_id: snapshot.process_id,
            worker_id: lease.worker_id.clone(),
            lease_token: lease.lease_token.clone(),
        })
    }

    async fn claim(
        &self,
        scope: ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<JournaledProcessSnapshot, ProcessInvocationError> {
        self.processes
            .claim_next_processes(ClaimProcessesRequest {
                worker_id: ProcessWorkerId::from_trusted(CAPABILITY_RUN_WORKER),
                scope_filter: Some(scope),
                process_id_filter: Some(Self::process_id(invocation_id)),
                process_kind_filter: Some(ProcessKind::CapabilityInvocationState),
                max_processes: 1,
            })
            .await
            .map_err(|error| map_process_error(error, invocation_id))?
            .into_iter()
            .next()
            .map(|claim| claim.state)
            .ok_or(ProcessInvocationError::UnknownInvocation { invocation_id })
    }

    async fn running_snapshot(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<JournaledProcessSnapshot, ProcessInvocationError> {
        let snapshot = self.snapshot(scope, invocation_id).await?;
        match snapshot.status {
            ProcessLifecycleStatus::Running => Ok(snapshot),
            ProcessLifecycleStatus::Queued => self.claim(scope.clone(), invocation_id).await,
            ProcessLifecycleStatus::Suspended => {
                self.processes
                    .resume_process(ResumeProcessRequest {
                        scope: scope.clone(),
                        process_id: snapshot.process_id,
                        operation_id: None,
                        expected_cursor: Some(snapshot.journal_cursor),
                        checkpoint_ref: snapshot.checkpoint_ref,
                        metadata: None,
                    })
                    .await
                    .map_err(|error| map_process_error(error, invocation_id))?;
                self.claim(scope.clone(), invocation_id).await
            }
            _ => Err(ProcessInvocationError::Backend(format!(
                "capability process {invocation_id} is terminal"
            ))),
        }
    }

    async fn suspend(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        metadata: CapabilityRunMetadata,
        suspension: ProcessSuspension,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        let snapshot = self.running_snapshot(scope, invocation_id).await?;
        let lease = Self::lease(&snapshot)?;
        let snapshot = self
            .processes
            .suspend_process(SuspendProcessRequest {
                process_id: snapshot.process_id,
                worker_id: lease.worker_id,
                lease_token: lease.lease_token,
                checkpoint_ref: Self::checkpoint_ref(invocation_id),
                suspension,
                metadata: Some(Self::encode_metadata(&metadata)?),
            })
            .await
            .map_err(|error| map_process_error(error, invocation_id))?;
        Self::record(snapshot)?.ok_or(ProcessInvocationError::UnknownInvocation { invocation_id })
    }
}

#[async_trait]
impl ProcessInvocationStatePort for ProcessInvocationStore {
    async fn start(
        &self,
        start: ProcessInvocationStart,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        let invocation_id = start.invocation_id;
        let scope = start.scope.clone();
        let owner_user_id = start.authenticated_actor_user_id.clone();
        let metadata = Self::encode_metadata(&Self::metadata(start))?;
        self.processes
            .submit_process(SubmitProcessRequest {
                process_id: Self::process_id(invocation_id),
                process_kind: ProcessKind::CapabilityInvocationState,
                scope: scope.clone(),
                exclusive_within_scope: false,
                operation_id: None,
                owner_user_id,
                concurrency_class: None,
                parent_process_id: None,
                root_process_id: None,
                spawn_tree_descendant_cap: None,
                dependency: None,
                checkpoint_ref: None,
                input: None,
                created_at: chrono::Utc::now(),
                metadata,
            })
            .await
            .map_err(|error| map_process_error(error, invocation_id))?;
        let snapshot = self.claim(scope, invocation_id).await?;
        Self::record(snapshot)?.ok_or(ProcessInvocationError::UnknownInvocation { invocation_id })
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        let snapshot = self.snapshot(scope, invocation_id).await?;
        let mut metadata = Self::decode_metadata(&snapshot)?
            .ok_or(ProcessInvocationError::UnknownInvocation { invocation_id })?;
        metadata.approval_request_id = Some(approval.id);
        metadata.error_kind = None;
        self.suspend(
            scope,
            invocation_id,
            metadata,
            ProcessSuspension {
                kind: ProcessSuspensionKind::Approval,
                gate_ref: Some(
                    TurnGateRef::new(format!("gate:approval-{}", approval.id))
                        .map_err(ProcessInvocationError::Backend)?,
                ),
                activity_id: None,
                credential_requirements: Vec::new(),
                detail: None,
            },
        )
        .await
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        let snapshot = self.snapshot(scope, invocation_id).await?;
        let mut metadata = Self::decode_metadata(&snapshot)?
            .ok_or(ProcessInvocationError::UnknownInvocation { invocation_id })?;
        metadata.approval_request_id = None;
        metadata.error_kind = Some(error_kind);
        self.suspend(
            scope,
            invocation_id,
            metadata,
            ProcessSuspension {
                kind: ProcessSuspensionKind::Authorization,
                gate_ref: None,
                activity_id: None,
                credential_requirements: Vec::new(),
                detail: None,
            },
        )
        .await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        let snapshot = self.running_snapshot(scope, invocation_id).await?;
        let mut metadata = Self::decode_metadata(&snapshot)?
            .ok_or(ProcessInvocationError::UnknownInvocation { invocation_id })?;
        metadata.approval_request_id = None;
        metadata.error_kind = None;
        let snapshot = self
            .processes
            .complete_process(ProcessStateTransitionRequest {
                lease: Self::lease(&snapshot)?,
                metadata: Some(Self::encode_metadata(&metadata)?),
            })
            .await
            .map_err(|error| map_process_error(error, invocation_id))?;
        Self::record(snapshot)?.ok_or(ProcessInvocationError::UnknownInvocation { invocation_id })
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        let snapshot = self.running_snapshot(scope, invocation_id).await?;
        let mut metadata = Self::decode_metadata(&snapshot)?
            .ok_or(ProcessInvocationError::UnknownInvocation { invocation_id })?;
        metadata.approval_request_id = None;
        metadata.error_kind = Some(error_kind.clone());
        let failure = SanitizedFailure::new(sanitize_error_kind(error_kind))
            .unwrap_or_else(|_| SanitizedFailure::from_trusted_static("unknown_failure"));
        let lease = Self::lease(&snapshot)?;
        let snapshot = self
            .processes
            .fail_process(FailProcessRequest {
                process_id: snapshot.process_id,
                worker_id: lease.worker_id,
                lease_token: lease.lease_token,
                failure,
                checkpoint_ref: snapshot.checkpoint_ref.clone(),
                metadata: Some(Self::encode_metadata(&metadata)?),
            })
            .await
            .map_err(|error| map_process_error(error, invocation_id))?;
        Self::record(snapshot)?.ok_or(ProcessInvocationError::UnknownInvocation { invocation_id })
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<ProcessInvocationRecord>, ProcessInvocationError> {
        match self.snapshot(scope, invocation_id).await {
            Ok(snapshot) => Self::record(snapshot),
            Err(ProcessInvocationError::UnknownInvocation { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessInvocationRecord>, ProcessInvocationError> {
        let snapshots = self
            .processes
            .process_snapshots(scope)
            .await
            .map_err(|error| ProcessInvocationError::Backend(error.to_string()))?;
        let mut records = Vec::new();
        for snapshot in snapshots {
            if snapshot.process_kind == ProcessKind::CapabilityInvocationState
                && let Some(record) = Self::record(snapshot)?
            {
                records.push(record);
            }
        }
        records.sort_by_key(|record| record.invocation_id.as_uuid());
        Ok(records)
    }
}

fn map_process_error(
    error: ProcessJournalStoreError,
    invocation_id: InvocationId,
) -> ProcessInvocationError {
    match error {
        ProcessJournalStoreError::UnknownProcess { .. } => {
            ProcessInvocationError::UnknownInvocation { invocation_id }
        }
        ProcessJournalStoreError::ProcessAlreadyExists { .. } => {
            ProcessInvocationError::InvocationAlreadyExists { invocation_id }
        }
        other => ProcessInvocationError::Backend(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProcessJournalSource;
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        MountAlias, MountGrant, MountPermissions, MountView, TenantId, VirtualPath,
    };

    fn process_store() -> (
        ProcessInvocationStore,
        Arc<crate::ProcessJournalStore<InMemoryBackend>>,
    ) {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/processes").unwrap(),
            VirtualPath::new("/engine/processes").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap();
        let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::new()),
            mounts,
        ));
        let journal = Arc::new(crate::ProcessJournalStore::new(filesystem));
        let runtime = Arc::clone(&journal) as Arc<dyn ProcessRuntimePort>;
        (ProcessInvocationStore::new(runtime), journal)
    }

    fn scope(invocation_id: InvocationId) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant").unwrap(),
            user_id: UserId::new("user").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id,
        }
    }

    #[tokio::test]
    async fn blocked_auth_and_completion_are_process_journal_transitions() {
        let (store, journal) = process_store();
        let invocation_id = InvocationId::new();
        let scope = scope(invocation_id);

        let running = store
            .start(ProcessInvocationStart {
                invocation_id,
                capability_id: CapabilityId::new("echo.say").unwrap(),
                scope: scope.clone(),
                authenticated_actor_user_id: Some(scope.user_id.clone()),
            })
            .await
            .unwrap();
        assert_eq!(running.status, ProcessInvocationStatus::Running);

        let blocked = store
            .block_auth(&scope, invocation_id, "credential_required".to_string())
            .await
            .unwrap();
        assert_eq!(blocked.status, ProcessInvocationStatus::BlockedAuth);

        let completed = store.complete(&scope, invocation_id).await.unwrap();
        assert_eq!(completed.status, ProcessInvocationStatus::Completed);

        let page = journal
            .read_process_journal_after(&scope, None, None, 16)
            .await
            .unwrap();
        let kinds = page
            .entries
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>();
        assert!(
            page.entries
                .iter()
                .all(|entry| entry.process_kind == ProcessKind::CapabilityInvocationState)
        );
        assert_eq!(
            kinds,
            vec![
                crate::ProcessJournalKind::Submitted,
                crate::ProcessJournalKind::Claimed,
                crate::ProcessJournalKind::Suspended,
                crate::ProcessJournalKind::Resumed,
                crate::ProcessJournalKind::Claimed,
                crate::ProcessJournalKind::Completed,
            ]
        );
    }
}
