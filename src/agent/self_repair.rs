//! Self-repair for stuck jobs and broken tools.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::context::{ContextManager, JobState};
use crate::error::RepairError;

/// A job that has been detected as stuck.
#[derive(Debug, Clone)]
pub struct StuckJob {
    pub job_id: Uuid,
    pub last_activity: DateTime<Utc>,
    pub stuck_duration: Duration,
    pub last_error: Option<String>,
    pub repair_attempts: u32,
}

/// A tool that has been detected as broken.
#[derive(Debug, Clone)]
pub struct BrokenTool {
    pub name: String,
    pub failure_count: u32,
    pub last_error: Option<String>,
    pub last_failure: DateTime<Utc>,
}

/// Result of a repair attempt.
#[derive(Debug)]
pub enum RepairResult {
    /// Repair was successful.
    Success { message: String },
    /// Repair failed but can be retried.
    Retry { message: String },
    /// Repair failed permanently.
    Failed { message: String },
    /// Manual intervention required.
    ManualRequired { message: String },
}

/// Trait for self-repair implementations.
#[async_trait]
pub trait SelfRepair: Send + Sync {
    /// Detect stuck jobs.
    async fn detect_stuck_jobs(&self) -> Vec<StuckJob>;

    /// Attempt to repair a stuck job.
    async fn repair_stuck_job(&self, job: &StuckJob) -> Result<RepairResult, RepairError>;

    /// Detect broken tools.
    async fn detect_broken_tools(&self) -> Vec<BrokenTool>;

    /// Attempt to repair a broken tool.
    async fn repair_broken_tool(&self, tool: &BrokenTool) -> Result<RepairResult, RepairError>;
}

/// Default self-repair implementation.
pub struct DefaultSelfRepair {
    context_manager: Arc<ContextManager>,
    stuck_threshold: Duration,
    max_repair_attempts: u32,
}

impl DefaultSelfRepair {
    /// Create a new self-repair instance.
    pub fn new(
        context_manager: Arc<ContextManager>,
        stuck_threshold: Duration,
        max_repair_attempts: u32,
    ) -> Self {
        Self {
            context_manager,
            stuck_threshold,
            max_repair_attempts,
        }
    }
}

#[async_trait]
impl SelfRepair for DefaultSelfRepair {
    async fn detect_stuck_jobs(&self) -> Vec<StuckJob> {
        let stuck_ids = self.context_manager.find_stuck_jobs().await;
        let mut stuck_jobs = Vec::new();

        for job_id in stuck_ids {
            if let Ok(ctx) = self.context_manager.get_context(job_id).await {
                if ctx.state == JobState::Stuck {
                    let stuck_duration = ctx
                        .started_at
                        .map(|start| {
                            let now = Utc::now();
                            let duration = now.signed_duration_since(start);
                            Duration::from_secs(duration.num_seconds().max(0) as u64)
                        })
                        .unwrap_or_default();

                    stuck_jobs.push(StuckJob {
                        job_id,
                        last_activity: ctx.started_at.unwrap_or(ctx.created_at),
                        stuck_duration,
                        last_error: None,
                        repair_attempts: ctx.repair_attempts,
                    });
                }
            }
        }

        stuck_jobs
    }

    async fn repair_stuck_job(&self, job: &StuckJob) -> Result<RepairResult, RepairError> {
        // Check if we've exceeded max repair attempts
        if job.repair_attempts >= self.max_repair_attempts {
            return Ok(RepairResult::ManualRequired {
                message: format!(
                    "Job {} has exceeded maximum repair attempts ({})",
                    job.job_id, self.max_repair_attempts
                ),
            });
        }

        // Try to recover the job
        let result = self
            .context_manager
            .update_context(job.job_id, |ctx| ctx.attempt_recovery())
            .await;

        match result {
            Ok(Ok(())) => {
                tracing::info!("Successfully recovered job {}", job.job_id);
                Ok(RepairResult::Success {
                    message: format!("Job {} recovered and will be retried", job.job_id),
                })
            }
            Ok(Err(e)) => {
                tracing::warn!("Failed to recover job {}: {}", job.job_id, e);
                Ok(RepairResult::Retry {
                    message: format!("Recovery attempt failed: {}", e),
                })
            }
            Err(e) => Err(RepairError::Failed {
                target_type: "job".to_string(),
                target_id: job.job_id,
                reason: e.to_string(),
            }),
        }
    }

    async fn detect_broken_tools(&self) -> Vec<BrokenTool> {
        // TODO: Implement tool failure tracking
        // Would need to track tool failures in the database
        vec![]
    }

    async fn repair_broken_tool(&self, tool: &BrokenTool) -> Result<RepairResult, RepairError> {
        // TODO: Implement tool repair via ToolBuilder
        Ok(RepairResult::ManualRequired {
            message: format!(
                "Tool '{}' repair not implemented - manual intervention required",
                tool.name
            ),
        })
    }
}

/// Background repair task that periodically checks for and repairs issues.
pub struct RepairTask {
    repair: Arc<dyn SelfRepair>,
    check_interval: Duration,
}

impl RepairTask {
    /// Create a new repair task.
    pub fn new(repair: Arc<dyn SelfRepair>, check_interval: Duration) -> Self {
        Self {
            repair,
            check_interval,
        }
    }

    /// Run the repair task.
    pub async fn run(&self) {
        loop {
            tokio::time::sleep(self.check_interval).await;

            // Check for stuck jobs
            let stuck_jobs = self.repair.detect_stuck_jobs().await;
            for job in stuck_jobs {
                tracing::info!("Attempting to repair stuck job {}", job.job_id);
                match self.repair.repair_stuck_job(&job).await {
                    Ok(RepairResult::Success { message }) => {
                        tracing::info!("Repair succeeded: {}", message);
                    }
                    Ok(RepairResult::Retry { message }) => {
                        tracing::warn!("Repair needs retry: {}", message);
                    }
                    Ok(RepairResult::Failed { message }) => {
                        tracing::error!("Repair failed: {}", message);
                    }
                    Ok(RepairResult::ManualRequired { message }) => {
                        tracing::warn!("Manual intervention needed: {}", message);
                    }
                    Err(e) => {
                        tracing::error!("Repair error: {}", e);
                    }
                }
            }

            // Check for broken tools
            let broken_tools = self.repair.detect_broken_tools().await;
            for tool in broken_tools {
                tracing::info!("Attempting to repair broken tool: {}", tool.name);
                match self.repair.repair_broken_tool(&tool).await {
                    Ok(result) => {
                        tracing::info!("Tool repair result: {:?}", result);
                    }
                    Err(e) => {
                        tracing::error!("Tool repair error: {}", e);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repair_result_variants() {
        let success = RepairResult::Success {
            message: "OK".to_string(),
        };
        assert!(matches!(success, RepairResult::Success { .. }));

        let manual = RepairResult::ManualRequired {
            message: "Help needed".to_string(),
        };
        assert!(matches!(manual, RepairResult::ManualRequired { .. }));
    }
}
