use std::sync::Arc;

use async_trait::async_trait;

use super::agent::Agent;
use super::error::MultiAgentError;
use super::executor::TaskExecutor;
use super::orchestrator::{Orchestrator, TaskIdGenerator};
use super::planner::DelegationPlanner;
use super::types::{AgentContext, Task, TaskResult};

pub struct SubAgent<P, E> {
    id: String,
    orchestrator: Orchestrator<P, E>,
}

impl<P, E> SubAgent<P, E>
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
}

#[async_trait]
impl<P, E> Agent for SubAgent<P, E>
where
    P: DelegationPlanner + 'static,
    E: TaskExecutor + 'static,
{
    fn id(&self) -> &str {
        &self.id
    }

    async fn handle(
        &self,
        task: &Task,
        ctx: &mut AgentContext,
    ) -> Result<TaskResult, MultiAgentError> {
        self.orchestrator
            .run_agent_task(self.id(), task, ctx, &mut TaskIdGenerator::new())
            .await
    }
}
