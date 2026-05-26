use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{ProcessId, ResourceScope};
use ironclaw_processes::{
    ProcessCancellationToken, ProcessExecutionError, ProcessExecutionRequest,
    ProcessExecutionResult, ProcessExecutor,
};
use serde::Serialize;
use serde_json::json;
use thiserror::Error;

use crate::{
    ProcessSandboxPlanError, SandboxProcessPhase, SandboxProcessPlan, ValidatedSandboxProcessPlan,
};

/// Backend execution request for a validated sandbox process.
///
/// The request carries host-owned identity and cancellation state alongside a
/// validated plan. Backends should use `scope` for audit and resource ownership,
/// not as a source of extra authority.
#[derive(Debug, Clone)]
pub struct SandboxProcessRequest {
    pub process_id: ProcessId,
    pub scope: ResourceScope,
    pub plan: ValidatedSandboxProcessPlan,
    pub cancellation: ProcessCancellationToken,
}

/// Completed sandbox process result returned by a backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxProcessResult {
    pub output: SandboxProcessOutput,
}

/// Ordered output from each executed sandbox phase.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SandboxProcessOutput {
    pub phases: Vec<SandboxPhaseOutput>,
}

/// Serializable output for one install or run phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SandboxPhaseOutput {
    pub phase: SandboxProcessPhase,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub wall_clock_ms: u64,
}

/// Stable process sandbox failure kinds exposed through `ProcessExecutor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessSandboxErrorKind {
    InvalidProcessSandboxPlan,
    DockerSpawnFailed,
    DockerIoFailed,
    Cancelled,
    Timeout,
}

impl ProcessSandboxErrorKind {
    /// Returns the stable machine-readable error kind string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidProcessSandboxPlan => "invalid_process_sandbox_plan",
            Self::DockerSpawnFailed => "docker_spawn_failed",
            Self::DockerIoFailed => "docker_io_failed",
            Self::Cancelled => "cancelled",
            Self::Timeout => "timeout",
        }
    }
}

impl std::fmt::Display for ProcessSandboxErrorKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Backend-level sandbox execution error.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("process sandbox execution failed: {kind}")]
pub struct ProcessSandboxError {
    pub kind: ProcessSandboxErrorKind,
}

impl ProcessSandboxError {
    /// Constructs a sandbox execution error from a stable kind.
    pub fn new(kind: ProcessSandboxErrorKind) -> Self {
        Self { kind }
    }
}

impl From<ProcessSandboxPlanError> for ProcessSandboxError {
    fn from(_: ProcessSandboxPlanError) -> Self {
        Self::new(ProcessSandboxErrorKind::InvalidProcessSandboxPlan)
    }
}

/// Backend contract for executing validated process sandbox requests.
///
/// Implementations own the physical isolation mechanism. They must not accept
/// raw Docker flags, raw host paths, or raw secret material from plan JSON.
#[async_trait]
pub trait ProcessSandboxBackend: Send + Sync {
    /// Executes the validated sandbox request.
    async fn execute(
        &self,
        request: SandboxProcessRequest,
    ) -> Result<SandboxProcessResult, ProcessSandboxError>;
}

/// `ProcessExecutor` adapter for the process sandbox backend.
///
/// This adapter owns JSON deserialization and validation so generic process
/// callers receive the same stable error kind for malformed or invalid plans.
#[derive(Clone)]
pub struct ProcessSandboxExecutor {
    backend: Arc<dyn ProcessSandboxBackend>,
}

impl ProcessSandboxExecutor {
    /// Constructs a process executor over a sandbox backend.
    pub fn new(backend: Arc<dyn ProcessSandboxBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl ProcessExecutor for ProcessSandboxExecutor {
    async fn execute(
        &self,
        request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        let plan = serde_json::from_value::<SandboxProcessPlan>(request.input)
            .map_err(|_| ProcessExecutionError::new("invalid_process_sandbox_plan"))?;
        let plan = ValidatedSandboxProcessPlan::new(plan)
            .map_err(|_| ProcessExecutionError::new("invalid_process_sandbox_plan"))?;
        let result = self
            .backend
            .execute(SandboxProcessRequest {
                process_id: request.process_id,
                scope: request.scope,
                plan,
                cancellation: request.cancellation,
            })
            .await
            .map_err(|error| ProcessExecutionError::new(error.kind.as_str()))?;
        Ok(ProcessExecutionResult {
            output: json!({
                "kind": "process_sandbox_result",
                "phases": result.output.phases,
            }),
        })
    }
}
