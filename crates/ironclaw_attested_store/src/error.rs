//! Backend-failure error type shared by the durable store impls.

use thiserror::Error;

/// A durable-backend I/O / SQL failure.
///
/// The store traits ([`ironclaw_attestation::SealedGrantStore`] etc.) carry
/// their own semantic error variants (`AlreadyClaimed`, `NotFound`,
/// `InvalidTransition`, ...); this type is only for the opaque backend failures
/// those traits funnel through their `Backend { reason }` variants. Keeping it
/// here lets the PG and libSQL impls share one mapping helper.
#[derive(Debug, Error)]
pub enum StoreError {
    /// A database / connection-pool failure with an opaque description.
    #[error("attested-store backend error: {0}")]
    Backend(String),
}

impl StoreError {
    /// Wrap any error as a backend failure. Only used by the durable backends,
    /// so gate it to avoid a dead-code warning on a no-backend build.
    #[cfg(any(feature = "postgres", feature = "libsql"))]
    pub(crate) fn backend(error: impl std::fmt::Display) -> Self {
        StoreError::Backend(error.to_string())
    }
}
