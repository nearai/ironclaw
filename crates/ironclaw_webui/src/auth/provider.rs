//! Provider abstraction for the WebChat v2 OAuth login flow.
//!
//! `OAuthProvider` is the generic contract every code-flow provider
//! implements: build an authorization URL the browser is redirected
//! to, then exchange the returned code for a normalized
//! [`OAuthUserProfile`]. The route handlers in `auth/routes.rs`
//! dispatch by `provider.name()` and never depend on a concrete
//! implementation.
//!
//! Today's impls are [`crate::auth::GoogleProvider`] and
//! [`crate::auth::GitHubProvider`]. NEAR (via a different sub-router,
//! since wallet login does not fit OAuth code flow) plugs in here
//! without touching the routes or the session machinery.

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
    ///
    /// # Safety: the `email_verified` contract
    ///
    /// A returned `profile.email` is authoritative for account linkage
    /// **only when `profile.email_verified == true`**. A provider may
    /// set `email` while reporting `email_verified == false` (e.g.
    /// GitHub, when the account has no verified address — it returns the
    /// unverified profile email so callers have *something* to log). The
    /// [`UserDirectory`](super::user_directory::UserDirectory) MUST NOT
    /// match or link an account by an unverified email; it must fall
    /// back to the provider-unique id (`{provider}:{provider_user_id}`)
    /// or reject the login. Treating an unverified email as an identity
    /// is an account-takeover vector: an attacker can add
    /// `victim@corp.com` (unverified) to their own provider account and,
    /// if the directory linked by raw email, log in as the victim.
    /// Unlike Google's OIDC `hd` claim, GitHub has no provider-level
    /// domain restriction to backstop this — the `email_verified` gate
    /// is the only line of defense.
    async fn exchange_code(
        &self,
        code: &str,
        callback_url: &str,
        code_verifier: &str,
    ) -> Result<OAuthUserProfile, OAuthError>;
}
