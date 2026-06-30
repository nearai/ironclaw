use std::sync::Arc;

use crate::default_planner::DefaultPlanner;
use crate::family::{ComponentDigest, LoopFamily};
use crate::planner::AgentLoopPlanner;
use crate::strategies::DefaultBudgetStrategy;

mod subagent;

pub use subagent::{SUBAGENT_FAMILY_DIGEST, subagent};

#[cfg(test)]
const DEFAULT_FAMILY_FINGERPRINT: &[u8] = concat!(
    "ironclaw_agent_loop.default_family.v1:",
    "family_id=default;",
    "identity=component_identity_v1;",
    "planner=DefaultPlanner;",
    "strategies=",
    "context:DefaultContextStrategy(max_messages=128),",
    "compaction:ActiveTaskPreservingCompactionStrategy(context_limit=128000,reserve=20000,preserve_tail=8000,min_compacted=3,min_tail=3,deadline_ms=30000),",
    "capability:DefaultCapabilityStrategy(all),",
    "model:DefaultModelStrategy(primary_or_fallback_index),",
    "batch:DefaultBatchPolicyStrategy(exclusive_sequential),",
    "gate:DefaultGateHandlingStrategy(block),",
    "recovery:DefaultRecoveryStrategy(max_attempts_per_class=2),",
    "reply_admission:DefaultReplyAdmissionStrategy(reject_empty_and_provider_transcript_artifacts),",
    "stop:DefaultStopConditionStrategy(window=5,repeat=3,failure_run=3,rejected_reply=invalid_model_output),",
    "drain:DefaultInputDrainStrategy(steering=true,followup=true),",
    "budget:DefaultBudgetStrategy(iteration_limit=32,wall_clock_limit=none)"
)
.as_bytes();

/// Stable digest: BLAKE3-256 of `DEFAULT_FAMILY_FINGERPRINT`.
///
/// Update this digest when the default family composition, planner behavior, or
/// identity schema changes in a replay-relevant way.
pub const DEFAULT_FAMILY_DIGEST: ComponentDigest = ComponentDigest([
    0xdd, 0x1f, 0x20, 0xe1, 0x17, 0xde, 0xcb, 0xe2, 0x2d, 0x48, 0x15, 0x8b, 0x05, 0x19, 0x27, 0xc4,
    0x2f, 0xf6, 0x85, 0xd9, 0x43, 0x27, 0x25, 0x37, 0xe8, 0x38, 0x7c, 0xe6, 0xd1, 0xe5, 0xe7, 0x25,
]);

/// The default loop family: the text-tool-use baseline.
pub fn default() -> LoopFamily {
    let planner = DefaultPlanner::compose_default();
    let id = planner.id().clone();
    let version = planner.version().clone();

    LoopFamily::new(id, version, Arc::new(planner))
}

/// The default loop family with a caller-supplied iteration limit.
///
/// Intended for test and local harnesses that need to exercise the hard budget
/// path without waiting for the production default of 32 iterations.
pub fn default_with_iteration_limit(iteration_limit: u32) -> LoopFamily {
    let planner = DefaultPlanner::compose_default().with_budget(Arc::new(DefaultBudgetStrategy {
        iteration_limit,
        wall_clock_limit: None,
    }));
    let id = planner.id().clone();
    let version = planner.version().clone();

    LoopFamily::new(id, version, Arc::new(planner))
}

#[cfg(test)]
mod tests {
    use crate::family::LoopFamilyId;

    use super::*;

    #[test]
    fn default_family_has_default_identity() {
        let family = default();

        assert_eq!(family.id(), &LoopFamilyId::DEFAULT);
        assert_eq!(family.version().id, "default");
        assert_ne!(family.version().digest, ComponentDigest([0; 32]));
        assert_eq!(family.version().digest, DEFAULT_FAMILY_DIGEST);
    }

    #[test]
    fn default_family_iteration_limit_can_be_overridden_for_harnesses() {
        let family = default_with_iteration_limit(5);
        let context = crate::test_support::test_run_context("default-family-budget-override");
        let state = crate::state::LoopExecutionState::initial_for_run(&context);

        assert_eq!(family.planner().budget().iteration_limit(&state), 5);
    }

    #[test]
    fn default_family_digest_matches_blake3_fingerprint() {
        assert_eq!(
            DEFAULT_FAMILY_DIGEST,
            ComponentDigest::from_blake3(DEFAULT_FAMILY_FINGERPRINT)
        );
    }
}
