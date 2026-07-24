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
use ironclaw_extension_host::activation_transaction::{
    ExtensionActivationOperations, ExtensionActivationTransactionResult, HostedMcpDiscoveryOutcome,
    run_extension_activation,
};
use ironclaw_extensions::{
    CapabilityVisibility, ExtensionError, ExtensionInstallation, ExtensionInstallationError,
    ExtensionInstallationId, ExtensionInstallationPersistedParts, ExtensionInstallationStorePort,
    ExtensionLifecycleService, ExtensionManifestRecord, ExtensionManifestRef, ExtensionPackage,
    InstallationOwner, ManifestHash, ManifestSource, canonicalize_installation_rows,
};
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, CapabilitySurfaceKind, EffectKind, ExtensionId,
    InstallationState, NetworkTargetPattern, PermissionMode, ProductSurfaceCaller,
    ProductSurfaceError, ResourceScope, RuntimeCredentialAuthRequirement,
    RuntimeCredentialRequirement, RuntimeHttpEgress, UserId, VendorId, VirtualPath,
    sha256_digest_token,
};
use ironclaw_product::adapter_registry::PRODUCT_ADAPTER_HOST_API_ID;
use ironclaw_product::{
    ChannelConnectionFacade, ChannelConnectionRequirement, ExtensionAccountSetupDescriptor,
    ExtensionAccountSetupError, ExtensionAccountSetupRegistry, LifecycleBlockerRef,
    LifecycleExtensionSummary, LifecycleInstalledExtensionSummary, LifecyclePackageKind,
    LifecyclePackageRef, LifecycleProductPayload, LifecycleProductResponse, LifecyclePublicState,
    LifecycleReadinessBlocker, LifecycleSearchExtensionSummary, ProductWorkflowError,
};
use tokio::sync::{Mutex, RwLock, Semaphore};

use crate::RebornProductAuthServices;
use crate::extension_host::host_api_contracts::product_extension_host_api_contract_registry;
use crate::extension_host::unzip_extension_bundle;

/// Narrow lifecycle-cleanup port over product-auth so extension removal can
/// revoke the removed extension's exclusively-owned reusable credential without
/// depending on the whole product-auth bundle (and so tests can record the
/// issued cleanup). Production forwards to the guardrail-sanctioned
/// [`RebornProductAuthServices::cleanup_credentials_for_lifecycle`]. This is the
/// single convergence point for both removal entrypoints (the WebUI facade and
/// the `builtin.extension_remove` agent capability), so revocation cannot be
/// bypassed through one door.
#[async_trait]
pub(crate) trait ExtensionCredentialCleanup: Send + Sync {
    async fn cleanup_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, ProductSurfaceError>;
}

#[async_trait]
impl ExtensionCredentialCleanup for RebornProductAuthServices {
    async fn cleanup_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, ProductSurfaceError> {
        RebornProductAuthServices::cleanup_credentials_for_lifecycle(self, request)
            .await
            .map_err(|error| {
                ProductSurfaceError::internal_from(format!(
                    "extension credential cleanup failed: {:?}",
                    error.code
                ))
            })
    }
}

mod active_publication;
#[cfg(test)]
pub(crate) mod hosted_mcp_test_support;
mod install_policy;

use crate::extension_host::available_extensions::{
    AvailableExtensionCatalog, AvailableExtensionPackage, imported_extension_package,
    materialize_available_extension, visible_capability_ids,
};
use crate::extension_host::extension_activation_credentials::{
    ExtensionActivationCredentialGate, ExtensionActivationCredentialReadiness,
    RuntimeExtensionActivationCredentialGate, UnavailableExtensionActivationCredentialGate,
};
use crate::extension_host::extension_credential_requirements::{
    manifest_runtime_credential_auth_requirements, package_runtime_credential_auth_requirements,
};
use crate::extension_host::extension_removal_cleanup::{
    ExtensionRemovalCleanupContext, ExtensionRemovalCleanupRegistry,
};
use crate::extension_host::lifecycle::response_with_payload;
use crate::extension_host::mcp_discovery::{
    HostedMcpDiscoveryError, discover_hosted_mcp_package, is_hosted_http_mcp_package,
};
use crate::product_auth::credentials::runtime_credentials::RuntimeCredentialAccountSelectionService;

pub(crate) use active_publication::ActiveExtensionPublisher;
#[cfg(test)]
use active_publication::extension_trust_policy_input;
use install_policy::{derive_owner, ensure_caller_may_operate, install_scope_for_owner};

/// Owner mode reused by the lifecycle capability and facade assembly; hosted
/// discovery and static activation tests below exercise both variants.
pub(crate) use ironclaw_extension_host::activation_transaction::ExtensionActivationMode;

const RETIRED_SLACK_USER_EXTENSION_ID: &str = "slack_user";

// This port is deliberately scoped to LocalSingleUser composition. The
// lifecycle service models the installed extension set, while active_registry
// is the model-visible capability surface read by host runtime dispatch.
// install/remove keep the lifecycle set durable; internal readiness
// reconciliation and final removal mirror lifecycle-managed packages into or
// out of active_registry. Caller membership/readiness is stored separately
// from the shared runtime publication.
pub(crate) struct ExtensionManagementPort {
    filesystem: Arc<dyn RootFilesystem>,
    catalog: Arc<RwLock<AvailableExtensionCatalog>>,
    installation_store: Arc<dyn ExtensionInstallationStorePort>,
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
    generic_host: std::sync::OnceLock<Arc<ironclaw_extension_host::ExtensionHost>>,
    /// Late-bound weak reference to the effective administrator-configuration
    /// resolver used when publishing channel adapters.
    admin_configuration: std::sync::OnceLock<
        Weak<
            crate::extension_host::admin_configuration::ComposedExtensionAdminConfigurationResolver,
        >,
    >,
    // Late-attached with `generic_host` (both need the fully wired host
    // runtime): stages hosted-MCP discovery authority — the connection
    // credential and the server network policy — under the discovery scope.
    // Discovery runs at activation, outside the dispatch obligation
    // pipeline, so nothing else stages these (the pre-P2 gap that made
    // live `tools/list` always fail transient and fall back).
    discovery_runtime_ports:
        std::sync::OnceLock<ironclaw_host_runtime::ProductAuthProviderRuntimePorts>,
    /// Bounds concurrent zip decode/validation in `import_bundle`. Each decode
    /// may expand up to [`crate::extension_host::extension_bundle::MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES`] into
    /// memory, so without a bound N concurrent operator uploads turn the
    /// per-request cap into N x 64 MiB of pressure before any lifecycle lock
    /// applies (#5499 review finding #3).
    import_decode_semaphore: Arc<Semaphore>,
    removal_cleanup: Arc<ExtensionRemovalCleanupRegistry>,
    /// Late-binding slot for the generic per-user channel-connection facade
    /// (extension-runtime §6.4), shared with
    /// the runtime `channel_disconnect_slot`. Removing
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
    channel_disconnect_slot: Arc<std::sync::OnceLock<Arc<dyn ChannelConnectionFacade>>>,
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
    provider_instance_readiness: std::collections::BTreeSet<VendorId>,
}

/// Concurrent `import_bundle` decodes allowed before further uploads wait.
/// 2 x [`crate::extension_host::extension_bundle::MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES`] caps worst-case decode
/// memory at 128 MiB; imports are a rare admin-only operation, so waiting is
/// the right trade against unbounded memory.
const MAX_CONCURRENT_IMPORT_DECODES: usize = 2;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ActiveExtensionCapability {
    pub(crate) id: CapabilityId,
    pub(crate) provider: ExtensionId,
    pub(crate) effects: Vec<EffectKind>,
    pub(crate) default_permission: PermissionMode,
    pub(crate) runtime_credentials: Vec<RuntimeCredentialRequirement>,
    /// Manifest-declared network egress allowlist, independent of credentials.
    pub(crate) network_targets: Vec<NetworkTargetPattern>,
    /// Manifest-declared per-capability egress cap (bytes), applied to the
    /// minted `NetworkPolicy.max_egress_bytes`. `None` = no cap.
    pub(crate) max_egress_bytes: Option<u64>,
    /// Who the providing extension's installation belongs to (#5459 P1).
    /// Tenant-owned capabilities are grant-minted for every user; user-owned
    /// ones only for their owner (filtered in `ExtensionCapabilitySurface`).
    pub(crate) owner: InstallationOwner,
}

impl ActiveExtensionCapability {
    fn from_descriptor(descriptor: &CapabilityDescriptor, owner: InstallationOwner) -> Self {
        Self {
            id: descriptor.id.clone(),
            provider: descriptor.provider.clone(),
            effects: descriptor.effects.clone(),
            default_permission: descriptor.default_permission,
            runtime_credentials: descriptor.runtime_credentials.clone(),
            network_targets: descriptor.network_targets.clone(),
            max_egress_bytes: descriptor.max_egress_bytes,
            owner,
        }
    }
}

pub(crate) async fn restore_extension_lifecycle_state(
    catalog: &AvailableExtensionCatalog,
    filesystem: &Arc<dyn RootFilesystem>,
    installation_store: &Arc<dyn ExtensionInstallationStorePort>,
    lifecycle_service: &Arc<Mutex<ExtensionLifecycleService>>,
    active_extensions: &ActiveExtensionPublisher,
    tenant_operator_user_id: &UserId,
) -> Result<(), ProductWorkflowError> {
    for installation in
        canonicalize_persisted_installation_rows(installation_store, tenant_operator_user_id)
            .await?
    {
        if remove_retired_internal_installation(installation_store, &installation).await? {
            continue;
        }
        let package_ref = LifecyclePackageRef::new(
            LifecyclePackageKind::Extension,
            installation.extension_id().as_str(),
        )?;
        // A row whose extension id the catalog does not (yet) materialize a
        // package for — e.g. a placeholder row written by the standalone
        // v1->Reborn migration tool ahead of catalog package materialization
        // — must not abort restore for every other installation (#5499
        // review). `resolve`'s only realistic failure here is "not found";
        // skip and keep the row (never delete/rewrite persisted state) so it
        // restores once the catalog gains the package.
        let available = match catalog.resolve(&package_ref) {
            Ok(available) => available,
            Err(error) => {
                tracing::warn!(
                    extension_id = installation.extension_id().as_str(),
                    installation_id = installation.installation_id().as_str(),
                    %error,
                    "skipping extension installation restore: not available in the catalog"
                );
                continue;
            }
        };
        if let Err(hash_error) = validate_restored_manifest_hash(&installation, &available) {
            migrate_host_bundled_manifest_hash(
                installation_store,
                &available,
                &installation,
                hash_error,
            )
            .await?;
        }
        materialize_available_extension(filesystem.as_ref(), &available).await?;
        {
            let mut lifecycle = lifecycle_service.lock().await;
            lifecycle
                .install(available.package.clone())
                .await
                .map_err(map_extension_error)?;
            // Hosted MCP tools are live-discovered per authenticated caller.
            // Their catalog is not durable, so boot must not claim readiness
            // by publishing the bundled connection template as active. An
            // idempotent install/setup retry performs discovery again.
            if !is_hosted_http_mcp_package(&available.package) {
                lifecycle
                    .enable(&available.package.id)
                    .await
                    .map_err(map_extension_error)?;
            }
        }
        if !is_hosted_http_mcp_package(&available.package) {
            active_extensions.publish(&available.package)?;
        }
    }
    Ok(())
}

async fn canonicalize_persisted_installation_rows(
    installation_store: &Arc<dyn ExtensionInstallationStorePort>,
    tenant_operator_user_id: &UserId,
) -> Result<Vec<ExtensionInstallation>, ProductWorkflowError> {
    let persisted = installation_store
        .list_installations()
        .await
        .map_err(map_extension_installation_error)?;
    // Narrow each legacy tenant-visible row before grouping. Canonicalizing
    // first would let the legacy variant take precedence and discard explicit
    // user memberships from sibling rows for the same extension.
    let caller_scoped = persisted
        .iter()
        .cloned()
        .map(|installation| {
            if installation.owner().is_tenant() {
                Ok(installation
                    .with_owner(InstallationOwner::user(tenant_operator_user_id.clone())))
            } else {
                Ok(installation)
            }
        })
        .collect::<Result<Vec<_>, ProductWorkflowError>>()?;
    let canonical =
        canonicalize_installation_rows(caller_scoped).map_err(map_extension_installation_error)?;
    if persisted == canonical {
        return Ok(canonical);
    }

    for installation in &canonical {
        installation_store
            .upsert_installation(installation.clone())
            .await
            .map_err(map_extension_installation_error)?;
    }

    let canonical_ids = canonical
        .iter()
        .map(|installation| installation.installation_id().clone())
        .collect::<BTreeSet<_>>();
    for installation in persisted {
        if canonical_ids.contains(installation.installation_id()) {
            continue;
        }
        installation_store
            .delete_installation(installation.installation_id())
            .await
            .map_err(map_extension_installation_error)?;
    }

    Ok(canonical)
}

async fn remove_retired_internal_installation(
    installation_store: &Arc<dyn ExtensionInstallationStorePort>,
    installation: &ExtensionInstallation,
) -> Result<bool, ProductWorkflowError> {
    if installation.extension_id().as_str() != RETIRED_SLACK_USER_EXTENSION_ID {
        return Ok(false);
    }

    tracing::info!(
        extension_id = installation.extension_id().as_str(),
        installation_id = installation.installation_id().as_str(),
        "removing retired internal extension installation during lifecycle restore"
    );
    installation_store
        .delete_installation(installation.installation_id())
        .await
        .map_err(map_extension_installation_error)?;
    match installation_store
        .delete_manifest(installation.extension_id())
        .await
    {
        Ok(()) | Err(ExtensionInstallationError::ManifestNotFound { .. }) => {}
        Err(error) => return Err(map_extension_installation_error(error)),
    }
    Ok(true)
}

