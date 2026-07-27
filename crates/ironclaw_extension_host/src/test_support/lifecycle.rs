use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_approvals::{ApprovalResolver, LeaseApproval, PersistentApprovalPolicyStore};
use ironclaw_auth::{
    AuthProductError, AuthProductScope, AuthSurface, RebornAuthContinuationDispatcher,
    RebornProductAuthServices, RuntimeCredentialAccountRefreshService,
    RuntimeCredentialAccountSelectionService, map_account_error,
    runtime_credential_account_selection_request,
};
use ironclaw_authorization::CapabilityLeaseStore;
use ironclaw_extensions::{
    ExtensionInstallationStore, ExtensionLifecycleService, ExtensionRegistry,
    SharedExtensionRegistry,
};
use ironclaw_filesystem::{InMemoryBackend, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    Action, CapabilityDescriptor, CapabilityId, CredentialStageError, Decision, ExecutionContext,
    ExtensionHostAssemblyConfig, MountAlias, MountGrant, MountPermissions, MountView, Obligations,
    Principal, ResourceEstimate, ResourceScope, ResourceUsage, VendorId, VirtualPath,
};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, FirstPartyCapabilityError, FirstPartyCapabilityHandler,
    FirstPartyCapabilityRegistry, FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
    HostRuntime, HostRuntimeServices, RuntimeCapabilityOutcome, RuntimeCredentialAccessSecret,
    RuntimeCredentialAccountRequest, RuntimeCredentialAccountResolver, RuntimeFailureKind,
};
use ironclaw_processes::ProcessServices;
use ironclaw_product::LifecycleProductSurfaceContext;
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_run_state::{ApprovalRequestStore, ApprovalRequestStorePort as _};
use ironclaw_secrets::{SecretStore, SecretStorePort};
use ironclaw_trust::{AdminConfig, HostTrustPolicy, InvalidationBus};

use crate::extension_lifecycle::{
    RebornLocalExtensionManagementPort, RebornProductAuthCredentialCleanup,
};
use crate::extension_lifecycle_capabilities;
use crate::lifecycle_product_service::ExtensionHostLifecycleProductService;
use crate::{
    ActiveExtensionPublisher, AvailableExtensionCatalog, ExtensionLifecycleManager,
    ExtensionRemovalCleanupRegistry, ProviderInstanceReadinessInput, boot_installation_records,
    build_generic_extension_host, first_party_reserved_extension_ids, hosted_http_mcp_runtime,
    product_extension_host_api_contract_registry, provider_instance_readiness_map,
    restore_extension_lifecycle_state,
};
use ironclaw_skills::ScopedSkillManagementPort;

pub type TestApprovalRequestStore = ApprovalRequestStore<InMemoryBackend>;
pub type TestCapabilityLeaseStore = CapabilityLeaseStore<InMemoryBackend>;

pub struct ExtensionLifecycleTestServices {
    pub host_runtime: Arc<dyn HostRuntime>,
    pub product_auth: Arc<RebornProductAuthServices>,
    pub extension_management: Arc<RebornLocalExtensionManagementPort>,
    pub lifecycle_service: Arc<ExtensionHostLifecycleProductService>,
    pub approval_requests: Arc<TestApprovalRequestStore>,
    pub capability_leases: Arc<TestCapabilityLeaseStore>,
    secret_store: Arc<dyn SecretStorePort>,
}

impl ExtensionLifecycleTestServices {
    pub fn secret_store(&self) -> Arc<dyn SecretStorePort> {
        Arc::clone(&self.secret_store)
    }
}

