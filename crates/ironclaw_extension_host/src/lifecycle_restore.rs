use std::{collections::BTreeSet, sync::Arc};

use ironclaw_extensions::{
    CapabilityVisibility, ExtensionError, ExtensionInstallation, ExtensionInstallationError,
    ExtensionInstallationId, ExtensionInstallationStorePort, ExtensionLifecycleService,
    ExtensionManifestRecord, ExtensionManifestRef, ExtensionPackage, InstallationOwner,
    ManifestHash, ManifestSource, canonicalize_installation_rows,
};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{approval::sha256_digest_token, ids::UserId};
use ironclaw_product::{LifecyclePackageKind, LifecyclePackageRef, ProductSurfaceFailure};
use tokio::sync::Mutex;

use crate::{
    ActiveExtensionPublisher, AvailableExtensionCatalog, AvailableExtensionPackage,
    materialize_available_extension, product_extension_host_api_contract_registry,
};

const RETIRED_SLACK_USER_EXTENSION_ID: &str = "slack_user";

pub async fn restore_extension_lifecycle_state(
    catalog: &mut AvailableExtensionCatalog,
    filesystem: &Arc<dyn RootFilesystem>,
    installation_store: &Arc<dyn ExtensionInstallationStorePort>,
    lifecycle_service: &Arc<Mutex<ExtensionLifecycleService>>,
    active_extensions: &ActiveExtensionPublisher,
    legacy_tenant_owner: &UserId,
) -> Result<(), ProductSurfaceFailure> {
    let mut catalog_registered_user_extension_ids: BTreeSet<String> = BTreeSet::new();
    for installation in
        canonicalize_persisted_installation_rows(installation_store, legacy_tenant_owner).await?
    {
        let stored_manifest = installation_store
            .get_manifest(installation.extension_id())
            .await
            .map_err(map_extension_installation_error)?;
        if let Some(manifest) = stored_manifest.as_ref()
            && manifest.manifest().source == ManifestSource::UserRegistered
        {
            let available = crate::hosted_mcp_manifest::available_package(manifest)?;
            catalog.extend(AvailableExtensionCatalog::from_packages(vec![available]));
            catalog_registered_user_extension_ids
                .insert(installation.extension_id().as_str().to_string());
        }
        if remove_retired_internal_installation(installation_store, &installation).await? {
            continue;
        }
        let package_ref = LifecyclePackageRef::new(
            LifecyclePackageKind::Extension,
            installation.extension_id().as_str(),
        )?;
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
        if available.source != ManifestSource::UserRegistered {
            materialize_available_extension(filesystem.as_ref(), &available).await?;
        }
        // Derived from the package itself, not a separate persisted flag: a
        // registration whose package does not yet publish any model-visible
        // capability or channel/hook surface has nothing to serve. Enabling
        // it now would mark it durably active before its setup — whatever
        // form that setup takes for this registration kind — has produced
        // anything to activate. Once a later restore or live preparation
        // pass rebuilds the package with a real surface, this same check
        // passes and the installation enables normally.
        let has_visible_capabilities =
            !package_visible_capability_ids(&available.package).is_empty();
        let has_activatable_surface = has_visible_capabilities
            || available.resolved_manifest.channel.is_some()
            || !available.resolved_manifest.hooks.is_empty();
        {
            let mut lifecycle = lifecycle_service.lock().await;
            lifecycle
                .install(available.package.clone())
                .await
                .map_err(map_extension_error)?;
            if !has_activatable_surface {
                tracing::debug!(
                    extension_id = installation.extension_id().as_str(),
                    "registered extension installation has no activatable capability or \
                     channel surface yet during offline restore; not enabling"
                );
                continue;
            }
            lifecycle
                .enable(&available.package.id)
                .await
                .map_err(map_extension_error)?;
        }
        active_extensions.publish(&available.package)?;
    }
    restore_registered_only_definitions(
        catalog,
        installation_store,
        &catalog_registered_user_extension_ids,
    )
    .await?;
    Ok(())
}

/// Repopulate the catalog with hosted MCP definitions that were registered
/// but never installed, or whose last installation was later removed. The
/// durable `registered-definitions/{id}.json` row survives that removal (see
/// `installations.rs`'s `registered_definition_survives_its_final_installation_removal`),
/// but no installation row exists to drive it through the loop above — so
/// without this sweep the definition silently disappears from the registry
/// after a restart. This only extends the in-memory catalog: it must not
/// install into the lifecycle service, enable, or publish, since a
/// registered-but-uninstalled definition confers no invocation and no active
/// surface.
async fn restore_registered_only_definitions(
    catalog: &mut AvailableExtensionCatalog,
    installation_store: &Arc<dyn ExtensionInstallationStorePort>,
    already_contributed: &BTreeSet<String>,
) -> Result<(), ProductSurfaceFailure> {
    let registered_definitions = installation_store
        .list_registered_package_definitions()
        .await
        .map_err(map_extension_installation_error)?;
    for manifest in &registered_definitions {
        if manifest.manifest().source != ManifestSource::UserRegistered {
            continue;
        }
        if already_contributed.contains(manifest.extension_id().as_str()) {
            continue;
        }
        match crate::hosted_mcp_manifest::available_package(manifest) {
            Ok(available) => {
                catalog.extend(AvailableExtensionCatalog::from_packages(vec![available]));
            }
            Err(error) => {
                tracing::warn!(
                    extension_id = manifest.extension_id().as_str(),
                    %error,
                    "skipping registered hosted MCP definition restore: definition is not \
                     convertible into an available package"
                );
            }
        }
    }
    Ok(())
}

