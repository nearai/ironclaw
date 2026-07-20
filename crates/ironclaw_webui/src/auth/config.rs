//! Configuration types for the WebChat v2 OAuth login surface.
//!
//! Host composition builds a [`GoogleOAuthConfig`] from operator
//! input (env vars, TOML config) and hands it to
//! [`webui_v2_auth_router`](super::webui_v2_auth_router) along with a
//! `SessionStore` and a `UserDirectory`. The composition layer is
//! responsible for picking which providers are enabled; this crate
//! never reads env vars directly so a binary that uses a different
//! config source can still wire it.

use std::time::Duration;

use secrecy::SecretString;

/// Google OAuth (OIDC) configuration. Mirrors the v1 gateway's
/// `GoogleOAuthConfig` shape so existing operator config can be
/// re-used by the v2 wire-up.
#[derive(Debug, Clone)]
pub struct GoogleOAuthConfig {
    /// OAuth 2.0 client id issued by Google Cloud Console.
    pub client_id: String,
    /// OAuth 2.0 client secret. Wrapped in [`SecretString`] so the
    /// `Debug` impl is redacted.
    pub client_secret: SecretString,
    /// Optional Google Workspace hosted domain restriction
    /// (e.g. `company.com`). When set, the authorization URL hints
    /// the account picker and the callback rejects any ID token
    /// whose `hd` claim does not match.
    pub allowed_hd: Option<String>,
    /// Optional per-call HTTP timeout for the token/userinfo requests.
    /// `None` uses the provider default. Operators on a slow or
    /// cross-border path to the provider can raise it.
    pub http_timeout: Option<Duration>,
}

/// GitHub OAuth configuration. Mirrors the v1 gateway's
/// `GitHubOAuthConfig` shape so existing operator config can be
/// re-used by the v2 wire-up.
///
/// GitHub's OAuth App flow does not support PKCE; CSRF is protected
/// solely by the `state` parameter the router mints, so there is no
/// hosted-domain analogue here — the provider just needs the client
/// credentials.
#[derive(Debug, Clone)]
pub struct GitHubOAuthConfig {
    /// OAuth App client id issued by GitHub.
    pub client_id: String,
    /// OAuth App client secret. Wrapped in [`SecretString`] so the
    /// `Debug` impl is redacted.
    pub client_secret: SecretString,
    /// Optional per-call HTTP timeout for the token/user/emails requests.
    /// `None` uses the provider default. Operators on a slow or
    /// cross-border path to `github.com` can raise it.
    pub http_timeout: Option<Duration>,
}