pub async fn build_lifecycle_test_services(
    owner_id: &str,
    network_http_egress: Option<Arc<dyn ironclaw_network::NetworkHttpEgress>>,
    google_oauth_configured: bool,
) -> ExtensionLifecycleTestServices {
    let owner_user_id = ironclaw_host_api::UserId::new(owner_id).expect("valid owner id");
    let filesystem = Arc::new(InMemoryBackend::new());
    let extension_filesystem: Arc<dyn RootFilesystem> = filesystem.clone();
    let secret_store: Arc<dyn SecretStorePort> = Arc::new(SecretStore::ephemeral());
    let continuation_dispatcher: Arc<dyn RebornAuthContinuationDispatcher> =
        Arc::new(NoopAuthContinuationDispatcher);
    let product_auth = RebornProductAuthServices::from_shared(
        Arc::new(ironclaw_auth::InMemoryAuthProductServices::new()),
        continuation_dispatcher,
    )
    .with_secret_store(Arc::clone(&secret_store));
    let host_scope = AuthProductScope::credential_owner(
        &webui_gate_resource_scope_for_owner(owner_id),
        AuthSurface::Api,
    );
    let product_auth = product_auth
        .with_host_managed_nearai_credential_scope(host_scope)
        .expect("host-managed NEAR AI scope is owner-granularity");
    let product_auth = Arc::new(product_auth);
    let credential_resolver = Arc::new(TestProductAuthRuntimeCredentialResolver::new(
        product_auth.runtime_credential_account_selection_service(),
        product_auth.runtime_credential_account_refresh_service(),
    ));

    let mut host_services = HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::clone(&filesystem),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(AllowLifecycleDispatchAuthorizer),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("extension-lifecycle-test-v1")
            .expect("valid surface version"),
    )
    .with_trust_policy(Arc::new(
        HostTrustPolicy::new(vec![Box::new(AdminConfig::new())]).expect("trust policy"),
    ))
    .with_secret_store_dyn(Arc::clone(&secret_store))
    .with_runtime_credential_account_resolver(credential_resolver);
    host_services = match network_http_egress {
        Some(egress) => host_services
            .try_with_host_http_egress(egress)
            .expect("test HTTP egress wires"),
        None => host_services,
    };
    if let Some(runtime_http_egress) = host_services.runtime_http_egress() {
        let shared_registry = host_services.shared_extension_registry();
        host_services = host_services.with_mcp_runtime(Arc::new(hosted_http_mcp_runtime(
            shared_registry,
            runtime_http_egress,
        )));
    }
    let runtime_ports = host_services.product_auth_provider_runtime_ports();
    host_services = host_services
        .try_with_default_wasm_runtime()
        .expect("test Wasm runtime wires");

    let bundles = crate::test_support::first_party_bundles_from_inventory();
    let first_party_reserved_ids = first_party_reserved_extension_ids(&bundles);
    let available_extensions =
        AvailableExtensionCatalog::from_first_party_assets_with_nearai_mcp_config(None, &bundles)
            .expect("first-party extension catalog")
            .with_reserved_bundled_ids(first_party_reserved_ids.clone());
    let extension_host_ports =
        ironclaw_host_runtime::default_host_port_catalog().expect("host port catalog");
    let extension_host_api_contracts =
        product_extension_host_api_contract_registry().expect("host contracts");
    let installation_store: Arc<dyn ironclaw_extensions::ExtensionInstallationStorePort> = Arc::new(
        ExtensionInstallationStore::load_at(
            Arc::clone(&extension_filesystem),
            ExtensionInstallationStore::default_state_path().expect("default state path"),
            extension_host_ports,
            extension_host_api_contracts,
        )
        .await
        .expect("extension installation store"),
    );
    let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
    let lifecycle_service = Arc::new(tokio::sync::Mutex::new(ExtensionLifecycleService::new(
        active_registry.snapshot_owned(),
    )));
    let active_extensions = ActiveExtensionPublisher::new(
        Arc::clone(&active_registry),
        Arc::new(HostTrustPolicy::new(vec![Box::new(AdminConfig::new())]).expect("trust policy")),
        Arc::new(InvalidationBus::new()),
    );
    restore_extension_lifecycle_state(
        &available_extensions,
        &extension_filesystem,
        &installation_store,
        &lifecycle_service,
        &active_extensions,
        &owner_user_id,
    )
    .await
    .expect("extension lifecycle restore");
    let mut extension_management = ExtensionLifecycleManager::new(
        Arc::clone(&extension_filesystem),
        available_extensions,
        Arc::clone(&installation_store),
        lifecycle_service,
        active_extensions,
        Some(Arc::new(RebornProductAuthCredentialCleanup::new(
            Arc::clone(&product_auth),
        ))),
        owner_user_id,
    )
    .with_removal_cleanup_registry(Arc::new(ExtensionRemovalCleanupRegistry::empty()));
    if google_oauth_configured {
        extension_management = extension_management.with_provider_instance_readiness(
            provider_instance_readiness_map([ProviderInstanceReadinessInput {
                provider: VendorId::new("google").expect("google vendor id"),
                configured: true,
                remediation: "configure google oauth".to_string(),
            }]),
        );
    }
    let extension_management = Arc::new(extension_management);
    if let Some(runtime_ports) = runtime_ports.clone() {
        extension_management.attach_discovery_runtime_ports(runtime_ports);
    }

    let mut first_party_registry = ironclaw_host_runtime::builtin_first_party_handlers(Arc::new(
        ironclaw_triggers::InMemoryTriggerRepository::default(),
    ))
    .expect("builtin first-party handlers");
    let mut package =
        ironclaw_host_runtime::builtin_first_party_package().expect("builtin package");
    package = extension_lifecycle_capabilities::extend_builtin_first_party_package(package)
        .expect("extend lifecycle package");
    host_services
        .shared_extension_registry()
        .insert(package)
        .expect("insert lifecycle package");
    extension_lifecycle_capabilities::insert_handlers(
        &mut first_party_registry,
        Arc::clone(&extension_management),
        product_auth.runtime_credential_account_selection_service(),
        host_services.runtime_http_egress(),
    )
    .expect("insert lifecycle handlers");
    register_bundled_first_party_handlers_for_lifecycle_tests(&mut first_party_registry)
        .expect("insert bundled first-party handlers");
    host_services = host_services.with_first_party_capabilities(Arc::new(first_party_registry));

    let generic = build_generic_extension_host(crate::GenericExtensionHostParams {
        binder: host_services.extension_lane_tool_binder(),
        native_factories: Vec::new(),
        channel_adapters: Vec::new(),
        installation_store: Arc::clone(&installation_store),
        boot_installations: boot_installation_records(&installation_store, None)
            .await
            .expect("boot installation records"),
        governor: Arc::new(InMemoryResourceGovernor::new()),
        assembly: ExtensionHostAssemblyConfig::new(
            first_party_reserved_ids
                .iter()
                .filter_map(|id| CapabilityId::new(id).ok())
                .collect(),
            Default::default(),
            std::time::Duration::from_secs(30),
        ),
        channel_egress_transport: None,
    })
    .await;
    extension_management.attach_generic_host(Arc::clone(&generic.host));
    host_services.set_extension_tool_resolver(Arc::new(crate::SnapshotToolResolver::new(
        generic.host.snapshot_watch(),
    )));

    let approval_mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/approvals").expect("valid approvals alias"),
        VirtualPath::new("/approvals").expect("valid approvals path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("valid approval mounts");
    let scoped_filesystem = Arc::new(ScopedFilesystem::new(filesystem, move |_| {
        Ok(approval_mounts.clone())
    }));
    let approval_requests = Arc::new(ApprovalRequestStore::new(Arc::clone(&scoped_filesystem)));
    let capability_leases = Arc::new(CapabilityLeaseStore::new(Arc::clone(&scoped_filesystem)));
    let persistent_approval_policies =
        Arc::new(PersistentApprovalPolicyStore::new(scoped_filesystem));
    host_services = host_services
        .with_approval_requests(Arc::clone(&approval_requests))
        .with_capability_leases(Arc::clone(&capability_leases))
        .with_persistent_approval_policies(persistent_approval_policies);

    let skill_management = Arc::new(ScopedSkillManagementPort::new(
        ironclaw_host_api::UserId::new(owner_id).expect("valid owner id"),
        Arc::clone(&extension_filesystem),
        MountView::default(),
    ));
    let mut lifecycle_service = ExtensionHostLifecycleProductService::new(skill_management)
        .with_extension_management(Arc::clone(&extension_management));
    if let Some(runtime_http_egress) = host_services.runtime_http_egress() {
        lifecycle_service = lifecycle_service.with_runtime_http_egress(runtime_http_egress);
    }
    lifecycle_service = lifecycle_service.with_runtime_credential_accounts(
        product_auth.runtime_credential_account_selection_service(),
    );

    ExtensionLifecycleTestServices {
        host_runtime: Arc::new(host_services.host_runtime_for_local_testing()),
        product_auth,
        extension_management: Arc::clone(&extension_management),
        lifecycle_service: Arc::new(lifecycle_service),
        approval_requests,
        capability_leases,
        secret_store,
    }
}

