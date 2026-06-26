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
#[cfg(feature = "capability-policy")]
use ironclaw_host_api::TenantId;
use ironclaw_host_api::UserId;
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

/// Authenticator that resolves a bearer **user-token** to its
/// `(UserId, UserRole)` through the durable [`LocalUserDirectoryStore`] —
/// the REST-created local-dev user directory (issue #5272). Users are created
/// through the `/admin/users` admin REST surface (which mints a token and
/// stores only its hash), so one operator can act as several users by swapping
/// the bearer token, and the resolved [`UserRole`] travels with the
/// authentication so the role-gated admin surface and the per-(tenant, user)
/// dispatch principal both work per token.
///
/// Unlike [`EnvBearerAuthenticator`] (single token → single operator), this
/// authenticator looks each candidate up by its hash and never grants operator
/// WebUI config privileges (`mounts_operator_webui_config_routes` is `false`);
/// deployment-wide config stays with the separate operator credential.
/// Production multi-user auth uses `SessionAuthenticator` / `OidcAuthenticator`.
#[cfg(feature = "capability-policy")]
pub struct LocalUserDirectoryAuthenticator {
    tenant_id: TenantId,
    store: Arc<dyn ironclaw_reborn_composition::LocalUserDirectoryStore>,
}

#[cfg(feature = "capability-policy")]
impl LocalUserDirectoryAuthenticator {
    /// Build an authenticator that resolves bearer tokens against `store`
    /// within `tenant_id`.
    pub fn new(
        tenant_id: TenantId,
        store: Arc<dyn ironclaw_reborn_composition::LocalUserDirectoryStore>,
    ) -> Self {
        Self { tenant_id, store }
    }
}

#[cfg(feature = "capability-policy")]
#[async_trait]
impl WebuiAuthenticator for LocalUserDirectoryAuthenticator {
    async fn authenticate(
        &self,
        candidate: &str,
    ) -> Option<ironclaw_reborn_composition::WebuiAuthentication> {
        // Hash the candidate token the same way the directory stores it, then
        // resolve. The raw token is never compared against stored material.
        let token_hash = ironclaw_reborn_composition::hash_user_token(candidate);
        match self.store.resolve_token(&self.tenant_id, &token_hash).await {
            Ok(Some(record)) => Some(
                ironclaw_reborn_composition::WebuiAuthentication::user(record.user_id)
                    .with_role(record.role),
            ),
            Ok(None) => None,
            Err(error) => {
                // A backend read failure is NOT a bad bearer; surface the cause
                // at the auth boundary rather than letting a transient store
                // outage masquerade as an authentication rejection (the request
                // still fails closed — we return `None`). Per
                // `.claude/rules/error-handling.md`, do not silently drop it.
                tracing::warn!(
                    target = "ironclaw::reborn::webui_ingress",
                    %error,
                    "local user directory token resolution failed; rejecting request",
                );
                None
            }
        }
    }

    fn mounts_operator_webui_config_routes(&self) -> bool {
        false
    }
}

/// Tries several [`WebuiAuthenticator`]s in order, returning the first match.
///
/// Used by the standalone `serve` command (under the `capability-policy`
/// feature) to layer the multi-user `LocalUserDirectoryAuthenticator` *over*
/// the single-operator [`EnvBearerAuthenticator`]: a REST-created user token
/// resolves to its `(UserId, role)`, and anything else falls through to the
/// operator credential — so the operator keeps its signing key, runtime-owner
/// pin, and operator WebUI routes while the extra users are added on top.
/// Operator-route mounting is the OR across layers (any layer that mounts them
/// wins).
pub struct LayeredWebuiAuthenticator {
    layers: Vec<Arc<dyn WebuiAuthenticator>>,
}

impl LayeredWebuiAuthenticator {
    /// Build from layers in match-priority order (earlier layers win).
    pub fn new(layers: Vec<Arc<dyn WebuiAuthenticator>>) -> Self {
        Self { layers }
    }
}

#[async_trait]
impl WebuiAuthenticator for LayeredWebuiAuthenticator {
    async fn authenticate(
        &self,
        candidate: &str,
    ) -> Option<ironclaw_reborn_composition::WebuiAuthentication> {
        for layer in &self.layers {
            if let Some(auth) = layer.authenticate(candidate).await {
                return Some(auth);
            }
        }
        None
    }

