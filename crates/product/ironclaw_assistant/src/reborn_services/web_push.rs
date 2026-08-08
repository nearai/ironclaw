//! Web-push enrollment product surface: the status view (VAPID public key +
//! enrolled browsers) and the subscribe/unsubscribe product commands the
//! WebUI settings panel drives.
//!
//! Like `set_notification_channels`, both writes are direct authenticated
//! user settings mutations with no approval-gate need, dispatched through
//! [`super::product_capability_handlers::ProductCommandHandler`]. Push
//! *delivery* is not here — the web-push channel adapter owns it behind the
//! delivery coordinator.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::ids::InvocationId;
use ironclaw_host_api::resource::ResourceScope;
use ironclaw_web_push::{
    PushEndpoint, PushSubscriptionKeys, PushSubscriptionRecord, PushSubscriptionUpsertOutcome,
    WebPushError, WebPushSubscriptionStore,
};

use super::{
    ProductSurfaceCaller, ProductSurfaceError, RebornWebPushStatusResponse,
    RebornWebPushSubscribeOutcome, RebornWebPushSubscribeRequest, RebornWebPushSubscribeResponse,
    RebornWebPushSubscriptionInfo, RebornWebPushUnsubscribeRequest,
    RebornWebPushUnsubscribeResponse,
};

/// Web-push enrollment operations for the authenticated caller.
#[async_trait]
pub trait WebPushProductService: Send + Sync {
    async fn status(
        &self,
        caller: ProductSurfaceCaller,
    ) -> Result<RebornWebPushStatusResponse, ProductSurfaceError>;

    async fn subscribe(
        &self,
        caller: ProductSurfaceCaller,
        request: RebornWebPushSubscribeRequest,
    ) -> Result<RebornWebPushSubscribeResponse, ProductSurfaceError>;

    async fn unsubscribe(
        &self,
        caller: ProductSurfaceCaller,
        request: RebornWebPushUnsubscribeRequest,
    ) -> Result<RebornWebPushUnsubscribeResponse, ProductSurfaceError>;
}

/// Fail-closed default until composition wires the real service.
pub struct UnsupportedWebPushProductService;

#[async_trait]
impl WebPushProductService for UnsupportedWebPushProductService {
    async fn status(
        &self,
        _caller: ProductSurfaceCaller,
    ) -> Result<RebornWebPushStatusResponse, ProductSurfaceError> {
        Err(web_push_unavailable())
    }

    async fn subscribe(
        &self,
        _caller: ProductSurfaceCaller,
        _request: RebornWebPushSubscribeRequest,
    ) -> Result<RebornWebPushSubscribeResponse, ProductSurfaceError> {
        Err(web_push_unavailable())
    }

    async fn unsubscribe(
        &self,
        _caller: ProductSurfaceCaller,
        _request: RebornWebPushUnsubscribeRequest,
    ) -> Result<RebornWebPushUnsubscribeResponse, ProductSurfaceError> {
        Err(web_push_unavailable())
    }
}

/// Production service over the web-push subscription store.
pub struct RebornWebPushProductService {
    subscriptions: Arc<dyn WebPushSubscriptionStore>,
    vapid_public_key: String,
    /// Push-service hosts enrollments may target — read from the web-push
    /// manifest's `[[channel.egress]]` declarations at composition, the same
    /// list restricted egress enforces at send time.
    allowed_push_hosts: Vec<String>,
}

impl RebornWebPushProductService {
    pub fn new(
        subscriptions: Arc<dyn WebPushSubscriptionStore>,
        vapid_public_key: String,
        allowed_push_hosts: Vec<String>,
    ) -> Self {
        Self {
            subscriptions,
            vapid_public_key,
            allowed_push_hosts,
        }
    }

