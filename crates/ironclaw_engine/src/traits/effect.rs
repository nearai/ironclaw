//! Effect executor trait.
//!
//! The engine delegates actual action execution to the host through this
//! trait. The main crate implements it by wrapping `ToolRegistry` and
//! `SafetyLayer` — the engine itself has no knowledge of specific tools.

use std::sync::Arc;

use crate::gate::GateController;
use crate::types::capability::{ActionDef, ActionInventory, CapabilityLease, CapabilitySummary};
use crate::types::conversation::ConversationId;
use crate::types::error::EngineError;
use crate::types::project::ProjectId;
use crate::types::step::{ActionResult, StepId};
use crate::types::thread::{ThreadId, ThreadType};
use ironclaw_common::ValidTimezone;

/// Contextual information about the thread requesting an effect.
///
/// Passed to the executor so it can make context-dependent decisions
/// (e.g. different tool behavior in background vs foreground threads).
#[derive(Clone)]
pub struct ThreadExecutionContext {
    pub thread_id: ThreadId,
    pub thread_type: ThreadType,
    pub project_id: ProjectId,
    pub user_id: String,
    pub step_id: StepId,
    pub current_call_id: Option<String>,
    /// The channel this thread's conversation originated from (e.g. "gateway", "repl").
    /// Used by mission_create to default `notify_channels` to the current channel.
    pub source_channel: Option<String>,
    /// Validated IANA timezone of the user (e.g. "America/New_York").
    /// Used by mission_create to default cron timezone, and exposed to CodeAct scripts.
    pub user_timezone: Option<ValidTimezone>,
    /// The original goal for the executing thread.
    /// Host adapters use this to distinguish immediate one-shot foreground
    /// requests from explicit mission/routine setup.
    pub thread_goal: Option<String>,
    /// Snapshot of callable actions visible to the current step.
    ///
    /// Populated by the orchestrator when an execution path needs on-demand
    /// discovery parity (for example `tool_info`).
    pub available_actions_snapshot: Option<Arc<[ActionDef]>>,
    /// Snapshot of the full action inventory visible to the current step.
    pub available_action_inventory_snapshot: Option<Arc<ActionInventory>>,
    /// Host-supplied callback that lets the executor pause inline on
    /// `Approval` gates instead of unwinding back to the orchestrator.
    ///
    /// Required. Code paths that don't pause (post-resolution replay,
    /// background mission writes, tests) supply
    /// [`crate::gate::CancellingGateController::arc()`], which surfaces
    /// any unexpected gate as a typed denial rather than the historical
    /// "execution paused by gate" RuntimeError leak.
    pub gate_controller: Arc<dyn GateController>,
    /// Set to `true` when the host has already collected user approval
    /// for *this specific call* (matched by `current_call_id`) and the
    /// executor is retrying it inline. The host's `EffectExecutor` impl
    /// uses this to skip the `ApprovalRequirement::Always` /
    /// `AskEachTime` gate that would otherwise re-fire on retry —
    /// mirrors the legacy `execute_resolved_pending_action` path that
    /// passes `approval_already_granted=true`.
    ///
    /// One-shot: scoped to a single retry call. Reset to `false` on
    /// any context not owned by an inline retry.
    pub call_approval_granted: bool,
    /// The conversation that originated this thread, if any. Carried
    /// into [`crate::gate::GatePauseRequest`] so the host can match a
    /// gate to the originating UI surface even when the same user has
    /// multiple concurrent conversations (e.g. two browser tabs).
    /// `None` for background mission threads with no user-facing
    /// conversation.
    pub conversation_id: Option<ConversationId>,
}

// Manual Debug impl: `dyn GateController` is not Debug, but the rest of
// the struct is. The controller is opaque host-supplied state — it
// renders as a constant marker rather than its internals.
impl std::fmt::Debug for ThreadExecutionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreadExecutionContext")
            .field("thread_id", &self.thread_id)
            .field("thread_type", &self.thread_type)
            .field("project_id", &self.project_id)
            .field("user_id", &self.user_id)
            .field("step_id", &self.step_id)
            .field("current_call_id", &self.current_call_id)
            .field("source_channel", &self.source_channel)
            .field("user_timezone", &self.user_timezone)
            .field("thread_goal", &self.thread_goal)
            .field(
                "available_actions_snapshot",
                &self.available_actions_snapshot.as_ref().map(|a| a.len()),
            )
            .field(
                "available_action_inventory_snapshot",
                &self.available_action_inventory_snapshot.is_some(),
            )
            .field("gate_controller", &"<dyn GateController>")
            .field("call_approval_granted", &self.call_approval_granted)
            .field("conversation_id", &self.conversation_id)
            .finish()
    }
}

/// Abstraction over capability action execution.
///
/// The main crate implements this by wrapping its `ToolRegistry`, `SafetyLayer`,
/// and tool execution pipeline. The engine calls `execute_action` and gets back
/// a result — all safety, sanitization, and actual tool invocation happens in
/// the host.
#[async_trait::async_trait]
pub trait EffectExecutor: Send + Sync {
    /// Execute a capability action.
    ///
    /// The executor is responsible for:
    /// 1. Looking up the actual tool implementation
    /// 2. Validating parameters
    /// 3. Applying safety checks (sanitization, leak detection)
    /// 4. Executing the tool
    /// 5. Returning the result
    async fn execute_action(
        &self,
        action_name: &str,
        parameters: serde_json::Value,
        lease: &CapabilityLease,
        context: &ThreadExecutionContext,
    ) -> Result<ActionResult, EngineError>;

    /// List available actions given the current set of active leases.
    ///
    /// Used to build the action definitions sent to the LLM.
    async fn available_actions(
        &self,
        leases: &[CapabilityLease],
        context: &ThreadExecutionContext,
    ) -> Result<Vec<ActionDef>, EngineError>;

    /// List the full action inventory for the current set of active leases.
    ///
    /// The default implementation mirrors `available_actions()`.
    async fn available_action_inventory(
        &self,
        leases: &[CapabilityLease],
        context: &ThreadExecutionContext,
    ) -> Result<ActionInventory, EngineError> {
        Ok(ActionInventory {
            inline: self.available_actions(leases, context).await?,
            discoverable: Vec::new(),
        })
    }

    /// List capability background summaries given the current runtime state.
    async fn available_capabilities(
        &self,
        leases: &[CapabilityLease],
        context: &ThreadExecutionContext,
    ) -> Result<Vec<CapabilitySummary>, EngineError>;
}
