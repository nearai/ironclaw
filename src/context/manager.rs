//! Context manager for handling multiple job contexts.

use std::collections::HashMap;

use tokio::sync::RwLock;
use uuid::Uuid;

use crate::context::{JobContext, Memory};
use crate::error::JobError;

/// Manages contexts for multiple concurrent jobs.
pub struct ContextManager {
    /// Active job contexts.
    contexts: RwLock<HashMap<Uuid, JobContext>>,
    /// Memory for each job.
    memories: RwLock<HashMap<Uuid, Memory>>,
    /// Maximum concurrent jobs.
    max_jobs: usize,
}

impl ContextManager {
    /// Create a new context manager.
    pub fn new(max_jobs: usize) -> Self {
        Self {
            contexts: RwLock::new(HashMap::new()),
            memories: RwLock::new(HashMap::new()),
            max_jobs,
        }
    }

    /// Create a new job context.
    pub async fn create_job(
        &self,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Uuid, JobError> {
        self.create_job_for_user("default", title, description)
            .await
    }

    /// Create a new job context for a specific user.
    pub async fn create_job_for_user(
        &self,
        user_id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Uuid, JobError> {
        // Hold write lock for the entire check-insert to prevent TOCTOU races
        // where two concurrent calls both pass the active_count check.
        let mut contexts = self.contexts.write().await;
        let active_count = contexts.values().filter(|c| c.state.is_active()).count();

        if active_count >= self.max_jobs {
            return Err(JobError::MaxJobsExceeded { max: self.max_jobs });
        }

        let context = JobContext::with_user(user_id, title, description);
        let job_id = context.job_id;
        contexts.insert(job_id, context);
        drop(contexts);

        let memory = Memory::new(job_id);
        self.memories.write().await.insert(job_id, memory);

        Ok(job_id)
    }

    /// Get a job context by ID.
    pub async fn get_context(&self, job_id: Uuid) -> Result<JobContext, JobError> {
        self.contexts
            .read()
            .await
            .get(&job_id)
            .cloned()
            .ok_or(JobError::NotFound { id: job_id })
    }

    /// Get a mutable reference to update a job context.
    pub async fn update_context<F, R>(&self, job_id: Uuid, f: F) -> Result<R, JobError>
    where
        F: FnOnce(&mut JobContext) -> R,
    {
        let mut contexts = self.contexts.write().await;
        let context = contexts
            .get_mut(&job_id)
            .ok_or(JobError::NotFound { id: job_id })?;
        Ok(f(context))
    }

    /// Get job memory.
    pub async fn get_memory(&self, job_id: Uuid) -> Result<Memory, JobError> {
        self.memories
            .read()
            .await
            .get(&job_id)
            .cloned()
            .ok_or(JobError::NotFound { id: job_id })
    }

    /// Update job memory.
    pub async fn update_memory<F, R>(&self, job_id: Uuid, f: F) -> Result<R, JobError>
    where
        F: FnOnce(&mut Memory) -> R,
    {
        let mut memories = self.memories.write().await;
        let memory = memories
            .get_mut(&job_id)
            .ok_or(JobError::NotFound { id: job_id })?;
        Ok(f(memory))
    }

    /// List all active job IDs.
    pub async fn active_jobs(&self) -> Vec<Uuid> {
        self.contexts
            .read()
            .await
            .iter()
            .filter(|(_, c)| c.state.is_active())
            .map(|(id, _)| *id)
            .collect()
    }

    /// List all job IDs.
    pub async fn all_jobs(&self) -> Vec<Uuid> {
        self.contexts.read().await.keys().cloned().collect()
    }

    /// List all active job IDs for a specific user.
    pub async fn active_jobs_for(&self, user_id: &str) -> Vec<Uuid> {
        self.contexts
            .read()
            .await
            .iter()
            .filter(|(_, c)| c.user_id == user_id && c.state.is_active())
            .map(|(id, _)| *id)
            .collect()
    }

    /// List all job IDs for a specific user.
    pub async fn all_jobs_for(&self, user_id: &str) -> Vec<Uuid> {
        self.contexts
            .read()
            .await
            .iter()
            .filter(|(_, c)| c.user_id == user_id)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Get count of active jobs.
    pub async fn active_count(&self) -> usize {
        self.contexts
            .read()
            .await
            .values()
            .filter(|c| c.state.is_active())
            .count()
    }

    /// Remove a completed job (cleanup).
    pub async fn remove_job(&self, job_id: Uuid) -> Result<(JobContext, Memory), JobError> {
        let context = self
            .contexts
            .write()
            .await
            .remove(&job_id)
            .ok_or(JobError::NotFound { id: job_id })?;

        let memory = self
            .memories
            .write()
            .await
            .remove(&job_id)
            .ok_or(JobError::NotFound { id: job_id })?;

        Ok((context, memory))
    }

    /// Find stuck jobs.
    pub async fn find_stuck_jobs(&self) -> Vec<Uuid> {
        self.contexts
            .read()
            .await
            .iter()
            .filter(|(_, c)| c.state == crate::context::JobState::Stuck)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Get summary of all jobs.
    pub async fn summary(&self) -> ContextSummary {
        let contexts = self.contexts.read().await;

        let mut summary = ContextSummary::default();
        for ctx in contexts.values() {
            match ctx.state {
                crate::context::JobState::Pending => summary.pending += 1,
                crate::context::JobState::InProgress => summary.in_progress += 1,
                crate::context::JobState::Completed => summary.completed += 1,
                crate::context::JobState::Submitted => summary.submitted += 1,
                crate::context::JobState::Accepted => summary.accepted += 1,
                crate::context::JobState::Failed => summary.failed += 1,
                crate::context::JobState::Stuck => summary.stuck += 1,
                crate::context::JobState::Cancelled => summary.cancelled += 1,
            }
        }

        summary.total = contexts.len();
        summary
    }

    /// Get summary of all jobs for a specific user.
    pub async fn summary_for(&self, user_id: &str) -> ContextSummary {
        let contexts = self.contexts.read().await;

        let mut summary = ContextSummary::default();
        for ctx in contexts.values().filter(|c| c.user_id == user_id) {
            match ctx.state {
                crate::context::JobState::Pending => summary.pending += 1,
                crate::context::JobState::InProgress => summary.in_progress += 1,
                crate::context::JobState::Completed => summary.completed += 1,
                crate::context::JobState::Submitted => summary.submitted += 1,
                crate::context::JobState::Accepted => summary.accepted += 1,
                crate::context::JobState::Failed => summary.failed += 1,
                crate::context::JobState::Stuck => summary.stuck += 1,
                crate::context::JobState::Cancelled => summary.cancelled += 1,
            }
        }

        summary.total = summary.pending
            + summary.in_progress
            + summary.completed
            + summary.submitted
            + summary.accepted
            + summary.failed
            + summary.stuck
            + summary.cancelled;
        summary
    }
}

impl Default for ContextManager {
    fn default() -> Self {
        Self::new(10)
    }
}

/// Summary of all job contexts.
#[derive(Debug, Default)]
pub struct ContextSummary {
    pub total: usize,
    pub pending: usize,
    pub in_progress: usize,
    pub completed: usize,
    pub submitted: usize,
    pub accepted: usize,
    pub failed: usize,
    pub stuck: usize,
    pub cancelled: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_job() {
        let manager = ContextManager::new(5);
        let job_id = manager.create_job("Test", "Description").await.unwrap();

        let context = manager.get_context(job_id).await.unwrap();
        assert_eq!(context.title, "Test");
    }

    #[tokio::test]
    async fn test_create_job_for_user_sets_user_id() {
        let manager = ContextManager::new(5);
        let job_id = manager
            .create_job_for_user("user-123", "Test", "Description")
            .await
            .unwrap();

        let context = manager.get_context(job_id).await.unwrap();
        assert_eq!(context.user_id, "user-123");
    }

    #[tokio::test]
    async fn test_max_jobs_limit() {
        let manager = ContextManager::new(2);

        manager.create_job("Job 1", "Desc").await.unwrap();
        manager.create_job("Job 2", "Desc").await.unwrap();

        // Start the jobs to make them active
        for job_id in manager.all_jobs().await {
            manager
                .update_context(job_id, |ctx| {
                    ctx.transition_to(crate::context::JobState::InProgress, None)
                })
                .await
                .unwrap()
                .unwrap();
        }

        // Third job should fail
        let result = manager.create_job("Job 3", "Desc").await;
        assert!(matches!(result, Err(JobError::MaxJobsExceeded { max: 2 })));
    }

    #[tokio::test]
    async fn test_update_context() {
        let manager = ContextManager::new(5);
        let job_id = manager.create_job("Test", "Desc").await.unwrap();

        manager
            .update_context(job_id, |ctx| {
                ctx.transition_to(crate::context::JobState::InProgress, None)
            })
            .await
            .unwrap()
            .unwrap();

        let context = manager.get_context(job_id).await.unwrap();
        assert_eq!(context.state, crate::context::JobState::InProgress);
    }

    // === QA Plan P3 - 4.2: Concurrent job stress tests ===

    #[tokio::test]
    async fn concurrent_creates_produce_unique_ids() {
        let manager = std::sync::Arc::new(ContextManager::new(100));

        let handles: Vec<_> = (0..50)
            .map(|i| {
                let mgr = std::sync::Arc::clone(&manager);
                tokio::spawn(async move {
                    mgr.create_job(
                        format!("Job {i}"),
                        format!("Desc {i}"),
                    )
                    .await
                })
            })
            .collect();

        let mut ids = std::collections::HashSet::new();
        for handle in handles {
            let result = handle.await.expect("task should not panic");
            let job_id = result.expect("create_job should succeed");
            assert!(ids.insert(job_id), "Duplicate job ID: {job_id}");
        }

        assert_eq!(ids.len(), 50);
        assert_eq!(manager.all_jobs().await.len(), 50);
    }

    #[tokio::test]
    async fn concurrent_creates_respect_max_jobs_limit() {
        // max_jobs = 5, but create_job only counts *active* jobs (InProgress).
        // Pending jobs don't count against the limit, so we need to transition them.
        let manager = std::sync::Arc::new(ContextManager::new(5));

        // First, create 5 jobs and make them active.
        for i in 0..5 {
            let id = manager.create_job(format!("Job {i}"), "desc").await.unwrap();
            manager
                .update_context(id, |ctx| {
                    ctx.transition_to(crate::context::JobState::InProgress, None)
                })
                .await
                .unwrap()
                .unwrap();
        }

        // Now try to create 10 more concurrently -- all should fail.
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let mgr = std::sync::Arc::clone(&manager);
                tokio::spawn(async move {
                    mgr.create_job(format!("Overflow {i}"), "desc").await
                })
            })
            .collect();

        for handle in handles {
            let result = handle.await.expect("task should not panic");
            assert!(
                matches!(result, Err(JobError::MaxJobsExceeded { .. })),
                "Expected MaxJobsExceeded, got: {:?}",
                result
            );
        }

        // Still exactly 5 jobs.
        assert_eq!(manager.all_jobs().await.len(), 5);
    }

    #[tokio::test]
    async fn concurrent_creates_and_reads_no_corruption() {
        let manager = std::sync::Arc::new(ContextManager::new(100));

        // Spawn writers that create jobs.
        let writer_handles: Vec<_> = (0..20)
            .map(|i| {
                let mgr = std::sync::Arc::clone(&manager);
                tokio::spawn(async move {
                    mgr.create_job_for_user(
                        format!("user-{}", i % 5),
                        format!("Job {i}"),
                        format!("Description for job {i}"),
                    )
                    .await
                })
            })
            .collect();

        // Concurrently, spawn readers that list jobs.
        let reader_handles: Vec<_> = (0..20)
            .map(|_| {
                let mgr = std::sync::Arc::clone(&manager);
                tokio::spawn(async move {
                    let _all = mgr.all_jobs().await;
                    let _active = mgr.active_jobs().await;
                    let _summary = mgr.summary().await;
                })
            })
            .collect();

        // Wait for all writers.
        let mut ids = Vec::new();
        for handle in writer_handles {
            let result = handle.await.expect("writer should not panic");
            ids.push(result.expect("create should succeed"));
        }

        // Wait for all readers.
        for handle in reader_handles {
            handle.await.expect("reader should not panic");
        }

        // All 20 jobs created with unique IDs.
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), 20);

        // Each user has 4 jobs (20 jobs / 5 users).
        for u in 0..5 {
            let user_jobs = manager.all_jobs_for(&format!("user-{u}")).await;
            assert_eq!(user_jobs.len(), 4, "user-{u} should have 4 jobs");
        }
    }

    #[tokio::test]
    async fn concurrent_updates_do_not_lose_state() {
        let manager = std::sync::Arc::new(ContextManager::new(100));

        // Create 10 jobs.
        let mut job_ids = Vec::new();
        for i in 0..10 {
            let id = manager.create_job(format!("Job {i}"), "desc").await.unwrap();
            job_ids.push(id);
        }

        // Concurrently transition all to InProgress.
        let handles: Vec<_> = job_ids
            .iter()
            .map(|&id| {
                let mgr = std::sync::Arc::clone(&manager);
                tokio::spawn(async move {
                    mgr.update_context(id, |ctx| {
                        ctx.transition_to(crate::context::JobState::InProgress, None)
                    })
                    .await
                })
            })
            .collect();

        for handle in handles {
            let result = handle.await.expect("task should not panic");
            result.expect("update should succeed").expect("transition should succeed");
        }

        // All 10 should now be InProgress.
        let active = manager.active_jobs().await;
        assert_eq!(active.len(), 10);
        for id in &job_ids {
            let ctx = manager.get_context(*id).await.unwrap();
            assert_eq!(ctx.state, crate::context::JobState::InProgress);
        }
    }
}