pub async fn invoke_json_with_standalone_approval(
    services: &ExtensionLifecycleTestServices,
    capability_id: &str,
    context: ExecutionContext,
    input: serde_json::Value,
) -> Result<serde_json::Value, RuntimeFailureKind> {
    match invoke_with_standalone_approval(services, capability_id, context, input).await {
        RuntimeCapabilityOutcome::Completed(completed) => Ok(completed.output),
        RuntimeCapabilityOutcome::Failed(failure) => Err(failure.kind),
        other => panic!("unexpected runtime outcome: {other:?}"),
    }
}

pub async fn invoke_with_standalone_approval(
    services: &ExtensionLifecycleTestServices,
    capability_id: &str,
    context: ExecutionContext,
    input: serde_json::Value,
) -> RuntimeCapabilityOutcome {
    let capability = CapabilityId::new(capability_id).expect("valid capability id");
    let estimate = ResourceEstimate::default();
    let outcome = services
        .host_runtime
        .invoke_capability((
            context.clone(),
            capability.clone(),
            estimate.clone(),
            input.clone(),
        ))
        .await
        .expect("runtime invocation completes");
    match outcome {
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => {
            let approval_record = services
                .approval_requests
                .get(&context.resource_scope, gate.approval_request_id)
                .await
                .expect("approval record read")
                .expect("approval request persisted");
            let Action::Dispatch { .. } = approval_record.request.action.as_ref() else {
                panic!(
                    "unexpected standalone lifecycle approval action: {:?}",
                    approval_record.request.action
                );
            };
            let approval = one_shot_lease_approval_from_context(&context, &capability);
            ApprovalResolver::new(
                services.approval_requests.as_ref(),
                services.capability_leases.as_ref(),
            )
            .approve_dispatch(&context.resource_scope, gate.approval_request_id, approval)
            .await
            .expect("approval issues dispatch resume lease");

            services
                .host_runtime
                .resume_capability((
                    context,
                    gate.approval_request_id,
                    capability,
                    estimate,
                    input,
                ))
                .await
                .expect("approved runtime invocation resumes")
        }
        other => other,
    }
}

