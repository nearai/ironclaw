use std::sync::Arc;
use std::time::Duration;

use crate::default_planner::DefaultPlanner;
use crate::family::{ComponentDigest, ComponentIdentity, LoopFamily, LoopFamilyId};
use crate::planner::AgentLoopPlanner;
use crate::strategies::DefaultBudgetStrategy;

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
    "compaction:ActiveTaskPreservingCompactionStrategy(context_limit=run_context,reserve=run_context,preserve_tail=8000,min_compacted=3,min_tail=3,deadline_ms=30000,ineffective_trip_limit=3),",
    "capability:DefaultCapabilityStrategy(all),",
    "model:DefaultModelStrategy(primary_or_fallback_index),",
    "batch:model_emitted_calls(bounded_fanout=4),",
    "gate:DefaultGateHandlingStrategy(block),",
    "recovery:DefaultRecoveryStrategy(max_attempts_per_class=2,model_availability_attempts=12,availability=retry_then_observe,stale_request=iteration_retry_then_observe,output_truncated=observe_then_continue,unauthorized=user_visible_terminal,checkpoint_rejected=abort,transcript_write_failed=user_visible_terminal),",
    "reply_admission:DefaultReplyAdmissionStrategy(reject_empty_and_provider_transcript_artifacts),",
    "stop:DefaultStopConditionStrategy(consecutive_repeat=3,no_progress_window=32,no_progress_threshold=8,rejected_reply=invalid_model_output),",
    "drain:DefaultInputDrainStrategy(steering=true,followup=true),",
    "budget:DefaultBudgetStrategy(iteration_limit=256,wall_clock_limit=none)"
)
.as_bytes();

pub const SUBAGENT_FAMILY_DIGEST: ComponentDigest = ComponentDigest([
    0x6f, 0xc8, 0xf8, 0x2f, 0x91, 0x6e, 0x9b, 0x17, 0x2f, 0xe0, 0x4d, 0xef, 0x53, 0xbf, 0xda, 0xea,
    0x31, 0x44, 0x69, 0x33, 0xc2, 0x6e, 0x9d, 0x5e, 0xa6, 0x37, 0x06, 0xf7, 0x1f, 0xed, 0x7d, 0xb0,
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

#[cfg(test)]
mod tests {
    use crate::families::DEFAULT_FAMILY_DIGEST;
    use crate::state::LoopExecutionState;
    use crate::strategies::CapabilityFilter;
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

    #[tokio::test]
    async fn subagent_family_keeps_default_non_budget_strategies() {
        let family = subagent();
        let context = test_run_context("subagent-family-defaults");
        let state = LoopExecutionState::initial_for_run(&context);

        assert_eq!(
            family.planner().capability().filter(&state).await,
            CapabilityFilter::All
        );
    }
}
