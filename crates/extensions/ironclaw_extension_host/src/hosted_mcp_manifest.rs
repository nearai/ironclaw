//! Hosted MCP registration responses and manifest transformations.
//!
//! This module owns the concrete hosted-MCP representation used by
//! [`super::hosted_mcp_preparation`]. The generic lifecycle manager delegates
//! preparation to that service and does not need to know how a hosted MCP is
//! represented, authenticated, or admitted.

use std::sync::Arc;

use ironclaw_extension_contracts::hosted_mcp::HostedMcpAuthSelection;
use ironclaw_extension_registry::{
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

/// Preserve a reconstruction failure for operators without logging the raw
/// endpoint carried by `HostApiError`.
pub(crate) fn endpoint_input_error(
    error: ironclaw_host_api::error::HostApiError,
) -> ProductOperationFailure {
    let error_kind = match error {
        ironclaw_host_api::error::HostApiError::InvalidId { kind, .. } => kind,
        _ => "host_api_validation",
    };
    tracing::debug!(error_kind, "hosted MCP endpoint reconstruction rejected");
    name_unavailable()
}

/// `HostedMcpAdmissionError` is a closed, endpoint-free enum, so its variant
/// can safely retain the canonicalization or vendor-ID failure in diagnostics.
pub(crate) fn endpoint_admission_error(
    error: hosted_mcp_admission::HostedMcpAdmissionError,
) -> ProductOperationFailure {
    tracing::debug!(?error, "hosted MCP endpoint reconstruction rejected");
    name_unavailable()
}

/// Prefix every hosted-MCP discovery failure carries, emitted by
/// [`discovery_error`] below. The post-install classifiers key on it, so it is
/// a **cross-module contract**, not an incidental message.
pub(crate) const HOSTED_MCP_PREPARATION_FAILURE_PREFIX: &str =
    "hosted MCP catalog preparation failed:";

/// The one discovery outcome that is not a preparation failure at all: the
/// server answered, but published nothing callable. Produced by
/// `product_lifecycle::generic_host_error` wrapping
/// `entrypoint.rs`'s `HostedMcpEntrypointError`.
pub(crate) const HOSTED_MCP_NO_CALLABLE_TOOLS_REASON: &str = concat!(
    "generic extension host rejected the activation: ",
    "hosted MCP discovery published no callable tools"
);

/// Whether a post-install `InvalidBindingRequest` reason means "hosted-MCP
/// discovery did not work out" — in which case the extension is still
/// **installed** and the caller is told so, rather than the whole install being
/// reported as a failure.
///
/// Lives here, beside the producer that emits the prefix, because it is the one
/// genuinely shared *decision* between the two post-install classifiers
/// (`lifecycle_product_service::install_activation_error` and
/// `extension_lifecycle_capabilities::install_activation_error`). Those two
/// classifiers are deliberately **not** merged — they have different return
/// types and their `Err` arms encode different policies, one preserving the
/// remediation text for the product surface and one collapsing it to a safe
/// summary for a model-facing capability. What they must agree on is exactly
/// this predicate: *which* failures still leave a usable install. It was
/// duplicated as two inline string comparisons string-coupled to a producer in
/// a third module, which is the shape that drifts silently. Raised by
/// CodeRabbit on #7000.
pub fn hosted_mcp_discovery_left_the_install_usable(reason: &str) -> bool {
    reason.starts_with(HOSTED_MCP_PREPARATION_FAILURE_PREFIX)
        || reason == HOSTED_MCP_NO_CALLABLE_TOOLS_REASON
}

pub(crate) fn discovery_error(error: HostedMcpDiscoveryError) -> ProductOperationFailure {
    match error {
        HostedMcpDiscoveryError::Transient(reason) => ProductOperationFailure::Transient {
            reason: format!("{HOSTED_MCP_PREPARATION_FAILURE_PREFIX} {reason}"),
        },
        HostedMcpDiscoveryError::Permanent(reason) => {
            ProductOperationFailure::InvalidBindingRequest {
                reason: format!("{HOSTED_MCP_PREPARATION_FAILURE_PREFIX} {reason}"),
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
    resolved.auth = vec![ironclaw_extension_registry::ResolvedAuthSurface {
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
                .map_err(endpoint_admission_error)?;
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
    let manifest_hash = ironclaw_extension_registry::ManifestHash::new(
        ironclaw_host_api::approval::sha256_digest_token(raw.as_bytes()),
    )
    .map_err(map_extension_installation_error)?;
    let parsed = ExtensionManifestRecord::from_toml_with_root_binding(
        raw.clone(),
        ManifestSource::UserRegistered,
        &ironclaw_host_api::host_port::default_host_port_catalog().map_err(|error| {
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
    let manifest: ironclaw_extension_registry::ExtensionManifest =
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
            standard_op: capability.standard_op,
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

#[cfg(test)]
mod tests {
    use super::{
        discovery_error, endpoint_admission_error, endpoint_input_error, oauth_admission_error,
    };
    use crate::HostedMcpDiscoveryError;
    use ironclaw_product_contracts::error::ProductOperationFailure;

    /// Hosted-MCP discovery talks to a third-party server, so its three
    /// outcomes must stay distinct: a blip the caller should retry, a server
    /// that will never work as configured, and a server asking the user to
    /// connect an account. `install_activation_error` keys on the exact
    /// "hosted MCP catalog preparation failed:" prefix to decide whether a
    /// post-install discovery failure still leaves a usable install, so the
    /// prefix is asserted, not just the variant.
    #[test]
    fn hosted_mcp_discovery_outcomes_stay_distinguishable() {
        assert_eq!(
            discovery_error(HostedMcpDiscoveryError::Transient(
                "upstream 503".to_string()
            )),
            ProductOperationFailure::Transient {
                reason: "hosted MCP catalog preparation failed: upstream 503".to_string(),
            },
            "a transport blip is retryable and keeps the prefix install keys on"
        );
        assert_eq!(
            discovery_error(HostedMcpDiscoveryError::Permanent(
                "not an MCP endpoint".to_string()
            )),
            ProductOperationFailure::InvalidBindingRequest {
                reason: "hosted MCP catalog preparation failed: not an MCP endpoint".to_string(),
            },
            "a permanently wrong endpoint is the registrant's to fix"
        );
        assert_eq!(
            discovery_error(HostedMcpDiscoveryError::CredentialsRejected(
                ironclaw_extension_contracts::hosted_mcp::McpAuthChallenge {
                    status: 401,
                    www_authenticate_metadata: Vec::new(),
                    protected_resource_metadata: Vec::new(),
                }
            )),
            ProductOperationFailure::InvalidBindingRequest {
                reason: "hosted MCP account setup is required".to_string(),
            },
            "a credential challenge must route the user to setup, not read as a transport failure"
        );
    }

    /// The producer above and the predicate the post-install classifiers key on
    /// are joined here rather than each asserting its own copy of the literal.
    ///
    /// This is the coupling that drifts silently: `discovery_error` emits the
    /// prefix in `hosted_mcp_manifest.rs`, and two classifiers in two other
    /// modules decide "still installed" from it. Asserting the prefix on one
    /// side and the comparison on the other would let a reworded producer pass
    /// both. Feeding the producer's *actual output* into the predicate is what
    /// makes a rewording fail — and the negative cases are what stop the
    /// predicate from degenerating into "always true".
    #[test]
    fn every_discovery_failure_the_producer_emits_still_reads_as_an_installed_extension() {
        for outcome in [
            HostedMcpDiscoveryError::Transient("upstream 503".to_string()),
            HostedMcpDiscoveryError::Permanent("not an MCP endpoint".to_string()),
        ] {
            let reason = match discovery_error(outcome) {
                ProductOperationFailure::Transient { reason }
                | ProductOperationFailure::InvalidBindingRequest { reason } => reason,
                other => panic!("discovery failures are transient or invalid, got {other:?}"),
            };
            assert!(
                super::hosted_mcp_discovery_left_the_install_usable(&reason),
                "the post-install classifiers must recognize what discovery_error emits: {reason:?}"
            );
        }

        assert!(
            super::hosted_mcp_discovery_left_the_install_usable(
                super::HOSTED_MCP_NO_CALLABLE_TOOLS_REASON
            ),
            "a server that published no callable tools still leaves the extension installed"
        );

        // A credential challenge is NOT a discovery failure — it must reach the
        // caller so the user is offered the connect step, so the same producer's
        // third outcome must read the other way.
        let credentials_rejected = discovery_error(HostedMcpDiscoveryError::CredentialsRejected(
            ironclaw_extension_contracts::hosted_mcp::McpAuthChallenge {
                status: 401,
                www_authenticate_metadata: Vec::new(),
                protected_resource_metadata: Vec::new(),
            },
        ));
        let ProductOperationFailure::InvalidBindingRequest { reason } = credentials_rejected else {
            panic!("a credential challenge is the caller's to fix");
        };
        assert!(
            !super::hosted_mcp_discovery_left_the_install_usable(&reason),
            "account setup must surface to the caller, not be swallowed as a usable install"
        );
        assert!(
            !super::hosted_mcp_discovery_left_the_install_usable("some unrelated rejection"),
            "the predicate is reason-specific; any other rejection must still surface"
        );
    }

    /// OAuth metadata arrives from the remote server, so the rejection text is
    /// deliberately fixed: the underlying error is logged, never forwarded.
    #[test]
    fn inadmissible_oauth_metadata_is_rejected_without_echoing_the_cause() {
        assert_eq!(
            oauth_admission_error(ironclaw_auth::AuthProductError::MalformedConfig),
            ProductOperationFailure::InvalidBindingRequest {
                reason: "hosted MCP OAuth metadata was not admissible".to_string(),
            },
        );
        assert_eq!(
            oauth_admission_error(ironclaw_auth::AuthProductError::ProviderDenied),
            ProductOperationFailure::InvalidBindingRequest {
                reason: "hosted MCP OAuth metadata was not admissible".to_string(),
            },
            "every admission failure collapses to one non-echoing reason"
        );
    }

    #[test]
    fn endpoint_reconstruction_errors_keep_client_responses_sanitized() {
        let unsafe_endpoint = "https://mcp.example.test/rpc?access_token=not-for-logs";
        let input_error = endpoint_input_error(ironclaw_host_api::error::HostApiError::invalid_id(
            "hosted_mcp_endpoint",
            unsafe_endpoint,
            "hosted MCP endpoint must not contain credentials",
        ));
        let expected = ProductOperationFailure::InvalidBindingRequest {
            reason: "hosted MCP extension name is unavailable".to_string(),
        };
        assert_eq!(input_error, expected);
        assert!(
            !format!("{input_error:?}").contains(unsafe_endpoint),
            "the product response must not echo an unsafe endpoint"
        );

        for admission_error in [
            crate::hosted_mcp_admission::HostedMcpAdmissionError::InvalidEndpoint,
            crate::hosted_mcp_admission::HostedMcpAdmissionError::InvalidVendorId,
        ] {
            assert_eq!(
                endpoint_admission_error(admission_error),
                expected,
                "canonicalization and vendor-ID failures retain only safe diagnostics"
            );
        }
    }

    /// `pending_manifest` builds a manifest by string interpolation, so its two
    /// input guards are the boundary that keeps caller-supplied text out of the
    /// generated TOML. Both are pinned against a control that is accepted, so
    /// the assertions cannot pass because the whole call fails for some other
    /// reason.
    #[test]
    fn hosted_mcp_registration_rejects_unusable_names_and_client_profiles() {
        let extension_id =
            ironclaw_host_api::ids::ExtensionId::new("mcp-linear").expect("valid extension id");
        let endpoint = crate::hosted_mcp_admission::CanonicalHostedMcpEndpoint::parse(
            &ironclaw_extension_contracts::hosted_mcp::HostedMcpEndpoint::new(
                "https://mcp.linear.app/rpc".to_string(),
            )
            .expect("valid endpoint"),
        )
        .expect("canonical endpoint");
        let no_auth = super::HostedMcpAuthSelection::NoAuth;

        let name_expected = ProductOperationFailure::InvalidBindingRequest {
            reason: "hosted MCP extension name is invalid".to_string(),
        };
        for unusable_name in ["", "   ", &"x".repeat(257)] {
            assert_eq!(
                super::pending_manifest(&extension_id, unusable_name, &endpoint, &no_auth)
                    .expect_err("an unusable display name must be rejected"),
                name_expected,
                "name {unusable_name:?} must not reach manifest interpolation"
            );
        }
        assert!(
            super::pending_manifest(&extension_id, "Linear", &endpoint, &no_auth).is_ok(),
            "a usable name must still register — the guard is about the name, not the call"
        );

        let profile_expected = ProductOperationFailure::InvalidBindingRequest {
            reason: "hosted MCP OAuth client profile is invalid".to_string(),
        };
        for unusable_profile in ["", "   ", "has\u{0}control", &"p".repeat(129)] {
            assert_eq!(
                super::pending_manifest(
                    &extension_id,
                    "Linear",
                    &endpoint,
                    &super::HostedMcpAuthSelection::OAuth {
                        client_profile_id: Some(unusable_profile.to_string()),
                    },
                )
                .expect_err("an unusable client profile must be rejected"),
                profile_expected,
                "client profile {unusable_profile:?} must not reach the manifest"
            );
        }
        assert!(
            super::pending_manifest(
                &extension_id,
                "Linear",
                &endpoint,
                &super::HostedMcpAuthSelection::OAuth {
                    client_profile_id: Some("linear-profile".to_string()),
                },
            )
            .is_ok(),
            "a usable client profile must still register"
        );
    }
}
