use std::collections::BTreeSet;
use std::sync::Arc;

use ironclaw_assistant::{
    ApprovalInteractionService, AuthInteractionService, ExtensionAccountSetupRegistry,
    ProjectFilesystemReader, RunDeliverySettings,
};
use ironclaw_attachments::InboundAttachmentLander;
use ironclaw_auth::product_prompt::{AuthChallengeProvider, BlockedAuthFlowCanceller};
use ironclaw_extension_contracts::extension::ExtensionHostAssemblyConfig;
use ironclaw_extension_registry::ExtensionInstallationStorePort;
use ironclaw_filesystem::{CompositeRootFilesystem, RootFilesystem};
use ironclaw_host_api::{
    ids::{CapabilityId, UserId},
    resource::ResourceScope,
};
use ironclaw_host_runtime::{ExtensionLaneToolBinder, HostRuntimeHttpEgressPort};
use ironclaw_product_contracts::account_setup::ExtensionAccountSetupDescriptor;
use ironclaw_product_contracts::prompt_source::{
    ApprovalPromptContextSource, BlockedAuthPromptSource,
};
use ironclaw_resources::ResourceGovernor;
use ironclaw_threads::{SessionThreadService, ThreadScope};
use ironclaw_turns::TurnCoordinator;

use crate::RebornBuildError;
use crate::factory::RebornRuntimeStores;
use crate::input::ChannelExtensionBinding;
use crate::outbound::MutableOutboundDeliveryTargetRegistry;
use ironclaw_product_contracts::account_setup::AccountConnectionStatusSource;
use ironclaw_product_contracts::admin_users::AdminUserService;
use ironclaw_product_contracts::delivery::ChannelDeliveryResolver;

pub(crate) struct BackendExtensionHostAssemblyInput {
    pub(crate) binder: ExtensionLaneToolBinder,
    pub(crate) native_factories: Vec<Arc<dyn ironclaw_extension_host::NativeExtensionFactory>>,
    pub(crate) channel_bindings: Vec<ChannelExtensionBinding>,
    pub(crate) installation_store: Arc<dyn ExtensionInstallationStorePort>,
    pub(crate) admin_configuration_resolver: Arc<ironclaw_extension_host::ChannelConfigService>,
    pub(crate) resource_governor: Arc<dyn ResourceGovernor>,
    pub(crate) reserved_capability_ids: BTreeSet<CapabilityId>,
    pub(crate) host_runtime_http_egress: Option<HostRuntimeHttpEgressPort>,
    pub(crate) channel_egress_scope: ResourceScope,
    pub(crate) deployment_channels: Arc<ironclaw_extension_host::DeploymentChannelRegistry>,
    pub(crate) filesystem: Arc<CompositeRootFilesystem>,
    pub(crate) outbound_state: Arc<dyn ironclaw_outbound::OutboundStateStorePort>,
    /// Host-owned per-user delivery registrations, handed to the coordinator
    /// so a channel with zero of them resolves to "no target" before any
    /// adapter call (design §8).
    pub(crate) delivery_registrations:
        Arc<dyn ironclaw_product_contracts::delivery::DeliveryRegistrationService>,
}

pub(crate) struct BackendExtensionHostAssembly {
    pub(crate) generic_host: Arc<ironclaw_extension_host::ExtensionHost>,
    pub(crate) resolver: Arc<ironclaw_extension_host::SnapshotToolResolver>,
    pub(crate) ingress: ironclaw_extension_host::extension_ingress::ExtensionIngressParts,
    pub(crate) installation_store: Arc<dyn ExtensionInstallationStorePort>,
    pub(crate) delivery_coordinator: Option<Arc<ironclaw_assistant::DeliveryCoordinator>>,
    pub(crate) channel_delivery_resolver: Option<Arc<dyn ChannelDeliveryResolver>>,
    #[cfg(feature = "test-support")]
    pub(crate) channel_egress_credential_bridges:
        Arc<ironclaw_extension_host::channel_egress::BridgedChannelEgressCredentials>,
}

