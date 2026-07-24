use std::{collections::BTreeSet, sync::Arc};

use ironclaw_extensions::{
    CapabilityVisibility, ExtensionActivationState, ExtensionError, ExtensionInstallation,
    ExtensionInstallationError, ExtensionInstallationId, ExtensionInstallationStore,
    ExtensionLifecycleService, ExtensionManifestRecord, ExtensionManifestRef, ExtensionPackage,
    InstallationOwner, ManifestHash, ManifestSource, canonicalize_installation_rows,
};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::sha256_digest_token;
use ironclaw_product::{LifecyclePackageKind, LifecyclePackageRef, ProductSurfaceFailure};
use tokio::sync::Mutex;

use crate::{
    ActiveExtensionPublisher, AvailableExtensionCatalog, AvailableExtensionPackage,
    materialize_available_extension, product_extension_host_api_contract_registry,
};

const RETIRED_SLACK_USER_EXTENSION_ID: &str = "slack_user";

pub async fn restore_extension_lifecycle_state(
    catalog: &AvailableExtensionCatalog,
    filesystem: &Arc<dyn RootFilesystem>,
    installation_store: &Arc<dyn ExtensionInstallationStore>,
    lifecycle_service: &Arc<Mutex<ExtensionLifecycleService>>,
    active_extensions: &ActiveExtensionPublisher,
) -> Result<(), ProductSurfaceFailure> {
    for installation in canonicalize_persisted_installation_rows(installation_store).await? {
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

async fn canonicalize_persisted_installation_rows(
    installation_store: &Arc<dyn ExtensionInstallationStore>,
) -> Result<Vec<ExtensionInstallation>, ProductSurfaceFailure> {
    let persisted = installation_store
        .list_installations()
        .await
        .map_err(map_extension_installation_error)?;
    let canonical = canonicalize_installation_rows(persisted.clone())
        .map_err(map_extension_installation_error)?;
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
    installation_store: &Arc<dyn ExtensionInstallationStore>,
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
    let manifest_record = ExtensionManifestRecord::from_toml(
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
