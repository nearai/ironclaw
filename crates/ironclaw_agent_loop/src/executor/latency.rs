use std::time::Instant;

use ironclaw_turns::run_profile::LoopRunContext;

pub(super) fn started_at() -> Option<Instant> {
    tracing::enabled!(target: "ironclaw_latency", tracing::Level::TRACE).then(Instant::now)
}

fn elapsed_ms(started_at: Instant) -> u64 {
    started_at
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
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

    let elapsed_ms = elapsed_ms(started_at);
    tracing::trace!(
        target: "ironclaw_latency",
        component = "agent_loop_executor",
        operation,
        elapsed_ms,
        outcome = "ok",
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

    let elapsed_ms = elapsed_ms(started_at);
    tracing::trace!(
        target: "ironclaw_latency",
        component = "agent_loop_executor",
        operation,
        elapsed_ms,
        outcome = "error",
        error_kind = "executor_error",
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

macro_rules! stage {
    ($operation:expr, $context:expr, $iteration:expr, $future:expr $(,)?) => {{
        let iteration = $iteration;
        let stage_started_at = $crate::executor::latency::started_at();
        let result = $future.await;
        $crate::executor::latency::result(
            $operation,
            $context,
            iteration,
            stage_started_at,
            &result,
        );
        result
    }};
}

pub(super) use stage;
