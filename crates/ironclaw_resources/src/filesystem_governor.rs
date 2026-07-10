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

use ironclaw_filesystem::{FilesystemError, RootFilesystem, ScopedFilesystem, SeqNo};
use ironclaw_host_api::{ResourceReservationId, ResourceScope};
use tracing::warn;

use crate::cas_snapshot::{AsyncStorageWorkerPoolCell, new_worker_pool_cell, run_on_worker_pool};
use crate::{
    AccountSnapshot, BudgetEvent, BudgetEventSink, Clock, FilesystemResourceGovernorStore,
    NoOpBudgetEventSink, ReservationOutcome, ResourceAccount, ResourceError, ResourceGovernor,
    ResourceGovernorStore, ResourceLimits, ResourceReceipt, ResourceTally, SystemClock,
    account_snapshot_in_state, advance_period_if_rolled_over, emit_reserve_events,
    most_specific_account, reconcile_in_state, release_in_state, reserve_with_outcome_in_state,
    set_limit_in_state,
};
use crate::{ResourceEstimate, ResourceUsage};

mod authority;
mod journal;

use authority::ResourceAuthority;
use journal::{
    PendingResourceDelta, ResourceDeltaJournal, ResourceGovernorDelta,
    compact_resource_governor_snapshot, replay_journal,
};

const DEFAULT_COMPACTION_INTERVAL: usize = 1024;

/// Filesystem-backed governor with process-local quota authority.
pub struct FilesystemResourceGovernor<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    snapshot_store: FilesystemResourceGovernorStore<F>,
    authority: Mutex<Option<Arc<ResourceAuthority>>>,
    delta_journal: ResourceDeltaJournal<F>,
    workers: AsyncStorageWorkerPoolCell,
    clock: Arc<dyn Clock>,
    event_sink: Arc<dyn BudgetEventSink>,
    compaction_interval: usize,
    deltas_since_compaction: AtomicUsize,
    compaction_in_flight: Arc<AtomicBool>,
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
            authority: Mutex::new(None),
            workers: new_worker_pool_cell(),
            clock: Arc::new(SystemClock),
            event_sink: Arc::new(NoOpBudgetEventSink),
            compaction_interval: DEFAULT_COMPACTION_INTERVAL,
            deltas_since_compaction: AtomicUsize::new(0),
            compaction_in_flight: Arc::new(AtomicBool::new(false)),
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

    fn authority(&self) -> Result<Arc<ResourceAuthority>, ResourceError> {
        let mut guard = self.authority.lock().map_err(|_| ResourceError::Storage {
            reason: "resource governor authority lock poisoned".to_string(),
        })?;
        if let Some(authority) = guard.as_ref() {
            return Ok(Arc::clone(authority));
        }
        let loaded = Arc::new(self.load_authority()?);
        *guard = Some(Arc::clone(&loaded));
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
        delta: ResourceGovernorDelta,
    ) -> Result<PendingResourceDelta, ResourceError> {
        self.delta_journal.enqueue(delta)
    }

    fn finish_delta(
        &self,
        authority: &ResourceAuthority,
        pending: PendingResourceDelta,
    ) -> Result<SeqNo, ResourceError> {
        let seq = pending.wait()?;
        authority.set_latest_seq(seq)?;
        self.maybe_compact();
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

    fn poison<T>(
        &self,
        authority: &ResourceAuthority,
        error: ResourceError,
    ) -> Result<T, ResourceError> {
        authority.poison(error.clone());
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
                match self.enqueue_delta(delta) {
                    Ok(pending) => Some(pending),
                    Err(error) => return self.poison(&authority, error),
                }
            } else {
                None
            };
            (tally, pending)
        };
        if let Some(pending) = pending
            && let Err(error) = self.finish_delta(&authority, pending)
        {
            return self.poison(&authority, error);
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
                match self.enqueue_delta(delta) {
                    Ok(pending) => Some(pending),
                    Err(error) => return self.poison(&authority, error),
                }
            } else {
                None
            };
            (tally, pending)
        };
        if let Some(pending) = pending
            && let Err(error) = self.finish_delta(&authority, pending)
        {
            return self.poison(&authority, error);
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
            match self.enqueue_delta(delta) {
                Ok(pending) => pending,
                Err(error) => return self.poison(&authority, error),
            }
        };
        if let Err(error) = self.finish_delta(&authority, pending) {
            return self.poison(&authority, error);
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
                    Err(error) => return self.poison(&authority, error),
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
                    let pending = match self.enqueue_delta(delta) {
                        Ok(pending) => pending,
                        Err(error) => return self.poison(&authority, error),
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
            return self.poison(&authority, error);
        }
        if let Err(error) = authority.check_available() {
            let result = Err(error);
            emit_reserve_events(self.event_sink.as_ref(), &result, now);
            return result;
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
                    Err(error) => return self.poison(&authority, error),
                }
            }
            let receipt = result?;
            let delta = ResourceGovernorDelta::Reconcile {
                reservation_id,
                actual,
                at: now,
            };
            let pending = match self.enqueue_delta(delta) {
                Ok(pending) => pending,
                Err(error) => return self.poison(&authority, error),
            };
            (receipt, pending)
        };
        if let Err(error) = self.finish_delta(&authority, pending) {
            return self.poison(&authority, error);
        }
        self.event_sink.emit(BudgetEvent::Reconciled {
            account: most_specific_account(&receipt.scope),
            receipt: receipt.clone(),
            at: now,
        });
        Ok(receipt)
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
                    Err(error) => return self.poison(&authority, error),
                }
            }
            let receipt = result?;
            let delta = ResourceGovernorDelta::Release {
                reservation_id,
                at: now,
            };
            let pending = match self.enqueue_delta(delta) {
                Ok(pending) => pending,
                Err(error) => return self.poison(&authority, error),
            };
            (receipt, pending)
        };
        if let Err(error) = self.finish_delta(&authority, pending) {
            return self.poison(&authority, error);
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
                match self.enqueue_delta(delta) {
                    Ok(pending) => Some(pending),
                    Err(error) => return self.poison(&authority, error),
                }
            } else {
                None
            };
            (snapshot, pending)
        };
        if let Some(pending) = pending
            && let Err(error) = self.finish_delta(&authority, pending)
        {
            return self.poison(&authority, error);
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
