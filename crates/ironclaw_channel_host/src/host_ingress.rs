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

use ironclaw_extensions::{
    ExtensionManifestRecord, HostApiContractRegistry, ManifestSource, ManifestV2Error,
};
use ironclaw_host_api::ingress::IngressRouteDescriptor;
use ironclaw_product_adapter_registry::product_adapter_sections;
use thiserror::Error;

/// The host-API contract registry every product-extension manifest parse uses:
/// the default host ports plus the product-adapter section contract. Shared by
/// bundled-extension installation and serve-time ingress projection so the two
/// paths can never diverge on parsing context.
pub fn product_extension_host_api_contract_registry()
-> Result<HostApiContractRegistry, ManifestV2Error> {
    let mut registry = ironclaw_host_runtime::default_host_api_contract_registry()?;
    ironclaw_product_adapter_registry::register_product_adapter_host_api_contract(&mut registry)
        .map_err(|error| ManifestV2Error::Invalid {
            reason: format!("product adapter host API contract registration failed: {error}"),
        })?;
    Ok(registry)
}

#[derive(Debug, Error)]
pub enum HostIngressProjectionError {
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
pub fn bundled_host_ingress_descriptors(
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
pub fn descriptor_for_route(
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

/// Per-installation token-bucket rate limiter for channel webhook ingress,
/// keyed on `(tenant, adapter installation)`. Shared by every channel host;
/// serve layers map [`InstallationRateExceeded`] onto their own sanitized
/// ingress error shapes.
#[cfg(feature = "webhook-serve")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallationRateLimitConfig {
    pub max_requests: std::num::NonZeroU32,
    pub window: std::time::Duration,
}

#[cfg(feature = "webhook-serve")]
impl InstallationRateLimitConfig {
    pub fn new(max_requests: std::num::NonZeroU32, window: std::time::Duration) -> Self {
        Self {
            max_requests,
            window,
        }
    }
}

#[cfg(feature = "webhook-serve")]
/// The limiter refused a request for this installation within the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallationRateExceeded {
    pub tenant_id: ironclaw_host_api::TenantId,
    pub adapter_installation_id: ironclaw_product_adapters::AdapterInstallationId,
}

#[cfg(feature = "webhook-serve")]
#[derive(Clone)]
pub struct InstallationRateLimiter {
    config: InstallationRateLimitConfig,
    buckets: std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<InstallationRateLimitKey, RateLimitBucket>>,
    >,
}

#[cfg(feature = "webhook-serve")]
impl InstallationRateLimiter {
    pub fn new(config: InstallationRateLimitConfig) -> Self {
        Self {
            config,
            buckets: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    pub fn check(
        &self,
        tenant_id: &ironclaw_host_api::TenantId,
        adapter_installation_id: &ironclaw_product_adapters::AdapterInstallationId,
    ) -> Result<(), InstallationRateExceeded> {
        let now = std::time::Instant::now();
        let key = InstallationRateLimitKey {
            tenant_id: tenant_id.clone(),
            adapter_installation_id: adapter_installation_id.clone(),
        };
        let mut buckets = match self.buckets.lock() {
            Ok(buckets) => buckets,
            Err(poisoned) => poisoned.into_inner(),
        };
        self.prune_stale_buckets(&mut buckets, now);
        let bucket = buckets
            .entry(key)
            .or_insert_with(|| RateLimitBucket::full(now, &self.config));
        bucket.refill(now, &self.config);
        if !bucket.try_consume() {
            return Err(InstallationRateExceeded {
                tenant_id: tenant_id.clone(),
                adapter_installation_id: adapter_installation_id.clone(),
            });
        }
        Ok(())
    }

    fn prune_stale_buckets(
        &self,
        buckets: &mut std::collections::HashMap<InstallationRateLimitKey, RateLimitBucket>,
        now: std::time::Instant,
    ) {
        // Prune on idle time alone: `last_refilled_at` advances on every
        // `check` for the key, so a bucket untouched for 2× the window is
        // stale regardless of its token level (a recreated bucket starts
        // full, which is exactly the fresh-window semantic). Keeping
        // sub-capacity buckets alive forever — the previous predicate — made
        // the map grow monotonically with every installation ever seen.
        let ttl = self.config.window.saturating_mul(2);
        buckets.retain(|_, bucket| now.duration_since(bucket.last_refilled_at) < ttl);
    }
}

#[cfg(feature = "webhook-serve")]
impl std::fmt::Debug for InstallationRateLimiter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InstallationRateLimiter")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "webhook-serve")]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct InstallationRateLimitKey {
    tenant_id: ironclaw_host_api::TenantId,
    adapter_installation_id: ironclaw_product_adapters::AdapterInstallationId,
}

#[cfg(feature = "webhook-serve")]
#[derive(Debug, Clone)]
struct RateLimitBucket {
    last_refilled_at: std::time::Instant,
    tokens: f64,
}

#[cfg(feature = "webhook-serve")]
impl RateLimitBucket {
    fn full(now: std::time::Instant, config: &InstallationRateLimitConfig) -> Self {
        Self {
            last_refilled_at: now,
            tokens: config.max_requests.get() as f64,
        }
    }

