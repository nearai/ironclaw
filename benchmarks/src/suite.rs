use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::error::BenchError;

/// A single task in a benchmark suite.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BenchTask {
    pub id: String,
    pub prompt: String,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub resources: Vec<TaskResource>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub expected_turns: Option<usize>,
    #[serde(default)]
    pub timeout: Option<Duration>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// A resource attached to a benchmark task (file, URL, etc.).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskResource {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub resource_type: ResourceType,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    #[default]
    File,
    Url,
    Directory,
}

/// What the agent produced for scoring.
#[derive(Debug, Clone)]
pub struct TaskSubmission {
    pub response: String,
    pub conversation: Vec<ConversationTurn>,
    pub tool_calls: Vec<String>,
    pub error: Option<String>,
}

/// A single turn in a multi-turn conversation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConversationTurn {
    pub role: TurnRole,
    pub content: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnRole {
    User,
    Assistant,
    System,
}

/// Score for a single task.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BenchScore {
    /// 0.0 to 1.0 (1.0 = perfect).
    pub value: f64,
    /// "pass" / "fail" / "partial".
    pub label: String,
    #[serde(default)]
    pub details: Option<String>,
}

impl BenchScore {
    pub fn pass() -> Self {
        Self {
            value: 1.0,
            label: "pass".to_string(),
            details: None,
        }
    }

    pub fn fail(details: impl Into<String>) -> Self {
        Self {
            value: 0.0,
            label: "fail".to_string(),
            details: Some(details.into()),
        }
    }

    pub fn partial(value: f64, details: impl Into<String>) -> Self {
        Self {
            value: value.clamp(0.0, 1.0),
            label: "partial".to_string(),
            details: Some(details.into()),
        }
    }
}

/// Trait for benchmark suite adapters.
///
/// Each suite (GAIA, Tau-bench, custom, etc.) implements this trait
/// to provide task loading, scoring, and optional lifecycle hooks.
#[async_trait]
pub trait BenchSuite: Send + Sync {
    /// Human-readable name (e.g., "GAIA Validation").
    fn name(&self) -> &str;

    /// Machine ID (e.g., "gaia").
    fn id(&self) -> &str;

    /// Load all tasks from the suite's data source.
    async fn load_tasks(&self) -> Result<Vec<BenchTask>, BenchError>;

    /// Score the agent's submission against the expected answer.
    async fn score(
        &self,
        task: &BenchTask,
        submission: &TaskSubmission,
    ) -> Result<BenchScore, BenchError>;

    /// Optional: set up environment before running a task (clone repo, init DB, etc.).
    async fn setup_task(&self, _task: &BenchTask) -> Result<(), BenchError> {
        Ok(())
    }

    /// Optional: tear down environment after a task completes.
    async fn teardown_task(&self, _task: &BenchTask) -> Result<(), BenchError> {
        Ok(())
    }

    /// Optional: additional tools to register for this suite's tasks.
    fn additional_tools(&self) -> Vec<Arc<dyn ironclaw::tools::Tool>> {
        vec![]
    }

    /// Multi-turn: generate next simulated user message based on conversation so far.
    /// Return `None` to end the conversation.
    async fn next_user_message(
        &self,
        _task: &BenchTask,
        _conversation: &[ConversationTurn],
    ) -> Result<Option<String>, BenchError> {
        Ok(None)
    }
}
