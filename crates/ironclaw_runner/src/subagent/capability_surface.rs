use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
    SubagentPromptMaterialSource,
};
use ironclaw_turns::run_profile::LoopRunContext;
use tracing::error;

use crate::planned_driver_factory::is_subagent_planned_run_profile;

pub(crate) struct SubagentCapabilitySurfaceResolver {
    inner: Arc<dyn CapabilitySurfaceProfileResolver>,
    material_source: Arc<dyn SubagentPromptMaterialSource>,
}

impl SubagentCapabilitySurfaceResolver {
    pub(crate) fn new(
        inner: Arc<dyn CapabilitySurfaceProfileResolver>,
        material_source: Arc<dyn SubagentPromptMaterialSource>,
    ) -> Self {
        Self {
            inner,
            material_source,
        }
    }
}

#[async_trait]
impl CapabilitySurfaceProfileResolver for SubagentCapabilitySurfaceResolver {
    async fn resolve(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        let base = self.inner.resolve(run_context).await?;
        if !is_subagent_planned_run_profile(run_context) {
            return Ok(base);
        }
        let material = self
            .material_source
            .material_for_run(run_context)
            .await
            .map_err(|error| CapabilityResolveError::unavailable(error.safe_summary))?;
        intersect_allow_sets(
            base,
            CapabilityAllowSet::allowlist(material.allowed_capabilities),
        )
    }
}

