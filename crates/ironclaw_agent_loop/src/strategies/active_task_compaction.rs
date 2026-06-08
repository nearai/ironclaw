use crate::state::{IndexedMessageKind, LoopExecutionState, MessageIndexEntry};
use ironclaw_turns::run_profile::LoopRunContext;

use super::compaction::{
    CompactionDecision, CompactionStrategy, DefaultCompactionStrategy, is_eligible_user_boundary,
};

/// Compaction policy for Reborn runs that must preserve the live active task.
///
/// The latest user message stays in the prompt tail so the next model turn can
/// answer the current request directly. Older user boundaries are still
/// compactable once enough prefix and tail context remains outside the summary.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ActiveTaskPreservingCompactionStrategy {
    pub base: DefaultCompactionStrategy,
    pub minimum_compacted_messages: usize,
    pub minimum_tail_messages: usize,
}

impl Default for ActiveTaskPreservingCompactionStrategy {
    fn default() -> Self {
        Self::from(DefaultCompactionStrategy::default())
    }
}

impl From<DefaultCompactionStrategy> for ActiveTaskPreservingCompactionStrategy {
    fn from(base: DefaultCompactionStrategy) -> Self {
        Self {
            base,
            minimum_compacted_messages: Self::DEFAULT_MINIMUM_COMPACTED_MESSAGES,
            minimum_tail_messages: Self::DEFAULT_MINIMUM_TAIL_MESSAGES,
        }
    }
}

impl ActiveTaskPreservingCompactionStrategy {
    pub const DEFAULT_MINIMUM_COMPACTED_MESSAGES: usize = 3;
    pub const DEFAULT_MINIMUM_TAIL_MESSAGES: usize = 3;
}

impl CompactionStrategy for ActiveTaskPreservingCompactionStrategy {
    fn should_compact(
        &self,
        state: &LoopExecutionState,
        _ctx: &LoopRunContext,
    ) -> CompactionDecision {
        if !self.base.can_evaluate(state) {
            return CompactionDecision::Skip;
        }

        let prompt_fingerprint = state.compaction_prompt.fingerprint();
        active_task_preserving_user_boundary(
            state,
            prompt_fingerprint,
            self.base.preserve_tail_tokens,
            self.minimum_tail_messages,
            self.minimum_compacted_messages,
        )
        .map(|sequence| self.base.trigger_at(sequence))
        .unwrap_or(CompactionDecision::Skip)
    }
}

fn active_task_preserving_user_boundary(
    state: &LoopExecutionState,
    prompt_fingerprint: u64,
    preserve_tail_tokens: u64,
    minimum_tail_messages: usize,
    minimum_compacted_messages: usize,
) -> Option<u64> {
    let mut tail_tokens = 0_u64;
    let mut tail_messages = 0_usize;
    let mut latest_user_seen = false;
    let mut candidate_sequence = None;
    let mut compacted_prefix_messages = 0_usize;

    for entry in state.compaction_prompt.message_index.iter().rev() {
        let is_latest_user = entry.kind == IndexedMessageKind::User && !latest_user_seen;
        if entry.kind == IndexedMessageKind::User {
            latest_user_seen = true;
        }

        if candidate_sequence.is_none()
            && tail_tokens >= preserve_tail_tokens
            && tail_messages >= minimum_tail_messages
            && !is_latest_user
            && is_eligible_user_boundary(entry, state, prompt_fingerprint)
        {
            candidate_sequence = Some(entry.sequence);
        }

        if candidate_sequence.is_some() && is_compaction_prefix_message(entry) {
            compacted_prefix_messages = compacted_prefix_messages.saturating_add(1);
        }

        tail_tokens = tail_tokens.saturating_add(entry.estimated_tokens);
        tail_messages = tail_messages.saturating_add(1);
    }

    candidate_sequence.filter(|_| compacted_prefix_messages >= minimum_compacted_messages)
}

fn is_compaction_prefix_message(entry: &MessageIndexEntry) -> bool {
    !matches!(
        entry.kind,
        IndexedMessageKind::System | IndexedMessageKind::Summary
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{CompactionPromptSnapshot, LoopExecutionState, MessageIndexEntry};

    fn active_task_preserving_strategy(
        preserve_tail_tokens: u64,
    ) -> ActiveTaskPreservingCompactionStrategy {
        ActiveTaskPreservingCompactionStrategy::from(DefaultCompactionStrategy {
            context_limit_tokens: 100,
            reserve_tokens: 10,
            main_loop_max_output_tokens: 0,
            preserve_tail_tokens,
            deadline_ms: 7,
        })
    }

    fn active_task_message_index() -> Vec<MessageIndexEntry> {
        vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 3,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 4,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 5,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 6,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 7,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 8,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 10,
            },
        ]
    }

    #[test]
    fn forced_compaction_does_not_drop_latest_user_message() {
        let context = crate::test_support::test_run_context("active-task-preserving-forced");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state.force_compact_on_next_iteration = true;
        state.compaction_prompt =
            CompactionPromptSnapshot::from_message_index(active_task_message_index());
        let strategy = active_task_preserving_strategy(1);

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Trigger {
                drop_through_seq: 5,
                preserve_tail_tokens: 1,
                deadline_ms: 7,
            }
        );
    }

    #[test]
    fn forced_compaction_skips_when_only_latest_user_is_safe_candidate() {
        let context = crate::test_support::test_run_context("active-task-preserving-only-latest");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state.force_compact_on_next_iteration = true;
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
        ]);
        let strategy = active_task_preserving_strategy(1);

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Skip
        );
    }

    #[test]
    fn forced_compaction_still_respects_tail_budget() {
        let context = crate::test_support::test_run_context("active-task-preserving-tail-budget");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state.force_compact_on_next_iteration = true;
        state.compaction_prompt =
            CompactionPromptSnapshot::from_message_index(active_task_message_index());
        let strategy = active_task_preserving_strategy(60);

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Skip
        );
    }

    #[test]
    fn threshold_driven_compaction_triggers_without_force() {
        let context = crate::test_support::test_run_context("active-task-preserving-threshold");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_prompt =
            CompactionPromptSnapshot::from_message_index(active_task_message_index());
        state.compaction_prompt.observed_prompt_tokens = 90;
        let strategy = active_task_preserving_strategy(1);

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Trigger {
                drop_through_seq: 5,
                preserve_tail_tokens: 1,
                deadline_ms: 7,
            }
        );
    }

    #[test]
    fn compaction_skips_when_index_shorter_than_minimum_compacted_messages() {
        let context = crate::test_support::test_run_context("active-task-preserving-short-prefix");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state.force_compact_on_next_iteration = true;
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
        ]);
        let mut strategy = active_task_preserving_strategy(0);
        strategy.minimum_tail_messages = 0;
        strategy.minimum_compacted_messages = 3;

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Skip
        );
    }

    #[test]
    fn compaction_skips_when_minimum_tail_messages_not_met() {
        let context = crate::test_support::test_run_context("active-task-preserving-tail-messages");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state.force_compact_on_next_iteration = true;
        state.compaction_prompt =
            CompactionPromptSnapshot::from_message_index(active_task_message_index());
        let mut strategy = active_task_preserving_strategy(0);
        strategy.minimum_compacted_messages = 0;
        strategy.minimum_tail_messages = active_task_message_index().len() + 1;

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Skip
        );
    }
}
