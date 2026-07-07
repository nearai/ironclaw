use std::sync::Arc;
use std::time::Duration;

use super::error::MultiAgentError;
use super::executor::{PlaceholderTaskExecutor, TaskExecutor};
use super::job_model::{AgentJob, AgentStatus};
use super::planner::{DelegationPlanner, HeuristicDelegationPlanner};
use super::report::aggregate_final_summary;
use super::store::{AgentJobStore, InMemoryAgentJobStore};
use super::types::{MultiAgentRunReport, TaskId, TaskResult};
use super::worker::JobWorker;

#[derive(Debug, Clone)]
pub struct MultiAgentRunConfig {
    pub max_depth: u32,
    pub max_iterations: u32,
    pub task_timeout: Duration,
    pub max_retries: u32,
}

impl MultiAgentRunConfig {
    pub fn new(
        max_depth: u32,
        max_iterations: u32,
        task_timeout: Duration,
        max_retries: u32,
    ) -> Self {
        Self {
            max_depth,
            max_iterations,
            task_timeout,
            max_retries,
        }
    }
}

pub async fn run_multi_agent_jobs(
    task: impl Into<String>,
    config: MultiAgentRunConfig,
) -> Result<MultiAgentRunReport, MultiAgentError> {
    run_multi_agent_jobs_with(
        task,
        config,
        Arc::new(HeuristicDelegationPlanner),
        Arc::new(PlaceholderTaskExecutor),
    )
    .await
}

/// Same job-queue runtime as [`run_multi_agent_jobs`], with an injectable
/// planner/executor pair. Every AgentRun in the resulting tree — master and
/// delegated alike — flows through the same [`JobWorker`], so this is the
/// seam future model/tool-backed planners and executors reuse instead of a
/// second runtime.
pub async fn run_multi_agent_jobs_with(
    task: impl Into<String>,
    config: MultiAgentRunConfig,
    planner: Arc<dyn DelegationPlanner>,
    executor: Arc<dyn TaskExecutor>,
) -> Result<MultiAgentRunReport, MultiAgentError> {
    let task = task.into();
    let store: Arc<dyn AgentJobStore> = Arc::new(InMemoryAgentJobStore::new(config.max_iterations));
    let root_id = store.next_job_id();
    let root = AgentJob::new_root(
        root_id.clone(),
        task.clone(),
        config.max_depth,
        config.max_retries,
    );
    store.insert_job(root)?;

    let worker = Arc::new(JobWorker::with_planner_and_executor(
        Arc::clone(&store),
        config.max_iterations,
        config.max_retries,
        planner,
        executor,
    ));
    tokio::time::timeout(config.task_timeout, worker.run_until_root_complete(&root_id))
        .await
        .map_err(|_| MultiAgentError::TaskTimeout {
            timeout_secs: config.task_timeout.as_secs(),
        })??;

    build_report(store, root_id, task)
}

fn build_report(
    store: Arc<dyn AgentJobStore>,
    root_id: String,
    master_task: String,
) -> Result<MultiAgentRunReport, MultiAgentError> {
    let jobs = store.list_jobs_for_root(&root_id)?;
    let events = store.list_events_for_root(&root_id)?;
    let root_job = store
        .get_job(&root_id)?
        .ok_or_else(|| MultiAgentError::JobNotFound {
            job_id: root_id.clone(),
        })?;
    let root_result = job_to_task_result(store.as_ref(), &root_job)?;
    let final_summary = aggregate_final_summary(&root_result);
    Ok(MultiAgentRunReport {
        master_task,
        root_id,
        root_result,
        final_summary,
        jobs,
        events,
    })
}

fn job_to_task_result(
    store: &dyn AgentJobStore,
    job: &AgentJob,
) -> Result<TaskResult, MultiAgentError> {
    let children = store.list_children(&job.id)?;
    let child_results = children
        .iter()
        .map(|child| job_to_task_result(store, child))
        .collect::<Result<Vec<_>, _>>()?;

    let agent_id = job.agent_kind.as_str().to_string();
    let task_id = TaskId(job.id.clone());
    Ok(match job.status {
        AgentStatus::Complete if child_results.is_empty() => TaskResult::completed(
            task_id,
            agent_id,
            job.result.clone().unwrap_or_else(|| job.task.clone()),
        ),
        AgentStatus::Complete => TaskResult::delegated(
            task_id,
            agent_id,
            job.result.clone().unwrap_or_default(),
            child_results,
        ),
        AgentStatus::Failed => TaskResult::failed(
            task_id,
            agent_id,
            job.error.clone().unwrap_or_else(|| "job failed".to_string()),
        ),
        AgentStatus::Cancelled => TaskResult::failed(task_id, agent_id, "job cancelled"),
        AgentStatus::WaitingForChildren if !child_results.is_empty() => TaskResult::delegated(
            task_id,
            agent_id,
            "waiting for child jobs".to_string(),
            child_results,
        ),
        _ => TaskResult::completed(task_id, agent_id, job.task.clone()),
    })
}

pub fn aggregate_job_summary(jobs: &[AgentJob]) -> String {
    let completed = jobs
        .iter()
        .filter(|job| job.status == AgentStatus::Complete)
        .count();
    let failed = jobs
        .iter()
        .filter(|job| job.status == AgentStatus::Failed)
        .count();
    let cancelled = jobs
        .iter()
        .filter(|job| job.status == AgentStatus::Cancelled)
        .count();
    format!(
        "{completed} job(s) complete, {failed} failed, {cancelled} cancelled ({} total)",
        jobs.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn job_runtime_delegates_recursively() {
        let report = run_multi_agent_jobs(
            "analyze dataset; summarize findings; draft report",
            MultiAgentRunConfig::new(3, 32, Duration::from_secs(30), 0),
        )
        .await
        .expect("job runtime succeeds");
        assert!(report.jobs.len() > 1);
        assert!(!report.events.is_empty());
        assert!(matches!(
            report.root_result.status,
            crate::multi_agent::types::TaskStatus::Completed
                | crate::multi_agent::types::TaskStatus::Delegated
        ));
    }

    #[tokio::test]
    async fn job_runtime_respects_max_depth() {
        let report = run_multi_agent_jobs(
            "one; two; three; four",
            MultiAgentRunConfig::new(1, 32, Duration::from_secs(30), 0),
        )
        .await
        .expect("depth capped run succeeds");
        assert!(
            report
                .jobs
                .iter()
                .all(|job| job.depth <= 1),
            "every job should respect max_depth"
        );
    }

    #[tokio::test]
    async fn job_runtime_aggregates_child_results() {
        let report = run_multi_agent_jobs(
            "collect inputs and merge outputs",
            MultiAgentRunConfig::new(2, 32, Duration::from_secs(30), 0),
        )
        .await
        .expect("aggregation run succeeds");
        assert!(report.final_summary.contains("successful node(s)"));
        assert!(aggregate_job_summary(&report.jobs).contains("complete"));
    }
}
