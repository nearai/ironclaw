//! `RecoveryStrategy` — decides what to do when a capability call OR a model
//! call fails with a (sanitized) error summary.
//!
//! Mutates `recovery_state` (attempt counters, fallback advance bookkeeping).
//! Async because future strategies may consult host state for circuit-breaker
//! counters, route health, etc.
//!
//! See `docs/reborn/agent-loop-skeleton.md` §6 ("Strategy decomposition" →
//! recovery) and §9 ("Sanitization at the host port boundary"). Strategies
//! never see raw provider errors, host paths, or secrets — sanitization
//! happens at the host port.

use async_trait::async_trait;
use ironclaw_turns::{LoopDiagnosticRef, LoopFailureKind};

use crate::state::{LoopExecutionState, RecoveryStrategyState};

/// Decides what to do when a capability call OR a model call fails with a
/// (sanitized) error summary.
///
/// `&self` only — strategies are value-immutable. The new `recovery_state`
/// slot value is carried in the returned [`RecoveryOutcome`]; the executor
/// swaps it into the next whole state.
#[async_trait]
pub(crate) trait RecoveryStrategy: Send + Sync {
    async fn on_capability_error(
        &self,
        state: &LoopExecutionState,
        err: &CapabilityErrorSummary,
    ) -> RecoveryOutcome;

    async fn on_model_error(
        &self,
        state: &LoopExecutionState,
        err: &ModelErrorSummary,
    ) -> RecoveryOutcome;
}

/// Sanitized capability error — class + safe summary string + opaque
/// diagnostic ref. Strategies never see raw provider errors, host paths,
/// or secrets (sanitization happens at the host port boundary, per master
/// doc §9).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CapabilityErrorSummary {
    pub(crate) class: CapabilityErrorClass,
    pub(crate) safe_summary: String,
    pub(crate) diagnostic_ref: Option<LoopDiagnosticRef>,
}

/// Wire-stable capability error classification. Snake_case names appear in
/// checkpoints and observability events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CapabilityErrorClass {
    Transient,
    Permanent,
    InputInvalid,
    PolicyDenied,
    Unavailable,
    Internal,
}

/// Sanitized model error — class + safe summary + opaque diagnostic ref.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ModelErrorSummary {
    pub(crate) class: ModelErrorClass,
    pub(crate) safe_summary: String,
    pub(crate) diagnostic_ref: Option<LoopDiagnosticRef>,
}

/// Wire-stable model error classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ModelErrorClass {
    Transient,
    ContextOverflow,
    ContentFiltered,
    Unavailable,
    Internal,
}

/// Strategy decision plus the new `recovery_state` slot value.
///
/// Variants:
/// - `Retry` — re-issue (the executor decides whether call-level or
///   iteration-level retry; `alter` carries the strategy's hint).
/// - `SkipResult` — drop this result and continue the batch.
/// - `Abort` — return `LoopExit::Failed { reason_kind: failure_kind }`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub(crate) enum RecoveryOutcome {
    Retry {
        recovery: RecoveryStrategyState,
        alter: Option<RetryAlteration>,
    },
    SkipResult {
        recovery: RecoveryStrategyState,
    },
    Abort {
        recovery: RecoveryStrategyState,
        failure_kind: LoopFailureKind,
    },
}

/// Reference baseline `RecoveryStrategy`: bounded retry per error class with
/// exponential backoff.
///
/// Per master spec §10 ("Production-safe escape" — per-error retry budget),
/// this strategy:
/// - Skips `PolicyDenied` so the model can try another authorized tool.
/// - Aborts immediately on `Permanent`, `InputInvalid`, `ContextOverflow`,
///   and `ContentFiltered` (no retry will help).
/// - Retries `Transient`, `Unavailable`, and `Internal` up to
///   [`Self::max_attempts_per_class`] times with `Backoff` alteration, then
///   aborts with the originating error class.
///
/// See `docs/reborn/agent-loop-skeleton.md` §6 ("The nine strategies" →
/// `RecoveryStrategy`) and §10 ("Production-safe escape").
#[derive(Debug, Clone, Copy)]
pub struct DefaultRecoveryStrategy {
    /// Max retries per error class before giving up. Default `2`.
    pub max_attempts_per_class: u32,
}

impl Default for DefaultRecoveryStrategy {
    fn default() -> Self {
        Self {
            max_attempts_per_class: 2,
        }
    }
}

