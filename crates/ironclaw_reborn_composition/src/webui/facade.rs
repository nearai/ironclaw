// arch-exempt: large_file, WebUI bundle composition awaiting Reborn composition helper extraction, plan #4471
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::Utc;

use async_trait::async_trait;
#[cfg(test)]
use ironclaw_extensions::SharedExtensionRegistry;
use ironclaw_host_api::{
    InvocationId, ProductSurface, ProductSurfaceCaller, ProductSurfaceError,
    ProductSurfaceErrorCode, ProductSurfaceErrorKind, ResourceScope,
};
use ironclaw_product::ProjectionStream;
use ironclaw_product::{
    ChannelConnectionFacade, OperatorStatusService, RebornOperatorStatusCheck,
    RebornOperatorStatusResponse, RebornOperatorStatusSeverity, RebornOperatorStatusState,
    RebornServices as ProductRebornServices, RebornSkillContentResponse, RebornSkillInfo,
    RebornSkillListResponse, RebornSkillSearchResponse, RebornSkillSourceKind,
    RebornSkillTrustLevel, SkillsProductFacade,
};

use ironclaw_triggers::TriggerRepository;

use crate::extension_host::admin_configuration::AdminConfigurationViewProvider;
use crate::operator_tool_catalog::ActiveRegistryOperatorToolCatalog;
use crate::webui::product_capability::RuntimeProductCapabilityInvoker;
use crate::{
    RebornAutomationProductFacade, RebornBuildError, RebornProductAuthServices, RebornReadiness,
    RebornReadinessDiagnostic, RebornReadinessDiagnosticStatus, RebornRuntime,
    extension_host::lifecycle::{LifecycleFacade, SkillManagementPort, SkillManagementPortError},
    extension_host::webui_extension_credentials::ProductAuthExtensionCredentialSetup,
    observability::OperatorServiceLifecycle,
    outbound::{
        OutboundDeliveryTargetProvider, OutboundDeliveryTargetRegistry,
        RebornOutboundPreferencesFacade, outbound_delivery_synthetic_provider,
        outbound_delivery_target_set_operator_tool_info,
    },
    support::fs::{
        MountScopedFilesystemReader, ProjectScopedAttachmentLander, ProjectScopedAttachmentReader,
        ProjectScopedFilesystemReader,
    },
};

/// WebUI-facing Reborn service bundle for host composition.
///
/// This bundle deliberately exposes facade-shaped product handles consumed
/// by WebChat v2 and the optional product-auth OAuth routes. HTTP routing, auth
/// middleware, static assets, and SSE transport live in the `ironclaw_webui`
/// crate (which folded up the former `ironclaw_webui_v2` route surface); only
/// the host-supplied route-mount vocabulary stays in the
/// [`crate::webui::route_mounts`] module here. Lower runtime handles stay behind
/// the existing Reborn runtime / composition services.
#[derive(Clone)]
pub struct RebornWebuiBundle {
    pub product_surface: Arc<dyn ProductSurface>,
    pub product_auth: Option<Arc<RebornProductAuthServices>>,
    pub readiness: RebornReadiness,
}

impl std::fmt::Debug for RebornWebuiBundle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornWebuiBundle")
            .field("product_surface", &"Arc<dyn ProductSurface>")
            .field("product_auth", &self.product_auth.is_some())
            .field("readiness", &self.readiness)
            .finish()
    }
}

/// A trigger repository paired with the turn-run snapshot source from the
/// SAME runtime. Local-dev and production graphs both carry these two
/// separately; mixing runtimes would let active-hold projections read run
/// state the poller of the *other* runtime writes, silently desyncing the
/// automations panel (#5886).
pub(crate) struct AutomationBacking {
    pub(crate) repository: Arc<dyn TriggerRepository>,
    pub(crate) snapshot_source: Arc<dyn crate::turn_run_snapshot::TurnRunSnapshotSource>,
}

/// Resolves the [`AutomationBacking`] pair from the runtime-owned stores.
pub(crate) fn automation_backing(runtime: &RebornRuntime) -> AutomationBacking {
    AutomationBacking {
        repository: Arc::clone(&runtime.trigger_repository),
        snapshot_source: Arc::clone(&runtime.turn_run_snapshot_source),
    }
}

