use std::sync::Arc;

use crate::default_planner::DefaultPlanner;
use crate::family::{ComponentDigest, LoopFamily};
use crate::planner::AgentLoopPlanner;
use crate::strategies::{DefaultBudgetStrategy, DefaultRecoveryStrategy};

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
    "recovery:DefaultRecoveryStrategy(max_attempts_per_class=2,model_availability_attempts=12),",
    "reply_admission:DefaultReplyAdmissionStrategy(reject_empty_and_provider_transcript_artifacts),",
    "stop:DefaultStopConditionStrategy(window=5,repeat=3,failure_run=3,rejected_reply=invalid_model_output),",
    "drain:DefaultInputDrainStrategy(steering=true,followup=true),",
    "budget:DefaultBudgetStrategy(iteration_limit=1024,wall_clock_limit=none)"
)
.as_bytes();

/// Stable digest: BLAKE3-256 of `DEFAULT_FAMILY_FINGERPRINT`.
///
/// Update this digest when the default family composition, planner behavior, or
/// identity schema changes in a replay-relevant way.
pub const DEFAULT_FAMILY_DIGEST: ComponentDigest = ComponentDigest([
    0x9b, 0x6b, 0x42, 0x36, 0x87, 0xdc, 0xbe, 0xe3, 0x5d, 0x5e, 0x4b, 0x5a, 0x41, 0xb4, 0x14, 0xad,
    0xe3, 0x65, 0x40, 0xd0, 0x5e, 0x85, 0x64, 0xd5, 0x11, 0xfd, 0xbf, 0xf6, 0xab, 0x28, 0x69, 0xbb,
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
/// path without waiting for the production backstop of 1024 iterations.
pub fn default_with_iteration_limit(iteration_limit: u32) -> LoopFamily {
    default_with_overrides(Some(iteration_limit), None)
}

/// The default loop family with optional iteration-limit and model
/// availability-retry overrides. `None` keeps the production default for that
/// knob.
///
/// The availability override shrinks (or deepens) how long the loop rides out
/// provider outages before aborting — test harnesses that script provider
/// failures set it low so a deliberately failed run reaches `Failed` in
/// seconds instead of retrying for minutes.
pub fn default_with_overrides(
    iteration_limit: Option<u32>,
    model_availability_attempts: Option<u32>,
) -> LoopFamily {
    let mut planner = DefaultPlanner::compose_default();
    if let Some(iteration_limit) = iteration_limit {
        planner = planner.with_budget(Arc::new(DefaultBudgetStrategy {
            iteration_limit,
            wall_clock_limit: None,
        }));
    }
    if let Some(max_model_availability_attempts) = model_availability_attempts {
        planner = planner.with_recovery(Arc::new(DefaultRecoveryStrategy {
            max_model_availability_attempts,
            ..DefaultRecoveryStrategy::default()
        }));
    }
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
