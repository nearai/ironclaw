/// Test double substituting the production `LoopCapabilityPortFactory` wiring
/// (`LocalDevLoopCapabilityPortFactory` / `HostRuntimeLoopCapabilityPortFactory`,
/// `crates/ironclaw_reborn_composition/src/runtime/local_dev.rs`).
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind, GrantConstraints,
    MountView, NetworkPolicy, Principal, SecretHandle, TrustClass,
};
use ironclaw_host_runtime::{CapabilitySurfacePolicy, SurfaceKind};
use ironclaw_loop_support::{HostRuntimeLoopCapabilityPortFactory, LoopCapabilityPortFactory};
use ironclaw_reborn_composition::{
    ProductLiveVisibleCapabilityRequestConfig, visible_capability_request_for_run,
};
use ironclaw_trust::EffectiveTrustClass;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, LoopCapabilityPort, LoopHostMilestoneSink,
    LoopRunContext,
};

use super::super::harness::HostRuntimeCapabilityHarness;
use super::recording_capability_result_writer::RecordingCapabilityResultWriter;
use super::recording_delegating_capability_port::RecordingDelegatingCapabilityPort;

pub(crate) struct HostRuntimeHarnessCapabilityPortFactory {
    pub(crate) harness: Arc<HostRuntimeCapabilityHarness>,
    pub(crate) milestone_sink: Arc<ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink>,
}

#[async_trait]
impl LoopCapabilityPortFactory for HostRuntimeHarnessCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        // C-MULTIUSER: resolve the execution user per run (owner/actor) when the
        // harness opts in, else the fixed harness user. Both the authority scope
        // and the grant grantee MUST use the SAME user so the lease is
        // self-consistent (grantee == execution user) — matching production.
        let dispatch_user = self.harness.dispatch_user_for_run(run_context);
        let mut authority = ProductLiveVisibleCapabilityRequestConfig::new(
            dispatch_user.clone(),
            self.harness.runtime_kind,
            TrustClass::FirstParty,
            SurfaceKind::new("agent_loop").map_err(host_runtime_harness_error)?,
            CapabilitySurfacePolicy::allow_all(),
        )
        .with_mounts(self.harness.mounts.clone())
        .with_grants(capability_grants(
            Principal::User(dispatch_user.clone()),
            &self.harness.capability_ids,
            self.harness.effect_kinds.clone(),
            self.harness.mounts.clone(),
            &self.harness.capability_mount_overrides,
            self.harness.network_policy.clone(),
            self.harness.secrets.clone(),
        ))
        .with_provider_trust_for_effects(
            self.harness.provider_id.clone(),
            EffectiveTrustClass::user_trusted(),
            self.harness.effect_kinds.clone(),
        );
        for (provider, effects) in &self.harness.additional_provider_trust {
            authority = authority.with_provider_trust_for_effects(
                provider.clone(),
                EffectiveTrustClass::user_trusted(),
                effects.clone(),
            );
        }
        let execution_mounts = self.harness.mounts.clone();
        let visible_request = visible_capability_request_for_run(run_context, authority)
            .map_err(host_runtime_harness_error)?;
        let milestone_sink: Arc<dyn LoopHostMilestoneSink> = self.milestone_sink.clone();
        let result_writer = Arc::new(RecordingCapabilityResultWriter {
            inner: self.harness.io.clone(),
            results: Arc::clone(&self.harness.results),
        });
        let mut factory = HostRuntimeLoopCapabilityPortFactory::new(
            Arc::clone(&self.harness.runtime),
            visible_request,
            self.harness.io.clone(),
            result_writer.clone(),
            milestone_sink,
        )
        .with_execution_mounts(execution_mounts);
        for (capability_id, mounts) in &self.harness.capability_mount_overrides {
            factory =
                factory.with_capability_execution_mount(capability_id.clone(), mounts.clone());
        }
        let port = factory.for_run_context(run_context.clone());
        // E-PROJ: see `apply_synthetic_capability_wrappers`'s doc comment.
        let port = self.harness.apply_synthetic_capability_wrappers(
            port,
            run_context,
            self.harness.io.clone(),
            result_writer,
        )?;
        Ok(Arc::new(RecordingDelegatingCapabilityPort {
            inner: port,
            invocations: Arc::clone(&self.harness.invocations),
        }))
    }
}

fn capability_grants(
    grantee: Principal,
    capabilities: &[CapabilityId],
    allowed_effects: Vec<EffectKind>,
    mounts: MountView,
    mount_overrides: &[(CapabilityId, MountView)],
    network: NetworkPolicy,
    secrets: Vec<SecretHandle>,
) -> CapabilitySet {
    CapabilitySet {
        grants: capabilities
            .iter()
            .map(|capability| {
                let mounts = mount_overrides
                    .iter()
                    .find(|(override_capability, _mounts)| override_capability == capability)
                    .map(|(_capability, mounts)| mounts.clone())
                    .unwrap_or_else(|| mounts.clone());
                CapabilityGrant {
                    id: CapabilityGrantId::new(),
                    capability: capability.clone(),
                    grantee: grantee.clone(),
                    issued_by: Principal::HostRuntime,
                    constraints: GrantConstraints {
                        allowed_effects: allowed_effects.clone(),
                        mounts,
                        network: network.clone(),
                        secrets: secrets.clone(),
                        resource_ceiling: None,
                        expires_at: None,
                        max_invocations: None,
                    },
                }
            })
            .collect(),
    }
}

fn host_runtime_harness_error(error: impl std::fmt::Display) -> AgentLoopHostError {
    AgentLoopHostError::new(AgentLoopHostErrorKind::InvalidInvocation, error.to_string())
}