pub(crate) async fn build_backend_extension_host(
    input: BackendExtensionHostAssemblyInput,
) -> Result<BackendExtensionHostAssembly, RebornBuildError> {
    let BackendExtensionHostAssemblyInput {
        binder,
        native_factories,
        channel_bindings,
        installation_store,
        admin_configuration_resolver,
        delivery_registrations,
        resource_governor,
        reserved_capability_ids,
        host_runtime_http_egress,
        channel_egress_scope,
        deployment_channels,
        filesystem,
        outbound_state,
    } = input;

    let channel_egress_credentials = Arc::new(
        ironclaw_extension_host::channel_egress::ChannelConfigEgressCredentials::new(Arc::clone(
            &admin_configuration_resolver,
        )),
    );
    #[cfg(feature = "test-support")]
    let channel_egress_credentials = Arc::new(
        ironclaw_extension_host::channel_egress::BridgedChannelEgressCredentials::new(
            channel_egress_credentials,
        ),
    );
    #[cfg(feature = "test-support")]
    let channel_egress_credential_bridges = Arc::clone(&channel_egress_credentials);

    let channel_egress_transport = host_runtime_http_egress.map(|port| {
        Arc::new(
            ironclaw_extension_host::channel_egress::HostRuntimeChannelEgressTransport::new(
                port,
                channel_egress_credentials,
                channel_egress_scope.clone(),
            ),
        ) as Arc<dyn ironclaw_extension_host::egress::ChannelEgressTransport>
    });
    let boot_installations = ironclaw_extension_host::boot_installation_records(
        &installation_store,
        Some(&admin_configuration_resolver),
    )
    .await
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("extension boot installation records could not be built: {error}"),
    })?;
    let generic = ironclaw_extension_host::build_generic_extension_host(
        ironclaw_extension_host::GenericExtensionHostParams {
            binder,
            native_factories,
            channel_adapters: channel_bindings
                .iter()
                .map(|binding| (binding.extension_id.clone(), binding.surfaces.clone()))
                .collect(),
            installation_store: Arc::clone(&installation_store),
            boot_installations,
            governor: resource_governor,
            assembly: ExtensionHostAssemblyConfig::new(
                reserved_capability_ids,
                ironclaw_extension_host::extension_ingress::reserved_fixed_ingress_routes(),
                std::time::Duration::from_secs(30),
            ),
            channel_egress_transport: channel_egress_transport.clone(),
        },
    )
    .await;
    let ingress_filesystem: Arc<dyn RootFilesystem> = filesystem;
    let ingress = ironclaw_extension_host::extension_ingress::build_extension_ingress(
        generic.host.snapshot_watch(),
        Arc::clone(&deployment_channels),
        Arc::new(ironclaw_extension_host::FilesystemReplyContextStore::new(
            Arc::clone(&ingress_filesystem),
            channel_egress_scope.tenant_id.clone(),
            channel_egress_scope.user_id.clone(),
        )),
        Arc::new(
            ironclaw_extension_host::FilesystemInboundBatchStore::new(
                ingress_filesystem,
                channel_egress_scope.tenant_id.clone(),
                channel_egress_scope.user_id.clone(),
            )
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("inbound batch store could not be configured: {error}"),
            })?,
        ),
        channel_egress_transport.clone(),
    );
    let (delivery_coordinator, channel_delivery_resolver) = match channel_egress_transport {
        Some(transport) => {
            let resolver: Arc<dyn ChannelDeliveryResolver> = Arc::new(
                ironclaw_extension_host::SnapshotChannelDeliveryResolver::new(
                    generic.host.snapshot_watch(),
                    transport,
                )
                .with_deployment_channels(deployment_channels),
            );
            let coordinator = Arc::new(ironclaw_assistant::DeliveryCoordinator::new(
                outbound_state,
                Arc::clone(&resolver),
                Arc::new(ironclaw_extension_host::IngressReplyContextSource::new(
                    Arc::clone(&ingress.reply_context),
                )),
                Arc::clone(&delivery_registrations),
                ironclaw_assistant::DeliveryRetryPolicy::default(),
            ));
            (Some(coordinator), Some(resolver))
        }
        None => (None, None),
    };

    Ok(BackendExtensionHostAssembly {
        generic_host: generic.host,
        resolver: generic.resolver,
        ingress,
        installation_store,
        delivery_coordinator,
        channel_delivery_resolver,
        #[cfg(feature = "test-support")]
        channel_egress_credential_bridges,
    })
}

