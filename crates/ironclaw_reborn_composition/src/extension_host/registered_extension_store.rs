//! Owner-scoped storage and catalog composition for user-registered MCP
//! extensions. Descriptors live at
//! `/system/extensions/registered/<owner>/<id>/manifest.toml`.
//!
//! **Boot-leak invariant (do not weaken):** the shared, process-wide
//! `AvailableExtensionCatalog` must never contain a `UserRegistered` package.
//! Its `search`/`resolve` do no owner filtering, so a registered package
//! reachable through it is installable by any owner. Every helper here reads
//! the owner overlay separately and merges at the call site.
//!
//! The write side (`put`/`delete`) lands with the register verb in T3.

use std::collections::BTreeSet;

use ironclaw_extensions::{
    ExtensionActivationState, ExtensionInstallationError, ExtensionInstallationStore,
    ManifestSource,
};
use ironclaw_filesystem::{FileType, FilesystemError, RootFilesystem};
use ironclaw_host_api::{ExtensionId, UserId, VirtualPath};
use ironclaw_product_workflow::{LifecyclePackageRef, ProductWorkflowError};

use crate::extension_host::available_extensions::{
    AvailableExtensionCatalog, AvailableExtensionPackage, is_internal_extension_package_ref,
    load_filesystem_packages, package_matches_search,
};

const REGISTERED_ROOT: &str = "/system/extensions/registered";

/// True for a package owned by a user's registered store. Such a package must
/// never be materialized under the shared `/system/extensions/<id>/` directory:
/// the next boot's catalog scan would re-adopt it as a first-party entry,
/// reopening the boot leak. Both writers in `extension_lifecycle.rs` (install
/// and restore) gate on this.
pub(crate) fn is_owner_registered(source: &ManifestSource) -> bool {
    matches!(source, ManifestSource::UserRegistered { .. })
}

/// T3-iso owner filter: a `UserRegistered` manifest is visible only to its
/// own owner; every other source is visible to any caller (including a
/// caller with no resolved identity).
pub(crate) fn manifest_visible_to_caller(source: &ManifestSource, caller: Option<&UserId>) -> bool {
    source.visible_to_caller(caller)
}

/// AC2: the set of extension ids that are BOTH enabled AND visible to
/// `owner` — the model-visible-capability filter
/// (`active_model_visible_capabilities`) intersects the shared registry
/// against this set, so a disabled or cross-owner `UserRegistered` extension's
/// capabilities never reach the model's toolbox. Deliberately NOT used by the
/// operator-tool-config surface below: `builtin.*` capabilities are never
/// installation-tracked, so an intersection against `list_enabled_installations()`
/// would silently drop them from Settings > Tools (see
/// `operator_config_excluded_extension_ids`'s doc for that surface's
/// default-allow design instead).
pub(crate) async fn owner_visible_enabled_extension_ids(
    installation_store: &dyn ExtensionInstallationStore,
    owner: &UserId,
) -> Result<BTreeSet<ExtensionId>, ExtensionInstallationError> {
    let enabled_ids: BTreeSet<ExtensionId> = installation_store
        .list_enabled_installations()
        .await?
        .into_iter()
        .map(|installation| installation.extension_id().clone())
        .collect();
    let owner_visible_ids: BTreeSet<ExtensionId> = installation_store
        .list_manifests()
        .await?
        .into_iter()
        .filter(|record| manifest_visible_to_caller(&record.manifest().source, Some(owner)))
        .map(|record| record.extension_id().clone())
        .collect();
    Ok(enabled_ids
        .intersection(&owner_visible_ids)
        .cloned()
        .collect())
}

/// Correction 10: the set of extension ids the operator-tool-config surface
/// (`ActiveRegistryOperatorToolCatalog`) must HIDE from `caller` — a
/// default-ALLOW exclusion list, unlike AC2's default-deny intersection.
/// `builtin.*` and other never-installation-tracked capabilities carry no
/// manifest or installation record at all, so they are never added here and
/// stay visible to every caller, matching Settings > Tools' pre-T3-iso
/// behavior for them. Only two things get excluded: a `UserRegistered`
/// manifest owned by someone else, and any tracked installation that is not
/// `Enabled` (disabled/quarantined).
pub(crate) async fn operator_config_excluded_extension_ids(
    installation_store: &dyn ExtensionInstallationStore,
    caller: &UserId,
) -> Result<BTreeSet<ExtensionId>, ExtensionInstallationError> {
    let mut excluded = BTreeSet::new();
    for record in installation_store.list_manifests().await? {
        if !manifest_visible_to_caller(&record.manifest().source, Some(caller)) {
            excluded.insert(record.extension_id().clone());
        }
    }
    for installation in installation_store.list_installations().await? {
        if installation.activation_state() != ExtensionActivationState::Enabled {
            excluded.insert(installation.extension_id().clone());
        }
    }
    Ok(excluded)
}

/// Owner-scoped read access to user-registered extension manifests.
pub(crate) struct RegisteredExtensionStore;

