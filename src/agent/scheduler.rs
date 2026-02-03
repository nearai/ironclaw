//! Job scheduler for parallel execution.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::agent::Worker;
use crate::config::AgentConfig;
use crate::context::{ContextManager, JobState};
use crate::error::JobError;
use crate::history::Store;
use crate::llm::LlmProvider;
use crate::safety::SafetyLayer;
use crate::tools::ToolRegistry;

/// Message to send to a worker.
#[derive(Debug)]
pub enum WorkerMessage {
    /// Start working on the job.
    Start,
    /// Stop the job.
    Stop,
    /// Check health.
    Ping,
}

/// Status of a scheduled job.
#[derive(Debug)]
pub struct ScheduledJob {
    pub job_id: Uuid,
    pub handle: JoinHandle<()>,
    pub tx: mpsc::Sender<WorkerMessage>,
}

/// Schedules and manages parallel job execution.
pub struct Scheduler {
    config: AgentConfig,
    context_manager: Arc<ContextManager>,
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
    tools: Arc<ToolRegistry>,
    store: Option<Arc<Store>>,
    /// Running jobs.
    jobs: RwLock<HashMap<Uuid, ScheduledJob>>,
}

impl Scheduler {
    /// Create a new scheduler.
    pub fn new(
        config: AgentConfig,
        context_manager: Arc<ContextManager>,
        llm: Arc<dyn LlmProvider>,
        safety: Arc<SafetyLayer>,
        tools: Arc<ToolRegistry>,
        store: Option<Arc<Store>>,
    ) -> Self {
        Self {
            config,
            context_manager,
            llm,
            safety,
            tools,
            store,
            jobs: RwLock::new(HashMap::new()),
        }
    }

    /// Schedule a job for execution.
    pub async fn schedule(&self, job_id: Uuid) -> Result<(), JobError> {
        // Check if already scheduled
        if self.jobs.read().await.contains_key(&job_id) {
            return Ok(());
        }

        // Check capacity
        let current_count = self.jobs.read().await.len();
        if current_count >= self.config.max_parallel_jobs {
            return Err(JobError::MaxJobsExceeded {
                max: self.config.max_parallel_jobs,
            });
        }

        // Transition job to in_progress
        self.context_manager
            .update_context(job_id, |ctx| {
                ctx.transition_to(
                    JobState::InProgress,
                    Some("Scheduled for execution".to_string()),
                )
            })
            .await?
            .map_err(|s| JobError::ContextError {
                id: job_id,
                reason: s,
            })?;

        // Create worker channel
        let (tx, rx) = mpsc::channel(16);

        // Create worker
        let worker = Worker::new(
            job_id,
            self.context_manager.clone(),
            self.llm.clone(),
            self.safety.clone(),
            self.tools.clone(),
            self.store.clone(),
            self.config.job_timeout,
        );

        // Spawn worker task
        let handle = tokio::spawn(async move {
            if let Err(e) = worker.run(rx).await {
                tracing::error!("Worker for job {} failed: {}", job_id, e);
            }
        });

        // Start the worker
        let _ = tx.send(WorkerMessage::Start).await;

        // Store the scheduled job
        self.jobs
            .write()
            .await
            .insert(job_id, ScheduledJob { job_id, handle, tx });

        tracing::info!("Scheduled job {} for execution", job_id);
        Ok(())
    }

    /// Stop a running job.
    pub async fn stop(&self, job_id: Uuid) -> Result<(), JobError> {
        let mut jobs = self.jobs.write().await;

        if let Some(scheduled) = jobs.remove(&job_id) {
            // Send stop signal
            let _ = scheduled.tx.send(WorkerMessage::Stop).await;

            // Give it a moment to clean up
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            // Abort if still running
            if !scheduled.handle.is_finished() {
                scheduled.handle.abort();
            }

            // Update job state
            self.context_manager
                .update_context(job_id, |ctx| {
                    let _ = ctx.transition_to(
                        JobState::Cancelled,
                        Some("Stopped by scheduler".to_string()),
                    );
                })
                .await?;

            // Persist cancellation (fire-and-forget)
            if let Some(ref store) = self.store {
                let store = store.clone();
                tokio::spawn(async move {
                    if let Err(e) = store
                        .update_job_status(
                            job_id,
                            JobState::Cancelled,
                            Some("Stopped by scheduler"),
                        )
                        .await
                    {
                        tracing::warn!("Failed to persist cancellation for job {}: {}", job_id, e);
                    }
                });
            }

            tracing::info!("Stopped job {}", job_id);
        }

        Ok(())
    }

    /// Check if a job is running.
    pub async fn is_running(&self, job_id: Uuid) -> bool {
        self.jobs.read().await.contains_key(&job_id)
    }

    /// Get count of running jobs.
    pub async fn running_count(&self) -> usize {
        self.jobs.read().await.len()
    }

    /// Get all running job IDs.
    pub async fn running_jobs(&self) -> Vec<Uuid> {
        self.jobs.read().await.keys().cloned().collect()
    }

    /// Clean up finished jobs.
    pub async fn cleanup_finished(&self) {
        let mut jobs = self.jobs.write().await;
        let mut finished = Vec::new();

        for (id, scheduled) in jobs.iter() {
            if scheduled.handle.is_finished() {
                finished.push(*id);
            }
        }

        for id in finished {
            jobs.remove(&id);
            tracing::debug!("Cleaned up finished job {}", id);
        }
    }

    /// Stop all jobs.
    pub async fn stop_all(&self) {
        let job_ids: Vec<Uuid> = self.jobs.read().await.keys().cloned().collect();

        for job_id in job_ids {
            let _ = self.stop(job_id).await;
        }
    }
}

#[cfg(test)]
mod tests {
    // Note: Full scheduler tests require mocking LLM provider
    // These are placeholder tests

    #[test]
    fn test_scheduler_creation() {
        // Would need to mock dependencies for proper testing
    }
}
