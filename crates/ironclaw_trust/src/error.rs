//! Trust policy error type.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TrustError {
    #[error("policy source '{source_name}' rejected package '{package}': {reason}")]
    SourceRejected {
        source_name: &'static str,
        package: String,
        reason: String,
    },
    #[error("trust policy invariant violation: {reason}")]
    InvariantViolation { reason: String },
}
