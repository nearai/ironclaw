//! Error types for the WebChat v2 OAuth login flow.
//!
//! The [`OAuthError`] variants distinguish operator-visible failure
//! reasons (provider HTTP errors, malformed callback payloads,
//! configuration gaps) from the generic responses returned to the
//! browser. Route handlers map the internal variant to a sanitized
//! HTTP response — provider error bodies, redirect targets, and JWT
//! parse details are logged via `tracing` but never echoed back to
//! the client.

use thiserror::Error;

/// Error raised while constructing an [`OAuthProvider`](super::provider::OAuthProvider).
///
/// The only failure today is the provider's `reqwest::Client` build,
/// which only fails if the rustls / tokio runtime cannot initialize.
/// Surfacing it as a `Result` from the provider factory (rather than
/// panicking inside a constructor) lets the host composition layer
/// fail startup loudly instead of aborting the process.
#[derive(Debug, Error)]
#[error("OAuth provider HTTP client init failed: {0}")]
pub struct ProviderInitError(pub(crate) String);

/// Errors produced by the OAuth backend.
#[derive(Debug, Error)]
pub enum OAuthError {
    /// The configured OAuth provider rejected the token-exchange
    /// request, or the HTTP call to the provider failed.
    #[error("code exchange failed: {0}")]
    CodeExchange(String),
    /// The token-exchange succeeded but the returned profile could
    /// not be decoded or failed claim validation (audience, issuer,
    /// hosted domain).
    #[error("profile fetch failed: {0}")]
    ProfileFetch(String),
    /// The configured allow-list rejected the resolved profile
    /// (unverified email, disallowed hosted domain, unmapped user).
    #[error("authorization denied: {0}")]
    Denied(String),
}
