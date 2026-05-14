#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContextStrategyState {}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CapabilityStrategyState {}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelStrategyState {
    pub fallback_index: u32,
}

/// Per-error-class attempt counter for the recovery strategy.
///
/// Semantics: the retry budget is *not* durable across resume — on rehydration
/// from a `BeforeSideEffect` checkpoint, `attempts` resets to 0 so a fresh
/// retry budget is granted post-resume. See master doc §10 for the
/// retry-budget durability note. WS-2 may grow this into a
/// `HashMap<LoopFailureKind, u32>` when `DefaultRecoveryStrategy` needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecoveryStrategyState {
    pub attempts: u32,
}

impl RecoveryStrategyState {
    /// Returns a new slot value with `attempts` incremented by one
    /// (saturating at `u32::MAX`).
    ///
    /// Used by `DefaultRecoveryStrategy` when classifying a fresh error so
    /// the next retry/abort decision sees the updated attempt count. See
    /// `docs/reborn/agent-loop-skeleton.md` §6 ("RecoveryStrategy") and §10
    /// ("Production-safe escape" — per-error retry budget).
    pub fn with_incremented_attempts(&self) -> Self {
        Self {
            attempts: self.attempts.saturating_add(1),
        }
    }
}

/// Persistent state owned by `StopConditionStrategy`. Split from a previously
/// shared `ControlStrategyState` so Stop and Gate evolve independently — a
/// future family's growth in stop-condition state cannot perturb gate-handler
/// invariants and vice versa.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StopStrategyState {
    /// Number of completed turns the StopConditionStrategy has observed.
    pub turns_completed: u32,
    /// Count of `terminate: true` hints seen in the most recent capability batch.
    /// Reset to 0 at the start of each batch.
    pub terminate_hints_in_last_batch: u32,
    /// Total number of results in the most recent capability batch (denominator
    /// for "all results said terminate").
    pub last_batch_total: u32,
}

/// Persistent state owned by `GateHandlingStrategy`. Empty in the skeleton;
/// future families may track gate fingerprints (for resume correlation),
/// per-gate-kind counters, or other gate-relevant bookkeeping here without
/// touching Stop-strategy state.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GateStrategyState {
    // skeleton: empty. WS-2 may extend when DefaultGateHandlingStrategy needs it.
}
