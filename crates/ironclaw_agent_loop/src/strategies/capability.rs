use async_trait::async_trait;
use ironclaw_host_api::CapabilityId;

use crate::state::LoopExecutionState;

/// Decides which capabilities are visible to the model this iteration.
///
/// Pure policy: returns a filter the executor passes to the host when
/// requesting the visible capability surface. Does NOT mutate state.
///
/// The host is the source of truth for the catalog and applies its own
/// scope/grant/auth filters AFTER the strategy filter; the strategy can only
/// narrow, never expand.
///
/// See `docs/reborn/agent-loop-skeleton.md` section 6.
#[async_trait]
pub(crate) trait CapabilityStrategy: Send + Sync {
    async fn filter(&self, state: &LoopExecutionState) -> CapabilityFilter;
}

#[allow(dead_code)]
fn _assert_object_safe(_: &dyn CapabilityStrategy) {}

/// Reference baseline `CapabilityStrategy`: never narrow the host surface.
///
/// The host applies its own scope/grant/auth filters on top — this default
/// strategy declines to filter further, leaving capability visibility entirely
/// to the host's authoritative policy.
///
/// See `docs/reborn/agent-loop-skeleton.md` §6 ("The nine strategies" →
/// `CapabilityStrategy`).
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultCapabilityStrategy;

#[async_trait]
impl CapabilityStrategy for DefaultCapabilityStrategy {
    async fn filter(&self, _state: &LoopExecutionState) -> CapabilityFilter {
        CapabilityFilter::All
    }
}

/// Strategy-side narrowing of the visible capability surface.
///
/// Variants are mutually exclusive. The host always applies its own
/// scope/grant/auth filters on top; this filter only narrows.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CapabilityFilter {
    /// Allow everything the host would otherwise expose.
    #[default]
    All,
    /// Only the capabilities whose IDs appear in the set.
    AllowOnly(Vec<CapabilityId>),
    /// Everything except the capabilities whose IDs appear in the set.
    Deny(Vec<CapabilityId>),
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{CapabilityId, TenantId, ThreadId};
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

    use super::{CapabilityFilter, CapabilityStrategy, DefaultCapabilityStrategy};
    use crate::state::LoopExecutionState;

    #[allow(dead_code)]
    fn _check(_: &dyn CapabilityStrategy) {}

    fn test_run_context() -> LoopRunContext {
        let scope = TurnScope::new(
            TenantId::new("tenant-default-cap").expect("valid"),
            None,
            None,
            ThreadId::new("thread-default-cap").expect("valid"),
        );
        let descriptor = AgentLoopDriverDescriptor {
            id: LoopDriverId::new("default_cap_test_driver").expect("valid"),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(
                CheckpointSchemaId::new("default_cap_test_checkpoint").expect("valid"),
            ),
            checkpoint_schema_version: Some(RunProfileVersion::new(1)),
        };
        let resolved_run_profile = ResolvedRunProfile {
            run_class_id: RunClassId::new("default_cap_test_class").expect("valid"),
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
            model_profile_id: ModelProfileId::new("default_cap_test_model").expect("valid"),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new(
                "default_cap_test_capabilities",
            )
            .expect("valid"),
            context_profile_id: ContextProfileId::new("default_cap_test_context").expect("valid"),
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
                tier: ResourceBudgetTier::new("default_cap_test_tier").expect("valid"),
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
            resolution_fingerprint: RunProfileFingerprint::new("default-cap-test-fingerprint")
                .expect("valid"),
            provenance: RedactedRunProfileProvenance {
                sources: vec![],
                effective_privileges: vec![],
            },
        };
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
    }

    #[tokio::test]
    async fn default_capability_strategy_returns_all() {
        let strategy = DefaultCapabilityStrategy;
        let state = LoopExecutionState::initial_for_run(&test_run_context());

        assert_eq!(strategy.filter(&state).await, CapabilityFilter::All);
    }

    #[test]
    fn default_filter_allows_all() {
        assert_eq!(CapabilityFilter::default(), CapabilityFilter::All);
    }

    #[test]
    fn filter_round_trips_through_json() {
        let capability_id = CapabilityId::new("test.echo").expect("valid capability id");
        let filters = vec![
            CapabilityFilter::All,
            CapabilityFilter::AllowOnly(vec![capability_id.clone()]),
            CapabilityFilter::Deny(vec![capability_id]),
        ];

        for filter in filters {
            let encoded = serde_json::to_string(&filter).expect("serialize filter");
            let decoded: CapabilityFilter =
                serde_json::from_str(&encoded).expect("deserialize filter");
            assert_eq!(decoded, filter);
        }
    }

    #[test]
    fn filter_serializes_with_snake_case_wire_form() {
        let capability_id = CapabilityId::new("test.echo").expect("valid capability id");

        assert_eq!(
            serde_json::to_string(&CapabilityFilter::All).expect("serialize all"),
            "\"all\""
        );
        assert_eq!(
            serde_json::to_string(&CapabilityFilter::AllowOnly(vec![capability_id.clone()]))
                .expect("serialize allow_only"),
            "{\"allow_only\":[\"test.echo\"]}"
        );
        assert_eq!(
            serde_json::to_string(&CapabilityFilter::Deny(vec![capability_id]))
                .expect("serialize deny"),
            "{\"deny\":[\"test.echo\"]}"
        );
    }
}
