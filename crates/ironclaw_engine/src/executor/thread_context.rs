use std::str::FromStr;
use std::sync::Arc;

use crate::gate::GateController;
use crate::traits::effect::ThreadExecutionContext;
use crate::types::conversation::ConversationId;
use crate::types::step::StepId;
use crate::types::thread::Thread;
use ironclaw_common::ValidTimezone;
use uuid::Uuid;

/// Build an execution context from the current thread state.
///
/// `gate_controller` is required: callers thread through the controller
/// they were constructed with so the executor can pause inline on
/// `Approval` gates. Code paths that don't pause supply
/// [`crate::gate::CancellingGateController::arc()`].
pub(crate) fn thread_execution_context(
    thread: &Thread,
    step_id: StepId,
    current_call_id: Option<String>,
    gate_controller: Arc<dyn GateController>,
) -> ThreadExecutionContext {
    ThreadExecutionContext {
        thread_id: thread.id,
        thread_type: thread.thread_type,
        project_id: thread.project_id,
        user_id: thread.user_id.clone(),
        step_id,
        current_call_id,
        source_channel: thread
            .metadata
            .get("source_channel")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        user_timezone: thread
            .metadata
            .get("user_timezone")
            .and_then(|v| v.as_str())
            .and_then(ValidTimezone::parse),
        thread_goal: Some(thread.goal.clone()),
        available_actions_snapshot: None,
        available_action_inventory_snapshot: None,
        gate_controller,
        call_approval_granted: false,
        conversation_id: thread
            .metadata
            .get("conversation_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::from_str(s).ok())
            .map(ConversationId),
    }
}
