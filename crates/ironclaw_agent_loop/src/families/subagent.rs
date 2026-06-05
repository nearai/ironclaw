use std::sync::Arc;
use std::time::Duration;

use crate::default_planner::DefaultPlanner;
use crate::family::{ComponentDigest, ComponentIdentity, LoopFamily, LoopFamilyId};
use crate::planner::AgentLoopPlanner;
use crate::strategies::DefaultBudgetStrategy;

const SUBAGENT_ITERATION_LIMIT: u32 = 16;
const SUBAGENT_WALL_CLOCK_LIMIT: Option<Duration> = None;

#[cfg(test)]
const SUBAGENT_FAMILY_FINGERPRINT: &[u8] = concat!(
    "ironclaw_agent_loop.subagent_family.v1:",
    "family_id=subagent;",
    "identity=component_identity_v1;",
    "planner=DefaultPlanner;",
    "strategies=",
    "context:DefaultContextStrategy(max_messages=16),",
    "capability:DefaultCapabilityStrategy(all),",
    "model:DefaultModelStrategy(primary_or_fallback_index),",
    "batch:DefaultBatchPolicyStrategy(parallel_unless_exclusive),",
    "gate:DefaultGateHandlingStrategy(block),",
    "recovery:DefaultRecoveryStrategy(max_attempts_per_class=2),",
    "reply_admission:DefaultReplyAdmissionStrategy(reject_empty_and_provider_transcript_artifacts),",
    "stop:DefaultStopConditionStrategy(window=5,repeat=3,failure_run=3,rejected_reply=invalid_model_output),",
    "drain:DefaultInputDrainStrategy(steering=true,followup=true),",
    "budget:DefaultBudgetStrategy(iteration_limit=16,wall_clock_limit=none)"
)
.as_bytes();

pub const SUBAGENT_FAMILY_DIGEST: ComponentDigest = ComponentDigest([
    0x78, 0xbe, 0x25, 0x71, 0xda, 0x80, 0xc0, 0x23, 0xb6, 0xb6, 0x02, 0xdd, 0xd6, 0xcd, 0xf4, 0x94,
    0xb6, 0x6c, 0x5e, 0x51, 0x25, 0x96, 0xa5, 0x99, 0x6b, 0x9d, 0xe0, 0x89, 0x70, 0x1d, 0x31, 0x66,
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
