use std::sync::Arc;
use std::time::Duration;

use crate::default_planner::DefaultPlanner;
use crate::families::ToolBatchStrategy;
use crate::family::{ComponentDigest, ComponentIdentity, LoopFamily, LoopFamilyId};
use crate::planner::AgentLoopPlanner;
use crate::strategies::{BoundedParallelBatchPolicyStrategy, DefaultBudgetStrategy};

const SUBAGENT_ITERATION_LIMIT: u32 = 256;
const SUBAGENT_WALL_CLOCK_LIMIT: Option<Duration> = None;

#[cfg(test)]
const SUBAGENT_FAMILY_FINGERPRINT: &[u8] = concat!(
    "ironclaw_agent_loop.subagent_family.v3:",
    "family_id=subagent;",
    "identity=component_identity_v1;",
    "planner=DefaultPlanner;",
    "strategies=",
    "context:DefaultContextStrategy(max_messages=128),",
    "compaction:ActiveTaskPreservingCompactionStrategy(context_limit=128000,reserve=20000,preserve_tail=8000,min_compacted=3,min_tail=3,deadline_ms=30000,ineffective_trip_limit=3),",
    "capability:DefaultCapabilityStrategy(all),",
    "model:DefaultModelStrategy(primary_or_fallback_index),",
    "batch:DefaultBatchPolicyStrategy(parallel_unless_exclusive),",
    "gate:DefaultGateHandlingStrategy(block),",
    "recovery:DefaultRecoveryStrategy(max_attempts_per_class=2,model_availability_attempts=12,availability=retry_then_observe,stale_request=iteration_retry_then_observe,output_truncated=observe_then_continue,unauthorized=user_visible_terminal,checkpoint_rejected=abort,transcript_write_failed=user_visible_terminal),",
    "reply_admission:DefaultReplyAdmissionStrategy(reject_empty_and_provider_transcript_artifacts),",
    "stop:DefaultStopConditionStrategy(consecutive_repeat=3,advisory_only,rejected_reply=invalid_model_output),",
    "drain:DefaultInputDrainStrategy(steering=true,followup=true),",
    "budget:DefaultBudgetStrategy(iteration_limit=256,wall_clock_limit=none)"
)
.as_bytes();

const SUBAGENT_PARALLEL_FAMILY_FINGERPRINT: &[u8] = concat!(
    "ironclaw_agent_loop.subagent_family.v3:",
    "family_id=subagent;",
    "identity=component_identity_v1;",
    "planner=DefaultPlanner;",
    "strategies=",
    "context:DefaultContextStrategy(max_messages=128),",
    "compaction:ActiveTaskPreservingCompactionStrategy(context_limit=128000,reserve=20000,preserve_tail=8000,min_compacted=3,min_tail=3,deadline_ms=30000,ineffective_trip_limit=3),",
    "capability:DefaultCapabilityStrategy(all),",
    "model:DefaultModelStrategy(primary_or_fallback_index),",
    "batch:BoundedParallelBatchPolicyStrategy(parallel_unless_exclusive,bounded_fanout=4),",
    "gate:DefaultGateHandlingStrategy(block),",
    "recovery:DefaultRecoveryStrategy(max_attempts_per_class=2,model_availability_attempts=12,availability=retry_then_observe,stale_request=iteration_retry_then_observe,output_truncated=observe_then_continue,unauthorized=user_visible_terminal,checkpoint_rejected=abort,transcript_write_failed=user_visible_terminal),",
    "reply_admission:DefaultReplyAdmissionStrategy(reject_empty_and_provider_transcript_artifacts),",
    "stop:DefaultStopConditionStrategy(consecutive_repeat=3,advisory_only,rejected_reply=invalid_model_output),",
    "drain:DefaultInputDrainStrategy(steering=true,followup=true),",
    "budget:DefaultBudgetStrategy(iteration_limit=256,wall_clock_limit=none)"
)
.as_bytes();

pub const SUBAGENT_FAMILY_DIGEST: ComponentDigest = ComponentDigest([
    0x5e, 0xaa, 0x14, 0xa7, 0x06, 0x28, 0x60, 0x59, 0x96, 0x77, 0x7b, 0xdd, 0x6c, 0xa5, 0x9f, 0xdd,
    0xd5, 0xb2, 0x03, 0x93, 0xa5, 0x67, 0x99, 0x6b, 0xea, 0xbf, 0x67, 0xe2, 0x52, 0x93, 0xe6, 0x4a,
]);

