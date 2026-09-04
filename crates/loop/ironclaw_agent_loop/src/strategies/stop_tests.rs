use async_trait::async_trait;
use ironclaw_host_api::turn::{LoopMessageRef, LoopResultRef};
use serde_json::json;

use super::*;

#[test]
fn stop_condition_strategy_is_object_safe() {
    struct AlwaysContinue;

    #[async_trait]
    impl StopConditionStrategy for AlwaysContinue {
        async fn observe_completed_turn(
            &self,
            state: &LoopExecutionState,
            _: &TurnSummary,
        ) -> StopStrategyState {
            state.stop_state.clone()
        }

        async fn should_stop_after_observed_turn(
            &self,
            _state: &LoopExecutionState,
            _: &TurnSummary,
        ) -> StopOutcome {
            StopOutcome::Continue {}
        }
    }

    assert_stop_condition_strategy_object_safe(&AlwaysContinue);
}

#[test]
fn turn_summary_round_trips_through_json() {
    let summary = TurnSummary {
        kind: TurnEndKind::AfterCapabilityBatch,
        assistant_message_ref: Some(LoopMessageRef::new("msg:assistant-1").unwrap()),
        batch_result_refs: vec![
            LoopResultRef::new("result:call-1").unwrap(),
            LoopResultRef::new("result:call-2").unwrap(),
        ],
        capability_batch: CapabilityBatchTurnSummary::default(),
    };

    let serialized = serde_json::to_string(&summary).unwrap();
    let deserialized = serde_json::from_str::<TurnSummary>(&serialized).unwrap();

    assert_eq!(deserialized, summary);
}

#[test]
fn stop_outcome_round_trips_through_json() {
    let outcome = StopOutcome::Stop {
        kind: StopKind::NoProgressDetected,
    };

    let value = serde_json::to_value(&outcome).unwrap();
    // Variant tag must be snake_case on the wire, matching sibling enums.
    assert!(
        value.get("stop").is_some(),
        "expected snake_case `stop` key, got {value}"
    );
    assert!(
        value.get("Stop").is_none(),
        "PascalCase `Stop` key leaked into wire form: {value}"
    );

    let deserialized = serde_json::from_value::<StopOutcome>(value).unwrap();
    assert_eq!(deserialized, outcome);

    let continue_outcome = StopOutcome::Continue {};
    let continue_value = serde_json::to_value(&continue_outcome).unwrap();
    assert!(
        continue_value.get("continue").is_some(),
        "expected snake_case `continue` key, got {continue_value}"
    );
    assert_eq!(
        serde_json::from_value::<StopOutcome>(continue_value).unwrap(),
        continue_outcome
    );
}

#[test]
fn aborted_stop_kind_preserves_failure_variant_tags() {
    for (failure_kind, wire_tag) in [
        (LoopFailureKind::PolicyDenied, "policy_denied"),
        (LoopFailureKind::ModelError, "model_error"),
    ] {
        let kind = StopKind::Aborted(failure_kind);
        let value = serde_json::to_value(kind).unwrap();

        assert_eq!(value, json!({ "aborted": wire_tag }));
        assert_eq!(serde_json::from_value::<StopKind>(value).unwrap(), kind);
    }
}

mod default_stop_condition_strategy {
    use ironclaw_host_api::ids::{CapabilityId, TenantId, ThreadId};
    use ironclaw_host_api::prepared_context::STRUCTURED_RESULT_CAPABILITY_ID;
    use ironclaw_host_api::turn::{
        LoopMessageRef, RunProfileId, RunProfileVersion, TurnId, TurnRunId, TurnScope,
    };
    use ironclaw_loop_contracts::{
        AgentLoopDriverDescriptor, CancellationPolicy, CapabilitySurfaceProfileId,
        CheckpointPolicy, CheckpointSchemaId, ConcurrencyClass, ContentDigest, ContextProfileId,
        LoopDriverId, LoopFailureKind, LoopRunContext, ModelProfileId,
        RedactedRunProfileProvenance, ResolvedRunProfile, ResourceBudgetPolicy, ResourceBudgetTier,
        RunClassId, RunProfileFingerprint, RuntimeProfileConstraints, SchedulingClass,
        SteeringPolicy,
    };
    use serde_json::json;

    use super::super::{
        CapabilityBatchTurnSummary, DefaultStopConditionStrategy, StopConditionStrategy, StopKind,
        StopOutcome, TurnEndKind, TurnSummary,
    };
    use crate::state::{
        CapabilityCallSignature, CapabilityOutputObservation, LoopExecutionState,
        RepeatedCallWarningPhase, RepeatedCallWarningState, StopStrategyState,
    };