pub fn lifecycle_product_context(
    scope: ResourceScope,
) -> ironclaw_product::LifecycleProductContext {
    ironclaw_product::LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
        tenant_id: scope.tenant_id,
        user_id: scope.user_id,
        agent_id: scope.agent_id,
        project_id: scope.project_id,
    })
}

pub fn webui_gate_resource_scope_for_owner(owner_id: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: ironclaw_host_api::TenantId::new("reborn-cli").expect("tenant"),
        user_id: ironclaw_host_api::UserId::new(owner_id).expect("user"),
        agent_id: Some(ironclaw_host_api::AgentId::new("reborn-cli-agent").expect("agent")),
        project_id: None,
        mission_id: None,
        thread_id: Some(
            ironclaw_host_api::ThreadId::new("80aa051d-7670-5534-a2c5-2c14339e8af7")
                .expect("thread"),
        ),
        invocation_id: ironclaw_host_api::InvocationId::new(),
    }
}

fn one_shot_lease_approval_from_context(
    context: &ExecutionContext,
    capability: &CapabilityId,
) -> LeaseApproval {
    let constraints = context
        .grants
        .grants
        .iter()
        .find(|grant| &grant.capability == capability)
        .expect("matching test capability grant")
        .constraints
        .clone();
    LeaseApproval {
        issued_by: Principal::HostRuntime,
        constraints: ironclaw_host_api::GrantConstraints {
            max_invocations: Some(1),
            ..constraints
        },
    }
}