    fn mounts_operator_webui_config_routes(&self) -> bool {
        self.layers
            .iter()
            .any(|layer| layer.mounts_operator_webui_config_routes())
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

    #[tokio::test]
    async fn layered_authenticator_returns_first_matching_layer() {
        // Ungated coverage of the generic layering: two env-bearer layers,
        // each accepting its own token; the layered authenticator returns the
        // first match and ORs the operator-route capability across layers.
        let first = EnvBearerAuthenticator::new(
            SecretString::from("first-token".to_string()),
            UserId::new("user:first").expect("user"),
        )
        .expect("first auth");
        let second = EnvBearerAuthenticator::new(
            SecretString::from("second-token".to_string()),
            UserId::new("user:second").expect("user"),
        )
        .expect("second auth");
        let layered = LayeredWebuiAuthenticator::new(vec![Arc::new(first), Arc::new(second)]);

        let first_match = layered.authenticate("first-token").await.expect("first");
        assert_eq!(first_match.user_id.as_str(), "user:first");
        let second_match = layered.authenticate("second-token").await.expect("second");
        assert_eq!(second_match.user_id.as_str(), "user:second");
        assert!(layered.authenticate("nope").await.is_none());
        // Both layers mount operator routes, so the OR is true.
        assert!(layered.mounts_operator_webui_config_routes());
    }

    #[cfg(feature = "capability-policy")]
    mod local_user_directory {
        use super::*;
        use ironclaw_host_api::UserRole;
        use ironclaw_reborn_composition::{
            LocalUserDirectoryStore, LocalUserRecord, hash_user_token,
        };

        /// In-memory fake of the durable user directory keyed by the same hashes
        /// the production `FilesystemLocalUserDirectoryStore` persists, so the
        /// authenticator's hash→record lookup is exercised end-to-end.
        struct FakeUserDirectoryStore {
            users: std::collections::HashMap<String, LocalUserRecord>,
            fail: bool,
        }

        impl FakeUserDirectoryStore {
            fn with_users(
                rows: impl IntoIterator<Item = (&'static str, &'static str, UserRole)>,
            ) -> Self {
                let mut users = std::collections::HashMap::new();
                for (token, user_id, role) in rows {
                    users.insert(
                        hash_user_token(token),
                        LocalUserRecord {
                            user_id: UserId::new(user_id).expect("user id"),
                            role,
                        },
                    );
                }
                Self { users, fail: false }
            }

            fn failing() -> Self {
                Self {
                    users: std::collections::HashMap::new(),
                    fail: true,
                }
            }
        }

        #[async_trait]
        impl LocalUserDirectoryStore for FakeUserDirectoryStore {
            async fn create_user(
                &self,
                _tenant_id: &TenantId,
                _user_id: &UserId,
                _role: UserRole,
                _token_hash: &str,
            ) -> Result<(), ironclaw_reborn_composition::LocalUserDirectoryError> {
                unimplemented!("authenticator never writes")
            }

            async fn set_role(
                &self,
                _tenant_id: &TenantId,
                _user_id: &UserId,
                _role: UserRole,
            ) -> Result<(), ironclaw_reborn_composition::LocalUserDirectoryError> {
                unimplemented!("authenticator never writes")
            }

            async fn list_users(
                &self,
                _tenant_id: &TenantId,
            ) -> Result<Vec<LocalUserRecord>, ironclaw_reborn_composition::LocalUserDirectoryError>
            {
                unimplemented!("authenticator never lists")
            }

            async fn delete_user(
                &self,
                _tenant_id: &TenantId,
                _user_id: &UserId,
            ) -> Result<(), ironclaw_reborn_composition::LocalUserDirectoryError> {
                unimplemented!("authenticator never deletes")
            }

            async fn resolve_token(
                &self,
                _tenant_id: &TenantId,
                token_hash: &str,
            ) -> Result<Option<LocalUserRecord>, ironclaw_reborn_composition::LocalUserDirectoryError>
            {
                if self.fail {
                    return Err(
                        ironclaw_reborn_composition::LocalUserDirectoryError::Backend(
                            "synthetic backend failure".to_string(),
                        ),
                    );
                }
                Ok(self.users.get(token_hash).cloned())
            }

            async fn resolve_user(
                &self,
                _tenant_id: &TenantId,
                user_id: &UserId,
            ) -> Result<Option<LocalUserRecord>, ironclaw_reborn_composition::LocalUserDirectoryError>
            {
                if self.fail {
                    return Err(
                        ironclaw_reborn_composition::LocalUserDirectoryError::Backend(
                            "synthetic backend failure".to_string(),
                        ),
                    );
                }
                Ok(self
                    .users
                    .values()
                    .find(|record| &record.user_id == user_id)
                    .cloned())
            }
        }

        fn directory_authenticator(
            store: FakeUserDirectoryStore,
        ) -> LocalUserDirectoryAuthenticator {
            LocalUserDirectoryAuthenticator::new(
                TenantId::new("tenant-local").expect("tenant"),
                Arc::new(store),
            )
        }

        #[tokio::test]
        async fn local_user_directory_authenticator_maps_token_to_user_and_role() {
            let auth = directory_authenticator(FakeUserDirectoryStore::with_users([
                ("director-token", "user:director", UserRole::Admin),
                ("bob-token", "user:bob", UserRole::Member),
            ]));

            let director = auth.authenticate("director-token").await.expect("director");
            assert_eq!(director.user_id.as_str(), "user:director");
            assert_eq!(director.role, UserRole::Admin);
            // A user-token authenticator never grants operator WebUI config.
            assert!(!director.capabilities.operator_webui_config);

            // Role propagation: a Member token resolves to Member, not Admin.
            let bob = auth.authenticate("bob-token").await.expect("bob");
            assert_eq!(bob.user_id.as_str(), "user:bob");
            assert_eq!(bob.role, UserRole::Member);
        }

        #[tokio::test]
        async fn local_user_directory_authenticator_rejects_unknown_token() {
            let auth = directory_authenticator(FakeUserDirectoryStore::with_users([(
                "director-token",
                "user:director",
                UserRole::Admin,
            )]));
            assert!(auth.authenticate("nope").await.is_none());
            assert!(auth.authenticate("").await.is_none());
            // A prefix of a real token hashes differently and must not match.
            assert!(auth.authenticate("director").await.is_none());
        }

        #[tokio::test]
        async fn local_user_directory_authenticator_rejects_on_backend_error() {
            // A store read failure must fail closed (reject) rather than
            // authenticate; the cause is logged at the boundary.
            let auth = directory_authenticator(FakeUserDirectoryStore::failing());
            assert!(auth.authenticate("director-token").await.is_none());
        }

        #[tokio::test]
        async fn local_user_directory_authenticator_does_not_mount_operator_routes() {
            let auth = directory_authenticator(FakeUserDirectoryStore::with_users([(
                "director-token",
                "user:director",
                UserRole::Admin,
            )]));
            assert!(!auth.mounts_operator_webui_config_routes());
        }

        #[tokio::test]
        async fn layered_authenticator_prefers_user_directory_then_falls_back_to_operator() {
            let user_directory = directory_authenticator(FakeUserDirectoryStore::with_users([(
                "director-token",
                "user:director",
                UserRole::Admin,
            )]));
            let operator = EnvBearerAuthenticator::new(
                SecretString::from("operator-token".to_string()),
                UserId::new("user:operator").expect("user"),
            )
            .expect("operator auth");
            let layered =
                LayeredWebuiAuthenticator::new(vec![Arc::new(user_directory), Arc::new(operator)]);

            // A user-directory token resolves to its own user + role (no operator caps).
            let director = layered
                .authenticate("director-token")
                .await
                .expect("director");
            assert_eq!(director.user_id.as_str(), "user:director");
            assert_eq!(director.role, UserRole::Admin);
            assert!(!director.capabilities.operator_webui_config);

            // The operator token falls through to the env-bearer layer.
            let operator = layered
                .authenticate("operator-token")
                .await
                .expect("operator");
            assert_eq!(operator.user_id.as_str(), "user:operator");
            assert!(operator.capabilities.operator_webui_config);

            assert!(layered.authenticate("nope").await.is_none());
            // Operator routes mount because a layer (the env-bearer) provides them.
            assert!(layered.mounts_operator_webui_config_routes());
        }
    }
}