impl ExtensionManagementPort {
    pub(crate) fn new(
        filesystem: Arc<dyn RootFilesystem>,
        catalog: AvailableExtensionCatalog,
        installation_store: Arc<dyn ExtensionInstallationStorePort>,
        lifecycle_service: Arc<Mutex<ExtensionLifecycleService>>,
        active_extensions: ActiveExtensionPublisher,
        credential_cleanup: Option<Arc<dyn ExtensionCredentialCleanup>>,
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
            admin_configuration: std::sync::OnceLock::new(),
            discovery_runtime_ports: std::sync::OnceLock::new(),
            import_decode_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_IMPORT_DECODES)),
            removal_cleanup: Arc::new(ExtensionRemovalCleanupRegistry::empty()),
            account_setups: ExtensionAccountSetupRegistry::default(),
            channel_disconnect_slot: Arc::new(std::sync::OnceLock::new()),
            provider_instance_readiness: std::collections::BTreeSet::new(),
        }
    }

    /// Attach the staging ports hosted-MCP discovery uses to make its
    /// authority available under the discovery scope.
    pub(crate) fn attach_discovery_runtime_ports(
        &self,
        ports: ironclaw_host_runtime::ProductAuthProviderRuntimePorts,
    ) {
        let _ = self.discovery_runtime_ports.set(ports);
    }

    /// Stage the hosted-MCP connection credential and server network policy
    /// for the discovery call. Best-effort by design: a staging failure
    /// leaves discovery to fail transient and readiness stays incomplete. A
    /// successful stage lets live `tools/list` run with the same injected
    /// authority a dispatched invocation would carry. Bundled declarations
    /// are never substituted for a failed live catalog. The returned guard
    /// revokes every staged handoff on success, error, or cancellation.
    async fn stage_hosted_mcp_discovery_authority(
        &self,
        scope: &ResourceScope,
        package: &ExtensionPackage,
        network_policy: ironclaw_host_api::NetworkPolicy,
    ) -> Option<ironclaw_host_runtime::ProductAuthRuntimeHandoffGuard> {
        let ports = self.discovery_runtime_ports.get()?;
        let descriptor = package.capabilities.first()?;
        let authority = ports.staged_handoff_guard(scope.clone(), descriptor.id.clone());
        ports.stage_network_policy_once(scope, &descriptor.id, network_policy);
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
                    "hosted MCP discovery credential staging failed; readiness remains incomplete until live discovery succeeds"
                );
            }
        }
        Some(authority)
    }

    /// The durable installation store handle (the generic host hydrates its
    /// working set from it at boot).
    pub(crate) fn installation_store_handle(&self) -> Arc<dyn ExtensionInstallationStorePort> {
        Arc::clone(&self.installation_store)
    }

    /// Attach the generic extension host so lifecycle mutations publish the
    /// active snapshot the dispatch chain resolves from.
    pub(crate) fn attach_generic_host(&self, host: Arc<ironclaw_extension_host::ExtensionHost>) {
        let _ = self.generic_host.set(host);
    }

    pub(crate) fn attach_admin_configuration(
        &self,
        admin_configuration: &Arc<
            crate::extension_host::admin_configuration::ComposedExtensionAdminConfigurationResolver,
        >,
    ) {
        let _ = self
            .admin_configuration
            .set(Arc::downgrade(admin_configuration));
    }

    /// The attached generic host, when this facade has one — the snapshot
    /// authority the channel host assembly reconciles against.
    pub(crate) fn generic_host(&self) -> Option<Arc<ironclaw_extension_host::ExtensionHost>> {
        self.generic_host.get().cloned()
    }

    /// Reconcile the shared runtime publication after manifest-declared
    /// administrator configuration changes. Membership remains the only
    /// persisted installation state; this refresh never mutates a user's
    /// lifecycle projection. The generic host builds and activates the next
    /// generation before its atomic snapshot replacement, so the prior
    /// generation remains dispatchable if refresh fails.
    pub(crate) async fn reconcile_runtime_after_admin_configuration(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ProductWorkflowError> {
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
        if self.generic_host.get().is_none() {
            return Ok(());
        }
        let active_package = self.lifecycle_package(extension_id).await?;
        self.publish_to_generic_host(
            extension_id,
            installation.installation_id(),
            &active_package,
        )
        .await
    }

    /// Mirror an activation into the generic host's snapshot. Runs after the
    /// registry publish succeeded. Composition assembles the effective host
    /// record; the generic host owns candidate validation, refresh retention,
    /// and the atomic generation swap. A failure here fails the activation
    /// (the caller compensates) because extension dispatch resolves from the
    /// host snapshot.
    async fn publish_to_generic_host(
        &self,
        extension_id: &ExtensionId,
        installation_id: &ExtensionInstallationId,
        active_package: &ExtensionPackage,
    ) -> Result<(), ProductWorkflowError> {
        let Some(host) = self.generic_host.get() else {
            return Ok(());
        };
        let base = self
            .installation_store
            .get_manifest(extension_id)
            .await
            .map_err(map_extension_installation_error)?
            .ok_or_else(|| ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "extension {} manifest is not installed",
                    extension_id.as_str()
                ),
            })?;
        let effective = crate::extension_host::generic_host::effective_resolved_for_package(
            base.resolved(),
            active_package,
        );
        // Authorized administrator values ride the published runtime record
        // so channel adapters see the current tenant configuration.
        let config = match self.admin_configuration.get().and_then(Weak::upgrade) {
            Some(admin_configuration) => admin_configuration
                .effective_non_secret_config(extension_id)
                .await
                .map_err(map_extension_admin_configuration_error)?,
            None => Vec::new(),
        };
        let record = ironclaw_extension_host::InstallationRecord {
            extension_id: extension_id.as_str().to_string(),
            installation_id: installation_id.as_str().to_string(),
            state: ironclaw_extension_host::InstallationState::Installed,
            resolved: Arc::new(effective),
            config,
            last_error: None,
        };
        host.publish_candidate(record)
            .await
            .map_err(generic_host_error)
    }

    /// Test-support twin of the production activation choke point: publish a
    /// bundled package directly into the registry AND mirror it into the
    /// generic host's snapshot (mirrors the owner-side activation transaction's
    /// active/runtime publication, without the durable install/credential legs).
    /// Direct registry publication alone would leave the package undispatchable
    /// now that extension dispatch resolves from the snapshot.
    /// Administrator values are not seeded here; this seam consumes the same
    /// authorized Admin Configuration projection as the production publish
    /// path.
    #[cfg(feature = "test-support")]
    pub(crate) async fn publish_bundled_package_for_test(
        &self,
        package: &ExtensionPackage,
        resolved: Option<&ironclaw_extensions::ResolvedExtensionManifest>,
    ) -> Result<(), ProductWorkflowError> {
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
                        ProductWorkflowError::InvalidBindingRequest {
                            reason: format!(
                                "host port catalog rejected bundled extension: {error}"
                            ),
                        }
                    })?;
                let contracts =
                    product_extension_host_api_contract_registry().map_err(|error| {
                        ProductWorkflowError::InvalidBindingRequest {
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
                )
                .map_err(|error| ProductWorkflowError::InvalidBindingRequest {
                    reason: format!("bundled extension manifest is invalid: {error}"),
                })?
                .resolved()
                .clone()
            }
        };
        let effective =
            crate::extension_host::generic_host::effective_resolved_for_package(&base, package);
        // This shortcut deliberately publishes without creating a durable
        // installation. A tool-only package has no channel configuration to
        // resolve, and asking the attached configuration consumer to load its
        // absent installed manifest would make the test-support seam fail
        // before the tool surface can be published.
        let config = match (
            effective.channel.is_some(),
            self.admin_configuration.get().and_then(Weak::upgrade),
        ) {
            (false, _) => Vec::new(),
            (true, Some(admin_configuration)) => admin_configuration
                .effective_non_secret_config(&package.id)
                .await
                .map_err(map_extension_admin_configuration_error)?,
            (true, None) => Vec::new(),
        };
        host.publish_candidate(ironclaw_extension_host::InstallationRecord {
            extension_id: package.id.as_str().to_string(),
            installation_id: format!("{}-test-install", package.id.as_str()),
            state: ironclaw_extension_host::InstallationState::Installed,
            resolved: Arc::new(effective),
            config,
            last_error: None,
        })
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
            Ok(()) | Err(ironclaw_extension_host::LifecycleError::NotInstalled { .. }) => {}
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

    pub(crate) fn with_account_setup_registry(
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
    pub(crate) fn with_provider_instance_readiness(
        mut self,
        provider_instance_readiness: std::collections::BTreeSet<VendorId>,
    ) -> Self {
        self.provider_instance_readiness = provider_instance_readiness;
        self
    }

    pub(crate) fn with_removal_cleanup_registry(
        mut self,
        removal_cleanup: Arc<ExtensionRemovalCleanupRegistry>,
    ) -> Self {
        self.removal_cleanup = removal_cleanup;
        self
    }

    /// Share the composition's late-binding channel-connection facade slot
    /// (see the field doc). Composition passes the SAME `Arc` stored on
    /// runtime services so a fill by runtime composition (or the
    /// channel-connection test bundle) is visible to the removal path here.
    pub(crate) fn with_channel_disconnect_slot(
        mut self,
        slot: Arc<std::sync::OnceLock<Arc<dyn ChannelConnectionFacade>>>,
    ) -> Self {
        self.channel_disconnect_slot = slot;
        self
    }

    /// Test-support access to the extension installation store.
    ///
    /// Mirrors the `installation_store` field that `build_local_runtime` wires
    /// in when constructing `ExtensionManagementPort`. For tests
    /// only — zero bytes shipped in production builds.
    #[cfg(feature = "test-support")]
    pub(crate) fn installation_store_for_test(&self) -> Arc<dyn ExtensionInstallationStorePort> {
        Arc::clone(&self.installation_store)
    }

    /// C-JOURNEY: test-support access to the active-extension publisher
    /// (registry + trust policy). The idempotent install transition ultimately
    /// delegates its model-visible-surface mutation to
    /// `self.active_extensions.publish(..)` after its setup gates are ready;
    /// this accessor reaches that same publish step directly so a test harness
    /// can make a bundled first-party WASM package genuinely dispatchable.
    /// Tests only — zero bytes shipped in production builds.
    #[cfg(feature = "test-support")]
    #[cfg(test)]
    pub(crate) fn active_extensions_for_test(&self) -> &ActiveExtensionPublisher {
        &self.active_extensions
    }

    pub(crate) async fn search(
        &self,
        query: &str,
        credential_gate: Option<&RuntimeExtensionActivationCredentialGate>,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
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
                "Search found external channel results whose personal setup is incomplete. For an explicit connect, pair, authenticate, or account-access request, call builtin.extension_install for the matching extension id so the manifest-declared connection flow can continue. For routine, trigger, or notification delivery, prefer the configured outbound delivery target when one is available."
                    .to_string(),
            );
        } else if extension_search_has_inactive_installed_result(response.payload.as_ref()) {
            response.message = Some(
                "Search found extension results whose setup is incomplete. Any visible_capability_ids on those results are catalog capabilities only, not currently callable tools. Call builtin.extension_install for the matching extension id to continue the manifest-declared setup; activation is an internal checkpoint, not a separate user step."
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

    pub(crate) async fn list_installed(
        &self,
        caller: &UserId,
        credential_gate: Option<&RuntimeExtensionActivationCredentialGate>,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let summaries = self.installed_summaries(caller, credential_gate).await?;
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

    pub(crate) async fn project(
        &self,
        package_ref: LifecyclePackageRef,
        caller: &UserId,
        credential_gate: Option<&RuntimeExtensionActivationCredentialGate>,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
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
        // A not-installed package projects the public `uninstalled` state.
        // `Removed` is the host's internal action signal that serializes to
        // that product vocabulary; no staged/installed checkpoint may leak.
        let phase = match installation.as_ref() {
            Some(installation) => {
                let available = {
                    let catalog = self.catalog.read().await;
                    catalog.resolve(&package_ref)?
                };
                self.caller_installation_phase(
                    &available,
                    installation,
                    credential_gate,
                    caller,
                    activation_errors.contains_key(installation.extension_id().as_str()),
                )
                .await?
            }
            None => InstallationState::Removed,
        };
        let install_scope = installation
            .as_ref()
            .map(|installation| install_scope_for_owner(installation.owner()));
        let available = {
            let catalog = self.catalog.read().await;
            catalog.resolve(&package_ref)?
        };
        let summary = self.summary_for_phase(&available, phase)?;
        let public_phase = LifecyclePublicState::from_host_checkpoint(phase);
        Ok(response_with_payload(
            Some(package_ref),
            phase,
            LifecycleProductPayload::ExtensionList {
                extensions: vec![LifecycleInstalledExtensionSummary {
                    summary,
                    phase: public_phase,
                    install_scope,
                }],
                count: 1,
            },
        ))
    }

    pub(crate) async fn active_model_visible_capabilities(
        &self,
    ) -> Result<Vec<ActiveExtensionCapability>, ProductWorkflowError> {
        // Carry each installed extension's membership onto its shared runtime
        // capabilities so the per-request grant minting in the local-dev
        // capability surface can filter them to members. Readiness remains a
        // caller-scoped derived check; the registry itself stays global.
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

    /// Resolve the exact extension ids callable by this caller.
    ///
    /// This is the authority-side projection used by capability surfaces. It
    /// deliberately reuses [`Self::caller_installation_phase`] rather than
    /// deriving readiness from membership or the provider-global active
    /// registry alone: personal account setup, runtime credentials, recorded
    /// activation failure, and runtime publication must all agree.
    pub(crate) async fn caller_active_extension_ids(
        &self,
        caller: &UserId,
        credential_gate: Option<&RuntimeExtensionActivationCredentialGate>,
    ) -> Result<BTreeSet<ExtensionId>, ProductWorkflowError> {
        let installations = self
            .installation_store
            .list_installations()
            .await
            .map_err(map_extension_installation_error)?;
        let activation_errors = self.installation_activation_errors().await?;
        let mut active = BTreeSet::new();
        for installation in installations {
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
            let phase = self
                .caller_installation_phase(
                    &available,
                    &installation,
                    credential_gate,
                    caller,
                    activation_errors.contains_key(installation.extension_id().as_str()),
                )
                .await?;
            if phase == InstallationState::Active {
                active.insert(installation.extension_id().clone());
            }
        }
        Ok(active)
    }

    /// Membership of every installation, keyed by extension id. The
    /// operator/settings tool catalog joins this to the
    /// global extension registry so it can hide another user's private tool —
    /// the registry snapshot alone carries no owner. Uses `list_installations`
    /// (not `_enabled_`) because the catalog reflects installed tools
    /// regardless of activation state.
    pub(crate) async fn installation_owners(
        &self,
    ) -> Result<std::collections::BTreeMap<ExtensionId, InstallationOwner>, ProductWorkflowError>
    {
        project_installation_owners(
            self.installation_store
                .list_installations()
                .await
                .map_err(map_extension_installation_error)?,
        )
    }

    pub(crate) async fn activation_credential_requirements(
        &self,
        package_ref: &LifecyclePackageRef,
        caller: &UserId,
    ) -> Result<Vec<RuntimeCredentialAuthRequirement>, ProductWorkflowError> {
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
        // WebUI card's owner-transaction credential gate never sees a
        // requirement shape for an unconfigured provider — they see this
        // `Err` instead.
        if requirements.iter().any(|requirement| {
            self.provider_instance_readiness
                .contains(&requirement.provider)
        }) {
            return Err(ProductWorkflowError::ProviderInstanceNotConfigured);
        }
        Ok(requirements)
    }

    /// Redacted per-extension activation errors from the generic host's
    /// working records, keyed by extension id. A record carries a `last_error`
    /// exactly when its last activation attempt recorded a terminal `Failed`.
    /// Empty when the generic host is not attached to this port. Both the
    /// installation-state projection (`Failed`) and the extensions wire's
    /// `activation_error` are driven from this one source.
    pub(crate) async fn installation_activation_errors(
        &self,
    ) -> Result<std::collections::HashMap<String, String>, ProductWorkflowError> {
        match self.generic_host.get() {
            Some(host) => {
                host.installation_errors()
                    .await
                    .map_err(|error| ProductWorkflowError::Transient {
                        reason: format!("extension activation errors could not be read: {error}"),
                    })
            }
            None => Ok(std::collections::HashMap::new()),
        }
    }

    async fn installed_summaries(
        &self,
        caller: &UserId,
        credential_gate: Option<&RuntimeExtensionActivationCredentialGate>,
    ) -> Result<Vec<LifecycleInstalledExtensionSummary>, ProductWorkflowError> {
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
            let phase = self
                .caller_installation_phase(
                    &available,
                    &installation,
                    credential_gate,
                    caller,
                    activation_errors.contains_key(installation.extension_id().as_str()),
                )
                .await?;
            summaries.push(LifecycleInstalledExtensionSummary {
                summary: self.summary_for_phase(&available, phase)?,
                phase: LifecyclePublicState::from_host_checkpoint(phase),
                install_scope: Some(install_scope_for_owner(installation.owner())),
            });
        }
        Ok(summaries)
    }

    async fn search_summary(
        &self,
        extension: &AvailableExtensionPackage,
        credential_gate: Option<&RuntimeExtensionActivationCredentialGate>,
        caller: &UserId,
        activation_errors: &std::collections::HashMap<String, String>,
    ) -> Result<LifecycleSearchExtensionSummary, ProductWorkflowError> {
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
        let phase = self
            .caller_installation_phase(
                extension,
                &installation,
                credential_gate,
                caller,
                has_last_error,
            )
            .await?;
        summary = self.summary_for_phase(extension, phase)?;
        suppress_search_credential_onboarding(&mut summary);
        Ok(LifecycleSearchExtensionSummary {
            summary,
            installation_phase: Some(LifecyclePublicState::from_host_checkpoint(phase)),
        })
    }

    /// Project capability metadata from the same effective package the host
    /// published for execution. Static catalog metadata remains authoritative
    /// until this caller is active; hosted-MCP discovery replaces that
    /// pre-discovery ceiling with the live tool contract atomically.
    fn summary_for_phase(
        &self,
        extension: &AvailableExtensionPackage,
        phase: InstallationState,
    ) -> Result<LifecycleExtensionSummary, ProductWorkflowError> {
        let mut summary = extension.summary();
        let active = self.active_extensions.snapshot();
        let (visible, visible_read_only) = ironclaw_extension_host::project_capability_ids(
            active.as_ref(),
            &extension.package.id,
            phase,
            &summary.visible_capability_ids,
            &summary.visible_read_only_capability_ids,
        )
        .map_err(|error| ProductWorkflowError::Transient {
            reason: error.to_string(),
        })?;
        summary.visible_capability_ids = visible;
        summary.visible_read_only_capability_ids = visible_read_only;
        Ok(summary)
    }

    /// Derive the caller-visible lifecycle state from the only durable user
    /// axis (membership) plus the manifest-declared personal setup contracts.
    /// Runtime publication is an internal readiness consequence, never a
    /// separately persisted user transition.
    async fn caller_installation_phase(
        &self,
        extension: &AvailableExtensionPackage,
        installation: &ExtensionInstallation,
        credential_gate: Option<&RuntimeExtensionActivationCredentialGate>,
        caller: &UserId,
        has_last_error: bool,
    ) -> Result<InstallationState, ProductWorkflowError> {
        if !installation.owner().visible_to(caller) {
            return Ok(InstallationState::Removed);
        }
        if has_last_error {
            return Ok(InstallationState::Failed);
        }
        if self
            .account_setups
            .missing_requirement(installation.extension_id(), caller)
            .await
            .map_err(map_account_setup_error)?
            .is_some()
        {
            return Ok(InstallationState::Installed);
        }
        let requirements = package_runtime_credential_auth_requirements(&extension.package);
        if !requirements.is_empty() {
            let Some(credential_gate) = credential_gate else {
                return Ok(InstallationState::Installed);
            };
            if !credential_gate
                .missing_requirements(requirements)
                .await
                .map_err(map_search_credential_stage_error)?
                .is_empty()
            {
                return Ok(InstallationState::Installed);
            }
        }
        let runtime_ready = self
            .active_extensions
            .snapshot()
            .get_extension(installation.extension_id())
            .is_some();
        Ok(if runtime_ready {
            InstallationState::Active
        } else {
            InstallationState::Installed
        })
    }

    async fn search_installation(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionInstallation>, ProductWorkflowError> {
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
            return Err(ProductWorkflowError::InvalidBindingRequest {
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
    /// existing install flow then reconciles it like any other available
    /// extension.
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
    /// uploads each expand [`crate::extension_host::extension_bundle::MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES`].
    pub(crate) async fn import_bundle(
        &self,
        bundle: Vec<u8>,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        // Hold the permit until the package has passed duplicate checks,
        // materialization, and catalog insertion. This bounds the number of
        // fully expanded packages retained by an import in addition to the
        // decode work itself.
        let _decode_permit = self.import_decode_semaphore.acquire().await.map_err(|_| {
            ProductWorkflowError::Transient {
                reason: "import decode limiter is closed".to_string(),
            }
        })?;
        let reserved_bundled_ids = self.catalog.read().await.reserved_bundled_ids().to_vec();
        let package = tokio::task::spawn_blocking(move || {
            let files = unzip_extension_bundle(&bundle)?;
            imported_extension_package(files, &reserved_bundled_ids)
        })
        .await
        .map_err(|error| ProductWorkflowError::Transient {
            reason: format!("import decode task failed: {error}"),
        })??;
        let package_ref = package.package_ref.clone();
        let summary = package.summary();
        let mut catalog = self.catalog.write().await;
        let _operation_guard = self.operation_lock.lock().await;
        if catalog.resolve(&package_ref).is_ok() {
            return Err(ProductWorkflowError::InvalidBindingRequest {
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

    pub(crate) async fn install(
        &self,
        package_ref: LifecyclePackageRef,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
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
            // The id is already installed: a new caller joins the member set;
            // an existing caller performs an idempotent install retry so the
            // outer command can reconcile any newly completed personal setup.
            // The bundle is already registered/materialized and needs no
            // compensating write.
            Some(existing) => {
                if let Some(new_owner) = existing
                    .owner()
                    .joined_by(caller)
                    .map_err(map_extension_installation_error)?
                {
                    self.installation_store
                        .upsert_installation(existing.with_owner(new_owner))
                        .await
                        .map_err(map_extension_installation_error)?;
                }
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
                next_step:
                    "IronClaw is completing manifest-declared setup and runtime publication."
                        .to_string(),
                connection_required: None,
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
    ) -> Result<(), ProductWorkflowError> {
        // An orphaned manifest row without an installation still counts as
        // occupied (pre-#5459 behavior, kept fail-closed).
        if self
            .installation_store
            .get_manifest(&available.package.id)
            .await
            .map_err(map_extension_installation_error)?
            .is_some()
        {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "extension {} is already installed; if a previous removal was interrupted, run remove again to finish its cleanup, then retry the install",
                    available.package.id.as_str()
                ),
            });
        }
        let owner = derive_owner(caller);
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

    pub(crate) async fn activate(
        &self,
        package_ref: LifecyclePackageRef,
        mode: ExtensionActivationMode,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let credential_gate = UnavailableExtensionActivationCredentialGate;
        self.activate_via_owner_transaction(package_ref, mode, &credential_gate, caller)
            .await
    }

    pub(crate) async fn activate_with_credential_gate(
        &self,
        package_ref: LifecyclePackageRef,
        mode: ExtensionActivationMode,
        credential_gate: impl ExtensionActivationCredentialGate,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        self.activate_via_owner_transaction(package_ref, mode, &credential_gate, caller)
            .await
    }

    /// Rebuild non-durable hosted-MCP tool contracts after the runtime host and
    /// product-auth ports have been attached.
    ///
    /// Installation membership is durable, but a hosted server's live
    /// `tools/list` contract is intentionally not. Each installed package is
    /// therefore rediscovered through the same credential-gated activation
    /// transaction used by an ordinary install. Members are tried
    /// independently: one credential-ready member is sufficient to publish the
    /// shared schema, while missing credentials or a failed provider must not
    /// prevent unrelated packages from restoring.
    pub(crate) async fn reconcile_hosted_mcp_runtime_after_restore(
        &self,
        scope_template: &ResourceScope,
        credential_accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
        runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    ) -> Result<(), ProductWorkflowError> {
        let installations = self
            .installation_store
            .list_installations()
            .await
            .map_err(map_extension_installation_error)?;
        for installation in installations {
            let package_ref = LifecyclePackageRef::new(
                LifecyclePackageKind::Extension,
                installation.extension_id().as_str(),
            )?;
            // Startup re-activation skips invalid rows instead of blocking
            // boot (§6.5): an installation whose package left the catalog
            // (orphan row) resolves as not-installed here — warn and move on.
            match self
                .package_requires_hosted_mcp_discovery(&package_ref)
                .await
            {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) => {
                    tracing::warn!(
                        extension_id = installation.extension_id().as_str(),
                        %error,
                        "skipping hosted MCP restart reconciliation for an unresolvable installation row"
                    );
                    continue;
                }
            }
            let Some(members) = installation.owner().members() else {
                tracing::warn!(
                    extension_id = installation.extension_id().as_str(),
                    "skipping hosted MCP restart reconciliation for a non-canonical owner"
                );
                continue;
            };
            for member in members {
                let mut scope = scope_template.clone();
                scope.user_id = member.clone();
                scope.invocation_id = ironclaw_host_api::InvocationId::new();
                let credential_gate = RuntimeExtensionActivationCredentialGate::new(
                    scope.clone(),
                    Arc::clone(&credential_accounts),
                );
                let mode = ExtensionActivationMode::HostedMcpDiscovery {
                    scope,
                    runtime_http_egress: Arc::clone(&runtime_http_egress),
                };
                match self
                    .activate_with_credential_gate(
                        package_ref.clone(),
                        mode,
                        credential_gate,
                        member,
                    )
                    .await
                {
                    Ok(response) if response.phase == LifecyclePublicState::Active => break,
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(
                            extension_id = installation.extension_id().as_str(),
                            %error,
                            "hosted MCP restart reconciliation failed for one installed member"
                        );
                    }
                }
            }
        }
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) async fn activate_with_prechecked_credentials_for_test(
        &self,
        package_ref: LifecyclePackageRef,
        mode: ExtensionActivationMode,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let credential_gate =
            crate::extension_host::extension_activation_credentials::PrecheckedExtensionActivationCredentialGate;
        self.activate_via_owner_transaction(package_ref, mode, &credential_gate, caller)
            .await
    }

    async fn activate_via_owner_transaction(
        &self,
        package_ref: LifecyclePackageRef,
        mode: ExtensionActivationMode,
        credential_gate: &dyn ExtensionActivationCredentialGate,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let (extension_id, installation_id) = extension_ids_from_package_ref(&package_ref)?;
        match self
            .run_activation_transaction(
                &extension_id,
                &installation_id,
                mode,
                credential_gate,
                caller,
            )
            .await?
        {
            ExtensionActivationTransactionResult::CredentialsMissing(missing) => {
                activation_credentials_incomplete_response(package_ref, missing)
            }
            ExtensionActivationTransactionResult::Activated(package) => {
                Ok(activation_success_response(
                    package_ref,
                    &package,
                    self.account_setups.descriptor(&extension_id),
                ))
            }
        }
    }

    async fn run_activation_transaction(
        &self,
        extension_id: &ExtensionId,
        installation_id: &ExtensionInstallationId,
        mode: ExtensionActivationMode,
        credential_gate: &dyn ExtensionActivationCredentialGate,
        caller: &UserId,
    ) -> Result<ExtensionActivationTransactionResult, ProductWorkflowError> {
        let operations = ComposedExtensionActivationOperations {
            management: self,
            credential_gate,
        };
        run_extension_activation(
            self.operation_lock.as_ref(),
            &operations,
            extension_id,
            installation_id,
            caller,
            mode,
        )
        .await
    }

    pub(crate) async fn package_requires_hosted_mcp_discovery(
        &self,
        package_ref: &LifecyclePackageRef,
    ) -> Result<bool, ProductWorkflowError> {
        let (extension_id, _) = extension_ids_from_package_ref(package_ref)?;
        let _operation_guard = self.operation_lock.lock().await;
        let package = self.lifecycle_package(&extension_id).await?;
        Ok(is_hosted_http_mcp_package(&package))
    }

    /// Remove an installed extension. This is the single convergence point both
    /// removal entrypoints call — the WebUI facade
    /// ([`LifecycleProductAction::ExtensionRemove`]) and the
    /// `builtin.extension_remove` agent capability — so the credential
    /// revocation below cannot be bypassed through one door.
    ///
    /// On success it revokes the removed extension's reusable personal
    /// credentials for providers now exclusive to it (see
    /// [`Self::revoke_exclusive_credentials`]).
    pub(crate) async fn remove(
        &self,
        package_ref: LifecyclePackageRef,
        scope: &ResourceScope,
        authenticated_actor_user_id: Option<&ironclaw_host_api::UserId>,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
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
            }
            let installed_manifest = self
                .installation_store
                .get_manifest(&extension_id)
                .await
                .map_err(map_extension_installation_error)?;
            if installation.is_none() && installed_manifest.is_none() && began_with_local_state {
                return Err(ProductWorkflowError::InvalidBindingRequest {
                    reason: format!("extension {} is not installed", extension_id.as_str()),
                });
            }
            if installation.is_some() && installed_manifest.is_none() {
                return Err(ProductWorkflowError::InvalidBindingRequest {
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
                prepare_install(&available, derive_owner(caller))?.manifest_record
            };
            let removed_providers =
                Self::removed_extension_providers_from_manifest(&removal_manifest)?;
            let cleanup_requirements = removal_manifest.removal_cleanup_requirements().to_vec();
            // §6.4: every channel surface can hold per-caller connection state.
            // OAuth channels own vendor credentials/identity bindings, while
            // proof-code channels own pairing records, identity bindings, DM
            // targets, and conversation-actor bindings. Removal runs the real
            // per-caller disconnect below while the installation still exists.
            // The generic facade discovers the same manifest-derived set.
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
                return Err(ProductWorkflowError::InvalidBindingRequest {
                    reason: "extension removal cleanup requires an authenticated actor".to_string(),
                });
            }
            if !removed_providers.is_empty() && authenticated_actor_user_id.is_none() {
                return Err(ProductWorkflowError::InvalidBindingRequest {
                    reason: "extension credential cleanup requires an authenticated actor"
                        .to_string(),
                });
            }
            if installed_manifest.is_none() {
                self.installation_store
                    .upsert_manifest(removal_manifest)
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
            // through the same generic facade the extensions page reads, so
            // connection state, durable bindings, lifecycle phase, and tool
            // dispatchability flip together on removal. Runs before teardown
            // so the installation-scoped binding prefix still resolves; a
            // failure keeps the installation authoritative and stays
            // retryable, mirroring the credential cleanup below.
            if removes_connectable_channel && let Some(actor_user_id) = authenticated_actor_user_id
            {
                // Fail closed on an empty slot: a channel surface may hold
                // per-caller OAuth or pairing state, and a composition that
                // gives this path no facade to disconnect it through must not
                // report the removal as successful.
                // Surface the same typed retryable error a failing disconnect
                // does; compositions that legitimately remove channel
                // extensions fill the slot (runtime composition in
                // `build_reborn_runtime`, the channel-connection test bundle).
                let Some(channel_connection) = self.channel_disconnect_slot.get() else {
                    return Err(ProductWorkflowError::Transient {
                        reason: format!(
                            "channel connection cleanup is unavailable for extension {}: no \
                             channel connection facade is composed; retry removal once the \
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
                    .map_err(|error| ProductWorkflowError::Transient {
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
    ) -> Result<Vec<AuthProviderId>, ProductWorkflowError> {
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
    ) -> Result<Vec<AuthProviderId>, ProductWorkflowError> {
        let mut providers = Vec::new();
        for requirement in requirements {
            let provider = AuthProviderId::new(requirement.provider.as_str()).map_err(|_| {
                ProductWorkflowError::InvalidBindingRequest {
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
    ) -> Result<(), ProductWorkflowError> {
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
            ProductWorkflowError::InvalidBindingRequest {
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
                ProductWorkflowError::Transient {
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
            return Err(ProductWorkflowError::Transient {
                reason: "extension credential cleanup was incomplete; retry removal".to_string(),
            });
        }
        if removed_providers.is_empty() {
            return Ok(());
        }
        let providers_still_in_use = self
            .providers_still_in_use(removed_extension_id, caller)
                .await
                .ok_or_else(|| ProductWorkflowError::Transient {
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
                ProductWorkflowError::Transient {
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
                return Err(ProductWorkflowError::Transient {
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
    ) -> Result<(), ProductWorkflowError> {
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
            if let Some(package) = restore_package
                && let Err(restore_error) = self.restore_lifecycle_package(package).await
            {
                return Err(compensation_failure(
                    "orphan extension file cleanup failed and lifecycle restore failed",
                    error,
                    restore_error,
                ));
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

    async fn remove_locked(
        &self,
        package_ref: LifecyclePackageRef,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let (extension_id, installation_id) = extension_ids_from_package_ref(&package_ref)?;
        let installation = self
            .load_installation(&extension_id, &installation_id)
            .await?;
        ensure_caller_may_operate(&installation, caller)?;
        // A caller leaves the shared package aggregate without affecting any
        // other member. Only the final member tears down shared runtime state.
        if let Some(remaining) = installation
            .owner()
            .without_member(caller)
            .map_err(map_extension_installation_error)?
        {
            // Evidence tripwire (`tool-evidence.md`): a leave that changed
            // nothing means the caller was never a member of this row. The
            // authorization gate above must have rejected that caller, so
            // reaching here is an internal invariant violation — never report
            // `removed: true` for a mutation that did not mutate.
            if &remaining == installation.owner() {
                return Err(ProductWorkflowError::Transient {
                    reason: format!(
                        "extension {} removal changed no membership for an authorized caller",
                        extension_id.as_str()
                    ),
                });
            }
            let remaining_installation = installation.clone().with_owner(remaining);
            self.installation_store
                .upsert_installation(remaining_installation)
                .await
                .map_err(map_extension_installation_error)?;
            return Ok(response_with_payload(
                Some(package_ref),
                InstallationState::Removed,
                LifecycleProductPayload::ExtensionRemove { removed: true },
            ));
        }
        let lifecycle_package = self.lifecycle_package(&extension_id).await?;
        // Hosted-MCP discovery can republish a package that differs from the
        // lifecycle-registered package; unpublish the active-registry package
        // and fall back only when nothing is currently active.
        let active_package_for_unpublish = self
            .active_extensions
            .snapshot()
            .get_extension(&extension_id)
            .cloned()
            .unwrap_or_else(|| lifecycle_package.clone());
        self.remove_lifecycle_package(&extension_id).await?;
        self.unpublish_from_generic_host(&extension_id).await;
        if let Err(error) = self
            .active_extensions
            .unpublish(&active_package_for_unpublish)
        {
            if let Err(restore_error) = self
                .restore_runtime_publication(&installation_id, &lifecycle_package)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to unpublish the runtime package and runtime restore failed",
                    error,
                    restore_error,
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
                .restore_runtime_publication(&installation_id, &lifecycle_package)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to delete installation and runtime restore failed",
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
            if let Err(restore_error) = self.restore_installation(&installation).await {
                return Err(compensation_failure(
                    "extension remove failed to delete files and installation restore failed",
                    error,
                    restore_error,
                ));
            }
            if let Err(restore_error) = self
                .restore_runtime_publication(&installation_id, &lifecycle_package)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to delete files and runtime restore failed",
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
    ) -> Result<(), ProductWorkflowError> {
        let mut lifecycle = self.lifecycle_service.lock().await;
        if lifecycle.registry().get_extension(&package.id).is_some() {
            return Err(ProductWorkflowError::InvalidBindingRequest {
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
    ) -> Result<(), ProductWorkflowError> {
        if self
            .installation_store
            .get_installation(installation_id)
            .await
            .map_err(map_extension_installation_error)?
            .is_some()
        {
            return Err(ProductWorkflowError::InvalidBindingRequest {
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
            return Err(ProductWorkflowError::InvalidBindingRequest {
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
    ) -> Result<ExtensionInstallation, ProductWorkflowError> {
        let installation = self
            .installation_store
            .get_installation(installation_id)
            .await
            .map_err(map_extension_installation_error)?
            .ok_or_else(|| ProductWorkflowError::InvalidBindingRequest {
                reason: format!("extension {} is not installed", extension_id.as_str()),
            })?;
        if installation.extension_id() != extension_id {
            return Err(ProductWorkflowError::InvalidBindingRequest {
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
    ) -> Result<ExtensionPackage, ProductWorkflowError> {
        let lifecycle = self.lifecycle_service.lock().await;
        lifecycle
            .registry()
            .get_extension(extension_id)
            .cloned()
            .ok_or_else(|| ProductWorkflowError::InvalidBindingRequest {
                reason: format!("extension {} is not installed", extension_id.as_str()),
            })
    }

    async fn enable_lifecycle_package(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ProductWorkflowError> {
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
    ) -> Result<(), ProductWorkflowError> {
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
    ) -> Result<(), ProductWorkflowError> {
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
    ) -> Result<(), ProductWorkflowError> {
        let mut lifecycle = self.lifecycle_service.lock().await;
        lifecycle
            .remove(extension_id)
            .await
            .map_err(map_extension_error)
    }

    async fn restore_lifecycle_package(
        &self,
        package: &ExtensionPackage,
    ) -> Result<(), ProductWorkflowError> {
        let mut lifecycle = self.lifecycle_service.lock().await;
        lifecycle
            .install(package.clone())
            .await
            .map_err(map_extension_error)?;
        lifecycle
            .enable(&package.id)
            .await
            .map_err(map_extension_error)?;
        Ok(())
    }

    async fn restore_installation(
        &self,
        installation: &ExtensionInstallation,
    ) -> Result<(), ProductWorkflowError> {
        self.installation_store
            .upsert_installation(installation.clone())
            .await
            .map_err(map_extension_installation_error)
    }

    async fn restore_runtime_publication(
        &self,
        installation_id: &ExtensionInstallationId,
        package: &ExtensionPackage,
    ) -> Result<(), ProductWorkflowError> {
        self.restore_lifecycle_package(package).await?;
        if let Err(error) = self.active_extensions.publish(package) {
            if let Err(rollback_error) = self.remove_lifecycle_package(&package.id).await {
                return Err(compensation_failure(
                    "extension runtime restore failed to publish and lifecycle rollback failed",
                    error,
                    rollback_error,
                ));
            }
            return Err(error);
        }
        if let Err(error) = self
            .publish_to_generic_host(&package.id, installation_id, package)
            .await
        {
            if let Err(rollback_error) = self.active_extensions.unpublish(package) {
                return Err(compensation_failure(
                    "extension runtime restore failed in the generic host and registry rollback failed",
                    error,
                    rollback_error,
                ));
            }
            if let Err(rollback_error) = self.remove_lifecycle_package(&package.id).await {
                return Err(compensation_failure(
                    "extension runtime restore failed in the generic host and lifecycle rollback failed",
                    error,
                    rollback_error,
                ));
            }
            return Err(error);
        }
        Ok(())
    }

    async fn persist_install_plan(
        &self,
        plan: ExtensionInstallPlan,
    ) -> Result<(), ProductWorkflowError> {
        let extension_id = plan.installation.extension_id().clone();
        if let Err(error) = self
            .installation_store
            .upsert_manifest(plan.manifest_record)
            .await
        {
            return Err(map_extension_installation_error(error));
        }
        if let Err(error) = self
            .installation_store
            .upsert_installation(plan.installation)
            .await
        {
            if let Err(cleanup_error) = self.installation_store.delete_manifest(&extension_id).await
            {
                // Fail loud: the installation upsert failed *and* the manifest
                // rollback failed, so a manifest is now orphaned with no
                // installation. `ensure_not_installed` treats any manifest as
                // installed, which would block every retry — surface both
                // failures so the orphan is visible rather than silently
                // poisoning future installs.
                return Err(compensation_failure(
                    "extension install persistence failed and manifest rollback failed",
                    map_extension_installation_error(error),
                    map_extension_installation_error(cleanup_error),
                ));
            }
            return Err(map_extension_installation_error(error));
        }
        Ok(())
    }

    async fn delete_materialized_extension_files(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ProductWorkflowError> {
        let Ok(extension_root) =
            VirtualPath::new(format!("/system/extensions/{}", extension_id.as_str()))
        else {
            return Ok(());
        };
        match self.filesystem.delete(&extension_root).await {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => {
                tracing::debug!(%error, %extension_id, "extension file removal failed");
                Err(ProductWorkflowError::Transient {
                    reason: "failed to remove extension files; retry removal".to_string(),
                })
            }
        }
    }
}

/// Fold the host's internal install and activation checkpoints into the one
/// product action users requested. `Installed` remains a durable rollback and
/// retry boundary, but it is never a second user-visible step: the returned
/// payload is either active or carries the manifest-derived setup blockers.
pub(crate) fn complete_install_response(
    mut install: LifecycleProductResponse,
    activation: LifecycleProductResponse,
) -> LifecycleProductResponse {
    let is_active = activation.phase == LifecyclePublicState::Active;
    let (visible_capability_ids, connection_required) = match activation.payload.as_ref() {
        Some(LifecycleProductPayload::ExtensionInstall {
            visible_capability_ids,
            connection_required,
            ..
        }) => (visible_capability_ids.clone(), connection_required.clone()),
        _ => (Vec::new(), None),
    };
    install.phase = activation.phase;
    install.blockers = activation.blockers;
    install.message = activation.message.or(install.message);
    if let Some(LifecycleProductPayload::ExtensionInstall {
        visible_capability_ids: install_capability_ids,
        next_step,
        connection_required: install_connection_required,
        ..
    }) = install.payload.as_mut()
    {
        if !visible_capability_ids.is_empty() {
            *install_capability_ids = visible_capability_ids;
        }
        *install_connection_required = connection_required;
        *next_step = if is_active {
            "Extension setup is complete and the extension is active.".to_string()
        } else {
            "Complete the manifest-declared personal setup to continue.".to_string()
        };
    }
    install
}

/// Concrete dependency adapter for the owner-side activation transaction.
///
/// This type deliberately contains no lifecycle ordering or compensation
/// policy: it only exposes the stores and runtime publishers assembled by the
/// composition root.
struct ComposedExtensionActivationOperations<'a> {
    management: &'a ExtensionManagementPort,
    credential_gate: &'a dyn ExtensionActivationCredentialGate,
}

#[async_trait]
impl ExtensionActivationOperations for ComposedExtensionActivationOperations<'_> {
    type Error = ProductWorkflowError;
    type HostedMcpDiscoveryAuthority =
        Option<ironclaw_host_runtime::ProductAuthRuntimeHandoffGuard>;

    async fn load_installation(
        &self,
        extension_id: &ExtensionId,
        installation_id: &ExtensionInstallationId,
    ) -> Result<ExtensionInstallation, Self::Error> {
        self.management
            .load_installation(extension_id, installation_id)
            .await
    }

    fn ensure_caller_may_operate(
        &self,
        installation: &ExtensionInstallation,
        caller: &UserId,
    ) -> Result<(), Self::Error> {
        ensure_caller_may_operate(installation, caller)
    }

    async fn lifecycle_package(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<ExtensionPackage, Self::Error> {
        self.management.lifecycle_package(extension_id).await
    }

    async fn installed_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<ExtensionManifestRecord, Self::Error> {
        self.management
            .installation_store
            .get_manifest(extension_id)
            .await
            .map_err(map_extension_installation_error)?
            .ok_or_else(|| ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "extension {} manifest is not installed",
                    extension_id.as_str()
                ),
            })
    }

    async fn missing_account_setup(
        &self,
        extension_id: &ExtensionId,
        caller: &UserId,
    ) -> Result<Option<RuntimeCredentialAuthRequirement>, Self::Error> {
        self.management
            .account_setups
            .missing_requirement(extension_id, caller)
            .await
            .map_err(map_account_setup_error)
    }

    async fn credential_readiness(
        &self,
        package: &ExtensionPackage,
    ) -> Result<ExtensionActivationCredentialReadiness, Self::Error> {
        self.credential_gate.credential_readiness(package).await
    }

    async fn stage_hosted_mcp_discovery_authority(
        &self,
        scope: &ResourceScope,
        package: &ExtensionPackage,
        network_policy: ironclaw_host_api::NetworkPolicy,
    ) -> Self::HostedMcpDiscoveryAuthority {
        self.management
            .stage_hosted_mcp_discovery_authority(scope, package, network_policy)
            .await
    }

    async fn discover_hosted_mcp_package(
        &self,
        package: &ExtensionPackage,
        max_tools: u32,
        scope: ResourceScope,
        runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    ) -> Result<HostedMcpDiscoveryOutcome, Self::Error> {
        match discover_hosted_mcp_package(package, max_tools, scope, runtime_http_egress).await {
            Ok(discovered) => Ok(HostedMcpDiscoveryOutcome::Discovered(Box::new(discovered))),
            Err(HostedMcpDiscoveryError::ReAuthRequired) => {
                // The provider rejected the staged credentials during
                // discovery. Route the caller back through the same credential
                // setup / OAuth path a pre-discovery missing credential uses,
                // re-deriving the extension's declared requirements from the
                // package. Nothing is discarded from the credential store.
                Ok(HostedMcpDiscoveryOutcome::CredentialsRejected(
                    package_runtime_credential_auth_requirements(package),
                ))
            }
            Err(other) => Err(hosted_mcp_discovery_error(other)),
        }
    }

    fn package_is_published(&self, extension_id: &ExtensionId, package: &ExtensionPackage) -> bool {
        self.management
            .active_extensions
            .snapshot()
            .get_extension(extension_id)
            == Some(package)
    }

    async fn enable_lifecycle_package(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), Self::Error> {
        self.management.enable_lifecycle_package(extension_id).await
    }

    async fn disable_lifecycle_package(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), Self::Error> {
        self.management
            .disable_lifecycle_package(extension_id)
            .await
    }

    fn publish_active_package(&self, package: &ExtensionPackage) -> Result<(), Self::Error> {
        self.management.active_extensions.publish(package)
    }

    fn unpublish_active_package(&self, package: &ExtensionPackage) -> Result<(), Self::Error> {
        self.management.active_extensions.unpublish(package)
    }

    async fn publish_runtime_package(
        &self,
        extension_id: &ExtensionId,
        installation_id: &ExtensionInstallationId,
        package: &ExtensionPackage,
    ) -> Result<(), Self::Error> {
        self.management
            .publish_to_generic_host(extension_id, installation_id, package)
            .await
    }

    fn map_authority_error(&self, error: ExtensionInstallationError) -> Self::Error {
        map_extension_installation_error(error)
    }

    fn discovery_recheck_error(&self, error: Option<Self::Error>) -> Self::Error {
        if let Some(error) = error {
            tracing::debug!(
                %error,
                "hosted MCP activation authority changed during discovery"
            );
        }
        hosted_mcp_changed_during_discovery_error()
    }

    fn compensation_failure(
        &self,
        context: &'static str,
        original: Self::Error,
        compensation: Self::Error,
    ) -> Self::Error {
        compensation_failure(context, original, compensation)
    }
}

struct ExtensionInstallPlan {
    manifest_record: ExtensionManifestRecord,
    installation: ExtensionInstallation,
}

fn prepare_install(
    available: &AvailableExtensionPackage,
    owner: InstallationOwner,
) -> Result<ExtensionInstallPlan, ProductWorkflowError> {
    let manifest_hash = available_manifest_hash(available)?;
    let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("host port catalog rejected extension install: {error}"),
        }
    })?;
    let contracts = product_extension_host_api_contract_registry().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("host API contract registry rejected extension install: {error}"),
        }
    })?;
    let manifest_record = ExtensionManifestRecord::from_toml(
        &available.manifest_toml,
        available.source,
        &host_ports,
        Some(manifest_hash.clone()),
        &contracts,
    )
    .map_err(map_extension_installation_error)?
    .with_removal_cleanup_requirements(available.cleanup_requirements.clone());
    let installation_id = ExtensionInstallationId::new(available.package.id.as_str().to_string())
        .map_err(map_extension_installation_error)?;
    let installation = ExtensionInstallation::new(
        installation_id,
        available.package.id.clone(),
        ExtensionManifestRef::new(available.package.id.clone(), Some(manifest_hash)),
        Vec::new(),
        chrono::Utc::now(),
        owner,
    )
    .map_err(map_extension_installation_error)?;
    Ok(ExtensionInstallPlan {
        manifest_record,
        installation,
    })
}

/// Build an [`ExtensionInstallPlan`] that carries the new manifest hash from `available`
/// while preserving membership, health, and credential bindings from `existing`.
/// Used during restore to migrate a stored installation when the bundled manifest changes.
fn prepare_manifest_migration(
    available: &AvailableExtensionPackage,
    existing: &ExtensionInstallation,
) -> Result<ExtensionInstallPlan, ProductWorkflowError> {
    let manifest_hash = available_manifest_hash(available)?;
    let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("host port catalog rejected manifest migration: {error}"),
        }
    })?;
    let contracts = product_extension_host_api_contract_registry().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("host API contract registry rejected manifest migration: {error}"),
        }
    })?;
    let manifest_record = ExtensionManifestRecord::from_toml(
        &available.manifest_toml,
        available.source,
        &host_ports,
        Some(manifest_hash.clone()),
        &contracts,
    )
    .map_err(map_extension_installation_error)?
    .with_removal_cleanup_requirements(available.cleanup_requirements.clone());
    let installation =
        ExtensionInstallation::from_persisted_parts(ExtensionInstallationPersistedParts {
            installation_id: existing.installation_id().clone(),
            extension_id: existing.extension_id().clone(),
            manifest_ref: ExtensionManifestRef::new(
                existing.extension_id().clone(),
                Some(manifest_hash),
            ),
            credential_bindings: existing.credential_bindings().to_vec(),
            health: existing.health().clone(),
            updated_at: chrono::Utc::now(),
            // A manifest migration changes only the compiled manifest. It
            // must not broaden installation membership.
            owner: existing.owner().clone(),
        })
        .map_err(map_extension_installation_error)?;
    Ok(ExtensionInstallPlan {
        manifest_record,
        installation,
    })
}

async fn migrate_host_bundled_manifest_hash(
    installation_store: &Arc<dyn ExtensionInstallationStorePort>,
    available: &AvailableExtensionPackage,
    installation: &ExtensionInstallation,
    hash_error: ProductWorkflowError,
) -> Result<(), ProductWorkflowError> {
    let stored_manifest = match installation_store
        .get_manifest(installation.extension_id())
        .await
        .map_err(map_extension_installation_error)?
    {
        Some(stored_manifest) => stored_manifest,
        None => return Err(hash_error),
    };
    if stored_manifest.manifest().source != ManifestSource::HostBundled {
        return Err(hash_error);
    }

    // For host-bundled (first-party) extensions, a manifest hash mismatch means
    // the binary was updated and the bundled manifest changed. Migrate the stored
    // records to the new hash while preserving activation state and bindings.
    tracing::warn!(
        extension_id = %installation.extension_id(),
        "bundled extension manifest hash changed; migrating stored installation to new manifest hash"
    );
    let migration_plan = prepare_manifest_migration(available, installation)?;
    installation_store
        .upsert_manifest_and_installation(
            migration_plan.manifest_record,
            migration_plan.installation,
        )
        .await
        .map_err(map_extension_installation_error)
}

fn validate_restored_manifest_hash(
    installation: &ExtensionInstallation,
    available: &AvailableExtensionPackage,
) -> Result<(), ProductWorkflowError> {
    let manifest_hash = available_manifest_hash(available)?;
    match installation.manifest_ref().manifest_hash() {
        Some(installed_hash) if installed_hash == &manifest_hash => Ok(()),
        _ => Err(map_extension_installation_error(
            ExtensionInstallationError::ManifestHashMismatch {
                extension_id: installation.extension_id().clone(),
            },
        )),
    }
}

fn available_manifest_hash(
    available: &AvailableExtensionPackage,
) -> Result<ManifestHash, ProductWorkflowError> {
    ManifestHash::new(sha256_digest_token(available.manifest_toml.as_bytes()))
        .map_err(map_extension_installation_error)
}

fn package_visible_capability_ids(package: &ExtensionPackage) -> Vec<String> {
    package
        .manifest
        .capabilities
        .iter()
        .filter(|capability| capability.visibility == CapabilityVisibility::Model)
        .map(|capability| capability.id.as_str().to_string())
        .collect()
}

fn activation_success_response(
    package_ref: LifecyclePackageRef,
    package: &ExtensionPackage,
    account_setup: Option<ExtensionAccountSetupDescriptor>,
) -> LifecycleProductResponse {
    let visible_capability_ids = package_visible_capability_ids(package);
    let message =
        connection_success_message(package, &visible_capability_ids, account_setup.as_ref());
    let connection_required = if package_declares_inbound_product_adapter(package) {
        projected_channel_connection_requirement(account_setup.as_ref())
    } else {
        None
    };
    let mut response = response_with_payload(
        Some(package_ref),
        InstallationState::Active,
        LifecycleProductPayload::ExtensionInstall {
            installed: true,
            visible_capability_ids,
            next_step: "Extension setup is complete and the extension is active.".to_string(),
            connection_required,
        },
    );
    response.message = Some(message);
    response
}

fn activation_credentials_incomplete_response(
    package_ref: LifecyclePackageRef,
    missing: Vec<RuntimeCredentialAuthRequirement>,
) -> Result<LifecycleProductResponse, ProductWorkflowError> {
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
        LifecycleProductPayload::ExtensionInstall {
            installed: true,
            visible_capability_ids: Vec::new(),
            next_step: "Complete the manifest-declared personal setup to continue.".to_string(),
            connection_required: None,
        },
    );
    response.blockers = blockers;
    response.message = Some(
        "Extension credentials were saved; connect the remaining credential providers and IronClaw will finish installation automatically."
            .to_string(),
    );
    Ok(response)
}

fn connection_success_message(
    package: &ExtensionPackage,
    visible_capability_ids: &[String],
    account_setup: Option<&ExtensionAccountSetupDescriptor>,
) -> String {
    if package_declares_inbound_product_adapter(package) {
        if let Some(account_setup) = account_setup {
            return account_setup.connection_success_message.clone();
        }
        let display_name = package.manifest.name.as_str();
        return format!(
            "{display_name} is installed as a channel surface. Follow the structured \
             connection state rendered from its manifest; do not invent pairing commands, \
             credentials, or administrator requirements. Final replies on this channel are \
             delivered by the host's outbound delivery, never by calling extension tools."
        );
    }
    if visible_capability_ids.is_empty() {
        return "Extension setup completed. No model-visible tools were published by this extension; follow any manifest-declared connection UI before claiming new capabilities are available.".to_string();
    }
    let mut message = String::from(
        "Extension setup completed and its tools are now available. No additional authorization or configuration is needed, including for write-capable tools, unless a later tool call reports auth_required.",
    );
    message.push_str(
        " These tools are now callable by exact name — invoke one directly with tool_call(name=\"<tool>\", arguments={ ... }), or tool_describe(name=\"<tool>\") first if you need its full schema. Do NOT call tool_search for these; you already have their names: ",
    );
    message.push_str(&visible_capability_ids.join(", "));
    message.push('.');
    message
}

fn projected_channel_connection_requirement(
    account_setup: Option<&ExtensionAccountSetupDescriptor>,
) -> Option<ChannelConnectionRequirement> {
    account_setup.map(|setup| setup.connection_requirement.clone())
}

fn generic_host_error(error: ironclaw_extension_host::LifecycleError) -> ProductWorkflowError {
    ProductWorkflowError::InvalidBindingRequest {
        reason: format!("generic extension host rejected the activation: {error}"),
    }
}

fn map_extension_admin_configuration_error(
    error: ironclaw_extension_host::ExtensionAdminConfigurationResolverError,
) -> ProductWorkflowError {
    tracing::warn!(error = %error, "effective extension configuration resolution failed");
    ProductWorkflowError::Transient {
        reason: "effective extension configuration is unavailable".to_string(),
    }
}

fn package_declares_inbound_product_adapter(package: &ExtensionPackage) -> bool {
    package.manifest.host_apis.iter().any(|host_api| {
        host_api.id.as_str() == PRODUCT_ADAPTER_HOST_API_ID
            && host_api.section.as_str() == "product_adapter.inbound"
    })
}
fn extension_ids_from_package_ref(
    package_ref: &LifecyclePackageRef,
) -> Result<(ExtensionId, ExtensionInstallationId), ProductWorkflowError> {
    package_ref.require_kind(LifecyclePackageKind::Extension)?;
    let extension_id = ExtensionId::new(package_ref.id.as_str().to_string()).map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
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
            Some(LifecyclePublicState::Active)
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
        extension.installation_phase == Some(LifecyclePublicState::SetupNeeded)
            && !extension
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
            Some(LifecyclePublicState::SetupNeeded | LifecyclePublicState::Active)
        ) && extension
            .summary
            .surface_kinds
            .contains(&CapabilitySurfaceKind::Channel)
    })
}

fn map_search_credential_stage_error(
    error: ironclaw_host_api::CredentialStageError,
) -> ProductWorkflowError {
    match error {
        ironclaw_host_api::CredentialStageError::AuthRequired => {
            ProductWorkflowError::InvalidBindingRequest {
                reason: "extension requires product auth credentials before search can project configured state".to_string(),
            }
        }
        ironclaw_host_api::CredentialStageError::Backend => {
            ProductWorkflowError::Transient {
                reason: "extension product auth credential state is temporarily unavailable"
                    .to_string(),
            }
        }
    }
}

fn map_account_setup_error(error: ExtensionAccountSetupError) -> ProductWorkflowError {
    match error {
        ExtensionAccountSetupError::HostUnavailable { extension_id } => {
            ProductWorkflowError::InvalidBindingRequest {
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
            ProductWorkflowError::Transient {
                reason: format!(
                    "account connection status is temporarily unavailable for extension {}",
                    extension_id.as_str()
                ),
            }
        }
    }
}

fn map_extension_error(error: ExtensionError) -> ProductWorkflowError {
    match error {
        ExtensionError::Filesystem(_) | ExtensionError::LifecycleEventSink { .. } => {
            ProductWorkflowError::Transient {
                reason: error.to_string(),
            }
        }
        _ => ProductWorkflowError::InvalidBindingRequest {
            reason: error.to_string(),
        },
    }
}

fn map_extension_installation_error(error: ExtensionInstallationError) -> ProductWorkflowError {
    match error {
        // #4091: a store IO/backend outage is retryable backend trouble, not a
        // malformed lifecycle request — surface it in the same Transient class
        // credential-cleanup failures already use so callers retry the
        // operation instead of abandoning it.
        error @ ExtensionInstallationError::StoreUnavailable { .. } => {
            ProductWorkflowError::Transient {
                reason: error.to_string(),
            }
        }
        error => ProductWorkflowError::InvalidBindingRequest {
            reason: error.to_string(),
        },
    }
}

fn project_installation_owners<I>(
    installations: I,
) -> Result<std::collections::BTreeMap<ExtensionId, InstallationOwner>, ProductWorkflowError>
where
    I: IntoIterator<Item = ExtensionInstallation>,
{
    let installations = canonicalize_installation_rows(installations.into_iter().collect())
        .map_err(map_extension_installation_error)?;
    let mut owners = std::collections::BTreeMap::new();
    for installation in installations {
        let owner = installation.owner().clone();
        let extension_id = installation.extension_id().clone();
        if owners.insert(extension_id.clone(), owner).is_some() {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "duplicate extension id in lifecycle owner projection: {}",
                    extension_id.as_str()
                ),
            });
        }
    }
    Ok(owners)
}

fn hosted_mcp_discovery_error(error: HostedMcpDiscoveryError) -> ProductWorkflowError {
    match error {
        HostedMcpDiscoveryError::Transient(reason) => ProductWorkflowError::Transient {
            reason: format!("hosted MCP discovery failed: {reason}"),
        },
        HostedMcpDiscoveryError::Permanent(reason) => ProductWorkflowError::InvalidBindingRequest {
            reason: format!("hosted MCP discovery failed: {reason}"),
        },
        // A provider credential rejection is routed to the credentials-missing
        // outcome by `discover_hosted_mcp_package` before it reaches this error
        // mapper, so this arm is defensive: if the invariant is ever violated,
        // fail closed (non-retryable) rather than folding back into a
        // retry-forever transient that re-hits the same rejection.
        HostedMcpDiscoveryError::ReAuthRequired => ProductWorkflowError::InvalidBindingRequest {
            reason: "hosted MCP discovery requires re-authentication".to_string(),
        },
    }
}

fn hosted_mcp_changed_during_discovery_error() -> ProductWorkflowError {
    ProductWorkflowError::Transient {
        reason: "extension changed while hosted MCP discovery was running; retry activation"
            .to_string(),
    }
}

fn compensation_failure(
    context: &str,
    original: impl std::fmt::Display,
    compensation: impl std::fmt::Display,
) -> ProductWorkflowError {
    ProductWorkflowError::Transient {
        reason: format!(
            "{context}; original error: {original}; compensation error: {compensation}"
        ),
    }
}

#[cfg(test)]
mod tests {

    fn capability_provider_contracts() -> ironclaw_extensions::HostApiContractRegistry {
        let mut contracts = ironclaw_extensions::HostApiContractRegistry::new();
        contracts
            .register(std::sync::Arc::new(
                ironclaw_extensions::CapabilityProviderHostApiContract::new()
                    .expect("capability provider contract"),
            ))
            .expect("register capability provider contract");
        contracts
    }
    use std::{
        collections::BTreeSet,
        sync::{
            Mutex as StdMutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use super::hosted_mcp_test_support::HostedMcpDiscoveryEgress;
    use super::*;
    use crate::extension_host::available_extensions::{
        AvailableExtensionAsset, AvailableExtensionAssetContent, AvailableExtensionPackage,
    };
    use crate::extension_host::extension_removal_cleanup::{
        ExtensionRemovalChannelId, ExtensionRemovalCleanupAdapter,
        ExtensionRemovalCleanupAdapterId, ExtensionRemovalCleanupBinding,
        ExtensionRemovalCleanupContext, ExtensionRemovalCleanupRegistry,
        ExtensionRemovalCleanupRequirement,
    };
    use async_trait::async_trait;
    use ironclaw_extensions::{
        ExtensionInstallationStore, ExtensionLifecycleEvent, ExtensionLifecycleEventSink,
        ExtensionLifecycleService, ExtensionManifest, ExtensionRegistry, SharedExtensionRegistry,
    };
    use ironclaw_filesystem::{
        DiskFilesystem, Fault, FaultInjecting, FilesystemOperation, InMemoryBackend,
    };
    use ironclaw_host_api::{
        AgentId, CapabilityId, ExtensionLifecycleOperation, HostPath, HostPortCatalog,
        InvocationId, MountAlias, MountGrant, MountPermissions, MountView, NetworkMethod,
        ProjectId, ResourceScope, RuntimeCredentialAccountSetup, RuntimeHttpEgress,
        RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, TenantId,
        TrustClass, UserId, VirtualPath,
    };
    use ironclaw_host_runtime::{SPAWN_SUBAGENT_CAPABILITY_ID, builtin_first_party_package};
    use ironclaw_product::{
        LifecycleExtensionRuntimeKind, LifecycleExtensionSource, LifecycleProductAction,
        LifecycleProductContext, LifecycleProductFacade, LifecycleProductSurfaceContext,
        LifecycleReadinessBlocker, RebornChannelConnectStrategy,
    };
    use ironclaw_trust::{HostTrustPolicy, InvalidationBus, TrustPolicy};

    mod private_install_tests;

    fn filesystem_installation_store() -> ExtensionInstallationStore {
        let host_ports =
            ironclaw_host_runtime::default_host_port_catalog().expect("default host port catalog");
        let contracts = product_extension_host_api_contract_registry().expect("host API contracts");
        futures::executor::block_on(ExtensionInstallationStore::load_at(
            Arc::new(InMemoryBackend::new()),
            VirtualPath::new("/system/extensions/.installations/test").expect("valid root"),
            host_ports,
            contracts,
        ))
        .expect("filesystem store")
    }

    #[tokio::test]
    async fn restore_narrows_legacy_tenant_membership_to_the_operator() {
        let store = Arc::new(filesystem_installation_store());
        let extension_id = ExtensionId::new("legacy-ready").expect("extension id");
        let installation_id =
            ExtensionInstallationId::new("legacy-ready").expect("installation id");
        let legacy_manifest = fixture_extension_manifest()
            .replace("id = \"fixture\"", "id = \"legacy-ready\"")
            .replace("id = \"fixture.", "id = \"legacy-ready.");
        store
            .upsert_manifest(fixture_manifest_record_with_source(
                &legacy_manifest,
                ManifestSource::HostBundled,
                None,
            ))
            .await
            .expect("persist legacy manifest");
        store
            .upsert_installation(
                ExtensionInstallation::new(
                    installation_id,
                    extension_id.clone(),
                    ExtensionManifestRef::new(extension_id, None),
                    Vec::new(),
                    chrono::Utc::now(),
                    InstallationOwner::Tenant,
                )
                .expect("legacy installation"),
            )
            .await
            .expect("persist legacy installation");
        let store: Arc<dyn ExtensionInstallationStorePort> = store;
        let operator = UserId::new("operator").expect("operator user id");

        let restored = canonicalize_persisted_installation_rows(&store, &operator)
            .await
            .expect("legacy row narrows");

        assert_eq!(restored.len(), 1);
        assert_eq!(
            restored[0].owner(),
            &InstallationOwner::user(operator.clone())
        );
    }

    #[tokio::test]
    async fn lifecycle_owner_projections_canonicalize_duplicate_extension_ids() {
        let (_dir, _storage_root, port, _active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let extension_id = ExtensionId::new("fixture").unwrap();
        installation_store
            .upsert_manifest(fixture_manifest_record_with_source(
                fixture_extension_manifest(),
                ManifestSource::HostBundled,
                None,
            ))
            .await
            .unwrap();

        for installation_id in ["fixture", "legacy-fixture"] {
            installation_store
                .upsert_installation(
                    ExtensionInstallation::new(
                        ExtensionInstallationId::new(installation_id).unwrap(),
                        extension_id.clone(),
                        ExtensionManifestRef::new(extension_id.clone(), None),
                        Vec::new(),
                        chrono::Utc::now(),
                        InstallationOwner::Tenant,
                    )
                    .unwrap(),
                )
                .await
                .unwrap();
        }

        let owners = port
            .installation_owners()
            .await
            .expect("duplicate owner rows canonicalize");
        assert_eq!(owners.get(&extension_id), Some(&InstallationOwner::Tenant));

        let active_capabilities = port
            .active_model_visible_capabilities()
            .await
            .expect("duplicate active owner rows canonicalize");
        assert!(active_capabilities.is_empty());
    }

    #[test]
    fn installed_external_channel_search_result_gets_activation_guidance() {
        let payload = LifecycleProductPayload::ExtensionSearch {
            extensions: vec![LifecycleSearchExtensionSummary {
                summary: LifecycleExtensionSummary {
                    package_ref: LifecyclePackageRef::new(
                        LifecyclePackageKind::Extension,
                        "acme-channel",
                    )
                    .expect("valid package ref"),
                    name: "Slack".to_string(),
                    version: "1.0.0".to_string(),
                    description: "Slack channel".to_string(),
                    source: LifecycleExtensionSource::HostBundled,
                    runtime_kind: LifecycleExtensionRuntimeKind::WasmTool,
                    surface_kinds: vec![CapabilitySurfaceKind::Channel],
                    channel_directions: None,
                    channel_connection: None,
                    channel_presentation: None,
                    visible_capability_ids: Vec::new(),
                    visible_read_only_capability_ids: Vec::new(),
                    credential_requirements: Vec::new(),
                    onboarding: None,
                },
                installation_phase: Some(LifecyclePublicState::SetupNeeded),
            }],
            count: 1,
        };

        assert!(extension_search_has_installed_external_channel_result(
            Some(&payload)
        ));
        assert!(!extension_search_has_ready_result(Some(&payload)));
    }

    #[test]
    fn activation_message_enumerates_published_tools_by_exact_name() {
        // Regression: the model only sees a *count* of deferred tools, so after
        // activating an extension it must be handed the exact tool names or it
        // assumes they are unavailable and gives up. The success message must name
        // every published capability and steer the model to direct invocation.
        let package = fixture_extension_package().package;
        let visible_capability_ids = vec!["fixture.search".to_string()];
        let message = connection_success_message(&package, &visible_capability_ids, None);
        assert!(message.contains("fixture.search"));
        assert!(
            message.contains("callable by exact name"),
            "must steer the model to tool_call by name, got: {message}"
        );
        assert!(
            message.contains("Do NOT call tool_search for these"),
            "must stop the model from re-searching for already-named tools, got: {message}"
        );
    }

    #[test]
    fn activation_message_without_published_tools_keeps_the_base_message_only() {
        // Channel-only / tool-less extensions publish no model tools; the message
        // must not invent an empty tool list or the direct-invocation guidance.
        let package = fixture_extension_package().package;
        let message = connection_success_message(&package, &[], None);
        assert!(message.contains("Extension setup completed"));
        assert!(
            !message.contains("callable by exact name"),
            "no tools published ⇒ no direct-invocation guidance, got: {message}"
        );
    }

    #[test]
    fn manifest_migration_preserves_the_exact_membership_set() {
        let extension_id = ExtensionId::new("fixture").expect("extension id");
        let alice = UserId::new("alice").expect("user id");
        let bob = UserId::new("bob").expect("user id");
        let owner = InstallationOwner::users(BTreeSet::from([alice.clone(), bob.clone()]))
            .expect("non-empty owners");
        let existing = ExtensionInstallation::new(
            ExtensionInstallationId::new("fixture").expect("installation id"),
            extension_id.clone(),
            ExtensionManifestRef::new(extension_id, None),
            Vec::new(),
            chrono::Utc::now(),
            owner,
        )
        .expect("installation");

        let plan = prepare_manifest_migration(&fixture_extension_package(), &existing)
            .expect("manifest migration plan");

        assert_eq!(
            plan.installation.owner().members(),
            Some(&BTreeSet::from([alice, bob]))
        );
    }

    #[tokio::test]
    async fn extension_lifecycle_installs_and_removes_catalog_package() {
        let (_dir, storage_root, facade, active_registry, _installation_store) =
            extension_lifecycle_fixture();

        // safety: test-only lifecycle facade calls; no database transaction is involved.
        let search = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionSearch {
                    query: "fixture".to_string(),
                },
            )
            .await
            .expect("search extensions");
        assert_eq!(search.phase, LifecyclePublicState::SetupNeeded);
        let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) =
            search.payload.as_ref()
        else {
            panic!("expected extension search payload");
        };
        assert_eq!(extensions.len(), 1);
        assert_eq!(
            extensions[0].summary.visible_read_only_capability_ids,
            vec!["fixture.search"]
        );

        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");
        let install = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install extension");
        assert_eq!(install.phase, LifecyclePublicState::Active);
        assert!(
            storage_root
                .join("system/extensions/fixture/manifest.toml")
                .exists()
        );
        assert!(
            storage_root
                .join("system/extensions/fixture/wasm/fixture.wasm")
                .exists()
        );
        let active = active_registry.snapshot();
        assert!(
            active
                .get_extension(&ExtensionId::new("fixture").unwrap())
                .is_some()
        );
        assert!(
            active
                .get_capability(&ironclaw_host_api::CapabilityId::new("fixture.search").unwrap())
                .is_some()
        );
        assert!(
            active
                .get_capability(&ironclaw_host_api::CapabilityId::new("fixture.write").unwrap())
                .is_some()
        );

        let remove = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove { package_ref },
            )
            .await
            .expect("remove extension");
        assert_eq!(remove.phase, LifecyclePublicState::Uninstalled);
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("fixture").unwrap())
                .is_none()
        );
        assert!(
            !storage_root
                .join("system/extensions/fixture/manifest.toml")
                .exists()
        );
        assert!(
            !storage_root
                .join("system/extensions/fixture/wasm/fixture.wasm")
                .exists()
        );
    }

    #[test]
    fn channel_connect_strategy_is_declared_only_by_the_resolved_manifest() {
        let catalog =
            crate::extension_host::available_extensions::AvailableExtensionCatalog::from_first_party_assets()
                .expect("first-party catalog");
        let slack_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "slack")
            .expect("slack package ref");
        let slack = catalog.resolve(&slack_ref).expect("slack package");
        let requirement = slack
            .summary()
            .channel_connection
            .expect("Slack manifest declares its channel connection");
        assert_eq!(requirement.strategy, RebornChannelConnectStrategy::OAuth);
        assert_eq!(requirement.channel, "slack");
        assert_eq!(requirement.display_name, "Slack");
        assert_eq!(requirement.input_placeholder, "");
        assert_eq!(requirement.submit_label, "Connect Slack");
        assert!(
            requirement
                .instructions
                .contains("Slack account with OAuth")
        );

        let bot_token_named_slack = fixture_external_channel_package("slack", "Slack");
        assert!(
            bot_token_named_slack.summary().channel_connection.is_none(),
            "a channel without [channel.connection] gets no inferred recipe, even when its id is slack",
        );
    }

    #[tokio::test]
    async fn readiness_reconciliation_does_not_invent_pairing_for_an_undeclared_channel() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![fixture_external_channel_package(
                    "signal", "Signal",
                )]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "signal").expect("valid ref");
        let install = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install external channel");

        assert_eq!(install.phase, LifecyclePublicState::Active);
        let message = install.message.as_deref().expect("install message");
        assert!(
            message.contains("Signal is installed as a channel surface")
                && message.contains("structured connection state")
                && message.contains("do not invent pairing commands")
                && message.contains("delivered by the host's outbound delivery"),
            "undeclared channel setup must remain honest, got: {message}"
        );
        let Some(LifecycleProductPayload::ExtensionInstall {
            visible_capability_ids,
            connection_required,
            ..
        }) = install.payload.as_ref()
        else {
            panic!("expected extension readiness payload");
        };
        assert!(
            visible_capability_ids.is_empty(),
            "a channel-only extension is valid without model tools"
        );
        assert!(
            connection_required.is_none(),
            "absence of [channel.connection] must not become a proof-code fallback"
        );
    }

    #[tokio::test]
    async fn generic_external_channel_remove_succeeds_without_cleanup_facade() {
        let (_dir, storage_root, facade, _active_registry, installation_store) =
            extension_lifecycle_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![fixture_external_channel_package(
                    "telegram", "Telegram",
                )]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "telegram")
            .expect("valid ref");
        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install external channel");

        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("generic external channel removes without host-owned cleanup");

        assert!(
            !storage_root.join("system/extensions/telegram").exists(),
            "package files must be deleted"
        );
        assert!(
            installation_store
                .get_manifest(&ExtensionId::new("telegram").expect("valid extension id"))
                .await
                .expect("read manifest")
                .is_none(),
            "manifest record must be deleted"
        );
        assert!(
            installation_store
                .get_installation(
                    &ExtensionInstallationId::new("telegram").expect("valid installation id")
                )
                .await
                .expect("read installation")
                .is_none(),
            "installation record must be deleted"
        );
    }

    #[tokio::test]
    async fn extension_remove_without_cleanup_or_credentials_does_not_require_actor() {
        let (_dir, storage_root, port, _active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![fixture_external_channel_package(
                    "telegram", "Telegram",
                )]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "telegram")
            .expect("valid ref");
        let scope = hosted_mcp_scope("lifecycle-owner");
        port.install(package_ref.clone(), &scope.user_id)
            .await
            .expect("install external channel");

        let remove = port
            .remove(package_ref, &scope, None)
            .await
            .expect("cleanup-free extension removal needs no actor");

        assert_eq!(remove.phase, LifecyclePublicState::Uninstalled);
        assert!(
            !storage_root.join("system/extensions/telegram").exists(),
            "package files must be deleted"
        );
        assert!(
            installation_store
                .list_installations()
                .await
                .expect("list installations")
                .is_empty(),
            "installation record must be deleted"
        );
    }

    #[derive(Debug, Clone)]
    struct RemovalCleanupObservation {
        context: ExtensionRemovalCleanupContext,
        binding: ExtensionRemovalCleanupBinding,
        package_files_present: bool,
        manifest_present: bool,
        installation_present: bool,
    }

    #[derive(Clone)]
    struct RemovalCleanupProbe {
        package_dir: std::path::PathBuf,
        installation_store: Arc<ExtensionInstallationStore>,
        extension_id: ExtensionId,
        installation_id: ExtensionInstallationId,
    }

    struct RecordingExtensionRemovalCleanupAdapter {
        id: ExtensionRemovalCleanupAdapterId,
        calls: StdMutex<Vec<RemovalCleanupObservation>>,
        probe: StdMutex<Option<RemovalCleanupProbe>>,
        failure_detail: Option<&'static str>,
    }

    impl RecordingExtensionRemovalCleanupAdapter {
        fn new(id: &str) -> Self {
            Self {
                id: ExtensionRemovalCleanupAdapterId::new(id).expect("valid cleanup adapter id"),
                calls: StdMutex::new(Vec::new()),
                probe: StdMutex::new(None),
                failure_detail: None,
            }
        }

        fn failing(id: &str, detail: &'static str) -> Self {
            Self {
                failure_detail: Some(detail),
                ..Self::new(id)
            }
        }

        fn set_probe(
            &self,
            storage_root: &std::path::Path,
            installation_store: Arc<ExtensionInstallationStore>,
            extension_id: &str,
        ) {
            *self.probe.lock().expect("cleanup probe lock") = Some(RemovalCleanupProbe {
                package_dir: storage_root.join(format!("system/extensions/{extension_id}")),
                installation_store,
                extension_id: ExtensionId::new(extension_id).expect("valid extension id"),
                installation_id: ExtensionInstallationId::new(extension_id)
                    .expect("valid installation id"),
            });
        }

        fn calls(&self) -> Vec<RemovalCleanupObservation> {
            self.calls.lock().expect("cleanup calls lock").clone()
        }
    }

    #[async_trait]
    impl ExtensionRemovalCleanupAdapter for RecordingExtensionRemovalCleanupAdapter {
        fn adapter_id(&self) -> ExtensionRemovalCleanupAdapterId {
            self.id.clone()
        }

        async fn cleanup(
            &self,
            context: &ExtensionRemovalCleanupContext,
            binding: &ExtensionRemovalCleanupBinding,
        ) -> Result<(), ProductSurfaceError> {
            let probe = self.probe.lock().expect("cleanup probe lock").clone();
            let (package_files_present, manifest_present, installation_present) =
                if let Some(probe) = probe {
                    let manifest_present = probe
                        .installation_store
                        .get_manifest(&probe.extension_id)
                        .await
                        .expect("manifest probe")
                        .is_some();
                    let installation_present = probe
                        .installation_store
                        .get_installation(&probe.installation_id)
                        .await
                        .expect("installation probe")
                        .is_some();
                    (
                        probe.package_dir.exists(),
                        manifest_present,
                        installation_present,
                    )
                } else {
                    (false, false, false)
                };
            self.calls
                .lock()
                .expect("cleanup calls lock")
                .push(RemovalCleanupObservation {
                    context: context.clone(),
                    binding: binding.clone(),
                    package_files_present,
                    manifest_present,
                    installation_present,
                });
            if let Some(detail) = self.failure_detail {
                return Err(ProductSurfaceError::internal_from(detail));
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn non_final_member_remove_runs_actor_cleanup_and_keeps_other_member_installed() {
        // safety: test-only facade calls are independent lifecycle requests, not database writes.
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").expect("valid ref");
        let github = fixture_github_package_with_cleanup(removal_cleanup_requirement(
            "fixture.cleanup",
            "github",
        ));

        let external_cleanup = Arc::new(RecordingExtensionRemovalCleanupAdapter::new(
            "fixture.cleanup",
        ));
        let external_cleanup_adapter: Arc<dyn ExtensionRemovalCleanupAdapter> =
            external_cleanup.clone();
        let external_cleanup_registry = Arc::new(
            ExtensionRemovalCleanupRegistry::try_from_adapters(vec![external_cleanup_adapter])
                .expect("unique cleanup adapter"),
        );
        let credential_cleanup = Arc::new(RecordingExtensionCredentialCleanup::default());
        let (_dir, storage_root, facade, _active_registry, installation_store) =
            extension_lifecycle_fixture_with_all_cleanup(
                AvailableExtensionCatalog::from_packages(vec![github]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
                Some(credential_cleanup.clone() as Arc<dyn ExtensionCredentialCleanup>),
                external_cleanup_registry,
                None,
            );
        external_cleanup.set_probe(&storage_root, installation_store.clone(), "github");

        for member in ["alice", "bob"] {
            facade
                .execute(
                    lifecycle_surface_context_for_user(member),
                    LifecycleProductAction::ExtensionInstall {
                        package_ref: package_ref.clone(),
                    },
                )
                .await
                .expect("member installs github");
        }

        facade
            .execute(
                lifecycle_surface_context_for_user("alice"),
                LifecycleProductAction::ExtensionRemove {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("alice leaves github");

        let calls = external_cleanup.calls();
        assert_eq!(calls.len(), 1, "external cleanup runs for the leaving user");
        assert_eq!(calls[0].context.authenticated_actor.as_str(), "alice");
        assert_eq!(calls[0].context.scope.user_id.as_str(), "alice");
        assert!(calls[0].package_files_present);
        assert!(calls[0].manifest_present);
        assert!(calls[0].installation_present);

        let (extension_request, credential_request) = {
            let credential_requests = credential_cleanup
                .requests
                .lock()
                .expect("credential cleanup lock");
            // The leaving member's remove issues the extension-keyed cleanup
            // (flows + grants) first, then the provider-selected revocation.
            assert_eq!(credential_requests.len(), 2);
            (
                credential_requests[0].clone(),
                credential_requests[1].clone(),
            )
        };
        assert!(extension_request.provider.is_none());
        assert_eq!(
            extension_request
                .lifecycle_package
                .as_ref()
                .map(|package| package.as_str()),
            Some("github")
        );
        assert_eq!(extension_request.scope.resource.user_id.as_str(), "alice");
        assert_eq!(
            credential_request
                .provider
                .as_ref()
                .map(AuthProviderId::as_str),
            Some("github")
        );
        assert_eq!(credential_request.scope.resource.user_id.as_str(), "alice");

        let installation = installation_store
            .get_installation(
                &ExtensionInstallationId::new("github").expect("valid installation id"),
            )
            .await
            .expect("installation lookup")
            .expect("bob keeps the installation");
        let alice = UserId::new("alice").expect("alice");
        let bob = UserId::new("bob").expect("bob");
        assert!(!installation.owner().visible_to(&alice));
        assert!(installation.owner().visible_to(&bob));
        assert!(storage_root.join("system/extensions/github").exists());
        assert!(
            installation_store
                .get_manifest(&ExtensionId::new("github").expect("extension id"))
                .await
                .expect("manifest lookup")
                .is_some()
        );
    }

    /// A v3 channel+auth fixture (mirrors the slack manifest shape): the
    /// §6.4 removal-disconnect predicate is manifest-derived — a `[channel]`
    /// surface plus at least one `[auth.*]` vendor means per-caller identity
    /// bindings can exist, so removal must run the per-caller disconnect.
    fn fixture_connectable_channel_package() -> AvailableExtensionPackage {
        let manifest_toml = r#"
schema_version = "reborn.extension_manifest.v3"
id = "acmechat"
name = "AcmeChat"
version = "0.1.0"
description = "connectable channel removal fixture"
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "acmechat.extension/v1"

[[tools]]
id = "acmechat.read_messages"
description = "Read AcmeChat messages"
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/acmechat/read_messages.input.v1.json"

[[tools.credentials]]
handle = "acmechat_user_token"
vendor = "acmechat"
scopes = ["messages.read"]
audience = { scheme = "https", host = "api.acmechat.example" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }

[channel]
id = "messages"
display_name = "AcmeChat messages"
inbound = true
outbound = true
conversation_model = "continuous"

[channel.ingress]
route_suffix = "events"
method = "post"
body_limit_bytes = 1048576

[channel.ingress.verification]
kind = "shared_secret_header"
secret_handle = "acmechat_webhook_secret"
header = "X-AcmeChat-Secret"

[admin_configuration]
group_id = "extension.acmechat"
display_name = "AcmeChat deployment configuration"
fields = [
  { handle = "acmechat_webhook_secret", label = "Webhook secret", secret = true, required = true },
  { handle = "acmechat_team_id", label = "Workspace ID", secret = false, required = true },
]

[channel.presentation]
supports_markdown = false
supports_threads = false

[auth.acmechat]
method = "oauth2_code"
display_name = "AcmeChat account"
authorization_endpoint = "https://auth.acmechat.example/authorize"
token_endpoint = "https://auth.acmechat.example/token"
scopes = ["messages.read"]
client_credentials = { client_id_handle = "acmechat_oauth_client_id" }

[auth.acmechat.token_response]
access_token = "/access_token"

[auth.acmechat.identity]
account_id = "/authed_user/id"
team_id = "/team/id"
"#;
        // Parse through the production version-dispatching entry point
        // (`ExtensionManifestRecord::from_toml`, the same seam
        // `bundled_extension_package` uses for the bundled v3 manifests);
        // `ExtensionManifest::parse` is the v2-only reader and rejects the
        // deliberate v3 shape above.
        let record = ExtensionManifestRecord::from_toml(
            manifest_toml,
            ManifestSource::HostBundled,
            &ironclaw_host_runtime::default_host_port_catalog().expect("host port catalog"),
            None,
            &product_extension_host_api_contract_registry().expect("host API contracts"),
        )
        .expect("connectable channel fixture manifest");
        let manifest: ExtensionManifest = record
            .manifest()
            .clone()
            .try_into()
            .expect("connectable channel fixture manifest lowers to a package manifest");
        fixture_extension_package_from_parsed_manifest(
            manifest_toml,
            "acmechat",
            manifest,
            Arc::new(record.resolved().clone()),
        )
    }

    /// A channel-only v3 fixture mirroring Telegram's manifest shape: the
    /// user's connection is owned by proof-code pairing, so there is no
    /// `[auth.*]` vendor even though removal must still disconnect the caller.
    fn fixture_pairing_channel_package() -> AvailableExtensionPackage {
        let manifest_toml = r#"
schema_version = "reborn.extension_manifest.v3"
id = "pairchat"
name = "PairChat"
version = "0.1.0"
description = "proof-code paired channel removal fixture"
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "pairchat.extension/v1"

[channel]
id = "messages"
display_name = "PairChat messages"
inbound = true
outbound = true
conversation_model = "continuous"

[channel.ingress]
route_suffix = "updates"
method = "post"
body_limit_bytes = 1048576

[channel.ingress.verification]
kind = "shared_secret_header"
secret_handle = "pairchat_webhook_secret"
header = "X-PairChat-Secret"

[admin_configuration]
group_id = "extension.pairchat"
display_name = "PairChat deployment configuration"
fields = [
  { handle = "pairchat_bot_token", label = "Bot token", secret = true, required = true },
  { handle = "pairchat_webhook_secret", label = "Webhook secret", secret = true, required = true },
]

[[channel.egress]]
scheme = "https"
host = "api.pairchat.example"
methods = ["post"]
credential_handle = "pairchat_bot_token"
injection = { type = "header", name = "authorization", prefix = "Bearer " }

[channel.presentation]
supports_markdown = false
supports_threads = true
"#;
        let record = ExtensionManifestRecord::from_toml(
            manifest_toml,
            ManifestSource::HostBundled,
            &ironclaw_host_runtime::default_host_port_catalog().expect("host port catalog"),
            None,
            &product_extension_host_api_contract_registry().expect("host API contracts"),
        )
        .expect("pairing channel fixture manifest");
        let manifest: ExtensionManifest = record
            .manifest()
            .clone()
            .try_into()
            .expect("pairing channel fixture manifest lowers to a package manifest");
        fixture_extension_package_from_parsed_manifest(
            manifest_toml,
            "pairchat",
            manifest,
            Arc::new(record.resolved().clone()),
        )
    }

    /// Recording double for the §6.4 per-caller disconnect the removal path
    /// dispatches through the late-bound facade slot. `fail_next(n)` scripts
    /// the next `n` disconnects to fail so retry convergence can be pinned.
    #[derive(Default)]
    struct RecordingChannelConnectionFacade {
        disconnects: StdMutex<Vec<(ProductSurfaceCaller, String)>>,
        failures_remaining: AtomicUsize,
    }

    impl RecordingChannelConnectionFacade {
        fn fail_next(&self, count: usize) {
            self.failures_remaining.store(count, Ordering::SeqCst);
        }

        fn disconnects(&self) -> Vec<(ProductSurfaceCaller, String)> {
            self.disconnects.lock().expect("disconnect lock").clone()
        }
    }

    #[async_trait]
    impl ChannelConnectionFacade for RecordingChannelConnectionFacade {
        async fn caller_channel_connections(
            &self,
            _caller: ProductSurfaceCaller,
        ) -> Result<std::collections::HashMap<String, bool>, ProductSurfaceError> {
            Ok(std::collections::HashMap::new())
        }

        async fn disconnect_channel_for_caller(
            &self,
            caller: ProductSurfaceCaller,
            channel: &str,
        ) -> Result<(), ProductSurfaceError> {
            self.disconnects
                .lock()
                .expect("disconnect lock")
                .push((caller, channel.to_string()));
            if self
                .failures_remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(ProductSurfaceError::internal_from("disconnect unavailable"));
            }
            Ok(())
        }
    }

    fn connectable_channel_removal_fixture(
        slot: Option<Arc<std::sync::OnceLock<Arc<dyn ChannelConnectionFacade>>>>,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        crate::extension_host::lifecycle::LifecycleFacade,
        Arc<SharedExtensionRegistry>,
        Arc<ExtensionInstallationStore>,
    ) {
        extension_lifecycle_fixture_with_all_cleanup(
            AvailableExtensionCatalog::from_packages(vec![fixture_connectable_channel_package()]),
            ExtensionLifecycleService::new(ExtensionRegistry::new()),
            None,
            Arc::new(ExtensionRemovalCleanupRegistry::empty()),
            slot,
        )
    }

    /// §6.4 / issue #6091: removing a channel+auth extension runs the REAL
    /// per-caller disconnect through the late-bound facade slot, with the
    /// authenticated caller's identity, before teardown — and an empty slot
    /// fails the removal closed (typed retryable error, installation kept)
    /// instead of skipping the disconnect.
    #[tokio::test]
    async fn extension_remove_of_connectable_channel_disconnects_the_caller() {
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "acmechat")
            .expect("valid ref");
        let channel_connection = Arc::new(RecordingChannelConnectionFacade::default());
        let slot: Arc<std::sync::OnceLock<Arc<dyn ChannelConnectionFacade>>> =
            Arc::new(std::sync::OnceLock::new());
        slot.set(channel_connection.clone() as Arc<dyn ChannelConnectionFacade>)
            .ok();
        let (_dir, _storage_root, facade, _active_registry, installation_store) =
            connectable_channel_removal_fixture(Some(Arc::clone(&slot)));

        facade
            .execute(
                lifecycle_surface_context_for_user("alice"),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("alice installs acmechat");
        facade
            .execute(
                lifecycle_surface_context_for_user("alice"),
                LifecycleProductAction::ExtensionRemove {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("alice removes acmechat");

        let disconnects = channel_connection.disconnects();
        assert_eq!(disconnects.len(), 1, "removal runs exactly one disconnect");
        assert_eq!(disconnects[0].1, "acmechat");
        assert_eq!(
            disconnects[0].0.user_id.as_str(),
            "alice",
            "the disconnect caller is the authenticated removal actor"
        );
        assert!(
            installation_store
                .get_installation(
                    &ExtensionInstallationId::new("acmechat").expect("valid installation id")
                )
                .await
                .expect("installation lookup")
                .is_none(),
            "the removal itself completed"
        );

        // Empty slot: fail closed. A channel surface backed by an auth vendor
        // may hold per-caller identity bindings, and a composition that gives
        // the removal path no facade to disconnect them through must not
        // report the removal as successful — the typed retryable error keeps
        // the installation authoritative for a retry.
        let (_dir2, _storage_root2, unwired_facade, _registry2, unwired_store) =
            connectable_channel_removal_fixture(None);
        unwired_facade
            .execute(
                lifecycle_surface_context_for_user("alice"),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("alice installs acmechat without a facade slot");
        let error = unwired_facade
            .execute(
                lifecycle_surface_context_for_user("alice"),
                LifecycleProductAction::ExtensionRemove { package_ref },
            )
            .await
            .expect_err("removal without a composed channel-connection facade must fail closed");
        assert!(
            matches!(
                &error,
                ProductWorkflowError::Transient { reason }
                    if reason.contains("channel connection cleanup")
            ),
            "empty-slot removal surfaces the typed retryable cleanup error: {error:?}"
        );
        assert!(
            unwired_store
                .get_installation(
                    &ExtensionInstallationId::new("acmechat").expect("valid installation id")
                )
                .await
                .expect("installation lookup")
                .is_some(),
            "fail-closed removal must keep the installation for a retry"
        );
    }

    /// Channel removal cleanup is keyed by the manifest's channel surface,
    /// not by OAuth. Proof-code paired channels hold the same caller-owned
    /// identity and conversation bindings and must cross the shared
    /// disconnect boundary before their installation row is deleted.
    #[tokio::test]
    async fn extension_remove_of_pairing_channel_disconnects_the_caller() {
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "pairchat")
            .expect("valid ref");
        let channel_connection = Arc::new(RecordingChannelConnectionFacade::default());
        let slot: Arc<std::sync::OnceLock<Arc<dyn ChannelConnectionFacade>>> =
            Arc::new(std::sync::OnceLock::new());
        slot.set(channel_connection.clone() as Arc<dyn ChannelConnectionFacade>)
            .ok();
        let (_dir, _storage_root, facade, _active_registry, installation_store) =
            extension_lifecycle_fixture_with_all_cleanup(
                AvailableExtensionCatalog::from_packages(vec![fixture_pairing_channel_package()]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
                None,
                Arc::new(ExtensionRemovalCleanupRegistry::empty()),
                Some(slot),
            );

        facade
            .execute(
                lifecycle_surface_context_for_user("alice"),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("alice installs pairchat");
        facade
            .execute(
                lifecycle_surface_context_for_user("alice"),
                LifecycleProductAction::ExtensionRemove { package_ref },
            )
            .await
            .expect("alice removes pairchat");

        let disconnects = channel_connection.disconnects();
        assert_eq!(disconnects.len(), 1, "removal runs exactly one disconnect");
        assert_eq!(disconnects[0].1, "pairchat");
        assert_eq!(disconnects[0].0.user_id.as_str(), "alice");
        assert!(
            installation_store
                .get_installation(
                    &ExtensionInstallationId::new("pairchat").expect("valid installation id")
                )
                .await
                .expect("installation lookup")
                .is_none(),
            "the installation is deleted only after disconnect succeeds"
        );
    }

    /// Retry convergence: a failing disconnect keeps the installation
    /// authoritative and surfaces a retryable error; the retry re-runs the
    /// full disconnect and converges once it succeeds.
    #[tokio::test]
    async fn extension_remove_stays_retryable_when_channel_disconnect_fails() {
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "acmechat")
            .expect("valid ref");
        let channel_connection = Arc::new(RecordingChannelConnectionFacade::default());
        channel_connection.fail_next(1);
        let slot: Arc<std::sync::OnceLock<Arc<dyn ChannelConnectionFacade>>> =
            Arc::new(std::sync::OnceLock::new());
        slot.set(channel_connection.clone() as Arc<dyn ChannelConnectionFacade>)
            .ok();
        let (_dir, _storage_root, facade, _active_registry, installation_store) =
            connectable_channel_removal_fixture(Some(Arc::clone(&slot)));

        facade
            .execute(
                lifecycle_surface_context_for_user("alice"),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("alice installs acmechat");
        let error = facade
            .execute(
                lifecycle_surface_context_for_user("alice"),
                LifecycleProductAction::ExtensionRemove {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect_err("disconnect failure must fail the removal");
        assert!(
            matches!(
                &error,
                ProductWorkflowError::Transient { reason }
                    if reason.contains("channel connection cleanup")
            ),
            "disconnect failures stay retryable: {error:?}"
        );
        assert!(
            installation_store
                .get_installation(
                    &ExtensionInstallationId::new("acmechat").expect("valid installation id")
                )
                .await
                .expect("installation lookup")
                .is_some(),
            "the installation must survive the failed removal so the owner can retry"
        );

        facade
            .execute(
                lifecycle_surface_context_for_user("alice"),
                LifecycleProductAction::ExtensionRemove { package_ref },
            )
            .await
            .expect("retry converges once the disconnect succeeds");
        assert_eq!(
            channel_connection.disconnects().len(),
            2,
            "the retry re-runs the full disconnect"
        );
    }

    #[tokio::test]
    async fn extension_remove_dispatches_only_declared_adapter_with_trusted_scope_before_deletion()
    {
        let matching = Arc::new(RecordingExtensionRemovalCleanupAdapter::new(
            "fixture.cleanup",
        ));
        let unrelated = Arc::new(RecordingExtensionRemovalCleanupAdapter::new(
            "unrelated.cleanup",
        ));
        let matching_adapter: Arc<dyn ExtensionRemovalCleanupAdapter> = matching.clone();
        let unrelated_adapter: Arc<dyn ExtensionRemovalCleanupAdapter> = unrelated.clone();
        let registry = Arc::new(
            ExtensionRemovalCleanupRegistry::try_from_adapters(vec![
                unrelated_adapter,
                matching_adapter,
            ])
            .expect("unique cleanup adapters"),
        );
        let package = fixture_external_channel_package_with_cleanup(
            "telegram",
            "Telegram",
            removal_cleanup_requirement("fixture.cleanup", "telegram"),
        );
        let (_dir, storage_root, port, _active_registry, installation_store) =
            extension_management_port_fixture_with_removal_cleanup(
                AvailableExtensionCatalog::from_packages(vec![package]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
                registry,
            );
        matching.set_probe(&storage_root, installation_store.clone(), "telegram");
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "telegram")
            .expect("valid ref");
        let authenticated_actor = UserId::new("authenticated-actor").expect("valid actor");
        port.install(package_ref.clone(), &authenticated_actor)
            .await
            .expect("install external channel");
        let mut removal_scope = hosted_mcp_scope("scope-owner");
        removal_scope.tenant_id = TenantId::new("trusted-tenant").expect("valid tenant");
        removal_scope.agent_id = Some(AgentId::new("trusted-agent").expect("valid agent"));
        removal_scope.project_id = Some(ProjectId::new("trusted-project").expect("valid project"));
        let remove = port
            .remove(package_ref, &removal_scope, Some(&authenticated_actor))
            .await
            .expect("declared cleanup and removal succeed");

        assert_eq!(remove.phase, LifecyclePublicState::Uninstalled);
        let calls = matching.calls();
        assert_eq!(calls.len(), 1, "matching adapter runs exactly once");
        assert!(
            unrelated.calls().is_empty(),
            "unrelated adapter must not run"
        );
        let call = &calls[0];
        assert_eq!(call.context.authenticated_actor, authenticated_actor);
        assert_eq!(call.context.scope.tenant_id.as_str(), "trusted-tenant");
        assert_eq!(call.context.scope.user_id.as_str(), "authenticated-actor");
        assert_eq!(
            call.context.scope.agent_id.as_ref().map(AgentId::as_str),
            Some("trusted-agent")
        );
        assert_eq!(
            call.context
                .scope
                .project_id
                .as_ref()
                .map(ProjectId::as_str),
            Some("trusted-project")
        );
        assert_eq!(
            call.binding,
            ExtensionRemovalCleanupBinding::ChannelConnection {
                channel: ExtensionRemovalChannelId::new("telegram").expect("valid channel")
            }
        );
        assert!(
            call.package_files_present,
            "cleanup must precede file deletion"
        );
        assert!(
            call.manifest_present,
            "cleanup must precede manifest deletion"
        );
        assert!(
            call.installation_present,
            "cleanup must precede installation deletion"
        );
        assert!(
            !storage_root.join("system/extensions/telegram").exists(),
            "package files are removed only after cleanup succeeds"
        );
    }

    #[tokio::test]
    async fn extension_remove_with_declared_cleanup_requires_authenticated_actor() {
        let adapter = Arc::new(RecordingExtensionRemovalCleanupAdapter::new(
            "fixture.cleanup",
        ));
        let adapter_trait: Arc<dyn ExtensionRemovalCleanupAdapter> = adapter.clone();
        let registry = Arc::new(
            ExtensionRemovalCleanupRegistry::try_from_adapters(vec![adapter_trait])
                .expect("unique cleanup adapter"),
        );
        let package = fixture_external_channel_package_with_cleanup(
            "telegram",
            "Telegram",
            removal_cleanup_requirement("fixture.cleanup", "telegram"),
        );
        let (_dir, storage_root, port, _active_registry, installation_store) =
            extension_management_port_fixture_with_removal_cleanup(
                AvailableExtensionCatalog::from_packages(vec![package]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
                registry,
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "telegram")
            .expect("valid ref");
        let scope = hosted_mcp_scope("scope-owner");
        port.install(package_ref.clone(), &scope.user_id)
            .await
            .expect("install external channel");

        let error = port
            .remove(package_ref, &scope, None)
            .await
            .expect_err("declared cleanup requires an authenticated actor");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { reason }
                if reason.contains("removal cleanup requires an authenticated actor")
        ));
        assert!(
            adapter.calls().is_empty(),
            "adapter must not run without actor"
        );
        assert_removal_target_preserved(&storage_root, &installation_store, "telegram").await;
    }

    #[tokio::test]
    async fn extension_remove_masks_foreign_private_install_before_cleanup_actor_validation() {
        let adapter = Arc::new(RecordingExtensionRemovalCleanupAdapter::new(
            "fixture.cleanup",
        ));
        let adapter_trait: Arc<dyn ExtensionRemovalCleanupAdapter> = adapter.clone();
        let registry = Arc::new(
            ExtensionRemovalCleanupRegistry::try_from_adapters(vec![adapter_trait])
                .expect("unique cleanup adapter"),
        );
        let package = fixture_external_channel_package_with_cleanup(
            "telegram",
            "Telegram",
            removal_cleanup_requirement("fixture.cleanup", "telegram"),
        );
        let (_dir, storage_root, port, _active_registry, installation_store) =
            extension_management_port_fixture_with_removal_cleanup(
                AvailableExtensionCatalog::from_packages(vec![package]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
                registry,
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "telegram")
            .expect("valid ref");
        port.install(
            package_ref.clone(),
            &UserId::new("private-owner").expect("private owner"),
        )
        .await
        .expect("install private external channel");
        let foreign_scope = hosted_mcp_scope("foreign-user");

        let error = port
            .remove(package_ref, &foreign_scope, None)
            .await
            .expect_err("foreign private removal must stay masked");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { reason }
                if reason.contains("extension telegram is not installed")
                    && !reason.contains("authenticated actor")
        ));
        assert!(
            adapter.calls().is_empty(),
            "authorization must fail before cleanup dispatch"
        );
        assert_removal_target_preserved(&storage_root, &installation_store, "telegram").await;
    }

    #[tokio::test]
    async fn extension_remove_fails_closed_when_declared_cleanup_adapter_is_missing() {
        let registry = Arc::new(
            ExtensionRemovalCleanupRegistry::try_from_adapters(Vec::new())
                .expect("empty cleanup registry"),
        );
        let package = fixture_external_channel_package_with_cleanup(
            "telegram",
            "Telegram",
            removal_cleanup_requirement("missing.cleanup", "telegram"),
        );
        let (_dir, storage_root, port, _active_registry, installation_store) =
            extension_management_port_fixture_with_removal_cleanup(
                AvailableExtensionCatalog::from_packages(vec![package]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
                registry,
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "telegram")
            .expect("valid ref");
        let scope = hosted_mcp_scope("authenticated-actor");
        port.install(package_ref.clone(), &scope.user_id)
            .await
            .expect("install external channel");

        let error = port
            .remove(package_ref, &scope, Some(&scope.user_id))
            .await
            .expect_err("missing declared cleanup adapter must fail closed");

        assert!(matches!(
            error,
            ProductWorkflowError::Transient { reason }
                if reason.contains("required extension removal cleanup adapter is unavailable")
        ));
        assert_removal_target_preserved(&storage_root, &installation_store, "telegram").await;
    }

    #[tokio::test]
    async fn extension_remove_fails_closed_when_declared_cleanup_adapter_errors() {
        let secret_detail = "opaque backend detail: /private/credential-store";
        let adapter = Arc::new(RecordingExtensionRemovalCleanupAdapter::failing(
            "fixture.cleanup",
            secret_detail,
        ));
        let adapter_trait: Arc<dyn ExtensionRemovalCleanupAdapter> = adapter.clone();
        let registry = Arc::new(
            ExtensionRemovalCleanupRegistry::try_from_adapters(vec![adapter_trait])
                .expect("unique cleanup adapter"),
        );
        let package = fixture_external_channel_package_with_cleanup(
            "telegram",
            "Telegram",
            removal_cleanup_requirement("fixture.cleanup", "telegram"),
        );
        let (_dir, storage_root, port, _active_registry, installation_store) =
            extension_management_port_fixture_with_removal_cleanup(
                AvailableExtensionCatalog::from_packages(vec![package]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
                registry,
            );
        adapter.set_probe(&storage_root, installation_store.clone(), "telegram");
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "telegram")
            .expect("valid ref");
        let scope = hosted_mcp_scope("authenticated-actor");
        port.install(package_ref.clone(), &scope.user_id)
            .await
            .expect("install external channel");

        let error = port
            .remove(package_ref, &scope, Some(&scope.user_id))
            .await
            .expect_err("declared cleanup adapter failure must fail closed");

        let ProductWorkflowError::Transient { reason } = error else {
            panic!("adapter failure must be retryable");
        };
        assert!(reason.contains("fixture.cleanup"));
        assert!(!reason.contains(secret_detail));
        let calls = adapter.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].package_files_present);
        assert!(calls[0].manifest_present);
        assert!(calls[0].installation_present);
        assert_removal_target_preserved(&storage_root, &installation_store, "telegram").await;
    }

    #[tokio::test]
    async fn concurrent_extension_removals_run_declared_cleanup_once_under_single_operation_lock() {
        let adapter = Arc::new(RecordingExtensionRemovalCleanupAdapter::new(
            "fixture.cleanup",
        ));
        let adapter_trait: Arc<dyn ExtensionRemovalCleanupAdapter> = adapter.clone();
        let registry = Arc::new(
            ExtensionRemovalCleanupRegistry::try_from_adapters(vec![adapter_trait])
                .expect("unique cleanup adapter"),
        );
        let package = fixture_external_channel_package_with_cleanup(
            "telegram",
            "Telegram",
            removal_cleanup_requirement("fixture.cleanup", "telegram"),
        );
        let (_dir, _storage_root, port, _active_registry, _installation_store) =
            extension_management_port_fixture_with_removal_cleanup(
                AvailableExtensionCatalog::from_packages(vec![package]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
                registry,
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "telegram")
            .expect("valid ref");
        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install external channel");

        let operation_guard = port.operation_lock.lock().await;
        let start = Arc::new(tokio::sync::Barrier::new(3));
        let first_port = Arc::clone(&port);
        let first_ref = package_ref.clone();
        let first_start = Arc::clone(&start);
        let first = tokio::spawn(async move {
            let scope = hosted_mcp_scope("lifecycle-owner");
            let actor = scope.user_id.clone();
            first_start.wait().await;
            first_port.remove(first_ref, &scope, Some(&actor)).await
        });
        let second_port = Arc::clone(&port);
        let second_start = Arc::clone(&start);
        let second = tokio::spawn(async move {
            let scope = hosted_mcp_scope("lifecycle-owner");
            let actor = scope.user_id.clone();
            second_start.wait().await;
            second_port.remove(package_ref, &scope, Some(&actor)).await
        });
        start.wait().await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        drop(operation_guard);

        let (first_result, second_result) = tokio::join!(first, second);
        let results = [
            first_result.expect("first removal task joins"),
            second_result.expect("second removal task joins"),
        ];
        assert_eq!(
            results.iter().filter(|result| result.is_ok()).count(),
            1,
            "exactly one concurrent removal succeeds"
        );
        assert_eq!(
            results.iter().filter(|result| result.is_err()).count(),
            1,
            "the second concurrent removal observes the package is gone"
        );
        assert_eq!(
            adapter.calls().len(),
            1,
            "installation preflight, cleanup, and deletion must share one operation lock"
        );
    }

    #[tokio::test]
    async fn declared_cleanup_survives_fresh_service_restart_without_catalog_package() {
        let adapter = Arc::new(RecordingExtensionRemovalCleanupAdapter::new(
            "fixture.cleanup",
        ));
        let adapter_trait: Arc<dyn ExtensionRemovalCleanupAdapter> = adapter.clone();
        let registry = Arc::new(
            ExtensionRemovalCleanupRegistry::try_from_adapters(vec![adapter_trait])
                .expect("unique cleanup adapter"),
        );
        let package = fixture_external_channel_package_with_cleanup(
            "telegram",
            "Telegram",
            removal_cleanup_requirement("fixture.cleanup", "telegram"),
        );
        let (_dir, storage_root, installed_port, _active_registry, installation_store) =
            extension_management_port_fixture_with_removal_cleanup(
                AvailableExtensionCatalog::from_packages(vec![package]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
                Arc::clone(&registry),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "telegram")
            .expect("valid ref");
        installed_port
            .install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install extension");

        let restarted_port = ExtensionManagementPort::new(
            Arc::clone(&installed_port.filesystem),
            AvailableExtensionCatalog::from_packages(Vec::new()),
            installation_store.clone(),
            Arc::new(Mutex::new(ExtensionLifecycleService::new(
                ExtensionRegistry::new(),
            ))),
            installed_port.active_extensions.clone(),
            None,
        )
        .with_removal_cleanup_registry(registry);
        let scope = hosted_mcp_scope("lifecycle-owner");

        let response = restarted_port
            .remove(package_ref, &scope, Some(&scope.user_id))
            .await
            .expect("durable cleanup metadata must support restart removal");

        assert!(matches!(
            response.payload,
            Some(LifecycleProductPayload::ExtensionRemove { removed: true })
        ));
        assert_eq!(adapter.calls().len(), 1);
        assert!(!storage_root.join("system/extensions/telegram").exists());
        let extension_id = ExtensionId::new("telegram").expect("valid extension id");
        let installation_id =
            ExtensionInstallationId::new("telegram").expect("valid installation id");
        assert!(
            installation_store
                .get_installation(&installation_id)
                .await
                .expect("installation lookup")
                .is_none()
        );
        assert!(
            installation_store
                .get_manifest(&extension_id)
                .await
                .expect("manifest lookup")
                .is_none()
        );
    }

    #[tokio::test]
    async fn extension_search_distinguishes_external_channel_connect_from_delivery() {
        // Generic external-channel search guidance. Uses a neutral `example_bot`
        // fixture rather than the real Slack bot: under model B `slack_bot` is
        // hidden from search, so a Slack-named fixture would be filtered out and
        // this generic guidance would go untested.
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![fixture_external_channel_package(
                    "example_bot",
                    "Example",
                )]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "example_bot")
            .expect("valid ref");
        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install example channel");
        let search = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionSearch {
                    query: "example".to_string(),
                },
            )
            .await
            .expect("search active example channel");

        let message = search.message.as_deref().expect("search guidance");
        assert!(
            message.contains("external channel")
                && message.contains("explicit connect")
                && message.contains("builtin.extension_install")
                && message.contains("outbound delivery target")
                && message.contains("personal setup is incomplete"),
            "active external channel search should distinguish connect requests from delivery, got: {message}"
        );
        assert!(
            !message.contains("Treat those results as ready"),
            "active external channels must not use ready-extension guidance: {message}"
        );
        let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) =
            search.payload.as_ref()
        else {
            panic!("expected extension search payload");
        };
        let example = extensions
            .iter()
            .find(|extension| extension.summary.package_ref.id.as_str() == "example_bot")
            .expect("example search result");
        assert_eq!(
            example.installation_phase,
            Some(LifecyclePublicState::Active)
        );
    }

    #[tokio::test]
    async fn slack_tools_extension_install_publishes_capabilities_when_ready() {
        let (_dir, _storage_root, port, _active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        // Model B: the user-installable Slack extension is the tools package
        // (`slack`); the bot channel (`slack_bot`) is operator-provisioned and
        // hidden. Installing the tools extension installs only itself — there is
        // no hidden companion.
        let slack_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "slack").expect("slack ref");

        port.install(slack_ref.clone(), &lifecycle_owner())
            .await
            .expect("install Slack tools extension");

        let installed_ids = installation_store
            .list_installations()
            .await
            .expect("list installations")
            .into_iter()
            .map(|installation| installation.extension_id().as_str().to_string())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            installed_ids,
            ["slack"]
                .into_iter()
                .map(String::from)
                .collect::<BTreeSet<_>>(),
            "installing the Slack tools extension installs only itself, with no hidden companion"
        );

        let list = port
            .list_installed(&lifecycle_owner(), None)
            .await
            .expect("list installed");
        let Some(LifecycleProductPayload::ExtensionList { extensions, count }) = list.payload
        else {
            panic!("expected extension list payload");
        };
        assert_eq!(count, 1);
        assert_eq!(extensions[0].summary.package_ref.id.as_str(), "slack");

        let search = port
            .search("slack", None, &lifecycle_owner())
            .await
            .expect("search slack");
        let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) = search.payload
        else {
            panic!("expected extension search payload");
        };
        assert_eq!(
            extensions
                .iter()
                .map(|extension| extension.summary.package_ref.id.as_str())
                .collect::<Vec<_>>(),
            vec!["slack"],
            "search exposes the tools extension (slack); the bot channel (slack_bot) is hidden"
        );

        port.activate_with_prechecked_credentials_for_test(
            slack_ref,
            ExtensionActivationMode::Static,
            &lifecycle_owner(),
        )
        .await
        .expect("activate Slack tools extension");

        let active_capability_ids = port
            .active_model_visible_capabilities()
            .await
            .expect("active capabilities")
            .into_iter()
            .map(|capability| capability.id.as_str().to_string())
            .collect::<BTreeSet<_>>();
        assert!(
            active_capability_ids.contains("slack.search_messages"),
            "activating the Slack tools extension publishes its read tools"
        );
        assert!(
            active_capability_ids.contains("slack.send_message"),
            "activating the Slack tools extension publishes its write tool"
        );
    }

    #[tokio::test]
    async fn slack_tools_extension_activation_requires_personal_oauth() {
        let (_dir, _storage_root, port, _active_registry, _installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let slack_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "slack").expect("slack ref");

        port.install(slack_ref.clone(), &lifecycle_owner())
            .await
            .expect("install public Slack extension");

        let requirements = port
            .activation_credential_requirements(&slack_ref, &lifecycle_owner())
            .await
            .expect("Slack activation requirements");
        assert_eq!(requirements.len(), 1);
        let requirement = &requirements[0];
        assert_eq!(requirement.provider.as_str(), "slack");
        assert_eq!(requirement.requester_extension.as_str(), "slack");
        let expected_scopes = [
            "channels:history",
            "channels:read",
            "chat:write",
            "groups:history",
            "groups:read",
            "im:history",
            "im:read",
            "mpim:history",
            "mpim:read",
            "search:read",
            "users:read",
        ]
        .into_iter()
        .map(String::from)
        .collect::<BTreeSet<_>>();
        assert_eq!(
            requirement
                .provider_scopes
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>(),
            expected_scopes
        );
        let RuntimeCredentialAccountSetup::OAuth { scopes } = &requirement.setup else {
            panic!("Slack personal setup should use OAuth");
        };
        assert_eq!(
            scopes.iter().cloned().collect::<BTreeSet<_>>(),
            expected_scopes
        );
    }

    #[tokio::test]
    async fn slack_tools_extension_removal_fails_closed_without_channel_cleanup() {
        let (_dir, _storage_root, port, _active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let slack_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "slack").expect("slack ref");

        port.install(slack_ref.clone(), &lifecycle_owner())
            .await
            .expect("install public Slack extension");
        port.activate_with_prechecked_credentials_for_test(
            slack_ref.clone(),
            ExtensionActivationMode::Static,
            &lifecycle_owner(),
        )
        .await
        .expect("activate Slack and internal user tools");
        let removal_scope = hosted_mcp_scope("lifecycle-owner");
        let error = port
            .remove(slack_ref, &removal_scope, Some(&removal_scope.user_id))
            .await
            .expect_err("Slack removal without its cleanup facade must fail closed");
        assert!(matches!(error, ProductWorkflowError::Transient { .. }));

        let installed_ids = installation_store
            .list_installations()
            .await
            .expect("list installations")
            .into_iter()
            .map(|installation| installation.extension_id().as_str().to_string())
            .collect::<BTreeSet<_>>();
        assert!(
            installed_ids.contains("slack"),
            "failed cleanup must preserve the public Slack extension for a retry"
        );
        let active_capability_ids = port
            .active_model_visible_capabilities()
            .await
            .expect("active capabilities")
            .into_iter()
            .map(|capability| capability.id.as_str().to_string())
            .collect::<BTreeSet<_>>();
        assert!(
            active_capability_ids
                .iter()
                .any(|capability_id| capability_id.starts_with("slack.")),
            "failed cleanup must not partially remove active Slack tools"
        );
    }

    #[tokio::test]
    async fn active_model_visible_capabilities_only_include_enabled_lifecycle_extensions() {
        let (_dir, _storage_root, port, active_registry, _installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        active_registry
            .upsert(builtin_first_party_package().expect("builtin package"))
            .expect("seed builtin package");
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");
        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install fixture extension");
        port.activate_with_prechecked_credentials_for_test(
            package_ref,
            ExtensionActivationMode::Static,
            &lifecycle_owner(),
        )
        .await
        .expect("activate fixture extension");

        let capability_ids = port
            .active_model_visible_capabilities()
            .await
            .expect("active capabilities")
            .into_iter()
            .map(|capability| capability.id)
            .collect::<Vec<_>>();

        assert!(capability_ids.contains(&CapabilityId::new("fixture.search").unwrap()));
        assert!(!capability_ids.contains(&CapabilityId::new("fixture.write").unwrap()));
        assert!(
            !capability_ids.contains(&CapabilityId::new(SPAWN_SUBAGENT_CAPABILITY_ID).unwrap())
        );
    }

    #[test]
    fn activation_credential_requirements_coalesce_google_oauth_scope_sets() {
        let catalog =
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets");
        for (extension_id, expected_scopes) in [
            (
                "google-calendar",
                vec![
                    "https://www.googleapis.com/auth/calendar.events",
                    "https://www.googleapis.com/auth/calendar.readonly",
                ],
            ),
            (
                "gmail",
                vec![
                    "https://www.googleapis.com/auth/gmail.modify",
                    "https://www.googleapis.com/auth/gmail.readonly",
                    "https://www.googleapis.com/auth/gmail.send",
                ],
            ),
        ] {
            let package_ref =
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, extension_id)
                    .expect("valid package ref");
            let package = catalog
                .resolve(&package_ref)
                .expect("bundled Google package");

            let requirements = package_runtime_credential_auth_requirements(&package.package);

            assert_eq!(
                requirements.len(),
                1,
                "{extension_id} should activate with one Google OAuth requirement"
            );
            let requirement = &requirements[0];
            assert_eq!(requirement.provider.as_str(), "google");
            assert_eq!(requirement.requester_extension.as_str(), extension_id);
            let expected = expected_scopes
                .into_iter()
                .map(str::to_string)
                .collect::<BTreeSet<_>>();
            assert_eq!(
                requirement
                    .provider_scopes
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>(),
                expected
            );
            let RuntimeCredentialAccountSetup::OAuth { scopes } = &requirement.setup else {
                panic!("{extension_id} should use OAuth setup");
            };
            assert_eq!(scopes.iter().cloned().collect::<BTreeSet<_>>(), expected);
        }
    }

    #[tokio::test]
    async fn hosted_mcp_activation_publishes_discovered_tool_schemas() {
        let catalog =
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets");
        let (_dir, _storage_root, port, active_registry, _installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                catalog,
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");
        let egress = Arc::new(HostedMcpDiscoveryEgress::default());

        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install Notion MCP");
        port.activate_with_prechecked_credentials_for_test(
            package_ref,
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: ResourceScope::local_default(
                    UserId::new("hosted-mcp-user").unwrap(),
                    InvocationId::new(),
                )
                .unwrap(),
                runtime_http_egress: egress.clone(),
            },
            &lifecycle_owner(),
        )
        .await
        .expect("activate with discovery");

        let snapshot = active_registry.snapshot();
        assert!(
            snapshot
                .get_capability(&CapabilityId::new("notion.notion-fetch").unwrap())
                .is_none()
        );
        let search = snapshot
            .get_capability(&CapabilityId::new("notion.live-search").unwrap())
            .expect("discovered capability");
        assert_eq!(
            search.parameters_schema,
            serde_json::json!({
                "type": "object",
                "properties": {"query": {"type": "string"}},
                "required": ["query"]
            })
        );
        assert_eq!(
            egress.methods(),
            vec![
                "initialize".to_string(),
                "notifications/initialized".to_string(),
                "tools/list".to_string(),
            ]
        );
        assert_eq!(egress.credential_counts(), vec![1, 1, 1]);
    }

    #[tokio::test]
    async fn hosted_mcp_remove_unpublishes_discovered_active_package_after_absent_cleanup() {
        let catalog =
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets");
        let (_dir, _storage_root, port, active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                catalog,
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");
        let removal_scope = hosted_mcp_scope("lifecycle-owner");

        let absent_remove = port
            .remove(
                package_ref.clone(),
                &removal_scope,
                Some(&removal_scope.user_id),
            )
            .await
            .expect("already-absent remove is idempotent");
        assert!(matches!(
            absent_remove.payload.as_ref(),
            Some(LifecycleProductPayload::ExtensionRemove { removed: false })
        ));

        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install Notion MCP");
        port.activate_with_prechecked_credentials_for_test(
            package_ref.clone(),
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: hosted_mcp_scope("hosted-mcp-remove-discovered"),
                runtime_http_egress: Arc::new(HostedMcpDiscoveryEgress::default()),
            },
            &lifecycle_owner(),
        )
        .await
        .expect("activate with discovery");
        assert!(
            active_registry
                .snapshot()
                .get_capability(&CapabilityId::new("notion.live-search").unwrap())
                .is_some(),
            "discovered active package must publish before removal"
        );

        let removed = port
            .remove(package_ref, &removal_scope, Some(&removal_scope.user_id))
            .await
            .expect("remove unpublishes the discovered active package");
        assert_eq!(removed.phase, LifecyclePublicState::Uninstalled);
        let extension_id = ExtensionId::new("notion").expect("valid extension id");
        assert!(
            active_registry
                .snapshot()
                .get_extension(&extension_id)
                .is_none(),
            "active registry entry must be removed"
        );
        assert!(
            installation_store
                .get_manifest(&extension_id)
                .await
                .expect("manifest lookup")
                .is_none(),
            "successful finalization removes the cleanup tombstone"
        );
    }

    #[tokio::test]
    async fn first_party_extension_remove_succeeds_after_absent_cleanup_reinstall_and_activate() {
        let catalog =
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets");
        let (_dir, _storage_root, port, active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                catalog,
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "web-access")
            .expect("valid ref");
        let removal_scope = hosted_mcp_scope("lifecycle-owner");

        port.remove(
            package_ref.clone(),
            &removal_scope,
            Some(&removal_scope.user_id),
        )
        .await
        .expect("already-absent remove is idempotent");
        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install Web Access");
        port.activate_with_prechecked_credentials_for_test(
            package_ref.clone(),
            ExtensionActivationMode::Static,
            &lifecycle_owner(),
        )
        .await
        .expect("activate Web Access");

        port.remove(package_ref, &removal_scope, Some(&removal_scope.user_id))
            .await
            .expect("remove Web Access after reinstall");
        let extension_id = ExtensionId::new("web-access").expect("valid extension id");
        assert!(
            active_registry
                .snapshot()
                .get_extension(&extension_id)
                .is_none(),
            "active registry entry must be removed"
        );
        assert!(
            installation_store
                .get_manifest(&extension_id)
                .await
                .expect("manifest lookup")
                .is_none(),
            "successful finalization removes the cleanup tombstone"
        );
    }

    #[tokio::test]
    async fn hosted_mcp_activation_without_discovered_or_static_tools_stays_installed() {
        let (_dir, _storage_root, facade, active_registry, installation_store) =
            extension_lifecycle_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let facade = facade
            .with_runtime_credential_accounts(Arc::new(ConfiguredRuntimeCredentialAccounts))
            .with_runtime_http_egress(Arc::new(EmptyToolsHostedMcpEgress));
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");

        // safety: sequential caller actions in a hermetic lifecycle test, not
        // database statements that must share an atomic transaction.
        let error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect_err("zero discovered and static tools must not report install success");

        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
        let installation_id =
            ExtensionInstallationId::new("notion").expect("valid installation id");
        let installation = installation_store
            .get_installation(&installation_id)
            .await
            .expect("read installation")
            .expect("Notion installation remains retryable");
        assert!(installation.owner().visible_to(&lifecycle_owner()));
        let projection = facade
            .project_package(lifecycle_surface_context(), package_ref)
            .await
            .expect("failed initial discovery remains projectable");
        assert_eq!(
            projection.phase,
            LifecyclePublicState::SetupNeeded,
            "without a successful catalog the caller remains setup-needed"
        );
        let snapshot = active_registry.snapshot();
        assert!(
            snapshot
                .get_extension(&ExtensionId::new("notion").expect("valid extension id"))
                .is_none(),
            "failed discovery must publish neither the hidden connection template nor tools"
        );
    }

    #[tokio::test]
    async fn hosted_mcp_activation_with_malformed_catalog_stays_installed_without_a_surface() {
        let (_dir, _storage_root, facade, active_registry, installation_store) =
            extension_lifecycle_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let facade = facade
            .with_runtime_credential_accounts(Arc::new(ConfiguredRuntimeCredentialAccounts))
            .with_runtime_http_egress(Arc::new(MalformedToolsHostedMcpEgress));
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");

        let error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect_err("a malformed live catalog must not report install success");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        let installation = installation_store
            .get_installation(&ExtensionInstallationId::new("notion").expect("installation id"))
            .await
            .expect("read installation")
            .expect("malformed discovery leaves membership retryable");
        assert!(installation.owner().visible_to(&lifecycle_owner()));
        let projection = facade
            .project_package(lifecycle_surface_context(), package_ref)
            .await
            .expect("malformed initial discovery remains projectable");
        assert_eq!(projection.phase, LifecyclePublicState::SetupNeeded);
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("notion").expect("extension id"))
                .is_none(),
            "an invalid catalog must publish no extension surface"
        );
    }

    #[tokio::test]
    async fn hosted_mcp_discovery_failure_never_publishes_bundled_static_tools() {
        let catalog =
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets");
        let (_dir, _storage_root, port, active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                catalog,
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "nearai").expect("valid ref");

        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install hosted MCP extension");
        let error = port
            .activate_with_prechecked_credentials_for_test(
                package_ref,
                ExtensionActivationMode::HostedMcpDiscovery {
                    scope: hosted_mcp_scope("hosted-mcp-no-static-fallback"),
                    runtime_http_egress: Arc::new(EmptyToolsHostedMcpEgress),
                },
                &lifecycle_owner(),
            )
            .await
            .expect_err("live hosted-MCP discovery is authoritative");

        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
        let installation = installation_store
            .get_installation(&ExtensionInstallationId::new("nearai").expect("installation id"))
            .await
            .expect("read installation")
            .expect("membership remains installed for retry");
        assert!(installation.owner().visible_to(&lifecycle_owner()));
        assert!(
            active_registry
                .snapshot()
                .get_capability(&CapabilityId::new("nearai.web_search").unwrap())
                .is_none(),
            "bundled static schemas must not masquerade as a successful live catalog"
        );
    }

    #[tokio::test]
    async fn hosted_mcp_activation_rechecks_credentials_after_discovery_before_publish() {
        let catalog =
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets");
        let (_dir, _storage_root, port, active_registry, _installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                catalog,
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");
        let credential_gate = FailsSecondCredentialGate {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let calls = Arc::clone(&credential_gate.calls);

        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install Notion MCP");
        let error = port
            .activate_with_credential_gate(
                package_ref,
                ExtensionActivationMode::HostedMcpDiscovery {
                    scope: hosted_mcp_scope("hosted-mcp-credential-recheck"),
                    runtime_http_egress: Arc::new(HostedMcpDiscoveryEgress::default()),
                },
                credential_gate,
                &lifecycle_owner(),
            )
            .await
            .expect_err("post-discovery credential recheck should fail activation");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "hosted MCP activation must check credentials before and after discovery"
        );
        assert!(
            active_registry
                .snapshot()
                .get_capability(&CapabilityId::new("notion.live-search").unwrap())
                .is_none(),
            "discovered tools must not publish after post-discovery credential failure"
        );
    }

    #[tokio::test]
    async fn hosted_mcp_activation_routes_provider_auth_rejection_to_reauth() {
        // Credentials are present pre-discovery, but the provider rejects them
        // mid-`tools/list` (401). This must route the caller back through OAuth
        // (a credentials-incomplete response with credential blockers), NOT
        // fold into a retry-forever transient that leaves the extension stuck
        // re-hitting the same 401. Nothing may publish.
        let catalog =
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets");
        let (_dir, _storage_root, port, active_registry, _installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                catalog,
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");

        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install Notion MCP");
        let response = port
            .activate_with_credential_gate(
                package_ref,
                ExtensionActivationMode::HostedMcpDiscovery {
                    scope: hosted_mcp_scope("hosted-mcp-provider-auth-rejection"),
                    runtime_http_egress: Arc::new(AuthRejectedDiscoveryHostedMcpEgress),
                },
                crate::extension_host::extension_activation_credentials::PrecheckedExtensionActivationCredentialGate,
                &lifecycle_owner(),
            )
            .await
            .expect("a provider 401 during discovery must route to re-auth, not a transient retry");

        assert!(
            !response.blockers.is_empty()
                && response
                    .blockers
                    .iter()
                    .all(|blocker| matches!(blocker, LifecycleReadinessBlocker::Credential { .. })),
            "the user must be routed back to connect credentials (re-auth), got {:?}",
            response.blockers
        );
        assert!(
            active_registry
                .snapshot()
                .get_capability(&CapabilityId::new("notion.live-search").unwrap())
                .is_none(),
            "no catalog may publish after a provider credential rejection"
        );
    }

    #[tokio::test]
    async fn hosted_mcp_activation_discards_discovery_when_credential_epoch_changes() {
        let catalog =
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets");
        let (_dir, _storage_root, port, active_registry, _installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                catalog,
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");
        let scope = hosted_mcp_scope("hosted-mcp-credential-epoch");
        let credential_gate = RuntimeExtensionActivationCredentialGate::new(
            scope.clone(),
            Arc::new(ChangingRuntimeCredentialAccounts {
                calls: AtomicUsize::new(0),
            }),
        );

        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install Notion MCP");
        let error = port
            .activate_with_credential_gate(
                package_ref,
                ExtensionActivationMode::HostedMcpDiscovery {
                    scope,
                    runtime_http_egress: Arc::new(HostedMcpDiscoveryEgress::default()),
                },
                credential_gate,
                &lifecycle_owner(),
            )
            .await
            .expect_err("stale credential authority must discard discovery");

        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
        assert!(
            active_registry
                .snapshot()
                .get_capability(&CapabilityId::new("notion.live-search").unwrap())
                .is_none(),
            "a catalog discovered under a prior credential epoch must not publish"
        );
    }

    #[tokio::test]
    async fn hosted_mcp_activation_returns_transient_when_package_removed_during_discovery() {
        let catalog =
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets");
        let (_dir, _storage_root, port, _active_registry, _installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                catalog,
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");
        let (egress, tools_list_started, release_tools_list) =
            BlockingToolsListHostedMcpEgress::new();

        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install Notion MCP");
        let activation = tokio::spawn({
            let port = Arc::clone(&port);
            let package_ref = package_ref.clone();
            async move {
                port.activate_with_prechecked_credentials_for_test(
                    package_ref,
                    ExtensionActivationMode::HostedMcpDiscovery {
                        scope: hosted_mcp_scope("hosted-mcp-remove-race"),
                        runtime_http_egress: egress,
                    },
                    &lifecycle_owner(),
                )
                .await
            }
        });
        tools_list_started
            .await
            .expect("tools/list request should start");

        let removal_scope = hosted_mcp_scope("lifecycle-owner");
        port.remove(package_ref, &removal_scope, Some(&removal_scope.user_id))
            .await
            .expect("remove can proceed while discovery is in flight");
        release_tools_list
            .send(())
            .expect("release blocked tools/list response");
        let error = activation
            .await
            .expect("activation task joins")
            .expect_err("remove during discovery should be retryable");

        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
    }

    #[tokio::test]
    async fn hosted_mcp_activation_discards_discovery_when_manifest_inputs_change() {
        let catalog =
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets");
        let (_dir, _storage_root, port, active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                catalog,
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");
        let extension_id = ExtensionId::new("notion").expect("valid extension id");
        let (egress, tools_list_started, release_tools_list) =
            BlockingToolsListHostedMcpEgress::new();

        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install Notion MCP");
        let activation = tokio::spawn({
            let port = Arc::clone(&port);
            let package_ref = package_ref.clone();
            async move {
                port.activate_with_prechecked_credentials_for_test(
                    package_ref,
                    ExtensionActivationMode::HostedMcpDiscovery {
                        scope: hosted_mcp_scope("hosted-mcp-manifest-race"),
                        runtime_http_egress: egress,
                    },
                    &lifecycle_owner(),
                )
                .await
            }
        });
        tools_list_started
            .await
            .expect("tools/list request should start");

        let installed_manifest = installation_store
            .get_manifest(&extension_id)
            .await
            .expect("read installed manifest")
            .expect("installed manifest exists");
        let mut changed_resolved = installed_manifest.resolved().clone();
        changed_resolved
            .mcp
            .as_mut()
            .expect("hosted MCP declaration")
            .max_tools = 1;
        let changed_raw = format!(
            "{}\n# concurrent manifest generation\n",
            installed_manifest.raw_toml()
        );
        let changed_hash = ManifestHash::new(sha256_digest_token(changed_raw.as_bytes()))
            .expect("changed manifest hash");
        let changed_manifest = ExtensionManifestRecord::from_resolved(
            changed_raw,
            ManifestSource::HostBundled,
            changed_resolved,
            Some(changed_hash.clone()),
        )
        .expect("changed manifest record")
        .with_removal_cleanup_requirements(
            installed_manifest.removal_cleanup_requirements().to_vec(),
        );
        let installed = installation_store
            .get_installation(
                &ExtensionInstallationId::new("notion").expect("valid installation id"),
            )
            .await
            .expect("read installation")
            .expect("installation exists");
        let changed_installation =
            ExtensionInstallation::from_persisted_parts(ExtensionInstallationPersistedParts {
                installation_id: installed.installation_id().clone(),
                extension_id: installed.extension_id().clone(),
                manifest_ref: ExtensionManifestRef::new(extension_id.clone(), Some(changed_hash)),
                credential_bindings: installed.credential_bindings().to_vec(),
                health: installed.health().clone(),
                updated_at: installed.updated_at(),
                owner: installed.owner().clone(),
            })
            .expect("changed installation manifest reference");
        installation_store
            .upsert_manifest_and_installation(changed_manifest, changed_installation)
            .await
            .expect("replace manifest generation while discovery is in flight");

        release_tools_list
            .send(())
            .expect("release blocked tools/list response");
        let error = activation
            .await
            .expect("activation task joins")
            .expect_err("stale manifest discovery should be retryable");

        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
        assert!(
            active_registry
                .snapshot()
                .get_capability(&CapabilityId::new("notion.live-search").unwrap())
                .is_none(),
            "a catalog discovered from stale manifest inputs must not publish"
        );
    }

    #[tokio::test]
    async fn hosted_mcp_rediscovery_replaces_the_published_tool_set_completely() {
        // TOOL-9 ("a refresh replaces the set completely"): discovery is
        // loader-owned and has no separate refresh API — a refresh is a
        // re-activation that re-runs tools/list and atomically republishes.
        // The second discovery returns a *different* tool, so the published set
        // is replaced wholesale: the first discovered capability is gone (not
        // merged), only the second remains.
        let catalog =
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets");
        let (_dir, _storage_root, port, active_registry, _installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                catalog,
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");

        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install Notion MCP");
        port.activate_with_prechecked_credentials_for_test(
            package_ref.clone(),
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: hosted_mcp_scope("hosted-mcp-refresh"),
                runtime_http_egress: Arc::new(HostedMcpDiscoveryEgress::with_tool_name(
                    "search-one",
                )),
            },
            &lifecycle_owner(),
        )
        .await
        .expect("initial discovery activation");
        assert!(
            active_registry
                .snapshot()
                .get_capability(&CapabilityId::new("notion.search-one").unwrap())
                .is_some(),
            "the first discovered tool publishes"
        );

        // Refresh: re-activate; tools/list now yields a different tool.
        port.activate_with_prechecked_credentials_for_test(
            package_ref,
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: hosted_mcp_scope("hosted-mcp-refresh"),
                runtime_http_egress: Arc::new(HostedMcpDiscoveryEgress::with_tool_name(
                    "search-two",
                )),
            },
            &lifecycle_owner(),
        )
        .await
        .expect("re-discovery activation");

        let snapshot = active_registry.snapshot();
        assert!(
            snapshot
                .get_capability(&CapabilityId::new("notion.search-two").unwrap())
                .is_some(),
            "the refreshed set contains the newly discovered tool"
        );
        assert!(
            snapshot
                .get_capability(&CapabilityId::new("notion.search-one").unwrap())
                .is_none(),
            "the refresh replaced the set completely — the prior discovered tool is gone, not merged"
        );
    }

    #[tokio::test]
    async fn hosted_mcp_rediscovery_failure_leaves_the_prior_tool_set_intact() {
        // TOOL-9 ("or not at all"): when a refresh fails after discovery but
        // before the atomic publish (here the post-discovery credential recheck
        // fails), the swap never happens — the previously published discovered
        // set stays live and the new set is not partially applied.
        let catalog =
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets");
        let (_dir, _storage_root, port, active_registry, _installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                catalog,
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");

        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install Notion MCP");
        port.activate_with_prechecked_credentials_for_test(
            package_ref.clone(),
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: hosted_mcp_scope("hosted-mcp-refresh-fail"),
                runtime_http_egress: Arc::new(HostedMcpDiscoveryEgress::with_tool_name(
                    "search-one",
                )),
            },
            &lifecycle_owner(),
        )
        .await
        .expect("initial discovery activation");

        // Refresh attempt: tools/list would yield a new tool, but the
        // post-discovery credential recheck fails before publish.
        let error = port
            .activate_with_credential_gate(
                package_ref,
                ExtensionActivationMode::HostedMcpDiscovery {
                    scope: hosted_mcp_scope("hosted-mcp-refresh-fail"),
                    runtime_http_egress: Arc::new(HostedMcpDiscoveryEgress::with_tool_name(
                        "search-two",
                    )),
                },
                FailsSecondCredentialGate {
                    calls: Arc::new(AtomicUsize::new(0)),
                },
                &lifecycle_owner(),
            )
            .await
            .expect_err("post-discovery credential failure aborts the refresh");
        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));

        let snapshot = active_registry.snapshot();
        assert!(
            snapshot
                .get_capability(&CapabilityId::new("notion.search-one").unwrap())
                .is_some(),
            "the prior discovered set survives a failed refresh"
        );
        assert!(
            snapshot
                .get_capability(&CapabilityId::new("notion.search-two").unwrap())
                .is_none(),
            "a failed refresh publishes nothing — no partial swap to the new set"
        );
    }

    #[tokio::test]
    async fn lifecycle_refresh_failure_reports_error_but_keeps_active_projection() {
        let (_dir, _storage_root, facade, active_registry, _installation_store) =
            extension_lifecycle_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let facade = facade
            .with_runtime_credential_accounts(Arc::new(ConfiguredRuntimeCredentialAccounts))
            .with_runtime_http_egress(Arc::new(FailsSecondToolsListHostedMcpEgress::default()));
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");

        let first = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("initial discovery publishes a catalog");
        assert_eq!(first.phase, LifecyclePublicState::Active);

        let refresh_error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect_err("refresh failure remains a separate retryable error");
        assert!(matches!(
            refresh_error,
            ProductWorkflowError::Transient { .. }
        ));

        let projection = facade
            .project_package(lifecycle_surface_context(), package_ref)
            .await
            .expect("prior active projection remains readable");
        assert_eq!(
            projection.phase,
            LifecyclePublicState::Active,
            "a failed refresh must not demote a previously published catalog"
        );
        assert!(
            active_registry
                .snapshot()
                .get_capability(&CapabilityId::new("notion.live-search").unwrap())
                .is_some(),
            "the prior callable tool set survives the failed refresh"
        );
    }

    #[tokio::test]
    async fn lifecycle_empty_or_malformed_refresh_keeps_the_prior_active_projection() {
        for (label, second_result, expected_permanent) in [
            ("empty", serde_json::json!({ "tools": [] }), false),
            (
                "malformed",
                serde_json::json!({
                    "tools": [{
                        "name": "unsupported tool name",
                        "description": "invalid capability suffix",
                        "inputSchema": {"type": "object"}
                    }]
                }),
                true,
            ),
        ] {
            let (_dir, _storage_root, facade, active_registry, _installation_store) =
                extension_lifecycle_fixture_with_catalog_and_service(
                    AvailableExtensionCatalog::from_first_party_assets()
                        .expect("first-party assets"),
                    ExtensionLifecycleService::new(ExtensionRegistry::new()),
                );
            let facade = facade
                .with_runtime_credential_accounts(Arc::new(ConfiguredRuntimeCredentialAccounts))
                .with_runtime_http_egress(Arc::new(SecondToolsListResultHostedMcpEgress::new(
                    second_result,
                )));
            let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion")
                .expect("valid ref");

            let first = facade
                .execute(
                    lifecycle_surface_context(),
                    LifecycleProductAction::ExtensionInstall {
                        package_ref: package_ref.clone(),
                    },
                )
                .await
                .expect("initial discovery publishes a catalog");
            assert_eq!(first.phase, LifecyclePublicState::Active);

            let error = facade
                .execute(
                    lifecycle_surface_context(),
                    LifecycleProductAction::ExtensionInstall {
                        package_ref: package_ref.clone(),
                    },
                )
                .await
                .expect_err("invalid refresh must not replace the active catalog");
            assert!(
                if expected_permanent {
                    matches!(&error, ProductWorkflowError::InvalidBindingRequest { .. })
                } else {
                    matches!(&error, ProductWorkflowError::Transient { .. })
                },
                "unexpected {label} refresh error: {error}"
            );

            let projection = facade
                .project_package(lifecycle_surface_context(), package_ref)
                .await
                .expect("the prior active projection remains readable");
            assert_eq!(
                projection.phase,
                LifecyclePublicState::Active,
                "the {label} refresh must not demote the prior catalog"
            );
            assert!(
                active_registry
                    .snapshot()
                    .get_capability(&CapabilityId::new("notion.live-search").unwrap())
                    .is_some(),
                "the prior callable tool set survives the {label} refresh"
            );
        }
    }

    #[tokio::test]
    async fn extension_activation_updates_local_dev_host_trust_policy() {
        let (_dir, _storage_root, port, _active_registry, _installation_store, trust_policy) =
            extension_management_port_fixture_with_catalog_service_and_trust(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package = fixture_extension_package().package;
        let trust_input = extension_trust_policy_input(&package).expect("trust input");
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");

        assert_eq!(
            trust_policy
                .evaluate(&trust_input)
                .expect("pre-activation trust")
                .effective_trust
                .class(),
            TrustClass::Sandbox
        );

        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install fixture extension");
        port.activate_with_prechecked_credentials_for_test(
            package_ref.clone(),
            ExtensionActivationMode::Static,
            &lifecycle_owner(),
        )
        .await
        .expect("activate fixture extension");
        let active_decision = trust_policy
            .evaluate(&trust_input)
            .expect("active extension trust");
        assert_eq!(
            active_decision.effective_trust.class(),
            TrustClass::UserTrusted
        );
        assert_eq!(
            active_decision.provenance,
            ironclaw_trust::TrustProvenance::AdminConfig
        );
        assert_eq!(
            active_decision.authority_ceiling.allowed_effects,
            vec![EffectKind::Network, EffectKind::ExternalWrite]
        );

        port.remove(package_ref, &hosted_mcp_scope("lifecycle-owner"), None)
            .await
            .expect("remove fixture extension");
        let removed_decision = trust_policy
            .evaluate(&trust_input)
            .expect("removed extension trust");
        assert_eq!(
            removed_decision.effective_trust.class(),
            TrustClass::Sandbox
        );
        assert!(
            removed_decision
                .authority_ceiling
                .allowed_effects
                .is_empty()
        );
    }

    #[tokio::test]
    async fn activation_transaction_rolls_back_when_publish_fails() {
        let (_dir, _storage_root, port, active_registry, _installation_store) =
            extension_management_port_fixture_with_catalog_service_and_trust_policy(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
                Arc::new(HostTrustPolicy::fail_closed()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");

        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install extension");
        let error = port
            .activate(
                package_ref,
                ExtensionActivationMode::Static,
                &lifecycle_owner(),
            )
            .await
            .expect_err("publish failure is reported");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("fixture").unwrap())
                .is_none()
        );
    }

    #[tokio::test]
    async fn repeated_activation_of_same_published_package_is_idempotent() {
        let (_dir, _storage_root, port, active_registry, _installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");
        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install extension");

        for attempt in 0..2 {
            let response = port
                .activate_with_prechecked_credentials_for_test(
                    package_ref.clone(),
                    ExtensionActivationMode::Static,
                    &lifecycle_owner(),
                )
                .await
                .unwrap_or_else(|error| panic!("activation attempt {attempt} failed: {error}"));
            assert_eq!(response.phase, LifecyclePublicState::Active);
        }
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("fixture").expect("valid extension id"))
                .is_some()
        );
    }

    #[tokio::test]
    async fn extension_lifecycle_search_propagates_installation_store_read_error() {
        let (_dir, port, _active_registry, _failing_store, _trust_policy) =
            extension_port_with_failing_store(
                ExtensionRegistry::new(),
                DeleteInstallationFailingStore::fail_get_installation(),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );

        let error = port
            .search("fixture", None, &lifecycle_owner())
            .await
            .expect_err("search reports installation-store read failure");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
    }

    #[tokio::test]
    async fn extension_lifecycle_search_rejects_mismatched_installation_row() {
        let (_dir, port, _active_registry, _failing_store, _trust_policy) =
            extension_port_with_failing_store(
                ExtensionRegistry::new(),
                DeleteInstallationFailingStore::mismatched_get_installation(),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );

        let error = port
            .search("fixture", None, &lifecycle_owner())
            .await
            .expect_err("search reports mismatched installation row");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
    }

    #[tokio::test]
    async fn active_extension_trust_policy_is_digest_pinned() {
        let (_dir, _storage_root, port, _active_registry, _installation_store, trust_policy) =
            extension_management_port_fixture_with_catalog_service_and_trust(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");

        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install fixture extension");
        port.activate_with_prechecked_credentials_for_test(
            package_ref,
            ExtensionActivationMode::Static,
            &lifecycle_owner(),
        )
        .await
        .expect("activate fixture extension");

        let changed_package = fixture_extension_package_with_description(
            "Lifecycle fixture extension with changed manifest",
        )
        .package;
        let changed_trust_input =
            extension_trust_policy_input(&changed_package).expect("changed trust input");
        let changed_decision = trust_policy
            .evaluate(&changed_trust_input)
            .expect("changed active extension trust");
        assert_eq!(
            changed_decision.effective_trust.class(),
            TrustClass::Sandbox
        );
        assert_eq!(
            changed_decision.provenance,
            ironclaw_trust::TrustProvenance::Default
        );
    }

    #[tokio::test]
    async fn restore_enabled_extension_updates_local_dev_host_trust_policy() {
        let (_dir, _storage_root, port, _active_registry, installation_store, _trust_policy) =
            extension_management_port_fixture_with_catalog_service_and_trust(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");
        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install fixture extension");
        port.activate_with_prechecked_credentials_for_test(
            package_ref,
            ExtensionActivationMode::Static,
            &lifecycle_owner(),
        )
        .await
        .expect("activate fixture extension");

        let restored_catalog =
            AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]);
        let restored_lifecycle = Arc::new(Mutex::new(ExtensionLifecycleService::new(
            ExtensionRegistry::new(),
        )));
        let restored_active_registry =
            Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let restored_trust_policy = test_extension_trust_policy();
        let restored_active_extensions = test_active_extension_publisher(
            Arc::clone(&restored_active_registry),
            Arc::clone(&restored_trust_policy),
        );
        let installation_store: Arc<dyn ExtensionInstallationStorePort> = installation_store;

        restore_extension_lifecycle_state(
            &restored_catalog,
            &port.filesystem,
            &installation_store,
            &restored_lifecycle,
            &restored_active_extensions,
            &lifecycle_owner(),
        )
        .await
        .expect("restore enabled extension lifecycle state");

        let package = fixture_extension_package().package;
        let trust_input = extension_trust_policy_input(&package).expect("trust input");
        assert_eq!(
            restored_trust_policy
                .evaluate(&trust_input)
                .expect("restored active extension trust")
                .effective_trust
                .class(),
            TrustClass::UserTrusted
        );
        assert!(
            restored_active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("fixture").unwrap())
                .is_some()
        );
    }

    fn retired_slack_user_manifest() -> &'static str {
        r#"
schema_version = "reborn.extension_manifest.v2"
id = "slack_user"
name = "Retired Slack User Extension"
version = "0.1.0"
description = "Retired internal Slack user tools companion"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/slack_user_tool.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "slack_user.search"
description = "Search Slack messages"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"
"#
    }

    #[tokio::test]
    async fn restore_removes_retired_slack_user_installation_without_catalog_entry() {
        let installation_store = Arc::new(filesystem_installation_store());
        let extension_id =
            ExtensionId::new(RETIRED_SLACK_USER_EXTENSION_ID).expect("valid extension id");
        let installation_id =
            ExtensionInstallationId::new(RETIRED_SLACK_USER_EXTENSION_ID).expect("valid install");
        let manifest_hash = "sha256:retired-slack-user".to_string();
        installation_store
            .upsert_manifest(fixture_manifest_record_with_source(
                retired_slack_user_manifest(),
                ManifestSource::HostBundled,
                Some(manifest_hash.clone()),
            ))
            .await
            .expect("upsert retired slack_user manifest");
        installation_store
            .upsert_installation(
                ExtensionInstallation::new(
                    installation_id.clone(),
                    extension_id.clone(),
                    ExtensionManifestRef::new(
                        extension_id.clone(),
                        Some(ManifestHash::new(manifest_hash).expect("valid hash")),
                    ),
                    Vec::new(),
                    chrono::Utc::now(),
                    InstallationOwner::Tenant,
                )
                .expect("retired slack_user installation"),
            )
            .await
            .expect("upsert retired slack_user installation");
        let restored_lifecycle = Arc::new(Mutex::new(ExtensionLifecycleService::new(
            ExtensionRegistry::new(),
        )));
        let restored_active_registry =
            Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let restored_trust_policy = test_extension_trust_policy();
        let restored_active_extensions = test_active_extension_publisher(
            Arc::clone(&restored_active_registry),
            Arc::clone(&restored_trust_policy),
        );
        let installation_store_trait: Arc<dyn ExtensionInstallationStorePort> =
            installation_store.clone();
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(DiskFilesystem::new());

        restore_extension_lifecycle_state(
            &AvailableExtensionCatalog::from_packages(Vec::new()),
            &filesystem,
            &installation_store_trait,
            &restored_lifecycle,
            &restored_active_extensions,
            &UserId::new("restore-test-operator").expect("valid operator"),
        )
        .await
        .expect("retired slack_user install is cleaned up during restore");

        assert!(
            installation_store
                .get_installation(&installation_id)
                .await
                .expect("read retired installation")
                .is_none()
        );
        assert!(
            installation_store
                .get_manifest(&extension_id)
                .await
                .expect("read retired manifest")
                .is_none()
        );
        assert!(
            restored_active_registry
                .snapshot()
                .get_extension(&extension_id)
                .is_none()
        );
    }

    #[tokio::test]
    async fn restore_skips_installation_absent_from_catalog_and_restores_valid_installation() {
        // Regression for PR #5499 review finding: a persisted installation
        // row whose extension id the catalog does not (yet) materialize a
        // package for — e.g. a placeholder row written by the standalone
        // v1->Reborn migration tool — must not abort restore for every other
        // installation.
        let (_dir, _storage_root, port, _active_registry, installation_store, _trust_policy) =
            extension_management_port_fixture_with_catalog_service_and_trust(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");
        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install fixture extension");
        port.activate_with_prechecked_credentials_for_test(
            package_ref,
            ExtensionActivationMode::Static,
            &lifecycle_owner(),
        )
        .await
        .expect("activate fixture extension");

        let orphan_extension_id = ExtensionId::new("orphan_migrated").expect("valid extension id");
        let orphan_installation_id =
            ExtensionInstallationId::new("orphan_migrated").expect("valid installation");
        installation_store
            .upsert_manifest(fixture_manifest_record_with_source(
                &orphan_migrated_manifest(),
                ManifestSource::InstalledLocal,
                None,
            ))
            .await
            .expect("upsert orphan manifest absent from the catalog");
        installation_store
            .upsert_installation(
                ExtensionInstallation::new(
                    orphan_installation_id.clone(),
                    orphan_extension_id.clone(),
                    ExtensionManifestRef::new(orphan_extension_id.clone(), None),
                    Vec::new(),
                    chrono::Utc::now(),
                    InstallationOwner::Tenant,
                )
                .expect("orphan installation"),
            )
            .await
            .expect("upsert orphan installation absent from the catalog");

        let restored_catalog =
            AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]);
        let restored_lifecycle = Arc::new(Mutex::new(ExtensionLifecycleService::new(
            ExtensionRegistry::new(),
        )));
        let restored_active_registry =
            Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let restored_trust_policy = test_extension_trust_policy();
        let restored_active_extensions = test_active_extension_publisher(
            Arc::clone(&restored_active_registry),
            Arc::clone(&restored_trust_policy),
        );
        let installation_store: Arc<dyn ExtensionInstallationStorePort> = installation_store;

        restore_extension_lifecycle_state(
            &restored_catalog,
            &port.filesystem,
            &installation_store,
            &restored_lifecycle,
            &restored_active_extensions,
            &lifecycle_owner(),
        )
        .await
        .expect("restore succeeds by skipping the orphan installation");

        // The valid installation still restores normally.
        assert!(
            restored_active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("fixture").expect("valid extension id"))
                .is_some()
        );
        // The orphan row is preserved (never deleted or rewritten) for when
        // the migration tool later materializes its catalog package.
        assert!(
            installation_store
                .get_installation(&orphan_installation_id)
                .await
                .expect("read orphan installation")
                .is_some()
        );
        assert!(
            restored_active_registry
                .snapshot()
                .get_extension(&orphan_extension_id)
                .is_none()
        );
    }

    #[tokio::test]
    async fn restore_refreshes_materialized_extension_assets_from_catalog() {
        let (_dir, storage_root, port, _active_registry, installation_store, _trust_policy) =
            extension_management_port_fixture_with_catalog_service_and_trust(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");
        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install fixture extension");
        port.activate_with_prechecked_credentials_for_test(
            package_ref,
            ExtensionActivationMode::Static,
            &lifecycle_owner(),
        )
        .await
        .expect("activate fixture extension");

        let wasm_path = storage_root.join("system/extensions/fixture/wasm/fixture.wasm");
        std::fs::write(&wasm_path, b"stale-installed-module").expect("corrupt installed module");

        let restored_lifecycle = Arc::new(Mutex::new(ExtensionLifecycleService::new(
            ExtensionRegistry::new(),
        )));
        let restored_active_registry =
            Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let restored_trust_policy = test_extension_trust_policy();
        let restored_active_extensions = test_active_extension_publisher(
            Arc::clone(&restored_active_registry),
            Arc::clone(&restored_trust_policy),
        );
        let installation_store: Arc<dyn ExtensionInstallationStorePort> = installation_store;

        restore_extension_lifecycle_state(
            &AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
            &port.filesystem,
            &installation_store,
            &restored_lifecycle,
            &restored_active_extensions,
            &lifecycle_owner(),
        )
        .await
        .expect("restore extension lifecycle state");

        assert_eq!(
            std::fs::read(wasm_path).expect("refreshed module"),
            b"\0asm\x01\0\0\0"
        );
    }

    #[tokio::test]
    async fn restore_enabled_host_bundled_extension_migrates_manifest_hash_and_trust_policy() {
        let (_dir, _storage_root, port, _active_registry, installation_store, _trust_policy) =
            extension_management_port_fixture_with_catalog_service_and_trust(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");
        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install fixture extension");
        port.activate_with_prechecked_credentials_for_test(
            package_ref,
            ExtensionActivationMode::Static,
            &lifecycle_owner(),
        )
        .await
        .expect("activate fixture extension");

        let changed_available = fixture_extension_package_with_description(
            "Lifecycle fixture extension with changed manifest",
        );
        let changed_hash = available_manifest_hash(&changed_available).expect("changed hash");
        let changed_package = changed_available.package.clone();
        let changed_catalog = AvailableExtensionCatalog::from_packages(vec![changed_available]);
        let restored_lifecycle = Arc::new(Mutex::new(ExtensionLifecycleService::new(
            ExtensionRegistry::new(),
        )));
        let restored_active_registry =
            Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let restored_trust_policy = test_extension_trust_policy();
        let restored_active_extensions = test_active_extension_publisher(
            Arc::clone(&restored_active_registry),
            Arc::clone(&restored_trust_policy),
        );
        let installation_store: Arc<dyn ExtensionInstallationStorePort> = installation_store;

        restore_extension_lifecycle_state(
            &changed_catalog,
            &port.filesystem,
            &installation_store,
            &restored_lifecycle,
            &restored_active_extensions,
            &lifecycle_owner(),
        )
        .await
        .expect("host-bundled manifest hash mismatch migrates");

        let extension_id = ExtensionId::new("fixture").expect("valid extension id");
        let installation_id = ExtensionInstallationId::new("fixture").expect("valid installation");
        let stored_manifest = installation_store
            .get_manifest(&extension_id)
            .await
            .expect("read migrated manifest")
            .expect("migrated manifest");
        assert_eq!(stored_manifest.manifest_hash(), Some(&changed_hash));
        let stored_installation = installation_store
            .get_installation(&installation_id)
            .await
            .expect("read migrated installation")
            .expect("migrated installation");
        assert_eq!(
            stored_installation.manifest_ref().manifest_hash(),
            Some(&changed_hash)
        );
        let trust_input = extension_trust_policy_input(&changed_package).expect("trust input");
        assert_eq!(
            restored_trust_policy
                .evaluate(&trust_input)
                .expect("migrated extension trust")
                .effective_trust
                .class(),
            TrustClass::UserTrusted
        );
        assert!(
            restored_active_registry
                .snapshot()
                .get_extension(&extension_id)
                .is_some()
        );
    }

    #[tokio::test]
    async fn restore_enabled_local_extension_rejects_manifest_hash_mismatch() {
        let changed_available = fixture_extension_package_with_description(
            "Lifecycle fixture extension with changed manifest",
        );
        let package = changed_available.package.clone();
        let catalog = AvailableExtensionCatalog::from_packages(vec![changed_available]);
        let installation_store = Arc::new(filesystem_installation_store());
        let manifest_record = fixture_manifest_record_with_source(
            fixture_installed_local_manifest(),
            ManifestSource::InstalledLocal,
            Some("sha256:old".to_string()),
        );
        installation_store
            .upsert_manifest(manifest_record)
            .await
            .expect("upsert manifest");
        installation_store
            .upsert_installation(fixture_installation(Some("sha256:old".to_string())))
            .await
            .expect("upsert installation");
        let restored_lifecycle = Arc::new(Mutex::new(ExtensionLifecycleService::new(
            ExtensionRegistry::new(),
        )));
        let restored_active_registry =
            Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let restored_trust_policy = test_extension_trust_policy();
        let restored_active_extensions = test_active_extension_publisher(
            Arc::clone(&restored_active_registry),
            Arc::clone(&restored_trust_policy),
        );
        let installation_store: Arc<dyn ExtensionInstallationStorePort> = installation_store;
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(DiskFilesystem::new());

        let error = restore_extension_lifecycle_state(
            &catalog,
            &filesystem,
            &installation_store,
            &restored_lifecycle,
            &restored_active_extensions,
            &UserId::new("restore-test-operator").expect("valid operator"),
        )
        .await
        .expect_err("non-host-bundled manifest hash mismatch fails closed");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        let trust_input = extension_trust_policy_input(&package).expect("trust input");
        assert_eq!(
            restored_trust_policy
                .evaluate(&trust_input)
                .expect("missing-hash extension trust")
                .effective_trust
                .class(),
            TrustClass::Sandbox
        );
        assert!(
            restored_active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("fixture").unwrap())
                .is_none()
        );
    }

    #[tokio::test]
    async fn extension_lifecycle_installs_and_removes_github() {
        let (_dir, storage_root, facade, active_registry, _installation_store) =
            github_extension_lifecycle_fixture();
        let facade =
            facade.with_runtime_credential_accounts(Arc::new(ConfiguredRuntimeCredentialAccounts));

        let search = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionSearch {
                    query: "github".to_string(),
                },
            )
            .await
            .expect("search extensions");
        assert_eq!(search.phase, LifecyclePublicState::SetupNeeded);
        let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) =
            search.payload.as_ref()
        else {
            panic!("expected extension search payload");
        };
        assert_eq!(extensions.len(), 1);
        assert!(
            extensions[0]
                .summary
                .visible_read_only_capability_ids
                .iter()
                .any(|id| id == "github.search_issues")
        );
        assert!(
            extensions[0]
                .summary
                .visible_read_only_capability_ids
                .iter()
                .any(|id| id == "github.search_issues_pull_requests")
        );
        assert!(
            extensions[0]
                .summary
                .visible_read_only_capability_ids
                .iter()
                .any(|id| id == "github.get_issue")
        );

        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").expect("valid ref");
        let install = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install extension");
        assert_eq!(install.phase, LifecyclePublicState::Active);
        assert!(
            storage_root
                .join("system/extensions/github/manifest.toml")
                .exists()
        );
        assert!(
            storage_root
                .join("system/extensions/github/wasm/github_tool.wasm")
                .exists()
        );
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("github").unwrap())
                .is_some()
        );

        let installed_search = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionSearch {
                    query: "github".to_string(),
                },
            )
            .await
            .expect("search installed extension");
        let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) =
            installed_search.payload.as_ref()
        else {
            panic!("expected extension search payload");
        };
        let github = extensions
            .iter()
            .find(|extension| extension.summary.package_ref.id.as_str() == "github")
            .expect("github search result");
        assert_eq!(
            github.installation_phase,
            Some(LifecyclePublicState::Active)
        );
        assert!(
            github.summary.credential_requirements.is_empty(),
            "active GitHub search results must not expose satisfied PAT requirements"
        );
        assert!(
            github.summary.onboarding.is_none(),
            "active GitHub search results must not expose stale PAT setup onboarding"
        );
        let active = active_registry.snapshot();
        assert!(
            active
                .get_extension(&ExtensionId::new("github").unwrap())
                .is_some()
        );
        assert!(
            active
                .get_capability(
                    &ironclaw_host_api::CapabilityId::new("github.search_issues").unwrap()
                )
                .is_some()
        );
        assert!(
            active
                .get_capability(
                    &ironclaw_host_api::CapabilityId::new("github.comment_issue").unwrap()
                )
                .is_some()
        );

        let active_search = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionSearch {
                    query: "github".to_string(),
                },
            )
            .await
            .expect("search active extension");
        let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) =
            active_search.payload.as_ref()
        else {
            panic!("expected extension search payload");
        };
        let github = extensions
            .iter()
            .find(|extension| extension.summary.package_ref.id.as_str() == "github")
            .expect("github search result");
        assert_eq!(
            github.installation_phase,
            Some(LifecyclePublicState::Active)
        );
        assert!(
            github.summary.credential_requirements.is_empty(),
            "active GitHub search results must not expose satisfied PAT requirements"
        );
        assert!(
            github.summary.onboarding.is_none(),
            "active GitHub search results must not expose stale PAT setup onboarding"
        );

        let remove = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove { package_ref },
            )
            .await
            .expect("remove extension");
        assert_eq!(remove.phase, LifecyclePublicState::Uninstalled);
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("github").unwrap())
                .is_none()
        );
        assert!(
            !storage_root
                .join("system/extensions/github/manifest.toml")
                .exists()
        );
        assert!(
            !storage_root
                .join("system/extensions/github/wasm/github_tool.wasm")
                .exists()
        );
    }

    #[tokio::test]
    async fn extension_install_reports_credential_backend_failure_as_transient() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            github_extension_lifecycle_fixture();
        let facade = facade.with_runtime_credential_accounts(Arc::new(
            BackendUnavailableRuntimeCredentialAccounts,
        ));
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").expect("valid ref");

        let error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall { package_ref },
            )
            .await
            .expect_err("install readiness reports credential backend failure");

        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
    }

    #[tokio::test]
    async fn lifecycle_facade_reports_typed_credential_blockers_from_install() {
        let (_dir, _storage_root, facade, active_registry, _installation_store) =
            github_extension_lifecycle_fixture();
        let facade =
            facade.with_runtime_credential_accounts(Arc::new(MissingRuntimeCredentialAccounts));
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").expect("valid ref");

        let response = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("missing product-auth account is a typed readiness outcome");

        assert_eq!(response.phase, LifecyclePublicState::SetupNeeded);
        assert!(matches!(
            response.payload,
            Some(LifecycleProductPayload::ExtensionInstall {
                installed: true,
                ..
            })
        ));
        assert!(!response.blockers.is_empty());
        assert!(
            response
                .blockers
                .iter()
                .all(|blocker| matches!(blocker, LifecycleReadinessBlocker::Credential { .. }))
        );
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("github").unwrap())
                .is_none()
        );
    }

    #[tokio::test]
    async fn lifecycle_facade_rejects_hosted_mcp_install_without_runtime_egress() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let facade =
            facade.with_runtime_credential_accounts(Arc::new(ConfiguredRuntimeCredentialAccounts));
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");

        let error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall { package_ref },
            )
            .await
            .expect_err("hosted MCP install needs runtime egress services");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
    }

    #[tokio::test]
    async fn setup_needed_search_projects_catalog_capability_metadata() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            github_extension_lifecycle_fixture();

        let search = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionSearch {
                    query: "google calendar".to_string(),
                },
            )
            .await
            .expect("search setup-needed extension");

        assert_eq!(search.phase, LifecyclePublicState::SetupNeeded);
        let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) = search.payload
        else {
            panic!("expected extension search payload");
        };
        let calendar = extensions
            .into_iter()
            .find(|extension| extension.summary.package_ref.id.as_str() == "google-calendar")
            .expect("google-calendar catalog result");
        assert!(
            calendar
                .summary
                .visible_capability_ids
                .iter()
                .any(|capability_id| capability_id == "google-calendar.create_event"),
            "setup-needed projection must retain catalog-visible write capabilities"
        );
        assert!(
            calendar
                .summary
                .visible_read_only_capability_ids
                .iter()
                .any(|capability_id| capability_id == "google-calendar.list_events"),
            "setup-needed projection must retain catalog read-only metadata"
        );
        assert!(
            !calendar
                .summary
                .visible_read_only_capability_ids
                .iter()
                .any(|capability_id| capability_id == "google-calendar.create_event"),
            "catalog read-only projection must not include write capabilities"
        );
    }

    #[tokio::test]
    async fn lifecycle_facade_installs_hosted_mcp_with_runtime_egress() {
        let (_dir, _storage_root, facade, active_registry, _installation_store) =
            extension_lifecycle_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let facade = facade
            .with_runtime_credential_accounts(Arc::new(ConfiguredRuntimeCredentialAccounts))
            .with_runtime_http_egress(Arc::new(HostedMcpDiscoveryEgress::default()));
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");

        let install = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall { package_ref },
            )
            .await
            .expect("hosted MCP install should use discovery egress");

        assert_eq!(install.phase, LifecyclePublicState::Active);
        assert!(
            active_registry
                .snapshot()
                .get_capability(&CapabilityId::new("notion.live-search").unwrap())
                .is_some()
        );

        let listed = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionList,
            )
            .await
            .expect("list active hosted MCP extension");
        let Some(LifecycleProductPayload::ExtensionList { extensions, .. }) = listed.payload else {
            panic!("expected extension list payload");
        };
        let notion = extensions
            .into_iter()
            .find(|extension| extension.summary.package_ref.id.as_str() == "notion")
            .expect("installed Notion extension");
        assert_eq!(notion.phase, LifecyclePublicState::Active);
        assert_eq!(
            notion.summary.visible_capability_ids,
            vec!["notion.live-search"],
            "active hosted MCP projections must expose the discovered runtime contract"
        );
    }

    #[tokio::test]
    async fn extension_lifecycle_installs_and_removes_gsuite() {
        let (_dir, storage_root, facade, active_registry, _installation_store) =
            github_extension_lifecycle_fixture();
        let facade =
            facade.with_runtime_credential_accounts(Arc::new(ConfiguredRuntimeCredentialAccounts));

        let search = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionSearch {
                    query: "google".to_string(),
                },
            )
            .await
            .expect("search extensions");
        assert_eq!(search.phase, LifecyclePublicState::SetupNeeded);
        let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) =
            search.payload.as_ref()
        else {
            panic!("expected extension search payload");
        };
        let extension_ids = extensions
            .iter()
            .map(|extension| extension.summary.package_ref.id.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            extension_ids,
            BTreeSet::from([
                "gmail",
                "google-calendar",
                "google-docs",
                "google-drive",
                "google-sheets",
                "google-slides",
            ])
        );
        let calendar = extensions
            .iter()
            .find(|extension| extension.summary.package_ref.id.as_str() == "google-calendar")
            .expect("google-calendar search result");
        assert_eq!(
            calendar.summary.visible_capability_ids,
            vec![
                "google-calendar.list_calendars",
                "google-calendar.list_events",
                "google-calendar.get_event",
                "google-calendar.find_free_slots",
                "google-calendar.create_event",
                "google-calendar.update_event",
                "google-calendar.delete_event",
                "google-calendar.add_attendees",
                "google-calendar.set_reminder",
            ]
        );
        assert_eq!(
            calendar.summary.visible_read_only_capability_ids,
            vec![
                "google-calendar.list_calendars",
                "google-calendar.list_events",
                "google-calendar.get_event",
                "google-calendar.find_free_slots",
            ]
        );
        let search = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionSearch {
                    query: "gmail".to_string(),
                },
            )
            .await
            .expect("search Gmail extension");
        assert_eq!(search.phase, LifecyclePublicState::SetupNeeded);
        let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) =
            search.payload.as_ref()
        else {
            panic!("expected extension search payload");
        };
        assert_eq!(extensions.len(), 1);
        assert_eq!(extensions[0].summary.package_ref.id.as_str(), "gmail");
        assert_eq!(
            extensions[0].summary.visible_capability_ids,
            vec![
                "gmail.list_messages",
                "gmail.get_message",
                "gmail.send_message",
                "gmail.create_draft",
                "gmail.reply_to_message",
                "gmail.trash_message",
            ]
        );
        assert_eq!(
            extensions[0].summary.visible_read_only_capability_ids,
            vec!["gmail.list_messages", "gmail.get_message"]
        );

        let calendar_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "google-calendar")
                .expect("valid ref");
        let gmail_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "gmail").expect("valid ref");
        for package_ref in [calendar_ref.clone(), gmail_ref.clone()] {
            let install = facade
                .execute(
                    lifecycle_surface_context(),
                    LifecycleProductAction::ExtensionInstall {
                        package_ref: package_ref.clone(),
                    },
                )
                .await
                .expect("install extension");
            assert_eq!(install.phase, LifecyclePublicState::Active);
        }
        for path in [
            "system/extensions/google-calendar/manifest.toml",
            "system/extensions/google-calendar/schemas/google-calendar/list_events.input.v1.json",
            "system/extensions/google-calendar/prompts/google-calendar/create_event.md",
            "system/extensions/gmail/manifest.toml",
            "system/extensions/gmail/schemas/gmail/send_message.input.v1.json",
            "system/extensions/gmail/prompts/gmail/send_message.md",
        ] {
            assert!(storage_root.join(path).exists(), "missing {path}");
        }
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("google-calendar").unwrap())
                .is_some()
        );
        let active = active_registry.snapshot();
        assert!(
            active
                .get_capability(
                    &ironclaw_host_api::CapabilityId::new("google-calendar.list_events").unwrap()
                )
                .is_some()
        );
        assert!(
            active
                .get_capability(
                    &ironclaw_host_api::CapabilityId::new("gmail.send_message").unwrap()
                )
                .is_some()
        );

        for package_ref in [calendar_ref, gmail_ref] {
            let remove = facade
                .execute(
                    lifecycle_surface_context(),
                    LifecycleProductAction::ExtensionRemove { package_ref },
                )
                .await
                .expect("remove extension");
            assert_eq!(remove.phase, LifecyclePublicState::Uninstalled);
        }
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("gmail").unwrap())
                .is_none()
        );
        assert!(
            !storage_root
                .join("system/extensions/google-calendar/manifest.toml")
                .exists()
        );
        assert!(
            !storage_root
                .join("system/extensions/gmail/manifest.toml")
                .exists()
        );
    }

    #[tokio::test]
    async fn extension_install_rejects_skill_package_ref() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture();

        let error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Skill, "fixture")
                        .expect("valid skill ref"),
                },
            )
            .await
            .expect_err("extension install rejects non-extension refs");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
    }

    #[tokio::test]
    async fn extension_install_retry_is_idempotent_without_overwriting_materialized_files() {
        let (_dir, storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture();
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");

        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("initial install");
        let wasm_path = storage_root.join("system/extensions/fixture/wasm/fixture.wasm");
        std::fs::write(&wasm_path, b"existing-live-module").expect("rewrite installed module");

        let retry = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall { package_ref },
            )
            .await
            .expect("same-member install retry is idempotent");

        assert_eq!(retry.phase, LifecyclePublicState::Active);
        assert_eq!(
            std::fs::read(wasm_path).expect("installed module remains"),
            b"existing-live-module"
        );
    }

    #[tokio::test]
    async fn readiness_reconciliation_rejects_lifecycle_package_without_installation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");
        let mut filesystem = DiskFilesystem::new();
        filesystem
            .mount_local(
                VirtualPath::new("/system/extensions").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.join("system/extensions")),
            )
            .expect("mount system extensions");
        let package = fixture_extension_package().package;
        let mut lifecycle_registry = ExtensionRegistry::new();
        lifecycle_registry
            .insert(package.clone())
            .expect("lifecycle package");
        let mut active_registry_initial = ExtensionRegistry::new();
        active_registry_initial
            .insert(package)
            .expect("active package");
        let active_registry = Arc::new(SharedExtensionRegistry::new(active_registry_initial));
        let port = ExtensionManagementPort::new(
            Arc::new(filesystem),
            AvailableExtensionCatalog::from_packages(Vec::new()),
            Arc::new(filesystem_installation_store()),
            Arc::new(Mutex::new(ExtensionLifecycleService::new(
                lifecycle_registry,
            ))),
            test_active_extension_publisher(
                Arc::clone(&active_registry),
                test_extension_trust_policy(),
            ),
            None,
        );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");

        let error = port
            .activate(
                package_ref,
                ExtensionActivationMode::Static,
                &lifecycle_owner(),
            )
            .await
            .expect_err("activation requires an installation record");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("fixture").unwrap())
                .is_some()
        );
    }

    #[tokio::test]
    async fn extension_remove_rejects_uninstalled_ref_without_deleting_files() {
        let (_dir, storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture();
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "unmanaged-fixture")
                .expect("valid ref");
        let manifest_path = storage_root.join("system/extensions/unmanaged-fixture/manifest.toml");
        std::fs::create_dir_all(manifest_path.parent().expect("manifest parent"))
            .expect("extension directory");
        std::fs::write(&manifest_path, b"unmanaged manifest").expect("write unmanaged file");

        let error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove { package_ref },
            )
            .await
            .expect_err("remove requires an installation record");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        assert_eq!(
            std::fs::read(manifest_path).expect("unmanaged file remains"),
            b"unmanaged manifest"
        );
    }

    #[tokio::test]
    async fn extension_remove_aborts_when_personal_cleanup_discovery_fails() {
        let (_dir, storage_root, port, _active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").expect("valid ref");
        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install credentialed extension");
        let extension_id = ExtensionId::new("github").expect("valid extension id");
        port.lifecycle_service
            .lock()
            .await
            .remove(&extension_id)
            .await
            .expect("simulate provider discovery failure");
        let removal_scope = hosted_mcp_scope("extension-remove-provider-discovery");

        let error = port
            .remove(package_ref, &removal_scope, Some(&removal_scope.user_id))
            .await
            .expect_err("provider discovery failure must abort removal");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        let installation_id =
            ExtensionInstallationId::new("github").expect("valid installation id");
        assert!(
            installation_store
                .get_installation(&installation_id)
                .await
                .expect("installation lookup")
                .is_some(),
            "installation state must remain when cleanup discovery fails"
        );
        assert!(
            storage_root
                .join("system/extensions/github/manifest.toml")
                .exists(),
            "materialized package must remain when cleanup discovery fails"
        );
    }

    #[tokio::test]
    async fn extension_remove_lifecycle_failure_preserves_state() {
        let lifecycle_service = ExtensionLifecycleService::new(ExtensionRegistry::new())
            .with_event_sink(Arc::new(FailingRemoveLifecycleSink));
        let (_dir, storage_root, facade, active_registry, installation_store) =
            extension_lifecycle_fixture_with_service(lifecycle_service);
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");

        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install extension");

        let error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove { package_ref },
            )
            .await
            .expect_err("lifecycle remove failure is reported");

        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
        let extension_id = ExtensionId::new("fixture").expect("valid extension id");
        let installation_id = ExtensionInstallationId::new("fixture").expect("valid installation");
        assert!(
            active_registry
                .snapshot()
                .get_extension(&extension_id)
                .is_some()
        );
        assert!(
            storage_root
                .join("system/extensions/fixture/manifest.toml")
                .exists()
        );
        assert!(
            storage_root
                .join("system/extensions/fixture/wasm/fixture.wasm")
                .exists()
        );
        let installation = installation_store
            .get_installation(&installation_id)
            .await
            .expect("read installation")
            .expect("installation remains");
        assert!(installation.owner().visible_to(&lifecycle_owner()));
        assert!(
            installation_store
                .get_manifest(&extension_id)
                .await
                .expect("read manifest")
                .is_some()
        );
    }

    #[tokio::test]
    async fn extension_remove_installation_delete_failure_restores_active_trust_policy() {
        let (_dir, port, active_registry, failing_store, trust_policy) =
            extension_port_with_delete_installation_failing_store(ExtensionRegistry::new());
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");

        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install extension");
        port.activate_with_prechecked_credentials_for_test(
            package_ref.clone(),
            ExtensionActivationMode::Static,
            &lifecycle_owner(),
        )
        .await
        .expect("activate extension");
        let package = fixture_extension_package().package;
        let trust_input = extension_trust_policy_input(&package).expect("trust input");
        assert_eq!(
            trust_policy
                .evaluate(&trust_input)
                .expect("active extension trust")
                .effective_trust
                .class(),
            TrustClass::UserTrusted
        );

        let error = port
            .remove(package_ref, &hosted_mcp_scope("lifecycle-owner"), None)
            .await
            .expect_err("delete installation failure is reported");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        let extension_id = ExtensionId::new("fixture").expect("valid extension id");
        let installation_id = ExtensionInstallationId::new("fixture").expect("valid installation");
        let installation = failing_store
            .get_installation(&installation_id)
            .await
            .expect("read installation")
            .expect("installation remains");
        assert!(installation.owner().visible_to(&lifecycle_owner()));
        assert!(
            active_registry
                .snapshot()
                .get_extension(&extension_id)
                .is_some()
        );
        assert_eq!(
            trust_policy
                .evaluate(&trust_input)
                .expect("restored active extension trust")
                .effective_trust
                .class(),
            TrustClass::UserTrusted
        );
    }

    #[tokio::test]
    async fn extension_remove_manifest_delete_failure_leaves_retry_tombstone() {
        let (_dir, port, active_registry, failing_store, trust_policy) =
            extension_port_with_delete_manifest_failing_store();
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");

        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install extension");
        port.activate_with_prechecked_credentials_for_test(
            package_ref.clone(),
            ExtensionActivationMode::Static,
            &lifecycle_owner(),
        )
        .await
        .expect("activate extension");
        let package = fixture_extension_package().package;
        let trust_input = extension_trust_policy_input(&package).expect("trust input");

        let error = port
            .remove(package_ref, &hosted_mcp_scope("lifecycle-owner"), None)
            .await
            .expect_err("delete manifest failure is reported");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        let extension_id = ExtensionId::new("fixture").expect("valid extension id");
        let installation_id =
            ExtensionInstallationId::new("fixture").expect("valid installation id");
        assert!(
            failing_store
                .get_installation(&installation_id)
                .await
                .expect("installation lookup")
                .is_none(),
            "the runtime package is already removed"
        );
        assert!(
            active_registry
                .snapshot()
                .get_extension(&extension_id)
                .is_none(),
            "removed tools must stay unpublished"
        );
        assert_ne!(
            trust_policy
                .evaluate(&trust_input)
                .expect("removed extension trust")
                .effective_trust
                .class(),
            TrustClass::UserTrusted,
            "removed extension trust must stay revoked"
        );
        assert!(
            failing_store
                .get_manifest(&extension_id)
                .await
                .expect("manifest lookup")
                .is_some(),
            "failed finalization retains the durable cleanup tombstone"
        );
    }

    #[tokio::test]
    async fn extension_remove_file_delete_failure_restores_active_trust_policy() {
        let (_dir, port, active_registry, installation_store, trust_policy) =
            extension_port_with_file_delete_failing_filesystem();
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");

        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install extension");
        port.activate_with_prechecked_credentials_for_test(
            package_ref.clone(),
            ExtensionActivationMode::Static,
            &lifecycle_owner(),
        )
        .await
        .expect("activate extension");
        let package = fixture_extension_package().package;
        let trust_input = extension_trust_policy_input(&package).expect("trust input");

        let error = port
            .remove(package_ref, &hosted_mcp_scope("lifecycle-owner"), None)
            .await
            .expect_err("delete files failure is reported");

        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
        assert_installed_runtime_ready(&active_registry, installation_store.as_ref()).await;
        assert_eq!(
            trust_policy
                .evaluate(&trust_input)
                .expect("restored active extension trust")
                .effective_trust
                .class(),
            TrustClass::UserTrusted
        );
    }

    #[tokio::test]
    async fn project_package_returns_uninstalled_available_extension_projection() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture();
        let response = facade
            .project_package(
                lifecycle_surface_context(),
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").unwrap(),
            )
            .await
            .expect("extension projection");

        assert_eq!(response.phase, LifecyclePublicState::Uninstalled);
        let Some(LifecycleProductPayload::ExtensionList { extensions, count }) = response.payload
        else {
            panic!("expected extension list projection");
        };
        assert_eq!(count, 1);
        assert_eq!(extensions[0].phase, LifecyclePublicState::Uninstalled);
        assert_eq!(extensions[0].summary.package_ref.id.as_str(), "fixture");
    }

    fn extension_lifecycle_fixture() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        crate::extension_host::lifecycle::LifecycleFacade,
        Arc<SharedExtensionRegistry>,
        Arc<ExtensionInstallationStore>,
    ) {
        extension_lifecycle_fixture_with_catalog_and_service(
            AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
            ExtensionLifecycleService::new(ExtensionRegistry::new()),
        )
    }

    fn extension_lifecycle_fixture_with_service(
        lifecycle_service: ExtensionLifecycleService,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        crate::extension_host::lifecycle::LifecycleFacade,
        Arc<SharedExtensionRegistry>,
        Arc<ExtensionInstallationStore>,
    ) {
        extension_lifecycle_fixture_with_catalog_and_service(
            AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
            lifecycle_service,
        )
    }

    fn github_extension_lifecycle_fixture() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        crate::extension_host::lifecycle::LifecycleFacade,
        Arc<SharedExtensionRegistry>,
        Arc<ExtensionInstallationStore>,
    ) {
        extension_lifecycle_fixture_with_catalog_and_service(
            AvailableExtensionCatalog::from_first_party_assets()
                .expect("first-party GitHub catalog"),
            ExtensionLifecycleService::new(ExtensionRegistry::new()),
        )
    }

    #[derive(Default)]
    struct RecordingExtensionCredentialCleanup {
        requests: std::sync::Mutex<Vec<SecretCleanupRequest>>,
    }

    #[async_trait]
    impl ExtensionCredentialCleanup for RecordingExtensionCredentialCleanup {
        async fn cleanup_for_lifecycle(
            &self,
            request: SecretCleanupRequest,
        ) -> Result<SecretCleanupReport, ProductSurfaceError> {
            self.requests.lock().expect("cleanup lock").push(request);
            Ok(SecretCleanupReport::default())
        }
    }

    #[derive(Default)]
    struct FailThenQuarantineExtensionCredentialCleanup {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl ExtensionCredentialCleanup for FailThenQuarantineExtensionCredentialCleanup {
        async fn cleanup_for_lifecycle(
            &self,
            _request: SecretCleanupRequest,
        ) -> Result<SecretCleanupReport, ProductSurfaceError> {
            match self.calls.fetch_add(1, Ordering::SeqCst) {
                0 => Err(ProductSurfaceError::internal_from(
                    "credential cleanup backend unavailable",
                )),
                1 => Ok(SecretCleanupReport {
                    quarantined_accounts: vec![ironclaw_auth::SecretCleanupQuarantine {
                        account_id: ironclaw_auth::CredentialAccountId::new(),
                        reason: ironclaw_auth::SecretCleanupQuarantineReason::BackendUnavailable,
                    }],
                    ..SecretCleanupReport::default()
                }),
                _ => Ok(SecretCleanupReport::default()),
            }
        }
    }

    #[tokio::test]
    async fn ui_facade_extension_remove_retries_incomplete_credential_cleanup_until_converged() {
        let cleanup = Arc::new(FailThenQuarantineExtensionCredentialCleanup::default());
        let (_dir, storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture_with_catalog_service_and_cleanup(
                AvailableExtensionCatalog::from_first_party_assets()
                    .expect("first-party GitHub catalog"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
                Some(cleanup.clone() as Arc<dyn ExtensionCredentialCleanup>),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").expect("valid ref");

        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install github");
        let backend_error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect_err("cleanup backend failure must make removal retryable");
        let ProductWorkflowError::Transient { reason } = backend_error else {
            panic!("cleanup backend failure must be operational and retryable");
        };
        assert!(reason.contains("retry removal"));
        assert!(
            storage_root.join("system/extensions/github").exists(),
            "the owned installation remains authoritative until actor-scoped cleanup converges"
        );

        let quarantined_error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect_err("quarantined cleanup must not report removal success");
        let ProductWorkflowError::Transient { reason } = quarantined_error else {
            panic!("quarantined cleanup must be operational and retryable");
        };
        assert!(reason.contains("retry removal"));

        let retry = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove { package_ref },
            )
            .await
            .expect("owner retry completes quarantined cleanup and removal");
        assert!(matches!(
            retry.payload,
            Some(LifecycleProductPayload::ExtensionRemove { removed: true })
        ));
        assert!(retry.message.is_none());
        assert!(!storage_root.join("system/extensions/github").exists());
        // Four calls: the failing then quarantined extension-keyed attempts
        // (one per rejected removal), then the converging removal's
        // extension-keyed + provider-selected pair.
        assert_eq!(cleanup.calls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn ui_facade_extension_remove_revokes_exclusive_credential_at_convergence_point() {
        // Convergence coverage: the WebUI facade removal door (`ExtensionRemove`)
        // and the `builtin.extension_remove` agent capability both call
        // `ExtensionManagementPort::remove`, so credential revocation
        // cannot be bypassed through the UI door — the door users actually use.
        let cleanup = Arc::new(RecordingExtensionCredentialCleanup::default());
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture_with_catalog_service_and_cleanup(
                AvailableExtensionCatalog::from_first_party_assets()
                    .expect("first-party GitHub catalog"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
                Some(cleanup.clone() as Arc<dyn ExtensionCredentialCleanup>),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").expect("valid ref");

        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install github");
        let remove = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("remove github via the WebUI facade");
        assert_eq!(remove.phase, LifecyclePublicState::Uninstalled);
        let retry = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove { package_ref },
            )
            .await
            .expect("retry removal after github is absent");
        assert_eq!(retry.phase, LifecyclePublicState::Uninstalled);
        assert!(matches!(
            retry.payload,
            Some(LifecycleProductPayload::ExtensionRemove { removed: false })
        ));

        let requests = cleanup.requests.lock().expect("cleanup lock");
        assert_eq!(
            requests.len(),
            4,
            "initial removal and an already-absent retry must both run the \
             extension-keyed cleanup and revoke the exclusive github credential"
        );
        for pair in requests.chunks(2) {
            assert!(pair[0].provider.is_none());
            assert_eq!(
                pair[0]
                    .lifecycle_package
                    .as_ref()
                    .map(|package| package.as_str()),
                Some("github")
            );
            assert_eq!(
                pair[1].provider.as_ref().map(|provider| provider.as_str()),
                Some("github")
            );
            for request in pair {
                assert_eq!(request.extension_id.as_str(), "github");
                assert_eq!(request.action, SecretCleanupAction::Uninstall);
            }
        }
    }

    #[tokio::test]
    async fn extension_remove_does_not_hold_operation_lock_while_waiting_for_catalog() {
        let (_dir, _storage_root, port, _active_registry, _installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").expect("valid ref");
        let removal_scope = hosted_mcp_scope("lifecycle-owner");

        let catalog_guard = port.catalog.write().await;
        let spawned_port = Arc::clone(&port);
        let spawned_scope = removal_scope.clone();
        let removal = tokio::spawn(async move {
            spawned_port
                .remove(package_ref, &spawned_scope, Some(&spawned_scope.user_id))
                .await
        });
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let operation_guard = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            port.operation_lock.lock(),
        )
        .await
        .expect("remove must wait for the catalog before taking operation_lock");
        drop(operation_guard);
        drop(catalog_guard);

        removal
            .await
            .expect("remove task joins")
            .expect("already-absent repair converges after catalog lock release");
    }

    #[tokio::test]
    async fn extension_remove_uses_installed_manifest_when_catalog_entry_disappears() {
        let cleanup = Arc::new(RecordingExtensionCredentialCleanup::default());
        let (_dir, storage_root, installed_port, _active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").expect("valid ref");
        installed_port
            .install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install github");

        // Simulate a process restart after the bundled catalog dropped or
        // renamed the entry. The persisted manifest remains the authoritative
        // record of the cleanup owed by this installed package.
        let port = ExtensionManagementPort::new(
            Arc::clone(&installed_port.filesystem),
            AvailableExtensionCatalog::from_packages(Vec::new()),
            installation_store,
            Arc::clone(&installed_port.lifecycle_service),
            installed_port.active_extensions.clone(),
            Some(cleanup.clone() as Arc<dyn ExtensionCredentialCleanup>),
        );
        let removal_scope = hosted_mcp_scope("lifecycle-owner");
        let response = port
            .remove(package_ref, &removal_scope, Some(&removal_scope.user_id))
            .await
            .expect("installed manifest permits complete removal without a catalog entry");

        assert!(matches!(
            response.payload,
            Some(LifecycleProductPayload::ExtensionRemove { removed: true })
        ));
        assert!(
            !storage_root.join("system/extensions/github").exists(),
            "installed files must be removed"
        );
        let requests = cleanup.requests.lock().expect("cleanup lock");
        assert_eq!(requests.len(), 2);
        assert!(requests[0].provider.is_none());
        assert_eq!(
            requests[0]
                .lifecycle_package
                .as_ref()
                .map(|package| package.as_str()),
            Some("github")
        );
        assert_eq!(
            requests[1].provider.as_ref().map(AuthProviderId::as_str),
            Some("github"),
            "cleanup provider must come from the persisted installed manifest"
        );
    }

    #[tokio::test]
    async fn extension_remove_retries_actor_scoped_cleanup_without_catalog_entry() {
        let cleanup = Arc::new(FailThenQuarantineExtensionCredentialCleanup::default());
        let (_dir, storage_root, installed_port, _active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").expect("valid ref");
        let removal_scope = hosted_mcp_scope("alice");
        installed_port
            .install(package_ref.clone(), &removal_scope.user_id)
            .await
            .expect("install github");
        let port = ExtensionManagementPort::new(
            Arc::clone(&installed_port.filesystem),
            AvailableExtensionCatalog::from_packages(Vec::new()),
            installation_store.clone(),
            Arc::clone(&installed_port.lifecycle_service),
            installed_port.active_extensions.clone(),
            Some(cleanup.clone() as Arc<dyn ExtensionCredentialCleanup>),
        );
        let error = port
            .remove(
                package_ref.clone(),
                &removal_scope,
                Some(&removal_scope.user_id),
            )
            .await
            .expect_err("backend failure keeps the owned installation authoritative");
        assert!(matches!(error, ProductWorkflowError::Transient { .. }));

        let foreign_scope = hosted_mcp_scope("bob");
        let foreign_error = port
            .remove(
                package_ref.clone(),
                &foreign_scope,
                Some(&foreign_scope.user_id),
            )
            .await
            .expect_err("another user cannot take over the cleanup retry");
        assert!(matches!(
            foreign_error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));

        let error = port
            .remove(
                package_ref.clone(),
                &removal_scope,
                Some(&removal_scope.user_id),
            )
            .await
            .expect_err("quarantined cleanup remains retryable by the owner");
        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
        assert!(
            storage_root.join("system/extensions/github").exists(),
            "package state remains owned until actor-scoped cleanup converges"
        );
        let extension_id = ExtensionId::new("github").expect("valid extension id");
        let installation_id =
            ExtensionInstallationId::new("github").expect("valid installation id");
        assert!(
            installation_store
                .get_installation(&installation_id)
                .await
                .expect("installation lookup")
                .is_some(),
            "the owner row prevents a foreign user from finalizing cleanup"
        );
        assert!(
            installation_store
                .get_manifest(&extension_id)
                .await
                .expect("manifest lookup")
                .is_some(),
            "persisted manifest remains authoritative while cleanup retries"
        );

        let response = port
            .remove(package_ref, &removal_scope, Some(&removal_scope.user_id))
            .await
            .expect("owner retry converges without catalog metadata");
        assert!(matches!(
            response.payload,
            Some(LifecycleProductPayload::ExtensionRemove { removed: true })
        ));
        assert!(!storage_root.join("system/extensions/github").exists());
        assert!(
            installation_store
                .get_manifest(&extension_id)
                .await
                .expect("manifest lookup")
                .is_none(),
            "cleanup tombstone is deleted only after convergence"
        );
    }

    #[tokio::test]
    async fn already_absent_catalog_repair_persists_tombstone_before_cleanup() {
        let cleanup = Arc::new(FailThenQuarantineExtensionCredentialCleanup::default());
        let (_dir, _storage_root, base_port, _active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").expect("valid ref");
        let removal_scope = hosted_mcp_scope("lifecycle-owner");
        let repair_port = ExtensionManagementPort::new(
            Arc::clone(&base_port.filesystem),
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
            installation_store.clone(),
            Arc::clone(&base_port.lifecycle_service),
            base_port.active_extensions.clone(),
            Some(cleanup.clone() as Arc<dyn ExtensionCredentialCleanup>),
        );
        repair_port
            .remove(
                package_ref.clone(),
                &removal_scope,
                Some(&removal_scope.user_id),
            )
            .await
            .expect_err("first repair cleanup fails after seeding its tombstone");

        let extension_id = ExtensionId::new("github").expect("valid extension id");
        assert!(
            installation_store
                .get_manifest(&extension_id)
                .await
                .expect("manifest lookup")
                .is_some(),
            "catalog repair metadata must survive a cleanup failure"
        );
        let no_catalog_port = ExtensionManagementPort::new(
            Arc::clone(&base_port.filesystem),
            AvailableExtensionCatalog::from_packages(Vec::new()),
            installation_store.clone(),
            Arc::clone(&base_port.lifecycle_service),
            base_port.active_extensions.clone(),
            Some(cleanup.clone() as Arc<dyn ExtensionCredentialCleanup>),
        );
        no_catalog_port
            .remove(
                package_ref.clone(),
                &removal_scope,
                Some(&removal_scope.user_id),
            )
            .await
            .expect_err("quarantined repair remains retryable without catalog metadata");
        no_catalog_port
            .remove(package_ref, &removal_scope, Some(&removal_scope.user_id))
            .await
            .expect("repair converges from tombstone after catalog removal");
        assert!(
            installation_store
                .get_manifest(&extension_id)
                .await
                .expect("manifest lookup")
                .is_none()
        );
    }

    #[tokio::test]
    async fn manifest_only_retry_removes_orphaned_active_runtime_and_files() {
        let cleanup = Arc::new(RecordingExtensionCredentialCleanup::default());
        let (_dir, storage_root, port, active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").expect("valid ref");
        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install github");
        port.activate_with_prechecked_credentials_for_test(
            package_ref.clone(),
            ExtensionActivationMode::Static,
            &lifecycle_owner(),
        )
        .await
        .expect("activate github");
        let installation_id =
            ExtensionInstallationId::new("github").expect("valid installation id");
        installation_store
            .delete_installation(&installation_id)
            .await
            .expect("simulate failed installation restoration");

        let retry_port = ExtensionManagementPort::new(
            Arc::clone(&port.filesystem),
            AvailableExtensionCatalog::from_packages(Vec::new()),
            installation_store.clone(),
            Arc::clone(&port.lifecycle_service),
            port.active_extensions.clone(),
            Some(cleanup as Arc<dyn ExtensionCredentialCleanup>),
        );
        let removal_scope = hosted_mcp_scope("lifecycle-owner");
        retry_port
            .remove(package_ref, &removal_scope, Some(&removal_scope.user_id))
            .await
            .expect("manifest-only retry converges orphan runtime state");

        let extension_id = ExtensionId::new("github").expect("valid extension id");
        assert!(
            active_registry
                .snapshot()
                .get_extension(&extension_id)
                .is_none()
        );
        assert!(
            retry_port
                .lifecycle_service
                .lock()
                .await
                .registry()
                .get_extension(&extension_id)
                .is_none()
        );
        assert!(
            !storage_root.join("system/extensions/github").exists(),
            "orphan materialized files are deleted"
        );
        assert!(
            installation_store
                .get_manifest(&extension_id)
                .await
                .expect("manifest lookup")
                .is_none(),
            "cleanup tombstone is finalized"
        );
    }

    #[tokio::test]
    async fn fresh_catalog_repair_removes_orphan_runtime_without_installation_records() {
        let (_dir, storage_root, port, active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").expect("valid ref");
        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install github");
        port.activate_with_prechecked_credentials_for_test(
            package_ref.clone(),
            ExtensionActivationMode::Static,
            &lifecycle_owner(),
        )
        .await
        .expect("activate github");
        let extension_id = ExtensionId::new("github").expect("valid extension id");
        let installation_id =
            ExtensionInstallationId::new("github").expect("valid installation id");
        installation_store
            .delete_installation(&installation_id)
            .await
            .expect("delete installation");
        installation_store
            .delete_manifest(&extension_id)
            .await
            .expect("delete manifest");

        let repair_port = ExtensionManagementPort::new(
            Arc::clone(&port.filesystem),
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
            installation_store,
            Arc::clone(&port.lifecycle_service),
            port.active_extensions.clone(),
            None,
        );
        let removal_scope = hosted_mcp_scope("lifecycle-owner");
        repair_port
            .remove(package_ref, &removal_scope, Some(&removal_scope.user_id))
            .await
            .expect("catalog-authorized orphan cleanup converges");

        assert!(
            active_registry
                .snapshot()
                .get_extension(&extension_id)
                .is_none()
        );
        assert!(
            repair_port
                .lifecycle_service
                .lock()
                .await
                .registry()
                .get_extension(&extension_id)
                .is_none()
        );
        assert!(!storage_root.join("system/extensions/github").exists());
    }

    #[tokio::test]
    async fn fresh_catalog_repair_removes_files_only_orphan() {
        let (_dir, storage_root, port, _active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").expect("valid ref");
        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install github");
        let extension_id = ExtensionId::new("github").expect("valid extension id");
        let installation_id =
            ExtensionInstallationId::new("github").expect("valid installation id");
        installation_store
            .delete_installation(&installation_id)
            .await
            .expect("delete installation");
        installation_store
            .delete_manifest(&extension_id)
            .await
            .expect("delete manifest");
        port.lifecycle_service
            .lock()
            .await
            .remove(&extension_id)
            .await
            .expect("remove runtime registry entry");
        assert!(storage_root.join("system/extensions/github").exists());

        let repair_port = ExtensionManagementPort::new(
            Arc::clone(&port.filesystem),
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
            installation_store,
            Arc::clone(&port.lifecycle_service),
            port.active_extensions.clone(),
            None,
        );
        let removal_scope = hosted_mcp_scope("lifecycle-owner");
        repair_port
            .remove(package_ref, &removal_scope, Some(&removal_scope.user_id))
            .await
            .expect("files-only orphan cleanup converges");

        assert!(!storage_root.join("system/extensions/github").exists());
    }

    #[tokio::test]
    async fn ui_facade_extension_remove_preserves_credential_still_shared_with_another_extension() {
        // Fail-safe coverage for `revoke_exclusive_credentials`: `gmail` and
        // `google-calendar` both authorize against the shared `google` provider.
        // Removing `gmail` while `google-calendar` remains installed must NOT
        // revoke the personal Google credential — it is still exclusive to the
        // remaining extension, and deleting it would silently break Calendar.
        // This exercises the `providers_still_in_use` preservation branch the
        // single-extension revoke test above cannot reach.
        let cleanup = Arc::new(RecordingExtensionCredentialCleanup::default());
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture_with_catalog_service_and_cleanup(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
                Some(cleanup.clone() as Arc<dyn ExtensionCredentialCleanup>),
            );
        let gmail =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "gmail").expect("valid ref");
        let calendar = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "google-calendar")
            .expect("valid ref");

        for package_ref in [gmail.clone(), calendar.clone()] {
            facade
                .execute(
                    lifecycle_surface_context(),
                    LifecycleProductAction::ExtensionInstall { package_ref },
                )
                .await
                .expect("install Google extension");
        }

        let remove = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove { package_ref: gmail },
            )
            .await
            .expect("remove gmail via the WebUI facade");
        assert_eq!(remove.phase, LifecyclePublicState::Uninstalled);

        let requests = cleanup.requests.lock().expect("cleanup lock");
        assert!(
            requests.iter().all(|request| request.provider.is_none()),
            "the shared google credential must be preserved while google-calendar \
             still authorizes against it, got cleanup requests: {requests:?}"
        );
        // The removed package's OWN cleanup (flows + grants) still runs.
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]
                .lifecycle_package
                .as_ref()
                .map(|package| package.as_str()),
            Some("gmail")
        );
    }

    fn extension_management_port_fixture_with_catalog_and_service(
        catalog: AvailableExtensionCatalog,
        lifecycle_service: ExtensionLifecycleService,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        Arc<ExtensionManagementPort>,
        Arc<SharedExtensionRegistry>,
        Arc<ExtensionInstallationStore>,
    ) {
        let (dir, storage_root, extension_management, active_registry, installation_store, _) =
            extension_management_port_fixture_with_catalog_service_and_trust(
                catalog,
                lifecycle_service,
            );
        (
            dir,
            storage_root,
            extension_management,
            active_registry,
            installation_store,
        )
    }

    fn extension_management_port_fixture_with_removal_cleanup(
        catalog: AvailableExtensionCatalog,
        lifecycle_service: ExtensionLifecycleService,
        removal_cleanup: Arc<ExtensionRemovalCleanupRegistry>,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        Arc<ExtensionManagementPort>,
        Arc<SharedExtensionRegistry>,
        Arc<ExtensionInstallationStore>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");

        let mut filesystem = DiskFilesystem::new();
        filesystem
            .mount_local(
                VirtualPath::new("/projects").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.clone()),
            )
            .expect("mount storage root");
        filesystem
            .mount_local(
                VirtualPath::new("/system/extensions").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.join("system/extensions")),
            )
            .expect("mount system extensions");
        let root_filesystem: Arc<dyn RootFilesystem> = Arc::new(filesystem);
        let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let installation_store = Arc::new(filesystem_installation_store());
        let extension_management = Arc::new(
            ExtensionManagementPort::new(
                root_filesystem,
                catalog,
                installation_store.clone(),
                Arc::new(Mutex::new(lifecycle_service)),
                test_active_extension_publisher(
                    Arc::clone(&active_registry),
                    test_extension_trust_policy(),
                ),
                None,
            )
            .with_removal_cleanup_registry(removal_cleanup),
        );
        (
            dir,
            storage_root,
            extension_management,
            active_registry,
            installation_store,
        )
    }

    fn extension_management_port_fixture_with_credential_cleanup(
        catalog: AvailableExtensionCatalog,
        credential_cleanup: Arc<dyn ExtensionCredentialCleanup>,
    ) -> (tempfile::TempDir, Arc<ExtensionManagementPort>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");
        let mut filesystem = DiskFilesystem::new();
        filesystem
            .mount_local(
                VirtualPath::new("/projects").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.clone()),
            )
            .expect("mount storage root");
        filesystem
            .mount_local(
                VirtualPath::new("/system/extensions").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.join("system/extensions")),
            )
            .expect("mount system extensions");
        let root_filesystem: Arc<dyn RootFilesystem> = Arc::new(filesystem);
        let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let installation_store = Arc::new(filesystem_installation_store());
        let extension_management = Arc::new(ExtensionManagementPort::new(
            root_filesystem,
            catalog,
            installation_store,
            Arc::new(Mutex::new(ExtensionLifecycleService::new(
                ExtensionRegistry::new(),
            ))),
            test_active_extension_publisher(
                Arc::clone(&active_registry),
                test_extension_trust_policy(),
            ),
            Some(credential_cleanup),
        ));
        (dir, extension_management)
    }

    /// Removing an extension whose provider is still declared by another
    /// installed extension must skip the provider-selected revocation (the
    /// shared personal credential survives) but must STILL issue the
    /// extension-keyed cleanup so the removed package's own flows and grants
    /// die with it.
    #[tokio::test]
    async fn remove_with_shared_provider_issues_extension_keyed_cleanup() {
        let cleanup = Arc::new(RecordingExtensionCredentialCleanup::default());
        let (_dir, port) = extension_management_port_fixture_with_credential_cleanup(
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets"),
            cleanup.clone(),
        );
        let actor = UserId::new("authenticated-actor").expect("valid actor");
        let gmail =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "gmail").expect("valid ref");
        let calendar = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "google-calendar")
            .expect("valid ref");
        port.install(gmail.clone(), &actor)
            .await
            .expect("install gmail");
        port.install(calendar, &actor)
            .await
            .expect("install google-calendar");

        port.remove(
            gmail,
            &hosted_mcp_scope("authenticated-actor"),
            Some(&actor),
        )
        .await
        .expect("remove gmail");

        let requests = cleanup.requests.lock().expect("cleanup lock").clone();
        assert!(
            requests.iter().all(|request| request.provider.is_none()),
            "google is still used by google-calendar, so no provider-selected revocation may run: {requests:?}"
        );
        assert_eq!(
            requests.len(),
            1,
            "the extension-keyed cleanup must run exactly once: {requests:?}"
        );
        assert_eq!(requests[0].extension_id.as_str(), "gmail");
        assert_eq!(
            requests[0]
                .lifecycle_package
                .as_ref()
                .map(|package| package.as_str()),
            Some("gmail"),
            "the removed package ref rides the cleanup request so its own flows are canceled"
        );
        assert!(matches!(requests[0].action, SecretCleanupAction::Uninstall));
    }

    /// A removed extension that declares NO credential providers still gets
    /// the extension-keyed cleanup: grants pointing at it are stripped, so a
    /// later reinstall of the same id cannot silently inherit stale
    /// credential authorization.
    #[tokio::test]
    async fn remove_without_declared_providers_still_issues_extension_keyed_cleanup() {
        let cleanup = Arc::new(RecordingExtensionCredentialCleanup::default());
        let (_dir, port) = extension_management_port_fixture_with_credential_cleanup(
            AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
            cleanup.clone(),
        );
        let actor = UserId::new("authenticated-actor").expect("valid actor");
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");
        port.install(package_ref.clone(), &actor)
            .await
            .expect("install fixture");

        port.remove(
            package_ref,
            &hosted_mcp_scope("authenticated-actor"),
            Some(&actor),
        )
        .await
        .expect("remove fixture");

        let requests = cleanup.requests.lock().expect("cleanup lock").clone();
        assert_eq!(
            requests.len(),
            1,
            "the extension-keyed cleanup must run even with no declared providers: {requests:?}"
        );
        assert!(requests[0].provider.is_none());
        assert_eq!(
            requests[0]
                .lifecycle_package
                .as_ref()
                .map(|package| package.as_str()),
            Some("fixture")
        );
        assert!(matches!(requests[0].action, SecretCleanupAction::Uninstall));
    }

    async fn assert_removal_target_preserved(
        storage_root: &std::path::Path,
        installation_store: &ExtensionInstallationStore,
        extension_id: &str,
    ) {
        assert!(
            storage_root
                .join(format!("system/extensions/{extension_id}"))
                .exists(),
            "package files must remain when cleanup fails"
        );
        assert!(
            installation_store
                .get_manifest(&ExtensionId::new(extension_id).expect("valid extension id"))
                .await
                .expect("manifest lookup")
                .is_some(),
            "manifest must remain when cleanup fails"
        );
        assert!(
            installation_store
                .get_installation(
                    &ExtensionInstallationId::new(extension_id).expect("valid installation id")
                )
                .await
                .expect("installation lookup")
                .is_some(),
            "installation must remain when cleanup fails"
        );
    }

    fn extension_management_port_fixture_with_catalog_service_and_trust(
        catalog: AvailableExtensionCatalog,
        lifecycle_service: ExtensionLifecycleService,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        Arc<ExtensionManagementPort>,
        Arc<SharedExtensionRegistry>,
        Arc<ExtensionInstallationStore>,
        Arc<HostTrustPolicy>,
    ) {
        let trust_policy = test_extension_trust_policy();
        let (dir, storage_root, extension_management, active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_service_and_trust_policy(
                catalog,
                lifecycle_service,
                Arc::clone(&trust_policy),
            );
        (
            dir,
            storage_root,
            extension_management,
            active_registry,
            installation_store,
            trust_policy,
        )
    }

    fn extension_management_port_fixture_with_catalog_service_and_trust_policy(
        catalog: AvailableExtensionCatalog,
        lifecycle_service: ExtensionLifecycleService,
        trust_policy: Arc<HostTrustPolicy>,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        Arc<ExtensionManagementPort>,
        Arc<SharedExtensionRegistry>,
        Arc<ExtensionInstallationStore>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");

        let mut filesystem = DiskFilesystem::new();
        filesystem
            .mount_local(
                VirtualPath::new("/projects").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.clone()),
            )
            .expect("mount storage root");
        filesystem
            .mount_local(
                VirtualPath::new("/system/extensions").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.join("system/extensions")),
            )
            .expect("mount system extensions");
        let filesystem = Arc::new(filesystem);
        let root_filesystem: Arc<dyn RootFilesystem> = filesystem.clone();
        let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let installation_store = Arc::new(filesystem_installation_store());
        let extension_management = Arc::new(ExtensionManagementPort::new(
            root_filesystem,
            catalog,
            installation_store.clone(),
            Arc::new(Mutex::new(lifecycle_service)),
            test_active_extension_publisher(
                Arc::clone(&active_registry),
                Arc::clone(&trust_policy),
            ),
            None,
        ));
        (
            dir,
            storage_root,
            extension_management,
            active_registry,
            installation_store,
        )
    }

    /// Same assembly as [`extension_management_port_fixture_with_catalog_service_and_trust_policy`],
    /// plus an opted-in provider-instance readiness map. The Google-family
    /// variant below exercises the readiness-map chokepoint in
    /// `activation_credential_requirements` directly, since the OTHER port
    /// fixtures in this module deliberately build with
    /// `ExtensionManagementPort::new` alone (proving the opt-in
    /// default stays empty for them).
    fn extension_management_port_fixture_with_readiness_map(
        catalog: AvailableExtensionCatalog,
        lifecycle_service: ExtensionLifecycleService,
        provider_instance_readiness: std::collections::BTreeSet<VendorId>,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        Arc<ExtensionManagementPort>,
        Arc<SharedExtensionRegistry>,
        Arc<ExtensionInstallationStore>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");

        let mut filesystem = DiskFilesystem::new();
        filesystem
            .mount_local(
                VirtualPath::new("/projects").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.clone()),
            )
            .expect("mount storage root");
        filesystem
            .mount_local(
                VirtualPath::new("/system/extensions").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.join("system/extensions")),
            )
            .expect("mount system extensions");
        let filesystem = Arc::new(filesystem);
        let root_filesystem: Arc<dyn RootFilesystem> = filesystem.clone();
        let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let installation_store = Arc::new(filesystem_installation_store());
        let extension_management = Arc::new(
            ExtensionManagementPort::new(
                root_filesystem,
                catalog,
                installation_store.clone(),
                Arc::new(Mutex::new(lifecycle_service)),
                test_active_extension_publisher(
                    Arc::clone(&active_registry),
                    test_extension_trust_policy(),
                ),
                None,
            )
            .with_provider_instance_readiness(provider_instance_readiness),
        );
        (
            dir,
            storage_root,
            extension_management,
            active_registry,
            installation_store,
        )
    }

    /// A provider-instance readiness-map entry (the operator never
    /// configured this provider's OAuth backend on this instance at all)
    /// must fail `activation_credential_requirements`
    /// BEFORE the per-account credential gate without exposing administrator
    /// field metadata — the one chokepoint both the LLM tool handler and the
    /// WebUI card path call through. The integration-tier
    /// regression for this exact user-visible behavior lives in
    /// `tests/integration/group_extensions/scenario_extension_activation_instance_not_configured.rs`;
    /// this crate-tier test pins the underlying port contract directly.
    #[tokio::test]
    async fn google_family_activation_fails_closed_when_provider_instance_not_configured() {
        let mut readiness = std::collections::BTreeSet::new();
        readiness
            .insert(VendorId::new(ironclaw_auth::GOOGLE_PROVIDER_ID).expect("valid provider id"));
        let (_dir, _storage_root, port, _active_registry, _installation_store) =
            extension_management_port_fixture_with_readiness_map(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
                readiness,
            );
        let gcal_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "google-calendar")
            .expect("google-calendar ref");
        port.install(gcal_ref.clone(), &lifecycle_owner())
            .await
            .expect("install google-calendar");

        let error = port
            .activation_credential_requirements(&gcal_ref, &lifecycle_owner())
            .await
            .expect_err("an unconfigured provider instance must fail closed");

        let ProductWorkflowError::ProviderInstanceNotConfigured = error else {
            panic!("expected ProviderInstanceNotConfigured, got {error:?}");
        };
    }

    // Default-empty-map regression: every OTHER
    // `activation_credential_requirements` test in this module (e.g.
    // `slack_tools_extension_activation_requires_personal_oauth` above, the
    // telegram tests below) builds its port via
    // `ExtensionManagementPort::new` with no
    // `.with_provider_instance_readiness` call and keeps passing unchanged —
    // proving the opt-in default-empty map is a true no-op for every port
    // that never opts in. No new test is added for this: it is exactly what
    // those pre-existing tests already demonstrate.

    fn extension_lifecycle_fixture_with_catalog_and_service(
        catalog: AvailableExtensionCatalog,
        lifecycle_service: ExtensionLifecycleService,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        crate::extension_host::lifecycle::LifecycleFacade,
        Arc<SharedExtensionRegistry>,
        Arc<ExtensionInstallationStore>,
    ) {
        extension_lifecycle_fixture_with_catalog_service_and_cleanup(
            catalog,
            lifecycle_service,
            None,
        )
    }

    fn extension_lifecycle_fixture_with_catalog_service_and_cleanup(
        catalog: AvailableExtensionCatalog,
        lifecycle_service: ExtensionLifecycleService,
        credential_cleanup: Option<Arc<dyn ExtensionCredentialCleanup>>,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        crate::extension_host::lifecycle::LifecycleFacade,
        Arc<SharedExtensionRegistry>,
        Arc<ExtensionInstallationStore>,
    ) {
        extension_lifecycle_fixture_with_all_cleanup(
            catalog,
            lifecycle_service,
            credential_cleanup,
            Arc::new(ExtensionRemovalCleanupRegistry::empty()),
            None,
        )
    }

    fn extension_lifecycle_fixture_with_all_cleanup(
        catalog: AvailableExtensionCatalog,
        lifecycle_service: ExtensionLifecycleService,
        credential_cleanup: Option<Arc<dyn ExtensionCredentialCleanup>>,
        removal_cleanup: Arc<ExtensionRemovalCleanupRegistry>,
        channel_connection_slot: Option<Arc<std::sync::OnceLock<Arc<dyn ChannelConnectionFacade>>>>,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        crate::extension_host::lifecycle::LifecycleFacade,
        Arc<SharedExtensionRegistry>,
        Arc<ExtensionInstallationStore>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");

        let mut filesystem = DiskFilesystem::new();
        filesystem
            .mount_local(
                VirtualPath::new("/projects").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.clone()),
            )
            .expect("mount storage root");
        filesystem
            .mount_local(
                VirtualPath::new("/system/extensions").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.join("system/extensions")),
            )
            .expect("mount system extensions");
        let filesystem = Arc::new(filesystem);
        let root_filesystem: Arc<dyn RootFilesystem> = filesystem.clone();
        let skill_management =
            Arc::new(crate::extension_host::lifecycle::SkillManagementPort::new(
                UserId::new("lifecycle-owner").expect("valid user"),
                root_filesystem.clone(),
                MountView::new(vec![MountGrant::new(
                    MountAlias::new("/skills").expect("valid alias"),
                    VirtualPath::new("/projects/skills").expect("valid path"),
                    MountPermissions::read_write_list_delete(),
                )])
                .expect("valid mount view"),
            ));
        let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let installation_store = Arc::new(filesystem_installation_store());
        let mut extension_management_port = ExtensionManagementPort::new(
            root_filesystem,
            catalog,
            installation_store.clone(),
            Arc::new(Mutex::new(lifecycle_service)),
            test_active_extension_publisher(
                Arc::clone(&active_registry),
                test_extension_trust_policy(),
            ),
            credential_cleanup,
        )
        .with_removal_cleanup_registry(removal_cleanup);
        if let Some(slot) = channel_connection_slot {
            extension_management_port =
                extension_management_port.with_channel_disconnect_slot(slot);
        }
        let extension_management = Arc::new(extension_management_port);
        let facade = crate::extension_host::lifecycle::LifecycleFacade::new(skill_management)
            .with_extension_management(extension_management);
        (
            dir,
            storage_root,
            facade,
            active_registry,
            installation_store,
        )
    }

    fn extension_port_with_delete_installation_failing_store(
        initial_active_registry: ExtensionRegistry,
    ) -> (
        tempfile::TempDir,
        ExtensionManagementPort,
        Arc<SharedExtensionRegistry>,
        Arc<DeleteInstallationFailingStore>,
        Arc<HostTrustPolicy>,
    ) {
        extension_port_with_delete_failing_store(
            initial_active_registry,
            DeleteInstallationFailingStore::default(),
        )
    }

    fn extension_port_with_delete_manifest_failing_store() -> (
        tempfile::TempDir,
        ExtensionManagementPort,
        Arc<SharedExtensionRegistry>,
        Arc<DeleteInstallationFailingStore>,
        Arc<HostTrustPolicy>,
    ) {
        extension_port_with_delete_failing_store(
            ExtensionRegistry::new(),
            DeleteInstallationFailingStore::fail_manifest_delete(),
        )
    }

    fn extension_port_with_delete_failing_store(
        initial_active_registry: ExtensionRegistry,
        failing_store: DeleteInstallationFailingStore,
    ) -> (
        tempfile::TempDir,
        ExtensionManagementPort,
        Arc<SharedExtensionRegistry>,
        Arc<DeleteInstallationFailingStore>,
        Arc<HostTrustPolicy>,
    ) {
        extension_port_with_failing_store(
            initial_active_registry,
            failing_store,
            ExtensionLifecycleService::new(ExtensionRegistry::new()),
        )
    }

    fn extension_port_with_failing_store(
        initial_active_registry: ExtensionRegistry,
        failing_store: DeleteInstallationFailingStore,
        lifecycle_service: ExtensionLifecycleService,
    ) -> (
        tempfile::TempDir,
        ExtensionManagementPort,
        Arc<SharedExtensionRegistry>,
        Arc<DeleteInstallationFailingStore>,
        Arc<HostTrustPolicy>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");
        let mut filesystem = DiskFilesystem::new();
        filesystem
            .mount_local(
                VirtualPath::new("/projects").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.clone()),
            )
            .expect("mount storage root");
        filesystem
            .mount_local(
                VirtualPath::new("/system/extensions").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.join("system/extensions")),
            )
            .expect("mount system extensions");
        let filesystem = Arc::new(filesystem);
        let root_filesystem: Arc<dyn RootFilesystem> = filesystem.clone();
        let active_registry = Arc::new(SharedExtensionRegistry::new(initial_active_registry));
        let trust_policy = test_extension_trust_policy();
        let failing_store = Arc::new(failing_store);
        let installation_store: Arc<dyn ExtensionInstallationStorePort> = failing_store.clone();
        let port = ExtensionManagementPort::new(
            root_filesystem,
            AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
            installation_store,
            Arc::new(Mutex::new(lifecycle_service)),
            test_active_extension_publisher(
                Arc::clone(&active_registry),
                Arc::clone(&trust_policy),
            ),
            None,
        );
        (dir, port, active_registry, failing_store, trust_policy)
    }

    fn extension_port_with_file_delete_failing_filesystem() -> (
        tempfile::TempDir,
        ExtensionManagementPort,
        Arc<SharedExtensionRegistry>,
        Arc<ExtensionInstallationStore>,
        Arc<HostTrustPolicy>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");
        let mut filesystem = DiskFilesystem::new();
        filesystem
            .mount_local(
                VirtualPath::new("/projects").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.clone()),
            )
            .expect("mount storage root");
        filesystem
            .mount_local(
                VirtualPath::new("/system/extensions").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.join("system/extensions")),
            )
            .expect("mount system extensions");
        let root_filesystem: Arc<dyn RootFilesystem> = Arc::new(
            FaultInjecting::new(filesystem)
                .with_fault(Fault::on(FilesystemOperation::Delete).backend("delete failed")),
        );
        let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let trust_policy = test_extension_trust_policy();
        let installation_store = Arc::new(filesystem_installation_store());
        let extension_installation_store: Arc<dyn ExtensionInstallationStorePort> =
            installation_store.clone();
        let port = ExtensionManagementPort::new(
            root_filesystem,
            AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
            extension_installation_store,
            Arc::new(Mutex::new(ExtensionLifecycleService::new(
                ExtensionRegistry::new(),
            ))),
            test_active_extension_publisher(
                Arc::clone(&active_registry),
                Arc::clone(&trust_policy),
            ),
            None,
        );
        (dir, port, active_registry, installation_store, trust_policy)
    }

    struct FailingRemoveLifecycleSink;

    #[async_trait]
    impl ExtensionLifecycleEventSink for FailingRemoveLifecycleSink {
        async fn record_extension_lifecycle_event(
            &self,
            event: ExtensionLifecycleEvent,
        ) -> Result<(), ExtensionError> {
            if event.operation == ExtensionLifecycleOperation::Remove {
                return Err(ExtensionError::LifecycleEventSink {
                    extension_id: event.extension_id,
                    operation: event.operation,
                });
            }
            Ok(())
        }
    }

    struct DeleteInstallationFailingStore {
        inner: ExtensionInstallationStore,
        fail_manifest_delete: bool,
        fail_get_installation: bool,
        mismatched_get_installation: bool,
        /// #5459 P1: fail the NEXT `upsert_installation` once, then clear —
        /// simulates a mid-install persist failure so the retry can heal.
        fail_next_upsert_installation: std::sync::atomic::AtomicBool,
    }

    impl Default for DeleteInstallationFailingStore {
        fn default() -> Self {
            Self {
                inner: filesystem_installation_store(),
                fail_manifest_delete: false,
                fail_get_installation: false,
                mismatched_get_installation: false,
                fail_next_upsert_installation: std::sync::atomic::AtomicBool::new(false),
            }
        }
    }

    impl DeleteInstallationFailingStore {
        fn fail_manifest_delete() -> Self {
            Self {
                fail_manifest_delete: true,
                ..Self::default()
            }
        }

        fn fail_get_installation() -> Self {
            Self {
                fail_get_installation: true,
                ..Self::default()
            }
        }

        fn mismatched_get_installation() -> Self {
            Self {
                mismatched_get_installation: true,
                ..Self::default()
            }
        }
    }

    #[async_trait]
    impl ExtensionInstallationStorePort for DeleteInstallationFailingStore {
        async fn list_manifests(
            &self,
        ) -> Result<Vec<ExtensionManifestRecord>, ExtensionInstallationError> {
            self.inner.list_manifests().await
        }

        async fn get_manifest(
            &self,
            extension_id: &ExtensionId,
        ) -> Result<Option<ExtensionManifestRecord>, ExtensionInstallationError> {
            self.inner.get_manifest(extension_id).await
        }

        async fn upsert_manifest(
            &self,
            manifest: ExtensionManifestRecord,
        ) -> Result<(), ExtensionInstallationError> {
            self.inner.upsert_manifest(manifest).await
        }

        async fn upsert_manifest_and_installation(
            &self,
            manifest: ExtensionManifestRecord,
            installation: ExtensionInstallation,
        ) -> Result<(), ExtensionInstallationError> {
            self.inner
                .upsert_manifest_and_installation(manifest, installation)
                .await
        }

        async fn list_installations(
            &self,
        ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
            self.inner.list_installations().await
        }

        async fn get_installation(
            &self,
            installation_id: &ExtensionInstallationId,
        ) -> Result<Option<ExtensionInstallation>, ExtensionInstallationError> {
            if self.fail_get_installation {
                return Err(ExtensionInstallationError::InvalidInstallation {
                    reason: "get installation failed".to_string(),
                });
            }
            if self.mismatched_get_installation {
                let extension_id = ExtensionId::new("other-fixture").expect("valid extension id");
                let installation = ExtensionInstallation::new(
                    installation_id.clone(),
                    extension_id.clone(),
                    ExtensionManifestRef::new(extension_id, None),
                    Vec::new(),
                    chrono::Utc::now(),
                    InstallationOwner::Tenant,
                )
                .expect("mismatched installation fixture");
                return Ok(Some(installation));
            }
            self.inner.get_installation(installation_id).await
        }

        async fn upsert_installation(
            &self,
            installation: ExtensionInstallation,
        ) -> Result<(), ExtensionInstallationError> {
            if self
                .fail_next_upsert_installation
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(ExtensionInstallationError::InvalidInstallation {
                    reason: "upsert installation failed".to_string(),
                });
            }
            self.inner.upsert_installation(installation).await
        }

        async fn delete_installation(
            &self,
            installation_id: &ExtensionInstallationId,
        ) -> Result<(), ExtensionInstallationError> {
            if self.fail_manifest_delete {
                self.inner.delete_installation(installation_id).await
            } else {
                Err(ExtensionInstallationError::InvalidInstallation {
                    reason: "delete installation failed".to_string(),
                })
            }
        }

        async fn delete_manifest(
            &self,
            extension_id: &ExtensionId,
        ) -> Result<(), ExtensionInstallationError> {
            if self.fail_manifest_delete {
                Err(ExtensionInstallationError::InvalidInstallation {
                    reason: "delete manifest failed".to_string(),
                })
            } else {
                self.inner.delete_manifest(extension_id).await
            }
        }

        async fn update_health(
            &self,
            installation_id: &ExtensionInstallationId,
            health: ironclaw_extensions::ExtensionHealthSnapshot,
        ) -> Result<(), ExtensionInstallationError> {
            self.inner.update_health(installation_id, health).await
        }
    }

    async fn assert_installed_runtime_ready<S>(
        active_registry: &SharedExtensionRegistry,
        installation_store: &S,
    ) where
        S: ExtensionInstallationStorePort + ?Sized,
    {
        let extension_id = ExtensionId::new("fixture").expect("valid extension id");
        let installation_id = ExtensionInstallationId::new("fixture").expect("valid installation");
        let installation = installation_store
            .get_installation(&installation_id)
            .await
            .expect("read installation")
            .expect("installation remains");
        assert!(installation.owner().visible_to(&lifecycle_owner()));
        assert!(
            active_registry
                .snapshot()
                .get_extension(&extension_id)
                .is_some()
        );
    }

    fn hosted_mcp_scope(user_id: &str) -> ResourceScope {
        ResourceScope::local_default(
            UserId::new(user_id).expect("valid user"),
            InvocationId::new(),
        )
        .expect("valid local scope")
    }

    struct FailsSecondCredentialGate {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ExtensionActivationCredentialGate for FailsSecondCredentialGate {
        async fn ensure_credentials(
            &self,
            _package: &ExtensionPackage,
        ) -> Result<(), ProductWorkflowError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return Ok(());
            }
            Err(ProductWorkflowError::InvalidBindingRequest {
                reason: "post-discovery credential recheck failed".to_string(),
            })
        }
    }

    struct EmptyToolsHostedMcpEgress;

    #[async_trait]
    impl RuntimeHttpEgress for EmptyToolsHostedMcpEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            hosted_mcp_response_for_request(request, serde_json::json!({ "tools": [] })).await
        }
    }

    /// Discovery whose staged credentials are rejected mid-`tools/list`: the
    /// `initialize`/`notifications/initialized` handshake succeeds, then
    /// `tools/list` answers HTTP 401 (token expired/revoked after the
    /// pre-discovery credential check). Exercises the `AuthRequired` re-auth
    /// routing.
    struct AuthRejectedDiscoveryHostedMcpEgress;

    #[async_trait]
    impl RuntimeHttpEgress for AuthRejectedDiscoveryHostedMcpEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            let request_bytes = request.body.len() as u64;
            let body = parse_test_json_rpc_body(&request)?;
            let method = body
                .get("method")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| RuntimeHttpEgressError::Request {
                    reason: "missing_json_rpc_method".to_string(),
                    request_bytes,
                    response_bytes: 0,
                })?;
            match method {
                "initialize" => test_runtime_json_response(
                    body["id"].as_u64(),
                    serde_json::json!({
                        "protocolVersion": "2024-11-05",
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "notion-test", "version": "1.0.0"}
                    }),
                    vec![("Mcp-Session-Id".to_string(), "session-1".to_string())],
                ),
                "notifications/initialized" => {
                    test_runtime_json_response(None, serde_json::json!({}), Vec::new())
                }
                "tools/list" => Ok(RuntimeHttpEgressResponse {
                    status: 401,
                    headers: vec![("content-type".to_string(), "application/json".to_string())],
                    body: br#"{"error":"invalid_token"}"#.to_vec(),
                    saved_body: None,
                    request_bytes,
                    response_bytes: 25,
                    redaction_applied: false,
                }),
                _ => Err(RuntimeHttpEgressError::Request {
                    reason: "unexpected_method".to_string(),
                    request_bytes,
                    response_bytes: 0,
                }),
            }
        }
    }

    struct MalformedToolsHostedMcpEgress;

    #[async_trait]
    impl RuntimeHttpEgress for MalformedToolsHostedMcpEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            hosted_mcp_response_for_request(
                request,
                serde_json::json!({
                    "tools": [{
                        "name": "unsupported tool name",
                        "description": "invalid capability suffix",
                        "inputSchema": {"type": "object"}
                    }]
                }),
            )
            .await
        }
    }

    struct SecondToolsListResultHostedMcpEgress {
        tools_list_calls: AtomicUsize,
        second_result: serde_json::Value,
    }

    impl SecondToolsListResultHostedMcpEgress {
        fn new(second_result: serde_json::Value) -> Self {
            Self {
                tools_list_calls: AtomicUsize::new(0),
                second_result,
            }
        }
    }

    #[async_trait]
    impl RuntimeHttpEgress for SecondToolsListResultHostedMcpEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            let request_bytes = request.body.len() as u64;
            let body = parse_test_json_rpc_body(&request)?;
            let result = if body.get("method").and_then(serde_json::Value::as_str)
                == Some("tools/list")
                && self.tools_list_calls.fetch_add(1, Ordering::SeqCst) > 0
            {
                self.second_result.clone()
            } else {
                discovered_tools_payload()
            };
            hosted_mcp_response_for_body(body, request_bytes, result)
        }
    }

    #[derive(Default)]
    struct FailsSecondToolsListHostedMcpEgress {
        tools_list_calls: AtomicUsize,
    }

    #[async_trait]
    impl RuntimeHttpEgress for FailsSecondToolsListHostedMcpEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            let request_bytes = request.body.len() as u64;
            let body = parse_test_json_rpc_body(&request)?;
            if body.get("method").and_then(serde_json::Value::as_str) == Some("tools/list")
                && self.tools_list_calls.fetch_add(1, Ordering::SeqCst) > 0
            {
                return Err(RuntimeHttpEgressError::Request {
                    reason: "refresh_tools_list_failed".to_string(),
                    request_bytes,
                    response_bytes: 0,
                });
            }
            hosted_mcp_response_for_body(body, request_bytes, discovered_tools_payload())
        }
    }

    struct BlockingToolsListHostedMcpEgress {
        started: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: tokio::sync::Mutex<tokio::sync::oneshot::Receiver<()>>,
    }

    impl BlockingToolsListHostedMcpEgress {
        fn new() -> (
            Arc<Self>,
            tokio::sync::oneshot::Receiver<()>,
            tokio::sync::oneshot::Sender<()>,
        ) {
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            (
                Arc::new(Self {
                    started: std::sync::Mutex::new(Some(started_tx)),
                    release: tokio::sync::Mutex::new(release_rx),
                }),
                started_rx,
                release_tx,
            )
        }
    }

    #[async_trait]
    impl RuntimeHttpEgress for BlockingToolsListHostedMcpEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            let body = parse_test_json_rpc_body(&request)?;
            if body.get("method").and_then(serde_json::Value::as_str) == Some("tools/list") {
                if let Some(started) = self
                    .started
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                {
                    #[allow(clippy::let_underscore_must_use)]
                    // oneshot notify; dropped receiver is expected
                    let _ = started.send(());
                }
                let mut release = self.release.lock().await;
                #[allow(clippy::let_underscore_must_use)] // gate await; result intentionally unused
                let _ = (&mut *release).await;
            }
            hosted_mcp_response_for_body(
                body,
                request.body.len() as u64,
                discovered_tools_payload(),
            )
        }
    }

    async fn hosted_mcp_response_for_request(
        request: RuntimeHttpEgressRequest,
        tools_list_result: serde_json::Value,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let request_bytes = request.body.len() as u64;
        let body = parse_test_json_rpc_body(&request)?;
        hosted_mcp_response_for_body(body, request_bytes, tools_list_result)
    }

    fn parse_test_json_rpc_body(
        request: &RuntimeHttpEgressRequest,
    ) -> Result<serde_json::Value, RuntimeHttpEgressError> {
        if request.method != NetworkMethod::Post {
            return Err(RuntimeHttpEgressError::Request {
                reason: "unexpected_method".to_string(),
                request_bytes: request.body.len() as u64,
                response_bytes: 0,
            });
        }
        serde_json::from_slice(&request.body).map_err(|_| RuntimeHttpEgressError::Request {
            reason: "invalid_json_rpc_body".to_string(),
            request_bytes: request.body.len() as u64,
            response_bytes: 0,
        })
    }

    fn hosted_mcp_response_for_body(
        body: serde_json::Value,
        request_bytes: u64,
        tools_list_result: serde_json::Value,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let method = body
            .get("method")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| RuntimeHttpEgressError::Request {
                reason: "missing_json_rpc_method".to_string(),
                request_bytes,
                response_bytes: 0,
            })?;
        match method {
            "initialize" => test_runtime_json_response(
                body["id"].as_u64(),
                serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "notion-test", "version": "1.0.0"}
                }),
                vec![("Mcp-Session-Id".to_string(), "session-1".to_string())],
            ),
            "notifications/initialized" => {
                test_runtime_json_response(None, serde_json::json!({}), Vec::new())
            }
            "tools/list" => {
                test_runtime_json_response(body["id"].as_u64(), tools_list_result, Vec::new())
            }
            _ => Err(RuntimeHttpEgressError::Request {
                reason: "unexpected_method".to_string(),
                request_bytes,
                response_bytes: 0,
            }),
        }
    }

    fn discovered_tools_payload() -> serde_json::Value {
        serde_json::json!({
            "tools": [
                {
                    "name": "live-search",
                    "description": "Search live Notion content",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"query": {"type": "string"}},
                        "required": ["query"]
                    }
                }
            ]
        })
    }

    fn test_runtime_json_response(
        id: Option<u64>,
        result: serde_json::Value,
        extra_headers: Vec<(String, String)>,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let mut headers = vec![("content-type".to_string(), "application/json".to_string())];
        headers.extend(extra_headers);
        let body = serde_json::to_vec(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }))
        .expect("serialize test JSON-RPC response");
        Ok(RuntimeHttpEgressResponse {
            status: 200,
            headers,
            response_bytes: body.len() as u64,
            body,
            saved_body: None,
            request_bytes: 0,
            redaction_applied: false,
        })
    }

    struct MissingRuntimeCredentialAccounts;

    #[async_trait]
    impl crate::product_auth::credentials::runtime_credentials::RuntimeCredentialAccountSelectionService
        for MissingRuntimeCredentialAccounts
    {
        async fn select_configured_account_for_binding(
            &self,
            _lookup: ironclaw_auth::CredentialAccountSelectionRequest,
            _runtime_scope: ironclaw_auth::AuthProductScope,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            Err(ironclaw_auth::AuthProductError::CredentialMissing)
        }

        async fn select_unique_configured_runtime_account(
            &self,
            _request: crate::product_auth::credentials::runtime_credentials::RuntimeCredentialAccountSelectionRequest,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            Err(ironclaw_auth::AuthProductError::CredentialMissing)
        }
    }

    struct ConfiguredRuntimeCredentialAccounts;

    #[async_trait]
    impl crate::product_auth::credentials::runtime_credentials::RuntimeCredentialAccountSelectionService
        for ConfiguredRuntimeCredentialAccounts
    {
        async fn select_configured_account_for_binding(
            &self,
            _lookup: ironclaw_auth::CredentialAccountSelectionRequest,
            _runtime_scope: ironclaw_auth::AuthProductScope,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            Err(ironclaw_auth::AuthProductError::CredentialMissing)
        }

        async fn select_unique_configured_runtime_account(
            &self,
            _request: crate::product_auth::credentials::runtime_credentials::RuntimeCredentialAccountSelectionRequest,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            let epoch = chrono::DateTime::parse_from_rfc3339("2026-07-22T00:00:00Z")
                .expect("valid fixed credential epoch")
                .with_timezone(&chrono::Utc);
            Ok(configured_runtime_credential_account(epoch))
        }
    }

    struct ChangingRuntimeCredentialAccounts {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl crate::product_auth::credentials::runtime_credentials::RuntimeCredentialAccountSelectionService
        for ChangingRuntimeCredentialAccounts
    {
        async fn select_configured_account_for_binding(
            &self,
            _lookup: ironclaw_auth::CredentialAccountSelectionRequest,
            _runtime_scope: ironclaw_auth::AuthProductScope,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            Err(ironclaw_auth::AuthProductError::CredentialMissing)
        }

        async fn select_unique_configured_runtime_account(
            &self,
            _request: crate::product_auth::credentials::runtime_credentials::RuntimeCredentialAccountSelectionRequest,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let base = chrono::DateTime::parse_from_rfc3339("2026-07-22T00:00:00Z")
                .expect("valid fixed credential epoch")
                .with_timezone(&chrono::Utc);
            // The discovery authority fence deliberately tolerates a benign
            // timestamp bump, so an epoch change must rotate a real authority
            // input — here the access-secret handle — to still discard a
            // catalog discovered under the superseded credential.
            let mut account = configured_runtime_credential_account(
                base + chrono::Duration::seconds(call as i64),
            );
            account.access_secret = Some(
                ironclaw_host_api::SecretHandle::new(format!("test-secret-epoch-{call}"))
                    .expect("valid secret handle"),
            );
            Ok(account)
        }
    }

    fn configured_runtime_credential_account(
        epoch: chrono::DateTime<chrono::Utc>,
    ) -> ironclaw_auth::CredentialAccount {
        ironclaw_auth::CredentialAccount {
            id: ironclaw_auth::CredentialAccountId::from_uuid(uuid::Uuid::nil()),
            scope: ironclaw_auth::AuthProductScope::new(
                ResourceScope::local_default(
                    UserId::new("credential-user").expect("valid user"),
                    InvocationId::from_uuid(uuid::Uuid::nil()),
                )
                .expect("valid scope"),
                ironclaw_auth::AuthSurface::Api,
            ),
            provider: ironclaw_auth::AuthProviderId::new("test-provider").expect("valid provider"),
            label: ironclaw_auth::CredentialAccountLabel::new("test-provider")
                .expect("valid label"),
            status: ironclaw_auth::CredentialAccountStatus::Configured,
            ownership: ironclaw_auth::CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(
                ironclaw_host_api::SecretHandle::new("test-secret").expect("valid secret handle"),
            ),
            refresh_secret: None,
            scopes: Vec::new(),
            provider_identity: None,
            created_at: epoch,
            updated_at: epoch,
        }
    }

    struct BackendUnavailableRuntimeCredentialAccounts;

    #[async_trait]
    impl crate::product_auth::credentials::runtime_credentials::RuntimeCredentialAccountSelectionService
        for BackendUnavailableRuntimeCredentialAccounts
    {
        async fn select_configured_account_for_binding(
            &self,
            _lookup: ironclaw_auth::CredentialAccountSelectionRequest,
            _runtime_scope: ironclaw_auth::AuthProductScope,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            Err(ironclaw_auth::AuthProductError::BackendUnavailable)
        }

        async fn select_unique_configured_runtime_account(
            &self,
            _request: crate::product_auth::credentials::runtime_credentials::RuntimeCredentialAccountSelectionRequest,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            Err(ironclaw_auth::AuthProductError::BackendUnavailable)
        }
    }

    fn lifecycle_surface_context() -> LifecycleProductContext {
        lifecycle_surface_context_for_user("lifecycle-owner")
    }

    /// Surface context for an arbitrary member user (#5459 P1 tests). The
    /// fixture wires `lifecycle-owner` as the tenant operator, so any other
    /// user id here acts as a plain member whose installs derive `User(..)`.
    fn lifecycle_surface_context_for_user(user: &str) -> LifecycleProductContext {
        LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
            tenant_id: TenantId::new("lifecycle-tenant").expect("valid tenant"),
            user_id: UserId::new(user).expect("valid user"),
            agent_id: None,
            project_id: None,
        })
    }

    /// The fixture's tenant-operator identity — matches the operator user id
    /// wired into every test `ExtensionManagementPort`.
    fn lifecycle_owner() -> UserId {
        UserId::new("lifecycle-owner").expect("valid user")
    }

    fn test_extension_trust_policy() -> Arc<HostTrustPolicy> {
        Arc::new(
            HostTrustPolicy::new(vec![Box::new(ironclaw_trust::AdminConfig::new())])
                .expect("test trust policy"),
        )
    }

    fn test_active_extension_publisher(
        active_registry: Arc<SharedExtensionRegistry>,
        trust_policy: Arc<HostTrustPolicy>,
    ) -> ActiveExtensionPublisher {
        ActiveExtensionPublisher::new(
            active_registry,
            trust_policy,
            Arc::new(InvalidationBus::new()),
        )
    }

    fn fixture_extension_package() -> AvailableExtensionPackage {
        fixture_extension_package_from_manifest(fixture_extension_manifest())
    }

    fn fixture_extension_package_with_description(description: &str) -> AvailableExtensionPackage {
        let manifest = fixture_extension_manifest().replace(
            "description = \"Lifecycle fixture extension\"",
            &format!("description = \"{description}\""),
        );
        fixture_extension_package_from_manifest(&manifest)
    }

    fn fixture_external_channel_package(id: &str, name: &str) -> AvailableExtensionPackage {
        let manifest = format!(
            r#"
schema_version = "reborn.extension_manifest.v2"
id = "{id}"
name = "{name}"
version = "0.1.0"
description = "{name} channel fixture"
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "{id}_host"

[[host_api]]
id = "ironclaw.product_adapter/v1"
section = "product_adapter.inbound"

[product_adapter.inbound]
surface_kind = "external_channel"

[product_adapter.inbound.auth]
kind = "request_signature"
header_name = "X-Channel-Signature"
timestamp_header_name = "X-Channel-Timestamp"

[product_adapter.inbound.capabilities]
flags = ["inbound_messages"]

[[product_adapter.inbound.required_credentials]]
handle = "{id}_bot_token"

[[product_adapter.inbound.egress]]
host = "example.com"
credential_handle = "{id}_bot_token"
"#
        );
        let mut package =
            fixture_extension_package_from_manifest_with_product_adapter_contracts(&manifest, id);
        package.surface_kinds = vec![CapabilitySurfaceKind::Channel];
        package
    }

    fn fixture_external_channel_package_with_cleanup(
        id: &str,
        name: &str,
        requirement: ExtensionRemovalCleanupRequirement,
    ) -> AvailableExtensionPackage {
        let mut package = fixture_external_channel_package(id, name);
        package.cleanup_requirements = vec![requirement];
        package
    }

    fn fixture_github_package_with_cleanup(
        requirement: ExtensionRemovalCleanupRequirement,
    ) -> AvailableExtensionPackage {
        let manifest = r#"
schema_version = "reborn.extension_manifest.v2"
id = "github"
name = "GitHub"
version = "0.1.0"
description = "GitHub cleanup fixture"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/github.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "github.read"
description = "Read GitHub data"
effects = ["network", "use_secret"]
runtime_credentials = [
  { handle = "github_runtime_token", source = { type = "product_auth_account", provider = "github" }, audience = { scheme = "https", host_pattern = "api.github.com" }, target = { type = "header", name = "authorization", prefix = "Bearer " } },
]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/read.input.json"
output_schema_ref = "schemas/read.output.json"
"#;
        let mut package = fixture_extension_package_from_manifest_with_root(manifest, "github");
        package.cleanup_requirements = vec![requirement];
        package
    }

    fn removal_cleanup_requirement(
        adapter_id: &str,
        channel: &str,
    ) -> ExtensionRemovalCleanupRequirement {
        ExtensionRemovalCleanupRequirement::channel_connection(
            ExtensionRemovalCleanupAdapterId::new(adapter_id).expect("valid cleanup adapter id"),
            ExtensionRemovalChannelId::new(channel).expect("valid cleanup channel id"),
        )
    }

    fn fixture_extension_manifest() -> &'static str {
        r#"
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

