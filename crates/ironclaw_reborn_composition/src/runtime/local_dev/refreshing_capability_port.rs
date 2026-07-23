use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex as StdMutex};

use ironclaw_authorization::CapabilityLeaseStore;
use ironclaw_host_api::{
    CapabilityId, ExtensionId, MountView, Resolution, ResolutionBatch, UserId,
};
use ironclaw_host_runtime::HostRuntime;
use ironclaw_loop_host::{
    HostRuntimeLoopCapabilityPortFactory, LoopCapabilityInputResolver, LoopCapabilityResultWriter,
};
use ironclaw_product::{
    LifecycleProductContext, LifecycleProductSurfaceContext, OutboundPreferencesProductFacade,
    ProjectService,
};
use ironclaw_run_state::ApprovalRequestStore;
use ironclaw_threads::SessionThreadService;
use ironclaw_trust::TrustDecision;
use ironclaw_turns::ExternalToolCatalog;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityCallCandidate, LoopCapabilityPort,
    LoopHostMilestoneSink, LoopRequest, LoopRequestBatch, LoopRunContext, ProviderToolCall,
    ProviderToolCallCapabilityIds, ProviderToolDefinition, RegisterProviderToolCallRequest,
    VisibleCapabilityRequest, VisibleCapabilitySurface,
};
use tokio::sync::Mutex as AsyncMutex;

use crate::builtin_capability_policy::BuiltinCapabilityPolicy;
use crate::profile_approval_authorization::ApprovalSettingsProvider;
use crate::runtime::ComposedSelectableSkillContextSource;
use crate::runtime::extension_surface::ExtensionCapabilitySurfaceSource;
use crate::runtime::local_dev::external_tool_capability::wrap_external_tools;
use crate::runtime::local_dev::outbound_delivery::outbound_delivery_capabilities;
use crate::runtime::local_dev::project_create::project_create_capability;
use crate::runtime::local_dev::result_read::result_read_capability;
use crate::runtime::local_dev::skill_activation::skill_activation_capability;
use crate::runtime::local_dev::surface_disclosure::wrap_surface_disclosure;
use crate::runtime::local_dev::synthetic_capability::wrap_synthetic_capabilities;

use super::{
    VisibleCapabilityInputs, capability_io_error, host_api_agent_loop_error,
    local_dev_resource_scope_for_run, visible_capability_request,
};

pub(crate) struct RefreshingCapabilityPortConfig {
    pub(super) runtime: Arc<dyn HostRuntime>,
    pub(super) run_context: LoopRunContext,
    pub(super) fallback_user_id: UserId,
    pub(super) policy: Arc<BuiltinCapabilityPolicy>,
    pub(super) workspace_mounts: MountView,
    pub(super) skill_mounts: MountView,
    pub(super) memory_mounts: MountView,
    pub(super) system_extensions_lifecycle_mounts: MountView,
    pub(super) extension_surface_source: ExtensionCapabilitySurfaceSource,
    pub(super) input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    pub(super) result_writer: Arc<dyn LoopCapabilityResultWriter>,
    pub(super) milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    pub(super) skill_activation_source: Option<Arc<ComposedSelectableSkillContextSource>>,
    pub(super) project_service: Arc<dyn ProjectService>,
    pub(super) thread_service: Arc<dyn SessionThreadService>,
    pub(super) trajectory_observer: Option<Arc<dyn crate::RebornTrajectoryObserver>>,
    pub(super) outbound_preferences_facade: Option<Arc<dyn OutboundPreferencesProductFacade>>,
    pub(super) outbound_delivery_target_set_requires_approval: bool,
    pub(super) approval_settings: Arc<dyn ApprovalSettingsProvider>,
    pub(super) approval_requests: Arc<dyn ApprovalRequestStore>,
    pub(super) capability_leases: Arc<dyn CapabilityLeaseStore>,
    /// Durable model-visible gate-record store the built capability port persists
    /// pending-gate records into (wires the #6245 production gap closed).
    pub(super) gate_record_store: Arc<dyn ironclaw_run_state::GateRecordStore>,
    /// Durable host-private replay-payload store the built capability port
    /// persists gate/auth replay payloads into and reconstitutes on resume
    /// (arch-simplification §5.3 Stage 2a-i).
    pub(super) replay_payload_store: Arc<dyn ironclaw_capabilities::ReplayPayloadStore>,
    pub(super) external_tool_catalog: Arc<dyn ExternalToolCatalog>,
    /// Per-capability mount overrides, merged via `with_capability_execution_mount`.
    /// Always empty at the sole production call site (`local_dev.rs`'s `create_capability_port`);
    /// populated only by the `test-support` constructor.
    pub(super) capability_execution_mount_overrides: HashMap<CapabilityId, MountView>,
    /// Extra provider-trust entries merged into the visible request's
    /// provider-trust map. Always empty at the sole production call site
    /// (`local_dev.rs`'s `create_capability_port`); populated only by the `test-support` constructor.
    pub(super) additional_provider_trust: BTreeMap<ExtensionId, TrustDecision>,
    /// Narrows the FULL granted-capability set (builtin grants plus any
    /// appended extension grants) to this id set via `retain` in
    /// `build_inner`. `None` = no filtering (production's value at the sole
    /// call site, `local_dev.rs`'s `create_capability_port`); `Some(set)`
    /// keeps exactly `set`, including `Some(empty)` = zero grants.
    pub(super) capability_id_filter: Option<HashSet<CapabilityId>>,
    /// Synthetic grants for capability ids that neither the static builtin
    /// policy nor `extension_surface_source` produces (ad-hoc test-only
    /// `HostRuntime` backends). Applied in `build_inner` before
    /// `capability_id_filter`'s retain, only for ids not already granted.
    /// Always empty at the sole production call site (`local_dev.rs`'s
    /// `create_capability_port`); populated only by the `test-support`
    /// constructor.
    pub(super) additional_capability_grants: Vec<ironclaw_host_api::CapabilityGrant>,
}

