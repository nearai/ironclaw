//! Generic notification-setup product surface: status/enable/disable by
//! `extension_id`, against **host-owned** per-user delivery registrations.
//!
//! This module used to resolve the channel's adapter and forward a
//! channel-opaque payload to three adapter methods. Design §8 deleted those
//! methods, and the reason was not surface area: while the adapter owned
//! enrollment storage the host could not answer "is this user set up?", so
//! there was no guardrail before a delivery — the send simply failed inside
//! the vendor path.
//!
//! What generic code does here now, and nothing more:
//!
//! 1. Resolve the channel's declared `[[channel.egress]]` hosts and **admit
//!    the submitted endpoint against them before storage**. That is the one
//!    security-critical check (§8), and it is generic because the host owns
//!    the allowlist: without it, enrollment is an SSRF primitive that makes
//!    the host POST to an attacker's URL.
//! 2. Bound the opaque document and store it.
//! 3. Publish the channel's client bootstrap document — the public half of a
//!    credential the host already holds — so a client can enroll without the
//!    channel exposing a bespoke status endpoint.
//!
//! It still knows nothing about push endpoints, key material, or VAPID. What
//! changed is that the ignorance is now *host-side* rather than delegated.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_product_contracts::delivery::{
    ChannelDeliveryResolver, DeliveryRegistrationError, DeliveryRegistrationRequest,
    DeliveryRegistrationScope, DeliveryRegistrationService, ResolvedChannelDelivery,
};

use super::{
    ProductSurfaceCaller, ProductSurfaceError, RebornNotificationSetupMutationRequest,
    RebornNotificationSetupRequest, RebornNotificationSetupStatusResponse,
};

/// Publishes the non-secret bootstrap document a channel's client needs to
/// enroll (e.g. the public half of a signing key the host holds).
///
/// A port rather than a lookup because the *material* is host-held credential
/// state and the *shape* is channel-specific: the host publishes it
/// generically instead of the channel exposing its own status document.
/// Absence is a legitimate answer — most channels need no bootstrap at all.
pub trait DeliveryClientBootstrap: Send + Sync {
    fn bootstrap(
        &self,
        extension_id: &str,
    ) -> Result<Option<serde_json::Value>, DeliveryClientBootstrapError>;
}

/// Sanitized failure to publish a channel's client bootstrap document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("delivery client bootstrap is unavailable")]
pub struct DeliveryClientBootstrapError;

/// Fail-closed default: no channel publishes bootstrap data.
pub struct NoDeliveryClientBootstrap;

impl DeliveryClientBootstrap for NoDeliveryClientBootstrap {
    fn bootstrap(
        &self,
        _extension_id: &str,
    ) -> Result<Option<serde_json::Value>, DeliveryClientBootstrapError> {
        Ok(None)
    }
}

/// Per-channel notification-setup operations for the authenticated caller.
#[async_trait]
pub trait ChannelNotificationSetupService: Send + Sync {
    async fn status(
        &self,
        caller: ProductSurfaceCaller,
        request: RebornNotificationSetupRequest,
    ) -> Result<RebornNotificationSetupStatusResponse, ProductSurfaceError>;

    async fn enable(
        &self,
        caller: ProductSurfaceCaller,
        request: RebornNotificationSetupMutationRequest,
    ) -> Result<RebornNotificationSetupStatusResponse, ProductSurfaceError>;

    async fn disable(
        &self,
        caller: ProductSurfaceCaller,
        request: RebornNotificationSetupMutationRequest,
    ) -> Result<RebornNotificationSetupStatusResponse, ProductSurfaceError>;
}

/// Fail-closed default until composition wires the registration-backed
/// service.
pub struct UnsupportedChannelNotificationSetupService;

#[async_trait]
impl ChannelNotificationSetupService for UnsupportedChannelNotificationSetupService {
    async fn status(
        &self,
        _caller: ProductSurfaceCaller,
        _request: RebornNotificationSetupRequest,
    ) -> Result<RebornNotificationSetupStatusResponse, ProductSurfaceError> {
        Err(notification_setup_unavailable())
    }

    async fn enable(
        &self,
        _caller: ProductSurfaceCaller,
        _request: RebornNotificationSetupMutationRequest,
    ) -> Result<RebornNotificationSetupStatusResponse, ProductSurfaceError> {
        Err(notification_setup_unavailable())
    }

    async fn disable(
        &self,
        _caller: ProductSurfaceCaller,
        _request: RebornNotificationSetupMutationRequest,
    ) -> Result<RebornNotificationSetupStatusResponse, ProductSurfaceError> {
        Err(notification_setup_unavailable())
    }
}

