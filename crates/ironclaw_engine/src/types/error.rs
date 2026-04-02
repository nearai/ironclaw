//! Engine error types.

use std::fmt;

use crate::types::capability::EffectType;
use crate::types::thread::{ThreadId, ThreadState};

/// Top-level engine error.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("thread error: {0}")]
    Thread(#[from] ThreadError),

    #[error("step error: {0}")]
    Step(#[from] StepError),

    #[error("capability error: {0}")]
    Capability(#[from] CapabilityError),

    #[error("store error: {reason}")]
    Store { reason: String },

    #[error("LLM error: {reason}")]
    Llm { reason: String },

    #[error("effect execution error: {reason}")]
    Effect { reason: String },

    #[error("invalid state transition: {from} -> {to}")]
    InvalidTransition { from: ThreadState, to: ThreadState },

    #[error("thread not found: {0}")]
    ThreadNotFound(ThreadId),

    #[error("project not found: {0}")]
    ProjectNotFound(ProjectId),

    #[error("lease not found: {lease_id}")]
    LeaseNotFound { lease_id: String },

    #[error("lease expired for capability: {capability_name}")]
    LeaseExpired { capability_name: String },

    #[error("lease denied: {reason}")]
    LeaseDenied { reason: String },

    #[error("max iterations reached: {limit}")]
    MaxIterations { limit: usize },

    #[error("token limit exceeded: {used} of {limit}")]
    TokenLimitExceeded { used: u64, limit: u64 },

    #[error("consecutive error threshold exceeded: {count} errors (limit: {threshold})")]
    ConsecutiveErrors { count: u32, threshold: u32 },

    #[error("thread timeout: {elapsed:?} of {limit:?}")]
    Timeout {
        elapsed: std::time::Duration,
        limit: std::time::Duration,
    },

    #[error("skill error: {reason}")]
    Skill { reason: String },

    #[error("access denied: user '{user_id}' cannot access {entity}")]
    AccessDenied { user_id: String, entity: String },

    #[error("gate paused: {gate_name} requires {action_name}")]
    GatePaused {
        gate_name: String,
        action_name: String,
        call_id: String,
        parameters: Box<serde_json::Value>,
        resume_kind: Box<crate::gate::ResumeKind>,
        resume_output: Option<Box<serde_json::Value>>,
    },
}

use crate::types::project::ProjectId;

/// Thread-specific errors.
#[derive(Debug, thiserror::Error)]
pub enum ThreadError {
    #[error("thread already running: {0}")]
    AlreadyRunning(ThreadId),

    #[error("thread is in terminal state: {0}")]
    Terminal(ThreadState),

    #[error("cannot spawn child: parent thread {0} is not running")]
    ParentNotRunning(ThreadId),
}

/// Step-specific errors.
#[derive(Debug, thiserror::Error)]
pub enum StepError {
    #[error("step timed out after {0:?}")]
    Timeout(std::time::Duration),

    #[error("action not permitted by capability lease: {action}")]
    ActionDenied { action: String },
}

/// Capability-specific errors.
#[derive(Debug, thiserror::Error)]
pub enum CapabilityError {
    #[error("capability not found: {0}")]
    NotFound(String),

    #[error("effect type {effect:?} not permitted by policy")]
    EffectDenied { effect: EffectType },
}

// Display impls for types used in error messages that don't already impl Display.

impl fmt::Display for ThreadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for ThreadState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
