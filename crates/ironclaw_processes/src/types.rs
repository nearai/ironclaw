//! Shared process lifecycle data types, errors, and traits.
//!
//! This module is the public interface surface most other crates import. The
//! lifecycle/storage backends, host helpers, and decorators that depend on
//! these types live in sibling modules.

use async_trait::async_trait;
use ironclaw_filesystem::FilesystemError;
use ironclaw_host_api::{
    AgentId, CapabilityId, CapabilitySet, ExtensionId, HostApiError, InvocationId, MissionId,
    MountView, ProcessId, ProjectId, ResourceEstimate, ResourceReservation, ResourceReservationId,
    ResourceScope, RuntimeKind, TenantId, ThreadId, UserId, VirtualPath,
};
use ironclaw_resources::ResourceError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::cancellation::ProcessCancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessStatus {
    Running,
    Completed,
    Failed,
    Killed,
}

impl ProcessStatus {
    pub fn is_terminal(self) -> bool {
        self != Self::Running
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessRecord {
    pub process_id: ProcessId,
    pub parent_process_id: Option<ProcessId>,
    pub invocation_id: InvocationId,
    pub scope: ResourceScope,
    pub extension_id: ExtensionId,
    pub capability_id: CapabilityId,
    pub runtime: RuntimeKind,
    pub status: ProcessStatus,
    pub grants: CapabilitySet,
    pub mounts: MountView,
    pub estimated_resources: ResourceEstimate,
    pub resource_reservation_id: Option<ResourceReservationId>,
    pub error_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessStart {
    pub process_id: ProcessId,
    pub parent_process_id: Option<ProcessId>,
    pub invocation_id: InvocationId,
    pub scope: ResourceScope,
    pub extension_id: ExtensionId,
    pub capability_id: CapabilityId,
    pub runtime: RuntimeKind,
    pub grants: CapabilitySet,
    pub mounts: MountView,
    pub estimated_resources: ResourceEstimate,
    pub resource_reservation_id: Option<ResourceReservationId>,
    pub input: Value,
}

/// Terminal process state returned by host-facing await operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessExit {
    pub process_id: ProcessId,
    pub scope: ResourceScope,
    pub extension_id: ExtensionId,
    pub capability_id: CapabilityId,
    pub runtime: RuntimeKind,
    pub status: ProcessStatus,
    pub error_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessResultRecord {
    pub process_id: ProcessId,
    pub scope: ResourceScope,
    pub status: ProcessStatus,
    pub output: Option<Value>,
    pub output_ref: Option<VirtualPath>,
    pub error_kind: Option<String>,
}

impl ProcessExit {
    pub(crate) fn from_terminal(record: ProcessRecord) -> Self {
        debug_assert!(record.status.is_terminal());
        Self {
            process_id: record.process_id,
            scope: record.scope,
            extension_id: record.extension_id,
            capability_id: record.capability_id,
            runtime: record.runtime,
            status: record.status,
            error_kind: record.error_kind,
        }
    }
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("unknown process {process_id}")]
    UnknownProcess { process_id: ProcessId },
    #[error("process {process_id} already exists")]
    ProcessAlreadyExists { process_id: ProcessId },
    #[error("process {process_id} cannot transition from {from:?} to {to:?}")]
    InvalidTransition {
        process_id: ProcessId,
        from: ProcessStatus,
        to: ProcessStatus,
    },
    #[error("process {process_id} returned reservation {actual:?}, expected {expected}")]
    ResourceReservationMismatch {
        process_id: ProcessId,
        expected: ResourceReservationId,
        actual: Option<ResourceReservationId>,
    },
    #[error(
        "process {process_id} start cannot supply pre-existing resource reservation {reservation_id}"
    )]
    ResourceReservationAlreadyAssigned {
        process_id: ProcessId,
        reservation_id: ResourceReservationId,
    },
    #[error(
        "process {process_id} resource reservation {reservation_id:?} is not owned by this store"
    )]
    ResourceReservationNotOwned {
        process_id: ProcessId,
        reservation_id: Option<ResourceReservationId>,
    },
    #[error("resource lifecycle error: {0}")]
    Resource(ResourceError),
    #[error("resource cleanup failed after process error: original={original}; cleanup={cleanup}")]
    ResourceCleanupFailed {
        original: Box<ProcessError>,
        cleanup: ResourceError,
    },
    #[error("process result store is not configured")]
    ProcessResultStoreUnavailable,
    #[error("process result is unavailable for {process_id}")]
    ProcessResultUnavailable { process_id: ProcessId },
    #[error("invalid stored process record: {reason}")]
    InvalidStoredRecord { reason: String },
    #[error("invalid storage path: {0}")]
    InvalidPath(String),
    #[error("filesystem error: {0}")]
    Filesystem(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("deserialization error: {0}")]
    Deserialization(String),
}

/// Wraps a `HostApiError` raised while constructing a virtual path into the
/// typed `InvalidPath` variant.
///
/// The broad `From<HostApiError> for ProcessError` impl was removed because it
/// silently mismapped every `HostApiError` (validation, invariant, mount,
/// network) to `InvalidPath`, poisoning error-kind classification downstream.
/// Use this helper at path-construction sites where the only `HostApiError`
/// shape is in fact a path/identifier validation failure.
pub(crate) fn invalid_path(error: HostApiError) -> ProcessError {
    ProcessError::InvalidPath(error.to_string())
}

