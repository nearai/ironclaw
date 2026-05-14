use std::sync::Arc;

use crate::default_planner::DefaultPlanner;
use crate::family::{ComponentDigest, LoopFamily};
use crate::planner::AgentLoopPlanner;

pub const DEFAULT_FAMILY_DIGEST_SEED: &str = concat!(
    "ironclaw_agent_loop.default_family.v1:",
    "family_id=default;",
    "identity=component_identity_v1;",
    "planner=DefaultPlanner;",
    "strategies=",
    "context:DefaultContextStrategy,",
    "capability:DefaultCapabilityStrategy,",
    "model:DefaultModelStrategy,",
    "batch:DefaultBatchPolicyStrategy,",
    "gate:DefaultGateHandlingStrategy,",
    "recovery:DefaultRecoveryStrategy,",
    "stop:DefaultStopConditionStrategy,",
    "drain:DefaultInputDrainStrategy,",
    "budget:DefaultBudgetStrategy",
);

/// Stable digest: SHA-256 of `DEFAULT_FAMILY_DIGEST_SEED`.
///
/// Update this digest when the default family composition, planner behavior, or
/// identity schema changes in a replay-relevant way.
pub const DEFAULT_FAMILY_DIGEST: ComponentDigest = ComponentDigest([
    0x12, 0x0d, 0xb1, 0x6b, 0x2b, 0x95, 0xe8, 0xde, 0x59, 0x51, 0x7e, 0x8a, 0x2e, 0x30, 0xeb, 0x98,
    0x60, 0xf9, 0xb9, 0x74, 0xc1, 0xb2, 0xd0, 0x57, 0x92, 0x55, 0x01, 0x8f, 0x9c, 0xaa, 0xf2, 0x82,
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
    use sha2::{Digest, Sha256};

    use super::*;

    #[test]
    fn default_family_has_default_identity() {
        let family = default();

        assert_eq!(family.id(), &crate::family::LoopFamilyId::DEFAULT);
        assert_eq!(family.version().id, "default");
        assert_ne!(family.version().digest, ComponentDigest([0; 32]));
        assert_eq!(family.version().digest, DEFAULT_FAMILY_DIGEST);
    }

    #[test]
    fn default_family_digest_matches_current_seed() {
        let actual: [u8; 32] = Sha256::digest(DEFAULT_FAMILY_DIGEST_SEED)
            .as_slice()
            .try_into()
            .expect("sha256 digest length is 32 bytes");

        assert_eq!(DEFAULT_FAMILY_DIGEST, ComponentDigest(actual));
    }
}