pub(crate) struct BackendChannelPairingAssemblyInput {
    pub(crate) descriptors: Vec<ExtensionAccountSetupDescriptor>,
    pub(crate) account_setups: ExtensionAccountSetupRegistry,
    pub(crate) filesystem: Arc<dyn RootFilesystem>,
    pub(crate) scope: ResourceScope,
    pub(crate) installation_store: Arc<dyn ExtensionInstallationStorePort>,
    pub(crate) admin_configuration_resolver: Arc<ironclaw_extension_host::ChannelConfigService>,
    pub(crate) continuation: Arc<dyn ironclaw_auth::RebornAuthContinuationDispatcher>,
    pub(crate) identity_store: Arc<ironclaw_extension_host::FilesystemChannelIdentityStore>,
    pub(crate) dm_targets: Arc<ironclaw_extension_host::FilesystemChannelDmTargetStore>,
    pub(crate) credential_cleanup:
        Arc<dyn ironclaw_extension_host::channel_connection::ChannelCredentialCleanup>,
    pub(crate) account_status_reader:
        Arc<dyn ironclaw_extension_host::channel_connection::ChannelAccountStatusReader>,
    pub(crate) disconnect_slot:
        Arc<std::sync::OnceLock<Arc<dyn ironclaw_auth::ChannelConnectionService>>>,
}

pub(crate) async fn build_backend_channel_pairing(
    input: BackendChannelPairingAssemblyInput,
) -> Result<Arc<ironclaw_extension_host::channel_pairing::ChannelPairingRegistry>, RebornBuildError>
{
    use ironclaw_extension_host::channel_host::default_channel_workflow_storage_roots;
    use ironclaw_extension_host::channel_pairing::{
        ChannelPairingRegistry, ChannelPairingService, ChannelPairingServiceParts,
        FilesystemChannelPairingStore,
    };

    let BackendChannelPairingAssemblyInput {
        descriptors,
        account_setups,
        filesystem,
        scope,
        installation_store,
        admin_configuration_resolver,
        continuation,
        identity_store,
        dm_targets,
        credential_cleanup,
        account_status_reader,
        disconnect_slot,
    } = input;
    let registry = Arc::new(ChannelPairingRegistry::default());

    for descriptor in &descriptors {
        if !account_setups.declare(descriptor.clone()) {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!(
                    "duplicate account-setup descriptor for extension `{}`",
                    descriptor.extension_id.as_str()
                ),
            });
        }
        if descriptor.connection_requirement.strategy
            != ironclaw_assistant::RebornChannelConnectStrategy::WebGeneratedCode
        {
            continue;
        }

        let extension_id = descriptor.extension_id.clone();
        let pairing_store = Arc::new(FilesystemChannelPairingStore::new(
            Arc::clone(&filesystem),
            scope.tenant_id.clone(),
            scope.user_id.clone(),
            extension_id.clone(),
        ));
        let installation = Arc::new(
            ironclaw_extension_host::channel_pairing::StoredPairingInstallationSource::new(
                Arc::clone(&installation_store),
                extension_id.clone(),
            ),
        );
        let template_values = Arc::new(
            ironclaw_extension_host::channel_pairing::ChannelConfigPairingTemplateValues::new(
                Arc::clone(&admin_configuration_resolver),
                extension_id.clone(),
                descriptor.pairing_deep_link_template.as_deref(),
            ),
        );
        let workflow_roots =
            default_channel_workflow_storage_roots(&scope.tenant_id, extension_id.as_str())
                .map_err(|reason| RebornBuildError::InvalidConfig { reason })?;
        // Same roots, same mount aliases, same construction the channel
        // workflow factory uses (§12.11 D-A) — the pairing lane must bind
        // actors into the SAME durable conversation store the workflow
        // resolves through, so both go through product's one builder.
        let pairing_conversations = ironclaw_assistant::channel_conversation_services(
            &filesystem,
            &workflow_roots,
            scope.clone(),
        )
        .await
        .map_err(|reason| RebornBuildError::InvalidConfig { reason })?;
        let agent_id = match scope.agent_id.clone() {
            Some(agent_id) => agent_id,
            None => ironclaw_host_api::ids::AgentId::new("reborn").map_err(|error| {
                RebornBuildError::InvalidConfig {
                    reason: format!("fallback channel pairing agent id is invalid: {error}"),
                }
            })?,
        };
        let service = Arc::new(ChannelPairingService::new(ChannelPairingServiceParts {
            tenant_id: scope.tenant_id.clone(),
            agent_id,
            project_id: scope.project_id.clone(),
            extension_id: descriptor.extension_id.clone(),
            connection_notices: descriptor.connection_notices.clone(),
            connection_requirement: descriptor.connection_requirement.clone(),
            deep_link_template: descriptor.pairing_deep_link_template.clone(),
            inbound_code_prefixes: descriptor.inbound_code_prefixes.clone(),
            store: pairing_store,
            installation,
            template_values,
            identity_bind: Arc::clone(&identity_store)
                as Arc<dyn ironclaw_host_api::user_identity::RebornUserIdentityBindingStore>,
            identity_lookup: Arc::clone(&identity_store)
                as Arc<dyn ironclaw_host_api::user_identity::RebornUserIdentityLookup>,
            identity_delete: Arc::clone(&identity_store)
                as Arc<dyn ironclaw_host_api::user_identity::RebornUserIdentityBindingDeleteStore>,
            continuation: Arc::clone(&continuation),
            conversation_actor_pairings: Arc::clone(&pairing_conversations)
                as Arc<dyn ironclaw_conversations::ConversationActorPairingService>,
            dm_targets: Arc::clone(&dm_targets),
        }));
        if !account_setups.connect(
            &descriptor.extension_id,
            Arc::clone(&service) as Arc<dyn AccountConnectionStatusSource>,
        ) {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!(
                    "account-setup status source for `{}` was already connected",
                    descriptor.extension_id.as_str()
                ),
            });
        }
        registry.register(service);
    }

    let _ = disconnect_slot.set(Arc::new(
        ironclaw_extension_host::channel_connection::GenericChannelConnectionService::new(
            scope.tenant_id,
            Vec::new(),
            Some(installation_store),
            Arc::clone(&identity_store)
                as Arc<dyn ironclaw_host_api::user_identity::RebornUserIdentityLookup>,
            identity_store
                as Arc<dyn ironclaw_host_api::user_identity::RebornUserIdentityBindingDeleteStore>,
            Some(credential_cleanup),
            Some(account_status_reader),
            Some(dm_targets),
            Some(Arc::clone(&registry)),
        ),
    ));

    Ok(registry)
}

