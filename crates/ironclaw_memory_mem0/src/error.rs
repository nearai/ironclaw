//! mem0 provider error types.
//!
//! These are the provider-internal backend causes. They never reach the model
//! or user directly: the [`crate::service::Mem0MemoryService`] maps every one of
//! them into a sanitized [`ironclaw_memory::MemoryServiceError`] via
//! `operation_from` / `unavailable_from`, which preserves the cause for host
//! logging while exposing only a stable, redacted surface. The construction-time
//! variants ([`Mem0Error::InvalidUrl`] / [`Mem0Error::Client`]) surface from
//! [`crate::Mem0HttpTransport::new`] and are handled by the composition factory
//! (fail-closed: a provider that cannot be built is not registered).

use thiserror::Error;

/// A failure originating from the mem0 backend, its transport construction, or
/// its response decoding.
#[derive(Debug, Error)]
pub enum Mem0Error {
    /// The configured mem0 base URL failed the baseline SSRF check: it did not
    /// parse, used a non-http(s) scheme, had no host, or named an always-blocked
    /// literal IP (cloud-metadata / link-local / multicast / unspecified).
    #[error("invalid mem0 base URL '{url}': {reason}")]
    InvalidUrl { url: String, reason: String },

    /// The `reqwest` client could not be constructed (e.g. an API key that is
    /// not a valid HTTP header value, or a TLS backend failure).
    #[error("failed to build the mem0 HTTP client: {reason}")]
    Client { reason: String },

    /// The mem0 REST API returned a non-success HTTP status. Response-body
    /// parsing is deliberately tolerant (an unrecognized search/list shape
    /// degrades to "no results"), so a malformed-but-2xx body is not an error;
    /// only the HTTP status is.
    #[error("mem0 API returned status {status} for {operation}")]
    Api {
        operation: &'static str,
        status: u16,
    },

    /// The requested IronClaw memory operation has no faithful representation in
    /// the mem0 data model (e.g. substring patch, named-document clear). Mapped
    /// to an `Operation` failure so the model sees a stable error rather than a
    /// silently wrong result.
    #[error("operation '{operation}' is not supported by the mem0 provider model: {detail}")]
    Unsupported {
        operation: &'static str,
        detail: &'static str,
    },
}
