//! Engine error types.

use std::fmt;

use crate::types::capability::EffectType;
use crate::types::thread::{ThreadId, ThreadState};

/// Typed classification of an orchestrator-VM failure.
///
/// Replaces the previous `format!()`-built string reason so the user-facing
/// message and the low-level detail stay separate. The low-level detail
/// (Monty interpreter trace, Python traceback, underlying HTTP body) is
/// preserved in [`OrchestratorFailure::debug_detail`] and surfaced via
/// gateway debug mode rather than leaked into the user's reply.
#[derive(Debug, Clone, thiserror::Error)]
pub enum OrchestratorFailureKind {
    #[error(
        "{prefix}: time budget exhausted after {limit_secs}s (set IRONCLAW_ORCHESTRATOR_MAX_DURATION_SECS to raise the limit or simplify the task)"
    )]
    TimeLimit { prefix: String, limit_secs: u64 },

    #[error("{prefix}: resource budget exhausted (memory or allocations)")]
    ResourceLimit { prefix: String },

    #[error("{prefix}: internal orchestrator failure (see debug logs for details)")]
    Traceback { prefix: String },

    #[error("{prefix}: Monty VM panicked during {phase}")]
    VmPanic { prefix: String, phase: &'static str },

    /// Unclassified Monty failure. The raw message is deliberately NOT
    /// part of the Display rendering — it can carry internal file paths,
    /// Python tracebacks, or upstream HTTP bodies that haven't matched
    /// any of the explicit classifiers above. Channel-edge surfaces that
    /// bypass `bridge::user_facing_errors::user_facing_thread_failure`
    /// (mission notifications, third-party integrations) would otherwise
    /// leak it. The full raw text lives in `OrchestratorFailure::debug_detail`
    /// for operator triage — see the PR #2753 review discussion.
    #[error("{prefix}: internal orchestrator failure")]
    Other { prefix: String },
}

/// Structured orchestrator failure: user-safe classification plus preserved
/// low-level detail.
///
/// The low-level detail (`debug_detail`) is NEVER surfaced in the Display
/// impl — that path is user-visible. It's only readable via
/// [`OrchestratorFailure::debug_detail`] so gateway debug mode can opt in
/// to surface it.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{kind}")]
pub struct OrchestratorFailure {
    pub kind: OrchestratorFailureKind,
    pub debug_detail: String,
}

impl OrchestratorFailure {
    pub fn new(kind: OrchestratorFailureKind, debug_detail: impl Into<String>) -> Self {
        Self {
            kind,
            debug_detail: debug_detail.into(),
        }
    }

    pub fn user_message(&self) -> String {
        self.kind.to_string()
    }

    pub fn debug_detail(&self) -> &str {
        &self.debug_detail
    }
}

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

    #[error("orchestrator failure: {0}")]
    Orchestrator(OrchestratorFailure),

    #[error("invalid cadence: {reason}")]
    InvalidCadence { reason: String },

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

    #[error("invalid input: {reason}")]
    InvalidInput { reason: String },

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
        paused_lease: Option<Box<crate::types::capability::CapabilityLease>>,
    },
}

impl EngineError {
    /// Low-level detail for gateway debug mode. Never surfaced to users.
    ///
    /// Today only [`EngineError::Orchestrator`] carries one, but the
    /// accessor exists on the top-level error so callers don't have to
    /// pattern-match against the full variant set just to pull a debug
    /// string out. New variants can opt in by returning `Some(..)`.
    pub fn debug_detail(&self) -> Option<&str> {
        match self {
            EngineError::Orchestrator(failure) => Some(failure.debug_detail()),
            _ => None,
        }
    }
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