pub(crate) struct ChannelHostAssemblyWiring {
    pub(crate) thread_service: Arc<dyn SessionThreadService>,
    pub(crate) turn_coordinator: Arc<dyn TurnCoordinator>,
    pub(crate) input_enqueue: Arc<dyn ironclaw_loop_host::HostInputEnqueuePort>,
    pub(crate) llm_config: Option<Arc<ironclaw_operator::RebornLlmConfigService>>,
    pub(crate) approval_interaction: Option<Arc<dyn ApprovalInteractionService>>,
    pub(crate) auth_interaction: Option<Arc<dyn AuthInteractionService>>,
    pub(crate) identity: ironclaw_extension_host::channel_host::ChannelHostIdentity,
    pub(crate) approval_context: Option<Arc<dyn ApprovalPromptContextSource>>,
    pub(crate) blocked_auth_prompts: Option<Arc<dyn BlockedAuthPromptSource>>,
    pub(crate) auth_flow_cancel: Option<Arc<dyn BlockedAuthFlowCanceller>>,
    pub(crate) run_delivery_settings: RunDeliverySettings,
    pub(crate) admin_users: Arc<dyn AdminUserService>,
}

pub(crate) struct RuntimeExtensionHostAssemblyWiring<'a> {
    pub(crate) thread_service: Arc<dyn SessionThreadService>,
    pub(crate) turn_coordinator: Arc<dyn TurnCoordinator>,
    pub(crate) input_enqueue: Arc<dyn ironclaw_loop_host::HostInputEnqueuePort>,
    pub(crate) llm_config: Option<Arc<ironclaw_operator::RebornLlmConfigService>>,
    pub(crate) approval_interaction: Arc<dyn ApprovalInteractionService>,
    pub(crate) auth_interaction: Arc<dyn AuthInteractionService>,
    pub(crate) thread_scope: &'a ThreadScope,
    pub(crate) actor_user_id: UserId,
    pub(crate) auth_challenges: Option<Arc<dyn AuthChallengeProvider>>,
    pub(crate) outbound_delivery_targets: Option<&'a Arc<MutableOutboundDeliveryTargetRegistry>>,
    pub(crate) local_runtime: Option<&'a RebornRuntimeStores>,
}

