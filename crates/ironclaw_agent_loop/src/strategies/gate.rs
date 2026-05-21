//! `GateHandlingStrategy` — decides what to do when a capability invocation
//! returns a gate (Approval, Auth, or Resource).
//!
//! Mutates `gate_state` (e.g. record gate fingerprints for resume).
//! Async because future strategies may consult host state for grant-history
//! or auth-flow lookups.
//!
//! Sanitization at the host port boundary means strategies never see raw
//! input, secrets, or auth state.

use async_trait::async_trait;
use ironclaw_turns::{LoopFailureKind, LoopGateRef};

use crate::state::{GateStrategyState, LoopExecutionState};

/// Decides what to do when a capability invocation comes back with a gate.
///
/// `&self` only — strategies are value-immutable. The new `gate_state`
/// slot value is carried in the returned [`GateOutcome`]; the executor
/// swaps it into the next whole state.
#[async_trait]
pub(crate) trait GateHandlingStrategy: Send + Sync {
    async fn handle(&self, state: &LoopExecutionState, gate: &GateSummary) -> GateOutcome;
}

/// Compile-time object-safety check.
#[allow(dead_code)]
fn _gate_handling_strategy_object_safe(_: &dyn GateHandlingStrategy) {}

/// Reference baseline `GateHandlingStrategy`: always `Block`.
///
/// The executor checkpoints (`BeforeBlock`) and returns
/// `LoopExit::Blocked`. Loop families that want skip-and-continue or abort
/// semantics swap this strategy.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultGateHandlingStrategy;

#[async_trait]
impl GateHandlingStrategy for DefaultGateHandlingStrategy {
    async fn handle(&self, state: &LoopExecutionState, _gate: &GateSummary) -> GateOutcome {
        GateOutcome::Block {
            gate: state.gate_state.clone(),
        }
    }
}

/// Loop-side projection of a host capability gate — kind + opaque ref only.
/// The strategy never sees raw input, secrets, or auth state (per
/// `contracts/turns-agent-loop.md` §6 + `contracts/lightweight-agent-loop.md`
/// §8).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct GateSummary {
    pub(crate) kind: GateKind,
    pub(crate) gate_ref: LoopGateRef,
}

/// Wire-stable gate classification. Snake_case names are part of the public
/// contract — they appear in checkpoints and observability events.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GateKind {
    Approval,
    Auth,
    Resource,
    AwaitDependentRun,
}

