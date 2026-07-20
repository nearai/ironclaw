//! WebUI v2 Telegram channel routes: operator setup + per-member pairing.
//!
//! Setup routes are operator-gated (tenant mismatch → 404 anti-enumeration,
//! non-operator → 403). Pairing routes are per-authenticated-member — any
//! signed-in user mints/reads/cancels THEIR OWN pairing. Composition wraps
//! the raw route fragment into its generic protected-route mount so the
//! descriptor-driven body/rate limits apply exactly like every other
//! bearer-authed v2 route.

use std::num::{NonZeroU32, NonZeroU64};
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Extension, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor,
    IngressScopeSource, ListenerClass, RateLimitPolicy, RateLimitScope, StreamingMode,
    WebSocketOriginPolicy,
};
use ironclaw_host_api::{HostApiError, NetworkMethod};
use ironclaw_product_workflow::WebUiAuthenticatedCaller;
use ironclaw_safety::SafetyLayer;
use secrecy::SecretString;
use serde::Deserialize;
use thiserror::Error;

use crate::pairing::{
    PairingIssue, TelegramPairingError, TelegramPairingService, TelegramPairingStatus,
};
use crate::setup::{
    TelegramInstallationSetupStatus, TelegramInstallationSetupUpdate, TelegramSetupService,
};

pub const WEBUI_V2_CHANNELS_TELEGRAM_SETUP_PATH: &str = "/api/webchat/v2/channels/telegram/setup";
pub const WEBUI_V2_CHANNELS_TELEGRAM_PAIRING_PATH: &str =
    "/api/webchat/v2/channels/telegram/pairing";

const TELEGRAM_SETUP_GET_ROUTE_ID: &str = "webui.v2.channels.telegram.setup.get";
const TELEGRAM_SETUP_SAVE_ROUTE_ID: &str = "webui.v2.channels.telegram.setup.save";
const TELEGRAM_SETUP_CLEAR_ROUTE_ID: &str = "webui.v2.channels.telegram.setup.clear";
const TELEGRAM_PAIRING_START_ROUTE_ID: &str = "webui.v2.channels.telegram.pairing.start";
const TELEGRAM_PAIRING_STATUS_ROUTE_ID: &str = "webui.v2.channels.telegram.pairing.status";
const TELEGRAM_PAIRING_DISCONNECT_ROUTE_ID: &str = "webui.v2.channels.telegram.pairing.disconnect";

const TELEGRAM_ROUTES_BODY_LIMIT_BYTES: NonZeroU64 = match NonZeroU64::new(16 * 1024) {
    Some(value) => value,
    None => NonZeroU64::MIN,
};
const TELEGRAM_ROUTES_MAX_REQUESTS: NonZeroU32 = match NonZeroU32::new(60) {
    Some(value) => value,
    None => NonZeroU32::MIN,
};
const TELEGRAM_ROUTES_RATE_WINDOW_SECONDS: NonZeroU32 = match NonZeroU32::new(60) {
    Some(value) => value,
    None => NonZeroU32::MIN,
};

/// Post-save extension activation trigger (mirrors the Slack setup
/// activation): the telegram host mounts wire it to lifecycle `activate` on
/// the `telegram` package with rollback on failure.
#[async_trait::async_trait]
pub trait TelegramChannelSetupActivation: Send + Sync {
    async fn activate_telegram_channel_after_setup_save(
        &self,
    ) -> Result<(), TelegramChannelSetupActivationError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("telegram channel activation failed: {reason}")]
pub struct TelegramChannelSetupActivationError {
    reason: String,
}

