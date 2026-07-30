// arch-exempt: large_file, shared extension removal convergence and compatibility tests, plan #6329
use std::{
    collections::BTreeSet,
    sync::{Arc, Weak},
};

use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, SecretCleanupAction, SecretCleanupReport,
    SecretCleanupRequest,
};
use ironclaw_extensions::{
    CapabilityVisibility, ExtensionError, ExtensionInstallation, ExtensionInstallationError,
    ExtensionInstallationId, ExtensionLifecycleService, ExtensionManifestRecord, ExtensionPackage,
    InstallationOwner, MembershipDeactivation, canonicalize_installation_rows,
};
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    CapabilitySurfaceKind, ExtensionId, InstallationState, ProductSurfaceCaller,
    ProductSurfaceError, ResourceScope, RuntimeCredentialAuthRequirement, RuntimeHttpEgress,
    UserId, VendorId, VirtualPath,
};
use ironclaw_product::{
    ChannelConnectionService, ExtensionAccountSetupDescriptor, ExtensionAccountSetupError,
    ExtensionAccountSetupRegistry, LifecycleBlockerRef, LifecycleExtensionSummary,
    LifecycleInstalledExtensionSummary, LifecyclePackageKind, LifecyclePackageRef,
    LifecycleProductPayload, LifecycleProductResponse, LifecycleReadinessBlocker,
    LifecycleSearchExtensionSummary, ProductSurfaceFailure, RebornChannelConnectStrategy,
};
use tokio::sync::{Mutex, RwLock, Semaphore};

fn unzip_extension_bundle_for_product(
    bundle: &[u8],
) -> Result<Vec<(String, Vec<u8>)>, ProductSurfaceFailure> {
    crate::unzip_extension_bundle(bundle).map_err(|error| {
        ProductSurfaceFailure::InvalidBindingRequest {
            reason: error.reason().to_string(),
        }
    })
}

/// Narrow lifecycle-cleanup port over product-auth so extension removal can
/// revoke the removed extension's exclusively-owned reusable credential without
/// depending on the whole product-auth bundle (and so tests can record the
/// issued cleanup). Production forwards to the guardrail-sanctioned
/// the product-auth cleanup service. This is the
/// single convergence point for both removal entrypoints (the WebUI service and
/// the `builtin.extension_remove` agent capability), so revocation cannot be
/// bypassed through one door.
#[async_trait]
pub trait ExtensionCredentialCleanup: Send + Sync {
    async fn cleanup_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, ProductSurfaceError>;
}

use crate::{
    ActiveExtensionCapability, AvailableExtensionCatalog, AvailableExtensionPackage,
    ExtensionActivationMode, ExtensionInstallPlan, imported_extension_package,
    materialize_available_extension, package_visible_capability_ids, prepare_install,
    visible_capability_ids,
};
use crate::{
    ExtensionActivationCredentialGate, ExtensionActivationCredentialReadiness,
    UnavailableExtensionActivationCredentialGate,
};
use crate::{ExtensionRemovalCleanupContext, ExtensionRemovalCleanupRegistry};
use crate::{
    HostedMcpDiscoveryError, channel_connect_strategy, channel_connection_requirement,
    discover_hosted_mcp_package, is_hosted_http_mcp_package,
    manifest_runtime_credential_auth_requirements, package_declares_inbound_product_adapter,
    package_runtime_credential_auth_requirements,
};

