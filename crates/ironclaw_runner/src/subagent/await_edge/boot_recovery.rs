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
use ironclaw_loop_host::{
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
                // Re-drive the resolver's own write -> resume -> release ->
                // prune -> delete path (`drain_settled_group`), not a bare
                // `close_edge` -- a crash after this child settled but
                // before drain ran left the parent's result reference
                // unwritten and the parent still blocked; jumping straight
                // to `close_edge` here used to delete the evidence without
                // ever writing/resuming (external review finding on this
                // PR).
                match resolver
                    .drain_settled_group(scope, parent_run_id, child_run_id)
                    .await
                {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        tracing::debug!(error = %error, %parent_run_id, %child_run_id, "await-edge recovery drain failed");
                        ironclaw_loop_host::ResolveOutcome::AlreadyClosed
                    }
                }
            }
            super::AwaitEdgeState::Drained | super::AwaitEdgeState::Abandoned => {
                match resolver
                    .close_edge(scope, parent_run_id, edge.tree_root_run_id, child_run_id)
                    .await
                {
                    Ok(()) => ironclaw_loop_host::ResolveOutcome::Drained,
                    Err(_) => ironclaw_loop_host::ResolveOutcome::AlreadyClosed,
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
///
/// Must preserve `key.user_id` as the scope's explicit owner (mirroring
/// `TurnScope::to_resource_scope`'s forward mapping in reverse): multi-user
/// edges live under the owner's mount, so a bare `TurnScope::new` here would
/// probe the system/`ActorFallback` mount and silently see zero unclosed
/// edges for every owner-scoped roster entry (external review finding on
/// this PR, #5720-class).
fn roster_key_to_probe_scope(key: &RosterKey) -> TurnScope {
    // `from_trusted` bypasses `validate_scope_id` — safe here because this
    // is a fixed literal, never caller-supplied, and never persisted or
    // resolved against a real thread (recovery only lists/closes edges by
    // scope axes). Avoids `.expect()` on a "known-valid" literal per repo
    // style (no unwrap/expect in production code).
    let owner = if key.user_id.as_str() == ironclaw_host_api::SYSTEM_RESERVED_ID {
        None
    } else {
        Some(key.user_id.clone())
    };
    TurnScope::new_with_owner(
        key.tenant_id.clone(),
        key.agent_id.clone(),
        key.project_id.clone(),
        ironclaw_host_api::ThreadId::from_trusted("await-edge-recovery-probe".to_string()),
        owner,
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
    // Unbounded in principle (no eviction) but bounded in practice by real
    // tenant/user/agent/project scope cardinality — a process-lifetime cache.
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

/// RAII release of one `in_progress` claim, so a panic anywhere in the
/// spawned lazy-recovery task (most notably inside `recover_scope`) still
/// unblocks the scope for a future admission attempt instead of wedging it
/// shut forever. Hand-rolled rather than pulling in `scopeguard` — the whole
/// type is this one field plus a three-line `Drop` impl.
struct InProgressReleaseGuard {
    in_progress: Arc<Mutex<HashSet<String>>>,
    key: String,
}

impl InProgressReleaseGuard {
    fn new(in_progress: Arc<Mutex<HashSet<String>>>, key: String) -> Self {
        Self { in_progress, key }
    }
}

impl Drop for InProgressReleaseGuard {
    fn drop(&mut self) {
        let mut in_progress = self
            .in_progress
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        in_progress.remove(&self.key);
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
        // Claim this scope's first-touch check atomically *before* the async
        // `list_unclosed_for_scope` call below, not after it — otherwise N
        // concurrent first-touches on the same new/recovering scope would
        // all pass this gate while the key is still absent and each
        // redundantly run the async list call (external review finding on
        // this PR). The claim below is the single admission decision point:
        // exactly one caller sees `already_claimed == false` and goes on to
        // check/spawn recovery; every other concurrent caller for this key
        // is rejected immediately, no wasted I/O.
        let already_claimed = {
            let mut in_progress = Self::lock_set(&self.in_progress);
            if in_progress.contains(&key) {
                true
            } else {
                in_progress.insert(key.clone());
                false
            }
        };
        if already_claimed {
            return Err(ScopeRecoveryInProgress {
                retry_after_hint: Duration::from_millis(200),
            });
        }
        // This call now uniquely owns the `in_progress` claim for `key` —
        // check whether there is actually anything to recover before ever
        // rejecting admission. A scope with no unclosed edges (the
        // overwhelmingly common case — a brand new scope's very first
        // spawn) has nothing a background recovery task would do; gating it
        // behind `ScopeRecoveryInProgress` regardless would reject every
        // first-ever spawn for every scope, which is not what §5.3 intends
        // (recovery exists for scopes that *might* have unclosed edges from
        // a prior crash, not as a tax on first contact).
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
            Self::lock_set(&self.in_progress).remove(&key);
            Self::lock_set(&self.booted).insert(key);
            return Ok(());
        }
        let resolver = Arc::clone(&self.resolver);
        let store = Arc::clone(&self.store);
        let semaphore = Arc::clone(&self.semaphore);
        let in_progress = Arc::clone(&self.in_progress);
        let booted = Arc::clone(&self.booted);
        let scope = scope.clone();
        let key = key.clone();
        tokio::spawn(async move {
            // Panic-safety (external review finding on this PR): a panic
            // inside `recover_scope` must still release the `in_progress`
            // claim, or the scope is wedged shut (never admitted again)
            // until process restart. The guard is constructed before the
            // permit/recovery work so it covers the whole task, and only
            // releases `in_progress` — `booted` is intentionally left alone
            // here, since wedging admission *open* on panic (retry from
            // scratch next touch) is safer than wedging it permanently shut.
            let _release_guard = InProgressReleaseGuard::new(Arc::clone(&in_progress), key.clone());
            let _permit = semaphore.acquire().await;
            let _ = recover_scope(&resolver, &store, &scope).await;
            Self::lock_set(&booted).insert(key);
        });
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
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use ironclaw_filesystem::{
        CasExpectation, DirEntry, Entry, FileStat, FilesystemError, InMemoryBackend, RecordVersion,
        ScopedFilesystem, VersionedEntry,
    };
    use ironclaw_host_api::{
        MountAlias, MountGrant, MountPermissions, MountView, TenantId, UserId, VirtualPath,
    };
    use ironclaw_threads::{InMemorySessionThreadService, ThreadScope};
    use ironclaw_turns::TurnSpawnTreeStateStore;
    use tokio::sync::Notify;

    use super::*;

    struct NoopResultWriter;

    #[async_trait::async_trait]
    impl ironclaw_loop_host::LoopCapabilityResultWriter for NoopResultWriter {
        async fn write_capability_result(
            &self,
            _write: ironclaw_loop_host::CapabilityResultWrite<'_>,
        ) -> Result<ironclaw_loop_host::CapabilityWriteResult, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                ironclaw_turns::run_profile::AgentLoopHostErrorKind::Unavailable,
                "not exercised by shared-semaphore tests",
            ))
        }
    }

    // External review finding on this PR (#5720-class): a roster key
    // carrying a real, non-system owner must probe a scope with that same
    // explicit owner, not silently fall back to `ActorFallback` (which
    // resolves to the system mount, not the owner's) — otherwise boot
    // recovery would list zero unclosed edges for every multi-user scope.
    // Mutation: revert `roster_key_to_probe_scope` to `TurnScope::new(...)`
    // -> RED (`explicit_owner_user_id()` becomes `None` for a real owner).
    #[test]
    fn roster_key_to_probe_scope_preserves_explicit_owner() {
        let key = RosterKey {
            tenant_id: TenantId::new("probe-tenant").unwrap(),
            user_id: UserId::new("probe-owner").unwrap(),
            agent_id: None,
            project_id: None,
        };
        let scope = roster_key_to_probe_scope(&key);
        assert_eq!(
            scope.explicit_owner_user_id(),
            Some(&UserId::new("probe-owner").unwrap()),
            "probe scope must carry the roster key's owner explicitly"
        );
        assert_eq!(
            scope.to_resource_scope().user_id,
            UserId::new("probe-owner").unwrap(),
            "reverse mapping must round-trip through to_resource_scope's forward mapping"
        );
    }

    // The system-sentinel roster key (agent-scoped / ownerless edges) must
    // probe with `ActorFallback`, not an explicit "owner" of the system
    // sentinel string — mirrors `to_resource_scope`'s forward direction where
    // an absent explicit owner is *encoded* as the sentinel.
    #[test]
    fn roster_key_to_probe_scope_maps_system_sentinel_to_actor_fallback() {
        let key = RosterKey {
            tenant_id: TenantId::new("probe-tenant").unwrap(),
            user_id: UserId::from_trusted(ironclaw_host_api::SYSTEM_RESERVED_ID.to_string()),
            agent_id: None,
            project_id: None,
        };
        let scope = roster_key_to_probe_scope(&key);
        assert_eq!(scope.explicit_owner_user_id(), None);
    }

    /// Wraps an `InMemoryBackend`, counting every `list_dir` call and — on
    /// the very first call only — holding it open behind a `Notify` gate
    /// until the test explicitly releases it, so a second concurrent caller
    /// can be raced against the first while it is provably still in flight.
    /// Same delegating-decorator shape as
    /// `ironclaw_authorization`'s `CountingFilesystem` test helper
    /// (`capability_lease_contract.rs`), adapted to add the gate.
    struct GatedCountingBackend {
        inner: InMemoryBackend,
        list_dir_calls: Arc<AtomicUsize>,
        gate_armed: AtomicBool,
        entered: Notify,
        release: Notify,
    }

    impl GatedCountingBackend {
        fn new(inner: InMemoryBackend) -> Self {
            Self {
                inner,
                list_dir_calls: Arc::new(AtomicUsize::new(0)),
                gate_armed: AtomicBool::new(true),
                entered: Notify::new(),
                release: Notify::new(),
            }
        }

        fn list_dir_calls(&self) -> usize {
            self.list_dir_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl RootFilesystem for GatedCountingBackend {
        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            self.list_dir_calls.fetch_add(1, Ordering::SeqCst);
            if self.gate_armed.swap(false, Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            self.inner.list_dir(path).await
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            self.inner.stat(path).await
        }

        async fn put(
            &self,
            path: &VirtualPath,
            entry: Entry,
            cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            self.inner.put(path, entry, cas).await
        }

        async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
            self.inner.get(path).await
        }

        async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            self.inner.delete(path).await
        }

        async fn delete_if_version(
            &self,
            path: &VirtualPath,
            expected_version: RecordVersion,
        ) -> Result<(), FilesystemError> {
            self.inner.delete_if_version(path, expected_version).await
        }
    }

    // External review finding on this PR: `check_scope_recovered`'s claim
    // (the sync `in_progress` insert) must happen *before* the async
    // `list_unclosed_for_scope` call, not after — otherwise two concurrent
    // first-touches for the same never-seen scope would both pass the
    // claim check while the first is still awaiting its list call, and both
    // redundantly run it. Proven here by gating the backend's first
    // `list_dir` call open and racing a second `check_scope_recovered` call
    // against it while it is provably still in flight.
    // Mutation: swap the two blocks in `check_scope_recovered` (list before
    // claim) -> RED (`list_dir_calls()` observes 2, and/or B is wrongly
    // admitted instead of rejected).
    #[tokio::test]
    async fn check_scope_recovered_claims_before_listing_so_two_concurrent_first_touches_list_exactly_once()
     {
        let backend = Arc::new(GatedCountingBackend::new(InMemoryBackend::new()));
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/turns").unwrap(),
            VirtualPath::new("/turns").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap();
        let fs = Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::clone(&backend),
            mounts,
        ));
        let store = Arc::new(FilesystemAwaitEdgeStore::new(Arc::clone(&fs)));
        let goal_store: Arc<dyn ironclaw_loop_host::SubagentSpawnGoalStore> =
            Arc::new(crate::subagent::goal_store::InMemoryBoundedSubagentGoalStore::new());
        let turn_state_store: Arc<dyn TurnSpawnTreeStateStore> =
            Arc::new(ironclaw_turns::test_support::in_memory_turn_state_store());
        let result_writer: Arc<dyn ironclaw_loop_host::LoopCapabilityResultWriter> =
            Arc::new(NoopResultWriter);
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let resolver = Arc::new(AwaitEdgeResolver::new_unbound(
            Arc::clone(&store),
            goal_store,
            turn_state_store,
            result_writer,
            thread_service,
        ));
        let driver = Arc::new(ScopeRecoveryDriver::new(resolver, store));

        let scope = TurnScope::new(
            TenantId::new("concurrent-first-touch-tenant").unwrap(),
            None,
            None,
            ironclaw_host_api::ThreadId::from_trusted("concurrent-first-touch-thread".to_string()),
        );

        let driver_a = Arc::clone(&driver);
        let scope_a = scope.clone();
        let mut task_a =
            tokio::spawn(async move { driver_a.check_scope_recovered(&scope_a).await });

        // Wait until A's list call is actually in flight before racing B
        // against it -- not a fixed sleep.
        backend.entered.notified().await;

        let result_b = driver.check_scope_recovered(&scope).await;
        assert!(
            result_b.is_err(),
            "a concurrent first-touch for the same never-seen scope must be rejected \
             while A's claim is live, not independently re-list"
        );
        assert_eq!(
            backend.list_dir_calls(),
            1,
            "claim must be staked before the async list call so a second concurrent \
             caller never reaches it"
        );

        backend.release.notify_one();

        let result_a = tokio::time::timeout(Duration::from_secs(5), &mut task_a)
            .await
            .expect("task a should not hang")
            .expect("task a should not panic");
        assert!(
            result_a.is_ok(),
            "the sole caller that actually listed sees no unclosed edges on a \
             never-touched scope and must be admitted"
        );
        assert_eq!(backend.list_dir_calls(), 1);
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
        let goal_store: Arc<dyn ironclaw_loop_host::SubagentSpawnGoalStore> =
            Arc::new(crate::subagent::goal_store::InMemoryBoundedSubagentGoalStore::new());
        let turn_state_store: Arc<dyn TurnSpawnTreeStateStore> =
            Arc::new(ironclaw_turns::test_support::in_memory_turn_state_store());
        let result_writer: Arc<dyn ironclaw_loop_host::LoopCapabilityResultWriter> =
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

    // A crash-settled edge must re-drive write+resume, not just `close_edge`,
    // or the parent stays blocked forever (external review, PR #5819).
    // Mutation: revert the `Settled` branch to a bare `close_edge` call ->
    // RED (parent never leaves `BlockedDependentRun`, `report.resumed == 0`).
    #[tokio::test]
    async fn recover_scope_redrives_write_and_resume_for_a_crash_settled_undrained_edge() {
        use ironclaw_turns::{
            DefaultTurnCoordinator, GateRef, SubmitChildRunRequest, SubmitTurnRequest, TurnActor,
            TurnCoordinator, TurnSpawnTreePort, runner::TurnRunTransitionPort,
        };

        let fs = boot_sem_scoped_fs();
        let store = Arc::new(FilesystemAwaitEdgeStore::new(Arc::clone(&fs)));
        let state_store = Arc::new(ironclaw_turns::test_support::in_memory_turn_state_store());
        let coordinator = DefaultTurnCoordinator::new(Arc::clone(&state_store));
        let thread_service = Arc::new(InMemorySessionThreadService::default());

        let tenant_id = ironclaw_host_api::TenantId::new("recover-settled-tenant").unwrap();
        let agent_id = ironclaw_host_api::AgentId::new("recover-settled-agent").unwrap();
        let owner = UserId::new("recover-settled-owner").unwrap();
        let actor = TurnActor::new(owner.clone());
        let parent_thread_id =
            ironclaw_host_api::ThreadId::new("recover-settled-parent-thread").unwrap();
        let parent_scope = TurnScope::new_with_owner(
            tenant_id.clone(),
            Some(agent_id.clone()),
            None,
            parent_thread_id.clone(),
            Some(owner.clone()),
        );

        // 1. Submit + block the parent on a dependent-run gate.
        let ironclaw_turns::SubmitTurnResponse::Accepted {
            run_id: parent_run_id,
            ..
        } = coordinator
            .submit_turn(SubmitTurnRequest {
                requested_model: None,
                scope: parent_scope.clone(),
                actor: actor.clone(),
                accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new(
                    "msg:recover-settled-parent",
                )
                .unwrap(),
                source_binding_ref: ironclaw_turns::SourceBindingRef::new(
                    "source:recover-settled-parent",
                )
                .unwrap(),
                reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new(
                    "reply:recover-settled-parent",
                )
                .unwrap(),
                requested_run_profile: None,
                idempotency_key: ironclaw_turns::IdempotencyKey::new("idem:recover-settled-parent")
                    .unwrap(),
                received_at: chrono::Utc::now(),
                requested_run_id: None,
                parent_run_id: None,
                subagent_depth: 0,
                spawn_tree_root_run_id: None,
                product_context: None,
            })
            .await
            .unwrap();
        let runner_id = ironclaw_turns::TurnRunnerId::new();
        let lease_token = ironclaw_turns::TurnLeaseToken::new();
        state_store
            .claim_next_run(ironclaw_turns::runner::ClaimRunRequest {
                runner_id,
                lease_token,
                scope_filter: None,
            })
            .await
            .unwrap()
            .expect("parent run claimable");
        let gate_ref = GateRef::new("gate:recover-settled-test").unwrap();
        state_store
            .block_run(ironclaw_turns::runner::BlockRunRequest {
                run_id: parent_run_id,
                runner_id,
                lease_token,
                checkpoint_id: ironclaw_turns::TurnCheckpointId::new(),
                state_ref: ironclaw_turns::run_profile::LoopCheckpointStateRef::new(
                    "checkpoint:recover-settled-test",
                )
                .unwrap(),
                reason: ironclaw_turns::BlockedReason::AwaitDependentRun {
                    gate_ref: gate_ref.clone(),
                },
            })
            .await
            .unwrap();

        // 2. Submit the child as a real lineage child of the parent (its own
        // run status never needs to advance -- the edge below carries the
        // already-`Settled` terminal state directly, simulating "crashed
        // after settle, before drain").
        let child_thread_id =
            ironclaw_host_api::ThreadId::new("recover-settled-child-thread").unwrap();
        let child_scope = TurnScope::new_with_owner(
            tenant_id.clone(),
            Some(agent_id.clone()),
            None,
            child_thread_id.clone(),
            Some(owner.clone()),
        );
        let ironclaw_turns::SubmitTurnResponse::Accepted {
            run_id: child_run_id,
            ..
        } = coordinator
            .submit_child_run(SubmitChildRunRequest {
                parent_scope: parent_scope.clone(),
                parent_run_id,
                child_scope: child_scope.clone(),
                actor: actor.clone(),
                accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new(
                    "msg:recover-settled-child",
                )
                .unwrap(),
                source_binding_ref: ironclaw_turns::SourceBindingRef::new(
                    "source:recover-settled-child",
                )
                .unwrap(),
                reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new(
                    "reply:recover-settled-child",
                )
                .unwrap(),
                requested_run_profile: None,
                idempotency_key: ironclaw_turns::IdempotencyKey::new("idem:recover-settled-child")
                    .unwrap(),
                received_at: chrono::Utc::now(),
                requested_run_id: None,
                spawn_tree_descendant_cap: 16,
            })
            .await
            .unwrap();

        // 3. Seed both threads and the parent's spawn-time tool-result
        // placeholder.
        thread_service
            .ensure_thread(ironclaw_threads::EnsureThreadRequest {
                scope: ThreadScope {
                    tenant_id: tenant_id.clone(),
                    agent_id: agent_id.clone(),
                    project_id: None,
                    owner_user_id: Some(owner.clone()),
                    mission_id: None,
                },
                thread_id: Some(child_thread_id.clone()),
                created_by_actor_id: "test".to_string(),
                title: Some("Subagent".to_string()),
                metadata_json: None,
            })
            .await
            .unwrap();
        thread_service
            .ensure_thread(ironclaw_threads::EnsureThreadRequest {
                scope: ThreadScope {
                    tenant_id: tenant_id.clone(),
                    agent_id: agent_id.clone(),
                    project_id: None,
                    owner_user_id: Some(owner.clone()),
                    mission_id: None,
                },
                thread_id: Some(parent_thread_id.clone()),
                created_by_actor_id: "test".to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        let result_ref =
            ironclaw_turns::LoopResultRef::new("result:subagent.recover-settled").unwrap();
        thread_service
            .append_tool_result_reference(ironclaw_threads::AppendToolResultReferenceRequest {
                scope: ThreadScope {
                    tenant_id: tenant_id.clone(),
                    agent_id: agent_id.clone(),
                    project_id: None,
                    owner_user_id: Some(owner.clone()),
                    mission_id: None,
                },
                thread_id: parent_thread_id.clone(),
                turn_run_id: parent_run_id.to_string(),
                result_ref: result_ref.as_str().to_string(),
                safe_summary: ironclaw_threads::ToolResultSafeSummary::new("subagent spawned")
                    .unwrap(),
                provider_call: None,
                model_observation: None,
            })
            .await
            .unwrap();

        // 4. Open the edge already in `Settled` state -- simulating a crash
        // that landed after the settle CAS write but before drain ran.
        let mut parent_run_context =
            ironclaw_agent_loop::test_support::test_run_context("recover-settled-parent-ctx");
        parent_run_context.scope = parent_scope.clone();
        parent_run_context.thread_id = parent_thread_id.clone();
        parent_run_context.run_id = parent_run_id;
        parent_run_context.actor = Some(actor.clone());
        let edge = super::super::AwaitEdge {
            child_scope: child_scope.clone(),
            child_thread_id: child_thread_id.clone(),
            parent_thread_id: parent_thread_id.clone(),
            parent_run_context,
            tree_root_run_id: parent_run_id,
            gate_ref: gate_ref.clone(),
            source_binding_ref: ironclaw_turns::SourceBindingRef::new(
                "subagent-source:recover-settled",
            )
            .unwrap(),
            reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new(
                "subagent-reply:recover-settled",
            )
            .unwrap(),
            subagent_kind: ironclaw_loop_host::SubagentKindId::new("general").unwrap(),
            spawn_capability_id: ironclaw_host_api::CapabilityId::new(
                ironclaw_loop_host::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID,
            )
            .unwrap(),
            result_ref,
            mode: ironclaw_loop_host::SpawnSubagentMode::Blocking,
            state: super::super::AwaitEdgeState::Settled,
            terminal_kind: Some(super::super::EdgeTerminalKind::Completed),
            terminal_byte_len: None,
            terminal_reason: None,
            reservation_release: super::super::ReservationReleaseState::Unclaimed,
            created_at: chrono::Utc::now(),
            settled_at: Some(chrono::Utc::now()),
        };
        store
            .open(&child_scope, parent_run_id, child_run_id, edge)
            .await
            .unwrap();

        // 5. Build the resolver, bind the real coordinator, and re-drive
        // recovery over this exact scope.
        let goal_store: Arc<dyn ironclaw_loop_host::SubagentSpawnGoalStore> =
            Arc::new(crate::subagent::goal_store::InMemoryBoundedSubagentGoalStore::new());
        let turn_state_store: Arc<dyn TurnSpawnTreeStateStore> = state_store.clone();
        let resolver = Arc::new(AwaitEdgeResolver::new_unbound(
            Arc::clone(&store),
            goal_store,
            turn_state_store,
            Arc::new(NoopResultWriter),
            Arc::clone(&thread_service),
        ));
        let coordinator_dyn: Arc<dyn ironclaw_turns::TurnCoordinator> = Arc::new(coordinator);
        resolver
            .bind_coordinator(Arc::clone(&coordinator_dyn))
            .unwrap();

        let report = recover_scope(&resolver, &store, &child_scope).await;
        assert_eq!(
            report.resumed, 1,
            "recovery must actually drive the write+resume path, not just close the edge"
        );
        assert_eq!(report.failed, 0);

        // 6. The parent actually left `BlockedDependentRun` -- not stuck.
        let parent_state = coordinator_dyn
            .get_run_state(ironclaw_turns::GetRunStateRequest {
                scope: parent_scope,
                run_id: parent_run_id,
            })
            .await
            .unwrap();
        assert_ne!(
            parent_state.status,
            ironclaw_turns::TurnStatus::BlockedDependentRun,
            "the parent must actually resume, not stay stuck on its dependent-run gate"
        );

        // 7. The edge is actually gone -- the close half of the sequence
        // still ran too.
        assert!(
            store
                .list_unclosed_for_scope(&child_scope)
                .await
                .unwrap()
                .is_empty()
        );
    }

    // External review finding on this PR: a panic inside the lazy-recovery
    // task (most likely `recover_scope`) must not leave the scope's
    // `in_progress` claim set forever -- that would wedge the scope shut,
    // rejecting every future admission attempt until process restart.
    // Exercises `InProgressReleaseGuard` directly (not the full spawned-task
    // path, which needs cooperative-scheduling yields to observe a
    // backgrounded panic and would be flaky under `#[tokio::test]`'s
    // current-thread runtime) -- this still pins the exact defect: the guard
    // must release the claim across an unwind, not just on a normal return.
    // Mutation: delete the `Drop` impl's body (or skip constructing the
    // guard) -> RED (`in_progress` still contains the key after the panic).
    #[test]
    fn in_progress_release_guard_releases_the_claim_across_a_panic_unwind() {
        let in_progress: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        in_progress
            .lock()
            .unwrap()
            .insert("panic-guard-scope-key".to_string());

        let guard_in_progress = Arc::clone(&in_progress);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = InProgressReleaseGuard::new(
                Arc::clone(&guard_in_progress),
                "panic-guard-scope-key".to_string(),
            );
            panic!("simulated recover_scope crash");
        }));
        assert!(unwound.is_err(), "the closure should have panicked");
        assert!(
            !in_progress
                .lock()
                .unwrap()
                .contains("panic-guard-scope-key"),
            "the in_progress claim must be released even when the recovery task \
             panics, or the scope is wedged shut until process restart"
        );
    }
}
