//! Hosted MCP registration responses and manifest transformations.
//!
//! This module owns the concrete hosted-MCP representation used by
//! [`super::hosted_mcp_preparation`]. The generic lifecycle manager delegates
//! preparation to that service and does not need to know how a hosted MCP is
//! represented, authenticated, or admitted.

use std::sync::Arc;

use ironclaw_extension_contracts::hosted_mcp::{HostedMcpAuthSelection, HostedMcpEndpoint};
use ironclaw_extensions::{
    ExtensionManifestRecord, ExtensionPackage, ManifestSource, PackageDefinitionRetention,
    PackageRootBinding,
};
use ironclaw_host_api::{
    action::{NetworkPolicy, NetworkScheme, NetworkTargetPattern},
    capability::{
        CapabilityDescriptor, RuntimeCredentialAccountSetup, RuntimeCredentialRequirement,
        RuntimeCredentialRequirementSource,
    },
    http::RuntimeCredentialTarget,
    ids::{ExtensionId, SecretHandle, VendorId},
};
use ironclaw_product_contracts::error::ProductOperationFailure;
use ironclaw_product_contracts::package_lifecycle::{
    LifecyclePackageKind, LifecyclePackageRef, LifecycleProductPayload, LifecycleProductResponse,
};

use crate::{
    AvailableExtensionPackage, HostedMcpDiscoveryError, hosted_mcp_admission,
    product_extension_host_api_contract_registry,
    product_lifecycle::{map_extension_error, map_extension_installation_error},
    surface_kinds_from_manifest_record,
};

pub(crate) fn registration_response(package_ref: LifecyclePackageRef) -> LifecycleProductResponse {
    LifecycleProductResponse {
        package_ref: Some(package_ref),
        phase: ironclaw_extension_contracts::state::InstallationState::Installed,
        blockers: Vec::new(),
        message: Some("Hosted MCP registration accepted.".to_string()),
        payload: Some(LifecycleProductPayload::ExtensionInstall {
            installed: false,
            visible_capability_ids: Vec::new(),
            next_step: "Install this registered extension through the ordinary lifecycle."
                .to_string(),
        }),
    }
}

pub(crate) fn name_unavailable() -> ProductOperationFailure {
    ProductOperationFailure::InvalidBindingRequest {
        reason: "hosted MCP extension name is unavailable".to_string(),
    }
}

pub(crate) fn discovery_error(error: HostedMcpDiscoveryError) -> ProductOperationFailure {
    match error {
        HostedMcpDiscoveryError::Transient(reason) => ProductOperationFailure::Transient {
            reason: format!("hosted MCP catalog preparation failed: {reason}"),
        },
        HostedMcpDiscoveryError::Permanent(reason) => {
            ProductOperationFailure::InvalidBindingRequest {
                reason: format!("hosted MCP catalog preparation failed: {reason}"),
            }
        }
        HostedMcpDiscoveryError::CredentialsRejected(_) => {
            ProductOperationFailure::InvalidBindingRequest {
                reason: "hosted MCP account setup is required".to_string(),
            }
        }
    }
}

pub(crate) fn oauth_admission_error(
    error: ironclaw_auth::AuthProductError,
) -> ProductOperationFailure {
    tracing::debug!(?error, "hosted MCP OAuth metadata admission rejected");
    ProductOperationFailure::InvalidBindingRequest {
        reason: "hosted MCP OAuth metadata was not admissible".to_string(),
    }
}

pub(crate) fn metadata_network_policy(url: &str) -> Result<NetworkPolicy, ProductOperationFailure> {
    let parsed = url::Url::parse(url)
        .map_err(|_| oauth_admission_error(ironclaw_auth::AuthProductError::MalformedConfig))?;
    if parsed.scheme() != "https"
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.host_str().is_none()
        || parsed.fragment().is_some()
    {
        return Err(oauth_admission_error(
            ironclaw_auth::AuthProductError::MalformedConfig,
        ));
    }
    Ok(NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: parsed.host_str().unwrap_or_default().to_ascii_lowercase(),
            port: parsed.port(),
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(64 * 1024),
    })
}

