//! `GateHandlingStrategy` - decides what to do when a capability invocation
//! returns a gate (Approval, Auth, or Resource).
//!
//! Mutates `gate_state` (e.g. record gate fingerprints for resume).
//! Async because future strategies may consult host state for grant-history
//! or auth-flow lookups.
//!
//! See `docs/reborn/agent-loop-skeleton.md` section 6 ("Strategy decomposition"
//! -> gate handling) and section 8 ("Outcome enums"). Sanitization at the host
//! port boundary (per master doc section 9 + `contracts/turns-agent-loop.md`
//! section 6 + `contracts/lightweight-agent-loop.md` section 8) means
//! strategies never see raw input, secrets, or auth state.

use async_trait::async_trait;
use ironclaw_turns::{LoopFailureKind, LoopGateRef};

use crate::state::{GateStrategyState, LoopExecutionState};

/// Decides what to do when a capability invocation comes back with a gate.
///
/// `&self` only - strategies are value-immutable. The new `gate_state`
/// slot value is carried in the returned [`GateOutcome`]; the executor
/// swaps it into the next whole state.
#[async_trait]
pub(crate) trait GateHandlingStrategy: Send + Sync {
    async fn handle(&self, state: &LoopExecutionState, gate: &GateSummary) -> GateOutcome;
}

/// Compile-time object-safety check.
#[allow(dead_code)]
fn _gate_handling_strategy_object_safe(_: &dyn GateHandlingStrategy) {}

/// Loop-side projection of a host capability gate - kind + opaque ref only.
/// The strategy never sees raw input, secrets, or auth state (per
/// `contracts/turns-agent-loop.md` section 6 +
/// `contracts/lightweight-agent-loop.md` section 8).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct GateSummary {
    pub(crate) kind: GateKind,
    pub(crate) gate_ref: LoopGateRef,
}

/// Wire-stable gate classification. Snake_case names are part of the public
/// contract - they appear in checkpoints and observability events.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GateKind {
    Approval,
    Auth,
    Resource,
}

/// Strategy decision for a gate, plus the new `gate_state` slot value.
///
/// Variants:
/// - `Block` - the executor checkpoints (`BeforeBlock`) and returns
///   `LoopExit::Blocked`. The standard production path.
/// - `SkipAndContinue` - drop this call's result entirely and proceed with
///   the rest of the batch. Use sparingly; intended for fire-and-forget
///   tools where a missing approval is non-fatal.
/// - `Abort` - return `LoopExit::Failed { reason_kind: failure_kind }`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub(crate) enum GateOutcome {
    Block {
        gate: GateStrategyState,
    },
    SkipAndContinue {
        gate: GateStrategyState,
    },
    Abort {
        gate: GateStrategyState,
        failure_kind: LoopFailureKind,
    },
}

impl GateOutcome {
    /// Validate the outcome against the originating gate kind before an
    /// executor honors it. WS-6 executor code must call this first.
    pub(crate) fn validate_for_gate_kind(&self, kind: GateKind) -> Result<(), LoopFailureKind> {
        match (kind, self) {
            (GateKind::Approval, GateOutcome::SkipAndContinue { .. }) => {
                Err(LoopFailureKind::DriverBug)
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_gate() -> GateStrategyState {
        GateStrategyState::default()
    }

    #[test]
    fn gate_kind_round_trips_snake_case() {
        for (variant, wire) in [
            (GateKind::Approval, "approval"),
            (GateKind::Auth, "auth"),
            (GateKind::Resource, "resource"),
        ] {
            let value = serde_json::to_value(variant).expect("serialize");
            assert_eq!(value, serde_json::json!(wire));
            let restored: GateKind = serde_json::from_value(value).expect("deserialize");
            assert_eq!(restored, variant);
        }
    }

    #[test]
    fn gate_summary_round_trips() {
        let summary = GateSummary {
            kind: GateKind::Approval,
            gate_ref: LoopGateRef::new("gate:approval-demo").expect("valid"),
        };
        let value = serde_json::to_value(&summary).expect("serialize");
        let restored: GateSummary = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, summary);
    }

    #[test]
    fn gate_outcome_block_carries_gate_slot() {
        let outcome = GateOutcome::Block {
            gate: sample_gate(),
        };
        let value = serde_json::to_value(&outcome).expect("serialize");
        assert_eq!(value["gate"], serde_json::json!({}));
        let restored: GateOutcome = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, outcome);
        // Field is named `gate` and is the strategy slot type.
        match restored {
            GateOutcome::Block { gate } => assert_eq!(gate, sample_gate()),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn gate_outcome_skip_and_continue_carries_gate_slot() {
        let outcome = GateOutcome::SkipAndContinue {
            gate: sample_gate(),
        };
        let value = serde_json::to_value(&outcome).expect("serialize");
        assert_eq!(value["gate"], serde_json::json!({}));
        let restored: GateOutcome = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, outcome);
        match restored {
            GateOutcome::SkipAndContinue { gate } => assert_eq!(gate, sample_gate()),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn gate_outcome_abort_carries_gate_slot_and_failure_kind() {
        let outcome = GateOutcome::Abort {
            gate: sample_gate(),
            failure_kind: LoopFailureKind::PolicyDenied,
        };
        let value = serde_json::to_value(&outcome).expect("serialize");
        assert_eq!(value["gate"], serde_json::json!({}));
        let restored: GateOutcome = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, outcome);
        match restored {
            GateOutcome::Abort { gate, failure_kind } => {
                assert_eq!(gate, sample_gate());
                assert_eq!(failure_kind, LoopFailureKind::PolicyDenied);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn approval_gate_rejects_skip_and_continue_outcome() {
        let outcome = GateOutcome::SkipAndContinue {
            gate: sample_gate(),
        };
        assert_eq!(
            outcome.validate_for_gate_kind(GateKind::Approval),
            Err(LoopFailureKind::DriverBug)
        );
    }

    #[test]
    fn auth_and_resource_gates_allow_skip_and_continue_outcome() {
        let outcome = GateOutcome::SkipAndContinue {
            gate: sample_gate(),
        };
        assert_eq!(outcome.validate_for_gate_kind(GateKind::Auth), Ok(()));
        assert_eq!(outcome.validate_for_gate_kind(GateKind::Resource), Ok(()));
    }
}
