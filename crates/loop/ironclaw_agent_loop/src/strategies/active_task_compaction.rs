use crate::state::{IndexedMessageKind, LoopExecutionState, MessageIndexEntry};
use ironclaw_loop_contracts::{CompactionInitiator, LoopRunContext};

use super::compaction::{
    CompactionDecision, CompactionStrategy, DefaultCompactionStrategy,
    eligible_window_eviction_boundary, is_eligible_user_boundary,
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
        ctx: &LoopRunContext,
    ) -> CompactionDecision {
        let budget = self.base.effective_budget(ctx);
        if !self.base.can_evaluate(state, budget) {
            return CompactionDecision::Skip;
        }

        let prompt_fingerprint = state.compaction_prompt.fingerprint();
        if state.compaction_state.force_compact_initiator
            == Some(CompactionInitiator::WindowEviction)
        {
            let preserve_from_sequence = latest_additional_user_sequence(state);
            return eligible_window_eviction_boundary(
                state,
                prompt_fingerprint,
                preserve_from_sequence,
            )
            .map(|sequence| self.base.trigger_at(state, budget, sequence))
            .unwrap_or(CompactionDecision::Skip);
        }
        active_task_preserving_user_boundary(
            state,
            prompt_fingerprint,
            self.base.effective_preserve_tail_tokens(budget),
            self.minimum_tail_messages,
            self.minimum_compacted_messages,
        )
        .map(|sequence| self.base.trigger_at(state, budget, sequence))
        .unwrap_or(CompactionDecision::Skip)
    }
}

