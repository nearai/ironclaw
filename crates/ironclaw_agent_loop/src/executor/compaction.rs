use async_trait::async_trait;
use ironclaw_turns::LoopFailureKind;
use ironclaw_turns::{
    LoopExit,
    run_profile::{
        CompactionInitiator, LoopCompactionError, LoopCompactionMode, LoopCompactionRequest,
        LoopProgressEvent, LoopSafeSummary, SystemInferenceTaskId,
    },
};
use std::time::Duration;

use crate::state::{CheckpointKind, LoopExecutionState};
use crate::strategies::CompactionDecision;

use super::{
    AgentLoopExecutorError, CheckpointStage, ExecutorStage, PendingInputAck, StageContext,
    failed_exit,
};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct CompactionStage;

pub(crate) struct CompactionInput {
    pub(crate) state: LoopExecutionState,
    pub(crate) pending_input_ack: PendingInputAck,
}

pub(crate) struct CompactionOutput {
    pub(crate) state: Box<LoopExecutionState>,
    pub(crate) pending_input_ack: PendingInputAck,
    pub(crate) exit: Option<LoopExit>,
}

#[async_trait]
impl ExecutorStage<CompactionInput> for CompactionStage {
    type Output = CompactionOutput;

    async fn process(
        &self,
        ctx: StageContext<'_>,
        input: CompactionInput,
    ) -> Result<Self::Output, AgentLoopExecutorError> {
        let mut state = input.state;
        let decision = ctx
            .planner
            .compaction()
            .should_compact(&state, ctx.host.run_context());

        match decision {
            CompactionDecision::Skip => {}
            CompactionDecision::Trigger {
                drop_through_seq,
                preserve_tail_tokens,
                deadline_ms,
            } => {
                let task_id = SystemInferenceTaskId::new();
                CheckpointStage
                    .emit_progress(
                        ctx,
                        LoopProgressEvent::CompactionStarted {
                            task_id,
                            initiator: CompactionInitiator::Auto,
                        },
                    )
                    .await;
                let compaction_result = tokio::time::timeout(
                    Duration::from_millis(deadline_ms),
                    ctx.host.compact_loop_context(LoopCompactionRequest {
                        task_id,
                        thread_id: ctx.host.run_context().thread_id.clone(),
                        last_compacted_through_seq: state
                            .compaction_state
                            .last_compacted_through_seq,
                        drop_through_seq,
                        preserve_tail_tokens,
                        mode: LoopCompactionMode::Fresh,
                        deadline_ms,
                    }),
                )
                .await;
                let response = match compaction_result {
                    Ok(Ok(response)) => response,
                    Ok(Err(error)) => {
                        CheckpointStage
                            .emit_progress(
                                ctx,
                                LoopProgressEvent::CompactionFailed {
                                    task_id,
                                    reason_kind: loop_compaction_reason(&error),
                                },
                            )
                            .await;
                        let checked = CheckpointStage
                            .write(ctx, state, CheckpointKind::Final)
                            .await?;
                        let exit = failed_exit(
                            ctx.host,
                            checked.state,
                            LoopFailureKind::CompactionUnavailable,
                            Some(checked.checkpoint_id),
                        )?;
                        return Ok(CompactionOutput {
                            state: Box::new(LoopExecutionState::initial_for_run(
                                ctx.host.run_context(),
                            )),
                            pending_input_ack: input.pending_input_ack,
                            exit: Some(exit),
                        });
                    }
                    Err(_) => {
                        let error = LoopCompactionError::InferenceFailed {
                            safe_summary: safe("compaction deadline exceeded"),
                        };
                        CheckpointStage
                            .emit_progress(
                                ctx,
                                LoopProgressEvent::CompactionFailed {
                                    task_id,
                                    reason_kind: loop_compaction_reason(&error),
                                },
                            )
                            .await;
                        let checked = CheckpointStage
                            .write(ctx, state, CheckpointKind::Final)
                            .await?;
                        let exit = failed_exit(
                            ctx.host,
                            checked.state,
                            LoopFailureKind::CompactionUnavailable,
                            Some(checked.checkpoint_id),
                        )?;
                        return Ok(CompactionOutput {
                            state: Box::new(LoopExecutionState::initial_for_run(
                                ctx.host.run_context(),
                            )),
                            pending_input_ack: input.pending_input_ack,
                            exit: Some(exit),
                        });
                    }
                };
                state.compaction_state.last_compacted_through_seq = Some(drop_through_seq);
                state.compaction_state.force_compact_on_next_iteration = false;
                state
                    .compaction_prompt
                    .retain_after_sequence(drop_through_seq);
                CheckpointStage
                    .emit_progress(
                        ctx,
                        LoopProgressEvent::CompactionCompleted {
                            task_id,
                            compression_ratio_ppm: response.compression_ratio_ppm,
                        },
                    )
                    .await;
            }
        }

        Ok(CompactionOutput {
            state: Box::new(state),
            pending_input_ack: input.pending_input_ack,
            exit: None,
        })
    }
}

fn loop_compaction_reason(error: &LoopCompactionError) -> LoopSafeSummary {
    let value = match error {
        LoopCompactionError::InvalidCutPoint => "invalid cut point",
        LoopCompactionError::InputTooLarge => "input too large",
        LoopCompactionError::SecurityRejected { .. } => "security rejected",
        LoopCompactionError::InferenceFailed { .. } => "inference failed",
        LoopCompactionError::PersistenceFailed { .. } => "persistence failed",
    };
    LoopSafeSummary::new(value).unwrap_or_else(|_| LoopSafeSummary::model_gateway_failed())
}

fn safe(value: &'static str) -> LoopSafeSummary {
    LoopSafeSummary::new(value).unwrap_or_else(|_| LoopSafeSummary::model_gateway_failed())
}
