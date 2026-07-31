use ironclaw_host_api::{
    approval::sha256_digest_token,
    capability::CapabilityDescriptor,
    ids::{ExtensionId, PackageId},
    path::VirtualPath,
    trust::{PackageIdentity, PackageSource},
};
use ironclaw_trust::TrustPolicyInput;
use std::collections::{BTreeSet, HashSet};

use crate::{CapabilityManifest, ExtensionError, ExtensionManifest, ManifestSource};

/// Validated package rooted under `/system/extensions/<extension>`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionPackage {
    pub id: ExtensionId,
    pub root: VirtualPath,
    pub manifest: ExtensionManifest,
    pub capabilities: Vec<CapabilityDescriptor>,
    pub manifest_digest: Option<String>,
    pub descriptor_schema_mode: CapabilityDescriptorSchemaMode,
}

/// How package capability descriptor schemas are derived from the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityDescriptorSchemaMode {
    /// Descriptors must carry the manifest's `$ref` schema projection.
    ManifestRefs,
    /// Descriptors may carry inline schemas, but all non-schema fields must
    /// still match the manifest projection exactly.
    InlineDynamic,
}

impl ExtensionPackage {
    pub fn from_manifest(
        manifest: ExtensionManifest,
        root: VirtualPath,
    ) -> Result<Self, ExtensionError> {
        Self::from_manifest_with_digest(manifest, root, None)
    }

    pub fn from_manifest_toml(
        manifest: ExtensionManifest,
        root: VirtualPath,
        manifest_toml: &str,
    ) -> Result<Self, ExtensionError> {
        Self::from_manifest_with_digest(
            manifest,
            root,
            Some(sha256_digest_token(manifest_toml.as_bytes())),
        )
    }

    pub fn from_manifest_with_digest(
        manifest: ExtensionManifest,
        root: VirtualPath,
        manifest_digest: Option<String>,
    ) -> Result<Self, ExtensionError> {
        ensure_extension_root_matches(&manifest.id, &root)?;
        let capabilities = capability_descriptors_from_manifest(&manifest)?;

        Ok(Self {
            id: manifest.id.clone(),
            root,
            manifest,
            capabilities,
            manifest_digest,
            descriptor_schema_mode: CapabilityDescriptorSchemaMode::ManifestRefs,
        })
    }

    pub fn from_host_bundled_manifest_with_inline_dynamic_schemas(
        manifest: ExtensionManifest,
        root: VirtualPath,
        manifest_digest: Option<String>,
        capabilities: Vec<CapabilityDescriptor>,
    ) -> Result<Self, ExtensionError> {
        if manifest.source != ManifestSource::HostBundled {
            return Err(ExtensionError::InvalidManifest {
                reason:
                    "inline dynamic descriptor schemas are only supported for host-bundled packages"
                        .to_string(),
            });
        }
        ensure_extension_root_matches(&manifest.id, &root)?;
        let expected = capability_descriptors_from_manifest(&manifest)?;
        if !descriptors_match_except_schema(&capabilities, &expected) {
            return Err(ExtensionError::InvalidManifest {
                reason: "inline dynamic capability descriptors do not match manifest declarations"
                    .to_string(),
            });
        }
        Ok(Self {
            id: manifest.id.clone(),
            root,
            manifest,
            capabilities,
            manifest_digest,
            descriptor_schema_mode: CapabilityDescriptorSchemaMode::InlineDynamic,
        })
    }

    pub fn manifest_digest(&self) -> Option<String> {
        self.manifest_digest.clone()
    }

    pub(crate) fn validate_consistency(&self) -> Result<(), ExtensionError> {
        if self.id != self.manifest.id {
            return Err(ExtensionError::InvalidManifest {
                reason: format!(
                    "package id {} does not match manifest id {}",
                    self.id, self.manifest.id
                ),
            });
        }
        ensure_extension_root_matches(&self.manifest.id, &self.root)?;
        let expected = capability_descriptors_from_manifest(&self.manifest)?;
        let consistent = match self.descriptor_schema_mode {
            CapabilityDescriptorSchemaMode::ManifestRefs => self.capabilities == expected,
            CapabilityDescriptorSchemaMode::InlineDynamic => {
                self.manifest.source == ManifestSource::HostBundled
                    && descriptors_match_except_schema(&self.capabilities, &expected)
            }
        };
        if !consistent {
            return Err(ExtensionError::InvalidManifest {
                reason: "package capability descriptors do not match manifest declarations"
                    .to_string(),
            });
        }
        Ok(())
    }

    /// Build the trust-policy identity for this package.
    ///
    /// `PackageId` and `ExtensionId` share the same underlying vocabulary in
    /// V1; the conversion still goes through the validated constructor so this
    /// crate does not rely on representation details.
    pub fn package_identity(
        &self,
        source: PackageSource,
        digest: Option<String>,
        signer: Option<String>,
    ) -> Result<PackageIdentity, ExtensionError> {
        crate::registry::validate_package_consistency(self)?;
        Ok(PackageIdentity::new(
            PackageId::new(self.manifest.id.as_str().to_string())?,
            source,
            digest,
            signer,
        ))
    }

