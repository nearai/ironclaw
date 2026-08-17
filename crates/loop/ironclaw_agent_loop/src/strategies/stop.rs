//! Stop-condition strategy contract.

use async_trait::async_trait;
use ironclaw_host_api::ids::CapabilityId;
use ironclaw_host_api::turn::{LoopMessageRef, LoopResultRef};
use ironclaw_loop_contracts::LoopFailureKind;

use crate::state::{
    CapabilityCallSignature, LoopExecutionState, RepeatedCallWarningState, StopStrategyState,
};

/// Observes completed turns and decides whether the loop should stop.
///
/// Observation and terminal decision are split so the executor can always
/// account for a completed turn before any follow-up input preempts final exit.
/// Async because future strategies may consult host state for milestone
/// tracking.
#[async_trait]
pub(crate) trait StopConditionStrategy: Send + Sync {
    /// Called exactly once after a turn completes to update resumable stop
    /// state.
    async fn observe_completed_turn(
        &self,
        state: &LoopExecutionState,
        just_completed: &TurnSummary,
    ) -> StopStrategyState;

    /// Called after `observe_completed_turn` has been applied to `state`.
    async fn should_stop_after_observed_turn(
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
    #[serde(default)]
    pub capability_batch: CapabilityBatchTurnSummary,
}

impl TurnSummary {
    pub(crate) fn reply_only(reply_ref: LoopMessageRef) -> Self {
        Self {
            kind: TurnEndKind::ReplyOnly,
            assistant_message_ref: Some(reply_ref),
            batch_result_refs: Vec::new(),
            capability_batch: CapabilityBatchTurnSummary::default(),
        }
    }

    pub(crate) fn after_capability_batch(
        result_refs: Vec<LoopResultRef>,
        capability_batch: CapabilityBatchTurnSummary,
    ) -> Self {
        Self {
            kind: TurnEndKind::AfterCapabilityBatch,
            assistant_message_ref: None,
            batch_result_refs: result_refs,
            capability_batch,
        }
    }

    pub(crate) fn reply_rejected() -> Self {
        Self {
            kind: TurnEndKind::ReplyRejected,
            assistant_message_ref: None,
            batch_result_refs: Vec::new(),
            capability_batch: CapabilityBatchTurnSummary::default(),
        }
    }

