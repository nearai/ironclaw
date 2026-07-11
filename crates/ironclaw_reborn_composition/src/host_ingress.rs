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
//! default host-port catalog and the default host-API contract registry. A
//! bundled manifest therefore cannot be installable but fail serve-time
//! projection (or vice versa) because the two paths diverged on parsing
//! context. `ironclaw_product_adapter_registry` owns the section
//! validation and the fail-closed credential coherence (every auth-required
//! route names a verifying credential declared in `required_credentials`);
//! this module only projects the descriptors and selects one by id.

use std::num::{NonZeroU32, NonZeroU64};

use ironclaw_extensions::{ExtensionManifestRecord, ManifestSource};
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor,
    IngressScopeSource, ListenerClass, RateLimitPolicy, RateLimitScope, StreamingMode,
    WebSocketOriginPolicy,
};
use ironclaw_host_api::{ChannelIngressMethod, NetworkMethod};
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum HostIngressProjectionError {
    #[error("bundled manifest failed to project host-ingress routes: {reason}")]
    Projection { reason: String },
    #[error("bundled manifest declares no host-ingress route {route_id}")]
    RouteNotDeclared { route_id: String },
}

/// Build the host-ingress descriptor for a bundled extension's declared
/// channel (manifest v3 `[channel.ingress]`).
///
/// The manifest carries the route method and body limit as channel data; the
/// listener policy floors (public webhook, signature auth, host-resolved
/// scope, the global rate limit, public-callback audit) are host-owned
/// constants here until the generic ingress router (extension-runtime P4)
/// owns route mounting. The mounted `route_pattern` is likewise the caller's
/// (each legacy channel mount path stays until the canonical
/// `/webhooks/extensions/{id}/{suffix}` cutover).
pub(crate) fn bundled_channel_ingress_descriptor(
    manifest_toml: &str,
    route_id: &str,
    route_pattern: &str,
) -> Result<IngressRouteDescriptor, HostIngressProjectionError> {
    let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(projection)?;
    let contracts =
        ironclaw_host_runtime::default_host_api_contract_registry().map_err(projection)?;
    let record = ExtensionManifestRecord::from_toml(
        manifest_toml,
        ManifestSource::HostBundled,
        &host_ports,
        None,
        &contracts,
    )
    .map_err(projection)?;
    let channel = record.resolved().channel.as_ref().ok_or_else(|| {
        HostIngressProjectionError::RouteNotDeclared {
            route_id: route_id.to_string(),
        }
    })?;
    let ingress =
        channel
            .ingress
            .as_ref()
            .ok_or_else(|| HostIngressProjectionError::RouteNotDeclared {
                route_id: route_id.to_string(),
            })?;
    let method = match ingress.method {
        ChannelIngressMethod::Post => NetworkMethod::Post,
    };
    let max_bytes = NonZeroU64::new(ingress.body_limit_bytes)
        .ok_or_else(|| projection("channel ingress body limit must be non-zero"))?;
    let policy = IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::PublicWebhook,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::WebhookSignature],
        },
        scope_source: IngressScopeSource::HostResolved,
        body_limit: BodyLimitPolicy::Limited { max_bytes },
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::Global,
            max_requests: NonZeroU32::new(PUBLIC_WEBHOOK_MAX_REQUESTS)
                .expect("non-zero rate limit"),
            window_seconds: NonZeroU32::new(PUBLIC_WEBHOOK_WINDOW_SECONDS)
                .expect("non-zero rate window"),
        },
        cors: CorsPolicy::NotApplicable,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::PublicCallback,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .map_err(projection)?;
    IngressRouteDescriptor::new(route_id, method, route_pattern, policy).map_err(projection)
}

/// Host policy floor for public webhook ingress (previously carried as data
/// in v2 channel manifests' host_ingress sections).
const PUBLIC_WEBHOOK_MAX_REQUESTS: u32 = 12_000;
const PUBLIC_WEBHOOK_WINDOW_SECONDS: u32 = 60;

fn projection(error: impl std::fmt::Display) -> HostIngressProjectionError {
    HostIngressProjectionError::Projection {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "testext"
name = "Test Extension"
version = "0.1.0"
description = "Host-ingress projection fixture."
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "test_service"

[channel]
id = "messages"
display_name = "Test messages"
inbound = true
outbound = false
conversation_model = "continuous"

[channel.ingress]
route_suffix = "events"
method = "post"
body_limit_bytes = 1024

[channel.ingress.verification]
kind = "hmac_sha256"
secret_handle = "test_signing_secret"
signature_header = "X-Test-Signature"
timestamp_header = "X-Test-Timestamp"
max_age_seconds = 300
signed_payload = [ { body = true } ]

[channel.config]
fields = [ { handle = "test_signing_secret", label = "Signing secret", secret = true } ]
"#;

    #[test]
    fn bundled_channel_ingress_descriptor_projects_the_declared_channel() {
        let descriptor = bundled_channel_ingress_descriptor(
            VALID_MANIFEST,
            "testext.events",
            "/webhooks/testext/events",
        )
        .expect("valid manifest projects");
        assert_eq!(descriptor.route_id().as_str(), "testext.events");
        assert_eq!(
            descriptor.route_pattern().as_str(),
            "/webhooks/testext/events"
        );
        assert_eq!(descriptor.method(), NetworkMethod::Post);
    }

    #[test]
    fn bundled_channel_ingress_descriptor_rejects_manifests_without_a_channel() {
        let manifest = VALID_MANIFEST
            .split("[channel]")
            .next()
            .expect("fixture has a channel section");
        let error = bundled_channel_ingress_descriptor(
            manifest,
            "testext.events",
            "/webhooks/testext/events",
        )
        .expect_err("channel-less manifest must be rejected");
        assert!(
            matches!(
                &error,
                HostIngressProjectionError::RouteNotDeclared { route_id }
                    if route_id == "testext.events"
            ),
            "expected RouteNotDeclared, got: {error}"
        );
    }
}
