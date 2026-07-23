use std::time::Instant;

use ironclaw_host_api::ThreadId;
use ironclaw_turns::TurnRunId;

pub(super) fn trace_runtime_latency_ok(
    operation: &'static str,
    thread_id: &ThreadId,
    run_id: Option<TurnRunId>,
    started_at: Option<Instant>,
) {
    let run_id = run_id.map(|id| id.to_string()).unwrap_or_default();
    ironclaw_observability::live_latency_trace_ok!(
        "ironclaw_runtime",
        operation,
        started_at,
        thread_id = %thread_id,
        run_id = run_id.as_str(),
        "IronClaw runtime operation completed",
    );
}

pub(super) fn trace_runtime_latency_error<E: ?Sized>(
    operation: &'static str,
    thread_id: &ThreadId,
    run_id: Option<TurnRunId>,
    started_at: Option<Instant>,
    _error: &E,
) {
    let run_id = run_id.map(|id| id.to_string()).unwrap_or_default();
    ironclaw_observability::live_latency_trace_error!(
        "ironclaw_runtime",
        operation,
        started_at,
        "runtime_error",
        thread_id = %thread_id,
        run_id = run_id.as_str(),
        "IronClaw runtime operation failed",
    );
}
