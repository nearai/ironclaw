//! The resolved extension contract — the deliberate serialization layer for
//! installed manifests.
//!
//! A manifest (v2 or v3) is compiled **once** per install/upgrade into a
//! [`ResolvedExtensionManifest`]; the installation store persists it next to
//! the raw source, and production projection reads the record — raw TOML is
//! kept for diagnostics and recompilation only
//! (`docs/reborn/extension-runtime/overview.md` §3.3, checklist REC-1..4).
//!
//! Rehydration goes through [`ResolvedExtensionManifest::to_internal`], which
//! rebuilds the validated in-memory model without reparsing TOML. Component
//! newtypes re-validate on deserialize, and manifest-source rules
//! (first-party runtime/trust require a host-bundled source) are re-checked
//! during rehydration.

use ironclaw_host_api::{
    ChannelDescriptor, EffectKind, ExtensionId, MemoryDescriptor, PermissionMode,
    RequestedTrustClass, RuntimeCredentialAccountSetup, SecretHandle, VendorAuthRecipe, VendorId,
    VirtualPath,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ExtensionAdminConfigurationDescriptor;

use crate::v2::{
    CapabilityDeclV2, CapabilitySurfaceDeclV2, ExtensionManifestV2, ExtensionRuntimeV2,
    HookSectionEntryV2, HostApiId, HostApiRefV2, ManifestSectionPath, ManifestSource,
    ManifestV2Error, requested_trust_to_descriptor_trust,
};

/// Whether an extension package has a filesystem tree, and if so where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageRootBinding {
    /// Compatibility state for rows written before package roots were typed.
    /// A designated materialization boundary must resolve this before the
    /// manifest reaches a filesystem consumer.
    FabricateOnLoad,
    /// A real package tree exists at this path.
    Materialized(VirtualPath),
    /// The package is remote-only and has no filesystem tree.
    Virtual,
}

impl Default for PackageRootBinding {
    fn default() -> Self {
        Self::FabricateOnLoad
    }
}

/// A caller requested filesystem access for a package without a materialized
/// root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PackageRootError {
    #[error("the legacy package root must be materialized before filesystem access")]
    FabricationRequired,
    #[error("virtual packages do not have a filesystem root")]
    Virtual,
}

impl PackageRootBinding {
    pub fn materialized_root(&self) -> Result<&VirtualPath, PackageRootError> {
        match self {
            Self::Materialized(root) => Ok(root),
            Self::FabricateOnLoad => Err(PackageRootError::FabricationRequired),
            Self::Virtual => Err(PackageRootError::Virtual),
        }
    }
}

/// The persisted, serializable projection of one validated extension
/// manifest. Everything production needs to project surfaces, dispatch
/// tools, mount ingress, run auth, and render UI without touching raw TOML.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedExtensionManifest {
    pub schema_version: String,
    pub id: ExtensionId,
    pub name: String,
    pub version: String,
    pub description: String,
    pub requested_trust: RequestedTrustClass,
    pub runtime: ExtensionRuntimeV2,
    /// Canonical package-root contract. Compatibility for the former
    /// `root: Option<VirtualPath>` wire is private to `WireManifestRecord`.
    #[serde(default)]
    pub root_binding: PackageRootBinding,
    /// Whether a fresh installation can activate immediately or must first
    /// pass through the composed preparation strategy.
    #[serde(default)]
    pub initial_preparation: PreparationRequirement,
    /// Present iff the manifest declares `[mcp]` (v3 hosted MCP servers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<ResolvedMcpDeclaration>,
    /// Tool declarations (static `[[tools]]`, v2 projected capabilities, or
    /// the synthesized host-internal MCP connection template).
    pub tools: Vec<CapabilityDeclV2>,
    /// The declared channel surface (v3 `[channel]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<ChannelDescriptor>,
    /// The declared memory-provider surface (v3 `[memory]`). Host-internal: the
    /// compose-time memory-provider binding reads it to recognize a backend for
    /// the host memory adapter. Never a product lifecycle surface (memory is
    /// always-on and not installed/removed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryDescriptor>,
    /// Deployment-owned values required before users can use this extension.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub admin_configuration: Vec<ExtensionAdminConfigurationDescriptor>,
    /// One entry per vendor the extension authenticates against.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auth: Vec<ResolvedAuthSurface>,
    /// v2 host-api contract references (empty for v3 manifests).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub host_apis: Vec<ResolvedHostApiRef>,
    /// v2 contract-projected section surfaces (empty for v3 manifests).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub section_surfaces: Vec<ResolvedSectionSurface>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<HookSectionEntryV2>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreparationRequirement {
    #[default]
    Ready,
    Required,
}

/// A hosted-MCP server declaration (`[mcp]`): the proxied server whose
/// `tools/list` is the tool source of truth, plus the ceiling every
/// discovered tool must fit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedMcpDeclaration {
    /// The server URL (https).
    pub server: String,
    /// Discovered tools publish as `{namespace}.<tool>`; equals the
    /// extension id.
    pub namespace: String,
    pub max_tools: u32,
    pub default_permission: PermissionMode,
    /// Effect ceiling for every discovered tool.
    pub effects: Vec<EffectKind>,
    /// The server-connection credential handles (injected on every server
    /// call; discovered tools cannot declare their own).
    pub credential_handles: Vec<SecretHandle>,
    /// Admission-validated schemas for a remote-only discovered catalog.
    /// Filesystem-backed packages leave this empty and resolve schema refs
    /// through their materialized package root.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub dynamic_input_schemas: std::collections::BTreeMap<String, serde_json::Value>,
    /// Immutable registration-time authentication selection for a remote
    /// user-registered MCP definition. Bundled/static providers use NoAuth
    /// here because their concrete auth recipe remains the authority.
    #[serde(default)]
    pub registration_auth: ironclaw_host_api::HostedMcpAuthSelection,
}

