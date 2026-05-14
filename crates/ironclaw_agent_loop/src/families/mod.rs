use std::sync::Arc;

use crate::family::{
    ComponentDigest, ComponentIdentity, LoopFamily, LoopFamilyId, LoopFamilyPlanner,
};

struct DefaultLoopFamilyPlanner;

impl LoopFamilyPlanner for DefaultLoopFamilyPlanner {}

/// Stable digest: SHA-256 of
/// `ironclaw_agent_loop.default_family.v1:planner=DefaultLoopFamilyPlanner;schema=component_identity_v1;family_id=default`.
///
/// Update this digest when the default family composition, planner behavior, or
/// identity schema changes in a replay-relevant way.
pub const DEFAULT_FAMILY_DIGEST: ComponentDigest = ComponentDigest([
    0xf5, 0xa3, 0x2b, 0xd7, 0x15, 0x2f, 0xf4, 0x9a, 0x2b, 0xb7, 0x92, 0xee, 0x97, 0xe2, 0xa4, 0x54,
    0x35, 0x4f, 0x0d, 0xab, 0x6a, 0x81, 0xcc, 0x3a, 0xbe, 0x35, 0xe9, 0x33, 0x55, 0xc9, 0x2a, 0xcf,
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
}