impl TelegramChannelSetupActivationError {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

#[derive(Clone)]
pub struct TelegramChannelRouteConfig {
    tenant_id: ironclaw_host_api::TenantId,
    operator_user_id: ironclaw_host_api::UserId,
    setup_service: Arc<TelegramSetupService>,
    pairing_service: Arc<TelegramPairingService>,
    safety_layer: Arc<SafetyLayer>,
    // arch-exempt: optional_arc, lifecycle activation is absent in supported hosts without extension management and setup remains usable with fail-closed rollback whenever the strategy is installed, plan #6159
    setup_activation: Option<Arc<dyn TelegramChannelSetupActivation>>,
}

impl TelegramChannelRouteConfig {
    pub fn new(
        setup_service: Arc<TelegramSetupService>,
        pairing_service: Arc<TelegramPairingService>,
        safety_layer: Arc<SafetyLayer>,
    ) -> Self {
        Self {
            tenant_id: setup_service.tenant_id().clone(),
            operator_user_id: setup_service.operator_user_id().clone(),
            setup_service,
            pairing_service,
            safety_layer,
            setup_activation: None,
        }
    }

    pub fn with_setup_activation(
        mut self,
        activation: Arc<dyn TelegramChannelSetupActivation>,
    ) -> Self {
        self.setup_activation = Some(activation);
        self
    }
}

/// Build the raw setup/pairing route fragment: the axum router (state
/// applied) plus the route descriptors. Composition wraps the pair into its
/// bearer-authed protected-route mount — this crate cannot name composition's
/// mount types without a cycle.
pub fn telegram_channel_route_parts(
    config: TelegramChannelRouteConfig,
) -> Result<(Router, Vec<IngressRouteDescriptor>), HostApiError> {
    let router = Router::new()
        .route(
            WEBUI_V2_CHANNELS_TELEGRAM_SETUP_PATH,
            get(get_setup_handler)
                .put(save_setup_handler)
                .delete(clear_setup_handler),
        )
        .route(
            WEBUI_V2_CHANNELS_TELEGRAM_PAIRING_PATH,
            post(start_pairing_handler)
                .get(pairing_status_handler)
                .delete(disconnect_pairing_handler),
        )
        .with_state(config);
    Ok((router, telegram_channel_route_descriptors()?))
}

fn telegram_channel_route_descriptors() -> Result<Vec<IngressRouteDescriptor>, HostApiError> {
    Ok(vec![
        descriptor(
            TELEGRAM_SETUP_GET_ROUTE_ID,
            NetworkMethod::Get,
            WEBUI_V2_CHANNELS_TELEGRAM_SETUP_PATH,
            BodyLimitPolicy::NoBody,
        )?,
        descriptor(
            TELEGRAM_SETUP_SAVE_ROUTE_ID,
            NetworkMethod::Put,
            WEBUI_V2_CHANNELS_TELEGRAM_SETUP_PATH,
            BodyLimitPolicy::Limited {
                max_bytes: TELEGRAM_ROUTES_BODY_LIMIT_BYTES,
            },
        )?,
        descriptor(
            TELEGRAM_SETUP_CLEAR_ROUTE_ID,
            NetworkMethod::Delete,
            WEBUI_V2_CHANNELS_TELEGRAM_SETUP_PATH,
            BodyLimitPolicy::NoBody,
        )?,
        descriptor(
            TELEGRAM_PAIRING_START_ROUTE_ID,
            NetworkMethod::Post,
            WEBUI_V2_CHANNELS_TELEGRAM_PAIRING_PATH,
            BodyLimitPolicy::Limited {
                max_bytes: TELEGRAM_ROUTES_BODY_LIMIT_BYTES,
            },
        )?,
        descriptor(
            TELEGRAM_PAIRING_STATUS_ROUTE_ID,
            NetworkMethod::Get,
            WEBUI_V2_CHANNELS_TELEGRAM_PAIRING_PATH,
            BodyLimitPolicy::NoBody,
        )?,
        descriptor(
            TELEGRAM_PAIRING_DISCONNECT_ROUTE_ID,
            NetworkMethod::Delete,
            WEBUI_V2_CHANNELS_TELEGRAM_PAIRING_PATH,
            BodyLimitPolicy::NoBody,
        )?,
    ])
}

fn descriptor(
    route_id: &'static str,
    method: NetworkMethod,
    path: &'static str,
    body_limit: BodyLimitPolicy,
) -> Result<IngressRouteDescriptor, HostApiError> {
    IngressRouteDescriptor::new(route_id, method, path, route_policy(body_limit)?)
}

fn route_policy(body_limit: BodyLimitPolicy) -> Result<IngressPolicy, HostApiError> {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::BearerToken],
        },
        scope_source: IngressScopeSource::AuthenticatedCaller,
        body_limit,
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::PerCaller,
            max_requests: TELEGRAM_ROUTES_MAX_REQUESTS,
            window_seconds: TELEGRAM_ROUTES_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::UserAction,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TelegramRouteError {
    #[error("bad request")]
    BadRequest,
    #[error("forbidden")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("telegram channel backend unavailable")]
    Unavailable,
    #[error("{0}")]
    UserFacing(String),
}

