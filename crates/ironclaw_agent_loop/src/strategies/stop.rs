//! Stop-condition strategy contract.

use async_trait::async_trait;
use ironclaw_turns::{LoopFailureKind, LoopMessageRef, LoopResultRef};

use crate::state::{LoopExecutionState, StopStrategyState};

/// Decides whether the loop should stop after the current turn finishes.
///
/// Implementations return a new `stop_state` slot value. Async because
/// future strategies may consult host state for milestone tracking.
#[async_trait]
pub(crate) trait StopConditionStrategy: Send + Sync {
    /// Called after a turn completes.
    async fn should_stop_after_turn(
        &self,
        state: &LoopExecutionState,
        just_completed: &TurnSummary,
    ) -> StopOutcome;
}

#[allow(dead_code)]
fn assert_stop_condition_strategy_object_safe(_: &dyn StopConditionStrategy) {}

/// Loop-side projection of what just happened in the completed turn.
///
/// This carries refs only. Strategies that need content must read it through
/// host ports so host-side redaction and scope policy remain authoritative.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct TurnSummary {
    pub kind: TurnEndKind,
    pub assistant_message_ref: Option<LoopMessageRef>,
    pub batch_result_refs: Vec<LoopResultRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub(crate) enum TurnEndKind {
    /// The model returned a reply and no capability batch executed this turn.
    ReplyOnly,
    /// The model returned capability calls and the listed refs are the
    /// finalized batch outcomes for this turn.
    AfterCapabilityBatch,
}

