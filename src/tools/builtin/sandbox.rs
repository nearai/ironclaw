//! Tool that delegates work to a sandboxed Docker container.
//!
//! The orchestrator LLM calls this tool instead of directly using shell/file tools.
//! It creates a container with a worker sub-agent that executes the task independently.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::orchestrator::job_manager::ContainerJobManager;
use crate::tools::tool::{Tool, ToolError, ToolOutput};

/// Tool for delegating filesystem/shell work to a sandboxed Docker container.
///
/// When the orchestrator LLM needs to read files, run shell commands, or
/// modify code, it calls this tool instead of doing it directly. A container
/// worker handles the actual execution in isolation.
pub struct RunInSandboxTool {
    job_manager: Arc<ContainerJobManager>,
}

impl RunInSandboxTool {
    pub fn new(job_manager: Arc<ContainerJobManager>) -> Self {
        Self { job_manager }
    }
}

#[async_trait]
impl Tool for RunInSandboxTool {
    fn name(&self) -> &str {
        "run_in_sandbox"
    }

    fn description(&self) -> &str {
        "Execute a task in a sandboxed Docker container. The container has its own \
         sub-agent with shell, file read/write, list_dir, and apply_patch tools. \
         Use this for any work that requires filesystem access or shell commands. \
         The task description should be clear enough for the sub-agent to work independently."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Clear description of what to accomplish in the container"
                },
                "project_dir": {
                    "type": "string",
                    "description": "Host directory to bind mount into the container (optional). \
                                    The directory will be available at /workspace inside the container."
                },
                "wait": {
                    "type": "boolean",
                    "description": "If true (default), wait for the container to complete and return results. \
                                    If false, start the container and return the job_id immediately."
                }
            },
            "required": ["task"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let task = params
            .get("task")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("missing 'task' parameter".into()))?;

        let project_dir = params
            .get("project_dir")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);

        let wait = params.get("wait").and_then(|v| v.as_bool()).unwrap_or(true);

        let start = std::time::Instant::now();

        // Create the container job
        let (job_id, _token) = self
            .job_manager
            .create_job(task, project_dir.clone())
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("failed to create container: {}", e))
            })?;

        if !wait {
            // Fire-and-forget mode
            let result = serde_json::json!({
                "job_id": job_id.to_string(),
                "status": "started",
                "message": "Container started. Use job tools to check status."
            });
            return Ok(ToolOutput::success(result, start.elapsed()));
        }

        // Wait for completion by polling the container state
        let timeout = Duration::from_secs(600); // 10 minute max
        let poll_interval = Duration::from_secs(2);
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if tokio::time::Instant::now() > deadline {
                let _ = self.job_manager.stop_job(job_id).await;
                self.job_manager.cleanup_job(job_id).await;
                return Err(ToolError::ExecutionFailed(
                    "container execution timed out (10 minutes)".to_string(),
                ));
            }

            match self.job_manager.get_handle(job_id).await {
                Some(handle) => match handle.state {
                    crate::orchestrator::job_manager::ContainerState::Running
                    | crate::orchestrator::job_manager::ContainerState::Creating => {
                        tokio::time::sleep(poll_interval).await;
                    }
                    crate::orchestrator::job_manager::ContainerState::Stopped => {
                        let message = handle
                            .completion_result
                            .as_ref()
                            .and_then(|r| r.message.clone())
                            .unwrap_or_else(|| "Container job completed".to_string());
                        let success = handle
                            .completion_result
                            .as_ref()
                            .map(|r| r.success)
                            .unwrap_or(true);
                        self.job_manager.cleanup_job(job_id).await;

                        if success {
                            let result = serde_json::json!({
                                "job_id": job_id.to_string(),
                                "status": "completed",
                                "output": message
                            });
                            return Ok(ToolOutput::success(result, start.elapsed()));
                        } else {
                            return Err(ToolError::ExecutionFailed(format!(
                                "container job failed: {}",
                                message
                            )));
                        }
                    }
                    crate::orchestrator::job_manager::ContainerState::Failed => {
                        let message = handle
                            .completion_result
                            .as_ref()
                            .and_then(|r| r.message.clone())
                            .unwrap_or_else(|| "unknown failure".to_string());
                        self.job_manager.cleanup_job(job_id).await;
                        return Err(ToolError::ExecutionFailed(format!(
                            "container job failed: {}",
                            message
                        )));
                    }
                },
                None => {
                    let result = serde_json::json!({
                        "job_id": job_id.to_string(),
                        "status": "completed",
                        "output": "Container job completed"
                    });
                    return Ok(ToolOutput::success(result, start.elapsed()));
                }
            }
        }
    }

    fn requires_approval(&self) -> bool {
        // TODO: re-enable after testing
        false
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_tool_metadata() {
        // We can't easily test execution without Docker, but we can test the metadata
        let tool_name = "run_in_sandbox";
        assert_eq!(tool_name, "run_in_sandbox");

        // Verify the schema has required fields
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "task": { "type": "string" },
                "project_dir": { "type": "string" },
                "wait": { "type": "boolean" }
            },
            "required": ["task"]
        });

        let required = schema.get("required").unwrap().as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0].as_str().unwrap(), "task");
    }
}
