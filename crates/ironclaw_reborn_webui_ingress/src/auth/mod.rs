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
//!   the SPA with the bearer token in the URL fragment.
//! - `POST /auth/logout` — revoke the current session.
//!
//! The crate ships a Google provider only at the moment; GitHub and
//! NEAR are out of scope for this iteration of issue #4116. The
//! [`OAuthProvider`] trait is the seam those will plug into.

mod config;
mod error;
mod google;
mod pending;
mod profile;
mod provider;
mod routes;
mod user_directory;

pub use config::GoogleOAuthConfig;
pub use error::OAuthError;
pub use google::GoogleProvider;
pub use profile::OAuthUserProfile;
pub use provider::OAuthProvider;
pub use routes::{OAuthRouterConfig, PublicRouteMount, webui_v2_auth_router};
#[cfg(any(test, feature = "dev-in-memory-session"))]
pub use user_directory::EmailUserDirectory;
pub use user_directory::{UserDirectory, UserDirectoryError};
