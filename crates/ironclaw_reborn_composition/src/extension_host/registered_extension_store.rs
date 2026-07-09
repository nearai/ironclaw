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

use ironclaw_extensions::ManifestSource;
use ironclaw_filesystem::{FileType, FilesystemError, RootFilesystem};
use ironclaw_host_api::{UserId, VirtualPath};
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
