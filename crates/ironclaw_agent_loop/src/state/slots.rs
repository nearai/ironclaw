use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContextStrategyState {}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CapabilityStrategyState {}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelStrategyState {
    pub fallback_index: u32,
}

/// Per-error-class attempt counters for the recovery strategy.
///
/// Semantics: the retry budget is *not* durable across resume — on rehydration
/// from a `BeforeSideEffect` checkpoint, counters reset to 0 so a fresh
/// retry budget is granted post-resume. See master doc §10 for the
/// retry-budget durability note.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecoveryStrategyState {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attempts_by_class: BTreeMap<RecoveryAttemptClass, u32>,
}

impl RecoveryStrategyState {
    /// Returns the attempt count already consumed for `class`.
    pub fn attempts_for(&self, class: RecoveryAttemptClass) -> u32 {
        self.attempts_by_class.get(&class).copied().unwrap_or(0)
    }

    /// Returns a new slot value with the attempt count for `class`
    /// incremented by one (saturating at `u32::MAX`).
    ///
    /// Used by `DefaultRecoveryStrategy` when classifying a fresh error so
    /// the next retry/abort decision sees the updated attempt count. See
    /// `docs/reborn/agent-loop-skeleton.md` §6 ("RecoveryStrategy") and §10
    /// ("Production-safe escape" — per-error retry budget).
    pub fn with_incremented_attempts_for(&self, class: RecoveryAttemptClass) -> Self {
        let mut attempts_by_class = self.attempts_by_class.clone();
        attempts_by_class.insert(class, self.attempts_for(class).saturating_add(1));
        Self { attempts_by_class }
    }

    pub fn with_attempts_for(class: RecoveryAttemptClass, attempts: u32) -> Self {
        let mut attempts_by_class = BTreeMap::new();
        attempts_by_class.insert(class, attempts);
        Self { attempts_by_class }
    }

    /// Clears retry accounting after a terminal or non-retry decision so it
    /// cannot poison an unrelated later retryable error.
    pub fn cleared_attempts(&self) -> Self {
        Self::default()
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAttemptClass {
    CapabilityTransient,
    CapabilityUnavailable,
    CapabilityInternal,
    ModelTransient,
    ModelContextOverflow,
    ModelUnavailable,
    ModelInternal,
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