/// Compose the WebUI-facing product facade from an already-built Reborn runtime.
///
/// This function does not create a second turn coordinator, thread service,
/// host runtime or route server. It reuses the runtime's existing task-level
/// composition and attaches the runtime-owned projection stream unless the
/// caller supplies a custom stream.
pub fn build_webui_services(
    runtime: &RebornRuntime,
    event_stream: Option<Arc<dyn ProjectionStream>>,
) -> Result<RebornWebuiBundle, RebornBuildError> {
    // The generic per-user channel-connection facade (extension-runtime
    // §6.3): channel extensions are discovered from the durable installation
    // store; no per-vendor lane registers anything.
    let channel_connection = runtime.generic_channel_connection_facade();
    build_webui_services_with_channel_connection(
        runtime,
        event_stream,
        channel_connection,
        Vec::new(),
    )
}

pub(crate) fn build_webui_services_with_channel_connection(
    runtime: &RebornRuntime,
    event_stream: Option<Arc<dyn ProjectionStream>>,
    channel_connection: Option<Arc<dyn ChannelConnectionFacade>>,
    mut outbound_delivery_target_providers: Vec<Arc<dyn OutboundDeliveryTargetProvider>>,
) -> Result<RebornWebuiBundle, RebornBuildError> {
    if let Some(provider) = runtime.outbound_delivery_target_provider() {
        outbound_delivery_target_providers.push(provider);
    }

    let admin_configuration_view = AdminConfigurationViewProvider::new(
        runtime.admin_configuration.clone(),
        runtime.admin_configuration_uses.as_ref().clone(),
        runtime.extension_management.installation_store_handle(),
    );
    let mut api = ProductRebornServices::new_with_product_ports(
        runtime.product_thread_service(),
        runtime.product_turn_coordinator(),
        RuntimeProductCapabilityInvoker::from_runtime(runtime),
        admin_configuration_view,
    )
    .with_approval_interactions(runtime.webui_approval_interaction_service())
    .with_auth_interactions(runtime.webui_auth_interaction_service());
    // Admin user-management surface: the directory and secret provisioner are
    // core runtime handles; only token minting is deployment-supplied.
    if let Some(minter) = runtime.reborn_admin_token_minter() {
        api = api.with_admin_user_service(Arc::new(
            crate::admin_user_directory::RebornAdminUserDirectory::new(
                runtime.reborn_user_directory(),
                runtime.reborn_admin_secret_provisioner(),
                minter,
            ),
        ));
    }
    if let Some(workspace_filesystem) = runtime.webui_workspace_filesystem() {
        api = api
            .with_inbound_attachments(Arc::new(ProjectScopedAttachmentLander::new(Arc::clone(
                &workspace_filesystem,
            ))))
            // Read-only project filesystem backing directory listing and file
            // download chips, over the same workspace mount.
            .with_project_filesystem_reader(Arc::new(ProjectScopedFilesystemReader::new(
                Arc::clone(&workspace_filesystem),
            )))
            // Read counterpart: serves landed attachment bytes back to the
            // browser (image thumbnails) through the same workspace mount.
            .with_inbound_attachment_reader(Arc::new(ProjectScopedAttachmentReader::new(
                workspace_filesystem,
            )));
    }
    // Standalone read-only filesystem viewer: browses memory + workspace over a
    // dedicated read-only multi-mount view (not the read-write workspace handle
    // above), so navigation can never become a write path.
    if let Some(browse_filesystem) = runtime.webui_browse_filesystem() {
        api = api.with_filesystem_browser(Arc::new(MountScopedFilesystemReader::new(
            browse_filesystem,
        )));
    }
    if let Some(skill_activation_source) = runtime.webui_skill_activation_source() {
        let activation_recorder = Arc::clone(&skill_activation_source);
        let activation_clearer = skill_activation_source;
        api = api.with_skill_activation_hooks(
            move |scope, accepted_message_ref, message| {
                activation_recorder
                    .record_user_message(scope.clone(), accepted_message_ref.clone(), message)
                    .map_err(|_| ProductSurfaceError {
                        code: ProductSurfaceErrorCode::Internal,
                        kind: ProductSurfaceErrorKind::Internal,
                        status_code: 500,
                        retryable: false,
                        field: None,
                        validation_code: None,
                    })
            },
            move |scope, accepted_message_ref| {
                activation_clearer
                    .clear_accepted_message(scope, accepted_message_ref)
                    .map_err(|_| ProductSurfaceError {
                        code: ProductSurfaceErrorCode::Internal,
                        kind: ProductSurfaceErrorKind::Internal,
                        status_code: 500,
                        retryable: false,
                        field: None,
                        validation_code: None,
                    })
            },
        );
    }
    {
        let tool_permission_overrides = &runtime.tool_permission_overrides;
        let auto_approve_settings = &runtime.auto_approve_settings;
        let persistent_approval_policies = &runtime.persistent_approval_policies;
        let tool_permission_overrides: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStore> =
            tool_permission_overrides.clone();
        let auto_approve_settings: Arc<dyn ironclaw_approvals::AutoApproveSettingStore> =
            auto_approve_settings.clone();
        let persistent_approval_policies: Arc<
            dyn ironclaw_approvals::PersistentApprovalPolicyStore,
        > = persistent_approval_policies.clone();
        let tool_registry = runtime.shared_extension_registry.clone();
        let synthetic_operator_tools = if outbound_delivery_target_providers.is_empty() {
            Vec::new()
        } else {
            let provider = outbound_delivery_synthetic_provider().map_err(|error| {
                RebornBuildError::InvalidConfig {
                    reason: format!("outbound delivery synthetic provider id is invalid: {error}"),
                }
            })?;
            vec![
                outbound_delivery_target_set_operator_tool_info(provider).map_err(|error| {
                    RebornBuildError::InvalidConfig {
                        reason: format!("outbound delivery operator tool is invalid: {error}"),
                    }
                })?,
            ]
        };
        api = api.with_operator_approval_config(
            tool_permission_overrides,
            auto_approve_settings,
            persistent_approval_policies,
            Arc::new(ActiveRegistryOperatorToolCatalog::new(
                tool_registry,
                synthetic_operator_tools,
                Some(runtime.extension_management.clone()),
            )),
        );
        let mut lifecycle_facade = LifecycleFacade::new(Arc::clone(&runtime.skill_management));
        lifecycle_facade =
            lifecycle_facade.with_extension_management(runtime.extension_management.clone());
        lifecycle_facade = lifecycle_facade
            .with_admin_configuration_resolver(runtime.admin_configuration_resolver.clone());
        if let Some(runtime_http_egress) = &runtime.runtime_http_egress {
            lifecycle_facade =
                lifecycle_facade.with_runtime_http_egress(runtime_http_egress.clone());
        }
        lifecycle_facade = lifecycle_facade.with_runtime_credential_accounts(
            runtime
                .product_auth
                .runtime_credential_account_selection_service(),
        );
        api = api.with_lifecycle_product_facade(Arc::new(lifecycle_facade));
    }
    // The manifest-declared administrator-configuration surface is rendered and
    // routed through `admin_configuration_view` (built above from the canonical
    // Admin Configuration service); no separate channel-config facade port.
    // Share the activation selector's live master switch when the selected skill
    // context reads it. Deployments without that selector pass `None`, so the
    // toggle reports unavailable rather than writing to an orphan flag.
    let auto_activate_flag = Some(runtime.skill_auto_activate_learned.clone());
    api = api.with_skills_product_facade(Arc::new(LocalSkillsProductFacade::new(
        Arc::clone(&runtime.skill_management),
        auto_activate_flag,
    )));
    api = api.with_extension_credentials(Arc::new(ProductAuthExtensionCredentialSetup::new(
        Arc::clone(&runtime.product_auth),
    )));
    let backing = automation_backing(runtime);
    let active_run_lookup: Arc<dyn ironclaw_triggers::TriggerActiveRunLookup> = Arc::new(
        crate::automation::trigger_poller::SnapshotActiveRunLookup::new(backing.snapshot_source),
    );
    api = api.with_automation_product_facade(Arc::new(
        RebornAutomationProductFacade::new(backing.repository, active_run_lookup)
            .with_scheduler_enabled(runtime.readiness.workers.trigger_poller),
    ));
    // First-class projects + membership (ACL). Built once per runtime over the
    // scoped substrate and shared by every deployment path.
    api = api.with_project_service(runtime.reborn_project_service());
    api = api.with_outbound_preferences_facade(Arc::new(RebornOutboundPreferencesFacade::new(
        Arc::clone(&runtime.outbound_preferences),
        Arc::new(OutboundDeliveryTargetRegistry::new(
            outbound_delivery_target_providers,
        )),
    )));
    if let Some(channel_connection) = channel_connection {
        api = api.with_channel_connection_facade(channel_connection);
    }
    api = api.with_event_stream(event_stream.unwrap_or_else(|| runtime.product_event_stream()));
    api = api.with_operator_status_service(Arc::new(ReadinessOperatorStatusService::new(
        runtime.readiness.clone(),
    )));
    api = api.with_operator_logs_service(crate::operator_log_buffer());
    {
        let webui_boot_config = runtime.webui_boot_config();
        api = api.with_operator_service_lifecycle_service(Arc::new(
            OperatorServiceLifecycle::new_for_operator_with_boot_config(
                runtime.webui_tenant_id().clone(),
                runtime.owner_user_id.clone(),
                webui_boot_config,
            ),
        ));
    }

    // Compose the operator LLM-config settings service when the runtime was
    // assembled with a boot config. The secret store stays private to this
    // crate; the service is the only facade-shaped handle that leaves.
    if let Some(llm_config) = build_llm_config_service(runtime) {
        api = api.with_llm_config_service(llm_config);
    }

    // Wire the live active-model reader so a default-model run (no explicit
    // `model`, hence no `resolved_model_route`) is still priced — against the
    // model that actually ran, tracking operator model swaps.
    if let Some(active_model_reader) = runtime.webui_active_model_reader() {
        api = api.with_active_model_reader(active_model_reader);
    }

    Ok(RebornWebuiBundle {
        product_surface: Arc::new(api),
        product_auth: Some(Arc::clone(&runtime.product_auth)),
        readiness: runtime.readiness.clone(),
    })
}

