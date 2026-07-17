// arch-exempt: large_file, shared extension removal convergence and compatibility tests, plan #5905
use std::{collections::BTreeSet, sync::Arc};

use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, SecretCleanupAction, SecretCleanupReport,
    SecretCleanupRequest,
};
use ironclaw_extensions::{
    CapabilityVisibility, ExtensionActivationState, ExtensionError, ExtensionInstallation,
    ExtensionInstallationError, ExtensionInstallationId, ExtensionInstallationStore,
    ExtensionLifecycleService, ExtensionManifestRecord, ExtensionManifestRef, ExtensionPackage,
    InstallationOwner, ManifestHash, ManifestSource,
};
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, EffectKind, ExtensionId, NetworkTargetPattern,
    PermissionMode, ResourceScope, RuntimeCredentialAuthRequirement, RuntimeCredentialRequirement,
    RuntimeHttpEgress, UserId, VirtualPath, sha256_digest_token,
};
use ironclaw_product_adapter_registry::PRODUCT_ADAPTER_HOST_API_ID;
use ironclaw_product_workflow::{
    ChannelConnectionRequirement, LifecycleBlockerRef, LifecycleExtensionSummary,
    LifecycleExtensionSurfaceKind, LifecycleInstalledExtensionSummary, LifecyclePackageKind,
    LifecyclePackageRef, LifecyclePhase, LifecycleProductPayload, LifecycleProductResponse,
    LifecycleReadinessBlocker, LifecycleSearchExtensionSummary, ProductWorkflowError,
    RebornChannelConnectStrategy, RebornServicesError,
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
    ) -> Result<SecretCleanupReport, RebornServicesError>;
}

#[async_trait]
impl ExtensionCredentialCleanup for RebornProductAuthServices {
    async fn cleanup_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, RebornServicesError> {
        RebornProductAuthServices::cleanup_credentials_for_lifecycle(self, request)
            .await
            .map_err(|error| {
                RebornServicesError::internal_from(format!(
                    "extension credential cleanup failed: {:?}",
                    error.code
                ))
            })
    }
}

mod active_publication;
#[cfg(test)]
mod hosted_mcp_test_support;
mod install_policy;

use crate::extension_host::available_extensions::{
    AvailableExtensionCatalog, AvailableExtensionPackage, imported_extension_package,
    is_internal_extension_package_ref, materialize_available_extension, visible_capability_ids,
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

pub(crate) use active_publication::ActiveExtensionPublisher;
#[cfg(test)]
use active_publication::extension_trust_policy_input;
use install_policy::{
    RemoveDecision, decide_install_on_existing, decide_remove, derive_owner,
    ensure_caller_may_operate, install_scope_for_owner,
};

const RETIRED_SLACK_USER_EXTENSION_ID: &str = "slack_user";

// This port is deliberately scoped to LocalSingleUser composition. The
// lifecycle service models the installed extension set, while active_registry
// is the model-visible capability surface read by host runtime dispatch.
// install/remove keep the lifecycle set durable; activate/remove are the only
// local-dev writers that should mirror lifecycle-managed packages into or out
// of active_registry. Production and multi-tenant reuse require scoped storage
// and registry ownership first; tracked in #4091.
pub(crate) struct RebornLocalExtensionManagementPort {
    filesystem: Arc<dyn RootFilesystem>,
    catalog: Arc<RwLock<AvailableExtensionCatalog>>,
    installation_store: Arc<dyn ExtensionInstallationStore>,
    lifecycle_service: Arc<Mutex<ExtensionLifecycleService>>,
    active_extensions: ActiveExtensionPublisher,
    operation_lock: Arc<Mutex<()>>,
    // Genuinely optional (not an `optional_arc` smell): a composition without
    // product auth cannot have minted a reusable OAuth credential, so there is
    // nothing to revoke on removal.
    credential_cleanup: Option<Arc<dyn ExtensionCredentialCleanup>>,
    /// Bounds concurrent zip decode/validation in `import_bundle`. Each decode
    /// may expand up to [`crate::extension_host::extension_bundle::MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES`] into
    /// memory, so without a bound N concurrent operator uploads turn the
    /// per-request cap into N x 64 MiB of pressure before any lifecycle lock
    /// applies (#5499 review finding #3).
    import_decode_semaphore: Arc<Semaphore>,
    /// The tenant operator identity (#5459 P1). In local-dev this is the base
    /// owner user (`IRONCLAW_REBORN_WEBUI_USER_ID` semantics); installs by this
    /// user derive [`InstallationOwner::Tenant`] (shared), installs by anyone
    /// else make (or join) the member set [`InstallationOwner::Users`].
    /// Resolved ONCE here — when P0 role wiring lands, this becomes a
    /// role-derived resolver instead of an identity comparison; callers do
    /// not re-derive admin-ness.
    tenant_operator_user_id: UserId,
    removal_cleanup: Arc<ExtensionRemovalCleanupRegistry>,
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
    /// Who the providing extension's installation belongs to (#5459 P1).
    /// Tenant-owned capabilities are grant-minted for every user; user-owned
    /// ones only for their owner (filtered in `LocalDevExtensionSurface`).
    pub(crate) owner: InstallationOwner,
}

#[derive(Clone)]
pub(crate) enum ExtensionActivationMode {
    Static,
    HostedMcpDiscovery {
        scope: ResourceScope,
        runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    },
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
            owner,
        }
    }
}

impl ExtensionActivationMode {
    pub(crate) fn from_dispatch_context(
        scope: ResourceScope,
        runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
    ) -> Self {
        match runtime_http_egress {
            Some(runtime_http_egress) => Self::HostedMcpDiscovery {
                scope,
                runtime_http_egress,
            },
            None => Self::Static,
        }
    }
}

pub(crate) async fn restore_extension_lifecycle_state(
    catalog: &AvailableExtensionCatalog,
    filesystem: &Arc<dyn RootFilesystem>,
    installation_store: &Arc<dyn ExtensionInstallationStore>,
    lifecycle_service: &Arc<Mutex<ExtensionLifecycleService>>,
    active_extensions: &ActiveExtensionPublisher,
) -> Result<(), ProductWorkflowError> {
    for installation in installation_store
        .list_installations()
        .await
        .map_err(map_extension_installation_error)?
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
            match installation.activation_state() {
                ExtensionActivationState::Enabled => {
                    lifecycle
                        .enable(&available.package.id)
                        .await
                        .map_err(map_extension_error)?;
                }
                ExtensionActivationState::Installed | ExtensionActivationState::Disabled => {
                    lifecycle
                        .disable(&available.package.id)
                        .await
                        .map_err(map_extension_error)?;
                }
            }
        }
        if installation.activation_state() == ExtensionActivationState::Enabled {
            active_extensions.publish(&available.package)?;
        }
    }
    Ok(())
}

