//! Public NEAR AI login callback route.
//!
//! NEAR AI completes its own GitHub/Google OAuth and redirects the browser to
//! this server's `…/nearai/{state}/auth/callback?token=…`. The route consumes
//! the state from an authenticated login-start flow, stores the session token on
//! the live provider, makes NEAR AI active, hot-swaps the running provider, and
//! bounces the tab to the app. It does not require a bearer token — the browser
//! arrives straight from NEAR AI — but the descriptor still records the
//! one-time OAuth state guard and host-resolved effect path before mutation.

use std::num::NonZeroU32;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::response::Redirect;
use axum::routing::get;
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor, ListenerClass,
    RateLimitPolicy, RateLimitScope, StreamingMode, WebSocketOriginPolicy,
};
use ironclaw_host_api::{IngressScopeSource, NetworkMethod};
use ironclaw_reborn_config::RebornBootConfig;
use serde::Deserialize;

use crate::LlmReloadTrigger;
use crate::llm_admin::llm_config_service::{
    NEARAI_LOGIN_CALLBACK_PATH, NearAiLoginStateStore, apply_nearai_login,
};
use ironclaw_host_ingress::PublicRouteMount;

const NEARAI_CALLBACK_RATE_WINDOW_SECONDS: NonZeroU32 = match NonZeroU32::new(60) {
    Some(value) => value,
    // SAFETY: 60 is a non-zero literal rate-limit window.
    None => unreachable!(),
};
const NEARAI_CALLBACK_RATE_MAX: NonZeroU32 = match NonZeroU32::new(60) {
    Some(value) => value,
    // SAFETY: 60 is a non-zero literal rate limit.
    None => unreachable!(),
};
const NEARAI_LOGIN_SUCCESS_REDIRECT: &str = "/chat";
const NEARAI_LOGIN_ERROR_REDIRECT: &str = "/settings/inference?nearai_login=error";

#[derive(Clone)]
struct NearAiCallbackState {
    session: Arc<ironclaw_llm::SessionManager>,
    reload: Arc<dyn LlmReloadTrigger>,
    boot: RebornBootConfig,
    states: Arc<NearAiLoginStateStore>,
}

#[derive(Deserialize)]
struct CallbackQuery {
    #[serde(default)]
    token: Option<String>,
}

async fn nearai_callback(
    State(state): State<NearAiCallbackState>,
    Path(login_state): Path<String>,
    Query(query): Query<CallbackQuery>,
) -> Redirect {
    if !state.states.consume(&login_state).await {
        return Redirect::to(NEARAI_LOGIN_ERROR_REDIRECT);
    }
    let Some(token) = query.token.filter(|token| !token.trim().is_empty()) else {
        return Redirect::to(NEARAI_LOGIN_ERROR_REDIRECT);
    };
    match apply_nearai_login(&state.session, &state.boot, state.reload.as_ref(), &token).await {
        Ok(()) => Redirect::to(NEARAI_LOGIN_SUCCESS_REDIRECT),
        Err(error) => {
            tracing::warn!(%error, "NEAR AI login callback failed");
            Redirect::to(NEARAI_LOGIN_ERROR_REDIRECT)
        }
    }
}

/// Build the public NEAR AI login callback mount for composition to merge via
/// `ironclaw_webui::WebuiServeConfig::with_public_route_mount`.
pub(crate) fn nearai_login_callback_mount(
    session: Arc<ironclaw_llm::SessionManager>,
    reload: Arc<dyn LlmReloadTrigger>,
    boot: RebornBootConfig,
    states: Arc<NearAiLoginStateStore>,
) -> PublicRouteMount {
    let router = Router::new()
        .route(NEARAI_LOGIN_CALLBACK_PATH, get(nearai_callback))
        .with_state(NearAiCallbackState {
            session,
            reload,
            boot,
            states,
        });
    PublicRouteMount::new(router, vec![nearai_callback_descriptor()])
}

