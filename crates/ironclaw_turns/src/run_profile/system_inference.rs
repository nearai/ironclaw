use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use super::host::LoopSafeSummary;

/// Opaque id for an internal system inference task.
///
/// This is distinct from turn/run ids because the task is host-owned work
/// performed on behalf of the loop, not a user-visible turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct SystemInferenceTaskId(Uuid);

impl SystemInferenceTaskId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for SystemInferenceTaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl TryFrom<String> for SystemInferenceTaskId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Uuid::parse_str(&value)
            .map(Self)
            .map_err(|error| format!("system inference task id must be a UUID: {error}"))
    }
}

impl<'de> Deserialize<'de> for SystemInferenceTaskId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(serde::de::Error::custom)
    }
}

/// Stable identifier for an embedded system prompt source.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct SystemPromptId(String);

impl SystemPromptId {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty() {
            return Err("system prompt id must not be empty".to_string());
        }
        if value.len() > 128 {
            return Err("system prompt id is too long".to_string());
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err("system prompt id must be snake_case".to_string());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SystemPromptId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<String> for SystemPromptId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for SystemPromptId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// System-owned inference job class.
///
/// These tasks run through a host-owned internal model path, are not assistant
/// replies, and must not dispatch capabilities.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemTaskKind {
    Compaction,
    GoalRefresh,
}

/// Origin metadata for the system prompt used by a system inference task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "source")]
pub enum SystemPromptSource {
    /// Static prompt embedded in the host binary or support crate.
    Static { prompt_id: SystemPromptId },
}

/// Auditable identity for a host-owned inference call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemInferenceIdentity {
    /// Class of host task being executed.
    pub task_kind: SystemTaskKind,
    /// Stable source id for the prompt text.
    pub prompt_source: SystemPromptSource,
    /// Sanitized system prompt text passed to the host-owned model path.
    pub system_prompt: String,
}

/// Request to run bounded, host-owned inference outside the assistant loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemInferenceRequest {
    /// Unique task id used for progress and internal refs.
    pub task_id: SystemInferenceTaskId,
    /// Prompt identity and content.
    pub identity: SystemInferenceIdentity,
    /// Sanitized user-side task input.
    pub input_text: String,
    /// Preflight token ceiling for `input_text`.
    pub max_input_tokens: u64,
    /// Wall-clock deadline for the underlying model call.
    pub deadline_ms: u64,
}

/// Successful system inference result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemInferenceResponse {
    /// Echoes the request task id.
    pub task_id: SystemInferenceTaskId,
    /// Sanitized model output text.
    pub output_text: String,
    /// Elapsed model-call time in milliseconds.
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Error, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SystemInferenceError {
    #[error("system inference input too large")]
    InputTooLarge,
    #[error("system inference timed out")]
    Timeout,
    #[error("system inference cancelled")]
    Cancelled,
    #[error("system inference failed: {safe_summary}")]
    Failed { safe_summary: LoopSafeSummary },
}

/// Host boundary for internal inference tasks such as compaction.
///
/// Implementations must dispatch through a host-owned internal model path that
/// does not expose capabilities or ordinary assistant prompt-bundle authority,
/// and must not expose raw prompt/input content on public progress surfaces.
#[async_trait]
pub trait SystemInferencePort: Send + Sync {
    async fn call_system_inference(
        &self,
        request: SystemInferenceRequest,
    ) -> Result<SystemInferenceResponse, SystemInferenceError>;
}
