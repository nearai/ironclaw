use crate::state::{IndexedMessageKind, LoopExecutionState, MessageIndexEntry};
use ironclaw_turns::run_profile::LoopRunContext;

use super::compaction::{
    CompactionDecision, CompactionStrategy, DefaultCompactionStrategy,
    tail_preserving_user_boundary,
};

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
        let guard = ActiveTaskBoundaryGuard::new(
            &state.compaction_prompt.message_index,
            self.minimum_compacted_messages,
        );
        tail_preserving_user_boundary(
            state,
            prompt_fingerprint,
            self.base.preserve_tail_tokens,
            self.minimum_tail_messages,
            |entry| guard.allows(entry),
        )
        .map(|sequence| self.base.trigger_at(sequence))
        .unwrap_or(CompactionDecision::Skip)
    }
}

#[derive(Debug, Clone, Copy)]
struct ActiveTaskBoundaryGuard {
    latest_user_sequence: Option<u64>,
    minimum_boundary_after_seq: Option<u64>,
}

impl ActiveTaskBoundaryGuard {
    fn new(message_index: &[MessageIndexEntry], minimum_compacted_messages: usize) -> Self {
        Self {
            latest_user_sequence: latest_user_sequence(message_index),
            minimum_boundary_after_seq: minimum_boundary_after_sequence(
                message_index,
                minimum_compacted_messages,
            ),
        }
    }

    fn allows(&self, entry: &MessageIndexEntry) -> bool {
        Some(entry.sequence) != self.latest_user_sequence
            && is_after_minimum_prefix(entry, self.minimum_boundary_after_seq)
    }
}

fn latest_user_sequence(message_index: &[MessageIndexEntry]) -> Option<u64> {
    message_index
        .iter()
        .rev()
        .find(|entry| entry.kind == IndexedMessageKind::User)
        .map(|entry| entry.sequence)
}

fn minimum_boundary_after_sequence(
    message_index: &[MessageIndexEntry],
    minimum_compacted_messages: usize,
) -> Option<u64> {
    if minimum_compacted_messages == 0 {
        return None;
    }
    message_index
        .iter()
        .filter(|entry| {
            !matches!(
                entry.kind,
                IndexedMessageKind::System | IndexedMessageKind::Summary
            )
        })
        .take(minimum_compacted_messages)
        .last()
        .map(|entry| entry.sequence)
}

fn is_after_minimum_prefix(
    entry: &MessageIndexEntry,
    minimum_boundary_after_seq: Option<u64>,
) -> bool {
    minimum_boundary_after_seq.is_none_or(|sequence| entry.sequence > sequence)
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
}