#[async_trait]
impl RecoveryStrategy for DefaultRecoveryStrategy {
    async fn on_capability_error(
        &self,
        state: &LoopExecutionState,
        err: &CapabilityErrorSummary,
    ) -> RecoveryOutcome {
        let next = state.recovery_state.with_incremented_attempts();
        let kind = capability_error_to_failure_kind(err.class);
        match err.class {
            CapabilityErrorClass::PolicyDenied => RecoveryOutcome::SkipResult { recovery: next },
            CapabilityErrorClass::Permanent | CapabilityErrorClass::InputInvalid => {
                RecoveryOutcome::Abort {
                    recovery: next,
                    failure_kind: kind,
                }
            }
            CapabilityErrorClass::Transient
            | CapabilityErrorClass::Unavailable
            | CapabilityErrorClass::Internal => {
                if state.recovery_state.attempts >= self.max_attempts_per_class {
                    RecoveryOutcome::Abort {
                        recovery: next,
                        failure_kind: kind,
                    }
                } else {
                    RecoveryOutcome::Retry {
                        recovery: next,
                        alter: Some(RetryAlteration::Backoff {
                            delay: backoff_for(state.recovery_state.attempts),
                        }),
                    }
                }
            }
        }
    }

    async fn on_model_error(
        &self,
        state: &LoopExecutionState,
        err: &ModelErrorSummary,
    ) -> RecoveryOutcome {
        let next = state.recovery_state.with_incremented_attempts();
        let kind = model_error_to_failure_kind(err.class);
        match err.class {
            // No retry will help — abort with the originating class.
            ModelErrorClass::ContextOverflow | ModelErrorClass::ContentFiltered => {
                RecoveryOutcome::Abort {
                    recovery: next,
                    failure_kind: kind,
                }
            }
            ModelErrorClass::Transient
            | ModelErrorClass::Unavailable
            | ModelErrorClass::Internal => {
                if state.recovery_state.attempts >= self.max_attempts_per_class {
                    RecoveryOutcome::Abort {
                        recovery: next,
                        failure_kind: kind,
                    }
                } else {
                    RecoveryOutcome::Retry {
                        recovery: next,
                        alter: Some(RetryAlteration::Backoff {
                            delay: backoff_for(state.recovery_state.attempts),
                        }),
                    }
                }
            }
        }
    }
}

/// Maps a sanitized capability error class to the loop-level failure kind that
/// the executor surfaces in `LoopExit::Failed { reason_kind }`.
fn capability_error_to_failure_kind(class: CapabilityErrorClass) -> LoopFailureKind {
    match class {
        CapabilityErrorClass::PolicyDenied => LoopFailureKind::PolicyDenied,
        CapabilityErrorClass::Transient
        | CapabilityErrorClass::Permanent
        | CapabilityErrorClass::InputInvalid
        | CapabilityErrorClass::Unavailable
        | CapabilityErrorClass::Internal => LoopFailureKind::CapabilityProtocolError,
    }
}

/// Maps a sanitized model error class to the loop-level failure kind.
fn model_error_to_failure_kind(class: ModelErrorClass) -> LoopFailureKind {
    match class {
        ModelErrorClass::Transient
        | ModelErrorClass::ContextOverflow
        | ModelErrorClass::ContentFiltered
        | ModelErrorClass::Unavailable
        | ModelErrorClass::Internal => LoopFailureKind::ModelError,
    }
}

/// Exponential backoff for retry attempts: `250ms × 2^attempt`, capped at 5s.
///
/// Strictly monotonic in `attempt` until the 5s cap kicks in. The executor
/// honors this as a sleep before re-issuing the call.
fn backoff_for(attempt: u32) -> std::time::Duration {
    let shift = attempt.min(5);
    let ms = 250u64.saturating_mul(1u64 << shift);
    std::time::Duration::from_millis(ms.min(5_000))
}

/// Strategy hint about WHAT to alter on retry. Skeleton supports prompt-shape
/// alterations only; model-route swap is reserved for the deferred
/// `ModelRouteChain` follow-up (master doc §9).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "alteration")]
pub(crate) enum RetryAlteration {
    /// Shrink context for the next attempt (e.g. on context-overflow).
    ShrinkContext { drop_messages: u32 },
    /// Backoff before retry (executor honors as a sleep).
    Backoff { delay: std::time::Duration },
    /// Reserved for future `ModelRouteChain` landing. Skeleton executor MUST
    /// reject this alteration with `LoopFailureKind::DriverBug` until the
    /// chain mechanism lands.
    AdvanceFallback,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// Compile-time object-safety check.
    #[allow(dead_code)]
    fn _check(_: &dyn RecoveryStrategy) {}