/// One vendor the extension authenticates against: the account setup this
/// extension requires plus, for v3 manifests, the recipe the host auth
/// engine executes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedAuthSurface {
    pub vendor: VendorId,
    pub setup: RuntimeCredentialAccountSetup,
    /// `None` for v2 manifests (no recipe vocabulary); required in v3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<VendorAuthRecipe>,
    /// Exact RFC 9728 document admitted while preparing a user-registered
    /// OAuth MCP. This stays with the recipe so later DCR uses the advertised
    /// location rather than guessing a well-known resource path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protected_resource_metadata_url: Option<ironclaw_host_api::HttpsEndpoint>,
}

/// Serializable mirror of a v2 `[[host_api]]` reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedHostApiRef {
    pub id: String,
    pub section: String,
}

/// Serializable mirror of a v2 contract-projected section surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedSectionSurface {
    pub kind: ironclaw_host_api::CapabilitySurfaceKind,
    pub host_api: String,
    pub section: String,
}

impl ResolvedExtensionManifest {
    pub fn package_root_binding(&self) -> &PackageRootBinding {
        &self.root_binding
    }

    pub fn materialized_root(&self) -> Result<&VirtualPath, PackageRootError> {
        self.root_binding.materialized_root()
    }

    /// Project a validated v2 manifest into the resolved contract.
    pub fn from_v2(manifest: &ExtensionManifestV2) -> Self {
        let auth = manifest
            .capability_surfaces()
            .into_iter()
            .filter_map(|surface| match surface {
                CapabilitySurfaceDeclV2::Auth { provider, setup } => Some(ResolvedAuthSurface {
                    vendor: provider,
                    setup,
                    recipe: None,
                    protected_resource_metadata_url: None,
                }),
                _ => None,
            })
            .collect();
        let section_surfaces = manifest
            .host_api_surfaces
            .iter()
            .filter_map(|surface| match surface {
                CapabilitySurfaceDeclV2::HostApiSection {
                    kind,
                    host_api,
                    section,
                } => Some(ResolvedSectionSurface {
                    kind: *kind,
                    host_api: host_api.as_str().to_string(),
                    section: section.as_str().to_string(),
                }),
                _ => None,
            })
            .collect();
        Self {
            schema_version: manifest.schema_version.clone(),
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            description: manifest.description.clone(),
            requested_trust: manifest.requested_trust,
            runtime: manifest.runtime.clone(),
            // Parsing precedes package materialization. Only the designated
            // legacy loader may resolve this compatibility state.
            root_binding: PackageRootBinding::FabricateOnLoad,
            initial_preparation: PreparationRequirement::Ready,
            mcp: None,
            tools: manifest.capabilities.clone(),
            channel: None,
            memory: None,
            admin_configuration: Vec::new(),
            auth,
            host_apis: manifest
                .host_apis
                .iter()
                .map(|host_api| ResolvedHostApiRef {
                    id: host_api.id.as_str().to_string(),
                    section: host_api.section.as_str().to_string(),
                })
                .collect(),
            section_surfaces,
            hooks: manifest.hooks.clone(),
        }
    }

    /// Rebuild the validated in-memory manifest from the resolved contract
    /// — no TOML reparse. Manifest-source rules are re-checked so a record
    /// copied to a less-privileged source cannot smuggle first-party runtime
    /// or trust.
    pub fn to_internal(
        &self,
        source: ManifestSource,
    ) -> Result<ExtensionManifestV2, ManifestV2Error> {
        if !source.allows_first_party()
            && matches!(
                self.requested_trust,
                RequestedTrustClass::FirstPartyRequested | RequestedTrustClass::SystemRequested
            )
        {
            return Err(ManifestV2Error::TrustForbiddenForSource {
                manifest_source: source,
                requested: self.requested_trust,
            });
        }
        if !source.allows_first_party()
            && matches!(
                self.runtime,
                ExtensionRuntimeV2::FirstParty { .. } | ExtensionRuntimeV2::System { .. }
            )
        {
            return Err(ManifestV2Error::RuntimeForbiddenForSource {
                manifest_source: source,
                kind: self.runtime.kind(),
            });
        }

        let mut host_api_surfaces = Vec::new();
        if let Some(channel) = &self.channel {
            host_api_surfaces.push(CapabilitySurfaceDeclV2::Channel {
                channel: channel.id.clone(),
            });
        }
        for surface in &self.section_surfaces {
            host_api_surfaces.push(CapabilitySurfaceDeclV2::HostApiSection {
                kind: surface.kind,
                host_api: HostApiId::new(surface.host_api.clone())?,
                section: ManifestSectionPath::new(surface.section.clone())?,
            });
        }
        let host_apis = self
            .host_apis
            .iter()
            .map(|host_api| {
                Ok(HostApiRefV2 {
                    id: HostApiId::new(host_api.id.clone())?,
                    section: ManifestSectionPath::new(host_api.section.clone())?,
                })
            })
            .collect::<Result<Vec<_>, ManifestV2Error>>()?;

        Ok(ExtensionManifestV2 {
            schema_version: self.schema_version.clone(),
            id: self.id.clone(),
            name: self.name.clone(),
            version: self.version.clone(),
            description: self.description.clone(),
            source,
            requested_trust: self.requested_trust,
            descriptor_trust_default: requested_trust_to_descriptor_trust(self.requested_trust),
            runtime: self.runtime.clone(),
            host_apis,
            capabilities: self.tools.clone(),
            host_api_surfaces,
            hooks: self.hooks.clone(),
        })
    }
}