fn nearai_callback_descriptor() -> IngressRouteDescriptor {
    let policy = IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::OAuthCallback,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::OAuthState],
        },
        scope_source: IngressScopeSource::HostResolved,
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
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .expect("nearai login callback policy must validate"); // safety: OAuthCallback + OAuthState + HostResolved is the host callback shape; handler validation consumes one-time login state before credential/provider mutation.
    IngressRouteDescriptor::new(
        "webui.v2.nearai_login_callback".to_string(),
        NetworkMethod::Get,
        NEARAI_LOGIN_CALLBACK_PATH.to_string(),
        policy,
    )
    .expect("nearai login callback descriptor must validate") // safety: route id/path are crate-local literals, and the policy above validates the OAuth callback effect shape.
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use ironclaw_llm::{SessionConfig, SessionManager};
    use ironclaw_reborn_config::{RebornHome, RebornProfile};
    use tower::ServiceExt;

    use super::*;

    #[derive(Default)]
    struct RecordingReload {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl LlmReloadTrigger for RecordingReload {
        async fn reload(&self) -> Result<(), String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn callback_mount(
        root: &std::path::Path,
        states: Arc<NearAiLoginStateStore>,
        reload: Arc<RecordingReload>,
    ) -> PublicRouteMount {
        let home =
            RebornHome::resolve_from_env_parts(Some(root.as_os_str().to_os_string()), None, None)
                .expect("temporary Reborn home is valid");
        let session = Arc::new(SessionManager::new(SessionConfig {
            auth_base_url: "https://private.near.ai".to_string(),
            session_path: root.join("nearai-session.json"),
        }));
        nearai_login_callback_mount(
            session,
            reload,
            RebornBootConfig::new(home, RebornProfile::LocalDev),
            states,
        )
    }

    #[test]
    fn nearai_callback_descriptor_records_state_guarded_effectful_workflow() {
        let descriptor = nearai_callback_descriptor();
        let policy = descriptor.policy();

        assert_eq!(policy.listener_class(), ListenerClass::OAuthCallback);
        assert!(matches!(
            policy.auth(),
            IngressAuthPolicy::Required { schemes }
                if schemes.as_slice() == [IngressAuthScheme::OAuthState]
        ));
        assert_eq!(policy.scope_source(), IngressScopeSource::HostResolved);
        assert_eq!(policy.effect_path(), &AllowedEffectPath::ProductWorkflow);
    }

    #[tokio::test]
    async fn nearai_callback_redirects_success_to_root_chat() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let states = Arc::new(NearAiLoginStateStore::new());
        let login_state = states.issue().await;
        let reload = Arc::new(RecordingReload::default());
        let mount = callback_mount(temp.path(), states, Arc::clone(&reload));

        let response = mount
            .router
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/webchat/v2/llm/nearai/{login_state}/auth/callback?token=session-token"
                    ))
                    .body(Body::empty())
                    .expect("callback request"),
            )
            .await
            .expect("callback response");

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok()),
            Some("/chat"),
        );
        assert_eq!(reload.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn nearai_callback_redirects_invalid_or_incomplete_login_to_root_settings() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let states = Arc::new(NearAiLoginStateStore::new());
        let valid_state = states.issue().await;
        let reload = Arc::new(RecordingReload::default());
        let mount = callback_mount(temp.path(), states, Arc::clone(&reload));

        for uri in [
            "/api/webchat/v2/llm/nearai/unknown/auth/callback?token=session-token".to_string(),
            format!("/api/webchat/v2/llm/nearai/{valid_state}/auth/callback"),
        ] {
            let response = mount
                .router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .body(Body::empty())
                        .expect("callback request"),
                )
                .await
                .expect("callback response");

            assert_eq!(response.status(), StatusCode::SEE_OTHER);
            assert_eq!(
                response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|value| value.to_str().ok()),
                Some("/settings/inference?nearai_login=error"),
            );
        }
        assert_eq!(reload.calls.load(Ordering::SeqCst), 0);
    }
}
