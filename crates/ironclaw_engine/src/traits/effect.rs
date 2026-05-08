//! Effect executor trait.
//!
//! The engine delegates actual action execution to the host through this
//! trait. The main crate implements it by wrapping `ToolRegistry` and
//! `SafetyLayer` — the engine itself has no knowledge of specific tools.

use std::sync::Arc;

use crate::types::capability::{ActionDef, ActionInventory, CapabilityLease, CapabilitySummary};
use crate::types::error::EngineError;
use crate::types::project::ProjectId;
use crate::types::step::{ActionResult, StepId};
use crate::types::thread::{ThreadId, ThreadType};
use ironclaw_common::ValidTimezone;

/// Contextual information about the thread requesting an effect.
///
/// Passed to the executor so it can make context-dependent decisions
/// (e.g. different tool behavior in background vs foreground threads).
#[derive(Debug, Clone)]
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
