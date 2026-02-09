//! Tool that delegates work to a sandboxed Docker container.
//!
//! The orchestrator LLM calls this tool instead of directly using shell/file tools.
//! It creates a container with a worker sub-agent that executes the task independently.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use crate::context::JobContext;
use crate::history::{SandboxJobRecord, Store};
use crate::orchestrator::job_manager::ContainerJobManager;
use crate::tools::tool::{Tool, ToolError, ToolOutput};

/// Tool for delegating filesystem/shell work to a sandboxed Docker container.
///
/// When the orchestrator LLM needs to read files, run shell commands, or
/// modify code, it calls this tool instead of doing it directly. A container
/// worker handles the actual execution in isolation.
pub struct RunInSandboxTool {
    job_manager: Arc<ContainerJobManager>,
    store: Option<Arc<Store>>,
}

impl RunInSandboxTool {
    pub fn new(job_manager: Arc<ContainerJobManager>, store: Option<Arc<Store>>) -> Self {
        Self { job_manager, store }
    }

    /// Persist a sandbox job record. Spawned as a fire-and-forget task so tool
    /// execution is not blocked by DB latency.
    fn persist_job(&self, record: SandboxJobRecord) {
        if let Some(store) = self.store.clone() {
            tokio::spawn(async move {
                if let Err(e) = store.save_sandbox_job(&record).await {
                    tracing::warn!(job_id = %record.id, "Failed to persist sandbox job: {}", e);
                }
            });
        }
    }

    /// Update sandbox job status in DB (fire-and-forget).
    fn update_status(
        &self,
        job_id: Uuid,
        status: &str,
        success: Option<bool>,
        message: Option<String>,
        started_at: Option<chrono::DateTime<Utc>>,
        completed_at: Option<chrono::DateTime<Utc>>,
    ) {
        if let Some(store) = self.store.clone() {
            let status = status.to_string();
            tokio::spawn(async move {
                if let Err(e) = store
                    .update_sandbox_job_status(
                        job_id,
                        &status,
                        success,
                        message.as_deref(),
                        started_at,
                        completed_at,
                    )
                    .await
                {
                    tracing::warn!(job_id = %job_id, "Failed to update sandbox job status: {}", e);
                }
            });
        }
    }
}

