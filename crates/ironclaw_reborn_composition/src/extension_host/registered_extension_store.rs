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

use std::sync::Arc;

use ironclaw_extensions::{
    ExtensionInstallation, ExtensionInstallationId, ExtensionInstallationPersistedParts,
    ExtensionInstallationStore, ExtensionManifestRecord, ExtensionManifestRef, ExtensionRuntime,
    ExtensionRuntimeV2, ManifestHash, ManifestSource,
};
use ironclaw_filesystem::{FileType, FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    ExtensionId, InvocationId, ResourceScope, TenantId, UserId, VirtualPath, sha256_digest_token,
};
use ironclaw_product_workflow::{LifecyclePackageRef, ProductWorkflowError};

use crate::extension_host::available_extensions::{
    AssetLoading, AvailableExtensionPackage, is_internal_extension_package_ref,
    load_filesystem_packages, package_matches_search,
};

const REGISTERED_ROOT: &str = "/system/extensions/registered";
const HOSTED_MCP_ID_PREFIX: &str = "mcp-";
const HOSTED_MCP_DIGEST_LEN: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct HostedMcpExtensionId(ExtensionId);

#[derive(Debug, thiserror::Error)]
#[error("extension id is not a minted hosted MCP id")]
pub(crate) struct NotHostedMcp;

impl HostedMcpExtensionId {
    /// `account_label` is deliberately folded into the mint digest now even
    /// though every production call site passes `""` today (single-account
    /// ids only) — this keeps the id stable when multi-account support lands
    /// additively later, instead of changing the hash shape retroactively.
    pub(crate) fn mint(
        tenant_id: &TenantId,
        owner: &UserId,
        url: &str,
        account_label: &str,
    ) -> Result<Self, ProductWorkflowError> {
        let normalized_url = normalized_hosted_mcp_url(url)?;
        let mut input = Vec::new();
        for field in [
            tenant_id.as_str().as_bytes(),
            owner.as_str().as_bytes(),
            normalized_url.as_bytes(),
            account_label.as_bytes(),
        ] {
            input.extend_from_slice(&(field.len() as u64).to_le_bytes());
            input.extend_from_slice(field);
        }
        let digest = sha256_digest_token(&input);
        let suffix = digest
            .strip_prefix("sha256:")
            .unwrap_or(digest.as_str())
            .chars()
            .take(HOSTED_MCP_DIGEST_LEN)
            .collect::<String>();
        ExtensionId::new(format!("{HOSTED_MCP_ID_PREFIX}{suffix}"))
            .map(Self)
            .map_err(map_binding_error)
    }

    pub(crate) fn parse(extension_id: &ExtensionId) -> Result<Self, NotHostedMcp> {
        let Some(suffix) = extension_id.as_str().strip_prefix(HOSTED_MCP_ID_PREFIX) else {
            return Err(NotHostedMcp);
        };
        if suffix.len() != HOSTED_MCP_DIGEST_LEN
            || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(NotHostedMcp);
        }
        Ok(Self(extension_id.clone()))
    }

    // Also needed under `test-support` (not just `cfg(test)`): cross-crate
    // integration-test fixtures mint via `test_support::mint_registered_mcp_extension_id_for_test`
    // as a normal dependency of this crate, not as its own `cfg(test)` build.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn into_extension_id(self) -> ExtensionId {
        self.0
    }
}

pub(crate) fn is_hosted_mcp_id_namespace(extension_id: &ExtensionId) -> bool {
    extension_id.as_str().starts_with(HOSTED_MCP_ID_PREFIX)
}

fn normalized_hosted_mcp_url(url: &str) -> Result<String, ProductWorkflowError> {
    let parsed = url::Url::parse(url).map_err(map_binding_error)?;
    let host = parsed
        .host_str()
        .ok_or_else(|| ProductWorkflowError::InvalidBindingRequest {
            reason: "registered MCP URL must include a host".to_string(),
        })?
        .to_ascii_lowercase();
    // Query strings differentiate accounts/endpoints out-of-band from
    // `account_label` (the designed-for-purpose mechanism, see
    // `HostedMcpExtensionId::mint`'s doc comment) and would otherwise require
    // exact query normalization to avoid reintroducing a collision at a
    // smaller blast radius — reject instead, mirroring `HostedMcpEndpoint::parse`.
    if parsed.query().is_some() {
        return Err(ProductWorkflowError::InvalidBindingRequest {
            reason: "registered MCP URL must not include a query string".to_string(),
        });
    }
    let path = parsed.path().trim_end_matches('/');
    let normalized_path = if path.is_empty() { "/" } else { path };
    let port = parsed
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    Ok(format!(
        "{}://{host}{port}{normalized_path}",
        parsed.scheme().to_ascii_lowercase()
    ))
}

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
        let packages = load_filesystem_packages(
            fs,
            &root,
            ManifestSource::UserRegistered {
                tenant_id: scope.tenant_id.clone(),
                owner: scope.user_id.clone(),
            },
            asset_loading,
        )
        .await?;
        Ok(packages
            .into_iter()
            .filter(|package| registered_package_has_minted_id(package, scope))
            .collect())
    }
}

