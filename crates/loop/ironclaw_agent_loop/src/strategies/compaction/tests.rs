use super::*;
use crate::state::{
    CompactionEffectivenessBaseline, CompactionPromptSnapshot, CompactionStrategyState,
    DeferredCompactionWatermark, LoopExecutionState, MessageIndexEntry,
};
use ironclaw_host_api::ids::CapabilityId;
use ironclaw_loop_contracts::PromptContextTokenBudget;

#[test]
fn evaluate_skips_when_message_index_is_empty() {
    let context = crate::test_support::test_run_context("compaction-strategy-empty");
    let mut state = LoopExecutionState::initial_for_run(&context);
    state.compaction_state.force_compact_on_next_iteration = true;
    let strategy = DefaultCompactionStrategy {
        prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
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
        prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
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
        prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
        preserve_tail_tokens: 60,
        deadline_ms: 1,
    };

    assert_eq!(
        strategy.should_compact(&state, &context),
        CompactionDecision::Skip
    );
}

#[test]
fn can_evaluate_skips_when_visible_threshold_equals_preserve_tail() {
    let context = crate::test_support::test_run_context("compaction-strategy-equal-tail");
    let mut state = LoopExecutionState::initial_for_run(&context);
    state.compaction_prompt =
        CompactionPromptSnapshot::from_message_index(vec![MessageIndexEntry {
            sequence: 1,
            kind: IndexedMessageKind::User,
            estimated_tokens: 100,
        }]);
    let strategy = DefaultCompactionStrategy {
        prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
        preserve_tail_tokens: 90,
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
        prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
        preserve_tail_tokens: 60,
        deadline_ms: 7,
    };

    assert_eq!(
        strategy.should_compact(&state, &context),
        CompactionDecision::Trigger {
            drop_through_seq: 1,
            preserve_tail_tokens: 60,
            deadline_ms: 7,
            effectiveness_baseline: CompactionEffectivenessBaseline::TriggerThresholdTokens {
                tokens: 90,
            },
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
        prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
        preserve_tail_tokens: 60,
        deadline_ms: 7,
    };

    assert_eq!(
        strategy.should_compact(&state, &context),
        CompactionDecision::Trigger {
            drop_through_seq: 1,
            preserve_tail_tokens: 60,
            deadline_ms: 7,
            effectiveness_baseline: CompactionEffectivenessBaseline::TriggerThresholdTokens {
                tokens: 90,
            },
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
        prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
        preserve_tail_tokens: 60,
        deadline_ms: 7,
    };

    assert_eq!(
        strategy.should_compact(&state, &context),
        CompactionDecision::Skip
    );
}

#[test]
fn evaluate_skips_previously_deferred_boundary_when_forced() {
    let context = crate::test_support::test_run_context("compaction-strategy-deferred");
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
            kind: IndexedMessageKind::Assistant,
            estimated_tokens: 10,
        },
        MessageIndexEntry {
            sequence: 3,
            kind: IndexedMessageKind::User,
            estimated_tokens: 10,
        },
    ]);
    state.compaction_state.last_deferred = Some(DeferredCompactionWatermark {
        through_seq: 3,
        prompt_fingerprint: state.compaction_prompt.fingerprint(),
    });
    let strategy = DefaultCompactionStrategy {
        prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
        preserve_tail_tokens: 1,
        deadline_ms: 7,
    };

    assert_eq!(
        strategy.should_compact(&state, &context),
        CompactionDecision::Trigger {
            drop_through_seq: 1,
            preserve_tail_tokens: 1,
            deadline_ms: 7,
            effectiveness_baseline: CompactionEffectivenessBaseline::PreCompactionPromptTokens {
                tokens: 30
            },
        }
    );
}

