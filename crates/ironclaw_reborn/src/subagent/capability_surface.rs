use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
    SubagentPromptMaterialSource,
};
use ironclaw_turns::run_profile::LoopRunContext;

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
        Ok(intersect_allow_sets(
            base,
            CapabilityAllowSet::allowlist(material.allowed_capabilities),
        ))
    }
}

fn intersect_allow_sets(left: CapabilityAllowSet, right: CapabilityAllowSet) -> CapabilityAllowSet {
    match (left, right) {
        (CapabilityAllowSet::All, other) | (other, CapabilityAllowSet::All) => other,
        (CapabilityAllowSet::Allowlist(left), CapabilityAllowSet::Allowlist(right)) => {
            CapabilityAllowSet::allowlist(left.into_iter().filter(|id| right.contains(id)))
        }
        _ => CapabilityAllowSet::allowlist([]),
    }
}