/// Concrete owner-scoped registered-extension reader wrapping a composition
/// filesystem handle. The single production caller is
/// `RebornLocalExtensionManagementPort::registered_store`; a concrete struct
/// with inherent methods is enough since there is exactly one implementor and
/// no `dyn` injection point (types.md "traits must earn their keep"). Each
/// method delegates to the matching generic free function/associated
/// function above, so behavior is byte-for-byte identical to today's
/// call sites — this type only saves callers from re-passing
/// `self.filesystem.as_ref()` at every call.
pub(crate) struct FilesystemRegisteredExtensionStore {
    filesystem: Arc<dyn RootFilesystem>,
}

impl FilesystemRegisteredExtensionStore {
    pub(crate) fn new(filesystem: Arc<dyn RootFilesystem>) -> Self {
        Self { filesystem }
    }

    pub(crate) async fn list_for_scope(
        &self,
        scope: &ResourceScope,
        asset_loading: AssetLoading,
    ) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError> {
        RegisteredExtensionStore::list_for_scope(self.filesystem.as_ref(), scope, asset_loading)
            .await
    }

    pub(crate) async fn search_with_owner_overlay(
        &self,
        scope: &ResourceScope,
        query: &str,
    ) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError> {
        search_with_owner_overlay_for_scope(self.filesystem.as_ref(), scope, query).await
    }

    pub(crate) async fn resolve_for_scope(
        &self,
        scope: &ResourceScope,
        package_ref: &LifecyclePackageRef,
    ) -> Result<Option<AvailableExtensionPackage>, ProductWorkflowError> {
        resolve_registered_for_scope(self.filesystem.as_ref(), scope, package_ref).await
    }

    pub(crate) async fn list_for_owner(
        &self,
        tenant_id: &TenantId,
        owner: &UserId,
    ) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError> {
        list_for_owner(self.filesystem.as_ref(), tenant_id, owner).await
    }
}

fn registered_package_has_minted_id(
    package: &AvailableExtensionPackage,
    scope: &ResourceScope,
) -> bool {
    let ExtensionRuntime::Mcp { url: Some(url), .. } = &package.package.manifest.runtime else {
        return false;
    };
    let Ok(parsed) = HostedMcpExtensionId::parse(&package.package.id) else {
        tracing::debug!(
            extension_id = package.package.id.as_str(),
            "skipping registered descriptor with an unminted id"
        );
        return false;
    };
    match HostedMcpExtensionId::mint(&scope.tenant_id, &scope.user_id, url, "") {
        Ok(expected) if expected == parsed => true,
        Ok(_) | Err(_) => {
            tracing::debug!(
                extension_id = package.package.id.as_str(),
                "skipping registered descriptor whose id does not match its owner and endpoint"
            );
            false
        }
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

/// Re-key descriptors and installed rows created before hosted MCP ids were
/// minted. New state is written before old state is removed so interruption
/// leaves a retryable duplicate instead of an orphaned installation.
pub(crate) async fn migrate_unminted_registered_ids<F>(
    fs: &F,
    installation_store: &Arc<dyn ExtensionInstallationStore>,
) -> Result<(), ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    let manifests = installation_store
        .list_manifests()
        .await
        .map_err(map_transient)?;
    for manifest_record in manifests {
        let ManifestSource::UserRegistered { tenant_id, owner } =
            &manifest_record.manifest().source
        else {
            continue;
        };
        let ExtensionRuntimeV2::Mcp { url: Some(url), .. } = &manifest_record.manifest().runtime
        else {
            continue;
        };
        let minted = match HostedMcpExtensionId::mint(tenant_id, owner, url, "") {
            Ok(minted) => minted,
            Err(error) => {
                tracing::debug!(
                    extension_id = manifest_record.extension_id().as_str(),
                    %error,
                    "skipping registered extension id migration: failed to mint id"
                );
                continue;
            }
        };
        if HostedMcpExtensionId::parse(manifest_record.extension_id())
            .is_ok_and(|parsed| parsed == minted)
        {
            continue;
        }
        if let Err(error) = migrate_registered_id(
            fs,
            installation_store,
            tenant_id,
            owner,
            &manifest_record,
            &minted.0,
        )
        .await
        {
            tracing::debug!(
                extension_id = manifest_record.extension_id().as_str(),
                %error,
                "skipping registered extension id migration"
            );
        }
    }
    Ok(())
}

async fn migrate_registered_id<F>(
    fs: &F,
    installation_store: &Arc<dyn ExtensionInstallationStore>,
    tenant_id: &TenantId,
    owner: &UserId,
    old_manifest: &ExtensionManifestRecord,
    new_id: &ExtensionId,
) -> Result<(), ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    let old_id = old_manifest.extension_id();
    let mut value =
        toml::from_str::<toml::Value>(old_manifest.raw_toml()).map_err(map_binding_error)?;
    value["id"] = toml::Value::String(new_id.as_str().to_string());
    let new_toml = toml::to_string(&value).map_err(map_binding_error)?;
    let new_hash =
        ManifestHash::new(sha256_digest_token(new_toml.as_bytes())).map_err(map_binding_error)?;
    let host_ports =
        ironclaw_host_runtime::default_host_port_catalog().map_err(map_binding_error)?;
    let contracts =
        ironclaw_host_runtime::default_host_api_contract_registry().map_err(map_binding_error)?;
    let new_manifest = ExtensionManifestRecord::from_toml_with_contracts(
        new_toml.clone(),
        ManifestSource::UserRegistered {
            tenant_id: tenant_id.clone(),
            owner: owner.clone(),
        },
        &host_ports,
        Some(new_hash.clone()),
        &contracts,
    )
    .map_err(map_binding_error)?;

    let old_installation_id =
        ExtensionInstallationId::new(old_id.as_str()).map_err(map_binding_error)?;
    let old_installation = installation_store
        .get_installation(&old_installation_id)
        .await
        .map_err(map_transient)?;
    let new_installation = old_installation
        .as_ref()
        .map(|installation| {
            ExtensionInstallation::from_persisted_parts(ExtensionInstallationPersistedParts {
                installation_id: ExtensionInstallationId::new(new_id.as_str())?,
                extension_id: new_id.clone(),
                activation_state: installation.activation_state(),
                manifest_ref: ExtensionManifestRef::new(new_id.clone(), Some(new_hash)),
                credential_bindings: installation.credential_bindings().to_vec(),
                health: installation.health().clone(),
                updated_at: installation.updated_at(),
                owner: installation.owner().clone(),
            })
        })
        .transpose()
        .map_err(map_binding_error)?;

    let source = descriptor_root(tenant_id, owner, old_id)?;
    let destination = descriptor_root(tenant_id, owner, new_id)?;
    let destination_manifest = VirtualPath::new(format!("{}/manifest.toml", destination.as_str()))
        .map_err(map_binding_error)?;
    match fs.stat(&destination_manifest).await {
        Ok(_) => {}
        Err(FilesystemError::NotFound { .. }) => {
            copy_tree(fs, &source, &destination).await?;
            fs.write_file(&destination_manifest, new_toml.as_bytes())
                .await
                .map_err(map_transient)?;
        }
        Err(error) => return Err(map_transient(error)),
    }
    match new_installation {
        Some(installation) => installation_store
            .upsert_manifest_and_installation(new_manifest, installation)
            .await
            .map_err(map_transient)?,
        None => installation_store
            .upsert_manifest(new_manifest)
            .await
            .map_err(map_transient)?,
    }
    if old_installation.is_some() {
        installation_store
            .delete_installation(&old_installation_id)
            .await
            .map_err(map_transient)?;
    }
    installation_store
        .delete_manifest(old_id)
        .await
        .map_err(map_transient)?;
    match fs.delete(&source).await {
        Ok(()) | Err(FilesystemError::NotFound { .. }) => {}
        Err(error) => return Err(map_transient(error)),
    }
    Ok(())
}

