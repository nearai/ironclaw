//! Recovery projection for unresolved process dependencies.

use std::sync::Arc;

use ironclaw_loop_host::{AwaitEdgeWriter, ResolveReport};
use ironclaw_threads::SessionThreadService;
use ironclaw_turns::{TurnScope, run_profile::AgentLoopHostError};

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
            super::AwaitEdgeState::Settled => {
                resolver
                    .drain_settled_group(scope, parent_run_id, child_run_id)
                    .await
            }
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
        let _ = recover_scope(&self.resolver, &self.store, scope).await;
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
