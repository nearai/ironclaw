use std::collections::BTreeMap;

use ironclaw_host_api::CapabilityId;
use ironclaw_turns::run_profile::CompactionInitiator;

use super::CapabilityCallSignature;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_deferred: Option<DeferredCompactionWatermark>,
    #[serde(default)]
    pub force_compact_on_next_iteration: bool,
    /// Initiator to emit on the NEXT iteration's `CompactionStarted` event
    /// when `force_compact_on_next_iteration` causes the compactor to run.
    /// Set by `PostCapabilityStage` when its policy trips; consumed
    /// (.take()) by `PromptCompactionStep` so the event has the
    /// proximate-cause initiator (e.g. `CapabilityResultOverflow`)
    /// instead of falling back to `Auto`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_compact_initiator: Option<CompactionInitiator>,
    /// Consecutive completed compactions whose refreshed prompt token
    /// estimate stayed at or above their effectiveness baseline —
    /// compaction ran but did not relieve context pressure, so it would fire
    /// again immediately. Reset to zero by any effective compaction.
    #[serde(default)]
    pub consecutive_ineffective_compactions: u32,
    /// One-way circuit breaker: opened by `PromptCompactionStep` after
    /// [`Self::INEFFECTIVE_COMPACTION_TRIP_LIMIT`] consecutive ineffective
    /// compactions. While open, compaction strategies skip *automatic
    /// threshold-triggered* compaction for the remainder of the run so a
    /// doomed compact-recompact loop cannot keep burning summarization
    /// inference on a prompt it can never shrink. Forced/recovery
    /// compactions (`force_compact_on_next_iteration` — context-overflow
    /// retry, byte-cap overflow) bypass the breaker: they are the loop's
    /// only way to shrink an oversized prompt before a retry.
    #[serde(default)]
    pub compaction_circuit_open: bool,
    /// Effectiveness baseline for a compaction that completed but whose
    /// post-compaction prompt (including the injected summary) has not been
    /// rebuilt yet. Set by `PromptCompactionStep` when a compaction
    /// completes; consumed by the executor once the prompt bundle is next
    /// rebuilt and `compaction_prompt.observed_prompt_tokens` reflects the
    /// summary, at which point the effectiveness comparison runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_effectiveness_baseline: Option<CompactionEffectivenessBaseline>,
}

impl CompactionStrategyState {
    /// Consecutive ineffective compactions tolerated before the circuit opens.
    pub const INEFFECTIVE_COMPACTION_TRIP_LIMIT: u32 = 3;

    /// Returns a new slot value with one completed compaction's effectiveness
    /// recorded.
    ///
    /// `post_compaction_prompt_tokens` is the executor's refreshed prompt
    /// token estimate after the compaction (summary included); `baseline` is
    /// the trigger-kind-specific yardstick stamped on the compaction
    /// decision. Staying at or above the baseline counts as ineffective;
    /// dropping below it resets the consecutive counter. Once the circuit
    /// opens it never closes for the rest of the run.
    pub fn with_compaction_effectiveness_observed(
        &self,
        post_compaction_prompt_tokens: u64,
        baseline: CompactionEffectivenessBaseline,
    ) -> Self {
        let mut next = self.clone();
        if post_compaction_prompt_tokens >= baseline.tokens() {
            next.consecutive_ineffective_compactions =
                self.consecutive_ineffective_compactions.saturating_add(1);
            if next.consecutive_ineffective_compactions >= Self::INEFFECTIVE_COMPACTION_TRIP_LIMIT {
                next.compaction_circuit_open = true;
            }
        } else {
            next.consecutive_ineffective_compactions = 0;
        }
        next
    }
}

/// Trigger-kind-specific yardstick for judging whether a completed compaction
/// relieved context pressure.
///
/// A threshold-triggered compaction is only useful if it brings the prompt
/// back below the trigger threshold. A forced compaction (context-overflow
/// recovery, byte-cap overflow) is triggered independently of the transcript
/// threshold, so it is judged against the prompt size it started from: it
/// helped iff the prompt actually shrank.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionEffectivenessBaseline {
    /// Automatic threshold-triggered compaction: effective iff the refreshed
    /// post-compaction prompt estimate drops below the visible-transcript
    /// threshold that triggered it.
    TriggerThresholdTokens { tokens: u64 },
    /// Forced compaction: effective iff the refreshed post-compaction prompt
    /// estimate drops below the pre-compaction prompt estimate.
    PreCompactionPromptTokens { tokens: u64 },
}

