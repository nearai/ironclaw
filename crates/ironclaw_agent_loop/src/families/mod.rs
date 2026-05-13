use std::sync::Arc;

use crate::family::{
    ComponentDigest, ComponentIdentity, LoopFamily, LoopFamilyId, LoopFamilyPlanner,
};

struct DefaultLoopFamilyPlanner;

impl LoopFamilyPlanner for DefaultLoopFamilyPlanner {}

/// The default loop family: the text-tool-use baseline once the planner and
/// executor workstreams land.
pub fn default() -> LoopFamily {
    LoopFamily::new(
        LoopFamilyId::DEFAULT,
        ComponentIdentity::from_static("default", ComponentDigest([0; 32])),
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
        assert_eq!(family.version().digest, ComponentDigest([0; 32]));
    }
}