pub(crate) struct ChannelHostAssemblySource {
    pub(crate) generic_host: Arc<ironclaw_extension_host::ExtensionHost>,
    pub(crate) ingress_registry:
        Arc<ironclaw_extension_host::extension_ingress::ExtensionIngressRegistry>,
    pub(crate) workflow_filesystem: Arc<dyn RootFilesystem>,
    pub(crate) inbound_attachments: Arc<dyn InboundAttachmentLander>,
    pub(crate) project_filesystem: Arc<dyn ProjectFilesystemReader>,
    pub(crate) delivery_coordinator: Option<Arc<ironclaw_assistant::DeliveryCoordinator>>,
    pub(crate) outbound_state: Arc<dyn ironclaw_outbound::OutboundStateStorePort>,
    pub(crate) delivered_gate_routes: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore>,
    pub(crate) outbound_preferences: Arc<dyn ironclaw_outbound::CommunicationPreferenceRepository>,
    /// Durable outcome record for proactive (trigger-fired) deliveries; the
    /// workflow factory wires it into every per-extension triggered driver.
    pub(crate) triggered_delivery_store: Arc<dyn ironclaw_outbound::TriggeredRunDeliveryStore>,
    /// The owner-scoped outbound target catalog the background-run notifier
    /// resolves stored notification-channel ids through at fire time.
    pub(crate) outbound_delivery_targets:
        Arc<dyn ironclaw_outbound::OutboundDeliveryTargetProvider>,
    pub(crate) identity_lookup: Arc<dyn ironclaw_host_api::user_identity::RebornUserIdentityLookup>,
    pub(crate) dm_targets:
        Arc<ironclaw_extension_host::channel_dm_targets::FilesystemChannelDmTargetStore>,
    pub(crate) deployment_channels: Arc<ironclaw_extension_host::DeploymentChannelRegistry>,
    pub(crate) channel_config: Arc<ironclaw_extension_host::ChannelConfigService>,
    pub(crate) channel_pairing:
        Option<Arc<ironclaw_extension_host::channel_pairing::ChannelPairingRegistry>>,
}

fn channel_host_source(services: &RebornRuntimeStores) -> Option<ChannelHostAssemblySource> {
    // Lander and reader share ONE handle so an inbound attachment is read back
    // from the subtree it landed in. Under a per-caller workspace policy that
    // subtree is the caller's own; the shared read-only `workspace_filesystem`
    // would address the root instead. Mirrors the test-support wiring in
    // `RebornRuntime::start_channel_host_assembly_for_test`.
    let inbound_filesystem = crate::runtime_mounts::read_write_workspace_filesystem(
        &services.extension_filesystem,
        &services.workspace_mounts,
    )?;
    let inbound_attachments: Arc<dyn InboundAttachmentLander> = Arc::new(
        ironclaw_attachments::ProjectScopedAttachmentLander::new(Arc::clone(&inbound_filesystem)),
    );
    let project_filesystem: Arc<dyn ProjectFilesystemReader> = Arc::new(
        ironclaw_assistant::ProjectScopedFilesystemReader::with_max_read_bytes(
            inbound_filesystem,
            ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes as u64,
        ),
    );
    Some(ChannelHostAssemblySource {
        generic_host: services.extension_management.generic_host()?,
        ingress_registry: Arc::clone(&services.extension_ingress.as_ref()?.registry),
        workflow_filesystem: services.extension_filesystem.clone(),
        inbound_attachments,
        project_filesystem,
        delivery_coordinator: services.delivery_coordinator.clone(),
        outbound_state: Arc::clone(&services.outbound_state),
        delivered_gate_routes: Arc::clone(&services.delivered_gate_routes),
        outbound_preferences: Arc::clone(&services.outbound_preferences),
        triggered_delivery_store: Arc::clone(&services.triggered_run_delivery),
        outbound_delivery_targets: Arc::clone(&services.outbound_delivery_targets)
            as Arc<dyn ironclaw_outbound::OutboundDeliveryTargetProvider>,
        identity_lookup: Arc::clone(&services.channel_identity_store)
            as Arc<dyn ironclaw_host_api::user_identity::RebornUserIdentityLookup>,
        dm_targets: Arc::clone(&services.channel_dm_target_store),
        deployment_channels: Arc::clone(&services.deployment_channels),
        channel_config: Arc::clone(&services.channel_config_service),
        channel_pairing: services.channel_pairing.clone(),
    })
}

