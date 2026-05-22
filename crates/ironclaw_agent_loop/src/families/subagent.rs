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
    "stop:DefaultStopConditionStrategy(window=5,repeat=3,failure_run=3),",
    "drain:DefaultInputDrainStrategy(steering=true,followup=true),",
    "budget:DefaultBudgetStrategy(iteration_limit=16,wall_clock_limit=none)"
)
.as_bytes();

pub const SUBAGENT_FAMILY_DIGEST: ComponentDigest = ComponentDigest([
    0xec, 0xe7, 0x1c, 0x75, 0x33, 0x42, 0x67, 0x31, 0x4a, 0x1d, 0xc9, 0x96, 0x18, 0x4a, 0x2a, 0xf5,
    0x57, 0x1a, 0xe0, 0x0a, 0xfa, 0xc0, 0xce, 0x3b, 0x7b, 0x31, 0x80, 0x01, 0x00, 0xae, 0x92, 0xa3,
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