pub(crate) async fn create_refreshing_capability_port(
    config: RefreshingCapabilityPortConfig,
) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
    let port = Arc::new(RefreshingCapabilityPort {
        runtime: config.runtime,
        run_context: config.run_context,
        fallback_user_id: config.fallback_user_id,
        policy: config.policy,
        workspace_mounts: config.workspace_mounts,
        skill_mounts: config.skill_mounts,
        memory_mounts: config.memory_mounts,
        system_extensions_lifecycle_mounts: config.system_extensions_lifecycle_mounts,
        extension_surface_source: config.extension_surface_source,
        input_resolver: config.input_resolver,
        result_writer: config.result_writer,
        milestone_sink: config.milestone_sink,
        skill_activation_source: config.skill_activation_source,
        project_service: config.project_service,
        thread_service: config.thread_service,
        trajectory_observer: config.trajectory_observer,
        outbound_preferences_facade: config.outbound_preferences_facade,
        outbound_delivery_target_set_requires_approval: config
            .outbound_delivery_target_set_requires_approval,
        approval_settings: config.approval_settings,
        approval_requests: config.approval_requests,
        capability_leases: config.capability_leases,
        gate_record_store: config.gate_record_store,
        replay_payload_store: config.replay_payload_store,
        external_tool_catalog: config.external_tool_catalog,
        capability_execution_mount_overrides: config.capability_execution_mount_overrides,
        additional_provider_trust: config.additional_provider_trust,
        capability_id_filter: config.capability_id_filter,
        additional_capability_grants: config.additional_capability_grants,
        current: StdMutex::new(None),
        refresh_lock: AsyncMutex::new(()),
    });
    let (initial, _) = port
        .refresh_with_surface(VisibleCapabilityRequest {})
        .await?;
    port.replace_current(initial)?;
    Ok(port)
}

