//! Public NEAR AI login callback route.
//!
//! NEAR AI completes its own GitHub/Google OAuth and redirects the browser to
//! this server's `…/nearai/auth/callback?token=…`. The route stores the session
//! token on the live provider, makes NEAR AI active, hot-swaps the running
//! provider, and bounces the tab to the app. It is PUBLIC (no bearer — the
//! browser arrives straight from NEAR AI) and is merged via `PublicRouteMount`
//! the same way the SSO callbacks are, so it inherits the per-route policy
//! middleware.

use std::num::NonZeroU32;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Query, State};
use axum::response::Redirect;
use axum::routing::get;
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressJustification, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor, ListenerClass,
    RateLimitPolicy, RateLimitScope, StreamingMode, WebSocketOriginPolicy,
};
use ironclaw_host_api::{IngressScopeSource, NetworkMethod};
use ironclaw_reborn_config::RebornBootConfig;
use serde::Deserialize;

use crate::LlmReloadTrigger;
use crate::llm_config_service::{NEARAI_LOGIN_CALLBACK_PATH, apply_nearai_login};
use crate::webui_serve::PublicRouteMount;

const NEARAI_CALLBACK_RATE_WINDOW_SECONDS: NonZeroU32 = NonZeroU32::new(60).expect("60 != 0");
const NEARAI_CALLBACK_RATE_MAX: NonZeroU32 = NonZeroU32::new(60).expect("60 != 0");

#[derive(Clone)]
struct NearAiCallbackState {
    session: Arc<ironclaw_llm::SessionManager>,
    reload: Arc<dyn LlmReloadTrigger>,
    boot: RebornBootConfig,
}

#[derive(Deserialize)]
struct CallbackQuery {
    #[serde(default)]
    token: Option<String>,
}

async fn nearai_callback(
    State(state): State<NearAiCallbackState>,
    Query(query): Query<CallbackQuery>,
) -> Redirect {
    let Some(token) = query.token.filter(|token| !token.trim().is_empty()) else {
        return Redirect::to("/v2/settings/inference?nearai_login=error");
    };
    match apply_nearai_login(&state.session, &state.boot, state.reload.as_ref(), &token).await {
        Ok(()) => Redirect::to("/v2/chat"),
        Err(error) => {
            tracing::warn!(%error, "NEAR AI login callback failed");
            Redirect::to("/v2/settings/inference?nearai_login=error")
        }
    }
}

/// Build the public NEAR AI login callback mount for composition to merge via
/// [`crate::webui_serve::WebuiServeConfig::with_public_route_mount`].
pub fn nearai_login_callback_mount(
    session: Arc<ironclaw_llm::SessionManager>,
    reload: Arc<dyn LlmReloadTrigger>,
    boot: RebornBootConfig,
) -> PublicRouteMount {
    let router = Router::new()
        .route(NEARAI_LOGIN_CALLBACK_PATH, get(nearai_callback))
        .with_state(NearAiCallbackState {
            session,
            reload,
            boot,
        });
    PublicRouteMount::new(router, vec![nearai_callback_descriptor()])
}

fn nearai_callback_descriptor() -> IngressRouteDescriptor {
    let policy = IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::OAuthCallback,
        auth: IngressAuthPolicy::Public {
            justification: IngressJustification::new(
                "webui-v2 nearai login",
                "NEAR AI redirects the browser here with the session token after \
                 its own GitHub/Google login; the route has no session yet and \
                 stores the operator LLM credential",
            )
            .expect("nearai login justification must validate"),
        },
        scope_source: IngressScopeSource::PublicRoute,
        body_limit: BodyLimitPolicy::NoBody,
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::PerIp,
            max_requests: NEARAI_CALLBACK_RATE_MAX,
            window_seconds: NEARAI_CALLBACK_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::NotApplicable,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::PublicCallback,
        effect_path: AllowedEffectPath::NoEffect,
    })
    .expect("nearai login callback policy must validate"); // safety: OAuthCallback + Public + NoEffect is the permitted public-callback shape (same as the SSO callback).
    IngressRouteDescriptor::new(
        "webui.v2.nearai_login_callback".to_string(),
        NetworkMethod::Get,
        NEARAI_LOGIN_CALLBACK_PATH.to_string(),
        policy,
    )
    .expect("nearai login callback descriptor must validate")
}
