use std::{num::NonZeroU32, sync::Arc};

use ironclaw_agent_loop::{
    families,
    family::{LoopFamilyRegistry, LoopFamilyRegistryError},
};

/// Build the production loop-family registry.
///
/// This is the Reborn composition root for loop families. Adding another
/// Builtin family means adding its factory here; the framework crate exports
/// family factories but does not decide which ones are bound in production.
pub fn build_loop_family_registry() -> Result<Arc<LoopFamilyRegistry>, LoopFamilyRegistryError> {
    build_loop_family_registry_with_default_iteration_limit(None)
}

pub fn build_loop_family_registry_with_default_iteration_limit(
    default_iteration_limit: Option<NonZeroU32>,
) -> Result<Arc<LoopFamilyRegistry>, LoopFamilyRegistryError> {
    let default_family = match default_iteration_limit {
        Some(iteration_limit) => families::default_with_iteration_limit(iteration_limit.get()),
        None => families::default(),
    };
    LoopFamilyRegistry::with_families(vec![
        Arc::new(default_family),
        Arc::new(families::subagent()),
    ])
}

#[cfg(test)]
mod tests {
    use ironclaw_agent_loop::family::LoopFamilyId;

    use super::*;

    #[test]
    fn production_registry_binds_default_and_subagent_families() {
        let registry = build_loop_family_registry().expect("valid production registry");

        assert!(registry.get(&LoopFamilyId::DEFAULT).is_some());
        assert!(registry.get(&LoopFamilyId::SUBAGENT).is_some());
        assert!(
            registry
                .get(&LoopFamilyId::new("unknown").expect("valid test id"))
                .is_none()
        );
        assert_eq!(registry.ids().count(), 2);
    }
}
