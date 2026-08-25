use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use ironclaw_approvals::{ApprovalRequestStore, ApprovalRequestStorePort as _};
use ironclaw_approvals::{ApprovalResolver, LeaseApproval, PersistentApprovalPolicyStore};
use ironclaw_auth::{
    AuthProductError, AuthProductScope, AuthSurface, FilesystemAuthProductServices,
    RebornAuthContinuationDispatcher, RebornProductAuthServicePorts, RebornProductAuthServices,
    RuntimeCredentialAccountRefreshService, RuntimeCredentialAccountSelectionService,
    UnavailableAuthProviderClient, map_account_error, runtime_credential_account_selection_request,
};
use ironclaw_authorization::CapabilityLeaseStore;
use ironclaw_extension_contracts::extension::ExtensionHostAssemblyConfig;
use ironclaw_extension_registry::{
    ExtensionInstallationStore, ExtensionLifecycleService, ExtensionManifest,
    ExtensionManifestRecord, ExtensionPackage, ExtensionRegistry, ManifestSource,
};
use ironclaw_filesystem::{
    Fault, FaultInjecting, InMemoryBackend, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    action::Action,
    capability::CapabilityDescriptor,
    decision::{Decision, Obligation, Obligations},
    dispatch::CredentialStageError,
    ids::{CapabilityId, VendorId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
    resource::{ResourceEstimate, ResourceScope, ResourceUsage},
    result_meta::FailureKind,
    scope::{ExecutionContext, Principal},
};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, FirstPartyCapabilityError, FirstPartyCapabilityHandler,
    FirstPartyCapabilityRegistry, FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
    HostRuntime, HostRuntimeServices, RuntimeCapabilityOutcome, RuntimeCredentialAccessSecret,
    RuntimeCredentialAccountRequest, RuntimeCredentialAccountResolver,
};
use ironclaw_processes::ProcessServices;
use ironclaw_product_contracts::lifecycle_service::{
    LifecycleProductService, LifecycleProductSurfaceContext,
};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_secrets::{SecretStore, SecretStorePort};
use ironclaw_trust::{AdminConfig, HostTrustPolicy, InvalidationBus};

use crate::extension_lifecycle_capabilities;
use crate::lifecycle_product_service::ExtensionHostLifecycleProductService;
use ironclaw_extension_host::extension_lifecycle::{
    RebornLocalExtensionManagementPort, RebornProductAuthCredentialCleanup,
};
use ironclaw_extension_host::{
    ActiveExtensionPublisher, AvailableExtensionCatalog, ExtensionLifecycleManager,
    ExtensionRemovalCleanupRegistry, ProviderInstanceReadinessInput, boot_installation_records,
    build_generic_extension_host, first_party_reserved_extension_ids, hosted_http_mcp_runtime,
    product_extension_host_api_contract_registry, provider_instance_readiness_map,
    restore_extension_lifecycle_state,
};
use ironclaw_product_contracts::lifecycle_service::LifecycleProductContext;
use ironclaw_product_contracts::package_lifecycle::{LifecyclePackageKind, LifecyclePackageRef};
use ironclaw_skills::ScopedSkillManagementPort;

/// Public web origin every lifecycle fixture reports, so device-link guidance
/// renders its setup link deterministically.
///
/// Production reads this from `IRONCLAW_REBORN_WEBUI_BASE_URL`
/// (`connect_link_base_url_from_env`). Process-global env under parallel tests
/// is not a seam a test can own, so the fixture supplies the value directly and
/// the env read stays covered by the composition call sites.
pub const LIFECYCLE_TEST_SETUP_LINK_BASE_URL: &str = "https://webui.test";

pub type TestApprovalRequestStore = ApprovalRequestStore<FaultInjecting<InMemoryBackend>>;
pub type TestCapabilityLeaseStore = CapabilityLeaseStore<FaultInjecting<InMemoryBackend>>;

