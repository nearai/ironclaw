//! Self-repair for stuck jobs and broken tools.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::context::{ContextManager, JobState};
use crate::db::Database;
use crate::error::RepairError;
use crate::tools::{BuildRequirement, Language, SoftwareBuilder, SoftwareType, ToolRegistry};

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
    pub first_failure: DateTime<Utc>,
    pub last_failure: DateTime<Utc>,
    pub last_build_result: Option<serde_json::Value>,
    pub repair_attempts: u32,
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
    // TODO: use for time-based stuck detection (currently only max_repair_attempts is checked)
    #[allow(dead_code)]
    stuck_threshold: Duration,
    max_repair_attempts: u32,
    store: Option<Arc<dyn Database>>,
    builder: Option<Arc<dyn SoftwareBuilder>>,
    // TODO: use for tool hot-reload after repair
    #[allow(dead_code)]
    tools: Option<Arc<ToolRegistry>>,
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
            store: None,
            builder: None,
            tools: None,
        }
    }

    /// Add a Store for tool failure tracking.
    #[allow(dead_code)] // TODO: wire up in main.rs when persistence is needed
    pub(crate) fn with_store(mut self, store: Arc<dyn Database>) -> Self {
        self.store = Some(store);
        self
    }

    /// Add a Builder and ToolRegistry for automatic tool repair.
    #[allow(dead_code)] // TODO: wire up in main.rs when auto-repair is needed
    pub(crate) fn with_builder(
        mut self,
        builder: Arc<dyn SoftwareBuilder>,
        tools: Arc<ToolRegistry>,
    ) -> Self {
        self.builder = Some(builder);
        self.tools = Some(tools);
        self
    }
}

