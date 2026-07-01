use std::time::Instant;

use ironclaw_observability::live_latency_started_at;
use ironclaw_turns::run_profile::LoopRunContext;

pub(super) fn started_at() -> Option<Instant> {
    live_latency_started_at()
}

pub(super) fn operation_ok(
    operation: &'static str,
    context: &LoopRunContext,
    iteration: u32,
    started_at: Option<Instant>,
) {
    let Some(started_at) = started_at else {
        return;
    };

    ironclaw_observability::live_latency_trace_ok!(
        "agent_loop_executor",
        operation,
        Some(started_at),
        tenant_id = %context.scope.tenant_id,
        agent_id = context.scope.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        project_id = context.scope.project_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        thread_id = %context.thread_id,
        owner_user_id = context.scope.explicit_owner_user_id().map(|id| id.as_str()).unwrap_or(""),
        run_id = %context.run_id,
        turn_id = %context.turn_id,
        iteration,
        "agent loop executor operation completed",
    );
}

pub(super) fn operation_error<E: ?Sized>(
    operation: &'static str,
    context: &LoopRunContext,
    iteration: u32,
    started_at: Option<Instant>,
    _error: &E,
) {
    let Some(started_at) = started_at else {
        return;
    };

    ironclaw_observability::live_latency_trace_error!(
        "agent_loop_executor",
        operation,
        Some(started_at),
        "executor_error",
        tenant_id = %context.scope.tenant_id,
        agent_id = context.scope.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        project_id = context.scope.project_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        thread_id = %context.thread_id,
        owner_user_id = context.scope.explicit_owner_user_id().map(|id| id.as_str()).unwrap_or(""),
        run_id = %context.run_id,
        turn_id = %context.turn_id,
        iteration,
        "agent loop executor operation failed",
    );
}

pub(super) fn result<T, E>(
    operation: &'static str,
    context: &LoopRunContext,
    iteration: u32,
    started_at: Option<Instant>,
    result: &Result<T, E>,
) {
    match result {
        Ok(_) => operation_ok(operation, context, iteration, started_at),
        Err(error) => operation_error(operation, context, iteration, started_at, error),
    }
}
