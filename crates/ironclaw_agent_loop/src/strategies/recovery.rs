//! `RecoveryStrategy` — decides what to do when a capability call OR a model
//! call fails with a (sanitized) error summary.
//!
//! Mutates `recovery_state` (attempt counters, fallback advance bookkeeping).
//! Async because future strategies may consult host state for circuit-breaker
//! counters, route health, etc.
//!
//! Strategies never see raw provider errors, host paths, or secrets;
//! sanitization happens at the host port.

use async_trait::async_trait;
use ironclaw_turns::{LoopDiagnosticRef, LoopFailureKind, run_profile::LoopSafeSummary};

use crate::state::{LoopExecutionState, RecoveryAttemptClass, RecoveryStrategyState};

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

/// Compile-time object-safety check.
#[allow(dead_code)]
fn _recovery_strategy_object_safe(_: &dyn RecoveryStrategy) {}

/// Sanitized, strategy-visible error summary text.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(transparent)]
pub(crate) struct SanitizedStrategySummary(String);

impl SanitizedStrategySummary {
    pub(crate) fn new(summary: impl Into<String>) -> Result<Self, String> {
        let summary = summary.into();
        LoopSafeSummary::new(summary.clone()).map(|_| Self(summary))
    }

    pub(crate) fn from_trusted_static(summary: &'static str) -> Self {
        // Invariant: callers pass reviewed hard-coded summaries, so failure
        // here is a programming error in a literal rather than runtime input.
        match Self::new(summary) {
            Ok(summary) => summary,
            Err(reason) => panic!("invalid trusted static strategy summary: {reason}"),
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> serde::Deserialize<'de> for SanitizedStrategySummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let summary = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::new(summary).map_err(serde::de::Error::custom)
    }
}

/// Sanitized capability error — class + safe summary string + opaque
/// diagnostic ref. Strategies never see raw provider errors, host paths,
/// or secrets; sanitization happens at the host port boundary before recovery
/// strategy code runs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CapabilityErrorSummary {
    pub(crate) class: CapabilityErrorClass,
    pub(crate) safe_summary: SanitizedStrategySummary,
    pub(crate) diagnostic_ref: Option<LoopDiagnosticRef>,
}

/// Wire-stable capability error classification. Snake_case names appear in
/// checkpoints and observability events.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CapabilityErrorClass {
    /// Retryable capability-side failure such as timeout or temporary outage.
    Transient,
    /// Non-retryable capability-side failure.
    Permanent,
    /// Host rejected malformed capability input.
    InputInvalid,
    /// Host policy denied the capability call.
    PolicyDenied,
    /// Capability provider or backing service is unavailable.
    Unavailable,
    /// Capability host failed internally without safe caller detail.
    Internal,
}

/// Sanitized model error — class + safe summary + opaque diagnostic ref.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ModelErrorSummary {
    pub(crate) class: ModelErrorClass,
    pub(crate) safe_summary: SanitizedStrategySummary,
    pub(crate) diagnostic_ref: Option<LoopDiagnosticRef>,
}

/// Wire-stable model error classification.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ModelErrorClass {
    /// Retryable model/provider failure such as timeout or temporary outage.
    Transient,
    /// Prompt/context exceeded the selected model's limits.
    ContextOverflow,
    /// Provider rejected or filtered the content.
    ContentFiltered,
    /// Model route, credentials, or provider is unavailable.
    Unavailable,
    /// Model gateway failed internally without safe caller detail.
    Internal,
}

/// Strategy decision plus the new `recovery_state` slot value.
///
/// Variants:
/// - `Retry` — re-issue (the executor decides whether call-level or
///   iteration-level retry from `scope`; `alter` carries the strategy's
///   prompt/model hint).
/// - `SkipResult` — drop this result and continue the batch.
/// - `Abort` — return `LoopExit::Failed { reason_kind: failure_kind }`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub(crate) enum RecoveryOutcome {
    Retry {
        recovery: RecoveryStrategyState,
        scope: RetryScope,
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

/// Where the executor should apply a retry outcome.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RetryScope {
    /// Retry only the capability call or model call that produced the error.
    Call,
    /// Re-run the current loop iteration after rebuilding iteration context.
    Iteration,
}

