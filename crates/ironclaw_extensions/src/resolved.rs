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

use std::collections::{BTreeMap, BTreeSet};

use ironclaw_host_api::{
    ChannelDescriptor, EffectKind, ExtensionId, PermissionMode, RequestedTrustClass,
    RuntimeCredentialAccountSetup, RuntimeCredentialRequirementSource, SecretHandle,
    VendorAuthRecipe, VendorId,
};
use serde::{Deserialize, Serialize};

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

// ---------------------------------------------------------------------------
// Contract diff (upgrade widening classification)
// ---------------------------------------------------------------------------

/// Result of comparing an old and new resolved contract on upgrade.
/// Widening requires renewed user approval; equal or narrower contracts do
/// not (`docs/reborn/extension-runtime/overview.md` §3.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContractDelta {
    Equal,
    Narrowed,
    Widened { reasons: Vec<WideningReason> },
}

/// One axis on which the new contract asks for more authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "axis")]
pub enum WideningReason {
    NewVendorScopes {
        vendor: String,
        scopes: Vec<String>,
    },
    NewEgressHosts {
        hosts: Vec<String>,
    },
    NewEffects {
        effects: Vec<EffectKind>,
    },
    NewCredentialHandles {
        handles: Vec<String>,
    },
    IngressRouteChanged {
        old: Option<String>,
        new: Option<String>,
    },
}

impl std::fmt::Display for WideningReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NewVendorScopes { vendor, scopes } => {
                write!(f, "new scopes for vendor {vendor}: {}", scopes.join(", "))
            }
            Self::NewEgressHosts { hosts } => {
                write!(f, "new egress hosts: {}", hosts.join(", "))
            }
            Self::NewEffects { effects } => {
                let rendered: Vec<String> = effects.iter().map(effect_key).collect();
                write!(f, "new effects: {}", rendered.join(", "))
            }
            Self::NewCredentialHandles { handles } => {
                write!(f, "new credential handles: {}", handles.join(", "))
            }
            Self::IngressRouteChanged { old, new } => write!(
                f,
                "ingress route changed: {} -> {}",
                old.as_deref().unwrap_or("<none>"),
                new.as_deref().unwrap_or("<none>")
            ),
        }
    }
}

fn effect_key(effect: &EffectKind) -> String {
    serde_json::to_value(effect)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{effect:?}"))
}

struct ContractAuthorityView {
    vendor_scopes: BTreeMap<String, BTreeSet<String>>,
    egress_hosts: BTreeSet<String>,
    effects: BTreeSet<String>,
    credential_handles: BTreeSet<String>,
    ingress_route: Option<String>,
}

fn authority_view(contract: &ResolvedExtensionManifest) -> ContractAuthorityView {
    let mut vendor_scopes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut egress_hosts = BTreeSet::new();
    let mut effects = BTreeSet::new();
    let mut credential_handles = BTreeSet::new();

    for auth in &contract.auth {
        let entry = vendor_scopes
            .entry(auth.vendor.as_str().to_string())
            .or_default();
        if let RuntimeCredentialAccountSetup::OAuth { scopes } = &auth.setup {
            entry.extend(scopes.iter().cloned());
        }
        if let Some(recipe) = &auth.recipe {
            entry.extend(recipe.scope_ceiling().iter().cloned());
        }
    }
    for tool in &contract.tools {
        for effect in &tool.effects {
            effects.insert(effect_key(effect));
        }
        for credential in &tool.runtime_credentials {
            credential_handles.insert(credential.handle.as_str().to_string());
            egress_hosts.insert(credential.audience.host_pattern.clone());
            if let RuntimeCredentialRequirementSource::ProductAuthAccount { provider, .. } =
                &credential.source
            {
                vendor_scopes
                    .entry(provider.as_str().to_string())
                    .or_default()
                    .extend(credential.provider_scopes.iter().cloned());
            }
        }
    }
    if let Some(mcp) = &contract.mcp {
        for effect in &mcp.effects {
            effects.insert(effect_key(effect));
        }
        for handle in &mcp.credential_handles {
            credential_handles.insert(handle.as_str().to_string());
        }
        if let Some(host) = mcp
            .server
            .split("://")
            .nth(1)
            .and_then(|rest| rest.split(['/', '?']).next().map(str::to_string))
        {
            egress_hosts.insert(host);
        }
    }
    let mut ingress_route = None;
    if let Some(channel) = &contract.channel {
        for egress in &channel.egress {
            egress_hosts.insert(egress.host.clone());
            if let Some(handle) = &egress.credential_handle {
                credential_handles.insert(handle.as_str().to_string());
            }
        }
        for field in &channel.config.fields {
            credential_handles.insert(field.handle.as_str().to_string());
        }
        if let Some(ingress) = &channel.ingress {
            ingress_route = Some(format!("post:{}", ingress.route_suffix.as_str()));
        }
    }

    ContractAuthorityView {
        vendor_scopes,
        egress_hosts,
        effects,
        credential_handles,
        ingress_route,
    }
}

