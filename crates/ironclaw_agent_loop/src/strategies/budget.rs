//! Budget strategy contract.

use std::time::Duration;

use crate::state::LoopExecutionState;

/// Hard caps on loop execution.
///
/// This is sync, read-only policy. The executor owns enforcement.
pub trait BudgetStrategy: Send + Sync {
    /// Maximum number of iterations before the loop is forcibly failed.
    fn iteration_limit(&self, state: &LoopExecutionState) -> u32;

    /// Optional wall-clock cap. `None` means no time limit.
    fn wall_clock_limit(&self, state: &LoopExecutionState) -> Option<Duration>;
}

/// No wall-clock cap and a 32-iteration limit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnlimitedBudget;

impl BudgetStrategy for UnlimitedBudget {
    fn iteration_limit(&self, _: &LoopExecutionState) -> u32 {
        32
    }

    fn wall_clock_limit(&self, _: &LoopExecutionState) -> Option<Duration> {
        None
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

    #[test]
    fn budget_strategy_is_object_safe() {
        fn _check(_: &dyn BudgetStrategy) {}

        _check(&UnlimitedBudget);
    }

    #[test]
    fn unlimited_budget_exercises_trait_surface() {
        let state = LoopExecutionState::initial_for_run(&test_run_context());
        let strategy: &dyn BudgetStrategy = &UnlimitedBudget;

        assert_eq!(
            (
                strategy.iteration_limit(&state),
                strategy.wall_clock_limit(&state)
            ),
            (32, None)
        );
    }

    fn test_run_context() -> LoopRunContext {
        let scope = TurnScope::new(
            TenantId::new("tenant-budget-strategy").expect("valid"),
            None,
            None,
            ThreadId::new("thread-budget-strategy").expect("valid"),
        );
        let descriptor = AgentLoopDriverDescriptor {
            id: LoopDriverId::new("budget_strategy_test_driver").expect("valid"),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(
                CheckpointSchemaId::new("budget_strategy_test_checkpoint").expect("valid"),
            ),
            checkpoint_schema_version: Some(RunProfileVersion::new(1)),
        };
        let resolved_run_profile = ResolvedRunProfile {
            run_class_id: RunClassId::new("budget_strategy_test_class").expect("valid"),
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
            model_profile_id: ModelProfileId::new("budget_strategy_test_model").expect("valid"),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new(
                "budget_strategy_test_capabilities",
            )
            .expect("valid"),
            context_profile_id: ContextProfileId::new("budget_strategy_test_context")
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
                tier: ResourceBudgetTier::new("budget_strategy_test_tier").expect("valid"),
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
            resolution_fingerprint: RunProfileFingerprint::new("budget-strategy-test-fingerprint")
                .expect("valid"),
            provenance: RedactedRunProfileProvenance {
                sources: vec![],
                effective_privileges: vec![],
            },
        };
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
    }
}
