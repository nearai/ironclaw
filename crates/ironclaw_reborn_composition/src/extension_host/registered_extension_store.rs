//! Owner-scoped storage and catalog composition for user-registered MCP
//! extensions. Descriptors live at
//! `/system/extensions/registered/<tenant>/<owner>/<id>/manifest.toml`.
//!
//! **Boot-leak invariant (do not weaken):** the shared, process-wide
//! `AvailableExtensionCatalog` must never contain a `UserRegistered` package.
//! Its `search`/`resolve` do no owner filtering, so a registered package
//! reachable through it is installable by any owner. Every helper here reads
//! the owner overlay separately and merges at the call site.
//!
//! The write side (`put`/`delete`) lands with the register verb in T3.
//!
//! This module is landed unwired: `extension_lifecycle.rs`'s
//! `search`/`install`/`activate`/`remove` do not yet consult these helpers.
//! That wiring — plus stamping `InstallationOwner::user(owner)` on a
//! registered package's installation row — lands in the T1 install/list/
//! search/activate/remove integration commit. Until then every item here is
//! read-only-reachable from its own unit tests only, hence the blanket
//! `dead_code` allow.
#![allow(dead_code)]

use ironclaw_extensions::ManifestSource;
use ironclaw_filesystem::{FileType, FilesystemError, RootFilesystem};
use ironclaw_host_api::{InvocationId, ResourceScope, TenantId, UserId, VirtualPath};
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

    /// `/system/extensions/registered/<tenant>/<owner>` — the directory
    /// [`load_filesystem_packages`] lists for one tenant-owner's registered
    /// set.
    fn owner_root_for_tenant(
        tenant_id: &TenantId,
        owner: &UserId,
    ) -> Result<VirtualPath, ProductWorkflowError> {
        VirtualPath::new(format!(
            "{REGISTERED_ROOT}/{}/{}",
            tenant_id.as_str(),
            owner.as_str()
        ))
        .map_err(map_binding_error)
    }

    /// One tenant-owner's registered packages, reusing the shared filesystem
    /// package parser tagged with `ManifestSource::UserRegistered`.
    pub(crate) async fn list_for_scope<F>(
        fs: &F,
        scope: &ResourceScope,
    ) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError>
    where
        F: RootFilesystem + ?Sized,
    {
        let root = Self::owner_root_for_tenant(&scope.tenant_id, &scope.user_id)?;
        load_filesystem_packages(
            fs,
            &root,
            ManifestSource::UserRegistered {
                tenant_id: scope.tenant_id.clone(),
                owner: scope.user_id.clone(),
            },
        )
        .await
    }

    /// Every tenant's every owner's registered packages. Boot-time-only
    /// concern: an `ExtensionInstallation` record carries no owner field yet
    /// (plan risk 2), so restore's fallback must search across all
    /// tenant-owners. Never call this from the live search/install path,
    /// which must stay scoped to the calling tenant-owner via
    /// [`list_for_scope`].
    pub(crate) async fn list_all<F>(
        fs: &F,
    ) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError>
    where
        F: RootFilesystem + ?Sized,
    {
        let root = Self::registered_root()?;
        let tenant_entries = match fs.list_dir(&root).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) | Err(FilesystemError::MountNotFound { .. }) => {
                return Ok(Vec::new());
            }
            Err(error) => {
                return Err(ProductWorkflowError::Transient {
                    reason: format!("failed to list registered extension tenants: {error}"),
                });
            }
        };
        let mut packages = Vec::new();
        for tenant_entry in tenant_entries {
            if tenant_entry.file_type != FileType::Directory {
                continue;
            }
            let Ok(tenant_id) = TenantId::new(tenant_entry.name.clone()) else {
                continue;
            };
            let tenant_root = VirtualPath::new(format!("{REGISTERED_ROOT}/{}", tenant_id.as_str()))
                .map_err(map_binding_error)?;
            let owner_entries = match fs.list_dir(&tenant_root).await {
                Ok(entries) => entries,
                Err(FilesystemError::NotFound { .. })
                | Err(FilesystemError::MountNotFound { .. }) => continue,
                Err(error) => {
                    // Skip-and-log, not `?`: this feeds boot-time
                    // `resolve_any_owner_for_restore` for every tenant, so one
                    // tenant's transient directory error must not abort every
                    // other tenant's restore (cross-tenant DoS).
                    tracing::warn!(
                        tenant = tenant_id.as_str(),
                        %error,
                        "skipping tenant's registered extensions: directory listing failed"
                    );
                    continue;
                }
            };
            for entry in owner_entries {
                if entry.file_type != FileType::Directory {
                    continue;
                }
                let Ok(owner) = UserId::new(entry.name.clone()) else {
                    continue;
                };
                let mut scope = ResourceScope::local_default(owner.clone(), InvocationId::new())
                    .map_err(map_binding_error)?;
                scope.tenant_id = tenant_id.clone();
                // Skip-and-log, not `?`: one tenant-owner's transient
                // directory error must not abort every other tenant-owner's
                // restore (cross-tenant DoS).
                match Self::list_for_scope(fs, &scope).await {
                    Ok(owner_packages) => packages.extend(owner_packages),
                    Err(error) => {
                        tracing::warn!(
                            tenant = tenant_id.as_str(),
                            owner = owner.as_str(),
                            %error,
                            "skipping owner's registered extensions: directory listing failed"
                        );
                    }
                }
            }
        }
        Ok(packages)
    }
}