pub struct ExtensionLifecycleTestServices {
    pub host_runtime: Arc<dyn HostRuntime>,
    pub product_auth: Arc<RebornProductAuthServices>,
    pub extension_management: Arc<RebornLocalExtensionManagementPort>,
    pub skill_management: Arc<ScopedSkillManagementPort>,
    pub filesystem: Arc<dyn RootFilesystem>,
    filesystem_faults: Arc<FaultInjecting<InMemoryBackend>>,
    pub lifecycle_service: Arc<ExtensionHostLifecycleProductService>,
    pub approval_requests: Arc<TestApprovalRequestStore>,
    pub capability_leases: Arc<TestCapabilityLeaseStore>,
    /// The exact trust policy instance `ActiveExtensionPublisher` publishes
    /// into. Tests read back the published `AuthorityCeiling` through
    /// `TrustPolicy::evaluate` — the real consumption seam
    /// (`ironclaw_authorization::effects_are_covered`) — instead of exposing
    /// publisher internals.
    pub trust_policy: Arc<HostTrustPolicy>,
    secret_store: Arc<dyn SecretStorePort>,
}

impl ExtensionLifecycleTestServices {
    pub fn secret_store(&self) -> Arc<dyn SecretStorePort> {
        Arc::clone(&self.secret_store)
    }

    pub fn add_filesystem_fault(&self, fault: Fault) {
        self.filesystem_faults.add_fault(fault);
    }
}

pub async fn build_lifecycle_test_services(
    owner_id: &str,
    network_http_egress: Option<Arc<dyn ironclaw_network::NetworkHttpEgress>>,
    google_oauth_configured: bool,
) -> ExtensionLifecycleTestServices {
    build_lifecycle_test_services_with_auth_provider(
        owner_id,
        network_http_egress,
        google_oauth_configured,
        Arc::new(UnavailableAuthProviderClient),
    )
    .await
}

/// Test-only construction seam for lifecycle callers that must drive a real
/// product-auth callback. Production composition supplies its own provider
/// client; the default helper remains fail-closed for tests that do not need
/// OAuth completion.
pub async fn build_lifecycle_test_services_with_auth_provider(
    owner_id: &str,
    network_http_egress: Option<Arc<dyn ironclaw_network::NetworkHttpEgress>>,
    google_oauth_configured: bool,
    auth_provider_client: Arc<dyn ironclaw_auth::AuthProviderClient>,
) -> ExtensionLifecycleTestServices {
    build_lifecycle_test_services_over_backing(
        owner_id,
        LifecycleHarnessOptions {
            network_http_egress,
            google_oauth_configured,
            auth_provider_client,
            oauth_client_profiles: Arc::new(ironclaw_auth::EmptyOAuthClientProfileRegistry),
            filesystem: Arc::new(FaultInjecting::new(InMemoryBackend::new())),
            secret_store: Arc::new(SecretStore::ephemeral()),
            extra_catalog_package: None,
            attach_generic_host: true,
        },
    )
    .await
}

/// Test-only construction seam for registration journeys that select an
/// operator-managed OAuth client profile. The production path receives this
/// registry from composition; the default test helpers remain fail-closed.
pub async fn build_lifecycle_test_services_with_oauth_client_profiles(
    owner_id: &str,
    network_http_egress: Option<Arc<dyn ironclaw_network::NetworkHttpEgress>>,
    google_oauth_configured: bool,
    oauth_client_profiles: Arc<dyn ironclaw_auth::OAuthClientProfileRegistry>,
) -> ExtensionLifecycleTestServices {
    build_lifecycle_test_services_over_backing(
        owner_id,
        LifecycleHarnessOptions {
            network_http_egress,
            google_oauth_configured,
            auth_provider_client: Arc::new(UnavailableAuthProviderClient),
            oauth_client_profiles,
            filesystem: Arc::new(FaultInjecting::new(InMemoryBackend::new())),
            secret_store: Arc::new(SecretStore::ephemeral()),
            extra_catalog_package: None,
            attach_generic_host: true,
        },
    )
    .await
}

