use super::with_shared_host_runtime_wiring;
use super::*;
use ironclaw_product_contracts::lifecycle_service::LifecycleProductService;
use ironclaw_product_contracts::operator_tools::RebornOperatorToolCatalog;

pub(crate) async fn build_libsql_production_host_runtime_services<TPolicy, TWake>(
    config: crate::LibSqlProductionSubstrateConfig<TPolicy, TWake>,
) -> Result<crate::LibSqlProductionHostRuntimeServices, crate::RebornCompositionError>
where
    TPolicy: ironclaw_trust::TrustPolicy + 'static,
    TWake: ironclaw_turns::TurnRunWakeNotifier + 'static,
{
    if !config.runtime.target_matches(&config.database_path_or_url) {
        return Err(crate::RebornCompositionError::InvalidConfig {
            reason: "libSQL production runtime target provenance does not match the configured durable target".to_string(),
        });
    }
    ensure_libsql_resource_governor_authority(config.process_local_resource_governor_singleton)?;
    let filesystem = Arc::new(LibSqlRootFilesystem::from_runtime(config.runtime));
    filesystem.run_migrations().await?;
    let scoped_filesystem = crate::wrap_scoped(Arc::clone(&filesystem));
    let resource_governor = FilesystemResourceGovernor::new(scoped_filesystem);
    let event_store = ironclaw_event_store::RebornEventStoreConfig::LibsqlFilesystem {
        filesystem: Arc::clone(&filesystem),
        path_or_url: config.database_path_or_url,
    };
    build_filesystem_production_host_runtime_services(
        FilesystemProductionHostRuntimeServicesInput {
            filesystem,
            resource_governor,
            event_store: ProductionEventStoresInput::Config(event_store),
            secret_master_key: config.secret_master_key,
            trust_policy: config.trust_policy,
            runtime_policy: config.runtime_policy,
            turn_run_wake_notifier: config.turn_run_wake_notifier,
            surface_version: config.surface_version,
        },
    )
    .await
}

fn ensure_libsql_resource_governor_authority(
    process_local_singleton: bool,
) -> Result<(), crate::RebornCompositionError> {
    if process_local_singleton {
        return Ok(());
    }
    Err(crate::RebornCompositionError::InvalidConfig {
        reason: "libSQL production FilesystemResourceGovernor uses process-local tallies; configure a singleton or elected resource-governor owner before sharing one database across runtime processes".to_string(),
    })
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn ensure_libsql_resource_governor_authority_for_build(
    process_local_singleton: bool,
) -> Result<(), RebornBuildError> {
    if process_local_singleton {
        return Ok(());
    }
    Err(RebornBuildError::InvalidConfig {
        reason: "libSQL FilesystemResourceGovernor uses process-local tallies; configure a singleton or elected resource-governor owner before sharing one database across runtime processes".to_string(),
    })
}

pub(crate) async fn build_postgres_production_host_runtime_services<TPolicy, TWake>(
    config: crate::PostgresProductionSubstrateConfig<TPolicy, TWake>,
) -> Result<crate::PostgresProductionHostRuntimeServices, crate::RebornCompositionError>
where
    TPolicy: ironclaw_trust::TrustPolicy + 'static,
    TWake: ironclaw_turns::TurnRunWakeNotifier + 'static,
{
    let pool = config.pool;
    ensure_postgres_resource_governor_authority(config.process_local_resource_governor_singleton)?;
    let filesystem = Arc::new(ironclaw_filesystem::PostgresRootFilesystem::new(
        pool.clone(),
    ));
    ensure_postgres_event_store_config(&config.event_store)?;
    filesystem.run_migrations().await?;
    let resource_governor = filesystem_resource_governor(&filesystem);
    let event_store = ironclaw_event_store::build_reborn_event_stores_from_root_filesystem(
        Arc::clone(&filesystem),
    )?;
    build_filesystem_production_host_runtime_services(
        FilesystemProductionHostRuntimeServicesInput {
            filesystem,
            resource_governor,
            event_store: ProductionEventStoresInput::Prebuilt(event_store),
            secret_master_key: config.secret_master_key,
            trust_policy: config.trust_policy,
            runtime_policy: config.runtime_policy,
            turn_run_wake_notifier: config.turn_run_wake_notifier,
            surface_version: config.surface_version,
        },
    )
    .await
}

fn ensure_postgres_resource_governor_authority(
    process_local_singleton: bool,
) -> Result<(), crate::RebornCompositionError> {
    if process_local_singleton {
        return Ok(());
    }
    Err(crate::RebornCompositionError::InvalidConfig {
        reason: "Postgres production FilesystemResourceGovernor uses process-local tallies; configure a singleton or elected resource-governor owner before sharing one database across runtime processes".to_string(),
    })
}

pub(super) fn ensure_postgres_resource_governor_authority_for_build(
    process_local_singleton: bool,
) -> Result<(), RebornBuildError> {
    if process_local_singleton {
        return Ok(());
    }
    Err(RebornBuildError::InvalidConfig {
        reason: "Postgres FilesystemResourceGovernor uses process-local tallies; configure a singleton or elected resource-governor owner before sharing one database across runtime processes".to_string(),
    })
}

struct FilesystemProductionHostRuntimeServicesInput<F, TPolicy, TWake>
where
    F: RootFilesystem + 'static,
{
    filesystem: Arc<F>,
    resource_governor: FilesystemResourceGovernor<F>,
    event_store: ProductionEventStoresInput,
    secret_master_key: Option<ironclaw_secrets::SecretMaterial>,
    trust_policy: Arc<TPolicy>,
    runtime_policy: crate::RebornProductionRuntimePolicy,
    turn_run_wake_notifier: Arc<TWake>,
    surface_version: CapabilitySurfaceVersion,
}

enum ProductionEventStoresInput {
    Config(ironclaw_event_store::RebornEventStoreConfig),
    Prebuilt(ironclaw_event_store::RebornEventStores),
}

fn ensure_postgres_event_store_config(
    config: &ironclaw_event_store::RebornEventStoreConfig,
) -> Result<(), crate::RebornCompositionError> {
    match config {
        ironclaw_event_store::RebornEventStoreConfig::Postgres { .. } => Ok(()),
        ironclaw_event_store::RebornEventStoreConfig::PostgresPool { .. } => Ok(()),
        _ => Err(crate::RebornCompositionError::InvalidConfig {
            reason: "PostgreSQL production substrate requires a PostgreSQL event store".to_string(),
        }),
    }
}

async fn warm_resource_governor_with_error<F, E, J>(
    resource_governor: FilesystemResourceGovernor<F>,
    map_join_error: J,
) -> Result<FilesystemResourceGovernor<F>, E>
where
    F: RootFilesystem + 'static,
    E: From<ironclaw_resources::ResourceError>,
    J: FnOnce(tokio::task::JoinError) -> E,
{
    let resource_governor = tokio::task::spawn_blocking(move || {
        resource_governor.warm_authority()?;
        Ok::<_, ironclaw_resources::ResourceError>(resource_governor)
    })
    .await
    .map_err(map_join_error)??;
    Ok(resource_governor)
}

async fn warm_resource_governor_for_composition<F>(
    resource_governor: FilesystemResourceGovernor<F>,
) -> Result<FilesystemResourceGovernor<F>, crate::RebornCompositionError>
where
    F: RootFilesystem + 'static,
{
    warm_resource_governor_with_error(resource_governor, |error| {
        crate::RebornCompositionError::InvalidConfig {
            reason: format!("resource governor warm-up task failed: {error}"),
        }
    })
    .await
}

async fn build_filesystem_production_host_runtime_services<F, TPolicy, TWake>(
    input: FilesystemProductionHostRuntimeServicesInput<F, TPolicy, TWake>,
) -> Result<FilesystemProductionHostRuntimeServices<F>, crate::RebornCompositionError>
where
    F: RootFilesystem + 'static,
    TPolicy: ironclaw_trust::TrustPolicy + 'static,
    TWake: ironclaw_turns::TurnRunWakeNotifier + 'static,
{
    let FilesystemProductionHostRuntimeServicesInput {
        filesystem,
        resource_governor,
        event_store,
        secret_master_key,
        trust_policy,
        runtime_policy,
        turn_run_wake_notifier,
        surface_version,
    } = input;
    let scoped_filesystem = crate::wrap_scoped(Arc::clone(&filesystem));
    let process_journal_store = Arc::new(ProcessJournalStore::new(
        crate::wrap_process_journal_scoped(Arc::clone(&filesystem)),
    ));
    process_journal_store
        .migrate_legacy_journal()
        .await
        .map_err(|error| crate::RebornCompositionError::InvalidConfig {
            reason: format!("process journal startup migration failed: {error}"),
        })?;
    let processes = ProcessRuntimeSystem::from_process_journal_store(process_journal_store);
    let turn_state = Arc::new(processes.agent_turn_runtime());
    let process_services = ProcessServices::filesystem(Arc::clone(&scoped_filesystem));
    let secret_credentials = build_filesystem_secret_credential_stores(
        Arc::clone(&scoped_filesystem),
        secret_master_key,
    )
    .await?;
    let resource_governor = warm_resource_governor_for_composition(resource_governor).await?;
    let governor = Arc::new(resource_governor);
    let capability_leases = Arc::new(CapabilityLeaseStore::new(Arc::clone(&scoped_filesystem)));
    let persistent_approval_policies = Arc::new(PersistentApprovalPolicyStore::new(Arc::clone(
        &scoped_filesystem,
    )));
    let (runtime_policy, process_binding) = runtime_policy.into_parts();

    let services = with_shared_host_runtime_wiring!(
        HostRuntimeServices::new(
            Arc::new(ExtensionRegistry::new()),
            filesystem,
            governor,
            Arc::new(GrantAuthorizer::new()),
            process_services,
            surface_version,
        ),
        trust_policy = trust_policy,
        runtime_policy = runtime_policy,
        capability_leases = capability_leases,
        persistent_approval_policies = persistent_approval_policies,
        secret_store = Arc::clone(&secret_credentials.secret_store),
        credential_broker = secret_credentials.credential_broker,
        process_runtime = processes.runtime(),
        approval_filesystem = Arc::clone(&scoped_filesystem),
        turn_state = turn_state,
        run_profile_resolver = Arc::new(
            ironclaw_turn_runner::planned_driver_factory::default_planned_run_profile_resolver()?,
        ),
    )
    .with_turn_run_wake_notifier(turn_run_wake_notifier);
    let services = match event_store {
        ProductionEventStoresInput::Config(config) => {
            services
                .with_reborn_event_store_config(
                    ironclaw_event_store::RebornProfile::Production,
                    config,
                )
                .await?
        }
        ProductionEventStoresInput::Prebuilt(stores) => {
            services.with_production_reborn_event_stores(stores)
        }
    };
    let services = apply_production_runtime_process_binding(services, process_binding);
    let services = match PostEditCheckConfig::from_env() {
        Ok(Some(config)) => services.with_post_edit_check(config),
        Ok(None) => services,
        Err(error) => {
            return Err(crate::RebornCompositionError::InvalidConfig {
                reason: error.to_string(),
            });
        }
    };

    let services = services
        .try_with_host_http_egress_with_body_store(
            default_host_http_egress().map_err(|error| {
                crate::RebornCompositionError::InvalidConfig {
                    reason: error.to_string(),
                }
            })?,
            Arc::clone(&scoped_filesystem),
        )
        .map_err(crate::RebornCompositionError::from)?;

    Ok(services)
}

pub(crate) fn production_skill_management_mount_view(
    scope: &ResourceScope,
) -> Result<MountView, HostApiError> {
    MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/skills")?,
            VirtualPath::new(format!(
                "/tenants/{}/users/{}/skills",
                scope.tenant_id.as_str(),
                scope.user_id.as_str()
            ))?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/system/skills")?,
            VirtualPath::new("/system/skills")?,
            MountPermissions::read_only(),
        ),
    ])
}

