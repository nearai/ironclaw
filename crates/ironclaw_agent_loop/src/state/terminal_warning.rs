use std::collections::BTreeSet;

use ironclaw_turns::LoopFailureKind;

const TERMINAL_WARNING_SCHEMA_VERSION: u32 = 1;

/// Typed, host-authored warning delivered before an otherwise-terminal loop
/// condition receives its one final recovery iteration.
///
/// Only bounded typed facts are stored. Raw model/provider content and backend
/// diagnostics are deliberately excluded from checkpoint state and prompt text.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct TerminalWarningObservation {
    #[serde(default = "current_schema_version")]
    pub(crate) schema_version: u32,
    detail: TerminalWarningDetail,
}

impl TerminalWarningObservation {
    pub(crate) fn no_progress(
        repeated_call_count: Option<u32>,
        last_failure: Option<LoopFailureKind>,
    ) -> Self {
        Self {
            schema_version: TERMINAL_WARNING_SCHEMA_VERSION,
            detail: TerminalWarningDetail::NoProgressDetected {
                repeated_call_count,
                last_failure,
            },
        }
    }

    pub(crate) fn iteration_limit(limit: u32) -> Self {
        Self {
            schema_version: TERMINAL_WARNING_SCHEMA_VERSION,
            detail: TerminalWarningDetail::IterationLimit { limit },
        }
    }

    pub(crate) fn kind(&self) -> TerminalWarningKind {
        match self.detail {
            TerminalWarningDetail::NoProgressDetected { .. } => {
                TerminalWarningKind::NoProgressDetected
            }
            TerminalWarningDetail::IterationLimit { .. } => TerminalWarningKind::IterationLimit,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema_version != TERMINAL_WARNING_SCHEMA_VERSION {
            return Err(format!(
                "terminal warning schema version {} is unsupported",
                self.schema_version
            ));
        }
        Ok(())
    }

    pub(crate) fn model_instruction(&self) -> String {
        match self.detail {
            TerminalWarningDetail::NoProgressDetected {
                repeated_call_count,
                last_failure,
            } => {
                let repeated = repeated_call_count
                    .filter(|count| *count > 1)
                    .map(|count| format!(" after the same capability call repeated {count} times"))
                    .unwrap_or_default();
                let failure = last_failure
                    .map(|kind| format!(" last_failure={}", kind.as_str()))
                    .unwrap_or_default();
                format!(
                    "loop warning: no progress detected{repeated};{failure} change approach or provide a final answer now"
                )
            }
            TerminalWarningDetail::IterationLimit { limit } => format!(
                "loop warning: iteration limit {limit} reached; this is the final recovery iteration; complete the task or provide the best final answer now"
            ),
        }
    }
}

fn current_schema_version() -> u32 {
    TERMINAL_WARNING_SCHEMA_VERSION
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TerminalWarningDetail {
    NoProgressDetected {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repeated_call_count: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_failure: Option<LoopFailureKind>,
    },
    IterationLimit {
        limit: u32,
    },
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TerminalWarningKind {
    NoProgressDetected,
    IterationLimit,
}

/// Checkpoint-persistent accounting for pre-termination warning iterations.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TerminalWarningState {
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    attempted: BTreeSet<TerminalWarningKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending: Option<TerminalWarningObservation>,
    /// Warning whose model response is being processed. This survives a
    /// capability gate/resume so the first completed warning turn can be
    /// evaluated without reopening an unbounded no-progress window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active: Option<TerminalWarningKind>,
}

impl TerminalWarningState {
    /// Schedule a warning exactly once per terminal class.
    pub(crate) fn schedule(&mut self, observation: TerminalWarningObservation) -> bool {
        let kind = observation.kind();
        if self.pending.is_some() || self.attempted.contains(&kind) {
            return false;
        }
        self.attempted.insert(kind);
        self.pending = Some(observation);
        true
    }

    #[cfg(test)]
    pub(crate) fn attempted(&self, kind: TerminalWarningKind) -> bool {
        self.attempted.contains(&kind)
    }

    pub(crate) fn pending(&self) -> Option<&TerminalWarningObservation> {
        self.pending.as_ref()
    }

    pub(crate) fn clear_pending(&mut self) {
        self.pending = None;
    }

    /// Mark the pending warning as delivered only after a model response was
    /// returned. Gate-shaped errors occur before provider dispatch and must
    /// leave the warning pending for the approved retry.
    pub(crate) fn mark_delivered(&mut self) {
        if let Some(observation) = self.pending.take() {
            self.active = Some(observation.kind());
        }
    }

    pub(crate) fn active(&self) -> Option<TerminalWarningKind> {
        self.active
    }

    pub(crate) fn clear_active(&mut self) {
        self.active = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_progress_instruction_contains_only_typed_recovery_context() {
        let observation =
            TerminalWarningObservation::no_progress(Some(4), Some(LoopFailureKind::PolicyDenied));

        observation.validate().expect("observation validates");
        assert_eq!(
            observation.model_instruction(),
            "loop warning: no progress detected after the same capability call repeated 4 times; last_failure=policy_denied change approach or provide a final answer now"
        );
    }

    #[test]
    fn warning_state_schedules_each_terminal_class_once() {
        let mut state = TerminalWarningState::default();

        assert!(state.schedule(TerminalWarningObservation::iteration_limit(8)));
        state.clear_pending();
        assert!(!state.schedule(TerminalWarningObservation::iteration_limit(8)));
        assert!(state.schedule(TerminalWarningObservation::no_progress(None, None)));
    }

    #[test]
    fn warning_state_never_replaces_an_unconsumed_warning() {
        let mut state = TerminalWarningState::default();

        assert!(state.schedule(TerminalWarningObservation::no_progress(None, None)));
        assert!(!state.schedule(TerminalWarningObservation::iteration_limit(8)));
        assert_eq!(
            state.pending().map(TerminalWarningObservation::kind),
            Some(TerminalWarningKind::NoProgressDetected)
        );
        assert!(!state.attempted(TerminalWarningKind::IterationLimit));
    }

    #[test]
    fn observation_rejects_unknown_schema_version() {
        let mut observation = TerminalWarningObservation::iteration_limit(8);
        observation.schema_version += 1;

        assert_eq!(
            observation.validate(),
            Err("terminal warning schema version 2 is unsupported".to_string())
        );
    }
}
