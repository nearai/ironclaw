//! Extension manifest and registry contracts for IronClaw Reborn.
//!
//! `ironclaw_extensions` discovers and validates extension packages, extracts
//! capability descriptors, and records declarative runtime metadata. It does not
//! execute WASM modules, start Docker containers, connect to MCP servers, resolve
//! secrets, or reserve resources.

use std::collections::{HashMap, HashSet};

use ironclaw_filesystem::{FileType, FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, EffectKind, ExtensionId, HostApiError, PermissionMode,
    ResourceProfile, RuntimeKind, TrustClass, VirtualPath,
};
use serde::Deserialize;
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
    #[error("duplicate capability id {id}")]
    DuplicateCapability { id: CapabilityId },
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

/// Declarative runtime metadata for an extension package.
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
}

/// Validated extension manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionManifest {
    pub id: ExtensionId,
    pub name: String,
    pub version: String,
    pub description: String,
    pub trust: TrustClass,
    pub runtime: ExtensionRuntime,
    pub capabilities: Vec<CapabilityManifest>,
}

impl ExtensionManifest {
    pub fn parse(input: &str) -> Result<Self, ExtensionError> {
        let raw: RawManifest =
            toml::from_str(input).map_err(|error| ExtensionError::ManifestParse {
                reason: error.to_string(),
            })?;
        Self::from_raw(raw)
    }

    pub fn runtime_kind(&self) -> RuntimeKind {
        self.runtime.kind()
    }

    fn from_raw(raw: RawManifest) -> Result<Self, ExtensionError> {
        if raw.name.trim().is_empty() {
            return Err(ExtensionError::InvalidManifest {
                reason: "name must not be empty".to_string(),
            });
        }
        if raw.version.trim().is_empty() {
            return Err(ExtensionError::InvalidManifest {
                reason: "version must not be empty".to_string(),
            });
        }
        if raw.capabilities.is_empty() {
            return Err(ExtensionError::InvalidManifest {
                reason: "at least one capability is required".to_string(),
            });
        }

        let id = ExtensionId::new(raw.id)?;
        let runtime = raw.runtime.into_runtime()?;
        let capabilities = raw
            .capabilities
            .into_iter()
            .map(CapabilityManifest::from_raw)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            id,
            name: raw.name,
            version: raw.version,
            description: raw.description,
            trust: raw.trust,
            runtime,
            capabilities,
        })
    }
}

/// Manifest capability declaration before registry/package context is applied.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityManifest {
    pub id: CapabilityId,
    pub description: String,
    pub effects: Vec<EffectKind>,
    pub default_permission: PermissionMode,
    pub parameters_schema: serde_json::Value,
    pub resource_profile: Option<ResourceProfile>,
}

