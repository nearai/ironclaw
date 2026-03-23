//! Effect executor trait.
//!
//! The engine delegates actual action execution to the host through this
//! trait. The main crate implements it by wrapping `ToolRegistry` and
//! `SafetyLayer` — the engine itself has no knowledge of specific tools.

use crate::types::capability::{ActionDef, CapabilityLease};
use crate::types::error::EngineError;
use crate::types::project::ProjectId;
use crate::types::step::{ActionResult, StepId};
use crate::types::thread::{ThreadId, ThreadType};

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
    ) -> Result<Vec<ActionDef>, EngineError>;
}