    fn refill(&mut self, now: std::time::Instant, config: &InstallationRateLimitConfig) {
        let elapsed = now.duration_since(self.last_refilled_at);
        if elapsed.is_zero() {
            return;
        }
        let capacity = config.max_requests.get() as f64;
        let refill_ratio = if config.window.is_zero() {
            1.0
        } else {
            elapsed.as_secs_f64() / config.window.as_secs_f64()
        };
        self.tokens = capacity.min(self.tokens + refill_ratio * capacity);
        self.last_refilled_at = now;
    }

    fn try_consume(&mut self) -> bool {
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

#[cfg(feature = "webhook-serve")]
/// Sanitized webhook error vocabulary shared by every channel host's public
/// ingress route: the response body is `{"error": <category>}` and never
/// carries provider or internal detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookErrorCategory {
    Authentication,
    Capacity,
    MalformedPayload,
    Adapter,
    TemporarilyUnavailable,
}

#[cfg(feature = "webhook-serve")]
#[derive(Debug, serde::Serialize)]
struct WebhookErrorBody {
    error: WebhookErrorCategory,
}

#[cfg(feature = "webhook-serve")]
pub fn webhook_error_response(
    status: axum::http::StatusCode,
    category: WebhookErrorCategory,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    (status, axum::Json(WebhookErrorBody { error: category })).into_response()
}

#[cfg(feature = "webhook-serve")]
/// The shared `RunnerError` → HTTP mapping every channel webhook uses:
/// authentication failures 401, capacity 429, retryable adapter/workflow
/// faults 503, everything else a sanitized 400. Serve layers add their own
/// per-target debug diagnostics around it.
pub fn runner_error_status(
    error: &ironclaw_wasm_product_adapters::RunnerError,
) -> (axum::http::StatusCode, WebhookErrorCategory) {
    use axum::http::StatusCode;
    use ironclaw_wasm_product_adapters::RunnerError;
    match error {
        RunnerError::AuthenticationFailed { .. } => (
            StatusCode::UNAUTHORIZED,
            WebhookErrorCategory::Authentication,
        ),
        RunnerError::TooManyInFlight { .. } => (
            StatusCode::TOO_MANY_REQUESTS,
            WebhookErrorCategory::Capacity,
        ),
        RunnerError::Adapter(adapter_error) if adapter_error.is_retryable() => (
            StatusCode::SERVICE_UNAVAILABLE,
            WebhookErrorCategory::TemporarilyUnavailable,
        ),
        RunnerError::WorkflowTimeout { .. }
        | RunnerError::WorkflowJoinFailed
        | RunnerError::WorkflowPanicked
        | RunnerError::AdapterPanicked => (
            StatusCode::SERVICE_UNAVAILABLE,
            WebhookErrorCategory::TemporarilyUnavailable,
        ),
        RunnerError::Adapter(_) => (StatusCode::BAD_REQUEST, WebhookErrorCategory::Adapter),
    }
}

#[cfg(all(test, feature = "webhook-serve"))]
mod installation_rate_limiter_tests {
    use super::*;

    fn key_parts(
        label: &str,
    ) -> (
        ironclaw_host_api::TenantId,
        ironclaw_product_adapters::AdapterInstallationId,
    ) {
        (
            ironclaw_host_api::TenantId::new("tenant-a").expect("tenant"),
            ironclaw_product_adapters::AdapterInstallationId::new(label).expect("installation"),
        )
    }

    /// Regression: a bucket that consumed a token and then went idle must be
    /// pruned once it passes 2× the window — the old predicate kept every
    /// sub-capacity bucket alive forever, growing the map monotonically.
    #[test]
    fn idle_buckets_are_pruned_after_the_ttl() {
        let limiter = InstallationRateLimiter::new(InstallationRateLimitConfig::new(
            std::num::NonZeroU32::new(5).expect("nonzero"),
            std::time::Duration::from_millis(2),
        ));
        let (tenant, installation_a) = key_parts("tg-bot-1");
        limiter
            .check(&tenant, &installation_a)
            .expect("first check passes");
        assert_eq!(limiter.buckets.lock().expect("lock").len(), 1);

        std::thread::sleep(std::time::Duration::from_millis(10));
        let (_, installation_b) = key_parts("tg-bot-2");
        limiter
            .check(&tenant, &installation_b)
            .expect("second key passes");
        let buckets = limiter.buckets.lock().expect("lock");
        assert_eq!(
            buckets.len(),
            1,
            "the idle tg-bot-1 bucket must be pruned once past 2x the window"
        );
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
