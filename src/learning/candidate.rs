use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A candidate interaction that may be worth synthesizing into a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisCandidate {
    /// Conversation ID this candidate was extracted from.
    pub conversation_id: Uuid,
    /// User ID who triggered the interaction.
    pub user_id: String,
    /// Brief description of the task that was solved.
    pub task_summary: String,
    /// Tools that were used (ordered by invocation).
    pub tools_used: Vec<String>,
    /// Number of tool calls in the interaction.
    pub tool_call_count: usize,
    /// Number of turns in the conversation.
    pub turn_count: usize,
    /// Quality score from the evaluation system (0-100).
    pub quality_score: u32,
    /// Why this candidate was selected.
    pub detection_reason: DetectionReason,
    /// When the interaction completed.
    pub completed_at: DateTime<Utc>,
}

/// Why a particular interaction was flagged as synthesis-worthy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DetectionReason {
    /// Multi-step tool chain that completed successfully.
    ComplexToolChain { step_count: usize },
    /// Novel tool combination not seen before.
    NovelToolCombination { tools: Vec<String> },
    /// User explicitly requested skill creation.
    UserRequested,
    /// High quality score on a non-trivial task.
    HighQualityCompletion { score: u32 },
}
