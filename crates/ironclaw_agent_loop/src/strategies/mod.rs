//! Strategy trait contracts for the Reborn agent loop.
//!
//! Each strategy receives `&LoopExecutionState` and returns an outcome enum
//! that carries the new value of its own slot. The executor swaps the slot
//! into the next whole state.
//!
//! Checkpoint/observability wire enums are `#[non_exhaustive]`; later
//! changes should extend them without forcing consumers to assume the current
//! variants are closed.
//!
//! Pure policy traits with no host or future host consult may stay sync.
//! Gate and recovery traits are async because they can consult host/runtime
//! state such as grant history, auth flow status, route health, or
//! circuit-breaker counters.

// Keep the unused lint local to these crate-private strategy contracts.
#![allow(dead_code, unused_imports)]

pub(crate) mod batch;
mod budget;
mod capability;
mod context;
mod drain;
pub(crate) mod gate;
mod model;
pub(crate) mod recovery;
mod stop;

pub(crate) use batch::{
    BatchPolicy, BatchPolicyStrategy, CapabilityCallSummary, DefaultBatchPolicyStrategy,
};
pub(crate) use budget::{BudgetStrategy, DefaultBudgetStrategy};
pub(crate) use capability::{CapabilityFilter, CapabilityStrategy, DefaultCapabilityStrategy};
pub(crate) use context::{ContextStrategy, DefaultContextStrategy};
pub(crate) use drain::{DefaultInputDrainStrategy, InputDrainStrategy};
pub(crate) use gate::{
    DefaultGateHandlingStrategy, GateHandlingStrategy, GateKind, GateOutcome, GateSummary,
};
pub(crate) use model::{DefaultModelStrategy, ModelPreference, ModelStrategy};
pub(crate) use recovery::{
    BackoffDelayMs, CapabilityErrorClass, CapabilityErrorSummary, DefaultRecoveryStrategy,
    ModelErrorClass, ModelErrorSummary, RecoveryOutcome, RecoveryStrategy, RetryAlteration,
    RetryScope, SanitizedStrategySummary,
};
pub(crate) use stop::{
    DefaultStopConditionStrategy, StopConditionStrategy, StopKind, StopOutcome, TurnEndKind,
    TurnSummary,
};

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{TenantId, ThreadId};
    use ironclaw_turns::{
        AgentLoopDriverDescriptor, LoopFailureKind, RunProfileId, RunProfileVersion, TurnId,
        TurnRunId, TurnScope,
        run_profile::{
            CancellationPolicy, CapabilitySurfaceProfileId, CheckpointPolicy, CheckpointSchemaId,
            ConcurrencyClass, ContextProfileId, LoopDriverId, LoopRunContext, ModelProfileId,
            RedactedRunProfileProvenance, ResolvedRunProfile, ResourceBudgetPolicy,
            ResourceBudgetTier, RunClassId, RunProfileFingerprint, RuntimeProfileConstraints,
            SchedulingClass, SteeringPolicy,
        },
    };

    use super::*;
    use crate::state::{
        GateStrategyState, LoopExecutionState, RecoveryStrategyState, StopStrategyState,
    };

    #[test]
    fn strategy_outcomes_compose_through_loop_state_slots() {
        let state = LoopExecutionState::initial_for_run(&test_run_context());

        let gate_outcome = GateOutcome::Block {
            gate: GateStrategyState::default(),
        };
        let recovery_outcome = RecoveryOutcome::Retry {
            recovery: RecoveryStrategyState::with_attempts_for(
                crate::state::RecoveryAttemptClass::ModelTransient,
                2,
            ),
            scope: RetryScope::Call,
            alter: Some(RetryAlteration::ShrinkContext { drop_messages: 1 }),
        };
        let stop_outcome = StopOutcome::Stop {
            stop: StopStrategyState {
                turns_completed: 1,
                terminate_hints_in_last_batch: 1,
                last_batch_total: 1,
            },
            kind: StopKind::NoProgressDetected,
        };

        let mut next_state = state.clone();
        if let GateOutcome::Block { gate } = gate_outcome {
            next_state.gate_state = gate;
        }
        if let RecoveryOutcome::Retry { recovery, .. } = recovery_outcome {
            next_state.recovery_state = recovery;
        }
        if let StopOutcome::Stop { stop, kind } = stop_outcome {
            assert_eq!(kind, StopKind::NoProgressDetected);
            next_state.stop_state = stop;
        }

        let value = serde_json::to_value(&next_state).expect("serialize loop state");
        assert_eq!(
            value["recovery_state"]["attempts_by_class"]["model_transient"],
            2
        );
        assert_eq!(value["stop_state"]["turns_completed"], 1);
        assert_eq!(value["gate_state"], serde_json::json!({}));

        let restored: LoopExecutionState =
            serde_json::from_value(value).expect("deserialize loop state");
        assert_eq!(
            restored.recovery_state,
            RecoveryStrategyState::with_attempts_for(
                crate::state::RecoveryAttemptClass::ModelTransient,
                2,
            )
        );
        assert_eq!(
            restored.stop_state,
            StopStrategyState {
                turns_completed: 1,
                terminate_hints_in_last_batch: 1,
                last_batch_total: 1,
            }
        );
        assert_eq!(restored.gate_state, GateStrategyState::default());
    }

    fn test_run_context() -> LoopRunContext {
        let scope = TurnScope::new(
            TenantId::new("tenant-strategy-composition").expect("valid"),
            None,
            None,
            ThreadId::new("thread-strategy-composition").expect("valid"),
        );
        let descriptor = AgentLoopDriverDescriptor {
            id: LoopDriverId::new("strategy_composition_test_driver").expect("valid"),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(
                CheckpointSchemaId::new("strategy_composition_test_checkpoint").expect("valid"),
            ),
            checkpoint_schema_version: Some(RunProfileVersion::new(1)),
        };
        let resolved_run_profile = ResolvedRunProfile {
            run_class_id: RunClassId::new("strategy_composition_test_class").expect("valid"),
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
            model_profile_id: ModelProfileId::new("strategy_composition_test_model")
                .expect("valid"),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new(
                "strategy_composition_test_capabilities",
            )
            .expect("valid"),
            context_profile_id: ContextProfileId::new("strategy_composition_test_context")
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
                tier: ResourceBudgetTier::new("strategy_composition_test_tier").expect("valid"),
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
                "strategy-composition-test-fingerprint",
            )
            .expect("valid"),
            provenance: RedactedRunProfileProvenance {
                sources: vec![],
                effective_privileges: vec![],
            },
        };
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
    }
}
