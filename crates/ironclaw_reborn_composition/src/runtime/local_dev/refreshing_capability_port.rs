use std::sync::{Arc, Mutex as StdMutex};

use ironclaw_authorization::CapabilityLeaseStore;
use ironclaw_host_api::{MountView, UserId};
use ironclaw_host_runtime::HostRuntime;
use ironclaw_loop_support::{
    HostRuntimeLoopCapabilityPortFactory, LoopCapabilityInputResolver, LoopCapabilityResultWriter,
};
use ironclaw_product_workflow::{OutboundPreferencesProductFacade, ProjectService};
use ironclaw_run_state::ApprovalRequestStore;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityBatchInvocation, CapabilityBatchOutcome,
    CapabilityCallCandidate, CapabilityInvocation, CapabilityOutcome, LoopCapabilityPort,
    LoopHostMilestoneSink, LoopRunContext, ProviderToolCall, ProviderToolCallCapabilityIds,
    ProviderToolDefinition, VisibleCapabilityRequest, VisibleCapabilitySurface,
};
use tokio::sync::Mutex as AsyncMutex;

use crate::local_dev_capability_policy::LocalDevCapabilityPolicy;
use crate::runtime::LocalDevSelectableSkillContextSource;
use crate::runtime::local_dev::extension_surface::LocalDevExtensionSurfaceSource;
use crate::runtime::local_dev::outbound_delivery::outbound_delivery_capabilities;
use crate::runtime::local_dev::project_create::project_create_capability;
use crate::runtime::local_dev::skill_activation::skill_activation_capability;
use crate::runtime::local_dev::surface_disclosure::wrap_local_dev_surface_disclosure;
use crate::runtime::local_dev::synthetic_capability::wrap_local_dev_synthetic_capabilities;

use super::{capability_io_error, host_api_agent_loop_error, local_dev_visible_capability_request};

pub(super) struct RefreshingLocalDevCapabilityPortConfig {
    pub(super) runtime: Arc<dyn HostRuntime>,
    pub(super) run_context: LoopRunContext,
    pub(super) fallback_user_id: UserId,
    pub(super) policy: Arc<LocalDevCapabilityPolicy>,
    pub(super) workspace_mounts: MountView,
    pub(super) skill_mounts: MountView,
    pub(super) memory_mounts: MountView,
    pub(super) extension_surface_source: LocalDevExtensionSurfaceSource,
    pub(super) input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    pub(super) result_writer: Arc<dyn LoopCapabilityResultWriter>,
    pub(super) milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    pub(super) skill_activation_source: Option<Arc<LocalDevSelectableSkillContextSource>>,
    pub(super) project_service: Arc<dyn ProjectService>,
    pub(super) trajectory_observer: Option<Arc<dyn crate::RebornTrajectoryObserver>>,
    pub(super) outbound_preferences_facade: Option<Arc<dyn OutboundPreferencesProductFacade>>,
    pub(super) outbound_delivery_target_set_requires_approval: bool,
    pub(super) approval_requests: Arc<dyn ApprovalRequestStore>,
    pub(super) capability_leases: Arc<dyn CapabilityLeaseStore>,
}

pub(super) async fn create_refreshing_local_dev_capability_port(
    config: RefreshingLocalDevCapabilityPortConfig,
) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
    let port = Arc::new(RefreshingLocalDevCapabilityPort {
        runtime: config.runtime,
        run_context: config.run_context,
        fallback_user_id: config.fallback_user_id,
        policy: config.policy,
        workspace_mounts: config.workspace_mounts,
        skill_mounts: config.skill_mounts,
        memory_mounts: config.memory_mounts,
        extension_surface_source: config.extension_surface_source,
        input_resolver: config.input_resolver,
        result_writer: config.result_writer,
        milestone_sink: config.milestone_sink,
        skill_activation_source: config.skill_activation_source,
        project_service: config.project_service,
        trajectory_observer: config.trajectory_observer,
        outbound_preferences_facade: config.outbound_preferences_facade,
        outbound_delivery_target_set_requires_approval: config
            .outbound_delivery_target_set_requires_approval,
        approval_requests: config.approval_requests,
        capability_leases: config.capability_leases,
        current: StdMutex::new(None),
        refresh_lock: AsyncMutex::new(()),
    });
    let (initial, _) = port
        .refresh_with_surface(VisibleCapabilityRequest {})
        .await?;
    port.replace_current(initial)?;
    Ok(port)
}

struct RefreshingLocalDevCapabilityPort {
    runtime: Arc<dyn HostRuntime>,
    run_context: LoopRunContext,
    fallback_user_id: UserId,
    policy: Arc<LocalDevCapabilityPolicy>,
    workspace_mounts: MountView,
    skill_mounts: MountView,
    memory_mounts: MountView,
    extension_surface_source: LocalDevExtensionSurfaceSource,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    skill_activation_source: Option<Arc<LocalDevSelectableSkillContextSource>>,
    project_service: Arc<dyn ProjectService>,
    trajectory_observer: Option<Arc<dyn crate::RebornTrajectoryObserver>>,
    outbound_preferences_facade: Option<Arc<dyn OutboundPreferencesProductFacade>>,
    outbound_delivery_target_set_requires_approval: bool,
    approval_requests: Arc<dyn ApprovalRequestStore>,
    capability_leases: Arc<dyn CapabilityLeaseStore>,
    current: StdMutex<Option<Arc<dyn LoopCapabilityPort>>>,
    refresh_lock: AsyncMutex<()>,
}

