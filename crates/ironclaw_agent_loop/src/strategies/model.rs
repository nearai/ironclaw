use async_trait::async_trait;

use crate::state::LoopExecutionState;

/// Decides which model preference to pass on the next `stream_model` call.
///
/// Pure policy: returns a `ModelPreference` the executor includes in
/// `LoopModelRequest`. Does NOT mutate state.
///
/// The actual model the host calls is bound by `LoopRunContext`'s resolved model
/// route. The strategy's preference is a hint the host may interpret, such as
/// choosing among already-resolved fallbacks. Strategies cannot introduce new
/// routes mid-run.
#[async_trait]
pub(crate) trait ModelStrategy: Send + Sync {
    async fn preference(&self, state: &LoopExecutionState) -> ModelPreference;
}

#[allow(dead_code)]
fn _assert_object_safe(_: &dyn ModelStrategy) {}

/// Reference baseline `ModelStrategy`: track the executor-managed fallback
/// index in `state.model_state.fallback_index`.
///
/// In the skeleton the executor never advances `fallback_index`, so this
/// always returns `Primary`. The `Fallback` arm is wired through for a future
/// model-route-chain implementation.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultModelStrategy;

#[async_trait]
impl ModelStrategy for DefaultModelStrategy {
    async fn preference(&self, state: &LoopExecutionState) -> ModelPreference {
        match state.model_state.fallback_index {
            0 => ModelPreference::Primary,
            index => ModelPreference::Fallback { index },
        }
    }
}

/// Strategy hint to the host about which already-resolved route to use.
///
/// In the skeleton, `Primary` is the only value strategies produce. `Fallback`
/// is reserved for the deferred `ModelRouteChain` follow-up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ModelPreference {
    /// Route-chain index 0: the primary route.
    #[default]
    Primary,
    Fallback {
        /// Deferred route-chain index from `ModelStrategyState::fallback_index`.
        /// Valid fallback indexes are nonzero; `1` is the first fallback after
        /// `Primary`.
        index: u32,
    },
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

    use super::{DefaultModelStrategy, ModelPreference, ModelStrategy};
    use crate::state::LoopExecutionState;

    #[allow(dead_code)]
    fn _check(_: &dyn ModelStrategy) {}

    fn test_run_context() -> LoopRunContext {
        let scope = TurnScope::new(
            TenantId::new("tenant-default-model").expect("valid"),
            None,
            None,
            ThreadId::new("thread-default-model").expect("valid"),
        );
        let descriptor = AgentLoopDriverDescriptor {
            id: LoopDriverId::new("default_model_test_driver").expect("valid"),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(
                CheckpointSchemaId::new("default_model_test_checkpoint").expect("valid"),
            ),
            checkpoint_schema_version: Some(RunProfileVersion::new(1)),
        };
        let resolved_run_profile = ResolvedRunProfile {
            run_class_id: RunClassId::new("default_model_test_class").expect("valid"),
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
            model_profile_id: ModelProfileId::new("default_model_test_model").expect("valid"),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new(
                "default_model_test_capabilities",
            )
            .expect("valid"),
            context_profile_id: ContextProfileId::new("default_model_test_context").expect("valid"),
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
                tier: ResourceBudgetTier::new("default_model_test_tier").expect("valid"),
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
            resolution_fingerprint: RunProfileFingerprint::new("default-model-test-fingerprint")
                .expect("valid"),
            provenance: RedactedRunProfileProvenance {
                sources: vec![],
                effective_privileges: vec![],
            },
        };
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
    }

    #[test]
    fn default_preference_is_primary() {
        assert_eq!(ModelPreference::default(), ModelPreference::Primary);
    }

    #[tokio::test]
    async fn default_model_strategy_returns_primary_at_index_zero() {
        let strategy = DefaultModelStrategy;
        let state = LoopExecutionState::initial_for_run(&test_run_context());

        assert_eq!(strategy.preference(&state).await, ModelPreference::Primary);
    }

    #[tokio::test]
    async fn default_model_strategy_returns_fallback_when_index_nonzero() {
        let strategy = DefaultModelStrategy;
        let mut state = LoopExecutionState::initial_for_run(&test_run_context());
        state.model_state.fallback_index = 2;

        assert_eq!(
            strategy.preference(&state).await,
            ModelPreference::Fallback { index: 2 }
        );
    }

    #[test]
    fn preference_round_trips_through_snake_case_json() {
        let primary = serde_json::to_string(&ModelPreference::Primary).expect("serialize primary");
        assert_eq!(primary, "\"primary\"");
        let decoded_primary: ModelPreference =
            serde_json::from_str(&primary).expect("deserialize primary");
        assert_eq!(decoded_primary, ModelPreference::Primary);

        let fallback =
            serde_json::to_string(&ModelPreference::Fallback { index: 2 }).expect("serialize");
        assert_eq!(fallback, "{\"fallback\":{\"index\":2}}");
        let decoded_fallback: ModelPreference =
            serde_json::from_str(&fallback).expect("deserialize fallback");
        assert_eq!(decoded_fallback, ModelPreference::Fallback { index: 2 });
    }
}
