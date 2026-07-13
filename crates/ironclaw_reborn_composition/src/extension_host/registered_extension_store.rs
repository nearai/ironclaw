//! Owner-scoped storage for user-registered MCP extensions, wired into
//! `extension_lifecycle.rs`'s search/install/projection paths and boot
//! restore. Descriptors are sharded at
//! `/system/extensions/registered/<tenant>/<owner>/<id>/manifest.toml` —
//! that path sharding IS the visibility filter for the live scoped readers.
//! Manifest provenance (`ManifestSource::UserRegistered { tenant_id, owner }`)
//! records who registered a descriptor, while the installation ROW's
//! `InstallationOwner` is authoritative for who holds the install — the two
//! are paired by `extension_lifecycle::effective_owner_scope`.
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
use ironclaw_host_api::{InvocationId, ResourceScope, TenantId, UserId, VirtualPath};
use ironclaw_product_workflow::{LifecyclePackageRef, ProductWorkflowError};

use crate::extension_host::available_extensions::{
    AssetLoading, AvailableExtensionPackage, is_internal_extension_package_ref,
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
    /// `asset_loading` (item 6): search/list callers pass `Skip` since they
    /// never read `.assets`; callers that resolve a package for
    /// install/restore pass `Inline`.
    pub(crate) async fn list_for_scope<F>(
        fs: &F,
        scope: &ResourceScope,
        asset_loading: AssetLoading,
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
            asset_loading,
        )
        .await
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
/// for everyone else (same skip-and-log stance as boot restore).
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
            // silent-ok: a failed migration leaves that owner's legacy
            // descriptors in place (unmigrated, not deleted or corrupted) —
            // no data loss, and every other owner's migration still runs;
            // the un-migrated owner is simply invisible to the tenant-scoped
            // walkers until a later boot retries.
            tracing::debug!(
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
                tracing::debug!(
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

/// The masked lookup miss every scoped resolution path reports — a foreign
/// owner's registered package is indistinguishable from a nonexistent one.
pub(crate) fn available_extension_not_found() -> ProductWorkflowError {
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
/// shared catalog's own `catalog.search(query)` results. Registered packages
/// are visible only to their own tenant-owner (`scope`), never to an unscoped
/// caller — path sharding in [`RegisteredExtensionStore::list_for_scope`] is
/// the filter.
pub(crate) async fn search_with_owner_overlay_for_scope<F>(
    fs: &F,
    scope: &ResourceScope,
    query: &str,
) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    let normalized_query = query.trim().to_ascii_lowercase();
    // Item 6: search never reads `.assets`, only the summary — skip the
    // per-entry directory-asset read.
    let packages = RegisteredExtensionStore::list_for_scope(fs, scope, AssetLoading::Skip).await?;
    Ok(packages
        .into_iter()
        .filter(|package| !is_internal_extension_package_ref(&package.package_ref))
        .filter(|package| package_matches_search(package, &normalized_query))
        .collect())
}

/// Resolve one package by ref from `scope`'s registered set only. Callers try
/// the shared first-party catalog first (holding its lock only for that
/// synchronous lookup — never across this function's filesystem awaits) and
/// fall back here on a miss, since registered packages never enter the shared
/// catalog (boot-leak invariant). `Ok(None)` is a genuine miss (nonexistent
/// or foreign-owned — deliberately indistinguishable); `Err` is a real read
/// failure, so list-shaped callers can stay resilient without conflating the
/// two.
pub(crate) async fn resolve_registered_for_scope<F>(
    fs: &F,
    scope: &ResourceScope,
    package_ref: &LifecyclePackageRef,
) -> Result<Option<AvailableExtensionPackage>, ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    // Item 6: resolution feeds install/project, which materializes
    // `.assets` — keep `Inline`.
    Ok(
        RegisteredExtensionStore::list_for_scope(fs, scope, AssetLoading::Inline)
            .await?
            .into_iter()
            .find(|package| &package.package_ref == package_ref),
    )
}

/// One (tenant, owner)'s full registered set, read ONCE. Boot-only restore
/// fallback, reached on a `catalog.resolve()` miss during
/// `restore_extension_lifecycle_state`. Row-owner-keyed: the caller derives
/// `(tenant_id, owner)` from the installation row (`effective_owner_scope`)
/// and this loads that owner's shard directly — never a cross-owner scan,
/// which could otherwise serve a DIFFERENT owner's descriptor depending on
/// directory listing order. Restore groups installations by owner and calls
/// this at most once per distinct (tenant, owner) per boot, since multiple
/// installations frequently share an owner and each call is a full directory
/// walk + manifest parse. Callers that cannot establish a row owner must
/// skip-and-log rather than guess a scope.
pub(crate) async fn list_for_owner<F>(
    fs: &F,
    tenant_id: &TenantId,
    owner: &UserId,
) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    let mut scope = ResourceScope::local_default(owner.clone(), InvocationId::new())
        .map_err(map_binding_error)?;
    scope.tenant_id = tenant_id.clone();
    // Item 6: boot restore publishes/materializes the resolved package —
    // keep `Inline`.
    RegisteredExtensionStore::list_for_scope(fs, &scope, AssetLoading::Inline).await
}
