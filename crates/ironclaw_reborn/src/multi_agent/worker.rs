use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::task::JoinSet;

use super::error::MultiAgentError;
use super::executor::{PlaceholderTaskExecutor, TaskExecutor};
use super::job_model::{AgentJob, AgentStatus, ClaimLease};
use super::planner::{DelegationPlanner, HeuristicDelegationPlanner};
use super::store::AgentJobStore;
use super::types::{AgentContext, ExecutionPlan, Task, TaskId};
use super::AgentEvent;

const DEFAULT_CLAIM_LEASE_SECS: i64 = 3_600;

pub struct JobWorker {
    store: Arc<dyn AgentJobStore>,
    worker_id: String,
    planner: Arc<dyn DelegationPlanner>,
    executor: Arc<dyn TaskExecutor>,
    max_iterations: u32,
    max_retries: u32,
}

impl JobWorker {
    pub fn new(store: Arc<dyn AgentJobStore>, max_iterations: u32, max_retries: u32) -> Self {
        Self::with_planner_and_executor(
            store,
            max_iterations,
            max_retries,
            Arc::new(HeuristicDelegationPlanner),
            Arc::new(PlaceholderTaskExecutor),
        )
    }

    pub fn with_planner_and_executor(
        store: Arc<dyn AgentJobStore>,
        max_iterations: u32,
        max_retries: u32,
        planner: Arc<dyn DelegationPlanner>,
        executor: Arc<dyn TaskExecutor>,
    ) -> Self {
        Self {
            store,
            worker_id: "worker-1".to_string(),
            planner,
            executor,
            max_iterations,
            max_retries,
        }
    }

    /// Drive every job under `root_id` to a terminal state.
    ///
    /// The outer loop claims the root job and a few top-level pending jobs,
    /// but the heavy lifting happens recursively inside
    /// [`Self::process_claimed_job`]: when a job decides to delegate, it
    /// inserts children into the store (immediately marking them Claimed),
    /// then spawns its local work AND every child's `process_claimed_job`
    /// into the same [`JoinSet`] so they all start concurrently before any
    /// of them finishes.
    pub async fn run_until_root_complete(
        self: Arc<Self>,
        root_id: &str,
    ) -> Result<(), MultiAgentError> {
        let mut in_flight: JoinSet<Result<(), MultiAgentError>> = JoinSet::new();
        let now = Utc::now();

        loop {
            let root = self
                .store
                .get_job(root_id)?
                .ok_or_else(|| MultiAgentError::JobNotFound {
                    job_id: root_id.to_string(),
                })?;
            if root.is_terminal() {
                return Ok(());
            }

            self.store.requeue_expired_claims(now)?;

            while let Some(job) = self
                .store
                .claim_next_pending(&self.worker_id, Duration::from_secs(DEFAULT_CLAIM_LEASE_SECS as u64), now)?
            {
                let worker = Arc::clone(&self);
                in_flight.spawn(async move { worker.process_claimed_job(job).await });
            }

            if !in_flight.is_empty() {
                if let Some(joined) = in_flight.join_next().await {
                    joined.map_err(|e| MultiAgentError::OrchestrationFailed {
                        reason: format!("worker task panicked: {e}"),
                    })??;
                }
                continue;
            }

            // No in-flight tasks and root is not terminal — stalled.
            let root = self
                .store
                .get_job(root_id)?
                .ok_or_else(|| MultiAgentError::JobNotFound {
                    job_id: root_id.to_string(),
                })?;
            if root.is_terminal() {
                return Ok(());
            }
            return Err(MultiAgentError::OrchestrationFailed {
                reason: "multi-agent job stalled with no claimable work".to_string(),
            });
        }
    }

