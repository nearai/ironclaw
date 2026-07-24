#![forbid(unsafe_code)]

//! Host-owned listener binding + serve loop for the Reborn WebChat v2
//! HTTP gateway.
//!
//! `webui_v2_app` returns a fully composed axum [`Router`] but deliberately
//! stops before listener binding. This crate owns both the WebUI ingress
//! assembly and the host-owned serve loop helper that binds a `TcpListener`
//! and runs graceful shutdown.
//!
//! Path A (`docs/reborn/how-to-port-channel-to-reborn.md`) native
//! host-surface invariants:
//!
//! - Host auth stays host-owned: `WebuiAuthenticator` implementations
//!   live here, not in product/API crates.
//! - No external-protocol shims: no `ProductAdapter`, no
//!   `ProtocolAuthEvidence`, no fake `ExternalActorRef`.
//! - No v1 dependency: this crate carries no `src/` import and never
//!   reads v1 secrets / settings / DB.

mod auth;
mod cli_token_login;
mod oidc;
mod session;
mod signed_session_login;
// Folded in from the former `ironclaw_webui_v2` crate: the WebChat v2 HTTP
// route surface + static SPA bundle. Public so this crate's own middleware,
// composition's `#[cfg(test)]` unit tests, and downstream test crates can
// reach the route/handler/descriptor items.
pub mod webui_v2;
// Reborn WebChat v2 HTTP gateway assembly + middleware. `webui_v2_app`
// composes the fully wired axum Router (auth + rate/body limit + CORS +
// security headers + the v2 route surface); the middleware modules back it.
mod webui_body_limit;
mod webui_operator_auth;
mod webui_rate_limit;
mod webui_route_match;
mod webui_serve;
mod webui_ws_origin;

// WebChat v2 gateway assembly + the host-auth vocabulary it carries.
// Route-mount carriers live in `ironclaw_host_ingress`: composition can build
// mounts and this ingress crate can consume them without a reverse dependency
// on WebUI.
pub use ironclaw_host_ingress::{
    ProtectedRouteMount, PublicRouteDrain, PublicRouteDrains, PublicRouteMount, SplitRouteMount,
};
pub use webui_rate_limit::RateLimitConfigError;
pub use webui_serve::{
    WebuiAuthentication, WebuiAuthenticator, WebuiServeConfig, WebuiServeConfigError,
    WebuiServeError, WebuiV2App, webui_v2_app, webui_v2_app_with_lifecycle,
};

#[cfg(any(test, feature = "test-support"))]
pub use auth::EmailUserDirectory;
pub use auth::{
    GitHubOAuthConfig, GitHubProvider, GoogleOAuthConfig, GoogleProvider, OAuthError,
    OAuthProvider, OAuthProviderName, OAuthProviderNameError, OAuthRouterConfig, OAuthUserProfile,
    ProviderInitError, UserDirectory, UserDirectoryError, empty_webui_v2_auth_providers_mount,
    webui_v2_auth_router,
};
// Host-owned CLI-token bootstrap login (`GET /login?token=`); shares the
// OAuth surface's bearer/ticket-exchange contract (`POST
// /auth/session/exchange`) — no new SPA code needed. See
// `cli_token_login.rs` module docs.
pub use cli_token_login::{CliTokenLoginConfig, build_cli_token_login};
pub use oidc::{
    AudienceClaim, ClaimToUserIdFn, IdTokenClaims, OidcAuthenticator, OidcAuthenticatorConfig,
    OidcAuthenticatorError,
};
pub use session::{SessionAuthenticator, SessionRecord, SessionStoreError};
// Host-owned signed-token login surface (production-suitable, non-dev):
// the standalone `serve` binary supplies env config and calls the
// builder; the auth/session model lives here, not in the command crate.
pub use signed_session_login::{
    CompositeAuthenticator, SignedSessionLoginConfig, SignedSessionLoginWiring,
    SignedTokenSessionStore, build_signed_session_login, signed_session_store,
};

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{any, get},
};
use ironclaw_host_api::UserId;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tower::ServiceExt;