struct TestProductAuthRuntimeCredentialResolver {
    accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
    refresher: Arc<dyn RuntimeCredentialAccountRefreshService>,
}

impl std::fmt::Debug for TestProductAuthRuntimeCredentialResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TestProductAuthRuntimeCredentialResolver")
            .finish()
    }
}

impl TestProductAuthRuntimeCredentialResolver {
    fn new(
        accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
        refresher: Arc<dyn RuntimeCredentialAccountRefreshService>,
    ) -> Self {
        Self {
            accounts,
            refresher,
        }
    }
}

#[async_trait]
impl RuntimeCredentialAccountResolver for TestProductAuthRuntimeCredentialResolver {
    async fn resolve_access_secret(
        &self,
        request: RuntimeCredentialAccountRequest<'_>,
    ) -> Result<RuntimeCredentialAccessSecret, CredentialStageError> {
        let selection_request = runtime_credential_account_selection_request(
            request.scope,
            request.provider,
            request.setup.clone(),
            request.provider_scopes,
            request.requester_extension,
        )?;
        let account = self
            .accounts
            .select_unique_configured_runtime_account(selection_request.clone())
            .await
            .map_err(map_account_error)?;
        let account = self
            .refresher
            .refresh_configured_runtime_account(selection_request, account, self.accounts.as_ref())
            .await
            .map_err(map_account_error)?;
        if account.status != ironclaw_auth::CredentialAccountStatus::Configured {
            return Err(CredentialStageError::AuthRequired);
        }
        let handle = account.access_secret.ok_or(CredentialStageError::Backend)?;
        Ok(RuntimeCredentialAccessSecret {
            scope: account.scope.resource,
            handle,
        })
    }
}

struct NoopAuthContinuationDispatcher;

#[async_trait]
impl RebornAuthContinuationDispatcher for NoopAuthContinuationDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        _event: ironclaw_auth::AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        Ok(())
    }

    async fn dispatch_canceled_auth_continuation(
        &self,
        _event: ironclaw_auth::AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        Ok(())
    }
}

struct AllowLifecycleDispatchAuthorizer;

#[async_trait]
impl ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer
    for AllowLifecycleDispatchAuthorizer
{
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &ironclaw_trust::TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::empty(),
        }
    }
}

fn register_bundled_first_party_handlers_for_lifecycle_tests(
    registry: &mut FirstPartyCapabilityRegistry,
) -> Result<(), ironclaw_host_api::HostApiError> {
    let handler = Arc::new(NoopFirstPartyHandler);
    registry.insert_handler(
        CapabilityId::new(ironclaw_first_party_extensions::FIRST_PARTY_WEB_SEARCH_CAPABILITY_ID)?,
        handler.clone(),
    );
    registry.insert_handler(
        CapabilityId::new(
            ironclaw_first_party_extensions::FIRST_PARTY_WEB_GET_CONTENT_CAPABILITY_ID,
        )?,
        handler.clone(),
    );
    for package in ironclaw_first_party_extensions::gsuite_package_specs() {
        for capability in package.capabilities {
            registry.insert_handler(CapabilityId::new(capability.id)?, handler.clone());
        }
    }
    Ok(())
}

struct NoopFirstPartyHandler;

#[async_trait]
impl FirstPartyCapabilityHandler for NoopFirstPartyHandler {
    async fn dispatch(
        &self,
        _request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        Ok(FirstPartyCapabilityResult::new(
            serde_json::json!({"ok": true}),
            ResourceUsage::default(),
        ))
    }
}
