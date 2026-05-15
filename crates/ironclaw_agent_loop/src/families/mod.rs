use std::sync::Arc;

use crate::family::{
    ComponentDigest, ComponentIdentity, LoopFamily, LoopFamilyId, LoopFamilyPlanner,
};

struct DefaultLoopFamilyPlanner;

impl LoopFamilyPlanner for DefaultLoopFamilyPlanner {}

#[cfg(test)]
const DEFAULT_FAMILY_FINGERPRINT: &[u8] =
    b"ironclaw_agent_loop.default_family.v1:planner=DefaultLoopFamilyPlanner;schema=component_identity_v1;family_id=default";

/// Stable digest: BLAKE3-256 of `DEFAULT_FAMILY_FINGERPRINT`.
///
/// Update this digest when the default family composition, planner behavior, or
/// identity schema changes in a replay-relevant way.
pub const DEFAULT_FAMILY_DIGEST: ComponentDigest = ComponentDigest([
    0x40, 0xe2, 0xeb, 0x31, 0x69, 0x81, 0x22, 0x31, 0x39, 0x76, 0x00, 0x25, 0x49, 0x4a, 0x0e, 0x14,
    0xb5, 0xa1, 0x7a, 0x0a, 0x57, 0x59, 0x7d, 0xcd, 0xa7, 0x48, 0xae, 0x38, 0x11, 0x75, 0xf8, 0x0f,
]);

/// The default loop family: the text-tool-use baseline once the planner and
/// executor workstreams land.
pub fn default() -> LoopFamily {
    LoopFamily::new(
        LoopFamilyId::DEFAULT,
        ComponentIdentity::from_static("default", DEFAULT_FAMILY_DIGEST),
        Arc::new(DefaultLoopFamilyPlanner),
    )
}

#[cfg(test)]
mod tests {
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
