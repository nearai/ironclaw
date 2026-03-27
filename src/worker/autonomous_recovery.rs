use crate::llm::{ResponseAnomaly, ResponseMetadata};

pub(crate) const EMPTY_TOOL_COMPLETION_NUDGE: &str = "\
Your previous tool-enabled response was empty or malformed.\n\
If you need to use a tool, call it now with valid arguments.\n\
Otherwise, provide a real status update about work already completed.";

pub(crate) const FORCE_TEXT_RECOVERY_PROMPT: &str = "\
Your previous tool-enabled responses were empty or malformed.\n\
Do not call any more tools in the next reply.\n\
Instead, provide a concise final status based only on work already completed.\n\
If the job is complete, say so explicitly. If not, explain what blocked you.";

pub(crate) const EMPTY_TOOL_COMPLETION_FAILURE: &str = "Execution failed: the selected model repeatedly returned empty or malformed tool-completion responses and is not reliable for autonomous tool use.";

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AutonomousRecoveryState {
    consecutive_empty_tool_completions: u8,
    force_text_recovery_pending: bool,
    force_text_recovery_active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutonomousRecoveryAction {
    Continue,
    ToolModeNudge,
    ForceTextRecovery,
    Fail,
}

impl AutonomousRecoveryState {
    pub(crate) fn begin_iteration(&mut self) -> bool {
        if self.force_text_recovery_pending {
            self.force_text_recovery_pending = false;
            self.force_text_recovery_active = true;
            true
        } else {
            self.force_text_recovery_active
        }
    }

    pub(crate) fn on_text_response(
        &mut self,
        metadata: ResponseMetadata,
        text: &str,
    ) -> AutonomousRecoveryAction {
        match metadata.anomaly {
            Some(ResponseAnomaly::EmptyToolCompletion) => {
                self.consecutive_empty_tool_completions += 1;
                self.force_text_recovery_active = false;
                match self.consecutive_empty_tool_completions {
                    1 => AutonomousRecoveryAction::ToolModeNudge,
                    2 => {
                        self.force_text_recovery_pending = true;
                        AutonomousRecoveryAction::ForceTextRecovery
                    }
                    _ => AutonomousRecoveryAction::Fail,
                }
            }
            Some(ResponseAnomaly::EmptyTextResponse) if self.force_text_recovery_active => {
                self.force_text_recovery_active = false;
                AutonomousRecoveryAction::Fail
            }
            _ if !text.trim().is_empty() => {
                self.reset();
                AutonomousRecoveryAction::Continue
            }
            _ => AutonomousRecoveryAction::Continue,
        }
    }

    pub(crate) fn on_valid_tool_call(&mut self) {
        self.reset();
    }

    fn reset(&mut self) {
        self.consecutive_empty_tool_completions = 0;
        self.force_text_recovery_pending = false;
        self.force_text_recovery_active = false;
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
    fn second_empty_tool_completion_schedules_text_recovery() {
        let mut state = AutonomousRecoveryState::default();
        let _ = state.on_text_response(metadata(ResponseAnomaly::EmptyToolCompletion), "fallback");
        let action =
            state.on_text_response(metadata(ResponseAnomaly::EmptyToolCompletion), "fallback");
        assert_eq!(action, AutonomousRecoveryAction::ForceTextRecovery);
        assert!(state.begin_iteration());
    }

    #[test]
    fn forced_text_recovery_fallback_fails() {
        let mut state = AutonomousRecoveryState::default();
        let _ = state.on_text_response(metadata(ResponseAnomaly::EmptyToolCompletion), "fallback");
        let _ = state.on_text_response(metadata(ResponseAnomaly::EmptyToolCompletion), "fallback");
        assert!(state.begin_iteration());
        let action =
            state.on_text_response(metadata(ResponseAnomaly::EmptyTextResponse), "fallback");
        assert_eq!(action, AutonomousRecoveryAction::Fail);
    }

    #[test]
    fn valid_tool_call_resets_counter() {
        let mut state = AutonomousRecoveryState::default();
        let _ = state.on_text_response(metadata(ResponseAnomaly::EmptyToolCompletion), "fallback");
        state.on_valid_tool_call();
        let action =
            state.on_text_response(metadata(ResponseAnomaly::EmptyToolCompletion), "fallback");
        assert_eq!(action, AutonomousRecoveryAction::ToolModeNudge);
    }

    #[test]
    fn meaningful_text_after_text_recovery_resets_state() {
        let mut state = AutonomousRecoveryState::default();
        let _ = state.on_text_response(metadata(ResponseAnomaly::EmptyToolCompletion), "fallback");
        let _ = state.on_text_response(metadata(ResponseAnomaly::EmptyToolCompletion), "fallback");
        assert!(state.begin_iteration());

        let action = state.on_text_response(ResponseMetadata::default(), "Still working on step 2");
        assert_eq!(action, AutonomousRecoveryAction::Continue);

        let next =
            state.on_text_response(metadata(ResponseAnomaly::EmptyToolCompletion), "fallback");
        assert_eq!(next, AutonomousRecoveryAction::ToolModeNudge);
    }
}
