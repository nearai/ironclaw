use async_trait::async_trait;

use super::error::MultiAgentError;
use super::types::{AgentContext, Task, TaskResult};

#[async_trait]
pub trait Agent: Send + Sync {
    fn id(&self) -> &str;

    async fn handle(
        &self,
        task: &Task,
        ctx: &mut AgentContext,
    ) -> Result<TaskResult, MultiAgentError>;
}
