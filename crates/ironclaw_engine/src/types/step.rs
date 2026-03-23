//! Step — the unit of execution within a thread.
//!
//! Each step corresponds to one LLM call plus its subsequent action
//! executions. This replaces the implicit "iteration" counter in the
//! existing `run_agentic_loop`.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::thread::ThreadId;

/// Strongly-typed step identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StepId(pub Uuid);

impl StepId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for StepId {
    fn default() -> Self {
        Self::new()
    }
}

/// Status of a step within its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepStatus {
    Pending,
    LlmCalling,
    Executing,
    Completed,
    Failed,
}

/// Which execution tier handles the step's code/actions.
///
/// Monty is the sole CodeAct/RLM executor. WASM and Docker are used for
/// third-party tool isolation and thread sandboxing (Phase 8), not for
/// running LLM-generated Python.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionTier {
    /// Structured tool calls (JSON action calls from LLM).
    Structured,
    /// Embedded Python via Monty (CodeAct/RLM pattern).
    Scripting,
}

/// A single execution step within a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub id: StepId,
    pub thread_id: ThreadId,
    /// 1-indexed sequence within the thread.
    pub sequence: usize,
    pub status: StepStatus,
    pub tier: ExecutionTier,
    pub llm_response: Option<LlmResponse>,
    pub action_results: Vec<ActionResult>,
    pub tokens_used: TokenUsage,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl Step {
    pub fn new(thread_id: ThreadId, sequence: usize) -> Self {
        Self {
            id: StepId::new(),
            thread_id,
            sequence,
            status: StepStatus::Pending,
            tier: ExecutionTier::Structured,
            llm_response: None,
            action_results: Vec::new(),
            tokens_used: TokenUsage::default(),
            started_at: Utc::now(),
            completed_at: None,
        }
    }
}

// ── LLM response types ─────────────────────────────────────

/// Response from the LLM: text, action calls, or executable code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LlmResponse {
    /// Final text response.
    Text(String),
    /// One or more action calls (with optional reasoning text).
    ActionCalls {
        calls: Vec<ActionCall>,
        content: Option<String>,
    },
    /// Executable Python code (CodeAct). Tool calls happen as function
    /// calls within the code; the runtime suspends at each one and
    /// delegates to the EffectExecutor.
    Code {
        code: String,
        content: Option<String>,
    },
}

/// A request from the LLM to execute a capability action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionCall {
    /// Unique call identifier (echoed in the result).
    pub id: String,
    /// Action name (e.g. "web_fetch", "create_issue").
    pub action_name: String,
    /// Action parameters as JSON.
    pub parameters: serde_json::Value,
}

/// Result of executing a capability action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResult {
    /// The call ID this result corresponds to.
    pub call_id: String,
    /// The action that was executed.
    pub action_name: String,
    /// Output value.
    pub output: serde_json::Value,
    /// Whether this result represents an error.
    pub is_error: bool,
    /// How long the action took.
    #[serde(with = "duration_millis")]
    pub duration: Duration,
}

/// Token usage for a single LLM call.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

impl TokenUsage {
    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

/// Serde helper for Duration as milliseconds.
mod duration_millis {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_millis() as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let millis = u64::deserialize(d)?;
        Ok(Duration::from_millis(millis))
    }
}