pub(crate) fn production_system_extensions_lifecycle_mount_view() -> Result<MountView, HostApiError>
{
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/system/extensions")?,
        VirtualPath::new("/system/extensions")?,
        MountPermissions::read_write_list_delete(),
    )])
}

pub(super) async fn build_backend_production(
    context: RebornProductionBuildContext,
    stores: ProductionStoreBundle,
    trigger_repository: Arc<dyn TriggerRepository>,
    leader_lock: ironclaw_auth::CredentialRefreshLeaderLock,
) -> Result<RebornRuntimeStores, RebornBuildError> {
    let RebornProductionBuildContext {
        profile,
        workspace_scoped_per_caller,
        wiring_config,
        production_wiring,
        local_process_port,
        product_auth_ports,
        oauth_provider_configs,
        oauth_dcr_callback,
        owner_id,
        local_runtime_identity,
        process_concurrency_limits,
        resolved_memory,
        scheduler_wake_wiring,
        mut account_setup_descriptors,
        nearai_mcp_bootstrap_config,
        native_extension_factories,
        channel_extension_bindings,
        first_party_bundles,
        first_party_registrars,
        credential_account_visibility_policy,
        ironhub_manifest_url,
        workspace_filesystems,
        standalone_storage_root,
        default_system_prompt_path,
        #[cfg(any(test, feature = "test-support"))]
        network_http_egress_for_test,
        #[cfg(any(test, feature = "test-support"))]
        trust_fixture_extensions_for_test,
    } = context;
    let deployment_is_local_single_user = matches!(
        production_wiring.runtime_policy.deployment,
        DeploymentMode::LocalSingleUser
    );
    let uses_local_host_runtime = local_process_port.is_some() || deployment_is_local_single_user;
    let first_party_reserved_ids = first_party_reserved_extension_ids(&first_party_bundles);
    let google_oauth_configured = google_oauth_configured(&oauth_provider_configs);
    let google_provider = VendorId::new(ironclaw_auth::GOOGLE_PROVIDER_ID).map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("provider instance readiness map could not be built: {error}"),
        }
    })?;
    let provider_instance_readiness =
        provider_instance_readiness_map([ProviderInstanceReadinessInput {
            provider: google_provider,
            configured: google_oauth_configured,
            remediation: "configure Google OAuth credentials".to_string(),
        }]);
    let owner_user_id = UserId::new(owner_id).map_err(|error| RebornBuildError::InvalidConfig {
        reason: error.to_string(),
    })?;
    let turn_state_scope = match local_runtime_identity.as_ref() {
        Some(identity) => configured_runtime_owner_scope(owner_user_id.clone(), identity),
        None => {
            default_runtime_owner_scope(owner_user_id.clone()).map_err(RebornBuildError::Mount)?
        }
    };
    let secret_store: Arc<dyn SecretStorePort> = stores.secret_credentials.secret_store.clone();
    let skill_management_filesystem: Arc<dyn RootFilesystem> = stores.filesystem.clone();
    let skill_management = Arc::new(ScopedSkillManagementPort::new_with_mount_resolver(
        owner_user_id.clone(),
        skill_management_filesystem,
        Arc::new(production_skill_management_mount_view),
    ));
    let extension_lifecycle_surface_context = extension_lifecycle_surface_context(
        owner_user_id.clone(),
        local_runtime_identity.as_ref(),
    )?;
    let channel_egress_scope = turn_state_scope.clone();
    let (skill_filesystem, workspace_filesystem, runtime_workspace_mounts) =
        match workspace_filesystems {
            Some(filesystems) => filesystems,
            None => {
                let read_only_workspace_mounts =
                    workspace_mount_view(MountPermissions::read_only(), &[]).map_err(|error| {
                        RebornBuildError::InvalidConfig {
                            reason: error.to_string(),
                        }
                    })?;
                let runtime_workspace_mounts =
                    crate::runtime_mounts::WorkspaceMountPolicy::resolve(
                        workspace_scoped_per_caller,
                        &[],
                        &[],
                    )
                    .map_err(|error| RebornBuildError::InvalidConfig {
                        reason: error.to_string(),
                    })?;
                (
                    Arc::new(ScopedFilesystem::new(
                        Arc::clone(&stores.filesystem),
                        scoped_skill_context_mount_view,
                    )),
                    Arc::new(ScopedFilesystem::with_fixed_view(
                        Arc::clone(&stores.filesystem),
                        read_only_workspace_mounts,
                    )),
                    runtime_workspace_mounts,
                )
            }
        };
    let skill_mounts =
        skill_management_mount_view().map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
    let memory_mounts =
        memory_mount_view(MountPermissions::read_write_list_delete()).map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: error.to_string(),
            }
        })?;
    let system_extensions_lifecycle_mounts = production_system_extensions_lifecycle_mount_view()
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
    let approval_requests = Arc::new(ApprovalRequestStore::new(Arc::clone(
        &stores.scoped_filesystem,
    )));
    let capability_policy =
        Arc::new(
            builtin_capability_policy().map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("capability policy is invalid: {error}"),
            })?,
        );
    let tool_permission_overrides = Arc::new(ComposedToolPermissionOverrideStore::new(Arc::clone(
        &stores.scoped_filesystem,
    )));
    let auto_approve_settings = Arc::new(ComposedAutoApproveSettingStore::new(Arc::clone(
        &stores.scoped_filesystem,
    )));
    let persistent_approval_policies_for_settings: Arc<
        dyn ironclaw_approvals::PersistentApprovalPolicyStorePort,
    > = Arc::clone(&stores.persistent_approval_policies)
        as Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStorePort>;
    let approval_settings_provider = Arc::new(StoreApprovalSettingsProvider::new(
        Arc::clone(&tool_permission_overrides)
            as Arc<dyn ironclaw_approvals::CapabilityPermissionOverrideStorePort>,
        Arc::clone(&auto_approve_settings)
            as Arc<dyn ironclaw_approvals::AutoApproveSettingStorePort>,
        persistent_approval_policies_for_settings,
    ));
    let runtime_policy = production_wiring.runtime_policy.clone();
    let runtime_policy_for_return = Some(runtime_policy.clone());
    let authorizer = capability_authorizer(
        Some(&runtime_policy),
        Arc::clone(&capability_policy),
        approval_settings_provider,
    );
    let outbound_stores = build_outbound_stores(Arc::clone(&stores.filesystem));
    let outbound_delivery_targets = host_owned_outbound_delivery_target_registry()?;
    let skill_auto_activate_learned = Arc::new(AtomicBool::new(true));
    let process_backend = production_wiring.runtime_policy.process_backend;
    let extension_registry =
        production_builtin_extension_registry(process_backend, resolved_memory.package.as_ref())?;
    let extension_registry = Arc::new(extension_registry);
    let BudgetSinks {
        budget_event_sink,
        #[cfg(any(test, feature = "test-support"))]
        in_memory_budget_event_sink,
        broadcast_budget_event_sink,
        ..
    } = build_budget_sinks();
    let process_journal_store = Arc::new(
        ProcessJournalStore::new(crate::wrap_process_journal_scoped(Arc::clone(
            &stores.filesystem,
        )))
        .with_concurrency_limits(process_concurrency_limits),
    );
    process_journal_store
        .migrate_legacy_journal()
        .await
        .map_err(|error| crate::RebornCompositionError::InvalidConfig {
            reason: format!("process journal startup migration failed: {error}"),
        })?;
    let processes =
        ProcessRuntimeSystem::from_process_journal_store(Arc::clone(&process_journal_store));
    let process_lifecycle_lookup_source = processes.lifecycle();
    let process_gate_query_source = processes.gates();
    let process_turn_state = Arc::new(processes.agent_turn_runtime());
    let trigger_source_reply_target: Arc<std::sync::RwLock<Arc<dyn TriggerSourceReplyTarget>>> =
        Arc::new(std::sync::RwLock::new(Arc::new(
            TurnStateTriggerSourceReplyTarget::new(
                Arc::clone(&process_turn_state) as Arc<dyn AgentTurnRuntimePort>
            ),
        )));
    let trigger_create_hook = Arc::new(TriggerCreatorPairingHook {
        outbound_delivery_targets: Arc::clone(&outbound_delivery_targets),
        source_reply_target: Arc::clone(&trigger_source_reply_target),
        scoped_filesystem: Arc::clone(&stores.scoped_filesystem),
        conversations: tokio::sync::OnceCell::new(),
    });
    let thread_service: Arc<dyn SessionThreadService> = Arc::new(
        FilesystemSessionThreadService::new(Arc::clone(&stores.scoped_filesystem)),
    );
    let resource_governor = Arc::new(
        stores
            .resource_governor
            .with_event_sink(Arc::clone(&budget_event_sink)),
    );
    let production_resource_governor: Arc<dyn ResourceGovernor> = resource_governor.clone();
    let budget_gate_store: Arc<dyn BudgetGateStorePort> =
        Arc::new(BudgetGateStore::new(Arc::clone(&stores.scoped_filesystem)));
    let event_stores = ironclaw_event_store::build_reborn_event_stores(
        profile.to_event_store_profile(),
        stores.event_store,
    )
    .await?;
    let event_log = Arc::clone(&event_stores.events);
    let audit_log = Arc::clone(&event_stores.audit);
    let admin_secret_provisioner: Arc<dyn ironclaw_assistant::AdminSecretProvisioner> =
        Arc::new(crate::admin_secrets::FilesystemAdminSecretProvisioner::new(
            Arc::clone(&stores.filesystem),
            Arc::clone(&stores.secret_credentials.crypto),
        ));
    let project_agent_id =
        ironclaw_host_api::ids::AgentId::new("reborn-projects").map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: format!("invalid project agent id: {error}"),
            }
        })?;
    let project_repository: Arc<dyn ProjectRepository> = Arc::new(
        ironclaw_identity::projects::FilesystemProjectRepository::new(
            Arc::clone(&stores.scoped_filesystem),
            owner_user_id.clone(),
            project_agent_id,
        ),
    );
    let project_service: Arc<dyn ProjectService> =
        Arc::new(RebornProjectService::new(project_repository));
    let trigger_conversation_services =
        RebornFilesystemConversationServices::new(Arc::clone(&stores.scoped_filesystem))
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("trigger conversation services unavailable: {error}"),
            })?;
    let trigger_process_lifecycle_source: Arc<
        std::sync::RwLock<
            Arc<
                dyn ironclaw_processes::ProcessLifecycleLookupSource<
                        Error = ironclaw_turns::TurnError,
                    >,
            >,
        >,
    > = Arc::new(std::sync::RwLock::new(Arc::clone(
        &process_lifecycle_lookup_source,
    )));
    let trigger_active_run_lookup: Arc<dyn TriggerActiveRunLookup> = Arc::new(
        crate::automation::trigger_poller::ProcessActiveRunLookup::new(Arc::new(
            crate::automation::trigger_poller::RebindableProcessLifecycleLookupSource::new(
                Arc::clone(&trigger_process_lifecycle_source),
            ),
        )
            as Arc<
                dyn ironclaw_processes::ProcessLifecycleLookupSource<
                        Error = ironclaw_turns::TurnError,
                    >,
            >),
    );
    let mut first_party_registry = production_first_party_registry_with_trigger_create_hook(
        Arc::clone(&trigger_repository),
        trigger_create_hook,
        trigger_active_run_lookup,
        process_backend,
    )?;
    if let (Some(package), Some(handler)) = (
        resolved_memory.package.as_ref(),
        resolved_memory.tool_handler.as_ref(),
    ) {
        ironclaw_host_runtime::register_memory_tool_handler(
            &mut first_party_registry,
            package,
            Arc::clone(handler),
        );
    }
    let product_auth_filesystem = Arc::clone(&stores.scoped_filesystem);
    let services = with_shared_host_runtime_wiring!(
        HostRuntimeServices::new(
            Arc::clone(&extension_registry),
            Arc::clone(&stores.filesystem),
            Arc::new(InMemoryResourceGovernor::new()),
            authorizer,
            ProcessServices::filesystem(Arc::clone(&stores.scoped_filesystem)),
            CapabilitySurfaceVersion::new("reborn-app-v1")?,
        ),
        trust_policy = Arc::clone(&production_wiring.trust_policy),
        runtime_policy = runtime_policy,
        capability_leases = Arc::clone(&stores.leases),
        persistent_approval_policies = Arc::clone(&stores.persistent_approval_policies),
        secret_store = Arc::clone(&stores.secret_credentials.secret_store),
        credential_broker = stores.secret_credentials.credential_broker,
        process_runtime = processes.runtime(),
        approval_filesystem = Arc::clone(&stores.scoped_filesystem),
        turn_state = Arc::clone(&process_turn_state),
        run_profile_resolver = planned_run_profile_resolver()?,
    )
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_resource_governor(Arc::clone(&resource_governor))
    .with_production_reborn_event_stores(event_stores)
    .with_turn_run_wake_notifier_dyn(production_wiring.turn_run_wake_notifier);
    #[cfg(any(test, feature = "test-support"))]
    let network_http_egress = match network_http_egress_for_test {
        Some(test_egress) => test_egress,
        None => Arc::new(default_host_http_egress()?),
    };
    #[cfg(not(any(test, feature = "test-support")))]
    let network_http_egress: Arc<dyn ironclaw_network::NetworkHttpEgress> =
        Arc::new(default_host_http_egress()?);
    let http_body_store = Arc::clone(&stores.scoped_filesystem);
    let services =
        services.try_with_host_http_egress_with_body_store(network_http_egress, http_body_store)?;
    // Provider-client assembly needs mediated HTTP and secret staging before
    // product-auth itself exists. Account-backed credential resolution is
    // attached below, after product-auth services have been composed.
    let provider_runtime_ports = require_product_auth_runtime_ports(&services)?;
    let services = attach_hosted_mcp_runtime(services)?;
    let extension_filesystem: Arc<dyn RootFilesystem> = stores.filesystem.clone();
    let extension_host_ports =
        ironclaw_host_api::host_port::default_host_port_catalog().map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: format!("extension host port catalog could not be loaded: {error}"),
            }
        })?;
    let extension_host_api_contracts =
        product_extension_host_api_contract_registry().map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: format!("extension host API contracts could not be loaded: {error}"),
            }
        })?;
    let extension_installation_state_path = ExtensionInstallationStore::default_state_path()
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("extension installation state path is invalid: {error}"),
        })?;
    let extension_installation_store: Arc<dyn ExtensionInstallationStorePort> = Arc::new(
        ExtensionInstallationStore::load_at(
            extension_filesystem.clone(),
            extension_installation_state_path,
            extension_host_ports,
            extension_host_api_contracts,
        )
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("extension installation state could not be loaded: {error}"),
        })?,
    );
    let admin_configuration_credential_slot = AdminConfigurationCredentialSlot::default();
    let provider_composition = compose_provider_client(
        oauth_provider_configs,
        oauth_dcr_callback,
        Arc::clone(&secret_store),
        provider_runtime_ports,
        admin_configuration_credential_slot.clone(),
        &first_party_bundles,
        Arc::clone(&extension_installation_store),
    )?;
    let services = if let Some(process_port) = local_process_port {
        services.with_runtime_process_port(Arc::new(process_port))
    } else {
        services
    };
    let services = apply_production_runtime_process_binding(
        services,
        production_wiring.runtime_process_binding,
    );
    let services = apply_post_edit_check_from_env(services)?;
    let security_audit_sink = services.security_audit_sink();

    let turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> =
        Arc::new(services.turn_coordinator_for_production()?);
    let credential_refresh_candidate_source: Option<
        Arc<dyn ironclaw_auth::KeepaliveCandidateSource>,
    >;
    let product_auth_flow_record_source: Option<Arc<dyn ironclaw_auth::AuthFlowRecordSource>>;
    let product_auth_ports = match product_auth_ports {
        Some(ports) => {
            credential_refresh_candidate_source = None;
            product_auth_flow_record_source = None;
            ports
        }
        None => {
            let durable = Arc::new(FilesystemAuthProductServices::new_with_root(
                product_auth_filesystem,
                Arc::clone(&stores.filesystem),
                Arc::clone(&secret_store),
            ));
            credential_refresh_candidate_source =
                Some(Arc::clone(&durable) as Arc<dyn ironclaw_auth::KeepaliveCandidateSource>);
            product_auth_flow_record_source =
                Some(Arc::clone(&durable) as Arc<dyn ironclaw_auth::AuthFlowRecordSource>);
            RebornProductAuthServicePorts::from_shared_with_provider(
                durable,
                provider_composition
                    .client
                    .clone()
                    .unwrap_or_else(|| Arc::new(UnavailableAuthProviderClient)),
            )
        }
    };
    let keepalive_recipes = provider_composition
        .engine
        .as_ref()
        .map(|engine| Arc::clone(engine.recipes()));
    let (product_auth_core, base_auth_continuation) =
        compose_product_auth_services(ProductAuthServicesCompositionInput {
            ports: product_auth_ports,
            turn_coordinator: turn_coordinator.clone(),
            blocked_auth_snapshot_source: Some(Arc::clone(&process_gate_query_source)),
            provider_composition,
            security_audit_sink,
            secret_store: Arc::clone(&secret_store),
            nearai_mcp_host_managed_scope: Some(AuthProductScope::new(
                channel_egress_scope.clone(),
                AuthSurface::Api,
            )),
            credential_account_visibility_policy,
            flow_record_source: product_auth_flow_record_source,
        })?;
    let product_auth_dependencies = Arc::new(product_auth_core.clone());
    let product_auth_ready = true;
    let mut services = services.with_runtime_credential_account_resolver(Arc::new(
        ProductAuthRuntimeCredentialResolver::new_with_refresh(
            product_auth_dependencies.runtime_credential_account_selection_service(),
            product_auth_dependencies.runtime_credential_account_refresh_service(),
        ),
    ));
    services = attach_wasm_runtime(services)?;
    // Re-project the ports after attaching product-auth. Hosted MCP
    // preparation and first-party registrars must receive the account-aware
    // obligation handler, not the earlier provider-bootstrap projection.
    let product_auth_runtime_ports = require_product_auth_runtime_ports(&services)?;
    let first_party_registrar_context = FirstPartyRegistrarContext {
        credential_account_service: product_auth_dependencies.credential_account_service(),
        credential_account_record_source: product_auth_dependencies
            .credential_account_record_source(),
        product_auth_runtime_ports: product_auth_runtime_ports.clone(),
        oauth_backend_configured: google_oauth_configured,
    };
    for registrar in &first_party_registrars {
        registrar
            .register(&mut first_party_registry, &first_party_registrar_context)
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("first-party capability handlers are invalid: {error}"),
            })?;
    }
    let persisted_manifest_sources = extension_installation_store
        .list_manifests()
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("extension installation manifests could not be loaded: {error}"),
        })?
        .into_iter()
        .map(|record| {
            // Keep the persisted identity typed: the record already carries a
            // validated ExtensionId, and downgrading it to String lets an
            // unnormalized key silently miss every lookup.
            (record.manifest().id.clone(), record.manifest().source)
        })
        .collect::<BTreeMap<ExtensionId, ManifestSource>>();
    let extensions_root = VirtualPath::new("/system/extensions")?;
    #[cfg(any(test, feature = "test-support"))]
    let filesystem_catalog = if trust_fixture_extensions_for_test {
        AvailableExtensionCatalog::from_trusted_fixture_filesystem_root(
            stores.filesystem.as_ref(),
            &extensions_root,
            &first_party_reserved_ids,
        )
        .await
    } else {
        AvailableExtensionCatalog::from_filesystem_root_with_manifest_sources(
            stores.filesystem.as_ref(),
            &extensions_root,
            &first_party_reserved_ids,
            &persisted_manifest_sources,
        )
        .await
    };
    #[cfg(not(any(test, feature = "test-support")))]
    let filesystem_catalog = AvailableExtensionCatalog::from_filesystem_root_with_manifest_sources(
        stores.filesystem.as_ref(),
        &extensions_root,
        &first_party_reserved_ids,
        &persisted_manifest_sources,
    )
    .await;
    let mut available_extensions =
        filesystem_catalog.map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("available extension catalog could not be loaded: {error}"),
        })?;
    let nearai_mcp_catalog_config = nearai_mcp_bootstrap_config
        .clone()
        .map(|config| {
            let endpoint = config
                .endpoint()
                .map_err(|error| format!("NEAR AI MCP catalog endpoint is invalid: {error}"))?;
            ironclaw_extension_host::NearAiMcpBootstrapConfig::new(
                endpoint.url,
                config.into_api_key(),
            )
            .map_err(|error| error.to_string())
        })
        .transpose()
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("nearai MCP catalog config is invalid: {error}"),
        })?;
    available_extensions.extend(
        AvailableExtensionCatalog::from_first_party_assets_with_nearai_mcp_config(
            nearai_mcp_catalog_config.as_ref(),
            &first_party_bundles,
        )
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("first-party extension catalog could not be loaded: {error}"),
        })?,
    );
    available_extensions =
        available_extensions.with_reserved_bundled_ids(first_party_reserved_ids.clone());
    let admin_configuration_uses = available_extensions.admin_configuration_uses();
    let mut admin_configuration_consumers = std::collections::BTreeMap::new();
    for usage in &admin_configuration_uses {
        let extension_id =
            ironclaw_host_api::ids::ExtensionId::new(usage.package_id.clone()).map_err(|error| {
                RebornBuildError::InvalidConfig {
                    reason: format!(
                        "administrator configuration consumer `{}` has an invalid extension id: {error}",
                        usage.package_id
                    ),
                }
            })?;
        admin_configuration_consumers
            .entry(usage.descriptor.group_id.clone())
            .or_insert_with(std::collections::BTreeSet::new)
            .insert(extension_id);
    }
    let available_manifests = available_extensions.resolved_manifests();
    account_setup_descriptors.extend(manifest_channel_account_setup_descriptors(
        &available_manifests,
    ));
    let deployment_bindings = available_manifests
        .iter()
        .filter(|manifest| {
            manifest
                .channel
                .as_ref()
                .is_some_and(|channel| channel.inbound && channel.ingress.is_some())
        })
        .filter_map(|manifest| {
            channel_extension_bindings
                .iter()
                .find(|binding| binding.extension_id == manifest.id)
                .map(|binding| {
                    ironclaw_extension_host::DeploymentChannelBinding::new(
                        Arc::clone(manifest),
                        Arc::clone(&binding.adapter),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("deployment channel registry could not be built: {error}"),
        })?;
    let deployment_channels = Arc::new(
        ironclaw_extension_host::DeploymentChannelRegistry::try_new(deployment_bindings).map_err(
            |error| RebornBuildError::InvalidConfig {
                reason: format!("deployment channel registry could not be built: {error}"),
            },
        )?,
    );
    let admin_configuration_filesystem: Arc<dyn RootFilesystem> = stores.filesystem.clone();
    let admin_configuration = Arc::new(
        AdminConfigurationService::new(
            FilesystemAdminConfigurationStore::new(Arc::new(ScopedFilesystem::new(
                admin_configuration_filesystem,
                crate::invocation_mount_view,
            ))),
            Arc::clone(&secret_store),
            admin_configuration_uses
                .iter()
                .map(|usage| usage.descriptor.clone()),
        )
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("admin configuration service could not be built: {error}"),
        })?,
    );
    let extension_lifecycle_service = Arc::new(tokio::sync::Mutex::new(
        ExtensionLifecycleService::new(services.shared_extension_registry().snapshot_owned()),
    ));
    let active_extensions = ActiveExtensionPublisher::new(
        services.shared_extension_registry(),
        Arc::clone(&production_wiring.trust_policy),
        Arc::new(ironclaw_trust::InvalidationBus::new()),
    );
    restore_extension_lifecycle_state(
        &mut available_extensions,
        &extension_filesystem,
        &extension_installation_store,
        &extension_lifecycle_service,
        &active_extensions,
        &owner_user_id,
    )
    .await
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("extension lifecycle state could not be restored: {error}"),
    })?;
    let removal_cleanup_adapters: Vec<Arc<dyn ExtensionRemovalCleanupAdapter>> = Vec::new();
    let removal_cleanup = Arc::new(
        ExtensionRemovalCleanupRegistry::try_from_adapters(removal_cleanup_adapters).map_err(
            |error| RebornBuildError::InvalidConfig {
                reason: format!("extension removal cleanup registry could not be built: {error}"),
            },
        )?,
    );
    let account_setups = ExtensionAccountSetupRegistry::default();
    let channel_disconnect_slot: Arc<
        std::sync::OnceLock<Arc<dyn ironclaw_auth::ChannelConnectionService>>,
    > = Arc::new(std::sync::OnceLock::new());
    let extension_management = Arc::new(
        RebornLocalExtensionManagementPort::new(
            ironclaw_extension_host::ExtensionLifecycleManagerDependencies {
                filesystem: extension_filesystem,
                catalog: available_extensions,
                installation_store: extension_installation_store,
                lifecycle_service: extension_lifecycle_service,
                active_extensions,
                credential_cleanup: Some(Arc::new(RebornProductAuthCredentialCleanup::new(
                    Arc::clone(&product_auth_dependencies),
                )) as Arc<dyn ExtensionCredentialCleanup>),
                tenant_operator_user_id: channel_egress_scope.user_id.clone(),
                hosted_mcp_dependencies:
                    ironclaw_extension_host::HostedMcpPreparationDependencies {
                        runtime_ports: Some(product_auth_runtime_ports.clone()),
                        catalog_safety: ironclaw_extension_host::McpCatalogAdmissionPolicy::new(
                            Arc::new(ironclaw_safety::Sanitizer::new()),
                        ),
                        oauth_client_profiles: Arc::new(
                            ironclaw_auth::EmptyOAuthClientProfileRegistry,
                        ),
                    },
            },
        )
        .with_account_setup_registry(Arc::new(account_setups.clone()))
        .with_removal_cleanup_registry(removal_cleanup)
        .with_provider_instance_readiness(provider_instance_readiness)
        .with_channel_disconnect_slot(Arc::clone(&channel_disconnect_slot)),
    );
    let nearai_mcp_bootstrap_outcome = crate::llm_admin::nearai_mcp::bootstrap_nearai_mcp(
        nearai_mcp_bootstrap_config,
        &product_auth_dependencies,
        &extension_management,
        channel_egress_scope.clone(),
    )
    .await?;
    nearai_mcp_bootstrap_outcome.log_completion();
    let admin_configuration_resolver = Arc::new(
        ChannelConfigService::new(
            extension_management.installation_store_handle(),
            Arc::clone(&secret_store),
            channel_egress_scope.clone(),
            Arc::clone(&extension_management)
                as Arc<dyn ironclaw_extension_host::ChannelConfigReactivation>,
        )
        .with_admin_configuration(
            Arc::clone(&admin_configuration),
            channel_egress_scope.clone(),
        )
        .with_available_manifests(available_manifests.clone()),
    );
    extension_management.attach_channel_config(&admin_configuration_resolver);
    admin_configuration_credential_slot.fill(Arc::clone(&admin_configuration_resolver));
    let lifecycle_continuation_facade: Arc<dyn LifecycleProductService> = Arc::new(
        ironclaw_extension_manager::ExtensionHostLifecycleProductService::new(Arc::clone(
            &skill_management,
        ))
        .with_extension_management(Arc::clone(&extension_management))
        .with_channel_config(Arc::clone(&admin_configuration_resolver))
        .with_runtime_credential_accounts(
            product_auth_dependencies.runtime_credential_account_selection_service(),
        ),
    );
    let lifecycle_wrapped_product_continuation =
        ironclaw_assistant::lifecycle_auth_continuation_dispatcher(
            lifecycle_continuation_facade,
            base_auth_continuation,
        );
    let lifecycle_wrapped_auth_continuation: Arc<dyn RebornAuthContinuationDispatcher> =
        lifecycle_wrapped_product_continuation;
    let product_auth_services = Arc::new(
        product_auth_core
            .with_continuation_dispatcher(Arc::clone(&lifecycle_wrapped_auth_continuation)),
    );
    let credential_refresh_worker = match (credential_refresh_candidate_source, keepalive_recipes) {
        (Some(candidate_source), Some(recipes)) => CredentialRefreshWorkerReady::Ready {
            candidate_source,
            recipes,
            leader_lock,
            refresh_port: Arc::clone(&product_auth_services),
        },
        _ => CredentialRefreshWorkerReady::Absent,
    };
    let fold_filesystem: Arc<dyn RootFilesystem> = stores.filesystem.clone();
    let channel_identity_store = Arc::new(
        ironclaw_extension_host::FilesystemChannelIdentityStore::new(
            Arc::clone(&fold_filesystem),
            channel_egress_scope.tenant_id.clone(),
            channel_egress_scope.user_id.clone(),
        ),
    );
    let channel_dm_target_store = Arc::new(
        ironclaw_extension_host::FilesystemChannelDmTargetStore::new(
            Arc::clone(&fold_filesystem),
            channel_egress_scope.tenant_id.clone(),
            channel_egress_scope.user_id.clone(),
        ),
    );
    let runtime_http_egress = Some(product_auth_runtime_ports.runtime_http_egress());
    let host_runtime_http_egress = services.host_runtime_http_egress_port();
    let ironhub_link_state = Arc::new(
        ironclaw_extension_manager::ironhub::IronhubLinkStateStore::new(Arc::clone(
            &fold_filesystem,
        )),
    );
    insert_extension_lifecycle_handlers(
        &mut first_party_registry,
        Arc::clone(&extension_management),
        product_auth_services.runtime_credential_account_selection_service(),
    )
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("extension lifecycle handlers are invalid: {error}"),
    })?;
    insert_ironhub_handlers(
        &mut first_party_registry,
        Arc::clone(&skill_management),
        Arc::clone(&extension_management),
        Arc::clone(&ironhub_link_state),
        ironhub_manifest_url,
    )
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("IronHub handlers are invalid: {error}"),
    })?;
    insert_admin_configuration_handler(
        &mut first_party_registry,
        Arc::clone(&admin_configuration),
        channel_egress_scope.user_id.clone(),
        Arc::clone(&extension_management)
            as Arc<dyn ironclaw_extension_host::ChannelConfigReactivation>,
        admin_configuration_consumers,
    )
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("admin configuration handler is invalid: {error}"),
    })?;
    let operator_auto_approve_settings: Arc<dyn ironclaw_approvals::AutoApproveSettingStorePort> =
        Arc::clone(&auto_approve_settings)
            as Arc<dyn ironclaw_approvals::AutoApproveSettingStorePort>;
    let operator_tool_permission_overrides: Arc<
        dyn ironclaw_approvals::CapabilityPermissionOverrideStorePort,
    > = Arc::clone(&tool_permission_overrides)
        as Arc<dyn ironclaw_approvals::CapabilityPermissionOverrideStorePort>;
    let operator_persistent_approval_policies: Arc<
        dyn ironclaw_approvals::PersistentApprovalPolicyStorePort,
    > = Arc::clone(&stores.persistent_approval_policies)
        as Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStorePort>;
    let operator_synthetic_tools = {
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
    let operator_tool_catalog: Arc<dyn RebornOperatorToolCatalog> =
        Arc::new(ActiveRegistryOperatorToolCatalog::new(
            services.shared_extension_registry(),
            operator_synthetic_tools,
            Some(Arc::clone(&extension_management)),
        ));
    insert_operator_config_handler(
        &mut first_party_registry,
        operator_auto_approve_settings,
        operator_tool_permission_overrides,
        operator_persistent_approval_policies,
        operator_tool_catalog,
    )
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("operator configuration handler is invalid: {error}"),
    })?;
    let outbound_target_provider = Arc::clone(&outbound_delivery_targets)
        as Arc<dyn crate::outbound::OutboundDeliveryTargetProvider>;
    let outbound_preferences_facade: Arc<dyn OutboundPreferencesProductService> =
        Arc::new(crate::outbound::RebornOutboundPreferencesService::new(
            Arc::clone(&outbound_stores.outbound_preferences),
            outbound_target_provider,
        ));
    insert_outbound_preferences_handler(&mut first_party_registry, outbound_preferences_facade)
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("outbound preferences handler is invalid: {error}"),
        })?;
    ironclaw_host_runtime::register_reply_attachment_first_party_handler(
        &mut first_party_registry,
        Arc::clone(&outbound_stores.reply_attachment_intents),
    )
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("reply attachment handler is invalid: {error}"),
    })?;
    insert_skill_auto_activate_handler(
        &mut first_party_registry,
        Arc::clone(&skill_auto_activate_learned),
    )
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("skill auto-activation handler is invalid: {error}"),
    })?;
    services = services.with_first_party_capabilities(Arc::new(first_party_registry));
    let admin_configuration_resolver_for_generic = Arc::clone(&admin_configuration_resolver);
    let channel_pairing_registry;
    let channel_host_wiring = {
        let mut reserved_capability_ids: std::collections::BTreeSet<_> = services
            .shared_extension_registry()
            .snapshot()
            .capabilities()
            .filter(|descriptor| {
                descriptor.provider.as_str() == ironclaw_host_runtime::BUILTIN_FIRST_PARTY_PROVIDER
            })
            .map(|descriptor| descriptor.id.clone())
            .collect();
        reserved_capability_ids.extend(ironclaw_loop_host::bridge_capability_ids());
        let generic_installation_store = extension_management.installation_store_handle();
        let backend_extension_host =
            build_backend_extension_host(BackendExtensionHostAssemblyInput {
                binder: services.extension_lane_tool_binder(),
                native_factories: native_extension_factories,
                channel_bindings: channel_extension_bindings.clone(),
                installation_store: generic_installation_store,
                admin_configuration_resolver: Arc::clone(&admin_configuration_resolver_for_generic),
                resource_governor: Arc::clone(&resource_governor)
                    as Arc<dyn ironclaw_resources::ResourceGovernor>,
                reserved_capability_ids,
                host_runtime_http_egress: host_runtime_http_egress.clone(),
                channel_egress_scope: channel_egress_scope.clone(),
                deployment_channels: Arc::clone(&deployment_channels),
                filesystem: Arc::clone(&stores.filesystem),
                outbound_state: Arc::clone(&outbound_stores.outbound_state)
                    as Arc<dyn ironclaw_outbound::OutboundStateStorePort>,
            })
            .await?;
        let pairing_installation_store = Arc::clone(&backend_extension_host.installation_store);
        extension_management.attach_generic_host(Arc::clone(&backend_extension_host.generic_host));
        services.set_extension_tool_resolver(backend_extension_host.resolver);
        let channel_pairing_registry_built =
            build_backend_channel_pairing(BackendChannelPairingAssemblyInput {
                descriptors: account_setup_descriptors,
                account_setups,
                filesystem: Arc::clone(&fold_filesystem),
                scope: channel_egress_scope.clone(),
                installation_store: pairing_installation_store,
                admin_configuration_resolver: Arc::clone(&admin_configuration_resolver_for_generic),
                continuation: lifecycle_wrapped_auth_continuation,
                identity_store: Arc::clone(&channel_identity_store),
                dm_targets: Arc::clone(&channel_dm_target_store),
                credential_cleanup: Arc::clone(&product_auth_services)
                    as Arc<
                        dyn ironclaw_extension_host::channel_connection::ChannelCredentialCleanup,
                    >,
                account_status_reader: Arc::clone(&product_auth_services)
                    as Arc<
                        dyn ironclaw_extension_host::channel_connection::ChannelAccountStatusReader,
                    >,
                disconnect_slot: Arc::clone(&channel_disconnect_slot),
            })
            .await?;
        channel_pairing_registry = Some(Arc::clone(&channel_pairing_registry_built));
        ChannelHostWiring {
            extension_ingress: Some(backend_extension_host.ingress),
            delivery_coordinator: backend_extension_host.delivery_coordinator,
            channel_delivery_resolver: backend_extension_host.channel_delivery_resolver,
            #[cfg(feature = "test-support")]
            channel_egress_credential_bridges: Some(
                backend_extension_host.channel_egress_credential_bridges,
            ),
        }
    };
    let shared_extension_registry = services.shared_extension_registry();

    #[cfg(test)]
    let standalone_wasm_runtime_credential_provider_captured =
        services.wasm_runtime_credential_provider_captured_for_test();
    let host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime> = if uses_local_host_runtime {
        Arc::new(services.host_runtime_for_local_testing())
    } else {
        Arc::new(services.host_runtime_for_production(&wiring_config)?)
    };

    Ok(RebornRuntimeStores {
        host_runtime,
        #[cfg(test)]
        turn_coordinator,
        readiness: readiness_for(profile, true, true, product_auth_ready),
        product_auth: product_auth_services,
        skill_management,
        extension_lifecycle_surface_context,
        owner_user_id,
        approval_requests: Arc::clone(&approval_requests),
        capability_leases: Arc::clone(&stores.leases),
        external_tool_catalog: Arc::new(InMemoryExternalToolCatalog::new()),
        runtime_policy: runtime_policy_for_return,
        persistent_approval_policies: Arc::clone(&stores.persistent_approval_policies),
        tool_permission_overrides: Arc::clone(&tool_permission_overrides),
        auto_approve_settings: Arc::clone(&auto_approve_settings),
        #[cfg(any(test, feature = "test-support"))]
        capability_policy: Arc::clone(&capability_policy),
        outbound_preferences: outbound_stores.outbound_preferences,
        outbound_delivery_targets: Arc::clone(&outbound_delivery_targets),
        skill_auto_activate_learned: Arc::clone(&skill_auto_activate_learned),
        outbound_state: outbound_stores.outbound_state,
        reply_attachment_intents: outbound_stores.reply_attachment_intents,
        delivered_gate_routes: outbound_stores.delivered_gate_routes,
        triggered_run_delivery: outbound_stores.triggered_run_delivery,
        process_gate_query_source,
        #[cfg(any(test, feature = "test-support"))]
        trigger_process_lifecycle_source,
        #[cfg(any(test, feature = "test-support"))]
        trigger_source_reply_target,
        extension_management,
        admin_configuration,
        admin_configuration_uses: Arc::new(admin_configuration_uses),
        channel_config_service: Arc::clone(&admin_configuration_resolver),
        channel_identity_store,
        channel_dm_target_store,
        channel_disconnect_slot,
        runtime_http_egress,
        ironhub_link_state,
        skill_mounts,
        memory_mounts,
        system_extensions_lifecycle_mounts,
        skill_filesystem,
        workspace_filesystem,
        extension_filesystem: Arc::clone(&stores.filesystem),
        memory_service_resolver: resolved_memory.resolver.clone(),
        memory_lifecycle: resolved_memory.lifecycle.clone(),
        workspace_mounts: runtime_workspace_mounts,
        standalone_storage_root,
        default_system_prompt_path,
        #[cfg(any(test, feature = "test-support"))]
        in_memory_budget_event_sink,
        extension_registry: Arc::clone(&extension_registry),
        shared_extension_registry,
        scoped_filesystem: Arc::clone(&stores.scoped_filesystem),
        processes,
        thread_service,
        trigger_repository: Arc::clone(&trigger_repository),
        resource_governor: production_resource_governor,
        budget_gate_store,
        broadcast_budget_event_sink,
        event_log,
        audit_log,
        admin_secret_provisioner,
        project_service,
        trigger_conversation_services,
        production_scheduler_wake: Some(scheduler_wake_wiring),
        secret_store,
        #[cfg(test)]
        standalone_wasm_runtime_credential_provider_captured,
        credential_refresh_worker,
        channel_extension_bindings,
        deployment_channels,
        extension_ingress: channel_host_wiring.extension_ingress,
        channel_pairing: channel_pairing_registry,
        delivery_coordinator: channel_host_wiring.delivery_coordinator,
        channel_delivery_resolver: channel_host_wiring.channel_delivery_resolver,
        #[cfg(feature = "test-support")]
        channel_egress_credential_bridges: channel_host_wiring.channel_egress_credential_bridges,
    })
}

