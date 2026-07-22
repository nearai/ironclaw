//! Journaled filesystem-backed resource governor.
//!
//! Resource quotas are process-global in the hosted runtime: one process owns
//! the authoritative reservation/tally state and persists an append-only delta
//! journal through the caller's [`RootFilesystem`]. That makes in-process
//! authority sound for quota decisions while avoiding per-reservation database
//! transactions. This is not a distributed quota service: deployments must not
//! run multiple independent filesystem governors over the same quota domain.
//! Multi-replica deployments need one elected authority for each quota domain
//! or a different distributed admission primitive. Durable recovery loads the compacted
//! [`ResourceGovernorSnapshot`] written through [`FilesystemResourceGovernorStore`]
//! and replays `/resources/deltas/log` from the snapshot cursor.
//!
//! [`FilesystemResourceGovernorStore`] remains the CAS snapshot mechanism for
//! compaction only. Hot reserve/reconcile/release paths update per-account
//! shards in memory, enqueue one delta, and ack the caller after the group
//! commit flusher durably appends that delta.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ironclaw_filesystem::{FilesystemError, RootFilesystem, ScopedFilesystem, SeqNo};
use ironclaw_host_api::{ResourceReservation, ResourceReservationId, ResourceScope};
use tracing::warn;

use crate::cas_snapshot::{AsyncStorageWorkerPoolCell, new_worker_pool_cell, run_on_worker_pool};
use crate::{
    AccountSnapshot, BudgetEvent, BudgetEventSink, Clock, FilesystemResourceGovernorStore,
    NoOpBudgetEventSink, ReservationOutcome, ResourceAccount, ResourceError, ResourceGovernor,
    ResourceGovernorStore, ResourceLimits, ResourceReceipt, ResourceTally, SystemClock,
    account_snapshot_in_state, advance_period_if_rolled_over, emit_reserve_events,
    most_specific_account, reconcile_in_state, release_in_state, reserve_with_outcome_in_state,
    set_limit_in_state, validate_reservation_in_state,
};
use crate::{ResourceEstimate, ResourceUsage};

mod authority;
mod journal;

use authority::ResourceAuthority;
#[cfg(test)]
use journal::JournalRestartHook;
use journal::{
    PendingResourceDelta, ResourceDeltaJournal, ResourceGovernorDelta,
    compact_resource_governor_snapshot, replay_journal,
};

const DEFAULT_COMPACTION_INTERVAL: usize = 1024;

enum AuthorityLifecycle {
    Vacant,
    Ready(Arc<ResourceAuthority>),
    Recovering,
}

#[cfg(test)]
type PostCommitHook = Arc<dyn Fn(&Arc<ResourceAuthority>) + Send + Sync>;

/// Filesystem-backed governor with process-local quota authority.
pub struct FilesystemResourceGovernor<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    snapshot_store: FilesystemResourceGovernorStore<F>,
    authority: Mutex<AuthorityLifecycle>,
    delta_journal: ResourceDeltaJournal<F>,
    workers: AsyncStorageWorkerPoolCell,
    clock: Arc<dyn Clock>,
    event_sink: Arc<dyn BudgetEventSink>,
    compaction_interval: usize,
    deltas_since_compaction: AtomicUsize,
    compaction_in_flight: Arc<AtomicBool>,
    #[cfg(test)]
    post_commit_hook: Option<PostCommitHook>,
}

