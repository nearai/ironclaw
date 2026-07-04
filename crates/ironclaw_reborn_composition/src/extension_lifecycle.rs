use std::sync::Arc;

use ironclaw_extensions::{
    CapabilityVisibility, ExtensionActivationState, ExtensionError, ExtensionInstallation,
    ExtensionInstallationError, ExtensionInstallationId, ExtensionInstallationStore,
    ExtensionLifecycleService, ExtensionManifestRecord, ExtensionManifestRef, ExtensionPackage,
    ExtensionRuntime, InstallationOwner, ManifestHash, ManifestSource,
};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, EffectKind, ExtensionId, NetworkTargetPattern,
    PermissionMode, ResourceScope, RuntimeCredentialAuthRequirement, RuntimeCredentialRequirement,
    RuntimeHttpEgress, UserId, VirtualPath, sha256_digest_token,
};
use ironclaw_product_workflow::{
    ExtensionImportMode, LifecycleExtensionSummary, LifecycleInstallScope,
    LifecycleInstalledExtensionSummary, LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase,
    LifecycleProductPayload, LifecycleProductResponse, LifecycleSearchExtensionSummary,
    ProductWorkflowError,
};
use tokio::sync::{Mutex, RwLock};

mod active_publication;
#[cfg(test)]
mod hosted_mcp_test_support;

use crate::available_extensions::{
    AvailableExtensionCatalog, AvailableExtensionPackage, ExtensionAssetStash,
    extension_asset_paths, has_disk_sourced_module, imported_extension_package,
    list_extension_files, materialize_available_extension, materialize_extension_for_replace,
    visible_capability_ids,
};
use crate::extension_activation_credentials::{
    ExtensionActivationCredentialGate, RuntimeExtensionActivationCredentialGate,
    UnavailableExtensionActivationCredentialGate,
};
use crate::extension_credential_requirements::package_runtime_credential_auth_requirements;
use crate::lifecycle::response_with_payload;
use crate::mcp_discovery::{
    HostedMcpDiscoveryError, discover_hosted_mcp_package, is_hosted_http_mcp_package,
};

pub(crate) use active_publication::ActiveExtensionPublisher;
#[cfg(test)]
use active_publication::extension_trust_policy_input;

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
    /// The tenant operator identity (#5459 P1). In local-dev this is the base
    /// owner user (`IRONCLAW_REBORN_WEBUI_USER_ID` semantics); installs by this
    /// user derive [`InstallationOwner::Tenant`] (shared), installs by anyone
    /// else derive [`InstallationOwner::User`] (private). Resolved ONCE here —
    /// when P0 role wiring lands, this becomes a role-derived resolver instead
    /// of an identity comparison; callers do not re-derive admin-ness.
    tenant_operator_user_id: UserId,
}

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

/// Zip-bomb guards for [`unzip_extension_bundle`]: the HTTP route caps only the
/// COMPRESSED body (8 MiB), so these bound what an uploaded bundle may expand
/// to in memory. Generous for real tool bundles (wasm + schemas + prompts),
/// tight enough that a hostile upload cannot OOM the host.
const MAX_EXTENSION_BUNDLE_FILES: usize = 512;
const MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES: usize = 64 * 1024 * 1024;

/// Extract an uploaded tool bundle (a zip) into `(path, bytes)` pairs, guarding
/// against zip-slip: absolute paths, `..` traversal, and backslash separators
/// are rejected rather than trusted.
fn unzip_extension_bundle(bundle: &[u8]) -> Result<Vec<(String, Vec<u8>)>, ProductWorkflowError> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bundle)).map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("uploaded tool bundle is not a valid zip: {error}"),
        }
    })?;
    let mut files = Vec::new();
    let mut seen_names = std::collections::HashSet::new();
    let mut total_bytes = 0usize;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("uploaded tool bundle has a corrupt entry: {error}"),
            }
        })?;
        if !entry.is_file() {
            continue;
        }
        if files.len() >= MAX_EXTENSION_BUNDLE_FILES {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "uploaded tool bundle contains too many files (limit {MAX_EXTENSION_BUNDLE_FILES})"
                ),
            });
        }
        let name = entry.name().to_string();
        if name.is_empty()
            || name.starts_with('/')
            || name.contains('\\')
            || name.split('/').any(|component| component == "..")
        {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!("uploaded tool bundle contains an unsafe path: {name}"),
            });
        }
        // Defense in depth against duplicate entry names (`zip -g` / archive
        // concat). The advertised bundle digest is taken from the FIRST matching
        // asset while materialization keeps the LAST write on disk; if both ever
        // reached the catalog the two would disagree and the compiled-module
        // cache would miss forever. The current `zip` reader collapses duplicate
        // names to a single last-wins entry (see the
        // `unzip_extension_bundle_collapses_duplicate_entry_names` canary), so
        // this guard does not fire today — but it fails closed rather than depend
        // on that reader behavior surviving a version bump.
        if !seen_names.insert(name.clone()) {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!("uploaded tool bundle contains a duplicate entry: {name}"),
            });
        }
        // `take(allowance + 1)` bounds what a hostile entry can buffer: the
        // declared zip sizes are attacker-controlled lies, so the guard must sit
        // on the actual decompressed stream, never on entry metadata.
        let allowance = MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES - total_bytes;
        let mut bytes = Vec::new();
        entry
            .take(allowance as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| ProductWorkflowError::InvalidBindingRequest {
                reason: format!("failed to read `{name}` from the uploaded bundle: {error}"),
            })?;
        if bytes.len() > allowance {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "uploaded tool bundle expands past the {MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES}-byte decompressed limit"
                ),
            });
        }
        total_bytes += bytes.len();
        files.push((name, bytes));
    }
    Ok(files)
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
        let package_ref = LifecyclePackageRef::new(
            LifecyclePackageKind::Extension,
            installation.extension_id().as_str(),
        )?;
        let available = catalog.resolve(&package_ref)?;
        if let Err(hash_error) = validate_restored_manifest_hash(&installation, available) {
            migrate_host_bundled_manifest_hash(
                installation_store,
                available,
                &installation,
                hash_error,
            )
            .await?;
        }
        materialize_available_extension(filesystem.as_ref(), available).await?;
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

impl RebornLocalExtensionManagementPort {
    pub(crate) fn new(
        filesystem: Arc<dyn RootFilesystem>,
        catalog: AvailableExtensionCatalog,
        installation_store: Arc<dyn ExtensionInstallationStore>,
        lifecycle_service: Arc<Mutex<ExtensionLifecycleService>>,
        active_extensions: ActiveExtensionPublisher,
        tenant_operator_user_id: UserId,
    ) -> Self {
        Self {
            filesystem,
            catalog: Arc::new(RwLock::new(catalog)),
            installation_store,
            lifecycle_service,
            active_extensions,
            operation_lock: Arc::new(Mutex::new(())),
            tenant_operator_user_id,
        }
    }

    /// Derive who a NEW install belongs to (#5459 P1): the tenant operator
    /// installs for the whole tenant; anyone else installs privately.
    fn derive_owner(&self, caller: &UserId) -> InstallationOwner {
        if caller == &self.tenant_operator_user_id {
            InstallationOwner::Tenant
        } else {
            InstallationOwner::user(caller.clone())
        }
    }

