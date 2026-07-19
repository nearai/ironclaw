//! Reborn operator migration errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("failed to open Reborn target store: {0}")]
    OpenTarget(String),

    #[error("failed to write Reborn state ({domain}): {reason}")]
    WriteTarget { domain: String, reason: String },

    #[error("failed to read Reborn state ({domain}): {reason}")]
    ReadTarget { domain: String, reason: String },

    #[error("invalid migration input: {0}")]
    InvalidInput(String),
}
