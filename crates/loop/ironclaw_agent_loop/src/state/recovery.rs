use std::collections::{BTreeMap, BTreeSet};

/// Per-error-class attempt counters for the recovery strategy.
///
/// Retry counters and one-shot observation attempts are serialized into
/// checkpoints so resume/rebase cannot silently grant an unbounded fresh
/// recovery budget. Successful and ordinary non-retry decisions clear the
/// slot; retry-exhaustion terminal checkpoints retain it as evidence of
/// consumed attempts.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecoveryStrategyState {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attempts_by_class: BTreeMap<RecoveryAttemptClass, u32>,
    /// Model-error classes that have already received their one
    /// observation-assisted repair attempt. This bound prevents a recovery
    /// observation from turning exhaustion into an infinite retry loop.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub observation_attempted_by_class: BTreeSet<ModelErrorObservationClass>,
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
    /// the next retry/abort decision sees the updated attempt count.
    pub fn with_incremented_attempts_for(&self, class: RecoveryAttemptClass) -> Self {
        let mut attempts_by_class = self.attempts_by_class.clone();
        attempts_by_class.insert(class, self.attempts_for(class).saturating_add(1));
        Self {
            attempts_by_class,
            observation_attempted_by_class: self.observation_attempted_by_class.clone(),
        }
    }

    pub fn with_attempts_for(class: RecoveryAttemptClass, attempts: u32) -> Self {
        let mut attempts_by_class = BTreeMap::new();
        attempts_by_class.insert(class, attempts);
        Self {
            attempts_by_class,
            observation_attempted_by_class: BTreeSet::new(),
        }
    }

    pub fn observation_attempted_for(&self, class: ModelErrorObservationClass) -> bool {
        self.observation_attempted_by_class.contains(&class)
    }

    pub fn with_observation_attempted_for(&self, class: ModelErrorObservationClass) -> Self {
        let mut next = self.clone();
        next.observation_attempted_by_class.insert(class);
        next
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
    ModelInvalidOutput,
    ModelUnavailable,
    ModelInternal,
    ModelStaleRequest,
}

/// Model-error classes eligible for one observation-assisted repair attempt.
///
/// This is deliberately separate from [`RecoveryAttemptClass`]: it is stored
/// only in the newly defaulted `observation_attempted_by_class` field, so an
/// older binary can ignore the whole field after rollback without encountering
/// an unknown key in the pre-existing `attempts_by_class` map.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ModelErrorObservationClass {
    ContextOverflow,
    ContentFiltered,
    InvalidOutput,
    Transient,
    Unavailable,
    Internal,
    StaleRequest,
    OutputTruncated,
}
