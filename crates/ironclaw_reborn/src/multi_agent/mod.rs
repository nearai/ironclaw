mod agent;
mod demo;
mod error;
mod executor;
mod job_model;
mod master;
mod orchestrator;
mod planner;
mod progress;
mod report;
mod runtime;
mod store;
mod sub_agent;
mod types;
mod worker;

pub use agent::Agent;
pub use demo::{
    AgentDemoConfig, AgentDemoResult, HarnessEvent, RunAction, RunRecord, RunStatus,
    format_demo_progress, run_multi_agent_demo,
};
pub use error::MultiAgentError;
pub use executor::{PlaceholderTaskExecutor, TaskExecutor};
#[cfg(feature = "root-llm-provider")]
pub use executor::LlmTaskExecutor;
pub use job_model::{AgentEvent, AgentJob, AgentKind, AgentStatus, ClaimLease};
pub use master::MasterAgent;
pub use planner::{DelegationPlanner, HeuristicDelegationPlanner};
pub use progress::{format_job_progress_report, format_run_output};
pub use report::{aggregate_final_summary, format_run_report};
pub use runtime::{
    MultiAgentRunConfig, aggregate_job_summary, run_multi_agent_jobs, run_multi_agent_jobs_with,
};
pub use store::{AgentJobStore, InMemoryAgentJobStore};
pub use sub_agent::SubAgent;
pub use types::{
    AgentContext, DelegationPlan, ExecutionPlan, MultiAgentRunReport, Task, TaskId, TaskResult,
    TaskStatus,
};
pub use worker::JobWorker;

/// Run a top-level task through the persisted job-queue multi-agent runtime.
pub async fn run_multi_agent_task(
    task_description: impl Into<String>,
    max_depth: u32,
    max_iterations: u32,
    task_timeout: std::time::Duration,
) -> Result<MultiAgentRunReport, MultiAgentError> {
    run_multi_agent_jobs(
        task_description,
        MultiAgentRunConfig::new(max_depth, max_iterations, task_timeout, 0),
    )
    .await
}