/// Classify an upgrade from `old` to `new` (checklist REC-4).
pub fn diff_resolved_contracts(
    old: &ResolvedExtensionManifest,
    new: &ResolvedExtensionManifest,
) -> ContractDelta {
    let old_view = authority_view(old);
    let new_view = authority_view(new);

    let mut reasons = Vec::new();

    for (vendor, scopes) in &new_view.vendor_scopes {
        let known: Vec<String> = match old_view.vendor_scopes.get(vendor) {
            Some(old_scopes) => scopes.difference(old_scopes).cloned().collect(),
            None => scopes.iter().cloned().collect(),
        };
        let is_new_vendor = !old_view.vendor_scopes.contains_key(vendor);
        if !known.is_empty() || is_new_vendor {
            reasons.push(WideningReason::NewVendorScopes {
                vendor: vendor.clone(),
                scopes: known,
            });
        }
    }
    let new_hosts: Vec<String> = new_view
        .egress_hosts
        .difference(&old_view.egress_hosts)
        .cloned()
        .collect();
    if !new_hosts.is_empty() {
        reasons.push(WideningReason::NewEgressHosts { hosts: new_hosts });
    }
    let new_effects: Vec<String> = new_view
        .effects
        .difference(&old_view.effects)
        .cloned()
        .collect();
    if !new_effects.is_empty() {
        // Re-key into EffectKind for the typed reason where possible.
        let effects = new_effects
            .iter()
            .filter_map(|key| serde_json::from_value(serde_json::Value::String(key.clone())).ok())
            .collect();
        reasons.push(WideningReason::NewEffects { effects });
    }
    let new_handles: Vec<String> = new_view
        .credential_handles
        .difference(&old_view.credential_handles)
        .cloned()
        .collect();
    if !new_handles.is_empty() {
        reasons.push(WideningReason::NewCredentialHandles {
            handles: new_handles,
        });
    }
    if new_view.ingress_route != old_view.ingress_route && new_view.ingress_route.is_some() {
        reasons.push(WideningReason::IngressRouteChanged {
            old: old_view.ingress_route.clone(),
            new: new_view.ingress_route.clone(),
        });
    }

    if !reasons.is_empty() {
        return ContractDelta::Widened { reasons };
    }

    let narrowed = old_view.vendor_scopes.iter().any(|(vendor, scopes)| {
        new_view
            .vendor_scopes
            .get(vendor)
            .map(|new_scopes| new_scopes != scopes)
            .unwrap_or(true)
    }) || old_view.egress_hosts != new_view.egress_hosts
        || old_view.effects != new_view.effects
        || old_view.credential_handles != new_view.credential_handles
        || old_view.ingress_route != new_view.ingress_route;

    if narrowed {
        ContractDelta::Narrowed
    } else {
        ContractDelta::Equal
    }
}