pub(crate) fn manifest_with_admitted_oauth(
    seed: ExtensionManifestRecord,
    endpoint: &hosted_mcp_admission::CanonicalHostedMcpEndpoint,
    admitted: ironclaw_auth::ResolvedVendorAuthRecipe,
) -> Result<ExtensionManifestRecord, ProductOperationFailure> {
    if admitted.token_exchange_resource.as_deref() != Some(endpoint.as_str()) {
        return Err(oauth_admission_error(
            ironclaw_auth::AuthProductError::MalformedConfig,
        ));
    }
    let vendor = VendorId::new(admitted.vendor.clone())
        .map_err(|_| oauth_admission_error(ironclaw_auth::AuthProductError::MalformedConfig))?;
    let scopes = admitted.recipe.scope_ceiling().to_vec();
    let setup = RuntimeCredentialAccountSetup::OAuth {
        scopes: scopes.clone(),
    };
    let parsed_endpoint = url::Url::parse(endpoint.as_str())
        .map_err(|_| oauth_admission_error(ironclaw_auth::AuthProductError::MalformedConfig))?;
    let handle = SecretHandle::new("hosted_mcp_account")
        .map_err(|_| oauth_admission_error(ironclaw_auth::AuthProductError::MalformedConfig))?;
    let requirement = RuntimeCredentialRequirement {
        handle: handle.clone(),
        source: RuntimeCredentialRequirementSource::ProductAuthAccount {
            provider: vendor.clone(),
            setup: setup.clone(),
        },
        provider_scopes: scopes,
        audience: NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: parsed_endpoint
                .host_str()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            port: parsed_endpoint.port(),
        },
        target: RuntimeCredentialTarget::Header {
            name: "authorization".to_string(),
            prefix: Some("Bearer ".to_string()),
        },
        required: true,
    };
    let mut resolved = seed.resolved().clone();
    resolved.auth = vec![ironclaw_extensions::ResolvedAuthSurface {
        vendor,
        setup,
        recipe: Some(admitted.recipe),
        protected_resource_metadata_url: admitted.protected_resource_metadata_url,
    }];
    let mcp = resolved
        .mcp
        .as_mut()
        .ok_or_else(|| oauth_admission_error(ironclaw_auth::AuthProductError::MalformedConfig))?;
    mcp.credential_handles = vec![handle];
    for tool in &mut resolved.tools {
        tool.runtime_credentials = vec![requirement.clone()];
    }
    ExtensionManifestRecord::from_resolved(
        seed.raw_toml(),
        ManifestSource::UserRegistered,
        resolved,
        seed.manifest_hash().cloned(),
    )
    .map(|record| record.with_definition_retention(seed.definition_retention()))
    .map_err(map_extension_installation_error)
}

pub(crate) fn manifest_with_bearer(
    seed: ExtensionManifestRecord,
) -> Result<ExtensionManifestRecord, ProductOperationFailure> {
    let resolved = seed.resolved();
    let server = resolved
        .mcp
        .as_ref()
        .map(|mcp| mcp.server.clone())
        .ok_or_else(name_unavailable)?;
    let endpoint = hosted_mcp_admission::CanonicalHostedMcpEndpoint::parse(
        &HostedMcpEndpoint::new(server).map_err(|_| name_unavailable())?,
    )
    .map_err(|_| name_unavailable())?;
    pending_manifest(
        &resolved.id,
        &resolved.name,
        &endpoint,
        &HostedMcpAuthSelection::Bearer,
    )
}

