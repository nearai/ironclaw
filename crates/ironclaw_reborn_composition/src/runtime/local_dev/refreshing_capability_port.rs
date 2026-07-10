use std::sync::{Arc, Mutex as StdMutex};

use ironclaw_authorization::CapabilityLeaseStore;
use ironclaw_host_api::{MountView, UserId};
use ironclaw_host_runtime::HostRuntime;
use ironclaw_loop_support::{
    HostRuntimeLoopCapabilityPortFactory, LoopCapabilityInputResolver, LoopCapabilityResultWriter,
};
use ironclaw_product_workflow::{OutboundPreferencesProductFacade, ProjectService};
use ironclaw_run_state::ApprovalRequestStore;
use ironclaw_turns::ExternalToolCatalog;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityBatchInvocation, CapabilityBatchOutcome,
    CapabilityCallCandidate, CapabilityInvocation, CapabilityOutcome, LoopCapabilityPort,
    LoopHostMilestoneSink, LoopRunContext, ProviderToolCall, ProviderToolCallCapabilityIds,
    ProviderToolDefinition, RegisterProviderToolCallRequest, VisibleCapabilityRequest,
    VisibleCapabilitySurface,
};
use tokio::sync::Mutex as AsyncMutex;

use crate::local_dev_capability_policy::LocalDevCapabilityPolicy;
use crate::profile_approval_authorization::ApprovalSettingsProvider;
use crate::runtime::LocalDevSelectableSkillContextSource;
use crate::runtime::local_dev::extension_surface::LocalDevExtensionSurfaceSource;
use crate::runtime::local_dev::external_tool_capability::wrap_local_dev_external_tools;
use crate::runtime::local_dev::outbound_delivery::outbound_delivery_capabilities;
use crate::runtime::local_dev::project_create::project_create_capability;
use crate::runtime::local_dev::skill_activation::skill_activation_capability;
use crate::runtime::local_dev::surface_disclosure::wrap_local_dev_surface_disclosure;
use crate::runtime::local_dev::synthetic_capability::wrap_local_dev_synthetic_capabilities;

use super::{
    LocalDevVisibleCapabilityInputs, capability_io_error, host_api_agent_loop_error,
    local_dev_visible_capability_request,
};

pub(super) struct RefreshingLocalDevCapabilityPortConfig {
    pub(super) runtime: Arc<dyn HostRuntime>,
    pub(super) run_context: LoopRunContext,
    pub(super) fallback_user_id: UserId,
    pub(super) policy: Arc<LocalDevCapabilityPolicy>,
    pub(super) workspace_mounts: MountView,
    pub(super) skill_mounts: MountView,
    pub(super) memory_mounts: MountView,
    pub(super) system_extensions_lifecycle_mounts: MountView,
    pub(super) extension_surface_source: LocalDevExtensionSurfaceSource,
    pub(super) input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    pub(super) result_writer: Arc<dyn LoopCapabilityResultWriter>,
    pub(super) milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    pub(super) skill_activation_source: Option<Arc<LocalDevSelectableSkillContextSource>>,
    pub(super) project_service: Arc<dyn ProjectService>,
    pub(super) trajectory_observer: Option<Arc<dyn crate::RebornTrajectoryObserver>>,
    pub(super) outbound_preferences_facade: Option<Arc<dyn OutboundPreferencesProductFacade>>,
    pub(super) outbound_delivery_target_set_requires_approval: bool,
    pub(super) approval_settings: Arc<dyn ApprovalSettingsProvider>,
    pub(super) approval_requests: Arc<dyn ApprovalRequestStore>,
    pub(super) capability_leases: Arc<dyn CapabilityLeaseStore>,
    pub(super) external_tool_catalog: Arc<dyn ExternalToolCatalog>,
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
        system_extensions_lifecycle_mounts: config.system_extensions_lifecycle_mounts,
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
        approval_settings: config.approval_settings,
        approval_requests: config.approval_requests,
        capability_leases: config.capability_leases,
        external_tool_catalog: config.external_tool_catalog,
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
    system_extensions_lifecycle_mounts: MountView,
    extension_surface_source: LocalDevExtensionSurfaceSource,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    skill_activation_source: Option<Arc<LocalDevSelectableSkillContextSource>>,
    project_service: Arc<dyn ProjectService>,
    trajectory_observer: Option<Arc<dyn crate::RebornTrajectoryObserver>>,
    outbound_preferences_facade: Option<Arc<dyn OutboundPreferencesProductFacade>>,
    outbound_delivery_target_set_requires_approval: bool,
    approval_settings: Arc<dyn ApprovalSettingsProvider>,
    approval_requests: Arc<dyn ApprovalRequestStore>,
    capability_leases: Arc<dyn CapabilityLeaseStore>,
    external_tool_catalog: Arc<dyn ExternalToolCatalog>,
    current: StdMutex<Option<Arc<dyn LoopCapabilityPort>>>,
    refresh_lock: AsyncMutex<()>,
}

