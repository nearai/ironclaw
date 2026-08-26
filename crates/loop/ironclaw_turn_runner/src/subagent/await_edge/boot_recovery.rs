//! Recovery projection for unresolved process dependencies.

use std::sync::Arc;

use ironclaw_loop_contracts::AgentLoopHostError;
use ironclaw_loop_host::{AwaitEdgeWriter, ResolveReport};
use ironclaw_threads::SessionThreadService;
use ironclaw_turns::TurnScope;

use super::{resolver::AwaitEdgeResolver, store::AwaitEdgeStore};

pub(super) async fn recover_scope<S>(
    resolver: &AwaitEdgeResolver<S>,
    store: &AwaitEdgeStore,
    scope: &TurnScope,
) -> ResolveReport
where
    S: SessionThreadService + ?Sized,
{
    let mut report = ResolveReport::default();
    let unclosed = match store.list_unclosed_for_scope(scope).await {
        Ok(edges) => edges,
        Err(error) => {
            tracing::debug!(error = %error, "process dependency recovery query failed");
            report.record_failed();
            return report;
        }
    };
    for (parent_run_id, child_run_id, edge) in unclosed {
        let outcome = match edge.state {
            super::AwaitEdgeState::Open => continue,
            // Blocking-mode `Settled` keeps its pre-existing group drain
            // (resumes the blocked parent once every group member has
            // settled). Background-mode `Settled` never reaches
            // `drain_settled_group` — `resume_parent` is exclusive to the
            // blocking dependent-run gate, which a background parent never
            // parks on (`deliver_background`'s own doc comment) — so it takes
            // the same `deliver_background` re-drive path as the other
            // background delivery substates below.
            super::AwaitEdgeState::Settled
                if edge.mode == ironclaw_loop_host::SpawnSubagentMode::Background =>
            {
                resolver
                    .deliver_background(&edge, parent_run_id, child_run_id, false)
                    .await
            }
            super::AwaitEdgeState::Settled => {
                resolver
                    .drain_settled_group(scope, parent_run_id, child_run_id)
                    .await
            }
            // Crash-recovery re-drive of a half-delivered background result
            // (System provenance — boot recovery is not a human/permitted
            // start): `ResultAppended` re-attends (append is a no-op
            // replay); `AttentionScheduled` closes only, both through
            // `deliver_background`'s existing idempotent re-drive contract.
            super::AwaitEdgeState::ResultAppended | super::AwaitEdgeState::AttentionScheduled => {
                resolver
                    .deliver_background(&edge, parent_run_id, child_run_id, false)
                    .await
            }
            // A streak-capped edge stays parked at boot — boot recovery is
            // never a human-initiated start, so it must not drain this
            // forward; a later permitted/human run-start sweep does.
            super::AwaitEdgeState::AttentionDeferredStreakCap => continue,
            super::AwaitEdgeState::Drained | super::AwaitEdgeState::Abandoned => continue,
        };
        match outcome {
            Ok(outcome) => report.record(outcome),
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    %parent_run_id,
                    %child_run_id,
                    "process dependency recovery drain failed"
                );
                report.record_failed();
            }
        }
    }
    report
}

/// Spawn-side compatibility adapter. Recovery no longer needs a roster or an
/// admission cache: unresolved dependencies are authoritative journal
/// projections and every close transition is idempotent.
pub struct ScopeRecoveryDriver<S: SessionThreadService + ?Sized> {
    resolver: Arc<AwaitEdgeResolver<S>>,
    store: Arc<AwaitEdgeStore>,
}

impl<S> ScopeRecoveryDriver<S>
where
    S: SessionThreadService + ?Sized,
{
    pub fn new(resolver: Arc<AwaitEdgeResolver<S>>, store: Arc<AwaitEdgeStore>) -> Self {
        Self { resolver, store }
    }
}

#[async_trait::async_trait]
impl<S> AwaitEdgeWriter for ScopeRecoveryDriver<S>
where
    S: SessionThreadService + ?Sized + 'static,
{
    async fn check_scope_recovered(
        &self,
        scope: &TurnScope,
    ) -> Result<(), ironclaw_loop_host::ScopeRecoveryInProgress> {
        let report = recover_scope(&self.resolver, &self.store, scope).await;
        if report.failed > 0 {
            return Err(ironclaw_loop_host::ScopeRecoveryInProgress {
                retry_after_hint: std::time::Duration::from_millis(50),
            });
        }
        Ok(())
    }

    async fn abandon_awaited_child(
        &self,
        child_scope: &TurnScope,
        parent_run_id: ironclaw_turns::TurnRunId,
        child_run_id: ironclaw_turns::TurnRunId,
    ) -> Result<(), AgentLoopHostError> {
        self.store
            .abandon_awaited_child(child_scope, parent_run_id, child_run_id)
            .await
    }
}

#[cfg(test)]
mod tests;