    fn sample_recovery() -> RecoveryStrategyState {
        RecoveryStrategyState { attempts: 2 }
    }

    #[test]
    fn capability_error_class_round_trips_snake_case() {
        for (variant, wire) in [
            (CapabilityErrorClass::Transient, "transient"),
            (CapabilityErrorClass::Permanent, "permanent"),
            (CapabilityErrorClass::InputInvalid, "input_invalid"),
            (CapabilityErrorClass::PolicyDenied, "policy_denied"),
            (CapabilityErrorClass::Unavailable, "unavailable"),
            (CapabilityErrorClass::Internal, "internal"),
        ] {
            let value = serde_json::to_value(variant).expect("serialize");
            assert_eq!(value, serde_json::json!(wire));
            let restored: CapabilityErrorClass =
                serde_json::from_value(value).expect("deserialize");
            assert_eq!(restored, variant);
        }
    }

    #[test]
    fn model_error_class_round_trips_snake_case() {
        for (variant, wire) in [
            (ModelErrorClass::Transient, "transient"),
            (ModelErrorClass::ContextOverflow, "context_overflow"),
            (ModelErrorClass::ContentFiltered, "content_filtered"),
            (ModelErrorClass::Unavailable, "unavailable"),
            (ModelErrorClass::Internal, "internal"),
        ] {
            let value = serde_json::to_value(variant).expect("serialize");
            assert_eq!(value, serde_json::json!(wire));
            let restored: ModelErrorClass = serde_json::from_value(value).expect("deserialize");
            assert_eq!(restored, variant);
        }
    }

