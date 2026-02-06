//! Job management tools.
//!
//! These tools allow the LLM to manage jobs:
//! - Create new jobs/tasks
//! - List existing jobs
//! - Check job status
//! - Cancel running jobs

use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use crate::context::{ContextManager, JobContext, JobState};
use crate::tools::tool::{Tool, ToolError, ToolOutput};

/// Tool for creating a new job.
pub struct CreateJobTool {
    context_manager: Arc<ContextManager>,
}

impl CreateJobTool {
    pub fn new(context_manager: Arc<ContextManager>) -> Self {
        Self { context_manager }
    }
}

#[async_trait]
impl Tool for CreateJobTool {
    fn name(&self) -> &str {
        "create_job"
    }

    fn description(&self) -> &str {
        "Create a new job or task for the agent to work on. Use this when the user wants \
         you to do something substantial that should be tracked as a separate job."
    }

    fn parameters_schema(&self) -> serde_json::Value {
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

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let title = params
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("missing 'title' parameter".into()))?;

        let description = params
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("missing 'description' parameter".into())
            })?;

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

        let job_id_str = params
            .get("job_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("missing 'job_id' parameter".into()))?;

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

        let job_id_str = params
            .get("job_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("missing 'job_id' parameter".into()))?;

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
    async fn test_create_job_tool() {
        let manager = Arc::new(ContextManager::new(5));
        let tool = CreateJobTool::new(manager.clone());

        let params = serde_json::json!({
            "title": "Test Job",
            "description": "A test job description"
        });

        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await.unwrap();

        let job_id = result.result.get("job_id").unwrap().as_str().unwrap();
        assert!(!job_id.is_empty());
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
}