impl RefreshingLocalDevCapabilityPort {
    async fn build_inner(&self) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let extension_surface = self
            .extension_surface_source
            .snapshot()
            .await
            .map_err(host_api_agent_loop_error)?;
        let visible_request = local_dev_visible_capability_request(
            &self.run_context,
            &self.fallback_user_id,
            self.workspace_mounts.clone(),
            self.skill_mounts.clone(),
            self.memory_mounts.clone(),
            &self.policy,
            &extension_surface,
        )?;
        let mut factory = HostRuntimeLoopCapabilityPortFactory::new(
            Arc::clone(&self.runtime),
            visible_request,
            Arc::clone(&self.input_resolver),
            Arc::clone(&self.result_writer),
            Arc::clone(&self.milestone_sink),
        )
        .with_execution_mounts(self.workspace_mounts.clone())
        // Adapt the composition-owned observer to the loop-support substrate
        // trait the capability port consumes (the input hook). The result hook
        // calls the composition trait directly from `LocalDevCapabilityIo`.
        .with_trajectory_observer(
            self.trajectory_observer
                .clone()
                .map(crate::trajectory_observer::as_capability_observer),
        );
        for capability_id in self.policy.skill_management_capability_ids() {
            factory = factory
                .with_capability_execution_mount(capability_id.clone(), self.skill_mounts.clone());
        }
        for capability_id in self.policy.memory_capability_ids() {
            factory = factory
                .with_capability_execution_mount(capability_id.clone(), self.memory_mounts.clone());
        }
        let port = factory.for_run_context(self.run_context.clone());
        let mut synthetic_capabilities = match &self.skill_activation_source {
            Some(skill_activation_source) => {
                vec![skill_activation_capability(Arc::clone(
                    skill_activation_source,
                ))?]
            }
            None => Vec::new(),
        };
        synthetic_capabilities.push(project_create_capability(
            Arc::clone(&self.project_service),
            self.fallback_user_id.clone(),
        )?);
        if let Some(outbound_preferences_facade) = &self.outbound_preferences_facade {
            synthetic_capabilities.extend(outbound_delivery_capabilities(
                Arc::clone(outbound_preferences_facade),
                self.fallback_user_id.clone(),
                Arc::clone(&self.approval_requests),
                Arc::clone(&self.capability_leases),
                self.outbound_delivery_target_set_requires_approval,
            )?);
        }
        let port = wrap_local_dev_synthetic_capabilities(
            port,
            synthetic_capabilities,
            self.run_context.clone(),
            Arc::clone(&self.input_resolver),
            Arc::clone(&self.result_writer),
            // Synthetic capabilities bypass the inner port's input hook, so the
            // wrapper needs the observer to emit `on_capability_input` itself.
            self.trajectory_observer.clone(),
        )?;
        Ok(wrap_local_dev_surface_disclosure(
            port,
            &self.workspace_mounts,
        ))
    }

    async fn refresh_with_surface(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<(Arc<dyn LoopCapabilityPort>, VisibleCapabilitySurface), AgentLoopHostError> {
        let port = self.build_inner().await?;
        let surface = port.visible_capabilities(request).await?;
        Ok((port, surface))
    }

    fn current_port(&self) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        self.current
            .lock()
            .map_err(|_| capability_io_error())?
            .clone()
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::StaleSurface,
                    "capability surface is unavailable",
                )
            })
    }

    fn replace_current(&self, port: Arc<dyn LoopCapabilityPort>) -> Result<(), AgentLoopHostError> {
        *self.current.lock().map_err(|_| capability_io_error())? = Some(port);
        Ok(())
    }

    async fn refresh_current(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<(Arc<dyn LoopCapabilityPort>, VisibleCapabilitySurface), AgentLoopHostError> {
        let _guard = self.refresh_lock.lock().await;
        let (port, surface) = self.refresh_with_surface(request).await?;
        self.replace_current(port.clone())?;
        Ok((port, surface))
    }

    async fn current_or_refresh(&self) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        match self.current_port() {
            Ok(port) => Ok(port),
            Err(error) if error.kind == AgentLoopHostErrorKind::StaleSurface => {
                let (port, _) = self.refresh_current(VisibleCapabilityRequest {}).await?;
                Ok(port)
            }
            Err(error) => Err(error),
        }
    }
}

#[async_trait::async_trait]
impl LoopCapabilityPort for RefreshingLocalDevCapabilityPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        self.current_port()?.tool_definitions()
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        self.current_port()?
            .provider_tool_call_capability_ids(tool_call)
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        self.current_port()?.validate_provider_tool_call(tool_call)
    }

    async fn register_provider_tool_call(
        &self,
        tool_call: ProviderToolCall,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        self.current_or_refresh()
            .await?
            .register_provider_tool_call(tool_call)
            .await
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        let (_, surface) = self.refresh_current(request).await?;
        Ok(surface)
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        self.current_or_refresh()
            .await?
            .invoke_capability(request)
            .await
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        self.current_or_refresh()
            .await?
            .invoke_capability_batch(request)
            .await
    }
}