/// Errors raised while running the host serve loop.
#[derive(Debug, Error)]
pub enum RebornWebuiServeError {
    #[error("failed to bind WebUI listener at {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    #[error("WebUI serve loop terminated with error: {0}")]
    Serve(#[source] std::io::Error),
    #[error("failed to read bound listener address: {0}")]
    LocalAddr(#[source] std::io::Error),
}

/// Owner-supplied input to [`serve_webui_v2`].
///
/// The `Router` is whatever `webui_v2_app` returned; the host binary
/// owns address resolution, signal handling, and (optionally) the
/// `bound_addr_tx` channel that surfaces the actual bound port back to
/// the caller — useful for tests that pass `0` as the port and need to
/// learn which port the kernel picked.
pub struct RebornWebuiServeOptions {
    pub addr: SocketAddr,
    pub router: Router,
    pub shutdown: tokio::sync::oneshot::Receiver<()>,
    pub bound_addr_tx: Option<tokio::sync::oneshot::Sender<SocketAddr>>,
}

/// Handle used by host startup code to publish the real WebUI router
/// after runtime assembly finishes.
#[derive(Clone)]
pub struct DeferredWebuiRouterHandle {
    router_tx: watch::Sender<Option<Router>>,
}

/// Errors raised while publishing the ready router to a deferred
/// startup listener.
#[derive(Debug, Error)]
pub enum DeferredWebuiRouterError {
    #[error("deferred WebUI startup listener stopped before the runtime router became ready")]
    ListenerStopped,
}

/// Build a startup router for orchestrator healthchecks while the
/// host-owned runtime is still assembling.
///
/// `/api/health` returns healthy immediately. Every other route returns
/// 503 until [`DeferredWebuiRouterHandle::publish_ready_router`] is
/// called, then delegates each request to the real composed WebUI
/// router without rebinding the listener.
pub fn deferred_webui_v2_startup_router() -> (Router, DeferredWebuiRouterHandle) {
    let (router_tx, router_rx) = watch::channel(None);
    let state = DeferredWebuiRouterState { router_rx };
    let router = Router::new()
        .route("/api/health", get(deferred_webui_health_handler))
        .fallback(any(deferred_webui_handler))
        .with_state(state);
    (router, DeferredWebuiRouterHandle { router_tx })
}

impl DeferredWebuiRouterHandle {
    pub fn publish_ready_router(&self, router: Router) -> Result<(), DeferredWebuiRouterError> {
        self.router_tx
            .send(Some(router))
            .map_err(|_| DeferredWebuiRouterError::ListenerStopped)
    }
}

#[derive(Clone)]
struct DeferredWebuiRouterState {
    router_rx: watch::Receiver<Option<Router>>,
}

#[derive(Serialize)]
struct DeferredWebuiHealthResponse {
    status: &'static str,
    channel: &'static str,
}

async fn deferred_webui_health_handler() -> Json<DeferredWebuiHealthResponse> {
    Json(DeferredWebuiHealthResponse {
        status: "healthy",
        channel: "reborn",
    })
}

async fn deferred_webui_handler(
    State(state): State<DeferredWebuiRouterState>,
    request: Request,
) -> Response {
    let Some(router) = state.router_rx.borrow().clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Reborn runtime is starting",
        )
            .into_response();
    };

    router
        .oneshot(request)
        .await
        .unwrap_or_else(|err: Infallible| match err {})
}

/// Bind a `TcpListener` at `opts.addr`, run the axum serve loop with
/// the composed `Router`, and wait for `opts.shutdown` to fire before
/// returning. Graceful shutdown gives in-flight requests a chance to
/// complete before the listener closes.
pub async fn serve_webui_v2(opts: RebornWebuiServeOptions) -> Result<(), RebornWebuiServeError> {
    let RebornWebuiServeOptions {
        addr,
        router,
        shutdown,
        bound_addr_tx,
    } = opts;

    let listener = TcpListener::bind(addr)
        .await
        .map_err(|source| RebornWebuiServeError::Bind { addr, source })?;

    let bound = listener
        .local_addr()
        .map_err(RebornWebuiServeError::LocalAddr)?;
    tracing::info!(
        target = "ironclaw::reborn::webui_ingress",
        %bound,
        "WebChat v2 listener bound",
    );
    if let Some(tx) = bound_addr_tx {
        // Receiver may have been dropped (test exited early). Ignore
        // — that's a test bug, not a serve-loop concern.
        let _ = tx.send(bound);
    }

    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        // If the host drops the sender without firing, treat that
        // as "shutdown requested" so the serve loop returns
        // cleanly rather than running forever.
        let _ = shutdown.await;
        tracing::info!(
            target = "ironclaw::reborn::webui_ingress",
            "WebChat v2 graceful shutdown signal received",
        );
    })
    .await
    .map_err(RebornWebuiServeError::Serve)
}