impl<F> FilesystemResourceGovernor<F>
where
    F: RootFilesystem + 'static,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            snapshot_store: FilesystemResourceGovernorStore::new(Arc::clone(&filesystem)),
            delta_journal: ResourceDeltaJournal::new(Arc::clone(&filesystem)),
            filesystem,
            authority: Mutex::new(AuthorityLifecycle::Vacant),
            workers: new_worker_pool_cell(),
            clock: Arc::new(SystemClock),
            event_sink: Arc::new(NoOpBudgetEventSink),
            compaction_interval: DEFAULT_COMPACTION_INTERVAL,
            deltas_since_compaction: AtomicUsize::new(0),
            compaction_in_flight: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            post_commit_hook: None,
        }
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_event_sink(mut self, sink: Arc<dyn BudgetEventSink>) -> Self {
        self.event_sink = sink;
        self
    }

    /// Load and replay the durable governor state before hot-path
    /// reservations arrive.
    pub fn warm_authority(&self) -> Result<(), ResourceError> {
        self.authority()?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn with_compaction_interval(mut self, interval: usize) -> Self {
        self.compaction_interval = interval.max(1);
        self
    }

    #[cfg(test)]
    fn with_post_commit_hook(mut self, hook: PostCommitHook) -> Self {
        self.post_commit_hook = Some(hook);
        self
    }

    #[cfg(test)]
    fn with_journal_restart_hook(mut self, hook: JournalRestartHook) -> Self {
        self.delta_journal = self.delta_journal.with_restart_hook(hook);
        self
    }

    fn authority(&self) -> Result<Arc<ResourceAuthority>, ResourceError> {
        let mut guard = self.authority.lock().map_err(|_| ResourceError::Storage {
            reason: "resource governor authority lock poisoned".to_string(),
        })?;
        match &*guard {
            AuthorityLifecycle::Ready(authority) => return Ok(Arc::clone(authority)),
            AuthorityLifecycle::Recovering => {
                return Err(ResourceError::Storage {
                    reason: "resource governor authority recovery in progress".to_string(),
                });
            }
            AuthorityLifecycle::Vacant => {}
        }
        let started = Instant::now();
        let loaded = Arc::new(self.load_authority()?);
        *guard = AuthorityLifecycle::Ready(Arc::clone(&loaded));
        tracing::debug!(
            elapsed_ms = started.elapsed().as_millis(),
            "resource governor durable authority loaded"
        );
        Ok(loaded)
    }

    fn load_authority(&self) -> Result<ResourceAuthority, ResourceError> {
        let snapshot = self
            .snapshot_store
            .inspect(|snapshot| Ok(snapshot.clone()))?;
        let filesystem = Arc::clone(&self.filesystem);
        let from = SeqNo::from_backend(snapshot.journal_seq);
        let (state, latest_seq) = run_on_worker_pool(
            &self.workers,
            "resource-governor-filesystem",
            1,
            move || replay_journal(filesystem, snapshot.state, from),
        )?;
        Ok(ResourceAuthority::from_state(state, latest_seq))
    }

    fn enqueue_delta(
        &self,
        authority: &Arc<ResourceAuthority>,
        delta: ResourceGovernorDelta,
    ) -> Result<PendingResourceDelta, ResourceError> {
        let guard = self.authority.lock().map_err(|_| ResourceError::Storage {
            reason: "resource governor authority lock poisoned while enqueueing delta".to_string(),
        })?;
        let is_current = matches!(
            &*guard,
            AuthorityLifecycle::Ready(current) if Arc::ptr_eq(current, authority)
        );
        if !is_current {
            return Err(ResourceError::Storage {
                reason: "resource governor authority changed before delta enqueue".to_string(),
            });
        }
        authority.check_available()?;
        self.delta_journal.enqueue(delta)
    }

    fn finish_delta(
        &self,
        authority: &Arc<ResourceAuthority>,
        pending: PendingResourceDelta,
    ) -> Result<SeqNo, ResourceError> {
        let seq = pending.wait()?;
        authority.set_latest_seq(seq)?;
        self.maybe_compact();
        #[cfg(test)]
        if let Some(hook) = self.post_commit_hook.as_ref() {
            hook(authority);
        }
        Ok(seq)
    }

    fn maybe_compact(&self) {
        let interval = self.compaction_interval.max(1);
        let prior = self.deltas_since_compaction.fetch_add(1, Ordering::Relaxed);
        if !(prior + 1).is_multiple_of(interval) {
            return;
        }
        if self.compaction_in_flight.swap(true, Ordering::AcqRel) {
            return;
        }
        let snapshot_store = self.snapshot_store.clone();
        let filesystem = Arc::clone(&self.filesystem);
        let in_flight = Arc::clone(&self.compaction_in_flight);
        let spawn = std::thread::Builder::new()
            .name("resource-governor-compactor".to_string())
            .spawn(move || {
                struct CompactionGuard(Arc<AtomicBool>);

                impl Drop for CompactionGuard {
                    fn drop(&mut self) {
                        self.0.store(false, Ordering::Release);
                    }
                }

                let _guard = CompactionGuard(in_flight);
                let compacted = compact_resource_governor_snapshot(snapshot_store, filesystem);
                if let Err(error) = compacted {
                    warn!(reason = %error, "resource governor compaction write failed");
                }
            });
        if let Err(error) = spawn {
            warn!(reason = %error, "resource governor compaction thread failed to start");
            self.compaction_in_flight.store(false, Ordering::Release);
        }
    }

    fn invalidate_authority<T>(
        &self,
        authority: &Arc<ResourceAuthority>,
        error: ResourceError,
    ) -> Result<T, ResourceError> {
        authority.poison(error.clone());
        let mut guard = match self.authority.lock() {
            Ok(guard) => guard,
            Err(_) => {
                warn!(
                    error_kind = "authority_lock",
                    "resource governor authority lock failed while starting recovery"
                );
                return Err(error);
            }
        };
        let restart_journal = matches!(
            &*guard,
            AuthorityLifecycle::Ready(current) if Arc::ptr_eq(current, authority)
        );
        if restart_journal {
            let error_kind = match &error {
                ResourceError::Storage { .. } => "storage",
                _ => "authority_invariant",
            };
            warn!(
                error_kind,
                "resource governor authority invalidated; coordinating journal replacement"
            );
            *guard = AuthorityLifecycle::Recovering;
        }
        drop(guard);
        if !restart_journal {
            return Err(error);
        }

        let started = Instant::now();
        let restart = self.delta_journal.restart();
        let mut guard = match self.authority.lock() {
            Ok(guard) => guard,
            Err(_) => {
                warn!(
                    error_kind = "authority_lock",
                    "resource governor authority lock failed while completing recovery"
                );
                return Err(error);
            }
        };
        match restart {
            Ok(()) => {
                // `Recovering` prevented both reload and enqueue while the
                // authority lock was released for thread creation.
                *guard = AuthorityLifecycle::Vacant;
                warn!(
                    recovery_elapsed_ms = started.elapsed().as_millis(),
                    "resource governor journal replacement installed; the next operation will reload durable state"
                );
            }
            Err(_) => {
                // Preserve the primary request error and keep the poisoned
                // authority installed. A secondary recovery failure must fail
                // later work closed. Do not log the raw secondary error.
                *guard = AuthorityLifecycle::Ready(Arc::clone(authority));
                warn!(
                    error_kind = "journal_restart",
                    recovery_elapsed_ms = started.elapsed().as_millis(),
                    "resource governor delta journal restart failed after authority invalidation"
                );
            }
        }
        drop(guard);
        Err(error)
    }

    pub fn reserved_for(&self, account: &ResourceAccount) -> Result<ResourceTally, ResourceError> {
        let authority = self.authority()?;
        authority.check_available()?;
        let now = self.clock.now();
        let (tally, pending) = {
            let _commit = authority.lock_commit_for_accounts(std::slice::from_ref(account))?;
            let mut locked = authority.lock_accounts(std::slice::from_ref(account))?;
            let before = locked.account_parts(account);
            let mut state =
                locked.state_for_accounts(std::slice::from_ref(account), HashMap::new());
            advance_period_if_rolled_over(&mut state, account, now);
            let tally = state
                .reserved_by_account
                .get(account)
                .cloned()
                .unwrap_or_default();
            locked.write_accounts_from_state(std::slice::from_ref(account), &state);
            let after = locked.account_parts(account);
            let pending = if before != after {
                let delta = ResourceGovernorDelta::AccountSnapshot {
                    account: account.clone(),
                    at: now,
                };
                match self.enqueue_delta(&authority, delta) {
                    Ok(pending) => Some(pending),
                    Err(error) => return self.invalidate_authority(&authority, error),
                }
            } else {
                None
            };
            (tally, pending)
        };
        if let Some(pending) = pending
            && let Err(error) = self.finish_delta(&authority, pending)
        {
            return self.invalidate_authority(&authority, error);
        }
        Ok(tally)
    }

    pub fn usage_for(&self, account: &ResourceAccount) -> Result<ResourceTally, ResourceError> {
        let authority = self.authority()?;
        authority.check_available()?;
        let now = self.clock.now();
        let (tally, pending) = {
            let _commit = authority.lock_commit_for_accounts(std::slice::from_ref(account))?;
            let mut locked = authority.lock_accounts(std::slice::from_ref(account))?;
            let before = locked.account_parts(account);
            let mut state =
                locked.state_for_accounts(std::slice::from_ref(account), HashMap::new());
            advance_period_if_rolled_over(&mut state, account, now);
            let tally = state
                .usage_by_account
                .get(account)
                .cloned()
                .unwrap_or_default();
            locked.write_accounts_from_state(std::slice::from_ref(account), &state);
            let after = locked.account_parts(account);
            let pending = if before != after {
                let delta = ResourceGovernorDelta::AccountSnapshot {
                    account: account.clone(),
                    at: now,
                };
                match self.enqueue_delta(&authority, delta) {
                    Ok(pending) => Some(pending),
                    Err(error) => return self.invalidate_authority(&authority, error),
                }
            } else {
                None
            };
            (tally, pending)
        };
        if let Some(pending) = pending
            && let Err(error) = self.finish_delta(&authority, pending)
        {
            return self.invalidate_authority(&authority, error);
        }
        Ok(tally)
    }
}