/// Production service over the host-owned registration store.
pub struct RegistrationChannelNotificationSetupService {
    resolver: Arc<dyn ChannelDeliveryResolver>,
    registrations: Arc<dyn DeliveryRegistrationService>,
    bootstrap: Arc<dyn DeliveryClientBootstrap>,
}

impl RegistrationChannelNotificationSetupService {
    pub fn new(
        resolver: Arc<dyn ChannelDeliveryResolver>,
        registrations: Arc<dyn DeliveryRegistrationService>,
        bootstrap: Arc<dyn DeliveryClientBootstrap>,
    ) -> Self {
        Self {
            resolver,
            registrations,
            bootstrap,
        }
    }

    /// Resolve one generation-pinned channel view. Enrollment requirement and
    /// endpoint allowlist must never come from separate snapshot reads.
    fn resolve(&self, extension_id: &str) -> Result<ResolvedChannelDelivery, ProductSurfaceError> {
        self.resolver
            .resolve_channel_delivery(extension_id)
            .ok_or_else(ProductSurfaceError::not_found)
    }

    fn scope(
        &self,
        caller: &ProductSurfaceCaller,
        extension_id: &str,
    ) -> Result<DeliveryRegistrationScope, ProductSurfaceError> {
        let extension_id = ironclaw_host_api::ids::ExtensionId::new(extension_id)
            .map_err(|_| ProductSurfaceError::not_found())?;
        Ok(DeliveryRegistrationScope {
            // Both halves come from the AUTHENTICATED caller, never from the
            // request body: an enrollment that could name its own user would
            // let anyone add a delivery endpoint to anyone's account.
            tenant_id: caller.tenant_id.clone(),
            user_id: caller.user_id.clone(),
            extension_id,
        })
    }

    /// Project the stored set into the sanitized wire shape.
    ///
    /// Endpoints never cross this boundary — an endpoint URL is a capability
    /// to send to a user's device. What a settings UI needs is enough to tell
    /// its own registrations apart, which is the host-minted id plus the
    /// non-secret client metadata the channel chose to record.
    async fn project(
        &self,
        extension_id: &str,
        scope: &DeliveryRegistrationScope,
    ) -> Result<RebornNotificationSetupStatusResponse, ProductSurfaceError> {
        let registrations = self
            .registrations
            .list(scope)
            .await
            .map_err(|error| map_registration_error(extension_id, error))?;
        let clients: Vec<serde_json::Value> = registrations
            .iter()
            .map(|registration| {
                serde_json::json!({
                    "registration_id": registration.registration_id,
                    "created_at": registration.created_at,
                    // Lowercase hex SHA-256 of the stored endpoint — the
                    // correlation key the channel's own client compares
                    // against its local subscription's digest
                    // (`device-push.ts::endpointDigestHex`), so a browser can
                    // tell whether ITS subscription belongs to this account
                    // without the endpoint capability URL ever crossing the
                    // wire.
                    "endpoint_digest":
                        ironclaw_common::hashing::sha256_hex(registration.endpoint.as_bytes()),
                })
            })
            .collect();
        let mut detail = serde_json::json!({
            "registration_count": registrations.len(),
            "registrations": clients,
        });
        match self.bootstrap.bootstrap(extension_id) {
            Ok(Some(bootstrap)) => detail["bootstrap"] = bootstrap,
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(
                    extension_id,
                    %error,
                    "delivery client bootstrap is unavailable"
                );
                return Err(ProductSurfaceError::service_unavailable(true));
            }
        }
        Ok(RebornNotificationSetupStatusResponse {
            extension_id: extension_id.to_string(),
            requires_setup: true,
            enabled: !registrations.is_empty(),
            detail,
        })
    }

    /// Shared prologue: unknown channels are not-found, and a channel with no
    /// per-user setup is deliverable as-is (so a mutation on it is a caller
    /// error, not a silent no-op).
    fn admit(
        &self,
        channel: &ResolvedChannelDelivery,
        extension_id: &str,
        mutating: bool,
    ) -> Result<Option<RebornNotificationSetupStatusResponse>, ProductSurfaceError> {
        if channel.requires_enrollment {
            return Ok(None);
        }
        if mutating {
            return Err(ProductSurfaceError::validation(
                "extension_id",
                super::ProductSurfaceValidationCode::InvalidValue,
            ));
        }
        Ok(Some(RebornNotificationSetupStatusResponse {
            extension_id: extension_id.to_string(),
            requires_setup: false,
            enabled: true,
            detail: serde_json::Value::Null,
        }))
    }
}

