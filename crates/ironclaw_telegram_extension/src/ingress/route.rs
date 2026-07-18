//! Axum route and manifest projection for Telegram updates.

use std::num::NonZeroU32;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::ingress::IngressRouteDescriptor;
use ironclaw_wasm_product_adapters::RunnerError;

use crate::setup::TELEGRAM_UPDATES_ROUTE_PATH;

use super::{DynamicTelegramInstallationResolver, TelegramIngressError};

/// `/webhooks/extensions/telegram/updates` — aliases the setup-pipeline
/// constant so the path `setWebhook` registers and the path this module
/// mounts cannot drift; the descriptor test pins both to the manifest.
#[cfg_attr(not(test), allow(dead_code))]
pub const TELEGRAM_UPDATES_PATH: &str = TELEGRAM_UPDATES_ROUTE_PATH;
pub(crate) const TELEGRAM_UPDATES_ROUTE_ID: &str = "telegram.updates";

const TELEGRAM_INSTALLATION_MAX_REQUESTS: NonZeroU32 = match NonZeroU32::new(120) {
    Some(value) => value,
    None => NonZeroU32::MIN,
};
const TELEGRAM_INSTALLATION_RATE_WINDOW: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct TelegramIngressService {
    resolver: Arc<DynamicTelegramInstallationResolver>,
    installation_rate_limiter: ironclaw_channel_host::host_ingress::InstallationRateLimiter,
}

impl TelegramIngressService {
    pub fn new(resolver: Arc<DynamicTelegramInstallationResolver>) -> Self {
        Self::with_rate_limit_config(
            resolver,
            ironclaw_channel_host::host_ingress::InstallationRateLimitConfig::new(
                TELEGRAM_INSTALLATION_MAX_REQUESTS,
                TELEGRAM_INSTALLATION_RATE_WINDOW,
            ),
        )
    }

    pub fn with_rate_limit_config(
        resolver: Arc<DynamicTelegramInstallationResolver>,
        rate_limit: ironclaw_channel_host::host_ingress::InstallationRateLimitConfig,
    ) -> Self {
        Self {
            resolver,
            installation_rate_limiter:
                ironclaw_channel_host::host_ingress::InstallationRateLimiter::new(rate_limit),
        }
    }

    async fn handle_updates(&self, headers: HeaderMap, body: Bytes) -> Response {
        let installation = match self.resolver.resolve_ingress(&headers, body.as_ref()).await {
            Ok(installation) => installation,
            Err(error) => return ingress_error_response(error),
        };
        if let Err(exceeded) = self.installation_rate_limiter.check(
            installation.tenant_id(),
            installation.adapter_installation_id(),
        ) {
            return ingress_error_response(TelegramIngressError::InstallationRateLimited {
                tenant_id: exceeded.tenant_id,
                adapter_installation_id: exceeded.adapter_installation_id,
            });
        }
        match installation
            .dispatcher()
            .process_verified_update(
                body.as_ref(),
                installation.evidence(),
                // Revision-scoped: the resolver rebuilds the observer together
                // with the workflow on every setup change, so no static
                // route-state observer exists to fall back to.
                installation.workflow_observer(),
            )
            .await
        {
            Ok(_) => (StatusCode::OK, "ok").into_response(),
            Err(error) => runner_error_response(error),
        }
    }

    pub async fn drain_installations(&self) {
        self.resolver.drain_installations().await;
    }
}

impl std::fmt::Debug for TelegramIngressService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TelegramIngressService")
            .field("resolver", &"DynamicTelegramInstallationResolver")
            .field("installation_rate_limiter", &self.installation_rate_limiter)
            .finish()
    }
}

/// Route state for the updates webhook. Unlike Slack's `SlackEventsRouteState`
/// there is no static route-level workflow observer: the Telegram observer is
/// rebuilt per setup revision and travels on the resolved installation.
#[derive(Clone)]
pub struct TelegramUpdatesRouteState {
    ingress: TelegramIngressService,
}

impl TelegramUpdatesRouteState {
    pub fn new(ingress: TelegramIngressService) -> Self {
        Self { ingress }
    }

    pub fn from_resolver(resolver: Arc<DynamicTelegramInstallationResolver>) -> Self {
        Self::new(TelegramIngressService::new(resolver))
    }

    pub async fn drain_immediate_ack_tasks(&self) {
        self.ingress.drain_installations().await;
    }
}

impl std::fmt::Debug for TelegramUpdatesRouteState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TelegramUpdatesRouteState")
            .field("ingress", &self.ingress)
            .finish()
    }
}

/// Build the raw updates-route fragment: the axum router (state applied) plus
/// the manifest-projected route descriptors. Composition wraps the pair into
/// its public-route mount shape and installs
/// [`TelegramUpdatesRouteState::drain_immediate_ack_tasks`] as the drain —
/// this crate cannot name composition's mount types without a cycle.
pub fn telegram_updates_route_parts(
    state: TelegramUpdatesRouteState,
) -> (Router, Vec<IngressRouteDescriptor>) {
    let descriptor = TELEGRAM_INGRESS_DESCRIPTORS.updates.clone();
    (
        Router::new()
            .route(
                descriptor.route_pattern().as_str(),
                post(telegram_updates_handler),
            )
            .with_state(state),
        vec![descriptor],
    )
}

