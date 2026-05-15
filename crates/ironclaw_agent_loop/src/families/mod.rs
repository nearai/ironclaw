use std::sync::Arc;

use crate::default_planner::DefaultPlanner;
use crate::family::{ComponentDigest, LoopFamily};
use crate::planner::AgentLoopPlanner;

#[cfg(test)]
const DEFAULT_FAMILY_FINGERPRINT: &[u8] = concat!(
    "ironclaw_agent_loop.default_family.v1:",
    "family_id=default;",
    "identity=component_identity_v1;",
    "planner=DefaultPlanner;",
    "strategies=",
    "context:DefaultContextStrategy(max_messages=16),",
    "capability:DefaultCapabilityStrategy(all),",
    "model:DefaultModelStrategy(primary_or_fallback_index),",
    "batch:DefaultBatchPolicyStrategy(exclusive_sequential),",
    "gate:DefaultGateHandlingStrategy(block),",
    "recovery:DefaultRecoveryStrategy(max_attempts_per_class=2),",
    "stop:DefaultStopConditionStrategy(window=5,repeat=3,failure_run=3),",
    "drain:DefaultInputDrainStrategy(steering=true,followup=true),",
    "budget:DefaultBudgetStrategy(iteration_limit=32,wall_clock_limit=none)"
)
.as_bytes();

/// Stable digest: BLAKE3-256 of `DEFAULT_FAMILY_FINGERPRINT`.
///
/// Update this digest when the default family composition, planner behavior, or
/// identity schema changes in a replay-relevant way.
pub const DEFAULT_FAMILY_DIGEST: ComponentDigest = ComponentDigest([
    0x65, 0x5e, 0xde, 0x7b, 0xff, 0x4c, 0x2d, 0x95, 0x70, 0xb5, 0xa2, 0xf7, 0x6f, 0x9c, 0x32, 0x53,
    0x59, 0x19, 0xbe, 0x95, 0xbe, 0xcc, 0x1d, 0xc5, 0x77, 0x47, 0x5f, 0xd1, 0x78, 0xec, 0xbd, 0x93,
]);

/// The default loop family: the text-tool-use baseline once the planner and
/// executor workstreams land.
pub fn default() -> LoopFamily {
    let planner = DefaultPlanner::compose_default();
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
    fn default_family_digest_matches_blake3_fingerprint() {
        assert_eq!(
            DEFAULT_FAMILY_DIGEST,
            ComponentDigest::from_blake3(DEFAULT_FAMILY_FINGERPRINT)
        );
    }
}