impl RegisteredExtensionStore {
    fn registered_root() -> Result<VirtualPath, ProductWorkflowError> {
        VirtualPath::new(REGISTERED_ROOT).map_err(map_binding_error)
    }

    /// `/system/extensions/registered/<owner>` — the directory
    /// [`load_filesystem_packages`] lists for one owner's registered set.
    fn owner_root(owner: &UserId) -> Result<VirtualPath, ProductWorkflowError> {
        VirtualPath::new(format!("{REGISTERED_ROOT}/{}", owner.as_str())).map_err(map_binding_error)
    }

    /// One owner's registered packages, reusing the shared filesystem
    /// package parser tagged with `ManifestSource::UserRegistered`.
    pub(crate) async fn list_for_owner<F>(
        fs: &F,
        owner: &UserId,
    ) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError>
    where
        F: RootFilesystem + ?Sized,
    {
        let root = Self::owner_root(owner)?;
        load_filesystem_packages(
            fs,
            &root,
            ManifestSource::UserRegistered {
                owner: owner.clone(),
            },
        )
        .await
    }

    /// Every owner's registered packages. Boot-time-only concern: an
    /// `ExtensionInstallation` record carries no owner field yet (plan risk
    /// 2), so restore's fallback must search across all owners. Never call
    /// this from the live search/install path, which must stay scoped to
    /// the calling owner via [`list_for_owner`].
    pub(crate) async fn list_all<F>(
        fs: &F,
    ) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError>
    where
        F: RootFilesystem + ?Sized,
    {
        let root = Self::registered_root()?;
        let entries = match fs.list_dir(&root).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) | Err(FilesystemError::MountNotFound { .. }) => {
                return Ok(Vec::new());
            }
            Err(error) => {
                return Err(ProductWorkflowError::Transient {
                    reason: format!("failed to list registered extension owners: {error}"),
                });
            }
        };
        let mut packages = Vec::new();
        for entry in entries {
            if entry.file_type != FileType::Directory {
                continue;
            }
            let Ok(owner) = UserId::new(entry.name.clone()) else {
                continue;
            };
            packages.extend(Self::list_for_owner(fs, &owner).await?);
        }
        Ok(packages)
    }
}

fn not_found() -> ProductWorkflowError {
    ProductWorkflowError::InvalidBindingRequest {
        reason: "available extension was not found".to_string(),
    }
}

fn map_binding_error(error: impl std::fmt::Display) -> ProductWorkflowError {
    ProductWorkflowError::InvalidBindingRequest {
        reason: error.to_string(),
    }
}

/// The additional (owner-registered) search matches to overlay on top of the
/// shared catalog's own `catalog.search(query)` results. `owner: None` (no
/// caller identity — e.g. a boot-time/system caller) contributes no overlay:
/// registered packages are visible only to their own owner, never to an
/// unscoped caller.
pub(crate) async fn search_with_owner_overlay<F>(
    fs: &F,
    owner: Option<&UserId>,
    query: &str,
) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    let Some(owner) = owner else {
        return Ok(Vec::new());
    };
    let normalized_query = query.trim().to_ascii_lowercase();
    let packages = RegisteredExtensionStore::list_for_owner(fs, owner).await?;
    Ok(packages
        .into_iter()
        .filter(|package| !is_internal_extension_package_ref(&package.package_ref))
        .filter(|package| package_matches_search(package, &normalized_query))
        .collect())
}

/// Resolve one package by ref: shared first-party catalog first, then `owner`'s
/// registered set. `owner: None` is a caller with no owner scope (the boot-time
/// NEAR AI bootstrap installer) and can reach first-party packages only.
pub(crate) async fn resolve_with_owner_overlay<F>(
    catalog: &AvailableExtensionCatalog,
    fs: &F,
    owner: Option<&UserId>,
    package_ref: &LifecyclePackageRef,
) -> Result<AvailableExtensionPackage, ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    if let Ok(available) = catalog.resolve(package_ref) {
        return Ok(available.clone());
    }
    let Some(owner) = owner else {
        return Err(not_found());
    };
    let owner_packages = RegisteredExtensionStore::list_for_owner(fs, owner).await?;
    owner_packages
        .into_iter()
        .find(|package| &package.package_ref == package_ref)
        .ok_or_else(not_found)
}

/// Boot-only restore fallback, reached on a `catalog.resolve()` miss during
/// `restore_extension_lifecycle_state`. Deliberately any-owner because
/// installations carry no owner field yet; never call it from a live
/// search/install path, which must stay scoped via
/// [`resolve_with_owner_overlay`].
pub(crate) async fn resolve_any_owner_for_restore<F>(
    fs: &F,
    package_ref: &LifecyclePackageRef,
) -> Result<AvailableExtensionPackage, ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    let packages = RegisteredExtensionStore::list_all(fs).await?;
    packages
        .into_iter()
        .find(|package| &package.package_ref == package_ref)
        .ok_or_else(not_found)
}