    #[test]
    fn capability_error_summary_round_trips() {
        let summary = CapabilityErrorSummary {
            class: CapabilityErrorClass::Transient,
            safe_summary: "upstream timed out".to_string(),
            diagnostic_ref: Some(LoopDiagnosticRef::new("diag:cap-1").expect("valid")),
        };
        let value = serde_json::to_value(&summary).expect("serialize");
        let restored: CapabilityErrorSummary = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, summary);
    }

    #[test]
    fn model_error_summary_round_trips() {
        let summary = ModelErrorSummary {
            class: ModelErrorClass::ContextOverflow,
            safe_summary: "context window exceeded".to_string(),
            diagnostic_ref: None,
        };
        let value = serde_json::to_value(&summary).expect("serialize");
        let restored: ModelErrorSummary = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, summary);
    }

    #[test]
    fn retry_alteration_shrink_context_round_trips() {
        let alteration = RetryAlteration::ShrinkContext { drop_messages: 4 };
        let value = serde_json::to_value(&alteration).expect("serialize");
        let restored: RetryAlteration = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, alteration);
        match restored {
            RetryAlteration::ShrinkContext { drop_messages } => {
                assert_eq!(drop_messages, 4)
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn retry_alteration_backoff_round_trips() {
        let alteration = RetryAlteration::Backoff {
            delay: Duration::from_millis(250),
        };
        let value = serde_json::to_value(&alteration).expect("serialize");
        let restored: RetryAlteration = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, alteration);
        match restored {
            RetryAlteration::Backoff { delay } => {
                assert_eq!(delay, Duration::from_millis(250))
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn retry_alteration_advance_fallback_round_trips() {
        let alteration = RetryAlteration::AdvanceFallback;
        let value = serde_json::to_value(&alteration).expect("serialize");
        let restored: RetryAlteration = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, alteration);
    }

    #[test]
    fn recovery_outcome_retry_carries_recovery_slot_and_optional_alteration() {
        let outcome = RecoveryOutcome::Retry {
            recovery: sample_recovery(),
            alter: Some(RetryAlteration::ShrinkContext { drop_messages: 2 }),
        };
        let value = serde_json::to_value(&outcome).expect("serialize");
        let restored: RecoveryOutcome = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, outcome);
        match restored {
            RecoveryOutcome::Retry { recovery, alter } => {
                assert_eq!(recovery, sample_recovery());
                assert_eq!(
                    alter,
                    Some(RetryAlteration::ShrinkContext { drop_messages: 2 })
                );
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn recovery_outcome_skip_result_carries_recovery_slot() {
        let outcome = RecoveryOutcome::SkipResult {
            recovery: sample_recovery(),
        };
        let value = serde_json::to_value(&outcome).expect("serialize");
        let restored: RecoveryOutcome = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, outcome);
        match restored {
            RecoveryOutcome::SkipResult { recovery } => {
                assert_eq!(recovery, sample_recovery())
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn recovery_outcome_abort_carries_recovery_slot_and_failure_kind() {
        let outcome = RecoveryOutcome::Abort {
            recovery: sample_recovery(),
            failure_kind: LoopFailureKind::NoProgressDetected,
        };
        let value = serde_json::to_value(&outcome).expect("serialize");
        let restored: RecoveryOutcome = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, outcome);
        match restored {
            RecoveryOutcome::Abort {
                recovery,
                failure_kind,
            } => {
                assert_eq!(recovery, sample_recovery());
                assert_eq!(failure_kind, LoopFailureKind::NoProgressDetected);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    mod default_recovery_strategy {
        use ironclaw_host_api::{TenantId, ThreadId};
        use ironclaw_turns::{
            AgentLoopDriverDescriptor, RunProfileId, RunProfileVersion, TurnId, TurnRunId,
            TurnScope,
            run_profile::{
                CancellationPolicy, CapabilitySurfaceProfileId, CheckpointPolicy,
                CheckpointSchemaId, ConcurrencyClass, ContextProfileId, LoopDriverId,
                LoopRunContext, ModelProfileId, RedactedRunProfileProvenance, ResolvedRunProfile,
                ResourceBudgetPolicy, ResourceBudgetTier, RunClassId, RunProfileFingerprint,
                RuntimeProfileConstraints, SchedulingClass, SteeringPolicy,
            },
        };

        use super::super::{
            CapabilityErrorClass, CapabilityErrorSummary, DefaultRecoveryStrategy, ModelErrorClass,
            ModelErrorSummary, RecoveryOutcome, RecoveryStrategy, RetryAlteration, backoff_for,
        };
        use crate::state::{LoopExecutionState, RecoveryStrategyState};
        use ironclaw_turns::LoopFailureKind;

        fn test_run_context() -> LoopRunContext {
            let scope = TurnScope::new(
                TenantId::new("tenant-default-recovery").expect("valid"),
                None,
                None,
                ThreadId::new("thread-default-recovery").expect("valid"),
            );
            let descriptor = AgentLoopDriverDescriptor {
                id: LoopDriverId::new("default_recovery_test_driver").expect("valid"),
                version: RunProfileVersion::new(1),
                checkpoint_schema_id: Some(
                    CheckpointSchemaId::new("default_recovery_test_checkpoint").expect("valid"),
                ),
                checkpoint_schema_version: Some(RunProfileVersion::new(1)),
            };
            let resolved_run_profile = ResolvedRunProfile {
                run_class_id: RunClassId::new("default_recovery_test_class").expect("valid"),
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
                model_profile_id: ModelProfileId::new("default_recovery_test_model")
                    .expect("valid"),
                capability_surface_profile_id: CapabilitySurfaceProfileId::new(
                    "default_recovery_test_capabilities",
                )
                .expect("valid"),
                context_profile_id: ContextProfileId::new("default_recovery_test_context")
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
                    tier: ResourceBudgetTier::new("default_recovery_test_tier").expect("valid"),
                    max_model_calls: 32,
                    max_capability_invocations: 64,
                },
                runtime_constraints: RuntimeProfileConstraints {
                    allow_raw_runtime_backend_selection: false,
                    allow_broad_capability_surface: false,
                },
                runner_pool_id: None,
                scheduling_class: SchedulingClass::new("interactive").expect("valid"),
                concurrency_class: ConcurrencyClass::new("thread_serial").expect("valid"),
                resolution_fingerprint: RunProfileFingerprint::new(
                    "default-recovery-test-fingerprint",
                )
                .expect("valid"),
                provenance: RedactedRunProfileProvenance {
                    sources: vec![],
                    effective_privileges: vec![],
                },
            };
            LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
        }

        fn state_with_attempts(attempts: u32) -> LoopExecutionState {
            let mut state = LoopExecutionState::initial_for_run(&test_run_context());
            state.recovery_state = RecoveryStrategyState { attempts };
            state
        }

        fn cap_err(class: CapabilityErrorClass) -> CapabilityErrorSummary {
            CapabilityErrorSummary {
                class,
                safe_summary: "test".to_string(),
                diagnostic_ref: None,
            }
        }

        fn model_err(class: ModelErrorClass) -> ModelErrorSummary {
            ModelErrorSummary {
                class,
                safe_summary: "test".to_string(),
                diagnostic_ref: None,
            }
        }

        #[test]
        fn default_max_attempts_is_two() {
            assert_eq!(DefaultRecoveryStrategy::default().max_attempts_per_class, 2);
        }

        #[tokio::test]
        async fn capability_permanent_aborts_immediately() {
            let strategy = DefaultRecoveryStrategy::default();
            let state = state_with_attempts(0);

            let outcome = strategy
                .on_capability_error(&state, &cap_err(CapabilityErrorClass::Permanent))
                .await;

            assert!(matches!(
                outcome,
                RecoveryOutcome::Abort {
                    failure_kind: LoopFailureKind::CapabilityProtocolError,
                    ..
                }
            ));
        }

        #[tokio::test]
        async fn capability_input_invalid_aborts_immediately() {
            let strategy = DefaultRecoveryStrategy::default();
            let state = state_with_attempts(0);

            let outcome = strategy
                .on_capability_error(&state, &cap_err(CapabilityErrorClass::InputInvalid))
                .await;

            assert!(matches!(outcome, RecoveryOutcome::Abort { .. }));
        }

        #[tokio::test]
        async fn capability_policy_denied_skips_result() {
            let strategy = DefaultRecoveryStrategy::default();
            let state = state_with_attempts(0);

            let outcome = strategy
                .on_capability_error(&state, &cap_err(CapabilityErrorClass::PolicyDenied))
                .await;

            match outcome {
                RecoveryOutcome::SkipResult { recovery } => {
                    assert_eq!(recovery.attempts, 1);
                }
                other => panic!("expected SkipResult, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn capability_transient_retries_then_aborts_at_budget() {
            let strategy = DefaultRecoveryStrategy::default();

            for attempts in 0..2 {
                let state = state_with_attempts(attempts);
                let outcome = strategy
                    .on_capability_error(&state, &cap_err(CapabilityErrorClass::Transient))
                    .await;
                assert!(
                    matches!(
                        outcome,
                        RecoveryOutcome::Retry {
                            alter: Some(RetryAlteration::Backoff { .. }),
                            ..
                        }
                    ),
                    "expected retry at attempts={attempts}, got {outcome:?}"
                );
            }

            let state = state_with_attempts(2);
            let outcome = strategy
                .on_capability_error(&state, &cap_err(CapabilityErrorClass::Transient))
                .await;
            assert!(matches!(
                outcome,
                RecoveryOutcome::Abort {
                    failure_kind: LoopFailureKind::CapabilityProtocolError,
                    ..
                }
            ));
        }

        #[tokio::test]
        async fn model_context_overflow_aborts_immediately() {
            let strategy = DefaultRecoveryStrategy::default();
            let state = state_with_attempts(0);

            let outcome = strategy
                .on_model_error(&state, &model_err(ModelErrorClass::ContextOverflow))
                .await;

            assert!(matches!(
                outcome,
                RecoveryOutcome::Abort {
                    failure_kind: LoopFailureKind::ModelError,
                    ..
                }
            ));
        }

        #[tokio::test]
        async fn model_transient_retries_then_aborts_at_budget() {
            let strategy = DefaultRecoveryStrategy::default();

            let state = state_with_attempts(0);
            let outcome = strategy
                .on_model_error(&state, &model_err(ModelErrorClass::Transient))
                .await;
            assert!(matches!(
                outcome,
                RecoveryOutcome::Retry {
                    alter: Some(RetryAlteration::Backoff { .. }),
                    ..
                }
            ));

            let state = state_with_attempts(2);
            let outcome = strategy
                .on_model_error(&state, &model_err(ModelErrorClass::Transient))
                .await;
            assert!(matches!(outcome, RecoveryOutcome::Abort { .. }));
        }

        #[test]
        fn backoff_increases_with_attempt_until_cap() {
            let zero = backoff_for(0);
            let one = backoff_for(1);
            let two = backoff_for(2);
            assert!(one > zero, "expected backoff(1) > backoff(0)");
            assert!(two > one, "expected backoff(2) > backoff(1)");

            let cap = std::time::Duration::from_millis(5_000);
            assert!(backoff_for(10) <= cap);
            assert!(backoff_for(99) <= cap);
        }
    }
}
