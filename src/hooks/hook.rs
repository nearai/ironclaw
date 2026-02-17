//! Core hook types and traits.

use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;

/// Points in the agent lifecycle where hooks can be attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum HookPoint {
    /// Before processing an inbound user message.
    BeforeInbound,
    /// Before executing a tool call.
    BeforeToolCall,
    /// Before sending an outbound response.
    BeforeOutbound,
    /// When a new session starts.
    OnSessionStart,
    /// When a session ends (pruned or expired).
    OnSessionEnd,
    /// Transform the final response before completing a turn.
    TransformResponse,
    /// After parsing the user submission, before routing.
    AfterParse,
    /// Before entering the agentic tool-execution loop.
    BeforeAgenticLoop,
    /// Before each LLM call inside the agentic loop.
    BeforeLlmCall,
    /// After a tool call completes (result available).
    AfterToolCall,
    /// Before presenting a tool-approval request to the user.
    BeforeApproval,
}

/// Contextual data carried with each hook invocation.
#[derive(Debug, Clone, Serialize)]
pub enum HookEvent {
    /// An inbound user message about to be processed.
    Inbound {
        user_id: String,
        channel: String,
        content: String,
        thread_id: Option<String>,
    },
    /// A tool call about to be executed.
    ToolCall {
        tool_name: String,
        parameters: serde_json::Value,
        user_id: String,
        /// "chat" for interactive, or a job ID string for autonomous jobs.
        context: String,
    },
    /// An outbound response about to be sent.
    Outbound {
        user_id: String,
        channel: String,
        content: String,
        thread_id: Option<String>,
    },
    /// A new session was created.
    SessionStart { user_id: String, session_id: String },
    /// A session was ended (pruned).
    SessionEnd { user_id: String, session_id: String },
    /// The final response is being transformed before completing a turn.
    ResponseTransform {
        user_id: String,
        thread_id: String,
        response: String,
    },
    /// After parsing the user submission, before routing.
    Parse {
        user_id: String,
        channel: String,
        raw_input: String,
        parsed_intent: String,
    },
    /// Before entering the agentic tool-execution loop.
    AgenticLoopStart {
        user_id: String,
        thread_id: String,
        message_count: usize,
    },
    /// Before each LLM call inside the agentic loop.
    LlmCall {
        user_id: String,
        thread_id: String,
        message_count: usize,
        tool_count: usize,
    },
    /// After a tool call completes with its result.
    ToolResult {
        tool_name: String,
        user_id: String,
        result: String,
        success: bool,
        elapsed_ms: u64,
    },
    /// Before presenting a tool-approval request to the user.
    ApprovalRequest {
        tool_name: String,
        user_id: String,
        parameters: serde_json::Value,
        description: String,
    },
}

impl HookEvent {
    /// Returns the [`HookPoint`] this event corresponds to.
    pub fn hook_point(&self) -> HookPoint {
        match self {
            HookEvent::Inbound { .. } => HookPoint::BeforeInbound,
            HookEvent::ToolCall { .. } => HookPoint::BeforeToolCall,
            HookEvent::Outbound { .. } => HookPoint::BeforeOutbound,
            HookEvent::SessionStart { .. } => HookPoint::OnSessionStart,
            HookEvent::SessionEnd { .. } => HookPoint::OnSessionEnd,
            HookEvent::ResponseTransform { .. } => HookPoint::TransformResponse,
            HookEvent::Parse { .. } => HookPoint::AfterParse,
            HookEvent::AgenticLoopStart { .. } => HookPoint::BeforeAgenticLoop,
            HookEvent::LlmCall { .. } => HookPoint::BeforeLlmCall,
            HookEvent::ToolResult { .. } => HookPoint::AfterToolCall,
            HookEvent::ApprovalRequest { .. } => HookPoint::BeforeApproval,
        }
    }

    /// Apply a modification string to the event's primary content field.
    pub fn apply_modification(&mut self, modified: &str) {
        match self {
            HookEvent::Inbound { content, .. } | HookEvent::Outbound { content, .. } => {
                *content = modified.to_string();
            }
            HookEvent::ToolCall { parameters, .. }
            | HookEvent::ApprovalRequest { parameters, .. } => match serde_json::from_str(modified)
            {
                Ok(parsed) => *parameters = parsed,
                Err(e) => {
                    tracing::warn!(
                        "Hook returned non-JSON modification for ToolCall/ApprovalRequest, ignoring: {}",
                        e
                    );
                }
            },
            HookEvent::ResponseTransform { response, .. } => {
                *response = modified.to_string();
            }
            HookEvent::Parse { parsed_intent, .. } => {
                *parsed_intent = modified.to_string();
            }
            HookEvent::ToolResult { result, .. } => {
                *result = modified.to_string();
            }
            HookEvent::SessionStart { .. }
            | HookEvent::SessionEnd { .. }
            | HookEvent::AgenticLoopStart { .. }
            | HookEvent::LlmCall { .. } => {
                // These events don't have modifiable content
            }
        }
    }
}

/// The result of executing a hook.
#[derive(Debug, Clone)]
pub enum HookOutcome {
    /// Continue processing, optionally with modified content.
    Continue {
        /// If `Some`, replace the event's primary content with this value.
        modified: Option<String>,
    },
    /// Reject the event entirely.
    Reject {
        /// Human-readable reason for the rejection.
        reason: String,
    },
}

impl HookOutcome {
    /// Shorthand for `Continue { modified: None }`.
    pub fn ok() -> Self {
        HookOutcome::Continue { modified: None }
    }

    /// Shorthand for `Continue { modified: Some(value) }`.
    pub fn modify(value: String) -> Self {
        HookOutcome::Continue {
            modified: Some(value),
        }
    }

    /// Shorthand for `Reject { reason }`.
    pub fn reject(reason: impl Into<String>) -> Self {
        HookOutcome::Reject {
            reason: reason.into(),
        }
    }
}

/// How to handle hook execution failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookFailureMode {
    /// On error/timeout, continue processing as if the hook returned `ok()`.
    FailOpen,
    /// On error/timeout, reject the event.
    FailClosed,
}

/// Hook execution errors.
#[derive(Debug, thiserror::Error)]
pub enum HookError {
    #[error("Hook execution failed: {reason}")]
    ExecutionFailed { reason: String },

    #[error("Hook timed out after {timeout:?}")]
    Timeout { timeout: Duration },

    #[error("Hook rejected: {reason}")]
    Rejected { reason: String },
}

/// Context passed to hooks alongside the event.
pub struct HookContext {
    /// Arbitrary metadata hooks can use.
    pub metadata: serde_json::Value,
}

impl Default for HookContext {
    fn default() -> Self {
        Self {
            metadata: serde_json::Value::Null,
        }
    }
}

/// Trait for implementing lifecycle hooks.
///
/// Hooks intercept and can modify agent operations at well-defined points.
#[async_trait]
pub trait Hook: Send + Sync {
    /// A unique name for this hook.
    fn name(&self) -> &str;

    /// The lifecycle points this hook should be called at.
    fn hook_points(&self) -> &[HookPoint];

    /// How to handle failures in this hook.
    ///
    /// Default: `FailOpen` (continue on error).
    fn failure_mode(&self) -> HookFailureMode {
        HookFailureMode::FailOpen
    }

    /// Maximum time this hook is allowed to run.
    ///
    /// Default: 5 seconds.
    fn timeout(&self) -> Duration {
        Duration::from_secs(5)
    }

    /// Execute the hook.
    async fn execute(&self, event: &HookEvent, ctx: &HookContext)
    -> Result<HookOutcome, HookError>;
}
