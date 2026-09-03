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
fn can_evaluate_skips_when_the_budget_leaves_no_visible_transcript() {
    // Old assertion (`can_evaluate_skips_when_visible_threshold_equals_preserve_tail`)
    // pinned the defect: it relied on the raw `preserve_tail_tokens` (90)
    // being >= `threshold` (90) to trip the guard, which is exactly the
    // condition the small-advertised-window bug hit (a run-resolved
    // threshold now legitimately falls below a compiled-in tail). With the
    // tail clamped to half the visible transcript, that guard no longer
    // trips here (threshold 90 > clamped tail 45), and forcing would no
    // longer be suppressed — which is correct: the guard's real purpose is
    // "there is no visible transcript to compact," not "the tail happens to
    // be large." This rewrite asserts that real purpose directly with a
    // budget whose `visible_transcript_tokens()` is 0, and proves it still
    // holds even for the forced/recovery path.
    let context =
        crate::test_support::test_run_context("compaction-strategy-no-visible-transcript");
    let mut state = LoopExecutionState::initial_for_run(&context);
    state.compaction_state.force_compact_on_next_iteration = true;
    state.compaction_prompt =
        CompactionPromptSnapshot::from_message_index(vec![MessageIndexEntry {
            sequence: 1,
            kind: IndexedMessageKind::User,
            estimated_tokens: 100,
        }]);
    let strategy = DefaultCompactionStrategy {
        prompt_context_budget: PromptContextTokenBudget::new(10, 10, 0),
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
            // Clamped from the configured 60: visible (90) / 2 = 45. The
            // walk-back to sequence 1 is unaffected by the clamp here (it
            // still needs to cross two message blocks either way), so this
            // is the genuine clamp output, not an incidental input choice.
            preserve_tail_tokens: 45,
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
            // Clamped from the configured 60 to visible (90) / 2 = 45; the
            // boundary walk-back is unaffected (genuine clamp output).
            preserve_tail_tokens: 45,
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
            // Clamped from the configured 60 to visible (90) / 2 = 45; the
            // boundary walk-back (and the deferred-boundary rejection at
            // sequence 3) is unaffected (genuine clamp output).
            preserve_tail_tokens: 45,
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

// --- Small-advertised-window tail clamp regression (context-length bug) ---
//
// `preserve_tail_tokens` is compiled-in at 8,000, but `threshold` is now
// derived from the run's model-advertised window. An 8k-window model derives
// visible_transcript_tokens() == 5,400 (< 8,000), so both the automatic and
// forced-recovery paths must clamp the tail to half the visible transcript
// instead of disabling compaction outright.

#[test]
fn small_advertised_window_still_allows_forced_compaction() {
    let ctx = crate::test_support::test_run_context("compaction-small-window-forced")
        .with_resolved_context_budget(PromptContextTokenBudget::from_advertised_window(Some(
            8_000,
        )));
    let mut state = state_with_observed_prompt_tokens(80, &ctx);
    state.compaction_state.force_compact_on_next_iteration = true;
    let strategy = DefaultCompactionStrategy {
        prompt_context_budget: PromptContextTokenBudget::default(),
        preserve_tail_tokens: DefaultCompactionStrategy::DEFAULT_PRESERVE_TAIL_TOKENS,
        deadline_ms: 30_000,
    };

    assert_eq!(
        strategy.should_compact(&state, &ctx),
        CompactionDecision::Trigger {
            // Forced/recovery compaction uses `latest_eligible_user_boundary`,
            // which is not tail-aware — it always cuts at the most recent
            // eligible user message (sequence 3 here), not the earliest.
            drop_through_seq: 3,
            preserve_tail_tokens: 2_700,
            deadline_ms: 30_000,
            effectiveness_baseline: CompactionEffectivenessBaseline::PreCompactionPromptTokens {
                tokens: 80,
            },
        }
    );
}

#[test]
fn small_advertised_window_still_triggers_automatic_compaction() {
    let ctx = crate::test_support::test_run_context("compaction-small-window-automatic")
        .with_resolved_context_budget(PromptContextTokenBudget::from_advertised_window(Some(
            8_000,
        )));
    let state = state_with_observed_prompt_tokens(5_400, &ctx);
    let strategy = DefaultCompactionStrategy {
        prompt_context_budget: PromptContextTokenBudget::default(),
        preserve_tail_tokens: DefaultCompactionStrategy::DEFAULT_PRESERVE_TAIL_TOKENS,
        deadline_ms: 30_000,
    };

    assert_eq!(
        strategy.should_compact(&state, &ctx),
        CompactionDecision::Trigger {
            drop_through_seq: 1,
            preserve_tail_tokens: 2_700,
            deadline_ms: 30_000,
            effectiveness_baseline: CompactionEffectivenessBaseline::TriggerThresholdTokens {
                tokens: 5_400,
            },
        }
    );
}

#[test]
fn default_budget_keeps_the_full_configured_tail() {
    let ctx = crate::test_support::test_run_context("compaction-default-budget-tail");
    let state = state_with_observed_prompt_tokens(120_000, &ctx);
    let strategy = DefaultCompactionStrategy::default();

    assert_eq!(
        strategy.should_compact(&state, &ctx),
        CompactionDecision::Trigger {
            // Each 30,000-token message block alone exceeds the 8,000-token
            // tail, so the walk-back returns at the first eligible user
            // boundary it finds (sequence 3), not the earliest message.
            drop_through_seq: 3,
            preserve_tail_tokens: 8_000,
            deadline_ms: DefaultCompactionStrategy::DEFAULT_DEADLINE_MS,
            effectiveness_baseline: CompactionEffectivenessBaseline::TriggerThresholdTokens {
                tokens: 108_000,
            },
        }
    );
}
