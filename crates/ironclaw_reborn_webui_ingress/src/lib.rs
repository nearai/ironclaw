#![forbid(unsafe_code)]

//! Host-owned listener binding + serve loop for the Reborn WebChat v2
//! HTTP gateway.
//!
//! `ironclaw_reborn_composition::webui_v2_app` returns a fully composed
//! axum [`Router`] but deliberately stops at the
//! `reborn_product_api_crates_do_not_bind_http_ingress` boundary — that
//! crate must not bind sockets or call `axum::serve`. This crate is
//! the host-owned counterpart: it accepts the `Router` from composition
//! plus the listen address, binds a `TcpListener`, and runs the serve
//! loop with graceful shutdown.
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
mod oidc;
mod session;
mod signed_session_login;

#[cfg(any(test, feature = "dev-in-memory-session"))]
pub use auth::EmailUserDirectory;
pub use auth::{
    GitHubOAuthConfig, GitHubProvider, GoogleOAuthConfig, GoogleProvider, OAuthError,
    OAuthProvider, OAuthProviderName, OAuthProviderNameError, OAuthRouterConfig, OAuthUserProfile,
    ProviderInitError, PublicRouteMount, UserDirectory, UserDirectoryError, webui_v2_auth_router,
};
pub use oidc::{
    AudienceClaim, ClaimToUserIdFn, IdTokenClaims, OidcAuthenticator, OidcAuthenticatorConfig,
    OidcAuthenticatorError,
};
pub use session::{SessionAuthenticator, SessionRecord, SessionStore, SessionStoreError};
// Host-owned signed-token login surface (production-suitable, non-dev):
// the standalone `serve` binary supplies env config and calls the
// builder; the auth/session model lives here, not in the command crate.
pub use signed_session_login::{
    SignedSessionLoginConfig, SignedSessionLoginWiring, build_signed_session_login,
};
// `InMemorySessionStore` is gated behind `dev-in-memory-session` so a
// production binary cannot accidentally wire a process-local store as
// a `SessionStore` impl. Local dev and tests opt in via the feature.
#[cfg(any(test, feature = "dev-in-memory-session"))]
pub use session::InMemorySessionStore;

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
use ironclaw_host_api::{UserId, UserRole};
use ironclaw_reborn_composition::WebuiAuthenticator;
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
    async fn authenticate(
        &self,
        candidate: &str,
    ) -> Option<ironclaw_reborn_composition::WebuiAuthentication> {
        // Constant-time comparison so an attacker cannot use response
        // timing to learn the prefix of the configured token. Both
        // operands are coerced to `&[u8]` of the same length to make
        // the underlying `subtle::ConstantTimeEq` impl meaningful
        // (`subtle` returns "not equal" for length mismatch in
        // constant time too).
        let expected = self.token.expose_secret().as_bytes();
        let candidate = candidate.as_bytes();
        if expected.ct_eq(candidate).into() {
            Some(ironclaw_reborn_composition::WebuiAuthentication::operator(
                self.user_id.clone(),
            ))
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

/// Authenticator that maps several bearer **user-tokens** to distinct
/// `(UserId, UserRole)` pairs, so one operator can act as several users — e.g.
/// `director@` (Admin), `Bob` / `Carol` (Member), and a shared `engineering@`
/// (Member) — by swapping the bearer token. This is the local-dev / standalone
/// way to exercise per-user capability policy (issue #5272): unlike
/// [`EnvBearerAuthenticator`] (single token → single operator), the resolved
/// [`UserRole`] travels with the authentication so the role-gated admin surface
/// and the per-(tenant, user) dispatch principal both work per token.
///
/// Deliberately minimal and local-dev-oriented. It does **not** grant
/// operator WebUI config privileges (`mounts_operator_webui_config_routes` is
/// `false`); deployment-wide config stays with the separate operator
/// credential. Production multi-user auth uses `SessionAuthenticator` /
/// `OidcAuthenticator`.
#[derive(Debug)]
pub struct StaticUserTokenAuthenticator {
    entries: Vec<UserTokenEntry>,
}

#[derive(Debug)]
struct UserTokenEntry {
    /// `SecretString` so token material is redacted in `Debug` / logs.
    token: SecretString,
    user_id: UserId,
    role: UserRole,
}

/// One `(token, user_id, role)` row as parsed from the
/// `IRONCLAW_REBORN_USER_TOKENS` JSON env. `role` defaults to the
/// least-privilege `Member` when omitted.
#[derive(Debug, serde::Deserialize)]
struct UserTokenConfig {
    token: String,
    user_id: String,
    #[serde(default)]
    role: UserRole,
}

impl StaticUserTokenAuthenticator {
    /// Build from explicit `(token, user_id, role)` rows. Rejects an empty
    /// table and any empty token (a bare `Authorization: Bearer ` would
    /// otherwise match).
    pub fn new(
        rows: impl IntoIterator<Item = (SecretString, UserId, UserRole)>,
    ) -> Result<Self, UserTokenConfigError> {
        let mut entries = Vec::new();
        for (token, user_id, role) in rows {
            if token.expose_secret().is_empty() {
                return Err(UserTokenConfigError::EmptyToken);
            }
            entries.push(UserTokenEntry {
                token,
                user_id,
                role,
            });
        }
        if entries.is_empty() {
            return Err(UserTokenConfigError::Empty);
        }
        Ok(Self { entries })
    }

    /// Parse a JSON array of `{ "token", "user_id", "role" }` objects, e.g.
    /// `[{"token":"d-tok","user_id":"user:director","role":"admin"}, …]`.
    /// `role` accepts `owner` / `admin` / `member` and defaults to `member`.
    pub fn from_json(json: &str) -> Result<Self, UserTokenConfigError> {
        let configs: Vec<UserTokenConfig> = serde_json::from_str(json)
            .map_err(|error| UserTokenConfigError::Parse(error.to_string()))?;
        let mut rows = Vec::with_capacity(configs.len());
        for config in configs {
            let user_id = UserId::new(&config.user_id).map_err(|error| {
                UserTokenConfigError::InvalidUserId {
                    value: config.user_id.clone(),
                    reason: error.to_string(),
                }
            })?;
            rows.push((SecretString::from(config.token), user_id, config.role));
        }
        Self::new(rows)
    }
}

/// Errors raised when constructing [`StaticUserTokenAuthenticator`].
#[derive(Debug, Error)]
pub enum UserTokenConfigError {
    #[error("user-token table must not be empty")]
    Empty,
    #[error("bearer token must not be empty")]
    EmptyToken,
    #[error("invalid user-token JSON: {0}")]
    Parse(String),
    #[error("user-token entry has invalid user_id `{value}`: {reason}")]
    InvalidUserId { value: String, reason: String },
}

#[async_trait]
impl WebuiAuthenticator for StaticUserTokenAuthenticator {
    async fn authenticate(
        &self,
        candidate: &str,
    ) -> Option<ironclaw_reborn_composition::WebuiAuthentication> {
        // Compare against every entry with a constant-time equality and never
        // short-circuit, so neither the matched token's content nor its
        // position in the table leaks through response timing.
        let candidate = candidate.as_bytes();
        let mut matched: Option<&UserTokenEntry> = None;
        for entry in &self.entries {
            if entry
                .token
                .expose_secret()
                .as_bytes()
                .ct_eq(candidate)
                .into()
            {
                matched = Some(entry);
            }
        }
        matched.map(|entry| {
            ironclaw_reborn_composition::WebuiAuthentication::user(entry.user_id.clone())
                .with_role(entry.role)
        })
    }

    fn mounts_operator_webui_config_routes(&self) -> bool {
        false
    }
}

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

    const USER_TOKENS_JSON: &str = r#"[
        {"token":"director-token","user_id":"user:director","role":"admin"},
        {"token":"bob-token","user_id":"user:bob","role":"member"},
        {"token":"eng-token","user_id":"user:engineering"}
    ]"#;

    #[tokio::test]
    async fn static_user_token_authenticator_maps_token_to_user_and_role() {
        let auth = StaticUserTokenAuthenticator::from_json(USER_TOKENS_JSON).expect("auth");

        let director = auth.authenticate("director-token").await.expect("director");
        assert_eq!(director.user_id.as_str(), "user:director");
        assert_eq!(director.role, UserRole::Admin);
        // A user-token authenticator never grants operator WebUI config.
        assert!(!director.capabilities.operator_webui_config);

        let bob = auth.authenticate("bob-token").await.expect("bob");
        assert_eq!(bob.user_id.as_str(), "user:bob");
        assert_eq!(bob.role, UserRole::Member);

        // `role` omitted → least-privilege Member default.
        let eng = auth.authenticate("eng-token").await.expect("engineering");
        assert_eq!(eng.user_id.as_str(), "user:engineering");
        assert_eq!(eng.role, UserRole::Member);
    }

    #[tokio::test]
    async fn static_user_token_authenticator_rejects_unknown_and_empty_tokens() {
        let auth = StaticUserTokenAuthenticator::from_json(USER_TOKENS_JSON).expect("auth");
        assert!(auth.authenticate("nope").await.is_none());
        assert!(auth.authenticate("").await.is_none());
        // Prefix of a real token must not match.
        assert!(auth.authenticate("director").await.is_none());
    }

    #[test]
    fn static_user_token_authenticator_rejects_empty_table_and_empty_token() {
        assert!(matches!(
            StaticUserTokenAuthenticator::from_json("[]").expect_err("empty table"),
            UserTokenConfigError::Empty
        ));
        assert!(matches!(
            StaticUserTokenAuthenticator::from_json(
                r#"[{"token":"","user_id":"user:x","role":"member"}]"#
            )
            .expect_err("empty token"),
            UserTokenConfigError::EmptyToken
        ));
    }

    #[test]
    fn static_user_token_authenticator_rejects_invalid_user_id_and_bad_json() {
        assert!(matches!(
            StaticUserTokenAuthenticator::from_json(r#"[{"token":"t","user_id":""}]"#)
                .expect_err("invalid user id"),
            UserTokenConfigError::InvalidUserId { .. }
        ));
        assert!(matches!(
            StaticUserTokenAuthenticator::from_json("not json").expect_err("bad json"),
            UserTokenConfigError::Parse(_)
        ));
    }

    #[tokio::test]
    async fn static_user_token_authenticator_does_not_mount_operator_routes() {
        let auth = StaticUserTokenAuthenticator::from_json(USER_TOKENS_JSON).expect("auth");
        assert!(!auth.mounts_operator_webui_config_routes());
    }
}
