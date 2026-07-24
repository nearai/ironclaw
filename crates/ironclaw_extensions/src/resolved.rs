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
};
use serde::{Deserialize, Serialize};

use crate::ExtensionAdminConfigurationDescriptor;

use crate::v2::{
    CapabilityDeclV2, CapabilitySurfaceDeclV2, ExtensionManifestV2, ExtensionRuntimeV2,
    HookSectionEntryV2, HostApiId, HostApiRefV2, ManifestSectionPath, ManifestSource,
    ManifestV2Error, requested_trust_to_descriptor_trust,
};

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
