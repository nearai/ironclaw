use crate::llm::{ResponseAnomaly, ResponseMetadata};

pub(crate) const EMPTY_TOOL_COMPLETION_NUDGE: &str = "\
Your previous tool-enabled response was empty or malformed.\n\
Call valid tool(s) now if more work is required.\n\
If the job is done or blocked, call `finish_job` with valid `status` and `summary` arguments.";

pub(crate) const EMPTY_TOOL_COMPLETION_STRICT: &str = "\
Your previous recovery attempts did not include valid tool calls.\n\
In the next reply, you must either call valid tool(s) to continue your work \
or call `finish_job` with valid `status` and `summary` arguments.";

pub(crate) const EMPTY_TOOL_COMPLETION_FAILURE: &str = "the selected model repeatedly returned invalid autonomous responses and is not reliable for autonomous tool use.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum RecoveryStage {
    #[default]
    Idle,
    Nudged,
    StrictPending,
    StrictActive,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AutonomousRecoveryState {
    stage: RecoveryStage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutonomousRecoveryAction {
    Continue,
    ToolModeNudge,
    StrictToolRecovery,
    Fail,
}

impl AutonomousRecoveryState {
    pub(crate) fn begin_iteration(&mut self) -> bool {
        if self.stage == RecoveryStage::StrictPending {
            self.stage = RecoveryStage::StrictActive;
        }
        self.stage == RecoveryStage::StrictActive
    }

    pub(crate) fn on_text_response(
        &mut self,
        metadata: ResponseMetadata,
        _text: &str,
    ) -> AutonomousRecoveryAction {
        match self.stage {
            RecoveryStage::StrictPending | RecoveryStage::StrictActive => {
                self.reset();
                return AutonomousRecoveryAction::Fail;
            }
            RecoveryStage::Nudged => {
                self.stage = RecoveryStage::StrictPending;
                return AutonomousRecoveryAction::StrictToolRecovery;
            }
            RecoveryStage::Idle => {}
        }

        match metadata.anomaly {
            Some(ResponseAnomaly::EmptyToolCompletion) => {
                self.stage = RecoveryStage::Nudged;
                AutonomousRecoveryAction::ToolModeNudge
            }
            _ => AutonomousRecoveryAction::Continue,
        }
    }

    pub(crate) fn on_valid_tool_call(&mut self) {
        self.reset();
    }

    fn reset(&mut self) {
        self.stage = RecoveryStage::Idle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(anomaly: ResponseAnomaly) -> ResponseMetadata {
        ResponseMetadata {
            anomaly: Some(anomaly),
        }
    }

    #[test]
    fn first_empty_tool_completion_issues_nudge() {
        let mut state = AutonomousRecoveryState::default();
        let action = state.on_text_response(
            metadata(ResponseAnomaly::EmptyToolCompletion),
            "I'm not sure how to respond to that.",
        );
        assert_eq!(action, AutonomousRecoveryAction::ToolModeNudge);
        assert!(!state.begin_iteration());
    }

    #[test]
    fn second_invalid_response_after_nudge_schedules_strict_recovery() {
        let mut state = AutonomousRecoveryState::default();
        let _ = state.on_text_response(metadata(ResponseAnomaly::EmptyToolCompletion), "fallback");
        let action = state.on_text_response(ResponseMetadata::default(), "still working");
        assert_eq!(action, AutonomousRecoveryAction::StrictToolRecovery);
        assert!(state.begin_iteration());
    }

    #[test]
    fn strict_recovery_fails_on_plain_text() {
        let mut state = AutonomousRecoveryState::default();
        let _ = state.on_text_response(metadata(ResponseAnomaly::EmptyToolCompletion), "fallback");
        let _ = state.on_text_response(ResponseMetadata::default(), "still working");
        assert!(state.begin_iteration());

        let action = state.on_text_response(ResponseMetadata::default(), "done");
        assert_eq!(action, AutonomousRecoveryAction::Fail);
    }

    #[test]
    fn strict_recovery_fails_on_empty_tool_completion() {
        let mut state = AutonomousRecoveryState::default();
        let _ = state.on_text_response(metadata(ResponseAnomaly::EmptyToolCompletion), "fallback");
        let _ = state.on_text_response(
            metadata(ResponseAnomaly::EmptyToolCompletion),
            "still malformed",
        );
        assert!(state.begin_iteration());

        let action = state.on_text_response(
            metadata(ResponseAnomaly::EmptyToolCompletion),
            "still malformed",
        );
        assert_eq!(action, AutonomousRecoveryAction::Fail);
    }

    #[test]
    fn valid_tool_call_resets_counter() {
        let mut state = AutonomousRecoveryState::default();
        let _ = state.on_text_response(metadata(ResponseAnomaly::EmptyToolCompletion), "fallback");
        let _ = state.on_text_response(ResponseMetadata::default(), "still working");
        state.on_valid_tool_call();

        let action =
            state.on_text_response(metadata(ResponseAnomaly::EmptyToolCompletion), "fallback");
        assert_eq!(action, AutonomousRecoveryAction::ToolModeNudge);
    }

    #[test]
    fn normal_text_outside_recovery_continues() {
        let mut state = AutonomousRecoveryState::default();
        let action = state.on_text_response(ResponseMetadata::default(), "Still working on step 2");
        assert_eq!(action, AutonomousRecoveryAction::Continue);
    }
}
