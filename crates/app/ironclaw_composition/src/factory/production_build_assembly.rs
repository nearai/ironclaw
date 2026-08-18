use super::*;
use ironclaw_product_contracts::account_setup::ExtensionAccountSetupDescriptor;

pub(super) async fn build_production_shaped(
    input: RebornHostBindings,
) -> Result<RebornRuntimeStores, RebornBuildError> {
    let RebornHostBindings {
        deployment,
        storage,
        ironhub_manifest_url,
        production_trust_policy,
        // Compatibility input; the build mints one shared scheduler channel.
        turn_run_wake_notifier: _,
        runtime_process_binding,
        product_auth_ports,
        native_extension_factories,
        channel_extension_bindings,
        first_party_registrars,
        credential_account_visibility_policy,
        #[cfg(any(test, feature = "test-support"))]
        network_http_egress_for_test,
        #[cfg(any(test, feature = "test-support"))]
        trust_fixture_extensions_for_test,
        memory_binding_policy,
        memory_provider_connection,
        ..
    } = input;
    let owner_id = deployment.owner_id.clone();
    let local_runtime_identity = deployment.local_runtime_identity.clone();
    let runtime_policy = deployment.runtime_policy.clone();
    let account_setup_descriptors = deployment.account_setup_descriptors.clone();
    let oauth_provider_configs = deployment.oauth_provider_configs.clone();
    let oauth_dcr_callback = deployment.oauth_dcr_callback.clone();
    let nearai_mcp_bootstrap_config = deployment.nearai_mcp_bootstrap_config.clone();
    let process_concurrency_limits = deployment.process_concurrency_limits.clone();
    let first_party_bundles = deployment.first_party_bundles.clone();
    let traffic_policy = deployment.traffic();
    // Scope an implicit mem0 app id to the standalone storage root.
    let resolved_memory_provider = {
        let mut memory_provider_connection = memory_provider_connection;
        if memory_provider_connection.app_id.is_none()
            && let crate::input::RebornStorageInput::LocalFilesystem { root, .. } = &storage
        {
            use std::hash::{DefaultHasher, Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            root.hash(&mut hasher);
            memory_provider_connection.app_id = Some(format!("ws-{:016x}", hasher.finish()));
        }
        crate::resolve_memory_provider(
            memory_binding_policy,
            &crate::MemoryProviderDeps::for_third_party(memory_provider_connection),
        )?
    };
    let profile = deployment.profile();
    let wiring_config = production_config(
        deployment.required_runtime_backends.clone(),
        deployment.require_runtime_http_egress,
        deployment.require_wasm_credentials,
    );
    // Build default trust from the injected first-party inventory.
    let production_trust_policy = match production_trust_policy {
        Some(policy) => Some(policy),
        None => Some(Arc::new(production_first_party_trust_policy(
            &first_party_bundles,
        )?)),
    };
    let workspace_scoped_per_caller = deployment.workspace_scoped_per_caller();
    let build_context = |production_wiring, scheduler_wake_wiring| RebornProductionBuildContext {
        profile,
        workspace_scoped_per_caller,
        wiring_config,
        production_wiring,
        local_process_port: None,
        product_auth_ports,
        oauth_provider_configs,
        oauth_dcr_callback,
        owner_id,
        local_runtime_identity,
        process_concurrency_limits,
        resolved_memory: resolved_memory_provider,
        scheduler_wake_wiring,
        account_setup_descriptors,
        nearai_mcp_bootstrap_config,
        native_extension_factories,
        channel_extension_bindings,
        first_party_bundles,
        first_party_registrars,
        credential_account_visibility_policy,
        ironhub_manifest_url,
        workspace_filesystems: None,
        standalone_storage_root: None,
        default_system_prompt_path: None,
        #[cfg(any(test, feature = "test-support"))]
        network_http_egress_for_test,
        #[cfg(any(test, feature = "test-support"))]
        trust_fixture_extensions_for_test,
    };
    match storage {
        RebornStorageInput::Disabled => Err(RebornBuildError::InvalidConfig {
            reason: format!(
                "profile={} requires durable database-backed Reborn storage",
                profile
            ),
        }),
        RebornStorageInput::LocalFilesystem {
            root,
            workspace_root,
            host_home_root,
        } => {
            let scheduler_wake_wiring =
                ironclaw_turn_runner::runtime::SchedulerWakeWiring::channel();
            let runtime_policy_for_local_process = runtime_policy.clone();
            let production_wiring = production_wiring(
                traffic_policy,
                production_trust_policy,
                runtime_policy,
                scheduler_wake_wiring.notifier(),
                runtime_process_binding,
            )?;
            let context = build_context(production_wiring, scheduler_wake_wiring);
            build_local_storage_production_shaped(
                context,
                LocalStorageProductionInput {
                    root,
                    workspace_root,
                    host_home_root,
                    storage_backend_input: DurableStorageInput::EmbeddedLibsql,
                    process_journal_pool: None,
                    explicit_secret_master_key: None,
                    runtime_policy_for_local_process,
                    postgres_resource_governor_singleton: None,
                },
            )
            .await
        }
        RebornStorageInput::HostedSingleTenantPostgres {
            root,
            workspace_root,
            host_home_root,
            pool_source,
            secret_master_key,
            process_local_resource_governor_singleton,
        } => {
            let pools = open_postgres_pools_from_source(pool_source)?;
            let scheduler_wake_wiring =
                ironclaw_turn_runner::runtime::SchedulerWakeWiring::channel();
            let runtime_policy_for_local_process = runtime_policy.clone();
            let production_wiring = production_wiring(
                traffic_policy,
                production_trust_policy,
                runtime_policy,
                scheduler_wake_wiring.notifier(),
                runtime_process_binding,
            )?;
            let context = build_context(production_wiring, scheduler_wake_wiring);
            build_local_storage_production_shaped(
                context,
                LocalStorageProductionInput {
                    root,
                    workspace_root,
                    host_home_root,
                    storage_backend_input: DurableStorageInput::Postgres(pools.data_plane),
                    process_journal_pool: pools.process_journal,
                    explicit_secret_master_key: Some(secret_master_key),
                    runtime_policy_for_local_process,
                    postgres_resource_governor_singleton: Some(
                        process_local_resource_governor_singleton,
                    ),
                },
            )
            .await
        }
        #[cfg(any(test, feature = "test-support"))]
        RebornStorageInput::Libsql {
            database_path_or_url,
            runtime,
            secret_master_key,
            process_local_resource_governor_singleton,
        } => {
            let scheduler_wake_wiring =
                ironclaw_turn_runner::runtime::SchedulerWakeWiring::channel();
            let production_wiring = production_wiring(
                traffic_policy,
                production_trust_policy,
                runtime_policy,
                scheduler_wake_wiring.notifier(),
                runtime_process_binding,
            )?;
            let secret_master_key = resolve_secret_master_key(secret_master_key).await?;
            let context = build_context(production_wiring, scheduler_wake_wiring);
            build_libsql_production(
                context,
                runtime,
                database_path_or_url,
                secret_master_key,
                process_local_resource_governor_singleton,
            )
            .await
        }
        RebornStorageInput::Postgres {
            pool_source,
            secret_master_key,
            process_local_resource_governor_singleton,
        } => {
            let pools = open_postgres_pools_from_source(pool_source)?;
            let scheduler_wake_wiring =
                ironclaw_turn_runner::runtime::SchedulerWakeWiring::channel();
            let production_wiring = production_wiring(
                traffic_policy,
                production_trust_policy,
                runtime_policy,
                scheduler_wake_wiring.notifier(),
                runtime_process_binding,
            )?;
            let secret_master_key = resolve_secret_master_key(secret_master_key).await?;
            let context = build_context(production_wiring, scheduler_wake_wiring);
            build_postgres_production(
                context,
                pools.data_plane,
                pools.process_journal,
                secret_master_key,
                process_local_resource_governor_singleton,
            )
            .await
        }
    }
}

async fn resolve_secret_master_key(
    explicit: Option<ironclaw_secrets::SecretMaterial>,
) -> Result<ironclaw_secrets::SecretMaterial, RebornBuildError> {
    resolve_explicit_or_keychain_master_key(explicit)
        .await?
        .ok_or(RebornBuildError::MissingSecretMasterKey)
}

struct LocalStorageProductionInput {
    root: PathBuf,
    workspace_root: Option<PathBuf>,
    host_home_root: Option<PathBuf>,
    storage_backend_input: DurableStorageInput,
    /// Dedicated Postgres pool for the process journal, when the deployment has
    /// one. `None` leaves the journal on the shared data-plane handle.
    process_journal_pool: Option<deadpool_postgres::Pool>,
    explicit_secret_master_key: Option<ironclaw_secrets::SecretMaterial>,
    runtime_policy_for_local_process: Option<EffectiveRuntimePolicy>,
    postgres_resource_governor_singleton: Option<bool>,
}

async fn build_local_storage_production_shaped(
    mut context: RebornProductionBuildContext,
    input: LocalStorageProductionInput,
) -> Result<RebornRuntimeStores, RebornBuildError> {
    let LocalStorageProductionInput {
        root,
        workspace_root,
        host_home_root,
        storage_backend_input,
        process_journal_pool,
        explicit_secret_master_key,
        runtime_policy_for_local_process,
        postgres_resource_governor_singleton,
    } = input;
    let host_access = build_host_access(
        root,
        workspace_root,
        host_home_root,
        runtime_policy_for_local_process,
        // The shell must scope `/workspace` exactly as the file tools do, or one alias names two
        // directories and a file written by one is invisible to the other.
        context.workspace_scoped_per_caller,
    )?;
    let root = &host_access.storage_root;
    let workspace_root = &host_access.workspace_root;
    let host_home_root = host_access.host_home_root.as_ref();
    let owner_user_id =
        UserId::new(context.owner_id.clone()).map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
    let default_system_prompt_path = bootstrap_standalone_host(root, &owner_user_id).await?;

    let filesystem_bundle =
        build_filesystem(root, workspace_root, host_home_root, storage_backend_input).await?;
    let trigger_repository =
        trigger_repository_for_durable_backend(&filesystem_bundle.durable_backend).await?;
    let refresh_lock_pool = match &filesystem_bundle.durable_backend {
        DurableBackend::LibSql { .. } => None,
        DurableBackend::Postgres(pool) => Some(pool.clone()),
    };
    let event_store = match &filesystem_bundle.durable_backend {
        DurableBackend::LibSql { filesystem, .. } => {
            ironclaw_event_store::RebornEventStoreConfig::LibsqlFilesystem {
                filesystem: Arc::clone(filesystem),
                path_or_url: standalone_db_path(root).to_string_lossy().into_owned(),
            }
        }
        DurableBackend::Postgres(pool) => {
            ironclaw_event_store::RebornEventStoreConfig::PostgresPool {
                pool: ironclaw_filesystem::PostgresConnectionPool::new(pool.clone()),
            }
        }
    };
    let filesystem = filesystem_bundle.filesystem;
    // Skills are read only from the database now, so anything the legacy backfill (or a pre-upgrade
    // agent install) left on the host disk has to be brought across or it is silently lost.
    crate::standalone_bootstrap_assembly::import_host_disk_skills_into_database(root, &filesystem)
        .await?;
    context.workspace_filesystems = Some(host_access.build_workspace_filesystems(
        Arc::clone(&filesystem),
        context.workspace_scoped_per_caller,
    )?);
    context.local_process_port = host_access.process_port;
    context.standalone_storage_root = Some(root.clone());
    context.default_system_prompt_path = Some(default_system_prompt_path);
    let scoped_filesystem = crate::wrap_scoped(Arc::clone(&filesystem));
    let (_secret_store, crypto) = build_secret_store(
        root,
        Arc::clone(&scoped_filesystem),
        explicit_secret_master_key,
    )
    .await?;
    let secret_credentials = SecretCredentialStores::new(scoped_filesystem, crypto);
    // libSQL journals share one bounded lane (#7714); Postgres keeps its
    // existing process-journal pool and data-plane governor (#7471).
    let libsql_journal_lane = match &filesystem_bundle.durable_backend {
        DurableBackend::LibSql { runtime, .. } => Some(
            crate::filesystem_assembly::libsql_journal_lane_filesystem(runtime, |backend| {
                crate::filesystem_assembly::process_journal_root_filesystem(backend)
            })?,
        ),
        DurableBackend::Postgres(_) => None,
    };
    let resource_governor =
        filesystem_resource_governor(libsql_journal_lane.as_ref().unwrap_or(&filesystem));
    if let Some(singleton) = postgres_resource_governor_singleton {
        ensure_postgres_resource_governor_authority_for_build(singleton)?;
    }
    let process_journal_filesystem = match libsql_journal_lane {
        Some(lane) => Some(lane),
        None => process_journal_pool
            .map(|pool| {
                crate::filesystem_assembly::process_journal_root_filesystem(Arc::new(
                    ironclaw_filesystem::PostgresRootFilesystem::new(pool),
                ))
            })
            .transpose()?,
    };
    let stores = ProductionStoreBundle::with_secret_credentials(
        filesystem,
        resource_governor,
        secret_credentials,
        event_store,
    )
    .await?
    .with_process_journal_filesystem(process_journal_filesystem);
    build_backend_production(
        context,
        stores,
        trigger_repository,
        match refresh_lock_pool {
            Some(pool) => ironclaw_auth::CredentialRefreshLeaderLock::for_postgres(pool),
            None => ironclaw_auth::CredentialRefreshLeaderLock::always_leader_for_single_writer(),
        },
    )
    .await
}

pub(super) struct RebornProductionWiring {
    pub(super) trust_policy: Arc<HostTrustPolicy>,
    pub(super) runtime_policy: EffectiveRuntimePolicy,
    pub(super) turn_run_wake_notifier: Arc<dyn ironclaw_turns::TurnRunWakeNotifier>,
    pub(super) runtime_process_binding: RebornRuntimeProcessBinding,
}

pub(super) struct RebornProductionBuildContext {
    pub(super) profile: RebornCompositionProfile,
    /// The deployment's resolved workspace scoping decision. Carried, not
    /// re-derived from `profile`: the assembling host may raise it (SSO on a
    /// standalone-composed deployment), and a second derivation here would
    /// silently drop that.
    pub(super) workspace_scoped_per_caller: bool,
    pub(super) wiring_config: ironclaw_host_runtime::ProductionWiringConfig,
    pub(super) production_wiring: RebornProductionWiring,
    pub(super) local_process_port: Option<HostProcessPort>,
    pub(super) product_auth_ports: Option<RebornProductAuthServicePorts>,
    pub(super) oauth_provider_configs: Vec<crate::input::OAuthProviderBackendConfig>,
    pub(super) oauth_dcr_callback: Option<crate::input::OAuthDcrCallbackConfig>,
    pub(super) owner_id: String,
    pub(super) local_runtime_identity: Option<RebornLocalRuntimeIdentity>,
    pub(super) process_concurrency_limits: ProcessConcurrencyLimits,
    pub(super) resolved_memory: crate::ResolvedMemoryProvider,
    pub(super) scheduler_wake_wiring: ironclaw_turn_runner::runtime::SchedulerWakeWiring,
    pub(super) account_setup_descriptors: Vec<ExtensionAccountSetupDescriptor>,
    pub(super) nearai_mcp_bootstrap_config:
        Option<ironclaw_operator::llm_admin::nearai_mcp::NearAiMcpBootstrapConfig>,
    pub(super) native_extension_factories:
        Vec<Arc<dyn ironclaw_extension_host::NativeExtensionFactory>>,
    pub(super) channel_extension_bindings: Vec<crate::input::ChannelExtensionBinding>,
    pub(super) first_party_bundles: Vec<ironclaw_extension_host::FirstPartyPackageBundle>,
    pub(super) first_party_registrars:
        Vec<Arc<dyn ironclaw_extension_host::FirstPartyHandlerRegistrar>>,
    pub(super) credential_account_visibility_policy:
        Option<Arc<dyn ironclaw_auth::RuntimeCredentialAccountVisibilityPolicy>>,
    pub(super) ironhub_manifest_url: ironclaw_extension_manager::ironhub::IronhubManifestUrl,
    pub(super) workspace_filesystems: Option<WorkspaceFilesystems>,
    pub(super) standalone_storage_root: Option<PathBuf>,
    pub(super) default_system_prompt_path: Option<PathBuf>,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) network_http_egress_for_test: Option<Arc<dyn ironclaw_network::NetworkHttpEgress>>,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) trust_fixture_extensions_for_test: bool,
}