impl RefreshingLocalDevCapabilityPort {
    async fn build_inner(&self) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        // T3-iso: resolve the SAME acting owner `local_dev_visible_capability_request`
        // below resolves, so the extension-surface filter and the granted
        // capability set never diverge on whose registered extensions are visible.
        let scope =
            super::local_dev_resource_scope_for_run(&self.run_context, &self.fallback_user_id);
        let extension_surface = self
            .extension_surface_source
            .snapshot(&scope)
            .await
            .map_err(host_api_agent_loop_error)?;
        let visible_request = local_dev_visible_capability_request(
            &self.run_context,
            &self.fallback_user_id,
            LocalDevVisibleCapabilityInputs {
                workspace_mounts: &self.workspace_mounts,
                skill_mounts: &self.skill_mounts,
                memory_mounts: &self.memory_mounts,
                system_extensions_lifecycle_mounts: &self.system_extensions_lifecycle_mounts,
                policy: &self.policy,
                extension_surface: &extension_surface,
            },
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
                Arc::clone(&self.approval_settings),
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
        let port = wrap_local_dev_surface_disclosure(port, &self.workspace_mounts);
        // Outermost: external (client-supplied) tools see the full resolved
        // surface (for shadow-rejection) and park instead of executing.
        Ok(wrap_local_dev_external_tools(
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

#[cfg(test)]
mod tests {
    use super::super::LocalDevCapabilityIo;
    use super::*;
    use crate::extension_host::extension_lifecycle::RebornLocalExtensionManagementPort;
    use ironclaw_extensions::{
        ExtensionActivationState, ExtensionInstallation, ExtensionInstallationId,
        ExtensionInstallationStore, ExtensionManifest, ExtensionManifestRecord,
        ExtensionManifestRef, ExtensionPackage, ManifestSource, SharedExtensionRegistry,
    };
    use ironclaw_host_api::{
        AgentId, ExtensionId, HostPortCatalog, ProjectId, TenantId, ThreadId, VirtualPath,
    };
    use ironclaw_turns::{
        RunProfileResolutionRequest, RunProfileResolver, TurnActor, TurnId, TurnRunId, TurnScope,
        run_profile::{InMemoryLoopHostMilestoneSink, InMemoryRunProfileResolver},
    };

    /// Seeds an enabled owner-registered extension and one model-visible
    /// capability. T3's register verb does not exist yet, so this uses the
    /// installation store and registry directly.
    async fn seed_owner_registered_capability(
        installation_store: &Arc<dyn ExtensionInstallationStore>,
        shared_registry: &SharedExtensionRegistry,
        local_dev_storage_root: &std::path::Path,
        owner: &UserId,
        extension_id_str: &str,
    ) {
        let extension_id = ExtensionId::new(extension_id_str).expect("valid extension id");
        let manifest_record = ExtensionManifestRecord::from_toml(
            format!(
                r#"
schema_version = "reborn.extension_manifest.v2"
id = "{extension_id_str}"
name = "{extension_id_str}"
version = "0.1.0"
description = "Owner-registered MCP server (T3-iso wrapper fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://127.0.0.1:9/mcp"
"#
            ),
            ManifestSource::UserRegistered {
                tenant_id: ironclaw_host_api::TenantId::from_trusted(
                    ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID.to_string(),
                ),
                owner: owner.clone(),
            },
            &HostPortCatalog::empty(),
            None,
        )
        .expect("owner manifest record parses");
        installation_store
            .upsert_manifest(manifest_record)
            .await
            .expect("seed owner manifest");
        let installation = ExtensionInstallation::new(
            ExtensionInstallationId::new(extension_id.as_str().to_string())
                .expect("valid installation id"),
            extension_id.clone(),
            ExtensionActivationState::Enabled,
            ExtensionManifestRef::new(extension_id.clone(), None),
            Vec::new(),
            chrono::Utc::now(),
        )
        .expect("valid installation");
        installation_store
            .upsert_installation(installation)
            .await
            .expect("seed installation");

        let capability_manifest = ExtensionManifest::parse(
            &format!(
                r#"
schema_version = "reborn.extension_manifest.v2"
id = "{extension_id_str}"
name = "{extension_id_str}"
version = "0.1.0"
description = "Registered MCP capability probe fixture"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/{extension_id_str}.wasm"

[[capabilities]]
id = "{extension_id_str}.search"
description = "Registered MCP search capability (fixture)"
effects = ["network"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"
"#
            ),
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
        )
        .expect("capability manifest parses");
        let capability_package = ExtensionPackage::from_manifest(
            capability_manifest,
            VirtualPath::new(format!("/system/extensions/{extension_id_str}")).expect("valid root"),
        )
        .expect("capability package builds");
        // The descriptor schema refs are read through the real mounted
        // filesystem; empty schemas are enough because this test only checks
        // capability ids.
        let schema_dir = local_dev_storage_root
            .join("system/extensions")
            .join(extension_id_str)
            .join("schemas");
        std::fs::create_dir_all(&schema_dir).expect("create schema dir");
        std::fs::write(schema_dir.join("search.input.json"), "{}").expect("write input schema");
        std::fs::write(schema_dir.join("search.output.json"), "{}").expect("write output schema");
        shared_registry
            .upsert(capability_package)
            .expect("publish capability into registry");
    }

    async fn run_context(
        label: &str,
        explicit_owner: Option<UserId>,
        actor: Option<UserId>,
    ) -> LoopRunContext {
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("profile resolves"); // safety: test-only assertion in #[cfg(test)] module.
        let scope = TurnScope::new_with_owner(
            TenantId::new(format!("tenant-{label}")).expect("tenant id"), // safety: test-only assertion in #[cfg(test)] module.
            Some(AgentId::new(format!("agent-{label}")).expect("agent id")), // safety: test-only assertion in #[cfg(test)] module.
            Some(ProjectId::new(format!("project-{label}")).expect("project id")), // safety: test-only assertion in #[cfg(test)] module.
            ThreadId::new(format!("thread-{label}")).expect("thread id"), // safety: test-only assertion in #[cfg(test)] module.
            explicit_owner,
        );
        let context = LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved);
        match actor {
            Some(actor) => context.with_actor(TurnActor::new(actor)),
            None => context,
        }
    }

    fn visible_capability_ids(surface: &VisibleCapabilitySurface) -> Vec<String> {
        // Prefer the callable set so advertised-surface narrowing cannot mask
        // the owner-scoping invariant.
        match &surface.callable_capability_ids {
            Some(ids) => ids.iter().map(|id| id.as_str().to_string()).collect(),
            None => surface
                .descriptors
                .iter()
                .map(|descriptor| descriptor.capability_id.as_str().to_string())
                .collect(),
        }
    }

    /// Pins the caller-level invariant that `build_inner` resolves the run
    /// owner before querying user-registered capabilities.
    ///
    /// This drives `create_refreshing_local_dev_capability_port` because the
    /// helper test would still pass if the wrapper used `fallback_user_id` for
    /// every run.
    #[tokio::test]
    async fn build_inner_threads_the_resolved_run_owner_not_the_fallback() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = crate::build_reborn_services(crate::RebornBuildInput::local_dev(
            "wrapper-build-owner",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");
        let runtime = services.host_runtime.clone().expect("host runtime");
        let local_runtime = services
            .local_runtime
            .as_ref()
            .expect("local runtime substrate"); // safety: test-only assertion in #[cfg(test)] module.
        let extension_management: Arc<RebornLocalExtensionManagementPort> = local_runtime
            .extension_management
            .clone()
            .expect("extension management");
        let shared_registry = local_runtime
            .shared_extension_registry
            .clone()
            .expect("shared extension registry");
        let installation_store = extension_management.installation_store();

        let fallback_user_id = UserId::new("wrapper-owner-fallback").expect("user id");
        let owner_explicit = UserId::new("wrapper-owner-explicit").expect("user id");
        let owner_actor = UserId::new("wrapper-owner-actor").expect("user id");
        seed_owner_registered_capability(
            &installation_store,
            &shared_registry,
            &local_runtime.local_dev_storage_root,
            &fallback_user_id,
            "acme-fallback",
        )
        .await;
        seed_owner_registered_capability(
            &installation_store,
            &shared_registry,
            &local_runtime.local_dev_storage_root,
            &owner_explicit,
            "acme-explicit",
        )
        .await;
        seed_owner_registered_capability(
            &installation_store,
            &shared_registry,
            &local_runtime.local_dev_storage_root,
            &owner_actor,
            "acme-actor",
        )
        .await;

        let capability_io = Arc::new(LocalDevCapabilityIo::default());
        let input_resolver: Arc<dyn LoopCapabilityInputResolver> = capability_io.clone();
        let result_writer: Arc<dyn LoopCapabilityResultWriter> = capability_io.clone();
        let policy = Arc::new(
            crate::local_dev_capability_policy::local_dev_capability_policy()
                .expect("policy parses"),
        );

        let make_config = |run_context: LoopRunContext| RefreshingLocalDevCapabilityPortConfig {
            runtime: Arc::clone(&runtime),
            run_context,
            fallback_user_id: fallback_user_id.clone(),
            policy: Arc::clone(&policy),
            workspace_mounts: local_runtime.workspace_mounts.clone(),
            skill_mounts: local_runtime.skill_mounts.clone(),
            memory_mounts: local_runtime.memory_mounts.clone(),
            system_extensions_lifecycle_mounts: local_runtime
                .system_extensions_lifecycle_mounts
                .clone(),
            extension_surface_source: LocalDevExtensionSurfaceSource::new(Some(Arc::clone(
                &extension_management,
            ))),
            input_resolver: Arc::clone(&input_resolver),
            result_writer: Arc::clone(&result_writer),
            milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
            skill_activation_source: None,
            project_service: Arc::clone(&local_runtime.project_service),
            trajectory_observer: None,
            outbound_preferences_facade: None,
            outbound_delivery_target_set_requires_approval: false,
            approval_settings: Arc::new(
                crate::profile_approval_authorization::EmptyApprovalSettingsProvider,
            ),
            approval_requests: local_runtime.approval_requests.clone(),
            capability_leases: local_runtime.capability_leases.clone(),
            external_tool_catalog: Arc::new(ironclaw_turns::InMemoryExternalToolCatalog::new()),
        };

        let explicit_run_context =
            run_context("explicit", Some(owner_explicit.clone()), None).await;
        let explicit_port =
            create_refreshing_local_dev_capability_port(make_config(explicit_run_context))
                .await
                .expect("capability port for explicit owner");
        let explicit_surface = explicit_port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible surface for explicit owner");
        let explicit_ids = visible_capability_ids(&explicit_surface);

        let actor_run_context = run_context("actor", None, Some(owner_actor.clone())).await;
        let actor_port =
            create_refreshing_local_dev_capability_port(make_config(actor_run_context))
                .await
                .expect("capability port for actor owner");
        let actor_surface = actor_port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible surface for actor owner");
        let actor_ids = visible_capability_ids(&actor_surface);

        let fallback_run_context = run_context("fallback", None, None).await;
        let fallback_port =
            create_refreshing_local_dev_capability_port(make_config(fallback_run_context))
                .await
                .expect("capability port for fallback owner");
        let fallback_surface = fallback_port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible surface for fallback owner");
        let fallback_ids = visible_capability_ids(&fallback_surface);

        assert!(
            explicit_ids.contains(&"acme-explicit.search".to_string()),
            "explicit thread owner must see its own registered capability, got {explicit_ids:?}"
        );
        assert!(
            !explicit_ids.contains(&"acme-actor.search".to_string())
                && !explicit_ids.contains(&"acme-fallback.search".to_string()),
            "explicit thread owner must not see another owner's registered capability, got {explicit_ids:?}"
        );

        assert!(
            actor_ids.contains(&"acme-actor.search".to_string()),
            "run actor must see its own registered capability when no explicit thread owner is set, got {actor_ids:?}"
        );
        assert!(
            !actor_ids.contains(&"acme-explicit.search".to_string())
                && !actor_ids.contains(&"acme-fallback.search".to_string()),
            "run actor must not see another owner's registered capability, got {actor_ids:?}"
        );

        assert!(
            fallback_ids.contains(&"acme-fallback.search".to_string()),
            "fallback owner must see its own registered capability when neither an explicit \
             thread owner nor a run actor is set, got {fallback_ids:?}"
        );
        assert!(
            !fallback_ids.contains(&"acme-explicit.search".to_string())
                && !fallback_ids.contains(&"acme-actor.search".to_string()),
            "fallback owner must not see another owner's registered capability, got {fallback_ids:?}"
        );
    }
}