pub fn subagent() -> LoopFamily {
    let budget = Arc::new(DefaultBudgetStrategy {
        iteration_limit: SUBAGENT_ITERATION_LIMIT,
        wall_clock_limit: SUBAGENT_WALL_CLOCK_LIMIT,
    });
    let planner = DefaultPlanner::compose_default()
        .with_id(LoopFamilyId::SUBAGENT)
        .with_version(ComponentIdentity::from_static(
            "subagent",
            SUBAGENT_FAMILY_DIGEST,
        ))
        .with_budget(budget);
    let id = planner.id().clone();
    let version = planner.version().clone();

    LoopFamily::new(id, version, Arc::new(planner))
}

/// The subagent family with a selected capability batch strategy.
///
/// The host-batch path preserves the stable production identity exactly.
pub fn subagent_with_tool_batch_strategy(strategy: ToolBatchStrategy) -> LoopFamily {
    match strategy {
        ToolBatchStrategy::HostBatch => subagent(),
        ToolBatchStrategy::BoundedParallel => {
            let budget = Arc::new(DefaultBudgetStrategy {
                iteration_limit: SUBAGENT_ITERATION_LIMIT,
                wall_clock_limit: SUBAGENT_WALL_CLOCK_LIMIT,
            });
            let digest = ComponentDigest::from_blake3(SUBAGENT_PARALLEL_FAMILY_FINGERPRINT);
            let planner = DefaultPlanner::compose_default()
                .with_id(LoopFamilyId::SUBAGENT)
                .with_version(ComponentIdentity::new("subagent", digest))
                .with_budget(budget)
                .with_batch(Arc::new(BoundedParallelBatchPolicyStrategy));
            let id = planner.id().clone();
            let version = planner.version().clone();

            LoopFamily::new(id, version, Arc::new(planner))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::families::DEFAULT_FAMILY_DIGEST;
    use crate::state::LoopExecutionState;
    use crate::strategies::{BatchPolicy, CapabilityFilter};
    use crate::test_support::test_run_context;

    use super::*;

    #[test]
    fn subagent_family_has_subagent_identity() {
        let family = subagent();

        assert_eq!(family.id(), &LoopFamilyId::SUBAGENT);
        assert_eq!(family.version().id, "subagent");
        assert_eq!(family.version().digest, SUBAGENT_FAMILY_DIGEST);
        assert_ne!(family.version().digest, ComponentDigest([0; 32]));
    }

    #[test]
    fn subagent_family_digest_matches_blake3_fingerprint() {
        assert_eq!(
            SUBAGENT_FAMILY_DIGEST,
            ComponentDigest::from_blake3(SUBAGENT_FAMILY_FINGERPRINT)
        );
    }

    #[test]
    fn subagent_family_digest_differs_from_default() {
        assert_ne!(SUBAGENT_FAMILY_DIGEST, DEFAULT_FAMILY_DIGEST);
    }

    #[test]
    fn subagent_family_budget_is_tightened() {
        let family = subagent();
        let context = test_run_context("subagent-family-budget");
        let state = LoopExecutionState::initial_for_run(&context);

        assert_eq!(
            family.planner().budget().iteration_limit(&state),
            SUBAGENT_ITERATION_LIMIT
        );
    }

    #[test]
    fn subagent_family_batch_strategy_selects_execution_mode() {
        use crate::strategies::CapabilityBatchExecutionMode;

        let host_batch = subagent_with_tool_batch_strategy(ToolBatchStrategy::HostBatch);
        assert_eq!(host_batch.version().digest, SUBAGENT_FAMILY_DIGEST);
        assert_eq!(
            host_batch.planner().batch().execution_mode(),
            CapabilityBatchExecutionMode::HostBatch
        );

        let bounded = subagent_with_tool_batch_strategy(ToolBatchStrategy::BoundedParallel);
        assert_ne!(bounded.version().digest, SUBAGENT_FAMILY_DIGEST);
        assert_eq!(
            bounded.planner().batch().execution_mode(),
            CapabilityBatchExecutionMode::BoundedParallel
        );
    }

    #[tokio::test]
    async fn subagent_family_keeps_default_non_budget_strategies() {
        let family = subagent();
        let context = test_run_context("subagent-family-defaults");
        let state = LoopExecutionState::initial_for_run(&context);

        assert_eq!(
            family.planner().batch().policy(&state, &[]),
            BatchPolicy::Parallel
        );
        assert_eq!(
            family.planner().capability().filter(&state).await,
            CapabilityFilter::All
        );
    }
}