async fn finish_production_backend(
    context: RebornProductionBuildContext,
    filesystem: Arc<CompositeRootFilesystem>,
    trigger_repository: Arc<dyn TriggerRepository>,
    secret_master_key: ironclaw_secrets::SecretMaterial,
    event_store_config: ironclaw_event_store::RebornEventStoreConfig,
    leader_lock: ironclaw_auth::CredentialRefreshLeaderLock,
) -> Result<RebornRuntimeStores, RebornBuildError> {
    let resource_governor = filesystem_resource_governor(&filesystem);
    let stores = ProductionStoreBundle::new(
        filesystem,
        resource_governor,
        secret_master_key,
        event_store_config,
    )
    .await?;
    build_backend_production(context, stores, trigger_repository, leader_lock).await
}

#[cfg(any(test, feature = "test-support"))]
pub(super) async fn build_libsql_production(
    context: RebornProductionBuildContext,
    runtime: Arc<ironclaw_libsql_runtime::LibSqlRuntime>,
    path_or_url: String,
    secret_master_key: ironclaw_secrets::SecretMaterial,
    process_local_resource_governor_singleton: bool,
) -> Result<RebornRuntimeStores, RebornBuildError> {
    use ironclaw_filesystem::LibSqlRootFilesystem;

    ensure_libsql_resource_governor_authority_for_build(process_local_resource_governor_singleton)?;
    let database_filesystem = Arc::new(LibSqlRootFilesystem::from_runtime(Arc::clone(&runtime)));
    database_filesystem.run_migrations().await?;
    let trigger_repository = Arc::new(ironclaw_triggers::LibSqlTriggerRepository::from_runtime(
        runtime,
    ));
    trigger_repository
        .run_migrations()
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("libSQL trigger repository migrations failed: {error}"),
        })?;
    let filesystem = production_database_root_filesystem(
        Arc::clone(&database_filesystem),
        "production-libsql-reborn-state",
    )?;
    let event_store_config = ironclaw_event_store::RebornEventStoreConfig::LibsqlFilesystem {
        filesystem: database_filesystem,
        path_or_url,
    };
    finish_production_backend(
        context,
        filesystem,
        trigger_repository,
        secret_master_key,
        event_store_config,
        ironclaw_auth::CredentialRefreshLeaderLock::always_leader_for_single_writer(),
    )
    .await
}

