//! Error types for the apply engine.

use thiserror::Error;

use crate::report::Domain;

/// The actor is not allowed to apply at the requested scope. Fails closed:
/// anything beyond `scope = { user = self }` requires admin authority, per the
/// epic invariant ("apply must be admin-scoped for anything beyond self").
#[derive(Debug, Clone, Error)]
#[error(
    "not authorized to apply at scope `{scope}`: {reason} \
     (cross-tenant/foreign scope requires admin authority)"
)]
pub struct AuthorityError {
    pub scope: String,
    pub reason: String,
}

/// A single reconciler failed while planning or writing. Fail-loud: a repo
/// error during apply aborts the whole apply, it never silently skips a write.
#[derive(Debug, Error)]
#[error("reconciler for domain `{domain}` failed: {reason}")]
pub struct ReconcileError {
    pub domain: Domain,
    pub reason: String,
}

impl ReconcileError {
    pub fn new(domain: Domain, reason: impl Into<String>) -> Self {
        Self {
            domain,
            reason: reason.into(),
        }
    }
}

/// Top-level apply failure.
#[derive(Debug, Error)]
pub enum ApplyError {
    #[error(transparent)]
    Authority(#[from] AuthorityError),
    #[error(transparent)]
    Reconcile(#[from] ReconcileError),
}