/// Strategy decision plus the new `stop_state` slot value.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub(crate) enum StopOutcome {
    Continue {
        stop: StopStrategyState,
    },
    /// Carries the final stop slot for checkpoint, observability, and final
    /// state assembly even though no later strategy consumes it after exit.
    Stop {
        stop: StopStrategyState,
        kind: StopKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub(crate) enum StopKind {
    /// Strategy is satisfied; the executor maps this to graceful completion.
    GracefulStop,
    /// Safety-net escape for repeated calls or repeated failures; default
    /// logic should read `state.recent_call_signatures` and
    /// `state.recent_failure_kinds`.
    NoProgressDetected,
    /// Strategy aborts with an explicit failure kind.
    Aborted(LoopFailureKind),
}

/// Reference baseline `StopConditionStrategy`, including repetition and
/// no-progress safety-net escapes:
///
/// 1. **Graceful terminate-hint**: every result in the just-completed batch
///    asked to terminate → `Stop { GracefulStop }`.
/// 2. **Repetition escape**: the same `CapabilityCallSignature` is observed
///    in `repetition_threshold` (default 3) of the last `repetition_window`
///    (default 5) iterations → `Stop { NoProgressDetected }`.
/// 3. **Failure-run escape**: the same `LoopFailureKind` appears
///    `failure_run_threshold` (default 3) times in a row →
///    `Stop { NoProgressDetected }`.
///
/// On no signal, returns `Continue` with `turns_completed += 1`.
#[derive(Debug, Clone, Copy)]
pub struct DefaultStopConditionStrategy {
    /// Window size for the "same call signature ≥ N times" check.
    pub repetition_window: usize,
    /// Min repeated count within the window to trigger `NoProgressDetected`.
    pub repetition_threshold: usize,
    /// Min trailing run length of identical failure kinds to trigger
    /// `NoProgressDetected`.
    pub failure_run_threshold: usize,
}

impl Default for DefaultStopConditionStrategy {
    fn default() -> Self {
        Self {
            repetition_window: 5,
            repetition_threshold: 3,
            failure_run_threshold: 3,
        }
    }
}

#[async_trait]
impl StopConditionStrategy for DefaultStopConditionStrategy {
    async fn should_stop_after_turn(
        &self,
        state: &LoopExecutionState,
        just_completed: &TurnSummary,
    ) -> StopOutcome {
        // Bump `turns_completed` regardless of stop/continue — every
        // completed turn counts.
        let next = StopStrategyState {
            turns_completed: state.stop_state.turns_completed.saturating_add(1),
            ..state.stop_state.clone()
        };

        // (a) graceful terminate-hint: every result in the just-completed
        // batch said terminate.
        if just_completed.kind == TurnEndKind::AfterCapabilityBatch
            && state.stop_state.last_batch_total > 0
            && state.stop_state.terminate_hints_in_last_batch == state.stop_state.last_batch_total
        {
            return StopOutcome::Stop {
                stop: next,
                kind: StopKind::GracefulStop,
            };
        }

        // (b) repetition escape — same call signature observed in
        // `repetition_threshold` of the last `repetition_window` iterations.
        if state
            .recent_call_signatures
            .most_common_count_in(self.repetition_window)
            >= self.repetition_threshold
        {
            return StopOutcome::Stop {
                stop: next,
                kind: StopKind::NoProgressDetected,
            };
        }

        // (c) failure-run escape — same failure kind ≥ threshold in a row.
        if state.recent_failure_kinds.same_run_length() >= self.failure_run_threshold {
            return StopOutcome::Stop {
                stop: next,
                kind: StopKind::NoProgressDetected,
            };
        }

        StopOutcome::Continue { stop: next }
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use ironclaw_turns::{LoopMessageRef, LoopResultRef};
    use serde_json::json;

    use super::*;

    #[test]
    fn stop_condition_strategy_is_object_safe() {
        struct AlwaysContinue;

        #[async_trait]
        impl StopConditionStrategy for AlwaysContinue {
            async fn should_stop_after_turn(
                &self,
                _: &LoopExecutionState,
                _: &TurnSummary,
            ) -> StopOutcome {
                StopOutcome::Continue {
                    stop: StopStrategyState::default(),
                }
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
        };

        let serialized = serde_json::to_string(&summary).unwrap();
        let deserialized = serde_json::from_str::<TurnSummary>(&serialized).unwrap();

        assert_eq!(deserialized, summary);
    }

    #[test]
    fn stop_outcome_round_trips_through_json() {
        let outcome = StopOutcome::Stop {
            stop: StopStrategyState {
                turns_completed: 3,
                terminate_hints_in_last_batch: 1,
                last_batch_total: 2,
            },
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

        let continue_outcome = StopOutcome::Continue {
            stop: StopStrategyState::default(),
        };
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
        use ironclaw_host_api::{CapabilityId, TenantId, ThreadId};
        use ironclaw_turns::{
            AgentLoopDriverDescriptor, LoopFailureKind, LoopMessageRef, RunProfileId,
            RunProfileVersion, TurnId, TurnRunId, TurnScope,
            run_profile::{
                CancellationPolicy, CapabilitySurfaceProfileId, CheckpointPolicy,
                CheckpointSchemaId, ConcurrencyClass, ContextProfileId, LoopDriverId,
                LoopRunContext, ModelProfileId, RedactedRunProfileProvenance, ResolvedRunProfile,
                ResourceBudgetPolicy, ResourceBudgetTier, RunClassId, RunProfileFingerprint,
                RuntimeProfileConstraints, SchedulingClass, SteeringPolicy,
            },
        };
        use serde_json::json;

        use super::super::{
            DefaultStopConditionStrategy, StopConditionStrategy, StopKind, StopOutcome,
            TurnEndKind, TurnSummary,
        };
        use crate::state::{CapabilityCallSignature, LoopExecutionState, StopStrategyState};

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
                context_profile_id: ContextProfileId::new("default_stop_test_context")
                    .expect("valid"),
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
                },
                resource_budget_policy: ResourceBudgetPolicy {
                    tier: ResourceBudgetTier::new("default_stop_test_tier").expect("valid"),
                    max_model_calls: 32,
                    max_capability_invocations: 64,
                },
                personal_context_policy:
                    ironclaw_turns::run_profile::PersonalContextPolicy::Excluded,
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
                assistant_message_ref: Some(
                    LoopMessageRef::new("msg:default-stop").expect("valid"),
                ),
                batch_result_refs: Vec::new(),
            }
        }

        #[test]
        fn defaults_match_documented_baseline() {
            let strategy = DefaultStopConditionStrategy::default();
            assert_eq!(strategy.repetition_window, 5);
            assert_eq!(strategy.repetition_threshold, 3);
            assert_eq!(strategy.failure_run_threshold, 3);
        }

        #[tokio::test]
        async fn no_signal_continues_with_turns_completed_incremented() {
            let strategy = DefaultStopConditionStrategy::default();
            let mut state = LoopExecutionState::initial_for_run(&test_run_context());
            state.stop_state = StopStrategyState {
                turns_completed: 4,
                terminate_hints_in_last_batch: 0,
                last_batch_total: 0,
            };

            let outcome = strategy
                .should_stop_after_turn(&state, &after_batch())
                .await;

            match outcome {
                StopOutcome::Continue { stop } => {
                    assert_eq!(stop.turns_completed, 5);
                }
                other => panic!("expected Continue, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn all_results_terminate_hint_returns_graceful_stop() {
            let strategy = DefaultStopConditionStrategy::default();
            let mut state = LoopExecutionState::initial_for_run(&test_run_context());
            state.stop_state = StopStrategyState {
                turns_completed: 1,
                terminate_hints_in_last_batch: 3,
                last_batch_total: 3,
            };

            let outcome = strategy
                .should_stop_after_turn(&state, &after_batch())
                .await;

            match outcome {
                StopOutcome::Stop { stop, kind } => {
                    assert_eq!(stop.turns_completed, 2);
                    assert_eq!(kind, StopKind::GracefulStop);
                }
                other => panic!("expected Stop GracefulStop, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn terminate_hint_ignored_when_batch_was_empty() {
            let strategy = DefaultStopConditionStrategy::default();
            let mut state = LoopExecutionState::initial_for_run(&test_run_context());
            // last_batch_total == 0: no batch this turn — strategy must not
            // graceful-stop on a vacuous "all-terminated" check.
            state.stop_state = StopStrategyState {
                turns_completed: 0,
                terminate_hints_in_last_batch: 0,
                last_batch_total: 0,
            };

            let outcome = strategy
                .should_stop_after_turn(
                    &state,
                    &TurnSummary {
                        kind: TurnEndKind::ReplyOnly,
                        assistant_message_ref: None,
                        batch_result_refs: Vec::new(),
                    },
                )
                .await;

            assert!(matches!(outcome, StopOutcome::Continue { .. }));
        }

        #[tokio::test]
        async fn same_signature_three_times_triggers_no_progress() {
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

            let outcome = strategy
                .should_stop_after_turn(&state, &after_batch())
                .await;

            assert!(matches!(
                outcome,
                StopOutcome::Stop {
                    kind: StopKind::NoProgressDetected,
                    ..
                }
            ));
        }

        #[tokio::test]
        async fn same_failure_kind_three_times_triggers_no_progress() {
            let strategy = DefaultStopConditionStrategy::default();
            let mut state = LoopExecutionState::initial_for_run(&test_run_context());
            for _ in 0..3 {
                state.recent_failure_kinds.push(LoopFailureKind::ModelError);
            }

            let outcome = strategy
                .should_stop_after_turn(&state, &after_batch())
                .await;

            assert!(matches!(
                outcome,
                StopOutcome::Stop {
                    kind: StopKind::NoProgressDetected,
                    ..
                }
            ));
        }
    }
}