    /// Invariant: the scope is derived from the authenticated caller, never
    /// from request-body tenant/user fields.
    fn scope(caller: &ProductSurfaceCaller) -> ResourceScope {
        ResourceScope {
            tenant_id: caller.tenant_id.clone(),
            user_id: caller.user_id.clone(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }
}

#[async_trait]
impl WebPushProductService for RebornWebPushProductService {
    async fn status(
        &self,
        caller: ProductSurfaceCaller,
    ) -> Result<RebornWebPushStatusResponse, ProductSurfaceError> {
        let scope = Self::scope(&caller);
        let subscriptions = self
            .subscriptions
            .list_subscriptions(&scope)
            .await
            .map_err(map_web_push_error)?;
        let infos = subscriptions
            .iter()
            .map(|record| {
                Ok(RebornWebPushSubscriptionInfo {
                    subscription_id: record.subscription_id.clone(),
                    endpoint_host: record.endpoint.host().map_err(map_web_push_error)?,
                    endpoint_digest: record.endpoint.digest(),
                    user_agent: record.user_agent.clone(),
                    created_at: record.created_at.clone(),
                })
            })
            .collect::<Result<Vec<_>, ProductSurfaceError>>()?;
        Ok(RebornWebPushStatusResponse {
            vapid_public_key: self.vapid_public_key.clone(),
            subscription_count: u32::try_from(infos.len()).unwrap_or(u32::MAX),
            subscriptions: infos,
        })
    }

    async fn subscribe(
        &self,
        caller: ProductSurfaceCaller,
        request: RebornWebPushSubscribeRequest,
    ) -> Result<RebornWebPushSubscribeResponse, ProductSurfaceError> {
        let endpoint = PushEndpoint::new(request.endpoint).map_err(map_web_push_error)?;
        endpoint
            .validate_against_push_services(&self.allowed_push_hosts)
            .map_err(map_web_push_error)?;
        let keys = PushSubscriptionKeys::new(request.keys.p256dh, request.keys.auth)
            .map_err(map_web_push_error)?;
        let record = PushSubscriptionRecord::new(
            endpoint,
            keys,
            request.user_agent,
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        );
        let scope = Self::scope(&caller);
        let outcome = self
            .subscriptions
            .upsert_subscription(&scope, record)
            .await
            .map_err(map_web_push_error)?;
        Ok(RebornWebPushSubscribeResponse {
            outcome: match outcome {
                PushSubscriptionUpsertOutcome::Enrolled => RebornWebPushSubscribeOutcome::Enrolled,
                PushSubscriptionUpsertOutcome::Refreshed => {
                    RebornWebPushSubscribeOutcome::Refreshed
                }
            },
        })
    }

    async fn unsubscribe(
        &self,
        caller: ProductSurfaceCaller,
        request: RebornWebPushUnsubscribeRequest,
    ) -> Result<RebornWebPushUnsubscribeResponse, ProductSurfaceError> {
        let endpoint = PushEndpoint::new(request.endpoint).map_err(map_web_push_error)?;
        let scope = Self::scope(&caller);
        let removed = self
            .subscriptions
            .remove_subscription(&scope, &endpoint)
            .await
            .map_err(map_web_push_error)?;
        Ok(RebornWebPushUnsubscribeResponse { removed })
    }
}

pub(super) fn web_push_unavailable() -> ProductSurfaceError {
    ProductSurfaceError::service_unavailable(false)
}

/// Map domain failures onto the sanitized product taxonomy. Caller-correctable
/// input problems become validation errors; storage trouble stays a retryable
/// service failure. Endpoint URLs never enter the payload — the domain error
/// carries hosts only.
fn map_web_push_error(error: WebPushError) -> ProductSurfaceError {
    match &error {
        WebPushError::InvalidSubscription { .. } | WebPushError::UnsupportedPushService { .. } => {
            ProductSurfaceError::validation(
                "subscription",
                super::ProductSurfaceValidationCode::InvalidValue,
            )
        }
        WebPushError::SubscriptionLimitReached { .. } => ProductSurfaceError::validation(
            "subscription",
            super::ProductSurfaceValidationCode::TooLong,
        ),
        WebPushError::InvalidScope { .. } => {
            tracing::error!(%error, "web push scope projection failed");
            ProductSurfaceError::internal_from(error)
        }
        WebPushError::Store => {
            tracing::warn!(%error, "web push subscription store failure");
            ProductSurfaceError::service_unavailable(true)
        }
        WebPushError::RuntimeUnavailable | WebPushError::RuntimeAlreadyInstalled => {
            ProductSurfaceError::service_unavailable(false)
        }
        WebPushError::PayloadTooLarge { .. } | WebPushError::Crypto { .. } => {
            tracing::error!(%error, "unexpected web push failure on the enrollment surface");
            ProductSurfaceError::internal_from(error)
        }
    }
}