/// Compose the operator LLM-config settings service from the runtime's boot
/// config, secret store, and optional reload/session/login-state handles.
///
/// Returns `None` when the runtime was assembled without a boot config. Shared
/// by `build_webui_services` (operator LLM routes) and the OpenAI-compatible
/// `/v1/models` catalog so both read the same configured-model source.
pub(crate) fn build_llm_config_service(
    runtime: &RebornRuntime,
) -> Option<Arc<dyn ironclaw_product::LlmConfigService>> {
    let boot = runtime.webui_boot_config()?;
    let keys = crate::LlmKeyStore::new(runtime.secret_store());
    let mut llm_config = crate::RebornLlmConfigService::new(boot.clone(), keys);
    if let Some(reload) = runtime.webui_llm_reload_trigger() {
        llm_config = llm_config.with_reload_trigger(reload);
    }
    if let Some(session) = runtime.webui_llm_session() {
        llm_config = llm_config.with_nearai_session(session);
    }
    if let Some(states) = runtime.webui_nearai_login_states() {
        llm_config = llm_config.with_nearai_login_states(states);
    }
    Some(Arc::new(llm_config))
}

struct ReadinessOperatorStatusService {
    readiness: RebornReadiness,
}

impl ReadinessOperatorStatusService {
    fn new(readiness: RebornReadiness) -> Self {
        Self { readiness }
    }
}

