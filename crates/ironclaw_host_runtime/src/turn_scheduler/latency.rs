use std::{fmt, time::Instant};

use ironclaw_observability::elapsed_ms;
use ironclaw_turns::{TurnRunId, TurnRunWake, TurnScope, runner::ClaimedTurnRun};

pub(super) struct RunFields {
    thread_id: String,
    run_id: TurnRunId,
}

pub(super) fn run_fields_from_wake(
    started_at: Option<Instant>,
    wake: &TurnRunWake,
) -> Option<RunFields> {
    started_at?;
    Some(RunFields {
        thread_id: wake.scope.thread_id.as_str().to_string(),
        run_id: wake.run_id,
    })
}

pub(super) fn scope_thread_id(
    started_at: Option<Instant>,
    scope: Option<&TurnScope>,
) -> Option<String> {
    started_at?;
    scope.map(|scope| scope.thread_id.as_str().to_string())
}

pub(super) fn operation_ok(
    operation: &'static str,
    thread_id: &str,
    run_id: TurnRunId,
    started_at: Option<Instant>,
) {
    trace_ok(operation, Some(thread_id), Some(run_id), started_at);
}

pub(super) fn operation_error<E>(
    operation: &'static str,
    thread_id: &str,
    run_id: TurnRunId,
    started_at: Option<Instant>,
    error: &E,
) where
    E: fmt::Display + ?Sized,
{
    trace_error(operation, Some(thread_id), Some(run_id), started_at, error);
}

pub(super) fn notify_queued_run_result<E>(
    fields: Option<&RunFields>,
    started_at: Option<Instant>,
    result: &Result<(), E>,
) where
    E: fmt::Display,
{
    let Some(fields) = fields else {
        return;
    };

    match result {
        Ok(()) => trace_ok(
            "notify_queued_run",
            Some(fields.thread_id.as_str()),
            Some(fields.run_id),
            started_at,
        ),
        Err(error) => trace_error(
            "notify_queued_run",
            Some(fields.thread_id.as_str()),
            Some(fields.run_id),
            started_at,
            error,
        ),
    }
}

pub(super) fn claim_next_run_result<E>(
    scope_filter_thread_id: Option<&str>,
    started_at: Option<Instant>,
    claim: &Result<Option<ClaimedTurnRun>, E>,
) where
    E: fmt::Display,
{
    match claim {
        Ok(Some(claimed)) => trace_ok(
            "claim_next_run",
            Some(claimed.state.scope.thread_id.as_str()),
            Some(claimed.state.run_id),
            started_at,
        ),
        Ok(None) => trace_ok(
            "claim_next_run_empty",
            scope_filter_thread_id,
            None,
            started_at,
        ),
        Err(error) => trace_error(
            "claim_next_run",
            scope_filter_thread_id,
            None,
            started_at,
            error,
        ),
    }
}

fn trace_ok(
    operation: &'static str,
    thread_id: Option<&str>,
    run_id: Option<TurnRunId>,
    started_at: Option<Instant>,
) {
    let Some(started_at) = started_at else {
        return;
    };

    let run_id = run_id.map(|id| id.to_string()).unwrap_or_default();
    ironclaw_observability::live_latency_trace!(
        component = "turn_scheduler",
        operation,
        thread_id = thread_id.unwrap_or(""),
        run_id = run_id.as_str(),
        elapsed_ms = elapsed_ms(started_at),
        outcome = "ok",
        "turn scheduler operation completed",
    );
}

fn trace_error<E>(
    operation: &'static str,
    thread_id: Option<&str>,
    run_id: Option<TurnRunId>,
    started_at: Option<Instant>,
    error: &E,
) where
    E: fmt::Display + ?Sized,
{
    let Some(started_at) = started_at else {
        return;
    };

    let run_id = run_id.map(|id| id.to_string()).unwrap_or_default();
    ironclaw_observability::live_latency_trace!(
        component = "turn_scheduler",
        operation,
        thread_id = thread_id.unwrap_or(""),
        run_id = run_id.as_str(),
        elapsed_ms = elapsed_ms(started_at),
        outcome = "error",
        error = %error,
        "turn scheduler operation failed",
    );
}
