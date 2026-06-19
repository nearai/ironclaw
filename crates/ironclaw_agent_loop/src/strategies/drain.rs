//! Input-drain strategy contract.

use async_trait::async_trait;

use crate::state::LoopExecutionState;

/// Decides when to drain the host's steering and followup queues.
///
/// This is pure policy: implementations do not mutate strategy state. Async
/// leaves room for future host-backed queue hints or priority checks.
#[async_trait]
pub(crate) trait InputDrainStrategy: Send + Sync {
    /// Called at the start of each tick before prompt construction.
    async fn drain_steering(&self, state: &LoopExecutionState) -> bool;

    /// Called after the loop would otherwise stop, before returning completed.
    async fn drain_followup(&self, state: &LoopExecutionState) -> bool;
}

#[allow(dead_code)]
fn assert_input_drain_strategy_object_safe(_: &dyn InputDrainStrategy) {}

/// Reference baseline `InputDrainStrategy`: drain both queues every time the
/// executor asks.
///
/// Returns `(drain_steering: true, drain_followup: true)` from the two hook
/// points: steering before each model call, followup before otherwise
/// stopping.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultInputDrainStrategy;

#[async_trait]
impl InputDrainStrategy for DefaultInputDrainStrategy {
    async fn drain_steering(&self, _state: &LoopExecutionState) -> bool {
        true
    }

    async fn drain_followup(&self, _state: &LoopExecutionState) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use ironclaw_turns::{LoopMessageRef, LoopResultRef};

    use super::*;
    use crate::strategies::{CapabilityBatchTurnSummary, TurnEndKind, TurnSummary};

    #[test]
    fn drain_strategy_is_object_safe() {
        struct NeverDrain;

        #[async_trait]
        impl InputDrainStrategy for NeverDrain {
            async fn drain_steering(&self, _: &LoopExecutionState) -> bool {
                false
            }

            async fn drain_followup(&self, _: &LoopExecutionState) -> bool {
                false
            }
        }

        assert_input_drain_strategy_object_safe(&NeverDrain);
    }

    #[tokio::test]
    async fn default_input_drain_strategy_returns_true_for_both_hooks() {
        use ironclaw_host_api::{TenantId, ThreadId};
        use ironclaw_turns::{
            AgentLoopDriverDescriptor, RunProfileId, RunProfileVersion, TurnId, TurnRunId,
            TurnScope,
            run_profile::{
                CancellationPolicy, CapabilitySurfaceProfileId, CheckpointPolicy,
                CheckpointSchemaId, ConcurrencyClass, ContextProfileId, LoopDriverId,
                LoopRunContext, ModelProfileId, RedactedRunProfileProvenance, ResolvedRunProfile,
                ResourceBudgetPolicy, ResourceBudgetTier, RunClassId, RunProfileFingerprint,
                RuntimeProfileConstraints, SchedulingClass, SteeringPolicy,
            },
        };

        let scope = TurnScope::new(
            TenantId::new("tenant-default-drain").expect("valid"),
            None,
            None,
            ThreadId::new("thread-default-drain").expect("valid"),
        );
        let descriptor = AgentLoopDriverDescriptor {
            id: LoopDriverId::new("default_drain_test_driver").expect("valid"),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(
                CheckpointSchemaId::new("default_drain_test_checkpoint").expect("valid"),
            ),
            checkpoint_schema_version: Some(RunProfileVersion::new(1)),
        };
        let resolved_run_profile = ResolvedRunProfile {
            run_class_id: RunClassId::new("default_drain_test_class").expect("valid"),
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
            model_profile_id: ModelProfileId::new("default_drain_test_model").expect("valid"),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new(
                "default_drain_test_capabilities",
            )
            .expect("valid"),
            context_profile_id: ContextProfileId::new("default_drain_test_context").expect("valid"),
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
                tier: ResourceBudgetTier::new("default_drain_test_tier").expect("valid"),
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
            resolution_fingerprint: RunProfileFingerprint::new("default-drain-test-fingerprint")
                .expect("valid"),
            provenance: RedactedRunProfileProvenance {
                sources: vec![],
                effective_privileges: vec![],
            },
        };
        let context =
            LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile);
        let state = crate::state::LoopExecutionState::initial_for_run(&context);
        let strategy = super::DefaultInputDrainStrategy;

        assert!(strategy.drain_steering(&state).await);
        assert!(strategy.drain_followup(&state).await);
    }

    #[test]
    fn turn_summary_round_trips_through_json() {
        let summary = TurnSummary {
            kind: TurnEndKind::AfterCapabilityBatch,
            assistant_message_ref: Some(LoopMessageRef::new("msg:assistant-1").unwrap()),
            batch_result_refs: vec![
                LoopResultRef::new("result:call-1").unwrap(),
                LoopResultRef::new("result:call-2").unwrap(),
            ],
            capability_batch: CapabilityBatchTurnSummary::default(),
        };

        let serialized = serde_json::to_string(&summary).unwrap();
        let deserialized = serde_json::from_str::<TurnSummary>(&serialized).unwrap();

        assert_eq!(deserialized, summary);
    }
}