    fn test_run_context() -> LoopRunContext {
        let scope = TurnScope::new(
            TenantId::new("tenant-default-stop").expect("valid"),
            None,
            None,
            ThreadId::new("thread-default-stop").expect("valid"),
        );
        let descriptor = AgentLoopDriverDescriptor {
            id: LoopDriverId::new("default_stop_test_driver").expect("valid"),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(
                CheckpointSchemaId::new("default_stop_test_checkpoint").expect("valid"),
            ),
            checkpoint_schema_version: Some(RunProfileVersion::new(1)),
        };
        let resolved_run_profile = ResolvedRunProfile {
            run_class_id: RunClassId::new("default_stop_test_class").expect("valid"),
            profile_id: RunProfileId::default_profile(),
            profile_version: RunProfileVersion::new(1),
            loop_driver: descriptor.clone(),
            checkpoint_schema_id: descriptor
                .checkpoint_schema_id
                .clone()
                .expect("descriptor checkpoint id"),
            checkpoint_schema_version: descriptor
                .checkpoint_schema_version
                .expect("descriptor checkpoint version"),
            model_profile_id: ModelProfileId::new("default_stop_test_model").expect("valid"),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new(
                "default_stop_test_capabilities",
            )
            .expect("valid"),
            context_profile_id: ContextProfileId::new("default_stop_test_context").expect("valid"),
            steering_policy: SteeringPolicy {
                allow_steering: false,
                allow_interrupt: true,
                allow_driver_specific_nudges: false,
            },
            cancellation_policy: CancellationPolicy {
                allow_cancel: true,
                require_checkpoint_before_cancel: false,
            },
            checkpoint_policy: CheckpointPolicy {
                require_before_model: false,
                require_before_side_effect: false,
                require_before_block: true,
                max_checkpoint_bytes: 64 * 1024,
                require_final_checkpoint: false,
                allow_no_reply_completion: false,
                before_model_checkpoint_interval: 1,
            },
            resource_budget_policy: ResourceBudgetPolicy {
                tier: ResourceBudgetTier::new("default_stop_test_tier").expect("valid"),
                max_model_calls: 32,
                max_capability_invocations: 64,
                max_wall_clock_seconds: None,
            },
            personal_context_policy: ironclaw_loop_contracts::PersonalContextPolicy::Excluded,
            runtime_constraints: RuntimeProfileConstraints {
                allow_raw_runtime_backend_selection: false,
                allow_broad_capability_surface: false,
            },
            runner_pool_id: None,
            scheduling_class: SchedulingClass::new("interactive").expect("valid"),
            concurrency_class: ConcurrencyClass::new("thread_serial").expect("valid"),
            resolution_fingerprint: RunProfileFingerprint::new("default-stop-test-fingerprint")
                .expect("valid"),
            provenance: RedactedRunProfileProvenance {
                sources: vec![],
                effective_privileges: vec![],
            },
        };
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
    }

    fn after_batch() -> TurnSummary {
        TurnSummary {
            kind: TurnEndKind::AfterCapabilityBatch,
            assistant_message_ref: Some(LoopMessageRef::new("msg:default-stop").expect("valid")),
            batch_result_refs: Vec::new(),
            capability_batch: CapabilityBatchTurnSummary::default(),
        }
    }

    fn after_batch_with_capability_summary(
        capability_batch: CapabilityBatchTurnSummary,
    ) -> TurnSummary {
        TurnSummary {
            capability_batch,
            ..after_batch()
        }
    }

    async fn observe_and_decide(
        strategy: &DefaultStopConditionStrategy,
        mut state: LoopExecutionState,
        summary: TurnSummary,
    ) -> (LoopExecutionState, StopOutcome) {
        state.stop_state = strategy.observe_completed_turn(&state, &summary).await;
        let outcome = strategy
            .should_stop_after_observed_turn(&state, &summary)
            .await;
        (state, outcome)
    }

    #[test]
    fn defaults_match_documented_baseline() {
        let strategy = DefaultStopConditionStrategy::default();
        assert_eq!(strategy.repetition_threshold, 3);
        assert_eq!(strategy.rejected_reply_threshold, 3);
    }

