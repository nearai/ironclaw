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
    action::{NetworkScheme, NetworkTargetPattern},
    capability::{
        EffectKind, OriginGateMatrix, PermissionMode, RuntimeCredentialAccountSetup,
        RuntimeCredentialRequirementSource,
    },
    error::HostApiError,
    host_port::{HOST_RUNTIME_HTTP_EGRESS_PORT_ID, HostPortCatalog},
    http::RuntimeCredentialTarget,
    ids::{ExtensionId, VendorId},
    messaging::StandardMessagingOp,
    trust::RequestedTrustClass,
};

use ironclaw_extension_contracts::{
    channel::{ChannelDescriptor, ChannelDescriptorError},
    memory::{MemoryDescriptor, MemoryScheduledOpKind},
    recipe::{RecipeValidationError, VendorAuthRecipe},
};
use serde::Deserialize;
use thiserror::Error;

use crate::ExtensionAdminConfigurationDescriptor;
use crate::resolved::{
    PackageRootBinding, ResolvedAuthSurface, ResolvedExtensionManifest, ResolvedMcpDeclaration,
};
use crate::v2::{
    CapabilityDeclV2, CapabilitySurfaceDeclV2, ExtensionManifestV2, ExtensionRuntimeV2,
    MAX_MANIFEST_BYTES, ManifestSource, RawCapabilityV2, RawRuntimeCredentialV2,
    requested_trust_to_descriptor_trust,
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
    #[error("[memory] is invalid: {reason}")]
    InvalidMemory { reason: String },
    #[error(
        "credential vendor `{vendor}` has no [auth.{vendor}] recipe; v3 manifests must declare \
         one for every referenced vendor"
    )]
    MissingAuthRecipe { vendor: String },
    #[error("[auth.{vendor}] recipe is not referenced by any credential")]
    UnreferencedAuthRecipe { vendor: String },
    #[error(
        "manifest declares more than one `method = \"device_link\"` auth surface \
         ([auth.{first}] and [auth.{second}]); an extension links at most one account"
    )]
    MultipleDeviceLinkRecipes { first: String, second: String },
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
    memory: Option<MemoryDescriptor>,
    #[serde(default)]
    auth: BTreeMap<String, VendorAuthRecipe>,
    #[serde(default)]
    admin_configuration: Option<ExtensionAdminConfigurationDescriptor>,
    /// Free-form authoring metadata, ignored (same exemption as v2's
    /// `metadata` root).
    #[serde(default)]
    #[serde(rename = "metadata")]
    _metadata: Option<toml::Value>,
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
    /// The standard messaging operation this tool binds to (standardized
    /// messaging framework spec §6). `#[serde(default)]` so a bespoke tool
    /// omits the field entirely; validated and consumed in the per-tool loop
    /// in [`parse_v3`], never threaded through unchecked.
    #[serde(default)]
    standard_op: Option<StandardMessagingOp>,
    /// Required for a bespoke tool; must be omitted for a `standard_op`
    /// binding, which uses the host-synthesized
    /// `standard:messaging/<op>.input.v1` ref instead. `#[serde(default)]`
    /// so a bound entry can omit it — the per-tool loop enforces the inverse
    /// rule (non-bound tools must declare one) explicitly.
    #[serde(default)]
    input_schema_ref: Option<String>,
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
    network_targets: Vec<ironclaw_host_api::action::NetworkTargetPattern>,
    /// Optional per-tool egress cap (bytes). `#[serde(default)]` so tools
    /// without the key parse to `None` (no cap). Threads to the capability's
    /// `NetworkPolicy.max_egress_bytes` at grant issuance.
    #[serde(default)]
    max_egress_bytes: Option<u64>,
    #[serde(default)]
    resource_profile: Option<ironclaw_host_api::resource::ResourceProfile>,
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
    #[serde(default)]
    placeholder_env: Option<String>,
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
    server: ironclaw_extension_contracts::recipe::HttpsEndpoint,
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
    // Both reserved-prefix rules (`ironclaw.` = host-bundled only, `mcp-` =
    // user-registered only) are enforced by one shared table
    // (`crate::v2::check_reserved_id_prefix`) so v2's `from_raw` and this
    // parser can never drift apart on the reservation.
    if let Err(violation) = crate::v2::check_reserved_id_prefix(&id, source) {
        return Err(ManifestV3Error::Invalid {
            reason: format!(
                "extension id `{}` uses the reserved `{}` prefix, which is {}",
                violation.id, violation.prefix, violation.permitted_description
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
    // A `[memory]` surface declares the extension a backend for the host memory
    // adapter. It is host-bundled + first_party only (the host owns the
    // compose-time provider binding). The manifest's `[[tools]]` array is the
    // provider's tool surface and `lifecycle` — any subset, including empty —
    // declares the host-initiated hooks it participates in, so no further
    // shape constraint applies here.
    if raw.memory.is_some() && !matches!(runtime, ExtensionRuntimeV2::FirstParty { .. }) {
        return Err(ManifestV3Error::InvalidMemory {
            reason: "[memory] requires a first_party runtime".to_string(),
        });
    }
    if let Some(memory) = &raw.memory {
        memory
            .validate()
            .map_err(|error| ManifestV3Error::InvalidMemory {
                reason: error.to_string(),
            })?;
        validate_memory_scheduled_ops(memory, raw.trust, &raw.tools)?;
    }
    let sandboxed_runtime = matches!(
        runtime,
        ExtensionRuntimeV2::Wasm { .. } | ExtensionRuntimeV2::Mcp { .. }
    );

    // Validate recipes.
    let mut recipes: BTreeMap<VendorId, VendorAuthRecipe> = BTreeMap::new();
    // At most one device-link surface per extension, structurally, the way
    // `[channel]` is at most one. Nothing downstream can express two: the
    // binding slot holds a single adapter, and the host resolves the flow's
    // vendor by scanning the surfaces and taking the first device-link match —
    // so a second one would not be rejected, it would be silently ignored, and
    // flows, minted accounts, and custody grants would all attribute to
    // whichever vendor happened to sort first.
    let mut device_link_vendor: Option<String> = None;
    for (vendor, recipe) in raw.auth {
        recipe
            .validate()
            .map_err(|error| ManifestV3Error::InvalidRecipe {
                vendor: vendor.clone(),
                error,
            })?;
        if matches!(recipe, VendorAuthRecipe::DeviceLink(_)) {
            if let Some(first) = &device_link_vendor {
                return Err(ManifestV3Error::MultipleDeviceLinkRecipes {
                    first: first.clone(),
                    second: vendor.clone(),
                });
            }
            device_link_vendor = Some(vendor.clone());
        }
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
    // A `[memory]`-declaring manifest (host-bundled only — checked above) may
    // declare tools under the reserved stable memory namespace, with its
    // requested gating clamped below.
    let memory_tool_namespace = raw
        .memory
        .is_some()
        .then_some(ironclaw_extension_contracts::memory::MEMORY_TOOL_ID_NAMESPACE);
    let mut referenced_vendors: BTreeMap<VendorId, ()> = BTreeMap::new();
    let mut capabilities = Vec::new();
    let mut mcp_template_credentials = None;
    if let Some(mcp) = &mcp {
        let template_credentials = mcp
            .credentials
            .iter()
            .map(|credential| {
                credential_from_v3(
                    CredentialProjection {
                        handle: &credential.handle,
                        vendor: &credential.vendor,
                        scopes: &credential.scopes,
                        audience: RawAudienceV3 {
                            scheme: NetworkScheme::Https,
                            host: mcp.server.host(),
                            port: None,
                        },
                        injection: credential.injection.clone(),
                        placeholder_env: None,
                        required: true,
                    },
                    &recipes,
                    &mut referenced_vendors,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let raw_capability = RawCapabilityV2 {
            id: format!("{id}.mcp_server"),
            network_targets: Vec::new(),
            max_egress_bytes: None,
            description: format!(
                "Hosted MCP server connection for {} (discovery template; never model-visible)",
                raw.name
            ),
            effects: with_dispatch_effect(mcp.effects.clone()),
            default_permission: mcp.default_permission,
            visibility: crate::v2::CapabilityVisibility::HostInternal,
            standard_op: None,
            input_schema_ref: format!("schemas/{id}/dynamic/mcp_server.input.v1.json"),
            output_schema_ref: None,
            prompt_doc_ref: None,
            required_host_ports: derived_host_ports(&mcp.effects, true),
            runtime_credentials: template_credentials.clone(),
            resource_profile: None,
            origin_gate_matrix: mcp.origin_gate_matrix.clone(),
        };
        capabilities.push(
            CapabilityDeclV2::from_raw(raw_capability, &id, host_port_catalog, None).map_err(
                |error| ManifestV3Error::Invalid {
                    reason: error.to_string(),
                },
            )?,
        );
        mcp_template_credentials = Some(template_credentials);
    }
    let mut seen_standard_ops: std::collections::HashSet<StandardMessagingOp> = Default::default();
    for tool in raw.tools {
        // A `standard_op` binding claims host-owned canonical vocabulary
        // (standardized messaging framework spec §6): the operation must
        // have graduated a contract, the tool id must be the
        // extension-namespaced op name, the schema refs are host-synthesized
        // (never author-declared), a write op must declare `external_write`,
        // an extension may bind an op at most once, and — because an `[mcp]`
        // manifest's static tools inherit the server connection template
        // wholesale (including a hardcoded `output_schema_ref: None` below)
        // — a `standard_op` cannot combine with `[mcp]` at all.
        if let Some(op) = tool.standard_op {
            if mcp.is_some() {
                return Err(ManifestV3Error::Invalid {
                    reason: format!(
                        "tool `{}` declares standard_op on an [mcp] manifest; static tools \
                         inherit the server connection template and cannot bind a standard op",
                        tool.id
                    ),
                });
            }
            if op.contract().is_none() {
                return Err(ManifestV3Error::Invalid {
                    reason: format!(
                        "standard_op `{}` is reserved and not yet bindable",
                        op.op_name()
                    ),
                });
            }
            let expected_id = format!("{}.{}", id.as_str(), op.op_name());
            if tool.id != expected_id {
                return Err(ManifestV3Error::Invalid {
                    reason: format!(
                        "standard op tool id must be `{expected_id}`, got `{}`",
                        tool.id
                    ),
                });
            }
            if tool.input_schema_ref.is_some() || tool.output_schema_ref.is_some() {
                return Err(ManifestV3Error::Invalid {
                    reason: format!(
                        "standard op `{}` uses host-canonical schemas; remove \
                         input_schema_ref/output_schema_ref",
                        tool.id
                    ),
                });
            }
            if op.is_write() && !tool.effects.contains(&EffectKind::ExternalWrite) {
                return Err(ManifestV3Error::Invalid {
                    reason: format!(
                        "standard op `{}` is a write operation and must declare the \
                         external_write effect",
                        tool.id
                    ),
                });
            }
            if !seen_standard_ops.insert(op) {
                return Err(ManifestV3Error::Invalid {
                    reason: format!(
                        "standard op `{}` may be bound at most once per extension",
                        op.op_name()
                    ),
                });
            }
        }
        // A bound tool's schemas are the host-synthesized canonical refs; a
        // bespoke tool must declare its own — the inverse of the check
        // above, enforced here so both arms below get a resolved `String`
        // even though `RawToolV3::input_schema_ref` is optional on the wire.
        let (input_schema_ref, output_schema_ref) = match tool.standard_op {
            Some(op) => (
                ironclaw_host_api::capability_profile::CapabilityProfileSchemaRef::standard_messaging_input(op)
                    .map_err(|error| ManifestV3Error::Invalid { reason: error.to_string() })?
                    .into_string(),
                Some(
                    ironclaw_host_api::capability_profile::CapabilityProfileSchemaRef::standard_messaging_output(op)
                        .map_err(|error| ManifestV3Error::Invalid { reason: error.to_string() })?
                        .into_string(),
                ),
            ),
            None => {
                let input_schema_ref =
                    tool.input_schema_ref
                        .ok_or_else(|| ManifestV3Error::Invalid {
                            reason: format!("tool {} requires input_schema_ref", tool.id),
                        })?;
                (input_schema_ref, tool.output_schema_ref.clone())
            }
        };

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
                    description: tool.description,
                    effects: with_dispatch_effect(mcp.effects.clone()),
                    default_permission: tool.default_permission,
                    visibility: tool.visibility,
                    // The guard above rejects standard operations on MCP
                    // manifests; keep this inherited template explicitly
                    // incapable of acquiring a standard binding.
                    standard_op: None,
                    input_schema_ref,
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
                description: tool.description,
                effects: with_dispatch_effect(tool.effects.clone()),
                default_permission: tool.default_permission,
                visibility: tool.visibility,
                standard_op: tool.standard_op,
                input_schema_ref,
                output_schema_ref,
                prompt_doc_ref: tool.prompt_doc_ref,
                required_host_ports: derived_host_ports(&tool.effects, sandboxed_runtime),
                runtime_credentials: tool
                    .credentials
                    .into_iter()
                    .map(|credential| {
                        credential_from_v3(
                            CredentialProjection {
                                handle: &credential.handle,
                                vendor: &credential.vendor,
                                scopes: &credential.scopes,
                                audience: credential.audience,
                                injection: credential.injection,
                                placeholder_env: credential.placeholder_env,
                                required: credential.required,
                            },
                            &recipes,
                            &mut referenced_vendors,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                resource_profile: tool.resource_profile,
                origin_gate_matrix: tool.origin_gate_matrix,
            },
        };
        // Requested, not granted: a memory provider's declared tool gating is
        // clamped by the host — `ungated` survives only where the reviewed
        // allowlist grants it, exactly as host trust policy already clamps
        // `trust = "first_party_requested"`.
        let mut raw_capability = raw_capability;
        if memory_tool_namespace.is_some()
            && let Some(matrix) = raw_capability.origin_gate_matrix.take()
        {
            raw_capability.origin_gate_matrix =
                Some(matrix.clamp_requested_for_memory_tool(&raw_capability.id));
        }
        capabilities.push(
            CapabilityDeclV2::from_raw(
                raw_capability,
                &id,
                host_port_catalog,
                memory_tool_namespace,
            )
            .map_err(|error| ManifestV3Error::Invalid {
                reason: error.to_string(),
            })?,
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
            let setup = account_setup_for_recipe(&recipe);
            ResolvedAuthSurface {
                vendor,
                setup,
                recipe: Some(recipe),
                oauth_resource: None,
                protected_resource_metadata_url: None,
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
        // Parsing precedes package materialization. The manifest-record
        // boundary replaces this for materialized and remote-only packages.
        root_binding: PackageRootBinding::FabricateOnLoad,
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
            dynamic_input_schemas: std::collections::BTreeMap::new(),
            registration_auth:
                ironclaw_extension_contracts::hosted_mcp::HostedMcpAuthSelection::NoAuth,
        }),
        tools: manifest.capabilities.clone(),
        channel: raw.channel,
        memory: raw.memory,
        admin_configuration: raw.admin_configuration.into_iter().collect(),
        auth,
        host_apis: Vec::new(),
        section_surfaces: Vec::new(),
        hooks: Vec::new(),
    };

    Ok((manifest, resolved))
}

/// Validate `[memory].scheduled_ops` against the rest of the manifest.
///
/// The neutral memory contract validates one op's own shape (trigger
/// vocabulary, op-kind tagging, cost floor, model-call ceiling, one op per
/// trigger). The two rules here need the manifest-wide view, which is why they
/// live at this layer — the same reason `[admin_configuration]`'s channel
/// cross-check does:
///
/// - **Trust.** A `pass` op is a manifest-authored prompt executing with write
///   tools, as every user, on a schedule. That is a strictly larger grant than
///   a tool call the model chose, so only a manifest whose requested trust is
///   first-party/system may declare one — the same default-deny wall the
///   after-turn hook tiers apply, for the same reason. Requested trust is the
///   right input here because the source gate above has already refused a
///   first-party/system request from any non-host-bundled source; host trust
///   policy still computes effective trust downstream.
/// - **Tool subset.** A pass's `tools` must be ids the SAME manifest declares.
///   Declaration is selection, never authority: a memory provider must not be
///   able to schedule a pass wielding another extension's tools, and only this
///   layer can see the `[[tools]]` array beside the `[memory]` section.
fn validate_memory_scheduled_ops(
    memory: &MemoryDescriptor,
    trust: RequestedTrustClass,
    tools: &[RawToolV3],
) -> Result<(), ManifestV3Error> {
    for scheduled in &memory.scheduled_ops {
        match &scheduled.op {
            MemoryScheduledOpKind::Pass(pass) => {
                if !matches!(
                    trust,
                    RequestedTrustClass::FirstPartyRequested | RequestedTrustClass::SystemRequested
                ) {
                    return Err(ManifestV3Error::InvalidMemory {
                        reason: format!(
                            "a `pass` scheduled op runs a manifest-authored prompt with the \
                             declared tools on a schedule, so it is restricted to first-party \
                             manifests; trust `{trust:?}` may not declare one"
                        ),
                    });
                }
                for tool_id in &pass.tools {
                    if !tools.iter().any(|tool| tool.id == tool_id.as_str()) {
                        return Err(ManifestV3Error::InvalidMemory {
                            reason: format!(
                                "scheduled pass op for trigger `{}` selects tool `{tool_id}`, \
                                 which this manifest does not declare in [[tools]]",
                                scheduled.trigger.as_str()
                            ),
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

/// Validate every channel runtime reference against the one manifest-owned
/// deployment configuration schema. The neutral channel contract validates
/// channel structure; the extension-manifest layer owns this cross-section
/// relationship because only it can see `[admin_configuration]`.
fn validate_channel_admin_configuration(
    channel: &ChannelDescriptor,
    descriptor: Option<&ExtensionAdminConfigurationDescriptor>,
) -> Result<(), ManifestV3Error> {
    let require_field = |handle: &ironclaw_host_api::ids::SecretHandle,
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
                let handle = ironclaw_host_api::ids::SecretHandle::new(placeholder).map_err(
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

/// Project a declared auth recipe onto the credential-account setup a runtime
/// credential requirement carries.
///
/// One function, two call sites (the per-credential requirement and the
/// per-vendor resolved auth surface) precisely because they must never
/// disagree: a vendor whose surface says `oauth` while its credentials say
/// `manual_token` produces a connect affordance that services a challenge the
/// engine cannot mint.
fn account_setup_for_recipe(recipe: &VendorAuthRecipe) -> RuntimeCredentialAccountSetup {
    match recipe {
        VendorAuthRecipe::Oauth2Code(oauth) => RuntimeCredentialAccountSetup::OAuth {
            scopes: oauth.scopes.clone(),
        },
        VendorAuthRecipe::ApiKey(_) => RuntimeCredentialAccountSetup::ManualToken,
        // Display metadata only — the recipe declares no scopes and no
        // endpoints, so there is nothing to carry onto the setup. Satisfaction
        // is the stored session the device link produces.
        VendorAuthRecipe::DeviceLink(_) => RuntimeCredentialAccountSetup::DeviceLink,
    }
}

struct CredentialProjection<'a> {
    handle: &'a str,
    vendor: &'a str,
    scopes: &'a [String],
    audience: RawAudienceV3,
    injection: RuntimeCredentialTarget,
    placeholder_env: Option<String>,
    required: bool,
}

fn credential_from_v3(
    credential: CredentialProjection<'_>,
    recipes: &BTreeMap<VendorId, VendorAuthRecipe>,
    referenced_vendors: &mut BTreeMap<VendorId, ()>,
) -> Result<RawRuntimeCredentialV2, ManifestV3Error> {
    let CredentialProjection {
        handle,
        vendor,
        scopes,
        audience,
        injection,
        placeholder_env,
        required,
    } = credential;
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
    let setup = account_setup_for_recipe(recipe);
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
        placeholder_env,
        required,
    })
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::host_port::{HostPortCatalog, HostPortCatalogEntry, HostPortId};

    use super::*;

    fn catalog() -> HostPortCatalog {
        HostPortCatalog::new(vec![HostPortCatalogEntry::new(
            HostPortId::new(HOST_RUNTIME_HTTP_EGRESS_PORT_ID).expect("host port id"),
        )])
        .expect("host port catalog")
    }

    /// Minimal `[mcp]` manifest with a parameterized id/namespace, for
    /// exercising the reserved `mcp-` id namespace directly through
    /// `parse_v3` — the second of the two import paths the single arm must
    /// cover (the other is `ExtensionManifestRecord::from_toml`, exercised
    /// in `tests/manifest_v3_contract.rs`).
    fn mcp_manifest_with_id(id: &str) -> String {
        format!(
            r#"
schema_version = "{MANIFEST_SCHEMA_VERSION_V3}"
id = "{id}"
name = "Zeta"
version = "0.1.0"
description = "Hosted MCP fixture"
trust = "third_party"

[mcp]
server = "https://mcp.zeta.example/mcp"
namespace = "{id}"
max_tools = 64
default_permission = "ask"
effects = ["network", "use_secret"]
"#
        )
    }

    /// Minimal host-bundled `[memory]` provider manifest with a parameterized
    /// trust class and `[[memory.scheduled_ops]]` body. Both cross-section
    /// rules need the whole file, so they can only be exercised here.
    fn memory_manifest(trust: &str, scheduled_ops: &str) -> String {
        format!(
            r#"
schema_version = "{MANIFEST_SCHEMA_VERSION_V3}"
id = "ironclaw.memory"
name = "Memory"
version = "0.1.0"
description = "Memory provider fixture"
trust = "{trust}"

[runtime]
kind = "first_party"
service = "native_memory_provider"

[memory]
lifecycle = ["record_interaction"]
{scheduled_ops}

[[tools]]
id = "ironclaw.memory.read"
description = "Read a memory document."
effects = ["read_filesystem"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/memory/document-read.input.v1.json"
"#
        )
    }

    const DECLARED_TOOL_PASS_OP: &str = r#"
[[memory.scheduled_ops]]
trigger = "after_turn"
interval_turns = 10
pass = { prompt = "prompts/memory_curation.md", tools = ["ironclaw.memory.read"], max_model_calls = 10 }
"#;

    #[test]
    fn memory_pass_op_parses_when_first_party_and_selecting_a_declared_tool() {
        let (_manifest, resolved) = parse_v3(
            &memory_manifest("first_party_requested", DECLARED_TOOL_PASS_OP),
            ManifestSource::HostBundled,
            &catalog(),
        )
        .expect("a first-party manifest may schedule a pass over its own tools");
        let memory = resolved.memory.expect("[memory] survives parsing");
        assert_eq!(memory.scheduled_ops.len(), 1);
    }

    /// A pass runs a manifest-authored prompt with write tools, as every user,
    /// on a schedule — a strictly larger grant than a model-chosen tool call.
    /// Host-bundled alone is not enough; the manifest must request first-party
    /// trust, the same default-deny wall the after-turn hook tiers apply.
    #[test]
    fn memory_pass_op_is_refused_without_first_party_trust() {
        let error = parse_v3(
            &memory_manifest("third_party", DECLARED_TOOL_PASS_OP),
            ManifestSource::HostBundled,
            &catalog(),
        )
        .expect_err("a third-party manifest must not declare a pass op");
        assert!(
            error.to_string().contains("first-party"),
            "error must name the trust wall: {error}"
        );
    }

    /// Declaration is selection, never authority: a memory provider must not be
    /// able to schedule a pass wielding tools some other extension declares.
    #[test]
    fn memory_pass_op_is_refused_when_a_selected_tool_is_not_declared_here() {
        let error = parse_v3(
            &memory_manifest(
                "first_party_requested",
                r#"
[[memory.scheduled_ops]]
trigger = "after_turn"
interval_turns = 10
pass = { prompt = "prompts/memory_curation.md", tools = ["ironclaw.memory.read", "other.extension.write"], max_model_calls = 10 }
"#,
            ),
            ManifestSource::HostBundled,
            &catalog(),
        )
        .expect_err("an undeclared tool must not be selectable");
        assert!(
            error.to_string().contains("other.extension.write"),
            "error must name the undeclared tool: {error}"
        );
    }

    /// The section-local rules reach manifest parsing too, not just a direct
    /// descriptor parse: two ops on one trigger have no well-defined cadence.
    #[test]
    fn memory_duplicate_scheduled_trigger_is_refused_at_manifest_parse() {
        let error = parse_v3(
            &memory_manifest(
                "first_party_requested",
                r#"
[[memory.scheduled_ops]]
trigger = "after_turn"
interval_turns = 10
pass = { prompt = "prompts/a.md", tools = [], max_model_calls = 4 }

[[memory.scheduled_ops]]
trigger = "after_turn"
interval_turns = 20
pass = { prompt = "prompts/b.md", tools = [], max_model_calls = 4 }
"#,
            ),
            ManifestSource::HostBundled,
            &catalog(),
        )
        .expect_err("two ops on one trigger must be refused");
        assert!(
            error.to_string().contains("after_turn"),
            "error must name the duplicated trigger: {error}"
        );
    }

    /// A manifest written before `scheduled_ops` existed must keep parsing.
    #[test]
    fn memory_manifest_without_scheduled_ops_still_parses() {
        let (_manifest, resolved) = parse_v3(
            &memory_manifest("first_party_requested", ""),
            ManifestSource::HostBundled,
            &catalog(),
        )
        .expect("a pre-scheduled_ops memory manifest parses unchanged");
        let memory = resolved.memory.expect("[memory] survives parsing");
        assert!(memory.scheduled_ops.is_empty());
    }

    #[test]
    fn reserved_mcp_namespace_rejects_non_user_registered_source_via_direct_parse_v3() {
        let catalog = catalog();
        for source in [
            ManifestSource::HostBundled,
            ManifestSource::InstalledLocal,
            ManifestSource::RegistryInstalled,
        ] {
            let error = parse_v3(&mcp_manifest_with_id("mcp-foo"), source, &catalog)
                .expect_err("non-user-registered source must not claim the mcp- namespace");
            assert!(
                error.to_string().contains("mcp-"),
                "error should name the reserved prefix, got: {error}"
            );
        }
    }

    #[test]
    fn reserved_mcp_namespace_allows_user_registered_source_via_direct_parse_v3() {
        let catalog = catalog();
        let (manifest, _resolved) = parse_v3(
            &mcp_manifest_with_id("mcp-foo"),
            ManifestSource::UserRegistered,
            &catalog,
        )
        .expect("user-registered source may declare an mcp- id");
        assert_eq!(manifest.id.as_str(), "mcp-foo");
    }
}