#[test]
fn evaluate_skips_deferred_boundary_in_threshold_overflow_path() {
    let context = crate::test_support::test_run_context("compaction-strategy-deferred-threshold");
    let mut state = LoopExecutionState::initial_for_run(&context);
    state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
        MessageIndexEntry {
            sequence: 1,
            kind: IndexedMessageKind::User,
            estimated_tokens: 50,
        },
        MessageIndexEntry {
            sequence: 2,
            kind: IndexedMessageKind::Assistant,
            estimated_tokens: 50,
        },
        MessageIndexEntry {
            sequence: 3,
            kind: IndexedMessageKind::User,
            estimated_tokens: 50,
        },
        MessageIndexEntry {
            sequence: 4,
            kind: IndexedMessageKind::Assistant,
            estimated_tokens: 50,
        },
    ]);
    state.compaction_state.last_deferred = Some(DeferredCompactionWatermark {
        through_seq: 3,
        prompt_fingerprint: state.compaction_prompt.fingerprint(),
    });
    let strategy = DefaultCompactionStrategy {
        prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
        preserve_tail_tokens: 60,
        deadline_ms: 7,
    };

    assert_eq!(
        strategy.should_compact(&state, &context),
        CompactionDecision::Trigger {
            drop_through_seq: 1,
            preserve_tail_tokens: 60,
            deadline_ms: 7,
            effectiveness_baseline: CompactionEffectivenessBaseline::TriggerThresholdTokens {
                tokens: 90,
            },
        }
    );
}

#[test]
fn evaluate_skips_when_only_deferred_boundary_is_eligible_in_threshold_overflow_path() {
    let context = crate::test_support::test_run_context("compaction-strategy-deferred-skip");
    let mut state = LoopExecutionState::initial_for_run(&context);
    state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
        MessageIndexEntry {
            sequence: 1,
            kind: IndexedMessageKind::User,
            estimated_tokens: 50,
        },
        MessageIndexEntry {
            sequence: 2,
            kind: IndexedMessageKind::Assistant,
            estimated_tokens: 50,
        },
    ]);
    state.compaction_state.last_deferred = Some(DeferredCompactionWatermark {
        through_seq: 1,
        prompt_fingerprint: state.compaction_prompt.fingerprint(),
    });
    let strategy = DefaultCompactionStrategy {
        prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
        preserve_tail_tokens: 60,
        deadline_ms: 7,
    };

    assert_eq!(
        strategy.should_compact(&state, &context),
        CompactionDecision::Skip
    );
}

#[test]
fn evaluate_retries_deferred_boundary_after_prompt_snapshot_changes() {
    let context = crate::test_support::test_run_context("compaction-strategy-deferred-changed");
    let mut state = LoopExecutionState::initial_for_run(&context);
    state.compaction_state.last_deferred = Some(DeferredCompactionWatermark {
        through_seq: 3,
        prompt_fingerprint: 42,
    });
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
    ]);
    let strategy = DefaultCompactionStrategy {
        prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
        preserve_tail_tokens: 1,
        deadline_ms: 7,
    };

    assert_eq!(
        strategy.should_compact(&state, &context),
        CompactionDecision::Trigger {
            drop_through_seq: 3,
            preserve_tail_tokens: 1,
            deadline_ms: 7,
            effectiveness_baseline: CompactionEffectivenessBaseline::PreCompactionPromptTokens {
                tokens: 30
            },
        }
    );
}

#[test]
fn evaluate_retries_after_transcript_advances_past_deferred_boundary() {
    let context = crate::test_support::test_run_context("compaction-strategy-deferred-newer");
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
    ]);
    state.compaction_state.last_deferred = Some(DeferredCompactionWatermark {
        through_seq: 3,
        prompt_fingerprint: state.compaction_prompt.fingerprint(),
    });
    let strategy = DefaultCompactionStrategy {
        prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
        preserve_tail_tokens: 1,
        deadline_ms: 7,
    };

    assert_eq!(
        strategy.should_compact(&state, &context),
        CompactionDecision::Trigger {
            drop_through_seq: 5,
            preserve_tail_tokens: 1,
            deadline_ms: 7,
            effectiveness_baseline: CompactionEffectivenessBaseline::PreCompactionPromptTokens {
                tokens: 50
            },
        }
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
        prompt_context_budget: PromptContextTokenBudget::new(100, 10, 30),
        preserve_tail_tokens: 1,
        deadline_ms: 7,
    };

    assert_eq!(
        strategy.should_compact(&state, &context),
        CompactionDecision::Trigger {
            drop_through_seq: 1,
            preserve_tail_tokens: 1,
            deadline_ms: 7,
            effectiveness_baseline: CompactionEffectivenessBaseline::TriggerThresholdTokens {
                tokens: 70,
            },
        }
    );
}

