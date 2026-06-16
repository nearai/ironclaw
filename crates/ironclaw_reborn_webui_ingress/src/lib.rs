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

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use ironclaw_host_api::UserId;
use ironclaw_reborn_composition::WebuiAuthenticator;
use secrecy::{ExposeSecret, SecretString};
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::net::TcpListener;

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
