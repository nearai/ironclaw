//! Task types for the scheduler.
//!
//! Tasks are the unit of work that can be scheduled for execution.
//! They can represent full LLM-driven jobs, parallel tool batches,
//! or background computations.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::Error;

/// Result of a task execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOutput {
    /// The result data.
    pub result: serde_json::Value,
    /// Time taken to execute.
    pub duration: Duration,
}

impl TaskOutput {
    /// Create a new task output.
    pub fn new(result: serde_json::Value, duration: Duration) -> Self {
        Self { result, duration }
    }

    /// Create a text result.
    #[cfg(test)]
    pub fn text(text: impl Into<String>, duration: Duration) -> Self {
        Self {
            result: serde_json::Value::String(text.into()),
            duration,
        }
    }

    /// Create an empty success result.
    #[cfg(test)]
    pub fn empty(duration: Duration) -> Self {
        Self {
            result: serde_json::Value::Null,
            duration,
        }
    }
}

/// Context passed to task handlers.
#[derive(Debug, Clone)]
pub struct TaskContext {
    /// Task ID.
    pub task_id: Uuid,
    /// Parent task ID (if this is a sub-task).
    pub parent_id: Option<Uuid>,
    /// Arbitrary metadata for the task.
    pub metadata: serde_json::Value,
}

impl TaskContext {
    /// Create a new task context.
    pub fn new(task_id: Uuid) -> Self {
        Self {
            task_id,
            parent_id: None,
            metadata: serde_json::Value::Null,
        }
    }

    /// Set the parent task ID.
    pub fn with_parent(mut self, parent_id: Uuid) -> Self {
        self.parent_id = Some(parent_id);
        self
    }

    /// Set metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}

/// Handler for custom background tasks.
#[async_trait]
pub trait TaskHandler: Send + Sync {
    /// Run the task and return the result.
    async fn run(&self, ctx: TaskContext) -> Result<TaskOutput, Error>;

    /// Get a description of this handler for logging.
    fn description(&self) -> &str {
        "background task"
    }
}

/// A task that can be scheduled for execution.
#[derive(Clone)]
pub enum Task {
    /// Full LLM-driven job (current Worker behavior).
    Job {
        id: Uuid,
        title: String,
        description: String,
    },

    /// Single tool execution as a sub-task.
    ToolExec {
        /// ID of the parent job this tool execution belongs to.
        parent_id: Uuid,
        /// Name of the tool to execute.
        tool_name: String,
        /// Parameters to pass to the tool.
        params: serde_json::Value,
    },

    /// Background computation (no LLM, uses a custom handler).
    /// Note: The handler is wrapped in Arc for cloning.
    Background {
        id: Uuid,
        handler: std::sync::Arc<dyn TaskHandler>,
    },
}

impl Task {
    /// Create a new Job task.
    pub fn job(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self::Job {
            id: Uuid::new_v4(),
            title: title.into(),
            description: description.into(),
        }
    }

    /// Create a new Job task with a specific ID.
    #[cfg(test)]
    pub fn job_with_id(id: Uuid, title: impl Into<String>, description: impl Into<String>) -> Self {
        Self::Job {
            id,
            title: title.into(),
            description: description.into(),
        }
    }

    /// Create a new ToolExec task.
    pub fn tool_exec(
        parent_id: Uuid,
        tool_name: impl Into<String>,
        params: serde_json::Value,
    ) -> Self {
        Self::ToolExec {
            parent_id,
            tool_name: tool_name.into(),
            params,
        }
    }

    /// Create a new Background task.
    #[cfg(test)]
    pub fn background(handler: std::sync::Arc<dyn TaskHandler>) -> Self {
        Self::Background {
            id: Uuid::new_v4(),
            handler,
        }
    }

    /// Create a new Background task with a specific ID.
    #[cfg(test)]
    pub fn background_with_id(id: Uuid, handler: std::sync::Arc<dyn TaskHandler>) -> Self {
        Self::Background { id, handler }
    }

    /// Get the task ID, if applicable.
    pub fn id(&self) -> Option<Uuid> {
        match self {
            Self::Job { id, .. } => Some(*id),
            Self::ToolExec { .. } => None, // Tool execs don't have their own ID
            Self::Background { id, .. } => Some(*id),
        }
    }

    /// Get the parent ID for sub-tasks.
    #[cfg(test)]
    pub fn parent_id(&self) -> Option<Uuid> {
        match self {
            Self::Job { .. } => None,
            Self::ToolExec { parent_id, .. } => Some(*parent_id),
            Self::Background { .. } => None,
        }
    }

    /// Get a short description for logging.
    pub fn description(&self) -> String {
        match self {
            Self::Job { title, .. } => format!("job: {}", title),
            Self::ToolExec { tool_name, .. } => format!("tool: {}", tool_name),
            Self::Background { handler, .. } => format!("background: {}", handler.description()),
        }
    }
}

impl fmt::Debug for Task {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Job {
                id,
                title,
                description,
            } => f
                .debug_struct("Task::Job")
                .field("id", id)
                .field("title", title)
                .field("description", description)
                .finish(),
            Self::ToolExec {
                parent_id,
                tool_name,
                params,
            } => f
                .debug_struct("Task::ToolExec")
                .field("parent_id", parent_id)
                .field("tool_name", tool_name)
                .field("params", params)
                .finish(),
            Self::Background { id, handler } => f
                .debug_struct("Task::Background")
                .field("id", id)
                .field("handler", &handler.description())
                .finish(),
        }
    }
}

/// Status of a scheduled task.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    /// Task is queued waiting for execution.
    Queued,
    /// Task is currently running.
    Running,
    /// Task completed successfully.
    Completed,
    /// Task failed with an error.
    Failed,
    /// Task was cancelled.
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_output() {
        let output = TaskOutput::text("hello", Duration::from_secs(1));
        assert_eq!(output.result, serde_json::json!("hello"));
        assert_eq!(output.duration, Duration::from_secs(1));
    }

    #[test]
    fn test_task_context() {
        let parent = Uuid::new_v4();
        let ctx = TaskContext::new(Uuid::new_v4()).with_parent(parent);
        assert_eq!(ctx.parent_id, Some(parent));
    }

    #[test]
    fn test_task_job() {
        let task = Task::job("Test Job", "Test description");
        assert!(task.id().is_some());
        assert!(task.parent_id().is_none());
        assert!(task.description().contains("job:"));
    }

    #[test]
    fn test_task_tool_exec() {
        let parent_id = Uuid::new_v4();
        let task = Task::tool_exec(parent_id, "echo", serde_json::json!({"message": "hi"}));
        assert!(task.id().is_none());
        assert_eq!(task.parent_id(), Some(parent_id));
        assert!(task.description().contains("tool:"));
    }
}