impl From<FilesystemError> for ProcessError {
    fn from(error: FilesystemError) -> Self {
        Self::Filesystem(error.to_string())
    }
}

impl From<ResourceError> for ProcessError {
    fn from(error: ResourceError) -> Self {
        Self::Resource(error)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessExecutionRequest {
    pub process_id: ProcessId,
    pub invocation_id: InvocationId,
    pub scope: ResourceScope,
    pub extension_id: ExtensionId,
    pub capability_id: CapabilityId,
    pub runtime: RuntimeKind,
    pub estimate: ResourceEstimate,
    pub mounts: MountView,
    pub resource_reservation: Option<ResourceReservation>,
    pub input: Value,
    pub cancellation: ProcessCancellationToken,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessExecutionResult {
    pub output: Value,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("process execution failed: {kind}")]
pub struct ProcessExecutionError {
    pub kind: String,
}

impl ProcessExecutionError {
    pub fn new(kind: impl Into<String>) -> Self {
        Self { kind: kind.into() }
    }
}

#[async_trait]
pub trait ProcessExecutor: Send + Sync {
    /// Runs one background process request and must observe cooperative cancellation where possible.
    async fn execute(
        &self,
        request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError>;
}

#[async_trait]
pub trait ProcessManager: Send + Sync {
    /// Starts process lifecycle tracking before detached execution begins.
    async fn spawn(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError>;
}

#[async_trait]
pub trait ProcessResultStore: Send + Sync {
    /// Stores successful process output separately from the lifecycle record.
    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        output: Value,
    ) -> Result<ProcessResultRecord, ProcessError>;

    /// Stores a classified process failure without raw backend detail strings.
    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessResultRecord, ProcessError>;

    /// Stores killed process result metadata without implying executor preemption succeeded.
    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessResultRecord, ProcessError>;

    /// Loads scoped result metadata; wrong-scope lookups must look unknown.
    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessResultRecord>, ProcessError>;

    /// Loads scoped process output, keeping large/sensitive output outside lifecycle records.
    async fn output(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<Value>, ProcessError> {
        Ok(self
            .get(scope, process_id)
            .await?
            .and_then(|record| record.output))
    }
}

#[async_trait]
pub trait ProcessStore: Send + Sync {
    /// Persists a running process record without storing raw input.
    async fn start(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError>;

    /// Transitions a scoped running process to completed.
    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError>;

    /// Transitions a scoped running process to failed with a classified error kind.
    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError>;

    /// Marks a scoped process killed and must not reveal cross-tenant process existence.
    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError>;

    /// Loads scoped process lifecycle metadata; wrong-scope lookups must look unknown.
    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError>;

    /// Lists process lifecycle records visible to the exact resource-owner scope only.
    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessRecord>, ProcessError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ProcessKey {
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    mission_id: Option<MissionId>,
    thread_id: Option<ThreadId>,
    process_id: ProcessId,
}

impl ProcessKey {
    pub(crate) fn new(scope: &ResourceScope, process_id: ProcessId) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            mission_id: scope.mission_id.clone(),
            thread_id: scope.thread_id.clone(),
            process_id,
        }
    }
}

pub(crate) fn ensure_status_transition(
    process_id: ProcessId,
    from: ProcessStatus,
    to: ProcessStatus,
) -> Result<(), ProcessError> {
    if from != ProcessStatus::Running {
        return Err(ProcessError::InvalidTransition {
            process_id,
            from,
            to,
        });
    }
    Ok(())
}

pub(crate) fn same_scope_owner(left: &ResourceScope, right: &ResourceScope) -> bool {
    left.tenant_id == right.tenant_id
        && left.user_id == right.user_id
        && left.agent_id == right.agent_id
        && left.project_id == right.project_id
        && left.mission_id == right.mission_id
        && left.thread_id == right.thread_id
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: the broad `From<HostApiError> for ProcessError` impl previously
    /// mismapped every `HostApiError` shape (validation, invariant, mount, network)
    /// to `ProcessError::InvalidPath`, poisoning error-kind classification at
    /// path-construction call sites. The replacement `invalid_path` helper must
    /// only be used where the call site has already established that a
    /// `HostApiError` represents a path validation failure, and it must preserve
    /// the original message in the `InvalidPath` payload.
    #[test]
    fn invalid_path_helper_maps_host_api_error_into_invalid_path_variant() {
        let host_error = VirtualPath::new("not-absolute")
            .expect_err("non-absolute virtual paths must be rejected by VirtualPath::new");
        let host_error_message = host_error.to_string();

        let process_error = invalid_path(host_error);

        match process_error {
            ProcessError::InvalidPath(reason) => {
                assert_eq!(
                    reason, host_error_message,
                    "invalid_path helper must preserve the original HostApiError message"
                );
            }
            other => panic!(
                "invalid_path helper must map HostApiError to ProcessError::InvalidPath, got {other:?}"
            ),
        }
    }
}