fn intersect_allow_sets(
    left: CapabilityAllowSet,
    right: CapabilityAllowSet,
) -> Result<CapabilityAllowSet, CapabilityResolveError> {
    match (left, right) {
        (CapabilityAllowSet::All, other) | (other, CapabilityAllowSet::All) => Ok(other),
        (CapabilityAllowSet::Allowlist(left), CapabilityAllowSet::Allowlist(right)) => Ok(
            CapabilityAllowSet::allowlist(left.into_iter().filter(|id| right.contains(id))),
        ),
        unexpected => {
            let reason = "unhandled CapabilityAllowSet variant in subagent allowlist intersection";
            error!(?unexpected, "{reason}");
            Err(CapabilityResolveError::internal(reason))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, sync::Arc};

    use async_trait::async_trait;
    use ironclaw_agent_loop::test_support::test_run_context;
    use ironclaw_host_api::CapabilityId;
    use ironclaw_loop_support::CapabilitySurfaceProfileResolver;
    use ironclaw_loop_support::{
        CapabilityAllowSet, CapabilityResolveError, SubagentPromptMaterial,
        SubagentPromptMaterialSource,
    };
    use ironclaw_turns::run_profile::{AgentLoopHostError, AgentLoopHostErrorKind, LoopRunContext};
    use ironclaw_turns::{RunProfileId, RunProfileVersion};

    use crate::planned_driver_factory::{
        PLANNED_DRIVER_DEFAULT_VERSION, SUBAGENT_PLANNED_DRIVER_ID, SUBAGENT_PLANNED_PROFILE_ID,
    };

    use super::{SubagentCapabilitySurfaceResolver, intersect_allow_sets};

    fn cap(value: &str) -> CapabilityId {
        CapabilityId::new(value).expect("valid capability id")
    }

    fn planned_subagent_context() -> LoopRunContext {
        let mut context = test_run_context("subagent-capability-surface");
        context.resolved_run_profile.profile_id =
            RunProfileId::new(SUBAGENT_PLANNED_PROFILE_ID).expect("subagent planned profile id");
        context.resolved_run_profile.loop_driver.id =
            ironclaw_turns::run_profile::LoopDriverId::new(SUBAGENT_PLANNED_DRIVER_ID)
                .expect("subagent planned driver id");
        context.resolved_run_profile.loop_driver.version =
            RunProfileVersion::new(PLANNED_DRIVER_DEFAULT_VERSION);
        context
    }

    fn allowlist(ids: &[&str]) -> CapabilityAllowSet {
        CapabilityAllowSet::allowlist(ids.iter().copied().map(cap))
    }

    struct StaticResolver(CapabilityAllowSet);

    #[async_trait]
    impl ironclaw_loop_support::CapabilitySurfaceProfileResolver for StaticResolver {
        async fn resolve(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
            Ok(self.0.clone())
        }
    }

    struct FailingSource;

    #[async_trait]
    impl SubagentPromptMaterialSource for FailingSource {
        async fn material_for_run(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<SubagentPromptMaterial, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "prompt material unavailable for test",
            ))
        }
    }

    struct SucceedingSource(SubagentPromptMaterial);

    #[async_trait]
    impl SubagentPromptMaterialSource for SucceedingSource {
        async fn material_for_run(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<SubagentPromptMaterial, AgentLoopHostError> {
            Ok(self.0.clone())
        }
    }

    struct FailingResolver;

    #[async_trait]
    impl ironclaw_loop_support::CapabilitySurfaceProfileResolver for FailingResolver {
        async fn resolve(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
            Err(CapabilityResolveError::internal(
                "inner resolver failed for test",
            ))
        }
    }

    #[test]
    fn intersect_allow_sets_all_all_returns_all() {
        assert_eq!(
            intersect_allow_sets(CapabilityAllowSet::All, CapabilityAllowSet::All).unwrap(),
            CapabilityAllowSet::All
        );
    }

    #[test]
    fn intersect_allow_sets_allowlist_all_returns_allowlist() {
        assert_eq!(
            intersect_allow_sets(allowlist(&["builtin.read_file"]), CapabilityAllowSet::All)
                .unwrap(),
            allowlist(&["builtin.read_file"])
        );
    }

    #[test]
    fn intersect_allow_sets_allowlist_allowlist_empty_intersection() {
        assert_eq!(
            intersect_allow_sets(
                allowlist(&["builtin.read_file"]),
                allowlist(&["builtin.write_file"])
            )
            .unwrap(),
            CapabilityAllowSet::allowlist(BTreeSet::new())
        );
    }

    #[tokio::test]
    async fn resolve_propagates_material_source_error_as_unavailable() {
        let resolver = SubagentCapabilitySurfaceResolver::new(
            Arc::new(StaticResolver(CapabilityAllowSet::All)),
            Arc::new(FailingSource),
        );

        let error = resolver
            .resolve(&planned_subagent_context())
            .await
            .expect_err("material source failure should surface as unavailable");

        match error {
            CapabilityResolveError::Unavailable { reason } => {
                assert_eq!(reason, "prompt material unavailable for test");
            }
            other => panic!("unexpected resolve error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn resolve_propagates_inner_resolver_error_on_subagent_context() {
        let resolver = SubagentCapabilitySurfaceResolver::new(
            Arc::new(FailingResolver),
            Arc::new(FailingSource),
        );

        let error = resolver
            .resolve(&planned_subagent_context())
            .await
            .expect_err("inner resolver failure should surface unchanged");

        match error {
            CapabilityResolveError::Internal { reason } => {
                assert_eq!(reason, "inner resolver failed for test");
            }
            other => panic!("unexpected resolve error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn resolve_intersects_base_and_material_allowsets_for_subagent_context() {
        let base = allowlist(&["builtin.read_file", "builtin.write_file"]);
        let material_caps =
            BTreeSet::from([cap("builtin.read_file"), cap("builtin.spawn_subagent")]);
        let resolver = SubagentCapabilitySurfaceResolver::new(
            Arc::new(StaticResolver(base.clone())),
            Arc::new(SucceedingSource(SubagentPromptMaterial {
                direction_markdown: "test".to_string(),
                goal: ironclaw_loop_support::SubagentPromptGoal {
                    task: "test".to_string(),
                    handoff: None,
                },
                allowed_capabilities: material_caps,
            })),
        );

        let resolved = resolver
            .resolve(&planned_subagent_context())
            .await
            .expect("subagent runs should intersect base and material allowsets");

        assert_eq!(resolved, allowlist(&["builtin.read_file"]));
    }

    #[tokio::test]
    async fn resolve_returns_material_allowset_when_base_is_all_for_subagent_context() {
        let material_caps =
            BTreeSet::from([cap("builtin.read_file"), cap("builtin.spawn_subagent")]);
        let resolver = SubagentCapabilitySurfaceResolver::new(
            Arc::new(StaticResolver(CapabilityAllowSet::All)),
            Arc::new(SucceedingSource(SubagentPromptMaterial {
                direction_markdown: "test".to_string(),
                goal: ironclaw_loop_support::SubagentPromptGoal {
                    task: "test".to_string(),
                    handoff: None,
                },
                allowed_capabilities: material_caps.clone(),
            })),
        );

        let resolved = resolver
            .resolve(&planned_subagent_context())
            .await
            .expect("subagent runs with an all base allowset should return the material allowset");

        assert_eq!(resolved, CapabilityAllowSet::allowlist(material_caps));
    }

    #[tokio::test]
    async fn resolve_returns_base_allowset_for_non_subagent_runs_without_material_source() {
        let base = allowlist(&["builtin.read_file", "builtin.write_file"]);
        let resolver = SubagentCapabilitySurfaceResolver::new(
            Arc::new(StaticResolver(base.clone())),
            Arc::new(FailingSource),
        );

        let resolved = resolver
            .resolve(&test_run_context("non-subagent-capability-surface"))
            .await
            .expect("non-subagent runs should return the base allowset");

        assert_eq!(resolved, base);
    }
}