fn production_wiring(
    traffic_policy: TrafficPolicy,
    trust_policy: Option<Arc<HostTrustPolicy>>,
    runtime_policy: Option<EffectiveRuntimePolicy>,
    turn_run_wake_notifier: Arc<ironclaw_turn_runner::turn_scheduler::SchedulerTurnRunWakeNotifier>,
    runtime_process_binding: RebornRuntimeProcessBinding,
) -> Result<RebornProductionWiring, RebornBuildError> {
    let trust_policy = trust_policy.ok_or(RebornBuildError::MissingProductionTrustPolicy)?;
    if !trust_policy.has_sources() {
        return Err(RebornBuildError::EmptyProductionTrustPolicy);
    }
    let runtime_policy = runtime_policy.ok_or(RebornBuildError::MissingRuntimePolicy)?;
    if traffic_policy.requires_production_runtime_policy_preflight() {
        validate_production_runtime_policy(&runtime_policy)?;
    }
    validate_production_process_binding(&runtime_policy, &runtime_process_binding)?;
    let turn_run_wake_notifier: Arc<dyn ironclaw_turns::TurnRunWakeNotifier> =
        turn_run_wake_notifier;
    Ok(RebornProductionWiring {
        trust_policy,
        runtime_policy,
        turn_run_wake_notifier,
        runtime_process_binding,
    })
}