/// Reference baseline `RecoveryStrategy`: bounded retry per error class with
/// exponential backoff.
///
/// This strategy:
/// - Skips `PolicyDenied` and `InputInvalid` so the model can observe the tool
///   error and try another call without consuming retry budget.
/// - Aborts immediately on `Permanent` and `ContentFiltered`.
/// - Retries capability/model transient, unavailable, and internal errors up
///   to [`Self::max_attempts_per_class`] times with `Backoff`.
/// - Retries `ContextOverflow` at iteration scope with `ShrinkContext`.
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
        let kind = capability_error_to_failure_kind(err.class);
        match err.class {
            CapabilityErrorClass::PolicyDenied | CapabilityErrorClass::InputInvalid => {
                RecoveryOutcome::SkipResult {
                    recovery: state.recovery_state.cleared_attempts(),
                }
            }
            CapabilityErrorClass::Permanent => RecoveryOutcome::Abort {
                recovery: state.recovery_state.cleared_attempts(),
                failure_kind: kind,
            },
            CapabilityErrorClass::Transient
            | CapabilityErrorClass::Unavailable
            | CapabilityErrorClass::Internal => {
                let Some(attempt_class) = capability_retry_attempt_class(err.class) else {
                    return RecoveryOutcome::Abort {
                        recovery: state.recovery_state.cleared_attempts(),
                        failure_kind: LoopFailureKind::DriverBug,
                    };
                };
                retry_or_abort(
                    state,
                    attempt_class,
                    self.max_attempts_per_class,
                    kind,
                    RetryScope::Call,
                    |attempts| {
                        Some(RetryAlteration::Backoff {
                            delay_ms: backoff_for(attempts),
                        })
                    },
                )
            }
        }
    }

    async fn on_model_error(
        &self,
        state: &LoopExecutionState,
        err: &ModelErrorSummary,
    ) -> RecoveryOutcome {
        let kind = model_error_to_failure_kind(err.class);
        match err.class {
            ModelErrorClass::ContentFiltered => RecoveryOutcome::Abort {
                recovery: state.recovery_state.cleared_attempts(),
                failure_kind: kind,
            },
            ModelErrorClass::ContextOverflow => {
                let Some(attempt_class) = model_retry_attempt_class(err.class) else {
                    return RecoveryOutcome::Abort {
                        recovery: state.recovery_state.cleared_attempts(),
                        failure_kind: LoopFailureKind::DriverBug,
                    };
                };
                retry_or_abort(
                    state,
                    attempt_class,
                    self.max_attempts_per_class,
                    kind,
                    RetryScope::Iteration,
                    |_| Some(RetryAlteration::ShrinkContext { drop_messages: 4 }),
                )
            }
            ModelErrorClass::Transient
            | ModelErrorClass::Unavailable
            | ModelErrorClass::Internal => {
                let Some(attempt_class) = model_retry_attempt_class(err.class) else {
                    return RecoveryOutcome::Abort {
                        recovery: state.recovery_state.cleared_attempts(),
                        failure_kind: LoopFailureKind::DriverBug,
                    };
                };
                retry_or_abort(
                    state,
                    attempt_class,
                    self.max_attempts_per_class,
                    kind,
                    RetryScope::Call,
                    |attempts| {
                        Some(RetryAlteration::Backoff {
                            delay_ms: backoff_for(attempts),
                        })
                    },
                )
            }
        }
    }
}

