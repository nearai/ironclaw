//! Owner-registered lifecycle slice, extracted from the parent module
//! (behavior-preserving move; no logic changes) to keep
//! `extension_lifecycle.rs` from growing further: the row-authoritative
//! owner-scope helpers, the live (non-boot) registered resolution path, the
//! registered-vs-scope tenant guard, and boot restore's per-owner-batched
//! registered fallback. Mirrors the `install_policy.rs` precedent (pure
//! decisions extracted alongside the lifecycle I/O that calls them) — the
//! I/O (installation store reads, filesystem walks) still lives here since
//! these helpers ARE that I/O, but the store/service wiring and every other
//! lifecycle concern stays in the parent.

use std::{collections::HashMap, sync::Arc};

use ironclaw_extensions::{ExtensionInstallation, ExtensionInstallationStore, ManifestSource};
use ironclaw_host_api::{ExtensionId, ResourceScope, TenantId, UserId};
use ironclaw_product_workflow::{LifecyclePackageRef, ProductWorkflowError};

use crate::extension_host::available_extensions::AvailableExtensionPackage;
use crate::extension_host::registered_extension_store::{
    FilesystemRegisteredExtensionStore, is_owner_registered,
};

use super::{
    RebornLocalExtensionManagementPort, extension_ids_from_package_ref,
    map_extension_installation_error,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OwnerScope {
    tenant_id: TenantId,
    owner: UserId,
}

impl OwnerScope {
    #[cfg(test)]
    pub(super) fn new(tenant_id: TenantId, owner: UserId) -> Self {
        Self { tenant_id, owner }
    }

    pub(super) fn matches(&self, scope: &ResourceScope) -> bool {
        self.tenant_id == scope.tenant_id && self.owner == scope.user_id
    }

    pub(super) fn matches_tenant(&self, tenant_id: &TenantId) -> bool {
        &self.tenant_id == tenant_id
    }

    pub(super) fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub(super) fn owner(&self) -> &UserId {
        &self.owner
    }
}

/// Effective owner scope (tenant + user) of a registered installation,
/// ROW-AUTHORITATIVE on the user axis: the row's `InstallationOwner`
/// singleton member wins over a disagreeing `UserRegistered.owner` (a stale
/// re-registered manifest must not re-point the install). Tenant has no row
/// counterpart, so it comes from manifest provenance. `None` for
/// non-registered sources or a non-singleton owner set.
pub(super) fn effective_owner_scope(
    installation: &ExtensionInstallation,
    source: &ManifestSource,
) -> Option<OwnerScope> {
    let ManifestSource::UserRegistered { tenant_id, .. } = source else {
        return None;
    };
    let members = installation.owner().members()?;
    let mut members = members.iter();
    let row_owner = members.next()?;
    if members.next().is_some() {
        return None;
    }
    Some(OwnerScope {
        tenant_id: tenant_id.clone(),
        owner: row_owner.clone(),
    })
}

/// The row-authoritative registered-store owner scope for an installation.
/// `Ok(None)` is a genuine miss (no stored manifest, non-registered source,
/// or non-singleton owner set); `Err` is a real store I/O failure and must
/// never collapse into `Ok(None)`, or a mutation guard's `None` arm fails
/// OPEN on a transient read error. Boot restore skips-and-logs on `Err`;
/// install/activate/remove guards propagate it.
pub(super) async fn installation_effective_owner_scope(
    installation_store: &Arc<dyn ExtensionInstallationStore>,
    installation: &ExtensionInstallation,
) -> Result<Option<OwnerScope>, ProductWorkflowError> {
    let stored_manifest = match installation_store
        .get_manifest(installation.extension_id())
        .await
        .map_err(map_extension_installation_error)?
    {
        Some(record) => record,
        None => {
            // Unreachable by construction today: install/rollback ordering
            // never persists an installation row without its manifest, so
            // this collapses to the same `Ok(None)` a non-registered source
            // produces. Logged so a future persistence change that broke
            // that invariant fails loudly in review instead of silently
            // bypassing the tenant guard.
            tracing::debug!(
                extension_id = installation.extension_id().as_str(),
                installation_id = installation.installation_id().as_str(),
                "installation row has no stored manifest; treating as non-registered"
            );
            return Ok(None);
        }
    };
    Ok(effective_owner_scope(
        installation,
        &stored_manifest.manifest().source,
    ))
}

/// A `registered_by_owner` cache entry: either the loaded registered set, or
/// a marker that this owner's `list_for_owner` walk already failed this
/// boot. The `Failed` marker is what makes the failure itself cacheable —
/// without it, every subsequent installation for the same owner re-walks the
/// filesystem and re-logs the same failure.
pub(super) enum RegisteredOwnerLookup {
    Loaded(HashMap<ExtensionId, AvailableExtensionPackage>),
    Failed,
}

/// Boot restore's row-provenance-wins registered lookup (review item 1):
/// called only once the caller has already determined the row's stored
/// manifest source is `UserRegistered` (`installation_effective_owner_scope`
/// returned `Some`). Batches each distinct (tenant, owner)'s registered set
/// (or its failure) into `registered_by_owner` at most once per boot —
/// multiple installations frequently share an owner, and each load is a full
/// directory walk + manifest parse. `Ok(None)` means the caller must
/// skip-and-log-and-continue this one installation (already logged here);
/// `Ok(Some(_))` is the resolved package to restore.
pub(super) async fn resolve_registered_installation_for_restore(
    registered_store: &FilesystemRegisteredExtensionStore,
    registered_by_owner: &mut HashMap<(TenantId, UserId), RegisteredOwnerLookup>,
    tenant_id: &TenantId,
    owner: &UserId,
    installation: &ExtensionInstallation,
) -> Result<Option<Arc<AvailableExtensionPackage>>, ProductWorkflowError> {
    let owner_key = (tenant_id.clone(), owner.clone());
    if !registered_by_owner.contains_key(&owner_key) {
        match registered_store.list_for_owner(tenant_id, owner).await {
            Ok(packages) => {
                let by_id = packages
                    .into_iter()
                    .map(|package| (package.package.id.clone(), package))
                    .collect();
                registered_by_owner.insert(owner_key.clone(), RegisteredOwnerLookup::Loaded(by_id));
            }
            Err(error) => {
                tracing::debug!(
                    extension_id = installation.extension_id().as_str(),
                    installation_id = installation.installation_id().as_str(),
                    %error,
                    "skipping extension installation restore: failed to load its row-owned registered set"
                );
                registered_by_owner.insert(owner_key.clone(), RegisteredOwnerLookup::Failed);
                return Ok(None);
            }
        }
    }
    match registered_by_owner.get(&owner_key) {
        Some(RegisteredOwnerLookup::Loaded(by_id)) => {
            match by_id.get(installation.extension_id()) {
                Some(available) => Ok(Some(Arc::new(available.clone()))),
                None => {
                    tracing::debug!(
                        extension_id = installation.extension_id().as_str(),
                        installation_id = installation.installation_id().as_str(),
                        "skipping extension installation restore: row is registered-scoped but its descriptor is not in its row-owned registered store"
                    );
                    Ok(None)
                }
            }
        }
        // The owner's walk already failed for an earlier installation this
        // boot (already logged then) — skip without re-walking or re-logging.
        Some(RegisteredOwnerLookup::Failed) | None => Ok(None),
    }
}

impl RebornLocalExtensionManagementPort {
    /// True unless `source` is a registered package whose installation row's
    /// own stored-manifest tenant diverges from `scope`'s tenant. The
    /// installation store has no tenant axis (one flat map keyed by extension
    /// id), so a registered row must be re-checked against its OWN effective
    /// tenant before it's allowed to project as installed for this scope;
    /// non-registered sources have no such row to diverge from and always
    /// match.
    pub(super) async fn registered_row_matches_scope_tenant(
        &self,
        source: &ManifestSource,
        installation: &ExtensionInstallation,
        scope: &ResourceScope,
    ) -> Result<bool, ProductWorkflowError> {
        if !is_owner_registered(source) {
            return Ok(true);
        }
        let effective_tenant =
            installation_effective_owner_scope(&self.installation_store, installation)
                .await?
                .map(|owner_scope| owner_scope.matches(scope));
        Ok(effective_tenant == Some(true))
    }

    /// Pure (no I/O) sibling of [`Self::registered_row_matches_scope_tenant`]
    /// for listing loops that have already batch-loaded every stored
    /// manifest once (item 5): `stored_source` is the row's OWN manifest
    /// source, looked up from that batch instead of a per-row
    /// `get_manifest` call. Same semantics as the async version — a missing
    /// stored manifest collapses to "does not match" for a registered
    /// source, matching `installation_effective_owner_scope`'s `None` arm.
    pub(super) fn registered_row_matches_scope_tenant_batched(
        source: &ManifestSource,
        installation: &ExtensionInstallation,
        stored_source: Option<&ManifestSource>,
        scope: &ResourceScope,
    ) -> bool {
        if !is_owner_registered(source) {
            return true;
        }
        let effective_tenant = stored_source
            .and_then(|stored_source| effective_owner_scope(installation, stored_source))
            .map(|owner_scope| owner_scope.matches(scope));
        effective_tenant == Some(true)
    }

    /// The caller's registered packages, read once and keyed by extension id
    /// — the batched replacement for calling `resolve_registered_for_scope`
    /// (which internally lists the caller's entire registered directory) once
    /// per catalog-miss in a listing loop. A read failure is logged and
    /// treated as an empty registered set for this request (list-shaped
    /// callers stay resilient, matching `resolve_registered_for_scope`'s
    /// per-entry blast-radius stance).
    pub(super) async fn registered_packages_by_id(
        &self,
        scope: &ResourceScope,
    ) -> HashMap<ExtensionId, Arc<AvailableExtensionPackage>> {
        // Item 6: `installed_summaries` (this method's only caller) reads
        // only summary fields, never `.assets` — skip the per-entry
        // directory-asset read.
        match self
            .registered_store
            .list_for_scope(
                scope,
                crate::extension_host::available_extensions::AssetLoading::Skip,
            )
            .await
        {
            Ok(packages) => packages
                .into_iter()
                .map(|package| (package.package.id.clone(), Arc::new(package)))
                .collect(),
            Err(error) => {
                // silent-ok: a read failure here only degrades one request's
                // listing to catalog-only entries (registered rows silently
                // absent from `installed_summaries`); it never mutates state
                // or masks an ownership decision, so failing open is safe.
                tracing::debug!(
                    %error,
                    "skipping registered extension listing for installed summaries: batched read failed"
                );
                HashMap::new()
            }
        }
    }

    /// `Ok(Some(_))` only when an installation row exists for `package_ref`'s
    /// id, its stored manifest source is `UserRegistered`, AND `scope`'s
    /// caller may see that row — the row-provenance-wins case
    /// `resolve_available_for_scope` must resolve before ever consulting the
    /// shared catalog. The visibility check keeps this from becoming a
    /// resolution oracle for a foreign caller: without it, a foreign owner's
    /// private registered row would resolve successfully here (row-provenance
    /// consults the ROW's own owner, not the caller's), undoing the
    /// masked-as-not-found behavior every ownership-aware caller relies on.
    /// Reuses `installation_effective_owner_scope` (row-authoritative on the
    /// user axis) to derive the row's OWN tenant-owner scope, then
    /// `list_for_owner` (the same helper boot restore batches by owner) to
    /// locate the descriptor — never the caller's scope for the LOOKUP
    /// (which may legitimately differ in tenant from the row for a T2
    /// cross-tenant caller sharing a user id, handled by the visibility
    /// check above). `Ok(None)` when there is no row, the row isn't
    /// registered-sourced, the caller cannot see it, or the row's own
    /// registered descriptor cannot be found (a genuine miss, not a catalog
    /// fallback — the row's provenance already ruled the catalog out).
    pub(super) async fn resolve_registered_row_for_package_ref(
        &self,
        package_ref: &LifecyclePackageRef,
        scope: &ResourceScope,
    ) -> Result<Option<Arc<AvailableExtensionPackage>>, ProductWorkflowError> {
        let (_, installation_id) = extension_ids_from_package_ref(package_ref)?;
        let Some(installation) = self
            .installation_store
            .get_installation(&installation_id)
            .await
            .map_err(map_extension_installation_error)?
        else {
            return Ok(None);
        };
        let Some(owner_scope) =
            installation_effective_owner_scope(&self.installation_store, &installation).await?
        else {
            return Ok(None);
        };
        if !owner_scope.matches(scope) {
            return Ok(None);
        }
        Ok(self
            .registered_store
            .list_for_owner(owner_scope.tenant_id(), owner_scope.owner())
            .await?
            .into_iter()
            .find(|package| &package.package_ref == package_ref)
            .map(Arc::new))
    }
}
