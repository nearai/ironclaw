//! Project host-ingress route descriptors from a bundled extension manifest.
//!
//! This is the read side of the manifest-driven ingress contract: an
//! extension's `manifest.toml` declares its inbound HTTP routes as data
//! (`[[product_adapter.<sub>.host_ingress]]`), and this helper projects the
//! validated [`IngressRouteDescriptor`] the serve layer mounts. The route's
//! *policy/shape* is data in the manifest; the axum handler and the webhook
//! verifier (behavior) stay in the owning serve module.
//!
//! `ironclaw_product_adapter_registry` owns the parsing/validation and the
//! fail-closed credential coherence (every route names a verifying credential
//! declared in `required_credentials`); this module only selects a route by id
//! and hands back its descriptor.

use ironclaw_extensions::ManifestSource;
use ironclaw_host_api::HostPortCatalog;
use ironclaw_host_api::ingress::IngressRouteDescriptor;
use ironclaw_product_adapter_registry::{
    parse_product_adapter_manifest_record, product_adapter_sections,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum HostIngressProjectionError {
    #[error("bundled manifest failed to project host-ingress routes: {reason}")]
    Projection { reason: String },
    #[error("bundled manifest declares no host-ingress route {route_id}")]
    RouteNotDeclared { route_id: String },
}

/// Project the [`IngressRouteDescriptor`] for `route_id` from a bundled
/// (host-compiled) extension manifest.
///
/// The descriptor is validated by `ironclaw_host_api` on deserialize (dotted
/// route id, absolute path, and every policy invariant including the
/// fail-closed floor that a `public_webhook` listener must require
/// `webhook_signature`), and by the registry for ingress credential coherence.
/// Intended for compile-time bundled manifests, so callers may treat a failure
/// as a startup invariant violation.
pub(crate) fn bundled_host_ingress_descriptor(
    manifest_toml: &str,
    route_id: &str,
) -> Result<IngressRouteDescriptor, HostIngressProjectionError> {
    let record = parse_product_adapter_manifest_record(
        manifest_toml,
        ManifestSource::HostBundled,
        &HostPortCatalog::empty(),
        None,
    )
    .map_err(|error| HostIngressProjectionError::Projection {
        reason: error.to_string(),
    })?;
    let sections = product_adapter_sections(&record).map_err(|error| {
        HostIngressProjectionError::Projection {
            reason: error.to_string(),
        }
    })?;
    sections
        .iter()
        .flat_map(|section| section.host_ingress())
        .find(|route| route.descriptor().route_id().as_str() == route_id)
        .map(|route| route.descriptor().clone())
        .ok_or_else(|| HostIngressProjectionError::RouteNotDeclared {
            route_id: route_id.to_string(),
        })
}