impl CompactionEffectivenessBaseline {
    /// Token count the refreshed post-compaction prompt is compared against;
    /// staying at or above it marks the compaction ineffective.
    pub fn tokens(&self) -> u64 {
        match self {
            Self::TriggerThresholdTokens { tokens }
            | Self::PreCompactionPromptTokens { tokens } => *tokens,
        }
    }
}

/// Records the deferred cut point and prompt snapshot fingerprint for a
/// compaction attempt that should not be retried against the same prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeferredCompactionWatermark {
    pub through_seq: u64,
    pub prompt_fingerprint: u64,
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

    pub fn fingerprint(&self) -> u64 {
        let mut fingerprint = 0xcbf2_9ce4_8422_2325_u64;
        fingerprint = mix_fingerprint(fingerprint, self.observed_prompt_tokens);
        fingerprint = mix_fingerprint(fingerprint, self.message_index.len() as u64);
        for entry in &self.message_index {
            fingerprint = mix_fingerprint(fingerprint, entry.sequence);
            fingerprint = mix_fingerprint(fingerprint, indexed_message_kind_code(entry.kind));
            fingerprint = mix_fingerprint(fingerprint, entry.estimated_tokens);
        }
        fingerprint
    }
}

fn mix_fingerprint(current: u64, value: u64) -> u64 {
    current
        .wrapping_mul(0x0000_0100_0000_01b3)
        .wrapping_add(value)
}

