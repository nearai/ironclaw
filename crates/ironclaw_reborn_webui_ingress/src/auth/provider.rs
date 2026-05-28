//! Provider abstraction for the WebChat v2 OAuth login flow.
//!
//! `OAuthProvider` is the generic contract every code-flow provider
//! implements: build an authorization URL the browser is redirected
//! to, then exchange the returned code for a normalized
//! [`OAuthUserProfile`]. The route handlers in `auth/routes.rs`
//! dispatch by `provider.name()` and never depend on a concrete
//! implementation.
//!
//! Today's only impl is [`crate::auth::GoogleProvider`]. GitHub and
//! NEAR (the latter via a different sub-router, since wallet login
//! does not fit OAuth code flow) plug in here without touching the
//! routes or the session machinery.

use async_trait::async_trait;

use super::error::OAuthError;
use super::profile::OAuthUserProfile;
use super::provider_name::OAuthProviderName;

/// Generic provider contract — see module docs.
#[async_trait]
pub trait OAuthProvider: Send + Sync + 'static {
    /// Stable provider identifier exposed on `/auth/providers` and
    /// matched against the `{provider}` path segment on login /
    /// callback. Validated newtype so the URL-parsed segment, the
    /// pending-flow record, and the provider-self-id cannot drift.
    fn name(&self) -> &OAuthProviderName;

    /// Build the provider-side authorization URL the browser is
    /// redirected to. `callback_url` is the v2-owned
    /// `/auth/callback/{provider}` URL; `state` is the CSRF token
    /// stored in the pending-flow cache; `code_challenge` is the
    /// PKCE S256 challenge (providers that do not support PKCE may
    /// ignore it).
    fn authorization_url(&self, callback_url: &str, state: &str, code_challenge: &str) -> String;

    /// Exchange the authorization code returned by the provider for
    /// a normalized [`OAuthUserProfile`].
    async fn exchange_code(
        &self,
        code: &str,
        callback_url: &str,
        code_verifier: &str,
    ) -> Result<OAuthUserProfile, OAuthError>;
}