fn retry_or_abort(
    state: &LoopExecutionState,
    attempt_class: RecoveryAttemptClass,
    max_attempts_per_class: u32,
    failure_kind: LoopFailureKind,
    scope: RetryScope,
    alteration: impl FnOnce(u32) -> Option<RetryAlteration>,
) -> RecoveryOutcome {
    let attempts = state.recovery_state.attempts_for(attempt_class);
    let next = state
        .recovery_state
        .with_incremented_attempts_for(attempt_class);
    if attempts >= max_attempts_per_class {
        RecoveryOutcome::Abort {
            recovery: next,
            failure_kind,
        }
    } else {
        RecoveryOutcome::Retry {
            recovery: next,
            scope,
            alter: alteration(attempts),
        }
    }
}

fn capability_retry_attempt_class(class: CapabilityErrorClass) -> Option<RecoveryAttemptClass> {
    match class {
        CapabilityErrorClass::Transient => Some(RecoveryAttemptClass::CapabilityTransient),
        CapabilityErrorClass::Unavailable => Some(RecoveryAttemptClass::CapabilityUnavailable),
        CapabilityErrorClass::Internal => Some(RecoveryAttemptClass::CapabilityInternal),
        CapabilityErrorClass::Permanent
        | CapabilityErrorClass::InputInvalid
        | CapabilityErrorClass::PolicyDenied => None,
    }
}