impl CapabilityManifest {
    fn from_raw(raw: RawCapability) -> Result<Self, ExtensionError> {
        if raw.description.trim().is_empty() {
            return Err(ExtensionError::InvalidManifest {
                reason: format!("capability {} description must not be empty", raw.id),
            });
        }
        Ok(Self {
            id: CapabilityId::new(raw.id)?,
            description: raw.description,
            effects: raw.effects,
            default_permission: raw.default_permission,
            parameters_schema: raw.parameters_schema,
            resource_profile: raw.resource_profile,
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
                    trust_ceiling: manifest.trust,
                    description: capability.description.clone(),
                    parameters_schema: capability.parameters_schema.clone(),
                    effects: capability.effects.clone(),
                    default_permission: capability.default_permission,
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
}

/// Registry of validated extension packages and declared capabilities.
#[derive(Debug, Default)]
pub struct ExtensionRegistry {
    packages: HashMap<ExtensionId, ExtensionPackage>,
    capabilities: HashMap<CapabilityId, CapabilityDescriptor>,
    extension_order: Vec<ExtensionId>,
    capability_order: Vec<CapabilityId>,
}

impl ExtensionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, package: ExtensionPackage) -> Result<(), ExtensionError> {
        validate_package_consistency(&package)?;

        if self.packages.contains_key(&package.id) {
            return Err(ExtensionError::DuplicateExtension { id: package.id });
        }

        let mut seen_capabilities = HashSet::new();
        for descriptor in &package.capabilities {
            if !seen_capabilities.insert(descriptor.id.clone())
                || self.capabilities.contains_key(&descriptor.id)
            {
                return Err(ExtensionError::DuplicateCapability {
                    id: descriptor.id.clone(),
                });
            }
            if descriptor.provider != package.id {
                return Err(ExtensionError::InvalidManifest {
                    reason: format!(
                        "descriptor {} provider {} does not match package {}",
                        descriptor.id, descriptor.provider, package.id
                    ),
                });
            }
        }

        for descriptor in &package.capabilities {
            self.capability_order.push(descriptor.id.clone());
            self.capabilities
                .insert(descriptor.id.clone(), descriptor.clone());
        }
        self.extension_order.push(package.id.clone());
        self.packages.insert(package.id.clone(), package);
        Ok(())
    }

    pub fn get_extension(&self, id: &ExtensionId) -> Option<&ExtensionPackage> {
        self.packages.get(id)
    }

    pub fn get_capability(&self, id: &CapabilityId) -> Option<&CapabilityDescriptor> {
        self.capabilities.get(id)
    }

    pub fn extensions(&self) -> impl Iterator<Item = &ExtensionPackage> {
        self.extension_order
            .iter()
            .filter_map(|id| self.packages.get(id))
    }

    pub fn capabilities(&self) -> impl Iterator<Item = &CapabilityDescriptor> {
        self.capability_order
            .iter()
            .filter_map(|id| self.capabilities.get(id))
    }
}

fn validate_package_consistency(package: &ExtensionPackage) -> Result<(), ExtensionError> {
    let expected = ExtensionPackage::from_manifest(package.manifest.clone(), package.root.clone())?;
    if package.id != expected.id {
        return Err(ExtensionError::InvalidManifest {
            reason: format!(
                "package id {} does not match manifest/root id {}",
                package.id, expected.id
            ),
        });
    }
    if package.capabilities != expected.capabilities {
        return Err(ExtensionError::InvalidManifest {
            reason: "package capability descriptors do not match manifest declarations".to_string(),
        });
    }
    Ok(())
}

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
            let manifest = ExtensionManifest::parse(&text)?;
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    id: String,
    name: String,
    version: String,
    description: String,
    trust: TrustClass,
    runtime: RawRuntime,
    #[serde(default)]
    capabilities: Vec<RawCapability>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RawRuntime {
    Wasm {
        module: String,
    },
    Script {
        runner: Option<String>,
        backend: Option<String>,
        image: Option<String>,
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Mcp {
        transport: String,
        command: Option<String>,
        #[serde(default)]
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

impl RawRuntime {
    fn into_runtime(self) -> Result<ExtensionRuntime, ExtensionError> {
        match self {
            Self::Wasm { module } => Ok(ExtensionRuntime::Wasm {
                module: ExtensionAssetPath::new(module)?,
            }),
            Self::Script {
                runner,
                backend,
                image,
                command,
                args,
            } => {
                let runner = match (runner, backend) {
                    (Some(runner), None) => runner,
                    (None, Some(backend)) => backend,
                    (Some(_), Some(_)) => {
                        return Err(ExtensionError::InvalidManifest {
                            reason: "script runtime must specify either runner or legacy backend, not both".to_string(),
                        });
                    }
                    (None, None) => {
                        return Err(ExtensionError::InvalidManifest {
                            reason: "script runtime runner is required".to_string(),
                        });
                    }
                };
                validate_non_empty("script runner", &runner)?;
                if runner == "docker" {
                    let image = image.as_deref().unwrap_or_default();
                    validate_non_empty("script image", image)?;
                }
                validate_non_empty("script command", &command)?;
                Ok(ExtensionRuntime::Script {
                    runner,
                    image,
                    command,
                    args,
                })
            }
            Self::Mcp {
                transport,
                command,
                args,
                url,
            } => {
                validate_mcp_runtime_shape(&transport, command.as_deref(), url.as_deref())?;
                Ok(ExtensionRuntime::Mcp {
                    transport,
                    command,
                    args,
                    url,
                })
            }
            Self::FirstParty { service } => {
                validate_non_empty("first-party service", &service)?;
                Err(ExtensionError::InvalidManifest {
                    reason: "first-party and system runtimes are host-assigned and cannot be self-asserted by manifests".to_string(),
                })
            }
            Self::System { service } => {
                validate_non_empty("system service", &service)?;
                Err(ExtensionError::InvalidManifest {
                    reason: "first-party and system runtimes are host-assigned and cannot be self-asserted by manifests".to_string(),
                })
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCapability {
    id: String,
    description: String,
    effects: Vec<EffectKind>,
    default_permission: PermissionMode,
    parameters_schema: serde_json::Value,
    #[serde(default)]
    resource_profile: Option<ResourceProfile>,
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

fn validate_mcp_runtime_shape(
    transport: &str,
    command: Option<&str>,
    url: Option<&str>,
) -> Result<(), ExtensionError> {
    validate_non_empty("mcp transport", transport)?;
    if let Some(command) = command {
        validate_non_empty("mcp command", command)?;
    }
    if let Some(url) = url {
        validate_non_empty("mcp url", url)?;
    }

    match transport {
        "stdio" => {
            if url.is_some() {
                return Err(ExtensionError::InvalidManifest {
                    reason: "mcp stdio transport must not specify url".to_string(),
                });
            }
            if command.is_none() {
                return Err(ExtensionError::InvalidManifest {
                    reason: "mcp stdio transport requires command".to_string(),
                });
            }
        }
        "http" | "sse" => {
            if command.is_some() {
                return Err(ExtensionError::InvalidManifest {
                    reason: format!("mcp {transport} transport must not specify command"),
                });
            }
            let Some(url) = url else {
                return Err(ExtensionError::InvalidManifest {
                    reason: format!("mcp {transport} transport requires url"),
                });
            };
            validate_mcp_http_url(transport, url)?;
        }
        _ => {
            return Err(ExtensionError::InvalidManifest {
                reason: "mcp transport must be one of stdio, http, or sse".to_string(),
            });
        }
    }

    Ok(())
}

fn validate_mcp_http_url(transport: &str, value: &str) -> Result<(), ExtensionError> {
    let parsed = url::Url::parse(value).map_err(|_| ExtensionError::InvalidManifest {
        reason: format!("mcp {transport} transport URL must be absolute http(s) URL"),
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ExtensionError::InvalidManifest {
            reason: format!("mcp {transport} transport URL must use http or https"),
        });
    }
    Ok(())
}

fn validate_non_empty(kind: &str, value: &str) -> Result<(), ExtensionError> {
    if value.trim().is_empty() {
        Err(ExtensionError::InvalidManifest {
            reason: format!("{kind} must not be empty"),
        })
    } else {
        Ok(())
    }
}

fn looks_like_windows_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        || (bytes.len() >= 3 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/'))
}