pub(crate) fn pending_manifest(
    extension_id: &ExtensionId,
    desired_name: &str,
    endpoint: &hosted_mcp_admission::CanonicalHostedMcpEndpoint,
    selection: &HostedMcpAuthSelection,
) -> Result<ExtensionManifestRecord, ProductOperationFailure> {
    if desired_name.trim().is_empty() || desired_name.len() > 256 {
        return Err(ProductOperationFailure::InvalidBindingRequest {
            reason: "hosted MCP extension name is invalid".to_string(),
        });
    }
    if let HostedMcpAuthSelection::OAuth {
        client_profile_id: Some(profile),
    } = selection
        && (profile.trim().is_empty()
            || profile.len() > 128
            || profile.chars().any(char::is_control))
    {
        return Err(ProductOperationFailure::InvalidBindingRequest {
            reason: "hosted MCP OAuth client profile is invalid".to_string(),
        });
    }
    let quoted_name = toml::Value::String(desired_name.trim().to_string()).to_string();
    let quoted_endpoint = toml::Value::String(endpoint.as_str().to_string()).to_string();
    let id = extension_id.as_str();
    let auth = match selection {
        HostedMcpAuthSelection::Auto | HostedMcpAuthSelection::NoAuth => String::new(),
        HostedMcpAuthSelection::Bearer => {
            let vendor = hosted_mcp_admission::hosted_mcp_vendor_id(endpoint)
                .map_err(|_| name_unavailable())?;
            format!(
                r#"
[[mcp.credentials]]
handle = "hosted_mcp_account"
vendor = "{}"
injection = {{ type = "header", name = "authorization", prefix = "Bearer " }}

[auth.{}]
method = "api_key"
display_name = "Hosted MCP bearer token"
fields = [{{ handle = "hosted_mcp_account", label = "Bearer token", secret = true }}]
"#,
                vendor.as_str(),
                vendor.as_str()
            )
        }
        HostedMcpAuthSelection::OAuth { .. } => String::new(),
    };
    let raw = format!(
        r#"schema_version = "reborn.extension_manifest.v3"
id = "{id}"
name = {quoted_name}
version = "0.1.0"
description = "User-registered hosted MCP server"
trust = "third_party"

[mcp]
origin_gate_matrix = {{ loop_run = "gated_unless_granted", product = "forbidden", automation = "forbidden" }}
server = {quoted_endpoint}
namespace = "{id}"
max_tools = 1024
default_permission = "ask"
effects = ["network", "use_secret"]
{auth}"#
    );
    let manifest_hash = ironclaw_extensions::ManifestHash::new(
        ironclaw_host_api::approval::sha256_digest_token(raw.as_bytes()),
    )
    .map_err(map_extension_installation_error)?;
    let parsed = ExtensionManifestRecord::from_toml_with_root_binding(
        raw.clone(),
        ManifestSource::UserRegistered,
        &ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
            ProductOperationFailure::InvalidBindingRequest {
                reason: format!("host port catalog rejected hosted MCP registration: {error}"),
            }
        })?,
        Some(manifest_hash.clone()),
        &product_extension_host_api_contract_registry().map_err(|error| {
            ProductOperationFailure::InvalidBindingRequest {
                reason: format!("host API contracts rejected hosted MCP registration: {error}"),
            }
        })?,
        PackageRootBinding::Virtual,
    )
    .map_err(map_extension_installation_error)?;
    let mut resolved = parsed.resolved().clone();
    resolved.root_binding = PackageRootBinding::Virtual;
    if let Some(mcp) = resolved.mcp.as_mut() {
        mcp.registration_auth = selection.clone();
    }
    ExtensionManifestRecord::from_resolved(
        raw,
        ManifestSource::UserRegistered,
        resolved,
        Some(manifest_hash),
    )
    // The seed declares no model-visible capability — those arrive from
    // discovery — so "not resolved yet" is already readable from the package
    // itself and needs no stored flag alongside it.
    .map(|record| record.with_definition_retention(PackageDefinitionRetention::RetainInCatalog))
    .map_err(map_extension_installation_error)
}

pub(crate) fn available_package(
    record: &ExtensionManifestRecord,
) -> Result<AvailableExtensionPackage, ProductOperationFailure> {
    let id = record.resolved().id.as_str();
    let manifest: ironclaw_extensions::ExtensionManifest =
        record.manifest().clone().try_into().map_err(|error| {
            ProductOperationFailure::InvalidBindingRequest {
                reason: format!("hosted MCP package manifest is invalid: {error}"),
            }
        })?;
    let schemas = record
        .resolved()
        .mcp
        .as_ref()
        .map(|mcp| &mcp.dynamic_input_schemas);
    let capabilities = manifest
        .capabilities
        .iter()
        .map(|capability| CapabilityDescriptor {
            id: capability.id.clone(),
            provider: manifest.id.clone(),
            runtime: manifest.runtime.kind(),
            trust_ceiling: manifest.descriptor_trust_default,
            description: capability.description.clone(),
            parameters_schema: schemas
                .and_then(|schemas| schemas.get(capability.id.as_str()))
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            effects: capability.effects.clone(),
            default_permission: capability.default_permission,
            runtime_credentials: capability.runtime_credentials.clone(),
            network_targets: capability.network_targets.clone(),
            max_egress_bytes: capability.max_egress_bytes,
            resource_profile: capability.resource_profile.clone(),
            origin_gate_matrix: capability.origin_gate_matrix.clone(),
        })
        .collect();
    let package = ExtensionPackage::from_virtual_manifest(
        manifest,
        Some(ironclaw_host_api::approval::sha256_digest_token(
            record.raw_toml().as_bytes(),
        )),
        capabilities,
    )
    .map_err(map_extension_error)?;
    Ok(AvailableExtensionPackage {
        package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, id)?,
        manifest_toml: record.raw_toml().to_string(),
        resolved_manifest: Arc::new(record.resolved().clone()),
        source: ManifestSource::UserRegistered,
        package,
        cleanup_requirements: Vec::new(),
        surface_kinds: surface_kinds_from_manifest_record(record, id)?,
        channel_directions: None,
        channel_presentation: None,
        assets: Vec::new(),
        onboarding_override: None,
        oauth_setup_override: None,
        search_aliases: Vec::new(),
    })
}
