use async_trait::async_trait;
use ironclaw_host_api::ThreadId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{host::LoopSafeSummary, system_inference::SystemInferenceTaskId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionInitiator {
    Auto,
    Overflow,
    SubagentScoped,
}

/// Requested compaction algorithm shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopCompactionMode {
    /// Build a fresh summary for the selected transcript range.
    Fresh,
    /// Update an existing summary with a later transcript range.
    Update,
}

/// Request for host-managed context compaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopCompactionRequest {
    /// Unique task id shared by progress events and host-owned inference refs.
    pub task_id: SystemInferenceTaskId,
    /// Thread whose canonical transcript should be compacted.
    pub thread_id: ThreadId,
    /// Previous compaction high-water mark, if any.
    pub last_compacted_through_seq: Option<u64>,
    /// Inclusive transcript sequence through which context may be replaced.
    pub drop_through_seq: u64,
    /// Estimated tail budget the strategy wanted preserved outside the range.
    pub preserve_tail_tokens: u64,
    /// Fresh versus incremental summary mode.
    pub mode: LoopCompactionMode,
    /// Deadline for any inference work needed by compaction.
    pub deadline_ms: u64,
}

/// Opaque reference to a durable summary artifact produced by compaction.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub struct LoopSummaryArtifactId(String);

impl LoopSummaryArtifactId {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty() {
            return Err("summary artifact id must not be empty".to_string());
        }
        if value.len() > 256 {
            return Err("summary artifact id is too long".to_string());
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'.' | b'_' | b'-'))
        {
            return Err("summary artifact id contains unsupported characters".to_string());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for LoopSummaryArtifactId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<String> for LoopSummaryArtifactId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl Serialize for LoopSummaryArtifactId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

/// Durable artifact produced by host-managed compaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopCompactionResponse {
    /// Summary artifact id persisted by the thread service.
    pub summary_artifact_id: LoopSummaryArtifactId,
    /// Output bytes divided by input bytes, scaled by 1,000,000.
    pub compression_ratio_ppm: u32,
}

/// Failure classes returned by host-managed compaction.
#[derive(Debug, Clone, Error, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LoopCompactionError {
    #[error("compaction cut point is invalid")]
    InvalidCutPoint,
    #[error("compaction mode is not supported")]
    UnsupportedMode,
    #[error("compaction input is too large")]
    InputTooLarge,
    #[error("compaction security check failed: {safe_summary}")]
    SecurityRejected { safe_summary: LoopSafeSummary },
    #[error("compaction inference failed: {safe_summary}")]
    InferenceFailed { safe_summary: LoopSafeSummary },
    #[error("compaction was cancelled")]
    Cancelled,
    #[error("compaction persistence failed: {safe_summary}")]
    PersistenceFailed { safe_summary: LoopSafeSummary },
}

/// Host boundary for compaction.
///
/// The agent loop decides when compaction is needed; the host owns transcript
/// reads, scope checks, security scanning, inference, and summary persistence.
#[async_trait]
pub trait LoopCompactionPort: Send + Sync {
    async fn compact_loop_context(
        &self,
        request: LoopCompactionRequest,
    ) -> Result<LoopCompactionResponse, LoopCompactionError>;
}
