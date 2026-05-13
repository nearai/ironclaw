//! `GateHandlingStrategy` — decides what to do when a capability invocation
//! returns a gate (Approval, Auth, or Resource).
//!
//! Mutates `control_state` (e.g. record gate fingerprints for resume).
//! Async because future strategies may consult host state for grant-history
//! or auth-flow lookups.
//!
//! See `docs/reborn/agent-loop-skeleton.md` §6 ("Strategy decomposition" →
//! gate handling) and §8 ("Outcome enums"). Sanitization at the host port
//! boundary (per master doc §9 + `contracts/turns-agent-loop.md` §6 +
//! `contracts/lightweight-agent-loop.md` §8) means strategies never see
//! raw input, secrets, or auth state.

use async_trait::async_trait;
use ironclaw_turns::{LoopFailureKind, LoopGateRef};

use crate::state::{ControlStrategyState, LoopExecutionState};

/// Decides what to do when a capability invocation comes back with a gate.
///
/// `&self` only — strategies are value-immutable. The new `control_state`
/// slot value is carried in the returned [`GateOutcome`]; the executor
/// swaps it into the next whole state.
#[async_trait]
pub trait GateHandlingStrategy: Send + Sync {
    async fn handle(&self, state: &LoopExecutionState, gate: &GateSummary) -> GateOutcome;
}

/// Loop-side projection of a host capability gate — kind + opaque ref only.
/// The strategy never sees raw input, secrets, or auth state (per
/// `contracts/turns-agent-loop.md` §6 + `contracts/lightweight-agent-loop.md`
/// §8).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GateSummary {
    pub kind: GateKind,
    pub gate_ref: LoopGateRef,
}

/// Wire-stable gate classification. Snake_case names are part of the public
/// contract — they appear in checkpoints and observability events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateKind {
    Approval,
    Auth,
    Resource,
}

/// Strategy decision for a gate, plus the new `control_state` slot value.
///
/// Variants:
/// - `Block` — the executor checkpoints (`BeforeBlock`) and returns
///   `LoopExit::Blocked`. The standard production path.
/// - `SkipAndContinue` — drop this call's result entirely and proceed with
///   the rest of the batch. Use sparingly; intended for fire-and-forget
///   tools where a missing approval is non-fatal.
/// - `Abort` — return `LoopExit::Failed { reason_kind: failure_kind }`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum GateOutcome {
    Block {
        control: ControlStrategyState,
    },
    SkipAndContinue {
        control: ControlStrategyState,
    },
    Abort {
        control: ControlStrategyState,
        failure_kind: LoopFailureKind,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time object-safety check.
    #[allow(dead_code)]
    fn _check(_: &dyn GateHandlingStrategy) {}

    fn sample_control() -> ControlStrategyState {
        ControlStrategyState {
            turns_completed: 3,
            terminate_hints_in_last_batch: 1,
            last_batch_total: 4,
        }
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
    fn gate_outcome_block_carries_control_slot() {
        let outcome = GateOutcome::Block {
            control: sample_control(),
        };
        let value = serde_json::to_value(&outcome).expect("serialize");
        let restored: GateOutcome = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, outcome);
        // Field is named `control` and is the strategy slot type.
        match restored {
            GateOutcome::Block { control } => assert_eq!(control, sample_control()),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn gate_outcome_skip_and_continue_carries_control_slot() {
        let outcome = GateOutcome::SkipAndContinue {
            control: sample_control(),
        };
        let value = serde_json::to_value(&outcome).expect("serialize");
        let restored: GateOutcome = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, outcome);
        match restored {
            GateOutcome::SkipAndContinue { control } => {
                assert_eq!(control, sample_control())
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn gate_outcome_abort_carries_control_slot_and_failure_kind() {
        let outcome = GateOutcome::Abort {
            control: sample_control(),
            failure_kind: LoopFailureKind::DriverBug,
        };
        let value = serde_json::to_value(&outcome).expect("serialize");
        let restored: GateOutcome = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, outcome);
        match restored {
            GateOutcome::Abort {
                control,
                failure_kind,
            } => {
                assert_eq!(control, sample_control());
                assert_eq!(failure_kind, LoopFailureKind::DriverBug);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }
}