    /// Process one claimed job to completion.
    ///
    /// Returns a `BoxFuture` (explicit `Send + 'static`) rather than a plain
    /// `async fn` because the function is recursively spawned into a
    /// [`JoinSet`].  Rust cannot prove that a recursive `async fn` is `Send`,
    /// but it *can* prove it for a boxed concrete type — hence the explicit
    /// return type.
    ///
    /// When this job decides to delegate it:
    /// 1. Inserts children into the store already in `Claimed` state (so the
    ///    outer loop never double-claims them).
    /// 2. Spawns the local subtask AND every child's `process_claimed_job`
    ///    into a single [`JoinSet`] — all start at the same instant.
    /// 3. Awaits the whole set, then writes the aggregated result and marks
    ///    itself `Complete`.
    fn process_claimed_job(
        self: Arc<Self>,
        job: AgentJob,
    ) -> Pin<Box<dyn Future<Output = Result<(), MultiAgentError>> + Send + 'static>> {
        Box::pin(self.do_process(job))
    }

    async fn do_process(self: Arc<Self>, mut job: AgentJob) -> Result<(), MultiAgentError> {
        job.status = AgentStatus::Running;
        job.updated_at = Utc::now();
        self.store.update_job(job.clone())?;
        self.emit_event(&job, AgentStatus::Running, "started")?;

        let task = Task {
            id: TaskId(job.id.clone()),
            parent_id: job.parent_id.clone().map(TaskId::new),
            description: job.task.clone(),
            depth: job.depth,
        };
        let ctx = AgentContext::new(job.max_depth, self.max_iterations, Duration::from_secs(3_600));
        self.ensure_no_cycle(&job)?;

        let plan = self.planner.plan(&task, &ctx);

        // ── Local execution ────────────────────────────────────────────────
        if plan.execute_directly || plan.subtasks.is_empty() {
            let decision = format!("local  ({})", plan.reason);
            job.plan_decision = Some(decision.clone());
            self.emit_event(&job, AgentStatus::Running, format!("decision: {decision}"))?;

            match self.executor.execute(&task).await {
                Ok(result) => {
                    job.status = AgentStatus::Complete;
                    job.result = Some(result.clone());
                    job.claim_lease = None;
                    job.updated_at = Utc::now();
                    self.store.update_job(job.clone())?;
                    self.emit_event(&job, AgentStatus::Complete, result)?;
                }
                Err(error) => self.fail_or_retry(job, error).await?,
            }
            return Ok(());
        }

        // ── Split execution ────────────────────────────────────────────────
        let execution_plan = ExecutionPlan::from_subtasks(plan.subtasks.clone());
        let n_delegated = execution_plan.delegated_tasks.len();

        if execution_plan.is_pure_local() {
            // Only one subtask — execute directly even though the planner
            // returned subtasks (shouldn't normally happen, but be safe).
            let decision = format!("local  (single subtask after split)");
            job.plan_decision = Some(decision.clone());
            self.emit_event(&job, AgentStatus::Running, format!("decision: {decision}"))?;
            let mut solo = task.clone();
            solo.description = execution_plan.local_tasks[0].clone();
            match self.executor.execute(&solo).await {
                Ok(result) => {
                    job.status = AgentStatus::Complete;
                    job.result = Some(result.clone());
                    job.claim_lease = None;
                    job.updated_at = Utc::now();
                    self.store.update_job(job.clone())?;
                    self.emit_event(&job, AgentStatus::Complete, result)?;
                }
                Err(error) => self.fail_or_retry(job, error).await?,
            }
            return Ok(());
        }

        let decision = format!(
            "split: 1 local + {n_delegated} delegated  ({})",
            plan.reason
        );
        job.plan_decision = Some(decision.clone());
        self.emit_event(
            &job,
            AgentStatus::Running,
            format!(
                "decision: {decision} — spawning {} concurrent tasks",
                1 + n_delegated
            ),
        )?;

        // Pre-claim all children so the outer loop never double-claims them,
        // and build their AgentJob structs before entering the JoinSet so
        // that start timestamps are tightly grouped.
        let now = Utc::now();
        let lease_expiry =
            now + chrono::Duration::seconds(DEFAULT_CLAIM_LEASE_SECS);
        let mut child_jobs: Vec<AgentJob> = execution_plan
            .delegated_tasks
            .iter()
            .map(|subtask| {
                let mut child = AgentJob::new_child(
                    self.store.next_job_id(),
                    &job,
                    subtask.clone(),
                    self.max_retries,
                );
                child.status = AgentStatus::Claimed;
                child.claim_lease = Some(ClaimLease {
                    worker_id: self.worker_id.clone(),
                    claimed_at: now,
                    expires_at: lease_expiry,
                });
                child
            })
            .collect();

        for child in &child_jobs {
            self.store.insert_job(child.clone())?;
        }

        // Build the local task description before entering the JoinSet.
        let local_desc = execution_plan.local_tasks[0].clone();

        // ── Concurrent launch: local task + all delegated AgentRuns ──────────
        let mut set: JoinSet<Result<Option<String>, MultiAgentError>> = JoinSet::new();

        // Local subtask (returns Some(result))
        {
            let executor = Arc::clone(&self.executor);
            let mut local_task = task.clone();
            local_task.description = local_desc;
            set.spawn(async move {
                let result = executor.execute(&local_task).await?;
                Ok(Some(result))
            });
        }

        // Each delegated AgentRun (handles its own store updates, returns None)
        for child in child_jobs.drain(..) {
            let worker = Arc::clone(&self);
            set.spawn(async move {
                worker.process_claimed_job(child).await?;
                Ok(None)
            });
        }

        // ── Collect results (all running in parallel) ─────────────────────
        let mut local_result = String::new();
        let mut any_error: Option<MultiAgentError> = None;
        while let Some(joined) = set.join_next().await {
            match joined.map_err(|e| MultiAgentError::OrchestrationFailed {
                reason: format!("spawned task panicked: {e}"),
            })? {
                Ok(Some(result)) => local_result = result,
                Ok(None) => {}
                Err(e) => any_error = Some(e),
            }
        }

        // ── Aggregate ─────────────────────────────────────────────────────
        // Read final child statuses from the store for the summary.
        let children = self.store.list_children(&job.id)?;
        let completed_children = children
            .iter()
            .filter(|c| c.status == AgentStatus::Complete)
            .count();
        let failed_children = children
            .iter()
            .filter(|c| c.status == AgentStatus::Failed)
            .count();

        let aggregate = format!(
            "Aggregated {n_delegated} child job(s): \
             {completed_children} complete, {failed_children} failed"
        );

        let combined_result = if local_result.is_empty() {
            aggregate.clone()
        } else {
            format!("local: {local_result}\n{aggregate}")
        };

        if let Some(err) = any_error {
            if completed_children == 0 && failed_children > 0 {
                self.fail_or_retry(job, err).await?;
                return Ok(());
            }
        }

        job.status = AgentStatus::Complete;
        job.result = Some(combined_result.clone());
        job.claim_lease = None;
        job.updated_at = Utc::now();
        self.store.update_job(job.clone())?;
        self.emit_event(&job, AgentStatus::Complete, combined_result)?;
        Ok(())
    }

    async fn fail_or_retry(
        &self,
        mut job: AgentJob,
        error: MultiAgentError,
    ) -> Result<(), MultiAgentError> {
        if job.retry_count < job.max_retries {
            job.retry_count = job.retry_count.saturating_add(1);
            job.status = AgentStatus::Pending;
            job.claim_lease = None;
            job.error = Some(error.to_string());
            job.updated_at = Utc::now();
            self.store.update_job(job.clone())?;
            self.emit_event(
                &job,
                AgentStatus::Pending,
                format!("retry {}/{} scheduled", job.retry_count, job.max_retries),
            )?;
            return Ok(());
        }
        job.status = AgentStatus::Failed;
        job.error = Some(error.to_string());
        job.claim_lease = None;
        job.updated_at = Utc::now();
        self.store.update_job(job.clone())?;
        self.emit_event(
            &job,
            AgentStatus::Failed,
            job.error.clone().unwrap_or_default(),
        )?;
        Ok(())
    }

    fn ensure_no_cycle(&self, job: &AgentJob) -> Result<(), MultiAgentError> {
        let mut current = job.parent_id.clone();
        while let Some(parent_id) = current {
            if parent_id == job.id {
                return Err(MultiAgentError::CycleDetected {
                    task_id: job.id.clone(),
                });
            }
            current = self
                .store
                .get_job(&parent_id)?
                .and_then(|p| p.parent_id.clone());
        }
        Ok(())
    }

    fn emit_event(
        &self,
        job: &AgentJob,
        status: AgentStatus,
        message: impl Into<String>,
    ) -> Result<(), MultiAgentError> {
        let event = AgentEvent::new(self.store.next_event_id(), job, status, message);
        self.store.append_event(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multi_agent::runtime::{MultiAgentRunConfig, run_multi_agent_jobs};
    use std::time::Duration;

    #[tokio::test]
    async fn worker_sets_plan_decision_on_every_job() {
        let report = run_multi_agent_jobs(
            "task one; task two; task three",
            MultiAgentRunConfig::new(2, 32, Duration::from_secs(30), 0),
        )
        .await
        .expect("run");
        for job in &report.jobs {
            assert!(
                job.plan_decision.is_some(),
                "job {} has no plan_decision",
                job.id
            );
        }
    }

    #[tokio::test]
    async fn worker_runs_delegated_children_concurrently() {
        use crate::multi_agent::executor::TaskExecutor;
        use crate::multi_agent::runtime::run_multi_agent_jobs_with;
        use crate::multi_agent::planner::HeuristicDelegationPlanner;
        use async_trait::async_trait;

        // Executor that sleeps 50 ms per task so concurrency is measurable.
        struct SleepingExecutor;
        #[async_trait]
        impl TaskExecutor for SleepingExecutor {
            async fn execute(&self, task: &Task) -> Result<String, MultiAgentError> {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok(format!("done: {}", task.description))
            }
        }

        let start = std::time::Instant::now();
        run_multi_agent_jobs_with(
            "part one; part two; part three",
            MultiAgentRunConfig::new(2, 32, Duration::from_secs(10), 0),
            Arc::new(HeuristicDelegationPlanner),
            Arc::new(SleepingExecutor),
        )
        .await
        .expect("sleeping run");

        // If all 3 subtasks ran sequentially each taking 50 ms, wall time
        // would be ≥ 150 ms.  Concurrent execution should finish in ~ 50 ms.
        // We allow up to 140 ms to avoid flakiness on slow CI.
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(140),
            "expected concurrent execution (~50ms), got {:?}",
            elapsed
        );
    }
}
