//! Budget strategy contract.

use std::time::Duration;

use crate::state::LoopExecutionState;

/// Hard caps on loop execution iterations and elapsed wall-clock time.
///
/// Model-call and capability-call caps belong to
/// `ResolvedRunProfile.resource_budget_policy` / `ResourceBudgetPolicy`, not
/// this strategy. This stays sync because it is pure read-only policy with no
/// host consult; tenant/profile-backed budget logic should be resolved into
/// the run profile, or added later behind an explicit async contract change.
pub(crate) trait BudgetStrategy: Send + Sync {
    /// Maximum number of iterations before the loop is forcibly failed.
    fn iteration_limit(&self, state: &LoopExecutionState) -> u32;

    /// Optional wall-clock cap. `None` means no time limit.
    fn wall_clock_limit(&self, state: &LoopExecutionState) -> Option<Duration>;
}

#[allow(dead_code)]
fn assert_budget_strategy_object_safe(_: &dyn BudgetStrategy) {}

/// Reference baseline `BudgetStrategy`: 32-iteration cap with no wall-clock
/// limit.
///
/// The 32-iteration ceiling is the first safety net. Loop families that need
/// shorter or longer budgets construct this struct directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultBudgetStrategy {
    /// Hard ceiling on iteration count. Default `32`.
    pub iteration_limit: u32,
    /// Optional wall-clock cap. Default `None` (no limit).
    pub wall_clock_limit: Option<Duration>,
}

impl Default for DefaultBudgetStrategy {
    fn default() -> Self {
        Self {
            iteration_limit: 32,
            wall_clock_limit: None,
        }
    }
}

impl BudgetStrategy for DefaultBudgetStrategy {
    fn iteration_limit(&self, _: &LoopExecutionState) -> u32 {
        self.iteration_limit
    }

    fn wall_clock_limit(&self, _: &LoopExecutionState) -> Option<Duration> {
        self.wall_clock_limit
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
        assert_budget_strategy_object_safe(&DefaultBudgetStrategy::default());
    }

    #[test]
    fn fixed_budget_exercises_trait_surface() {
        let state = LoopExecutionState::initial_for_run(&test_run_context());
        let strategy: &dyn BudgetStrategy = &DefaultBudgetStrategy::default();

        assert_eq!(
            (
                strategy.iteration_limit(&state),
                strategy.wall_clock_limit(&state)
            ),
            (32, None)
        );
    }

    #[test]
    fn default_budget_strategy_returns_32_iterations_no_wall_clock() {
        let strategy = DefaultBudgetStrategy::default();
        let state = LoopExecutionState::initial_for_run(&test_run_context());

        assert_eq!(strategy.iteration_limit(&state), 32);
        assert_eq!(strategy.wall_clock_limit(&state), None);
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
            personal_context_policy: ironclaw_turns::run_profile::PersonalContextPolicy::Excluded,
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