fn model_retry_attempt_class(class: ModelErrorClass) -> Option<RecoveryAttemptClass> {
    match class {
        ModelErrorClass::Transient => Some(RecoveryAttemptClass::ModelTransient),
        ModelErrorClass::ContextOverflow => Some(RecoveryAttemptClass::ModelContextOverflow),
        ModelErrorClass::Unavailable => Some(RecoveryAttemptClass::ModelUnavailable),
        ModelErrorClass::Internal => Some(RecoveryAttemptClass::ModelInternal),
        ModelErrorClass::ContentFiltered => None,
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

/// Exponential backoff for retry attempts: `250ms x 2^attempt`, capped at 5s.
///
/// Strictly monotonic in `attempt` until the 5s cap kicks in. The executor
/// honors this as a sleep before re-issuing the call.
fn backoff_for(attempt: u32) -> BackoffDelayMs {
    let shift = attempt.min(5);
    let ms = 250u64.saturating_mul(1u64 << shift);
    BackoffDelayMs(ms.min(5_000))
}

/// Strategy hint about WHAT to alter on retry. Prompt-shape alteration is
/// supported; model-route swap is reserved for future fallback routing.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "alteration")]
pub(crate) enum RetryAlteration {
    /// Shrink context for the next attempt (e.g. on context-overflow).
    ShrinkContext { drop_messages: u32 },
    /// Backoff before retry (executor honors as a sleep).
    Backoff { delay_ms: BackoffDelayMs },
    /// Reserved for future `ModelRouteChain` landing. Skeleton executor MUST
    /// reject this alteration with `LoopFailureKind::DriverBug` until the
    /// chain mechanism lands.
    AdvanceFallback,
}

/// Bounded retry backoff delay in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BackoffDelayMs(u64);

impl BackoffDelayMs {
    pub(crate) const MAX_DELAY_MS: u64 = 60_000;

    pub(crate) fn new(delay_ms: u64) -> Result<Self, String> {
        if delay_ms <= Self::MAX_DELAY_MS {
            Ok(Self(delay_ms))
        } else {
            Err(format!(
                "backoff delay {delay_ms}ms exceeds max {}ms",
                Self::MAX_DELAY_MS
            ))
        }
    }

    pub(crate) fn as_u64(self) -> u64 {
        self.0
    }
}

impl serde::Serialize for BackoffDelayMs {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u64(self.0)
    }
}

impl<'de> serde::Deserialize<'de> for BackoffDelayMs {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let delay_ms = <u64 as serde::Deserialize>::deserialize(deserializer)?;
        Self::new(delay_ms).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_recovery() -> RecoveryStrategyState {
        RecoveryStrategyState::with_attempts_for(RecoveryAttemptClass::ModelTransient, 2)
    }

    #[test]
    fn sanitized_strategy_summary_serializes_as_string() {
        let summary = SanitizedStrategySummary::new("provider unavailable").expect("valid");
        let value = serde_json::to_value(&summary).expect("serialize");
        assert_eq!(value, serde_json::json!("provider unavailable"));
        let restored: SanitizedStrategySummary =
            serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored.as_str(), "provider unavailable");
    }

    #[test]
    fn sanitized_strategy_summary_rejects_unsafe_dynamic_values() {
        assert!(SanitizedStrategySummary::new("").is_err());
        assert!(SanitizedStrategySummary::new("/Users/alice/.ssh/id_rsa").is_err());
        assert!(SanitizedStrategySummary::new("provider returned sk-live-secret").is_err());
        assert!(SanitizedStrategySummary::new("a".repeat(513)).is_err());
    }

    #[test]
    fn sanitized_strategy_summary_validates_during_deserialization() {
        for unsafe_summary in [
            "",
            "/Users/alice/.ssh/id_rsa",
            "provider returned sk-live-secret",
        ] {
            let result = serde_json::from_value::<SanitizedStrategySummary>(serde_json::json!(
                unsafe_summary
            ));
            assert!(result.is_err(), "accepted unsafe summary: {unsafe_summary}");
        }

        let oversized = "a".repeat(513);
        let result =
            serde_json::from_value::<SanitizedStrategySummary>(serde_json::json!(oversized));
        assert!(result.is_err(), "accepted oversized summary");
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
            safe_summary: SanitizedStrategySummary::new("upstream timed out").expect("valid"),
            diagnostic_ref: Some(LoopDiagnosticRef::new("diag:cap-1").expect("valid")),
        };
        let value = serde_json::to_value(&summary).expect("serialize");
        assert_eq!(
            value["safe_summary"],
            serde_json::json!("upstream timed out")
        );
        let restored: CapabilityErrorSummary = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, summary);
        assert_eq!(restored.safe_summary.as_str(), "upstream timed out");
    }

    #[test]
    fn model_error_summary_round_trips() {
        let summary = ModelErrorSummary {
            class: ModelErrorClass::ContextOverflow,
            safe_summary: SanitizedStrategySummary::new("context window exceeded").expect("valid"),
            diagnostic_ref: None,
        };
        let value = serde_json::to_value(&summary).expect("serialize");
        assert_eq!(
            value["safe_summary"],
            serde_json::json!("context window exceeded")
        );
        let restored: ModelErrorSummary = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, summary);
        assert_eq!(restored.safe_summary.as_str(), "context window exceeded");
    }

    #[test]
    fn retry_scope_round_trips_snake_case() {
        for (variant, wire) in [
            (RetryScope::Call, "call"),
            (RetryScope::Iteration, "iteration"),
        ] {
            let value = serde_json::to_value(variant).expect("serialize");
            assert_eq!(value, serde_json::json!(wire));
            let restored: RetryScope = serde_json::from_value(value).expect("deserialize");
            assert_eq!(restored, variant);
        }
    }

    #[test]
    fn backoff_delay_ms_accepts_bounded_values_and_serializes_as_number() {
        let delay = BackoffDelayMs::new(250).expect("valid");
        assert_eq!(delay.as_u64(), 250);
        let value = serde_json::to_value(delay).expect("serialize");
        assert_eq!(value, serde_json::json!(250));
        let restored: BackoffDelayMs = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, delay);
    }

    #[test]
    fn backoff_delay_ms_rejects_values_above_max() {
        let too_large = BackoffDelayMs::MAX_DELAY_MS + 1;
        assert!(BackoffDelayMs::new(too_large).is_err());
        let result = serde_json::from_value::<BackoffDelayMs>(serde_json::json!(too_large));
        assert!(result.is_err());
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
            delay_ms: BackoffDelayMs::new(250).expect("valid"),
        };
        let value = serde_json::to_value(&alteration).expect("serialize");
        assert_eq!(value["delay_ms"], serde_json::json!(250));
        let restored: RetryAlteration = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, alteration);
        match restored {
            RetryAlteration::Backoff { delay_ms } => {
                assert_eq!(delay_ms.as_u64(), 250)
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
            scope: RetryScope::Call,
            alter: Some(RetryAlteration::ShrinkContext { drop_messages: 2 }),
        };
        let value = serde_json::to_value(&outcome).expect("serialize");
        let restored: RecoveryOutcome = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, outcome);
        match restored {
            RecoveryOutcome::Retry {
                recovery,
                scope,
                alter,
            } => {
                assert_eq!(recovery, sample_recovery());
                assert_eq!(scope, RetryScope::Call);
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
            ModelErrorSummary, RecoveryOutcome, RecoveryStrategy, RetryAlteration, RetryScope,
            SanitizedStrategySummary, backoff_for,
        };
        use crate::state::{LoopExecutionState, RecoveryAttemptClass, RecoveryStrategyState};
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
                personal_context_policy:
                    ironclaw_turns::run_profile::PersonalContextPolicy::Excluded,
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

        fn state_with_no_attempts() -> LoopExecutionState {
            let mut state = LoopExecutionState::initial_for_run(&test_run_context());
            state.recovery_state = RecoveryStrategyState::default();
            state
        }

        fn state_with_attempts_for(
            attempts: u32,
            attempt_class: RecoveryAttemptClass,
        ) -> LoopExecutionState {
            let mut state = LoopExecutionState::initial_for_run(&test_run_context());
            state.recovery_state =
                RecoveryStrategyState::with_attempts_for(attempt_class, attempts);
            state
        }

        fn cap_err(class: CapabilityErrorClass) -> CapabilityErrorSummary {
            CapabilityErrorSummary {
                class,
                safe_summary: SanitizedStrategySummary::from_trusted_static("test"),
                diagnostic_ref: None,
            }
        }

        fn model_err(class: ModelErrorClass) -> ModelErrorSummary {
            ModelErrorSummary {
                class,
                safe_summary: SanitizedStrategySummary::from_trusted_static("test"),
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
            let state = state_with_no_attempts();

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
        async fn capability_input_invalid_skips_result() {
            let strategy = DefaultRecoveryStrategy::default();
            let state = state_with_no_attempts();

            let outcome = strategy
                .on_capability_error(&state, &cap_err(CapabilityErrorClass::InputInvalid))
                .await;

            match outcome {
                RecoveryOutcome::SkipResult { recovery } => {
                    assert_eq!(recovery, RecoveryStrategyState::default());
                }
                other => panic!("expected SkipResult, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn capability_policy_denied_skips_result() {
            let strategy = DefaultRecoveryStrategy::default();
            let state = state_with_no_attempts();

            let outcome = strategy
                .on_capability_error(&state, &cap_err(CapabilityErrorClass::PolicyDenied))
                .await;

            match outcome {
                RecoveryOutcome::SkipResult { recovery } => {
                    assert_eq!(recovery, RecoveryStrategyState::default());
                }
                other => panic!("expected SkipResult, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn capability_transient_retries_then_aborts_at_budget() {
            let strategy = DefaultRecoveryStrategy::default();

            for attempts in 0..2 {
                let state =
                    state_with_attempts_for(attempts, RecoveryAttemptClass::CapabilityTransient);
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

            let state = state_with_attempts_for(2, RecoveryAttemptClass::CapabilityTransient);
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
        async fn model_context_overflow_retries_with_context_shrink_then_aborts_at_budget() {
            let strategy = DefaultRecoveryStrategy::default();
            let state = state_with_no_attempts();

            let outcome = strategy
                .on_model_error(&state, &model_err(ModelErrorClass::ContextOverflow))
                .await;

            match outcome {
                RecoveryOutcome::Retry {
                    recovery,
                    scope,
                    alter,
                } => {
                    assert_eq!(
                        recovery.attempts_for(RecoveryAttemptClass::ModelContextOverflow),
                        1
                    );
                    assert_eq!(scope, RetryScope::Iteration);
                    assert_eq!(
                        alter,
                        Some(RetryAlteration::ShrinkContext { drop_messages: 4 })
                    );
                }
                other => panic!("expected context overflow retry, got {other:?}"),
            }

            let state = state_with_attempts_for(2, RecoveryAttemptClass::ModelContextOverflow);
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

            let state = state_with_no_attempts();
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

            let state = state_with_attempts_for(2, RecoveryAttemptClass::ModelTransient);
            let outcome = strategy
                .on_model_error(&state, &model_err(ModelErrorClass::Transient))
                .await;
            assert!(matches!(outcome, RecoveryOutcome::Abort { .. }));
        }

        #[tokio::test]
        async fn retry_budget_tracks_each_error_class_independently() {
            let strategy = DefaultRecoveryStrategy::default();
            let mut state = state_with_attempts_for(2, RecoveryAttemptClass::CapabilityTransient);
            state.recovery_state = state
                .recovery_state
                .with_incremented_attempts_for(RecoveryAttemptClass::CapabilityUnavailable);

            let outcome = strategy
                .on_capability_error(&state, &cap_err(CapabilityErrorClass::Transient))
                .await;

            assert!(matches!(outcome, RecoveryOutcome::Abort { .. }));
        }

        #[tokio::test]
        async fn changed_error_class_keeps_prior_attempt_budget() {
            let strategy = DefaultRecoveryStrategy::default();
            let state = state_with_attempts_for(2, RecoveryAttemptClass::CapabilityTransient);

            let outcome = strategy
                .on_capability_error(&state, &cap_err(CapabilityErrorClass::Unavailable))
                .await;

            match outcome {
                RecoveryOutcome::Retry { recovery, .. } => {
                    assert_eq!(
                        recovery.attempts_for(RecoveryAttemptClass::CapabilityTransient),
                        2
                    );
                    assert_eq!(
                        recovery.attempts_for(RecoveryAttemptClass::CapabilityUnavailable),
                        1
                    );
                }
                other => panic!("expected changed class retry, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn non_retry_paths_do_not_poison_later_retry_budget() {
            let strategy = DefaultRecoveryStrategy::default();
            let state = state_with_attempts_for(2, RecoveryAttemptClass::CapabilityTransient);

            let outcome = strategy
                .on_capability_error(&state, &cap_err(CapabilityErrorClass::PolicyDenied))
                .await;
            let RecoveryOutcome::SkipResult { recovery } = outcome else {
                panic!("expected policy denied skip");
            };

            let mut next = LoopExecutionState::initial_for_run(&test_run_context());
            next.recovery_state = recovery;
            let outcome = strategy
                .on_model_error(&next, &model_err(ModelErrorClass::Transient))
                .await;

            assert!(matches!(
                outcome,
                RecoveryOutcome::Retry { recovery, .. }
                    if recovery.attempts_for(RecoveryAttemptClass::ModelTransient) == 1
            ));
        }

        #[test]
        fn backoff_increases_with_attempt_until_cap() {
            let zero = backoff_for(0);
            let one = backoff_for(1);
            let two = backoff_for(2);
            assert!(
                one.as_u64() > zero.as_u64(),
                "expected backoff(1) > backoff(0)"
            );
            assert!(
                two.as_u64() > one.as_u64(),
                "expected backoff(2) > backoff(1)"
            );

            assert!(backoff_for(10).as_u64() <= 5_000);
            assert!(backoff_for(99).as_u64() <= 5_000);
        }
    }
}
