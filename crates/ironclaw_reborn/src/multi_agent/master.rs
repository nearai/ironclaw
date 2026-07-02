use std::sync::Arc;

use tokio::time::timeout;

use super::error::MultiAgentError;
use super::executor::TaskExecutor;
use super::orchestrator::{Orchestrator, TaskIdGenerator};
use super::planner::DelegationPlanner;
use super::types::{AgentContext, Task, TaskResult};

pub struct MasterAgent<P, E> {
    id: String,
    orchestrator: Orchestrator<P, E>,
}

impl<P, E> MasterAgent<P, E>
where
    P: DelegationPlanner + 'static,
    E: TaskExecutor + 'static,
{
    pub fn new(id: impl Into<String>, planner: Arc<P>, executor: Arc<E>) -> Self {
        Self {
            id: id.into(),
            orchestrator: Orchestrator::new(planner, executor),
        }
    }

    pub async fn run(
        &self,
        task: Task,
        ctx: &mut AgentContext,
    ) -> Result<TaskResult, MultiAgentError> {
        timeout(ctx.task_timeout, async {
            self.orchestrator
                .run_agent_task(&self.id, &task, ctx, &mut TaskIdGenerator::new())
                .await
        })
        .await
        .map_err(|_| MultiAgentError::TaskTimeout {
            timeout_secs: ctx.task_timeout.as_secs(),
        })?
    }
}

pub(crate) async fn execute_leaf<E>(
    agent_id: &str,
    task: &Task,
    executor: &E,
) -> Result<TaskResult, MultiAgentError>
where
    E: TaskExecutor + ?Sized,
{
    let summary = executor.execute(task).await?;
    Ok(TaskResult::completed(
        task.id.clone(),
        agent_id,
        summary,
    ))
}

pub(crate) fn aggregate_child_results(
    task: &Task,
    agent_id: &str,
    child_results: Vec<TaskResult>,
) -> TaskResult {
    let failed = child_results
        .iter()
        .filter(|result| result.status == super::types::TaskStatus::Failed)
        .count();
    let completed = child_results.len().saturating_sub(failed);
    let summary = format!(
        "Delegated {} subtask(s): {completed} completed, {failed} failed",
        child_results.len()
    );
    TaskResult::delegated(task.id.clone(), agent_id, summary, child_results)
}