fn validate_production_runtime_policy(
    runtime_policy: &EffectiveRuntimePolicy,
) -> Result<(), RebornBuildError> {
    let mut issues = Vec::new();
    if let Some(reason) = local_only_runtime_policy_reason(runtime_policy) {
        issues.push(ironclaw_host_runtime::ProductionWiringIssue::new(
            ironclaw_host_runtime::ProductionWiringComponent::RuntimePolicy,
            ironclaw_host_runtime::ProductionWiringIssueKind::LocalOnlyImplementation,
            Some(reason),
        ));
    }
    if runtime_policy.process_backend == ProcessBackendKind::LocalHost {
        issues.push(ironclaw_host_runtime::ProductionWiringIssue::new(
            ironclaw_host_runtime::ProductionWiringComponent::RuntimeProcessPort,
            ironclaw_host_runtime::ProductionWiringIssueKind::LocalOnlyImplementation,
            Some("local_host_process"),
        ));
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(RebornBuildError::ProductionWiring {
            report: ironclaw_host_runtime::ProductionWiringReport::new(issues),
        })
    }
}

fn local_only_runtime_policy_reason(policy: &EffectiveRuntimePolicy) -> Option<&'static str> {
    if matches!(policy.deployment, DeploymentMode::LocalSingleUser) {
        return Some("local_single_user_deployment");
    }
    if matches!(
        policy.filesystem_backend,
        FilesystemBackendKind::HostWorkspace | FilesystemBackendKind::HostWorkspaceAndHome
    ) {
        return Some("host_workspace_filesystem");
    }
    if matches!(policy.process_backend, ProcessBackendKind::LocalHost) {
        return Some("local_host_process");
    }
    if matches!(policy.network_mode, NetworkMode::Direct) {
        return Some("direct_network");
    }
    if matches!(
        policy.secret_mode,
        SecretMode::ScrubbedEnv | SecretMode::InheritedEnv
    ) {
        return Some("local_secret_environment");
    }
    None
}

fn validate_production_process_binding(
    runtime_policy: &EffectiveRuntimePolicy,
    binding: &RebornRuntimeProcessBinding,
) -> Result<(), RebornBuildError> {
    binding
        .validate_for_production_policy(runtime_policy)
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })
}

pub(super) fn planned_run_profile_resolver()
-> Result<Arc<InMemoryRunProfileResolver>, RebornBuildError> {
    Ok(Arc::new(
        ironclaw_turn_runner::planned_driver_factory::default_planned_run_profile_resolver()
            .map_err(|error| RebornBuildError::PlannedRunProfileResolver {
                reason: error.to_string(),
            })?,
    ))
}

pub(super) type FilesystemProductionHostRuntimeServices<F> =
    HostRuntimeServices<F, FilesystemResourceGovernor<F>>;
