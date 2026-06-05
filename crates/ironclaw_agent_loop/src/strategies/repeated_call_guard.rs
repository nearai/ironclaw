use crate::{
    state::{
        CapabilityCallSignature, LoopExecutionState, RepeatedCallWarningPhase,
        RepeatedCallWarningState, StopStrategyState,
    },
    strategies::{TurnEndKind, TurnSummary},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepeatedCallObservation {
    pub(crate) signature: CapabilityCallSignature,
    pub(crate) count: usize,
}

pub(crate) fn dominant_repeated_call(
    state: &LoopExecutionState,
    window: usize,
    threshold: usize,
) -> Option<RepeatedCallObservation> {
    let (signature, count) = state.recent_call_signatures.most_common_in(window)?;
    if count < threshold {
        return None;
    }
    Some(RepeatedCallObservation { signature, count })
}

pub(crate) fn observe_repeated_call_warning(
    state: &LoopExecutionState,
    just_completed: &TurnSummary,
    mut stop_state: StopStrategyState,
    window: usize,
    threshold: usize,
) -> StopStrategyState {
    let Some(repeated) = dominant_repeated_call(state, window, threshold) else {
        stop_state.repeated_call_warning = None;
        return stop_state;
    };

    stop_state.repeated_call_warning = match state.stop_state.repeated_call_warning.as_ref() {
        Some(existing) if existing.signature == repeated.signature => {
            transition_existing_warning(existing, just_completed, repeated.signature)
        }
        _ => Some(RepeatedCallWarningState::pending_render(repeated.signature)),
    };
    stop_state
}

pub(crate) fn repeated_call_warning_is_terminal_ready(state: &LoopExecutionState) -> bool {
    state
        .stop_state
        .repeated_call_warning
        .as_ref()
        .is_some_and(|warning| warning.phase == RepeatedCallWarningPhase::TerminalReady)
}

fn transition_existing_warning(
    existing: &RepeatedCallWarningState,
    just_completed: &TurnSummary,
    signature: CapabilityCallSignature,
) -> Option<RepeatedCallWarningState> {
    match existing.phase {
        RepeatedCallWarningPhase::PendingRender => {
            Some(RepeatedCallWarningState::pending_render(signature))
        }
        RepeatedCallWarningPhase::Rendered => {
            if all_results_reported_no_progress(just_completed) {
                Some(RepeatedCallWarningState::terminal_ready(signature))
            } else {
                None
            }
        }
        RepeatedCallWarningPhase::TerminalReady => {
            Some(RepeatedCallWarningState::terminal_ready(signature))
        }
    }
}

fn all_results_reported_no_progress(just_completed: &TurnSummary) -> bool {
    just_completed.kind == TurnEndKind::AfterCapabilityBatch
        && just_completed.capability_batch.invocation_count > 0
        && just_completed.capability_batch.no_progress_count
            == just_completed.capability_batch.invocation_count
}
