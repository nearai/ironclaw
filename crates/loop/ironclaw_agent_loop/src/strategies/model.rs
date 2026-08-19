use async_trait::async_trait;
use ironclaw_host_api::ids::CapabilityId;
use ironclaw_loop_contracts::LoopModelToolChoice;

use crate::state::{LoopExecutionState, ReplyAdmissionRejectionReason};

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

    /// Provider tool-choice constraint for the next model call. `None` (the
    /// default) leaves the provider on its own "auto" behavior.
    async fn tool_choice(&self, _state: &LoopExecutionState) -> Option<LoopModelToolChoice> {
        None
    }
}

#[allow(dead_code)]
fn _assert_object_safe(_: &dyn ModelStrategy) {}

/// Reference baseline `ModelStrategy`: track the executor-managed fallback
/// index in `state.model_state.fallback_index`.
///
/// The executor advances `fallback_index` only after an explicit recovery
/// alteration, and the host returns authoritative route evidence on success.
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

/// `ModelStrategy` for structured-output runs (unbound-turn design §4.5):
/// route selection is the default fallback-chain behavior, but a model call
/// issued while a structured-output rejection is pending — a repair retry
/// after the model produced a plain-text final — forces the provider's
/// tool choice onto the synthetic result capability. First attempts stay on
/// provider "auto" so the model can use ordinary tools before finishing.
#[derive(Debug, Clone)]
pub struct StructuredResultModelStrategy {
    result_capability: CapabilityId,
}

impl StructuredResultModelStrategy {
    pub fn new(result_capability: CapabilityId) -> Self {
        Self { result_capability }
    }
}

#[async_trait]
impl ModelStrategy for StructuredResultModelStrategy {
    async fn preference(&self, state: &LoopExecutionState) -> ModelPreference {
        DefaultModelStrategy.preference(state).await
    }

    async fn tool_choice(&self, state: &LoopExecutionState) -> Option<LoopModelToolChoice> {
        let pending = state.reply_admission_state.pending_rejection.as_ref()?;
        match pending.reason_code {
            ReplyAdmissionRejectionReason::StructuredOutputRequired => {
                Some(LoopModelToolChoice::ForcedCapability {
                    capability_id: self.result_capability.clone(),
                })
            }
            ReplyAdmissionRejectionReason::StopConditionNotMet => None,
        }
    }
}

/// Strategy hint to the host about which already-resolved route to use.
///
/// `Fallback` selects a pre-resolved entry from the host's ordered chain.
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
    use ironclaw_host_api::ids::{TenantId, ThreadId};
    use ironclaw_host_api::turn::{RunProfileId, RunProfileVersion, TurnId, TurnRunId, TurnScope};
    use ironclaw_loop_contracts::{
        AgentLoopDriverDescriptor, CancellationPolicy, CapabilitySurfaceProfileId,
        CheckpointPolicy, CheckpointSchemaId, ConcurrencyClass, ContextProfileId, LoopDriverId,
        LoopRunContext, ModelProfileId, RedactedRunProfileProvenance, ResolvedRunProfile,
        ResourceBudgetPolicy, ResourceBudgetTier, RunClassId, RunProfileFingerprint,
        RuntimeProfileConstraints, SchedulingClass, SteeringPolicy,
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
                before_model_checkpoint_interval: 1,
            },
            resource_budget_policy: ResourceBudgetPolicy {
                tier: ResourceBudgetTier::new("default_model_test_tier").expect("valid"),
                max_model_calls: 32,
                max_capability_invocations: 64,
                max_wall_clock_seconds: None,
            },
            personal_context_policy: ironclaw_loop_contracts::PersonalContextPolicy::Excluded,
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

    #[tokio::test]
    async fn structured_result_strategy_forces_the_result_tool_only_on_repair_retries() {
        use super::StructuredResultModelStrategy;
        use crate::state::ReplyAdmissionRejection;
        use ironclaw_loop_contracts::LoopModelToolChoice;

        let capability =
            ironclaw_host_api::ids::CapabilityId::new("builtin.structured_result").expect("valid");
        let strategy = StructuredResultModelStrategy::new(capability.clone());
        let mut state = LoopExecutionState::initial_for_run(&test_run_context());

        // First attempt: no pending rejection, provider stays on "auto".
        assert_eq!(strategy.tool_choice(&state).await, None);

        // Repair retry after a rejected plain-text final: force the tool.
        state.reply_admission_state.pending_rejection =
            Some(ReplyAdmissionRejection::structured_output_required());
        assert_eq!(
            strategy.tool_choice(&state).await,
            Some(LoopModelToolChoice::ForcedCapability {
                capability_id: capability
            })
        );

        // A stop-condition rejection is not a structured repair.
        state.reply_admission_state.pending_rejection =
            Some(ReplyAdmissionRejection::stop_condition_not_met());
        assert_eq!(strategy.tool_choice(&state).await, None);

        // Route preference stays the default fallback-chain behavior.
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