async fn canonicalize_persisted_installation_rows(
    installation_store: &Arc<dyn ExtensionInstallationStorePort>,
    legacy_tenant_owner: &UserId,
) -> Result<Vec<ExtensionInstallation>, ProductSurfaceFailure> {
    let persisted = installation_store
        .list_installations()
        .await
        .map_err(map_extension_installation_error)?;
    let repaired = persisted
        .iter()
        .cloned()
        .map(|installation| {
            if installation.owner().is_tenant() {
                installation.with_owner(InstallationOwner::user(legacy_tenant_owner.clone()))
            } else {
                installation
            }
        })
        .collect::<Vec<_>>();
    let canonical =
        canonicalize_installation_rows(repaired).map_err(map_extension_installation_error)?;
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
) -> Result<bool, ProductSurfaceFailure> {
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

pub struct ExtensionInstallPlan {
    pub manifest_record: ExtensionManifestRecord,
    pub installation: ExtensionInstallation,
}

pub fn prepare_install(
    available: &AvailableExtensionPackage,
    owner: InstallationOwner,
    retained_definition: Option<ExtensionManifestRecord>,
) -> Result<ExtensionInstallPlan, ProductSurfaceFailure> {
    let manifest_hash = available_manifest_hash(available)?;
    let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
        ProductSurfaceFailure::InvalidBindingRequest {
            reason: format!("host port catalog rejected extension install: {error}"),
        }
    })?;
    let contracts = product_extension_host_api_contract_registry().map_err(|error| {
        ProductSurfaceFailure::InvalidBindingRequest {
            reason: format!("host API contract registry rejected extension install: {error}"),
        }
    })?;
    let catalog_record = manifest_record_for_available(
        available,
        &host_ports,
        &contracts,
        Some(manifest_hash.clone()),
    )?
    // Install records no readiness verdict. "Nothing model-visible yet" is
    // read from the package itself
    // (`ResolvedExtensionManifest::has_model_visible_capabilities`), so a
    // package whose capabilities arrive from a later runtime step needs no
    // stored flag — and, more importantly, a package that never has such a
    // step is never asked the question.
    .with_removal_cleanup_requirements(available.cleanup_requirements.clone());
    let manifest_record = match retained_definition {
        Some(retained)
            if retained.raw_toml() == catalog_record.raw_toml()
                && retained.resolved() == catalog_record.resolved()
                && retained.manifest_hash() == catalog_record.manifest_hash() =>
        {
            retained
        }
        Some(_) => {
            return Err(ProductSurfaceFailure::InvalidBindingRequest {
                reason: format!(
                    "extension {} has a conflicting registered definition",
                    available.package.id.as_str()
                ),
            });
        }
        None => catalog_record,
    };
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

fn manifest_record_for_available(
    available: &AvailableExtensionPackage,
    host_ports: &ironclaw_host_api::host_port::HostPortCatalog,
    contracts: &ironclaw_extensions::HostApiContractRegistry,
    manifest_hash: Option<ironclaw_extensions::ManifestHash>,
) -> Result<ExtensionManifestRecord, ProductSurfaceFailure> {
    let _ = (host_ports, contracts);
    let mut resolved = available.resolved_manifest.as_ref().clone();
    resolved.root_binding = available.package.root_binding.clone();
    ExtensionManifestRecord::from_resolved(
        &available.manifest_toml,
        available.source,
        resolved,
        manifest_hash,
    )
    .map_err(map_extension_installation_error)
}

fn prepare_manifest_migration(
    available: &AvailableExtensionPackage,
    existing: &ExtensionInstallation,
) -> Result<ExtensionInstallPlan, ProductSurfaceFailure> {
    let manifest_hash = available_manifest_hash(available)?;
    let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
        ProductSurfaceFailure::InvalidBindingRequest {
            reason: format!("host port catalog rejected manifest migration: {error}"),
        }
    })?;
    let contracts = product_extension_host_api_contract_registry().map_err(|error| {
        ProductSurfaceFailure::InvalidBindingRequest {
            reason: format!("host API contract registry rejected manifest migration: {error}"),
        }
    })?;
    let manifest_record = manifest_record_for_available(
        available,
        &host_ports,
        &contracts,
        Some(manifest_hash.clone()),
    )?
    .with_removal_cleanup_requirements(available.cleanup_requirements.clone());
    let installation = ExtensionInstallation::new(
        existing.installation_id().clone(),
        existing.extension_id().clone(),
        ExtensionManifestRef::new(existing.extension_id().clone(), Some(manifest_hash)),
        existing.credential_bindings().to_vec(),
        chrono::Utc::now(),
        existing.owner().clone(),
    )
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
    hash_error: ProductSurfaceFailure,
) -> Result<(), ProductSurfaceFailure> {
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
) -> Result<(), ProductSurfaceFailure> {
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

pub fn available_manifest_hash(
    available: &AvailableExtensionPackage,
) -> Result<ManifestHash, ProductSurfaceFailure> {
    ManifestHash::new(sha256_digest_token(available.manifest_toml.as_bytes()))
        .map_err(map_extension_installation_error)
}

pub fn package_visible_capability_ids(package: &ExtensionPackage) -> Vec<String> {
    package
        .manifest
        .capabilities
        .iter()
        .filter(|capability| capability.visibility == CapabilityVisibility::Model)
        .map(|capability| capability.id.as_str().to_string())
        .collect()
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
        error @ ExtensionInstallationError::StoreUnavailable { .. } => {
            ProductSurfaceFailure::Transient {
                reason: error.to_string(),
            }
        }
        error => ProductSurfaceFailure::InvalidBindingRequest {
            reason: error.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::RETIRED_SLACK_USER_EXTENSION_ID;

    #[test]
    fn retired_slack_user_id_remains_stable() {
        assert_eq!(RETIRED_SLACK_USER_EXTENSION_ID, "slack_user");
    }
}