/// One-shot boot-time migration of the PRE-TENANT descriptor layout
/// (`registered/<owner>/<id>/manifest.toml`, no tenant segment) into the
/// local default tenant (`registered/default/<owner>/<id>/…`), mirroring the
/// wire-format serde default in `extension_installation_store.rs`. Without
/// this the tenant-scoped walkers above cannot see pre-tenant registrations,
/// so they'd silently vanish from listing and boot restore.
///
/// Discriminator: a depth-1 directory is a legacy OWNER dir iff one of its
/// child directories contains `manifest.toml` directly (descriptors sit one
/// level below an owner). In the tenant-scoped layout the manifest is two
/// levels below the depth-1 (tenant) dir, so healthy tenant dirs never match.
/// Best-effort per entry: one broken legacy dir must not abort boot restore
/// for everyone else (same skip-and-log stance as [`RegisteredExtensionStore::list_all`]).
pub(crate) async fn migrate_legacy_owner_layout<F>(fs: &F) -> Result<(), ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    let root = VirtualPath::new(REGISTERED_ROOT).map_err(map_binding_error)?;
    let entries = match fs.list_dir(&root).await {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) | Err(FilesystemError::MountNotFound { .. }) => {
            return Ok(());
        }
        Err(error) => {
            return Err(ProductWorkflowError::Transient {
                reason: format!("failed to list registered extension root: {error}"),
            });
        }
    };
    for entry in entries {
        if entry.file_type != FileType::Directory {
            continue;
        }
        let Ok(owner) = UserId::new(entry.name.clone()) else {
            continue;
        };
        if let Err(error) = migrate_legacy_owner_dir(fs, &owner).await {
            tracing::warn!(
                owner = owner.as_str(),
                %error,
                "skipping legacy registered-extension migration for owner"
            );
        }
    }
    Ok(())
}

/// Migrates one candidate legacy owner directory. No-op when the directory
/// turns out to be a tenant dir (no child holds `manifest.toml` directly).
async fn migrate_legacy_owner_dir<F>(fs: &F, owner: &UserId) -> Result<(), ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    let legacy_root = VirtualPath::new(format!("{REGISTERED_ROOT}/{}", owner.as_str()))
        .map_err(map_binding_error)?;
    let children = fs.list_dir(&legacy_root).await.map_err(map_transient)?;
    let mut migrated_all = true;
    let mut found_descriptor = false;
    for child in &children {
        if child.file_type != FileType::Directory {
            migrated_all = false;
            continue;
        }
        let manifest = VirtualPath::new(format!(
            "{REGISTERED_ROOT}/{}/{}/manifest.toml",
            owner.as_str(),
            child.name
        ))
        .map_err(map_binding_error)?;
        match fs.stat(&manifest).await {
            Ok(stat) if stat.file_type == FileType::File => {}
            Ok(_) | Err(FilesystemError::NotFound { .. }) => {
                // Not a legacy descriptor (tenant-layout owner dir, or junk).
                migrated_all = false;
                continue;
            }
            Err(error) => return Err(map_transient(error)),
        }
        found_descriptor = true;
        let source = VirtualPath::new(format!(
            "{REGISTERED_ROOT}/{}/{}",
            owner.as_str(),
            child.name
        ))
        .map_err(map_binding_error)?;
        let destination = VirtualPath::new(format!(
            "{REGISTERED_ROOT}/{}/{}/{}",
            ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID,
            owner.as_str(),
            child.name
        ))
        .map_err(map_binding_error)?;
        let destination_manifest =
            VirtualPath::new(format!("{}/manifest.toml", destination.as_str()))
                .map_err(map_binding_error)?;
        match fs.stat(&destination_manifest).await {
            Ok(_) => {
                // A tenant-scoped registration already exists for this id.
                // Never clobber it, and never delete the divergent legacy
                // copy — leave it for manual inspection.
                tracing::warn!(
                    owner = owner.as_str(),
                    extension = child.name.as_str(),
                    "legacy registered descriptor also exists tenant-scoped; leaving legacy copy in place"
                );
                migrated_all = false;
                continue;
            }
            Err(FilesystemError::NotFound { .. }) => {}
            Err(error) => return Err(map_transient(error)),
        }
        copy_tree(fs, &source, &destination).await?;
        fs.delete(&source).await.map_err(map_transient)?;
    }
    // Only remove the legacy owner dir once everything in it was a migrated
    // descriptor; anything else stays put rather than being destroyed.
    if found_descriptor && migrated_all {
        fs.delete(&legacy_root).await.map_err(map_transient)?;
    }
    Ok(())
}