#[async_trait]
impl OperatorStatusService for ReadinessOperatorStatusService {
    async fn status(
        &self,
        _caller: ProductSurfaceCaller,
    ) -> Result<RebornOperatorStatusResponse, ProductSurfaceError> {
        Ok(status_response_from_readiness(&self.readiness))
    }
}

struct LocalSkillsProductFacade {
    skill_management: Arc<SkillManagementPort>,
    // `RebornRuntimeStores::skill_auto_activate_learned`); the read facade
    // reports it for the skills view. Writes go through the first-party
    // `builtin.skill_auto_activate_learned_set` capability. `None` when no
    // flag-reading selector is wired (the production assembly) — the toggle then
    // reports unavailable instead of writing to a flag nothing reads.
    //
    // Process-global by design: this is a single-operator local-dev switch, so it
    // is intentionally not scoped per caller. A future multi-user surface would
    // need a per-tenant flag.
    auto_activate_learned: Option<Arc<AtomicBool>>,
}

impl LocalSkillsProductFacade {
    fn new(
        skill_management: Arc<SkillManagementPort>,
        auto_activate_learned: Option<Arc<AtomicBool>>,
    ) -> Self {
        Self {
            skill_management,
            auto_activate_learned,
        }
    }
}

#[async_trait]
impl SkillsProductFacade for LocalSkillsProductFacade {
    async fn list_skills(
        &self,
        caller: ProductSurfaceCaller,
    ) -> Result<RebornSkillListResponse, ProductSurfaceError> {
        let scope = caller_skill_scope(caller);
        let skills = self
            .skill_management
            .list_for_scope(scope)
            .await
            .map_err(map_skill_management_error)?;
        Ok(skill_list_response(
            skills,
            self.auto_activate_learned
                .as_ref()
                .map(|flag| flag.load(Ordering::Relaxed))
                .unwrap_or(true),
        ))
    }