impl<F> ResourceGovernor for FilesystemResourceGovernor<F>
where
    F: RootFilesystem + 'static,
{
    fn set_limit(
        &self,
        account: ResourceAccount,
        limits: ResourceLimits,
    ) -> Result<(), ResourceError> {
        let authority = self.authority()?;
        authority.check_available()?;
        let now = self.clock.now();
        let pending = {
            let _commit = authority.lock_commit_for_accounts(std::slice::from_ref(&account))?;
            let mut locked = authority.lock_accounts(std::slice::from_ref(&account))?;
            let mut state =
                locked.state_for_accounts(std::slice::from_ref(&account), HashMap::new());
            set_limit_in_state(&mut state, account.clone(), limits.clone(), now);
            locked.write_accounts_from_state(std::slice::from_ref(&account), &state);
            let delta = ResourceGovernorDelta::SetLimit {
                account: account.clone(),
                limits,
                at: now,
            };
            match self.enqueue_delta(&authority, delta) {
                Ok(pending) => pending,
                Err(error) => return self.invalidate_authority(&authority, error),
            }
        };
        if let Err(error) = self.finish_delta(&authority, pending) {
            return self.invalidate_authority(&authority, error);
        }
        self.event_sink
            .emit(BudgetEvent::LimitChanged { account, at: now });
        Ok(())
    }

    fn reserve_with_outcome(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ReservationOutcome, ResourceError> {
        self.reserve_with_id_and_outcome(scope, estimate, ResourceReservationId::new())
    }

    fn reserve_with_id_and_outcome(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ReservationOutcome, ResourceError> {
        let authority = self.authority()?;
        authority.check_available()?;
        let now = self.clock.now();
        let accounts = ResourceAccount::cascade(&scope);
        let (outcome, pending) = {
            let _commit = authority.lock_commit_for_accounts(&accounts)?;
            let mut reservations = authority.lock_reservations()?;
            let mut locked = authority.lock_accounts(&accounts)?;
            let mut reservation_subset = HashMap::new();
            if let Some(existing) = reservations.get(&reservation_id) {
                reservation_subset.insert(reservation_id, existing.clone());
            }
            let mut state = locked.state_for_accounts(&accounts, reservation_subset);
            let result = reserve_with_outcome_in_state(
                &mut state,
                scope.clone(),
                estimate.clone(),
                reservation_id,
                now,
            );
            if result.is_ok() {
                locked.write_accounts_from_state(&accounts, &state);
                let record = state
                    .reservations
                    .get(&reservation_id)
                    .cloned()
                    .ok_or_else(|| storage_error("reserve did not produce reservation record"));
                match record {
                    Ok(record) => {
                        reservations.insert(reservation_id, record);
                    }
                    Err(error) => return self.invalidate_authority(&authority, error),
                }
            }
            match result {
                Ok(outcome) => {
                    let delta = ResourceGovernorDelta::Reserve {
                        scope,
                        estimate,
                        reservation_id,
                        at: now,
                    };
                    let pending = match self.enqueue_delta(&authority, delta) {
                        Ok(pending) => pending,
                        Err(error) => return self.invalidate_authority(&authority, error),
                    };
                    (outcome, pending)
                }
                Err(error) => {
                    let result = Err(error);
                    emit_reserve_events(self.event_sink.as_ref(), &result, now);
                    return result;
                }
            }
        };
        if let Err(error) = self.finish_delta(&authority, pending) {
            return self.invalidate_authority(&authority, error);
        }
        let result = Ok(outcome);
        emit_reserve_events(self.event_sink.as_ref(), &result, now);
        result
    }

    fn reconcile(
        &self,
        reservation_id: ResourceReservationId,
        actual: ResourceUsage,
    ) -> Result<ResourceReceipt, ResourceError> {
        let authority = self.authority()?;
        authority.check_available()?;
        let accounts = {
            let reservations = authority.lock_reservations()?;
            let Some(record) = reservations.get(&reservation_id) else {
                return Err(ResourceError::UnknownReservation { id: reservation_id });
            };
            record.accounts.clone()
        };
        let now = self.clock.now();
        let (receipt, pending) = {
            let _commit = authority.lock_commit_for_accounts(&accounts)?;
            let mut reservations = authority.lock_reservations()?;
            let Some(record) = reservations.get(&reservation_id).cloned() else {
                return Err(ResourceError::UnknownReservation { id: reservation_id });
            };
            let mut locked = authority.lock_accounts(&accounts)?;
            let mut reservation_subset = HashMap::new();
            reservation_subset.insert(reservation_id, record);
            let mut state = locked.state_for_accounts(&accounts, reservation_subset);
            let result = reconcile_in_state(&mut state, reservation_id, actual.clone(), now);
            if result.is_ok() {
                locked.write_accounts_from_state(&accounts, &state);
                let record = state
                    .reservations
                    .get(&reservation_id)
                    .cloned()
                    .ok_or_else(|| storage_error("reconcile removed reservation record"));
                match record {
                    Ok(record) => {
                        reservations.insert(reservation_id, record);
                    }
                    Err(error) => return self.invalidate_authority(&authority, error),
                }
            }
            let receipt = result?;
            let delta = ResourceGovernorDelta::Reconcile {
                reservation_id,
                actual,
                at: now,
            };
            let pending = match self.enqueue_delta(&authority, delta) {
                Ok(pending) => pending,
                Err(error) => return self.invalidate_authority(&authority, error),
            };
            (receipt, pending)
        };
        if let Err(error) = self.finish_delta(&authority, pending) {
            return self.invalidate_authority(&authority, error);
        }
        self.event_sink.emit(BudgetEvent::Reconciled {
            account: most_specific_account(&receipt.scope),
            receipt: receipt.clone(),
            at: now,
        });
        Ok(receipt)
    }

    fn validate_reservation(&self, reservation: &ResourceReservation) -> Result<(), ResourceError> {
        let authority = self.authority()?;
        authority.check_available()?;
        let accounts = {
            let reservations = authority.lock_reservations()?;
            let Some(record) = reservations.get(&reservation.id) else {
                return Err(ResourceError::UnknownReservation { id: reservation.id });
            };
            record.accounts.clone()
        };
        let _commit = authority.lock_commit_for_accounts(&accounts)?;
        let reservations = authority.lock_reservations()?;
        let Some(record) = reservations.get(&reservation.id).cloned() else {
            return Err(ResourceError::UnknownReservation { id: reservation.id });
        };
        let mut locked = authority.lock_accounts(&accounts)?;
        let mut reservation_subset = HashMap::new();
        reservation_subset.insert(reservation.id, record);
        let mut state = locked.state_for_accounts(&accounts, reservation_subset);
        validate_reservation_in_state(&mut state, reservation)
    }

    fn release(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReceipt, ResourceError> {
        let authority = self.authority()?;
        authority.check_available()?;
        let accounts = {
            let reservations = authority.lock_reservations()?;
            let Some(record) = reservations.get(&reservation_id) else {
                return Err(ResourceError::UnknownReservation { id: reservation_id });
            };
            record.accounts.clone()
        };
        let now = self.clock.now();
        let (receipt, pending) = {
            let _commit = authority.lock_commit_for_accounts(&accounts)?;
            let mut reservations = authority.lock_reservations()?;
            let Some(record) = reservations.get(&reservation_id).cloned() else {
                return Err(ResourceError::UnknownReservation { id: reservation_id });
            };
            let mut locked = authority.lock_accounts(&accounts)?;
            let mut reservation_subset = HashMap::new();
            reservation_subset.insert(reservation_id, record);
            let mut state = locked.state_for_accounts(&accounts, reservation_subset);
            let result = release_in_state(&mut state, reservation_id, now);
            if result.is_ok() {
                locked.write_accounts_from_state(&accounts, &state);
                let record = state
                    .reservations
                    .get(&reservation_id)
                    .cloned()
                    .ok_or_else(|| storage_error("release removed reservation record"));
                match record {
                    Ok(record) => {
                        reservations.insert(reservation_id, record);
                    }
                    Err(error) => return self.invalidate_authority(&authority, error),
                }
            }
            let receipt = result?;
            let delta = ResourceGovernorDelta::Release {
                reservation_id,
                at: now,
            };
            let pending = match self.enqueue_delta(&authority, delta) {
                Ok(pending) => pending,
                Err(error) => return self.invalidate_authority(&authority, error),
            };
            (receipt, pending)
        };
        if let Err(error) = self.finish_delta(&authority, pending) {
            return self.invalidate_authority(&authority, error);
        }
        self.event_sink.emit(BudgetEvent::Released {
            account: most_specific_account(&receipt.scope),
            receipt: receipt.clone(),
            at: now,
        });
        Ok(receipt)
    }

    fn account_snapshot(
        &self,
        account: &ResourceAccount,
    ) -> Result<Option<AccountSnapshot>, ResourceError> {
        let authority = self.authority()?;
        authority.check_available()?;
        let now = self.clock.now();
        let (snapshot, pending) = {
            let _commit = authority.lock_commit_for_accounts(std::slice::from_ref(account))?;
            let mut locked = authority.lock_accounts(std::slice::from_ref(account))?;
            let before = locked.account_parts(account);
            let mut state =
                locked.state_for_accounts(std::slice::from_ref(account), HashMap::new());
            let snapshot = account_snapshot_in_state(&mut state, account, now);
            locked.write_accounts_from_state(std::slice::from_ref(account), &state);
            let after = locked.account_parts(account);
            let pending = if before != after {
                let delta = ResourceGovernorDelta::AccountSnapshot {
                    account: account.clone(),
                    at: now,
                };
                match self.enqueue_delta(&authority, delta) {
                    Ok(pending) => Some(pending),
                    Err(error) => return self.invalidate_authority(&authority, error),
                }
            } else {
                None
            };
            (snapshot, pending)
        };
        if let Some(pending) = pending
            && let Err(error) = self.finish_delta(&authority, pending)
        {
            return self.invalidate_authority(&authority, error);
        }
        Ok(snapshot)
    }
}

fn fs_error(error: FilesystemError) -> ResourceError {
    storage_error(error)
}

fn storage_error(error: impl std::fmt::Display) -> ResourceError {
    ResourceError::Storage {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId,
        ResourceScope, TenantId, UserId, VirtualPath,
    };
    use rust_decimal_macros::dec;

    use super::*;
    use crate::{BudgetPeriod, FakeClock, ResourceGovernorStore, ResourceLimits};

    fn scoped_resources_fs() -> Arc<ScopedFilesystem<InMemoryBackend>> {
        let backend = Arc::new(InMemoryBackend::new());
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/resources").expect("alias"),
            VirtualPath::new("/tenants/tenant1/users/user1/resources").expect("target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view");
        Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
    }

    fn sample_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant1").unwrap(),
            user_id: UserId::new("user1").unwrap(),
            agent_id: None,
            project_id: Some(ProjectId::new("project1").unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    #[test]
    fn stale_authority_cannot_enqueue_into_restarted_journal_generation() {
        let governor = FilesystemResourceGovernor::new(scoped_resources_fs());
        let stale = governor.authority().expect("authority");
        *governor.authority.lock().expect("authority lock") = AuthorityLifecycle::Vacant;
        let account = ResourceAccount::tenant(sample_scope().tenant_id);

        let result = governor.enqueue_delta(
            &stale,
            ResourceGovernorDelta::AccountSnapshot {
                account,
                at: chrono::Utc::now(),
            },
        );

        assert!(
            matches!(result, Err(ResourceError::Storage { reason }) if reason.contains("authority changed")),
            "a stale operation must fail closed instead of writing into the replacement journal"
        );
    }

    #[test]
    fn recovering_lifecycle_rejects_reload_and_stale_enqueue() {
        let governor = FilesystemResourceGovernor::new(scoped_resources_fs());
        let stale = governor.authority().expect("authority");
        *governor.authority.lock().expect("authority lock") = AuthorityLifecycle::Recovering;
        let account = ResourceAccount::tenant(sample_scope().tenant_id);

        let reload = governor.authority();
        let enqueue = governor.enqueue_delta(
            &stale,
            ResourceGovernorDelta::AccountSnapshot {
                account,
                at: chrono::Utc::now(),
            },
        );

        assert!(
            matches!(reload, Err(ResourceError::Storage { reason }) if reason.contains("recovery in progress")),
            "new work must not publish an authority before journal replacement is installed"
        );
        assert!(
            matches!(enqueue, Err(ResourceError::Storage { reason }) if reason.contains("authority changed")),
            "stale work must not enqueue while the lifecycle is recovering"
        );
    }

    #[test]
    fn live_recovery_rejects_reload_and_stale_enqueue_until_restart_finishes() {
        let restart_entered = Arc::new(std::sync::Barrier::new(2));
        let restart_release = Arc::new(std::sync::Barrier::new(2));
        let hook_entered = Arc::clone(&restart_entered);
        let hook_release = Arc::clone(&restart_release);
        let governor = Arc::new(
            FilesystemResourceGovernor::new(scoped_resources_fs()).with_journal_restart_hook(
                Arc::new(move || {
                    hook_entered.wait();
                    hook_release.wait();
                }),
            ),
        );
        let stale = governor.authority().expect("authority");
        let recovering_governor = Arc::clone(&governor);
        let recovering_authority = Arc::clone(&stale);
        let primary_reason = "primary durable journal write failed";
        let recovery = std::thread::spawn(move || {
            recovering_governor.invalidate_authority::<()>(
                &recovering_authority,
                ResourceError::Storage {
                    reason: primary_reason.to_string(),
                },
            )
        });

        restart_entered.wait();
        let reload = governor.authority();
        let enqueue = governor.enqueue_delta(
            &stale,
            ResourceGovernorDelta::AccountSnapshot {
                account: ResourceAccount::tenant(sample_scope().tenant_id),
                at: chrono::Utc::now(),
            },
        );
        restart_release.wait();
        let recovery_result = recovery.join().expect("recovery thread");

        assert!(
            matches!(reload, Err(ResourceError::Storage { reason }) if reason.contains("recovery in progress")),
            "new work must not reload authority while journal replacement is in progress"
        );
        assert!(
            matches!(enqueue, Err(ResourceError::Storage { reason }) if reason.contains("authority changed")),
            "stale work must fail closed while journal replacement is in progress"
        );
        assert!(
            matches!(recovery_result, Err(ResourceError::Storage { reason }) if reason == primary_reason),
            "recovery must preserve the request's primary storage error"
        );
        let replacement = governor.authority().expect("replacement authority");
        assert!(
            !Arc::ptr_eq(&replacement, &stale),
            "successful restart must reload a fresh authority generation"
        );
    }

    #[test]
    fn journal_restart_failure_does_not_mask_primary_storage_error() {
        let governor = FilesystemResourceGovernor::new(scoped_resources_fs());
        let authority = governor.authority().expect("authority");
        governor.delta_journal.poison_sender_lock();
        let primary_reason = "primary durable journal write failed";

        let result: Result<(), ResourceError> = governor.invalidate_authority(
            &authority,
            ResourceError::Storage {
                reason: primary_reason.to_string(),
            },
        );

        assert!(
            matches!(result, Err(ResourceError::Storage { reason }) if reason == primary_reason),
            "journal restart failure must not replace the request's primary storage error"
        );
        let installed = governor.authority().expect("installed authority");
        assert!(
            Arc::ptr_eq(&installed, &authority),
            "a failed journal restart must leave the poisoned generation installed"
        );
        assert!(
            installed.check_available().is_err(),
            "new work must fail closed while no replacement journal is available"
        );
    }

    #[test]
    fn authority_lock_failure_does_not_mask_primary_storage_error() {
        let governor = FilesystemResourceGovernor::new(scoped_resources_fs());
        let authority = governor.authority().expect("authority");
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = governor.authority.lock().expect("authority lock");
            panic!("poison authority lock for recovery coverage");
        }));
        let primary_reason = "primary durable journal write failed";

        let result: Result<(), ResourceError> = governor.invalidate_authority(
            &authority,
            ResourceError::Storage {
                reason: primary_reason.to_string(),
            },
        );

        assert!(
            matches!(result, Err(ResourceError::Storage { reason }) if reason == primary_reason),
            "authority lock failure must not replace the request's primary storage error"
        );
    }

    #[test]
    fn durably_acked_reservation_is_not_retroactively_failed_by_generation_poison() {
        let governor = FilesystemResourceGovernor::new(scoped_resources_fs())
            .with_post_commit_hook(Arc::new(|authority| {
                authority.poison(ResourceError::Storage {
                    reason: "later journal generation failure".to_string(),
                });
            }));

        let result = governor.reserve(
            sample_scope(),
            ResourceEstimate {
                usd: Some(dec!(0.25)),
                ..ResourceEstimate::default()
            },
        );

        assert!(
            result.is_ok(),
            "the reservation's own durable ack is authoritative even if later work poisons the generation"
        );
    }

    #[test]
    fn compaction_snapshot_cursor_does_not_double_apply_journal_on_restart() {
        let scoped = scoped_resources_fs();
        let scope = sample_scope();
        let account = ResourceAccount::tenant(scope.tenant_id.clone());
        let governor =
            FilesystemResourceGovernor::new(Arc::clone(&scoped)).with_compaction_interval(3);

        governor
            .set_limit(
                account.clone(),
                ResourceLimits {
                    max_usd: Some(dec!(1.00)),
                    ..ResourceLimits::default()
                },
            )
            .unwrap();
        let reservation = governor
            .reserve(
                scope,
                ResourceEstimate {
                    usd: Some(dec!(0.25)),
                    ..ResourceEstimate::default()
                },
            )
            .unwrap();
        governor
            .reconcile(
                reservation.id,
                ResourceUsage {
                    usd: dec!(0.25),
                    ..ResourceUsage::default()
                },
            )
            .unwrap();

        let store = FilesystemResourceGovernorStore::new(Arc::clone(&scoped));
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let snapshot = store.inspect(|snapshot| Ok(snapshot.clone())).unwrap();
            if snapshot.journal_seq >= 3 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "compaction did not advance journal cursor; snapshot={snapshot:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        let reloaded = FilesystemResourceGovernor::new(scoped);
        reloaded.warm_authority().unwrap();
        let snapshot = reloaded.account_snapshot(&account).unwrap().unwrap();
        assert_eq!(snapshot.ledger.spent.usd, dec!(0.25));
        assert_eq!(snapshot.ledger.reserved.usd, dec!(0));
    }

    #[test]
    fn compaction_replay_preserves_rolled_over_usage_window() {
        let scoped = scoped_resources_fs();
        let start = chrono::DateTime::parse_from_rfc3339("2026-05-21T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let clock = FakeClock::new(start);
        let scope = sample_scope();
        let account = ResourceAccount::tenant(scope.tenant_id.clone());
        let governor = FilesystemResourceGovernor::new(Arc::clone(&scoped))
            .with_clock(Arc::new(clock.clone()))
            .with_compaction_interval(4);

        governor
            .set_limit(
                account.clone(),
                ResourceLimits {
                    max_usd: Some(dec!(5.00)),
                    period: BudgetPeriod::Rolling24h,
                    ..ResourceLimits::default()
                },
            )
            .unwrap();
        let reservation = governor
            .reserve(
                scope.clone(),
                ResourceEstimate {
                    usd: Some(dec!(4.50)),
                    ..ResourceEstimate::default()
                },
            )
            .unwrap();
        governor
            .reconcile(
                reservation.id,
                ResourceUsage {
                    usd: dec!(4.50),
                    ..ResourceUsage::default()
                },
            )
            .unwrap();

        clock.advance(chrono::Duration::hours(24) + chrono::Duration::minutes(1));
        let rolled = governor.account_snapshot(&account).unwrap().unwrap();
        assert_eq!(rolled.ledger.spent.usd, dec!(0));

        let store = FilesystemResourceGovernorStore::new(Arc::clone(&scoped));
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let snapshot = store.inspect(|snapshot| Ok(snapshot.clone())).unwrap();
            if snapshot.journal_seq >= 4 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "compaction did not include rolled period state; snapshot={snapshot:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        let reloaded = FilesystemResourceGovernor::new(scoped).with_clock(Arc::new(clock));
        let snapshot = reloaded.account_snapshot(&account).unwrap().unwrap();
        assert_eq!(snapshot.ledger.spent.usd, dec!(0));

        reloaded
            .reserve(
                scope,
                ResourceEstimate {
                    usd: Some(dec!(4.50)),
                    ..ResourceEstimate::default()
                },
            )
            .expect("rolled-over spend must not be resurrected by compaction replay");
    }
}
