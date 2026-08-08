//! Typed web-push domain failures. Reasons are sanitized summaries — no
//! endpoint URLs, key material, or backend error internals cross this
//! boundary (push endpoints are capability URLs and are treated as
//! sensitive).

/// Web-push domain errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WebPushError {
    /// A subscription payload from the browser failed validation.
    #[error("push subscription is invalid: {reason}")]
    InvalidSubscription { reason: String },
    /// The subscription endpoint's push service is not on the supported
    /// allowlist. Carries only the host, never the full capability URL.
    #[error("push service host is not supported: {host}")]
    UnsupportedPushService { host: String },
    /// The payload does not fit the single-record aes128gcm budget.
    #[error("push payload of {bytes} bytes exceeds the {limit}-byte limit")]
    PayloadTooLarge { bytes: usize, limit: usize },
    /// A cryptographic operation failed. The reason names the operation,
    /// never material.
    #[error("web push crypto failure: {reason}")]
    Crypto { reason: String },
    /// The subscription store rejected or failed the operation. Carries a
    /// fixed sanitized category only — backend diagnostics never enter the
    /// message. The originating cause is logged server-side at the store
    /// boundary (see [`WebPushError::store`]) before it is dropped.
    #[error("web push subscription store failure")]
    Store,
    /// The per-user subscription cap would be exceeded.
    #[error("subscription limit of {limit} browsers reached")]
    SubscriptionLimitReached { limit: usize },
    /// A caller-supplied scope or identity failed validation.
    #[error("web push scope is invalid: {reason}")]
    InvalidScope { reason: String },
    /// The runtime slot has not been installed yet (boot ordering).
    #[error("web push runtime is not available yet")]
    RuntimeUnavailable,
    /// The runtime slot was installed twice.
    #[error("web push runtime is already installed")]
    RuntimeAlreadyInstalled,
}

impl WebPushError {
    /// Map a store/backend cause to the sanitized [`WebPushError::Store`]
    /// category, logging the bound source server-side first so the diagnostic
    /// is retained without ever entering the error's `Display` (the
    /// redaction contract this module's header states).
    pub fn store(source: impl std::fmt::Display) -> Self {
        tracing::debug!(
            target: "ironclaw::web_push",
            error = %source,
            "web push subscription store operation failed"
        );
        Self::Store
    }

    pub fn crypto(reason: impl Into<String>) -> Self {
        Self::Crypto {
            reason: reason.into(),
        }
    }
}
