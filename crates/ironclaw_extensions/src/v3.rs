//! Extension Manifest v3.
//!
//! v3 is v2 plus explicit surface declarations: top-level `[[tools]]`, at
//! most one `[channel]`, `[auth.<vendor>]` recipes, and — for hosted MCP
//! servers — one `[mcp]` declaration instead of `[runtime]`. An `[mcp]`
//! manifest may still pin static `[[tools]]`: they are the surfaces
//! guaranteed present without live discovery (bundled fallback, first boot)
//! and inherit the connection template's credential/effect shape; a
//! successful tools/list discovery replaces them with the live catalog.
//! The host-api contract indirection (`[[host_api]]` + operational sections)
//! is gone: surfaces are first-class manifest vocabulary.
//!
//! Both schemas parse through the single entry point
//! (`ExtensionManifestRecord::from_toml`) and normalize into the same
//! internal model plus a [`crate::ResolvedExtensionManifest`]. Everything
//! here is fail-closed: unknown fields, non-https endpoints, wildcard hosts,
//! reserved authorize params, and undeclared vendors are rejected with
//! path-qualified errors.

use std::collections::BTreeMap;

use ironclaw_host_api::{
    ChannelDescriptor, ChannelDescriptorError, EffectKind, ExtensionId,
    HOST_RUNTIME_HTTP_EGRESS_PORT_ID, HostApiError, HostPortCatalog, NetworkScheme,
    NetworkTargetPattern, OriginGateMatrix, PermissionMode, RecipeValidationError,
    RequestedTrustClass, RuntimeCredentialAccountSetup, RuntimeCredentialRequirementSource,
    RuntimeCredentialTarget, VendorAuthRecipe, VendorId,
};
use serde::Deserialize;
use thiserror::Error;

use crate::ExtensionAdminConfigurationDescriptor;
use crate::resolved::{ResolvedAuthSurface, ResolvedExtensionManifest, ResolvedMcpDeclaration};
use crate::v2::{
    CapabilityDeclV2, CapabilitySurfaceDeclV2, ExtensionManifestV2, ExtensionRuntimeV2,
    MAX_MANIFEST_BYTES, ManifestSource, RESERVED_HOST_BUNDLED_ID_PREFIX, RawCapabilityV2,
    RawRuntimeCredentialV2, requested_trust_to_descriptor_trust,
};

/// Required value of the `schema_version` field for v3 manifests.
pub const MANIFEST_SCHEMA_VERSION_V3: &str = "reborn.extension_manifest.v3";