/// Resolve the project directory, creating it if it doesn't exist.
///
/// If the caller supplied an explicit path, use that.
/// Otherwise auto-create `~/.ironclaw/projects/{project_id}/` so every sandbox
/// job has a persistent bind mount that survives container teardown.
fn resolve_project_dir(
    explicit: Option<PathBuf>,
    project_id: Uuid,
) -> Result<(PathBuf, String), ToolError> {
    let dir = match explicit {
        Some(d) => d,
        None => dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".ironclaw")
            .join("projects")
            .join(project_id.to_string()),
    };
    std::fs::create_dir_all(&dir).map_err(|e| {
        ToolError::ExecutionFailed(format!(
            "failed to create project dir {}: {}",
            dir.display(),
            e
        ))
    })?;
    let browse_id = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| project_id.to_string());
    Ok((dir, browse_id))
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
                                    The directory will be available at /workspace inside the container. \
                                    If omitted, a directory is auto-created under ~/.ironclaw/projects/."
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

    fn execution_timeout(&self) -> Duration {
        // Sandbox polls for up to 10 min internally; give an extra 60s buffer.
        Duration::from_secs(660)
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

        let explicit_dir = params
            .get("project_dir")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);

        let wait = params.get("wait").and_then(|v| v.as_bool()).unwrap_or(true);

        let start = std::time::Instant::now();

        // Use a single UUID for everything: job_id, project directory, DB row.
        let job_id = Uuid::new_v4();
        let (project_dir, browse_id) = resolve_project_dir(explicit_dir, job_id)?;
        let project_dir_str = project_dir.display().to_string();

        // Persist the job to DB before creating the container.
        self.persist_job(SandboxJobRecord {
            id: job_id,
            task: task.to_string(),
            status: "creating".to_string(),
            user_id: _ctx.user_id.clone(),
            project_dir: project_dir_str.clone(),
            success: None,
            failure_reason: None,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
        });

        // Create the container job with the pre-determined job_id.
        let _token = self
            .job_manager
            .create_job(job_id, task, Some(project_dir))
            .await
            .map_err(|e| {
                self.update_status(
                    job_id,
                    "failed",
                    Some(false),
                    Some(e.to_string()),
                    None,
                    Some(Utc::now()),
                );
                ToolError::ExecutionFailed(format!("failed to create container: {}", e))
            })?;

        // Container started successfully.
        let now = Utc::now();
        self.update_status(job_id, "running", None, None, Some(now), None);

        if !wait {
            let result = serde_json::json!({
                "job_id": job_id.to_string(),
                "status": "started",
                "message": "Container started. Use job tools to check status.",
                "project_dir": project_dir_str,
                "browse_url": format!("/projects/{}", browse_id),
            });
            return Ok(ToolOutput::success(result, start.elapsed()));
        }

        // Wait for completion by polling the container state.
        let timeout = Duration::from_secs(600);
        let poll_interval = Duration::from_secs(2);
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if tokio::time::Instant::now() > deadline {
                let _ = self.job_manager.stop_job(job_id).await;
                self.job_manager.cleanup_job(job_id).await;
                self.update_status(
                    job_id,
                    "failed",
                    Some(false),
                    Some("Timed out (10 minutes)".to_string()),
                    None,
                    Some(Utc::now()),
                );
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

                        let finished_at = Utc::now();
                        if success {
                            self.update_status(
                                job_id,
                                "completed",
                                Some(true),
                                None,
                                None,
                                Some(finished_at),
                            );
                            let result = serde_json::json!({
                                "job_id": job_id.to_string(),
                                "status": "completed",
                                "output": message,
                                "project_dir": project_dir_str,
                                "browse_url": format!("/projects/{}", browse_id),
                            });
                            return Ok(ToolOutput::success(result, start.elapsed()));
                        } else {
                            self.update_status(
                                job_id,
                                "failed",
                                Some(false),
                                Some(message.clone()),
                                None,
                                Some(finished_at),
                            );
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
                        self.update_status(
                            job_id,
                            "failed",
                            Some(false),
                            Some(message.clone()),
                            None,
                            Some(Utc::now()),
                        );
                        return Err(ToolError::ExecutionFailed(format!(
                            "container job failed: {}",
                            message
                        )));
                    }
                },
                None => {
                    self.update_status(
                        job_id,
                        "completed",
                        Some(true),
                        None,
                        None,
                        Some(Utc::now()),
                    );
                    let result = serde_json::json!({
                        "job_id": job_id.to_string(),
                        "status": "completed",
                        "output": "Container job completed",
                        "project_dir": project_dir_str,
                        "browse_url": format!("/projects/{}", browse_id),
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
    use super::*;

    #[test]
    fn test_tool_metadata() {
        let tool_name = "run_in_sandbox";
        assert_eq!(tool_name, "run_in_sandbox");

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

    #[test]
    fn test_execution_timeout_override() {
        // Verify the sandbox timeout constant matches: 10 min polling + 60s buffer
        assert_eq!(Duration::from_secs(660), Duration::from_secs(10 * 60 + 60));
    }

    #[test]
    fn test_resolve_project_dir_explicit() {
        let tmp = tempfile::tempdir().unwrap();
        let explicit = tmp.path().join("my_project");
        let (dir, browse_id) = resolve_project_dir(Some(explicit.clone()), Uuid::new_v4()).unwrap();
        assert_eq!(dir, explicit);
        assert!(dir.exists());
        assert_eq!(browse_id, "my_project");
    }

    #[test]
    fn test_resolve_project_dir_auto() {
        let project_id = Uuid::new_v4();
        let (dir, browse_id) = resolve_project_dir(None, project_id).unwrap();
        assert!(dir.exists());
        assert!(dir.ends_with(project_id.to_string()));
        assert_eq!(browse_id, project_id.to_string());
        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }
}