// Manifest-projection parity seam, pinned by the descriptor test.
#[cfg_attr(not(test), allow(dead_code))]
pub fn telegram_updates_route_descriptors() -> Vec<IngressRouteDescriptor> {
    vec![TELEGRAM_INGRESS_DESCRIPTORS.updates.clone()]
}

/// The Telegram host-ingress route descriptor, projected from the bundled
/// Telegram channel manifest in a single parse on first use (the manifest is
/// a compile-time constant, so the projection is deterministic and cached for
/// the process lifetime).
///
/// The route's path/method/policy are declared as data in
/// `assets/telegram/manifest.toml` (`[[product_adapter.inbound.host_ingress]]`)
/// and validated by `ironclaw_host_api` plus
/// `ironclaw_product_adapter_registry`; only the declarative descriptor lives
/// in the manifest — the axum handler and the shared-secret verifier stay in
/// this module, and the mount function builds its route from the descriptor
/// so what axum mounts cannot drift from what the manifest declares. Panics
/// if the bundled manifest does not declare the route or declares it with a
/// non-POST method: `TELEGRAM_MANIFEST` is a compile-time constant, so either
/// is a build-time invariant violation, surfaced at startup.
static TELEGRAM_INGRESS_DESCRIPTORS: LazyLock<TelegramIngressDescriptors> = LazyLock::new(|| {
    let descriptors = ironclaw_channel_host::host_ingress::bundled_host_ingress_descriptors(
        crate::telegram_manifest::telegram_manifest_toml(),
    )
    .unwrap_or_else(|error| {
        panic!("bundled Telegram manifest must project host-ingress routes: {error}")
    });
    TelegramIngressDescriptors {
        updates: bundled_telegram_post_descriptor(&descriptors, TELEGRAM_UPDATES_ROUTE_ID),
    }
});

struct TelegramIngressDescriptors {
    updates: IngressRouteDescriptor,
}

fn bundled_telegram_post_descriptor(
    descriptors: &[IngressRouteDescriptor],
    route_id: &str,
) -> IngressRouteDescriptor {
    let descriptor =
        ironclaw_channel_host::host_ingress::descriptor_for_route(descriptors, route_id)
            .unwrap_or_else(|error| {
                panic!(
                    "bundled Telegram manifest must declare host-ingress route {route_id}: {error}"
                )
            });
    // The mount function wires its handler with `post(...)`; fail closed at
    // projection time if the manifest ever declares another method.
    if descriptor.method() != NetworkMethod::Post {
        panic!(
            "bundled Telegram manifest declares host-ingress route {route_id} with method {}, \
             but the serve layer mounts POST handlers",
            descriptor.method()
        );
    }
    descriptor
}

async fn telegram_updates_handler(
    State(state): State<TelegramUpdatesRouteState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    state.ingress.handle_updates(headers, body).await
}

pub(crate) fn ingress_error_response(error: TelegramIngressError) -> Response {
    match error {
        TelegramIngressError::Runner(error) => runner_error_response(error),
        TelegramIngressError::InstallationNotFound => {
            tracing::debug!(
                target = "ironclaw::reborn::telegram_updates",
                reason = "not_found",
                "Telegram updates installation resolution failed"
            );
            ironclaw_channel_host::host_ingress::webhook_error_response(
                StatusCode::UNAUTHORIZED,
                ironclaw_channel_host::host_ingress::WebhookErrorCategory::Authentication,
            )
        }
        TelegramIngressError::TemporarilyUnavailable => {
            tracing::debug!(
                target = "ironclaw::reborn::telegram_updates",
                reason = "unavailable",
                "Telegram updates installation resolution temporarily unavailable"
            );
            ironclaw_channel_host::host_ingress::webhook_error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                ironclaw_channel_host::host_ingress::WebhookErrorCategory::TemporarilyUnavailable,
            )
        }
        TelegramIngressError::InstallationRateLimited {
            tenant_id,
            adapter_installation_id,
        } => {
            tracing::debug!(
                target = "ironclaw::reborn::telegram_updates",
                tenant_id = %tenant_id,
                adapter_installation_id = %adapter_installation_id,
                "Telegram updates installation rate limit exceeded"
            );
            ironclaw_channel_host::host_ingress::webhook_error_response(
                StatusCode::TOO_MANY_REQUESTS,
                ironclaw_channel_host::host_ingress::WebhookErrorCategory::Capacity,
            )
        }
    }
}

pub(crate) fn runner_error_response(error: RunnerError) -> Response {
    let (status, category) = ironclaw_channel_host::host_ingress::runner_error_status(&error);
    tracing::debug!(
        target = "ironclaw::reborn::telegram_updates",
        status = status.as_u16(),
        error = %error,
        "Telegram updates webhook rejected"
    );
    ironclaw_channel_host::host_ingress::webhook_error_response(status, category)
}
