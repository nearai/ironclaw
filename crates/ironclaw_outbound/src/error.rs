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
}