    #[tokio::test]
    async fn no_signal_continues_with_turns_completed_incremented() {
        let strategy = DefaultStopConditionStrategy::default();
        let mut state = LoopExecutionState::initial_for_run(&test_run_context());
        state.stop_state = StopStrategyState {
            turns_completed: 4,
            trailing_rejected_replies: 0,
            ..StopStrategyState::default()
        };

        let (state, outcome) = observe_and_decide(&strategy, state, after_batch()).await;

        assert_eq!(state.stop_state.turns_completed, 5);
        assert!(matches!(outcome, StopOutcome::Continue { .. }));
    }

    #[tokio::test]
    async fn completed_structured_result_is_recorded_in_typed_stop_state() {
        let strategy = DefaultStopConditionStrategy::default();
        let state = LoopExecutionState::initial_for_run(&test_run_context());
        let summary = after_batch_with_capability_summary(CapabilityBatchTurnSummary {
            invocation_count: 1,
            terminate_hint_count: 1,
            observed_signatures: vec![
                CapabilityCallSignature::from_call(
                    CapabilityId::new(STRUCTURED_RESULT_CAPABILITY_ID).expect("capability id"),
                    &json!({"outcome": "nothing_to_report"}),
                )
                .expect("signature"),
            ],
        });

        let (state, outcome) = observe_and_decide(&strategy, state, summary).await;

        assert!(state.stop_state.structured_result_recorded);
        assert!(matches!(
            outcome,
            StopOutcome::Stop {
                kind: StopKind::GracefulStop
            }
        ));
    }

