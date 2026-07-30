use std::sync::Arc;

use ironclaw_extensions::{
    ExtensionError, ExtensionPackage, ExtensionRegistry, SharedExtensionRegistry,
};
use ironclaw_host_api::{EffectKind, PackageSource};
use ironclaw_product::ProductSurfaceFailure;
use ironclaw_trust::{
    AdminEntry, HostTrustAssignment, HostTrustPolicy, InvalidationBus, TrustError,
};

#[derive(Clone)]
pub struct ActiveExtensionPublisher {
    active_registry: Arc<SharedExtensionRegistry>,
    trust_policy: Arc<HostTrustPolicy>,
    trust_invalidation_bus: Arc<InvalidationBus>,
}

impl ActiveExtensionPublisher {
    pub fn new(
        active_registry: Arc<SharedExtensionRegistry>,
        trust_policy: Arc<HostTrustPolicy>,
        trust_invalidation_bus: Arc<InvalidationBus>,
    ) -> Self {
        Self {
            active_registry,
            trust_policy,
            trust_invalidation_bus,
        }
    }

    pub fn snapshot(&self) -> Arc<ExtensionRegistry> {
        self.active_registry.snapshot()
    }

    pub fn publish(&self, package: &ExtensionPackage) -> Result<(), ProductSurfaceFailure> {
        self.upsert_trust_policy(package)?;
        if let Err(error) = self
            .active_registry
            .upsert(package.clone())
            .map_err(map_extension_error)
        {
            if let Err(cleanup_error) = self.remove_trust_policy(package) {
                return Err(compensation_failure(
                    "extension publish failed to update active registry and trust policy rollback failed",
                    error,
                    cleanup_error,
                ));
            }
            return Err(error);
        }
        Ok(())
    }

    pub fn unpublish(&self, package: &ExtensionPackage) -> Result<(), ProductSurfaceFailure> {
        self.remove_trust_policy(package)?;
        self.active_registry.remove(&package.id);
        Ok(())
    }

    fn upsert_trust_policy(&self, package: &ExtensionPackage) -> Result<(), ProductSurfaceFailure> {
        if package.manifest.source == ironclaw_extensions::ManifestSource::UserRegistered {
            // Direct-remote packages remain on the trust engine's untrusted
            // default. Registration is provenance, never an implicit admin
            // elevation.
            return Ok(());
        }
        let input = extension_trust_policy_input(package)?;
        let manifest_path = extension_local_manifest_path(package)?;
        let entry = AdminEntry::for_local_manifest(
            input.identity.package_id.clone(),
            manifest_path,
            package.manifest_digest(),
            HostTrustAssignment::user_trusted(),
            extension_allowed_effects(package),
            None,
        );
        self.trust_policy
            .mutate_with(
                &self.trust_invalidation_bus,
                input.identity,
                input.requested_authority,
                input.requested_trust,
                move |sources| {
                    sources.admin_upsert(entry)?;
                    Ok(())
                },
            )
            .map_err(map_trust_policy_error)
    }

    fn remove_trust_policy(&self, package: &ExtensionPackage) -> Result<(), ProductSurfaceFailure> {
        if package.manifest.source == ironclaw_extensions::ManifestSource::UserRegistered {
            return Ok(());
        }
        let input = extension_trust_policy_input(package)?;
        let package_id = input.identity.package_id.clone();
        let source = extension_package_source(package)?;
        self.trust_policy
            .mutate_with(
                &self.trust_invalidation_bus,
                input.identity,
                input.requested_authority,
                input.requested_trust,
                move |sources| {
                    sources.admin_remove(&package_id, &source)?;
                    Ok(())
                },
            )
            .map(|_| ())
            .map_err(map_trust_policy_error)
    }
}

pub fn extension_trust_policy_input(
    package: &ExtensionPackage,
) -> Result<ironclaw_trust::TrustPolicyInput, ProductSurfaceFailure> {
    package
        .trust_policy_input(
            extension_package_source(package)?,
            package.manifest_digest(),
            None,
        )
        .map_err(map_extension_error)
}

fn extension_package_source(
    package: &ExtensionPackage,
) -> Result<PackageSource, ProductSurfaceFailure> {
    if package.manifest.source == ironclaw_extensions::ManifestSource::UserRegistered {
        let ironclaw_extensions::ExtensionRuntime::Mcp {
            url: Some(endpoint),
            ..
        } = &package.manifest.runtime
        else {
            return Err(ProductSurfaceFailure::InvalidBindingRequest {
                reason: "user-registered extension lacks a direct remote MCP endpoint".to_string(),
            });
        };
        return Ok(PackageSource::DirectRemote {
            endpoint: endpoint.clone(),
        });
    }
    Ok(PackageSource::LocalManifest {
        path: extension_local_manifest_path(package)?,
    })
}

fn extension_local_manifest_path(
    package: &ExtensionPackage,
) -> Result<String, ProductSurfaceFailure> {
    let root = package.materialized_root().map_err(|error| {
        ProductSurfaceFailure::InvalidBindingRequest {
            reason: format!("local extension package has no materialized root: {error}"),
        }
    })?;
    Ok(format!(
        "{}/manifest.toml",
        root.as_str().trim_end_matches('/')
    ))
}

fn extension_allowed_effects(package: &ExtensionPackage) -> Vec<EffectKind> {
    let mut effects = Vec::new();
    for descriptor in &package.capabilities {
        for effect in &descriptor.effects {
            if !effects.contains(effect) {
                effects.push(*effect);
            }
        }
    }
    effects
}

fn map_trust_policy_error(error: TrustError) -> ProductSurfaceFailure {
    ProductSurfaceFailure::InvalidBindingRequest {
        reason: format!("extension trust policy update failed: {error}"),
    }
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

fn compensation_failure(
    context: &str,
    original: impl std::fmt::Display,
    compensation: impl std::fmt::Display,
) -> ProductSurfaceFailure {
    ProductSurfaceFailure::Transient {
        reason: format!(
            "{context}; original error: {original}; compensation error: {compensation}"
        ),
    }
}
