use std::sync::Arc;

use crate::default_planner::DefaultPlanner;
use crate::family::LoopFamily;
use crate::planner::AgentLoopPlanner;

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
    use super::*;

    #[test]
    fn default_family_has_default_identity() {
        let family = default();

        assert_eq!(family.id(), &crate::family::LoopFamilyId::DEFAULT);
        assert_eq!(family.version().id, "default");
        assert_eq!(
            family.version().digest,
            crate::family::ComponentDigest([0; 32])
        );
    }
}
