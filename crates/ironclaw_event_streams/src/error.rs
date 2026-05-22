use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProjectionStreamError {
    #[error("projection stream request rejected: {reason}")]
    InvalidRequest { reason: &'static str },
    #[error("projection stream access denied")]
    AccessDenied,
    #[error("projection stream admission denied")]
    AdmissionDenied,
    #[error("projection stream source failed")]
    Source,
    #[error("projection stream payload failed redaction validation")]
    Redaction,
    #[error("projection stream outbound policy failed")]
    Outbound,
}