struct RefreshingCapabilityPort {
    runtime: Arc<dyn HostRuntime>,
    run_context: LoopRunContext,
    fallback_user_id: UserId,
    policy: Arc<BuiltinCapabilityPolicy>,
    workspace_mounts: MountView,
    skill_mounts: MountView,
    memory_mounts: MountView,
    system_extensions_lifecycle_mounts: MountView,
    extension_surface_source: ExtensionCapabilitySurfaceSource,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    skill_activation_source: Option<Arc<ComposedSelectableSkillContextSource>>,
    project_service: Arc<dyn ProjectService>,
    thread_service: Arc<dyn SessionThreadService>,
    trajectory_observer: Option<Arc<dyn crate::RebornTrajectoryObserver>>,
    outbound_preferences_facade: Option<Arc<dyn OutboundPreferencesProductFacade>>,
    outbound_delivery_target_set_requires_approval: bool,
    approval_settings: Arc<dyn ApprovalSettingsProvider>,
    approval_requests: Arc<dyn ApprovalRequestStore>,
    capability_leases: Arc<dyn CapabilityLeaseStore>,
    gate_record_store: Arc<dyn ironclaw_run_state::GateRecordStore>,
    replay_payload_store: Arc<dyn ironclaw_capabilities::ReplayPayloadStore>,
    external_tool_catalog: Arc<dyn ExternalToolCatalog>,
    capability_execution_mount_overrides: HashMap<CapabilityId, MountView>,
    additional_provider_trust: BTreeMap<ExtensionId, TrustDecision>,
    capability_id_filter: Option<HashSet<CapabilityId>>,
    additional_capability_grants: Vec<ironclaw_host_api::CapabilityGrant>,
    current: StdMutex<Option<Arc<dyn LoopCapabilityPort>>>,
    refresh_lock: AsyncMutex<()>,
}

