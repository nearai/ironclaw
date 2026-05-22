use async_trait::async_trait;
use ironclaw_turns::{
    LoopBlocked, LoopExit,
    run_profile::{CapabilityCallCandidate, CapabilityResultMessage, LoopProgressEvent},
};

use crate::{
    state::{CheckpointKind, LoopExecutionState},
    strategies::{GateKind, GateOutcome},
};

use super::{
    AgentLoopExecutorError, BatchStep, CancelCheck, CheckpointStage, ExecutorStage, StageContext,
    append_capability_result_ref, append_capability_safe_summary_ref, blocked_kind, exit_id,
    failed_exit, gate_tool_result_summary, loop_gate_kind, push_completed_result,
};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct GateStage;

pub(super) struct GateInput {
    pub(super) state: LoopExecutionState,
    pub(super) call: CapabilityCallCandidate,
    pub(super) kind: GateKind,
    pub(super) gate_ref: ironclaw_turns::LoopGateRef,
    /// Pre-resolved capability result that must be appended to the
    /// transcript before the gate suspends (Block) or skips
    /// (SkipAndContinue) — currently used by `AwaitDependentRun` so the
    /// spawned child's `result_ref` is durable before the parent blocks.
    pub(super) resolved_result: Option<CapabilityResultMessage>,
}

#[async_trait]
impl ExecutorStage<GateInput> for GateStage {
    type Output = BatchStep;

    async fn process(
        &self,
        ctx: StageContext<'_>,
        input: GateInput,
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        let mut state = input.state;
        let call = input.call;
        let kind = input.kind;
        let gate_ref = input.gate_ref;
        let resolved_result = input.resolved_result;
        let summary = crate::strategies::GateSummary {
            kind,
            gate_ref: gate_ref.clone(),
        };
        match ctx.planner.gate().handle(&state, &summary).await {
            GateOutcome::Block { gate } => {
                state.gate_state = gate;
                state.last_gate = Some(gate_ref.clone());
                if let Some(result) = resolved_result {
                    append_capability_result_ref(ctx.host, &call, &result).await?;
                    push_completed_result(&mut state, result);
                }
                match CheckpointStage.cancel_if_requested(ctx, state).await? {
                    CancelCheck::Continue(next) => state = *next,
                    CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                }
                CheckpointStage
                    .emit_progress(
                        ctx,
                        LoopProgressEvent::GateBlocked {
                            iteration: state.iteration,
                            gate_kind: loop_gate_kind(kind),
                        },
                    )
                    .await;
                let checked = CheckpointStage
                    .write(ctx, state, CheckpointKind::BeforeBlock)
                    .await?;
                Ok(BatchStep::Exit(LoopExit::Blocked(LoopBlocked {
                    kind: blocked_kind(kind),
                    gate_ref,
                    checkpoint_id: checked.checkpoint_id,
                    state_ref: checked.state_ref,
                    exit_id: exit_id(ctx.host, "blocked")?,
                })))
            }
            GateOutcome::SkipAndContinue { gate } => {
                state.gate_state = gate;
                if let Some(result) = resolved_result {
                    append_capability_result_ref(ctx.host, &call, &result).await?;
                    push_completed_result(&mut state, result);
                } else {
                    append_capability_safe_summary_ref(
                        ctx.host,
                        &mut state,
                        &call,
                        gate_tool_result_summary(kind, "skipped"),
                    )
                    .await?;
                }
                match CheckpointStage.cancel_if_requested(ctx, state).await? {
                    CancelCheck::Continue(next) => state = *next,
                    CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                }
                Ok(BatchStep::Continue(Box::new(state)))
            }
            GateOutcome::Abort { gate, failure_kind } => {
                state.gate_state = gate;
                append_capability_safe_summary_ref(
                    ctx.host,
                    &mut state,
                    &call,
                    gate_tool_result_summary(kind, "aborted"),
                )
                .await?;
                match CheckpointStage.cancel_if_requested(ctx, state).await? {
                    CancelCheck::Continue(next) => state = *next,
                    CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                }
                let checked = CheckpointStage
                    .write(ctx, state, CheckpointKind::Final)
                    .await?;
                Ok(BatchStep::Exit(failed_exit(
                    ctx.host,
                    checked.state,
                    failure_kind,
                    Some(checked.checkpoint_id),
                )?))
            }
        }
    }
}