    async fn search_skills(
        &self,
        caller: ProductSurfaceCaller,
        query: String,
    ) -> Result<RebornSkillSearchResponse, ProductSurfaceError> {
        let scope = caller_skill_scope(caller);
        let result = self
            .skill_management
            .search_for_scope(scope, &query, 50)
            .await
            .map_err(map_skill_management_error)?;
        Ok(RebornSkillSearchResponse {
            catalog: Vec::new(),
            installed: result.skills.into_iter().map(skill_info).collect(),
            registry_url: String::new(),
            catalog_error: None,
        })
    }

    async fn read_skill_content(
        &self,
        caller: ProductSurfaceCaller,
        name: String,
    ) -> Result<RebornSkillContentResponse, ProductSurfaceError> {
        let scope = caller_skill_scope(caller);
        let content = self
            .skill_management
            .read_content_for_scope(scope, &name)
            .await
            .map_err(map_skill_management_error)?;
        Ok(RebornSkillContentResponse {
            name: content.name,
            content: content.content,
        })
    }
}

fn caller_skill_scope(caller: ProductSurfaceCaller) -> ResourceScope {
    ResourceScope {
        tenant_id: caller.tenant_id,
        user_id: caller.user_id,
        agent_id: caller.agent_id,
        project_id: caller.project_id,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn skill_list_response(
    skills: Vec<ironclaw_skills::SkillSummary>,
    auto_activate_learned: bool,
) -> RebornSkillListResponse {
    let skills: Vec<_> = skills.into_iter().map(skill_info).collect();
    RebornSkillListResponse {
        count: skills.len(),
        skills,
        auto_activate_learned,
    }
}

fn skill_info(skill: ironclaw_skills::SkillSummary) -> RebornSkillInfo {
    let source_kind = match skill.source {
        ironclaw_skills::ManagedSkillSource::System => RebornSkillSourceKind::System,
        ironclaw_skills::ManagedSkillSource::User => RebornSkillSourceKind::User,
        ironclaw_skills::ManagedSkillSource::Installed => RebornSkillSourceKind::Installed,
    };
    let can_manage = matches!(
        source_kind,
        RebornSkillSourceKind::User | RebornSkillSourceKind::Installed
    );
    RebornSkillInfo {
        name: skill.name.clone(),
        description: skill.description,
        version: skill.version,
        trust: if source_kind == RebornSkillSourceKind::Installed {
            RebornSkillTrustLevel::Installed
        } else {
            RebornSkillTrustLevel::Trusted
        },
        source: source_kind,
        source_kind,
        keywords: skill.keywords,
        usage_hint: Some(format!(
            "Type `/{}` in chat to force-activate this skill.",
            skill.name
        )),
        setup_hint: None,
        bundle_path: None,
        install_source_url: None,
        has_requirements: false,
        has_scripts: false,
        can_edit: can_manage,
        can_delete: can_manage,
        auto_activate: skill.auto_activate,
    }
}

fn map_skill_management_error(error: SkillManagementPortError) -> ProductSurfaceError {
    match error {
        SkillManagementPortError::InvalidContext { .. } => internal_skill_error(),
        SkillManagementPortError::Skill(error) => match error.kind() {
            ironclaw_skills::SkillManagementErrorKind::NotFound => ProductSurfaceError {
                code: ProductSurfaceErrorCode::NotFound,
                kind: ProductSurfaceErrorKind::NotFound,
                status_code: 404,
                retryable: false,
                field: None,
                validation_code: None,
            },
            ironclaw_skills::SkillManagementErrorKind::Conflict => ProductSurfaceError {
                code: ProductSurfaceErrorCode::Conflict,
                kind: ProductSurfaceErrorKind::Conflict,
                status_code: 409,
                retryable: false,
                field: None,
                validation_code: None,
            },
            ironclaw_skills::SkillManagementErrorKind::Resource => ProductSurfaceError {
                code: ProductSurfaceErrorCode::Unavailable,
                kind: ProductSurfaceErrorKind::ServiceUnavailable,
                status_code: 503,
                retryable: true,
                field: None,
                validation_code: None,
            },
            ironclaw_skills::SkillManagementErrorKind::FilesystemDenied => ProductSurfaceError {
                code: ProductSurfaceErrorCode::Forbidden,
                kind: ProductSurfaceErrorKind::ParticipantDenied,
                status_code: 403,
                retryable: false,
                field: None,
                validation_code: None,
            },
            ironclaw_skills::SkillManagementErrorKind::InvalidInput
            | ironclaw_skills::SkillManagementErrorKind::InvalidSkill => invalid_skill_request(),
        },
    }
}

fn invalid_skill_request() -> ProductSurfaceError {
    ProductSurfaceError {
        code: ProductSurfaceErrorCode::InvalidRequest,
        kind: ProductSurfaceErrorKind::Validation,
        status_code: 400,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

fn internal_skill_error() -> ProductSurfaceError {
    ProductSurfaceError {
        code: ProductSurfaceErrorCode::Internal,
        kind: ProductSurfaceErrorKind::Internal,
        status_code: 500,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

fn status_response_from_readiness(readiness: &RebornReadiness) -> RebornOperatorStatusResponse {
    let mut checks = Vec::new();
    let (runtime_status, runtime_severity, runtime_remediation) = match readiness.state {
        crate::RebornReadinessState::Disabled => (
            RebornOperatorStatusState::NotConfigured,
            RebornOperatorStatusSeverity::Warning,
            Some("finish Reborn runtime setup before production use".to_string()),
        ),
        crate::RebornReadinessState::DevOnly => (
            RebornOperatorStatusState::Degraded,
            RebornOperatorStatusSeverity::Warning,
            Some("finish Reborn runtime setup before production use".to_string()),
        ),
        crate::RebornReadinessState::HostedSingleTenantValidated => (
            RebornOperatorStatusState::Ready,
            RebornOperatorStatusSeverity::Info,
            None,
        ),
        crate::RebornReadinessState::HostedSingleTenantVolumePreviewValidated => (
            RebornOperatorStatusState::Degraded,
            RebornOperatorStatusSeverity::Warning,
            Some("mounted-volume hosted preview is ready for single-tenant validation but is not production storage".to_string()),
        ),
        crate::RebornReadinessState::ProductionValidated => (
            RebornOperatorStatusState::Ready,
            RebornOperatorStatusSeverity::Info,
            None,
        ),
        crate::RebornReadinessState::MigrationDryRunValidated => (
            RebornOperatorStatusState::Ready,
            RebornOperatorStatusSeverity::Info,
            None,
        ),
    };
    checks.push(status_check(
        "runtime",
        runtime_status,
        runtime_severity,
        format!(
            "Reborn profile {:?} is {:?}",
            readiness.profile, readiness.state
        ),
        runtime_remediation,
    ));
    checks.push(bool_check(
        "storage",
        readiness.facades.turn_coordinator,
        "turn coordinator facade is ready",
        "turn coordinator facade is not wired",
    ));
    checks.push(bool_check(
        "secrets",
        readiness.facades.product_auth,
        "product auth and secret-backed flows are ready",
        "product auth facade is not wired",
    ));
    checks.push(bool_check(
        "provider_model",
        readiness.facades.host_runtime,
        "host runtime is ready for model-backed execution",
        "host runtime is not wired",
    ));
    checks.push(status_check(
        "webui",
        RebornOperatorStatusState::Ready,
        RebornOperatorStatusSeverity::Info,
        "WebUI v2 route facade is mounted".to_string(),
        None,
    ));
    checks.push(bool_check(
        "trigger_poller",
        readiness.workers.trigger_poller,
        "trigger poller worker is ready",
        "trigger poller worker is not running",
    ));
    checks.push(status_check(
        "channels",
        RebornOperatorStatusState::Unsupported,
        RebornOperatorStatusSeverity::Info,
        "channel-specific readiness probes are not wired yet".to_string(),
        Some("consult channel setup diagnostics for adapter-specific status".to_string()),
    ));
    checks.push(status_check(
        "extensions",
        RebornOperatorStatusState::Unsupported,
        RebornOperatorStatusSeverity::Info,
        "extension readiness probes are not wired yet".to_string(),
        Some("use extension inventory and setup endpoints for per-extension status".to_string()),
    ));
    checks.extend(
        readiness
            .diagnostics
            .iter()
            .map(status_check_from_readiness_diagnostic),
    );
    let overall = if checks
        .iter()
        .any(|check| check.status == RebornOperatorStatusState::Blocked)
    {
        RebornOperatorStatusState::Blocked
    } else if checks.iter().any(|check| {
        matches!(
            check.status,
            RebornOperatorStatusState::Degraded | RebornOperatorStatusState::NotConfigured
        )
    }) {
        RebornOperatorStatusState::Degraded
    } else {
        RebornOperatorStatusState::Ready
    };
    RebornOperatorStatusResponse {
        generated_at: Utc::now(),
        overall,
        checks,
    }
}

fn bool_check(
    id: &str,
    ready: bool,
    ready_summary: &str,
    missing_summary: &str,
) -> RebornOperatorStatusCheck {
    status_check(
        id,
        if ready {
            RebornOperatorStatusState::Ready
        } else {
            RebornOperatorStatusState::NotConfigured
        },
        if ready {
            RebornOperatorStatusSeverity::Info
        } else {
            RebornOperatorStatusSeverity::Warning
        },
        if ready {
            ready_summary
        } else {
            missing_summary
        }
        .to_string(),
        (!ready).then(|| format!("wire the {id} subsystem in Reborn composition")),
    )
}

fn status_check_from_readiness_diagnostic(
    diagnostic: &RebornReadinessDiagnostic,
) -> RebornOperatorStatusCheck {
    let component = readiness_diagnostic_component(diagnostic);
    let reason = readiness_diagnostic_reason(diagnostic);
    let id = format!("readiness_{component}");
    let status = match diagnostic.status {
        RebornReadinessDiagnosticStatus::Blocking => RebornOperatorStatusState::Blocked,
        RebornReadinessDiagnosticStatus::Warning | RebornReadinessDiagnosticStatus::Unknown(_) => {
            RebornOperatorStatusState::Degraded
        }
        RebornReadinessDiagnosticStatus::Info => RebornOperatorStatusState::Ready,
    };
    let severity = match diagnostic.status {
        RebornReadinessDiagnosticStatus::Blocking => RebornOperatorStatusSeverity::Critical,
        RebornReadinessDiagnosticStatus::Warning | RebornReadinessDiagnosticStatus::Unknown(_) => {
            RebornOperatorStatusSeverity::Warning
        }
        RebornReadinessDiagnosticStatus::Info => RebornOperatorStatusSeverity::Info,
    };
    let remediation = if diagnostic.blocks_production {
        "wire the required Reborn production component before exposing live traffic"
    } else {
        "review the Reborn readiness report for the component owner"
    };
    status_check(
        &id,
        status,
        severity,
        format!(
            "readiness diagnostic: component={component}, reason={reason}, profile={:?}",
            diagnostic.profile
        ),
        Some(remediation.to_string()),
    )
}

fn readiness_diagnostic_component(diagnostic: &RebornReadinessDiagnostic) -> String {
    readiness_diagnostic_wire_string(&diagnostic.component)
        .unwrap_or_else(|| "unknown_component".to_string())
}

fn readiness_diagnostic_reason(diagnostic: &RebornReadinessDiagnostic) -> String {
    readiness_diagnostic_wire_string(&diagnostic.reason)
        .unwrap_or_else(|| "unknown_reason".to_string())
}

fn readiness_diagnostic_wire_string(value: &impl serde::Serialize) -> Option<String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
}

fn status_check(
    id: &str,
    status: RebornOperatorStatusState,
    severity: RebornOperatorStatusSeverity,
    summary: String,
    remediation: Option<String>,
) -> RebornOperatorStatusCheck {
    RebornOperatorStatusCheck {
        id: id.to_string(),
        status,
        severity,
        summary,
        remediation,
    }
}

#[cfg(test)]
mod tests;
