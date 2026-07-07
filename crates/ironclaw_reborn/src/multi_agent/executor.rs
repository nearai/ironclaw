use async_trait::async_trait;

use super::error::MultiAgentError;
use super::types::Task;

/// Execution surface for leaf tasks. Implement this to swap in a real LLM,
/// a tool runner, or any other backend. The same executor instance is shared
/// across all AgentRuns (master and delegated) so it is `Send + Sync`.
#[async_trait]
pub trait TaskExecutor: Send + Sync {
    async fn execute(&self, task: &Task) -> Result<String, MultiAgentError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PlaceholderTaskExecutor;

#[async_trait]
impl TaskExecutor for PlaceholderTaskExecutor {
    async fn execute(&self, task: &Task) -> Result<String, MultiAgentError> {
        Ok(format!(
            "Executed leaf task at depth {}: {}",
            task.depth, task.description
        ))
    }
}

/// A real executor that sends each leaf task to an LLM provider as a
/// single-turn completion and returns the model's text response.
///
/// Every AgentRun (master and delegated, at any recursion depth) routes its
/// local task through this executor unchanged, so the same model does the
/// actual work everywhere in the tree.
#[cfg(feature = "root-llm-provider")]
pub struct LlmTaskExecutor {
    provider: std::sync::Arc<dyn ironclaw_llm::LlmProvider>,
}

#[cfg(feature = "root-llm-provider")]
impl LlmTaskExecutor {
    pub fn new(provider: std::sync::Arc<dyn ironclaw_llm::LlmProvider>) -> Self {
        Self { provider }
    }
}

#[cfg(feature = "root-llm-provider")]
#[async_trait]
impl TaskExecutor for LlmTaskExecutor {
    async fn execute(&self, task: &Task) -> Result<String, MultiAgentError> {
        use ironclaw_llm::{ChatMessage, CompletionRequest};

        let system = format!(
            "You are a focused sub-agent working on a single task. \
             Complete the task concisely and return only the result. \
             Depth in delegation tree: {}.",
            task.depth
        );
        let request = CompletionRequest::new(vec![
            ChatMessage::system(system),
            ChatMessage::user(task.description.clone()),
        ]);

        let response = self
            .provider
            .complete(request)
            .await
            .map_err(|error| MultiAgentError::SubAgentFailed {
                agent_id: format!("llm@depth-{}", task.depth),
                reason: error.to_string(),
            })?;

        Ok(response.content)
    }
}
