use std::{num::NonZeroU32, sync::Arc};

use ironclaw_agent_loop::{
    families::{self, ToolBatchStrategy},
    family::{LoopFamilyRegistry, LoopFamilyRegistryError},
};

/// Build the production loop-family registry.
///
/// This is the Reborn composition root for loop families. Adding another
/// Builtin family means adding its factory here; the framework crate exports
/// family factories but does not decide which ones are bound in production.
pub fn build_loop_family_registry() -> Result<Arc<LoopFamilyRegistry>, LoopFamilyRegistryError> {
    build_loop_family_registry_with_overrides(None, None, false)
}

pub fn build_loop_family_registry_with_overrides(
    default_iteration_limit: Option<NonZeroU32>,
    model_availability_attempts: Option<NonZeroU32>,
    parallel_tool_batches: bool,
) -> Result<Arc<LoopFamilyRegistry>, LoopFamilyRegistryError> {
    let tool_batch_strategy = if parallel_tool_batches {
        ToolBatchStrategy::BoundedParallel
    } else {
        ToolBatchStrategy::HostBatch
    };
    // `default_with_overrides` returns the pure-default composition (static
    // replay digest included) when no override is set.
    let default_family = families::default_with_overrides(families::FamilyOverrides {
        iteration_limit: default_iteration_limit.map(NonZeroU32::get),
        model_availability_attempts: model_availability_attempts.map(NonZeroU32::get),
        tool_batch_strategy,
    });
    LoopFamilyRegistry::with_families(vec![
        Arc::new(default_family),
        Arc::new(families::subagent_with_tool_batch_strategy(
            tool_batch_strategy,
        )),
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

    #[test]
    fn registry_applies_selected_batch_strategy_to_both_families() {
        use ironclaw_agent_loop::families::{DEFAULT_FAMILY_DIGEST, SUBAGENT_FAMILY_DIGEST};

        // Default strategy: both families keep their static host-batch
        // replay identity.
        let host_batch = build_loop_family_registry().expect("valid production registry");
        assert_eq!(
            host_batch
                .get(&LoopFamilyId::DEFAULT)
                .expect("default family")
                .version()
                .digest,
            DEFAULT_FAMILY_DIGEST
        );
        assert_eq!(
            host_batch
                .get(&LoopFamilyId::SUBAGENT)
                .expect("subagent family")
                .version()
                .digest,
            SUBAGENT_FAMILY_DIGEST
        );

        // Bounded-parallel strategy: both families carry a configuration-
        // specific identity for the same family ids.
        let bounded = build_loop_family_registry_with_overrides(None, None, true)
            .expect("valid production registry");
        for id in [LoopFamilyId::DEFAULT, LoopFamilyId::SUBAGENT] {
            let family = bounded.get(&id).expect("bound family");
            assert_ne!(family.version().digest, DEFAULT_FAMILY_DIGEST);
            assert_ne!(family.version().digest, SUBAGENT_FAMILY_DIGEST);
        }

        // The digest is a pure function of the selected strategy.
        let bounded_again = build_loop_family_registry_with_overrides(None, None, true)
            .expect("valid production registry");
        for id in [LoopFamilyId::DEFAULT, LoopFamilyId::SUBAGENT] {
            assert_eq!(
                bounded_again
                    .get(&id)
                    .expect("bound family")
                    .version()
                    .digest,
                bounded.get(&id).expect("bound family").version().digest
            );
        }
    }
}