impl RefreshingCapabilityPort {
    async fn build_inner(&self) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let caller_scope =
            local_dev_resource_scope_for_run(&self.run_context, &self.fallback_user_id);
        let extension_surface = self
            .extension_surface_source
            .snapshot(LifecycleProductContext::Surface(
                LifecycleProductSurfaceContext {
                    tenant_id: caller_scope.tenant_id,
                    user_id: caller_scope.user_id,
                    agent_id: caller_scope.agent_id,
                    project_id: caller_scope.project_id,
                },
            ))
            .await
            .map_err(host_api_agent_loop_error)?;
        let mut visible_request = visible_capability_request(
            &self.run_context,
            &self.fallback_user_id,
            VisibleCapabilityInputs {
                workspace_mounts: &self.workspace_mounts,
                skill_mounts: &self.skill_mounts,
                memory_mounts: &self.memory_mounts,
                system_extensions_lifecycle_mounts: &self.system_extensions_lifecycle_mounts,
                policy: &self.policy,
                extension_surface: &extension_surface,
            },
        )?;
        // Test-support-only synthetic grants (empty in production; see the
        // `additional_capability_grants` doc-comment for the invariant).
        // Overlays network+secrets onto a same-id builtin grant, or inserts
        // the whole synthetic grant when the id has no grant at all.
        if !self.additional_capability_grants.is_empty() {
            let mut missing: Vec<ironclaw_host_api::CapabilityGrant> = Vec::new();
            for synthetic in &self.additional_capability_grants {
                if extension_surface
                    .capability(&synthetic.capability)
                    .is_some()
                {
                    continue;
                }
                match visible_request
                    .context
                    .grants
                    .grants
                    .iter_mut()
                    .find(|existing| existing.capability == synthetic.capability)
                {
                    Some(existing) => {
                        existing.constraints.network = synthetic.constraints.network.clone();
                        existing.constraints.secrets = synthetic.constraints.secrets.clone();
                    }
                    None => missing.push(synthetic.clone()),
                }
            }
            visible_request.context.grants.grants.extend(missing);
        }
        // Test-support-only grant-constraint mount overrides (empty in
        // production; see `capability_execution_mount_overrides`'s
        // doc-comment). Authorization checks the grant's mounts, so this must
        // apply to the grant constraints, not just execution mounts.
        if !self.capability_execution_mount_overrides.is_empty() {
            for grant in &mut visible_request.context.grants.grants {
                if let Some(mounts) = self
                    .capability_execution_mount_overrides
                    .get(&grant.capability)
                {
                    grant.constraints.mounts = mounts.clone();
                }
            }
        }
        // Test-support-only narrowing (`None` in production; see the config
        // field doc-comment). `Some(set)` retains exactly `set`, including
        // `Some(empty)` = zero grants.
        if let Some(filter) = &self.capability_id_filter {
            visible_request
                .context
                .grants
                .grants
                .retain(|grant| filter.contains(&grant.capability));
        }
        // Test-support-only extra provider-trust entries (empty in production,
        // see the config field doc-comment): merge after the canonical helper
        // has built the base provider-trust map, so the production helper
        // stays byte-identical to the pre-seam version. Overwrite semantics
        // (a test entry wins over a same-id baseline entry) — pinned by
        // `additional_provider_trust_is_forwarded_to_visible_request`, so a
        // harness that supplies an entry for the builtin provider must supply
        // its FULL effect set (the integration harness's primary-provider
        // entry carries the profile's whole `effect_kinds` list).
        if !self.additional_provider_trust.is_empty() {
            visible_request
                .provider_trust
                .extend(self.additional_provider_trust.clone());
        }
        let mut factory = HostRuntimeLoopCapabilityPortFactory::new(
            Arc::clone(&self.runtime),
            visible_request,
            Arc::clone(&self.input_resolver),
            Arc::clone(&self.result_writer),
            Arc::clone(&self.milestone_sink),
        )
        .with_execution_mounts(self.workspace_mounts.clone())
        // Durable gate-record + host-private replay-payload stores (§5.2.9 /
        // §5.3 Stage 2a-i): without these the port defaults to a no-op gate
        // store and a fail-closed replay store, so production gate records never
        // persist and a gate/auth resume cannot reconstitute its replay input.
        .with_gate_record_store(Arc::clone(&self.gate_record_store))
        .with_replay_payload_store(Arc::clone(&self.replay_payload_store))
        // Adapt the composition-owned observer to the loop-host substrate
        // trait the capability port consumes (the input hook). The result hook
        // calls the composition trait directly from `StagedCapabilityIo`.
        .with_trajectory_observer(
            self.trajectory_observer
                .clone()
                .map(crate::observability::trajectory_observer::as_capability_observer),
        );
        for capability_id in self.policy.skill_management_capability_ids() {
            factory = factory
                .with_capability_execution_mount(capability_id.clone(), self.skill_mounts.clone());
        }
        for capability_id in self.policy.memory_capability_ids() {
            factory = factory
                .with_capability_execution_mount(capability_id.clone(), self.memory_mounts.clone());
        }
        for capability_id in self.policy.system_extensions_lifecycle_capability_ids() {
            factory = factory.with_capability_execution_mount(
                capability_id.clone(),
                self.system_extensions_lifecycle_mounts.clone(),
            );
        }
        // Test-support-only overrides (empty in production, see the config
        // field doc-comment): the factory bakes mounts in at construction, so
        // this is the only seam that can reach per-capability test mounts.
        for (capability_id, mounts) in &self.capability_execution_mount_overrides {
            factory =
                factory.with_capability_execution_mount(capability_id.clone(), mounts.clone());
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
        synthetic_capabilities.push(result_read_capability(
            Arc::clone(&self.thread_service),
            self.fallback_user_id.clone(),
        )?);
        if let Some(outbound_preferences_facade) = &self.outbound_preferences_facade {
            synthetic_capabilities.extend(outbound_delivery_capabilities(
                Arc::clone(outbound_preferences_facade),
                self.fallback_user_id.clone(),
                Arc::clone(&self.approval_requests),
                Arc::clone(&self.capability_leases),
                self.outbound_delivery_target_set_requires_approval,
                Arc::clone(&self.approval_settings),
                Arc::clone(&self.replay_payload_store),
                Arc::clone(&self.gate_record_store),
            )?);
        }
        let port = wrap_synthetic_capabilities(
            port,
            synthetic_capabilities,
            self.run_context.clone(),
            self.fallback_user_id.clone(),
            Arc::clone(&self.input_resolver),
            Arc::clone(&self.result_writer),
            // Synthetic capabilities bypass the inner port's input hook, so the
            // wrapper needs the observer to emit `on_capability_input` itself.
            self.trajectory_observer.clone(),
            Arc::clone(&self.replay_payload_store),
        )?;
        let port = wrap_surface_disclosure(port, &self.workspace_mounts);
        // Outermost: external (client-supplied) tools see the full resolved
        // surface (for shadow-rejection) and park instead of executing.
        Ok(wrap_external_tools(
            port,
            self.run_context.clone(),
            Arc::clone(&self.input_resolver),
            Arc::clone(&self.result_writer),
            Arc::clone(&self.external_tool_catalog),
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
impl LoopCapabilityPort for RefreshingCapabilityPort {
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
        request: RegisterProviderToolCallRequest,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        self.current_or_refresh()
            .await?
            .register_provider_tool_call(request)
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
        request: LoopRequest,
    ) -> Result<Resolution, AgentLoopHostError> {
        self.current_or_refresh()
            .await?
            .invoke_capability(request)
            .await
    }

    async fn invoke_capability_batch(
        &self,
        request: LoopRequestBatch,
    ) -> Result<ResolutionBatch, AgentLoopHostError> {
        self.current_or_refresh()
            .await?
            .invoke_capability_batch(request)
            .await
    }
}

/// Test-support constructor (harness-port-seam P1 seam): assembles a
/// [`RefreshingCapabilityPortConfig`] from the harness's injectable
/// parts and drives the REAL [`create_refreshing_capability_port`]
/// above, so the harness exercises every wrap layer `build_inner` applies.
/// Parts the harness has no opinion on get the same no-op production types
/// the sole call site (`local_dev.rs`'s `create_capability_port`) passes -- never `capability_wiring`'s
/// `RebornServices`-entangled defaults. For tests only -- gated behind
/// `test-support`, ships zero bytes in production builds.
#[cfg(feature = "test-support")]
pub(crate) async fn create_refreshing_capability_port_for_test(
    parts: crate::test_support::RefreshingCapabilityPortTestParts,
) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
    let crate::test_support::RefreshingCapabilityPortTestParts {
        runtime,
        run_context,
        fallback_user_id,
        workspace_mounts,
        skill_mounts,
        memory_mounts,
        system_extensions_lifecycle_mounts,
        input_resolver,
        result_writer,
        milestone_sink,
        skill_activation_source,
        project_service,
        thread_service,
        trajectory_observer,
        outbound_preferences_facade,
        outbound_delivery_target_set_requires_approval,
        tool_permission_overrides,
        auto_approve_settings,
        persistent_approval_policies,
        approval_requests,
        capability_leases,
        gate_record_store,
        replay_payload_store,
        capability_execution_mount_overrides,
        additional_provider_trust,
        capability_id_filter,
        extension_management,
        additional_capability_grants,
    } = parts;

    let policy = Arc::new(
        crate::builtin_capability_policy::builtin_capability_policy()
            .map_err(host_api_agent_loop_error)?,
    );
    let approval_settings: Arc<dyn ApprovalSettingsProvider> = Arc::new(
        crate::local_dev_authorization::StoreApprovalSettingsProvider::new(
            tool_permission_overrides,
            auto_approve_settings,
            persistent_approval_policies,
        ),
    );
    // Recover the crate-private `ComposedSelectableSkillContextSource` from
    // the opaque `SkillActivationTestSource` handle the harness passed in
    // (see the field's doc-comment on `RefreshingCapabilityPortTestParts`).
    let skill_activation_source = skill_activation_source.map(|handle| handle.activation_source());
    let extension_readiness_source = extension_management.map(|handle| handle.readiness_source());

    create_refreshing_capability_port(RefreshingCapabilityPortConfig {
        runtime,
        run_context,
        fallback_user_id,
        policy,
        workspace_mounts,
        skill_mounts,
        memory_mounts,
        system_extensions_lifecycle_mounts,
        // Harness-port-seam P1 Change 3: same constructor production's
        // `capability_wiring` calls (`runtime/local_dev.rs:132-133`), fed the
        // harness's `extension_management` handle recovered from the opaque
        // `ExtensionManagementTestHandle` (see its doc-comment); `None` when
        // the harness never wired one, reproducing the prior always-no-op
        // surface.
        extension_surface_source: ExtensionCapabilitySurfaceSource::new(extension_readiness_source),
        input_resolver,
        result_writer,
        milestone_sink,
        skill_activation_source,
        project_service,
        thread_service,
        trajectory_observer,
        outbound_preferences_facade,
        outbound_delivery_target_set_requires_approval,
        approval_settings,
        approval_requests,
        capability_leases,
        gate_record_store,
        replay_payload_store,
        external_tool_catalog: Arc::new(ironclaw_turns::InMemoryExternalToolCatalog::new()),
        capability_execution_mount_overrides,
        additional_provider_trust,
        capability_id_filter,
        additional_capability_grants,
    })
    .await
}
