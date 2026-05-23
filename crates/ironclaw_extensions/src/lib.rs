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
    TrustClass, VirtualPath,
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
}

impl ExtensionManifest {
    pub fn parse(
        input: &str,
        source: ManifestSource,
        host_port_catalog: &HostPortCatalog,
    ) -> Result<Self, ExtensionError> {
        ExtensionManifestV2::parse(input, source, host_port_catalog)?.try_into()
    }

    pub fn parse_with_host_api_contracts(
        input: &str,
        source: ManifestSource,
        host_port_catalog: &HostPortCatalog,
        registry: &HostApiContractRegistry,
    ) -> Result<Self, ExtensionError> {
        ExtensionManifestV2::parse_with_host_api_contracts(
            input,
            source,
            host_port_catalog,
            registry,
        )?
        .try_into()
    }

    pub fn parse_with_optional_host_api_contracts(
        input: &str,
        source: ManifestSource,
        host_port_catalog: &HostPortCatalog,
        registry: &HostApiContractRegistry,
    ) -> Result<Self, ExtensionError> {
        ExtensionManifestV2::parse_with_optional_host_api_contracts(
            input,
            source,
            host_port_catalog,
            registry,
        )?
        .try_into()
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
}

impl ExtensionPackage {
    pub fn from_manifest(
        manifest: ExtensionManifest,
        root: VirtualPath,
    ) -> Result<Self, ExtensionError> {
        ensure_extension_root_matches(&manifest.id, &root)?;
        let expected_prefix = format!("{}.", manifest.id.as_str());
        let mut seen_capabilities = HashSet::new();
        let capabilities = manifest
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
                    resource_profile: capability.resource_profile.clone(),
                })
            })
            .collect::<Result<Vec<_>, ExtensionError>>()?;

        Ok(Self {
            id: manifest.id.clone(),
            root,
            manifest,
            capabilities,
        })
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

pub mod host_api;
mod installations;
mod lifecycle;
mod registry;
pub mod v2;

pub use host_api::capability_provider::{
    CAPABILITY_PROVIDER_HOST_API_ID, CAPABILITY_PROVIDER_SECTION, CapabilityProviderHostApiContract,
};
pub use v2::{
    CapabilityDeclV2, CapabilityVisibility, ExtensionManifestV2, ExtensionRuntimeV2,
    HostApiContractRegistry, HostApiId, HostApiManifestContext, HostApiManifestContract,
    HostApiManifestProjection, HostApiMultiplicity, HostApiRefV2, MANIFEST_SCHEMA_VERSION,
    MAX_MANIFEST_BYTES, ManifestSectionPath, ManifestSource, ManifestV2Error,
    RESERVED_HOST_BUNDLED_ID_PREFIX,
};

pub type CapabilityManifest = CapabilityDeclV2;

pub use installations::{
    ExtensionActivationState, ExtensionCredentialBinding, ExtensionCredentialHandle,
    ExtensionHealthMessage, ExtensionHealthSnapshot, ExtensionHealthStatus, ExtensionInstallation,
    ExtensionInstallationError, ExtensionInstallationId, ExtensionInstallationStore,
    ExtensionManifestRecord, ExtensionManifestRef, InMemoryExtensionInstallationStore,
    ManifestHash,
};
pub use lifecycle::{
    ExtensionLifecycleEvent, ExtensionLifecycleEventSink, ExtensionLifecycleService,
};
pub use registry::ExtensionRegistry;

/// Filesystem-backed extension discovery.
pub struct ExtensionDiscovery;

impl ExtensionDiscovery {
    pub async fn discover<F>(
        fs: &F,
        root: &VirtualPath,
    ) -> Result<ExtensionRegistry, ExtensionError>
    where
        F: RootFilesystem,
    {
        let host_port_catalog = HostPortCatalog::empty();
        let host_api_contracts = HostApiContractRegistry::new();
        Self::discover_with_manifest_contracts(
            fs,
            root,
            ManifestSource::InstalledLocal,
            &host_port_catalog,
            &host_api_contracts,
        )
        .await
    }

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
            if entry.file_type != FileType::Directory {
                continue;
            }
            let Ok(expected) = ExtensionId::new(entry.name.clone()) else {
                continue;
            };
            let manifest_path = VirtualPath::new(format!(
                "{}/{}/manifest.toml",
                root.as_str().trim_end_matches('/'),
                entry.name
            ))?;
            let bytes = fs.read_file(&manifest_path).await?;
            let text = String::from_utf8(bytes).map_err(|error| ExtensionError::ManifestParse {
                reason: error.to_string(),
            })?;
            let manifest = ExtensionManifest::parse_with_optional_host_api_contracts(
                &text,
                source,
                host_port_catalog,
                host_api_contracts,
            )?;
            if manifest.id != expected {
                return Err(ExtensionError::ManifestIdMismatch {
                    root: entry.path,
                    expected,
                    actual: manifest.id,
                });
            }
            let package = ExtensionPackage::from_manifest(manifest, entry.path)?;
            registry.insert(package)?;
        }

        Ok(registry)
    }
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
