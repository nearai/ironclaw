use thiserror::Error;

#[derive(Debug, Error)]
pub enum OutboundError {
    #[error("outbound state backend unavailable")]
    Backend,
    #[error("outbound state serialization failed")]
    Serialization,
    #[error("outbound state request rejected: {reason}")]
    InvalidRequest { reason: &'static str },
    #[error("subscription cursor scope mismatch")]
    SubscriptionScopeMismatch,
    #[error("outbound access denied")]
    AccessDenied,
    #[error("outbound delivery not found")]
    DeliveryNotFound,
    /// Compare-and-swap precondition failed on the underlying filesystem. The
    /// caller observed a stale `RecordVersion`; a bounded retry loop should
    /// re-read the current entry and re-apply the transformation. Distinct
    /// from [`OutboundError::Backend`] so retry loops can match on the typed
    /// variant rather than collapsing transient races into a permanent
    /// failure. Stays internal to the crate — converted to
    /// [`OutboundError::Backend`] before returning to a caller once the
    /// bounded retry budget is exhausted.
    #[error("outbound state compare-and-swap conflict")]
    CasConflict,
}
