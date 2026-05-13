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
}
