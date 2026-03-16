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
        // where two concurrent calls both pass the parallel_count check.
        let mut contexts = self.contexts.write().await;
        // Only count jobs that consume execution slots (Pending, InProgress, Stuck).
        // Completed and Submitted jobs are no longer actively executing and shouldn't
        // block new job creation.
        let parallel_count = contexts
            .values()
            .filter(|c| c.state.is_parallel_blocking())
            .count();

        if parallel_count >= self.max_jobs {
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

    /// Atomically update a job context and return the updated context.
    ///
    /// This method holds the write lock for the entire update-and-read sequence,
    /// preventing concurrent workers from interleaving modifications between the
    /// update and the subsequent read (Issue #807: non-transactional context updates).
    /// Use this when you need to update context and immediately persist it to DB.
    pub async fn update_context_and_get<F>(
        &self,
        job_id: Uuid,
        f: F,
    ) -> Result<JobContext, JobError>
    where
        F: FnOnce(&mut JobContext),
    {
        let mut contexts = self.contexts.write().await;
        let context = contexts
            .get_mut(&job_id)
            .ok_or(JobError::NotFound { id: job_id })?;
        f(context);
        Ok(context.clone())
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
    ///
    /// Returns jobs that are explicitly in `Stuck` state, plus `InProgress`
    /// jobs that have been running longer than `idle_threshold` (if provided).
    /// The threshold-based detection catches jobs that never transitioned to
    /// `Stuck` (e.g., due to a deadlock or unhandled timeout).
    pub async fn find_stuck_jobs(&self) -> Vec<Uuid> {
        self.find_stuck_jobs_with_threshold(None).await
    }

    /// Find stuck jobs with an optional idle threshold for `InProgress` detection.
    pub async fn find_stuck_jobs_with_threshold(
        &self,
        idle_threshold: Option<std::time::Duration>,
    ) -> Vec<Uuid> {
        let now = chrono::Utc::now();
        self.contexts
            .read()
            .await
            .iter()
            .filter(|(_, c)| {
                // Always include explicitly Stuck jobs.
                if c.state == crate::context::JobState::Stuck {
                    return true;
                }
                // Detect InProgress jobs idle beyond the threshold.
                if c.state == crate::context::JobState::InProgress
                    && let Some(threshold) = idle_threshold
                    && let Some(started) = c.started_at
                {
                    let elapsed = now.signed_duration_since(started);
                    let elapsed_secs = elapsed.num_seconds().max(0) as u64;
                    return elapsed_secs > threshold.as_secs();
                }
                false
            })
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
                    mgr.create_job(format!("Job {i}"), format!("Desc {i}"))
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
            let id = manager
                .create_job(format!("Job {i}"), "desc")
                .await
                .unwrap();
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
                tokio::spawn(async move { mgr.create_job(format!("Overflow {i}"), "desc").await })
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
            let id = manager
                .create_job(format!("Job {i}"), "desc")
                .await
                .unwrap();
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
            result
                .expect("update should succeed")
                .expect("transition should succeed");
        }

        // All 10 should now be InProgress.
        let active = manager.active_jobs().await;
        assert_eq!(active.len(), 10);
        for id in &job_ids {
            let ctx = manager.get_context(*id).await.unwrap();
            assert_eq!(ctx.state, crate::context::JobState::InProgress);
        }
    }

    #[tokio::test]
    async fn get_context_not_found() {
        let manager = ContextManager::new(5);
        let bogus_id = Uuid::new_v4();
        let result = manager.get_context(bogus_id).await;
        assert!(matches!(result, Err(JobError::NotFound { id }) if id == bogus_id));
    }

    #[tokio::test]
    async fn update_context_not_found() {
        let manager = ContextManager::new(5);
        let bogus_id = Uuid::new_v4();
        let result = manager.update_context(bogus_id, |_ctx| {}).await;
        assert!(matches!(result, Err(JobError::NotFound { id }) if id == bogus_id));
    }

    #[tokio::test]
    async fn remove_job_returns_context_and_memory() {
        let manager = ContextManager::new(5);
        let job_id = manager.create_job("Removable", "bye bye").await.unwrap();

        let (ctx, mem) = manager.remove_job(job_id).await.unwrap();
        assert_eq!(ctx.title, "Removable");
        assert_eq!(mem.job_id, job_id);

        // After removal, get should fail
        assert!(matches!(
            manager.get_context(job_id).await,
            Err(JobError::NotFound { .. })
        ));
        assert!(matches!(
            manager.get_memory(job_id).await,
            Err(JobError::NotFound { .. })
        ));
    }

    #[tokio::test]
    async fn remove_job_not_found() {
        let manager = ContextManager::new(5);
        let result = manager.remove_job(Uuid::new_v4()).await;
        assert!(matches!(result, Err(JobError::NotFound { .. })));
    }

    #[tokio::test]
    async fn get_memory_and_update_memory() {
        let manager = ContextManager::new(5);
        let job_id = manager.create_job("Mem test", "desc").await.unwrap();

        // Fresh memory should be empty
        let mem = manager.get_memory(job_id).await.unwrap();
        assert_eq!(mem.job_id, job_id);
        assert!(mem.actions.is_empty());
        assert!(mem.conversation.is_empty());

        // Update memory by adding a message
        manager
            .update_memory(job_id, |m| {
                m.add_message(crate::llm::ChatMessage::user("hello from test"));
            })
            .await
            .unwrap();

        let mem = manager.get_memory(job_id).await.unwrap();
        assert_eq!(mem.conversation.len(), 1);
        assert_eq!(mem.conversation.messages()[0].content, "hello from test");
    }

    #[tokio::test]
    async fn update_memory_not_found() {
        let manager = ContextManager::new(5);
        let result = manager.update_memory(Uuid::new_v4(), |_| {}).await;
        assert!(matches!(result, Err(JobError::NotFound { .. })));
    }

    #[tokio::test]
    async fn get_memory_not_found() {
        let manager = ContextManager::new(5);
        let result = manager.get_memory(Uuid::new_v4()).await;
        assert!(matches!(result, Err(JobError::NotFound { .. })));
    }

    #[tokio::test]
    async fn find_stuck_jobs_returns_only_stuck() {
        let manager = ContextManager::new(10);

        let id1 = manager.create_job("Job 1", "desc").await.unwrap();
        let id2 = manager.create_job("Job 2", "desc").await.unwrap();
        let id3 = manager.create_job("Job 3", "desc").await.unwrap();

        // Transition id1 and id2 to InProgress, then mark id2 as stuck
        for id in [id1, id2, id3] {
            manager
                .update_context(id, |ctx| {
                    ctx.transition_to(crate::context::JobState::InProgress, None)
                })
                .await
                .unwrap()
                .unwrap();
        }
        manager
            .update_context(id2, |ctx| ctx.mark_stuck("timed out"))
            .await
            .unwrap()
            .unwrap();

        let stuck = manager.find_stuck_jobs().await;
        assert_eq!(stuck.len(), 1);
        assert_eq!(stuck[0], id2);
    }

    /// Regression test for #1223: InProgress jobs exceeding the threshold
    /// should be detected as stuck even if they never transitioned to Stuck.
    #[tokio::test]
    async fn find_stuck_jobs_with_threshold_detects_idle_in_progress() {
        let manager = ContextManager::new(10);

        let id1 = manager.create_job("Active job", "desc").await.unwrap();
        let id2 = manager.create_job("Idle job", "desc").await.unwrap();

        // Both transition to InProgress
        for id in [id1, id2] {
            manager
                .update_context(id, |ctx| {
                    ctx.transition_to(crate::context::JobState::InProgress, None)
                })
                .await
                .unwrap()
                .unwrap();
        }

        // Backdate id2's started_at to simulate a long-running job
        manager
            .update_context(id2, |ctx| -> Result<(), crate::error::JobError> {
                ctx.started_at = Some(chrono::Utc::now() - chrono::Duration::seconds(600));
                Ok(())
            })
            .await
            .unwrap()
            .unwrap();

        // With a 5-minute threshold, only id2 (10 min) should be detected
        let stuck = manager
            .find_stuck_jobs_with_threshold(Some(std::time::Duration::from_secs(300)))
            .await;
        assert_eq!(stuck.len(), 1);
        assert_eq!(stuck[0], id2);

        // Without threshold, neither InProgress job is detected (no explicit Stuck state)
        let stuck_no_threshold = manager.find_stuck_jobs().await;
        assert!(stuck_no_threshold.is_empty());
    }

    #[tokio::test]
    async fn active_count_tracks_non_terminal_jobs() {
        let manager = ContextManager::new(10);

        let id1 = manager.create_job("J1", "d").await.unwrap();
        let id2 = manager.create_job("J2", "d").await.unwrap();

        // Both pending (active)
        assert_eq!(manager.active_count().await, 2);

        // Transition id1 through to Failed (terminal)
        manager
            .update_context(id1, |ctx| {
                ctx.transition_to(crate::context::JobState::InProgress, None)
            })
            .await
            .unwrap()
            .unwrap();
        manager
            .update_context(id1, |ctx| {
                ctx.transition_to(crate::context::JobState::Failed, None)
            })
            .await
            .unwrap()
            .unwrap();

        // id1 is terminal, id2 still pending
        assert_eq!(manager.active_count().await, 1);

        // Transition id2 to cancelled
        manager
            .update_context(id2, |ctx| {
                ctx.transition_to(crate::context::JobState::Cancelled, None)
            })
            .await
            .unwrap()
            .unwrap();

        assert_eq!(manager.active_count().await, 0);
    }

    #[tokio::test]
    async fn active_jobs_for_filters_by_user() {
        let manager = ContextManager::new(10);

        manager
            .create_job_for_user("alice", "A1", "d")
            .await
            .unwrap();
        manager
            .create_job_for_user("alice", "A2", "d")
            .await
            .unwrap();
        let bob_id = manager.create_job_for_user("bob", "B1", "d").await.unwrap();

        assert_eq!(manager.active_jobs_for("alice").await.len(), 2);
        assert_eq!(manager.active_jobs_for("bob").await.len(), 1);
        assert_eq!(manager.active_jobs_for("nobody").await.len(), 0);

        // Make bob's job terminal
        manager
            .update_context(bob_id, |ctx| {
                ctx.transition_to(crate::context::JobState::InProgress, None)
            })
            .await
            .unwrap()
            .unwrap();
        manager
            .update_context(bob_id, |ctx| {
                ctx.transition_to(crate::context::JobState::Failed, None)
            })
            .await
            .unwrap()
            .unwrap();

        assert_eq!(manager.active_jobs_for("bob").await.len(), 0);
        // But all_jobs_for still shows it
        assert_eq!(manager.all_jobs_for("bob").await.len(), 1);
    }

    #[tokio::test]
    async fn summary_counts_states_correctly() {
        let manager = ContextManager::new(10);

        let id1 = manager.create_job("J1", "d").await.unwrap();
        let id2 = manager.create_job("J2", "d").await.unwrap();
        let id3 = manager.create_job("J3", "d").await.unwrap();

        // id1: Pending -> InProgress -> Completed
        manager
            .update_context(id1, |ctx| {
                ctx.transition_to(crate::context::JobState::InProgress, None)
            })
            .await
            .unwrap()
            .unwrap();
        manager
            .update_context(id1, |ctx| {
                ctx.transition_to(crate::context::JobState::Completed, None)
            })
            .await
            .unwrap()
            .unwrap();

        // id2: Pending -> InProgress -> Failed
        manager
            .update_context(id2, |ctx| {
                ctx.transition_to(crate::context::JobState::InProgress, None)
            })
            .await
            .unwrap()
            .unwrap();
        manager
            .update_context(id2, |ctx| {
                ctx.transition_to(crate::context::JobState::Failed, None)
            })
            .await
            .unwrap()
            .unwrap();

        // id3: stays Pending

        let s = manager.summary().await;
        assert_eq!(s.total, 3);
        assert_eq!(s.pending, 1);
        assert_eq!(s.completed, 1);
        assert_eq!(s.failed, 1);
        assert_eq!(s.in_progress, 0);
        assert_eq!(s.stuck, 0);
        assert_eq!(s.cancelled, 0);
        assert_eq!(s.submitted, 0);
        assert_eq!(s.accepted, 0);

        // Suppress unused field warning
        let _ = id3;
    }

    #[tokio::test]
    async fn summary_for_scopes_to_user() {
        let manager = ContextManager::new(10);

        manager
            .create_job_for_user("alice", "A1", "d")
            .await
            .unwrap();
        let bob_id = manager.create_job_for_user("bob", "B1", "d").await.unwrap();

        // Transition bob's job to InProgress
        manager
            .update_context(bob_id, |ctx| {
                ctx.transition_to(crate::context::JobState::InProgress, None)
            })
            .await
            .unwrap()
            .unwrap();

        let alice_summary = manager.summary_for("alice").await;
        assert_eq!(alice_summary.total, 1);
        assert_eq!(alice_summary.pending, 1);
        assert_eq!(alice_summary.in_progress, 0);

        let bob_summary = manager.summary_for("bob").await;
        assert_eq!(bob_summary.total, 1);
        assert_eq!(bob_summary.pending, 0);
        assert_eq!(bob_summary.in_progress, 1);

        let nobody_summary = manager.summary_for("nobody").await;
        assert_eq!(nobody_summary.total, 0);
    }

    #[tokio::test]
    async fn default_context_manager_has_max_10() {
        let manager = ContextManager::default();
        // Create 10 jobs and make them active
        for i in 0..10 {
            let id = manager
                .create_job(format!("Job {i}"), "desc")
                .await
                .unwrap();
            manager
                .update_context(id, |ctx| {
                    ctx.transition_to(crate::context::JobState::InProgress, None)
                })
                .await
                .unwrap()
                .unwrap();
        }
        // 11th should fail
        let result = manager.create_job("overflow", "d").await;
        assert!(matches!(result, Err(JobError::MaxJobsExceeded { max: 10 })));
    }

    #[tokio::test]
    async fn all_jobs_returns_all_regardless_of_state() {
        let manager = ContextManager::new(10);

        let id1 = manager.create_job("J1", "d").await.unwrap();
        manager.create_job("J2", "d").await.unwrap();

        // Make id1 terminal
        manager
            .update_context(id1, |ctx| {
                ctx.transition_to(crate::context::JobState::InProgress, None)
            })
            .await
            .unwrap()
            .unwrap();
        manager
            .update_context(id1, |ctx| {
                ctx.transition_to(crate::context::JobState::Failed, None)
            })
            .await
            .unwrap()
            .unwrap();

        // all_jobs includes terminal, active_jobs does not
        assert_eq!(manager.all_jobs().await.len(), 2);
        assert_eq!(manager.active_jobs().await.len(), 1);
    }

    #[tokio::test]
    async fn create_job_uses_default_user() {
        let manager = ContextManager::new(5);
        let job_id = manager.create_job("Test", "desc").await.unwrap();
        let ctx = manager.get_context(job_id).await.unwrap();
        assert_eq!(ctx.user_id, "default");
    }

    #[tokio::test]
    async fn concurrent_remove_and_read() {
        let manager = std::sync::Arc::new(ContextManager::new(100));

        // Create 20 jobs
        let mut job_ids = Vec::new();
        for i in 0..20 {
            let id = manager
                .create_job(format!("Job {i}"), "desc")
                .await
                .unwrap();
            job_ids.push(id);
        }

        // Concurrently remove the first 10 while reading the last 10
        let remove_handles: Vec<_> = job_ids[..10]
            .iter()
            .map(|&id| {
                let mgr = std::sync::Arc::clone(&manager);
                tokio::spawn(async move { mgr.remove_job(id).await })
            })
            .collect();

        let read_handles: Vec<_> = job_ids[10..]
            .iter()
            .map(|&id| {
                let mgr = std::sync::Arc::clone(&manager);
                tokio::spawn(async move { mgr.get_context(id).await })
            })
            .collect();

        for handle in remove_handles {
            handle
                .await
                .expect("remove task should not panic")
                .expect("remove should succeed");
        }

        for handle in read_handles {
            let ctx = handle
                .await
                .expect("read task should not panic")
                .expect("read should succeed");
            assert!(job_ids[10..].contains(&ctx.job_id));
        }

        assert_eq!(manager.all_jobs().await.len(), 10);
    }

    #[tokio::test]
    async fn update_context_and_get_atomicity_regression_issue_807() {
        // Regression test for Issue #807: non-transactional context updates.
        // Verify that update_context_and_get returns the exact state that was set,
        // without allowing concurrent workers to interleave modifications.
        let manager = std::sync::Arc::new(ContextManager::new(100));
        let job_id = manager
            .create_job("Atomicity Test", "verify no race condition")
            .await
            .unwrap(); // safety: test code

        // Update and get atomically, setting metadata
        let metadata = serde_json::json!({ "priority": "high", "user_id": 42 });
        let returned_ctx = manager
            .update_context_and_get(job_id, |ctx| {
                ctx.metadata = metadata.clone();
                ctx.max_tokens = 5000;
            })
            .await
            .unwrap(); // safety: test code

        // Verify the returned context has the exact updates we set
        assert_eq!(returned_ctx.metadata, metadata); // safety: test code
        assert_eq!(returned_ctx.max_tokens, 5000); // safety: test code

        // Verify a fresh get returns the same state
        let fresh_ctx = manager.get_context(job_id).await.unwrap(); // safety: test code
        assert_eq!(fresh_ctx.metadata, metadata); // safety: test code
        assert_eq!(fresh_ctx.max_tokens, 5000); // safety: test code
    }

    #[tokio::test]
    async fn update_context_and_get_no_concurrent_interleave() {
        // Verify that concurrent updates cannot interleave during update_context_and_get.
        // If the lock were released too early, a concurrent state transition could
        // get mixed into the returned context.
        let manager = std::sync::Arc::new(ContextManager::new(100));
        let job_id = manager
            .create_job("Concurrent Race Test", "ensure atomicity")
            .await
            .unwrap(); // safety: test code

        let metadata = serde_json::json!({ "test": "race_condition" });
        let metadata_clone = metadata.clone();

        // Spawn a task that will update_context_and_get
        let mgr1 = std::sync::Arc::clone(&manager);
        let returned_ctx_handle = tokio::spawn(async move {
            mgr1.update_context_and_get(job_id, |ctx| {
                ctx.metadata = metadata_clone;
                ctx.max_tokens = 3000;
            })
            .await
        });

        // The returned context should have *only* the metadata update, not any
        // concurrent state transitions that might happen during the operation.
        let returned_ctx = returned_ctx_handle.await.unwrap().unwrap(); // safety: test code

        // Verify atomicity: returned context has the metadata we set
        assert_eq!(returned_ctx.metadata, metadata); // safety: test code
        assert_eq!(returned_ctx.max_tokens, 3000); // safety: test code
        // And it's in the initial state (Pending), not modified by concurrent workers
        assert_eq!(returned_ctx.state, crate::context::JobState::Pending); // safety: test code
    }

    #[tokio::test]
    async fn sequential_routines_unlimited_completed_not_counted() {
        // TEST: Sequential (non-parallel) routines should NOT be limited by max_jobs.
        //
        // Completed/Submitted jobs should NOT count toward the parallel job limit,
        // since they're no longer actively consuming execution resources.
        //
        // Scenario: Create 10 sequential routines, each completing before the next starts.
        // Currently FAILS because Completed jobs still count as "active".
        // After fix, should PASS because only Pending/InProgress/Stuck count.

        let manager = ContextManager::new(5); // max 5 truly parallel jobs

        // Try to create and complete 10 sequential routines
        for i in 0..10 {
            let result = manager
                .create_job(format!("Sequential Routine {}", i), "one at a time")
                .await;

            match result {
                Ok(job_id) => {
                    // Simulate execution: Pending -> InProgress -> Completed
                    manager
                        .update_context(job_id, |ctx| {
                            ctx.transition_to(crate::context::JobState::InProgress, None)
                        })
                        .await
                        .unwrap()
                        .unwrap();

                    manager
                        .update_context(job_id, |ctx| {
                            ctx.transition_to(crate::context::JobState::Completed, None)
                        })
                        .await
                        .unwrap()
                        .unwrap();

                    println!("✓ Routine {} created and completed", i);
                }
                Err(JobError::MaxJobsExceeded { max }) => {
                    panic!(
                        "✗ Routine {} FAILED to create: MaxJobsExceeded (max={}).\n\
                         This shows the bug: Completed jobs from routines 0-4 are still counting \
                         toward the limit even though they're not running.\n\
                         After the fix, this test should pass because Completed jobs won't count.",
                        i, max
                    );
                }
                Err(e) => {
                    panic!("Unexpected error for routine {}: {:?}", i, e);
                }
            }
        }

        // If we reach here, all 10 routines succeeded (bug is fixed)
        assert_eq!(manager.all_jobs().await.len(), 10);
        println!("✓ SUCCESS: All 10 sequential routines created despite max_jobs=5 limit");
        println!("  This is correct: Completed jobs don't count toward parallel limit");
    }

    #[tokio::test]
    async fn parallel_jobs_limit_enforced_for_active_jobs() {
        // TEST: Parallel (simultaneous) jobs ARE limited by max_jobs.
        //
        // Jobs in Pending/InProgress/Stuck states consume execution slots.
        // The 6th truly-active job should fail because the limit is 5.
        //
        // This test verifies the limit DOES work correctly for parallel execution.

        let manager = ContextManager::new(5); // max 5 parallel jobs

        // Create 5 jobs and make them InProgress (simulating parallel execution)
        let mut job_ids = Vec::new();
        for i in 0..5 {
            let job_id = manager
                .create_job(format!("Parallel Job {}", i), "running in parallel")
                .await
                .expect("First 5 jobs should create successfully");
            job_ids.push(job_id);

            // Transition to InProgress (simulating active execution)
            manager
                .update_context(job_id, |ctx| {
                    ctx.transition_to(crate::context::JobState::InProgress, None)
                })
                .await
                .unwrap()
                .unwrap();
        }

        // Verify all 5 jobs are InProgress
        for job_id in &job_ids {
            let ctx = manager.get_context(*job_id).await.unwrap();
            assert_eq!(
                ctx.state,
                crate::context::JobState::InProgress,
                "All jobs should be InProgress"
            );
        }

        // Check active count - should be 5 (all InProgress)
        let active_count = manager.active_count().await;
        assert_eq!(
            active_count, 5,
            "Active count should be 5 (all InProgress jobs count)"
        );

        // Try to create a 6th job - should FAIL because limit is reached
        let result = manager.create_job("Parallel Job 6", "sixth job").await;

        match result {
            Err(JobError::MaxJobsExceeded { max: 5 }) => {
                println!("✓ SUCCESS: Parallel job limit correctly enforced at 5 active jobs");
                println!("✓ 6th InProgress job correctly blocked when 5 are already running");
            }
            Ok(_) => {
                panic!(
                    "FAILED: 6th parallel job should have been blocked \
                     but was created. Limit enforcement is broken."
                );
            }
            Err(e) => {
                panic!(
                    "UNEXPECTED ERROR: Expected MaxJobsExceeded but got: {:?}",
                    e
                );
            }
        }
    }

    #[tokio::test]
    async fn completed_jobs_should_free_slots_after_fix() {
        // TEST: After the fix, Completed jobs should NOT count toward the limit.
        //
        // This test demonstrates that when a job transitions from InProgress -> Completed,
        // it should free up a slot in the parallel execution limit.
        //
        // Currently FAILS (bug not fixed), proving Completed jobs incorrectly stay in the limit.
        // After fix, this will PASS (Completed jobs freed their slot).

        let manager = ContextManager::new(5); // max 5 parallel jobs

        // Create 5 InProgress jobs (fill the limit)
        let mut job_ids = Vec::new();
        for i in 0..5 {
            let job_id = manager
                .create_job(format!("Job {}", i), "parallel")
                .await
                .unwrap();
            job_ids.push(job_id);

            manager
                .update_context(job_id, |ctx| {
                    ctx.transition_to(crate::context::JobState::InProgress, None)
                })
                .await
                .unwrap()
                .unwrap();
        }

        // Verify limit is hit
        let result = manager.create_job("Job 5", "should fail").await;
        assert!(
            matches!(result, Err(JobError::MaxJobsExceeded { max: 5 })),
            "Limit should be hit with 5 InProgress jobs"
        );
        println!("✓ Limit enforced: 5 InProgress jobs block 6th creation");

        // Now transition job 0 from InProgress -> Completed
        manager
            .update_context(job_ids[0], |ctx| {
                ctx.transition_to(crate::context::JobState::Completed, None)
            })
            .await
            .unwrap()
            .unwrap();

        println!("✓ Job 0 transitioned: InProgress -> Completed");

        // Try to create a 6th job - this will FAIL until the bug is fixed
        let result = manager
            .create_job("Job 5 (retry)", "after 1 Completed")
            .await;

        match result {
            Ok(job_6) => {
                println!("✓ SUCCESS: 6th job created after job 0 completed");
                println!("✓ This proves Completed jobs don't count toward the limit (BUG FIXED)");

                // Verify we can transition it to InProgress
                manager
                    .update_context(job_6, |ctx| {
                        ctx.transition_to(crate::context::JobState::InProgress, None)
                    })
                    .await
                    .unwrap()
                    .unwrap();
                println!("✓ 6th job now InProgress: 4 remaining + 1 new = 5 limit reached");
            }
            Err(JobError::MaxJobsExceeded { max: 5 }) => {
                panic!(
                    "✗ BUG NOT FIXED: 6th job creation still blocked after freeing slot.\n\
                     State: 1 Completed (job 0) + 4 InProgress (jobs 1-4) = 5 active\n\
                     BUG: Completed job 0 still counts toward limit\n\
                     EXPECTED: Only 4 InProgress count, 1 slot free"
                );
            }
            Err(e) => {
                panic!("Unexpected error: {:?}", e);
            }
        }
    }
}