async fn remove_retired_internal_installation(
    installation_store: &Arc<dyn ExtensionInstallationStore>,
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

impl RebornLocalExtensionManagementPort {
    pub(crate) fn new(
        filesystem: Arc<dyn RootFilesystem>,
        catalog: AvailableExtensionCatalog,
        installation_store: Arc<dyn ExtensionInstallationStore>,
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
            import_decode_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_IMPORT_DECODES)),
            tenant_operator_user_id,
            removal_cleanup: Arc::new(ExtensionRemovalCleanupRegistry::empty()),
        }
    }

    pub(crate) fn with_removal_cleanup_registry(
        mut self,
        removal_cleanup: Arc<ExtensionRemovalCleanupRegistry>,
    ) -> Self {
        self.removal_cleanup = removal_cleanup;
        self
    }

    /// Test-support access to the extension installation store.
    ///
    /// Mirrors the `installation_store` field that `build_local_runtime` wires
    /// in when constructing `RebornLocalExtensionManagementPort`. For tests
    /// only — zero bytes shipped in production builds.
    #[cfg(feature = "test-support")]
    pub(crate) fn installation_store_for_test(&self) -> Arc<dyn ExtensionInstallationStore> {
        Arc::clone(&self.installation_store)
    }

    /// C-JOURNEY: test-support access to the active-extension publisher
    /// (registry + trust policy). `activate()` ultimately delegates the
    /// model-visible-surface mutation to `self.active_extensions.publish(..)`
    /// (see `active_publication.rs`) after its own install/credential-gate
    /// bookkeeping; this accessor reaches that SAME publish step directly so a
    /// test harness can make a bundled first-party WASM package (e.g. github)
    /// genuinely dispatchable without driving the full multi-turn
    /// install→activate capability handshake through the model. For tests
    /// only — zero bytes shipped in production builds.
    #[cfg(feature = "test-support")]
    pub(crate) fn active_extensions_for_test(&self) -> &ActiveExtensionPublisher {
        &self.active_extensions
    }

    /// The wired tenant-operator identity (#5459 P1). Used by tenant-wide
    /// host activations (e.g. Slack host-beta channel setup) that operate a
    /// shared install and therefore act as the operator rather than any
    /// individual member.
    #[cfg(feature = "slack-v2-host-beta")]
    pub(crate) fn tenant_operator_user_id(&self) -> &UserId {
        &self.tenant_operator_user_id
    }

    /// Test-support view of the wired tenant-operator identity (#5459 P1), so
    /// tests can act "as the operator" without re-deriving the id the runtime
    /// or fixture was built with. Mirrors the production owner wiring in
    /// `build_local_runtime`. Tests only — zero bytes in production builds.
    #[cfg(test)]
    pub(crate) fn tenant_operator_user_id_for_test(&self) -> &UserId {
        &self.tenant_operator_user_id
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
        let mut summaries = Vec::new();
        for extension in extensions {
            summaries.push(
                self.search_summary(&extension, credential_gate, caller)
                    .await?,
            );
        }
        let count = summaries.len();
        let mut response = response_with_payload(
            None,
            LifecyclePhase::Discovered,
            LifecycleProductPayload::ExtensionSearch {
                extensions: summaries,
                count,
            },
        );
        if extension_search_has_installed_external_channel_result(response.payload.as_ref()) {
            response.message = Some(
                "Search found installed external channel results. Search cannot prove the calling user's channel account is personally connected. For an explicit connect, pair, authenticate, or account-access request, call builtin.extension_activate for the matching extension id so channel-specific connection/setup instructions can be surfaced. For routine, trigger, or notification delivery, prefer the configured outbound delivery target when one is available; do not activate the channel just to send to an already configured delivery target."
                    .to_string(),
            );
        } else if extension_search_has_inactive_installed_result(response.payload.as_ref()) {
            response.message = Some(
                "Search found installed extension results that are not active yet. Report these as installed but not activated; configured only means required credentials appear present, not that tools are published. Any visible_capability_ids on inactive results are catalog capabilities only, not currently callable tools. To make the extension available, call builtin.extension_activate for the matching extension id."
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
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let summaries = self.installed_summaries(caller).await?;
        let count = summaries.len();
        Ok(response_with_payload(
            None,
            LifecyclePhase::Installed,
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
        let phase = installation
            .as_ref()
            .map(|installation| phase_for_activation_state(installation.activation_state()))
            .unwrap_or(LifecyclePhase::Discovered);
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

    pub(crate) async fn active_model_visible_capabilities(
        &self,
    ) -> Result<Vec<ActiveExtensionCapability>, ProductWorkflowError> {
        // #5459 P1: carry each enabled installation's owner onto its
        // capabilities so the per-request grant minting in the local-dev
        // capability surface can filter user-private extensions to their
        // owner. The registry itself stays global; owner is joined here.
        let owner_by_extension = project_installation_owners(
            self.installation_store
                .list_enabled_installations()
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
        let requirements = package_runtime_credential_auth_requirements(&package);
        Ok(requirements)
    }

    async fn installed_summaries(
        &self,
        caller: &UserId,
    ) -> Result<Vec<LifecycleInstalledExtensionSummary>, ProductWorkflowError> {
        let installations = self
            .installation_store
            .list_installations()
            .await
            .map_err(map_extension_installation_error)?;
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
            if is_internal_extension_package_ref(&package_ref) {
                continue;
            }
            let available = {
                let catalog = self.catalog.read().await;
                let Ok(available) = catalog.resolve(&package_ref) else {
                    continue;
                };
                available
            };
            summaries.push(LifecycleInstalledExtensionSummary {
                summary: available.summary(),
                phase: phase_for_activation_state(installation.activation_state()),
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
        let phase = search_installation_phase(extension, &installation, credential_gate).await?;
        Ok(LifecycleSearchExtensionSummary {
            summary,
            installation_phase: Some(phase),
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
        let package = tokio::task::spawn_blocking(move || {
            let files = unzip_extension_bundle(&bundle)?;
            imported_extension_package(files)
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
            LifecyclePhase::Discovered,
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
            // The id is already installed: membership decides whether the
            // caller JOINS the member set or the operator EVICTS it to
            // `Tenant` — either way a single row rewrite; the bundle is
            // already registered, materialized, and (if enabled) published,
            // so there is nothing to compensate.
            Some(existing) => {
                let new_owner = decide_install_on_existing(
                    &available.package.id,
                    existing.owner(),
                    caller,
                    &self.tenant_operator_user_id,
                )?;
                self.installation_store
                    .upsert_installation(existing.with_owner(new_owner))
                    .await
                    .map_err(map_extension_installation_error)?;
            }
            None => {
                self.install_fresh_locked(&available, caller).await?;
            }
        }

        Ok(response_with_payload(
            Some(package_ref.clone()),
            LifecyclePhase::Installed,
            LifecycleProductPayload::ExtensionInstall {
                installed: true,
                visible_capability_ids: visible_capability_ids(&available)
                    .map(|id| id.as_str().to_string())
                    .collect(),
                next_step: format!(
                    "Call builtin.extension_activate now with input {{\"extension_id\":\"{}\"}}. Activation publishes the tools and opens the auth gate if credentials are missing.",
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
                    "extension {} is already installed",
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

    pub(crate) async fn activate(
        &self,
        package_ref: LifecyclePackageRef,
        mode: ExtensionActivationMode,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let credential_gate = UnavailableExtensionActivationCredentialGate;
        self.activate_inner(package_ref, mode, &credential_gate, caller)
            .await
    }

    pub(crate) async fn activate_with_credential_gate(
        &self,
        package_ref: LifecyclePackageRef,
        mode: ExtensionActivationMode,
        credential_gate: impl ExtensionActivationCredentialGate,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        self.activate_inner(package_ref, mode, &credential_gate, caller)
            .await
    }

    #[cfg(test)]
    pub(crate) async fn activate_with_prechecked_credentials_for_test(
        &self,
        package_ref: LifecyclePackageRef,
        mode: ExtensionActivationMode,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let credential_gate =
            crate::extension_host::extension_activation_credentials::PrecheckedExtensionActivationCredentialGate;
        let caller = self.tenant_operator_user_id.clone();
        self.activate_inner(package_ref, mode, &credential_gate, &caller)
            .await
    }

    async fn activate_inner(
        &self,
        package_ref: LifecyclePackageRef,
        mode: ExtensionActivationMode,
        credential_gate: &dyn ExtensionActivationCredentialGate,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
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

        let active_package = match discover_hosted_mcp_package(
            &discovery.base_package,
            discovery.scope,
            discovery.runtime_http_egress,
        )
        .await
        {
            Ok(active_package) => active_package,
            Err(HostedMcpDiscoveryError::Transient(reason)) => {
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
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
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
            return Ok(activation_success_response(package_ref, &active_package));
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

        Ok(activation_success_response(package_ref, &active_package))
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
                prepare_install(
                    &available,
                    derive_owner(caller, &self.tenant_operator_user_id),
                )?
                .manifest_record
            };
            let removed_providers =
                Self::removed_extension_providers_from_manifest(&removal_manifest)?;
            let cleanup_requirements = removal_manifest.removal_cleanup_requirements().to_vec();
            // Deliberately validate cleanup actors only after caller
            // authorization and manifest/provider preflight. Hoisting this
            // check above the operation guard would change private-install
            // masking and concurrent error precedence.
            if !cleanup_requirements.is_empty() && authenticated_actor_user_id.is_none() {
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
                    LifecyclePhase::Removed,
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
        ensure_caller_may_mutate_tenant_installation(
            &installation,
            caller,
            &self.tenant_operator_user_id,
            "remove",
        )?;
        // Membership remove (#5459 P1 pivot): while other members still hold
        // the tool, the caller just LEAVES the member set — a single row
        // rewrite, no teardown. Only the last holder's remove (or the
        // operator removing a tenant-shared tool) tears the install down.
        if let RemoveDecision::LeaveMembers(remaining) =
            decide_remove(installation.owner(), caller)?
        {
            self.installation_store
                .upsert_installation(installation.with_owner(remaining))
                .await
                .map_err(map_extension_installation_error)?;
            return Ok(response_with_payload(
                Some(package_ref),
                LifecyclePhase::Removed,
                LifecycleProductPayload::ExtensionRemove { removed: true },
            ));
        }
        let previous_state = installation.activation_state();
        let lifecycle_package = self.lifecycle_package(&extension_id).await?;
        if let Err(error) = self
            .installation_store
            .set_activation_state(&installation_id, ExtensionActivationState::Disabled)
            .await
        {
            return Err(map_extension_installation_error(error));
        }
        if let Err(error) = self.remove_lifecycle_package(&extension_id).await {
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
        if let Err(error) = self.active_extensions.unpublish(&lifecycle_package) {
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
                self.restore_active_publication(&lifecycle_package, previous_state)
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
                self.restore_active_publication(&lifecycle_package, previous_state)
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
            LifecyclePhase::Removed,
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
                reason: format!("extension {} is already installed", extension_id.as_str()),
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
        previous_state: ExtensionActivationState,
    ) -> Result<(), ProductWorkflowError> {
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
    ) -> Result<(), ProductWorkflowError> {
        self.installation_store
            .upsert_installation(installation.clone())
            .await
            .map_err(map_extension_installation_error)
    }

    fn restore_active_publication(
        &self,
        package: &ExtensionPackage,
        previous_state: ExtensionActivationState,
    ) -> Result<(), ProductWorkflowError> {
        if previous_state == ExtensionActivationState::Enabled {
            self.active_extensions.publish(package)?;
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

struct HostedMcpDiscoveryRequest {
    base_package: ExtensionPackage,
    scope: ResourceScope,
    runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
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
    // Re-validate with the SAME source the package entered the catalog with:
    // stamping everything `HostBundled` here would launder an imported
    // (`InstalledLocal`) bundle into the stored-manifest tier that is allowed
    // first-party trust and the bundled hash-migration path below.
    let manifest_record = ExtensionManifestRecord::from_toml_with_contracts(
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
        ExtensionActivationState::Installed,
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
/// while preserving the activation state and credential bindings from `existing`.
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
    // Same source-preservation rule as `prepare_install`; the caller
    // (`migrate_host_bundled_manifest_hash`) additionally requires the STORED
    // manifest to be `HostBundled` before migrating, so an imported extension
    // whose on-disk manifest changed fails closed instead of migrating.
    let manifest_record = ExtensionManifestRecord::from_toml_with_contracts(
        &available.manifest_toml,
        available.source,
        &host_ports,
        Some(manifest_hash.clone()),
        &contracts,
    )
    .map_err(map_extension_installation_error)?
    .with_removal_cleanup_requirements(available.cleanup_requirements.clone());
    let installation = ExtensionInstallation::new(
        existing.installation_id().clone(),
        existing.extension_id().clone(),
        existing.activation_state(),
        ExtensionManifestRef::new(existing.extension_id().clone(), Some(manifest_hash)),
        existing.credential_bindings().to_vec(),
        chrono::Utc::now(),
        // Manifest migration preserves ownership — it changes the manifest
        // hash, never who the installation belongs to.
        existing.owner().clone(),
    )
    .map_err(map_extension_installation_error)?;
    Ok(ExtensionInstallPlan {
        manifest_record,
        installation,
    })
}

async fn migrate_host_bundled_manifest_hash(
    installation_store: &Arc<dyn ExtensionInstallationStore>,
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
) -> LifecycleProductResponse {
    let visible_capability_ids = package_visible_capability_ids(package);
    let message = activation_success_message(&package_ref, package, &visible_capability_ids);
    let connection_required = if package_declares_inbound_product_adapter(package) {
        Some(channel_connection_requirement(
            package_ref.id.as_str(),
            package.manifest.name.as_str(),
        ))
    } else {
        None
    };
    let mut response = response_with_payload(
        Some(package_ref),
        LifecyclePhase::Active,
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
        LifecyclePhase::Installed,
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
) -> String {
    if package_declares_inbound_product_adapter(package) {
        if package_ref.id.as_str() == "slack_bot" {
            return "Slack is installed as an inbound entrypoint. If WebChat shows a Slack account connection panel, tell the user to configure Slack OAuth for this extension rather than pasting anything into normal chat. If the user's Slack account is already connected, continue the user's original request; Slack DMs and WebUI chat can use the same user-scoped Slack tools.".to_string();
        }
        return format!(
            "{} is installed as an external channel. If WebChat shows a channel connection panel, tell the user to open the extension's app or bot, get the pairing code or connection challenge, and paste it into the WebChat connection panel rather than normal chat. If the user's channel account is already connected, continue the user's original request instead of asking them to pair again. Do not claim the channel can receive or send messages for the user until connection is confirmed.",
            package.manifest.name.as_str()
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

// Build the structured connect requirement for an inbound channel. The Slack OAuth
// copy is kept identical to the connectable-channels descriptor so the in-chat
// panel and the Settings panel read identically — enforced by
// `slack_requirement_copy_matches_connectable_descriptor`, not just by convention.
// Any other inbound channel gets a generic proof-code prompt. NOTE: no such
// channel ships today (Slack is the only inbound product adapter), and no
// backend mounts the generic proof-code redeem route — the first non-Slack
// inbound channel must mount one alongside this requirement or its submit
// will 404 (see PAIRING_REDEEM_PATH in the webui pairing-api.js).
pub(crate) fn channel_connection_requirement(
    channel_id: &str,
    display_name: &str,
) -> ChannelConnectionRequirement {
    if channel_id == "slack_bot" {
        ChannelConnectionRequirement {
            channel: "slack".to_string(),
            strategy: RebornChannelConnectStrategy::OAuth,
            instructions: "Connect Slack with OAuth from the extension configuration, then message the Slack bot directly.".to_string(),
            input_placeholder: String::new(),
            submit_label: "Connect Slack".to_string(),
            error_message: "Slack OAuth connection failed. Try configuring Slack again.".to_string(),
        }
    } else {
        ChannelConnectionRequirement {
            channel: channel_id.to_string(),
            strategy: RebornChannelConnectStrategy::InboundProofCode,
            instructions: format!(
                "Open {}'s app or bot, get the pairing code, and paste it here.",
                display_name
            ),
            input_placeholder: "Enter pairing code".to_string(),
            submit_label: "Connect".to_string(),
            error_message: "Pairing failed. Check the code and try again.".to_string(),
        }
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
fn phase_for_activation_state(state: ExtensionActivationState) -> LifecyclePhase {
    match state {
        ExtensionActivationState::Enabled => LifecyclePhase::Active,
        ExtensionActivationState::Disabled => LifecyclePhase::Disabled,
        ExtensionActivationState::Installed => LifecyclePhase::Installed,
    }
}

async fn search_installation_phase(
    extension: &AvailableExtensionPackage,
    installation: &ExtensionInstallation,
    credential_gate: Option<&RuntimeExtensionActivationCredentialGate>,
) -> Result<LifecyclePhase, ProductWorkflowError> {
    let phase = phase_for_activation_state(installation.activation_state());
    if phase != LifecyclePhase::Installed {
        return Ok(phase);
    }
    if search_credentials_configured(extension, credential_gate).await? {
        return Ok(LifecyclePhase::Configured);
    }
    Ok(phase)
}

async fn search_credentials_configured(
    extension: &AvailableExtensionPackage,
    credential_gate: Option<&RuntimeExtensionActivationCredentialGate>,
) -> Result<bool, ProductWorkflowError> {
    let requirements = package_runtime_credential_auth_requirements(&extension.package);
    if requirements.is_empty() {
        return Ok(false);
    }
    let Some(credential_gate) = credential_gate else {
        return Ok(false);
    };
    Ok(credential_gate
        .missing_requirements(requirements)
        .await
        .map_err(map_search_credential_stage_error)?
        .is_empty())
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
        matches!(extension.installation_phase, Some(LifecyclePhase::Active))
            && !extension
                .summary
                .surface_kinds
                .contains(&LifecycleExtensionSurfaceKind::ExternalChannel)
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
            Some(LifecyclePhase::Installed | LifecyclePhase::Configured | LifecyclePhase::Disabled)
        ) && !extension
            .summary
            .surface_kinds
            .contains(&LifecycleExtensionSurfaceKind::ExternalChannel)
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
            Some(LifecyclePhase::Installed | LifecyclePhase::Configured | LifecyclePhase::Active)
        ) && extension
            .summary
            .surface_kinds
            .contains(&LifecycleExtensionSurfaceKind::ExternalChannel)
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
    // TODO(#4091): split durable-store transient failures from malformed
    // lifecycle requests when ExtensionInstallationStore grows a DB backend.
    ProductWorkflowError::InvalidBindingRequest {
        reason: error.to_string(),
    }
}

fn project_installation_owners<I>(
    installations: I,
) -> Result<std::collections::BTreeMap<ExtensionId, InstallationOwner>, ProductWorkflowError>
where
    I: IntoIterator<Item = ExtensionInstallation>,
{
    let mut owners = std::collections::BTreeMap::new();
    for installation in installations {
        let extension_id = installation.extension_id().clone();
        if owners
            .insert(extension_id.clone(), installation.owner().clone())
            .is_some()
        {
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

fn ensure_caller_may_mutate_tenant_installation(
    installation: &ExtensionInstallation,
    caller: &UserId,
    tenant_operator: &UserId,
    operation: &str,
) -> Result<(), ProductWorkflowError> {
    if installation.owner().is_tenant() && caller != tenant_operator {
        return Err(ProductWorkflowError::InvalidBindingRequest {
            reason: format!(
                "extension {} is a shared tool; only the tenant admin can {operation} it",
                installation.extension_id().as_str()
            ),
        });
    }
    Ok(())
}

fn hosted_mcp_discovery_error(error: HostedMcpDiscoveryError) -> ProductWorkflowError {
    match error {
        HostedMcpDiscoveryError::Transient(reason) => ProductWorkflowError::Transient {
            reason: format!("hosted MCP discovery failed: {reason}"),
        },
        HostedMcpDiscoveryError::Permanent(reason) => ProductWorkflowError::InvalidBindingRequest {
            reason: format!("hosted MCP discovery failed: {reason}"),
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
        ExtensionLifecycleEvent, ExtensionLifecycleEventSink, ExtensionLifecycleService,
        ExtensionManifest, ExtensionRegistry, InMemoryExtensionInstallationStore,
        SharedExtensionRegistry,
    };
    use ironclaw_filesystem::{
        DirEntry, FileStat, FilesystemError, FilesystemOperation, LocalFilesystem,
    };
    use ironclaw_host_api::{
        AgentId, CapabilityId, ExtensionLifecycleOperation, HostPath, HostPortCatalog,
        InvocationId, MountAlias, MountGrant, MountPermissions, MountView, NetworkMethod,
        ProjectId, ResourceScope, RuntimeCredentialAccountSetup, RuntimeHttpEgress,
        RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, TenantId,
        TrustClass, UserId,
    };
    use ironclaw_host_runtime::{SPAWN_SUBAGENT_CAPABILITY_ID, builtin_first_party_package};
    use ironclaw_product_workflow::{
        LifecycleExtensionRuntimeKind, LifecycleExtensionSource, LifecycleProductAction,
        LifecycleProductContext, LifecycleProductFacade, LifecycleProductSurfaceContext,
        LifecycleReadinessBlocker,
    };
    use ironclaw_trust::{HostTrustPolicy, InvalidationBus, TrustPolicy};

    mod private_install_tests;

    #[tokio::test]
    async fn lifecycle_owner_projections_reject_duplicate_extension_ids() {
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
                        ExtensionActivationState::Enabled,
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

        let owners_error = port
            .installation_owners()
            .await
            .expect_err("duplicate owner rows fail closed");
        assert!(matches!(
            owners_error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));

        let active_error = port
            .active_model_visible_capabilities()
            .await
            .expect_err("duplicate active owner rows fail closed");
        assert!(matches!(
            active_error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
    }

    fn zip_bundle(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;

        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, bytes) in entries {
            writer.start_file(*name, options).expect("start zip entry");
            writer.write_all(bytes).expect("write zip entry");
        }
        writer.finish().expect("finish zip").into_inner()
    }

    #[test]
    fn installed_external_channel_search_result_gets_activation_guidance() {
        let payload = LifecycleProductPayload::ExtensionSearch {
            extensions: vec![LifecycleSearchExtensionSummary {
                summary: LifecycleExtensionSummary {
                    package_ref: LifecyclePackageRef::new(
                        LifecyclePackageKind::Extension,
                        "slack_bot",
                    )
                    .expect("valid package ref"),
                    name: "Slack".to_string(),
                    version: "1.0.0".to_string(),
                    description: "Slack channel".to_string(),
                    source: LifecycleExtensionSource::HostBundled,
                    runtime_kind: LifecycleExtensionRuntimeKind::WasmTool,
                    surface_kinds: vec![LifecycleExtensionSurfaceKind::ExternalChannel],
                    visible_capability_ids: Vec::new(),
                    visible_read_only_capability_ids: Vec::new(),
                    credential_requirements: Vec::new(),
                    onboarding: None,
                },
                installation_phase: Some(LifecyclePhase::Installed),
            }],
            count: 1,
        };

        assert!(extension_search_has_installed_external_channel_result(
            Some(&payload)
        ));
        assert!(!extension_search_has_ready_result(Some(&payload)));
    }

    #[test]
    fn disabled_extension_search_result_gets_inactive_activation_guidance() {
        let payload = LifecycleProductPayload::ExtensionSearch {
            extensions: vec![LifecycleSearchExtensionSummary {
                summary: LifecycleExtensionSummary {
                    package_ref: LifecyclePackageRef::new(
                        LifecyclePackageKind::Extension,
                        "disabled_fixture",
                    )
                    .expect("valid package ref"),
                    name: "Disabled fixture".to_string(),
                    version: "1.0.0".to_string(),
                    description: "Disabled lifecycle fixture".to_string(),
                    source: LifecycleExtensionSource::HostBundled,
                    runtime_kind: LifecycleExtensionRuntimeKind::WasmTool,
                    surface_kinds: Vec::new(),
                    visible_capability_ids: vec!["disabled_fixture.search".to_string()],
                    visible_read_only_capability_ids: vec!["disabled_fixture.search".to_string()],
                    credential_requirements: Vec::new(),
                    onboarding: None,
                },
                installation_phase: Some(LifecyclePhase::Disabled),
            }],
            count: 1,
        };

        assert!(extension_search_has_inactive_installed_result(Some(
            &payload
        )));
        assert!(!extension_search_has_ready_result(Some(&payload)));
        assert!(!extension_search_has_installed_external_channel_result(
            Some(&payload)
        ));
    }

    #[test]
    fn activation_message_enumerates_published_tools_by_exact_name() {
        // Regression: the model only sees a *count* of deferred tools, so after
        // activating an extension it must be handed the exact tool names or it
        // assumes they are unavailable and gives up. The success message must name
        // every published capability and steer the model to direct invocation.
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid package ref");
        let package = fixture_extension_package().package;
        let visible_capability_ids = vec!["fixture.search".to_string()];
        let message = activation_success_message(&package_ref, &package, &visible_capability_ids);
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
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid package ref");
        let package = fixture_extension_package().package;
        let message = activation_success_message(&package_ref, &package, &[]);
        assert!(message.contains("Extension activation succeeded"));
        assert!(
            !message.contains("callable by exact name"),
            "no tools published ⇒ no direct-invocation guidance, got: {message}"
        );
    }

    #[tokio::test]
    async fn extension_lifecycle_installs_activates_and_removes_catalog_package() {
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
        assert_eq!(search.phase, LifecyclePhase::Discovered);
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
        assert_eq!(install.phase, LifecyclePhase::Installed);
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
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("fixture").unwrap())
                .is_none()
        );

        let activate = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("activate extension");
        assert_eq!(activate.phase, LifecyclePhase::Active);
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
        assert_eq!(remove.phase, LifecyclePhase::Removed);
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

    /// A complete uploaded WASM tool bundle zip in the `InstalledLocal`-legal
    /// shape (trust `third_party`, capabilities via `capability_provider`
    /// host_api), for driving `import_bundle` through the facade.
    fn importable_tool_zip(id: &str) -> Vec<u8> {
        let manifest = format!(
            r#"
schema_version = "reborn.extension_manifest.v2"
id = "{id}"
name = "Imported Tool"
version = "0.1.0"
description = "Uploaded tool bundle fixture"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/tool.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "{id}.run"
description = "Run the tool"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/run.input.json"
output_schema_ref = "schemas/run.output.json"
"#
        );
        zip_bundle(&[
            ("manifest.toml", manifest.as_bytes()),
            // Component header — the import path rejects core modules.
            ("wasm/tool.wasm", b"\0asm\x0d\0\x01\0".as_slice()),
            ("schemas/run.input.json", b"{}".as_slice()),
            ("schemas/run.output.json", b"{}".as_slice()),
        ])
    }

    /// Happy path for the WebUI "Install Tool" upload flow: an uploaded zip
    /// imports into the catalog, then installs (assets materialized under
    /// `/system/extensions/<id>/`) and activates (capability published) through
    /// the SAME facade actions any catalog extension uses.
    #[tokio::test]
    async fn import_bundle_imports_installs_and_activates_uploaded_tool() {
        let (_dir, storage_root, facade, active_registry, _installation_store) =
            extension_lifecycle_fixture();

        let import = facade
            .import_extension_bundle(lifecycle_surface_context(), importable_tool_zip("uploaded"))
            .await
            .expect("import uploaded tool bundle");
        assert_eq!(import.phase, LifecyclePhase::Discovered);
        let Some(LifecycleProductPayload::ExtensionSearch { extensions, count }) =
            import.payload.as_ref()
        else {
            panic!("expected extension search payload from import");
        };
        assert_eq!(*count, 1);
        assert_eq!(extensions[0].summary.package_ref.id.as_str(), "uploaded");

        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "uploaded")
            .expect("valid ref");
        let install = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install imported extension");
        assert_eq!(install.phase, LifecyclePhase::Installed);
        assert!(
            storage_root
                .join("system/extensions/uploaded/manifest.toml")
                .exists()
        );
        assert!(
            storage_root
                .join("system/extensions/uploaded/wasm/tool.wasm")
                .exists()
        );

        let activate = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate { package_ref },
            )
            .await
            .expect("activate imported extension");
        assert_eq!(activate.phase, LifecyclePhase::Active);
        assert!(
            active_registry
                .snapshot()
                .get_capability(&ironclaw_host_api::CapabilityId::new("uploaded.run").unwrap())
                .is_some()
        );
    }

    /// Intended lifecycle for imported extensions: remove returns the package
    /// to "available" (the catalog keeps it, assets in memory) and installing
    /// it again from the Registry must work without re-uploading. (Dropping an
    /// imported package from the catalog entirely is a future endpoint.)
    #[tokio::test]
    async fn imported_extension_reinstalls_after_remove() {
        let (_dir, storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture();
        facade
            .import_extension_bundle(lifecycle_surface_context(), importable_tool_zip("uploaded"))
            .await
            .expect("import uploaded tool bundle");
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "uploaded")
            .expect("valid ref");
        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install imported extension");
        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("remove imported extension");
        assert!(
            !storage_root
                .join("system/extensions/uploaded/manifest.toml")
                .exists(),
            "remove must delete the materialized files"
        );
        let reinstall = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall { package_ref },
            )
            .await
            .expect("reinstalling a removed imported extension from the catalog must succeed");
        assert_eq!(reinstall.phase, LifecyclePhase::Installed);
        assert!(
            storage_root
                .join("system/extensions/uploaded/wasm/tool.wasm")
                .exists(),
            "reinstall must re-materialize the in-memory assets"
        );
    }

    /// `import_bundle` must reject ids that already exist — both a repeat of a
    /// previous import and an id already present in the catalog (a bundled or
    /// discovered package). Without this check the second import's
    /// materialization would overwrite `/system/extensions/<id>/` while
    /// catalog/lifecycle state still points at the original package.
    #[tokio::test]
    async fn import_bundle_rejects_duplicate_and_catalog_resident_ids() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture();

        facade
            .import_extension_bundle(lifecycle_surface_context(), importable_tool_zip("uploaded"))
            .await
            .expect("first import succeeds");
        let reimport_error = facade
            .import_extension_bundle(lifecycle_surface_context(), importable_tool_zip("uploaded"))
            .await
            .expect_err("re-importing the same id must be rejected");
        assert!(
            format!("{reimport_error}").contains("already exists in the catalog"),
            "unexpected error: {reimport_error}"
        );

        // "fixture" is already in the catalog (the fixture package); an upload
        // claiming that id must not be able to shadow or overwrite it.
        let shadow_error = facade
            .import_extension_bundle(lifecycle_surface_context(), importable_tool_zip("fixture"))
            .await
            .expect_err("importing an id already in the catalog must be rejected");
        assert!(
            format!("{shadow_error}").contains("already exists in the catalog"),
            "unexpected error: {shadow_error}"
        );
    }

    /// #5499 review finding #3 guard: the unzip/validation phase runs in
    /// `spawn_blocking` behind a bounded decode semaphore, acquired BEFORE the
    /// catalog-write + operation locks. Concurrent imports of distinct ids must
    /// interleave across semaphore -> catalog -> operation without deadlocking,
    /// and every import must still land in the catalog.
    #[tokio::test]
    async fn concurrent_imports_of_distinct_ids_all_succeed() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture();

        // More concurrent imports than decode permits, so the test exercises
        // both permit waiting and the lock handoff after decode.
        let (a, b, c) = tokio::join!(
            facade.import_extension_bundle(
                lifecycle_surface_context(),
                importable_tool_zip("uploaded-a")
            ),
            facade.import_extension_bundle(
                lifecycle_surface_context(),
                importable_tool_zip("uploaded-b")
            ),
            facade.import_extension_bundle(
                lifecycle_surface_context(),
                importable_tool_zip("uploaded-c")
            ),
        );
        a.expect("concurrent import a succeeds");
        b.expect("concurrent import b succeeds");
        c.expect("concurrent import c succeeds");

        for id in ["uploaded-a", "uploaded-b", "uploaded-c"] {
            let package_ref =
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, id).expect("valid ref");
            let install = facade
                .execute(
                    lifecycle_surface_context(),
                    LifecycleProductAction::ExtensionInstall { package_ref },
                )
                .await
                .unwrap_or_else(|error| panic!("imported {id} must be installable: {error}"));
            assert_eq!(install.phase, LifecyclePhase::Installed);
        }
    }

    #[tokio::test]
    async fn extension_activate_returns_slack_oauth_guidance_for_external_channel_package() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![fixture_external_channel_package(
                    "slack_bot",
                    "Slack",
                )]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "slack_bot")
            .expect("valid ref");
        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install slack channel");

        let activate = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("activate slack channel");

        assert_eq!(activate.phase, LifecyclePhase::Active);
        let message = activate.message.as_deref().expect("activation message");
        assert!(
            message.contains("configure Slack OAuth")
                && message.contains("WebChat")
                && message.contains("rather than pasting anything into normal chat")
                && message.contains("continue the user's original request")
                && message.contains("user-scoped Slack tools"),
            "Slack activation should guide the model into OAuth setup UI, got: {message}"
        );
        assert!(
            !message.contains("pairing"),
            "Slack activation must not mention legacy manual-code flows: {message}"
        );
        let Some(LifecycleProductPayload::ExtensionActivate {
            visible_capability_ids,
            connection_required,
            ..
        }) = activate.payload.as_ref()
        else {
            panic!("expected extension activate payload");
        };
        assert!(
            visible_capability_ids.is_empty(),
            "Slack channel activation must not imply model-visible Slack read tools"
        );
        // The structured connect requirement is what drives the in-chat
        // connection panel; the prose message above is model guidance only.
        let requirement = connection_required
            .as_ref()
            .expect("slack channel activation must carry a structured connection requirement");
        assert_eq!(requirement.channel, "slack");
        assert_eq!(requirement.strategy, RebornChannelConnectStrategy::OAuth);
        assert_eq!(requirement.input_placeholder, "");
        assert_eq!(requirement.submit_label, "Connect Slack");
        assert_eq!(
            requirement.instructions,
            "Connect Slack with OAuth from the extension configuration, then message the Slack bot directly."
        );
        assert_eq!(
            requirement.error_message,
            "Slack OAuth connection failed. Try configuring Slack again."
        );
    }

    #[tokio::test]
    async fn extension_activate_returns_generic_pairing_guidance_for_external_channel_package() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
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

        let activate = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate { package_ref },
            )
            .await
            .expect("activate external channel");

        assert_eq!(activate.phase, LifecyclePhase::Active);
        let message = activate.message.as_deref().expect("activation message");
        assert!(
            message.contains("Telegram is installed as an external channel")
                && message.contains("app or bot")
                && message.contains("pairing code")
                && message.contains("WebChat connection panel")
                && message.contains("rather than normal chat")
                && message.contains("continue the user's original request")
                && message.contains("already connected")
                && message.contains("until connection is confirmed"),
            "external channel activation should guide the model into generic pairing UI, got: {message}"
        );
        let Some(LifecycleProductPayload::ExtensionActivate {
            connection_required,
            ..
        }) = activate.payload.as_ref()
        else {
            panic!("expected extension activate payload");
        };
        let requirement = connection_required
            .as_ref()
            .expect("external channel activation must carry a structured connection requirement");
        assert_eq!(requirement.channel, "telegram");
        assert_eq!(
            requirement.strategy,
            RebornChannelConnectStrategy::InboundProofCode
        );
        assert!(
            requirement.instructions.contains("Telegram"),
            "generic channel copy should name the channel: {}",
            requirement.instructions
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

        assert_eq!(remove.phase, LifecyclePhase::Removed);
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
        installation_store: Arc<InMemoryExtensionInstallationStore>,
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
            installation_store: Arc<InMemoryExtensionInstallationStore>,
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
        ) -> Result<(), RebornServicesError> {
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
                return Err(RebornServicesError::internal_from(detail));
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

        let credential_request = {
            let credential_requests = credential_cleanup
                .requests
                .lock()
                .expect("credential cleanup lock");
            assert_eq!(credential_requests.len(), 1);
            credential_requests[0].clone()
        };
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

        assert_eq!(remove.phase, LifecyclePhase::Removed);
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

        let restarted_port = RebornLocalExtensionManagementPort::new(
            Arc::clone(&installed_port.filesystem),
            AvailableExtensionCatalog::from_packages(Vec::new()),
            installation_store.clone(),
            Arc::new(Mutex::new(ExtensionLifecycleService::new(
                ExtensionRegistry::new(),
            ))),
            installed_port.active_extensions.clone(),
            None,
            lifecycle_owner(),
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
        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("activate example channel");

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
                && message.contains("builtin.extension_activate")
                && message.contains("outbound delivery target")
                && message.contains("do not activate"),
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
        assert_eq!(example.installation_phase, Some(LifecyclePhase::Active));
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[tokio::test]
    async fn slack_tools_extension_installs_activates_and_publishes_capabilities() {
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
            .list_installed(&lifecycle_owner())
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

    #[cfg(feature = "slack-v2-host-beta")]
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
        assert_eq!(requirement.provider.as_str(), "slack_personal");
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

    #[cfg(feature = "slack-v2-host-beta")]
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
    async fn hosted_mcp_activation_falls_back_to_bundled_manifest_when_discovery_returns_no_tools()
    {
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
        let activate = port
            .activate_with_prechecked_credentials_for_test(
                package_ref,
                ExtensionActivationMode::HostedMcpDiscovery {
                    scope: hosted_mcp_scope("hosted-mcp-empty-tools"),
                    runtime_http_egress: Arc::new(EmptyToolsHostedMcpEgress),
                },
            )
            .await
            .expect("transient discovery failure should fall back to bundled manifest");

        assert_eq!(activate.phase, LifecyclePhase::Active);
        assert!(
            active_registry
                .snapshot()
                .get_capability(&CapabilityId::new("notion.notion-search").unwrap())
                .is_some(),
            "fallback activation must publish bundled Notion capabilities"
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
    async fn commit_activation_rolls_back_when_set_activation_state_fails() {
        let lifecycle_sink = Arc::new(RecordingLifecycleSink::default());
        let lifecycle_service = ExtensionLifecycleService::new(ExtensionRegistry::new())
            .with_event_sink(lifecycle_sink.clone());
        let (_dir, port, active_registry, failing_store, _trust_policy) =
            extension_port_with_set_activation_failing_store(lifecycle_service);
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
            .expect_err("activation-state persistence failure is reported");

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
        assert_eq!(
            fixture_installation_state(failing_store.as_ref()).await,
            ExtensionActivationState::Installed
        );
        assert!(
            lifecycle_sink
                .operations()
                .contains(&ExtensionLifecycleOperation::Disable)
        );
    }

    #[tokio::test]
    async fn commit_activation_rolls_back_when_publish_fails() {
        let (_dir, _storage_root, port, active_registry, installation_store) =
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
        assert_eq!(
            fixture_installation_state(installation_store.as_ref()).await,
            ExtensionActivationState::Installed
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
                )
                .await
                .unwrap_or_else(|error| panic!("activation attempt {attempt} failed: {error}"));
            assert!(matches!(
                response.payload,
                Some(LifecycleProductPayload::ExtensionActivate {
                    activated: true,
                    ..
                })
            ));
        }
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("fixture").expect("valid extension id"))
                .is_some()
        );
    }

    #[tokio::test]
    async fn commit_activation_publish_failure_preserves_previously_enabled_extension() {
        let lifecycle_sink = Arc::new(RecordingLifecycleSink::default());
        let lifecycle_service = ExtensionLifecycleService::new(ExtensionRegistry::new())
            .with_event_sink(lifecycle_sink.clone());
        let (_dir, _storage_root, port, _active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_service_and_trust_policy(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                lifecycle_service,
                Arc::new(HostTrustPolicy::fail_closed()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");
        let extension_id = ExtensionId::new("fixture").expect("valid extension id");
        let installation_id = ExtensionInstallationId::new("fixture").expect("valid installation");

        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect("install extension");
        installation_store
            .set_activation_state(&installation_id, ExtensionActivationState::Enabled)
            .await
            .expect("seed enabled installation");
        let error = port
            .commit_activation(
                package_ref,
                &extension_id,
                &installation_id,
                ExtensionActivationState::Enabled,
                fixture_extension_package().package,
            )
            .await
            .expect_err("publish failure is reported");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        assert_eq!(
            fixture_installation_state(installation_store.as_ref()).await,
            ExtensionActivationState::Enabled
        );
        let operations = lifecycle_sink.operations();
        assert!(operations.contains(&ExtensionLifecycleOperation::Enable));
        assert!(!operations.contains(&ExtensionLifecycleOperation::Disable));
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
        let installation_store: Arc<dyn ExtensionInstallationStore> = installation_store;

        restore_extension_lifecycle_state(
            &restored_catalog,
            &port.filesystem,
            &installation_store,
            &restored_lifecycle,
            &restored_active_extensions,
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

    #[tokio::test]
    async fn restore_removes_retired_slack_user_installation_without_catalog_entry() {
        let installation_store = Arc::new(InMemoryExtensionInstallationStore::default());
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
                    ExtensionActivationState::Enabled,
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
        let installation_store_trait: Arc<dyn ExtensionInstallationStore> =
            installation_store.clone();
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(LocalFilesystem::new());

        restore_extension_lifecycle_state(
            &AvailableExtensionCatalog::from_packages(Vec::new()),
            &filesystem,
            &installation_store_trait,
            &restored_lifecycle,
            &restored_active_extensions,
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
                    ExtensionActivationState::Enabled,
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
        let installation_store: Arc<dyn ExtensionInstallationStore> = installation_store;

        restore_extension_lifecycle_state(
            &restored_catalog,
            &port.filesystem,
            &installation_store,
            &restored_lifecycle,
            &restored_active_extensions,
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
        let installation_store: Arc<dyn ExtensionInstallationStore> = installation_store;

        restore_extension_lifecycle_state(
            &AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
            &port.filesystem,
            &installation_store,
            &restored_lifecycle,
            &restored_active_extensions,
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
        let installation_store: Arc<dyn ExtensionInstallationStore> = installation_store;

        restore_extension_lifecycle_state(
            &changed_catalog,
            &port.filesystem,
            &installation_store,
            &restored_lifecycle,
            &restored_active_extensions,
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
        let installation_store = Arc::new(InMemoryExtensionInstallationStore::default());
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
            .upsert_installation(fixture_installation(
                Some("sha256:old".to_string()),
                ExtensionActivationState::Enabled,
            ))
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
        let installation_store: Arc<dyn ExtensionInstallationStore> = installation_store;
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(LocalFilesystem::new());

        let error = restore_extension_lifecycle_state(
            &catalog,
            &filesystem,
            &installation_store,
            &restored_lifecycle,
            &restored_active_extensions,
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
    async fn extension_lifecycle_installs_activates_and_removes_github() {
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
        assert_eq!(search.phase, LifecyclePhase::Discovered);
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
        assert_eq!(install.phase, LifecyclePhase::Installed);
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
                .is_none()
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
        assert_eq!(github.installation_phase, Some(LifecyclePhase::Configured));
        assert!(
            github.summary.credential_requirements.is_empty(),
            "configured inactive GitHub search results must not expose satisfied PAT requirements"
        );
        assert!(
            github.summary.onboarding.is_none(),
            "configured inactive GitHub search results must not expose stale PAT setup onboarding"
        );

        let activate = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("activate extension");
        assert_eq!(activate.phase, LifecyclePhase::Active);
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
        assert_eq!(github.installation_phase, Some(LifecyclePhase::Active));
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
        assert_eq!(remove.phase, LifecyclePhase::Removed);
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
    async fn extension_lifecycle_search_reports_credential_backend_failure_as_transient() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            github_extension_lifecycle_fixture();
        let facade = facade.with_runtime_credential_accounts(Arc::new(
            BackendUnavailableRuntimeCredentialAccounts,
        ));
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
            .expect("install extension");
        let error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionSearch {
                    query: "github".to_string(),
                },
            )
            .await
            .expect_err("search reports credential backend failure");

        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
    }

    #[tokio::test]
    async fn lifecycle_facade_reports_typed_credential_blockers_without_activation() {
        let (_dir, _storage_root, facade, active_registry, _installation_store) =
            github_extension_lifecycle_fixture();
        let facade =
            facade.with_runtime_credential_accounts(Arc::new(MissingRuntimeCredentialAccounts));
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
            .expect("install extension");
        let response = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("missing product-auth account is a typed readiness outcome");

        assert_eq!(response.phase, LifecyclePhase::Installed);
        assert!(matches!(
            response.payload,
            Some(LifecycleProductPayload::ExtensionActivate {
                activated: false,
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

        // #5525 review: ownership masks BEFORE the credential preflight — a
        // non-owner activating a private credentialed install must get the
        // masked "is not installed" denial, never an auth-required response
        // that leaks the extension's existence and credential requirements.
        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("operator removes the shared installation");
        facade
            .execute(
                lifecycle_surface_context_for_user("alice"),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("alice installs the credentialed extension privately");
        let error = facade
            .execute(
                lifecycle_surface_context_for_user("bob"),
                LifecycleProductAction::ExtensionActivate { package_ref },
            )
            .await
            .expect_err("foreign private credentialed install must be inoperable");
        assert!(
            error.to_string().contains("is not installed"),
            "ownership must mask before the credential preflight: {error}"
        );
    }

    #[tokio::test]
    async fn lifecycle_facade_blocks_non_operator_activation_of_shared_installation() {
        let (_dir, _storage_root, facade, active_registry, _installation_store) =
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
            .expect("operator installs the shared extension");
        let error = facade
            .execute(
                lifecycle_surface_context_for_user("alice"),
                LifecycleProductAction::ExtensionActivate { package_ref },
            )
            .await
            .expect_err("non-operator must not activate a shared extension");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        assert!(
            error
                .to_string()
                .contains("only the tenant admin can activate it"),
            "unexpected activation denial: {error}"
        );
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("fixture").expect("valid extension id"))
                .is_none(),
            "denied activation must not publish shared capabilities"
        );
    }

    #[tokio::test]
    async fn lifecycle_facade_rejects_static_activation_for_hosted_mcp_packages() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let facade =
            facade.with_runtime_credential_accounts(Arc::new(ConfiguredRuntimeCredentialAccounts));
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");

        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install Notion MCP");
        let error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate { package_ref },
            )
            .await
            .expect_err("hosted MCP activation needs runtime egress services");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
    }

    #[tokio::test]
    async fn lifecycle_facade_activates_hosted_mcp_with_runtime_egress() {
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

        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install Notion MCP");
        let activate = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate { package_ref },
            )
            .await
            .expect("hosted MCP activation should use discovery egress");

        assert_eq!(activate.phase, LifecyclePhase::Active);
        assert!(
            active_registry
                .snapshot()
                .get_capability(&CapabilityId::new("notion.live-search").unwrap())
                .is_some()
        );
    }

    #[tokio::test]
    async fn extension_lifecycle_installs_activates_and_removes_gsuite() {
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
        assert_eq!(search.phase, LifecyclePhase::Discovered);
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
        assert_eq!(search.phase, LifecyclePhase::Discovered);
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
            assert_eq!(install.phase, LifecyclePhase::Installed);
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
                .is_none()
        );

        for package_ref in [calendar_ref.clone(), gmail_ref.clone()] {
            let activate = facade
                .execute(
                    lifecycle_surface_context(),
                    LifecycleProductAction::ExtensionActivate { package_ref },
                )
                .await
                .expect("activate extension");
            assert_eq!(activate.phase, LifecyclePhase::Active);
        }
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
            assert_eq!(remove.phase, LifecyclePhase::Removed);
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
    async fn extension_install_rejects_duplicate_without_overwriting_materialized_files() {
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

        let error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall { package_ref },
            )
            .await
            .expect_err("duplicate install is rejected before materialization");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        assert_eq!(
            std::fs::read(wasm_path).expect("installed module remains"),
            b"existing-live-module"
        );
    }

    #[tokio::test]
    async fn extension_activate_rejects_lifecycle_package_without_installation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");
        let mut filesystem = LocalFilesystem::new();
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
        let port = RebornLocalExtensionManagementPort::new(
            Arc::new(filesystem),
            AvailableExtensionCatalog::from_packages(Vec::new()),
            Arc::new(InMemoryExtensionInstallationStore::default()),
            Arc::new(Mutex::new(ExtensionLifecycleService::new(
                lifecycle_registry,
            ))),
            test_active_extension_publisher(
                Arc::clone(&active_registry),
                test_extension_trust_policy(),
            ),
            None,
            lifecycle_owner(),
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
        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("activate extension");

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
        assert_eq!(
            installation.activation_state(),
            ExtensionActivationState::Enabled
        );
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
        assert_eq!(
            installation.activation_state(),
            ExtensionActivationState::Enabled
        );
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
        assert_enabled_active_extension_state(&active_registry, installation_store.as_ref()).await;
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
    async fn extension_auth_and_configure_return_unsupported() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture();
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").unwrap();

        for action in [
            LifecycleProductAction::ExtensionAuth {
                package_ref: package_ref.clone(),
            },
            LifecycleProductAction::ExtensionConfigure {
                package_ref: package_ref.clone(),
                payload: None,
            },
        ] {
            let response = facade
                .execute(lifecycle_surface_context(), action)
                .await
                .expect("unsupported response");
            assert_unsupported_extension_response(
                response,
                "extension_auth_and_configure_not_yet_wired",
            );
        }
    }

    #[tokio::test]
    async fn project_package_returns_available_extension_projection() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture();
        let response = facade
            .project_package(
                lifecycle_surface_context(),
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").unwrap(),
            )
            .await
            .expect("extension projection");

        assert_eq!(response.phase, LifecyclePhase::Discovered);
        let Some(LifecycleProductPayload::ExtensionList { extensions, count }) = response.payload
        else {
            panic!("expected extension list projection");
        };
        assert_eq!(count, 1);
        assert_eq!(extensions[0].summary.package_ref.id.as_str(), "fixture");
    }

    fn extension_lifecycle_fixture() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        crate::extension_host::lifecycle::RebornLocalLifecycleFacade,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
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
        crate::extension_host::lifecycle::RebornLocalLifecycleFacade,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
    ) {
        extension_lifecycle_fixture_with_catalog_and_service(
            AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
            lifecycle_service,
        )
    }

    fn github_extension_lifecycle_fixture() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        crate::extension_host::lifecycle::RebornLocalLifecycleFacade,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
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
        ) -> Result<SecretCleanupReport, RebornServicesError> {
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
        ) -> Result<SecretCleanupReport, RebornServicesError> {
            match self.calls.fetch_add(1, Ordering::SeqCst) {
                0 => Err(RebornServicesError::internal_from(
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
        assert_eq!(cleanup.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn ui_facade_extension_remove_revokes_exclusive_credential_at_convergence_point() {
        // Convergence coverage: the WebUI facade removal door (`ExtensionRemove`)
        // and the `builtin.extension_remove` agent capability both call
        // `RebornLocalExtensionManagementPort::remove`, so credential revocation
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
        assert_eq!(remove.phase, LifecyclePhase::Removed);
        let retry = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove { package_ref },
            )
            .await
            .expect("retry removal after github is absent");
        assert_eq!(retry.phase, LifecyclePhase::Removed);
        assert!(matches!(
            retry.payload,
            Some(LifecycleProductPayload::ExtensionRemove { removed: false })
        ));

        let requests = cleanup.requests.lock().expect("cleanup lock");
        assert_eq!(
            requests.len(),
            2,
            "initial removal and an already-absent retry must both revoke the exclusive github credential"
        );
        for request in requests.iter() {
            assert_eq!(
                request.provider.as_ref().map(|provider| provider.as_str()),
                Some("github")
            );
            assert_eq!(request.extension_id.as_str(), "github");
            assert_eq!(request.action, SecretCleanupAction::Uninstall);
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
        let port = RebornLocalExtensionManagementPort::new(
            Arc::clone(&installed_port.filesystem),
            AvailableExtensionCatalog::from_packages(Vec::new()),
            installation_store,
            Arc::clone(&installed_port.lifecycle_service),
            installed_port.active_extensions.clone(),
            Some(cleanup.clone() as Arc<dyn ExtensionCredentialCleanup>),
            lifecycle_owner(),
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
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].provider.as_ref().map(AuthProviderId::as_str),
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
        let port = RebornLocalExtensionManagementPort::new(
            Arc::clone(&installed_port.filesystem),
            AvailableExtensionCatalog::from_packages(Vec::new()),
            installation_store.clone(),
            Arc::clone(&installed_port.lifecycle_service),
            installed_port.active_extensions.clone(),
            Some(cleanup.clone() as Arc<dyn ExtensionCredentialCleanup>),
            lifecycle_owner(),
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
        let repair_port = RebornLocalExtensionManagementPort::new(
            Arc::clone(&base_port.filesystem),
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
            installation_store.clone(),
            Arc::clone(&base_port.lifecycle_service),
            base_port.active_extensions.clone(),
            Some(cleanup.clone() as Arc<dyn ExtensionCredentialCleanup>),
            lifecycle_owner(),
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
        let no_catalog_port = RebornLocalExtensionManagementPort::new(
            Arc::clone(&base_port.filesystem),
            AvailableExtensionCatalog::from_packages(Vec::new()),
            installation_store.clone(),
            Arc::clone(&base_port.lifecycle_service),
            base_port.active_extensions.clone(),
            Some(cleanup.clone() as Arc<dyn ExtensionCredentialCleanup>),
            lifecycle_owner(),
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
        )
        .await
        .expect("activate github");
        let installation_id =
            ExtensionInstallationId::new("github").expect("valid installation id");
        installation_store
            .delete_installation(&installation_id)
            .await
            .expect("simulate failed installation restoration");

        let retry_port = RebornLocalExtensionManagementPort::new(
            Arc::clone(&port.filesystem),
            AvailableExtensionCatalog::from_packages(Vec::new()),
            installation_store.clone(),
            Arc::clone(&port.lifecycle_service),
            port.active_extensions.clone(),
            Some(cleanup as Arc<dyn ExtensionCredentialCleanup>),
            lifecycle_owner(),
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

        let repair_port = RebornLocalExtensionManagementPort::new(
            Arc::clone(&port.filesystem),
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
            installation_store,
            Arc::clone(&port.lifecycle_service),
            port.active_extensions.clone(),
            None,
            lifecycle_owner(),
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

        let repair_port = RebornLocalExtensionManagementPort::new(
            Arc::clone(&port.filesystem),
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
            installation_store,
            Arc::clone(&port.lifecycle_service),
            port.active_extensions.clone(),
            None,
            lifecycle_owner(),
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
        assert_eq!(remove.phase, LifecyclePhase::Removed);

        let requests = cleanup.requests.lock().expect("cleanup lock");
        assert!(
            requests.is_empty(),
            "the shared google credential must be preserved while google-calendar \
             still authorizes against it, got cleanup requests: {requests:?}"
        );
    }

    fn extension_management_port_fixture_with_catalog_and_service(
        catalog: AvailableExtensionCatalog,
        lifecycle_service: ExtensionLifecycleService,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        Arc<RebornLocalExtensionManagementPort>,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
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
        Arc<RebornLocalExtensionManagementPort>,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");

        let mut filesystem = LocalFilesystem::new();
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
        let installation_store = Arc::new(InMemoryExtensionInstallationStore::default());
        let extension_management = Arc::new(
            RebornLocalExtensionManagementPort::new(
                root_filesystem,
                catalog,
                installation_store.clone(),
                Arc::new(Mutex::new(lifecycle_service)),
                test_active_extension_publisher(
                    Arc::clone(&active_registry),
                    test_extension_trust_policy(),
                ),
                None,
                lifecycle_owner(),
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

    async fn assert_removal_target_preserved(
        storage_root: &std::path::Path,
        installation_store: &InMemoryExtensionInstallationStore,
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
        Arc<RebornLocalExtensionManagementPort>,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
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
        Arc<RebornLocalExtensionManagementPort>,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");

        let mut filesystem = LocalFilesystem::new();
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
        let installation_store = Arc::new(InMemoryExtensionInstallationStore::default());
        let extension_management = Arc::new(RebornLocalExtensionManagementPort::new(
            root_filesystem,
            catalog,
            installation_store.clone(),
            Arc::new(Mutex::new(lifecycle_service)),
            test_active_extension_publisher(
                Arc::clone(&active_registry),
                Arc::clone(&trust_policy),
            ),
            None,
            lifecycle_owner(),
        ));
        (
            dir,
            storage_root,
            extension_management,
            active_registry,
            installation_store,
        )
    }

    fn extension_lifecycle_fixture_with_catalog_and_service(
        catalog: AvailableExtensionCatalog,
        lifecycle_service: ExtensionLifecycleService,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        crate::extension_host::lifecycle::RebornLocalLifecycleFacade,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
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
        crate::extension_host::lifecycle::RebornLocalLifecycleFacade,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
    ) {
        extension_lifecycle_fixture_with_all_cleanup(
            catalog,
            lifecycle_service,
            credential_cleanup,
            Arc::new(ExtensionRemovalCleanupRegistry::empty()),
        )
    }

    fn extension_lifecycle_fixture_with_all_cleanup(
        catalog: AvailableExtensionCatalog,
        lifecycle_service: ExtensionLifecycleService,
        credential_cleanup: Option<Arc<dyn ExtensionCredentialCleanup>>,
        removal_cleanup: Arc<ExtensionRemovalCleanupRegistry>,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        crate::extension_host::lifecycle::RebornLocalLifecycleFacade,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");

        let mut filesystem = LocalFilesystem::new();
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
        let skill_management = Arc::new(
            crate::extension_host::lifecycle::RebornLocalSkillManagementPort::new(
                UserId::new("lifecycle-owner").expect("valid user"),
                root_filesystem.clone(),
                MountView::new(vec![MountGrant::new(
                    MountAlias::new("/skills").expect("valid alias"),
                    VirtualPath::new("/projects/skills").expect("valid path"),
                    MountPermissions::read_write_list_delete(),
                )])
                .expect("valid mount view"),
            ),
        );
        let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let installation_store = Arc::new(InMemoryExtensionInstallationStore::default());
        let extension_management = Arc::new(
            RebornLocalExtensionManagementPort::new(
                root_filesystem,
                catalog,
                installation_store.clone(),
                Arc::new(Mutex::new(lifecycle_service)),
                test_active_extension_publisher(
                    Arc::clone(&active_registry),
                    test_extension_trust_policy(),
                ),
                credential_cleanup,
                lifecycle_owner(),
            )
            .with_removal_cleanup_registry(removal_cleanup),
        );
        let facade =
            crate::extension_host::lifecycle::RebornLocalLifecycleFacade::new(skill_management)
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
        RebornLocalExtensionManagementPort,
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
        RebornLocalExtensionManagementPort,
        Arc<SharedExtensionRegistry>,
        Arc<DeleteInstallationFailingStore>,
        Arc<HostTrustPolicy>,
    ) {
        extension_port_with_delete_failing_store(
            ExtensionRegistry::new(),
            DeleteInstallationFailingStore::fail_manifest_delete(),
        )
    }

    fn extension_port_with_set_activation_failing_store(
        lifecycle_service: ExtensionLifecycleService,
    ) -> (
        tempfile::TempDir,
        RebornLocalExtensionManagementPort,
        Arc<SharedExtensionRegistry>,
        Arc<DeleteInstallationFailingStore>,
        Arc<HostTrustPolicy>,
    ) {
        extension_port_with_failing_store(
            ExtensionRegistry::new(),
            DeleteInstallationFailingStore::fail_set_activation_enabled(),
            lifecycle_service,
        )
    }

    fn extension_port_with_delete_failing_store(
        initial_active_registry: ExtensionRegistry,
        failing_store: DeleteInstallationFailingStore,
    ) -> (
        tempfile::TempDir,
        RebornLocalExtensionManagementPort,
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
        RebornLocalExtensionManagementPort,
        Arc<SharedExtensionRegistry>,
        Arc<DeleteInstallationFailingStore>,
        Arc<HostTrustPolicy>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");
        let mut filesystem = LocalFilesystem::new();
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
        let installation_store: Arc<dyn ExtensionInstallationStore> = failing_store.clone();
        let port = RebornLocalExtensionManagementPort::new(
            root_filesystem,
            AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
            installation_store,
            Arc::new(Mutex::new(lifecycle_service)),
            test_active_extension_publisher(
                Arc::clone(&active_registry),
                Arc::clone(&trust_policy),
            ),
            None,
            lifecycle_owner(),
        );
        (dir, port, active_registry, failing_store, trust_policy)
    }

    fn extension_port_with_file_delete_failing_filesystem() -> (
        tempfile::TempDir,
        RebornLocalExtensionManagementPort,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
        Arc<HostTrustPolicy>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");
        let mut filesystem = LocalFilesystem::new();
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
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(filesystem);
        let root_filesystem: Arc<dyn RootFilesystem> =
            Arc::new(DeleteFailingRootFilesystem { inner: filesystem });
        let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let trust_policy = test_extension_trust_policy();
        let installation_store = Arc::new(InMemoryExtensionInstallationStore::default());
        let extension_installation_store: Arc<dyn ExtensionInstallationStore> =
            installation_store.clone();
        let port = RebornLocalExtensionManagementPort::new(
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
            lifecycle_owner(),
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

    #[derive(Default)]
    struct RecordingLifecycleSink {
        operations: std::sync::Mutex<Vec<ExtensionLifecycleOperation>>,
    }

    impl RecordingLifecycleSink {
        fn operations(&self) -> Vec<ExtensionLifecycleOperation> {
            self.operations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    #[async_trait]
    impl ExtensionLifecycleEventSink for RecordingLifecycleSink {
        async fn record_extension_lifecycle_event(
            &self,
            event: ExtensionLifecycleEvent,
        ) -> Result<(), ExtensionError> {
            self.operations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(event.operation);
            Ok(())
        }
    }

    #[derive(Default)]
    struct DeleteInstallationFailingStore {
        inner: InMemoryExtensionInstallationStore,
        fail_manifest_delete: bool,
        fail_set_activation_enabled: bool,
        fail_get_installation: bool,
        mismatched_get_installation: bool,
        /// #5459 P1: fail the NEXT `upsert_installation` once, then clear —
        /// simulates a mid-install persist failure so the retry can heal.
        fail_next_upsert_installation: std::sync::atomic::AtomicBool,
    }

    impl DeleteInstallationFailingStore {
        fn fail_manifest_delete() -> Self {
            Self {
                fail_manifest_delete: true,
                ..Self::default()
            }
        }

        fn fail_set_activation_enabled() -> Self {
            Self {
                fail_set_activation_enabled: true,
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
    impl ExtensionInstallationStore for DeleteInstallationFailingStore {
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

        async fn list_enabled_installations(
            &self,
        ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
            self.inner.list_enabled_installations().await
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
                    ExtensionActivationState::Installed,
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

        async fn set_activation_state(
            &self,
            installation_id: &ExtensionInstallationId,
            state: ExtensionActivationState,
        ) -> Result<(), ExtensionInstallationError> {
            if self.fail_set_activation_enabled && state == ExtensionActivationState::Enabled {
                return Err(ExtensionInstallationError::InvalidInstallation {
                    reason: "set activation state failed".to_string(),
                });
            }
            self.inner
                .set_activation_state(installation_id, state)
                .await
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

    async fn fixture_installation_state<S>(store: &S) -> ExtensionActivationState
    where
        S: ExtensionInstallationStore + ?Sized,
    {
        let installation_id = ExtensionInstallationId::new("fixture").expect("valid installation");
        store
            .get_installation(&installation_id)
            .await
            .expect("read fixture installation")
            .expect("fixture installation remains")
            .activation_state()
    }

    struct DeleteFailingRootFilesystem {
        inner: Arc<dyn RootFilesystem>,
    }

    #[async_trait]
    impl RootFilesystem for DeleteFailingRootFilesystem {
        fn capabilities(&self) -> ironclaw_filesystem::BackendCapabilities {
            self.inner.capabilities()
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            self.inner.list_dir(path).await
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            self.inner.stat(path).await
        }

        async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
            self.inner.read_file(path).await
        }

        async fn write_file(
            &self,
            path: &VirtualPath,
            bytes: &[u8],
        ) -> Result<(), FilesystemError> {
            self.inner.write_file(path, bytes).await
        }

        async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::Delete,
                reason: "delete failed".to_string(),
            })
        }
    }

    async fn assert_enabled_active_extension_state<S>(
        active_registry: &SharedExtensionRegistry,
        installation_store: &S,
    ) where
        S: ExtensionInstallationStore + ?Sized,
    {
        let extension_id = ExtensionId::new("fixture").expect("valid extension id");
        let installation_id = ExtensionInstallationId::new("fixture").expect("valid installation");
        let installation = installation_store
            .get_installation(&installation_id)
            .await
            .expect("read installation")
            .expect("installation remains");
        assert_eq!(
            installation.activation_state(),
            ExtensionActivationState::Enabled
        );
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
            let now = chrono::Utc::now();
            Ok(ironclaw_auth::CredentialAccount {
                id: ironclaw_auth::CredentialAccountId::new(),
                scope: ironclaw_auth::AuthProductScope::new(
                    ResourceScope::local_default(
                        UserId::new("credential-user").expect("valid user"),
                        InvocationId::new(),
                    )
                    .expect("valid scope"),
                    ironclaw_auth::AuthSurface::Api,
                ),
                provider: ironclaw_auth::AuthProviderId::new("test-provider")
                    .expect("valid provider"),
                label: ironclaw_auth::CredentialAccountLabel::new("test-provider")
                    .expect("valid label"),
                status: ironclaw_auth::CredentialAccountStatus::Configured,
                ownership: ironclaw_auth::CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(
                    ironclaw_host_api::SecretHandle::new("test-secret")
                        .expect("valid secret handle"),
                ),
                refresh_secret: None,
                scopes: Vec::new(),
                provider_identity: None,
                created_at: now,
                updated_at: now,
            })
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
    /// wired into every test `RebornLocalExtensionManagementPort`.
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
        package.surface_kinds = vec![LifecycleExtensionSurfaceKind::ExternalChannel];
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

[[capabilities]]
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

[[capabilities]]
id = "fixture.search"
description = "Search fixture data"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"

[[capabilities]]
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

    fn fixture_extension_package_from_manifest(manifest_toml: &str) -> AvailableExtensionPackage {
        fixture_extension_package_from_manifest_with_root(manifest_toml, "fixture")
    }

    fn fixture_extension_package_from_manifest_with_root(
        manifest_toml: &str,
        root_id: &str,
    ) -> AvailableExtensionPackage {
        let manifest = ExtensionManifest::parse(
            manifest_toml,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
        )
        .expect("fixture manifest");
        fixture_extension_package_from_parsed_manifest(manifest_toml, root_id, manifest)
    }

    fn fixture_extension_package_from_manifest_with_product_adapter_contracts(
        manifest_toml: &str,
        root_id: &str,
    ) -> AvailableExtensionPackage {
        let mut contracts = ironclaw_extensions::HostApiContractRegistry::new();
        contracts
            .register(Arc::new(
                ironclaw_product_adapter_registry::ProductAdapterHostApiContract::new()
                    .expect("product adapter host API contract"),
            ))
            .expect("register product adapter host API contract");
        let manifest = ExtensionManifest::parse_with_host_api_contracts(
            manifest_toml,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
            &contracts,
        )
        .expect("fixture manifest");
        fixture_extension_package_from_parsed_manifest(manifest_toml, root_id, manifest)
    }

    fn fixture_extension_package_from_parsed_manifest(
        manifest_toml: &str,
        root_id: &str,
        manifest: ExtensionManifest,
    ) -> AvailableExtensionPackage {
        let root =
            VirtualPath::new(format!("/system/extensions/{root_id}")).expect("extension root");
        let package = ExtensionPackage::from_manifest_toml(manifest, root, manifest_toml)
            .expect("fixture package");
        AvailableExtensionPackage {
            package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, root_id)
                .expect("fixture package ref"),
            manifest_toml: manifest_toml.to_string(),
            source: ManifestSource::HostBundled,
            package,
            cleanup_requirements: Vec::new(),
            surface_kinds: Vec::new(),
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
        ExtensionManifestRecord::from_toml_with_contracts(
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

    fn fixture_installation(
        manifest_hash: Option<String>,
        activation_state: ExtensionActivationState,
    ) -> ExtensionInstallation {
        let extension_id = ExtensionId::new("fixture").expect("valid extension id");
        ExtensionInstallation::new(
            ExtensionInstallationId::new("fixture").expect("valid installation"),
            extension_id.clone(),
            activation_state,
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

    fn assert_unsupported_extension_response(
        response: LifecycleProductResponse,
        expected_ref: &str,
    ) {
        assert_eq!(response.phase, LifecyclePhase::UnsupportedOrLegacy);
        assert!(response.blockers.iter().any(|blocker| matches!(
            blocker,
            LifecycleReadinessBlocker::Runtime { ref_id: Some(ref_id) }
                if ref_id.as_str() == expected_ref
        )));
    }
}