#[async_trait]
impl SelfRepair for DefaultSelfRepair {
    async fn detect_stuck_jobs(&self) -> Vec<StuckJob> {
        let stuck_ids = self.context_manager.find_stuck_jobs().await;
        let mut stuck_jobs = Vec::new();

        for job_id in stuck_ids {
            if let Ok(ctx) = self.context_manager.get_context(job_id).await
                && ctx.state == JobState::Stuck
            {
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
        let Some(ref store) = self.store else {
            return vec![];
        };

        // Threshold: 5 failures before considering a tool broken
        match store.get_broken_tools(5).await {
            Ok(tools) => {
                if !tools.is_empty() {
                    tracing::info!("Detected {} broken tools needing repair", tools.len());
                }
                tools
            }
            Err(e) => {
                tracing::warn!("Failed to detect broken tools: {}", e);
                vec![]
            }
        }
    }

    async fn repair_broken_tool(&self, tool: &BrokenTool) -> Result<RepairResult, RepairError> {
        let Some(ref builder) = self.builder else {
            return Ok(RepairResult::ManualRequired {
                message: format!("Builder not available for repairing tool '{}'", tool.name),
            });
        };

        let Some(ref store) = self.store else {
            return Ok(RepairResult::ManualRequired {
                message: "Store not available for tracking repair".to_string(),
            });
        };

        // Check repair attempt limit
        if tool.repair_attempts >= self.max_repair_attempts {
            return Ok(RepairResult::ManualRequired {
                message: format!(
                    "Tool '{}' exceeded max repair attempts ({})",
                    tool.name, self.max_repair_attempts
                ),
            });
        }

        tracing::info!(
            "Attempting to repair tool '{}' (attempt {})",
            tool.name,
            tool.repair_attempts + 1
        );

        // Increment repair attempts
        if let Err(e) = store.increment_repair_attempts(&tool.name).await {
            tracing::warn!("Failed to increment repair attempts: {}", e);
        }

        // Create BuildRequirement for repair
        let requirement = BuildRequirement {
            name: tool.name.clone(),
            description: format!(
                "Repair broken WASM tool.\n\n\
                 Tool name: {}\n\
                 Previous error: {}\n\
                 Failure count: {}\n\n\
                 Analyze the error, fix the implementation, and rebuild.",
                tool.name,
                tool.last_error.as_deref().unwrap_or("Unknown error"),
                tool.failure_count
            ),
            software_type: SoftwareType::WasmTool,
            language: Language::Rust,
            input_spec: None,
            output_spec: None,
            dependencies: vec![],
            capabilities: vec!["http".to_string(), "workspace".to_string()],
        };

        // Attempt to build/repair
        match builder.build(&requirement).await {
            Ok(result) if result.success => {
                tracing::info!(
                    "Successfully rebuilt tool '{}' after {} iterations",
                    tool.name,
                    result.iterations
                );

                // Mark as repaired in database
                if let Err(e) = store.mark_tool_repaired(&tool.name).await {
                    tracing::warn!("Failed to mark tool as repaired: {}", e);
                }

                // Log if the tool was auto-registered
                if result.registered {
                    tracing::info!("Repaired tool '{}' auto-registered", tool.name);
                }

                Ok(RepairResult::Success {
                    message: format!(
                        "Tool '{}' repaired successfully after {} iterations",
                        tool.name, result.iterations
                    ),
                })
            }
            Ok(result) => {
                // Build completed but failed
                tracing::warn!(
                    "Repair build for '{}' completed but failed: {:?}",
                    tool.name,
                    result.error
                );
                Ok(RepairResult::Retry {
                    message: format!(
                        "Repair attempt {} for '{}' failed: {}",
                        tool.repair_attempts + 1,
                        tool.name,
                        result.error.unwrap_or_else(|| "Unknown error".to_string())
                    ),
                })
            }
            Err(e) => {
                tracing::error!("Repair build for '{}' errored: {}", tool.name, e);
                Ok(RepairResult::Retry {
                    message: format!("Repair build error: {}", e),
                })
            }
        }
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

    // === QA Plan - Self-repair stuck job tests ===

    #[tokio::test]
    async fn detect_no_stuck_jobs_when_all_healthy() {
        let cm = Arc::new(ContextManager::new(10));

        // Create a job and leave it Pending (not stuck).
        cm.create_job("Job 1", "desc").await.unwrap();

        let repair = DefaultSelfRepair::new(cm, Duration::from_secs(60), 3);
        let stuck = repair.detect_stuck_jobs().await;
        assert!(stuck.is_empty());
    }

    #[tokio::test]
    async fn detect_stuck_job_finds_stuck_state() {
        let cm = Arc::new(ContextManager::new(10));
        let job_id = cm.create_job("Stuck job", "desc").await.unwrap();

        // Transition to InProgress, then to Stuck.
        cm.update_context(job_id, |ctx| ctx.transition_to(JobState::InProgress, None))
            .await
            .unwrap()
            .unwrap();
        cm.update_context(job_id, |ctx| {
            ctx.transition_to(JobState::Stuck, Some("timed out".to_string()))
        })
        .await
        .unwrap()
        .unwrap();

        let repair = DefaultSelfRepair::new(cm, Duration::from_secs(60), 3);
        let stuck = repair.detect_stuck_jobs().await;
        assert_eq!(stuck.len(), 1);
        assert_eq!(stuck[0].job_id, job_id);
    }

    #[tokio::test]
    async fn repair_stuck_job_succeeds_within_limit() {
        let cm = Arc::new(ContextManager::new(10));
        let job_id = cm.create_job("Repairable", "desc").await.unwrap();

        // Move to InProgress -> Stuck.
        cm.update_context(job_id, |ctx| ctx.transition_to(JobState::InProgress, None))
            .await
            .unwrap()
            .unwrap();
        cm.update_context(job_id, |ctx| ctx.transition_to(JobState::Stuck, None))
            .await
            .unwrap()
            .unwrap();

        let repair = DefaultSelfRepair::new(Arc::clone(&cm), Duration::from_secs(60), 3);

        let stuck_job = StuckJob {
            job_id,
            last_activity: Utc::now(),
            stuck_duration: Duration::from_secs(120),
            last_error: None,
            repair_attempts: 0,
        };

        let result = repair.repair_stuck_job(&stuck_job).await.unwrap();
        assert!(
            matches!(result, RepairResult::Success { .. }),
            "Expected Success, got: {:?}",
            result
        );

        // Job should be back to InProgress after recovery.
        let ctx = cm.get_context(job_id).await.unwrap();
        assert_eq!(ctx.state, JobState::InProgress);
    }

    #[tokio::test]
    async fn repair_stuck_job_returns_manual_when_limit_exceeded() {
        let cm = Arc::new(ContextManager::new(10));
        let job_id = cm.create_job("Unrepairable", "desc").await.unwrap();

        let repair = DefaultSelfRepair::new(cm, Duration::from_secs(60), 2);

        let stuck_job = StuckJob {
            job_id,
            last_activity: Utc::now(),
            stuck_duration: Duration::from_secs(300),
            last_error: Some("persistent failure".to_string()),
            repair_attempts: 2, // == max
        };

        let result = repair.repair_stuck_job(&stuck_job).await.unwrap();
        assert!(
            matches!(result, RepairResult::ManualRequired { .. }),
            "Expected ManualRequired, got: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn detect_broken_tools_returns_empty_without_store() {
        let cm = Arc::new(ContextManager::new(10));
        let repair = DefaultSelfRepair::new(cm, Duration::from_secs(60), 3);

        // No store configured, should return empty.
        let broken = repair.detect_broken_tools().await;
        assert!(broken.is_empty());
    }

    #[tokio::test]
    async fn repair_broken_tool_returns_manual_without_builder() {
        let cm = Arc::new(ContextManager::new(10));
        let repair = DefaultSelfRepair::new(cm, Duration::from_secs(60), 3);

        let broken = BrokenTool {
            name: "test-tool".to_string(),
            failure_count: 10,
            last_error: Some("crash".to_string()),
            first_failure: Utc::now(),
            last_failure: Utc::now(),
            last_build_result: None,
            repair_attempts: 0,
        };

        let result = repair.repair_broken_tool(&broken).await.unwrap();
        assert!(
            matches!(result, RepairResult::ManualRequired { .. }),
            "Expected ManualRequired without builder, got: {:?}",
            result
        );
    }
}
