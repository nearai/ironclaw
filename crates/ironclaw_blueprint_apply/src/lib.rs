//! Reconcile a parsed [`ironclaw_blueprint::Blueprint`] into the typed Reborn
//! repositories — slice 2 of epic
//! [#3036](https://github.com/nearai/ironclaw/issues/3036).
//!
//! The blueprint is an *input*, never a source of truth: this engine diffs the
//! desired state against the repos and emits an [`ApplyReport`] of [`Change`]s.
//! Design invariants from the epic:
//!
//! - **Idempotent**: applying twice yields zero writes the second time.
//!   Idempotence is *structural* — values are compared by content hash
//!   ([`structural_hash`]), not source text, so reformatting never produces a
//!   spurious `Update`.
//! - **Non-destructive**: a key present in a repo but absent from the blueprint
//!   is reported as [`ChangeAction::DeleteDeferred`] drift, never deleted.
//! - **Admin-scoped**: anything beyond `scope = { user = self }` requires admin
//!   authority; the scope check fails closed before any reconciler runs.
//! - **Fail-loud**: a reconciler error aborts the apply rather than silently
//!   skipping a write.
//!
//! Per-domain repos plug in via the [`DomainReconciler`] trait; this crate owns
//! the orchestration, report shape, and scope gate only.

mod error;
mod reconciler;
mod report;
mod scope;
mod service;

pub use error::{ApplyError, AuthorityError, ReconcileError};
pub use reconciler::{ApplyMode, DomainReconciler, structural_hash};
pub use report::{ApplyReport, Change, ChangeAction, Domain};
pub use scope::{Actor, ApplyScope, authorize};
pub use service::BlueprintApplyService;