[[capability_provider.tools.capabilities]]
id = "fixture.write"
description = "Write fixture data"
effects = ["network", "external_write"]
default_permission = "ask"
visibility = "host_internal"
input_schema_ref = "schemas/write.input.json"
output_schema_ref = "schemas/write.output.json"
"#
    }

    fn fixture_installed_local_manifest() -> &'static str {
        r#"
schema_version = "reborn.extension_manifest.v2"
id = "fixture"
name = "Fixture Extension"
version = "0.1.0"
description = "Installed local fixture extension"
trust = "third_party"

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
"#
    }

    /// Manifest for an installation row persisted with an extension id the
    /// [`AvailableExtensionCatalog`] does not materialize a package for —
    /// mirrors the placeholder rows the standalone v1->Reborn migration tool
    /// writes ahead of catalog package materialization (#5459 review).
    fn orphan_migrated_manifest() -> String {
        r#"
schema_version = "reborn.extension_manifest.v2"
id = "orphan_migrated"
name = "Orphan Migrated Extension"
version = "0.1.0"
description = "Placeholder row from the v1->Reborn migration tool"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/orphan_migrated.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "orphan_migrated.search"
description = "Search orphan migrated data"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"
"#
        .to_string()
    }

    fn fixture_extension_package_from_manifest(manifest_toml: &str) -> AvailableExtensionPackage {
        fixture_extension_package_from_manifest_with_root(manifest_toml, "fixture")
    }

    fn fixture_extension_package_from_manifest_with_root(
        manifest_toml: &str,
        root_id: &str,
    ) -> AvailableExtensionPackage {
        let contracts = capability_provider_contracts();
        let manifest = ExtensionManifest::parse(
            manifest_toml,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
            &contracts,
        )
        .expect("fixture manifest");
        let resolved_manifest = Arc::new(
            ExtensionManifestRecord::from_toml(
                manifest_toml,
                ManifestSource::HostBundled,
                &HostPortCatalog::empty(),
                None,
                &contracts,
            )
            .expect("resolved fixture manifest")
            .resolved()
            .clone(),
        );
        fixture_extension_package_from_parsed_manifest(
            manifest_toml,
            root_id,
            manifest,
            resolved_manifest,
        )
    }

    fn fixture_extension_package_from_manifest_with_product_adapter_contracts(
        manifest_toml: &str,
        root_id: &str,
    ) -> AvailableExtensionPackage {
        let mut contracts = ironclaw_extensions::HostApiContractRegistry::new();
        contracts
            .register(Arc::new(
                ironclaw_product::adapter_registry::ProductAdapterHostApiContract::new()
                    .expect("product adapter host API contract"),
            ))
            .expect("register product adapter host API contract");
        let manifest = ExtensionManifest::parse(
            manifest_toml,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
            &contracts,
        )
        .expect("fixture manifest");
        let resolved_manifest = Arc::new(
            ExtensionManifestRecord::from_toml(
                manifest_toml,
                ManifestSource::HostBundled,
                &HostPortCatalog::empty(),
                None,
                &contracts,
            )
            .expect("resolved fixture manifest")
            .resolved()
            .clone(),
        );
        fixture_extension_package_from_parsed_manifest(
            manifest_toml,
            root_id,
            manifest,
            resolved_manifest,
        )
    }

    fn fixture_extension_package_from_parsed_manifest(
        manifest_toml: &str,
        root_id: &str,
        manifest: ExtensionManifest,
        resolved_manifest: Arc<ironclaw_extensions::ResolvedExtensionManifest>,
    ) -> AvailableExtensionPackage {
        let root =
            VirtualPath::new(format!("/system/extensions/{root_id}")).expect("extension root");
        let package = ExtensionPackage::from_manifest_toml(manifest, root, manifest_toml)
            .expect("fixture package");
        AvailableExtensionPackage {
            package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, root_id)
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

    fn fixture_manifest_record_with_source(
        manifest_toml: &str,
        source: ManifestSource,
        manifest_hash: Option<String>,
    ) -> ExtensionManifestRecord {
        let host_ports =
            ironclaw_host_runtime::default_host_port_catalog().expect("host port catalog");
        let contracts = product_extension_host_api_contract_registry().expect("host API contracts");
        ExtensionManifestRecord::from_toml(
            manifest_toml,
            source,
            &host_ports,
            manifest_hash
                .map(ManifestHash::new)
                .transpose()
                .expect("valid manifest hash"),
            &contracts,
        )
        .expect("fixture manifest record")
    }

    fn fixture_installation(manifest_hash: Option<String>) -> ExtensionInstallation {
        let extension_id = ExtensionId::new("fixture").expect("valid extension id");
        ExtensionInstallation::new(
            ExtensionInstallationId::new("fixture").expect("valid installation"),
            extension_id.clone(),
            ExtensionManifestRef::new(
                extension_id,
                manifest_hash
                    .map(ManifestHash::new)
                    .transpose()
                    .expect("valid manifest hash"),
            ),
            Vec::new(),
            chrono::Utc::now(),
            InstallationOwner::Tenant,
        )
        .expect("fixture installation")
    }
}