/// Copies every file under `source` to the same relative path under
/// `destination` (descriptor trees are tiny: a manifest plus at most a few
/// referenced docs/schemas).
async fn copy_tree<F>(
    fs: &F,
    source: &VirtualPath,
    destination: &VirtualPath,
) -> Result<(), ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    let mut pending = vec![(source.clone(), destination.clone())];
    while let Some((source_dir, destination_dir)) = pending.pop() {
        for entry in fs.list_dir(&source_dir).await.map_err(map_transient)? {
            let source_path = VirtualPath::new(format!("{}/{}", source_dir.as_str(), entry.name))
                .map_err(map_binding_error)?;
            let destination_path =
                VirtualPath::new(format!("{}/{}", destination_dir.as_str(), entry.name))
                    .map_err(map_binding_error)?;
            match entry.file_type {
                FileType::Directory => pending.push((source_path, destination_path)),
                FileType::File => {
                    let bytes = fs.read_file(&source_path).await.map_err(map_transient)?;
                    fs.write_file(&destination_path, &bytes)
                        .await
                        .map_err(map_transient)?;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn map_transient(error: impl std::fmt::Display) -> ProductWorkflowError {
    ProductWorkflowError::Transient {
        reason: format!("legacy registered-extension migration failed: {error}"),
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
/// shared catalog's own `catalog.search(query)` results. `scope: None` (no
/// caller identity — e.g. a boot-time/system caller) contributes no overlay:
/// registered packages are visible only to their own tenant-owner, never to
/// an unscoped caller.
pub(crate) async fn search_with_owner_overlay_for_scope<F>(
    fs: &F,
    scope: Option<&ResourceScope>,
    query: &str,
) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    let Some(scope) = scope else {
        return Ok(Vec::new());
    };
    let normalized_query = query.trim().to_ascii_lowercase();
    let packages = RegisteredExtensionStore::list_for_scope(fs, scope).await?;
    Ok(packages
        .into_iter()
        .filter(|package| !is_internal_extension_package_ref(&package.package_ref))
        .filter(|package| package_matches_search(package, &normalized_query))
        .collect())
}

/// Resolve one package by ref: shared first-party catalog first, then
/// `scope`'s registered set. `scope: None` is a caller with no tenant-owner
/// scope (the boot-time NEAR AI bootstrap installer) and can reach
/// first-party packages only.
pub(crate) async fn resolve_with_owner_overlay_for_scope<F>(
    catalog: &AvailableExtensionCatalog,
    fs: &F,
    scope: Option<&ResourceScope>,
    package_ref: &LifecyclePackageRef,
) -> Result<AvailableExtensionPackage, ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    if let Ok(available) = catalog.resolve(package_ref) {
        return Ok((*available).clone());
    }
    let Some(scope) = scope else {
        return Err(not_found());
    };
    RegisteredExtensionStore::list_for_scope(fs, scope)
        .await?
        .into_iter()
        .find(|package| &package.package_ref == package_ref)
        .ok_or_else(not_found)
}

/// Boot-only restore fallback, reached on a `catalog.resolve()` miss during
/// `restore_extension_lifecycle_state`. Deliberately any-tenant-owner because
/// installations carry no owner field yet; never call it from a live
/// search/install path, which must stay scoped via
/// [`resolve_with_owner_overlay_for_scope`].
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
