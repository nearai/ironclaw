//! Job management tools.
//!
//! These tools allow the LLM to manage jobs:
//! - Create new jobs/tasks (with optional sandbox delegation)
//! - List existing jobs
//! - Check job status
//! - Cancel running jobs

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use crate::context::{ContextManager, JobContext, JobState};
use crate::db::Database;
use crate::history::SandboxJobRecord;
use crate::orchestrator::job_manager::{ContainerJobManager, JobMode};
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

/// Tool for creating a new job.
///
/// When sandbox deps are injected (via `with_sandbox`), the tool automatically
/// delegates execution to a Docker container. Otherwise it creates an in-memory
/// job via the ContextManager. The LLM never needs to know the difference.
pub struct CreateJobTool {
    context_manager: Arc<ContextManager>,
    job_manager: Option<Arc<ContainerJobManager>>,
    store: Option<Arc<dyn Database>>,
}

impl CreateJobTool {
    pub fn new(context_manager: Arc<ContextManager>) -> Self {
        Self {
            context_manager,
            job_manager: None,
            store: None,
        }
    }

    /// Inject sandbox dependencies so `create_job` delegates to Docker containers.
    pub fn with_sandbox(
        mut self,
        job_manager: Arc<ContainerJobManager>,
        store: Option<Arc<dyn Database>>,
    ) -> Self {
        self.job_manager = Some(job_manager);
        self.store = store;
        self
    }

    fn sandbox_enabled(&self) -> bool {
        self.job_manager.is_some()
    }

    /// Persist a sandbox job record (fire-and-forget).
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

    /// Execute via in-memory ContextManager (no sandbox).
    async fn execute_local(
        &self,
        title: &str,
        description: &str,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        match self
            .context_manager
            .create_job_for_user(&ctx.user_id, title, description)
            .await
        {
            Ok(job_id) => {
                let result = serde_json::json!({
                    "job_id": job_id.to_string(),
                    "title": title,
                    "status": "pending",
                    "message": format!("Created job '{}'", title)
                });
                Ok(ToolOutput::success(result, start.elapsed()))
            }
            Err(e) => {
                let result = serde_json::json!({
                    "error": e.to_string()
                });
                Ok(ToolOutput::success(result, start.elapsed()))
            }
        }
    }

    /// Execute via sandboxed Docker container.
    async fn execute_sandbox(
        &self,
        task: &str,
        explicit_dir: Option<PathBuf>,
        wait: bool,
        mode: JobMode,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let jm = self.job_manager.as_ref().expect("sandbox deps required");

        let job_id = Uuid::new_v4();
        let (project_dir, browse_id) = resolve_project_dir(explicit_dir, job_id)?;
        let project_dir_str = project_dir.display().to_string();

        // Persist the job to DB before creating the container.
        self.persist_job(SandboxJobRecord {
            id: job_id,
            task: task.to_string(),
            status: "creating".to_string(),
            user_id: ctx.user_id.clone(),
            project_dir: project_dir_str.clone(),
            success: None,
            failure_reason: None,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
        });

        // Persist the job mode to DB
        if mode == JobMode::ClaudeCode
            && let Some(store) = self.store.clone()
        {
            let job_id_copy = job_id;
            tokio::spawn(async move {
                if let Err(e) = store
                    .update_sandbox_job_mode(job_id_copy, "claude_code")
                    .await
                {
                    tracing::warn!(job_id = %job_id_copy, "Failed to set job mode: {}", e);
                }
            });
        }

        // Create the container job with the pre-determined job_id.
        let _token = jm
            .create_job(job_id, task, Some(project_dir), mode)
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
                let _ = jm.stop_job(job_id).await;
                jm.cleanup_job(job_id).await;
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

            match jm.get_handle(job_id).await {
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
                        jm.cleanup_job(job_id).await;

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
                        jm.cleanup_job(job_id).await;
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
}

/// The base directory where all project directories must live.
fn projects_base() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw")
        .join("projects")
}

/// Resolve the project directory, creating it if it doesn't exist.
///
/// Auto-creates `~/.ironclaw/projects/{project_id}/` so every sandbox job has a
/// persistent bind mount that survives container teardown.
///
/// When an explicit path is provided (e.g. job restarts reusing the old dir),
/// it is validated to fall within `~/.ironclaw/projects/` after canonicalization.
fn resolve_project_dir(
    explicit: Option<PathBuf>,
    project_id: Uuid,
) -> Result<(PathBuf, String), ToolError> {
    let base = projects_base();
    std::fs::create_dir_all(&base).map_err(|e| {
        ToolError::ExecutionFailed(format!(
            "failed to create projects base {}: {}",
            base.display(),
            e
        ))
    })?;
    let canonical_base = base.canonicalize().map_err(|e| {
        ToolError::ExecutionFailed(format!("failed to canonicalize projects base: {}", e))
    })?;

    let dir = match explicit {
        Some(d) => d,
        None => canonical_base.join(project_id.to_string()),
    };

    std::fs::create_dir_all(&dir).map_err(|e| {
        ToolError::ExecutionFailed(format!(
            "failed to create project dir {}: {}",
            dir.display(),
            e
        ))
    })?;

    // Canonicalize resolves symlinks, `..`, etc. so we can do a reliable prefix check.
    let canonical_dir = dir.canonicalize().map_err(|e| {
        ToolError::ExecutionFailed(format!(
            "failed to canonicalize project dir {}: {}",
            dir.display(),
            e
        ))
    })?;

    if !canonical_dir.starts_with(&canonical_base) {
        return Err(ToolError::InvalidParameters(format!(
            "project directory must be under {}",
            canonical_base.display()
        )));
    }

    let browse_id = canonical_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| project_id.to_string());
    Ok((canonical_dir, browse_id))
}