fn indexed_message_kind_code(kind: IndexedMessageKind) -> u64 {
    match kind {
        IndexedMessageKind::User => 1,
        IndexedMessageKind::Assistant => 2,
        IndexedMessageKind::System => 3,
        IndexedMessageKind::Summary => 4,
        IndexedMessageKind::Other => 5,
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

    #[test]
    fn fingerprint_is_stable_for_empty_and_identical_snapshots() {
        let empty = CompactionPromptSnapshot::default();
        assert_ne!(empty.fingerprint(), 0);
        assert_eq!(
            empty.fingerprint(),
            CompactionPromptSnapshot::default().fingerprint()
        );

        let first = CompactionPromptSnapshot::from_message_index(vec![entry(1, 10), entry(2, 20)]);
        let second = CompactionPromptSnapshot::from_message_index(vec![entry(1, 10), entry(2, 20)]);

        assert_eq!(first.fingerprint(), second.fingerprint());
    }

    fn threshold_baseline(tokens: u64) -> CompactionEffectivenessBaseline {
        CompactionEffectivenessBaseline::TriggerThresholdTokens { tokens }
    }

    #[test]
    fn compaction_effectiveness_trips_circuit_after_consecutive_ineffective_runs() {
        let mut state = CompactionStrategyState::default();

        // Two ineffective compactions: counter grows, circuit stays closed.
        for expected in 1..CompactionStrategyState::INEFFECTIVE_COMPACTION_TRIP_LIMIT {
            state = state.with_compaction_effectiveness_observed(100, threshold_baseline(90));
            assert_eq!(state.consecutive_ineffective_compactions, expected);
            assert!(!state.compaction_circuit_open);
        }

        // Third consecutive ineffective compaction opens the circuit.
        state = state.with_compaction_effectiveness_observed(90, threshold_baseline(90));
        assert_eq!(
            state.consecutive_ineffective_compactions,
            CompactionStrategyState::INEFFECTIVE_COMPACTION_TRIP_LIMIT
        );
        assert!(state.compaction_circuit_open);

        // The circuit never closes, even after an effective compaction.
        state = state.with_compaction_effectiveness_observed(10, threshold_baseline(90));
        assert_eq!(state.consecutive_ineffective_compactions, 0);
        assert!(state.compaction_circuit_open);
    }

    #[test]
    fn compaction_effectiveness_resets_counter_when_tokens_drop_below_threshold() {
        let state = CompactionStrategyState::default()
            .with_compaction_effectiveness_observed(100, threshold_baseline(90))
            .with_compaction_effectiveness_observed(100, threshold_baseline(90))
            .with_compaction_effectiveness_observed(89, threshold_baseline(90));

        assert_eq!(state.consecutive_ineffective_compactions, 0);
        assert!(!state.compaction_circuit_open);
    }

    #[test]
    fn forced_compaction_effectiveness_judges_against_pre_compaction_prompt() {
        // BUG B3 regression: a forced compaction is judged against the prompt
        // size it started from, not the unrelated transcript threshold. A
        // prompt that shrank (140 < 260) is effective even though it would
        // still be over a 90-token threshold; a prompt that did not shrink is
        // ineffective even if it sits below that threshold.
        let shrank = CompactionStrategyState::default()
            .with_compaction_effectiveness_observed(100, threshold_baseline(90))
            .with_compaction_effectiveness_observed(
                140,
                CompactionEffectivenessBaseline::PreCompactionPromptTokens { tokens: 260 },
            );
        assert_eq!(shrank.consecutive_ineffective_compactions, 0);

        let did_not_shrink = CompactionStrategyState::default()
            .with_compaction_effectiveness_observed(
                80,
                CompactionEffectivenessBaseline::PreCompactionPromptTokens { tokens: 80 },
            );
        assert_eq!(did_not_shrink.consecutive_ineffective_compactions, 1);
    }

    #[test]
    fn compaction_state_deserializes_without_circuit_breaker_fields() {
        // Pre-breaker checkpoints must rehydrate with the breaker closed.
        let state: CompactionStrategyState =
            serde_json::from_str(r#"{"force_compact_on_next_iteration":true}"#).expect("decode");

        assert!(state.force_compact_on_next_iteration);
        assert_eq!(state.consecutive_ineffective_compactions, 0);
        assert!(!state.compaction_circuit_open);
        assert_eq!(state.pending_effectiveness_baseline, None);
    }

    #[test]
    fn fingerprint_changes_when_order_or_entries_change() {
        let baseline =
            CompactionPromptSnapshot::from_message_index(vec![entry(1, 10), entry(2, 20)]);
        let reordered =
            CompactionPromptSnapshot::from_message_index(vec![entry(2, 20), entry(1, 10)]);
        let added = CompactionPromptSnapshot::from_message_index(vec![
            entry(1, 10),
            entry(2, 20),
            entry(3, 30),
        ]);
        let changed_tokens =
            CompactionPromptSnapshot::from_message_index(vec![entry(1, 10), entry(2, 21)]);

        assert_ne!(baseline.fingerprint(), reordered.fingerprint());
        assert_ne!(baseline.fingerprint(), added.fingerprint());
        assert_ne!(baseline.fingerprint(), changed_tokens.fingerprint());
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
    ModelInvalidOutput,
    ModelUnavailable,
    ModelInternal,
    ModelStaleRequest,
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
    /// Consecutive turns where a model reply was rejected before transcript
    /// finalization.
    #[serde(default)]
    pub trailing_rejected_replies: u32,
    /// Consecutive completed capability-batch turns whose typed result
    /// progress reported no new evidence/state.
    #[serde(default)]
    pub trailing_no_progress_results: u32,
    /// Pending or rendered repeated-call warning that must be shown to the
    /// model before repeated calls can terminalize as no-progress.
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
    TerminalReady,
}

/// Persistent state owned by `GateHandlingStrategy`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GateStrategyState {}

/// Per-turn pipeline-directive state for `PostCapabilityStage`.
///
/// Unlike sibling `<Domain>StrategyState` types, this slot belongs to a
/// pipeline stage (not a strategy) and tracks two distinct lifecycles:
///
/// - `pending_capability_bytes` is **per-turn**: filled by
///   `push_completed_result` during a capability batch, cleared at the
///   end of every `PostCapabilityStage::process` call (BUG-N1 fix).
/// - `skip_model_this_iteration` is a **one-shot directive**: set by
///   `PostCapabilityStage` when its policy trips, then consumed by the
///   NEXT iteration's `PromptStage` which clears the flag and emits
///   `PromptStep::SkipModel` to short-circuit the model call.
///
/// The distinct naming (`StageState` vs `StrategyState`) marks the
/// category difference: stages own transient one-shot directives;
/// strategies own resumable accounting.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PostCapabilityStageState {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pending_capability_bytes: BTreeMap<CapabilityId, u64>,
    #[serde(default)]
    pub skip_model_this_iteration: bool,
}