/// Reconstructs the lifecycle assembly over the same durable test backing.
///
/// This is intentionally a narrow test seam: restart journeys need a fresh
/// host, registry, and auth service, but must retain the persisted lifecycle
/// and pending-auth records. Production composition owns its own restart path.
pub async fn rebuild_lifecycle_test_services_with_auth_provider(
    previous: &ExtensionLifecycleTestServices,
    owner_id: &str,
    network_http_egress: Option<Arc<dyn ironclaw_network::NetworkHttpEgress>>,
    google_oauth_configured: bool,
    auth_provider_client: Arc<dyn ironclaw_auth::AuthProviderClient>,
) -> ExtensionLifecycleTestServices {
    build_lifecycle_test_services_over_backing(
        owner_id,
        LifecycleHarnessOptions {
            network_http_egress,
            google_oauth_configured,
            auth_provider_client,
            oauth_client_profiles: Arc::new(ironclaw_auth::EmptyOAuthClientProfileRegistry),
            filesystem: Arc::clone(&previous.filesystem_faults),
            secret_store: Arc::clone(&previous.secret_store),
            extra_catalog_package: None,
            attach_generic_host: true,
        },
    )
    .await
}

/// Test-only construction seam for #7853 positive-coverage tests: a catalog
/// extended with [`device_link_channel_fixture_package`] (a package that
/// declares BOTH a channel and a required device-link tool credential) and no
/// generic host attached.
///
/// The skip matters. `activation_requirement_applies`
/// (`ironclaw_extension_host::extension_credential_requirements`) excludes a
/// device-link requirement from the pre-activation gate only for a package
/// that *declares a channel* — so a channel + device-link package is the only
/// shape that can reach `Active` for a caller who has not yet linked. But
/// reaching `Active` through the REAL generic host requires `build_active` to
/// bind a `ToolAdapter` and a `ChannelSurfaces` half
/// (`ironclaw_extension_host::entrypoint::check_binding`), which needs a
/// registered native channel adapter this crate-tier harness does not carry
/// (`GenericExtensionHostParams.channel_adapters` is empty in
/// `build_lifecycle_test_services_over_backing`, matching production's real
/// channel packages like `telegram`). Skipping `attach_generic_host` mirrors
/// `ironclaw_extension_host::product_lifecycle`'s own device-link fixtures
/// (`device_link_user_setup_requirement_resolves_from_the_channel_facet_alone`,
/// `fixture_extension_package`'s activation test), which never attach one
/// either: `commit_activation`'s `publish_to_generic_host` short-circuits
/// `Ok(())` when `self.generic_host.get()` is `None`, so activation reaches
/// `Active` from installation-state semantics alone, without touching bind.
/// None of the machinery under test — the pre-activation credential gate, the
/// `device_link_user_setup` notice resolution — is bypassed by this; only the
/// binding of the fixture's own (unused) tool/channel adapters is skipped.
pub async fn build_lifecycle_test_services_with_device_link_channel_fixture(
    owner_id: &str,
) -> ExtensionLifecycleTestServices {
    build_lifecycle_test_services_over_backing(
        owner_id,
        LifecycleHarnessOptions {
            network_http_egress: None,
            google_oauth_configured: false,
            auth_provider_client: Arc::new(UnavailableAuthProviderClient),
            oauth_client_profiles: Arc::new(ironclaw_auth::EmptyOAuthClientProfileRegistry),
            filesystem: Arc::new(FaultInjecting::new(InMemoryBackend::new())),
            secret_store: Arc::new(SecretStore::ephemeral()),
            extra_catalog_package: Some(device_link_channel_fixture_package()),
            attach_generic_host: false,
        },
    )
    .await
}

/// Extension id of [`device_link_channel_fixture_package`].
pub const DEVICE_LINK_CHANNEL_FIXTURE_EXTENSION_ID: &str = "fixture-device-link-channel";

