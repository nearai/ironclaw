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
    build_loop_family_registry_with_overrides(None, None)
}

pub fn build_loop_family_registry_with_overrides(
    default_iteration_limit: Option<NonZeroU32>,
    model_availability_attempts: Option<NonZeroU32>,
) -> Result<Arc<LoopFamilyRegistry>, LoopFamilyRegistryError> {
    // `default_with_overrides` returns the pure-default composition (static
    // replay digest included) when no override is set.
    let default_family = families::default_with_overrides(families::FamilyOverrides {
        iteration_limit: default_iteration_limit.map(NonZeroU32::get),
        model_availability_attempts: model_availability_attempts.map(NonZeroU32::get),
    });
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
