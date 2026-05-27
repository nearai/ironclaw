use crate::state::{IndexedMessageKind, LoopExecutionState};
use ironclaw_turns::run_profile::LoopRunContext;

/// Decides whether to replace older transcript context with a host-managed summary.
///
/// The strategy is pure policy: it reads durable compaction state and returns
/// either `Skip` or the inclusive user-message boundary the executor should
/// compact through. State mutation, transcript reads, inference, persistence,
/// and progress events stay in the executor and host compaction port.
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
    pub preserve_tail_tokens: u64,
    pub deadline_ms: u64,
}

impl DefaultCompactionStrategy {
    pub const DEFAULT_CONTEXT_LIMIT_TOKENS: u64 = 128_000;
    pub const DEFAULT_RESERVE_TOKENS: u64 = 20_000;
    pub const DEFAULT_PRESERVE_TAIL_TOKENS: u64 = 8_000;
    pub const DEFAULT_DEADLINE_MS: u64 = 30_000;
}

impl Default for DefaultCompactionStrategy {
    fn default() -> Self {
        Self {
            context_limit_tokens: Self::DEFAULT_CONTEXT_LIMIT_TOKENS,
            reserve_tokens: Self::DEFAULT_RESERVE_TOKENS,
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
        if state.compaction_state.message_index.is_empty() {
            return CompactionDecision::Skip;
        }
        let threshold = self
            .context_limit_tokens
            .saturating_sub(self.reserve_tokens);
        if threshold <= self.preserve_tail_tokens {
            return CompactionDecision::Skip;
        }
        let total_tokens: u64 = state
            .compaction_state
            .message_index
            .iter()
            .map(|entry| entry.estimated_tokens)
            .sum();
        if !state.compaction_state.force_compact_on_next_iteration && total_tokens < threshold {
            return CompactionDecision::Skip;
        }

        let mut tail_tokens_after_entry = 0_u64;
        for entry in state.compaction_state.message_index.iter().rev() {
            if entry.kind == IndexedMessageKind::User
                && Some(entry.sequence) > state.compaction_state.last_compacted_through_seq
                && tail_tokens_after_entry <= self.preserve_tail_tokens
            {
                return CompactionDecision::Trigger {
                    drop_through_seq: entry.sequence,
                    preserve_tail_tokens: self.preserve_tail_tokens,
                    deadline_ms: self.deadline_ms,
                };
            }
            tail_tokens_after_entry =
                tail_tokens_after_entry.saturating_add(entry.estimated_tokens);
        }
        CompactionDecision::Skip
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{CompactionStrategyState, LoopExecutionState, MessageIndexEntry};

    #[test]
    fn evaluate_skips_when_no_eligible_user_message_boundary_exists() {
        let context = crate::test_support::test_run_context("compaction-strategy");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state = CompactionStrategyState {
            message_index: vec![MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 100,
            }],
            ..Default::default()
        };
        let strategy = DefaultCompactionStrategy {
            context_limit_tokens: 100,
            reserve_tokens: 10,
            preserve_tail_tokens: 1,
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
        state.compaction_state = CompactionStrategyState {
            message_index: vec![
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
            ],
            ..Default::default()
        };
        let strategy = DefaultCompactionStrategy {
            context_limit_tokens: 100,
            reserve_tokens: 10,
            preserve_tail_tokens: 60,
            deadline_ms: 7,
        };

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Trigger {
                drop_through_seq: 3,
                preserve_tail_tokens: 60,
                deadline_ms: 7,
            }
        );
    }
}