/// A synthetic third-party package declaring both a channel
/// (`[channel.connection] strategy = "device_link"`) and a required
/// device-link tool credential (`[auth.fixture-device-link-channel] method =
/// "device_link"` backing `[[tools.credentials]] vendor =
/// "fixture-device-link-channel"`) — the one shape
/// `activation_requirement_applies` exempts from the pre-activation
/// credential gate, so activation can reach `Active` for a caller who has not
/// linked. Modeled on
/// `ironclaw_extension_host::test_support::DEVICE_LINK_CHANNEL_MANIFEST`
/// (`pub(crate)` to that crate, so not importable here) and on this crate's
/// own `fixture_extension_package_with_device_link_tool_credential` pattern
/// in `ironclaw_extension_host::product_lifecycle`'s test module.
fn device_link_channel_fixture_package() -> ironclaw_extension_host::AvailableExtensionPackage {
    let manifest_toml = r#"
schema_version = "reborn.extension_manifest.v3"
id = "fixture-device-link-channel"
name = "Fixture Device Link Channel"
version = "0.1.0"
description = "fixture: channel + device-link auth, for #7853 positive coverage"
trust = "third_party"

[runtime]
kind = "first_party"
service = "fixture-device-link-channel"

[[tools]]
id = "fixture-device-link-channel.whoami"
description = "Report the linked account."
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/fixture-device-link-channel/whoami.input.v1.json"

[[tools.credentials]]
handle = "fixture_device_link_session"
vendor = "fixture-device-link-channel"
scopes = ["session"]
audience = { scheme = "https", host = "api.fixture-device-link.example" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }

[channel]
id = "messages"
display_name = "Fixture device-link messages"
conversation_model = "continuous"

[channel.reply]
transport = "message"

[channel.delivery]
transport = "message"

[channel.connection]
provider = "fixture-device-link-channel"
strategy = "device_link"
instructions = "Link your fixture account."
input_placeholder = ""
submit_label = "Link"
error_message = "Linking failed."
connection_success_message = "Linked."

[channel.connection.notices]
connect_required = "Link first."
paired = "Linked."
already_paired_same_user = "Already linked."
already_bound_to_other_user = "Linked elsewhere."
expired_or_unknown = "Not linked."

[channel.ingress]
route_suffix = "events"
method = "post"
body_limit_bytes = 1048576

[channel.ingress.verification]
kind = "hmac_sha256"
secret_handle = "fixture_device_link_signing_secret"
signature_header = "X-Fixture-Signature"
signed_payload = [ { body = true } ]

[admin_configuration]
group_id = "fixture.device_link_channel"
display_name = "Fixture Device Link channel"
fields = [ { handle = "fixture_device_link_signing_secret", label = "Signing secret", secret = true } ]

[auth.fixture-device-link-channel]
method = "device_link"
display_name = "Fixture personal account"
default_mode_label = "Scan a code"
instructions = "Open the fixture app and scan the code."
"#;
    let contracts = product_extension_host_api_contract_registry().expect("host API contracts");
    let host_port_catalog =
        ironclaw_host_api::host_port::default_host_port_catalog().expect("host ports");
    let root =
        VirtualPath::new("/system/extensions/fixture-device-link-channel").expect("extension root");
    // `ExtensionManifest::parse` only understands the legacy v2 wire shape
    // (`[[host_api]] capability_provider.tools`) and rejects this fixture's
    // v3 `[[tools]]`/`[channel]`/`[auth.*]` sections outright. Parse once
    // through `ExtensionManifestRecord::from_toml` (which accepts both wire
    // versions) and derive the legacy `ExtensionManifest` view
    // `ExtensionPackage::from_manifest_toml` needs from its internal v2
    // representation — the same pattern
    // `ironclaw_extension_host::activation_credentials`'s own device-link
    // fixture helper (`package_from`) uses.
    let record = ExtensionManifestRecord::from_toml(
        manifest_toml,
        ManifestSource::HostBundled,
        &host_port_catalog,
        None,
        &contracts,
        Some(root.clone()),
    )
    .expect("device-link channel fixture manifest");
    let resolved_manifest = Arc::new(record.resolved().clone());
    let manifest =
        ExtensionManifest::try_from(record.manifest().clone()).expect("legacy package view");
    let package = ExtensionPackage::from_manifest_toml(manifest, root, manifest_toml)
        .expect("device-link channel fixture package");
    ironclaw_extension_host::AvailableExtensionPackage {
        package_ref: LifecyclePackageRef::new(
            LifecyclePackageKind::Extension,
            "fixture-device-link-channel",
        )
        .expect("fixture package ref"),
        manifest_toml: manifest_toml.to_string(),
        resolved_manifest,
        source: ManifestSource::HostBundled,
        package,
        cleanup_requirements: Vec::new(),
        surface_kinds: Vec::new(),
        channel_directions: None,
        channel_presentation: None,
        assets: vec![ironclaw_extension_host::AvailableExtensionAsset {
            path: "manifest.toml".to_string(),
            content: ironclaw_extension_host::AvailableExtensionAssetContent::Bytes(
                manifest_toml.as_bytes().to_vec(),
            ),
        }],
        onboarding_override: None,
        oauth_setup_override: None,
        search_aliases: Vec::new(),
    }
}