/// Run a top-level task with full runtime controls.
pub async fn run_multi_agent_task_with_config(
    task_description: impl Into<String>,
    config: MultiAgentRunConfig,
) -> Result<MultiAgentRunReport, MultiAgentError> {
    run_multi_agent_jobs(task_description, config).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multi_agent::orchestrator::Orchestrator;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct CountingPlanner {
        splits: AtomicUsize,
    }

    impl DelegationPlanner for CountingPlanner {
        fn plan(&self, task: &Task, ctx: &AgentContext) -> DelegationPlan {
            if !ctx.can_delegate(task) {
                return DelegationPlan::execute_directly();
            }
            if task.description.contains("split-me") {
                self.splits.fetch_add(1, Ordering::SeqCst);
                return DelegationPlan::delegate(vec![
                    "part one".to_string(),
                    "part two".to_string(),
                ]);
            }
            DelegationPlan::execute_directly()
        }
    }

    struct FailOnDescriptionPlanner;

    impl DelegationPlanner for FailOnDescriptionPlanner {
        fn plan(&self, task: &Task, ctx: &AgentContext) -> DelegationPlan {
            if !ctx.can_delegate(task) {
                return DelegationPlan::execute_directly();
            }
            if task.description.contains(" and ") {
                return DelegationPlan::delegate(
                    task
                        .description
                        .split(" and ")
                        .map(str::trim)
                        .filter(|part| !part.is_empty())
                        .map(str::to_string)
                        .collect(),
                );
            }
            DelegationPlan::execute_directly()
        }
    }

    struct BoomExecutor;

    #[async_trait::async_trait]
    impl TaskExecutor for BoomExecutor {
        async fn execute(&self, task: &Task) -> Result<String, MultiAgentError> {
            if task.description.contains("boom") {
                return Err(MultiAgentError::SubAgentFailed {
                    agent_id: "worker".to_string(),
                    reason: "simulated failure".to_string(),
                });
            }
            Ok(format!("ok: {}", task.description))
        }
    }

    #[tokio::test]
    async fn master_creates_subagents() {
        let planner = Arc::new(CountingPlanner {
            splits: AtomicUsize::new(0),
        });
        let master = MasterAgent::new("master", planner.clone(), Arc::new(PlaceholderTaskExecutor));
        let mut ctx = AgentContext::new(2, 20, Duration::from_secs(30));
        let task = Task::root("split-me now", TaskId::new("root"));
        let result = master.run(task, &mut ctx).await.expect("run succeeds");
        assert_eq!(result.status, TaskStatus::Delegated);
        assert_eq!(result.child_results.len(), 2);
        assert_eq!(planner.splits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn task_delegates_recursively_via_jobs() {
        let report = run_multi_agent_task(
            "analyze dataset; summarize findings; draft report",
            3,
            32,
            Duration::from_secs(30),
        )
        .await
        .expect("recursive run succeeds");
        assert!(report.jobs.len() > 1);
        assert!(!report.events.is_empty());
    }

    #[tokio::test]
    async fn recursion_stops_at_max_depth_via_jobs() {
        let report = run_multi_agent_task(
            "one; two; three; four",
            1,
            32,
            Duration::from_secs(30),
        )
        .await
        .expect("depth-capped run succeeds");
        assert!(report.jobs.iter().all(|job| job.depth <= 1));
    }

    #[tokio::test]
    async fn failed_subagent_result_is_captured_in_orchestrator() {
        let planner = Arc::new(FailOnDescriptionPlanner);
        let executor = Arc::new(BoomExecutor);
        let orchestrator = Orchestrator::new(planner, executor);
        let mut ctx = AgentContext::new(2, 20, Duration::from_secs(30));
        let root = Task::root("alpha and boom", TaskId::new("root"));
        let result = orchestrator
            .run_agent_task("master", &root, &mut ctx, &mut orchestrator::TaskIdGenerator::new())
            .await
            .expect("orchestrator run");
        assert_eq!(result.status, TaskStatus::Delegated);
        assert!(
            result
                .child_results
                .iter()
                .any(|child| child.status == TaskStatus::Failed)
        );
    }

    #[tokio::test]
    async fn final_output_aggregates_all_subagent_results() {
        let report = run_multi_agent_task(
            "collect inputs and merge outputs",
            2,
            32,
            Duration::from_secs(30),
        )
        .await
        .expect("aggregate run succeeds");
        assert!(report.final_summary.contains("successful node(s)"));
        let rendered = format_run_report(
            &report.master_task,
            &report.root_result,
            &report.final_summary,
        );
        assert!(rendered.contains("Agent work:"));
        assert!(rendered.contains("Final aggregated result:"));
    }

    #[tokio::test]
    async fn subagent_trait_entrypoint_delegates() {
        let planner = Arc::new(HeuristicDelegationPlanner);
        let executor = Arc::new(PlaceholderTaskExecutor);
        let subagent = SubAgent::new("sub-a", planner, executor);
        let mut ctx = AgentContext::new(2, 20, Duration::from_secs(30));
        let task = Task::root("first; second", TaskId::new("sub-root"));
        let result = subagent.handle(&task, &mut ctx).await.expect("subagent run");
        assert_eq!(result.status, TaskStatus::Delegated);
    }

    #[tokio::test]
    async fn leaf_execution_failure_surfaces_as_failed_result() {
        struct FailLeafExecutor;

        #[async_trait::async_trait]
        impl TaskExecutor for FailLeafExecutor {
            async fn execute(&self, _task: &Task) -> Result<String, MultiAgentError> {
                Err(MultiAgentError::SubAgentFailed {
                    agent_id: "worker".to_string(),
                    reason: "leaf failure".to_string(),
                })
            }
        }

        let planner = Arc::new(HeuristicDelegationPlanner);
        let executor = Arc::new(FailLeafExecutor);
        let orchestrator = Orchestrator::new(planner, executor);
        let mut ctx = AgentContext::new(1, 5, Duration::from_secs(30));
        let task = Task::root("do work", TaskId::new("leaf"));
        let result = orchestrator
            .run_agent_task("worker", &task, &mut ctx, &mut orchestrator::TaskIdGenerator::new())
            .await
            .expect("error is converted to failed result");
        assert_eq!(result.status, TaskStatus::Failed);
    }

    /// A `TaskExecutor` that sleeps before returning, so tests can measure
    /// whether independent AgentRuns actually overlap in wall-clock time
    /// instead of just being logically "concurrent" on paper.
    struct SleepingExecutor {
        delay: Duration,
    }

    #[async_trait::async_trait]
    impl TaskExecutor for SleepingExecutor {
        async fn execute(&self, task: &Task) -> Result<String, MultiAgentError> {
            tokio::time::sleep(self.delay).await;
            Ok(format!("done: {}", task.description))
        }
    }

    /// End-to-end proof of the recursive multi-agent runtime: the master
    /// keeps work for itself, delegates the rest as real AgentRuns through
    /// the shared job-queue runtime, delegated AgentRuns recursively delegate
    /// further, independent branches run in parallel (not one at a time),
    /// and every run's result is aggregated into one final response.
    #[tokio::test]
    async fn master_delegates_recursively_and_runs_independent_work_in_parallel() {
        let leaf_delay = Duration::from_millis(300);
        let planner = Arc::new(HeuristicDelegationPlanner);
        let executor = Arc::new(SleepingExecutor { delay: leaf_delay });

        let started = std::time::Instant::now();
        let report = run_multi_agent_jobs_with(
            "Draft the intro; Research topic A and summarize; Write the closing",
            MultiAgentRunConfig::new(3, 64, Duration::from_secs(10), 0),
            planner,
            executor,
        )
        .await
        .expect("parallel recursive job run succeeds");
        let elapsed = started.elapsed();

        // 1. The master keeps at least one task and executes it locally.
        let root_job = report
            .jobs
            .iter()
            .find(|job| job.id == report.root_id)
            .expect("root job present");
        assert!(
            root_job.result.as_deref().unwrap_or_default().contains("local:"),
            "master should retain and execute at least one task itself, got: {:?}",
            root_job.result
        );

        // 2. The master delegates at least one task to a new AgentRun.
        let root_children: Vec<_> = report
            .jobs
            .iter()
            .filter(|job| job.parent_id.as_deref() == Some(report.root_id.as_str()))
            .collect();
        assert!(
            !root_children.is_empty(),
            "master should delegate at least one task to a child AgentRun"
        );

        // 3. Delegated AgentRuns can recursively delegate further.
        assert!(
            report.jobs.iter().any(|job| job.depth >= 2),
            "a delegated AgentRun should be able to recursively delegate its own work"
        );

        // 4. Independent branches ran concurrently. This tree requires 4
        // sequential "waves" worth of executor calls if run one at a time
        // (root-local, child-local, sibling-leaf, grandchild-leaf); running
        // the independent siblings in parallel finishes in ~3 waves instead.
        let sequential_floor = leaf_delay * 4;
        assert!(
            elapsed < sequential_floor,
            "expected independent AgentRuns to overlap (elapsed {elapsed:?} should be well \
             under the fully-sequential floor of {sequential_floor:?})"
        );

        // 5. All AgentRun results are aggregated into one final response.
        assert!(report.final_summary.contains("successful node(s)"));
        let rendered = format_run_output(&report, true);
        assert!(rendered.contains("Execution Plan"));
        assert!(rendered.contains("Final Result"));
    }
}
