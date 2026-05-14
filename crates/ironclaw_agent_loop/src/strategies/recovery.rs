//! `RecoveryStrategy` - decides what to do when a capability call OR a model
//! call fails with a (sanitized) error summary.
//!
//! Mutates `recovery_state` (attempt counters, fallback advance bookkeeping).
//! Async because future strategies may consult host state for circuit-breaker
//! counters, route health, etc.
//!
//! See `docs/reborn/agent-loop-skeleton.md` section 6 ("Strategy decomposition"
//! -> recovery) and section 9 ("Sanitization at the host port boundary").
//! Strategies never see raw provider errors, host paths, or secrets -
//! sanitization happens at the host port.

use async_trait::async_trait;
use ironclaw_turns::{LoopDiagnosticRef, LoopFailureKind, run_profile::LoopSafeSummary};

use crate::state::{LoopExecutionState, RecoveryStrategyState};

/// Decides what to do when a capability call OR a model call fails with a
/// (sanitized) error summary.
///
/// `&self` only - strategies are value-immutable. The new `recovery_state`
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

/// Sanitized capability error - class + safe summary string + opaque
/// diagnostic ref. Strategies never see raw provider errors, host paths,
/// or secrets (sanitization happens at the host port boundary, per master
/// doc section 9).
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

/// Sanitized model error - class + safe summary + opaque diagnostic ref.
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
/// - `Retry` - re-issue (the executor decides whether call-level or
///   iteration-level retry from `scope`; `alter` carries the strategy's
///   prompt/model hint).
/// - `SkipResult` - drop this result and continue the batch.
/// - `Abort` - return `LoopExit::Failed { reason_kind: failure_kind }`.
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

/// Strategy hint about WHAT to alter on retry. Skeleton supports prompt-shape
/// alterations only; model-route swap is reserved for the deferred
/// `ModelRouteChain` follow-up (master doc section 9).
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
        RecoveryStrategyState { attempts: 2 }
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
}