/// Explicit assembly knobs for [`build_lifecycle_test_services_over_backing`].
///
/// Grouped into a struct (rather than more positional parameters) so that
/// `attach_generic_host` can stay its own named field instead of being
/// derived from `extra_catalog_package`. The two are unrelated harness
/// decisions: a future caller that extends the catalog for a reason
/// unrelated to device-link coverage must still choose explicitly whether it
/// wants the real generic host wired in, rather than silently losing tool
/// binding and the snapshot tool resolver because the catalog grew.
struct LifecycleHarnessOptions {
    network_http_egress: Option<Arc<dyn ironclaw_network::NetworkHttpEgress>>,
    google_oauth_configured: bool,
    auth_provider_client: Arc<dyn ironclaw_auth::AuthProviderClient>,
    oauth_client_profiles: Arc<dyn ironclaw_auth::OAuthClientProfileRegistry>,
    filesystem: Arc<FaultInjecting<InMemoryBackend>>,
    secret_store: Arc<dyn SecretStorePort>,
    extra_catalog_package: Option<ironclaw_extension_host::AvailableExtensionPackage>,
    /// Whether to build and attach the real generic extension host (tool
    /// binder, channel adapters, snapshot tool resolver).
    ///
    /// See `build_lifecycle_test_services_with_device_link_channel_fixture`'s
    /// doc comment for the one case where this is `false`: this harness has
    /// no native channel adapter for a synthetic device-link fixture package
    /// to bind against, so `commit_activation`'s `publish_to_generic_host`
    /// must be allowed to short-circuit `Ok(())` instead.
    attach_generic_host: bool,
}

