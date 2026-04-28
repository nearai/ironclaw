//! Trust policy error type.
//!
//! `TrustError` is the failure surface for `TrustPolicy::evaluate` and
//! `PolicySource::evaluate`. The contract distinguishes three outcomes:
//! `Ok(Some)` (this source matched), `Ok(None)` (this source did not
//! recognize the package — fall through), and `Err(TrustError)` (real
//! evaluation failure such as a corrupt config or signature error).
//!
//! Today no source produces `Err`; the variant exists because future
//! signed-registry / signature-verification work will need it. Adding the
//! variant up front keeps the trait signature stable across PRs.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TrustError {
    #[error("trust policy invariant violation: {reason}")]
    InvariantViolation { reason: String },
}