pub(super) async fn build_postgres_production(
    context: RebornProductionBuildContext,
    pool: deadpool_postgres::Pool,
    secret_master_key: ironclaw_secrets::SecretMaterial,
    process_local_resource_governor_singleton: bool,
) -> Result<RebornRuntimeStores, RebornBuildError> {
    use ironclaw_filesystem::PostgresRootFilesystem;

    ensure_postgres_resource_governor_authority_for_build(
        process_local_resource_governor_singleton,
    )?;
    let pool_for_refresh_lock = pool.clone();
    let database_filesystem = Arc::new(PostgresRootFilesystem::new(pool.clone()));
    database_filesystem.run_migrations().await?;
    let trigger_repository = Arc::new(ironclaw_triggers::PostgresTriggerRepository::new(
        pool.clone(),
    ));
    trigger_repository
        .run_migrations()
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("PostgreSQL trigger repository migrations failed: {error}"),
        })?;
    let filesystem = production_database_root_filesystem(
        database_filesystem,
        "production-postgres-reborn-state",
    )?;
    finish_production_backend(
        context,
        filesystem,
        trigger_repository,
        secret_master_key,
        ironclaw_event_store::RebornEventStoreConfig::PostgresPool {
            pool: ironclaw_filesystem::PostgresConnectionPool::new(pool),
        },
        ironclaw_auth::CredentialRefreshLeaderLock::for_postgres(pool_for_refresh_lock),
    )
    .await
}
