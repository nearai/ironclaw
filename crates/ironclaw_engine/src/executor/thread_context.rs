use crate::traits::effect::ThreadExecutionContext;
use crate::types::step::StepId;
use crate::types::thread::Thread;
use ironclaw_common::ValidTimezone;

/// Build an execution context from the current thread state.
pub(crate) fn thread_execution_context(
    thread: &Thread,
    step_id: StepId,
    current_call_id: Option<String>,
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
    }
}