fn descriptor_root(
    tenant_id: &TenantId,
    owner: &UserId,
    extension_id: &ExtensionId,
) -> Result<VirtualPath, ProductWorkflowError> {
    VirtualPath::new(format!(
        "{REGISTERED_ROOT}/{}/{}/{}",
        tenant_id.as_str(),
        owner.as_str(),
        extension_id.as_str()
    ))
    .map_err(map_binding_error)
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
        let manifest_bytes = fs.read_file(&manifest).await.map_err(map_transient)?;
        let (minted_id, minted_manifest) = match minted_manifest_for_legacy(&manifest_bytes, owner)
        {
            Ok(result) => result,
            Err(error) => {
                tracing::debug!(
                    owner = owner.as_str(),
                    extension = child.name.as_str(),
                    %error,
                    "skipping legacy registered descriptor: failed to mint id"
                );
                migrated_all = false;
                continue;
            }
        };
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
            minted_id.as_str()
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
        fs.write_file(&destination_manifest, minted_manifest.as_bytes())
            .await
            .map_err(map_transient)?;
        fs.delete(&source).await.map_err(map_transient)?;
    }
    // Only remove the legacy owner dir once everything in it was a migrated
    // descriptor; anything else stays put rather than being destroyed.
    if found_descriptor && migrated_all {
        fs.delete(&legacy_root).await.map_err(map_transient)?;
    }
    Ok(())
}

fn minted_manifest_for_legacy(
    bytes: &[u8],
    owner: &UserId,
) -> Result<(ExtensionId, String), ProductWorkflowError> {
    let raw = std::str::from_utf8(bytes).map_err(map_binding_error)?;
    let mut value = toml::from_str::<toml::Value>(raw).map_err(map_binding_error)?;
    let url = value
        .get("runtime")
        .and_then(|runtime| runtime.get("url"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| ProductWorkflowError::InvalidBindingRequest {
            reason: "legacy registered manifest has no hosted MCP URL".to_string(),
        })?;
    let tenant = TenantId::from_trusted(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID.to_string());
    let minted = HostedMcpExtensionId::mint(&tenant, owner, url, "")?.0;
    value["id"] = toml::Value::String(minted.as_str().to_string());
    let manifest = toml::to_string(&value).map_err(map_binding_error)?;
    Ok((minted, manifest))
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
