use super::*;

#[derive(Debug, Clone, Copy)]
pub(super) enum UserFacingInputDrainMode {
    Steering,
    FollowUp,
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
