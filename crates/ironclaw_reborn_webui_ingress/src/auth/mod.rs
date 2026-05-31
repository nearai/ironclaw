//! Host-owned OAuth login surface for the WebChat v2 gateway.
//!
//! Composition mounts [`webui_v2_auth_router`] as a public route
//! group alongside the bearer-protected WebChat v2 routes:
//!
//! - `GET  /auth/providers` — list configured OAuth providers (the
//!   SPA renders one button per entry).
//! - `GET  /auth/login/{provider}` — initiate the OAuth flow; mints
//!   a CSRF state + PKCE verifier and redirects to the provider.
//! - `GET  /auth/callback/{provider}` — exchange the code, resolve
//!   the user through [`UserDirectory`], create a session via
//!   [`SessionStore`](crate::SessionStore), and land the browser on
//!   the SPA with a one-time exchange ticket.
//! - `POST /auth/session/exchange` — consume the one-time ticket and
//!   return the bearer over same-origin JSON.
//! - `POST /auth/logout` — revoke the current session.
//!
//! The crate ships Google and GitHub providers today; NEAR wallet
//! login (which does not fit the OAuth code flow) is still out of
//! scope for this iteration of issue #4116. The [`OAuthProvider`]
//! trait is the seam those plug into.

mod config;
mod error;
mod github;
mod google;
mod pending;
mod profile;
mod provider;
mod provider_http;
mod provider_name;
mod routes;
mod user_directory;

pub use config::{GitHubOAuthConfig, GoogleOAuthConfig};
pub use error::{OAuthError, ProviderInitError};
pub use github::GitHubProvider;
pub use google::GoogleProvider;
pub use ironclaw_reborn_composition::PublicRouteMount;
pub use profile::OAuthUserProfile;
pub use provider::OAuthProvider;
pub use provider_name::{OAuthProviderName, OAuthProviderNameError};
pub use routes::{OAuthRouterConfig, webui_v2_auth_router};
#[cfg(any(test, feature = "dev-in-memory-session"))]
pub use user_directory::EmailUserDirectory;
pub use user_directory::{UserDirectory, UserDirectoryError};
