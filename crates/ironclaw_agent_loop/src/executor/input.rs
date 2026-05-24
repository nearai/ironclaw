use super::*;

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct InputStage;

#[derive(Debug, Clone, Copy)]
pub(super) enum UserFacingInputDrainMode {
    Steering,
    FollowUp,
}

pub(super) struct InputInput {
    pub(super) state: LoopExecutionState,
    pub(super) pending_input_ack: PendingInputAck,
    pub(super) mode: UserFacingInputDrainMode,
}

pub(super) enum InputStep {
    Continue {
        state: Box<LoopExecutionState>,
        pending_input_ack: PendingInputAck,
        drained: bool,
    },
    Exit(LoopExit),
}

#[async_trait]
impl ExecutorStage<InputInput> for InputStage {
    type Output = InputStep;

    async fn process(
        &self,
        ctx: StageContext<'_>,
        input: InputInput,
    ) -> Result<InputStep, AgentLoopExecutorError> {
        let mut state = input.state;
        let mut pending_input_ack = input.pending_input_ack;

        let should_drain = match input.mode {
            UserFacingInputDrainMode::Steering => {
                pending_input_ack.is_empty() && ctx.planner.drain().drain_steering(&state).await
            }
            UserFacingInputDrainMode::FollowUp => ctx.planner.drain().drain_followup(&state).await,
        };

        if should_drain {
            state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
                CancelCheck::Continue(state) => *state,
                CancelCheck::Exit(exit) => return Ok(InputStep::Exit(exit)),
            };
            let drained = self.drain(ctx, state, input.mode).await?;
            state = drained.state;
            pending_input_ack.replace(drained.ack_tokens)?;
            if let Some(reason_kind) = drained.cancelled_reason_kind {
                let checked = CheckpointStage
                    .write(ctx, state, CheckpointKind::Final)
                    .await?;
                pending_input_ack.ack(ctx.host).await?;
                return Ok(InputStep::Exit(cancelled_exit_with_reason(
                    ctx.host,
                    checked.state,
                    reason_kind,
                    Some(checked.checkpoint_id),
                )?));
            }
            state = match CheckpointStage
                .cancel_if_requested_after_pending_input_ack(ctx, state, &mut pending_input_ack)
                .await?
            {
                CancelCheck::Continue(state) => *state,
                CancelCheck::Exit(exit) => return Ok(InputStep::Exit(exit)),
            };
            return Ok(InputStep::Continue {
                state: Box::new(state),
                pending_input_ack,
                drained: drained.drained,
            });
        }

        if matches!(input.mode, UserFacingInputDrainMode::Steering) {
            state = match CheckpointStage
                .cancel_if_requested_after_pending_input_ack(ctx, state, &mut pending_input_ack)
                .await?
            {
                CancelCheck::Continue(state) => *state,
                CancelCheck::Exit(exit) => return Ok(InputStep::Exit(exit)),
            };
        }

        Ok(InputStep::Continue {
            state: Box::new(state),
            pending_input_ack,
            drained: false,
        })
    }
}

impl InputStage {
    #[cfg(test)]
    pub(super) async fn drain_user_inputs(
        &self,
        ctx: StageContext<'_>,
        state: LoopExecutionState,
    ) -> Result<DrainedInputs, AgentLoopExecutorError> {
        self.drain(ctx, state, UserFacingInputDrainMode::Steering)
            .await
    }

    #[cfg(test)]
    pub(super) async fn drain_followup(
        &self,
        ctx: StageContext<'_>,
        state: LoopExecutionState,
    ) -> Result<DrainedInputs, AgentLoopExecutorError> {
        self.drain(ctx, state, UserFacingInputDrainMode::FollowUp)
            .await
    }

    async fn drain(
        &self,
        ctx: StageContext<'_>,
        mut state: LoopExecutionState,
        mode: UserFacingInputDrainMode,
    ) -> Result<DrainedInputs, AgentLoopExecutorError> {
        let batch = ctx
            .host
            .poll_inputs(state.input_cursor.clone(), MAX_INPUT_DRAIN)
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                stage: HostStage::Input,
            })?;
        let (drained, ack_tokens, cancelled_reason_kind) =
            consume_drainable_inputs(&batch, mode, &mut state)?;
        Ok(DrainedInputs {
            state,
            drained,
            ack_tokens,
            cancelled_reason_kind,
        })
    }
}

pub(super) fn consume_drainable_inputs(
    batch: &LoopInputBatch,
    mode: UserFacingInputDrainMode,
    state: &mut LoopExecutionState,
) -> Result<
    (
        bool,
        Vec<LoopInputAckToken>,
        Option<LoopCancelledReasonKind>,
    ),
    AgentLoopExecutorError,
> {
    let mut consumed_len = 0;
    let mut drained = false;
    let mut cancelled_reason_kind = None;
    for input in &batch.inputs {
        if user_facing_input_matches_drain_mode(input, mode) {
            consumed_len += 1;
            drained = true;
            continue;
        }
        match input {
            LoopInput::Cancel { .. } => {
                consumed_len += 1;
                cancelled_reason_kind = Some(LoopCancelledReasonKind::HostCancellation);
                break;
            }
            LoopInput::Interrupt { .. } => {
                consumed_len += 1;
                cancelled_reason_kind = Some(LoopCancelledReasonKind::HostInterrupt);
                break;
            }
            LoopInput::GateResolved { .. } | LoopInput::CapabilitySurfaceChanged { .. } => break,
            LoopInput::UserMessage { .. }
            | LoopInput::FollowUp { .. }
            | LoopInput::Steering { .. } => {
                break;
            }
        }
    }
    if consumed_len == 0 {
        return Ok((false, Vec::new(), None));
    }
    if batch.input_acks.len() < consumed_len {
        return Err(AgentLoopExecutorError::PlannerContract {
            detail: "input batch omitted ack metadata for consumed inputs",
        });
    }
    let last_ack = &batch.input_acks[consumed_len - 1];
    state.input_cursor = last_ack.cursor.clone();
    let ack_tokens = batch
        .input_acks
        .iter()
        .take(consumed_len)
        .map(|ack| ack.token.clone())
        .collect();
    Ok((drained, ack_tokens, cancelled_reason_kind))
}

fn user_facing_input_matches_drain_mode(input: &LoopInput, mode: UserFacingInputDrainMode) -> bool {
    match mode {
        UserFacingInputDrainMode::Steering => {
            matches!(
                input,
                LoopInput::UserMessage { .. } | LoopInput::Steering { .. }
            )
        }
        UserFacingInputDrainMode::FollowUp => {
            matches!(
                input,
                LoopInput::FollowUp { .. } | LoopInput::UserMessage { .. }
            )
        }
    }
}
