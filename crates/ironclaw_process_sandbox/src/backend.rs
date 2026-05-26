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
    SandboxPlanError, SandboxProcessPhase, SandboxProcessPlan, ValidatedSandboxProcessPlan,
};

#[derive(Debug, Clone)]
pub struct SandboxProcessRequest {
    pub process_id: ProcessId,
    pub scope: ResourceScope,
    pub plan: ValidatedSandboxProcessPlan,
    pub cancellation: ProcessCancellationToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxProcessResult {
    pub output: SandboxProcessOutput,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SandboxProcessOutput {
    pub phases: Vec<SandboxPhaseOutput>,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("process sandbox execution failed: {kind}")]
pub struct ProcessSandboxError {
    pub kind: String,
}

impl ProcessSandboxError {
    pub fn new(kind: impl Into<String>) -> Self {
        Self { kind: kind.into() }
    }
}

impl From<SandboxPlanError> for ProcessSandboxError {
    fn from(_: SandboxPlanError) -> Self {
        Self::new("invalid_process_sandbox_plan")
    }
}

#[async_trait]
pub trait ProcessSandboxBackend: Send + Sync {
    async fn execute(
        &self,
        request: SandboxProcessRequest,
    ) -> Result<SandboxProcessResult, ProcessSandboxError>;
}

#[derive(Clone)]
pub struct ProcessSandboxExecutor {
    backend: Arc<dyn ProcessSandboxBackend>,
}

impl ProcessSandboxExecutor {
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
            .map_err(|error| ProcessExecutionError::new(error.kind))?;
        Ok(ProcessExecutionResult {
            output: json!({
                "kind": "process_sandbox_result",
                "phases": result.output.phases,
            }),
        })
    }
}
