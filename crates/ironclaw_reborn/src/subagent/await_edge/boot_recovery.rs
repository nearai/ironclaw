//! Boot-driven roster walk + lazy per-scope admission backstop (§4.3, §5.3).
//! Split from `resolver.rs` (plan-review fix) — different trigger (process
//! boot / admission calls, not a lifecycle event) and unrelated primitives
//! (a bounded scheduler vs. a single store CAS call).
//!
//! **Scope trim vs. the design, reported explicitly**: the full design
//! specifies a shared `Semaphore(4)` + a *bounded pending queue* with a
//! *per-tenant in-flight cap* (round-5, §4.3) so a cold-boot burst from one
//! tenant cannot starve every other tenant's recovery. This implementation
//! ships the round-4 floor — one shared semaphore across boot and lazy
//! recovery (`run_boot_recovery` takes the same `Arc<Semaphore>` the
//! `ScopeRecoveryDriver` it runs alongside uses, via
//! [`ScopeRecoveryDriver::semaphore`]) — plus the `in_progress` dedupe guard
//! (§5.3's core admission contract: a scope with in-flight recovery returns
//! `ScopeRecoveryInProgress` immediately, never blocks the caller). The
//! round-5 bounded-pending-queue-with-per-tenant-cap fairness refinement is
//! still deferred to PR2's boot-wiring change (`run_boot_recovery` has zero
//! production callers in PR1, so there is nothing yet to starve) — a
//! saturated semaphore currently blocks the *background task* (not the
//! caller) rather than dropping a queued task per the round-5 fairness rule.
//! Named here as a real, reported scope cut, not silently dropped.

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};

use ironclaw_filesystem::RootFilesystem;
use ironclaw_loop_support::{
    AwaitEdgeWriter, AwaitedChildSetRecord, ResolveReport, ScopeRecoveryInProgress,
};
use ironclaw_threads::SessionThreadService;
use ironclaw_turns::{TurnScope, run_profile::AgentLoopHostError};
use tokio::sync::Semaphore;

use super::{
    resolver::AwaitEdgeResolver,
    roster::{self, RosterKey},
    store::FilesystemAwaitEdgeStore,
};

/// Shared across boot and lazy recovery (round-4 fix: one limiter, not a
/// separate pool per origin).
pub const BOOT_RECOVERY_MAX_CONCURRENT_SCOPES: usize = 4;

/// Drives one scope's unclosed edges through the resolver's close machinery
/// (settle-if-still-open -> write -> resume -> release -> prune -> delete),
/// used by both the boot pass and a lazy first-touch recovery task.
async fn recover_scope<S, F>(
    resolver: &AwaitEdgeResolver<S, F>,
    store: &FilesystemAwaitEdgeStore<F>,
    scope: &TurnScope,
) -> ResolveReport
where
    S: SessionThreadService + ?Sized,
    F: RootFilesystem + ?Sized,
{
    let mut report = ResolveReport::default();
    let unclosed = match store.list_unclosed_for_scope(scope).await {
        Ok(edges) => edges,
        Err(error) => {
            tracing::debug!(error = %error, "await-edge scope recovery failed to list unclosed edges");
            report.record_failed();
            return report;
        }
    };
    for (parent_run_id, child_run_id, edge) in unclosed {
        let outcome = match edge.state {
            super::AwaitEdgeState::Open => {
                // Crash before settle: derive a synthetic terminal event
                // isn't safe without the child's real run record, so this
                // path re-enters via the resolver's own reconstruction —
                // recovery leans on the next lifecycle event / lazy touch
                // for this specific narrow window rather than guessing a
                // terminal status here.
                continue;
            }
            super::AwaitEdgeState::Settled => {
                match resolver
                    .close_edge(scope, parent_run_id, child_run_id)
                    .await
                {
                    Ok(()) => ironclaw_loop_support::ResolveOutcome::Drained,
                    Err(error) => {
                        tracing::debug!(error = %error, %parent_run_id, %child_run_id, "await-edge recovery close failed");
                        ironclaw_loop_support::ResolveOutcome::AlreadyClosed
                    }
                }
            }
            super::AwaitEdgeState::Drained | super::AwaitEdgeState::Abandoned => {
                match resolver
                    .close_edge(scope, parent_run_id, child_run_id)
                    .await
                {
                    Ok(()) => ironclaw_loop_support::ResolveOutcome::Drained,
                    Err(_) => ironclaw_loop_support::ResolveOutcome::AlreadyClosed,
                }
            }
        };
        report.record(outcome);
    }
    report
}

