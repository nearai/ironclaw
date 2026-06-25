use std::num::{NonZeroU32, NonZeroU64};
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor,
    IngressScopeSource, ListenerClass, RateLimitPolicy, RateLimitScope, StreamingMode,
    WebSocketOriginPolicy,
};
use ironclaw_product_workflow::{IronhubRegisterRequest, RebornServicesApi};

use crate::webui_serve::PublicRouteMount;

pub(crate) const IRONHUB_REGISTER_PATH: &str = "/api/ironhub/register";
const IRONHUB_REGISTER_ROUTE_ID: &str = "ironhub.register";
// safety: 8 KiB is a non-zero literal.
const IRONHUB_REGISTER_BODY_LIMIT_BYTES: NonZeroU64 = NonZeroU64::new(8 * 1024).unwrap();
// safety: 600 requests is a non-zero literal.
const IRONHUB_REGISTER_MAX_REQUESTS: NonZeroU32 = NonZeroU32::new(600).unwrap();
// safety: 60 seconds is a non-zero literal.
const IRONHUB_REGISTER_RATE_WINDOW_SECONDS: NonZeroU32 = NonZeroU32::new(60).unwrap();

#[derive(Clone)]
pub struct IronhubRegisterRouteState {
    api: Arc<dyn RebornServicesApi>,
}

impl IronhubRegisterRouteState {
    pub fn new(api: Arc<dyn RebornServicesApi>) -> Self {
        Self { api }
    }
}

impl std::fmt::Debug for IronhubRegisterRouteState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("IronhubRegisterRouteState").finish()
    }
}

pub fn ironhub_register_route_mount(state: IronhubRegisterRouteState) -> PublicRouteMount {
    PublicRouteMount::new(
        Router::new()
            .route(IRONHUB_REGISTER_PATH, post(ironhub_register_handler))
            .with_state(state),
        ironhub_register_route_descriptors(),
    )
}

pub(crate) fn ironhub_register_route_descriptors() -> Vec<IngressRouteDescriptor> {
    let descriptor = IngressRouteDescriptor::new(
        IRONHUB_REGISTER_ROUTE_ID,
        NetworkMethod::Post,
        IRONHUB_REGISTER_PATH,
        ironhub_register_policy(),
    )
    // safety: route id/path are crate-local literals and the policy is built by the sibling helper.
    .expect("IronHub register route descriptor must validate at startup");
    vec![descriptor]
}

fn ironhub_register_policy() -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::PublicWebhook,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::WebhookSignature],
        },
        scope_source: IngressScopeSource::HostResolved,
        body_limit: BodyLimitPolicy::Limited {
            max_bytes: IRONHUB_REGISTER_BODY_LIMIT_BYTES,
        },
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::Global,
            max_requests: IRONHUB_REGISTER_MAX_REQUESTS,
            window_seconds: IRONHUB_REGISTER_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::NotApplicable,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::PublicCallback,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    // safety: policy combines validated constants and a host-resolved webhook-signature scope.
    .expect("IronHub register ingress policy must validate")
}

async fn ironhub_register_handler(
    State(state): State<IronhubRegisterRouteState>,
    body: Bytes,
) -> Response {
    let request: IronhubRegisterRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match state.api.ironhub_register(request).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(error) => StatusCode::from_u16(error.status_code)
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
            .into_response(),
    }
}
