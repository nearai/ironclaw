use ironclaw_host_api::{
    CapabilityDescriptor, ExtensionId, PackageId, PackageIdentity, PackageSource, VirtualPath,
    sha256_digest_token,
};
use ironclaw_trust::TrustPolicyInput;
use std::collections::{BTreeSet, HashSet};

use crate::{
    CapabilityManifest, ExtensionError, ExtensionManifest, ExtensionRuntime, ManifestSource,
    PackageRootBinding, PackageRootError,
};

/// Validated package rooted under `/system/extensions/<extension>`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionPackage {
    pub id: ExtensionId,
    pub root_binding: PackageRootBinding,
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
        Self::from_manifest_with_binding(
            manifest,
            PackageRootBinding::Materialized(root),
            manifest_digest,
        )
    }

    /// Construct a remote-only user-registered package. Such a package has no
    /// filesystem tree and may never be routed through asset or schema I/O.
    pub fn from_virtual_manifest(
        manifest: ExtensionManifest,
        manifest_digest: Option<String>,
        capabilities: Vec<CapabilityDescriptor>,
    ) -> Result<Self, ExtensionError> {
        ensure_virtual_manifest(&manifest)?;
        let expected = capability_descriptors_from_manifest(&manifest)?;
        if !descriptors_match_except_schema(&capabilities, &expected) {
            return Err(ExtensionError::InvalidManifest {
                reason: "inline virtual capability descriptors do not match manifest declarations"
                    .to_string(),
            });
        }
        Ok(Self {
            id: manifest.id.clone(),
            root_binding: PackageRootBinding::Virtual,
            manifest,
            capabilities,
            manifest_digest,
            descriptor_schema_mode: CapabilityDescriptorSchemaMode::InlineDynamic,
        })
    }

    fn from_manifest_with_binding(
        manifest: ExtensionManifest,
        root_binding: PackageRootBinding,
        manifest_digest: Option<String>,
    ) -> Result<Self, ExtensionError> {
        let capabilities = capability_descriptors_from_manifest(&manifest)?;

        Ok(Self {
            id: manifest.id.clone(),
            root_binding,
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
            root_binding: PackageRootBinding::Materialized(root),
            manifest,
            capabilities,
            manifest_digest,
            descriptor_schema_mode: CapabilityDescriptorSchemaMode::InlineDynamic,
        })
    }

    pub fn manifest_digest(&self) -> Option<String> {
        self.manifest_digest.clone()
    }

    pub fn package_root_binding(&self) -> &PackageRootBinding {
        &self.root_binding
    }

    pub fn materialized_root(&self) -> Result<&VirtualPath, PackageRootError> {
        self.root_binding.materialized_root()
    }

    /// Return the immutable package source used for trust-policy evaluation.
    ///
    /// A materialized package is identified by its manifest path. A virtual
    /// package has no filesystem identity, so it is eligible only when it is
    /// the constrained direct HTTP MCP shape emitted by hosted-MCP discovery.
    /// `FabricateOnLoad` is a legacy loading sentinel, never an identity a
    /// trust policy may evaluate.
    pub fn trust_policy_source(&self) -> Result<PackageSource, ExtensionError> {
        match &self.root_binding {
            PackageRootBinding::Materialized(root) => Ok(PackageSource::LocalManifest {
                path: format!("{}/manifest.toml", root.as_str().trim_end_matches('/')),
            }),
            PackageRootBinding::Virtual => match &self.manifest.runtime {
                ExtensionRuntime::Mcp {
                    transport,
                    command: None,
                    args,
                    url: Some(endpoint),
                } if transport == "http" && args.is_empty() => Ok(PackageSource::DirectRemote {
                    endpoint: endpoint.clone(),
                }),
                _ => Err(ExtensionError::InvalidManifest {
                    reason: "virtual package is not a direct HTTP MCP endpoint".to_string(),
                }),
            },
            PackageRootBinding::FabricateOnLoad => Err(ExtensionError::InvalidManifest {
                reason: "package root must be materialized before trust evaluation".to_string(),
            }),
        }
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
        match &self.root_binding {
            PackageRootBinding::Materialized(root) => {
                ensure_extension_root_matches(&self.manifest.id, root)?;
            }
            PackageRootBinding::Virtual => ensure_virtual_manifest(&self.manifest)?,
            PackageRootBinding::FabricateOnLoad => {
                return Err(ExtensionError::InvalidManifest {
                    reason: "package root must be materialized before packaging".to_string(),
                });
            }
        }
        let expected = capability_descriptors_from_manifest(&self.manifest)?;
        let consistent = match self.descriptor_schema_mode {
            CapabilityDescriptorSchemaMode::ManifestRefs => self.capabilities == expected,
            CapabilityDescriptorSchemaMode::InlineDynamic => {
                (self.manifest.source == ManifestSource::HostBundled
                    || matches!(self.root_binding, PackageRootBinding::Virtual))
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

fn ensure_virtual_manifest(manifest: &ExtensionManifest) -> Result<(), ExtensionError> {
    if manifest.source != ManifestSource::UserRegistered {
        return Err(ExtensionError::InvalidManifest {
            reason: "virtual packages require user-registered provenance".to_string(),
        });
    }
    if matches!(manifest.runtime, ExtensionRuntime::Wasm { .. }) {
        return Err(ExtensionError::InvalidManifest {
            reason: "virtual packages cannot declare a filesystem-backed runtime".to_string(),
        });
    }
    if manifest
        .capabilities
        .iter()
        .any(|capability| capability.prompt_doc_ref.is_some())
    {
        return Err(ExtensionError::InvalidManifest {
            reason: "virtual packages cannot declare filesystem-backed prompt documents"
                .to_string(),
        });
    }
    Ok(())
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
    let reserved_memory_prefix = format!("{}.", ironclaw_host_api::MEMORY_TOOL_ID_NAMESPACE);
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

#[cfg(test)]
mod tests {
    use ironclaw_host_api::HostPortCatalog;

    use super::*;
    use crate::{CapabilityProviderHostApiContract, HostApiContractRegistry};

    const VIRTUAL_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "remote-tools"
name = "Remote Tools"
version = "0.1.0"
description = "Remote-only tool provider"
trust = "untrusted"

[runtime]
kind = "script"
runner = "remote"
command = "invoke"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "remote-tools.invoke"
description = "Invoke a remote tool"
effects = ["dispatch_capability"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/remote-tools/invoke.input.v1.json"
"#;

    const DIRECT_REMOTE_VIRTUAL_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "remote-tools"
name = "Remote Tools"
version = "0.1.0"
description = "Remote-only tool provider"
trust = "untrusted"

[runtime]
kind = "mcp"
transport = "http"
url = "https://mcp.example.test/mcp"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "remote-tools.invoke"
description = "Invoke a remote tool"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/remote-tools/invoke.input.v1.json"
"#;

    fn contracts() -> HostApiContractRegistry {
        let mut contracts = HostApiContractRegistry::new();
        contracts
            .register(std::sync::Arc::new(
                CapabilityProviderHostApiContract::new().expect("contract"),
            ))
            .expect("register contract");
        contracts
    }

    #[test]
    fn virtual_package_requires_inline_descriptors_and_never_exposes_a_root() {
        let manifest = ExtensionManifest::parse(
            VIRTUAL_MANIFEST,
            ManifestSource::UserRegistered,
            &HostPortCatalog::empty(),
            &contracts(),
        )
        .expect("manifest");
        let mut capabilities =
            capability_descriptors_from_manifest(&manifest).expect("descriptors");
        capabilities[0].parameters_schema = serde_json::json!({"type": "object"});

        let package = ExtensionPackage::from_virtual_manifest(manifest, None, capabilities)
            .expect("virtual package");
        assert_eq!(package.root_binding, PackageRootBinding::Virtual);
        assert_eq!(package.materialized_root(), Err(PackageRootError::Virtual));
        package.validate_consistency().expect("consistent package");
    }

    #[test]
    fn virtual_package_rejects_non_user_registered_provenance() {
        let manifest = ExtensionManifest::parse(
            VIRTUAL_MANIFEST,
            ManifestSource::InstalledLocal,
            &HostPortCatalog::empty(),
            &contracts(),
        )
        .expect("manifest");
        let capabilities = capability_descriptors_from_manifest(&manifest).expect("descriptors");
        let error = ExtensionPackage::from_virtual_manifest(manifest, None, capabilities)
            .expect_err("local provenance cannot become virtual");
        assert!(matches!(
            error,
            ExtensionError::InvalidManifest { reason }
                if reason.contains("user-registered provenance")
        ));
    }

    #[test]
    fn packaged_fabrication_state_fails_consistency() {
        let manifest = ExtensionManifest::parse(
            VIRTUAL_MANIFEST,
            ManifestSource::InstalledLocal,
            &HostPortCatalog::empty(),
            &contracts(),
        )
        .expect("manifest");
        let root = VirtualPath::new("/system/extensions/remote-tools").expect("root");
        let mut package = ExtensionPackage::from_manifest(manifest, root).expect("package");
        package.root_binding = PackageRootBinding::FabricateOnLoad;
        let error = package
            .validate_consistency()
            .expect_err("fabrication cannot survive packaging");
        assert!(matches!(error, ExtensionError::InvalidManifest { .. }));
    }

    #[test]
    fn trust_policy_source_uses_the_package_root_binding() {
        let materialized_manifest = ExtensionManifest::parse(
            VIRTUAL_MANIFEST,
            ManifestSource::InstalledLocal,
            &HostPortCatalog::empty(),
            &contracts(),
        )
        .expect("manifest");
        let materialized = ExtensionPackage::from_manifest(
            materialized_manifest,
            VirtualPath::new("/system/extensions/remote-tools").expect("root"),
        )
        .expect("package");
        assert_eq!(
            materialized.trust_policy_source().expect("source"),
            PackageSource::LocalManifest {
                path: "/system/extensions/remote-tools/manifest.toml".to_string(),
            }
        );

        let virtual_manifest = ExtensionManifest::parse(
            DIRECT_REMOTE_VIRTUAL_MANIFEST,
            ManifestSource::UserRegistered,
            &HostPortCatalog::empty(),
            &contracts(),
        )
        .expect("manifest");
        let capabilities =
            capability_descriptors_from_manifest(&virtual_manifest).expect("descriptors");
        let virtual_package =
            ExtensionPackage::from_virtual_manifest(virtual_manifest, None, capabilities)
                .expect("virtual package");
        assert_eq!(
            virtual_package.trust_policy_source().expect("source"),
            PackageSource::DirectRemote {
                endpoint: "https://mcp.example.test/mcp".to_string(),
            }
        );

        let non_direct_manifest = ExtensionManifest::parse(
            VIRTUAL_MANIFEST,
            ManifestSource::UserRegistered,
            &HostPortCatalog::empty(),
            &contracts(),
        )
        .expect("manifest");
        let non_direct_capabilities =
            capability_descriptors_from_manifest(&non_direct_manifest).expect("descriptors");
        let non_direct = ExtensionPackage::from_virtual_manifest(
            non_direct_manifest,
            None,
            non_direct_capabilities,
        )
        .expect("virtual package");
        assert!(matches!(
            non_direct.trust_policy_source(),
            Err(ExtensionError::InvalidManifest { .. })
        ));

        let mut fabricated = materialized;
        fabricated.root_binding = PackageRootBinding::FabricateOnLoad;
        assert!(matches!(
            fabricated.trust_policy_source(),
            Err(ExtensionError::InvalidManifest { .. })
        ));
    }
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