/// Boot-time roster walk (§4.3): enumerate every scope with unclosed edges
/// and drive each one's recovery, bounded by the caller-supplied `semaphore`
/// — the *same* `Arc<Semaphore>` a co-running [`ScopeRecoveryDriver`]'s lazy
/// backstop uses (via [`ScopeRecoveryDriver::semaphore`]), per the round-4
/// "one limiter, not a separate pool per origin" ruling. Callers must pass
/// `Arc::clone` of that shared semaphore, never a freshly constructed one.
pub async fn run_boot_recovery<S, F>(
    resolver: Arc<AwaitEdgeResolver<S, F>>,
    fs: Arc<ironclaw_filesystem::ScopedFilesystem<F>>,
    semaphore: Arc<Semaphore>,
) -> ResolveReport
where
    S: SessionThreadService + ?Sized + 'static,
    F: RootFilesystem + ?Sized + 'static,
{
    let keys = roster::walk_roster_shards(&fs).await;
    let mut report = ResolveReport::default();
    let mut handles = Vec::new();
    for key in keys {
        let semaphore = Arc::clone(&semaphore);
        let resolver = Arc::clone(&resolver);
        let store = Arc::clone(resolver.store());
        handles.push(tokio::spawn(async move {
            let Ok(_permit) = semaphore.acquire_owned().await else {
                return ResolveReport::default();
            };
            let scope = roster_key_to_probe_scope(&key);
            recover_scope(&resolver, &store, &scope).await
        }));
    }
    for handle in handles {
        if let Ok(scope_report) = handle.await {
            report.resumed += scope_report.resumed;
            report.drained += scope_report.drained;
            report.abandoned += scope_report.abandoned;
            report.already_closed += scope_report.already_closed;
            report.failed += scope_report.failed;
        }
    }
    report
}

/// A `TurnScope` carrying only the roster key's axes, for recovery-only use
/// (listing/closing edges never needs a real `ThreadId`). The literal
/// placeholder thread id is never persisted or resolved against — it exists
/// only because `TurnScope` requires the field.
fn roster_key_to_probe_scope(key: &RosterKey) -> TurnScope {
    // `from_trusted` bypasses `validate_scope_id` — safe here because this
    // is a fixed literal, never caller-supplied, and never persisted or
    // resolved against a real thread (recovery only lists/closes edges by
    // scope axes). Avoids `.expect()` on a "known-valid" literal per repo
    // style (no unwrap/expect in production code).
    TurnScope::new(
        key.tenant_id.clone(),
        key.agent_id.clone(),
        key.project_id.clone(),
        ironclaw_host_api::ThreadId::from_trusted("await-edge-recovery-probe".to_string()),
    )
}

/// Lazy per-scope admission backstop (§5.3): `AwaitEdgeWriter::check_scope_recovered`'s
/// real implementation. Wraps a `FilesystemAwaitEdgeStore` and implements
/// `AwaitEdgeWriter` by delegating writes to it while adding the admission
/// check on top.
pub struct ScopeRecoveryDriver<S: SessionThreadService + ?Sized, F: RootFilesystem + ?Sized> {
    resolver: Arc<AwaitEdgeResolver<S, F>>,
    store: Arc<FilesystemAwaitEdgeStore<F>>,
    semaphore: Arc<Semaphore>,
    // `Arc`-wrapped (not bare `Mutex<..>` fields) so the spawned recovery
    // task below can hold its own clone and update these sets on
    // completion without needing `Arc<Self>` — `check_scope_recovered` only
    // gets `&self` from the trait signature.
    in_progress: Arc<Mutex<HashSet<String>>>,
    booted: Arc<Mutex<HashSet<String>>>,
}