pub(crate) fn channel_admin_users(
    services: &RebornRuntimeStores,
    identity: &ironclaw_extension_host::channel_host::ChannelHostIdentity,
) -> Arc<dyn AdminUserService> {
    let directory: Arc<dyn ironclaw_identity::RebornUserDirectory> =
        crate::factory::filesystem_reborn_identity_store(
            Arc::clone(&services.scoped_filesystem),
            identity.tenant_id.clone(),
            identity.operator_user_id.clone(),
            identity.agent_id.clone(),
            identity.project_id.clone(),
        );
    Arc::new(ironclaw_assistant::RebornAdminUserDirectory::new(
        directory,
        Arc::clone(&services.admin_secret_provisioner),
        Arc::new(ironclaw_assistant::RejectingAdminApiTokenMinter),
    ))
}

/// The started channel host plus the product-side workflow factory it was
/// built over. The factory is returned rather than dropped because the generic
/// triggered-delivery hook needs the SAME one to build its per-extension
/// drivers (§12.11 D-A) — two factories would mean two ledgers and two
/// conversation stores at the same roots.
pub(crate) struct StartedChannelHost {
    pub(crate) assembly: Arc<ironclaw_extension_host::channel_host::GenericChannelHostAssembly>,
    pub(crate) workflow_factory: Arc<ironclaw_assistant::RebornChannelWorkflowFactory>,
}

pub(crate) fn start_channel_host(
    source: &ChannelHostAssemblySource,
    wiring: ChannelHostAssemblyWiring,
) -> StartedChannelHost {
    use ironclaw_extension_host::channel_host::{
        GenericChannelHostAssembly, GenericChannelHostDeps,
    };

    let ChannelHostAssemblyWiring {
        thread_service,
        turn_coordinator,
        input_enqueue,
        llm_config,
        approval_interaction,
        auth_interaction,
        identity,
        approval_context,
        blocked_auth_prompts,
        auth_flow_cancel,
        run_delivery_settings,
        admin_users,
    } = wiring;
    let ChannelHostAssemblySource {
        generic_host,
        ingress_registry: registry,
        workflow_filesystem,
        inbound_attachments,
        project_filesystem,
        delivery_coordinator,
        outbound_state,
        delivered_gate_routes,
        outbound_preferences,
        triggered_delivery_store,
        outbound_delivery_targets,
        identity_lookup,
        dm_targets,
        deployment_channels,
        channel_config,
        channel_pairing,
    } = source;
    let delivery = delivery_coordinator.clone().map(|coordinator| {
        ironclaw_assistant::ChannelWorkflowDeliveryServices {
            coordinator,
            outbound_store: Arc::clone(outbound_state),
            route_store: Arc::clone(delivered_gate_routes),
            communication_preferences: Arc::clone(outbound_preferences),
            delivery_targets: Arc::clone(outbound_delivery_targets),
            project_filesystem: Arc::clone(project_filesystem),
            approval_context,
            blocked_auth_prompts,
            auth_flow_cancel,
            settings: run_delivery_settings,
            triggered_delivery_store: Arc::clone(triggered_delivery_store),
        }
    });
    let workflow_factory = Arc::new(ironclaw_assistant::RebornChannelWorkflowFactory::new(
        ironclaw_assistant::RebornChannelWorkflowServices {
            filesystem: Arc::clone(workflow_filesystem),
            thread_service,
            turn_coordinator,
            inbound_attachments: Arc::clone(inbound_attachments),
            input_enqueue,
            llm_config: llm_config.map(|service| service as _),
            approval_interaction,
            auth_interaction,
            identity: ironclaw_assistant::ChannelWorkflowIdentity {
                tenant_id: identity.tenant_id.clone(),
                agent_id: identity.agent_id.clone(),
                project_id: identity.project_id.clone(),
                operator_user_id: identity.operator_user_id.clone(),
            },
            delivery,
        },
    ));
    let identity_lookup = Some(Arc::clone(identity_lookup));

    let assembly = GenericChannelHostAssembly::start(GenericChannelHostDeps {
        watch: generic_host.snapshot_watch(),
        deployment_channels: Arc::clone(deployment_channels),
        registry: Arc::clone(registry),
        channel_config: Arc::clone(channel_config),
        channel_workflow: Arc::clone(&workflow_factory)
            as Arc<dyn ironclaw_product_contracts::channel_workflow::ChannelWorkflowFactory>,
        identity,
        identity_lookup,
        dm_targets: Some(Arc::clone(dm_targets)),
        channel_pairing: channel_pairing.clone(),
        admin_users,
    });
    StartedChannelHost {
        assembly,
        workflow_factory,
    }
}