#[test]
fn tail_preserving_user_boundary_respects_minimum_tail_message_count() {
    let context = crate::test_support::test_run_context("compaction-strategy-min-tail");
    let mut state = LoopExecutionState::initial_for_run(&context);
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
            estimated_tokens: 10,
        },
    ]);

    let boundary =
        tail_preserving_user_boundary(&state, state.compaction_prompt.fingerprint(), 1, 2, |_| {
            true
        });

    assert_eq!(boundary, Some(1));
}

#[test]
fn evaluate_skips_threshold_trigger_when_circuit_is_open() {
    let context = crate::test_support::test_run_context("compaction-strategy-circuit-open");
    let mut state = LoopExecutionState::initial_for_run(&context);
    state.compaction_state.compaction_circuit_open = true;
    state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
        MessageIndexEntry {
            sequence: 1,
            kind: IndexedMessageKind::User,
            estimated_tokens: 100,
        },
        MessageIndexEntry {
            sequence: 2,
            kind: IndexedMessageKind::Assistant,
            estimated_tokens: 100,
        },
    ]);
    let strategy = DefaultCompactionStrategy {
        prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
        preserve_tail_tokens: 1,
        deadline_ms: 7,
    };

    assert_eq!(
        strategy.should_compact(&state, &context),
        CompactionDecision::Skip
    );
}

#[test]
fn evaluate_triggers_forced_compaction_even_when_circuit_is_open() {
    // BUG B1 regression: force_compact_on_next_iteration is how
    // context-overflow recovery and byte-cap overflow request a shrink.
    // An open breaker must not suppress it — only automatic
    // threshold-triggered compaction is gated.
    let context = crate::test_support::test_run_context("compaction-strategy-circuit-open-forced");
    let mut state = LoopExecutionState::initial_for_run(&context);
    state.compaction_state.compaction_circuit_open = true;
    state.compaction_state.force_compact_on_next_iteration = true;
    state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
        MessageIndexEntry {
            sequence: 1,
            kind: IndexedMessageKind::User,
            estimated_tokens: 100,
        },
        MessageIndexEntry {
            sequence: 2,
            kind: IndexedMessageKind::Assistant,
            estimated_tokens: 100,
        },
    ]);
    let strategy = DefaultCompactionStrategy {
        prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
        preserve_tail_tokens: 1,
        deadline_ms: 7,
    };

    assert_eq!(
        strategy.should_compact(&state, &context),
        CompactionDecision::Trigger {
            drop_through_seq: 1,
            preserve_tail_tokens: 1,
            deadline_ms: 7,
            effectiveness_baseline: CompactionEffectivenessBaseline::PreCompactionPromptTokens {
                tokens: 200
            },
        }
    );
}

// --- ByteCapStrategy tests ---

#[test]
fn byte_cap_strategy_trips_when_capability_exceeds_cap() {
    let context = crate::test_support::test_run_context("byte-cap-policy-trips");
    let mut state = LoopExecutionState::initial_for_run(&context);
    let id = CapabilityId::new("builtin.http").expect("valid capability");
    // 32_000 is the cap; 32_001 exceeds it.
    state
        .post_capability_state
        .pending_capability_bytes
        .insert(id, 32_001);

    let strategy = ByteCapStrategy::with_defaults();
    assert_eq!(
        strategy.should_force_compact(&state),
        Some(CompactionInitiator::CapabilityResultOverflow)
    );
}

#[test]
fn byte_cap_strategy_skips_when_under_threshold() {
    let context = crate::test_support::test_run_context("byte-cap-policy-under");
    let mut state = LoopExecutionState::initial_for_run(&context);
    let http_id = CapabilityId::new("builtin.http").expect("valid capability");
    let subagent_id = CapabilityId::new("builtin.spawn_subagent").expect("valid capability");
    // Both under their respective caps.
    state
        .post_capability_state
        .pending_capability_bytes
        .insert(http_id, 31_999);
    state
        .post_capability_state
        .pending_capability_bytes
        .insert(subagent_id, 47_999);

    let strategy = ByteCapStrategy::with_defaults();
    assert_eq!(strategy.should_force_compact(&state), None);
}

