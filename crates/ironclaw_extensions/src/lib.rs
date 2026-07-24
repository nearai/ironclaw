//! Extension manifest and registry contracts for IronClaw Reborn.
//!
//! `ironclaw_extensions` discovers and validates extension packages, extracts
//! capability descriptors, and records declarative runtime metadata. It does not
//! execute WASM modules, start Docker containers, connect to MCP servers, resolve
//! secrets, or reserve resources.

use ironclaw_filesystem::{FileType, FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, ExtensionId, ExtensionLifecycleOperation, HostApiError,
    HostPortCatalog, PackageId, PackageIdentity, PackageSource, RequestedTrustClass, RuntimeKind,
    TrustClass, VirtualPath, sha256_digest_token,
};
use ironclaw_trust::TrustPolicyInput;
use std::collections::{BTreeSet, HashSet};
use thiserror::Error;

/// Extension manifest and registry failures.
#[derive(Debug, Error)]
pub enum ExtensionError {
    #[error(transparent)]
    Contract(#[from] HostApiError),
    #[error("failed to parse extension manifest: {reason}")]
    ManifestParse { reason: String },
    #[error("invalid extension manifest: {reason}")]
    InvalidManifest { reason: String },
    #[error("invalid extension asset path '{path}': {reason}")]
    InvalidAssetPath { path: String, reason: String },
    #[error("extension manifest id mismatch at {root:?}: expected {expected}, actual {actual}")]
    ManifestIdMismatch {
        root: VirtualPath,
        expected: ExtensionId,
        actual: ExtensionId,
    },
    #[error("duplicate extension id {id}")]
    DuplicateExtension { id: ExtensionId },
    #[error("extension id {id} was not found")]
    ExtensionNotFound { id: ExtensionId },
    #[error("duplicate capability id {id}")]
    DuplicateCapability { id: CapabilityId },
    #[error("extension lifecycle event sink failed during {operation} for {extension_id}")]
    LifecycleEventSink {
        extension_id: ExtensionId,
        operation: ExtensionLifecycleOperation,
    },
    #[error(transparent)]
    ManifestV2(#[from] v2::ManifestV2Error),
    #[error(transparent)]
    Filesystem(#[from] FilesystemError),
}

/// Manifest-local path for assets such as WASM modules.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExtensionAssetPath(String);

impl ExtensionAssetPath {
    pub fn new(value: impl Into<String>) -> Result<Self, ExtensionError> {
        let value = value.into();
        validate_asset_path(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn resolve_under(&self, root: &VirtualPath) -> Result<VirtualPath, ExtensionError> {
        VirtualPath::new(format!(
            "{}/{}",
            root.as_str().trim_end_matches('/'),
            self.0
        ))
        .map_err(ExtensionError::from)
    }
}

/// Declarative runtime metadata for an extension package after boundary
/// validation has converted manifest strings into typed internal values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionRuntime {
    Wasm {
        module: ExtensionAssetPath,
    },
    Script {
        runner: String,
        image: Option<String>,
        command: String,
        args: Vec<String>,
    },
    Mcp {
        transport: String,
        command: Option<String>,
        args: Vec<String>,
        url: Option<String>,
    },
    FirstParty {
        service: String,
    },
    System {
        service: String,
    },
}

impl ExtensionRuntime {
    pub fn kind(&self) -> RuntimeKind {
        match self {
            Self::Wasm { .. } => RuntimeKind::Wasm,
            Self::Script { .. } => RuntimeKind::Script,
            Self::Mcp { .. } => RuntimeKind::Mcp,
            Self::FirstParty { .. } => RuntimeKind::FirstParty,
            Self::System { .. } => RuntimeKind::System,
        }
    }

    fn from_v2(runtime: ExtensionRuntimeV2) -> Result<Self, ExtensionError> {
        match runtime {
            ExtensionRuntimeV2::Wasm { module } => Ok(Self::Wasm {
                module: ExtensionAssetPath::new(module)?,
            }),
            ExtensionRuntimeV2::Script {
                runner,
                image,
                command,
                args,
            } => Ok(Self::Script {
                runner,
                image,
                command,
                args,
            }),
            ExtensionRuntimeV2::Mcp {
                transport,
                command,
                args,
                url,
            } => Ok(Self::Mcp {
                transport,
                command,
                args,
                url,
            }),
            ExtensionRuntimeV2::FirstParty { service } => Ok(Self::FirstParty { service }),
            ExtensionRuntimeV2::System { service } => Ok(Self::System { service }),
        }
    }
}

/// Validated production extension manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionManifest {
    pub schema_version: String,
    pub id: ExtensionId,
    pub name: String,
    pub version: String,
    pub description: String,
    pub source: ManifestSource,
    pub requested_trust: RequestedTrustClass,
    pub descriptor_trust_default: TrustClass,
    pub runtime: ExtensionRuntime,
    pub host_apis: Vec<HostApiRefV2>,
    pub capabilities: Vec<CapabilityManifest>,
    /// Surfaces projected by host API contract sections (channel and future
    /// section-declared kinds); tool and auth surfaces derive from
    /// capability declarations on demand — see
    /// [`Self::capability_surfaces`].
    pub host_api_surfaces: Vec<CapabilitySurfaceDeclV2>,
    /// Declarative hook entries the extension declared. Structurally
    /// validated by the v2 parser; projected into typed hook entries by the
    /// composition loader. Empty for the common no-hooks case.
    pub hooks: Vec<HookSectionEntryV2>,
}

impl ExtensionManifest {
    /// Derived, order-stable projection of every product-facing surface this
    /// manifest declares. See [`ExtensionManifestV2::capability_surfaces`]
    /// for the derivation rules; this mirror carries the identical data.
    pub fn capability_surfaces(&self) -> Vec<CapabilitySurfaceDeclV2> {
        v2::capability_surfaces_from_parts(&self.capabilities, &self.host_api_surfaces)
    }

    pub fn parse(
        input: &str,
        source: ManifestSource,
        host_port_catalog: &HostPortCatalog,
        registry: &HostApiContractRegistry,
    ) -> Result<Self, ExtensionError> {
        ExtensionManifestV2::parse(input, source, host_port_catalog, registry)?.try_into()
    }

    pub fn runtime_kind(&self) -> RuntimeKind {
        self.runtime.kind()
    }
}

impl TryFrom<ExtensionManifestV2> for ExtensionManifest {
    type Error = ExtensionError;

    fn try_from(manifest: ExtensionManifestV2) -> Result<Self, Self::Error> {
        Ok(Self {
            schema_version: manifest.schema_version,
            id: manifest.id,
            name: manifest.name,
            version: manifest.version,
            description: manifest.description,
            source: manifest.source,
            requested_trust: manifest.requested_trust,
            descriptor_trust_default: manifest.descriptor_trust_default,
            runtime: ExtensionRuntime::from_v2(manifest.runtime)?,
            host_apis: manifest.host_apis,
            capabilities: manifest.capabilities,
            host_api_surfaces: manifest.host_api_surfaces,
            hooks: manifest.hooks,
        })
    }
}

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
        registry::validate_package_consistency(self)?;
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

mod admin_configuration;
mod canonicalization;
pub mod host_api;
mod hosted_mcp_discovery;
mod installations;
mod lifecycle;
mod registry;
pub mod resolved;
pub mod v2;
pub mod v3;

pub use admin_configuration::{
    AdminConfigurationDescriptorError, AdminConfigurationField, AdminConfigurationGroupId,
    ExtensionAdminConfigurationDescriptor,
};
pub use host_api::capability_provider::{
    CAPABILITY_PROVIDER_HOST_API_ID, CAPABILITY_PROVIDER_SECTION, CapabilityProviderHostApiContract,
};
pub use hosted_mcp_discovery::{
    HostedMcpDiscoveredTool, HostedMcpDiscoveredToolAnnotations, is_hosted_http_mcp_package,
    package_with_discovered_hosted_mcp_tools,
};
pub use resolved::{
    ResolvedAuthSurface, ResolvedExtensionManifest, ResolvedHostApiRef, ResolvedMcpDeclaration,
    ResolvedSectionSurface,
};
pub use v2::{
    CapabilityDeclV2, CapabilitySurfaceDeclV2, CapabilityVisibility, ExtensionManifestV2,
    ExtensionRuntimeV2, HookSectionEntryV2, HostApiContractRegistry, HostApiId,
    HostApiManifestContext, HostApiManifestContract, HostApiManifestProjection,
    HostApiMultiplicity, HostApiRefV2, HostApiSectionError, MANIFEST_SCHEMA_VERSION,
    MAX_HOOK_ENTRY_BYTES, MAX_MANIFEST_BYTES, MAX_MANIFEST_HOOKS, ManifestSectionPath,
    ManifestSource, ManifestV2Error, RESERVED_HOST_BUNDLED_ID_PREFIX,
};
pub use v3::{MANIFEST_SCHEMA_VERSION_V3, ManifestV3Error};

pub type CapabilityManifest = CapabilityDeclV2;

pub use canonicalization::canonicalize_installation_rows;
pub use installations::{
    ExtensionCredentialBinding, ExtensionCredentialHandle, ExtensionHealthMessage,
    ExtensionHealthSnapshot, ExtensionHealthStatus, ExtensionInstallation,
    ExtensionInstallationError, ExtensionInstallationId, ExtensionInstallationPersistedParts,
    ExtensionInstallationStore, ExtensionManifestRecord, ExtensionManifestRef,
    ExtensionRemovalChannelId, ExtensionRemovalCleanupAdapterId, ExtensionRemovalCleanupBinding,
    ExtensionRemovalCleanupRequirement, FilesystemExtensionInstallationStore, InstallationOwner,
    ManifestHash,
};
pub use lifecycle::{
    ExtensionLifecycleEvent, ExtensionLifecycleEventSink, ExtensionLifecycleService,
};
pub use registry::{ExtensionRegistry, SharedExtensionRegistry};

/// Filesystem-backed extension discovery.
pub struct ExtensionDiscovery;

impl ExtensionDiscovery {
    pub async fn discover_with_manifest_contracts<F>(
        fs: &F,
        root: &VirtualPath,
        source: ManifestSource,
        host_port_catalog: &HostPortCatalog,
        host_api_contracts: &HostApiContractRegistry,
    ) -> Result<ExtensionRegistry, ExtensionError>
    where
        F: RootFilesystem,
    {
        let mut entries = fs.list_dir(root).await?;
        entries.sort_by(|left, right| left.name.cmp(&right.name));

        let mut registry = ExtensionRegistry::new();
        for entry in entries {
            let Some(expected) = Self::extension_dir_id(&entry) else {
                continue;
            };
            // All-or-nothing: any per-package failure fails the whole discovery.
            let package = Self::load_package_entry(
                fs,
                root,
                &entry,
                expected,
                source,
                host_port_catalog,
                host_api_contracts,
            )
            .await?;
            registry.insert(package)?;
        }

        Ok(registry)
    }

    /// Tolerant + **bounded** discovery (DoS-hardened entry point).
    ///
    /// Two security properties separate this from
    /// [`Self::discover_with_manifest_contracts`]:
    ///
    /// 1. **Bounded** — caps the expensive per-manifest read/parse/validate work
    ///    to at most `max_extensions` extension directories. The directory is
    ///    listed and sorted once (cheap), then only the FIRST `max_extensions`
    ///    valid extension directory entries are read; the remainder are recorded
    ///    as [`DiscoveryQuarantine`]s WITHOUT ever being read or parsed. A tenant
    ///    with thousands of extension directories therefore cannot force
    ///    unbounded read/parse work — the count cap fires *before* the read
    ///    storm, not after (the per-file `MAX_MANIFEST_BYTES` pre-read bound is
    ///    orthogonal and still applies to every read we do perform).
    /// 2. **Tolerant** — a single malformed / oversized / id-mismatched package
    ///    quarantines ONLY that package (collected into
    ///    [`TolerantBoundedDiscovery::quarantined`]) and discovery CONTINUES. The
    ///    only error that aborts the whole call is failure to LIST THE ROOT
    ///    itself (the directory is unreadable) — surfaced as the outer `Err`.
    ///
    /// `max_extensions` counts *valid extension directory entries considered*
    /// (post sort, post name-validation), so the surplus tail is quarantined
    /// deterministically by sorted name. A `max_extensions` of `0` reads nothing.
    pub async fn discover_with_manifest_contracts_tolerant_bounded<F>(
        fs: &F,
        root: &VirtualPath,
        source: ManifestSource,
        host_port_catalog: &HostPortCatalog,
        host_api_contracts: &HostApiContractRegistry,
        max_extensions: usize,
    ) -> Result<TolerantBoundedDiscovery, ExtensionError>
    where
        F: RootFilesystem,
    {
        // Listing the root is the ONLY fatal step: if the tenant's extension
        // directory is unreadable we cannot make a per-package decision, so the
        // caller falls back (e.g. to builtin-only). A FilesystemError here
        // propagates as the outer Err.
        let mut entries = fs.list_dir(root).await?;
        entries.sort_by(|left, right| left.name.cmp(&right.name));

        let mut registry = ExtensionRegistry::new();
        let mut quarantined: Vec<DiscoveryQuarantine> = Vec::new();
        let mut considered = 0usize;

        for entry in entries {
            let Some(expected) = Self::extension_dir_id(&entry) else {
                // Not an extension directory (file, or non-id name): skip
                // silently, do not count against the bound.
                continue;
            };

            // ── Bound BEFORE the expensive read/parse. ──
            // Once the count cap is hit, record the surplus as quarantined
            // without reading its manifest at all. This is the DoS ceiling.
            if considered >= max_extensions {
                quarantined.push(DiscoveryQuarantine {
                    extension_id: expected.as_str().to_string(),
                    reason: format!(
                        "exceeded discovery bound of {max_extensions} extension(s); \
                         not read"
                    ),
                });
                continue;
            }
            considered += 1;

            match Self::load_package_entry(
                fs,
                root,
                &entry,
                expected.clone(),
                source,
                host_port_catalog,
                host_api_contracts,
            )
            .await
            {
                Ok(package) => {
                    if let Err(error) = registry.insert(package) {
                        quarantined.push(DiscoveryQuarantine {
                            extension_id: expected.as_str().to_string(),
                            reason: error.to_string(),
                        });
                    }
                }
                Err(error) => {
                    // Tolerant: one bad package drops only itself.
                    quarantined.push(DiscoveryQuarantine {
                        extension_id: expected.as_str().to_string(),
                        reason: error.to_string(),
                    });
                }
            }
        }

        Ok(TolerantBoundedDiscovery {
            registry,
            quarantined,
        })
    }

    /// Map a directory entry to its expected [`ExtensionId`], or `None` if the
    /// entry is not a usable extension directory (not a directory, or a name
    /// that is not a valid extension id). Cheap: no filesystem read.
    fn extension_dir_id(entry: &ironclaw_filesystem::DirEntry) -> Option<ExtensionId> {
        if entry.file_type != FileType::Directory {
            return None;
        }
        ExtensionId::new(entry.name.clone()).ok()
    }

    /// Read + parse + validate a single extension directory entry into an
    /// [`ExtensionPackage`]. Shared by the all-or-nothing and tolerant+bounded
    /// discovery paths so the per-package semantics are identical; only the
    /// caller's handling of the `Err` differs (propagate vs quarantine).
    async fn load_package_entry<F>(
        fs: &F,
        root: &VirtualPath,
        entry: &ironclaw_filesystem::DirEntry,
        expected: ExtensionId,
        source: ManifestSource,
        host_port_catalog: &HostPortCatalog,
        host_api_contracts: &HostApiContractRegistry,
    ) -> Result<ExtensionPackage, ExtensionError>
    where
        F: RootFilesystem,
    {
        let manifest_path = VirtualPath::new(format!(
            "{}/{}/manifest.toml",
            root.as_str().trim_end_matches('/'),
            entry.name
        ))?;
        // DoS pre-read bound (threat-model: oversized manifest). Stat the
        // file and refuse to read it at all if it exceeds the manifest size
        // ceiling, rather than materializing the whole body first and only
        // then rejecting in `parse` (`MAX_MANIFEST_BYTES` is also re-checked
        // there as defense-in-depth). `read_file_bounded` stats before it
        // materializes, so an attacker-controlled multi-gigabyte manifest is
        // rejected without a full read.
        let bytes = match fs
            .read_file_bounded(&manifest_path, v2::MAX_MANIFEST_BYTES)
            .await?
        {
            Some(bytes) => bytes,
            None => {
                return Err(ExtensionError::InvalidManifest {
                    reason: format!(
                        "extension manifest at {} exceeds the {}-byte ceiling and was \
                         rejected before reading",
                        manifest_path.as_str(),
                        v2::MAX_MANIFEST_BYTES
                    ),
                });
            }
        };
        let text = String::from_utf8(bytes).map_err(|error| ExtensionError::ManifestParse {
            reason: error.to_string(),
        })?;
        let manifest =
            ExtensionManifest::parse(&text, source, host_port_catalog, host_api_contracts)?;
        if manifest.id != expected {
            return Err(ExtensionError::ManifestIdMismatch {
                root: entry.path.clone(),
                expected,
                actual: manifest.id,
            });
        }
        ExtensionPackage::from_manifest_toml(manifest, entry.path.clone(), &text)
    }
}

/// A package dropped during tolerant discovery, with a human-readable reason.
/// The caller (the hook projection) turns each into a `hook.quarantined` audit
/// event. Carries the extension id (directory name) so the audit names the
/// offending package even when the manifest failed to parse.
#[derive(Debug, Clone)]
pub struct DiscoveryQuarantine {
    pub extension_id: String,
    pub reason: String,
}

/// Result of [`ExtensionDiscovery::discover_with_manifest_contracts_tolerant_bounded`]:
/// the registry of packages that loaded successfully within the bound, plus the
/// per-package quarantine record for every package that was dropped (malformed,
/// duplicate, or beyond the discovery bound).
#[derive(Debug)]
pub struct TolerantBoundedDiscovery {
    pub registry: ExtensionRegistry,
    pub quarantined: Vec<DiscoveryQuarantine>,
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
    let mut seen_capabilities = HashSet::new();
    manifest
        .capabilities
        .iter()
        .map(|capability| {
            if !capability.id.as_str().starts_with(&expected_prefix) {
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

fn validate_asset_path(value: &str) -> Result<(), ExtensionError> {
    if value.is_empty() {
        return Err(ExtensionError::InvalidAssetPath {
            path: value.to_string(),
            reason: "asset path must not be empty".to_string(),
        });
    }
    if value.contains(' ') || value.chars().any(char::is_control) {
        return Err(ExtensionError::InvalidAssetPath {
            path: value.to_string(),
            reason: "NUL/control characters are not allowed".to_string(),
        });
    }
    if value.contains("://") {
        return Err(ExtensionError::InvalidAssetPath {
            path: value.to_string(),
            reason: "URLs are not extension asset paths".to_string(),
        });
    }
    if value.starts_with('/') {
        return Err(ExtensionError::InvalidAssetPath {
            path: value.to_string(),
            reason: "asset path must be relative".to_string(),
        });
    }
    if looks_like_windows_path(value) || value.contains('\\') {
        return Err(ExtensionError::InvalidAssetPath {
            path: value.to_string(),
            reason: "host path separators are not allowed".to_string(),
        });
    }
    for segment in value.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(ExtensionError::InvalidAssetPath {
                path: value.to_string(),
                reason: "empty or dot path segments are not allowed".to_string(),
            });
        }
    }
    Ok(())
}

fn looks_like_windows_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        || (bytes.len() >= 3 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/'))
}