    pub(crate) fn compaction_only() -> Self {
        Self {
            kind: TurnEndKind::CompactionOnly,
            assistant_message_ref: None,
            batch_result_refs: Vec::new(),
            capability_batch: CapabilityBatchTurnSummary::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CapabilityBatchTurnSummary {
    /// Number of capability invocations in the executed batch.
    pub invocation_count: u32,
    /// Count of completed results in the batch that requested natural termination.
    pub terminate_hint_count: u32,
    /// Completed-call signatures observed in this batch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observed_signatures: Vec<CapabilityCallSignature>,
}

impl CapabilityBatchTurnSummary {
    pub(crate) fn for_invocation_count(invocation_count: usize) -> Self {
        Self {
            invocation_count: invocation_count as u32,
            terminate_hint_count: 0,
            observed_signatures: Vec::new(),
        }
    }

    pub(crate) fn record_result(
        &mut self,
        signature: CapabilityCallSignature,
        terminate_hint: bool,
    ) {
        push_unique_signature(&mut self.observed_signatures, signature);
        if terminate_hint {
            self.terminate_hint_count = self.terminate_hint_count.saturating_add(1);
        }
    }
}

fn push_unique_signature(
    signatures: &mut Vec<CapabilityCallSignature>,
    signature: CapabilityCallSignature,
) {
    if !signatures.contains(&signature) {
        signatures.push(signature);
    }
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
    /// The model returned a reply that was rejected before transcript
    /// finalization.
    ReplyRejected,
    /// The turn ran proactive compaction and skipped the model call entirely.
    /// No assistant reply, no capability batch — just compaction, observe stop,
    /// then iterate. Emitted via `PromptStep::SkipModel`.
    CompactionOnly,
}

/// Strategy decision after completed-turn observation has already updated
/// `stop_state`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub(crate) enum StopOutcome {
    Continue {},
    Stop { kind: StopKind },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub(crate) enum StopKind {
    /// Strategy is satisfied; the executor maps this to graceful completion.
    GracefulStop,
    /// Explicit no-progress stop requested by a non-default strategy. The
    /// executor gives the model one recovery turn before resolving it to a
    /// typed `LoopFailureKind::NoProgressDetected` failure. The default
    /// strategy uses repeated calls only for an advisory warning.
    NoProgressDetected,
    /// Strategy aborts with an explicit failure kind.
    Aborted(LoopFailureKind),
}

/// Reference baseline `StopConditionStrategy`.
///
/// 1. **Reply completion**: a reply-only turn means the model returned its
///    assistant answer → `Stop { GracefulStop }`.
/// 2. **Graceful terminate-hint**: every result in the just-completed batch
///    asked to terminate → `Stop { GracefulStop }`.
/// 3. **Repetition advisory**: the same `CapabilityCallSignature` is observed
///    in `repetition_threshold` (default 3) consecutive iterations → render a
///    model-visible warning. Repetition never terminates the run; deterministic
///    iteration and budget strategies remain the backstop.
/// 4. **Rejected-reply escape**: reply admission rejects
///    `rejected_reply_threshold` replies in a row →
///    `Stop { Aborted(InvalidModelOutput) }`.
///
/// On no signal, returns `Continue`.
#[derive(Debug, Clone, Copy)]
pub struct DefaultStopConditionStrategy {
    /// Consecutive identical calls required before rendering an advisory.
    pub repetition_threshold: usize,
    /// Min trailing rejected replies required before aborting as invalid model
    /// output. Capability failures are deliberately not counted here:
    /// `LoopFailureKind` is too coarse to distinguish repeated failure from
    /// unrelated model attempts that happen to share the same category.
    pub rejected_reply_threshold: usize,
}

impl Default for DefaultStopConditionStrategy {
    fn default() -> Self {
        Self {
            repetition_threshold: 3,
            rejected_reply_threshold: 3,
        }
    }
}

#[async_trait]
impl StopConditionStrategy for DefaultStopConditionStrategy {
    async fn observe_completed_turn(
        &self,
        state: &LoopExecutionState,
        just_completed: &TurnSummary,
    ) -> StopStrategyState {
        // Bump `turns_completed` regardless of stop/continue — every
        // completed turn counts.
        let stop_state = StopStrategyState {
            turns_completed: state.stop_state.turns_completed.saturating_add(1),
            trailing_rejected_replies: if just_completed.kind == TurnEndKind::ReplyRejected {
                state.stop_state.trailing_rejected_replies.saturating_add(1)
            } else {
                0
            },
            // Retained in the checkpoint shape for rollback compatibility;
            // heuristic no-progress is advisory-only and no longer counted.
            trailing_no_progress_results: 0,
            // Counted only by the unbound structured-output strategy; the
            // default family leaves it at rest.
            trailing_all_failed_batches: 0,
            repeated_call_warning: state.stop_state.repeated_call_warning.clone(),
        };

        observe_repeated_call_warning(state, stop_state, self.repetition_threshold)
    }

    async fn should_stop_after_observed_turn(
        &self,
        state: &LoopExecutionState,
        just_completed: &TurnSummary,
    ) -> StopOutcome {
        // (a) reply completion: the executor already drained queued follow-up
        // input before asking the stop strategy, so a reply-only turn is
        // terminal for the default family.
        if just_completed.kind == TurnEndKind::ReplyOnly {
            return StopOutcome::Stop {
                kind: StopKind::GracefulStop,
            };
        }

        // (b) graceful terminate-hint: every result in the just-completed
        // batch said terminate.
        if just_completed.kind == TurnEndKind::AfterCapabilityBatch
            && just_completed.capability_batch.invocation_count > 0
            && just_completed.capability_batch.terminate_hint_count
                == just_completed.capability_batch.invocation_count
        {
            return StopOutcome::Stop {
                kind: StopKind::GracefulStop,
            };
        }

        // (c) rejected-reply escape — repeated rejected final-answer
        // candidates are invalid model output, not generic no-progress.
        // This threshold permits extra model calls after each rejection; keep
        // deployments with tight LLM budgets on a low value.
        if state.stop_state.trailing_rejected_replies as usize >= self.rejected_reply_threshold {
            return StopOutcome::Stop {
                kind: StopKind::Aborted(LoopFailureKind::InvalidModelOutput),
            };
        }

        StopOutcome::Continue {}
    }
}

/// Stop conditions for the unbound structured family (unbound turns
/// design §4.5): the run completes when the host-owned result tool records a
/// validated structured result — the completed-call signature in the batch
/// summary is the completion signal — and a run of all-failed capability
/// batches (repeated invalid result-tool arguments under a surface whose only
/// tool is the result tool) fails as `invalid_model_output`. Everything else
/// delegates to the default stop conditions.
#[derive(Debug, Clone)]
pub struct StructuredResultStopStrategy {
    inner: DefaultStopConditionStrategy,
    result_capability: CapabilityId,
    /// Trailing all-failed capability batches required to abort.
    all_failed_batch_threshold: usize,
}

impl StructuredResultStopStrategy {
    pub fn new(result_capability: CapabilityId) -> Self {
        Self {
            inner: DefaultStopConditionStrategy::default(),
            result_capability,
            all_failed_batch_threshold: 3,
        }
    }

    fn result_recorded(&self, summary: &TurnSummary) -> bool {
        summary.kind == TurnEndKind::AfterCapabilityBatch
            && summary
                .capability_batch
                .observed_signatures
                .iter()
                .any(|signature| signature.name == self.result_capability)
    }
}

#[async_trait]
impl StopConditionStrategy for StructuredResultStopStrategy {
    async fn observe_completed_turn(
        &self,
        state: &LoopExecutionState,
        just_completed: &TurnSummary,
    ) -> StopStrategyState {
        let mut stop_state = self
            .inner
            .observe_completed_turn(state, just_completed)
            .await;
        let all_failed = just_completed.kind == TurnEndKind::AfterCapabilityBatch
            && just_completed.capability_batch.invocation_count > 0
            && just_completed
                .capability_batch
                .observed_signatures
                .is_empty();
        stop_state.trailing_all_failed_batches = if all_failed {
            state
                .stop_state
                .trailing_all_failed_batches
                .saturating_add(1)
        } else {
            0
        };
        stop_state
    }

    async fn should_stop_after_observed_turn(
        &self,
        state: &LoopExecutionState,
        just_completed: &TurnSummary,
    ) -> StopOutcome {
        if self.result_recorded(just_completed) {
            return StopOutcome::Stop {
                kind: StopKind::GracefulStop,
            };
        }
        if state.stop_state.trailing_all_failed_batches as usize >= self.all_failed_batch_threshold
        {
            return StopOutcome::Stop {
                kind: StopKind::Aborted(LoopFailureKind::InvalidModelOutput),
            };
        }
        self.inner
            .should_stop_after_observed_turn(state, just_completed)
            .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RepeatedCallObservation {
    signature: CapabilityCallSignature,
}

fn trailing_repeated_call(
    state: &LoopExecutionState,
    threshold: usize,
) -> Option<RepeatedCallObservation> {
    let signature = state.recent_call_signatures.iter().next_back()?.clone();
    let count = state
        .recent_call_signatures
        .iter()
        .rev()
        .take_while(|candidate| **candidate == signature)
        .count();
    if count < threshold {
        return None;
    }
    Some(RepeatedCallObservation { signature })
}

fn observe_repeated_call_warning(
    state: &LoopExecutionState,
    mut stop_state: StopStrategyState,
    threshold: usize,
) -> StopStrategyState {
    let Some(repeated) = trailing_repeated_call(state, threshold) else {
        stop_state.repeated_call_warning = None;
        return stop_state;
    };

    stop_state.repeated_call_warning = match state.stop_state.repeated_call_warning.as_ref() {
        Some(existing) if existing.signature == repeated.signature => Some(
            RepeatedCallWarningState::rendered(existing.signature.clone()),
        ),
        _ => Some(RepeatedCallWarningState::pending_render(repeated.signature)),
    };
    stop_state
}

#[cfg(test)]
mod tests {
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
        use ironclaw_host_api::turn::{
            LoopMessageRef, RunProfileId, RunProfileVersion, TurnId, TurnRunId, TurnScope,
        };
        use ironclaw_loop_contracts::{
            AgentLoopDriverDescriptor, CancellationPolicy, CapabilitySurfaceProfileId,
            CheckpointPolicy, CheckpointSchemaId, ConcurrencyClass, ContextProfileId, LoopDriverId,
            LoopFailureKind, LoopRunContext, ModelProfileId, RedactedRunProfileProvenance,
            ResolvedRunProfile, ResourceBudgetPolicy, ResourceBudgetTier, RunClassId,
            RunProfileFingerprint, RuntimeProfileConstraints, SchedulingClass, SteeringPolicy,
        };
        use serde_json::json;

        use super::super::{
            CapabilityBatchTurnSummary, DefaultStopConditionStrategy, StopConditionStrategy,
            StopKind, StopOutcome, TurnEndKind, TurnSummary,
        };
        use crate::state::{
            CapabilityCallSignature, LoopExecutionState, RepeatedCallWarningPhase,
            RepeatedCallWarningState, StopStrategyState,
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
                assistant_message_ref: Some(
                    LoopMessageRef::new("msg:default-stop").expect("valid"),
                ),
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
}
