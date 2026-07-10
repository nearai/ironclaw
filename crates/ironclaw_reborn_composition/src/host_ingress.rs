//! Project host-ingress route descriptors from a bundled extension manifest.
//!
//! This is the read side of the manifest-driven ingress contract: an
//! extension's `manifest.toml` declares its inbound HTTP routes as data
//! (`[[product_adapter.<sub>.host_ingress]]`), and this helper projects the
//! validated [`IngressRouteDescriptor`]s the serve layer mounts. The route's
//! *policy/shape* is data in the manifest; the axum handler and the webhook
//! verifier (behavior) stay in the owning serve module.
//!
//! Parsing deliberately reuses the same context as bundled extension
//! installation (`available_extensions::bundled_extension_package`): the
//! default host-port catalog and the composition-owned product extension
//! host-API contract registry. A bundled manifest therefore cannot be
//! installable but fail serve-time projection (or vice versa) because the two
//! paths diverged on parsing context. `ironclaw_product_adapter_registry` owns
//! the section validation and the fail-closed credential coherence (every
//! auth-required route names a verifying credential declared in
//! `required_credentials`);
//! this module only projects the descriptors and selects one by id.

use ironclaw_extensions::{ExtensionManifestRecord, ManifestSource};
use ironclaw_host_api::ingress::IngressRouteDescriptor;
use ironclaw_product_adapter_registry::product_adapter_sections;
use thiserror::Error;

use crate::extension_host::host_api_contracts::product_extension_host_api_contract_registry;

#[derive(Debug, Error)]
pub(crate) enum HostIngressProjectionError {
    #[error("bundled manifest failed to project host-ingress routes: {reason}")]
    Projection { reason: String },
    #[error("bundled manifest declares no host-ingress route {route_id}")]
    RouteNotDeclared { route_id: String },
}

/// Project every [`IngressRouteDescriptor`] a bundled (host-compiled)
/// extension manifest declares, in one parse.
///
/// Each descriptor is validated by `ironclaw_host_api` on deserialize (dotted
/// route id, absolute path, and every policy invariant including the
/// fail-closed floor that a `public_webhook` listener must require
/// `webhook_signature`), and by the registry for ingress credential coherence.
/// Intended for compile-time bundled manifests, so callers may treat a failure
/// as a startup invariant violation.
pub(crate) fn bundled_host_ingress_descriptors(
    manifest_toml: &str,
) -> Result<Vec<IngressRouteDescriptor>, HostIngressProjectionError> {
    let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(projection)?;
    let contracts = product_extension_host_api_contract_registry().map_err(projection)?;
    let record = ExtensionManifestRecord::from_toml_with_contracts(
        manifest_toml,
        ManifestSource::HostBundled,
        &host_ports,
        None,
        &contracts,
    )
    .map_err(projection)?;
    let sections = product_adapter_sections(&record).map_err(projection)?;
    Ok(sections
        .iter()
        .flat_map(|section| section.host_ingress())
        .map(|route| route.descriptor().clone())
        .collect())
}

/// Select the descriptor for `route_id` from an already-projected set.
pub(crate) fn descriptor_for_route(
    descriptors: &[IngressRouteDescriptor],
    route_id: &str,
) -> Result<IngressRouteDescriptor, HostIngressProjectionError> {
    descriptors
        .iter()
        .find(|descriptor| descriptor.route_id().as_str() == route_id)
        .cloned()
        .ok_or_else(|| HostIngressProjectionError::RouteNotDeclared {
            route_id: route_id.to_string(),
        })
}

fn projection(error: impl std::fmt::Display) -> HostIngressProjectionError {
    HostIngressProjectionError::Projection {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "testext"
name = "Test Extension"
version = "0.1.0"
description = "Host-ingress projection fixture."
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "test_service"

[[host_api]]
id = "ironclaw.product_adapter/v1"
section = "product_adapter.inbound"

[product_adapter.inbound]
surface_kind = "external_channel"

[product_adapter.inbound.auth]
kind = "request_signature"
header_name = "X-Test-Signature"
timestamp_header_name = "X-Test-Timestamp"

[product_adapter.inbound.capabilities]
flags = ["inbound_messages"]

[[product_adapter.inbound.required_credentials]]
handle = "test_signing_secret"

[[product_adapter.inbound.host_ingress]]
credential_handles = ["test_signing_secret"]

[product_adapter.inbound.host_ingress.descriptor]
route_id = "testext.events"
method = "post"
route_pattern = "/webhooks/testext/events"

[product_adapter.inbound.host_ingress.descriptor.policy]
listener_class = "public_webhook"
auth = { type = "required", schemes = ["webhook_signature"] }
scope_source = "host_resolved"
body_limit = { type = "limited", max_bytes = 1024 }
rate_limit = { type = "limited", scope = "global", max_requests = 60, window_seconds = 60 }
cors = "not_applicable"
websocket_origin = "not_applicable"
streaming = "none"
audit = "public_callback"
effect_path = { type = "product_workflow" }
"#;

    #[test]
    fn bundled_host_ingress_descriptors_project_declared_routes() {
        let descriptors =
            bundled_host_ingress_descriptors(VALID_MANIFEST).expect("valid manifest projects");
        let descriptor =
            descriptor_for_route(&descriptors, "testext.events").expect("declared route resolves");
        assert_eq!(descriptor.route_id().as_str(), "testext.events");
        assert_eq!(
            descriptor.route_pattern().as_str(),
            "/webhooks/testext/events"
        );
    }

    #[test]
    fn bundled_host_ingress_descriptor_rejects_missing_route_id() {
        let descriptors =
            bundled_host_ingress_descriptors(VALID_MANIFEST).expect("valid manifest projects");
        let error = descriptor_for_route(&descriptors, "testext.absent")
            .expect_err("absent route id must be rejected");
        assert!(
            matches!(
                &error,
                HostIngressProjectionError::RouteNotDeclared { route_id }
                    if route_id == "testext.absent"
            ),
            "expected RouteNotDeclared, got: {error}"
        );
    }
}
