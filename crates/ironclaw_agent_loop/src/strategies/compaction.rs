use crate::state::{IndexedMessageKind, LoopExecutionState};
use ironclaw_turns::run_profile::LoopRunContext;

/// Decides whether to replace older transcript context with a host-managed summary.
///
/// The strategy is pure policy: it reads durable compaction state and returns
/// either `Skip` or the inclusive user-message boundary the executor should
/// compact through. State mutation, transcript reads, inference, persistence,
/// and progress events stay in the executor and host compaction port.
///
/// `Trigger.drop_through_seq` must point at a model-visible user message. The
/// host compaction port rejects non-user terminal boundaries so custom
/// strategies cannot compact through assistant, summary, or reference messages.
pub(crate) trait CompactionStrategy: Send + Sync {
    fn should_compact(
        &self,
        state: &LoopExecutionState,
        ctx: &LoopRunContext,
    ) -> CompactionDecision;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompactionDecision {
    Skip,
    Trigger {
        drop_through_seq: u64,
        preserve_tail_tokens: u64,
        deadline_ms: u64,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DefaultCompactionStrategy {
    pub context_limit_tokens: u64,
    pub reserve_tokens: u64,
    pub main_loop_max_output_tokens: u64,
    pub preserve_tail_tokens: u64,
    pub deadline_ms: u64,
}

impl DefaultCompactionStrategy {
    pub const DEFAULT_CONTEXT_LIMIT_TOKENS: u64 = 128_000;
    pub const DEFAULT_RESERVE_TOKENS: u64 = 20_000;
    pub const DEFAULT_MAIN_LOOP_MAX_OUTPUT_TOKENS: u64 = 0;
    pub const DEFAULT_PRESERVE_TAIL_TOKENS: u64 = 8_000;
    pub const DEFAULT_DEADLINE_MS: u64 = 30_000;
}

impl Default for DefaultCompactionStrategy {
    fn default() -> Self {
        Self {
            context_limit_tokens: Self::DEFAULT_CONTEXT_LIMIT_TOKENS,
            reserve_tokens: Self::DEFAULT_RESERVE_TOKENS,
            main_loop_max_output_tokens: Self::DEFAULT_MAIN_LOOP_MAX_OUTPUT_TOKENS,
            preserve_tail_tokens: Self::DEFAULT_PRESERVE_TAIL_TOKENS,
            deadline_ms: Self::DEFAULT_DEADLINE_MS,
        }
    }
}

impl CompactionStrategy for DefaultCompactionStrategy {
    fn should_compact(
        &self,
        state: &LoopExecutionState,
        _ctx: &LoopRunContext,
    ) -> CompactionDecision {
        if state.compaction_prompt.message_index.is_empty() {
            return CompactionDecision::Skip;
        }
        let output_buffer = self.reserve_tokens.max(self.main_loop_max_output_tokens);
        let threshold = self.context_limit_tokens.saturating_sub(output_buffer);
        if threshold <= self.preserve_tail_tokens {
            return CompactionDecision::Skip;
        }
        let total_tokens = state.compaction_prompt.observed_prompt_tokens;
        if !state.compaction_state.force_compact_on_next_iteration && total_tokens < threshold {
            return CompactionDecision::Skip;
        }
        if state.compaction_state.force_compact_on_next_iteration {
            return state
                .compaction_prompt
                .message_index
                .iter()
                .rev()
                .find(|entry| {
                    entry.kind == IndexedMessageKind::User
                        && Some(entry.sequence) > state.compaction_state.last_compacted_through_seq
                })
                .map(|entry| CompactionDecision::Trigger {
                    drop_through_seq: entry.sequence,
                    preserve_tail_tokens: self.preserve_tail_tokens,
                    deadline_ms: self.deadline_ms,
                })
                .unwrap_or(CompactionDecision::Skip);
        }

        let mut tail_tokens = 0_u64;
        for entry in state.compaction_prompt.message_index.iter().rev() {
            if entry.kind == IndexedMessageKind::User
                && Some(entry.sequence) > state.compaction_state.last_compacted_through_seq
                && tail_tokens >= self.preserve_tail_tokens
            {
                return CompactionDecision::Trigger {
                    drop_through_seq: entry.sequence,
                    preserve_tail_tokens: self.preserve_tail_tokens,
                    deadline_ms: self.deadline_ms,
                };
            }
            tail_tokens = tail_tokens.saturating_add(entry.estimated_tokens);
        }
        CompactionDecision::Skip
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{
        CompactionPromptSnapshot, CompactionStrategyState, LoopExecutionState, MessageIndexEntry,
    };

    #[test]
    fn evaluate_skips_when_message_index_is_empty() {
        let context = crate::test_support::test_run_context("compaction-strategy-empty");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state.force_compact_on_next_iteration = true;
        let strategy = DefaultCompactionStrategy {
            context_limit_tokens: 100,
            reserve_tokens: 10,
            main_loop_max_output_tokens: 0,
            preserve_tail_tokens: 1,
            deadline_ms: 1,
        };

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Skip
        );
    }

    #[test]
    fn evaluate_skips_when_no_eligible_user_message_boundary_exists() {
        let context = crate::test_support::test_run_context("compaction-strategy");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_prompt =
            CompactionPromptSnapshot::from_message_index(vec![MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 100,
            }]);
        let strategy = DefaultCompactionStrategy {
            context_limit_tokens: 100,
            reserve_tokens: 10,
            main_loop_max_output_tokens: 0,
            preserve_tail_tokens: 1,
            deadline_ms: 1,
        };
        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Skip
        );
    }

    #[test]
    fn evaluate_skips_when_below_threshold_with_valid_user_boundary_and_forcing_is_off() {
        let context = crate::test_support::test_run_context("compaction-strategy-below-threshold");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 20,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 20,
            },
        ]);
        let strategy = DefaultCompactionStrategy {
            context_limit_tokens: 100,
            reserve_tokens: 10,
            main_loop_max_output_tokens: 0,
            preserve_tail_tokens: 60,
            deadline_ms: 1,
        };

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Skip
        );
    }

    #[test]
    fn evaluate_triggers_at_latest_user_boundary_outside_tail() {
        let context = crate::test_support::test_run_context("compaction-strategy-trigger");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state = CompactionStrategyState::default();
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 30,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 30,
            },
            MessageIndexEntry {
                sequence: 3,
                kind: IndexedMessageKind::User,
                estimated_tokens: 30,
            },
            MessageIndexEntry {
                sequence: 4,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 30,
            },
        ]);
        let strategy = DefaultCompactionStrategy {
            context_limit_tokens: 100,
            reserve_tokens: 10,
            main_loop_max_output_tokens: 0,
            preserve_tail_tokens: 60,
            deadline_ms: 7,
        };

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Trigger {
                drop_through_seq: 1,
                preserve_tail_tokens: 60,
                deadline_ms: 7,
            }
        );
    }

    #[test]
    fn evaluate_triggers_when_newest_assistant_block_exceeds_tail_budget() {
        let context = crate::test_support::test_run_context("compaction-strategy-tail-overflow");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state = CompactionStrategyState::default();
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 100,
            },
        ]);
        let strategy = DefaultCompactionStrategy {
            context_limit_tokens: 100,
            reserve_tokens: 10,
            main_loop_max_output_tokens: 0,
            preserve_tail_tokens: 60,
            deadline_ms: 7,
        };

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Trigger {
                drop_through_seq: 1,
                preserve_tail_tokens: 60,
                deadline_ms: 7,
            }
        );
    }

    #[test]
    fn evaluate_skips_when_latest_user_boundary_was_already_compacted() {
        let context = crate::test_support::test_run_context("compaction-strategy-compacted");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state.last_compacted_through_seq = Some(3);
        state.compaction_state.force_compact_on_next_iteration = true;
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
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
                estimated_tokens: 100,
            },
        ]);
        let strategy = DefaultCompactionStrategy {
            context_limit_tokens: 100,
            reserve_tokens: 10,
            main_loop_max_output_tokens: 0,
            preserve_tail_tokens: 60,
            deadline_ms: 7,
        };

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Skip
        );
    }

    #[test]
    fn evaluate_uses_output_budget_when_larger_than_reserve() {
        let context = crate::test_support::test_run_context("compaction-strategy-output-budget");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 40,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 35,
            },
        ]);
        let strategy = DefaultCompactionStrategy {
            context_limit_tokens: 100,
            reserve_tokens: 10,
            main_loop_max_output_tokens: 30,
            preserve_tail_tokens: 1,
            deadline_ms: 7,
        };

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Trigger {
                drop_through_seq: 1,
                preserve_tail_tokens: 1,
                deadline_ms: 7,
            }
        );
    }
}