    #[tokio::test]
    async fn all_results_terminate_hint_returns_graceful_stop() {
        let strategy = DefaultStopConditionStrategy::default();
        let mut state = LoopExecutionState::initial_for_run(&test_run_context());
        state.stop_state = StopStrategyState {
            turns_completed: 1,
            trailing_rejected_replies: 0,
            ..StopStrategyState::default()
        };
        let summary = after_batch_with_capability_summary(CapabilityBatchTurnSummary {
            invocation_count: 3,
            terminate_hint_count: 3,
            ..CapabilityBatchTurnSummary::default()
        });

        let (state, outcome) = observe_and_decide(&strategy, state, summary).await;

        match outcome {
            StopOutcome::Stop { kind } => {
                assert_eq!(state.stop_state.turns_completed, 2);
                assert_eq!(kind, StopKind::GracefulStop);
            }
            other => panic!("expected Stop GracefulStop, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn partial_terminate_hint_batch_continues() {
        let strategy = DefaultStopConditionStrategy::default();
        let state = LoopExecutionState::initial_for_run(&test_run_context());
        let summary = after_batch_with_capability_summary(CapabilityBatchTurnSummary {
            invocation_count: 2,
            terminate_hint_count: 1,
            ..CapabilityBatchTurnSummary::default()
        });

        let (_state, outcome) = observe_and_decide(&strategy, state, summary).await;

        assert!(matches!(outcome, StopOutcome::Continue { .. }));
    }

    #[tokio::test]
    async fn reply_only_returns_graceful_stop() {
        let strategy = DefaultStopConditionStrategy::default();
        let state = LoopExecutionState::initial_for_run(&test_run_context());

        let (state, outcome) = observe_and_decide(
            &strategy,
            state,
            TurnSummary {
                kind: TurnEndKind::ReplyOnly,
                assistant_message_ref: Some(
                    LoopMessageRef::new("msg:default-stop").expect("valid"),
                ),
                batch_result_refs: Vec::new(),
                capability_batch: CapabilityBatchTurnSummary::default(),
            },
        )
        .await;

        match outcome {
            StopOutcome::Stop { kind } => {
                assert_eq!(state.stop_state.turns_completed, 1);
                assert_eq!(kind, StopKind::GracefulStop);
            }
            other => panic!("expected Stop GracefulStop, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_rejected_turn_resets_trailing_rejected_replies() {
        let strategy = DefaultStopConditionStrategy::default();
        let mut state = LoopExecutionState::initial_for_run(&test_run_context());
        state.stop_state.trailing_rejected_replies = 2;

        let (state, outcome) = observe_and_decide(&strategy, state, after_batch()).await;

        assert_eq!(state.stop_state.trailing_rejected_replies, 0);
        assert!(matches!(outcome, StopOutcome::Continue { .. }));
    }

    #[tokio::test]
    async fn terminate_hint_ignored_when_batch_was_empty() {
        let strategy = DefaultStopConditionStrategy::default();
        let mut state = LoopExecutionState::initial_for_run(&test_run_context());
        // invocation_count == 0: no batch this turn — strategy must not
        // graceful-stop on a vacuous "all-terminated" check.
        state.stop_state = StopStrategyState {
            turns_completed: 0,
            trailing_rejected_replies: 0,
            ..StopStrategyState::default()
        };

        let (_state, outcome) = observe_and_decide(
            &strategy,
            state,
            TurnSummary {
                kind: TurnEndKind::AfterCapabilityBatch,
                assistant_message_ref: None,
                batch_result_refs: Vec::new(),
                capability_batch: CapabilityBatchTurnSummary {
                    invocation_count: 0,
                    terminate_hint_count: 0,
                    ..CapabilityBatchTurnSummary::default()
                },
            },
        )
        .await;

        assert!(matches!(outcome, StopOutcome::Continue { .. }));
    }

    #[tokio::test]
    async fn same_signature_three_times_arms_warning_and_continues() {
        let strategy = DefaultStopConditionStrategy::default();
        let mut state = LoopExecutionState::initial_for_run(&test_run_context());
        let signature = CapabilityCallSignature::from_call(
            CapabilityId::new("demo.echo").expect("valid"),
            &json!({"x": 1}),
        )
        .expect("valid call signature");
        for _ in 0..3 {
            state.recent_call_signatures.push(signature.clone());
        }

        let (state, outcome) = observe_and_decide(&strategy, state, after_batch()).await;

        assert!(matches!(outcome, StopOutcome::Continue { .. }));
        let warning = state
            .stop_state
            .repeated_call_warning
            .expect("repeated call warning should be armed");
        assert_eq!(warning.signature, signature);
        assert_eq!(warning.phase, RepeatedCallWarningPhase::PendingRender);
    }

    #[tokio::test]
    async fn rendered_repeated_signature_warning_remains_advisory() {
        let strategy = DefaultStopConditionStrategy::default();
        let mut state = LoopExecutionState::initial_for_run(&test_run_context());
        let signature = CapabilityCallSignature::from_call(
            CapabilityId::new("demo.echo").expect("valid"),
            &json!({"x": 1}),
        )
        .expect("valid call signature");
        for _ in 0..3 {
            state.recent_call_signatures.push(signature.clone());
        }
        state.stop_state.repeated_call_warning =
            Some(RepeatedCallWarningState::rendered(signature.clone()));

        let (state, outcome) = observe_and_decide(&strategy, state, after_batch()).await;

        assert!(matches!(outcome, StopOutcome::Continue { .. }));
        let warning = state
            .stop_state
            .repeated_call_warning
            .expect("repeated call warning should remain rendered");
        assert_eq!(warning.signature, signature);
        assert_eq!(warning.phase, RepeatedCallWarningPhase::Rendered);
    }

    /// Also proves Task 2's windowed check cannot fire here: it reads
    /// seen_capability_output_digests, which this test never populates —
    /// not a count comparison, an empty ring.
    #[tokio::test]
    async fn non_consecutive_repetition_does_not_arm_warning() {
        let strategy = DefaultStopConditionStrategy::default();
        let mut state = LoopExecutionState::initial_for_run(&test_run_context());
        let repeated = CapabilityCallSignature::from_call(
            CapabilityId::new("demo.echo").expect("valid"),
            &json!({"x": 1}),
        )
        .expect("valid call signature");
        let intervening = CapabilityCallSignature::from_call(
            CapabilityId::new("demo.other").expect("valid"),
            &json!({"x": 2}),
        )
        .expect("valid call signature");
        state.recent_call_signatures.push(repeated.clone());
        state.recent_call_signatures.push(intervening);
        state.recent_call_signatures.push(repeated.clone());
        state.recent_call_signatures.push(repeated);

        let (state, outcome) = observe_and_decide(&strategy, state, after_batch()).await;

        assert!(matches!(outcome, StopOutcome::Continue { .. }));
        assert!(state.stop_state.repeated_call_warning.is_none());
    }

    fn output_observation(id: &str, arg: i64, digest: u64) -> CapabilityOutputObservation {
        CapabilityOutputObservation {
            signature: CapabilityCallSignature::from_call(
                CapabilityId::new(id).expect("valid"),
                &json!({ "x": arg }),
            )
            .expect("valid call signature"),
            output_digest: ContentDigest(digest),
        }
    }

    /// Proves `DefaultStopConditionStrategy` delegates its no-progress
    /// decision to `RepeatedOutputProgressStrategy` (progress.rs), which
    /// owns the window/threshold unit coverage.
    #[tokio::test]
    async fn dominant_repeated_output_reaching_threshold_returns_no_progress_detected() {
        // 24 distinct fillers (count 1 each) + 8 identical target = a
        // 32-wide window whose only dominant group is the target — same
        // call AND same output, not call alone.
        let strategy = DefaultStopConditionStrategy::default();
        let mut state = LoopExecutionState::initial_for_run(&test_run_context());
        for i in 0..24 {
            state
                .seen_capability_output_digests
                .push(output_observation("demo.filler", i, 1_000 + i as u64));
        }
        for _ in 0..8 {
            state
                .seen_capability_output_digests
                .push(output_observation("demo.echo", 1, 7));
        }
        let (_state, outcome) = observe_and_decide(&strategy, state, after_batch()).await;
        assert_eq!(
            outcome,
            StopOutcome::Stop {
                kind: StopKind::NoProgressDetected
            }
        );
    }

    /// Negative-path proof that `DefaultStopConditionStrategy` keeps
    /// delegating to `RepeatedOutputProgressStrategy` correctly for the
    /// three ways a ring can fail to be dominant: a changing digest, one
    /// call below threshold, and an empty ring. Window/threshold detail
    /// coverage lives in progress.rs; this only proves the composed
    /// strategy still returns `Continue` for each.
    #[tokio::test]
    async fn non_dominant_output_patterns_continue_through_the_stop_composer() {
        async fn continues(state: LoopExecutionState) {
            let strategy = DefaultStopConditionStrategy::default();
            let (_state, outcome) = observe_and_decide(&strategy, state, after_batch()).await;
            assert!(matches!(outcome, StopOutcome::Continue { .. }));
        }

        let mut same_signature_changing_output =
            LoopExecutionState::initial_for_run(&test_run_context());
        for i in 0..8 {
            same_signature_changing_output
                .seen_capability_output_digests
                .push(output_observation("demo.echo", 1, i as u64));
        }
        continues(same_signature_changing_output).await;

        let mut seven_identical = LoopExecutionState::initial_for_run(&test_run_context());
        for _ in 0..7 {
            seven_identical
                .seen_capability_output_digests
                .push(output_observation("demo.echo", 1, 7));
        }
        continues(seven_identical).await;

        continues(LoopExecutionState::initial_for_run(&test_run_context())).await;
    }

    #[tokio::test]
    async fn four_identical_polls_render_only_the_advisory_not_termination() {
        // #7486 anchor: 4 identical observations arm the advisory but
        // stay far below no_progress_threshold (8).
        let strategy = DefaultStopConditionStrategy::default();
        let mut state = LoopExecutionState::initial_for_run(&test_run_context());
        let observation = output_observation("demo.poll", 1, 5);
        for _ in 0..4 {
            state
                .recent_call_signatures
                .push(observation.signature.clone());
            state
                .seen_capability_output_digests
                .push(observation.clone());
        }
        let (state, outcome) = observe_and_decide(&strategy, state, after_batch()).await;
        assert!(matches!(outcome, StopOutcome::Continue { .. }));
        let warning = state
            .stop_state
            .repeated_call_warning
            .expect("advisory must render");
        assert_eq!(warning.signature, observation.signature);
    }

    #[tokio::test]
    async fn same_failure_kind_three_times_does_not_trigger_no_progress() {
        let strategy = DefaultStopConditionStrategy::default();
        let mut state = LoopExecutionState::initial_for_run(&test_run_context());
        for _ in 0..3 {
            state.recent_failure_kinds.push(LoopFailureKind::ModelError);
        }

        let (_state, outcome) = observe_and_decide(&strategy, state, after_batch()).await;

        assert!(matches!(outcome, StopOutcome::Continue { .. }));
    }

    #[tokio::test]
    async fn rejected_reply_run_triggers_invalid_model_output() {
        let strategy = DefaultStopConditionStrategy::default();
        let mut state = LoopExecutionState::initial_for_run(&test_run_context());
        let summary = TurnSummary::reply_rejected();

        for _ in 0..3 {
            state.stop_state = strategy.observe_completed_turn(&state, &summary).await;
        }
        let outcome = strategy
            .should_stop_after_observed_turn(&state, &summary)
            .await;

        assert!(matches!(
            outcome,
            StopOutcome::Stop {
                kind: StopKind::Aborted(LoopFailureKind::InvalidModelOutput)
            }
        ));
    }
}