#[async_trait]
impl Tool for CreateJobTool {
    fn name(&self) -> &str {
        "create_job"
    }

    fn description(&self) -> &str {
        if self.sandbox_enabled() {
            "Create and execute a job. The job runs in a sandboxed Docker container with its own \
             sub-agent that has shell, file read/write, list_dir, and apply_patch tools. Use this \
             whenever the user asks you to build, create, or work on something. The task \
             description should be detailed enough for the sub-agent to work independently. \
             Set wait=false to start immediately while continuing the conversation. Set mode \
             to 'claude_code' for complex software engineering tasks."
        } else {
            "Create a new job or task for the agent to work on. Use this when the user wants \
             you to do something substantial that should be tracked as a separate job."
        }
    }

    fn parameters_schema(&self) -> serde_json::Value {
        if self.sandbox_enabled() {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Clear description of what to accomplish"
                    },
                    "description": {
                        "type": "string",
                        "description": "Full description of what needs to be done"
                    },
                    "wait": {
                        "type": "boolean",
                        "description": "If true (default), wait for the container to complete and return results. \
                                        If false, start the container and return the job_id immediately."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["worker", "claude_code"],
                        "description": "Execution mode. 'worker' (default) uses the IronClaw sub-agent. \
                                        'claude_code' uses Claude Code CLI for full agentic software engineering."
                    }
                },
                "required": ["title", "description"]
            })
        } else {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "A short title for the job (max 100 chars)"
                    },
                    "description": {
                        "type": "string",
                        "description": "Full description of what needs to be done"
                    }
                },
                "required": ["title", "description"]
            })
        }
    }

    fn execution_timeout(&self) -> Duration {
        if self.sandbox_enabled() {
            // Sandbox polls for up to 10 min internally; give an extra 60s buffer.
            Duration::from_secs(660)
        } else {
            Duration::from_secs(30)
        }
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let title = require_str(&params, "title")?;

        let description = require_str(&params, "description")?;

        if self.sandbox_enabled() {
            let wait = params.get("wait").and_then(|v| v.as_bool()).unwrap_or(true);

            let mode = match params.get("mode").and_then(|v| v.as_str()) {
                Some("claude_code") => JobMode::ClaudeCode,
                _ => JobMode::Worker,
            };

            // Combine title and description into the task prompt for the sub-agent.
            let task = format!("{}\n\n{}", title, description);
            self.execute_sandbox(&task, None, wait, mode, ctx).await
        } else {
            self.execute_local(title, description, ctx).await
        }
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for listing jobs.
pub struct ListJobsTool {
    context_manager: Arc<ContextManager>,
}

impl ListJobsTool {
    pub fn new(context_manager: Arc<ContextManager>) -> Self {
        Self { context_manager }
    }
}

#[async_trait]
impl Tool for ListJobsTool {
    fn name(&self) -> &str {
        "list_jobs"
    }

    fn description(&self) -> &str {
        "List all jobs or filter by status. Shows job IDs, titles, and current status."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "filter": {
                    "type": "string",
                    "description": "Filter by status: 'active', 'completed', 'failed', 'all' (default: 'all')",
                    "enum": ["active", "completed", "failed", "all"]
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let filter = params
            .get("filter")
            .and_then(|v| v.as_str())
            .unwrap_or("all");

        let job_ids = match filter {
            "active" => self.context_manager.active_jobs_for(&ctx.user_id).await,
            _ => self.context_manager.all_jobs_for(&ctx.user_id).await,
        };

        let mut jobs = Vec::new();
        for job_id in job_ids {
            if let Ok(ctx) = self.context_manager.get_context(job_id).await {
                let include = match filter {
                    "completed" => ctx.state == JobState::Completed,
                    "failed" => ctx.state == JobState::Failed,
                    "active" => ctx.state.is_active(),
                    _ => true,
                };

                if include {
                    jobs.push(serde_json::json!({
                        "job_id": job_id.to_string(),
                        "title": ctx.title,
                        "status": format!("{:?}", ctx.state),
                        "created_at": ctx.created_at.to_rfc3339()
                    }));
                }
            }
        }

        let summary = self.context_manager.summary_for(&ctx.user_id).await;

        let result = serde_json::json!({
            "jobs": jobs,
            "summary": {
                "total": summary.total,
                "pending": summary.pending,
                "in_progress": summary.in_progress,
                "completed": summary.completed,
                "failed": summary.failed
            }
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for checking job status.
pub struct JobStatusTool {
    context_manager: Arc<ContextManager>,
}

impl JobStatusTool {
    pub fn new(context_manager: Arc<ContextManager>) -> Self {
        Self { context_manager }
    }
}

#[async_trait]
impl Tool for JobStatusTool {
    fn name(&self) -> &str {
        "job_status"
    }

    fn description(&self) -> &str {
        "Check the status and details of a specific job by its ID."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The UUID of the job to check"
                }
            },
            "required": ["job_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let requester_id = ctx.user_id.clone();

        let job_id_str = require_str(&params, "job_id")?;

        let job_id = Uuid::parse_str(job_id_str).map_err(|_| {
            ToolError::InvalidParameters(format!("invalid job ID format: {}", job_id_str))
        })?;

        match self.context_manager.get_context(job_id).await {
            Ok(job_ctx) => {
                if job_ctx.user_id != requester_id {
                    let result = serde_json::json!({
                        "error": "Job not found".to_string()
                    });
                    return Ok(ToolOutput::success(result, start.elapsed()));
                }
                let result = serde_json::json!({
                    "job_id": job_id.to_string(),
                    "title": job_ctx.title,
                    "description": job_ctx.description,
                    "status": format!("{:?}", job_ctx.state),
                    "created_at": job_ctx.created_at.to_rfc3339(),
                    "started_at": job_ctx.started_at.map(|t| t.to_rfc3339()),
                    "completed_at": job_ctx.completed_at.map(|t| t.to_rfc3339()),
                    "actual_cost": job_ctx.actual_cost.to_string()
                });
                Ok(ToolOutput::success(result, start.elapsed()))
            }
            Err(e) => {
                let result = serde_json::json!({
                    "error": format!("Job not found: {}", e)
                });
                Ok(ToolOutput::success(result, start.elapsed()))
            }
        }
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for canceling a job.
pub struct CancelJobTool {
    context_manager: Arc<ContextManager>,
}

impl CancelJobTool {
    pub fn new(context_manager: Arc<ContextManager>) -> Self {
        Self { context_manager }
    }
}

#[async_trait]
impl Tool for CancelJobTool {
    fn name(&self) -> &str {
        "cancel_job"
    }

    fn description(&self) -> &str {
        "Cancel a running or pending job. The job will be marked as cancelled and stopped."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The UUID of the job to cancel"
                }
            },
            "required": ["job_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let requester_id = ctx.user_id.clone();

        let job_id_str = require_str(&params, "job_id")?;

        let job_id = Uuid::parse_str(job_id_str).map_err(|_| {
            ToolError::InvalidParameters(format!("invalid job ID format: {}", job_id_str))
        })?;

        // Transition to cancelled state
        match self
            .context_manager
            .update_context(job_id, |ctx| {
                if ctx.user_id != requester_id {
                    return Err("Job not found".to_string());
                }
                ctx.transition_to(JobState::Cancelled, Some("Cancelled by user".to_string()))
            })
            .await
        {
            Ok(Ok(())) => {
                let result = serde_json::json!({
                    "job_id": job_id.to_string(),
                    "status": "cancelled",
                    "message": "Job cancelled successfully"
                });
                Ok(ToolOutput::success(result, start.elapsed()))
            }
            Ok(Err(reason)) => {
                let result = serde_json::json!({
                    "error": format!("Cannot cancel job: {}", reason)
                });
                Ok(ToolOutput::success(result, start.elapsed()))
            }
            Err(e) => {
                let result = serde_json::json!({
                    "error": format!("Job not found: {}", e)
                });
                Ok(ToolOutput::success(result, start.elapsed()))
            }
        }
    }

    fn requires_approval(&self) -> bool {
        true // Canceling a job should require approval
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_job_tool_local() {
        let manager = Arc::new(ContextManager::new(5));
        let tool = CreateJobTool::new(manager.clone());

        // Without sandbox deps, it should use the local path
        assert!(!tool.sandbox_enabled());

        let params = serde_json::json!({
            "title": "Test Job",
            "description": "A test job description"
        });

        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await.unwrap();

        let job_id = result.result.get("job_id").unwrap().as_str().unwrap();
        assert!(!job_id.is_empty());
        assert_eq!(
            result.result.get("status").unwrap().as_str().unwrap(),
            "pending"
        );
    }

    #[test]
    fn test_schema_changes_with_sandbox() {
        let manager = Arc::new(ContextManager::new(5));

        // Without sandbox
        let tool = CreateJobTool::new(Arc::clone(&manager));
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("title"));
        assert!(props.contains_key("description"));
        assert!(
            !props.contains_key("project_dir"),
            "project_dir must not be exposed to the LLM"
        );
        assert!(!props.contains_key("wait"));
        assert!(!props.contains_key("mode"));
    }

    #[test]
    fn test_execution_timeout_sandbox() {
        let manager = Arc::new(ContextManager::new(5));

        // Without sandbox: default timeout
        let tool = CreateJobTool::new(Arc::clone(&manager));
        assert_eq!(tool.execution_timeout(), Duration::from_secs(30));
    }

    #[tokio::test]
    async fn test_list_jobs_tool() {
        let manager = Arc::new(ContextManager::new(5));

        // Create some jobs
        manager.create_job("Job 1", "Desc 1").await.unwrap();
        manager.create_job("Job 2", "Desc 2").await.unwrap();

        let tool = ListJobsTool::new(manager);

        let params = serde_json::json!({});
        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await.unwrap();

        let jobs = result.result.get("jobs").unwrap().as_array().unwrap();
        assert_eq!(jobs.len(), 2);
    }

    #[tokio::test]
    async fn test_job_status_tool() {
        let manager = Arc::new(ContextManager::new(5));
        let job_id = manager.create_job("Test Job", "Description").await.unwrap();

        let tool = JobStatusTool::new(manager);

        let params = serde_json::json!({
            "job_id": job_id.to_string()
        });
        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await.unwrap();

        assert_eq!(
            result.result.get("title").unwrap().as_str().unwrap(),
            "Test Job"
        );
    }

    #[test]
    fn test_resolve_project_dir_auto() {
        let project_id = Uuid::new_v4();
        let (dir, browse_id) = resolve_project_dir(None, project_id).unwrap();
        assert!(dir.exists());
        assert!(dir.ends_with(project_id.to_string()));
        assert_eq!(browse_id, project_id.to_string());

        // Must be under the projects base
        let base = projects_base().canonicalize().unwrap();
        assert!(dir.starts_with(&base));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_project_dir_explicit_under_base() {
        let base = projects_base();
        std::fs::create_dir_all(&base).unwrap();
        let explicit = base.join("test_explicit_project");
        let project_id = Uuid::new_v4();

        let (dir, browse_id) = resolve_project_dir(Some(explicit.clone()), project_id).unwrap();
        assert!(dir.exists());
        assert_eq!(browse_id, "test_explicit_project");

        let canonical_base = base.canonicalize().unwrap();
        assert!(dir.starts_with(&canonical_base));

        let _ = std::fs::remove_dir_all(&explicit);
    }

    #[test]
    fn test_resolve_project_dir_rejects_outside_base() {
        let tmp = tempfile::tempdir().unwrap();
        let escape_attempt = tmp.path().join("evil_project");

        let result = resolve_project_dir(Some(escape_attempt), Uuid::new_v4());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("must be under"),
            "expected 'must be under' error, got: {}",
            err
        );
    }

    #[test]
    fn test_resolve_project_dir_rejects_traversal() {
        // Attempt to escape via `..` components
        let base = projects_base();
        let traversal = base.join("legit").join("..").join("..").join(".ssh");

        let result = resolve_project_dir(Some(traversal), Uuid::new_v4());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("must be under"),
            "expected 'must be under' error, got: {}",
            err
        );
    }
}
