use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::task::JoinSet;

use super::error::MultiAgentError;
use super::executor::{PlaceholderTaskExecutor, TaskExecutor};
use super::job_model::{AgentEvent, AgentJob, AgentStatus};
use super::planner::{DelegationPlanner, HeuristicDelegationPlanner};
use super::store::AgentJobStore;
use super::types::{AgentContext, ExecutionPlan, Task, TaskId};

const DEFAULT_CLAIM_LEASE: Duration = Duration::from_secs(30);

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

    /// Same worker/queue runtime as [`Self::new`], but with an injectable
    /// planner/executor pair. Used by tests that need to observe timing or
    /// behavior directly, and by future model/tool-backed integrations that
    /// want to reuse this same job-queue runtime instead of building a
    /// second one.
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

    /// Drive every job under `root_id` to a terminal state. Every tick claims
    /// *all* currently-pending jobs (master, delegated, and recursively
    /// delegated alike — they all flow through the same
    /// [`Self::process_claimed_job`] path) and runs them concurrently on the
    /// existing Tokio runtime via [`JoinSet`], so independent siblings created
    /// by the same delegation step actually overlap instead of running one at
    /// a time.
    pub async fn run_until_root_complete(
        self: Arc<Self>,
        root_id: &str,
    ) -> Result<(), MultiAgentError> {
        let mut in_flight: JoinSet<Result<(), MultiAgentError>> = JoinSet::new();
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

            let now = Utc::now();
            self.store.requeue_expired_claims(now)?;
            self.finalize_waiting_jobs()?;

            while let Some(job) = self
                .store
                .claim_next_pending(&self.worker_id, DEFAULT_CLAIM_LEASE, now)?
            {
                let worker = Arc::clone(&self);
                in_flight.spawn(async move { worker.process_claimed_job(job).await });
            }

            if !in_flight.is_empty() {
                if let Some(joined) = in_flight.join_next().await {
                    joined.map_err(|join_error| MultiAgentError::OrchestrationFailed {
                        reason: format!("worker task panicked: {join_error}"),
                    })??;
                }
                continue;
            }

            self.finalize_waiting_jobs()?;

            let root = self
                .store
                .get_job(root_id)?
                .ok_or_else(|| MultiAgentError::JobNotFound {
                    job_id: root_id.to_string(),
                })?;
            if root.is_terminal() {
                return Ok(());
            }

            if self.no_runnable_work(root_id)? {
                return Err(MultiAgentError::OrchestrationFailed {
                    reason: "multi-agent job run stalled with no claimable work".to_string(),
                });
            }
        }
    }

    async fn process_claimed_job(&self, mut job: AgentJob) -> Result<(), MultiAgentError> {
        job.status = AgentStatus::Running;
        job.updated_at = Utc::now();
        self.store.update_job(job.clone())?;
        self.emit_event(&job, AgentStatus::Running, "job started")?;

        let task = Task {
            id: TaskId(job.id.clone()),
            parent_id: job.parent_id.clone().map(TaskId::new),
            description: job.task.clone(),
            depth: job.depth,
        };
        let ctx = AgentContext::new(job.max_depth, self.max_iterations, Duration::from_secs(3600));
        self.ensure_no_cycle(&job)?;

        let plan = self.planner.plan(&task, &ctx);
        if plan.execute_directly || job.depth >= job.max_depth || plan.subtasks.is_empty() {
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

        // Every agent (master or delegated) makes the same local-vs-delegate
        // decision: keep one unit of work for itself, hand the rest to new
        // AgentRuns. This is what lets the master do useful work instead of
        // only ever coordinating.
        let execution_plan = ExecutionPlan::from_subtasks(plan.subtasks);
        if execution_plan.is_pure_local() {
            let mut solo_task = task.clone();
            solo_task.description = execution_plan.local_tasks[0].clone();
            match self.executor.execute(&solo_task).await {
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

        // Spawn delegated children *before* awaiting local work so they are
        // immediately claimable by other concurrent workers, letting this
        // agent's own local execution overlap with its delegated AgentRuns
        // instead of blocking on them.
        for subtask in &execution_plan.delegated_tasks {
            let child = AgentJob::new_child(
                self.store.next_job_id(),
                &job,
                subtask.clone(),
                self.max_retries,
            );
            self.store.insert_job(child)?;
        }
        let delegated_count = execution_plan.delegated_tasks.len();

        let mut local_task = task.clone();
        local_task.description = execution_plan.local_tasks[0].clone();
        let local_result = match self.executor.execute(&local_task).await {
            Ok(summary) => format!("local: {summary}"),
            Err(error) => format!("local failed: {error}"),
        };

        job.status = AgentStatus::WaitingForChildren;
        job.result = Some(local_result);
        job.claim_lease = None;
        job.updated_at = Utc::now();
        self.store.update_job(job.clone())?;
        self.emit_event(
            &job,
            AgentStatus::WaitingForChildren,
            format!(
                "kept 1 task locally, delegated {delegated_count} task(s): {:?}",
                execution_plan.delegated_tasks
            ),
        )?;
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
        self.emit_event(&job, AgentStatus::Failed, job.error.clone().unwrap_or_default())?;
        Ok(())
    }

    fn finalize_waiting_jobs(&self) -> Result<(), MultiAgentError> {
        for parent in self.store.jobs_waiting_for_children()? {
            let children = self.store.list_children(&parent.id)?;
            if children.is_empty() || !children.iter().all(|child| child.is_terminal()) {
                continue;
            }

            let failed = children
                .iter()
                .filter(|child| child.status == AgentStatus::Failed)
                .count();
            let completed = children
                .iter()
                .filter(|child| child.status == AgentStatus::Complete)
                .count();
            let cancelled = children
                .iter()
                .filter(|child| child.status == AgentStatus::Cancelled)
                .count();
            let child_summary = format!(
                "Aggregated {} child job(s): {completed} complete, {failed} failed, {cancelled} cancelled",
                children.len()
            );

            // `parent.result` already holds this agent's own local-task
            // outcome (set before children were spawned); fold the delegated
            // subtree's outcome into the same aggregated response.
            let local_note = parent.result.clone().unwrap_or_default();
            let local_failed = local_note.starts_with("local failed:");
            let summary = if local_note.is_empty() {
                child_summary.clone()
            } else {
                format!("{local_note}; {child_summary}")
            };

            let mut parent = parent;
            if local_failed && failed > 0 && completed == 0 {
                parent.status = AgentStatus::Failed;
                parent.error = Some(summary.clone());
            } else {
                parent.status = AgentStatus::Complete;
                parent.result = Some(summary.clone());
            }
            parent.claim_lease = None;
            parent.updated_at = Utc::now();
            self.store.update_job(parent.clone())?;
            self.emit_event(&parent, parent.status, summary)?;
        }
        Ok(())
    }

    fn no_runnable_work(&self, root_id: &str) -> Result<bool, MultiAgentError> {
        let jobs = self.store.list_jobs_for_root(root_id)?;
        let has_pending = jobs.iter().any(|job| job.status == AgentStatus::Pending);
        let has_active = jobs.iter().any(|job| {
            matches!(
                job.status,
                AgentStatus::Claimed | AgentStatus::Running | AgentStatus::WaitingForChildren
            )
        });
        Ok(!has_pending && !has_active)
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
                .and_then(|parent| parent.parent_id.clone());
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
