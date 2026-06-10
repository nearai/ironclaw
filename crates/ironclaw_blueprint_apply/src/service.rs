//! `BlueprintApplyService` — orchestrates per-domain reconcilers.

use ironclaw_blueprint::Blueprint;

use crate::error::ApplyError;
use crate::reconciler::{ApplyMode, DomainReconciler};
use crate::report::ApplyReport;
use crate::scope::{Actor, ApplyScope, authorize};

/// Runs a set of [`DomainReconciler`]s over a blueprint, producing an
/// [`ApplyReport`]. The blueprint is an *input*: this service writes through the
/// typed repos owned by the reconcilers and never becomes a source of truth.
pub struct BlueprintApplyService {
    reconcilers: Vec<Box<dyn DomainReconciler>>,
}

impl BlueprintApplyService {
    pub fn new(reconcilers: Vec<Box<dyn DomainReconciler>>) -> Self {
        Self { reconcilers }
    }

    /// Plan (and, in [`ApplyMode::Apply`], perform) the reconcile.
    ///
    /// Order of operations is deliberate and fails closed: authorize the scope
    /// first, then for each reconciler plan the changes, and only if writes are
    /// needed and the mode is `Apply` perform them. Any reconciler error aborts
    /// the whole apply — no partial silent skips.
    pub fn apply(
        &self,
        blueprint: &Blueprint,
        actor: &Actor,
        mode: ApplyMode,
    ) -> Result<ApplyReport, ApplyError> {
        let scope = ApplyScope::from_blueprint(blueprint);
        authorize(actor, &scope)?;

        let mut report = ApplyReport::default();
        for reconciler in &self.reconcilers {
            let changes = reconciler.plan(blueprint, &scope)?;
            let has_writes = changes.iter().any(|c| c.action.is_write());
            if mode == ApplyMode::Apply && has_writes {
                reconciler.apply(blueprint, &scope, &changes)?;
            }
            report.extend(changes);
        }
        Ok(report)
    }
}