#[test]
fn byte_cap_strategy_uses_default_cap_for_unknown_capability() {
    let context = crate::test_support::test_run_context("byte-cap-policy-unknown");
    let mut state = LoopExecutionState::initial_for_run(&context);
    let id = CapabilityId::new("custom.unknown_tool").expect("valid capability");
    // DEFAULT_FALLBACK_CAP_BYTES is 32_000; 32_001 exceeds it.
    state
        .post_capability_state
        .pending_capability_bytes
        .insert(id, ByteCapStrategy::DEFAULT_FALLBACK_CAP_BYTES + 1);

    let strategy = ByteCapStrategy::with_defaults();
    assert_eq!(
        strategy.should_force_compact(&state),
        Some(CompactionInitiator::CapabilityResultOverflow)
    );
}

#[test]
fn byte_cap_strategy_empty_accumulator_returns_none() {
    let context = crate::test_support::test_run_context("byte-cap-policy-empty");
    let state = LoopExecutionState::initial_for_run(&context);
    // pending_capability_bytes is empty by default.
    let strategy = ByteCapStrategy::with_defaults();
    assert_eq!(strategy.should_force_compact(&state), None);
}

#[test]
fn byte_cap_strategy_with_cap_overrides_default_cap() {
    let ctx = crate::test_support::test_run_context("byte-cap-with-cap");
    let mut state = LoopExecutionState::initial_for_run(&ctx);
    let id = CapabilityId::new("custom.large_tool").unwrap();
    state
        .post_capability_state
        .pending_capability_bytes
        .insert(id.clone(), 5_000);
    // Default cap (32_000) would NOT trip at 5_000; custom cap of 4_000 should trip.
    let strategy = ByteCapStrategy::with_defaults().with_cap(id, 4_000);
    assert_eq!(
        strategy.should_force_compact(&state),
        Some(CompactionInitiator::CapabilityResultOverflow)
    );
}

/// Four alternating messages summing to `tokens`, so the index both
/// trips the threshold and offers a compactable user boundary outside
/// the preserved tail. A single entry trips the threshold but can never
/// Trigger — there is no boundary to drop through.
fn state_with_observed_prompt_tokens(tokens: u64, context: &LoopRunContext) -> LoopExecutionState {
    let each = tokens / 4;
    let mut state = LoopExecutionState::initial_for_run(context);
    state.compaction_state = CompactionStrategyState::default();
    state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
        MessageIndexEntry {
            sequence: 1,
            kind: IndexedMessageKind::User,
            estimated_tokens: each,
        },
        MessageIndexEntry {
            sequence: 2,
            kind: IndexedMessageKind::Assistant,
            estimated_tokens: each,
        },
        MessageIndexEntry {
            sequence: 3,
            kind: IndexedMessageKind::User,
            estimated_tokens: each,
        },
        MessageIndexEntry {
            sequence: 4,
            kind: IndexedMessageKind::Assistant,
            estimated_tokens: each,
        },
    ]);
    state
}

#[test]
fn run_context_budget_overrides_the_strategy_default_for_compaction() {
    // The strategy's own budget would not trigger, but the run's model
    // has a far smaller real window, so this prompt is already over.
    let strategy = DefaultCompactionStrategy {
        prompt_context_budget: PromptContextTokenBudget::new(128_000, 20_000, 0),
        preserve_tail_tokens: 10,
        deadline_ms: 30_000,
    };
    let ctx = crate::test_support::test_run_context("compaction-budget-override")
        .with_resolved_context_budget(PromptContextTokenBudget::new(40_000, 5_000, 0));
    let state = state_with_observed_prompt_tokens(50_000, &ctx);

    assert!(matches!(
        strategy.should_compact(&state, &ctx),
        CompactionDecision::Trigger { .. }
    ));
}

#[test]
fn absent_run_context_budget_falls_back_to_the_strategy_default() {
    let strategy = DefaultCompactionStrategy {
        prompt_context_budget: PromptContextTokenBudget::new(128_000, 20_000, 0),
        preserve_tail_tokens: 10,
        deadline_ms: 30_000,
    };
    let ctx = crate::test_support::test_run_context("compaction-budget-fallback");
    let state = state_with_observed_prompt_tokens(50_000, &ctx);

    assert_eq!(
        strategy.should_compact(&state, &ctx),
        CompactionDecision::Skip
    );
}
