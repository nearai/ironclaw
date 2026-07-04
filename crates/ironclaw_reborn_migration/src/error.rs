//! Migration error type.
//!
//! Migration is fail-loud on infrastructure errors (can't open a DB, can't
//! write a record) — those abort the run. Per-item conversion problems are NOT
//! errors: they become [`crate::report::LossyItem`]s and the run continues.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("failed to open v1 source database: {0}")]
    OpenSource(String),

    #[error("failed to open Reborn target store: {0}")]
    OpenTarget(String),

    #[error("failed to read v1 state ({domain}): {reason}")]
    ReadSource { domain: String, reason: String },

    #[error("failed to write Reborn state ({domain}): {reason}")]
    WriteTarget { domain: String, reason: String },

    #[error("secrets master key required to migrate secrets but none was provided")]
    MissingSecretKey,

    #[error("invalid migration input: {0}")]
    InvalidInput(String),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}