impl<S, F> ScopeRecoveryDriver<S, F>
where
    S: SessionThreadService + ?Sized,
    F: RootFilesystem + ?Sized,
{
    pub fn new(
        resolver: Arc<AwaitEdgeResolver<S, F>>,
        store: Arc<FilesystemAwaitEdgeStore<F>>,
    ) -> Self {
        Self {
            resolver,
            store,
            semaphore: Arc::new(Semaphore::new(BOOT_RECOVERY_MAX_CONCURRENT_SCOPES)),
            in_progress: Arc::new(Mutex::new(HashSet::new())),
            booted: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn scope_key(scope: &TurnScope) -> String {
        roster::encode_roster_filename(&RosterKey::from_resource_scope(&scope.to_resource_scope()))
    }

    fn lock_set(set: &Mutex<HashSet<String>>) -> std::sync::MutexGuard<'_, HashSet<String>> {
        set.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    /// The shared limiter this driver's lazy recovery tasks acquire against
    /// (round-4: one limiter, not a separate pool per origin). Future boot
    /// wiring must pass `Arc::clone(&driver.semaphore())` into
    /// [`run_boot_recovery`] rather than constructing a second `Semaphore`.
    pub fn semaphore(&self) -> Arc<Semaphore> {
        Arc::clone(&self.semaphore)
    }
}

#[async_trait::async_trait]
impl<S, F> AwaitEdgeWriter for ScopeRecoveryDriver<S, F>
where
    S: SessionThreadService + ?Sized + 'static,
    F: RootFilesystem + ?Sized + 'static,
{
    async fn check_scope_recovered(
        &self,
        scope: &TurnScope,
    ) -> Result<(), ScopeRecoveryInProgress> {
        let key = Self::scope_key(scope);
        if Self::lock_set(&self.booted).contains(&key) {
            return Ok(());
        }
        if Self::lock_set(&self.in_progress).contains(&key) {
            return Err(ScopeRecoveryInProgress {
                retry_after_hint: Duration::from_millis(200),
            });
        }
        // First touch for this scope in this process: check whether there is
        // actually anything to recover before ever rejecting admission. A
        // scope with no unclosed edges (the overwhelmingly common case — a
        // brand new scope's very first spawn) has nothing a background
        // recovery task would do; gating it behind `ScopeRecoveryInProgress`
        // regardless would reject every first-ever spawn for every scope,
        // which is not what §5.3 intends (recovery exists for scopes that
        // *might* have unclosed edges from a prior crash, not as a tax on
        // first contact). Only scopes the roster/edge-tree actually shows
        // unclosed work for go through the async recovery+reject path below.
        let has_unclosed_edges = match self.store.list_unclosed_for_scope(scope).await {
            Ok(edges) => !edges.is_empty(),
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    "await-edge scope-recovery check failed to list unclosed edges; \
                     treating as needing recovery rather than silently admitting"
                );
                true
            }
        };
        if !has_unclosed_edges {
            Self::lock_set(&self.booted).insert(key);
            return Ok(());
        }
        let already_running = {
            let mut in_progress = Self::lock_set(&self.in_progress);
            if in_progress.contains(&key) {
                true
            } else {
                in_progress.insert(key.clone());
                false
            }
        };
        if !already_running {
            let resolver = Arc::clone(&self.resolver);
            let store = Arc::clone(&self.store);
            let semaphore = Arc::clone(&self.semaphore);
            let in_progress = Arc::clone(&self.in_progress);
            let booted = Arc::clone(&self.booted);
            let scope = scope.clone();
            let key = key.clone();
            tokio::spawn(async move {
                let _permit = semaphore.acquire().await;
                let _ = recover_scope(&resolver, &store, &scope).await;
                Self::lock_set(&in_progress).remove(&key);
                Self::lock_set(&booted).insert(key);
            });
        }
        Err(ScopeRecoveryInProgress {
            retry_after_hint: Duration::from_millis(200),
        })
    }

    async fn record_awaited_child(
        &self,
        record: AwaitedChildSetRecord,
    ) -> Result<(), AgentLoopHostError> {
        self.store.record_awaited_child(record).await
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
mod tests {
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        MountAlias, MountGrant, MountPermissions, MountView, TenantId, UserId, VirtualPath,
    };
    use ironclaw_threads::InMemorySessionThreadService;
    use ironclaw_turns::TurnSpawnTreeStateStore;

    use super::*;

    struct NoopResultWriter;

    #[async_trait::async_trait]
    impl ironclaw_loop_support::LoopCapabilityResultWriter for NoopResultWriter {
        async fn write_capability_result(
            &self,
            _write: ironclaw_loop_support::CapabilityResultWrite<'_>,
        ) -> Result<ironclaw_loop_support::CapabilityWriteResult, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                ironclaw_turns::run_profile::AgentLoopHostErrorKind::Unavailable,
                "not exercised by shared-semaphore tests",
            ))
        }
    }

    fn boot_sem_scoped_fs() -> Arc<ScopedFilesystem<InMemoryBackend>> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/turns").unwrap(),
            VirtualPath::new("/turns").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap();
        Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::new()),
            mounts,
        ))
    }

    fn boot_sem_resolver(
        fs: Arc<ScopedFilesystem<InMemoryBackend>>,
    ) -> Arc<AwaitEdgeResolver<InMemorySessionThreadService, InMemoryBackend>> {
        let store = Arc::new(FilesystemAwaitEdgeStore::new(fs));
        let goal_store: Arc<dyn ironclaw_loop_support::SubagentSpawnGoalStore> =
            Arc::new(crate::subagent::goal_store::InMemoryBoundedSubagentGoalStore::new());
        let turn_state_store: Arc<dyn TurnSpawnTreeStateStore> =
            Arc::new(ironclaw_turns::InMemoryTurnStateStore::default());
        let result_writer: Arc<dyn ironclaw_loop_support::LoopCapabilityResultWriter> =
            Arc::new(NoopResultWriter);
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        Arc::new(AwaitEdgeResolver::new_unbound(
            store,
            goal_store,
            turn_state_store,
            result_writer,
            thread_service,
        ))
    }

    // Required test (§4.3 round-4 fix, boot_recovery.rs module header):
    // `run_boot_recovery` and `ScopeRecoveryDriver`'s lazy backstop must
    // contend for the *same* semaphore, not one each. Proven by having a
    // lazy-shaped caller hold every permit on `driver.semaphore()`, then
    // driving `run_boot_recovery` against `Arc::clone` of that exact
    // semaphore and asserting it is blocked until the held permits are
    // released — a boot pass with its own separate semaphore would complete
    // immediately regardless of what the lazy origin was holding.
    // Mutation: give `run_boot_recovery` its own freshly constructed
    // `Semaphore::new(BOOT_RECOVERY_MAX_CONCURRENT_SCOPES)` internally
    // instead of taking the caller's -> RED (boot no longer blocks).
    #[tokio::test]
    async fn boot_and_lazy_recovery_share_one_semaphore_not_separate_pools() {
        let fs = boot_sem_scoped_fs();
        let resolver = boot_sem_resolver(Arc::clone(&fs));
        let store = Arc::new(FilesystemAwaitEdgeStore::new(Arc::clone(&fs)));
        let driver = ScopeRecoveryDriver::new(Arc::clone(&resolver), store);

        // Seed exactly one roster entry so boot's walk has one scope to
        // attempt a permit acquisition for.
        let roster_key = RosterKey {
            tenant_id: TenantId::new("boot-sem-tenant").unwrap(),
            user_id: UserId::new("boot-sem-user").unwrap(),
            agent_id: None,
            project_id: None,
        };
        roster::touch_roster_marker(&fs, &roster_key).await.unwrap();

        // Simulate `BOOT_RECOVERY_MAX_CONCURRENT_SCOPES` lazy-origin
        // recovery tasks already holding every permit on the shared
        // limiter.
        let shared_semaphore = driver.semaphore();
        let held_permits = shared_semaphore
            .try_acquire_many(BOOT_RECOVERY_MAX_CONCURRENT_SCOPES as u32)
            .expect("semaphore should start with every permit free");

        let mut boot_handle = tokio::spawn(run_boot_recovery(
            Arc::clone(&resolver),
            Arc::clone(&fs),
            Arc::clone(&shared_semaphore),
        ));

        let raced = tokio::time::timeout(Duration::from_millis(150), &mut boot_handle).await;
        assert!(
            raced.is_err(),
            "boot recovery completed while the shared semaphore was fully held \
             by another origin — it must be blocked on the SAME limiter, proving \
             it is not acquiring against a separate pool"
        );

        drop(held_permits);

        tokio::time::timeout(Duration::from_secs(5), boot_handle)
            .await
            .expect("boot recovery should complete once the shared semaphore frees up")
            .expect("boot recovery task should not panic");
    }
}