impl IntoResponse for TelegramRouteError {
    fn into_response(self) -> Response {
        let status = match &self {
            TelegramRouteError::BadRequest => StatusCode::BAD_REQUEST,
            TelegramRouteError::Forbidden => StatusCode::FORBIDDEN,
            TelegramRouteError::NotFound => StatusCode::NOT_FOUND,
            TelegramRouteError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            TelegramRouteError::UserFacing(_) => StatusCode::CONFLICT,
        };
        (
            status,
            Json(serde_json::json!({ "error": self.to_string() })),
        )
            .into_response()
    }
}

impl From<crate::setup::TelegramSetupError> for TelegramRouteError {
    fn from(error: crate::setup::TelegramSetupError) -> Self {
        use crate::setup::TelegramSetupError as E;
        match error {
            E::InvalidField { .. } | E::MissingField { .. } => TelegramRouteError::BadRequest,
            E::PublicUrlMissing | E::BotApi { .. } => {
                // Admin-actionable outcomes: surface the sanitized message.
                TelegramRouteError::UserFacing(error.to_string())
            }
            E::ConcurrentUpdate => TelegramRouteError::UserFacing(error.to_string()),
            E::StoreUnavailable | E::SecretStoreUnavailable { .. } => {
                TelegramRouteError::Unavailable
            }
        }
    }
}

impl From<TelegramPairingError> for TelegramRouteError {
    fn from(error: TelegramPairingError) -> Self {
        match error {
            TelegramPairingError::NotConfigured => TelegramRouteError::UserFacing(
                "an administrator must configure the Telegram bot first".to_string(),
            ),
            TelegramPairingError::ConcurrentUpdate => {
                TelegramRouteError::UserFacing(error.to_string())
            }
            TelegramPairingError::StoreUnavailable { .. }
            | TelegramPairingError::Setup { .. }
            | TelegramPairingError::ContinuationDispatch { .. } => TelegramRouteError::Unavailable,
        }
    }
}

impl From<TelegramChannelSetupActivationError> for TelegramRouteError {
    fn from(error: TelegramChannelSetupActivationError) -> Self {
        // The activation reason originates in the lifecycle backend and can
        // carry internal detail; the admin gets a stable category while the
        // cause stays in protected diagnostics.
        tracing::debug!(reason = %error, "telegram setup activation failed; setup rolled back");
        TelegramRouteError::UserFacing(
            "Telegram channel activation failed — the setup change was rolled back.".to_string(),
        )
    }
}

fn ensure_authorized_operator(
    config: &TelegramChannelRouteConfig,
    caller: &WebUiAuthenticatedCaller,
) -> Result<(), TelegramRouteError> {
    if caller.tenant_id != config.tenant_id {
        // 404, not 403: a cross-tenant probe must not learn the route exists.
        return Err(TelegramRouteError::NotFound);
    }
    if !caller.operator_webui_config || caller.user_id != config.operator_user_id {
        return Err(TelegramRouteError::Forbidden);
    }
    Ok(())
}

fn ensure_same_tenant_member(
    config: &TelegramChannelRouteConfig,
    caller: &WebUiAuthenticatedCaller,
) -> Result<(), TelegramRouteError> {
    if caller.tenant_id != config.tenant_id {
        return Err(TelegramRouteError::NotFound);
    }
    Ok(())
}

