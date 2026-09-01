use super::CapabilityCallSignature;

/// Persistent state owned by `StopConditionStrategy`. Split from a previously
/// shared `ControlStrategyState` so Stop and Gate evolve independently — a
/// future family's growth in stop-condition state cannot perturb gate-handler
/// invariants and vice versa.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StopStrategyState {
    /// Number of completed turns the StopConditionStrategy has observed.
    pub turns_completed: u32,
    /// Consecutive turns where a model reply was rejected before transcript
    /// finalization.
    #[serde(default)]
    pub trailing_rejected_replies: u32,
    /// Deprecated checkpoint tombstone retained for rollback compatibility.
    /// The default stop strategy always writes zero and never reads it.
    #[serde(default)]
    pub trailing_no_progress_results: u32,
    /// Consecutive completed capability-batch turns in which EVERY invocation
    /// failed (no completed-call signature was observed). Counted only by the
    /// structured-result stop strategy, where a run of all-failed
    /// batches is repeated invalid result-tool output.
    #[serde(default)]
    pub trailing_all_failed_batches: u32,
    /// A completed host-owned structured-result call was observed during this
    /// run. Failed calls never set this bit. The terminal mapper combines it
    /// with the scheduled suppression policy before producing NothingToReport.
    #[serde(default)]
    pub structured_result_recorded: bool,
    /// Pending or rendered advisory shown when the same capability call is
    /// repeated consecutively. This warning never authorizes a heuristic stop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeated_call_warning: Option<RepeatedCallWarningState>,
}

impl StopStrategyState {
    pub fn mark_repeated_call_warning_rendered(&mut self) {
        if let Some(warning) = self.repeated_call_warning.as_mut()
            && warning.phase == RepeatedCallWarningPhase::PendingRender
        {
            warning.phase = RepeatedCallWarningPhase::Rendered;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RepeatedCallWarningState {
    pub signature: CapabilityCallSignature,
    pub phase: RepeatedCallWarningPhase,
}

impl RepeatedCallWarningState {
    pub fn pending_render(signature: CapabilityCallSignature) -> Self {
        Self {
            signature,
            phase: RepeatedCallWarningPhase::PendingRender,
        }
    }

    pub fn rendered(signature: CapabilityCallSignature) -> Self {
        Self {
            signature,
            phase: RepeatedCallWarningPhase::Rendered,
        }
    }

    pub fn terminal_ready(signature: CapabilityCallSignature) -> Self {
        // Kept so tests and older checkpoint producers can exercise the legacy
        // wire value. Runtime observation normalizes it back to `Rendered`.
        Self {
            signature,
            phase: RepeatedCallWarningPhase::TerminalReady,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatedCallWarningPhase {
    PendingRender,
    Rendered,
    /// Legacy checkpoint value. New runtime policy never creates this phase.
    TerminalReady,
}