pub(crate) async fn build_runtime_channel_host(
    services: &RebornRuntimeStores,
    wiring: RuntimeExtensionHostAssemblyWiring<'_>,
) -> Option<StartedChannelHost> {
    let RuntimeExtensionHostAssemblyWiring {
        thread_service,
        turn_coordinator,
        approval_interaction,
        auth_interaction,
        input_enqueue,
        llm_config,
        thread_scope,
        actor_user_id,
        auth_challenges,
        outbound_delivery_targets,
        local_runtime,
    } = wiring;
    let source = channel_host_source(services)?;
    let approval_context = Some(Arc::new(
        ironclaw_extension_host::run_delivery_ports::ProjectionApprovalPromptContextSource::new(
            Arc::clone(&services.approval_requests)
                as Arc<dyn ironclaw_approvals::ApprovalRequestStorePort>,
        ),
    ) as Arc<dyn ApprovalPromptContextSource>);
    let blocked_auth_prompts = Some(Arc::new(
        ironclaw_extension_host::run_delivery_ports::ProductAuthBlockedAuthPromptSource::new(
            auth_challenges.clone(),
        ),
    ) as Arc<dyn BlockedAuthPromptSource>);
    let auth_flow_cancel = crate::runtime::blocked_auth_flow_canceller(&services.product_auth);
    let identity = ironclaw_extension_host::channel_host::ChannelHostIdentity {
        tenant_id: thread_scope.tenant_id.clone(),
        agent_id: thread_scope.agent_id.clone(),
        project_id: thread_scope.project_id.clone(),
        operator_user_id: actor_user_id,
    };
    let admin_users = channel_admin_users(services, &identity);
    let started = start_channel_host(
        &source,
        ChannelHostAssemblyWiring {
            thread_service,
            turn_coordinator,
            input_enqueue,
            llm_config,
            approval_interaction: Some(approval_interaction),
            auth_interaction: Some(auth_interaction),
            identity,
            approval_context,
            blocked_auth_prompts,
            auth_flow_cancel,
            run_delivery_settings: ironclaw_assistant::triggered_run_delivery_settings(),
            admin_users,
        },
    );
    let assembly = Arc::clone(&started.assembly);

    for binding in &services.channel_extension_bindings {
        assembly
            .register_extras(
                &binding.extension_id,
                ironclaw_extension_host::channel_host::ChannelExtras {
                    preference_target_codec: binding.preference_target_codec.clone(),
                    shared_admission: None,
                    storage_roots: None,
                },
            )
            .await;
    }

    if let (Some(registry), Some(local_runtime)) = (outbound_delivery_targets, local_runtime) {
        ironclaw_extension_host::channel_outbound_targets::register_generic_channel_outbound_targets(
                registry,
                ironclaw_extension_host::channel_outbound_targets::GenericChannelOutboundTargetDeps {
                    watch: assembly.snapshot_watch(),
                    assembly: Arc::clone(&assembly),
                    channel_config: Arc::clone(&local_runtime.channel_config_service),
                    dm_targets: local_runtime.channel_dm_target_store.clone(),
                    identity: ironclaw_extension_host::channel_outbound_targets::ChannelOutboundTargetIdentity {
                        tenant_id: thread_scope.tenant_id.clone(),
                        agent_id: thread_scope.agent_id.clone(),
                        project_id: thread_scope.project_id.clone(),
                    },
                },
            );
    }

    Some(started)
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn start_channel_host_from_stores(
    services: &RebornRuntimeStores,
    wiring: ChannelHostAssemblyWiring,
) -> Option<Arc<ironclaw_extension_host::channel_host::GenericChannelHostAssembly>> {
    let source = channel_host_source(services)?;
    Some(start_channel_host(&source, wiring).assembly)
}