async fn build_lifecycle_test_services_over_backing(
    owner_id: &str,
    options: LifecycleHarnessOptions,
) -> ExtensionLifecycleTestServices {
    let LifecycleHarnessOptions {
        network_http_egress,
        google_oauth_configured,
        auth_provider_client,
        oauth_client_profiles,
        filesystem,
        secret_store,
        extra_catalog_package,
        attach_generic_host,
    } = options;
    let owner_user_id = ironclaw_host_api::ids::UserId::new(owner_id).expect("valid owner id");
    let extension_filesystem: Arc<dyn RootFilesystem> = filesystem.clone();
    let auth_filesystem = Arc::new(ScopedFilesystem::new(Arc::clone(&filesystem), |scope| {
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/secrets")?,
            VirtualPath::new(format!(
                "/tenants/{}/users/{}/secrets",
                scope.tenant_id.as_str(),
                scope.user_id.as_str()
            ))?,
            MountPermissions::read_write_list_delete(),
        )])
    }));
    // Product auth is needed while assembling lifecycle (credential selection
    // and cleanup), while successful auth setup must re-enter that completed
    // lifecycle facade. Match production's two adapter layers with a narrow
    // late-bound bridge so the harness retains one canonical auth bundle.
    let terminal_continuation: Arc<dyn RebornAuthContinuationDispatcher> =
        Arc::new(NoopAuthContinuationDispatcher);
    let continuation_dispatcher = Arc::new(LateBoundAuthContinuationDispatcher::default());
    let durable_auth = Arc::new(FilesystemAuthProductServices::new_with_root(
        auth_filesystem,
        Arc::clone(&filesystem),
        Arc::clone(&secret_store),
    ));
    let product_auth = RebornProductAuthServicePorts::from_shared_with_provider(
        durable_auth,
        auth_provider_client,
    )
    .into_services(continuation_dispatcher.clone(), Arc::clone(&secret_store));
    let host_scope = AuthProductScope::credential_owner(
        &webui_gate_resource_scope_for_owner(owner_id),
        AuthSurface::Api,
    );
    let product_auth = product_auth
        .with_host_managed_nearai_credential_scope(host_scope)
        .expect("host-managed NEAR AI scope is owner-granularity");
    let product_auth = Arc::new(product_auth);
    let runtime_credential_accounts = product_auth.runtime_credential_account_selection_service();
    let credential_resolver = Arc::new(TestProductAuthRuntimeCredentialResolver::new(
        Arc::clone(&runtime_credential_accounts),
        product_auth.runtime_credential_account_refresh_service(),
    ));

    let mut host_services = HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::clone(&filesystem),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(LifecycleTestGrantAuthorizer),
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

    let bundles = ironclaw_extension_host::test_support::first_party_bundles_from_inventory();
    let first_party_reserved_ids = first_party_reserved_extension_ids(&bundles);
    let mut available_extensions =
        AvailableExtensionCatalog::from_first_party_assets_with_nearai_mcp_config(None, &bundles)
            .expect("first-party extension catalog")
            .with_reserved_bundled_ids(first_party_reserved_ids.clone());
    if let Some(extra_package) = extra_catalog_package {
        available_extensions.extend(AvailableExtensionCatalog::from_packages(vec![
            extra_package,
        ]));
    }
    let extension_host_ports =
        ironclaw_host_api::host_port::default_host_port_catalog().expect("host port catalog");
    let extension_host_api_contracts =
        product_extension_host_api_contract_registry().expect("host contracts");
    let installation_store: Arc<dyn ironclaw_extension_registry::ExtensionInstallationStorePort> =
        Arc::new(
            ExtensionInstallationStore::load_at(
                Arc::clone(&extension_filesystem),
                ExtensionInstallationStore::default_state_path().expect("default state path"),
                extension_host_ports,
                extension_host_api_contracts,
            )
            .await
            .expect("extension installation store"),
        );
    // Keep lifecycle publication and capability preflight on the same shared
    // registry, exactly as production composition does. The active snapshot
    // resolver is dispatch-only; `CapabilityHost` resolves the descriptor
    // from this registry before it reaches that resolver.
    let active_registry = host_services.shared_extension_registry();
    let lifecycle_service = Arc::new(tokio::sync::Mutex::new(ExtensionLifecycleService::new(
        active_registry.snapshot_owned(),
    )));
    let trust_policy =
        Arc::new(HostTrustPolicy::new(vec![Box::new(AdminConfig::new())]).expect("trust policy"));
    let active_extensions = ActiveExtensionPublisher::new(
        Arc::clone(&active_registry),
        Arc::clone(&trust_policy),
        Arc::new(InvalidationBus::new()),
    );
    restore_extension_lifecycle_state(
        &mut available_extensions,
        &extension_filesystem,
        &installation_store,
        &lifecycle_service,
        &active_extensions,
        &owner_user_id,
    )
    .await
    .expect("extension lifecycle restore");
    let mut extension_management = ExtensionLifecycleManager::new(
        ironclaw_extension_host::ExtensionLifecycleManagerDependencies {
            filesystem: Arc::clone(&extension_filesystem),
            catalog: available_extensions,
            installation_store: Arc::clone(&installation_store),
            lifecycle_service,
            active_extensions,
            credential_cleanup: Some(Arc::new(RebornProductAuthCredentialCleanup::new(
                Arc::clone(&product_auth),
            ))),
            tenant_operator_user_id: owner_user_id,
            hosted_mcp_dependencies: ironclaw_extension_host::HostedMcpPreparationDependencies {
                runtime_ports,
                catalog_safety: ironclaw_extension_host::McpCatalogAdmissionPolicy::new(Arc::new(
                    ironclaw_safety::Sanitizer::new(),
                )),
                oauth_client_profiles,
            },
        },
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
        runtime_credential_accounts,
        Some(LIFECYCLE_TEST_SETUP_LINK_BASE_URL.to_string()),
    )
    .expect("insert lifecycle handlers");
    register_bundled_first_party_handlers_for_lifecycle_tests(&mut first_party_registry)
        .expect("insert bundled first-party handlers");
    host_services = host_services.with_first_party_capabilities(Arc::new(first_party_registry));

    // Skipped for `build_lifecycle_test_services_with_device_link_channel_fixture`
    // (see its doc comment): with no generic host attached,
    // `commit_activation`'s `publish_to_generic_host` short-circuits `Ok(())`
    // instead of binding a `ToolAdapter`/`ChannelSurfaces` this harness has no
    // native channel adapter to satisfy.
    if attach_generic_host {
        let generic =
            build_generic_extension_host(ironclaw_extension_host::GenericExtensionHostParams {
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
                linked_sessions: ironclaw_extension_host::LinkedSessionStore::unavailable(),
                linked_accounts: std::sync::Arc::new(
                    ironclaw_extension_host::UnavailableLinkedAccountResolution,
                ),
                admin_secrets: None,
            })
            .await;
        extension_management.attach_generic_host(Arc::clone(&generic.host));
        host_services.set_extension_tool_resolver(Arc::new(
            ironclaw_extension_host::SnapshotToolResolver::new(generic.host.snapshot_watch()),
        ));
    }

    let approval_mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/approvals").expect("valid approvals alias"),
        VirtualPath::new("/approvals").expect("valid approvals path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("valid approval mounts");
    let scoped_filesystem = Arc::new(ScopedFilesystem::new(Arc::clone(&filesystem), move |_| {
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

    let skill_management = ironclaw_skills::build_scoped_skill_management_port(
        ironclaw_host_api::ids::UserId::new(owner_id).expect("valid owner id"),
        Arc::clone(&filesystem),
    );
    let lifecycle_service =
        ExtensionHostLifecycleProductService::new(Arc::clone(&skill_management))
            .with_extension_management(Arc::clone(&extension_management))
            .with_runtime_credential_accounts(
                product_auth.runtime_credential_account_selection_service(),
            )
            .with_setup_link_base_url(Some(LIFECYCLE_TEST_SETUP_LINK_BASE_URL.to_string()));
    let lifecycle_service = Arc::new(lifecycle_service);
    let lifecycle_product_continuation = ironclaw_assistant::lifecycle_auth_continuation_dispatcher(
        Arc::clone(&lifecycle_service) as Arc<dyn LifecycleProductService>,
        terminal_continuation,
    );
    assert!(
        continuation_dispatcher
            .bind(lifecycle_product_continuation)
            .is_ok(),
        "lifecycle auth continuation binds once"
    );

    ExtensionLifecycleTestServices {
        host_runtime: Arc::new(host_services.host_runtime_for_local_testing()),
        product_auth,
        extension_management: Arc::clone(&extension_management),
        skill_management,
        filesystem: extension_filesystem,
        filesystem_faults: filesystem,
        lifecycle_service,
        approval_requests,
        capability_leases,
        trust_policy,
        secret_store,
    }
}

pub async fn invoke_json_with_standalone_approval(
    services: &ExtensionLifecycleTestServices,
    capability_id: &str,
    context: ExecutionContext,
    input: serde_json::Value,
) -> Result<serde_json::Value, FailureKind> {
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

pub fn lifecycle_product_context(scope: ResourceScope) -> LifecycleProductContext {
    LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
        tenant_id: scope.tenant_id,
        user_id: scope.user_id,
        agent_id: scope.agent_id,
        project_id: scope.project_id,
    })
}

pub fn webui_gate_resource_scope_for_owner(owner_id: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: ironclaw_host_api::ids::TenantId::new("reborn-cli").expect("tenant"),
        user_id: ironclaw_host_api::ids::UserId::new(owner_id).expect("user"),
        agent_id: Some(ironclaw_host_api::ids::AgentId::new("reborn-cli-agent").expect("agent")),
        project_id: None,
        mission_id: None,
        thread_id: Some(
            ironclaw_host_api::ids::ThreadId::new("80aa051d-7670-5534-a2c5-2c14339e8af7")
                .expect("thread"),
        ),
        invocation_id: ironclaw_host_api::ids::InvocationId::new(),
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
        constraints: ironclaw_host_api::capability::GrantConstraints {
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

/// Breaks the construction-order cycle without exposing a second auth service:
/// all calls fail closed until the lifecycle service is ready, then use the
/// same wrapped continuation stack as production composition.
#[derive(Default)]
struct LateBoundAuthContinuationDispatcher {
    inner: OnceLock<Arc<dyn RebornAuthContinuationDispatcher>>,
}

impl LateBoundAuthContinuationDispatcher {
    fn bind(
        &self,
        dispatcher: Arc<dyn RebornAuthContinuationDispatcher>,
    ) -> Result<(), AuthProductError> {
        self.inner
            .set(dispatcher)
            .map_err(|_| AuthProductError::LifecycleActivationFailed)
    }

    fn dispatcher(&self) -> Result<&Arc<dyn RebornAuthContinuationDispatcher>, AuthProductError> {
        self.inner
            .get()
            .ok_or(AuthProductError::LifecycleActivationFailed)
    }
}

#[async_trait]
impl RebornAuthContinuationDispatcher for LateBoundAuthContinuationDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        event: ironclaw_auth::AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        self.dispatcher()?.dispatch_auth_continuation(event).await
    }

    async fn dispatch_canceled_auth_continuation(
        &self,
        event: ironclaw_auth::AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        self.dispatcher()?
            .dispatch_canceled_auth_continuation(event)
            .await
    }
}

/// Keeps lifecycle fixtures permissive while preserving the runtime
/// obligations declared by the resolved descriptor. Production derives the
/// same obligations through `GrantAuthorizer`; this fake exists only because
/// dynamic fixture tools are not known when the test grant is constructed.
struct LifecycleTestGrantAuthorizer;

#[async_trait]
impl ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer
    for LifecycleTestGrantAuthorizer
{
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &ironclaw_trust::TrustDecision,
    ) -> Decision {
        let policy = context
            .grants
            .grants
            .iter()
            .find(|grant| grant.capability == descriptor.id)
            .map(|grant| grant.constraints.network.clone())
            .unwrap_or_default();
        let mut obligations = vec![Obligation::ApplyNetworkPolicy { policy }];
        for credential in descriptor
            .runtime_credentials
            .iter()
            .filter(|credential| credential.required)
        {
            match &credential.source {
                ironclaw_host_api::capability::RuntimeCredentialRequirementSource::SecretHandle => {
                    obligations.push(Obligation::InjectSecretOnce {
                        handle: credential.handle.clone(),
                    });
                }
                ironclaw_host_api::capability::RuntimeCredentialRequirementSource::ProductAuthAccount {
                    provider,
                    setup,
                } => obligations.push(Obligation::InjectCredentialAccountOnce {
                    handle: credential.handle.clone(),
                    provider: provider.clone(),
                    setup: setup.clone(),
                    provider_scopes: credential.provider_scopes.clone(),
                    requester_extension: descriptor.provider.clone(),
                }),
            }
        }
        Decision::Allow {
            obligations: Obligations::new(obligations)
                .expect("lifecycle test descriptor obligations are valid"),
        }
    }
}

fn register_bundled_first_party_handlers_for_lifecycle_tests(
    registry: &mut FirstPartyCapabilityRegistry,
) -> Result<(), ironclaw_host_api::error::HostApiError> {
    let handler = Arc::new(NoopFirstPartyHandler);
    registry.insert_handler(
        CapabilityId::new(ironclaw_extension_support::FIRST_PARTY_WEB_SEARCH_CAPABILITY_ID)?,
        handler.clone(),
    );
    registry.insert_handler(
        CapabilityId::new(ironclaw_extension_support::FIRST_PARTY_WEB_GET_CONTENT_CAPABILITY_ID)?,
        handler.clone(),
    );
    for package in ironclaw_extension_support::gsuite_package_specs() {
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