/// Authenticator that compares the bearer token from the request
/// against a single host-installation token loaded from an environment
/// variable. Intended for the standalone `ironclaw-reborn` deployment
/// (single operator, single user) and for local dev.
///
/// Production deployments with multiple users / sessions / OIDC should
/// use a different `WebuiAuthenticator` impl. This one is deliberately
/// minimal.
#[derive(Debug)]
pub struct EnvBearerAuthenticator {
    /// `SecretString` `Debug` impl is redacted, so no token material
    /// leaks into trace logs / panics that print this struct.
    token: SecretString,
    user_id: UserId,
}

impl EnvBearerAuthenticator {
    /// Build an authenticator that accepts exactly `token` and maps a
    /// successful match to `user_id`. The token must be non-empty;
    /// passing an empty token is treated as a configuration error
    /// because a literal `Authorization: Bearer ` (no token) would
    /// then succeed.
    pub fn new(token: SecretString, user_id: UserId) -> Result<Self, EnvBearerConfigError> {
        if token.expose_secret().is_empty() {
            return Err(EnvBearerConfigError::EmptyToken);
        }
        Ok(Self { token, user_id })
    }
}

/// Errors raised when constructing [`EnvBearerAuthenticator`] from
/// host config.
#[derive(Debug, Error)]
pub enum EnvBearerConfigError {
    #[error("bearer token must not be empty")]
    EmptyToken,
}

#[async_trait]
impl WebuiAuthenticator for EnvBearerAuthenticator {
    async fn authenticate(&self, candidate: &str) -> Option<WebuiAuthentication> {
        // Constant-time comparison so an attacker cannot use response
        // timing to learn the prefix of the configured token. Both
        // operands are coerced to `&[u8]` of the same length to make
        // the underlying `subtle::ConstantTimeEq` impl meaningful
        // (`subtle` returns "not equal" for length mismatch in
        // constant time too).
        let expected = self.token.expose_secret().as_bytes();
        let candidate = candidate.as_bytes();
        if expected.ct_eq(candidate).into() {
            Some(WebuiAuthentication::operator(self.user_id.clone()))
        } else {
            None
        }
    }

    fn mounts_operator_webui_config_routes(&self) -> bool {
        true
    }
}

/// Concrete type alias for the trait object the standalone CLI builds
/// when constructing `WebuiServeConfig`. Exposed so binary code can
/// avoid spelling out `Arc<dyn WebuiAuthenticator>` at every call site.
pub type SharedWebuiAuthenticator = Arc<dyn WebuiAuthenticator>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn env_bearer_authenticator_accepts_exact_token() {
        let auth = EnvBearerAuthenticator::new(
            SecretString::from("right-token".to_string()),
            UserId::new("user-alpha").expect("user"),
        )
        .expect("auth");
        let result = auth.authenticate("right-token").await;
        assert_eq!(
            result.as_ref().map(|auth| auth.user_id.as_str()),
            Some("user-alpha")
        );
        assert_eq!(
            result
                .as_ref()
                .map(|auth| auth.capabilities.operator_webui_config),
            Some(true)
        );
    }

    #[tokio::test]
    async fn env_bearer_authenticator_rejects_wrong_token() {
        let auth = EnvBearerAuthenticator::new(
            SecretString::from("right-token".to_string()),
            UserId::new("user-alpha").expect("user"),
        )
        .expect("auth");
        assert!(auth.authenticate("wrong-token").await.is_none());
    }

    #[tokio::test]
    async fn env_bearer_authenticator_rejects_short_prefix() {
        // Prefix attack: a short candidate must still be rejected
        // even though it would be a prefix of the configured token.
        let auth = EnvBearerAuthenticator::new(
            SecretString::from("right-token".to_string()),
            UserId::new("user-alpha").expect("user"),
        )
        .expect("auth");
        assert!(auth.authenticate("right").await.is_none());
        assert!(auth.authenticate("").await.is_none());
    }

    #[test]
    fn env_bearer_authenticator_rejects_empty_configured_token() {
        let err = EnvBearerAuthenticator::new(
            SecretString::from(String::new()),
            UserId::new("user-alpha").expect("user"),
        )
        .expect_err("empty token must be rejected at construction");
        assert!(matches!(err, EnvBearerConfigError::EmptyToken));
    }
}