/// v3 manifest parse/validation errors.
#[derive(Debug, Error)]
pub enum ManifestV3Error {
    #[error("extension manifest exceeds maximum size of {max} bytes")]
    TooLarge { max: usize },
    #[error("failed to parse extension manifest: {reason}")]
    Parse { reason: String },
    #[error("invalid extension manifest: {reason}")]
    Invalid { reason: String },
    #[error("exactly one of [runtime] or [mcp] must declare the implementation")]
    RuntimeDeclaration,
    #[error("[mcp] is mutually exclusive with [channel]")]
    McpExclusivity,
    #[error("[auth.{vendor}] recipe is invalid: {error}")]
    InvalidRecipe {
        vendor: String,
        error: RecipeValidationError,
    },
    #[error("[channel] is invalid: {error}")]
    InvalidChannel { error: ChannelDescriptorError },
    #[error(
        "credential vendor `{vendor}` has no [auth.{vendor}] recipe; v3 manifests must declare \
         one for every referenced vendor"
    )]
    MissingAuthRecipe { vendor: String },
    #[error("[auth.{vendor}] recipe is not referenced by any credential")]
    UnreferencedAuthRecipe { vendor: String },
    #[error(
        "credential audience host `{host}` must be a literal host (wildcards are not allowed \
         in v3 manifests)"
    )]
    WildcardAudienceHost { host: String },
    #[error("[mcp].namespace `{namespace}` must equal the extension id `{id}`")]
    McpNamespaceMismatch { namespace: String, id: String },
    #[error("[mcp].max_tools must be at least 1")]
    McpZeroMaxTools,
    #[error(transparent)]
    Contract(#[from] HostApiError),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifestV3 {
    schema_version: String,
    id: String,
    name: String,
    version: String,
    description: String,
    #[serde(
        default = "crate::v2::default_requested_trust",
        deserialize_with = "crate::v2::deserialize_requested_trust"
    )]
    trust: RequestedTrustClass,
    #[serde(default)]
    runtime: Option<RawRuntimeV3>,
    #[serde(default)]
    mcp: Option<RawMcpV3>,
    #[serde(default)]
    tools: Vec<RawToolV3>,
    #[serde(default)]
    channel: Option<ChannelDescriptor>,
    #[serde(default)]
    auth: BTreeMap<String, VendorAuthRecipe>,
    #[serde(default)]
    admin_configuration: Option<ExtensionAdminConfigurationDescriptor>,
    /// Free-form authoring metadata, ignored (same exemption as v2's
    /// `metadata` root).
    #[serde(default)]
    #[allow(dead_code)]
    metadata: Option<toml::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RawRuntimeV3 {
    Wasm { module: String },
    FirstParty { service: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawToolV3 {
    id: String,
    #[serde(default)]
    origin_gate_matrix: Option<OriginGateMatrix>,
    description: String,
    #[serde(default)]
    effects: Vec<EffectKind>,
    default_permission: PermissionMode,
    #[serde(default = "default_tool_visibility")]
    visibility: crate::v2::CapabilityVisibility,
    input_schema_ref: String,
    #[serde(default)]
    output_schema_ref: Option<String>,
    #[serde(default)]
    prompt_doc_ref: Option<String>,
    #[serde(default)]
    credentials: Vec<RawToolCredentialV3>,
    /// Credential-free egress targets for a networked tool (e.g. a provider
    /// endpoint that 302-redirects to pre-signed blob storage). Populates the
    /// capability's egress allowlist directly; v2's network-effect validation
    /// still applies.
    #[serde(default)]
    network_targets: Vec<ironclaw_host_api::NetworkTargetPattern>,
    /// Optional per-tool egress cap (bytes). `#[serde(default)]` so tools
    /// without the key parse to `None` (no cap). Threads to the capability's
    /// `NetworkPolicy.max_egress_bytes` at grant issuance.
    #[serde(default)]
    max_egress_bytes: Option<u64>,
    #[serde(default)]
    resource_profile: Option<ironclaw_host_api::ResourceProfile>,
}

fn default_tool_visibility() -> crate::v2::CapabilityVisibility {
    crate::v2::CapabilityVisibility::Model
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawToolCredentialV3 {
    handle: String,
    vendor: String,
    #[serde(default)]
    scopes: Vec<String>,
    audience: RawAudienceV3,
    injection: RuntimeCredentialTarget,
    #[serde(default = "default_credential_required")]
    required: bool,
}

fn default_credential_required() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAudienceV3 {
    scheme: NetworkScheme,
    host: String,
    #[serde(default)]
    port: Option<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMcpV3 {
    server: ironclaw_host_api::HttpsEndpoint,
    #[serde(default)]
    origin_gate_matrix: Option<OriginGateMatrix>,
    namespace: String,
    max_tools: u32,
    #[serde(default = "default_mcp_permission")]
    default_permission: PermissionMode,
    effects: Vec<EffectKind>,
    #[serde(default)]
    credentials: Vec<RawMcpCredentialV3>,
}

fn default_mcp_permission() -> PermissionMode {
    PermissionMode::Ask
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMcpCredentialV3 {
    handle: String,
    vendor: String,
    #[serde(default)]
    scopes: Vec<String>,
    injection: RuntimeCredentialTarget,
}

/// Parse and normalize a v3 manifest into the internal model plus the
/// resolved contract.
pub(crate) fn parse_v3(
    input: &str,
    source: ManifestSource,
    host_port_catalog: &HostPortCatalog,
) -> Result<(ExtensionManifestV2, ResolvedExtensionManifest), ManifestV3Error> {
    if input.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestV3Error::TooLarge {
            max: MAX_MANIFEST_BYTES,
        });
    }
    let raw: RawManifestV3 = toml::from_str(input).map_err(|error| ManifestV3Error::Parse {
        reason: error.to_string(),
    })?;

    if raw.schema_version != MANIFEST_SCHEMA_VERSION_V3 {
        return Err(ManifestV3Error::Invalid {
            reason: format!(
                "schema_version must be {MANIFEST_SCHEMA_VERSION_V3}, got {}",
                raw.schema_version
            ),
        });
    }
    for (field, value) in [
        ("name", &raw.name),
        ("version", &raw.version),
        ("description", &raw.description),
    ] {
        if value.trim().is_empty() {
            return Err(ManifestV3Error::Invalid {
                reason: format!("{field} must not be empty"),
            });
        }
    }

    let id = ExtensionId::new(raw.id)?;
    if !source.allows_first_party() && id.as_str().starts_with(RESERVED_HOST_BUNDLED_ID_PREFIX) {
        return Err(ManifestV3Error::Invalid {
            reason: format!(
                "extension id `{id}` uses the reserved `{RESERVED_HOST_BUNDLED_ID_PREFIX}` \
                 prefix, which is host-bundled only"
            ),
        });
    }
    if !source.allows_first_party()
        && matches!(
            raw.trust,
            RequestedTrustClass::FirstPartyRequested | RequestedTrustClass::SystemRequested
        )
    {
        return Err(ManifestV3Error::Invalid {
            reason: format!(
                "trust `{:?}` is not allowed for this manifest source",
                raw.trust
            ),
        });
    }

    // Exactly one implementation declaration.
    let (runtime, mcp) = match (raw.runtime, raw.mcp) {
        (Some(runtime), None) => (runtime_from_raw(runtime), None),
        (None, Some(mcp)) => {
            if raw.channel.is_some() {
                return Err(ManifestV3Error::McpExclusivity);
            }
            if mcp.namespace != id.as_str() {
                return Err(ManifestV3Error::McpNamespaceMismatch {
                    namespace: mcp.namespace,
                    id: id.as_str().to_string(),
                });
            }
            if mcp.max_tools == 0 {
                return Err(ManifestV3Error::McpZeroMaxTools);
            }
            let runtime = ExtensionRuntimeV2::Mcp {
                transport: "http".to_string(),
                command: None,
                args: Vec::new(),
                url: Some(mcp.server.as_str().to_string()),
            };
            (runtime, Some(mcp))
        }
        _ => return Err(ManifestV3Error::RuntimeDeclaration),
    };
    if !source.allows_first_party()
        && matches!(
            runtime,
            ExtensionRuntimeV2::FirstParty { .. } | ExtensionRuntimeV2::System { .. }
        )
    {
        return Err(ManifestV3Error::Invalid {
            reason: "first_party runtime requires a host-bundled manifest source".to_string(),
        });
    }
    let sandboxed_runtime = matches!(
        runtime,
        ExtensionRuntimeV2::Wasm { .. } | ExtensionRuntimeV2::Mcp { .. }
    );

    // Validate recipes.
    let mut recipes: BTreeMap<VendorId, VendorAuthRecipe> = BTreeMap::new();
    for (vendor, recipe) in raw.auth {
        recipe
            .validate()
            .map_err(|error| ManifestV3Error::InvalidRecipe {
                vendor: vendor.clone(),
                error,
            })?;
        recipes.insert(VendorId::new(vendor)?, recipe);
    }

    // Validate the channel declaration.
    if let Some(channel) = &raw.channel {
        channel
            .validate()
            .map_err(|error| ManifestV3Error::InvalidChannel { error })?;
    }
    if let Some(descriptor) = &raw.admin_configuration {
        // Trust gate: an `[admin_configuration]` group declares deployment-owned,
        // operator-managed secrets and routing. Only a host-bundled (first-party)
        // manifest — one compiled into the host binary — may declare one. An
        // untrusted, filesystem-discovered, or registry-installed manifest must
        // not: otherwise it could collide with a first-party group id (aborting
        // boot via a descriptor conflict) or register itself as a consumer of a
        // first-party group's non-secret routing (a confused-deputy read). This
        // is the earliest fail-closed point; composition's fold applies the same
        // source gate as defense in depth.
        if !source.allows_first_party() {
            return Err(ManifestV3Error::Invalid {
                reason:
                    "[admin_configuration] declares a deployment-owned administrator group, which \
                     is reserved for host-bundled (first-party) manifests"
                        .to_string(),
            });
        }
        descriptor
            .validate()
            .map_err(|error| ManifestV3Error::Invalid {
                reason: format!("[admin_configuration] is invalid: {error}"),
            })?;
    }
    if let Some(channel) = &raw.channel {
        validate_channel_admin_configuration(channel, raw.admin_configuration.as_ref())?;
    }

    // Normalize tools (or the synthesized MCP connection template) into the
    // internal capability model, reusing the v2 validated construction path.
    let mut referenced_vendors: BTreeMap<VendorId, ()> = BTreeMap::new();
    let mut capabilities = Vec::new();
    let mut mcp_template_credentials = None;
    if let Some(mcp) = &mcp {
        let template_credentials = mcp
            .credentials
            .iter()
            .map(|credential| {
                credential_from_v3(
                    &credential.handle,
                    &credential.vendor,
                    &credential.scopes,
                    RawAudienceV3 {
                        scheme: NetworkScheme::Https,
                        host: mcp.server.host(),
                        port: None,
                    },
                    credential.injection.clone(),
                    true,
                    &recipes,
                    &mut referenced_vendors,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let raw_capability = RawCapabilityV2 {
            id: format!("{id}.mcp_server"),
            network_targets: Vec::new(),
            max_egress_bytes: None,
            implements: Vec::new(),
            description: format!(
                "Hosted MCP server connection for {} (discovery template; never model-visible)",
                raw.name
            ),
            effects: with_dispatch_effect(mcp.effects.clone()),
            default_permission: mcp.default_permission,
            visibility: crate::v2::CapabilityVisibility::HostInternal,
            input_schema_ref: format!("schemas/{id}/dynamic/mcp_server.input.v1.json"),
            output_schema_ref: None,
            prompt_doc_ref: None,
            required_host_ports: derived_host_ports(&mcp.effects, true),
            runtime_credentials: template_credentials.clone(),
            resource_profile: None,
            origin_gate_matrix: mcp.origin_gate_matrix.clone(),
        };
        capabilities.push(
            CapabilityDeclV2::from_raw(raw_capability, &id, host_port_catalog).map_err(
                |error| ManifestV3Error::Invalid {
                    reason: error.to_string(),
                },
            )?,
        );
        mcp_template_credentials = Some(template_credentials);
    }
    for tool in raw.tools {
        // Statically pinned tools on an `[mcp]` manifest are the surfaces
        // guaranteed present without live discovery (bundled fallback, first
        // boot); a successful tools/list discovery replaces them with the
        // server's live catalog. They inherit the connection template's
        // credential/effect/host-port shape so every capability on the
        // package stays template-consistent — including when composition
        // patches `[mcp].server` to a configured endpoint.
        let raw_capability = match (&mcp, &mcp_template_credentials) {
            (Some(mcp), Some(template_credentials)) => {
                if !tool.credentials.is_empty()
                    || !tool.effects.is_empty()
                    || tool.resource_profile.is_some()
                    || !tool.network_targets.is_empty()
                    || tool.max_egress_bytes.is_some()
                    || tool.output_schema_ref.is_some()
                {
                    return Err(ManifestV3Error::Invalid {
                        reason: format!(
                            "static tool `{}` on an [mcp] manifest inherits the server \
                             connection template; remove its credentials, effects, \
                             network_targets, max_egress_bytes, output_schema_ref, and \
                             resource_profile",
                            tool.id
                        ),
                    });
                }
                RawCapabilityV2 {
                    id: tool.id,
                    network_targets: Vec::new(),
                    max_egress_bytes: None,
                    implements: Vec::new(),
                    description: tool.description,
                    effects: with_dispatch_effect(mcp.effects.clone()),
                    default_permission: tool.default_permission,
                    visibility: tool.visibility,
                    input_schema_ref: tool.input_schema_ref,
                    output_schema_ref: None,
                    prompt_doc_ref: tool.prompt_doc_ref,
                    required_host_ports: derived_host_ports(&mcp.effects, true),
                    runtime_credentials: template_credentials.clone(),
                    resource_profile: None,
                    origin_gate_matrix: mcp.origin_gate_matrix.clone(),
                }
            }
            _ => RawCapabilityV2 {
                id: tool.id,
                network_targets: tool.network_targets,
                max_egress_bytes: tool.max_egress_bytes,
                implements: Vec::new(),
                description: tool.description,
                effects: with_dispatch_effect(tool.effects.clone()),
                default_permission: tool.default_permission,
                visibility: tool.visibility,
                input_schema_ref: tool.input_schema_ref,
                output_schema_ref: tool.output_schema_ref,
                prompt_doc_ref: tool.prompt_doc_ref,
                required_host_ports: derived_host_ports(&tool.effects, sandboxed_runtime),
                runtime_credentials: tool
                    .credentials
                    .into_iter()
                    .map(|credential| {
                        credential_from_v3(
                            &credential.handle,
                            &credential.vendor,
                            &credential.scopes,
                            credential.audience,
                            credential.injection,
                            credential.required,
                            &recipes,
                            &mut referenced_vendors,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                resource_profile: tool.resource_profile,
                origin_gate_matrix: tool.origin_gate_matrix,
            },
        };
        capabilities.push(
            CapabilityDeclV2::from_raw(raw_capability, &id, host_port_catalog).map_err(
                |error| ManifestV3Error::Invalid {
                    reason: error.to_string(),
                },
            )?,
        );
    }

    // Every declared recipe must be referenced, and (checked inside
    // credential_from_v3) every referenced vendor must have a recipe.
    for vendor in recipes.keys() {
        if !referenced_vendors.contains_key(vendor) {
            return Err(ManifestV3Error::UnreferencedAuthRecipe {
                vendor: vendor.as_str().to_string(),
            });
        }
    }

    let mut host_api_surfaces = Vec::new();
    if let Some(channel) = &raw.channel {
        host_api_surfaces.push(CapabilitySurfaceDeclV2::Channel {
            channel: channel.id.clone(),
        });
    }

    let manifest = ExtensionManifestV2 {
        schema_version: raw.schema_version,
        id: id.clone(),
        name: raw.name,
        version: raw.version,
        description: raw.description,
        source,
        requested_trust: raw.trust,
        descriptor_trust_default: requested_trust_to_descriptor_trust(raw.trust),
        runtime,
        host_apis: Vec::new(),
        capabilities,
        host_api_surfaces,
        hooks: Vec::new(),
    };

    let auth = recipes
        .into_iter()
        .map(|(vendor, recipe)| {
            let setup = match &recipe {
                VendorAuthRecipe::Oauth2Code(oauth) => RuntimeCredentialAccountSetup::OAuth {
                    scopes: oauth.scopes.clone(),
                },
                VendorAuthRecipe::ApiKey(_) => RuntimeCredentialAccountSetup::ManualToken,
            };
            ResolvedAuthSurface {
                vendor,
                setup,
                recipe: Some(recipe),
            }
        })
        .collect();
    let resolved = ResolvedExtensionManifest {
        schema_version: manifest.schema_version.clone(),
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        description: manifest.description.clone(),
        requested_trust: manifest.requested_trust,
        runtime: manifest.runtime.clone(),
        mcp: mcp.map(|mcp| ResolvedMcpDeclaration {
            server: mcp.server.as_str().to_string(),
            namespace: mcp.namespace,
            max_tools: mcp.max_tools,
            default_permission: mcp.default_permission,
            effects: mcp.effects,
            credential_handles: manifest
                .capabilities
                .first()
                .map(|template| {
                    template
                        .runtime_credentials
                        .iter()
                        .map(|credential| credential.handle.clone())
                        .collect()
                })
                .unwrap_or_default(),
        }),
        tools: manifest.capabilities.clone(),
        channel: raw.channel,
        admin_configuration: raw.admin_configuration.into_iter().collect(),
        auth,
        host_apis: Vec::new(),
        section_surfaces: Vec::new(),
        hooks: Vec::new(),
    };

    Ok((manifest, resolved))
}

/// Validate every channel runtime reference against the one manifest-owned
/// deployment configuration schema. The neutral channel contract validates
/// channel structure; the extension-manifest layer owns this cross-section
/// relationship because only it can see `[admin_configuration]`.
fn validate_channel_admin_configuration(
    channel: &ChannelDescriptor,
    descriptor: Option<&ExtensionAdminConfigurationDescriptor>,
) -> Result<(), ManifestV3Error> {
    let require_field = |handle: &ironclaw_host_api::SecretHandle,
                         secret: bool,
                         usage: &str|
     -> Result<(), ManifestV3Error> {
        let field = descriptor
            .and_then(|descriptor| {
                descriptor
                    .fields
                    .iter()
                    .find(|field| field.handle == *handle)
            })
            .ok_or_else(|| ManifestV3Error::Invalid {
                reason: format!(
                    "{usage} handle `{}` must be declared in [admin_configuration].fields",
                    handle.as_str()
                ),
            })?;
        if field.secret != secret {
            return Err(ManifestV3Error::Invalid {
                reason: format!(
                    "{usage} handle `{}` must be declared as secret = {secret} in [admin_configuration].fields",
                    handle.as_str()
                ),
            });
        }
        Ok(())
    };

    if let Some(handle) = channel
        .ingress
        .as_ref()
        .and_then(|ingress| ingress.verification.secret_handle())
    {
        require_field(handle, true, "channel ingress verification")?;
    }
    for egress in &channel.egress {
        if let Some(handle) = &egress.credential_handle {
            require_field(handle, true, "channel egress credential")?;
        }
        for body_credential in &egress.body_credentials {
            require_field(
                &body_credential.handle,
                true,
                "channel egress body credential",
            )?;
        }
    }
    if let Some(template) = channel
        .connection
        .as_ref()
        .and_then(|connection| connection.deep_link_template.as_deref())
    {
        let mut remainder = template;
        while let Some((_, after_open)) = remainder.split_once('{') {
            let Some((placeholder, after_close)) = after_open.split_once('}') else {
                break;
            };
            if placeholder != "code" {
                let handle = ironclaw_host_api::SecretHandle::new(placeholder).map_err(
                    |error| ManifestV3Error::Invalid {
                        reason: format!(
                            "channel connection placeholder `{{{placeholder}}}` is invalid: {error}"
                        ),
                    },
                )?;
                require_field(&handle, false, "channel connection placeholder")?;
            }
            remainder = after_close;
        }
    }
    Ok(())
}

fn runtime_from_raw(raw: RawRuntimeV3) -> ExtensionRuntimeV2 {
    match raw {
        RawRuntimeV3::Wasm { module } => ExtensionRuntimeV2::Wasm { module },
        RawRuntimeV3::FirstParty { service } => ExtensionRuntimeV2::FirstParty { service },
    }
}

/// Authors declare externally meaningful effects; dispatchability is an
/// implementation detail the normalizer adds.
fn with_dispatch_effect(mut effects: Vec<EffectKind>) -> Vec<EffectKind> {
    if !effects.contains(&EffectKind::DispatchCapability) {
        effects.insert(0, EffectKind::DispatchCapability);
    }
    effects
}

/// A network effect implies the host HTTP egress port for sandboxed
/// runtimes (WASM modules and proxied MCP servers); v3 authors never name
/// host ports directly. First-party services receive host services through
/// invocation wiring, not declared ports — their v2 manifests never
/// declared any.
fn derived_host_ports(effects: &[EffectKind], sandboxed_runtime: bool) -> Vec<String> {
    if sandboxed_runtime && effects.contains(&EffectKind::Network) {
        vec![HOST_RUNTIME_HTTP_EGRESS_PORT_ID.to_string()]
    } else {
        Vec::new()
    }
}

#[allow(clippy::too_many_arguments)] // arch-exempt: too_many_args, private normalization helper pending a CredentialContext bundle if it grows, extension-runtime P2
fn credential_from_v3(
    handle: &str,
    vendor: &str,
    scopes: &[String],
    audience: RawAudienceV3,
    injection: RuntimeCredentialTarget,
    required: bool,
    recipes: &BTreeMap<VendorId, VendorAuthRecipe>,
    referenced_vendors: &mut BTreeMap<VendorId, ()>,
) -> Result<RawRuntimeCredentialV2, ManifestV3Error> {
    if audience.host.contains('*') {
        return Err(ManifestV3Error::WildcardAudienceHost {
            host: audience.host,
        });
    }
    let vendor = VendorId::new(vendor)?;
    let Some(recipe) = recipes.get(&vendor) else {
        return Err(ManifestV3Error::MissingAuthRecipe {
            vendor: vendor.as_str().to_string(),
        });
    };
    let setup = match recipe {
        VendorAuthRecipe::Oauth2Code(oauth) => RuntimeCredentialAccountSetup::OAuth {
            scopes: oauth.scopes.clone(),
        },
        VendorAuthRecipe::ApiKey(_) => RuntimeCredentialAccountSetup::ManualToken,
    };
    referenced_vendors.insert(vendor.clone(), ());
    Ok(RawRuntimeCredentialV2 {
        handle: handle.to_string(),
        source: RuntimeCredentialRequirementSource::ProductAuthAccount {
            provider: vendor,
            setup,
        },
        provider_scopes: scopes.to_vec(),
        audience: NetworkTargetPattern {
            scheme: Some(audience.scheme),
            host_pattern: audience.host,
            port: audience.port,
        },
        target: injection,
        required,
    })
}
