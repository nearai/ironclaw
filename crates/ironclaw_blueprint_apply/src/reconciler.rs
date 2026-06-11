//! The per-domain reconciler contract.

use ironclaw_blueprint::Blueprint;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::ReconcileError;
use crate::report::{Change, Domain};
use crate::scope::ApplyScope;

/// Whether an apply writes or only reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyMode {
    /// Plan only; never touch a repo.
    DryRun,
    /// Plan, then write the `Create`/`Update` changes.
    Apply,
}

/// Reconciles one blueprint domain into its typed repo.
///
/// Two-phase by contract: [`plan`](DomainReconciler::plan) is pure and produces
/// the [`Change`] list (powering dry-run and idempotence); [`apply`] performs
/// only the planned writes and must be idempotent and transactional. A
/// reconciler must never emit a destructive action for a key merely absent from
/// the blueprint — drift is reported as `DeleteDeferred`, never deleted here.
pub trait DomainReconciler {
    fn domain(&self) -> Domain;

    /// Compute the changes needed to bring the repo to the blueprint's desired
    /// state. Pure — no writes, no side effects.
    fn plan(
        &self,
        blueprint: &Blueprint,
        scope: &ApplyScope,
    ) -> Result<Vec<Change>, ReconcileError>;

    /// Persist the planned `Create`/`Update` changes. Called only in
    /// [`ApplyMode::Apply`]. Re-applying the same plan must be a no-op.
    fn apply(
        &self,
        blueprint: &Blueprint,
        scope: &ApplyScope,
        changes: &[Change],
    ) -> Result<(), ReconcileError>;
}

/// SHA-256 (lowercase hex) of a value's canonical JSON form. Hashing the typed
/// value — not source text — makes idempotence *structural*: reformatting or
/// reordering the blueprint source cannot produce a spurious `Update`.
///
/// Fails loud rather than hashing a fallback: a serialization error must not
/// silently collapse two distinct values to the same hash.
pub fn structural_hash<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    let json = serde_json::to_vec(value)?;
    let mut hasher = Sha256::new();
    hasher.update(&json);
    Ok(hex::encode(hasher.finalize()))
}