use crate::ActiveExtensionPublisher;
use crate::{
    decide_install_on_existing, decide_remove, derive_owner, ensure_caller_may_operate,
    install_scope_for_owner,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExtensionActivationState {
    Installed,
    Enabled,
    Disabled,
}

trait ExtensionInstallationActivationCompat {
    fn activation_state(&self) -> ExtensionActivationState;
}

impl ExtensionInstallationActivationCompat for ExtensionInstallation {
    fn activation_state(&self) -> ExtensionActivationState {
        ExtensionActivationState::Enabled
    }
}

#[async_trait]
trait ExtensionInstallationStoreActivationCompat {
    async fn set_activation_state(
        &self,
        installation_id: &ExtensionInstallationId,
        state: ExtensionActivationState,
    ) -> Result<(), ExtensionInstallationError>;
}

#[async_trait]
impl<T> ExtensionInstallationStoreActivationCompat for T
where
    T: ironclaw_extensions::ExtensionInstallationStorePort + ?Sized,
{
    async fn set_activation_state(
        &self,
        _installation_id: &ExtensionInstallationId,
        _state: ExtensionActivationState,
    ) -> Result<(), ExtensionInstallationError> {
        Ok(())
    }
}

// This port is deliberately scoped to LocalSingleUser composition. The
// lifecycle service models the installed extension set, while active_registry
// is the model-visible capability surface read by host runtime dispatch.
// install/remove keep the lifecycle set durable; activate/remove are the only
// standalone writers that should mirror lifecycle-managed packages into or out
// of active_registry. Production and multi-tenant reuse require scoped storage
// and registry ownership first; tracked in #4091.
pub struct ExtensionLifecycleManager {
    filesystem: Arc<dyn RootFilesystem>,
    catalog: Arc<RwLock<AvailableExtensionCatalog>>,
    installation_store: Arc<dyn ironclaw_extensions::ExtensionInstallationStorePort>,
    lifecycle_service: Arc<Mutex<ExtensionLifecycleService>>,
    active_extensions: ActiveExtensionPublisher,
    operation_lock: Arc<Mutex<()>>,
    // Genuinely optional (not an `optional_arc` smell): a composition without
    // product auth cannot have minted a reusable OAuth credential, so there is
    // nothing to revoke on removal.
    credential_cleanup: Option<Arc<dyn ExtensionCredentialCleanup>>,
    // Late-attached by `build_local_runtime` after the host-runtime lanes are
    // configured (the generic host's loaders bind through them). Attached ⟺
    // the dispatch chain resolves extensions from the host's active snapshot;
    // unattached compositions (focused tests) keep registry-only dispatch.
    generic_host: std::sync::OnceLock<Arc<crate::ExtensionHost>>,
    /// Late-bound weak reference to the effective channel-configuration
    /// resolver. Weak ownership avoids the cycle created by that resolver's
    /// reactivation port pointing back to this lifecycle service.
    channel_config: std::sync::OnceLock<Weak<crate::ChannelConfigService>>,
    // Late-attached with `generic_host` (both need the fully wired host
    // runtime): stages hosted-MCP discovery authority — the connection
    // credential and the server network policy — under the discovery scope.
    // Discovery runs at activation, outside the dispatch obligation
    // pipeline, so nothing else stages these (the pre-P2 gap that made
    // live `tools/list` always fail transient and fall back).
    discovery_runtime_ports:
        std::sync::OnceLock<ironclaw_host_runtime::ProductAuthProviderRuntimePorts>,
    /// Bounds concurrent zip decode/validation in `import_bundle`. Each decode
    /// may expand up to [`crate::MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES`] into
    /// memory, so without a bound N concurrent operator uploads turn the
    /// per-request cap into N x 64 MiB of pressure before any lifecycle lock
    /// applies (#5499 review finding #3).
    import_decode_semaphore: Arc<Semaphore>,
    /// Serializes registry package publication with the lifecycle operations
    /// it coordinates. The ordinary lifecycle lock remains the state writer;
    /// this outer lock only prevents two catalog clients from replacing the
    /// same package between catalog publication and install.
    registry_install_lock: Arc<Mutex<()>>,
    /// The tenant operator identity (#5459 P1). In standalone this is the base
    /// owner user (`IRONCLAW_REBORN_WEBUI_USER_ID` semantics). Lifecycle
    /// installs by every caller, including this user, make or join the member
    /// set [`InstallationOwner::Users`]. Tenant-wide deployment state belongs
    /// to administrator configuration, not lifecycle ownership.
    /// Resolved ONCE here — when P0 role wiring lands, this becomes a
    /// role-derived resolver instead of an identity comparison; callers do
    /// not re-derive admin-ness.
    tenant_operator_user_id: UserId,
    removal_cleanup: Arc<ExtensionRemovalCleanupRegistry>,
    /// Late-binding slot for the generic per-user channel-connection service
    /// (extension-runtime §6.4), shared with
    /// `RebornLocalRuntimeServices::channel_disconnect_slot`. Removing
    /// an extension whose manifest declares a channel surface disconnects the
    /// authenticated caller through it (revoke any personal vendor credential
    /// → vendor/pairing cleanup → delete identity bindings) at this single
    /// convergence point, so `builtin.extension_remove` and the WebUI remove
    /// route cannot drift apart (issue #6091 shape).
    /// Fail-closed contract: removing such an extension with an authenticated
    /// actor while the slot is still empty FAILS the removal with a typed
    /// retryable error instead of skipping the disconnect — an unobservable
    /// binding is treated as a live one, and a removal that cannot run the
    /// per-caller disconnect must not report success. Compositions that
    /// legitimately remove channel extensions fill the slot (runtime
    /// composition in `build_reborn_runtime`, or the channel-connection test
    /// bundle). `new` defaults to a fresh unshared (never-filled) slot for
    /// focused tests.
    channel_disconnect_slot: Arc<std::sync::OnceLock<Arc<dyn ChannelConnectionService>>>,
    /// Product-owned account-setup metadata (activation message and
    /// connection-requirement overrides). Descriptors are declared during
    /// composition; the activation success path consults it and the pairing
    /// seam extends it.
    account_setups: ExtensionAccountSetupRegistry,
    /// Static per-provider instance-config readiness map. Opt-in, defaults
    /// empty via `new` — a third readiness axis alongside `account_setups`
    /// (per-user) and the package-level
    /// requirements `activation_credential_requirements` computes below; see
    /// `provider_instance_readiness.rs` module doc for the full distinction.
    /// Defaulting empty keeps every direct `::new(...)` construction outside
    /// the factory (e.g. test fixtures) unaffected until they opt in via
    /// `with_provider_instance_readiness`.
    provider_instance_readiness: std::collections::BTreeMap<VendorId, String>,
}

/// Concurrent `import_bundle` decodes allowed before further uploads wait.
/// 2 x [`crate::MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES`] caps worst-case decode
/// memory at 128 MiB; imports are a rare admin-only operation, so waiting is
/// the right trade against unbounded memory.
const MAX_CONCURRENT_IMPORT_DECODES: usize = 2;

impl ExtensionLifecycleManager {
    pub fn new(
        filesystem: Arc<dyn RootFilesystem>,
        catalog: AvailableExtensionCatalog,
        installation_store: Arc<dyn ironclaw_extensions::ExtensionInstallationStorePort>,
        lifecycle_service: Arc<Mutex<ExtensionLifecycleService>>,
        active_extensions: ActiveExtensionPublisher,
        credential_cleanup: Option<Arc<dyn ExtensionCredentialCleanup>>,
        tenant_operator_user_id: UserId,
    ) -> Self {
        Self {
            filesystem,
            catalog: Arc::new(RwLock::new(catalog)),
            installation_store,
            lifecycle_service,
            active_extensions,
            operation_lock: Arc::new(Mutex::new(())),
            credential_cleanup,
            generic_host: std::sync::OnceLock::new(),
            channel_config: std::sync::OnceLock::new(),
            discovery_runtime_ports: std::sync::OnceLock::new(),
            import_decode_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_IMPORT_DECODES)),
            registry_install_lock: Arc::new(Mutex::new(())),
            tenant_operator_user_id,
            removal_cleanup: Arc::new(ExtensionRemovalCleanupRegistry::empty()),
            account_setups: ExtensionAccountSetupRegistry::default(),
            channel_disconnect_slot: Arc::new(std::sync::OnceLock::new()),
            provider_instance_readiness: std::collections::BTreeMap::new(),
        }
    }

    /// Attach the staging ports hosted-MCP discovery uses to make its
    /// authority available under the discovery scope.
    pub fn attach_discovery_runtime_ports(
        &self,
        ports: ironclaw_host_runtime::ProductAuthProviderRuntimePorts,
    ) {
        let _ = self.discovery_runtime_ports.set(ports);
    }

    /// Stage the hosted-MCP connection credential and server network policy
    /// for the discovery call. Best-effort by design: a staging failure
    /// leaves discovery to fail transient; activation may use a real declared
    /// static tool as fallback, but never the host-internal connection
    /// template alone. A successful stage lets live `tools/list` run with the
    /// same injected authority a dispatched invocation would carry.
    async fn stage_hosted_mcp_discovery_authority(
        &self,
        scope: &ResourceScope,
        package: &ExtensionPackage,
    ) {
        let Some(ports) = self.discovery_runtime_ports.get() else {
            return;
        };
        let Some(descriptor) = package.capabilities.first() else {
            return;
        };
        if let Some(policy) = hosted_mcp_discovery_network_policy(package) {
            ports.stage_network_policy_once(scope, &descriptor.id, policy);
        }
        for requirement in &descriptor.runtime_credentials {
            if let Err(error) = ports
                .stage_credential_requirement_once(scope, &descriptor.id, requirement, &package.id)
                .await
            {
                tracing::debug!(
                    extension_id = package.id.as_str(),
                    capability_id = descriptor.id.as_str(),
                    required = requirement.required,
                    error = ?error,
                    "hosted MCP discovery credential staging failed; discovery will fail or use a declared static fallback"
                );
            }
        }
    }

    /// The durable installation store handle (the generic host hydrates its
    /// working set from it at boot).
    pub fn installation_store_handle(
        &self,
    ) -> Arc<dyn ironclaw_extensions::ExtensionInstallationStorePort> {
        Arc::clone(&self.installation_store)
    }

    pub async fn reserved_bundled_extension_ids(&self) -> Vec<String> {
        self.catalog.read().await.reserved_bundled_ids().to_vec()
    }

    /// Attach the generic extension host so lifecycle mutations publish the
    /// active snapshot the dispatch chain resolves from.
    pub fn attach_generic_host(&self, host: Arc<crate::ExtensionHost>) {
        let _ = self.generic_host.set(host);
    }

    pub fn attach_channel_config(&self, channel_config: &Arc<crate::ChannelConfigService>) {
        let _ = self.channel_config.set(Arc::downgrade(channel_config));
    }

    /// The attached generic host, when this service has one — the snapshot
    /// authority the channel host assembly reconciles against.
    pub fn generic_host(&self) -> Option<Arc<crate::ExtensionHost>> {
        self.generic_host.get().cloned()
    }

    /// Mirror an activation into the generic host's snapshot. Runs after the
    /// registry publish succeeded; a failure here fails the activation (the
    /// caller compensates) — extension dispatch resolves from the snapshot,
    /// so an unmirrored activation would produce undispatchable tools.
    async fn publish_to_generic_host(
        &self,
        extension_id: &ExtensionId,
        installation_id: &ExtensionInstallationId,
        active_package: &ExtensionPackage,
    ) -> Result<(), ProductSurfaceFailure> {
        let Some(host) = self.generic_host.get() else {
            return Ok(());
        };
        let base = self
            .installation_store
            .get_manifest(extension_id)
            .await
            .map_err(map_extension_installation_error)?
            .ok_or_else(|| ProductSurfaceFailure::InvalidBindingRequest {
                reason: format!(
                    "extension {} manifest is not installed",
                    extension_id.as_str()
                ),
            })?;
        let effective = crate::effective_resolved_for_package(base.resolved(), active_package);
        // Durable per-installation `[channel.config]` values ride the
        // published record so `ChannelAdapter::activate` sees them.
        let config = match self.channel_config.get().and_then(Weak::upgrade) {
            Some(channel_config) => channel_config
                .effective_non_secret_config(extension_id)
                .await
                .map_err(map_channel_config_error)?,
            None => Vec::new(),
        };
        let record = crate::InstallationRecord {
            extension_id: extension_id.as_str().to_string(),
            installation_id: installation_id.as_str().to_string(),
            state: crate::InstallationState::Installed,
            resolved: Arc::new(effective),
            config,
            last_error: None,
        };
        host.install(record).await.map_err(generic_host_error)?;
        host.activate(extension_id.as_str())
            .await
            .map_err(generic_host_error)
    }

    /// Test-support twin of the production activation choke point: publish a
    /// bundled package directly into the registry AND mirror it into the
    /// generic host's snapshot (mirrors `commit_activation` →
    /// `publish_to_generic_host`, without the durable install/credential
    /// legs). Direct registry publication alone would leave the package
    /// undispatchable now that extension dispatch resolves from the snapshot.
    /// Operator `[channel.config]` values are NOT seeded here — they flow
    /// exclusively through the production configure surface
    /// (`ChannelConfigService`), and this seam reads whatever that surface
    /// durably stored, exactly like the production publish path.
    pub async fn publish_bundled_package_for_test(
        &self,
        package: &ExtensionPackage,
        resolved: Option<&ironclaw_extensions::ResolvedExtensionManifest>,
    ) -> Result<(), ProductSurfaceFailure> {
        self.active_extensions.publish(package)?;
        let Some(host) = self.generic_host.get() else {
            return Ok(());
        };
        // The resolved base: caller-supplied for in-code fixture packages,
        // else parsed from the catalog entry's raw manifest.
        let base = match resolved {
            Some(resolved) => resolved.clone(),
            None => {
                let package_ref =
                    LifecyclePackageRef::new(LifecyclePackageKind::Extension, package.id.as_str())?;
                let available = self.catalog.read().await.resolve(&package_ref)?;
                let host_ports =
                    ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
                        ProductSurfaceFailure::InvalidBindingRequest {
                            reason: format!(
                                "host port catalog rejected bundled extension: {error}"
                            ),
                        }
                    })?;
                let contracts =
                    crate::product_extension_host_api_contract_registry().map_err(|error| {
                        ProductSurfaceFailure::InvalidBindingRequest {
                            reason: format!(
                                "host API contracts rejected bundled extension: {error}"
                            ),
                        }
                    })?;
                ironclaw_extensions::ExtensionManifestRecord::from_toml(
                    available.manifest_toml.clone(),
                    ironclaw_extensions::ManifestSource::HostBundled,
                    &host_ports,
                    None,
                    &contracts,
                    Some(package.root.clone()),
                )
                .map_err(|error| ProductSurfaceFailure::InvalidBindingRequest {
                    reason: format!("bundled extension manifest is invalid: {error}"),
                })?
                .resolved()
                .clone()
            }
        };
        let effective = crate::effective_resolved_for_package(&base, package);
        // This shortcut deliberately publishes without creating a durable
        // installation. A tool-only package has no channel configuration to
        // resolve, and asking the attached configuration consumer to load its
        // absent installed manifest would make the test-support seam fail
        // before the tool surface can be published.
        let config = match (
            effective.channel.is_some(),
            self.channel_config.get().and_then(Weak::upgrade),
        ) {
            (false, _) => Vec::new(),
            (true, Some(channel_config)) => channel_config
                .effective_non_secret_config(&package.id)
                .await
                .map_err(map_channel_config_error)?,
            (true, None) => Vec::new(),
        };
        host.install(crate::InstallationRecord {
            extension_id: package.id.as_str().to_string(),
            installation_id: format!("{}-test-install", package.id.as_str()),
            state: crate::InstallationState::Installed,
            resolved: Arc::new(effective),
            config,
            last_error: None,
        })
        .await
        .map_err(generic_host_error)?;
        host.activate(package.id.as_str())
            .await
            .map_err(generic_host_error)
    }

    /// Mirror an unpublish into the generic host's snapshot (deactivation is
    /// tolerant: a not-installed record is already unpublished).
    async fn unpublish_from_generic_host(&self, extension_id: &ExtensionId) {
        let Some(host) = self.generic_host.get() else {
            return;
        };
        match host.deactivate(extension_id.as_str()).await {
            Ok(()) | Err(crate::LifecycleError::NotInstalled { .. }) => {}
            Err(error) => {
                tracing::warn!(
                    extension_id = extension_id.as_str(),
                    error = ?error,
                    "generic extension host could not unpublish extension"
                );
            }
        }
        if let Some(host) = self.generic_host.get()
            && let Err(error) = host.remove_record(extension_id.as_str()).await
        {
            tracing::debug!(
                extension_id = extension_id.as_str(),
                error = %error,
                "generic extension host record cleanup failed"
            );
        }
    }

    pub fn with_account_setup_registry(
        mut self,
        account_setups: ExtensionAccountSetupRegistry,
    ) -> Self {
        self.account_setups = account_setups;
        self
    }

    /// Install the static per-provider instance-config readiness map.
    /// Defaults empty from `new`, so callers that never opt in (test
    /// fixtures, any composition without the build-time signal) see no
    /// behavior change.
    pub fn with_provider_instance_readiness(
        mut self,
        provider_instance_readiness: std::collections::BTreeMap<VendorId, String>,
    ) -> Self {
        self.provider_instance_readiness = provider_instance_readiness;
        self
    }

    pub fn with_removal_cleanup_registry(
        mut self,
        removal_cleanup: Arc<ExtensionRemovalCleanupRegistry>,
    ) -> Self {
        self.removal_cleanup = removal_cleanup;
        self
    }

    /// Share the composition's late-binding channel-connection service slot
    /// (see the field doc). Composition passes the SAME `Arc` stored on
    /// `RebornLocalRuntimeServices` so a fill by runtime composition (or the
    /// channel-connection test bundle) is visible to the removal path here.
    pub fn with_channel_disconnect_slot(
        mut self,
        slot: Arc<std::sync::OnceLock<Arc<dyn ChannelConnectionService>>>,
    ) -> Self {
        self.channel_disconnect_slot = slot;
        self
    }

    /// Test-support access to the extension installation store.
    ///
    /// Mirrors the `installation_store` field that `build_local_runtime` wires
    /// in when constructing `ExtensionLifecycleManager`. For tests
    /// only — zero bytes shipped in production builds.
    pub fn installation_store_for_test(
        &self,
    ) -> Arc<dyn ironclaw_extensions::ExtensionInstallationStorePort> {
        Arc::clone(&self.installation_store)
    }

    /// C-JOURNEY: test-support access to the active-extension publisher
    /// (registry + trust policy). `activate()` ultimately delegates the
    /// model-visible-surface mutation to `self.active_extensions.publish(..)`
    /// (see `active_publication.rs`) after its own install/credential-gate
    /// bookkeeping; this accessor reaches that SAME publish step directly so a
    /// test harness can make a bundled first-party WASM package
    /// genuinely dispatchable without driving the full multi-turn
    /// install→activate capability handshake through the model. For tests
    /// only — zero bytes shipped in production builds.
    pub fn active_extensions_for_test(&self) -> &ActiveExtensionPublisher {
        &self.active_extensions
    }

    /// Test-support view of the wired tenant-operator identity (#5459 P1), so
    /// tests can act "as the operator" without re-deriving the id the runtime
    /// or fixture was built with. Mirrors the production owner wiring in
    /// `build_local_runtime`. Tests only — zero bytes in production builds.
    pub fn tenant_operator_user_id_for_test(&self) -> &UserId {
        &self.tenant_operator_user_id
    }

    pub async fn search(
        &self,
        query: &str,
        credential_gate: Option<&dyn ExtensionActivationCredentialGate>,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
        let extensions = {
            let catalog = self.catalog.read().await;
            catalog.search(query).collect::<Vec<_>>()
        };
        let activation_errors = self.installation_activation_errors().await?;
        let mut summaries = Vec::new();
        for extension in extensions {
            summaries.push(
                self.search_summary(&extension, credential_gate, caller, &activation_errors)
                    .await?,
            );
        }
        let count = summaries.len();
        // The top-level phase of a multi-item search response is neutral; each
        // result carries its own `installation_phase`.
        let mut response = response_with_payload(
            None,
            InstallationState::Installed,
            LifecycleProductPayload::ExtensionSearch {
                extensions: summaries,
                count,
            },
        );
        if extension_search_has_installed_external_channel_result(response.payload.as_ref()) {
            response.message = Some(
                "Search found installed external channel results. Search cannot prove the calling user's channel account is personally connected. For an explicit connect, pair, authenticate, or account-access request, call builtin.extension_install for the matching extension id so install-driven activation can publish tools or surface channel-specific connection/setup instructions. For routine, trigger, or notification delivery, prefer the configured outbound delivery target when one is available; do not reconnect the channel just to send to an already configured delivery target."
                    .to_string(),
            );
        } else if extension_search_has_inactive_installed_result(response.payload.as_ref()) {
            response.message = Some(
                "Search found installed extension results that are not active yet. Report these as installed but not activated; configured only means required credentials appear present, not that tools are published. Any visible_capability_ids on inactive results are catalog capabilities only, not currently callable tools. To make the extension available, call builtin.extension_install for the matching extension id; install is idempotent and attempts activation."
                    .to_string(),
            );
        } else if extension_search_has_ready_result(response.payload.as_ref()) {
            response.message = Some(
                "Search found active installed extension results. Treat those results as ready for this connection request; do not ask the user for credentials unless a later tool call reports auth_required."
                    .to_string(),
            );
        }
        Ok(response)
    }

    pub async fn list_installed(
        &self,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
        let summaries = self.installed_summaries(caller).await?;
        let count = summaries.len();
        Ok(response_with_payload(
            None,
            InstallationState::Installed,
            LifecycleProductPayload::ExtensionList {
                extensions: summaries,
                count,
            },
        ))
    }

    pub async fn project(
        &self,
        package_ref: LifecyclePackageRef,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
        let (_, installation_id) = extension_ids_from_package_ref(&package_ref)?;
        let installation = self
            .installation_store
            .get_installation(&installation_id)
            .await
            .map_err(map_extension_installation_error)?
            // A foreign user-private install projects as not-installed for
            // this caller — same masking as search/list (#5459 P1).
            .filter(|installation| installation.owner().visible_to(caller));
        let activation_errors = self.installation_activation_errors().await?;
        // A not-installed package has no installation state; `install_scope`
        // (`None` below) is the not-installed signal, so the neutral `Installed`
        // here is never read as a resting state for an uninstalled package.
        let phase = installation
            .as_ref()
            .map(|installation| {
                installation_state_for_activation(
                    installation.activation_state(),
                    activation_errors.contains_key(installation.extension_id().as_str()),
                )
            })
            .unwrap_or(InstallationState::Installed);
        let install_scope = installation
            .as_ref()
            .map(|installation| install_scope_for_owner(installation.owner()));
        let summary = {
            let catalog = self.catalog.read().await;
            catalog.resolve(&package_ref)?.summary()
        };
        Ok(response_with_payload(
            Some(package_ref),
            phase,
            LifecycleProductPayload::ExtensionList {
                extensions: vec![LifecycleInstalledExtensionSummary {
                    summary,
                    phase,
                    install_scope,
                }],
                count: 1,
            },
        ))
    }

    pub async fn active_model_visible_capabilities(
        &self,
    ) -> Result<Vec<ActiveExtensionCapability>, ProductSurfaceFailure> {
        // #5459 P1: carry each enabled installation's owner onto its
        // capabilities so the per-request grant minting in the standalone
        // capability surface can filter user-private extensions to their
        // owner. The registry itself stays global; owner is joined here.
        let owner_by_extension = project_installation_owners(
            self.installation_store
                .list_installations()
                .await
                .map_err(map_extension_installation_error)?,
        )?;
        let registry = self.active_extensions.snapshot();
        Ok(registry
            .capabilities()
            .filter_map(|descriptor| {
                let owner = owner_by_extension.get(&descriptor.provider)?;
                let model_visible = registry
                    .capability_visibility(&descriptor.id)
                    .unwrap_or(CapabilityVisibility::Model)
                    == CapabilityVisibility::Model;
                model_visible
                    .then(|| ActiveExtensionCapability::from_descriptor(descriptor, owner.clone()))
            })
            .collect())
    }

    /// Owner of every installation (all activation states), keyed by extension
    /// id (#5459 P1). The operator/settings tool catalog joins this to the
    /// global extension registry so it can hide another user's private tool —
    /// the registry snapshot alone carries no owner. Uses `list_installations`
    /// (not `_enabled_`) because the catalog reflects installed tools
    /// regardless of activation state.
    pub async fn installation_owners(
        &self,
    ) -> Result<std::collections::BTreeMap<ExtensionId, InstallationOwner>, ProductSurfaceFailure>
    {
        project_installation_owners(
            self.installation_store
                .list_installations()
                .await
                .map_err(map_extension_installation_error)?,
        )
    }

    pub async fn activation_credential_requirements(
        &self,
        package_ref: &LifecyclePackageRef,
        caller: &UserId,
    ) -> Result<Vec<RuntimeCredentialAuthRequirement>, ProductSurfaceFailure> {
        let (extension_id, installation_id) = extension_ids_from_package_ref(package_ref)?;
        let _operation_guard = self.operation_lock.lock().await;
        let installation = self
            .load_installation(&extension_id, &installation_id)
            .await?;
        // Ownership masks before any credential preflight: a non-owner must
        // get the "is not installed" denial, never a requirement shape that
        // confirms a private credentialed install exists (#5525 review).
        ensure_caller_may_operate(&installation, caller)?;
        let package = self.lifecycle_package(&extension_id).await?;
        let mut requirements = package_runtime_credential_auth_requirements(&package);
        if let Some(requirement) = self
            .account_setups
            .missing_requirement(&extension_id, caller)
            .await
            .map_err(map_account_setup_error)?
        {
            requirements.push(requirement);
        }
        // Third readiness axis: a provider whose OPERATOR-level instance
        // config is missing entirely (no OAuth backend registered on this
        // build at all) fails here, before the per-user credential gate below
        // ever runs — distinct from `account_setups` (per-user account state)
        // and the package-level `requirements` just computed (per-package
        // static declarations). Mirrors the same three-axis distinction drawn
        // in `gsuite.rs:69-73` for the dispatch-time backstop that shares
        // this build-time signal. Both callers of this function share this
        // one chokepoint: the LLM tool handler's own `missing_requirements`
        // short-circuit (`extension_lifecycle_capabilities.rs`) and the
        // WebUI card's `activate_inner` credential gate never see a
        // requirement shape for an unconfigured provider — they see this
        // `Err` instead.
        if let Some(reason) = requirements.iter().find_map(|requirement| {
            self.provider_instance_readiness
                .get(&requirement.provider)
                .cloned()
        }) {
            return Err(ProductSurfaceFailure::ProviderInstanceNotConfigured { reason });
        }
        Ok(requirements)
    }

    /// Redacted per-extension activation errors from the generic host's
    /// working records, keyed by extension id. A record carries a `last_error`
    /// exactly when its last activation attempt recorded a terminal `Failed`.
    /// Empty when the generic host is not attached to this port. Both the
    /// installation-state projection (`Failed`) and the extensions wire's
    /// `activation_error` are driven from this one source.
    pub async fn installation_activation_errors(
        &self,
    ) -> Result<std::collections::HashMap<String, String>, ProductSurfaceFailure> {
        match self.generic_host.get() {
            Some(host) => {
                host.installation_errors()
                    .await
                    .map_err(|error| ProductSurfaceFailure::Transient {
                        reason: format!("extension activation errors could not be read: {error}"),
                    })
            }
            None => Ok(std::collections::HashMap::new()),
        }
    }

    async fn installed_summaries(
        &self,
        caller: &UserId,
    ) -> Result<Vec<LifecycleInstalledExtensionSummary>, ProductSurfaceFailure> {
        let installations = self
            .installation_store
            .list_installations()
            .await
            .map_err(map_extension_installation_error)?;
        let activation_errors = self.installation_activation_errors().await?;
        let mut summaries = Vec::with_capacity(installations.len());
        for installation in installations {
            // #5459 P1: a caller's list is tenant-shared entries plus their
            // OWN private entries; other users' private installs are invisible
            // (the operator included — private installs are not enumerable).
            if !installation.owner().visible_to(caller) {
                continue;
            }
            let Ok(package_ref) = LifecyclePackageRef::new(
                LifecyclePackageKind::Extension,
                installation.extension_id().as_str(),
            ) else {
                continue;
            };
            let available = {
                let catalog = self.catalog.read().await;
                let Ok(available) = catalog.resolve(&package_ref) else {
                    continue;
                };
                available
            };
            summaries.push(LifecycleInstalledExtensionSummary {
                summary: available.summary(),
                phase: installation_state_for_activation(
                    installation.activation_state(),
                    activation_errors.contains_key(installation.extension_id().as_str()),
                ),
                install_scope: Some(install_scope_for_owner(installation.owner())),
            });
        }
        Ok(summaries)
    }

    async fn search_summary(
        &self,
        extension: &AvailableExtensionPackage,
        credential_gate: Option<&dyn ExtensionActivationCredentialGate>,
        caller: &UserId,
        activation_errors: &std::collections::HashMap<String, String>,
    ) -> Result<LifecycleSearchExtensionSummary, ProductSurfaceFailure> {
        let mut summary = extension.summary();
        suppress_search_credential_onboarding(&mut summary);
        let installation = self
            .search_installation(&extension.package.id)
            .await?
            // A foreign user-private install reads as not-installed for this
            // caller (#5459 P1) — same masking as list/project.
            .filter(|installation| installation.owner().visible_to(caller));
        let Some(installation) = installation else {
            return Ok(LifecycleSearchExtensionSummary {
                summary,
                installation_phase: None,
            });
        };
        let has_last_error = activation_errors.contains_key(installation.extension_id().as_str());
        let phase =
            search_installation_phase(extension, &installation, credential_gate, has_last_error)
                .await?;
        Ok(LifecycleSearchExtensionSummary {
            summary,
            installation_phase: Some(phase),
        })
    }

    async fn search_installation(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionInstallation>, ProductSurfaceFailure> {
        let installation_id = ExtensionInstallationId::new(extension_id.as_str().to_string())
            .map_err(map_extension_installation_error)?;
        let installation = self
            .installation_store
            .get_installation(&installation_id)
            .await
            .map_err(map_extension_installation_error)?;
        if installation
            .as_ref()
            .is_some_and(|installation| installation.extension_id() != extension_id)
        {
            return Err(ProductSurfaceFailure::InvalidBindingRequest {
                reason: format!(
                    "installation {} does not belong to extension {}",
                    installation_id.as_str(),
                    extension_id.as_str()
                ),
            });
        }
        Ok(installation)
    }

    /// Import a standalone extension from an uploaded bundle (zip bytes) — the
    /// WebUI "Install Tool" path. Unzips (zip-slip guarded), validates the
    /// `manifest.toml`, writes the assets under `/system/extensions/<id>/` so it
    /// survives a restart (restart discovery reloads that root as
    /// `InstalledLocal`, never the first-party `HostBundled` tier), and extends
    /// the in-memory catalog so it shows in the Registry immediately. The
    /// existing install/activate flow then operates on it like any other
    /// available extension.
    ///
    /// Takes the catalog WRITE lock, then `operation_lock` — the same
    /// catalog-before-operation order `install` uses, so the two cannot
    /// deadlock. Both guards are held across the duplicate checks AND the
    /// filesystem materialization: concurrent imports of the same id would
    /// otherwise interleave file-by-file writes into the stable
    /// `/system/extensions/<id>/` root, and an import over an already
    /// installed id would swap the materialized files out from under the
    /// live lifecycle state.
    ///
    /// The unzip + manifest validation phase runs in `spawn_blocking` (it is
    /// CPU/blocking-IO work that must not stall the async runtime) behind a
    /// [`MAX_CONCURRENT_IMPORT_DECODES`]-permit semaphore acquired BEFORE any
    /// lifecycle lock, bounding decode memory instead of letting N concurrent
    /// uploads each expand [`crate::MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES`].
    pub async fn import_bundle(
        &self,
        bundle: Vec<u8>,
    ) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
        // Hold the permit until the package has passed duplicate checks,
        // materialization, and catalog insertion. This bounds the number of
        // fully expanded packages retained by an import in addition to the
        // decode work itself.
        let _decode_permit = self.import_decode_semaphore.acquire().await.map_err(|_| {
            ProductSurfaceFailure::Transient {
                reason: "import decode limiter is closed".to_string(),
            }
        })?;
        let reserved_bundled_ids = self.catalog.read().await.reserved_bundled_ids().to_vec();
        let package = tokio::task::spawn_blocking(move || {
            let files = unzip_extension_bundle_for_product(&bundle)?;
            imported_extension_package(files, &reserved_bundled_ids)
        })
        .await
        .map_err(|error| ProductSurfaceFailure::Transient {
            reason: format!("import decode task failed: {error}"),
        })??;
        let package_ref = package.package_ref.clone();
        let summary = package.summary();
        let mut catalog = self.catalog.write().await;
        let _operation_guard = self.operation_lock.lock().await;
        if catalog.resolve(&package_ref).is_ok() {
            return Err(ProductSurfaceFailure::InvalidBindingRequest {
                reason: format!(
                    "extension {} already exists in the catalog; remove it before importing a replacement",
                    package_ref.id.as_str()
                ),
            });
        }
        let installation_id = ExtensionInstallationId::new(package.package.id.as_str().to_string())
            .map_err(map_extension_installation_error)?;
        self.ensure_not_installed(&package.package.id, &installation_id)
            .await?;
        materialize_available_extension(self.filesystem.as_ref(), &package).await?;
        catalog.extend(AvailableExtensionCatalog::from_packages(vec![package]));
        drop(catalog);
        Ok(response_with_payload(
            Some(package_ref),
            InstallationState::Installed,
            LifecycleProductPayload::ExtensionSearch {
                extensions: vec![LifecycleSearchExtensionSummary {
                    summary,
                    installation_phase: None,
                }],
                count: 1,
            },
        ))
    }

    /// Publish and install a package whose registry client has already
    /// verified signature, provenance, size, and artifact digests.
    ///
    /// The package still enters through the extension-host validation
    /// boundary before this method. A forced replacement uses the ordinary
    /// removal/install convergence points and restores the previous inline
    /// catalog package if the replacement install fails.
    pub async fn install_registry_package(
        &self,
        package: AvailableExtensionPackage,
        force: bool,
        caller: &UserId,
        scope: &ResourceScope,
    ) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
        if package.source != ironclaw_extensions::ManifestSource::RegistryInstalled {
            return Err(ProductSurfaceFailure::InvalidBindingRequest {
                reason: "registry install requires a registry-validated package".to_string(),
            });
        }
        let _registry_guard = self.registry_install_lock.lock().await;
        let package_ref = package.package_ref.clone();
        let extension_id = package.package.id.clone();
        let previous = {
            let catalog = self.catalog.read().await;
            catalog.resolve(&package_ref).ok()
        };
        if let Some(previous) = &previous {
            let matches = previous.manifest_toml == package.manifest_toml
                && previous.assets == package.assets;
            if matches {
                return self
                    .install_and_activate_registry_package(package_ref, caller)
                    .await;
            }
            if !force {
                return Err(ProductSurfaceFailure::InvalidBindingRequest {
                    reason: format!(
                        "extension {} already exists in the catalog; retry with force to replace it",
                        extension_id.as_str()
                    ),
                });
            }
            if previous.source == ironclaw_extensions::ManifestSource::HostBundled {
                return Err(ProductSurfaceFailure::InvalidBindingRequest {
                    reason: format!(
                        "extension {} is host-bundled and cannot be replaced by a registry package",
                        extension_id.as_str()
                    ),
                });
            }
        }

        let previous_installation = self.search_installation(&extension_id).await?;
        let had_installation = previous_installation.is_some()
            || self
                .installation_store
                .get_manifest(&extension_id)
                .await
                .map_err(map_extension_installation_error)?
                .is_some();
        if had_installation && previous.is_none() {
            return Err(ProductSurfaceFailure::InvalidBindingRequest {
                reason: format!(
                    "extension {} has installed state but no restorable catalog package",
                    extension_id.as_str()
                ),
            });
        }
        let was_active = self
            .active_extensions
            .snapshot()
            .get_extension(&extension_id)
            .is_some();
        if had_installation {
            if !force {
                return Err(ProductSurfaceFailure::InvalidBindingRequest {
                    reason: format!(
                        "extension {} is already installed; retry with force to replace it",
                        extension_id.as_str()
                    ),
                });
            }
            self.remove(package_ref.clone(), scope, Some(caller))
                .await?;
        }

        {
            let mut catalog = self.catalog.write().await;
            catalog.extend(AvailableExtensionCatalog::from_packages(vec![package]));
        }
        match self
            .install_and_activate_registry_package(package_ref.clone(), caller)
            .await
        {
            Ok(response) => Ok(response),
            Err(original_error) => {
                if let Err(cleanup_error) =
                    self.remove(package_ref.clone(), scope, Some(caller)).await
                {
                    return Err(compensation_failure(
                        "registry install failed and replacement cleanup also failed",
                        original_error,
                        cleanup_error,
                    ));
                }
                {
                    let mut catalog = self.catalog.write().await;
                    catalog.remove(&package_ref);
                    if let Some(previous) = previous {
                        catalog.restore(previous);
                    }
                }
                if had_installation {
                    let restore = self.install(package_ref.clone(), caller).await;
                    if let Err(restore_error) = restore {
                        return Err(compensation_failure(
                            "registry replacement failed and the previous install could not be restored",
                            original_error,
                            restore_error,
                        ));
                    }
                    if let Some(installation) = &previous_installation
                        && let Err(restore_error) = self.restore_installation(installation).await
                    {
                        return Err(compensation_failure(
                            "registry replacement failed and the previous installation scope could not be restored",
                            original_error,
                            restore_error,
                        ));
                    }
                    if was_active
                        && let Err(restore_error) = self
                            .activate(package_ref, ExtensionActivationMode::Static, caller)
                            .await
                    {
                        return Err(compensation_failure(
                            "registry replacement failed and the previous activation could not be restored",
                            original_error,
                            restore_error,
                        ));
                    }
                }
                Err(original_error)
            }
        }
    }

    async fn install_and_activate_registry_package(
        &self,
        package_ref: LifecyclePackageRef,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
        self.install(package_ref.clone(), caller).await?;
        self.activate(package_ref, ExtensionActivationMode::Static, caller)
            .await
    }

    pub async fn install(
        &self,
        package_ref: LifecyclePackageRef,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
        // Snapshot the package before taking `operation_lock`. The catalog
        // lock must not be held across installation-store, filesystem, or
        // credential awaits. Acquiring the read lock first preserves the
        // catalog-before-operation ordering used by `import_bundle` without
        // retaining a borrow into the catalog.
        let available = {
            let catalog = self.catalog.read().await;
            catalog.resolve(&package_ref)?
        };
        let _operation_guard = self.operation_lock.lock().await;
        let installation_id =
            ExtensionInstallationId::new(available.package.id.as_str().to_string())
                .map_err(map_extension_installation_error)?;
        let existing = self
            .installation_store
            .get_installation(&installation_id)
            .await
            .map_err(map_extension_installation_error)?;
        match existing {
            // The id is already installed: the policy decision authorizes the
            // caller (tenant rows and non-member shapes error here), and the
            // store's membership operation performs the single-row join —
            // never an aggregate rewrite, which would reintroduce the
            // lost-update race between independent users. The bundle is
            // already registered, materialized, and (if enabled) published,
            // so there is nothing to compensate.
            Some(existing) => {
                decide_install_on_existing(
                    &available.package.id,
                    existing.owner(),
                    caller,
                    &self.tenant_operator_user_id,
                )?;
                self.installation_store
                    .activate_membership(&installation_id, caller)
                    .await
                    .map_err(map_extension_installation_error)?;
            }
            None => {
                self.install_fresh_locked(&available, caller).await?;
            }
        }

        Ok(response_with_payload(
            Some(package_ref.clone()),
            InstallationState::Installed,
            LifecycleProductPayload::ExtensionInstall {
                installed: true,
                visible_capability_ids: visible_capability_ids(&available)
                    .map(|id| id.as_str().to_string())
                    .collect(),
                next_step: format!(
                    "Installation will attempt activation for extension_id \"{}\". If credentials are missing, the install response opens the auth gate; otherwise the tools are published.",
                    package_ref.id.as_str()
                ),
            },
        ))
    }

    /// First install of an id: register the lifecycle package, materialize
    /// the bundle, and persist the installation plan, unwinding on failure.
    /// Callers hold `operation_lock` and have verified no installation row
    /// exists.
    async fn install_fresh_locked(
        &self,
        available: &AvailableExtensionPackage,
        caller: &UserId,
    ) -> Result<(), ProductSurfaceFailure> {
        // An orphaned manifest row without an installation still counts as
        // occupied (pre-#5459 behavior, kept fail-closed).
        if self
            .installation_store
            .get_manifest(&available.package.id)
            .await
            .map_err(map_extension_installation_error)?
            .is_some()
        {
            return Err(ProductSurfaceFailure::InvalidBindingRequest {
                reason: format!(
                    "extension {} is already installed; if a previous removal was interrupted, run remove again to finish its cleanup, then retry the install",
                    available.package.id.as_str()
                ),
            });
        }
        let owner = derive_owner(caller, &self.tenant_operator_user_id);
        let plan = prepare_install(available, owner)?;
        self.register_lifecycle_package(&available.package).await?;

        if let Err(error) =
            materialize_available_extension(self.filesystem.as_ref(), available).await
        {
            if let Err(rollback_error) =
                self.rollback_lifecycle_install(&available.package.id).await
            {
                return Err(compensation_failure(
                    "extension install materialization failed and lifecycle rollback failed",
                    error,
                    rollback_error,
                ));
            }
            return Err(error);
        }
        if let Err(error) = self.persist_install_plan(plan).await {
            if let Err(cleanup_error) = self
                .delete_materialized_extension_files(&available.package.id)
                .await
            {
                tracing::debug!(
                    error = ?cleanup_error,
                    "best-effort extension file cleanup failed"
                );
            }
            if let Err(rollback_error) =
                self.rollback_lifecycle_install(&available.package.id).await
            {
                return Err(compensation_failure(
                    "extension install persistence failed and lifecycle rollback failed",
                    error,
                    rollback_error,
                ));
            }
            return Err(error);
        }
        Ok(())
    }

    pub async fn activate(
        &self,
        package_ref: LifecyclePackageRef,
        mode: ExtensionActivationMode,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
        let credential_gate = UnavailableExtensionActivationCredentialGate;
        self.activate_inner(package_ref, mode, &credential_gate, caller)
            .await
    }

    pub async fn activate_with_credential_gate(
        &self,
        package_ref: LifecyclePackageRef,
        mode: ExtensionActivationMode,
        credential_gate: impl ExtensionActivationCredentialGate,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
        self.activate_inner(package_ref, mode, &credential_gate, caller)
            .await
    }

    pub async fn activate_with_prechecked_credentials_for_test(
        &self,
        package_ref: LifecyclePackageRef,
        mode: ExtensionActivationMode,
    ) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
        let caller = self.tenant_operator_user_id.clone();
        self.activate_with_prechecked_credentials_for_user_for_test(package_ref, mode, &caller)
            .await
    }

    pub async fn activate_with_prechecked_credentials_for_user_for_test(
        &self,
        package_ref: LifecyclePackageRef,
        mode: ExtensionActivationMode,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
        let credential_gate = crate::PrecheckedExtensionActivationCredentialGate;
        self.activate_inner(package_ref, mode, &credential_gate, caller)
            .await
    }

    async fn activate_inner(
        &self,
        package_ref: LifecyclePackageRef,
        mode: ExtensionActivationMode,
        credential_gate: &dyn ExtensionActivationCredentialGate,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
        let (extension_id, installation_id) = extension_ids_from_package_ref(&package_ref)?;

        let discovery = {
            let _operation_guard = self.operation_lock.lock().await;
            let installation = self
                .load_installation(&extension_id, &installation_id)
                .await?;
            ensure_caller_may_operate(&installation, caller)?;
            ensure_caller_may_mutate_tenant_installation(
                &installation,
                caller,
                &self.tenant_operator_user_id,
                "activate",
            )?;
            let package = self.lifecycle_package(&extension_id).await?;
            if let ExtensionActivationCredentialReadiness::Missing(missing) =
                credential_gate.credential_readiness(&package).await?
            {
                return activation_credentials_incomplete_response(package_ref, missing);
            }
            match mode {
                ExtensionActivationMode::HostedMcpDiscovery {
                    scope,
                    runtime_http_egress,
                } if is_hosted_http_mcp_package(&package) => HostedMcpDiscoveryRequest {
                    base_package: package,
                    scope,
                    runtime_http_egress,
                },
                _ => {
                    return self
                        .commit_activation(
                            package_ref,
                            &extension_id,
                            &installation_id,
                            installation.activation_state(),
                            package,
                        )
                        .await;
                }
            }
        };

        self.stage_hosted_mcp_discovery_authority(&discovery.scope, &discovery.base_package)
            .await;
        let active_package = match discover_hosted_mcp_package(
            &discovery.base_package,
            discovery.scope,
            discovery.runtime_http_egress,
        )
        .await
        {
            Ok(active_package) => active_package,
            Err(HostedMcpDiscoveryError::Transient(reason)) => {
                if package_visible_capability_ids(&discovery.base_package).is_empty() {
                    // The bundled hosted-MCP declaration may contain only the
                    // host-internal connection template. That template is
                    // discovery authority, not a callable fallback surface;
                    // reporting activation success here would publish no
                    // model-usable tools. Keep the install retryable instead.
                    return Err(hosted_mcp_discovery_error(
                        HostedMcpDiscoveryError::Transient(reason),
                    ));
                }
                tracing::debug!(
                    extension_id = %extension_id.as_str(),
                    reason,
                    "hosted MCP discovery failed during activation; falling back to bundled manifest"
                );
                discovery.base_package.clone()
            }
            Err(error @ HostedMcpDiscoveryError::Permanent(_)) => {
                return Err(hosted_mcp_discovery_error(error));
            }
        };

        let _operation_guard = self.operation_lock.lock().await;
        let installation = self
            .load_installation(&extension_id, &installation_id)
            .await
            .map_err(|error| {
                tracing::debug!(
                    %error,
                    extension_id = %extension_id.as_str(),
                    installation_id = %installation_id.as_str(),
                    "hosted MCP activation could not recheck the installation after discovery"
                );
                hosted_mcp_changed_during_discovery_error()
            })?;
        // #5459 P1: the installation's owner or member set may have changed
        // while the lock was dropped for discovery (eviction+reinstall /
        // remove+reinstall reuse the same installation id), so re-check
        // ownership before committing — phase 1's check is stale. A foreign
        // row must not be flipped to Enabled under this caller's action.
        ensure_caller_may_operate(&installation, caller).map_err(|error| {
            tracing::debug!(
                %error,
                extension_id = %extension_id.as_str(),
                installation_id = %installation_id.as_str(),
                "hosted MCP activation caller ownership changed during discovery"
            );
            hosted_mcp_changed_during_discovery_error()
        })?;
        ensure_caller_may_mutate_tenant_installation(
            &installation,
            caller,
            &self.tenant_operator_user_id,
            "activate",
        )
        .map_err(|error| {
            tracing::debug!(
                %error,
                extension_id = %extension_id.as_str(),
                installation_id = %installation_id.as_str(),
                "hosted MCP activation caller is not the tenant operator after discovery"
            );
            hosted_mcp_changed_during_discovery_error()
        })?;
        let current_package = self
            .lifecycle_package(&extension_id)
            .await
            .map_err(|error| {
                tracing::debug!(
                    %error,
                    extension_id = %extension_id.as_str(),
                    "hosted MCP activation could not recheck the lifecycle package after discovery"
                );
                hosted_mcp_changed_during_discovery_error()
            })?;
        if current_package != discovery.base_package {
            return Err(hosted_mcp_changed_during_discovery_error());
        };
        if let ExtensionActivationCredentialReadiness::Missing(missing) = credential_gate
            .credential_readiness(&active_package)
            .await?
        {
            return activation_credentials_incomplete_response(package_ref, missing);
        }
        self.commit_activation(
            package_ref,
            &extension_id,
            &installation_id,
            installation.activation_state(),
            active_package,
        )
        .await
    }

    async fn commit_activation(
        &self,
        package_ref: LifecyclePackageRef,
        extension_id: &ExtensionId,
        installation_id: &ExtensionInstallationId,
        previous_state: ExtensionActivationState,
        active_package: ExtensionPackage,
    ) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
        if previous_state == ExtensionActivationState::Enabled
            && self
                .active_extensions
                .snapshot()
                .get_extension(extension_id)
                == Some(&active_package)
        {
            // Lifecycle OAuth continuation dispatch is lease-recoverable. A
            // replacement claimant can therefore arrive after the original
            // claimant already activated this exact package. Treat that state
            // as the authoritative success instead of re-publishing and
            // risking a conflicting failure followed by credential rollback.
            return Ok(activation_success_response(
                package_ref,
                &active_package,
                self.account_setups.descriptor(extension_id),
            ));
        }
        self.enable_lifecycle_package(extension_id).await?;
        if let Err(error) = self
            .installation_store
            .set_activation_state(installation_id, ExtensionActivationState::Enabled)
            .await
        {
            if let Err(rollback_error) = self.disable_lifecycle_package(extension_id).await {
                return Err(compensation_failure(
                    "extension activation failed to persist enabled state and lifecycle disable rollback failed",
                    map_extension_installation_error(error),
                    rollback_error,
                ));
            }
            return Err(map_extension_installation_error(error));
        }
        if let Err(error) = self.active_extensions.publish(&active_package) {
            if previous_state != ExtensionActivationState::Enabled
                && let Err(rollback_error) = self.disable_lifecycle_package(extension_id).await
            {
                return Err(compensation_failure(
                    "extension activation failed to publish active package and lifecycle disable rollback failed",
                    error,
                    rollback_error,
                ));
            }
            if let Err(cleanup_error) = self
                .installation_store
                .set_activation_state(installation_id, previous_state)
                .await
            {
                return Err(compensation_failure(
                    "extension activation failed to publish active package and activation restore failed",
                    error,
                    map_extension_installation_error(cleanup_error),
                ));
            }
            return Err(error);
        }
        if let Err(error) = self
            .publish_to_generic_host(extension_id, installation_id, &active_package)
            .await
        {
            // Snapshot publication failed: the activation must not report
            // success (its tools would be undispatchable). Unwind the
            // registry publish and activation state.
            if let Err(cleanup_error) = self.active_extensions.unpublish(&active_package) {
                return Err(compensation_failure(
                    "extension activation failed to publish the dispatch snapshot and registry unpublish failed",
                    error,
                    cleanup_error,
                ));
            }
            if previous_state != ExtensionActivationState::Enabled {
                // Best-effort unwind: the state restore below is the critical
                // step, so a disable failure here is logged, not propagated
                // (returning early would skip the activation-state restore).
                if let Err(cleanup_error) = self.disable_lifecycle_package(extension_id).await {
                    tracing::warn!(
                        error = %cleanup_error,
                        "failed to disable lifecycle package during activation-failure compensation"
                    );
                }
            }
            if let Err(cleanup_error) = self
                .installation_store
                .set_activation_state(installation_id, previous_state)
                .await
            {
                return Err(compensation_failure(
                    "extension activation failed to publish the dispatch snapshot and activation restore failed",
                    error,
                    map_extension_installation_error(cleanup_error),
                ));
            }
            return Err(error);
        }

        let visible_capability_ids = package_visible_capability_ids(&active_package);
        let account_setup = ironclaw_host_api::ExtensionId::new(package_ref.id.as_str())
            .ok()
            .and_then(|id| self.account_setups.descriptor(&id));
        let message = activation_success_message(
            &package_ref,
            &active_package,
            &visible_capability_ids,
            account_setup.as_ref(),
        );
        // For an inbound-channel extension, attach the structured connect
        // requirement so WebChat can render the in-chat connection panel from
        // structured state (the activation message is model guidance only).
        let connection_required = if package_declares_inbound_product_adapter(&active_package) {
            Some(channel_connection_requirement(
                package_ref.id.as_str(),
                active_package.manifest.name.as_str(),
                channel_connect_strategy(&active_package),
                account_setup.as_ref(),
            ))
        } else {
            None
        };

        let mut response = response_with_payload(
            Some(package_ref),
            InstallationState::Active,
            LifecycleProductPayload::ExtensionActivate {
                activated: true,
                visible_capability_ids,
                connection_required,
            },
        );
        response.message = Some(message);
        Ok(response)
    }

    pub async fn package_requires_hosted_mcp_discovery(
        &self,
        package_ref: &LifecyclePackageRef,
    ) -> Result<bool, ProductSurfaceFailure> {
        let (extension_id, _) = extension_ids_from_package_ref(package_ref)?;
        let _operation_guard = self.operation_lock.lock().await;
        let package = self.lifecycle_package(&extension_id).await?;
        Ok(is_hosted_http_mcp_package(&package))
    }

    /// Remove an installed extension. This is the single convergence point both
    /// removal entrypoints call — the WebUI service
    /// ([`LifecycleProductAction::ExtensionRemove`]) and the
    /// `builtin.extension_remove` agent capability — so the credential
    /// revocation below cannot be bypassed through one door.
    ///
    /// On success it revokes the removed extension's reusable personal
    /// credentials for providers now exclusive to it (see
    /// [`Self::revoke_exclusive_credentials`]).
    pub async fn remove(
        &self,
        package_ref: LifecyclePackageRef,
        scope: &ResourceScope,
        authenticated_actor_user_id: Option<&ironclaw_host_api::UserId>,
    ) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
        let (removed_extension_id, _) = extension_ids_from_package_ref(&package_ref)?;
        // Record only whether this invocation began while local removal state
        // existed. Authority is re-checked under `operation_lock`; this bit is
        // used solely to distinguish an already-absent repair request from a
        // concurrent loser whose installed target disappeared while waiting.
        let began_with_local_state = self
            .search_installation(&removed_extension_id)
            .await?
            .is_some()
            || self
                .installation_store
                .get_manifest(&removed_extension_id)
                .await
                .map_err(map_extension_installation_error)?
                .is_some();
        // Match install/import lock ordering: never await the catalog while
        // holding the global lifecycle operation lock. A missing entry is not
        // immediately fatal because an installed manifest may be the durable
        // tombstone for cleanup after catalog removal.
        let available_catalog_fallback = {
            let catalog = self.catalog.read().await;
            catalog.resolve(&package_ref)
        };
        let caller = authenticated_actor_user_id.unwrap_or(&scope.user_id);
        let mut removal_scope = scope.clone();
        if let Some(actor_user_id) = authenticated_actor_user_id {
            removal_scope.user_id = actor_user_id.clone();
        }
        let mut response = {
            let _operation_guard = self.operation_lock.lock().await;
            let extension_id = removed_extension_id.clone();
            let installation = self.search_installation(&extension_id).await?;
            if let Some(installation) = installation.as_ref() {
                ensure_caller_may_operate(installation, caller)?;
                ensure_caller_may_mutate_tenant_installation(
                    installation,
                    caller,
                    &self.tenant_operator_user_id,
                    "remove",
                )?;
            }
            let installed_manifest = self
                .installation_store
                .get_manifest(&extension_id)
                .await
                .map_err(map_extension_installation_error)?;
            if installation.is_none() && installed_manifest.is_none() && began_with_local_state {
                return Err(ProductSurfaceFailure::InvalidBindingRequest {
                    reason: format!("extension {} is not installed", extension_id.as_str()),
                });
            }
            if installation.is_some() && installed_manifest.is_none() {
                return Err(ProductSurfaceFailure::InvalidBindingRequest {
                    reason: format!(
                        "extension {} manifest is not installed",
                        extension_id.as_str()
                    ),
                });
            }
            let removal_manifest = if let Some(manifest_record) = installed_manifest.as_ref() {
                manifest_record.clone()
            } else {
                let available = available_catalog_fallback?;
                prepare_install(
                    &available,
                    derive_owner(caller, &self.tenant_operator_user_id),
                )?
                .manifest_record
            };
            let removed_providers =
                Self::removed_extension_providers_from_manifest(&removal_manifest)?;
            let cleanup_requirements = removal_manifest.removal_cleanup_requirements().to_vec();
            // §6.4: every channel surface can hold per-caller connection state.
            // OAuth channels own vendor credentials/identity bindings, while
            // proof-code channels own pairing records, identity bindings, DM
            // targets, and conversation-actor bindings. Removal runs the real
            // per-caller disconnect below while the installation still exists.
            // The generic service discovers the same manifest-derived set.
            let removes_connectable_channel = {
                let resolved = removal_manifest.resolved();
                resolved.channel.is_some()
            };
            // Deliberately validate cleanup actors only after caller
            // authorization and manifest/provider preflight. Hoisting this
            // check above the operation guard would change private-install
            // masking and concurrent error precedence.
            if (!cleanup_requirements.is_empty() || removes_connectable_channel)
                && authenticated_actor_user_id.is_none()
            {
                return Err(ProductSurfaceFailure::InvalidBindingRequest {
                    reason: "extension removal cleanup requires an authenticated actor".to_string(),
                });
            }
            if !removed_providers.is_empty() && authenticated_actor_user_id.is_none() {
                return Err(ProductSurfaceFailure::InvalidBindingRequest {
                    reason: "extension credential cleanup requires an authenticated actor"
                        .to_string(),
                });
            }
            if installed_manifest.is_none() {
                // Durable cleanup tombstone: retain the definition so an
                // interrupted cleanup stays retryable without the catalog and
                // fresh imports stay blocked until removal converges.
                self.installation_store
                    .persist_removal_tombstone(removal_manifest)
                    .await
                    .map_err(map_extension_installation_error)?;
            }
            let cleanup_context = authenticated_actor_user_id.map(|actor_user_id| {
                ExtensionRemovalCleanupContext::new(removal_scope.clone(), actor_user_id.clone())
            });
            if let Some(cleanup_context) = cleanup_context.as_ref() {
                self.removal_cleanup
                    .cleanup_requirements(&cleanup_requirements, cleanup_context)
                    .await?;
            }
            // Per-caller channel disconnect (§6.4, issue #6091 shape): run the
            // REAL disconnect — revoke the caller's personal vendor credential
            // → vendor cleanup → delete the caller's identity bindings —
            // through the same generic service the extensions page reads, so
            // connection state, durable bindings, lifecycle phase, and tool
            // dispatchability flip together on removal. Runs before teardown
            // so the installation-scoped binding prefix still resolves; a
            // failure keeps the installation authoritative and stays
            // retryable, mirroring the credential cleanup below.
            if removes_connectable_channel && let Some(actor_user_id) = authenticated_actor_user_id
            {
                // Fail closed on an empty slot: a channel surface may hold
                // per-caller OAuth or pairing state, and a composition that
                // gives this path no service to disconnect it through must not
                // report the removal as successful.
                // Surface the same typed retryable error a failing disconnect
                // does; compositions that legitimately remove channel
                // extensions fill the slot (runtime composition in
                // `build_reborn_runtime`, the channel-connection test bundle).
                let Some(channel_connection) = self.channel_disconnect_slot.get() else {
                    return Err(ProductSurfaceFailure::Transient {
                        reason: format!(
                            "channel connection cleanup is unavailable for extension {}: no \
                             channel connection service is composed; retry removal once the \
                             host wires channel connections",
                            extension_id.as_str()
                        ),
                    });
                };
                channel_connection
                    .disconnect_channel_for_caller(
                        ProductSurfaceCaller::new(
                            removal_scope.tenant_id.clone(),
                            actor_user_id.clone(),
                            removal_scope.agent_id.clone(),
                            removal_scope.project_id.clone(),
                        ),
                        extension_id.as_str(),
                    )
                    .await
                    .map_err(|error| ProductSurfaceFailure::Transient {
                        reason: format!(
                            "channel connection cleanup did not complete for extension {}: {:?}; retry removal",
                            extension_id.as_str(),
                            error.code
                        ),
                    })?;
            }
            // Actor-scoped credential cleanup completes while an installed row
            // still proves who owns the retry. The operation is idempotent.
            self.revoke_exclusive_credentials(
                &removal_scope,
                &removed_extension_id,
                &removed_providers,
                caller,
            )
            .await?;
            let lifecycle_package_present = self
                .lifecycle_service
                .lock()
                .await
                .registry()
                .get_extension(&extension_id)
                .is_some();
            let response = if installation.is_some() && lifecycle_package_present {
                self.remove_locked(package_ref.clone(), caller).await
            } else {
                if let Some(installation) = installation.as_ref() {
                    self.installation_store
                        .delete_installation(installation.installation_id())
                        .await
                        .map_err(map_extension_installation_error)?;
                }
                if let Err(error) = self.remove_orphaned_runtime_state(&extension_id).await {
                    if let Some(installation) = installation.as_ref()
                        && let Err(restore_error) = self.restore_installation(installation).await
                    {
                        return Err(compensation_failure(
                            "orphan extension cleanup failed and installation restore failed",
                            error,
                            restore_error,
                        ));
                    }
                    return Err(error);
                }
                Ok(response_with_payload(
                    Some(package_ref.clone()),
                    InstallationState::Removed,
                    LifecycleProductPayload::ExtensionRemove {
                        removed: installation.is_some(),
                    },
                ))
            }?;
            // `remove_locked` retains the manifest as a cleanup tombstone. A
            // membership-only removal leaves the shared installation in place,
            // so its manifest remains too.
            if self.search_installation(&extension_id).await?.is_none() {
                match self.installation_store.delete_manifest(&extension_id).await {
                    Ok(()) | Err(ExtensionInstallationError::ManifestNotFound { .. }) => {}
                    Err(error) => return Err(map_extension_installation_error(error)),
                }
            }
            response
        };
        if matches!(
            response.payload.as_ref(),
            Some(LifecycleProductPayload::ExtensionRemove { removed: false })
        ) {
            response.message = Some(
                "Extension was already absent; external and credential cleanup completed."
                    .to_string(),
            );
        }
        Ok(response)
    }

    /// Credential providers the extension declares, captured before removal (its
    /// manifest is gone afterward). Discovery fails closed because an empty
    /// result would otherwise bypass authenticated-actor validation and personal
    /// credential cleanup.
    fn removed_extension_providers_from_manifest(
        manifest_record: &ExtensionManifestRecord,
    ) -> Result<Vec<AuthProviderId>, ProductSurfaceFailure> {
        let manifest = manifest_record
            .manifest()
            .clone()
            .try_into()
            .map_err(map_extension_error)?;
        let requirements = manifest_runtime_credential_auth_requirements(&manifest);
        Self::removed_extension_providers_from_requirements(requirements)
    }

    fn removed_extension_providers_from_requirements(
        requirements: Vec<RuntimeCredentialAuthRequirement>,
    ) -> Result<Vec<AuthProviderId>, ProductSurfaceFailure> {
        let mut providers = Vec::new();
        for requirement in requirements {
            let provider = AuthProviderId::new(requirement.provider.as_str()).map_err(|_| {
                ProductSurfaceFailure::InvalidBindingRequest {
                    reason: "extension credential provider is invalid for cleanup".to_string(),
                }
            })?;
            if !providers.contains(&provider) {
                providers.push(provider);
            }
        }
        Ok(providers)
    }

    /// After a successful removal, revoke the removed extension's reusable
    /// personal credentials for providers now exclusive to it (no other
    /// installed extension still declares them). Cleanup failures leave the
    /// actor-owned installation authoritative and return a retryable error, so
    /// another user cannot take over the cleanup retry.
    async fn revoke_exclusive_credentials(
        &self,
        scope: &ResourceScope,
        removed_extension_id: &ExtensionId,
        removed_providers: &[AuthProviderId],
        caller: &UserId,
    ) -> Result<(), ProductSurfaceFailure> {
        let Some(cleanup) = self.credential_cleanup.as_ref() else {
            return Ok(());
        };
        // One extension-keyed cleanup ALWAYS runs, independent of the
        // provider walk below: it cancels the removed package's own connect
        // flows (even when their provider is shared with — and therefore
        // retained for — another installed extension, where a surviving flow's
        // late callback could otherwise rewrite the shared account and its
        // failure compensation then revoke it), revokes extension-OWNED
        // accounts, and strips the extension from every granted account so a
        // later reinstall cannot silently inherit stale authorization.
        let lifecycle_package = ironclaw_auth::LifecyclePackageRef::new(
            removed_extension_id.as_str(),
        )
        .map_err(|error| {
            tracing::debug!(
                %error,
                extension_id = %removed_extension_id,
                "removed extension id could not form an auth lifecycle package ref"
            );
            ProductSurfaceFailure::InvalidBindingRequest {
                reason: "extension id is not a valid lifecycle package ref for cleanup".to_string(),
            }
        })?;
        let extension_request = SecretCleanupRequest {
            scope: AuthProductScope::credential_owner(scope, AuthSurface::Callback),
            extension_id: removed_extension_id.clone(),
            provider: None,
            lifecycle_package: Some(lifecycle_package),
            action: SecretCleanupAction::Uninstall,
        };
        let report = cleanup
            .cleanup_for_lifecycle(extension_request)
            .await
            .map_err(|error| {
                tracing::debug!(
                    error_code = ?error.code,
                    extension_id = %removed_extension_id,
                    "extension removal extension-keyed cleanup failed"
                );
                ProductSurfaceFailure::Transient {
                    reason: "extension credential cleanup did not complete; retry removal"
                        .to_string(),
                }
            })?;
        if !report.quarantined_accounts.is_empty() {
            tracing::debug!(
                extension_id = %removed_extension_id,
                quarantined_accounts = report.quarantined_accounts.len(),
                "extension removal extension-keyed cleanup was incomplete"
            );
            return Err(ProductSurfaceFailure::Transient {
                reason: "extension credential cleanup was incomplete; retry removal".to_string(),
            });
        }
        if removed_providers.is_empty() {
            return Ok(());
        }
        let providers_still_in_use = self
            .providers_still_in_use(removed_extension_id, caller)
                .await
                .ok_or_else(|| ProductSurfaceFailure::Transient {
                    reason: "extension credential cleanup could not determine whether credentials are shared; retry removal"
                        .to_string(),
                })?;
        for provider in removed_providers {
            if providers_still_in_use.contains(provider) {
                // Shared with another installed extension; preserve the account.
                continue;
            }
            let request = SecretCleanupRequest {
                scope: AuthProductScope::credential_owner(scope, AuthSurface::Callback),
                extension_id: removed_extension_id.clone(),
                provider: Some(provider.clone()),
                lifecycle_package: None,
                action: SecretCleanupAction::Uninstall,
            };
            let report = cleanup.cleanup_for_lifecycle(request).await.map_err(|error| {
                tracing::debug!(
                    error_code = ?error.code,
                    %provider,
                    "extension removal credential cleanup failed"
                );
                ProductSurfaceFailure::Transient {
                    reason: format!(
                        "extension credential cleanup did not complete for provider {provider}; retry removal"
                    ),
                }
            })?;
            if !report.quarantined_accounts.is_empty() {
                tracing::debug!(
                    %provider,
                    quarantined_accounts = report.quarantined_accounts.len(),
                    "extension removal credential cleanup was incomplete"
                );
                return Err(ProductSurfaceFailure::Transient {
                    reason: format!(
                        "extension credential cleanup was incomplete for provider {provider}; retry removal"
                    ),
                });
            }
        }
        Ok(())
    }

    /// Providers still declared by extensions that remain installed after a
    /// removal. Returns `None` when the set cannot be resolved so the caller
    /// fails safe and skips revocation rather than risk deleting a shared
    /// credential.
    ///
    /// Enumeration is caller-masked: another user's private install cannot be
    /// consuming the caller's personal credential account.
    async fn providers_still_in_use(
        &self,
        removed_extension_id: &ExtensionId,
        caller: &UserId,
    ) -> Option<BTreeSet<AuthProviderId>> {
        let installations = match self.installation_store.list_installations().await {
            Ok(installations) => installations,
            Err(error) => {
                tracing::debug!(
                    %error,
                    "could not enumerate installed extensions after removal; skipping credential cleanup"
                );
                return None;
            }
        };
        let mut providers = BTreeSet::new();
        for installation in installations {
            if installation.extension_id() == removed_extension_id
                || !installation.owner().visible_to(caller)
            {
                continue;
            }
            let manifest_record = match self
                .installation_store
                .get_manifest(installation.extension_id())
                .await
            {
                Ok(Some(manifest_record)) => manifest_record,
                Ok(None) => {
                    tracing::debug!(
                        extension_id = %installation.extension_id(),
                        "remaining extension manifest missing during credential cleanup discovery"
                    );
                    return None;
                }
                Err(error) => {
                    tracing::debug!(
                        %error,
                        extension_id = %installation.extension_id(),
                        "could not load a remaining extension manifest during credential cleanup discovery"
                    );
                    return None;
                }
            };
            let requirements = match Self::removed_extension_providers_from_manifest(
                &manifest_record,
            ) {
                Ok(requirements) => requirements,
                Err(error) => {
                    tracing::debug!(
                        %error,
                        extension_id = %installation.extension_id(),
                        "could not resolve a remaining extension's credential providers; skipping credential cleanup"
                    );
                    return None;
                }
            };
            for provider in requirements {
                providers.insert(provider);
            }
        }
        Some(providers)
    }

    /// Converge a manifest-only removal tombstone that may have been left by a
    /// compensated file/installation failure. The normal successful remove has
    /// already cleared these surfaces, so every step is idempotent there.
    async fn remove_orphaned_runtime_state(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ProductSurfaceFailure> {
        let lifecycle_package = {
            self.lifecycle_service
                .lock()
                .await
                .registry()
                .get_extension(extension_id)
                .cloned()
        };
        let active_package = self
            .active_extensions
            .snapshot()
            .get_extension(extension_id)
            .cloned();
        if let Some(package) = active_package.as_ref() {
            self.active_extensions.unpublish(package)?;
        }
        if lifecycle_package.is_some()
            && let Err(error) = self.remove_lifecycle_package(extension_id).await
        {
            if let Some(package) = active_package.as_ref()
                && let Err(restore_error) = self.active_extensions.publish(package)
            {
                return Err(compensation_failure(
                    "orphan extension cleanup failed and active publication restore failed",
                    error,
                    restore_error,
                ));
            }
            return Err(error);
        }
        if let Err(error) = self.delete_materialized_extension_files(extension_id).await {
            let restore_package = lifecycle_package.as_ref().or(active_package.as_ref());
            if let Some(package) = restore_package {
                let previous_state = if active_package.is_some() {
                    ExtensionActivationState::Enabled
                } else {
                    ExtensionActivationState::Installed
                };
                if let Err(restore_error) = self
                    .restore_lifecycle_package(package, previous_state)
                    .await
                {
                    return Err(compensation_failure(
                        "orphan extension file cleanup failed and lifecycle restore failed",
                        error,
                        restore_error,
                    ));
                }
            }
            if let Some(package) = active_package.as_ref()
                && let Err(restore_error) = self.active_extensions.publish(package)
            {
                return Err(compensation_failure(
                    "orphan extension file cleanup failed and active publication restore failed",
                    error,
                    restore_error,
                ));
            }
            return Err(error);
        }
        Ok(())
    }

    /// Release a held final-member reservation by restoring the pre-remove
    /// installation aggregate; a no-op when no reservation was taken (tenant
    /// rows have no membership lease).
    async fn restore_reserved_membership(
        &self,
        reserved: bool,
        installation: &ExtensionInstallation,
    ) -> Result<(), ProductSurfaceFailure> {
        if !reserved {
            return Ok(());
        }
        self.installation_store
            .upsert_installation(installation.clone())
            .await
            .map_err(map_extension_installation_error)
    }

    async fn remove_locked(
        &self,
        package_ref: LifecyclePackageRef,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
        let (extension_id, installation_id) = extension_ids_from_package_ref(&package_ref)?;
        let installation = self
            .load_installation(&extension_id, &installation_id)
            .await?;
        ensure_caller_may_operate(&installation, caller)?;
        ensure_caller_may_mutate_tenant_installation(
            &installation,
            caller,
            &self.tenant_operator_user_id,
            "remove",
        )?;
        // Membership remove (#5459 P1 pivot): while other members still hold
        // the tool, the caller just LEAVES the member set — a single-row
        // membership tombstone, no teardown. Only the last holder's remove
        // (or the operator removing a tenant-shared tool) tears the install
        // down. The store decides final-vs-not atomically under its mutation
        // lease; `decide_remove` stays as the pure policy pre-check.
        decide_remove(installation.owner(), caller)?;
        let mut membership_reserved = false;
        if !installation.owner().is_tenant() {
            match self
                .installation_store
                .deactivate_membership(&installation_id, caller)
                .await
                .map_err(map_extension_installation_error)?
            {
                MembershipDeactivation::MembershipRemoved(updated) => {
                    if updated
                        .owner()
                        .members()
                        .is_some_and(|members| members.contains(caller))
                    {
                        return Err(ProductSurfaceFailure::Transient {
                            reason: format!(
                                "extension {} membership store returned an invalid owner projection",
                                extension_id.as_str()
                            ),
                        });
                    }
                    return Ok(response_with_payload(
                        Some(package_ref),
                        InstallationState::Removed,
                        LifecycleProductPayload::ExtensionRemove { removed: true },
                    ));
                }
                MembershipDeactivation::FinalMemberReserved => {
                    membership_reserved = true;
                }
            }
        }
        let previous_state = installation.activation_state();
        let lifecycle_package = match self.lifecycle_package(&extension_id).await {
            Ok(package) => package,
            Err(error) => {
                if let Err(restore_error) = self
                    .restore_reserved_membership(membership_reserved, &installation)
                    .await
                {
                    return Err(compensation_failure(
                        "extension remove could not load the lifecycle package and membership reservation restore failed",
                        error,
                        restore_error,
                    ));
                }
                return Err(error);
            }
        };
        // Hosted-MCP discovery can republish a package that differs from the
        // lifecycle-registered package; unpublish the active-registry package
        // and fall back only when nothing is currently active.
        let active_package_for_unpublish = self
            .active_extensions
            .snapshot()
            .get_extension(&extension_id)
            .cloned()
            .unwrap_or_else(|| lifecycle_package.clone());
        if let Err(error) = self
            .installation_store
            .set_activation_state(&installation_id, ExtensionActivationState::Disabled)
            .await
        {
            let original_error = map_extension_installation_error(error);
            if let Err(restore_error) = self
                .restore_reserved_membership(membership_reserved, &installation)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to disable activation and membership reservation restore failed",
                    original_error,
                    restore_error,
                ));
            }
            return Err(original_error);
        }
        if let Err(error) = self.remove_lifecycle_package(&extension_id).await {
            if let Err(restore_error) = self
                .restore_reserved_membership(membership_reserved, &installation)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to remove lifecycle package and membership reservation restore failed",
                    error,
                    restore_error,
                ));
            }
            if let Err(cleanup_error) = self
                .installation_store
                .set_activation_state(&installation_id, previous_state)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to remove lifecycle package and activation restore failed",
                    error,
                    map_extension_installation_error(cleanup_error),
                ));
            }
            return Err(error);
        }
        self.unpublish_from_generic_host(&extension_id).await;
        if let Err(error) = self
            .active_extensions
            .unpublish(&active_package_for_unpublish)
        {
            if let Err(restore_error) = self
                .restore_reserved_membership(membership_reserved, &installation)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to unpublish active package and membership reservation restore failed",
                    error,
                    restore_error,
                ));
            }
            if let Err(restore_error) = self
                .restore_lifecycle_package(&lifecycle_package, previous_state)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to unpublish active package and lifecycle restore failed",
                    error,
                    restore_error,
                ));
            }
            if let Err(cleanup_error) = self
                .installation_store
                .set_activation_state(&installation_id, previous_state)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to unpublish active package and activation restore failed",
                    error,
                    map_extension_installation_error(cleanup_error),
                ));
            }
            return Err(error);
        }

        if let Err(error) = self
            .installation_store
            .delete_installation(&installation_id)
            .await
        {
            let original_error = map_extension_installation_error(error);
            if let Err(restore_error) = self
                .restore_reserved_membership(membership_reserved, &installation)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to delete installation and membership reservation restore failed",
                    original_error,
                    restore_error,
                ));
            }
            if let Err(restore_error) = self
                .restore_lifecycle_package(&lifecycle_package, previous_state)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to delete installation and lifecycle restore failed",
                    original_error,
                    restore_error,
                ));
            }
            if let Err(restore_error) =
                self.restore_active_publication(&active_package_for_unpublish, previous_state)
            {
                return Err(compensation_failure(
                    "extension remove failed to delete installation and active publication restore failed",
                    original_error,
                    restore_error,
                ));
            }
            if let Err(restore_error) = self
                .installation_store
                .set_activation_state(&installation_id, previous_state)
                .await
                .map_err(map_extension_installation_error)
            {
                return Err(compensation_failure(
                    "extension remove failed to delete installation and activation restore failed",
                    original_error,
                    restore_error,
                ));
            }
            return Err(original_error);
        }
        if let Err(error) = self
            .delete_materialized_extension_files(&extension_id)
            .await
        {
            if let Err(restore_error) = self
                .restore_lifecycle_package(&lifecycle_package, previous_state)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to delete files and lifecycle restore failed",
                    error,
                    restore_error,
                ));
            }
            if let Err(restore_error) =
                self.restore_active_publication(&active_package_for_unpublish, previous_state)
            {
                return Err(compensation_failure(
                    "extension remove failed to delete files and active publication restore failed",
                    error,
                    restore_error,
                ));
            }
            if let Err(restore_error) = self.restore_installation(&installation).await {
                return Err(compensation_failure(
                    "extension remove failed to delete files and installation restore failed",
                    error,
                    restore_error,
                ));
            }
            return Err(error);
        }

        Ok(response_with_payload(
            Some(package_ref),
            InstallationState::Removed,
            LifecycleProductPayload::ExtensionRemove { removed: true },
        ))
    }

    async fn register_lifecycle_package(
        &self,
        package: &ExtensionPackage,
    ) -> Result<(), ProductSurfaceFailure> {
        let mut lifecycle = self.lifecycle_service.lock().await;
        if lifecycle.registry().get_extension(&package.id).is_some() {
            return Err(ProductSurfaceFailure::InvalidBindingRequest {
                reason: format!("extension {} is already installed", package.id.as_str()),
            });
        }
        lifecycle
            .install(package.clone())
            .await
            .map_err(map_extension_error)?;
        Ok(())
    }

    /// Fail-closed id check for the catalog import path (#5499): reject a
    /// zip-imported bundle whose id already has an installation row or manifest
    /// — a bundle cannot be swapped under live installs. The membership rules
    /// in [`install_policy::decide_install_on_existing`] apply at install
    /// time; catalog import only needs the id to be free.
    async fn ensure_not_installed(
        &self,
        extension_id: &ExtensionId,
        installation_id: &ExtensionInstallationId,
    ) -> Result<(), ProductSurfaceFailure> {
        if self
            .installation_store
            .get_installation(installation_id)
            .await
            .map_err(map_extension_installation_error)?
            .is_some()
        {
            return Err(ProductSurfaceFailure::InvalidBindingRequest {
                reason: format!("extension {} is already installed", extension_id.as_str()),
            });
        }
        if self
            .installation_store
            .get_manifest(extension_id)
            .await
            .map_err(map_extension_installation_error)?
            .is_some()
        {
            return Err(ProductSurfaceFailure::InvalidBindingRequest {
                reason: format!(
                    "extension {} is already installed; if a previous removal was interrupted, run remove again to finish its cleanup, then retry the import",
                    extension_id.as_str()
                ),
            });
        }
        Ok(())
    }

    async fn load_installation(
        &self,
        extension_id: &ExtensionId,
        installation_id: &ExtensionInstallationId,
    ) -> Result<ExtensionInstallation, ProductSurfaceFailure> {
        let installation = self
            .installation_store
            .get_installation(installation_id)
            .await
            .map_err(map_extension_installation_error)?
            .ok_or_else(|| ProductSurfaceFailure::InvalidBindingRequest {
                reason: format!("extension {} is not installed", extension_id.as_str()),
            })?;
        if installation.extension_id() != extension_id {
            return Err(ProductSurfaceFailure::InvalidBindingRequest {
                reason: format!(
                    "installation {} does not belong to extension {}",
                    installation_id.as_str(),
                    extension_id.as_str()
                ),
            });
        }
        Ok(installation)
    }

    async fn lifecycle_package(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<ExtensionPackage, ProductSurfaceFailure> {
        let lifecycle = self.lifecycle_service.lock().await;
        lifecycle
            .registry()
            .get_extension(extension_id)
            .cloned()
            .ok_or_else(|| ProductSurfaceFailure::InvalidBindingRequest {
                reason: format!("extension {} is not installed", extension_id.as_str()),
            })
    }

    async fn enable_lifecycle_package(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ProductSurfaceFailure> {
        self.lifecycle_service
            .lock()
            .await
            .enable(extension_id)
            .await
            .map_err(map_extension_error)
    }

    async fn disable_lifecycle_package(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ProductSurfaceFailure> {
        self.lifecycle_service
            .lock()
            .await
            .disable(extension_id)
            .await
            .map_err(map_extension_error)
    }

    async fn remove_lifecycle_package(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ProductSurfaceFailure> {
        self.lifecycle_service
            .lock()
            .await
            .remove(extension_id)
            .await
            .map_err(map_extension_error)
    }

    async fn rollback_lifecycle_install(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ProductSurfaceFailure> {
        let mut lifecycle = self.lifecycle_service.lock().await;
        lifecycle
            .remove(extension_id)
            .await
            .map_err(map_extension_error)
    }

    async fn restore_lifecycle_package(
        &self,
        package: &ExtensionPackage,
        previous_state: ExtensionActivationState,
    ) -> Result<(), ProductSurfaceFailure> {
        let mut lifecycle = self.lifecycle_service.lock().await;
        lifecycle
            .install(package.clone())
            .await
            .map_err(map_extension_error)?;
        match previous_state {
            ExtensionActivationState::Enabled => {
                lifecycle
                    .enable(&package.id)
                    .await
                    .map_err(map_extension_error)?;
            }
            ExtensionActivationState::Installed | ExtensionActivationState::Disabled => {
                lifecycle
                    .disable(&package.id)
                    .await
                    .map_err(map_extension_error)?;
            }
        }
        Ok(())
    }

    async fn restore_installation(
        &self,
        installation: &ExtensionInstallation,
    ) -> Result<(), ProductSurfaceFailure> {
        self.installation_store
            .upsert_installation(installation.clone())
            .await
            .map_err(map_extension_installation_error)
    }

    fn restore_active_publication(
        &self,
        package: &ExtensionPackage,
        previous_state: ExtensionActivationState,
    ) -> Result<(), ProductSurfaceFailure> {
        if previous_state == ExtensionActivationState::Enabled {
            self.active_extensions.publish(package)?;
        }
        Ok(())
    }

    async fn persist_install_plan(
        &self,
        plan: ExtensionInstallPlan,
    ) -> Result<(), ProductSurfaceFailure> {
        // One merged record commits the definition and installation together,
        // so a failed install leaves nothing behind — the manifest-orphan
        // compensation the old two-write sequence needed no longer exists.
        self.installation_store
            .upsert_manifest_and_installation(plan.manifest_record, plan.installation)
            .await
            .map_err(map_extension_installation_error)
    }

    async fn delete_materialized_extension_files(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ProductSurfaceFailure> {
        let Ok(extension_root) =
            VirtualPath::new(format!("/system/extensions/{}", extension_id.as_str()))
        else {
            return Ok(());
        };
        match self.filesystem.delete(&extension_root).await {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => {
                tracing::debug!(%error, %extension_id, "extension file removal failed");
                Err(ProductSurfaceFailure::Transient {
                    reason: "failed to remove extension files; retry removal".to_string(),
                })
            }
        }
    }
}

/// §6.5: editing `[channel.config]` while Active runs an automatic
/// deactivate → reactivate cycle through the generic host — adapters are
/// rebuilt with the new values and `activate()` revalidates them. A no-op
/// for inactive installations (activation picks the values up when it
/// runs) and for compositions without an attached generic host. Failure
/// surfaces the typed error and leaves the host record per §6.1
/// (Installed + typed last error).
#[async_trait]
impl crate::ChannelConfigReactivation for ExtensionLifecycleManager {
    async fn reactivate_if_active(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), crate::ChannelConfigReactivationError> {
        let result: Result<(), ProductSurfaceFailure> = async {
            let _operation_guard = self.operation_lock.lock().await;
            let installations = self
                .installation_store
                .list_installations()
                .await
                .map_err(map_extension_installation_error)?;
            let Some(installation) = installations
                .into_iter()
                .find(|installation| installation.extension_id() == extension_id)
            else {
                return Ok(());
            };
            if installation.activation_state() != ExtensionActivationState::Enabled {
                return Ok(());
            }
            let Some(host) = self.generic_host.get() else {
                return Ok(());
            };
            match host.deactivate(extension_id.as_str()).await {
                Ok(()) | Err(crate::LifecycleError::NotInstalled { .. }) => {}
                Err(error) => return Err(generic_host_error(error)),
            }
            let active_package = self.lifecycle_package(extension_id).await?;
            self.publish_to_generic_host(
                extension_id,
                installation.installation_id(),
                &active_package,
            )
            .await
        }
        .await;
        result.map_err(|error| crate::ChannelConfigReactivationError::new(error.to_string()))
    }
}

struct HostedMcpDiscoveryRequest {
    base_package: ExtensionPackage,
    scope: ResourceScope,
    runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
}

fn response_with_payload(
    package_ref: Option<LifecyclePackageRef>,
    phase: InstallationState,
    payload: LifecycleProductPayload,
) -> LifecycleProductResponse {
    LifecycleProductResponse {
        package_ref,
        phase,
        blockers: Vec::new(),
        message: None,
        payload: Some(payload),
    }
}

fn activation_success_response(
    package_ref: LifecyclePackageRef,
    package: &ExtensionPackage,
    account_setup: Option<ExtensionAccountSetupDescriptor>,
) -> LifecycleProductResponse {
    let visible_capability_ids = package_visible_capability_ids(package);
    let message = activation_success_message(
        &package_ref,
        package,
        &visible_capability_ids,
        account_setup.as_ref(),
    );
    let connection_required = if package_declares_inbound_product_adapter(package) {
        Some(channel_connection_requirement(
            package_ref.id.as_str(),
            package.manifest.name.as_str(),
            channel_connect_strategy(package),
            account_setup.as_ref(),
        ))
    } else {
        None
    };
    let mut response = response_with_payload(
        Some(package_ref),
        InstallationState::Active,
        LifecycleProductPayload::ExtensionActivate {
            activated: true,
            visible_capability_ids,
            connection_required,
        },
    );
    response.message = Some(message);
    response
}

fn activation_credentials_incomplete_response(
    package_ref: LifecyclePackageRef,
    missing: Vec<RuntimeCredentialAuthRequirement>,
) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
    let blockers = missing
        .iter()
        .map(|requirement| {
            LifecycleBlockerRef::new(requirement.provider.as_str()).map(|ref_id| {
                LifecycleReadinessBlocker::Credential {
                    ref_id: Some(ref_id),
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut response = response_with_payload(
        Some(package_ref),
        InstallationState::Installed,
        LifecycleProductPayload::ExtensionActivate {
            activated: false,
            visible_capability_ids: Vec::new(),
            connection_required: None,
        },
    );
    response.blockers = blockers;
    response.message = Some(
        "Extension credentials were saved; connect the remaining credential providers before activation."
            .to_string(),
    );
    Ok(response)
}

fn activation_success_message(
    package_ref: &LifecyclePackageRef,
    package: &ExtensionPackage,
    visible_capability_ids: &[String],
    account_setup: Option<&ExtensionAccountSetupDescriptor>,
) -> String {
    if package_declares_inbound_product_adapter(package) {
        if let Some(account_setup) = account_setup {
            return account_setup.activation_success_message.clone();
        }
        let display_name = package.manifest.name.as_str();
        let connection = channel_connection_requirement(
            package_ref.id.as_str(),
            display_name,
            channel_connect_strategy(package),
            None,
        );
        let connect_guidance = match connection.strategy {
            RebornChannelConnectStrategy::OAuth => format!(
                "If WebChat shows an account connection panel, tell the user to connect \
                 {display_name} via OAuth from the extension's configuration rather than \
                 pasting anything into normal chat. If the user's account is already \
                 connected, continue the user's original request."
            ),
            RebornChannelConnectStrategy::InboundProofCode
            | RebornChannelConnectStrategy::WebGeneratedCode
            | RebornChannelConnectStrategy::QrCode
            | RebornChannelConnectStrategy::AdminManagedChannels => format!(
                "If WebChat shows a channel connection panel, tell the user to open \
                 {display_name}'s app or bot, get the pairing code or connection challenge, \
                 and paste it into the connection panel rather than normal chat. If the \
                 user's account is already connected, continue the user's original request \
                 instead of asking them to pair again. Do not claim the channel can receive \
                 or send messages for the user until connection is confirmed."
            ),
        };
        return format!(
            "{display_name} is installed as a channel surface. {connect_guidance} Final \
             replies on this channel are delivered by the host's outbound delivery, never \
             by calling the extension's tools."
        );
    }
    if visible_capability_ids.is_empty() {
        return "Extension activation succeeded. No model-visible tools were published by this extension; follow any extension-specific setup or connection UI before claiming new capabilities are available.".to_string();
    }
    let mut message = String::from(
        "Extension activation succeeded and its tools are now available. No additional authorization or configuration is needed, including for write-capable tools, unless a later tool call reports auth_required. Do not ask the user for a token, OAuth, authorization, or configuration after activated=true.",
    );
    message.push_str(
        " These tools are now callable by exact name — invoke one directly with tool_call(name=\"<tool>\", arguments={ ... }), or tool_describe(name=\"<tool>\") first if you need its full schema. Do NOT call tool_search for these; you already have their names: ",
    );
    message.push_str(&visible_capability_ids.join(", "));
    message.push('.');
    message
}

// Build the structured connect requirement for an inbound channel. This is
// the single source of channel OAuth connect copy: the in-chat panel and
// the Settings channels tab both render it from the extension's channel
// surface. Any other inbound channel gets a generic proof-code prompt. NOTE:
// no backend mounts the generic proof-code redeem route — the first
// inbound channel must mount one alongside this requirement or its submit
// will 404 (see PAIRING_REDEEM_PATH in the webui pairing-api.js).
/// The discovery call's network authority: the declared hosted-MCP server
/// host only (the same ceiling the dispatch pipeline derives for the
/// connection-template capability).
fn hosted_mcp_discovery_network_policy(
    package: &ExtensionPackage,
) -> Option<ironclaw_host_api::NetworkPolicy> {
    let ironclaw_extensions::ExtensionRuntime::Mcp { url: Some(url), .. } =
        &package.manifest.runtime
    else {
        return None;
    };
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    Some(ironclaw_host_api::NetworkPolicy {
        allowed_targets: vec![ironclaw_host_api::NetworkTargetPattern {
            scheme: Some(ironclaw_host_api::NetworkScheme::Https),
            host_pattern: host.to_string(),
            port: parsed.port(),
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: None,
    })
}

fn generic_host_error(error: crate::LifecycleError) -> ProductSurfaceFailure {
    ProductSurfaceFailure::InvalidBindingRequest {
        reason: format!("generic extension host rejected the activation: {error}"),
    }
}

fn map_channel_config_error(error: crate::ChannelConfigError) -> ProductSurfaceFailure {
    tracing::warn!(error = %error, "effective extension configuration resolution failed");
    ProductSurfaceFailure::Transient {
        reason: "effective extension configuration is unavailable".to_string(),
    }
}

fn extension_ids_from_package_ref(
    package_ref: &LifecyclePackageRef,
) -> Result<(ExtensionId, ExtensionInstallationId), ProductSurfaceFailure> {
    package_ref.require_kind(LifecyclePackageKind::Extension)?;
    let extension_id = ExtensionId::new(package_ref.id.as_str().to_string()).map_err(|error| {
        ProductSurfaceFailure::InvalidBindingRequest {
            reason: error.to_string(),
        }
    })?;
    let installation_id = ExtensionInstallationId::new(extension_id.as_str().to_string())
        .map_err(map_extension_installation_error)?;
    Ok((extension_id, installation_id))
}

/// Project an installation owner into the wire-facing install scope (#5459
/// P1): tenant-owned → `shared`, user-owned → `private`. Always `Some` for an
/// existing installation; callers pass `None` when the caller has no visible
/// installation at all.
/// The single installation-state projection (§6.1): the durable activation
/// intent plus whether the host recorded a terminal activation failure.
///
/// An `Enabled` extension is `Active` when serving and `Failed` when its last
/// activation attempt recorded a redacted `last_error` (a non-auth failure at
/// activation or boot re-activation; it does not auto-retry). An extension
/// whose durable intent rolled back to `Installed` after a failed activation
/// still surfaces `Failed` while the host record carries the reason.
/// `Configured` is derived one layer up from credential readiness.
fn installation_state_for_activation(
    state: ExtensionActivationState,
    has_last_error: bool,
) -> InstallationState {
    match state {
        ExtensionActivationState::Enabled => {
            if has_last_error {
                InstallationState::Failed
            } else {
                InstallationState::Active
            }
        }
        ExtensionActivationState::Disabled => InstallationState::Disabled,
        ExtensionActivationState::Installed => {
            if has_last_error {
                InstallationState::Failed
            } else {
                InstallationState::Installed
            }
        }
    }
}

async fn search_installation_phase(
    extension: &AvailableExtensionPackage,
    installation: &ExtensionInstallation,
    credential_gate: Option<&dyn ExtensionActivationCredentialGate>,
    has_last_error: bool,
) -> Result<InstallationState, ProductSurfaceFailure> {
    let phase = installation_state_for_activation(installation.activation_state(), has_last_error);
    if phase == InstallationState::Active
        && !package_runtime_credential_auth_requirements(&extension.package).is_empty()
        && !search_credentials_configured(extension, credential_gate).await?
    {
        return Ok(InstallationState::Installed);
    }
    if phase != InstallationState::Installed {
        return Ok(phase);
    }
    if search_credentials_configured(extension, credential_gate).await? {
        return Ok(InstallationState::Configured);
    }
    Ok(phase)
}

async fn search_credentials_configured(
    extension: &AvailableExtensionPackage,
    credential_gate: Option<&dyn ExtensionActivationCredentialGate>,
) -> Result<bool, ProductSurfaceFailure> {
    let Some(credential_gate) = credential_gate else {
        return Ok(false);
    };
    Ok(matches!(
        credential_gate
            .credential_readiness(&extension.package)
            .await?,
        ExtensionActivationCredentialReadiness::Ready
    ))
}

fn suppress_search_credential_onboarding(summary: &mut LifecycleExtensionSummary) {
    summary.credential_requirements.clear();
    summary.onboarding = None;
}

fn extension_search_has_ready_result(payload: Option<&LifecycleProductPayload>) -> bool {
    let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) = payload else {
        return false;
    };
    extensions.iter().any(|extension| {
        matches!(
            extension.installation_phase,
            Some(InstallationState::Active)
        ) && !extension
            .summary
            .surface_kinds
            .contains(&CapabilitySurfaceKind::Channel)
            && extension.summary.credential_requirements.is_empty()
            && extension.summary.onboarding.is_none()
    })
}

fn extension_search_has_inactive_installed_result(
    payload: Option<&LifecycleProductPayload>,
) -> bool {
    let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) = payload else {
        return false;
    };
    extensions.iter().any(|extension| {
        matches!(
            extension.installation_phase,
            Some(
                InstallationState::Installed
                    | InstallationState::Configured
                    | InstallationState::Disabled
            )
        ) && !extension
            .summary
            .surface_kinds
            .contains(&CapabilitySurfaceKind::Channel)
            && extension.summary.credential_requirements.is_empty()
            && extension.summary.onboarding.is_none()
    })
}

fn extension_search_has_installed_external_channel_result(
    payload: Option<&LifecycleProductPayload>,
) -> bool {
    let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) = payload else {
        return false;
    };
    extensions.iter().any(|extension| {
        matches!(
            extension.installation_phase,
            Some(
                InstallationState::Installed
                    | InstallationState::Configured
                    | InstallationState::Active
            )
        ) && extension
            .summary
            .surface_kinds
            .contains(&CapabilitySurfaceKind::Channel)
    })
}

fn map_account_setup_error(error: ExtensionAccountSetupError) -> ProductSurfaceFailure {
    match error {
        ExtensionAccountSetupError::HostUnavailable { extension_id } => {
            ProductSurfaceFailure::InvalidBindingRequest {
                reason: format!(
                    "the account setup host for extension {} is not enabled on this deployment",
                    extension_id.as_str()
                ),
            }
        }
        ExtensionAccountSetupError::StatusUnavailable {
            extension_id,
            source,
        } => {
            tracing::debug!(
                extension_id = %extension_id,
                error = %source,
                "extension account connection status read failed during activation"
            );
            ProductSurfaceFailure::Transient {
                reason: format!(
                    "account connection status is temporarily unavailable for extension {}",
                    extension_id.as_str()
                ),
            }
        }
    }
}

fn map_extension_error(error: ExtensionError) -> ProductSurfaceFailure {
    match error {
        ExtensionError::Filesystem(_) | ExtensionError::LifecycleEventSink { .. } => {
            ProductSurfaceFailure::Transient {
                reason: error.to_string(),
            }
        }
        _ => ProductSurfaceFailure::InvalidBindingRequest {
            reason: error.to_string(),
        },
    }
}

fn map_extension_installation_error(error: ExtensionInstallationError) -> ProductSurfaceFailure {
    match error {
        // #4091: a store IO/backend outage is retryable backend trouble, not a
        // malformed lifecycle request — surface it in the same Transient class
        // credential-cleanup failures already use so callers retry the
        // operation instead of abandoning it.
        error @ (ExtensionInstallationError::StoreUnavailable { .. }
        | ExtensionInstallationError::MembershipMutationInProgress { .. }) => {
            ProductSurfaceFailure::Transient {
                reason: error.to_string(),
            }
        }
        error => ProductSurfaceFailure::InvalidBindingRequest {
            reason: error.to_string(),
        },
    }
}

fn project_installation_owners<I>(
    installations: I,
) -> Result<std::collections::BTreeMap<ExtensionId, InstallationOwner>, ProductSurfaceFailure>
where
    I: IntoIterator<Item = ExtensionInstallation>,
{
    let installations = canonicalize_installation_rows(installations.into_iter().collect())
        .map_err(map_extension_installation_error)?;
    let mut owners = std::collections::BTreeMap::new();
    for installation in installations {
        let extension_id = installation.extension_id().clone();
        if owners
            .insert(extension_id.clone(), installation.owner().clone())
            .is_some()
        {
            return Err(ProductSurfaceFailure::InvalidBindingRequest {
                reason: format!(
                    "duplicate extension id in lifecycle owner projection: {}",
                    extension_id.as_str()
                ),
            });
        }
    }
    Ok(owners)
}

fn ensure_caller_may_mutate_tenant_installation(
    installation: &ExtensionInstallation,
    caller: &UserId,
    tenant_operator: &UserId,
    operation: &str,
) -> Result<(), ProductSurfaceFailure> {
    if installation.owner().is_tenant() && caller != tenant_operator {
        return Err(ProductSurfaceFailure::InvalidBindingRequest {
            reason: format!(
                "extension {} is a shared tool; only the tenant admin can {operation} it",
                installation.extension_id().as_str()
            ),
        });
    }
    Ok(())
}

fn hosted_mcp_discovery_error(error: HostedMcpDiscoveryError) -> ProductSurfaceFailure {
    match error {
        HostedMcpDiscoveryError::Transient(reason) => ProductSurfaceFailure::Transient {
            reason: format!("hosted MCP discovery failed: {reason}"),
        },
        HostedMcpDiscoveryError::Permanent(reason) => {
            ProductSurfaceFailure::InvalidBindingRequest {
                reason: format!("hosted MCP discovery failed: {reason}"),
            }
        }
    }
}

fn hosted_mcp_changed_during_discovery_error() -> ProductSurfaceFailure {
    ProductSurfaceFailure::Transient {
        reason: "extension changed while hosted MCP discovery was running; retry activation"
            .to_string(),
    }
}

fn compensation_failure(
    context: &str,
    original: impl std::fmt::Display,
    compensation: impl std::fmt::Display,
) -> ProductSurfaceFailure {
    ProductSurfaceFailure::Transient {
        reason: format!(
            "{context}; original error: {original}; compensation error: {compensation}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_extensions::{
        ExtensionInstallationStore, ExtensionInstallationStorePort as _, ExtensionLifecycleService,
        ExtensionManifest, ExtensionManifestRecord, ExtensionRegistry, HostApiContractRegistry,
        ManifestSource, SharedExtensionRegistry,
    };
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{HostPortCatalog, InvocationId, ResourceScope, UserId, VirtualPath};
    use ironclaw_product::{LifecyclePackageKind, LifecyclePackageRef};
    use ironclaw_trust::{HostTrustPolicy, InvalidationBus};

    use super::*;
    use crate::{AvailableExtensionAsset, AvailableExtensionAssetContent};

    #[tokio::test]
    async fn lifecycle_manager_installs_activates_and_removes_catalog_package() {
        let package = fixture_extension_package();
        let extension_id = package.package.id.clone();
        let catalog = AvailableExtensionCatalog::from_packages(vec![package]);
        let filesystem = Arc::new(InMemoryBackend::new());
        let installation_store = Arc::new(
            ExtensionInstallationStore::load_at(
                filesystem.clone(),
                VirtualPath::new("/system/extensions/.installations/test").expect("valid root"),
                ironclaw_host_runtime::default_host_port_catalog().expect("host ports"),
                crate::product_extension_host_api_contract_registry().expect("host contracts"),
            )
            .await
            .expect("installation store"),
        );
        let lifecycle_service = Arc::new(Mutex::new(ExtensionLifecycleService::new(
            ExtensionRegistry::new(),
        )));
        let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let active_extensions = ActiveExtensionPublisher::new(
            Arc::clone(&active_registry),
            Arc::new(
                HostTrustPolicy::new(vec![Box::new(ironclaw_trust::AdminConfig::new())])
                    .expect("trust policy"),
            ),
            Arc::new(InvalidationBus::new()),
        );
        let owner = UserId::new("lifecycle-owner").expect("valid owner");
        let manager = ExtensionLifecycleManager::new(
            filesystem,
            catalog,
            installation_store.clone(),
            lifecycle_service,
            active_extensions,
            None,
            owner.clone(),
        );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("package ref");

        manager
            .install(package_ref.clone(), &owner)
            .await
            .expect("install succeeds");
        manager
            .activate_with_prechecked_credentials_for_test(
                package_ref.clone(),
                ExtensionActivationMode::Static,
            )
            .await
            .expect("activate succeeds");
        assert!(
            active_registry
                .snapshot()
                .get_extension(&extension_id)
                .is_some()
        );

        manager
            .remove(
                package_ref,
                &ResourceScope::local_default(owner.clone(), InvocationId::new())
                    .expect("valid scope"),
                Some(&owner),
            )
            .await
            .expect("remove succeeds");

        assert!(
            active_registry
                .snapshot()
                .get_extension(&extension_id)
                .is_none()
        );
        assert!(
            installation_store
                .list_installations()
                .await
                .expect("list installations")
                .is_empty()
        );
    }

    /// Joining and leaving an existing installation must route through the
    /// store's membership operations, never an aggregate rewrite — the pin
    /// for the lost-update fix: a join leaves the installation record
    /// untouched (same row version), and a non-final leave removes only the
    /// caller.
    #[tokio::test]
    async fn membership_changes_route_through_membership_operations() {
        let package = fixture_extension_package();
        let catalog = AvailableExtensionCatalog::from_packages(vec![package]);
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let installation_store = Arc::new(
            ExtensionInstallationStore::load_at(
                filesystem.clone(),
                VirtualPath::new("/system/extensions/.installations/test").expect("valid root"),
                ironclaw_host_runtime::default_host_port_catalog().expect("host ports"),
                crate::product_extension_host_api_contract_registry().expect("host contracts"),
            )
            .await
            .expect("installation store"),
        );
        let lifecycle_service = Arc::new(Mutex::new(ExtensionLifecycleService::new(
            ExtensionRegistry::new(),
        )));
        let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let active_extensions = ActiveExtensionPublisher::new(
            Arc::clone(&active_registry),
            Arc::new(
                HostTrustPolicy::new(vec![Box::new(ironclaw_trust::AdminConfig::new())])
                    .expect("trust policy"),
            ),
            Arc::new(InvalidationBus::new()),
        );
        let alice = UserId::new("alice").expect("valid user");
        let bob = UserId::new("bob").expect("valid user");
        let manager = ExtensionLifecycleManager::new(
            Arc::clone(&filesystem),
            catalog,
            installation_store.clone(),
            lifecycle_service,
            active_extensions,
            None,
            alice.clone(),
        );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("package ref");

        manager
            .install(package_ref.clone(), &alice)
            .await
            .expect("alice installs");
        let record_root =
            VirtualPath::new("/system/extensions/.installations/test/v2/installations")
                .expect("valid prefix");
        let before = filesystem
            .query(
                &record_root,
                &ironclaw_filesystem::Filter::All,
                ironclaw_filesystem::Page::first(10),
            )
            .await
            .expect("record query");
        assert_eq!(before.len(), 1);

        manager
            .install(package_ref.clone(), &bob)
            .await
            .expect("bob joins");
        let after = filesystem
            .query(
                &record_root,
                &ironclaw_filesystem::Filter::All,
                ironclaw_filesystem::Page::first(10),
            )
            .await
            .expect("record query");
        assert_eq!(
            after[0].version, before[0].version,
            "a join must not rewrite the installation record"
        );
        let installation_id =
            ExtensionInstallationId::new("fixture").expect("valid installation id");
        let joined = installation_store
            .get_installation(&installation_id)
            .await
            .expect("installation lookup")
            .expect("installation present");
        assert!(
            joined
                .owner()
                .members()
                .expect("member owned")
                .contains(&bob)
        );

        let response = manager
            .remove(
                package_ref.clone(),
                &ResourceScope::local_default(alice.clone(), InvocationId::new())
                    .expect("valid scope"),
                Some(&alice),
            )
            .await
            .expect("alice leaves");
        assert!(matches!(
            response.payload,
            Some(LifecycleProductPayload::ExtensionRemove { removed: true })
        ));
        let remaining = installation_store
            .get_installation(&installation_id)
            .await
            .expect("installation lookup")
            .expect("installation still present");
        assert_eq!(
            remaining.owner().members().expect("member owned"),
            &std::collections::BTreeSet::from([bob.clone()]),
            "a non-final leave removes only the caller"
        );

        manager
            .remove(
                package_ref,
                &ResourceScope::local_default(bob.clone(), InvocationId::new())
                    .expect("valid scope"),
                Some(&bob),
            )
            .await
            .expect("bob's final remove tears down");
        assert!(
            installation_store
                .get_installation(&installation_id)
                .await
                .expect("installation lookup")
                .is_none()
        );
    }

    fn capability_provider_contracts() -> HostApiContractRegistry {
        let mut contracts = HostApiContractRegistry::new();
        contracts
            .register(Arc::new(
                ironclaw_extensions::CapabilityProviderHostApiContract::new()
                    .expect("capability provider contract"),
            ))
            .expect("register capability provider contract");
        contracts
    }

    fn fixture_extension_package() -> AvailableExtensionPackage {
        let manifest_toml = r#"
schema_version = "reborn.extension_manifest.v2"
id = "fixture"
name = "Fixture Extension"
version = "0.1.0"
description = "Lifecycle fixture extension"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/fixture.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "fixture.search"
description = "Search fixture data"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"
"#;
        let contracts = capability_provider_contracts();
        let manifest = ExtensionManifest::parse(
            manifest_toml,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
            &contracts,
        )
        .expect("fixture manifest");
        let root = VirtualPath::new("/system/extensions/fixture").expect("extension root");
        let resolved_manifest = Arc::new(
            ExtensionManifestRecord::from_toml(
                manifest_toml,
                ManifestSource::HostBundled,
                &HostPortCatalog::empty(),
                None,
                &contracts,
                Some(root.clone()),
            )
            .expect("resolved fixture manifest")
            .resolved()
            .clone(),
        );
        let package = ExtensionPackage::from_manifest_toml(manifest, root, manifest_toml)
            .expect("fixture package");
        AvailableExtensionPackage {
            package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
                .expect("fixture package ref"),
            manifest_toml: manifest_toml.to_string(),
            resolved_manifest,
            source: ManifestSource::HostBundled,
            package,
            cleanup_requirements: Vec::new(),
            surface_kinds: Vec::new(),
            channel_directions: None,
            channel_presentation: None,
            assets: vec![
                AvailableExtensionAsset {
                    path: "manifest.toml".to_string(),
                    content: AvailableExtensionAssetContent::Bytes(
                        manifest_toml.as_bytes().to_vec(),
                    ),
                },
                AvailableExtensionAsset {
                    path: "wasm/fixture.wasm".to_string(),
                    content: AvailableExtensionAssetContent::Bytes(b"\0asm\x01\0\0\0".to_vec()),
                },
            ],
            onboarding_override: None,
            oauth_setup_override: None,
            search_aliases: Vec::new(),
        }
    }
}
