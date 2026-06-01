use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContextStrategyState {}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CapabilityStrategyState {}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelStrategyState {
    pub fallback_index: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CompactionStrategyState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_compacted_through_seq: Option<u64>,
    #[serde(default)]
    pub force_compact_on_next_iteration: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CompactionPromptSnapshot {
    pub message_index: Vec<MessageIndexEntry>,
    pub observed_prompt_tokens: u64,
}

impl CompactionPromptSnapshot {
    pub fn from_message_index(message_index: Vec<MessageIndexEntry>) -> Self {
        let observed_prompt_tokens = message_index
            .iter()
            .map(|entry| entry.estimated_tokens)
            .sum();
        Self {
            message_index,
            observed_prompt_tokens,
        }
    }

    pub fn retain_after_sequence(&mut self, sequence: u64) {
        let mut removed_tokens = 0_u64;
        self.message_index.retain(|entry| {
            let keep = entry.sequence > sequence;
            if !keep {
                removed_tokens = removed_tokens.saturating_add(entry.estimated_tokens);
            }
            keep
        });
        self.observed_prompt_tokens = self.observed_prompt_tokens.saturating_sub(removed_tokens);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MessageIndexEntry {
    pub sequence: u64,
    pub kind: IndexedMessageKind,
    pub estimated_tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexedMessageKind {
    User,
    Assistant,
    System,
    Summary,
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(sequence: u64, estimated_tokens: u64) -> MessageIndexEntry {
        MessageIndexEntry {
            sequence,
            kind: IndexedMessageKind::User,
            estimated_tokens,
        }
    }

    #[test]
    fn retain_after_sequence_keeps_empty_snapshot_empty() {
        let mut snapshot = CompactionPromptSnapshot::default();

        snapshot.retain_after_sequence(1);

        assert!(snapshot.message_index.is_empty());
        assert_eq!(snapshot.observed_prompt_tokens, 0);
    }

    #[test]
    fn retain_after_sequence_can_retain_no_entries() {
        let mut snapshot = CompactionPromptSnapshot::from_message_index(vec![entry(1, 10)]);

        snapshot.retain_after_sequence(1);

        assert!(snapshot.message_index.is_empty());
        assert_eq!(snapshot.observed_prompt_tokens, 0);
    }

    #[test]
    fn retain_after_sequence_can_retain_all_entries() {
        let mut snapshot =
            CompactionPromptSnapshot::from_message_index(vec![entry(1, 10), entry(2, 20)]);

        snapshot.retain_after_sequence(0);

        assert_eq!(snapshot.message_index, vec![entry(1, 10), entry(2, 20)]);
        assert_eq!(snapshot.observed_prompt_tokens, 30);
    }

    #[test]
    fn retain_after_sequence_updates_tokens_for_partial_retention() {
        let mut snapshot = CompactionPromptSnapshot::from_message_index(vec![
            entry(1, 10),
            entry(2, 20),
            entry(3, 30),
        ]);

        snapshot.retain_after_sequence(1);

        assert_eq!(snapshot.message_index, vec![entry(2, 20), entry(3, 30)]);
        assert_eq!(snapshot.observed_prompt_tokens, 50);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GoalRefreshStrategyState {
    #[serde(default)]
    pub turns_since_refresh: u32,
}

/// Per-error-class attempt counters for the recovery strategy.
///
/// Semantics: the retry budget is *not* durable across resume — on rehydration
/// from a `BeforeSideEffect` checkpoint, counters reset to 0 so a fresh
/// retry budget is granted post-resume.
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
    /// the next retry/abort decision sees the updated attempt count.
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

/// Persistent state owned by `ReplyAdmissionStrategy`.
///
/// Rejected replies are loop-private candidates. The latest rejection is kept
/// until an accepted final reply clears it so checkpoints can resume from the
/// typed control state, while `pending_rejection_rendered` prevents repeating
/// the same control event every prompt.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReplyAdmissionStrategyState {
    #[serde(default)]
    pub rejected_reply_candidates: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_rejection: Option<ReplyAdmissionRejection>,
    #[serde(default)]
    pub pending_rejection_rendered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReplyAdmissionRejection {
    pub reason_code: ReplyAdmissionRejectionReason,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unmet_obligation_refs: Vec<ObligationRef>,
}

impl ReplyAdmissionRejection {
    pub fn stop_condition_not_met() -> Self {
        Self {
            reason_code: ReplyAdmissionRejectionReason::StopConditionNotMet,
            unmet_obligation_refs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ObligationRef(String);

impl ObligationRef {
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        if value.is_empty() {
            None
        } else {
            Some(Self(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyAdmissionRejectionReason {
    StopConditionNotMet,
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
    /// Consecutive turns where a model reply was rejected before transcript
    /// finalization.
    #[serde(default)]
    pub trailing_rejected_replies: u32,
}

/// Persistent state owned by `GateHandlingStrategy`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GateStrategyState {}