/// The submitted enrollment document, split into the one field the host must
/// see and the opaque remainder it must not interpret.
#[derive(serde::Deserialize)]
struct EnrollmentSubmission {
    endpoint: String,
    #[serde(flatten)]
    document: serde_json::Map<String, serde_json::Value>,
}

#[async_trait]
impl ChannelNotificationSetupService for RegistrationChannelNotificationSetupService {
    async fn status(
        &self,
        caller: ProductSurfaceCaller,
        request: RebornNotificationSetupRequest,
    ) -> Result<RebornNotificationSetupStatusResponse, ProductSurfaceError> {
        let channel = self.resolve(&request.extension_id)?;
        if let Some(response) = self.admit(&channel, &request.extension_id, false)? {
            return Ok(response);
        }
        let scope = self.scope(&caller, &request.extension_id)?;
        self.project(&request.extension_id, &scope).await
    }

    async fn enable(
        &self,
        caller: ProductSurfaceCaller,
        request: RebornNotificationSetupMutationRequest,
    ) -> Result<RebornNotificationSetupStatusResponse, ProductSurfaceError> {
        let channel = self.resolve(&request.extension_id)?;
        self.admit(&channel, &request.extension_id, true)?;
        let scope = self.scope(&caller, &request.extension_id)?;
        let submission: EnrollmentSubmission =
            serde_json::from_value(request.payload).map_err(|_| invalid_payload())?;

        // THE security-critical check, and it happens before anything is
        // stored: the endpoint must target a host this channel declares in
        // `[[channel.egress]]`. The allowlist is read from the same resolved
        // manifest egress policy enforces with, so there is no second copy to
        // drift.
        ironclaw_auth::validate_registration_endpoint(
            &submission.endpoint,
            &channel.declared_egress_hosts,
        )
        .map_err(|error| map_registration_error(&request.extension_id, error))?;

        self.registrations
            .enroll(
                &scope,
                DeliveryRegistrationRequest {
                    endpoint: submission.endpoint,
                    document: serde_json::Value::Object(submission.document).to_string(),
                },
            )
            .await
            .map_err(|error| map_registration_error(&request.extension_id, error))?;
        self.project(&request.extension_id, &scope).await
    }

    async fn disable(
        &self,
        caller: ProductSurfaceCaller,
        request: RebornNotificationSetupMutationRequest,
    ) -> Result<RebornNotificationSetupStatusResponse, ProductSurfaceError> {
        let channel = self.resolve(&request.extension_id)?;
        self.admit(&channel, &request.extension_id, true)?;
        let scope = self.scope(&caller, &request.extension_id)?;
        let submission: EnrollmentSubmission =
            serde_json::from_value(request.payload).map_err(|_| invalid_payload())?;
        // The browser edge still identifies its PushSubscription by endpoint.
        // Normalize that wire detail once against the caller-scoped canonical
        // records; internal removal is keyed only by the opaque host id.
        let registrations = self
            .registrations
            .list(&scope)
            .await
            .map_err(|error| map_registration_error(&request.extension_id, error))?;
        if let Some(registration_id) = registrations
            .iter()
            .find(|registration| registration.endpoint == submission.endpoint)
            .map(|registration| registration.registration_id.clone())
        {
            self.registrations
                .remove(&scope, &registration_id)
                .await
                .map_err(|error| map_registration_error(&request.extension_id, error))?;
        }
        self.project(&request.extension_id, &scope).await
    }
}

fn invalid_payload() -> ProductSurfaceError {
    ProductSurfaceError::validation("payload", super::ProductSurfaceValidationCode::InvalidValue)
}

pub(super) fn notification_setup_unavailable() -> ProductSurfaceError {
    ProductSurfaceError::service_unavailable(false)
}

/// Map registration-store failures onto the sanitized product taxonomy.
/// `Rejected` is caller-correctable — a bad endpoint, an undeclared host, an
/// oversized document, the per-user cap — and its reason is already
/// endpoint-free by construction. `Unavailable` is retryable storage trouble.
fn map_registration_error(
    extension_id: &str,
    error: DeliveryRegistrationError,
) -> ProductSurfaceError {
    match &error {
        DeliveryRegistrationError::Rejected { .. } => {
            tracing::debug!(extension_id, %error, "delivery registration rejected");
            invalid_payload()
        }
        DeliveryRegistrationError::Unavailable { .. } => {
            tracing::warn!(extension_id, %error, "delivery registration storage unavailable");
            ProductSurfaceError::service_unavailable(true)
        }
    }
}
