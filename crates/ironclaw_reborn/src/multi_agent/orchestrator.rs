use std::sync::Arc;

use super::error::MultiAgentError;
use super::executor::TaskExecutor;
use super::master::{aggregate_child_results, execute_leaf};
use super::planner::DelegationPlanner;
use super::types::{AgentContext, Task, TaskId, TaskResult};

pub(super) struct TaskIdGenerator(u64);

impl TaskIdGenerator {
    pub(super) fn new() -> Self {
        Self(0)
    }

    pub(super) fn next(&mut self, prefix: &str) -> TaskId {
        self.0 = self.0.saturating_add(1);
        TaskId::new(format!("{prefix}-{}", self.0))
    }
}

pub(super) struct Orchestrator<P, E> {
    planner: Arc<P>,
    executor: Arc<E>,
}

impl<P, E> Orchestrator<P, E>
where
    P: DelegationPlanner + 'static,
    E: TaskExecutor + 'static,
{
    pub(super) fn new(planner: Arc<P>, executor: Arc<E>) -> Self {
        Self { planner, executor }
    }

    pub(super) async fn run_agent_task(
        &self,
        agent_id: &str,
        task: &Task,
        ctx: &mut AgentContext,
        id_gen: &mut TaskIdGenerator,
    ) -> Result<TaskResult, MultiAgentError> {
        Box::pin(self.run_agent_task_rec(agent_id, task, ctx, id_gen)).await
    }

    async fn run_agent_task_rec(
        &self,
        agent_id: &str,
        task: &Task,
        ctx: &mut AgentContext,
        id_gen: &mut TaskIdGenerator,
    ) -> Result<TaskResult, MultiAgentError> {
        ctx.register_task(&task.id)?;
        ctx.consume_iteration()?;

        let plan = self.planner.plan(task, ctx);
        let result = if plan.execute_directly || !ctx.can_delegate(task) || plan.subtasks.is_empty()
        {
            match execute_leaf(agent_id, task, self.executor.as_ref()).await {
                Ok(result) => Ok(result),
                Err(error) => Ok(TaskResult::failed(
                    task.id.clone(),
                    agent_id,
                    error.to_string(),
                )),
            }
        } else {
            let mut child_results = Vec::with_capacity(plan.subtasks.len());
            for (index, description) in plan.subtasks.into_iter().enumerate() {
                let child_id = id_gen.next(&format!("{agent_id}-sub{index}"));
                let child_task = Task::child(task, description, child_id);
                let worker_id = format!("{agent_id}-worker-{index}");
                match Box::pin(self.run_agent_task_rec(&worker_id, &child_task, ctx, id_gen))
                    .await
                {
                    Ok(result) => child_results.push(result),
                    Err(error) => child_results.push(TaskResult::failed(
                        child_task.id,
                        worker_id,
                        error.to_string(),
                    )),
                }
            }
            Ok(aggregate_child_results(task, agent_id, child_results))
        };

        ctx.unregister_task(&task.id);
        result
    }
}
