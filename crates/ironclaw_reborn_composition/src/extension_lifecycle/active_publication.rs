use std::sync::Arc;

use ironclaw_extensions::{ExtensionPackage, ExtensionRegistry, SharedExtensionRegistry};
use ironclaw_host_api::{EffectKind, PackageSource};
use ironclaw_product_workflow::ProductWorkflowError;
use ironclaw_trust::{
    AdminEntry, HostTrustAssignment, HostTrustPolicy, InvalidationBus, TrustError,
};

use super::{compensation_failure, map_extension_error};

#[derive(Clone)]
pub(crate) struct ActiveExtensionPublisher {
    active_registry: Arc<SharedExtensionRegistry>,
    trust_policy: Arc<HostTrustPolicy>,
    trust_invalidation_bus: Arc<InvalidationBus>,
}

impl ActiveExtensionPublisher {
    pub(crate) fn new(
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

    pub(crate) fn snapshot(&self) -> Arc<ExtensionRegistry> {
        self.active_registry.snapshot()
    }

    pub(crate) fn publish(&self, package: &ExtensionPackage) -> Result<(), ProductWorkflowError> {
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

    pub(crate) fn unpublish(&self, package: &ExtensionPackage) -> Result<(), ProductWorkflowError> {
        self.remove_trust_policy(package)?;
        self.active_registry.remove(&package.id);
        Ok(())
    }

    fn upsert_trust_policy(&self, package: &ExtensionPackage) -> Result<(), ProductWorkflowError> {
        let input = extension_trust_policy_input(package)?;
        let manifest_path = extension_local_manifest_path(package);
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

    fn remove_trust_policy(&self, package: &ExtensionPackage) -> Result<(), ProductWorkflowError> {
        let input = extension_trust_policy_input(package)?;
        let package_id = input.identity.package_id.clone();
        let source = extension_local_manifest_source(package);
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

pub(crate) fn extension_trust_policy_input(
    package: &ExtensionPackage,
) -> Result<ironclaw_trust::TrustPolicyInput, ProductWorkflowError> {
    package
        .trust_policy_input(
            extension_local_manifest_source(package),
            package.manifest_digest(),
            None,
        )
        .map_err(map_extension_error)
}

fn extension_local_manifest_source(package: &ExtensionPackage) -> PackageSource {
    PackageSource::LocalManifest {
        path: extension_local_manifest_path(package),
    }
}

fn extension_local_manifest_path(package: &ExtensionPackage) -> String {
    format!(
        "{}/manifest.toml",
        package.root.as_str().trim_end_matches('/')
    )
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

fn map_trust_policy_error(error: TrustError) -> ProductWorkflowError {
    ProductWorkflowError::InvalidBindingRequest {
        reason: format!("extension trust policy update failed: {error}"),
    }
}
