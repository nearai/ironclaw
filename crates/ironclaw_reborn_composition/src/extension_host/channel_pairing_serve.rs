//! Bearer-authed generic pairing routes for `WebGeneratedCode` channels.
//!
//! Mounted through the shared [`ProtectedRouteMount`] seam (inside the WebUI
//! bearer-auth layer, with descriptor-driven body/rate limits):
//!
//! - `POST /api/webchat/v2/extensions/{extension_id}/pairing/mint` — mint or
//!   rotate the caller's code (fails closed when the channel is inactive).
//! - `GET  /api/webchat/v2/extensions/{extension_id}/pairing/status` — the
//!   caller's connection state plus any live pending code (also retries a
//!   durable pairing-completion outbox entry).
//! - `POST /api/webchat/v2/extensions/{extension_id}/pairing/unpair` —
//!   disconnect the caller (bindings, DM target, pending codes).
//!
//! Responses are sanitized DTOs; store/dispatch failures never leak backend
//! detail. An extension without a registered pairing service is a 404 — the
//! route surface is generic, the registry decides which extensions pair.

use std::sync::Arc;

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor, ListenerClass,
    RateLimitPolicy, RateLimitScope, StreamingMode, WebSocketOriginPolicy,
};
use ironclaw_host_api::{NetworkMethod, ProductSurfaceCaller};
use ironclaw_product::{
    ChannelPairingError, ChannelPairingIssue, ChannelPairingRegistry, ChannelPairingService,
    ChannelPairingStatus,
};
use serde::Serialize;

use crate::webui::route_mounts::ProtectedRouteMount;

const MINT_PATH: &str = "/api/webchat/v2/extensions/{extension_id}/pairing/mint";
const STATUS_PATH: &str = "/api/webchat/v2/extensions/{extension_id}/pairing/status";
const UNPAIR_PATH: &str = "/api/webchat/v2/extensions/{extension_id}/pairing/unpair";

const PAIRING_MUTATION_BODY_LIMIT_BYTES: std::num::NonZero<u64> =
    std::num::NonZero::new(4 * 1024).expect("nonzero literal"); // safety: const-evaluated — a zero literal fails the build, never runtime
const PAIRING_MUTATION_MAX_REQUESTS: std::num::NonZero<u32> =
    std::num::NonZero::new(60).expect("nonzero literal"); // safety: const-evaluated — a zero literal fails the build, never runtime
const PAIRING_READ_MAX_REQUESTS: std::num::NonZero<u32> =
    std::num::NonZero::new(120).expect("nonzero literal"); // safety: const-evaluated — a zero literal fails the build, never runtime
const PAIRING_RATE_WINDOW_SECONDS: std::num::NonZero<u32> =
    std::num::NonZero::new(60).expect("nonzero literal"); // safety: const-evaluated — a zero literal fails the build, never runtime

#[derive(Clone)]
struct PairingRouteState {
    registry: Arc<ChannelPairingRegistry>,
}

/// Build the protected pairing route mount over the composed registry.
pub(crate) fn channel_pairing_route_mount(
    registry: Arc<ChannelPairingRegistry>,
) -> ProtectedRouteMount {
    let state = PairingRouteState { registry };
    let router = Router::new()
        .route(MINT_PATH, post(mint))
        .route(STATUS_PATH, get(status))
        .route(UNPAIR_PATH, post(unpair))
        .with_state(state);
    ProtectedRouteMount::new(
        router,
        vec![
            descriptor(
                "webui.v2.extension_pairing_mint",
                NetworkMethod::Post,
                MINT_PATH,
                mutation_policy(),
            ),
            descriptor(
                "webui.v2.extension_pairing_status",
                NetworkMethod::Get,
                STATUS_PATH,
                read_policy(),
            ),
            descriptor(
                "webui.v2.extension_pairing_unpair",
                NetworkMethod::Post,
                UNPAIR_PATH,
                mutation_policy(),
            ),
        ],
    )
}

fn descriptor(
    route_id: &str,
    method: NetworkMethod,
    pattern: &str,
    policy: IngressPolicy,
) -> IngressRouteDescriptor {
    IngressRouteDescriptor::new(route_id.to_string(), method, pattern.to_string(), policy)
        .expect("channel pairing route descriptor must validate at startup") // safety: ids/patterns are crate-local literals; policies come from the sibling helpers below.
}

