use async_trait::async_trait;
use ironclaw_turns::LoopExit;
use tracing::debug;

use crate::{
    state::LoopExecutionState,
    strategies::{StopKind, StopOutcome, TurnSummary},
};

use super::{
    AgentLoopExecutorError, CancelCheck, CheckpointStage, ExecutorStage, PendingInputAck,
    StageContext, latency, loop_exit::completion_nudge_should_fire,
};

/// Stop-stage helper for callers that can observe and decide back-to-back.
///
/// Reply-only executor paths that need to drain queued follow-up input before
/// the terminal stop decision must call `observe`, perform the drain, then
/// call `decide` instead of using the combined `process` entry point.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct StopStage;

pub(super) struct StopInput {
    pub(super) state: LoopExecutionState,
    pub(super) summary: TurnSummary,
    pub(super) pending_input_ack: PendingInputAck,
}

pub(super) struct StopObservationInput {
    pub(super) state: LoopExecutionState,
    pub(super) summary: TurnSummary,
}

pub(super) enum StopObservationStep {
    Continue {
        state: Box<LoopExecutionState>,
        summary: TurnSummary,
    },
    Exit(LoopExit),
}

pub(super) enum StopStep {
    Continue {
        state: LoopExecutionState,
        pending_input_ack: PendingInputAck,
    },
    Stop {
        state: LoopExecutionState,
        kind: StopKind,
        pending_input_ack: PendingInputAck,
    },
    Exit(LoopExit),
}

#[async_trait]
impl ExecutorStage<StopInput> for StopStage {
    type Output = StopStep;

    async fn process(
        &self,
        ctx: StageContext<'_>,
        input: StopInput,
    ) -> Result<StopStep, AgentLoopExecutorError> {
        match self
            .observe(
                ctx,
                StopObservationInput {
                    state: input.state,
                    summary: input.summary,
                },
            )
            .await?
        {
            StopObservationStep::Continue { state, summary } => {
                self.decide(
                    ctx,
                    StopInput {
                        state: *state,
                        summary,
                        pending_input_ack: input.pending_input_ack,
                    },
                )
                .await
            }
            StopObservationStep::Exit(exit) => Ok(StopStep::Exit(exit)),
        }
    }
}

impl StopStage {
    pub(super) async fn observe(
        &self,
        ctx: StageContext<'_>,
        input: StopObservationInput,
    ) -> Result<StopObservationStep, AgentLoopExecutorError> {
        let mut state = input.state;
        state.stop_state = ctx
            .planner
            .stop()
            .observe_completed_turn(&state, &input.summary)
            .await;
        state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
            CancelCheck::Continue(state) => *state,
            CancelCheck::Exit(exit) => return Ok(StopObservationStep::Exit(exit)),
        };
        Ok(StopObservationStep::Continue {
            state: Box::new(state),
            summary: input.summary,
        })
    }

    pub(super) async fn decide(
        &self,
        ctx: StageContext<'_>,
        input: StopInput,
    ) -> Result<StopStep, AgentLoopExecutorError> {
        let mut state = input.state;
        let pending_input_ack = input.pending_input_ack;
        // `decide` is also a cancellation boundary for callers that split
        // observation from the terminal decision.
        match ctx
            .planner
            .stop()
            .should_stop_after_observed_turn(&state, &input.summary)
            .await
        {
            StopOutcome::Stop { kind } => {
                state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
                    CancelCheck::Continue(state) => *state,
                    CancelCheck::Exit(exit) => return Ok(StopStep::Exit(exit)),
                };
                Ok(StopStep::Stop {
                    state,
                    kind,
                    pending_input_ack,
                })
            }
            StopOutcome::Continue {} => {
                state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
                    CancelCheck::Continue(state) => *state,
                    CancelCheck::Exit(exit) => return Ok(StopStep::Exit(exit)),
                };
                Ok(StopStep::Continue {
                    state,
                    pending_input_ack,
                })
            }
        }
    }

    /// `decide`, plus the tools-capable completion-nudge check. Only the main
    /// per-iteration executor path calls this; the `ResumeApproval`/
    /// `ResumeAuth`/`ResumeExternalTool`/`SkipModel` paths call plain
    /// `decide` and never nudge — expressed here by which method canonical.rs
    /// calls, not by a flag threaded through every caller.
    pub(super) async fn decide_with_completion_nudge(
        &self,
        ctx: StageContext<'_>,
        input: StopInput,
    ) -> Result<StopStep, AgentLoopExecutorError> {
        match self.decide(ctx, input).await? {
            StopStep::Stop {
                state: stop_state,
                kind,
                pending_input_ack,
            } if completion_nudge_should_fire(ctx.host, &stop_state, &kind) => {
                // Instead of terminating, re-enter the loop for one more
                // iteration with the full tool surface and a completion-nudge
                // directive, so the model can finish the task (e.g. write a
                // required output file) before answering. Mirrors the
                // drained-follow-up continue (defer the ack returned by
                // stop.decide to the next iteration) rather than the
                // terminal, tool-free final-answer nudge.
                let mut stop_state = stop_state;
                stop_state.completion_nudges_used += 1;
                stop_state.completion_nudge_pending = true;
                stop_state.last_reply_trailed_off = false;
                debug!(
                    iteration = stop_state.iteration,
                    ?kind,
                    completion_nudges_used = stop_state.completion_nudges_used,
                    "agent loop issuing tools-capable completion nudge instead of stopping"
                );
                Ok(StopStep::Continue {
                    state: stop_state,
                    pending_input_ack,
                })
            }
            other => Ok(other),
        }
    }

    /// Latency-instrumented sibling of [`Self::observe`].
    pub(super) async fn observe_timed(
        &self,
        operation: &'static str,
        ctx: StageContext<'_>,
        iteration: u32,
        input: StopObservationInput,
    ) -> Result<StopObservationStep, AgentLoopExecutorError> {
        latency::stage!(
            operation,
            ctx.host.run_context(),
            iteration,
            self.observe(ctx, input)
        )
    }

    /// Latency-instrumented sibling of [`Self::decide`].
    pub(super) async fn decide_timed(
        &self,
        operation: &'static str,
        ctx: StageContext<'_>,
        iteration: u32,
        input: StopInput,
    ) -> Result<StopStep, AgentLoopExecutorError> {
        latency::stage!(
            operation,
            ctx.host.run_context(),
            iteration,
            self.decide(ctx, input)
        )
    }

    /// Latency-instrumented sibling of [`Self::decide_with_completion_nudge`].
    pub(super) async fn decide_with_completion_nudge_timed(
        &self,
        operation: &'static str,
        ctx: StageContext<'_>,
        iteration: u32,
        input: StopInput,
    ) -> Result<StopStep, AgentLoopExecutorError> {
        latency::stage!(
            operation,
            ctx.host.run_context(),
            iteration,
            self.decide_with_completion_nudge(ctx, input)
        )
    }
}
