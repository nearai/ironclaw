//! Local error type for the standalone Reborn Telegram v2 host.
//!
//! Deliberately not derived from any v1 error type — this crate has no
//! dependency on the `ironclaw` lib.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum HostError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("startup error: {0}")]
    Startup(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