fn mutation_policy() -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::BearerToken],
        },
        scope_source: ironclaw_host_api::IngressScopeSource::AuthenticatedCaller,
        body_limit: BodyLimitPolicy::Limited {
            max_bytes: PAIRING_MUTATION_BODY_LIMIT_BYTES,
        },
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::PerCaller,
            max_requests: PAIRING_MUTATION_MAX_REQUESTS,
            window_seconds: PAIRING_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::UserAction,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .expect("channel pairing mutation policy must validate") // safety: same authenticated local product-workflow shape the product-auth mutations use.
}

fn read_policy() -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::BearerToken],
        },
        scope_source: ironclaw_host_api::IngressScopeSource::AuthenticatedCaller,
        body_limit: BodyLimitPolicy::NoBody,
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::PerCaller,
            max_requests: PAIRING_READ_MAX_REQUESTS,
            window_seconds: PAIRING_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::UserAction,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .expect("channel pairing read policy must validate") // safety: bearer-authed NoBody read, the flow-status shape.
}

#[derive(Debug, Serialize)]
struct PairingIssueBody {
    code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    deep_link: Option<String>,
    expires_at: chrono::DateTime<chrono::Utc>,
}

impl From<ChannelPairingIssue> for PairingIssueBody {
    fn from(issue: ChannelPairingIssue) -> Self {
        Self {
            code: issue.code.as_str().to_string(),
            deep_link: issue.deep_link,
            expires_at: issue.expires_at,
        }
    }
}

#[derive(Debug, Serialize)]
struct PairingStatusBody {
    connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pending: Option<PairingIssueBody>,
}

impl From<ChannelPairingStatus> for PairingStatusBody {
    fn from(status: ChannelPairingStatus) -> Self {
        Self {
            connected: status.connected,
            pending: status.pending.map(PairingIssueBody::from),
        }
    }
}

#[derive(Debug, Serialize)]
struct PairingErrorBody {
    error: &'static str,
}

fn error_response(status: StatusCode, error: &'static str) -> Response {
    (status, Json(PairingErrorBody { error })).into_response()
}

fn map_pairing_error(error: ChannelPairingError) -> Response {
    match error {
        ChannelPairingError::NotConfigured => {
            error_response(StatusCode::CONFLICT, "not_configured")
        }
        ChannelPairingError::StoreUnavailable { reason } => {
            tracing::debug!(
                target: "ironclaw::reborn::channel_pairing",
                %reason,
                "pairing store unavailable"
            );
            error_response(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable")
        }
        ChannelPairingError::ContinuationDispatch { reason } => {
            tracing::debug!(
                target: "ironclaw::reborn::channel_pairing",
                %reason,
                "pairing continuation dispatch failed"
            );
            error_response(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable")
        }
    }
}

fn service_for(
    state: &PairingRouteState,
    extension_id: &str,
) -> Option<Arc<ChannelPairingService>> {
    state.registry.get(extension_id)
}

fn unknown_extension() -> Response {
    error_response(StatusCode::NOT_FOUND, "unknown_extension")
}

async fn mint(
    State(state): State<PairingRouteState>,
    Path(extension_id): Path<String>,
    Extension(caller): Extension<ProductSurfaceCaller>,
) -> Response {
    let Some(service) = service_for(&state, &extension_id) else {
        return unknown_extension();
    };
    match service.issue_or_rotate(&caller.user_id).await {
        Ok(issue) => (StatusCode::OK, Json(PairingIssueBody::from(issue))).into_response(),
        Err(error) => map_pairing_error(error),
    }
}

async fn status(
    State(state): State<PairingRouteState>,
    Path(extension_id): Path<String>,
    Extension(caller): Extension<ProductSurfaceCaller>,
) -> Response {
    let Some(service) = service_for(&state, &extension_id) else {
        return unknown_extension();
    };
    match service.status_for(&caller.user_id).await {
        Ok(status) => (StatusCode::OK, Json(PairingStatusBody::from(status))).into_response(),
        Err(error) => map_pairing_error(error),
    }
}

async fn unpair(
    State(state): State<PairingRouteState>,
    Path(extension_id): Path<String>,
    Extension(caller): Extension<ProductSurfaceCaller>,
) -> Response {
    let Some(service) = service_for(&state, &extension_id) else {
        return unknown_extension();
    };
    match service.unpair(&caller.user_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => map_pairing_error(error),
    }
}