    /// Fail-closed visibility check for lifecycle mutations on an existing
    /// installation: a user-private install is operable ONLY by its owner or
    /// the tenant operator. The error is deliberately the same "is not
    /// installed" shape a missing installation produces, so a foreign caller
    /// cannot distinguish (or enumerate) other users' private installs.
    fn ensure_caller_may_operate(
        &self,
        installation: &ExtensionInstallation,
        caller: &UserId,
    ) -> Result<(), ProductWorkflowError> {
        match installation.owner() {
            InstallationOwner::Tenant => Ok(()),
            InstallationOwner::User { user_id }
                if user_id == caller || caller == &self.tenant_operator_user_id =>
            {
                Ok(())
            }
            InstallationOwner::User { .. } => Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "extension {} is not installed",
                    installation.extension_id().as_str()
                ),
            }),
        }
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
        let catalog = self.catalog.read().await;
        let extensions = catalog.search(query);
        let mut summaries = Vec::new();
        for extension in extensions {
            summaries.push(
                self.search_summary(extension, credential_gate, caller)
                    .await?,
            );
        }
        drop(catalog);
        let count = summaries.len();
        let mut response = response_with_payload(
            None,
            LifecyclePhase::Discovered,
            LifecycleProductPayload::ExtensionSearch {
                extensions: summaries,
                count,
            },
        );
        if extension_search_has_ready_result(response.payload.as_ref()) {
            response.message = Some(
                "Search found installed extension results that are already configured or active. Treat those results as ready for this connection request; do not ask the user for credentials unless a later tool call reports auth_required."
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
            .and_then(|installation| install_scope_for_owner(installation.owner()));
        let summary = self.catalog.read().await.resolve(&package_ref)?.summary();
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
        let owner_by_extension = self
            .installation_store
            .list_enabled_installations()
            .await
            .map_err(map_extension_installation_error)?
            .into_iter()
            .map(|installation| {
                (
                    installation.extension_id().clone(),
                    installation.owner().clone(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
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
        Ok(self
            .installation_store
            .list_installations()
            .await
            .map_err(map_extension_installation_error)?
            .into_iter()
            .map(|installation| {
                (
                    installation.extension_id().clone(),
                    installation.owner().clone(),
                )
            })
            .collect())
    }

    pub(crate) async fn activation_credential_requirements(
        &self,
        package_ref: &LifecyclePackageRef,
    ) -> Result<Vec<RuntimeCredentialAuthRequirement>, ProductWorkflowError> {
        let (extension_id, installation_id) = extension_ids_from_package_ref(package_ref)?;
        let _operation_guard = self.operation_lock.lock().await;
        self.load_installation(&extension_id, &installation_id)
            .await?;
        let package = self.lifecycle_package(&extension_id).await?;
        Ok(package_runtime_credential_auth_requirements(&package))
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
            let catalog = self.catalog.read().await;
            let Ok(available) = catalog.resolve(&package_ref) else {
                continue;
            };
            summaries.push(LifecycleInstalledExtensionSummary {
                summary: available.summary(),
                phase: phase_for_activation_state(installation.activation_state()),
                install_scope: install_scope_for_owner(installation.owner()),
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
    /// survives a restart, and extends the in-memory catalog so it shows in the
    /// Registry immediately. The existing install/activate flow then operates on
    /// it like any other available extension.
    ///
    /// When the bundle's id is already INSTALLED, `mode` decides:
    /// - [`ExtensionImportMode::Add`] fails with a 409-mapped
    ///   [`ProductWorkflowError::ExtensionAlreadyInstalled`] instead of the
    ///   pre-#5459 silent behavior (overwriting a live extension's on-disk
    ///   assets and catalog entry with zero confirmation while the installed/
    ///   published state kept serving the old version — split-brain).
    /// - [`ExtensionImportMode::Replace`] performs the tenant-wide in-place
    ///   replacement (admin-only): activation state, credential bindings, and
    ///   owner survive; the published capability set swaps atomically. See
    ///   `docs/plans/2026-07-02-tenant-tool-replace-from-zip.md`.
    ///
    /// Locking: zip decode/validation is pure and stays outside all locks.
    /// The catalog WRITE lock is then taken before `operation_lock` — same
    /// relative order as `install` (catalog READ before `operation_lock`), so
    /// the two cannot deadlock. Both are held across the whole operation:
    /// replace mutates files + records + registries, and stalling catalog
    /// readers for the duration of one admin operation is an accepted cost.
    pub(crate) async fn import_bundle(
        &self,
        bundle: &[u8],
        mode: ExtensionImportMode,
        credential_gate: Option<&RuntimeExtensionActivationCredentialGate>,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let files = unzip_extension_bundle(bundle)?;
        let package = imported_extension_package(files)?;
        let package_ref = package.package_ref.clone();
        let extension_id = package.package.id.clone();

        let mut catalog = self.catalog.write().await;
        let _operation_guard = self.operation_lock.lock().await;

        let existing = self.search_installation(&extension_id).await?;
        let Some(existing) = existing else {
            // An orphaned manifest row with no installation row still counts
            // as an occupied slot (mirrors `ensure_slot_available`,
            // fail-closed): neither mode can safely write over it.
            if self
                .installation_store
                .get_manifest(&extension_id)
                .await
                .map_err(map_extension_installation_error)?
                .is_some()
            {
                return Err(ProductWorkflowError::ExtensionAlreadyInstalled {
                    reason: format!(
                        "extension {} has an orphaned installation record; remove it before importing",
                        extension_id.as_str()
                    ),
                });
            }
            // Vacant slot: plain import (both modes) — today's behavior.
            let summary = package.summary();
            materialize_available_extension(self.filesystem.as_ref(), &package).await?;
            catalog.extend(AvailableExtensionCatalog::from_packages(vec![package]));
            return Ok(response_with_payload(
                Some(package_ref),
                LifecyclePhase::Discovered,
                LifecycleProductPayload::ExtensionSearch {
                    extensions: vec![LifecycleSearchExtensionSummary {
                        summary,
                        installation_phase: None,
                    }],
                    count: 1,
                },
            ));
        };

        // Occupied slot. Import is an operator surface (route-gated on
        // `operator_webui_config`); if a non-operator caller ever reaches an
        // occupied slot here, fail closed without leaking who owns the id
        // (same masking rule as `ensure_slot_available`).
        if !matches!(self.derive_owner(caller), InstallationOwner::Tenant) {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!("extension id {} is unavailable", extension_id.as_str()),
            });
        }

        match mode {
            ExtensionImportMode::Add => Err(ProductWorkflowError::ExtensionAlreadyInstalled {
                reason: format!(
                    "extension {} is already installed; import with mode=replace to replace it tenant-wide",
                    extension_id.as_str()
                ),
            }),
            ExtensionImportMode::Replace => {
                self.replace_installed_bundle(&mut catalog, package, existing, credential_gate)
                    .await
            }
        }
    }

    /// Tenant-wide in-place replacement of an installed extension with a new
    /// bundle of the same id (#5459 P1.5). Two arms by slot owner:
    ///
    /// - `Tenant`-owned → hot swap preserving activation state, credential
    ///   bindings, and owner (the runtime mirror of the restart-time
    ///   `prepare_manifest_migration` path).
    /// - `User`-owned → the P1 admin-wins eviction, then a FRESH tenant
    ///   install of the new bundle. Activation state and bindings do not
    ///   transfer across owners by design; the evicted user's secrets are
    ///   never touched (supersede, don't destroy).
    ///
    /// Caller must already hold the catalog write guard and `operation_lock`.
    async fn replace_installed_bundle(
        &self,
        catalog: &mut AvailableExtensionCatalog,
        package: AvailableExtensionPackage,
        existing: ExtensionInstallation,
        credential_gate: Option<&RuntimeExtensionActivationCredentialGate>,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let extension_id = package.package.id.clone();
        let package_ref = package.package_ref.clone();

        if !matches!(
            package.package.manifest.runtime,
            ExtensionRuntime::Wasm { .. }
        ) {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "extension {} replace supports wasm tool bundles only",
                    extension_id.as_str()
                ),
            });
        }

        if matches!(existing.owner(), InstallationOwner::User { .. }) {
            return self
                .replace_evicting_private_installation(catalog, package, existing)
                .await;
        }

        let old_package = self.lifecycle_package(&extension_id).await?;
        // Hosted-MCP activation publishes the DISCOVERED package (inline
        // dynamic schemas from live discovery), not the catalog base package;
        // republishing a new base package would clobber that surface. Static
        // wasm bundles only, until replace re-runs discovery.
        if is_hosted_http_mcp_package(&old_package)
            || !matches!(old_package.manifest.runtime, ExtensionRuntime::Wasm { .. })
        {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "extension {} replace supports wasm tool bundles only",
                    extension_id.as_str()
                ),
            });
        }

        let was_enabled = existing.activation_state() == ExtensionActivationState::Enabled;
        let old_version = old_package.manifest.version.clone();
        let new_version = package.package.manifest.version.clone();
        let old_manifest = self
            .installation_store
            .get_manifest(&extension_id)
            .await
            .map_err(map_extension_installation_error)?
            .ok_or_else(|| ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "extension {} manifest is not installed",
                    extension_id.as_str()
                ),
            })?;

        // A hot-replace of an ENABLED install republishes the new capability set
        // straight into the active registry — bypassing the activation credential
        // gate the `activate` verb runs. A v2 that ADDS a required product-auth
        // credential the tenant hasn't configured would therefore go live
        // tenant-wide with unsatisfiable capabilities (every dispatch failing
        // credential staging), and the Enabled record would re-publish the same
        // broken surface on the next restart with no setup prompt ever shown.
        // Mirror the activate gate BEFORE any destructive step and fail closed:
        // configure the credentials (or deactivate) first, then replace.
        if was_enabled {
            let requirements = package_runtime_credential_auth_requirements(&package.package);
            if !requirements.is_empty() {
                let satisfied = match credential_gate {
                    Some(gate) => gate
                        .missing_requirements(requirements)
                        .await
                        .map_err(|error| ProductWorkflowError::InvalidBindingRequest {
                            reason: format!(
                                "extension {} product auth credential state is invalid: {error:?}",
                                extension_id.as_str()
                            ),
                        })?
                        .is_empty(),
                    // No credential-account service wired: we cannot confirm the
                    // new requirements are satisfied, so fail closed rather than
                    // publish an unsatisfiable surface.
                    None => false,
                };
                if !satisfied {
                    return Err(ProductWorkflowError::InvalidBindingRequest {
                        reason: format!(
                            "extension {} v{} requires product auth credentials that are not configured; configure them or deactivate the extension before replacing the enabled install",
                            extension_id.as_str(),
                            new_version
                        ),
                    });
                }
            }
        }

        // 1. Files: capture the prune baseline and the byte stash, then write
        //    with restore-on-failure semantics (never the delete-what-we-wrote
        //    rollback — that would destroy the live v1 files it overwrote and
        //    can brick the next restart).
        let old_files = list_extension_files(self.filesystem.as_ref(), &extension_id).await?;
        let stash = ExtensionAssetStash::capture(self.filesystem.as_ref(), &package).await?;
        materialize_extension_for_replace(self.filesystem.as_ref(), &package, &stash).await?;

        // 2. Lifecycle-registry swap. `update` validates capability-id
        //    collisions against OTHER extensions fail-closed and swaps the
        //    package's whole capability set atomically, preserving the
        //    disabled flag (remove+register would silently clear it).
        {
            let mut lifecycle = self.lifecycle_service.lock().await;
            if let Err(error) = lifecycle.update(package.package.clone()).await {
                drop(lifecycle);
                let error = map_extension_error(error);
                if let Err(restore_error) = stash.restore(self.filesystem.as_ref()).await {
                    return Err(compensation_failure(
                        "extension replace failed to update lifecycle package and file restore failed",
                        error,
                        restore_error,
                    ));
                }
                return Err(error);
            }
        }

        // 3. Durable records, migration shape: same installation id, new
        //    manifest hash, activation state + credential bindings + owner
        //    preserved (the restart-time migration's exact contract).
        let plan = prepare_manifest_migration(&package, &existing)?;
        if let Err(error) = self
            .installation_store
            .upsert_manifest_and_installation(plan.manifest_record, plan.installation)
            .await
        {
            let error = map_extension_installation_error(error);
            if let Err(restore_error) = self.restore_lifecycle_update(&old_package).await {
                return Err(compensation_failure(
                    "extension replace failed to persist records and lifecycle restore failed",
                    error,
                    restore_error,
                ));
            }
            if let Err(restore_error) = stash.restore(self.filesystem.as_ref()).await {
                return Err(compensation_failure(
                    "extension replace failed to persist records and file restore failed",
                    error,
                    restore_error,
                ));
            }
            return Err(error);
        }

        // 4. Republish for Enabled installs: trust AdminEntry re-pin + one
        //    atomic capability-set swap in the active registry. A FAILED
        //    publish removes the trust entry while the old package stays
        //    registered (old capabilities would fail trust closed), so the
        //    unwind must republish the old package, not merely stop.
        if was_enabled && let Err(error) = self.active_extensions.publish(&package.package) {
            if let Err(restore_error) = self.active_extensions.publish(&old_package) {
                return Err(compensation_failure(
                    "extension replace failed to republish and active publication restore failed",
                    error,
                    restore_error,
                ));
            }
            if let Err(restore_error) = self
                .restore_installation_records(old_manifest, existing)
                .await
            {
                return Err(compensation_failure(
                    "extension replace failed to republish and record restore failed",
                    error,
                    restore_error,
                ));
            }
            if let Err(restore_error) = self.restore_lifecycle_update(&old_package).await {
                return Err(compensation_failure(
                    "extension replace failed to republish and lifecycle restore failed",
                    error,
                    restore_error,
                ));
            }
            if let Err(restore_error) = stash.restore(self.filesystem.as_ref()).await {
                return Err(compensation_failure(
                    "extension replace failed to republish and file restore failed",
                    error,
                    restore_error,
                ));
            }
            return Err(error);
        }

        // 5. Catalog upsert LAST: it is in-memory and restart-rebuildable, so
        //    any failure above leaves it consistent with the still-v1 durable
        //    state (no catalog compensation needed).
        let summary = package.summary();
        let new_paths: std::collections::HashSet<String> = extension_asset_paths(&package)?
            .into_iter()
            .map(|path| path.as_str().to_string())
            .collect();
        catalog.extend(AvailableExtensionCatalog::from_packages(vec![package]));

        // 6. Prune files the new bundle no longer ships. The swap has already
        //    succeeded; a failed delete leaves the pre-#5459 status quo
        //    (inert garbage), so warn instead of unwinding a correct replace.
        for path in old_files {
            if !new_paths.contains(path.as_str())
                && let Err(error) = self.filesystem.delete(&path).await
            {
                tracing::warn!(
                    extension_id = %extension_id.as_str(),
                    path = %path.as_str(),
                    %error,
                    "failed to prune superseded extension file after replace"
                );
            }
        }

        tracing::warn!(
            extension_id = %extension_id.as_str(),
            old_version = %old_version,
            new_version = %new_version,
            enabled = was_enabled,
            "replaced tenant extension from imported bundle"
        );

        let mut response = response_with_payload(
            Some(package_ref),
            if was_enabled {
                LifecyclePhase::Active
            } else {
                LifecyclePhase::Installed
            },
            LifecycleProductPayload::ExtensionSearch {
                extensions: vec![LifecycleSearchExtensionSummary {
                    summary,
                    installation_phase: None,
                }],
                count: 1,
            },
        );
        response.message = Some(if was_enabled {
            format!(
                "Replaced extension {} v{} with v{} for the whole tenant. It is still active; the updated tools are live for every user without a restart.",
                extension_id.as_str(),
                old_version,
                new_version
            )
        } else {
            format!(
                "Replaced extension {} v{} with v{} for the whole tenant. It was not active; activate it to publish the updated tools.",
                extension_id.as_str(),
                old_version,
                new_version
            )
        });
        Ok(response)
    }

    /// Replace over a `User`-owned slot: admin-wins eviction (existing P1
    /// rule) followed by a fresh Tenant install of the new bundle. The fresh
    /// record starts at `Installed` with empty credential bindings — private
    /// activation/bindings deliberately do not transfer to the tenant slot.
    async fn replace_evicting_private_installation(
        &self,
        catalog: &mut AvailableExtensionCatalog,
        package: AvailableExtensionPackage,
        existing: ExtensionInstallation,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let extension_id = package.package.id.clone();
        let package_ref = package.package_ref.clone();

        // Verify the new bundle can take the slot BEFORE the destructive
        // eviction. `register_lifecycle_package` re-validates after eviction, but
        // by then the user's working install is already deregistered/unpublished
        // — a capability-id collision against ANOTHER extension (extension ids
        // may contain dots, so `a` v2 declaring `a.b.tool` collides with an
        // installed `a.b`) would then leave the user's tool dead with no way to
        // retry. `validate_replacement` runs the same collision check the swap
        // would, excluding the v1 package about to be superseded, so a colliding
        // bundle is rejected while the private install is still intact.
        {
            let lifecycle = self.lifecycle_service.lock().await;
            lifecycle
                .validate_replacement(&package.package)
                .map_err(map_extension_error)?;
        }

        self.evict_private_installation(&extension_id, &existing)
            .await?;
        let plan = prepare_install(&package, InstallationOwner::Tenant)?;
        self.register_lifecycle_package(&package.package).await?;

        let stash = ExtensionAssetStash::capture(self.filesystem.as_ref(), &package).await?;
        if let Err(error) =
            materialize_extension_for_replace(self.filesystem.as_ref(), &package, &stash).await
        {
            if let Err(rollback_error) = self.rollback_lifecycle_install(&extension_id).await {
                return Err(compensation_failure(
                    "extension replace-over-private materialization failed and lifecycle rollback failed",
                    error,
                    rollback_error,
                ));
            }
            return Err(error);
        }
        // Persist the fresh tenant records with the SAME atomic call the
        // tenant-owned arm uses. `persist_install_plan` would upsert the manifest
        // first, and the in-memory store validates EVERY stored installation of
        // this id against the new manifest — the just-evicted private row still
        // exists (eviction only flips it to Disabled) with the v1 hash, so it
        // fails `ManifestHashMismatch` against the v2 manifest for any real
        // upgrade. `upsert_manifest_and_installation` validates only the new
        // (manifest, installation) pair and overwrites the same-id row, flipping
        // the owner User -> Tenant in one step.
        if let Err(error) = self
            .installation_store
            .upsert_manifest_and_installation(plan.manifest_record, plan.installation)
            .await
            .map_err(map_extension_installation_error)
        {
            if let Err(restore_error) = stash.restore(self.filesystem.as_ref()).await {
                return Err(compensation_failure(
                    "extension replace-over-private persistence failed and file restore failed",
                    error,
                    restore_error,
                ));
            }
            if let Err(rollback_error) = self.rollback_lifecycle_install(&extension_id).await {
                return Err(compensation_failure(
                    "extension replace-over-private persistence failed and lifecycle rollback failed",
                    error,
                    rollback_error,
                ));
            }
            return Err(error);
        }

        let summary = package.summary();
        catalog.extend(AvailableExtensionCatalog::from_packages(vec![package]));

        let mut response = response_with_payload(
            Some(package_ref),
            LifecyclePhase::Installed,
            LifecycleProductPayload::ExtensionSearch {
                extensions: vec![LifecycleSearchExtensionSummary {
                    summary,
                    installation_phase: None,
                }],
                count: 1,
            },
        );
        response.message = Some(format!(
            "Replaced the private install of extension {} with the imported bundle; it is now shared tenant-wide. Activate it to publish the tools.",
            extension_id.as_str()
        ));
        Ok(response)
    }

    /// Compensation arm for a failed replace: swap the lifecycle registry
    /// back to the pre-replace package via the same atomic `update`.
    async fn restore_lifecycle_update(
        &self,
        old_package: &ExtensionPackage,
    ) -> Result<(), ProductWorkflowError> {
        let mut lifecycle = self.lifecycle_service.lock().await;
        lifecycle
            .update(old_package.clone())
            .await
            .map_err(map_extension_error)
    }

    pub(crate) async fn install(
        &self,
        package_ref: LifecyclePackageRef,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        // Read guard is held for the whole method because `available` borrows
        // from the catalog (used by prepare_install/materialize/visible_caps
        // below). Acquired BEFORE `operation_lock`; `import_bundle` takes the
        // write guard before `operation_lock` too, so the lock order is
        // consistent and the two cannot deadlock.
        let owner = self.derive_owner(caller);
        let catalog = self.catalog.read().await;
        let available = catalog.resolve(&package_ref)?;
        let plan = prepare_install(available, owner.clone())?;
        let _operation_guard = self.operation_lock.lock().await;
        self.ensure_slot_available(
            &available.package.id,
            plan.installation.installation_id(),
            &owner,
        )
        .await?;
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
            let _ = self
                .delete_materialized_extension_files(&available.package.id)
                .await;
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

        Ok(response_with_payload(
            Some(package_ref.clone()),
            LifecyclePhase::Installed,
            LifecycleProductPayload::ExtensionInstall {
                installed: true,
                visible_capability_ids: visible_capability_ids(available)
                    .map(|id| id.as_str().to_string())
                    .collect(),
                next_step: format!(
                    "Call builtin.extension_activate now with input {{\"extension_id\":\"{}\"}}. Activation publishes the tools and opens the auth gate if credentials are missing.",
                    package_ref.id.as_str()
                ),
            },
        ))
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
            crate::extension_activation_credentials::PrecheckedExtensionActivationCredentialGate;
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
            self.ensure_caller_may_operate(&installation, caller)?;
            let package = self.lifecycle_package(&extension_id).await?;
            credential_gate.ensure_credentials(&package).await?;
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
            .map_err(|_| hosted_mcp_changed_during_discovery_error())?;
        // #5459 P1: the slot may have changed hands while the lock was dropped
        // for discovery (eviction+reinstall / remove+reinstall reuse the same
        // installation id), so re-check ownership before committing — phase 1's
        // check is stale. A foreign row must not be flipped to Enabled under
        // this caller's action.
        self.ensure_caller_may_operate(&installation, caller)
            .map_err(|_| hosted_mcp_changed_during_discovery_error())?;
        let current_package = self
            .lifecycle_package(&extension_id)
            .await
            .map_err(|_| hosted_mcp_changed_during_discovery_error())?;
        if current_package != discovery.base_package {
            return Err(hosted_mcp_changed_during_discovery_error());
        };
        credential_gate.ensure_credentials(&active_package).await?;
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
        self.enable_lifecycle_package(extension_id).await?;
        if let Err(error) = self
            .installation_store
            .set_activation_state(installation_id, ExtensionActivationState::Enabled)
            .await
        {
            self.disable_lifecycle_package(extension_id).await;
            return Err(map_extension_installation_error(error));
        }
        if let Err(error) = self.active_extensions.publish(&active_package) {
            if previous_state != ExtensionActivationState::Enabled {
                self.disable_lifecycle_package(extension_id).await;
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

        let visible_capability_ids = package_visible_capability_ids(&active_package);

        let mut response = response_with_payload(
            Some(package_ref),
            LifecyclePhase::Active,
            LifecycleProductPayload::ExtensionActivate {
                activated: true,
                visible_capability_ids: visible_capability_ids.clone(),
            },
        );
        // Enumerate the now-available tools by exact name in the model-visible
        // message. Under progressive tool disclosure the model otherwise only
        // learns a *count* of deferred tools (the tool_search description) and
        // must guess a query to discover them — so after activating an extension
        // it frequently concludes the tools are "not exposed" and gives up
        // instead of using them. Handing over the exact capability ids lets it go
        // straight to tool_call(name=...) / tool_describe(name=...). This is
        // transient result text, NOT a promotion: the persistent advertised tool
        // surface still grows only when a tool is actually invoked (earned
        // promotion), so activating many extensions does not bloat the surface.
        response.message = Some(activation_success_message(&visible_capability_ids));
        Ok(response)
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

    pub(crate) async fn remove(
        &self,
        package_ref: LifecyclePackageRef,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let (extension_id, installation_id) = extension_ids_from_package_ref(&package_ref)?;
        let _operation_guard = self.operation_lock.lock().await;
        let installation = self
            .load_installation(&extension_id, &installation_id)
            .await?;
        self.ensure_caller_may_operate(&installation, caller)?;
        if installation.owner().is_tenant() && caller != &self.tenant_operator_user_id {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "extension {} is a shared tool; only the tenant admin can remove it",
                    extension_id.as_str()
                ),
            });
        }
        let manifest = self
            .installation_store
            .get_manifest(&extension_id)
            .await
            .map_err(map_extension_installation_error)?
            .ok_or_else(|| ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "extension {} manifest is not installed",
                    extension_id.as_str()
                ),
            })?;
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
        if let Err(error) = self.installation_store.delete_manifest(&extension_id).await {
            let original_error = map_extension_installation_error(error);
            if let Err(restore_error) = self
                .restore_lifecycle_package(&lifecycle_package, previous_state)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to delete manifest and lifecycle restore failed",
                    original_error,
                    restore_error,
                ));
            }
            if let Err(restore_error) =
                self.restore_active_publication(&lifecycle_package, previous_state)
            {
                return Err(compensation_failure(
                    "extension remove failed to delete manifest and active publication restore failed",
                    original_error,
                    restore_error,
                ));
            }
            if let Err(restore_error) = self.restore_installation(&installation).await {
                return Err(compensation_failure(
                    "extension remove failed to delete manifest and installation restore failed",
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
            if let Err(restore_error) = self
                .restore_installation_records(manifest, installation)
                .await
            {
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

    /// #5459 P1 slot rules — one installation slot per extension id per tenant,
    /// with a typed owner deciding who may claim an occupied slot:
    ///
    /// - vacant → anyone installs (owner already derived by the caller)
    /// - `Tenant`-owned → nobody re-installs over it (the update flow is a
    ///   separate concern); members see "already available as a shared tool"
    /// - `User`-owned → the owner sees "already installed"; OTHER users get a
    ///   generic "unavailable" (never leaking who holds the slot); a TENANT
    ///   install EVICTS the private install (admin-wins: a user must not be
    ///   able to squat an id against the whole tenant, and "two users want it
    ///   privately → admin installs it shared" self-heals through this rule)
    ///
    /// Eviction supersedes, never destroys: it unpublishes/deregisters the
    /// private install so the tenant install can take the slot, but touches no
    /// secret-store rows and no credential accounts — the evicted user's
    /// personal credentials keep resolving caller-first at dispatch.
    async fn ensure_slot_available(
        &self,
        extension_id: &ExtensionId,
        installation_id: &ExtensionInstallationId,
        claimant: &InstallationOwner,
    ) -> Result<(), ProductWorkflowError> {
        let existing = self
            .installation_store
            .get_installation(installation_id)
            .await
            .map_err(map_extension_installation_error)?;
        let Some(existing) = existing else {
            // No installation record; an orphaned manifest row still counts as
            // an occupied slot (pre-#5459 behavior, kept fail-closed).
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
            return Ok(());
        };
        match (existing.owner(), claimant) {
            (InstallationOwner::Tenant, InstallationOwner::User { .. }) => {
                Err(ProductWorkflowError::InvalidBindingRequest {
                    reason: format!(
                        "extension {} is already available as a shared tool",
                        extension_id.as_str()
                    ),
                })
            }
            (InstallationOwner::Tenant, InstallationOwner::Tenant) => {
                Err(ProductWorkflowError::InvalidBindingRequest {
                    reason: format!("extension {} is already installed", extension_id.as_str()),
                })
            }
            (InstallationOwner::User { user_id }, InstallationOwner::User { user_id: caller }) => {
                if user_id == caller {
                    Err(ProductWorkflowError::InvalidBindingRequest {
                        reason: format!("extension {} is already installed", extension_id.as_str()),
                    })
                } else {
                    // Generic wording: a foreign caller must not learn that a
                    // private install exists, let alone whose it is.
                    Err(ProductWorkflowError::InvalidBindingRequest {
                        reason: format!("extension id {} is unavailable", extension_id.as_str()),
                    })
                }
            }
            (InstallationOwner::User { .. }, InstallationOwner::Tenant) => {
                self.evict_private_installation(extension_id, &existing)
                    .await
            }
        }
    }

    /// Admin-wins eviction (#5459 P1): deregister/unpublish a user-private
    /// installation so a tenant install can take the slot. The subsequent
    /// install path overwrites the manifest + installation records via its
    /// normal upsert (same installation id).
    ///
    /// Retry-safe by construction: if a prior tenant install partially
    /// succeeded (evicted, then failed to materialize/persist and rolled the
    /// tenant package back out), the lifecycle package is already gone. Rather
    /// than dead-end at `lifecycle_package()` — which would leave the id
    /// un-installable/-removable tenant-wide until restart — eviction tolerates
    /// the absent package as already-done and returns Ok, so the admin's retry
    /// re-runs eviction as a no-op and reclaims the slot. Grant minting gates
    /// on the enabled-installation owner join, so the private capability is
    /// already denied the moment the row flips to Disabled here, independent of
    /// the active-registry publish state.
    async fn evict_private_installation(
        &self,
        extension_id: &ExtensionId,
        existing: &ExtensionInstallation,
    ) -> Result<(), ProductWorkflowError> {
        tracing::warn!(
            extension_id = %extension_id.as_str(),
            "tenant install is evicting a user-private installation (admin-wins slot rule)"
        );
        let was_enabled = existing.activation_state() == ExtensionActivationState::Enabled;
        let lifecycle_package = self.lifecycle_package(extension_id).await.ok();
        self.installation_store
            .set_activation_state(
                existing.installation_id(),
                ExtensionActivationState::Disabled,
            )
            .await
            .map_err(map_extension_installation_error)?;
        if let Some(lifecycle_package) = lifecycle_package {
            self.remove_lifecycle_package(extension_id).await?;
            if was_enabled {
                self.active_extensions.unpublish(&lifecycle_package)?;
            }
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

    async fn disable_lifecycle_package(&self, extension_id: &ExtensionId) {
        let _ = self
            .lifecycle_service
            .lock()
            .await
            .disable(extension_id)
            .await;
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

    async fn restore_installation_records(
        &self,
        manifest: ExtensionManifestRecord,
        installation: ExtensionInstallation,
    ) -> Result<(), ProductWorkflowError> {
        self.installation_store
            .upsert_manifest(manifest)
            .await
            .map_err(map_extension_installation_error)?;
        self.installation_store
            .upsert_installation(installation)
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
            let _ = self.installation_store.delete_manifest(&extension_id).await;
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
        self.filesystem
            .delete(&extension_root)
            .await
            .map_err(|error| ProductWorkflowError::Transient {
                reason: format!("failed to remove extension files: {error}"),
            })
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
    let contracts =
        ironclaw_host_runtime::default_host_api_contract_registry().map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("host API contract registry rejected extension install: {error}"),
            }
        })?;
    let manifest_record = ExtensionManifestRecord::from_toml_with_contracts(
        &available.manifest_toml,
        ManifestSource::HostBundled,
        &host_ports,
        Some(manifest_hash.clone()),
        &contracts,
    )
    .map_err(map_extension_installation_error)?;
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
    let contracts =
        ironclaw_host_runtime::default_host_api_contract_registry().map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("host API contract registry rejected manifest migration: {error}"),
            }
        })?;
    let manifest_record = ExtensionManifestRecord::from_toml_with_contracts(
        &available.manifest_toml,
        ManifestSource::HostBundled,
        &host_ports,
        Some(manifest_hash.clone()),
        &contracts,
    )
    .map_err(map_extension_installation_error)?;
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

    // Imported zip bundles are recorded as `HostBundled` too, but they are NOT
    // first-party: their module is read from disk, whereas a genuine first-party
    // bundled extension carries its module inline from the binary. A clean
    // import/replace always commits the record hash to match the on-disk
    // manifest, so a mismatch reaching here for a disk-sourced module means the
    // on-disk tree is torn or corrupt — e.g. a crash between the manifest and
    // module writes of a replace left a v2-manifest over v1-module chimera.
    // Only in-binary (inline) packages legitimately migrate on an upgrade; fail
    // closed for disk-sourced ones rather than bless v2 metadata over stale
    // module bytes.
    if has_disk_sourced_module(available) {
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

/// Build the model-visible activation success message.
///
/// When the activated extension publishes model-visible tools, the message
/// enumerates them by exact capability id so the model can invoke them directly
/// — closing the progressive-disclosure awareness gap where the model only sees
/// a *count* of deferred tools and gives up instead of discovering them.
fn activation_success_message(visible_capability_ids: &[String]) -> String {
    let mut message = String::from(
        "Extension activation succeeded and its tools are now available. No additional authorization or configuration is needed, including for write-capable tools, unless a later tool call reports auth_required. Do not ask the user for a token, OAuth, authorization, or configuration after activated=true.",
    );
    if !visible_capability_ids.is_empty() {
        message.push_str(
            " These tools are now callable by exact name — invoke one directly with tool_call(name=\"<tool>\", arguments={ ... }), or tool_describe(name=\"<tool>\") first if you need its full schema. Do NOT call tool_search for these; you already have their names: ",
        );
        message.push_str(&visible_capability_ids.join(", "));
        message.push('.');
    }
    message
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
fn install_scope_for_owner(owner: &InstallationOwner) -> Option<LifecycleInstallScope> {
    Some(match owner {
        InstallationOwner::Tenant => LifecycleInstallScope::Shared,
        InstallationOwner::User { .. } => LifecycleInstallScope::Private,
    })
}

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
        matches!(
            extension.installation_phase,
            Some(LifecyclePhase::Configured | LifecyclePhase::Active)
        ) && extension.summary.credential_requirements.is_empty()
            && extension.summary.onboarding.is_none()
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
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::hosted_mcp_test_support::HostedMcpDiscoveryEgress;
    use super::*;
    use crate::available_extensions::{
        AvailableExtensionAsset, AvailableExtensionAssetContent, AvailableExtensionPackage,
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
        CapabilityId, ExtensionLifecycleOperation, HostPath, HostPortCatalog, InvocationId,
        MountAlias, MountGrant, MountPermissions, MountView, NetworkMethod, ResourceScope,
        RuntimeCredentialAccountSetup, RuntimeHttpEgress, RuntimeHttpEgressError,
        RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, TenantId, TrustClass, UserId,
    };
    use ironclaw_host_runtime::{SPAWN_SUBAGENT_CAPABILITY_ID, builtin_first_party_package};
    use ironclaw_product_workflow::{
        LifecycleProductAction, LifecycleProductContext, LifecycleProductFacade,
        LifecycleProductSurfaceContext, LifecycleReadinessBlocker,
    };
    use ironclaw_trust::{HostTrustPolicy, InvalidationBus, TrustPolicy};

    #[test]
    fn activation_message_enumerates_published_tools_by_exact_name() {
        // Regression: the model only sees a *count* of deferred tools, so after
        // activating an extension it must be handed the exact tool names or it
        // assumes they are unavailable and gives up. The success message must name
        // every published capability and steer the model to direct invocation.
        let message = activation_success_message(&[
            "google-calendar.list_events".to_string(),
            "google-calendar.create_event".to_string(),
        ]);
        assert!(message.contains("google-calendar.list_events"));
        assert!(message.contains("google-calendar.create_event"));
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
        let message = activation_success_message(&[]);
        assert!(message.contains("Extension activation succeeded"));
        assert!(
            !message.contains("callable by exact name"),
            "no tools published ⇒ no direct-invocation guidance, got: {message}"
        );
    }

    /// Build an in-memory zip from `(entry_name, bytes)` pairs for
    /// [`unzip_extension_bundle`] boundary tests.
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

    /// The doc contract promises backslash separators are REJECTED; normalizing
    /// them instead silently accepts a path shape the guard claims to refuse.
    #[test]
    fn unzip_extension_bundle_rejects_backslash_entry_names() {
        let bundle = zip_bundle(&[("wasm\\module.wasm", b"x".as_slice())]);
        let error = unzip_extension_bundle(&bundle)
            .expect_err("backslash separators must be rejected, not normalized");
        assert!(
            format!("{error}").contains("unsafe path"),
            "unexpected error: {error}"
        );
    }

    /// Fix F canary: the `imported_extension_package` digest is taken from the
    /// FIRST matching asset while materialization writes the LAST — if the zip
    /// reader ever surfaced BOTH copies of a duplicate name, the advertised
    /// digest and the on-disk bytes would disagree and the compiled-module cache
    /// would miss forever. Today's `zip` reader collapses a duplicate name to a
    /// single LAST-wins entry, so the two always agree; this test locks that
    /// reader guarantee. If a version bump ever changes it, this fails and the
    /// `unzip_extension_bundle` duplicate guard becomes the live safety net.
    #[test]
    fn unzip_extension_bundle_collapses_duplicate_entry_names() {
        // The zip WRITER refuses to emit duplicate names, but `zip -g` / archive
        // concatenation produce them in the wild. Forge one: write two entries
        // with SAME-LENGTH names, then byte-patch the second name to collide with
        // the first (equal length keeps every header offset valid).
        let mut bundle = zip_bundle(&[
            ("wasm/dupe.a", b"first".as_slice()),
            ("wasm/dupe.b", b"second".as_slice()),
        ]);
        let needle = b"wasm/dupe.b";
        let replacement = b"wasm/dupe.a";
        assert_eq!(needle.len(), replacement.len());
        for start in 0..bundle.len().saturating_sub(needle.len()) {
            if &bundle[start..start + needle.len()] == needle {
                bundle[start..start + needle.len()].copy_from_slice(replacement);
            }
        }
        let files = unzip_extension_bundle(&bundle).expect("forged duplicate parses");
        let dupes: Vec<_> = files
            .iter()
            .filter(|(name, _)| name == "wasm/dupe.a")
            .collect();
        assert_eq!(
            dupes.len(),
            1,
            "reader must collapse a duplicate name to a single entry, got {dupes:?}"
        );
        // Last-wins: the surviving entry carries the SECOND copy's bytes, which is
        // exactly what materialization would write — so digest and disk agree.
        assert_eq!(
            dupes[0].1.as_slice(),
            b"second",
            "the surviving entry must be the last write"
        );
    }

    /// A small compressed upload must not be allowed to expand past the
    /// decompressed-bytes cap (zip bomb): the route body limit bounds only the
    /// COMPRESSED size, so the cap here is the actual memory guard.
    #[test]
    fn unzip_extension_bundle_caps_total_decompressed_bytes() {
        let oversized = vec![0u8; MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES + 1];
        let bundle = zip_bundle(&[("payload.bin", oversized.as_slice())]);
        assert!(
            bundle.len() < MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES,
            "test premise: the bomb must be small compressed"
        );
        let error = unzip_extension_bundle(&bundle)
            .expect_err("expansion past the decompressed cap must be rejected");
        assert!(
            format!("{error}").contains("expands past"),
            "unexpected error: {error}"
        );
    }

    /// Entry-count flooding is the other zip-bomb axis: many tiny entries.
    #[test]
    fn unzip_extension_bundle_caps_entry_count() {
        let names: Vec<String> = (0..=MAX_EXTENSION_BUNDLE_FILES)
            .map(|index| format!("assets/file-{index}.txt"))
            .collect();
        let entries: Vec<(&str, &[u8])> = names
            .iter()
            .map(|name| (name.as_str(), b"x".as_slice()))
            .collect();
        let bundle = zip_bundle(&entries);
        let error =
            unzip_extension_bundle(&bundle).expect_err("entry-count flooding must be rejected");
        assert!(
            format!("{error}").contains("too many files"),
            "unexpected error: {error}"
        );
    }

    /// #5459 P1 slot rules, driven through the facade (the caller surface the
    /// WebUI and agent-tool paths both enter):
    /// - a member's install is PRIVATE: invisible in others' lists, and
    ///   activate/remove/install by others fail without leaking that (or
    ///   whose) a private install exists
    /// - a member cannot remove a TENANT-shared tool
    /// - a tenant (operator) install EVICTS the private install (admin-wins),
    ///   after which everyone sees the shared tool
    #[tokio::test]
    async fn private_install_slot_rules_and_admin_eviction() {
        let (_dir, _root, facade, _registry, installation_store) = extension_lifecycle_fixture();
        let fixture_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("fixture ref");
        let installation_id =
            ExtensionInstallationId::new("fixture").expect("fixture installation id");

        // alice (member) installs → owner is User(alice) in the store.
        let response = facade
            .execute(
                lifecycle_surface_context_for_user("alice"),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: fixture_ref.clone(),
                },
            )
            .await
            .expect("alice installs privately");
        assert_eq!(response.phase, LifecyclePhase::Installed);
        let installation = installation_store
            .get_installation(&installation_id)
            .await
            .expect("store read")
            .expect("installation row");
        assert_eq!(
            installation.owner().as_user().map(UserId::as_str),
            Some("alice"),
            "member install must be user-owned"
        );

        // bob's list is empty; alice's list shows a PRIVATE entry.
        let bob_list = facade
            .execute(
                lifecycle_surface_context_for_user("bob"),
                LifecycleProductAction::ExtensionList,
            )
            .await
            .expect("bob lists");
        let Some(LifecycleProductPayload::ExtensionList { count: 0, .. }) =
            bob_list.payload.as_ref()
        else {
            panic!("alice's private install must be invisible to bob: {bob_list:?}");
        };
        let alice_list = facade
            .execute(
                lifecycle_surface_context_for_user("alice"),
                LifecycleProductAction::ExtensionList,
            )
            .await
            .expect("alice lists");
        let Some(LifecycleProductPayload::ExtensionList {
            extensions,
            count: 1,
        }) = alice_list.payload.as_ref()
        else {
            panic!("alice must see her own install: {alice_list:?}");
        };
        assert_eq!(
            extensions[0].install_scope,
            Some(LifecycleInstallScope::Private)
        );

        // bob cannot claim the slot — and the error must not leak the owner.
        let error = facade
            .execute(
                lifecycle_surface_context_for_user("bob"),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: fixture_ref.clone(),
                },
            )
            .await
            .expect_err("bob cannot claim an id held by another user's private install");
        let rendered = error.to_string();
        assert!(rendered.contains("unavailable"), "unexpected: {rendered}");
        assert!(
            !rendered.contains("alice"),
            "slot error must not leak the private owner: {rendered}"
        );

        // bob cannot activate or remove it — reads as not installed.
        for action in [
            LifecycleProductAction::ExtensionActivate {
                package_ref: fixture_ref.clone(),
            },
            LifecycleProductAction::ExtensionRemove {
                package_ref: fixture_ref.clone(),
            },
        ] {
            let error = facade
                .execute(lifecycle_surface_context_for_user("bob"), action)
                .await
                .expect_err("foreign private install must be inoperable");
            assert!(
                error.to_string().contains("is not installed"),
                "unexpected: {error}"
            );
        }

        // alice activates her private install.
        let response = facade
            .execute(
                lifecycle_surface_context_for_user("alice"),
                LifecycleProductAction::ExtensionActivate {
                    package_ref: fixture_ref.clone(),
                },
            )
            .await
            .expect("alice activates her private install");
        assert_eq!(response.phase, LifecyclePhase::Active);

        // The operator installs the same id → evicts alice's private install.
        let response = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: fixture_ref.clone(),
                },
            )
            .await
            .expect("tenant install evicts the private install");
        assert_eq!(response.phase, LifecyclePhase::Installed);
        let installation = installation_store
            .get_installation(&installation_id)
            .await
            .expect("store read")
            .expect("installation row");
        assert!(
            installation.owner().is_tenant(),
            "tenant install must own the slot after eviction"
        );

        // Everyone now sees the SHARED entry — bob included.
        let bob_list = facade
            .execute(
                lifecycle_surface_context_for_user("bob"),
                LifecycleProductAction::ExtensionList,
            )
            .await
            .expect("bob lists after eviction");
        let Some(LifecycleProductPayload::ExtensionList {
            extensions,
            count: 1,
        }) = bob_list.payload.as_ref()
        else {
            panic!("shared install must be visible to bob: {bob_list:?}");
        };
        assert_eq!(
            extensions[0].install_scope,
            Some(LifecycleInstallScope::Shared)
        );

        // Members (alice included) cannot remove the shared tool; the operator can.
        for member in ["alice", "bob"] {
            let error = facade
                .execute(
                    lifecycle_surface_context_for_user(member),
                    LifecycleProductAction::ExtensionRemove {
                        package_ref: fixture_ref.clone(),
                    },
                )
                .await
                .expect_err("members cannot remove a shared tool");
            assert!(
                error.to_string().contains("only the tenant admin"),
                "unexpected: {error}"
            );
        }
        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove {
                    package_ref: fixture_ref,
                },
            )
            .await
            .expect("operator removes the shared tool");
    }

    /// #5459 P1: the owner join in `active_model_visible_capabilities` — a
    /// privately installed+activated extension's capabilities carry the
    /// owning user, which is what the grant-minting filter keys on.
    #[tokio::test]
    async fn active_capabilities_carry_installation_owner() {
        let (_dir, _root, port, _registry, _store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let alice = UserId::new("alice").expect("valid user");
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("fixture ref");
        port.install(package_ref.clone(), &alice)
            .await
            .expect("alice installs privately");
        port.activate(package_ref, ExtensionActivationMode::Static, &alice)
            .await
            .expect("alice activates");

        let capabilities = port
            .active_model_visible_capabilities()
            .await
            .expect("active capabilities");
        assert!(!capabilities.is_empty(), "fixture capability published");
        for capability in &capabilities {
            assert_eq!(
                capability.owner.as_user().map(UserId::as_str),
                Some("alice"),
                "capability must carry the private owner for grant filtering"
            );
        }

        // The operator/settings tool catalog joins THIS owner map to hide a
        // foreign user's private tool (#5459 P1 leak fix). Pin that the map
        // reports the private owner keyed by extension id.
        let owners = port
            .installation_owners()
            .await
            .expect("installation owners");
        assert_eq!(
            owners
                .get(&ExtensionId::new("fixture").unwrap())
                .and_then(InstallationOwner::as_user)
                .map(UserId::as_str),
            Some("alice"),
            "installation_owners must report the private owner the catalog filters on"
        );
    }

    /// #5459 P1 (should-fix): a tenant install that fails AFTER eviction has
    /// deregistered the private package must not brick the id tenant-wide.
    /// Eviction is retry-safe, so the admin's retry heals the slot to Tenant.
    #[tokio::test]
    async fn admin_install_retry_heals_after_eviction_then_persist_failure() {
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
        // Concrete Arc so the test can arm the one-shot persist failure AFTER
        // alice's install; the port sees it as `dyn ExtensionInstallationStore`.
        let store = Arc::new(DeleteInstallationFailingStore::default());
        let store_dyn: Arc<dyn ExtensionInstallationStore> = store.clone();
        let port = RebornLocalExtensionManagementPort::new(
            root_filesystem,
            AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
            store_dyn,
            Arc::new(Mutex::new(ExtensionLifecycleService::new(
                ExtensionRegistry::new(),
            ))),
            test_active_extension_publisher(
                Arc::clone(&active_registry),
                test_extension_trust_policy(),
            ),
            lifecycle_owner(),
        );

        let alice = UserId::new("alice").expect("user");
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("fixture ref");

        // alice privately installs + activates (Enabled, package published).
        port.install(package_ref.clone(), &alice)
            .await
            .expect("alice installs privately");
        port.activate(package_ref.clone(), ExtensionActivationMode::Static, &alice)
            .await
            .expect("alice activates");

        // Admin install fails at persist (upsert_installation), AFTER eviction
        // has already deregistered alice's package.
        store
            .fail_next_upsert_installation
            .store(true, std::sync::atomic::Ordering::SeqCst);
        port.install(package_ref.clone(), &lifecycle_owner())
            .await
            .expect_err("persist failure aborts the tenant install");

        // Retry: eviction is now a no-op (package already gone) and the slot
        // heals to a tenant-owned install rather than dead-ending on
        // 'not installed'.
        port.install(package_ref, &lifecycle_owner())
            .await
            .expect("admin retry heals the slot after a partial eviction");

        let owners = port.installation_owners().await.expect("owners");
        assert!(
            owners
                .get(&ExtensionId::new("fixture").unwrap())
                .expect("fixture installed")
                .is_tenant(),
            "the slot must heal to a tenant install, not stay bricked"
        );
    }

    /// Manifest for a keyless, no-network wasm tool with the given capability
    /// short-names (each becomes `<id>.<name>`). Imports cleanly through the
    /// real host port catalog (same shape as the ascii-renderer fixture).
    fn replace_test_manifest(id: &str, version: &str, capabilities: &[&str]) -> String {
        let mut manifest = format!(
            "schema_version = \"reborn.extension_manifest.v2\"\n\
             id = \"{id}\"\n\
             name = \"Replace Test\"\n\
             version = \"{version}\"\n\
             description = \"Replace test fixture\"\n\
             trust = \"first_party_requested\"\n\n\
             [runtime]\n\
             kind = \"wasm\"\n\
             module = \"wasm/tool.wasm\"\n"
        );
        for cap in capabilities {
            manifest.push_str(&format!(
                "\n[[capabilities]]\n\
                 id = \"{id}.{cap}\"\n\
                 description = \"Capability {cap}\"\n\
                 effects = [\"dispatch_capability\"]\n\
                 default_permission = \"allow\"\n\
                 visibility = \"model\"\n\
                 input_schema_ref = \"schemas/{cap}.input.json\"\n\
                 output_schema_ref = \"schemas/{cap}.output.json\"\n"
            ));
        }
        manifest
    }

    /// Zip a replace-test bundle. `module_bytes` lets a test change the wasm
    /// content between versions to exercise content-aware behavior.
    fn replace_test_bundle(manifest: &str, module_bytes: &[u8]) -> Vec<u8> {
        zip_bundle(&[
            ("manifest.toml", manifest.as_bytes()),
            ("wasm/tool.wasm", module_bytes),
            ("schemas/one.input.json", b"{}".as_slice()),
            ("schemas/one.output.json", b"{}".as_slice()),
            ("schemas/two.input.json", b"{}".as_slice()),
            ("schemas/two.output.json", b"{}".as_slice()),
        ])
    }

    fn active_capability_ids(capabilities: &[ActiveExtensionCapability]) -> BTreeSet<String> {
        capabilities
            .iter()
            .map(|capability| capability.id.as_str().to_string())
            .collect()
    }

    /// Zip a replace-test bundle for an arbitrary id + capability set, emitting
    /// the schema files each capability's manifest references so the bundle is
    /// self-consistent for ids/caps outside the fixed `one`/`two` set.
    fn replace_bundle_for(id: &str, version: &str, caps: &[&str], module: &[u8]) -> Vec<u8> {
        let manifest = replace_test_manifest(id, version, caps);
        let mut owned: Vec<(String, Vec<u8>)> = vec![
            ("manifest.toml".to_string(), manifest.into_bytes()),
            ("wasm/tool.wasm".to_string(), module.to_vec()),
        ];
        for cap in caps {
            owned.push((format!("schemas/{cap}.input.json"), b"{}".to_vec()));
            owned.push((format!("schemas/{cap}.output.json"), b"{}".to_vec()));
        }
        let refs: Vec<(&str, &[u8])> = owned
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
            .collect();
        zip_bundle(&refs)
    }

    /// A replace-test manifest whose first capability declares a REQUIRED
    /// product-auth credential (`use_secret` effect + `product_auth_account`
    /// source) — used to exercise the credential gate on an enabled replace.
    fn replace_test_manifest_with_credential(id: &str, version: &str) -> String {
        format!(
            "schema_version = \"reborn.extension_manifest.v2\"\n\
             id = \"{id}\"\n\
             name = \"Replace Test\"\n\
             version = \"{version}\"\n\
             description = \"Replace test fixture\"\n\
             trust = \"first_party_requested\"\n\n\
             [runtime]\n\
             kind = \"wasm\"\n\
             module = \"wasm/tool.wasm\"\n\n\
             [[capabilities]]\n\
             id = \"{id}.one\"\n\
             description = \"Capability one\"\n\
             effects = [\"dispatch_capability\", \"use_secret\"]\n\
             default_permission = \"allow\"\n\
             visibility = \"model\"\n\
             input_schema_ref = \"schemas/one.input.json\"\n\
             output_schema_ref = \"schemas/one.output.json\"\n\
             runtime_credentials = [ {{ handle = \"github_runtime_token\", source = {{ type = \"product_auth_account\", provider = \"github\" }}, audience = {{ scheme = \"https\", host_pattern = \"api.github.com\" }}, target = {{ type = \"header\", name = \"authorization\", prefix = \"Bearer \" }} }} ]\n\n\
             [[capabilities]]\n\
             id = \"{id}.two\"\n\
             description = \"Capability two\"\n\
             effects = [\"dispatch_capability\"]\n\
             default_permission = \"allow\"\n\
             visibility = \"model\"\n\
             input_schema_ref = \"schemas/two.input.json\"\n\
             output_schema_ref = \"schemas/two.output.json\"\n"
        )
    }

    /// Fix A: an imported bundle that DECLARES a wasm module but omits it from
    /// the zip must be rejected at import — before this check the missing file
    /// silently imports, a later replace prunes the live previous module, and the
    /// next restart fails the whole runtime build on the absent file.
    #[tokio::test]
    async fn import_rejects_bundle_missing_declared_wasm_module() {
        let (_dir, _root, port, _registry, _store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let admin = lifecycle_owner();
        let manifest = replace_test_manifest("replace-me", "0.1.0", &["one"]);
        // manifest.toml + schemas, but deliberately NO wasm/tool.wasm entry.
        let bundle = zip_bundle(&[
            ("manifest.toml", manifest.as_bytes()),
            ("schemas/one.input.json", b"{}".as_slice()),
            ("schemas/one.output.json", b"{}".as_slice()),
        ]);
        let error = port
            .import_bundle(&bundle, ExtensionImportMode::Add, None, &admin)
            .await
            .expect_err("a bundle missing its declared module must be rejected");
        assert!(
            matches!(&error, ProductWorkflowError::InvalidBindingRequest { reason }
                if reason.contains("wasm/tool.wasm") && reason.contains("does not contain")),
            "expected missing-module rejection, got {error:?}"
        );
    }

    /// Fix B: an admin replace over a member's PRIVATE (User-owned) install must
    /// succeed and transfer the slot to the tenant — not fail closed with
    /// `ManifestHashMismatch` (the pre-fix bug that upserted the manifest while
    /// the evicted private row still carried the old hash).
    #[tokio::test]
    async fn import_replace_over_private_install_transfers_slot_to_tenant() {
        let (_dir, _root, port, _registry, store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let admin = lifecycle_owner();
        let member = UserId::new("member").expect("member");
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "replace-me")
            .expect("replace-me ref");

        let v1 = replace_test_manifest("replace-me", "0.1.0", &["one"]);
        port.import_bundle(
            &replace_test_bundle(&v1, b"\0asm\x01\0\0\0v1"),
            ExtensionImportMode::Add,
            None,
            &admin,
        )
        .await
        .expect("v1 import into a vacant slot");
        port.install(package_ref, &member)
            .await
            .expect("member installs v1 privately");
        let before = store
            .get_installation(&ExtensionInstallationId::new("replace-me").unwrap())
            .await
            .expect("store read")
            .expect("installed");
        assert!(
            matches!(before.owner(), InstallationOwner::User { .. }),
            "precondition: member owns the private slot"
        );

        let v2 = replace_test_manifest("replace-me", "0.2.0", &["one", "two"]);
        let response = port
            .import_bundle(
                &replace_test_bundle(&v2, b"\0asm\x01\0\0\0v2"),
                ExtensionImportMode::Replace,
                None,
                &admin,
            )
            .await
            .expect("admin replace over a private install must succeed");
        assert_eq!(response.phase, LifecyclePhase::Installed);

        let after = store
            .get_installation(&ExtensionInstallationId::new("replace-me").unwrap())
            .await
            .expect("store read")
            .expect("installed");
        assert!(
            after.owner().is_tenant(),
            "the slot must transfer to the tenant, got {:?}",
            after.owner()
        );
        assert_eq!(
            after.activation_state(),
            ExtensionActivationState::Installed,
            "a fresh tenant install lands at Installed"
        );
    }

    /// Fix E: when the replacement bundle's capability set collides with ANOTHER
    /// installed extension, the replace over a private slot must be rejected
    /// BEFORE the destructive eviction — the member's working install must
    /// survive intact (not be evicted with no way to retry).
    #[tokio::test]
    async fn import_replace_over_private_install_rejects_capability_collision_without_evicting() {
        let (_dir, _root, port, _registry, store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let admin = lifecycle_owner();
        let member = UserId::new("member").expect("member");

        // An installed sibling extension `alpha.beta` owns capability id
        // `alpha.beta.tool`.
        port.import_bundle(
            &replace_bundle_for("alpha.beta", "0.1.0", &["tool"], b"\0asm\x01\0\0\0ab"),
            ExtensionImportMode::Add,
            None,
            &admin,
        )
        .await
        .expect("import sibling alpha.beta");
        port.install(
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "alpha.beta")
                .expect("alpha.beta ref"),
            &admin,
        )
        .await
        .expect("install sibling alpha.beta");

        // A member privately installs extension `alpha` v1 (capability alpha.one).
        port.import_bundle(
            &replace_bundle_for("alpha", "0.1.0", &["one"], b"\0asm\x01\0\0\0a1"),
            ExtensionImportMode::Add,
            None,
            &admin,
        )
        .await
        .expect("import alpha v1");
        port.install(
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "alpha").expect("alpha ref"),
            &member,
        )
        .await
        .expect("member installs alpha privately");

        // v2 of `alpha` adds capability `beta.tool` -> id `alpha.beta.tool`,
        // colliding with the installed sibling. The replace must be rejected
        // before eviction.
        let error = port
            .import_bundle(
                &replace_bundle_for(
                    "alpha",
                    "0.2.0",
                    &["one", "beta.tool"],
                    b"\0asm\x01\0\0\0a2",
                ),
                ExtensionImportMode::Replace,
                None,
                &admin,
            )
            .await
            .expect_err("a colliding replace must be rejected");
        assert!(
            matches!(error, ProductWorkflowError::InvalidBindingRequest { .. }),
            "expected an InvalidBindingRequest, got {error:?}"
        );

        let alpha = store
            .get_installation(&ExtensionInstallationId::new("alpha").unwrap())
            .await
            .expect("store read")
            .expect("member install must survive");
        assert!(
            matches!(alpha.owner(), InstallationOwner::User { .. }),
            "the member's private install must NOT be evicted, owner={:?}",
            alpha.owner()
        );
        assert_eq!(
            alpha.activation_state(),
            ExtensionActivationState::Installed,
            "eviction would have set the row to Disabled; it must be untouched"
        );
    }

    /// Fix D: replacing an ENABLED install with a v2 that adds a REQUIRED
    /// product-auth credential — with no credential-account service wired to
    /// confirm it — must fail closed rather than republish an unsatisfiable
    /// surface (which would also re-publish broken on the next restart).
    #[tokio::test]
    async fn import_replace_enabled_install_requiring_unconfigured_credentials_is_rejected() {
        let (_dir, _root, port, _registry, store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let admin = lifecycle_owner();
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "replace-me")
            .expect("replace-me ref");

        let v1 = replace_test_manifest("replace-me", "0.1.0", &["one"]);
        port.import_bundle(
            &replace_test_bundle(&v1, b"\0asm\x01\0\0\0v1"),
            ExtensionImportMode::Add,
            None,
            &admin,
        )
        .await
        .expect("v1 import");
        port.install(package_ref.clone(), &admin)
            .await
            .expect("install v1");
        port.activate(package_ref, ExtensionActivationMode::Static, &admin)
            .await
            .expect("activate v1");

        let v2 = replace_test_manifest_with_credential("replace-me", "0.2.0");
        let error = port
            .import_bundle(
                &replace_test_bundle(&v2, b"\0asm\x01\0\0\0v2"),
                ExtensionImportMode::Replace,
                None,
                &admin,
            )
            .await
            .expect_err("enabled replace adding unconfigured credentials must be rejected");
        assert!(
            matches!(&error, ProductWorkflowError::InvalidBindingRequest { reason }
                if reason.contains("credential")),
            "expected a credential rejection, got {error:?}"
        );

        // The live install is untouched: still v1, still Enabled, old caps.
        let installation = store
            .get_installation(&ExtensionInstallationId::new("replace-me").unwrap())
            .await
            .expect("store read")
            .expect("installed");
        assert_eq!(
            installation.activation_state(),
            ExtensionActivationState::Enabled,
            "the enabled install must be untouched by a rejected replace"
        );
        let caps = active_capability_ids(
            &port
                .active_model_visible_capabilities()
                .await
                .expect("caps"),
        );
        assert!(
            caps.contains("replace-me.one") && !caps.contains("replace-me.two"),
            "v1's surface must remain published and v2 must not have gone live: {caps:?}"
        );
    }

    /// Import `mode=add` of an ALREADY-INSTALLED id must fail with the typed
    /// conflict (409) instead of the pre-#5459 silent clobber of the live
    /// extension's on-disk assets — and it must NOT overwrite those assets.
    #[tokio::test]
    async fn import_add_mode_conflicts_on_installed_id_without_clobbering() {
        let (_dir, storage_root, port, _registry, _store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let admin = lifecycle_owner();
        let v1 = replace_test_manifest("replace-me", "0.1.0", &["one"]);
        port.import_bundle(
            &replace_test_bundle(&v1, b"\0asm\x01\0\0\0v1"),
            ExtensionImportMode::Add,
            None,
            &admin,
        )
        .await
        .expect("v1 import into a vacant slot");
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "replace-me")
            .expect("replace-me ref");
        port.install(package_ref, &admin).await.expect("install v1");

        let v2 = replace_test_manifest("replace-me", "0.2.0", &["one", "two"]);
        let error = port
            .import_bundle(
                &replace_test_bundle(&v2, b"\0asm\x01\0\0\0v2"),
                ExtensionImportMode::Add,
                None,
                &admin,
            )
            .await
            .expect_err("add-mode import of an installed id must conflict");
        assert!(
            matches!(
                error,
                ProductWorkflowError::ExtensionAlreadyInstalled { .. }
            ),
            "expected ExtensionAlreadyInstalled, got {error:?}"
        );

        // The live wasm on disk must still be v1 — add-mode must not have
        // materialized the v2 bytes.
        let module =
            std::fs::read(storage_root.join("system/extensions/replace-me/wasm/tool.wasm"))
                .expect("materialized module");
        assert_eq!(
            module, b"\0asm\x01\0\0\0v1",
            "add-mode conflict must not overwrite the installed extension's assets"
        );
    }

    /// Replace of an ENABLED tenant install swaps the published capability set
    /// atomically (a new capability appears) while activation state and owner
    /// survive — the runtime mirror of the restart-time manifest migration.
    #[tokio::test]
    async fn import_replace_swaps_capabilities_and_preserves_active_state() {
        let (_dir, _root, port, _registry, store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let admin = lifecycle_owner();
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "replace-me")
            .expect("replace-me ref");

        let v1 = replace_test_manifest("replace-me", "0.1.0", &["one"]);
        port.import_bundle(
            &replace_test_bundle(&v1, b"\0asm\x01\0\0\0v1"),
            ExtensionImportMode::Add,
            None,
            &admin,
        )
        .await
        .expect("v1 import");
        port.install(package_ref.clone(), &admin)
            .await
            .expect("install v1");
        port.activate(package_ref.clone(), ExtensionActivationMode::Static, &admin)
            .await
            .expect("activate v1");

        let before = active_capability_ids(
            &port
                .active_model_visible_capabilities()
                .await
                .expect("v1 caps"),
        );
        assert!(before.contains("replace-me.one"));
        assert!(!before.contains("replace-me.two"));

        let v2 = replace_test_manifest("replace-me", "0.2.0", &["one", "two"]);
        let response = port
            .import_bundle(
                &replace_test_bundle(&v2, b"\0asm\x01\0\0\0v2"),
                ExtensionImportMode::Replace,
                None,
                &admin,
            )
            .await
            .expect("replace v1 with v2");
        assert_eq!(
            response.phase,
            LifecyclePhase::Active,
            "an enabled install stays active after replace"
        );

        let after = active_capability_ids(
            &port
                .active_model_visible_capabilities()
                .await
                .expect("v2 caps"),
        );
        assert!(
            after.contains("replace-me.one") && after.contains("replace-me.two"),
            "v2's added capability must be published: {after:?}"
        );

        let installation = store
            .get_installation(&ExtensionInstallationId::new("replace-me").unwrap())
            .await
            .expect("store read")
            .expect("installed");
        assert_eq!(
            installation.activation_state(),
            ExtensionActivationState::Enabled,
            "activation state must survive replace"
        );
        assert!(
            installation.owner().is_tenant(),
            "owner must survive replace"
        );
    }

    /// Replace of an INSTALLED-BUT-DISABLED tenant tool stays disabled and does
    /// not publish the new capability set.
    #[tokio::test]
    async fn import_replace_on_disabled_install_stays_disabled() {
        let (_dir, _root, port, _registry, store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let admin = lifecycle_owner();
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "replace-me")
            .expect("replace-me ref");

        let v1 = replace_test_manifest("replace-me", "0.1.0", &["one"]);
        port.import_bundle(
            &replace_test_bundle(&v1, b"\0asm\x01\0\0\0v1"),
            ExtensionImportMode::Add,
            None,
            &admin,
        )
        .await
        .expect("v1 import");
        port.install(package_ref, &admin).await.expect("install v1");

        let v2 = replace_test_manifest("replace-me", "0.2.0", &["one", "two"]);
        let response = port
            .import_bundle(
                &replace_test_bundle(&v2, b"\0asm\x01\0\0\0v2"),
                ExtensionImportMode::Replace,
                None,
                &admin,
            )
            .await
            .expect("replace disabled install");
        assert_eq!(response.phase, LifecyclePhase::Installed);

        let installation = store
            .get_installation(&ExtensionInstallationId::new("replace-me").unwrap())
            .await
            .expect("store read")
            .expect("installed");
        assert_eq!(
            installation.activation_state(),
            ExtensionActivationState::Installed,
            "a disabled install must stay disabled after replace"
        );
        let caps = port
            .active_model_visible_capabilities()
            .await
            .expect("caps");
        assert!(
            active_capability_ids(&caps).is_empty(),
            "a disabled install must not publish capabilities"
        );
    }

    /// A non-operator caller cannot replace an installed tenant tool through
    /// import; the slot is masked as unavailable without leaking the owner.
    #[tokio::test]
    async fn import_replace_by_member_is_unavailable() {
        let (_dir, _root, port, _registry, _store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let admin = lifecycle_owner();
        let member = UserId::new("member").expect("member");
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "replace-me")
            .expect("replace-me ref");

        let v1 = replace_test_manifest("replace-me", "0.1.0", &["one"]);
        port.import_bundle(
            &replace_test_bundle(&v1, b"\0asm\x01\0\0\0v1"),
            ExtensionImportMode::Add,
            None,
            &admin,
        )
        .await
        .expect("v1 import");
        port.install(package_ref, &admin).await.expect("install v1");

        let v2 = replace_test_manifest("replace-me", "0.2.0", &["one", "two"]);
        let error = port
            .import_bundle(
                &replace_test_bundle(&v2, b"\0asm\x01\0\0\0v2"),
                ExtensionImportMode::Replace,
                None,
                &member,
            )
            .await
            .expect_err("a member cannot replace a tenant tool");
        assert!(
            matches!(&error, ProductWorkflowError::InvalidBindingRequest { reason }
                if reason.contains("unavailable")),
            "member replace must be masked as unavailable, got {error:?}"
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

        port.remove(package_ref, &lifecycle_owner())
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

        port.remove(package_ref, &lifecycle_owner())
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

    /// Fix C: an IMPORTED extension (module read from disk, recorded as
    /// `HostBundled`) whose on-disk manifest hash no longer matches its stored
    /// record is only reachable via a torn/corrupt replace (a crash between the
    /// manifest and module writes). Restart migration must fail closed rather
    /// than bless the new manifest over possibly-stale module bytes — before this
    /// fix, the shared `HostBundled` label silently migrated the chimera.
    #[tokio::test]
    async fn restore_imported_extension_rejects_torn_manifest_hash_mismatch() {
        let changed_available = fixture_package_disk_sourced_module(
            "Lifecycle fixture extension with changed manifest",
        );
        let package = changed_available.package.clone();
        let catalog = AvailableExtensionCatalog::from_packages(vec![changed_available]);
        let installation_store = Arc::new(InMemoryExtensionInstallationStore::default());
        // Imported bundles are recorded as HostBundled; the stored hash is the
        // pre-replace (v1) hash, which no longer matches the on-disk manifest.
        let manifest_record = fixture_manifest_record_with_source(
            fixture_extension_manifest(),
            ManifestSource::HostBundled,
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
        .expect_err("a torn imported-extension manifest mismatch must fail closed");
        assert!(
            matches!(error, ProductWorkflowError::InvalidBindingRequest { .. }),
            "expected fail-closed InvalidBindingRequest, got {error:?}"
        );
        assert!(
            restored_active_registry
                .snapshot()
                .get_extension(&package.id)
                .is_none(),
            "a torn extension must not be published"
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
    async fn lifecycle_facade_blocks_credentialed_extension_activation_without_product_auth() {
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
        let error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate { package_ref },
            )
            .await
            .expect_err("missing product-auth account blocks activation");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("github").unwrap())
                .is_none()
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
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");
        let manifest_path = storage_root.join("system/extensions/fixture/manifest.toml");
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
            .remove(package_ref, &lifecycle_owner())
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
    async fn extension_remove_manifest_delete_failure_restores_active_trust_policy() {
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
            .remove(package_ref, &lifecycle_owner())
            .await
            .expect_err("delete manifest failure is reported");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        assert_enabled_active_extension_state(&active_registry, failing_store.as_ref()).await;
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
            .remove(package_ref, &lifecycle_owner())
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
        crate::lifecycle::RebornLocalLifecycleFacade,
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
        crate::lifecycle::RebornLocalLifecycleFacade,
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
        crate::lifecycle::RebornLocalLifecycleFacade,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
    ) {
        extension_lifecycle_fixture_with_catalog_and_service(
            AvailableExtensionCatalog::from_first_party_assets()
                .expect("first-party GitHub catalog"),
            ExtensionLifecycleService::new(ExtensionRegistry::new()),
        )
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
        crate::lifecycle::RebornLocalLifecycleFacade,
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
        let skill_management = Arc::new(crate::lifecycle::RebornLocalSkillManagementPort::new(
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
        let installation_store = Arc::new(InMemoryExtensionInstallationStore::default());
        let extension_management = Arc::new(RebornLocalExtensionManagementPort::new(
            root_filesystem,
            catalog,
            installation_store.clone(),
            Arc::new(Mutex::new(lifecycle_service)),
            test_active_extension_publisher(
                Arc::clone(&active_registry),
                test_extension_trust_policy(),
            ),
            lifecycle_owner(),
        ));
        let facade = crate::lifecycle::RebornLocalLifecycleFacade::new(skill_management)
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
                    let _ = started.send(());
                }
                let mut release = self.release.lock().await;
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
    impl crate::product_auth_runtime_credentials::RuntimeCredentialAccountSelectionService
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
            _request: crate::product_auth_runtime_credentials::RuntimeCredentialAccountSelectionRequest,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            Err(ironclaw_auth::AuthProductError::CredentialMissing)
        }
    }

    struct ConfiguredRuntimeCredentialAccounts;

    #[async_trait]
    impl crate::product_auth_runtime_credentials::RuntimeCredentialAccountSelectionService
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
            _request: crate::product_auth_runtime_credentials::RuntimeCredentialAccountSelectionRequest,
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
                created_at: now,
                updated_at: now,
            })
        }
    }

    struct BackendUnavailableRuntimeCredentialAccounts;

    #[async_trait]
    impl crate::product_auth_runtime_credentials::RuntimeCredentialAccountSelectionService
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
            _request: crate::product_auth_runtime_credentials::RuntimeCredentialAccountSelectionRequest,
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

    /// Like [`fixture_extension_package_with_description`], but with the wasm
    /// module asset pointed at an on-disk path (`Filesystem`) instead of inline
    /// `Bytes` — i.e. an IMPORTED/filesystem-discovered package, as opposed to a
    /// first-party in-binary one. Used to exercise the torn-replace fail-closed
    /// path at restart.
    fn fixture_package_disk_sourced_module(description: &str) -> AvailableExtensionPackage {
        let mut package = fixture_extension_package_with_description(description);
        for asset in &mut package.assets {
            if asset.path == "wasm/fixture.wasm" {
                asset.content = AvailableExtensionAssetContent::Filesystem(
                    VirtualPath::new("/system/extensions/fixture/wasm/fixture.wasm")
                        .expect("fixture module path"),
                );
            }
        }
        package
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
        let root =
            VirtualPath::new(format!("/system/extensions/{root_id}")).expect("extension root");
        let package = ExtensionPackage::from_manifest_toml(manifest, root, manifest_toml)
            .expect("fixture package");
        AvailableExtensionPackage {
            package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, root_id)
                .expect("fixture package ref"),
            manifest_toml: manifest_toml.to_string(),
            package,
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
        let contracts = ironclaw_host_runtime::default_host_api_contract_registry()
            .expect("host API contracts");
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
