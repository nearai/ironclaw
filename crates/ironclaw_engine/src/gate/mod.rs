//! Unified execution gate abstraction.
//!
//! All pre-execution checks (approval, authentication, rate limiting, hooks,
//! relay channel enforcement) are expressed as composable [`ExecutionGate`]
//! implementations evaluated through a [`GatePipeline`].
//!
//! Design invariants:
//! - [`GateDecision`] has no `None` variant — fail-closed by construction.
//! - [`ResumeKind`] is a closed enum — forces all pause paths through
//!   the same storage, resolution, and SSE machinery.
//! - [`GateContext`] borrows everything — zero cloning in the hot path.

pub mod lease;
pub mod pipeline;
pub mod tool_tier;

use std::collections::HashSet;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::types::capability::ActionDef;
use crate::types::thread::ThreadId;

// ── Gate decision ───────────────────────────────────────────

/// The outcome of evaluating an execution gate.
#[derive(Debug, Clone)]
pub enum GateDecision {
    /// Execution may proceed.
    Allow,
    /// Execution must pause until the user provides input.
    Pause {
        reason: String,
        resume_kind: ResumeKind,
    },
    /// Execution is denied outright.
    Deny { reason: String },
}

/// What kind of external input will resolve a paused gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResumeKind {
    /// User must approve or deny the tool invocation.
    Approval {
        /// Whether the "always approve this tool" option should be offered.
        allow_always: bool,
    },
    /// User must provide a credential (token, API key, OAuth flow).
    Authentication {
        /// Name of the credential that is missing.
        credential_name: String,
        /// User-facing setup instructions.
        instructions: String,
        /// Optional OAuth URL for browser-based flows.
        auth_url: Option<String>,
    },
    /// An external system must respond (webhook confirmation, etc.).
    External { callback_id: String },
}

impl ResumeKind {
    /// Short human-readable label for this kind.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Approval { .. } => "approval",
            Self::Authentication { .. } => "authentication",
            Self::External { .. } => "external confirmation",
        }
    }
}

// ── Gate resolution ─────────────────────────────────────────

/// How a paused gate is resolved by the user or external system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GateResolution {
    /// User approved the tool call.
    Approved { always: bool },
    /// User denied the tool call.
    Denied { reason: Option<String> },
    /// User provided a credential value.
    CredentialProvided { token: String },
    /// User or system cancelled the pending gate entirely.
    Cancelled,
    /// External callback received.
    ExternalCallback { payload: serde_json::Value },
}

// ── Execution mode ──────────────────────────────────────────

/// The execution context in which a tool call is being evaluated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionMode {
    /// Interactive session — a user can approve / authenticate.
    Interactive,
    /// Autonomous background job — no interactive user.
    /// The lease set determines what tools are available.
    Autonomous,
    /// Container-sandboxed execution.
    Container,
}

// ── Gate context ────────────────────────────────────────────

/// Immutable snapshot of everything a gate needs to make a decision.
///
/// All fields are borrowed to avoid cloning in the hot path.
#[derive(Debug)]
pub struct GateContext<'a> {
    pub user_id: &'a str,
    pub thread_id: ThreadId,
    pub source_channel: &'a str,
    pub action_name: &'a str,
    pub call_id: &'a str,
    pub parameters: &'a serde_json::Value,
    pub action_def: &'a ActionDef,
    pub execution_mode: ExecutionMode,
    /// Tools the session has auto-approved ("always" button).
    pub auto_approved: &'a HashSet<String>,
}

// ── Gate trait ───────────────────────────────────────────────

/// A single pre-execution check.
///
/// Implementations must be deterministic for a given context snapshot:
/// they must not hold mutable state that changes across evaluations
/// within a single pipeline run.
#[async_trait]
pub trait ExecutionGate: Send + Sync {
    /// Unique name for logging and persistence.
    fn name(&self) -> &str;

    /// Evaluation priority. Lower runs first. First `Pause` or `Deny` wins.
    fn priority(&self) -> u32;

    /// Evaluate whether the tool invocation should proceed.
    async fn evaluate(&self, ctx: &GateContext<'_>) -> GateDecision;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_kind_labels() {
        assert_eq!(
            ResumeKind::Approval { allow_always: true }.kind_name(),
            "approval"
        );
        assert_eq!(
            ResumeKind::Authentication {
                credential_name: "x".into(),
                instructions: "y".into(),
                auth_url: None,
            }
            .kind_name(),
            "authentication"
        );
        assert_eq!(
            ResumeKind::External {
                callback_id: "z".into()
            }
            .kind_name(),
            "external confirmation"
        );
    }
}
