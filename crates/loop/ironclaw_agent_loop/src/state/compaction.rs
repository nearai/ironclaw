use ironclaw_loop_contracts::{CompactionInitiator, LoopContextWindowTruncation};

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
    /// Exact durable boundary omitted by the bounded recent-message window.
    /// This watermark triggers selection of the newest safe compaction cut
    /// point in the retained prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_eviction: Option<LoopContextWindowTruncation>,
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
        IndexedMessageKind::ToolResult => 3,
        IndexedMessageKind::System => 4,
        IndexedMessageKind::Summary => 5,
        IndexedMessageKind::Other => 6,
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
    ToolResult,
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
        for expected in 1..CompactionStrategyState::INEFFECTIVE_COMPACTION_TRIP_LIMIT {
            state = state.with_compaction_effectiveness_observed(100, threshold_baseline(90));
            assert_eq!(state.consecutive_ineffective_compactions, expected);
            assert!(!state.compaction_circuit_open);
        }
        state = state.with_compaction_effectiveness_observed(90, threshold_baseline(90));
        assert_eq!(
            state.consecutive_ineffective_compactions,
            CompactionStrategyState::INEFFECTIVE_COMPACTION_TRIP_LIMIT
        );
        assert!(state.compaction_circuit_open);
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