fn latest_additional_user_sequence(state: &LoopExecutionState) -> Option<u64> {
    let mut user_entries = state
        .compaction_prompt
        .message_index
        .iter()
        .rev()
        .filter(|entry| entry.kind == IndexedMessageKind::User);
    let latest_user = user_entries.next()?;
    // The accepted task is re-pinned by the context port after compaction. A
    // second user entry is a later follow-up or steering instruction and has
    // no equivalent pin, so the cut point must remain strictly before it.
    user_entries.next().map(|_| latest_user.sequence)
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
    use crate::state::{
        CompactionEffectivenessBaseline, CompactionPromptSnapshot, LoopExecutionState,
        MessageIndexEntry,
    };
    use ironclaw_loop_contracts::PromptContextTokenBudget;

    fn active_task_preserving_strategy(
        preserve_tail_tokens: u64,
    ) -> ActiveTaskPreservingCompactionStrategy {
        ActiveTaskPreservingCompactionStrategy::from(DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
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
                effectiveness_baseline:
                    CompactionEffectivenessBaseline::PreCompactionPromptTokens { tokens: 80 },
            }
        );
    }

    #[test]
    fn active_task_strategy_finds_a_boundary_inside_a_small_window() {
        // Context-length regression: an 8k-advertised window derives
        // visible_transcript_tokens() == 5,400 (half == 2,700), while the
        // compiled-in tail is 8,000. Scale active_task_message_index()'s
        // eight 10-token entries up 100x (8,000 tokens total) so the
        // unclamped 8,000 tail can never be reached mid-walk (no boundary),
        // while the clamped 2,700 tail is reached with enough prefix and
        // tail messages to find one.
        let context = crate::test_support::test_run_context("active-task-small-window")
            .with_resolved_context_budget(PromptContextTokenBudget::from_advertised_window(Some(
                8_000,
            )));
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state.force_compact_on_next_iteration = true;
        let scaled_index: Vec<MessageIndexEntry> = active_task_message_index()
            .into_iter()
            .map(|entry| MessageIndexEntry {
                estimated_tokens: entry.estimated_tokens * 100,
                ..entry
            })
            .collect();
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(scaled_index);
        let strategy = ActiveTaskPreservingCompactionStrategy::from(DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::default(),
            preserve_tail_tokens: DefaultCompactionStrategy::DEFAULT_PRESERVE_TAIL_TOKENS,
            deadline_ms: 7,
        });

        assert!(matches!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Trigger {
                drop_through_seq,
                ..
            } if drop_through_seq > 0
        ));
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
        // `active_task_preserving_strategy`'s helper budget (100/10 -> 90
        // visible) would clamp a tail of 60 down to 45, changing this test's
        // premise (a candidate becomes reachable). Use a budget whose half-
        // visible ceiling comfortably exceeds 60 so the configured tail
        // survives the clamp intact and this test still proves the tail
        // budget alone can defeat every candidate.
        let strategy = ActiveTaskPreservingCompactionStrategy::from(DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(200, 10, 0),
            preserve_tail_tokens: 60,
            deadline_ms: 7,
        });

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
                effectiveness_baseline: CompactionEffectivenessBaseline::TriggerThresholdTokens {
                    tokens: 90
                },
            }
        );
    }

    #[test]
    fn window_eviction_bypasses_active_task_tail_thresholds_at_safe_tool_result() {
        let context = crate::test_support::test_run_context("active-task-window-eviction");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state.force_compact_on_next_iteration = true;
        state.compaction_state.force_compact_initiator = Some(CompactionInitiator::WindowEviction);
        state.compaction_state.window_eviction =
            Some(ironclaw_loop_contracts::LoopContextWindowTruncation {
                omitted_through_sequence: 2,
                omitted_through_kind:
                    ironclaw_loop_contracts::LoopContextCompactionKind::ToolResult,
            });
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 129,
                kind: IndexedMessageKind::ToolResult,
                estimated_tokens: 10,
            },
        ]);
        let strategy = ActiveTaskPreservingCompactionStrategy::from(DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(128_000, 20_000, 0),
            preserve_tail_tokens: 8_000,
            deadline_ms: 7,
        });

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Trigger {
                drop_through_seq: 129,
                preserve_tail_tokens: 8_000,
                deadline_ms: 7,
                effectiveness_baseline:
                    CompactionEffectivenessBaseline::PreCompactionPromptTokens { tokens: 20 },
            }
        );
    }

    #[test]
    fn window_eviction_keeps_latest_steering_user_outside_compacted_prefix() {
        let context = crate::test_support::test_run_context("active-task-window-steering");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state.force_compact_on_next_iteration = true;
        state.compaction_state.force_compact_initiator = Some(CompactionInitiator::WindowEviction);
        state.compaction_state.window_eviction =
            Some(ironclaw_loop_contracts::LoopContextWindowTruncation {
                omitted_through_sequence: 2,
                omitted_through_kind:
                    ironclaw_loop_contracts::LoopContextCompactionKind::ToolResult,
            });
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 129,
                kind: IndexedMessageKind::ToolResult,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 130,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 131,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 10,
            },
        ]);
        let strategy = ActiveTaskPreservingCompactionStrategy::from(DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(128_000, 20_000, 0),
            preserve_tail_tokens: 8_000,
            deadline_ms: 7,
        });

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Trigger {
                drop_through_seq: 129,
                preserve_tail_tokens: 8_000,
                deadline_ms: 7,
                effectiveness_baseline:
                    CompactionEffectivenessBaseline::PreCompactionPromptTokens { tokens: 40 },
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
    fn active_task_strategy_skips_when_compaction_circuit_is_open() {
        let context = crate::test_support::test_run_context("active-task-preserving-circuit-open");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state.compaction_circuit_open = true;
        state.compaction_prompt =
            CompactionPromptSnapshot::from_message_index(active_task_message_index());
        state.compaction_prompt.observed_prompt_tokens = 90;
        let strategy = active_task_preserving_strategy(1);

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Skip
        );
    }

    #[test]
    fn active_task_strategy_forced_compaction_bypasses_open_circuit() {
        // BUG B1 regression: forced/recovery compactions must run even after
        // the ineffective-compaction breaker opened — the breaker only gates
        // automatic threshold-triggered compaction.
        let context =
            crate::test_support::test_run_context("active-task-preserving-circuit-open-forced");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state.compaction_circuit_open = true;
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
                effectiveness_baseline:
                    CompactionEffectivenessBaseline::PreCompactionPromptTokens { tokens: 80 },
            }
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

    #[test]
    fn active_task_strategy_honors_the_run_context_budget() {
        // The strategy's own budget (100/10 -> 90 visible) leaves headroom
        // for this 80-token index, so it would not compact. The run's model
        // has a far smaller real window, which puts the prompt over.
        let context = crate::test_support::test_run_context("active-task-budget-override")
            .with_resolved_context_budget(PromptContextTokenBudget::new(50, 5, 0));
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_prompt =
            CompactionPromptSnapshot::from_message_index(active_task_message_index());
        let strategy = active_task_preserving_strategy(1);

        assert!(matches!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Trigger { .. }
        ));
    }

    #[test]
    fn active_task_strategy_falls_back_to_its_own_budget_without_a_resolved_one() {
        let context = crate::test_support::test_run_context("active-task-budget-fallback");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_prompt =
            CompactionPromptSnapshot::from_message_index(active_task_message_index());
        let strategy = active_task_preserving_strategy(1);

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Skip
        );
    }
}
