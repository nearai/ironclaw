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
use ironclaw_common::CredentialName;
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
        credential_name: CredentialName,
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
    /// Interactive session with auto-approve enabled.
    ///
    /// `UnlessAutoApproved` tools pass without prompting (shell, file_write,
    /// http, etc.). `Always`-gated tools (destructive operations) still pause
    /// for explicit approval. All other safeguards remain active: leases,
    /// rate limits, hooks, relay channel checks, authentication gates.
    ///
    /// Activated via `AGENT_AUTO_APPROVE_TOOLS=true` or settings.
    InteractiveAutoApprove,
    /// Autonomous background job — no interactive user.
    /// The lease set determines what tools are available.
    Autonomous,
    /// Container-sandboxed execution.
    Container,
}

// ── Gate context ────────────────────────────────────────────

/// Immutable snapshot of everything a gate needs to make a decision.
///
/// String and Value fields are borrowed to avoid cloning in the hot path.
/// `ThreadId` and `ExecutionMode` are `Copy` and stored inline.
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

// ── Inline gate await ────────────────────────────────────────

/// What the executor needs to surface to the user when an `Approval`
/// gate fires inside a live execution.
///
/// The host implementation of [`GateController`] is responsible for:
/// 1. Persisting whatever metadata the UI / channel layer needs to
///    render the approval prompt.
/// 2. Dispatching the prompt to the originating channel.
/// 3. Awaiting the user's response and returning it as a
///    [`GateResolution`] without re-entering the engine.
///
/// Carries `thread_id` and `user_id` so a single shared controller
/// can route the request to the right host-side per-execution context
/// (conversation id, channel metadata, etc.) without the engine having
/// to thread bridge-internal types through.
#[derive(Debug, Clone)]
pub struct GatePauseRequest {
    pub thread_id: crate::types::thread::ThreadId,
    pub user_id: String,
    pub gate_name: String,
    pub action_name: String,
    pub call_id: String,
    pub parameters: serde_json::Value,
    pub resume_kind: ResumeKind,
    /// Originating conversation, if any. Lets the host route an inline
    /// gate to the right UI surface when the same user has multiple
    /// concurrent conversations (e.g. two browser tabs). `None` for
    /// background mission threads.
    pub conversation_id: Option<crate::types::conversation::ConversationId>,
}

/// Host-supplied callback that pauses a live engine execution until
/// the user resolves an `Approval` gate.
///
/// This is the mechanism that lets both Tier 0 (structured) and Tier 1
/// (CodeAct/Monty) executions wait for user input *without* unwinding
/// the call stack. The executor stays inside its own loop awaiting
/// `pause()`, so all in-memory state (Monty VM frame, partially-executed
/// parallel batch, leases) is preserved across the wait. On resolution
/// the executor proceeds inline — no thread re-entry, no replay, no
/// double-execution of side effects from earlier tool calls in the
/// same step.
///
/// Restricted to `ResumeKind::Approval`. Authentication and External
/// resume kinds keep their existing re-entry-based flow because their
/// resolution installs new state (credentials, callback payloads) that
/// only takes effect on the next run-through, not via direct hand-back
/// to the suspended call.
#[async_trait]
pub trait GateController: Send + Sync {
    /// Pause execution until the user resolves the gate.
    ///
    /// Implementations MUST eventually return some [`GateResolution`]
    /// (returning `Cancelled` is acceptable on shutdown / timeout).
    /// They MUST NOT block forever — callers rely on this future
    /// completing so the surrounding execution can either continue or
    /// terminate cleanly.
    async fn pause(&self, request: GatePauseRequest) -> GateResolution;

    /// Wake any [`pause`] futures currently parked on `thread_id` with
    /// [`GateResolution::Cancelled`] and discard their pending state.
    ///
    /// `ThreadManager::stop_thread()` calls this before sending
    /// `ThreadSignal::Stop`. Without it, an engine task parked inside
    /// `pause()` is not polling the thread signal channel and will
    /// continue waiting for the user (or up to the host's gate-expiry
    /// window) before observing the stop request — leaving the running
    /// task and pending prompt orphaned.
    ///
    /// Default implementation is a no-op; overrides should be
    /// idempotent and tolerant of concurrent calls. Implementations
    /// that don't track per-thread waiters can ignore this call.
    ///
    /// [`pause`]: GateController::pause
    async fn cancel_thread(&self, _thread_id: crate::types::thread::ThreadId) {}
}

/// Default [`GateController`] that cancels every pause request.
///
/// `ThreadExecutionContext::gate_controller` is non-optional: every
/// execution context must carry *some* controller. This impl is the
/// drop-in for code paths where pausing is meaningless or already
/// resolved upstream:
///
/// - **Post-resolution replay** (`execute_pending_gate_action`) — the
///   gate has already been resolved before this call site runs.
/// - **Mission protected writes** — background paths with no
///   originating user channel to surface a prompt on.
/// - **Tests** that don't exercise the gate flow.
///
/// Returning [`GateResolution::Cancelled`] surfaces as a typed denial
/// in both Tier 0 and Tier 1 — never as the original user-visible
/// "execution paused by gate" bug message.
pub struct CancellingGateController;

impl CancellingGateController {
    /// Construct as a `dyn`-trait Arc, the form most call sites need.
    pub fn arc() -> std::sync::Arc<dyn GateController> {
        std::sync::Arc::new(Self)
    }
}

#[async_trait]
impl GateController for CancellingGateController {
    async fn pause(&self, _request: GatePauseRequest) -> GateResolution {
        GateResolution::Cancelled
    }
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
                credential_name: CredentialName::new("x").unwrap(),
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
