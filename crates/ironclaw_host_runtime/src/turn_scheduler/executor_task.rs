use std::{any::Any, time::Instant};

use ironclaw_turns::{SanitizedFailure, TurnRunId, TurnScope};

use super::{TurnRunExecutorError, latency, scheduler_failure};

pub(super) enum ExecutorTaskOutcome {
    Completed,
    TerminalFailure(Option<SanitizedFailure>),
}

pub(super) fn result_to_outcome(
    scope: &TurnScope,
    run_id: TurnRunId,
    started_at: Option<Instant>,
    result: Result<Result<(), TurnRunExecutorError>, Box<dyn Any + Send>>,
) -> ExecutorTaskOutcome {
    match result {
        Ok(Ok(())) => {
            latency::operation_ok("execute_claimed_run", scope, run_id, started_at);
            ExecutorTaskOutcome::Completed
        }
        Ok(Err(error)) => {
            latency::operation_error(
                "execute_claimed_run",
                scope,
                run_id,
                started_at,
                "executor_error",
            );
            ExecutorTaskOutcome::TerminalFailure(Some(error.failure().clone()))
        }
        Err(_) => {
            let reason = "scheduler_executor_panic";
            latency::operation_error("execute_claimed_run", scope, run_id, started_at, reason);
            ExecutorTaskOutcome::TerminalFailure(scheduler_failure(reason))
        }
    }
}