/// Strategy decision for a gate, plus the new `gate_state` slot value.
///
/// Variants:
/// - `Block` — the executor checkpoints (`BeforeBlock`) and returns
///   `LoopExit::Blocked`. The standard production path.
/// - `SkipAndContinue` — drop this call's result entirely and proceed with
///   the rest of the batch. Use sparingly; intended for fire-and-forget
///   tools where a missing approval is non-fatal.
/// - `Abort` — return `LoopExit::Failed { reason_kind: failure_kind }`.
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
    /// executor honors it.
    pub(crate) fn validate_for_gate_kind(&self, kind: GateKind) -> Result<(), LoopFailureKind> {
        match (kind, self) {
            (GateKind::Approval, GateOutcome::SkipAndContinue { .. })
            | (GateKind::AwaitDependentRun, GateOutcome::SkipAndContinue { .. }) => {
                Err(LoopFailureKind::DriverBug)
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{TenantId, ThreadId};
    use ironclaw_turns::{
        AgentLoopDriverDescriptor, RunProfileId, RunProfileVersion, TurnId, TurnRunId, TurnScope,
        run_profile::{
            CancellationPolicy, CapabilitySurfaceProfileId, CheckpointPolicy, CheckpointSchemaId,
            ConcurrencyClass, ContextProfileId, LoopDriverId, LoopRunContext, ModelProfileId,
            RedactedRunProfileProvenance, ResolvedRunProfile, ResourceBudgetPolicy,
            ResourceBudgetTier, RunClassId, RunProfileFingerprint, RuntimeProfileConstraints,
            SchedulingClass, SteeringPolicy,
        },
    };

    use super::*;
    use crate::state::LoopExecutionState;

    fn sample_gate() -> GateStrategyState {
        GateStrategyState::default()
    }

    fn test_run_context() -> LoopRunContext {
        let scope = TurnScope::new(
            TenantId::new("tenant-default-gate").expect("valid"),
            None,
            None,
            ThreadId::new("thread-default-gate").expect("valid"),
        );
        let descriptor = AgentLoopDriverDescriptor {
            id: LoopDriverId::new("default_gate_test_driver").expect("valid"),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(
                CheckpointSchemaId::new("default_gate_test_checkpoint").expect("valid"),
            ),
            checkpoint_schema_version: Some(RunProfileVersion::new(1)),
        };
        let resolved_run_profile = ResolvedRunProfile {
            run_class_id: RunClassId::new("default_gate_test_class").expect("valid"),
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
            model_profile_id: ModelProfileId::new("default_gate_test_model").expect("valid"),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new(
                "default_gate_test_capabilities",
            )
            .expect("valid"),
            context_profile_id: ContextProfileId::new("default_gate_test_context").expect("valid"),
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
                tier: ResourceBudgetTier::new("default_gate_test_tier").expect("valid"),
                max_model_calls: 32,
                max_capability_invocations: 64,
            },
            personal_context_policy: ironclaw_turns::run_profile::PersonalContextPolicy::Excluded,
            runtime_constraints: RuntimeProfileConstraints {
                allow_raw_runtime_backend_selection: false,
                allow_broad_capability_surface: false,
            },
            runner_pool_id: None,
            scheduling_class: SchedulingClass::new("interactive").expect("valid"),
            concurrency_class: ConcurrencyClass::new("thread_serial").expect("valid"),
            resolution_fingerprint: RunProfileFingerprint::new("default-gate-test-fingerprint")
                .expect("valid"),
            provenance: RedactedRunProfileProvenance {
                sources: vec![],
                effective_privileges: vec![],
            },
        };
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
    }

    #[test]
    fn gate_kind_round_trips_snake_case() {
        for (variant, wire) in [
            (GateKind::Approval, "approval"),
            (GateKind::Auth, "auth"),
            (GateKind::Resource, "resource"),
            (GateKind::AwaitDependentRun, "await_dependent_run"),
        ] {
            let value = serde_json::to_value(variant).expect("serialize");
            assert_eq!(value, serde_json::json!(wire));
            let restored: GateKind = serde_json::from_value(value).expect("deserialize");
            assert_eq!(restored, variant);
        }
    }

    #[test]
    fn gate_summary_round_trips() {
        for kind in [
            GateKind::Approval,
            GateKind::Auth,
            GateKind::Resource,
            GateKind::AwaitDependentRun,
        ] {
            let summary = GateSummary {
                kind,
                gate_ref: LoopGateRef::new("gate:approval-demo").expect("valid"),
            };
            let value = serde_json::to_value(&summary).expect("serialize");
            let restored: GateSummary = serde_json::from_value(value).expect("deserialize");
            assert_eq!(restored, summary);
        }
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
    fn await_dependent_run_gate_rejects_skip_and_continue_outcome() {
        let outcome = GateOutcome::SkipAndContinue {
            gate: sample_gate(),
        };
        assert_eq!(
            outcome.validate_for_gate_kind(GateKind::AwaitDependentRun),
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

    #[tokio::test]
    async fn default_gate_handling_strategy_blocks_for_every_kind() {
        let strategy = DefaultGateHandlingStrategy;
        let mut state = LoopExecutionState::initial_for_run(&test_run_context());
        state.gate_state = sample_gate();

        for kind in [
            GateKind::Approval,
            GateKind::Auth,
            GateKind::Resource,
            GateKind::AwaitDependentRun,
        ] {
            let summary = GateSummary {
                kind,
                gate_ref: LoopGateRef::new("gate:default-test").expect("valid"),
            };
            match strategy.handle(&state, &summary).await {
                GateOutcome::Block { gate } => assert_eq!(gate, sample_gate()),
                other => panic!("expected Block for {kind:?}, got {other:?}"),
            }
        }
    }
}
