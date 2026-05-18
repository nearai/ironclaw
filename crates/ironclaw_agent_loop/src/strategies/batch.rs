//! `BatchPolicyStrategy` — decides whether a capability batch executes
//! sequentially or in parallel.
//!
//! Pure policy and synchronous: the strategy never consults the host and
//! mutates nothing. Per-capability concurrency hints from descriptors are
//! authoritative for "this specific call must run alone"; this strategy
//! decides only the batch-level default.
//!
//! The loop never sees raw tool input, only the sanitized projection.

use ironclaw_host_api::CapabilityId;
use ironclaw_turns::run_profile::ConcurrencyHint;

use crate::state::LoopExecutionState;

/// Decides whether a capability batch executes sequentially or in parallel.
///
/// `&self` only — the strategy is value-immutable. The host's per-capability
/// concurrency hints (from descriptors) override this batch-level default
/// for any individual call that declares itself [`ConcurrencyHint::Exclusive`].
pub(crate) trait BatchPolicyStrategy: Send + Sync {
    fn policy(&self, state: &LoopExecutionState, calls: &[CapabilityCallSummary]) -> BatchPolicy;
}

/// Compile-time object-safety check. `BatchPolicyStrategy` is pure-sync
/// policy, but we still want it usable behind a trait object so the
/// executor can hold a heterogeneous strategy stack.
#[allow(dead_code)]
fn _batch_policy_strategy_object_safe(_: &dyn BatchPolicyStrategy) {}

/// Reference baseline `BatchPolicyStrategy`: run parallel unless any call in
/// the batch carries an `Exclusive` concurrency hint.
///
/// An empty batch returns `Parallel`. The default is a no-op for a zero-call
/// batch (the executor never invokes the strategy in that case in practice),
/// but a stable answer keeps the strategy total.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultBatchPolicyStrategy;

impl BatchPolicyStrategy for DefaultBatchPolicyStrategy {
    fn policy(&self, _state: &LoopExecutionState, calls: &[CapabilityCallSummary]) -> BatchPolicy {
        if calls
            .iter()
            .any(|call| matches!(call.concurrency_hint, ConcurrencyHint::Exclusive))
        {
            BatchPolicy::Sequential
        } else {
            BatchPolicy::Parallel
        }
    }
}

/// Batch-level execution mode. Wire-stable: serialized into checkpoints and
/// emitted on observability events, so the snake_case names are part of the
/// public contract.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BatchPolicy {
    Sequential,
    Parallel,
}

/// Loop-side projection of one entry in a `CapabilityCalls` batch — name plus
/// concurrency hint only. The strategy never sees raw args (per
/// `contracts/turns-agent-loop.md` §6 — sanitization happens at the host port
/// boundary).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CapabilityCallSummary {
    pub(crate) name: CapabilityId,
    pub(crate) concurrency_hint: ConcurrencyHint,
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
    use serde_json::json;

    use super::*;
    use crate::state::LoopExecutionState;

    fn test_run_context() -> LoopRunContext {
        let scope = TurnScope::new(
            TenantId::new("tenant-default-batch").expect("valid"),
            None,
            None,
            ThreadId::new("thread-default-batch").expect("valid"),
        );
        let descriptor = AgentLoopDriverDescriptor {
            id: LoopDriverId::new("default_batch_test_driver").expect("valid"),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(
                CheckpointSchemaId::new("default_batch_test_checkpoint").expect("valid"),
            ),
            checkpoint_schema_version: Some(RunProfileVersion::new(1)),
        };
        let resolved_run_profile = ResolvedRunProfile {
            run_class_id: RunClassId::new("default_batch_test_class").expect("valid"),
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
            model_profile_id: ModelProfileId::new("default_batch_test_model").expect("valid"),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new(
                "default_batch_test_capabilities",
            )
            .expect("valid"),
            context_profile_id: ContextProfileId::new("default_batch_test_context").expect("valid"),
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
                tier: ResourceBudgetTier::new("default_batch_test_tier").expect("valid"),
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
            resolution_fingerprint: RunProfileFingerprint::new("default-batch-test-fingerprint")
                .expect("valid"),
            provenance: RedactedRunProfileProvenance {
                sources: vec![],
                effective_privileges: vec![],
            },
        };
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
    }

    fn call(name: &str, hint: ConcurrencyHint) -> CapabilityCallSummary {
        CapabilityCallSummary {
            name: ironclaw_host_api::CapabilityId::new(name).expect("valid"),
            concurrency_hint: hint,
        }
    }

    #[test]
    fn default_batch_policy_returns_parallel_for_empty_batch() {
        let strategy = DefaultBatchPolicyStrategy;
        let state = LoopExecutionState::initial_for_run(&test_run_context());

        assert_eq!(strategy.policy(&state, &[]), BatchPolicy::Parallel);
    }

    #[test]
    fn default_batch_policy_returns_parallel_when_all_safe() {
        let strategy = DefaultBatchPolicyStrategy;
        let state = LoopExecutionState::initial_for_run(&test_run_context());
        let calls = [
            call("demo.echo", ConcurrencyHint::SafeForParallel),
            call("demo.read", ConcurrencyHint::SafeForParallel),
        ];

        assert_eq!(strategy.policy(&state, &calls), BatchPolicy::Parallel);
    }

    #[test]
    fn default_batch_policy_returns_sequential_when_any_exclusive() {
        let strategy = DefaultBatchPolicyStrategy;
        let state = LoopExecutionState::initial_for_run(&test_run_context());
        let calls = [
            call("demo.read", ConcurrencyHint::SafeForParallel),
            call("demo.write", ConcurrencyHint::Exclusive),
        ];

        assert_eq!(strategy.policy(&state, &calls), BatchPolicy::Sequential);
    }

    #[test]
    fn batch_policy_round_trips_snake_case() {
        for (variant, wire) in [
            (BatchPolicy::Sequential, "sequential"),
            (BatchPolicy::Parallel, "parallel"),
        ] {
            let value = serde_json::to_value(variant).expect("serialize");
            assert_eq!(value, json!(wire));
            let restored: BatchPolicy = serde_json::from_value(value).expect("deserialize");
            assert_eq!(restored, variant);
        }
    }

    #[test]
    fn capability_call_summary_round_trips() {
        let summary = CapabilityCallSummary {
            name: CapabilityId::new("demo.echo").expect("valid"),
            concurrency_hint: ConcurrencyHint::SafeForParallel,
        };
        let value = serde_json::to_value(&summary).expect("serialize");
        let restored: CapabilityCallSummary = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, summary);
    }
}