    /// Build the trust-policy input for this package.
    ///
    /// Requested authority is the canonical set of capability ids declared by
    /// the package. The returned value is still untrusted input; callers must
    /// pass it to `ironclaw_trust::TrustPolicy::evaluate` to get an effective
    /// [`ironclaw_trust::TrustDecision`].
    pub fn trust_policy_input(
        &self,
        source: PackageSource,
        digest: Option<String>,
        signer: Option<String>,
    ) -> Result<TrustPolicyInput, ExtensionError> {
        Ok(TrustPolicyInput {
            identity: self.package_identity(source, digest, signer)?,
            requested_trust: self.manifest.requested_trust,
            requested_authority: self
                .capabilities
                .iter()
                .map(|descriptor| descriptor.id.clone())
                .collect::<BTreeSet<_>>(),
        })
    }
}

fn descriptors_match_except_schema(
    actual: &[CapabilityDescriptor],
    expected: &[CapabilityDescriptor],
) -> bool {
    actual.len() == expected.len()
        && actual.iter().zip(expected).all(|(actual, expected)| {
            let mut normalized = actual.clone();
            normalized.parameters_schema = expected.parameters_schema.clone();
            normalized == *expected
        })
}

fn ensure_extension_root_matches(
    id: &ExtensionId,
    root: &VirtualPath,
) -> Result<(), ExtensionError> {
    let expected = extension_id_from_package_root(root)?;
    if &expected != id {
        return Err(ExtensionError::ManifestIdMismatch {
            root: root.clone(),
            expected,
            actual: id.clone(),
        });
    }
    Ok(())
}

fn extension_id_from_package_root(root: &VirtualPath) -> Result<ExtensionId, ExtensionError> {
    let Some(extension_id) = root.as_str().strip_prefix("/system/extensions/") else {
        return Err(invalid_package_root(root));
    };
    if extension_id.is_empty() || extension_id.contains('/') {
        return Err(invalid_package_root(root));
    }
    Ok(ExtensionId::new(extension_id.to_string())?)
}

fn capability_descriptors_from_manifest(
    manifest: &ExtensionManifest,
) -> Result<Vec<CapabilityDescriptor>, ExtensionError> {
    let expected_prefix = format!("{}.", manifest.id.as_str());
    // Descriptor-layer mirror of the parse-time provider-prefix rule. The one
    // extra namespace: a HOST-BUNDLED manifest may declare tools under the
    // reserved stable memory-tool namespace (`ironclaw.memory.*`), so a
    // swapped memory backend keeps the stable tool ids. The primary
    // enforcement is the v3 parser (`[memory]` requires a first_party runtime,
    // which requires a host-bundled source); this check keeps the namespace
    // closed to every non-host-bundled package as defense in depth.
    let reserved_memory_prefix = format!(
        "{}.",
        ironclaw_extension_contracts::memory::MEMORY_TOOL_ID_NAMESPACE
    );
    let mut seen_capabilities = HashSet::new();
    manifest
        .capabilities
        .iter()
        .map(|capability| {
            let in_reserved_memory_namespace = manifest.source == ManifestSource::HostBundled
                && capability.id.as_str().starts_with(&reserved_memory_prefix);
            if !capability.id.as_str().starts_with(&expected_prefix)
                && !in_reserved_memory_namespace
            {
                return Err(ExtensionError::InvalidManifest {
                    reason: format!(
                        "capability id {} must be provider-prefixed with {}",
                        capability.id.as_str(),
                        expected_prefix
                    ),
                });
            }
            if !seen_capabilities.insert(capability.id.clone()) {
                return Err(ExtensionError::DuplicateCapability {
                    id: capability.id.clone(),
                });
            }
            Ok(CapabilityDescriptor {
                id: capability.id.clone(),
                provider: manifest.id.clone(),
                runtime: manifest.runtime_kind(),
                trust_ceiling: manifest.descriptor_trust_default,
                description: capability.description.clone(),
                parameters_schema: descriptor_schema_ref(capability),
                effects: capability.effects.clone(),
                default_permission: capability.default_permission,
                runtime_credentials: capability.runtime_credentials.clone(),
                network_targets: capability.network_targets.clone(),
                max_egress_bytes: capability.max_egress_bytes,
                resource_profile: capability.resource_profile.clone(),
                origin_gate_matrix: capability.origin_gate_matrix.clone(),
            })
        })
        .collect()
}

fn invalid_package_root(root: &VirtualPath) -> ExtensionError {
    ExtensionError::InvalidManifest {
        reason: format!(
            "extension package root {} must be /system/extensions/<extension>",
            root.as_str()
        ),
    }
}

fn descriptor_schema_ref(capability: &CapabilityManifest) -> serde_json::Value {
    serde_json::json!({ "$ref": capability.input_schema_ref.as_str() })
}
