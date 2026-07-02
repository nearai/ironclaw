use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MultiAgentError {
    #[error("maximum recursion depth {max_depth} reached")]
    MaxDepthExceeded { max_depth: u32 },
    #[error("maximum iteration budget {max_iterations} exhausted")]
    MaxIterationsExceeded { max_iterations: u32 },
    #[error("task timed out after {timeout_secs}s")]
    TaskTimeout { timeout_secs: u64 },
    #[error("delegation cycle detected involving task {task_id}")]
    CycleDetected { task_id: String },
    #[error("subagent `{agent_id}` failed: {reason}")]
    SubAgentFailed { agent_id: String, reason: String },
    #[error("orchestration failed: {reason}")]
    OrchestrationFailed { reason: String },
    #[error("agent job not found: {job_id}")]
    JobNotFound { job_id: String },
}