fn scan_admin_field(
    config: &TelegramChannelRouteConfig,
    field: &'static str,
    value: &str,
) -> Result<(), TelegramRouteError> {
    let validation = config.safety_layer.validate_input(value);
    if !validation.is_valid {
        tracing::debug!(field, "telegram setup field failed safety validation");
        return Err(TelegramRouteError::BadRequest);
    }
    let sanitized = config.safety_layer.sanitize_tool_output(field, value);
    if !sanitized.warnings.is_empty() {
        tracing::debug!(
            field,
            warnings = sanitized.warnings.len(),
            "telegram setup field failed injection scan"
        );
        return Err(TelegramRouteError::BadRequest);
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TelegramSetupSaveRequest {
    bot_token: Option<String>,
    webhook_url: Option<String>,
}

impl TelegramSetupSaveRequest {
    fn into_update(self) -> TelegramInstallationSetupUpdate {
        TelegramInstallationSetupUpdate {
            bot_token: self
                .bot_token
                .filter(|token| !token.trim().is_empty())
                .map(SecretString::from),
            webhook_url_override: self.webhook_url.filter(|value| !value.trim().is_empty()),
        }
    }
}

async fn get_setup_handler(
    State(config): State<TelegramChannelRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<TelegramInstallationSetupStatus>, TelegramRouteError> {
    ensure_authorized_operator(&config, &caller)?;
    Ok(Json(config.setup_service.status().await?))
}

async fn save_setup_handler(
    State(config): State<TelegramChannelRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<TelegramSetupSaveRequest>,
) -> Result<Json<TelegramInstallationSetupStatus>, TelegramRouteError> {
    ensure_authorized_operator(&config, &caller)?;
    if let Some(webhook_url) = request.webhook_url.as_deref() {
        scan_admin_field(&config, "webhook_url", webhook_url)?;
    }
    let (previous_setup, saved_setup) = config
        .setup_service
        .save_with_previous(request.into_update())
        .await?;
    if let Some(activation) = config.setup_activation.as_ref()
        && let Err(error) = activation
            .activate_telegram_channel_after_setup_save()
            .await
    {
        config
            .setup_service
            .rollback_failed_activation_save(&saved_setup, previous_setup.as_ref())
            .await?;
        return Err(error.into());
    }
    Ok(Json(config.setup_service.status().await?))
}

async fn clear_setup_handler(
    State(config): State<TelegramChannelRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<StatusCode, TelegramRouteError> {
    ensure_authorized_operator(&config, &caller)?;
    config.setup_service.clear().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn start_pairing_handler(
    State(config): State<TelegramChannelRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<PairingIssue>, TelegramRouteError> {
    ensure_same_tenant_member(&config, &caller)?;
    Ok(Json(
        config
            .pairing_service
            .issue_or_rotate(&caller.user_id)
            .await?,
    ))
}

async fn pairing_status_handler(
    State(config): State<TelegramChannelRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<TelegramPairingStatus>, TelegramRouteError> {
    ensure_same_tenant_member(&config, &caller)?;
    Ok(Json(
        config.pairing_service.status_for(&caller.user_id).await?,
    ))
}

async fn disconnect_pairing_handler(
    State(config): State<TelegramChannelRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<StatusCode, TelegramRouteError> {
    ensure_same_tenant_member(&config, &caller)?;
    config.pairing_service.unpair(&caller.user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Handler-tier tests: drive the REAL router from
/// [`telegram_channel_route_parts`] via `tower::ServiceExt::oneshot`, with the
/// authenticated caller injected exactly like composition's bearer middleware
/// injects it. Manual-QA coverage: each test names the `qa-telegram:*` rows it
/// automates (the hermetic slice; browser rendering of the same surfaces stays
/// in the Browser E2E tier).
#[cfg(test)]
#[path = "channel_routes_tests.rs"]
mod handler_tests;
