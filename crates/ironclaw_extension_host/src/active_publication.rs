use std::sync::Arc;

use ironclaw_extensions::{ExtensionPackage, ExtensionRegistry, SharedExtensionRegistry};
use ironclaw_host_api::{capability::EffectKind, trust::PackageSource};
use ironclaw_product_contracts::error::ProductOperationFailure;
use ironclaw_trust::{
    AdminEntry, HostTrustAssignment, HostTrustPolicy, InvalidationBus, TrustError,
};

// One classifier for `ExtensionError`, not one per module. This file and
// `lifecycle_restore.rs` each carried a byte-identical private copy of the
// `pub(crate)` helper below, so the same `ExtensionError` could have drifted
// into different retry classifications on the publication, restore, and
// lifecycle paths. Raised by CodeRabbit on #7000. `hosted_mcp_manifest.rs`
// already imported the shared one — these two are simply catching up.
use crate::product_lifecycle::map_extension_error;

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

    pub fn publish(&self, package: &ExtensionPackage) -> Result<(), ProductOperationFailure> {
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

    pub fn unpublish(&self, package: &ExtensionPackage) -> Result<(), ProductOperationFailure> {
        self.remove_trust_policy(package)?;
        self.active_registry.remove(&package.id);
        Ok(())
    }

    fn upsert_trust_policy(
        &self,
        package: &ExtensionPackage,
    ) -> Result<(), ProductOperationFailure> {
        let input = extension_trust_policy_input(package)?;
        let entry = match &input.identity.source {
            PackageSource::DirectRemote { endpoint } => {
                // Registration only records untrusted provenance. Publication
                // is reached after lifecycle activation; this source- and
                // digest-pinned ceiling lets the kernel authorize that active
                // package. It is not a grant: the owner-filtered active
                // surface still mints the per-user invocation grant, so trust
                // alone cannot make a tenant-registered MCP callable.
                AdminEntry::for_direct_remote(
                    input.identity.package_id.clone(),
                    endpoint.clone(),
                    package.manifest_digest(),
                    HostTrustAssignment::user_trusted(),
                    extension_allowed_effects(package),
                    None,
                )
            }
            PackageSource::LocalManifest { path } => AdminEntry::for_local_manifest(
                input.identity.package_id.clone(),
                path.clone(),
                package.manifest_digest(),
                HostTrustAssignment::user_trusted(),
                extension_allowed_effects(package),
                None,
            ),
            source => {
                return Err(ProductOperationFailure::InvalidBindingRequest {
                    reason: format!("extension package has unsupported trust source: {source:?}"),
                });
            }
        };
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

    fn remove_trust_policy(
        &self,
        package: &ExtensionPackage,
    ) -> Result<(), ProductOperationFailure> {
        let input = extension_trust_policy_input(package)?;
        let package_id = input.identity.package_id.clone();
        let source = input.identity.source.clone();
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
) -> Result<ironclaw_trust::TrustPolicyInput, ProductOperationFailure> {
    package
        .trust_policy_input(
            package.trust_policy_source().map_err(map_extension_error)?,
            package.manifest_digest(),
            None,
        )
        .map_err(map_extension_error)
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

fn map_trust_policy_error(error: TrustError) -> ProductOperationFailure {
    ProductOperationFailure::InvalidBindingRequest {
        reason: format!("extension trust policy update failed: {error}"),
    }
}

fn compensation_failure(
    context: &str,
    original: impl std::fmt::Display,
    compensation: impl std::fmt::Display,
) -> ProductOperationFailure {
    ProductOperationFailure::Transient {
        reason: format!(
            "{context}; original error: {original}; compensation error: {compensation}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_extension_contracts::hosted_mcp::{HostedMcpAuthSelection, HostedMcpEndpoint};
    use ironclaw_extensions::{ExtensionRegistry, SharedExtensionRegistry};
    use ironclaw_host_api::{ids::ExtensionId, runtime::TrustClass};
    use ironclaw_trust::{
        AdminConfig, HostTrustPolicy, InvalidationBus, TrustPolicy, TrustProvenance,
    };

    use super::{
        ActiveExtensionPublisher, compensation_failure, extension_trust_policy_input,
        map_extension_error, map_trust_policy_error,
    };
    use ironclaw_extensions::ExtensionError;
    use ironclaw_product_contracts::error::ProductOperationFailure;
    use ironclaw_trust::TrustError;

    /// Publication runs inside the activation transaction, so its boundary
    /// mappers decide whether a half-published extension is retried or
    /// abandoned. A trust-policy rejection is always the definition's fault;
    /// an infrastructure failure never is.
    #[test]
    fn publication_failures_classify_trust_rejections_apart_from_infrastructure() {
        assert_eq!(
            map_trust_policy_error(TrustError::InvariantViolation {
                reason: "unknown trust class".to_string(),
            }),
            ProductOperationFailure::InvalidBindingRequest {
                reason: "extension trust policy update failed: trust policy invariant violation: \
                         unknown trust class"
                    .to_string(),
            },
            "the policy's own reason must reach the caller"
        );

        assert!(
            matches!(
                map_extension_error(ExtensionError::Filesystem(
                    ironclaw_filesystem::FilesystemError::MountNotFound {
                        path: ironclaw_host_api::path::VirtualPath::new("/system/extensions")
                            .expect("valid path"),
                    }
                )),
                ProductOperationFailure::Transient { .. }
            ),
            "a filesystem failure while publishing is retryable"
        );
        assert!(
            matches!(
                map_extension_error(ExtensionError::DuplicateExtension {
                    id: ExtensionId::new("gmail").expect("valid extension id"),
                }),
                ProductOperationFailure::InvalidBindingRequest { .. }
            ),
            "a duplicate registration is a request problem, not an outage"
        );
    }

    /// When publication fails and its rollback also fails, both causes must
    /// survive into one retryable failure — dropping either one leaves a
    /// half-published extension with no way to tell what happened.
    #[test]
    fn a_failed_publication_rollback_reports_both_causes() {
        assert_eq!(
            compensation_failure(
                "active publication rollback failed",
                "publish rejected",
                "restore rejected",
            ),
            ProductOperationFailure::Transient {
                reason: "active publication rollback failed; original error: publish rejected; \
                         compensation error: restore rejected"
                    .to_string(),
            },
        );
    }

    #[test]
    fn publishing_user_registered_mcp_elevates_only_the_active_pinned_definition() {
        let policy = Arc::new(
            HostTrustPolicy::new(vec![Box::new(AdminConfig::new())]).expect("valid policy"),
        );
        let publisher = ActiveExtensionPublisher::new(
            Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new())),
            Arc::clone(&policy),
            Arc::new(InvalidationBus::new()),
        );
        let endpoint = crate::hosted_mcp_admission::CanonicalHostedMcpEndpoint::parse(
            &HostedMcpEndpoint::new("https://mcp.linear.app/rpc".to_string())
                .expect("valid endpoint"),
        )
        .expect("canonical endpoint");
        let record = crate::hosted_mcp_manifest::pending_manifest(
            &ExtensionId::new("mcp-linear").expect("valid extension id"),
            "Linear",
            &endpoint,
            &HostedMcpAuthSelection::NoAuth,
        )
        .expect("valid pending hosted MCP manifest");
        let package = crate::hosted_mcp_manifest::available_package(&record)
            .expect("available user-registered package")
            .package;
        let input = extension_trust_policy_input(&package).expect("trust input");

        let before = policy.evaluate(&input).expect("policy evaluates");
        assert_eq!(before.effective_trust.class(), TrustClass::Sandbox);
        assert_eq!(before.provenance, TrustProvenance::Default);

        publisher
            .publish(&package)
            .expect("activation publishes package");
        let active = policy.evaluate(&input).expect("policy evaluates");
        assert_eq!(active.effective_trust.class(), TrustClass::UserTrusted);
        assert_eq!(active.provenance, TrustProvenance::AdminConfig);

        publisher
            .unpublish(&package)
            .expect("deactivation removes trust");
        let removed = policy.evaluate(&input).expect("policy evaluates");
        assert_eq!(removed.effective_trust.class(), TrustClass::Sandbox);
        assert_eq!(removed.provenance, TrustProvenance::Default);
    }
}
