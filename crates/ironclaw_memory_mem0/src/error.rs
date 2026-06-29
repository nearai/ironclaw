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
    /// parse, used a non-http(s) scheme, had no host, named an always-blocked
    /// literal IP (cloud-metadata / link-local / multicast / unspecified), or named
    /// the hosted mem0 cloud (this adapter is self-hosted-OSS only).
    ///
    /// The raw URL is deliberately NOT carried in this error: a misconfigured base
    /// URL can embed a sensitive host or a query-string token, and this `Display`
    /// reaches host logs. Only the redacted `reason` is kept. The blocked-host
    /// cases name the offending host *inside* `reason`, but only for hosts that are
    /// well-known and non-secret by construction (a blocked literal IP or a public
    /// mem0 cloud host).
    #[error("invalid mem0 base URL: {reason}")]
    InvalidUrl { reason: String },

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

    /// A stored `kind=profile` memory exists but does not parse as a JSON object.
    /// Surfaced as an `Operation` failure so the `profile_set` read-merge-write
    /// fails loud rather than silently treating the corrupt blob as empty — which
    /// would drop every previously-stored profile field on the next write.
    #[error("stored mem0 profile is not a JSON object: {reason}")]
    CorruptProfile { reason: String },

    /// A 2xx mem0 response body matched none of the recognized list shapes (a
    /// bare array, or an object wrapping `results`/`memories`/`data`). Surfaced
    /// as a failure so list-shaped reads fail loud instead of silently treating
    /// an unrecognized body as "no memories" — which would, for example, let
    /// `profile_set` overwrite an existing profile it merely failed to decode. A
    /// genuinely-empty list is a recognized shape (an empty array) and is not an
    /// error.
    #[error("mem0 returned an unrecognized response body shape")]
    UnrecognizedResponse,
}
