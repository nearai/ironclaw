//! Journaled filesystem-backed resource governor.
//!
//! Resource quotas are process-global in the hosted runtime: one process owns
//! the authoritative reservation/tally state and persists an append-only delta
//! journal through the caller's [`RootFilesystem`]. That makes in-process
//! authority sound for quota decisions while avoiding per-reservation database
//! transactions. Durable recovery loads the compacted
//! [`ResourceGovernorSnapshot`] written through [`FilesystemResourceGovernorStore`]
//! and replays `/resources/deltas/log` from the snapshot cursor.
//!
//! [`FilesystemResourceGovernorStore`] remains the CAS snapshot mechanism for
//! compaction only. Hot reserve/reconcile/release paths update per-account
//! shards in memory, enqueue one delta, and ack the caller after the group
//! commit flusher durably appends that delta.

use std::collections::{BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};

use chrono::{DateTime, Utc};
use ironclaw_filesystem::{FilesystemError, RootFilesystem, ScopedFilesystem, SeqNo};
use ironclaw_host_api::{ReservationStatus, ResourceReservationId, ResourceScope, ScopedPath};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::cas_snapshot::{AsyncStorageWorkerPoolCell, new_worker_pool_cell, run_on_worker_pool};
use crate::{
    AccountSnapshot, BudgetEvent, BudgetEventSink, Clock, FilesystemResourceGovernorStore,
    NoOpBudgetEventSink, ReservationOutcome, ReservationRecord, ResourceAccount, ResourceError,
    ResourceGovernor, ResourceGovernorStore, ResourceLimits, ResourceReceipt, ResourceState,
    ResourceTally, SystemClock, account_snapshot_in_state, advance_period_if_rolled_over,
    emit_reserve_events, most_specific_account, reconcile_in_state, release_in_state,
    reserve_with_outcome_in_state, set_limit_in_state,
};
use crate::{ResourceEstimate, ResourceUsage};

const DELTA_LOG_PATH: &str = "/resources/deltas/log";
const DELTA_JOURNAL_MAX_BATCH: usize = 256;
const ACCOUNT_SHARDS: usize = 64;
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

    fn persist_delta(
        &self,
        authority: &ResourceAuthority,
        delta: ResourceGovernorDelta,
    ) -> Result<SeqNo, ResourceError> {
        let seq = self.delta_journal.persist(delta)?;
        authority.set_latest_seq(seq)?;
        self.maybe_compact();
        Ok(seq)
    }

    fn maybe_compact(&self) {
        let interval = self.compaction_interval.max(1);
        let prior = self.deltas_since_compaction.fetch_add(1, Ordering::Relaxed);
        if (prior + 1) % interval != 0 {
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
                let compacted = compact_resource_governor_snapshot(snapshot_store, filesystem);
                if let Err(error) = compacted {
                    warn!(reason = %error, "resource governor compaction write failed");
                }
                in_flight.store(false, Ordering::Release);
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
        let (tally, changed) = {
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
            (tally, before != after)
        };
        if changed {
            let delta = ResourceGovernorDelta::AccountSnapshot {
                account: account.clone(),
                at: now,
            };
            if let Err(error) = self.persist_delta(&authority, delta) {
                return self.poison(&authority, error);
            }
        }
        Ok(tally)
    }

    pub fn usage_for(&self, account: &ResourceAccount) -> Result<ResourceTally, ResourceError> {
        let authority = self.authority()?;
        authority.check_available()?;
        let now = self.clock.now();
        let (tally, changed) = {
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
            (tally, before != after)
        };
        if changed {
            let delta = ResourceGovernorDelta::AccountSnapshot {
                account: account.clone(),
                at: now,
            };
            if let Err(error) = self.persist_delta(&authority, delta) {
                return self.poison(&authority, error);
            }
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
        {
            let mut locked = authority.lock_accounts(std::slice::from_ref(&account))?;
            let mut state =
                locked.state_for_accounts(std::slice::from_ref(&account), HashMap::new());
            set_limit_in_state(&mut state, account.clone(), limits.clone(), now);
            locked.write_accounts_from_state(std::slice::from_ref(&account), &state);
        }
        let delta = ResourceGovernorDelta::SetLimit {
            account: account.clone(),
            limits,
            at: now,
        };
        if let Err(error) = self.persist_delta(&authority, delta) {
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
        let result = {
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
            result
        };

        match result {
            Ok(outcome) => {
                let delta = ResourceGovernorDelta::Reserve {
                    scope,
                    estimate,
                    reservation_id,
                    at: now,
                };
                if let Err(error) = self.persist_delta(&authority, delta) {
                    return self.poison(&authority, error);
                }
                let result = Ok(outcome);
                emit_reserve_events(self.event_sink.as_ref(), &result, now);
                result
            }
            Err(error) => {
                let result = Err(error);
                emit_reserve_events(self.event_sink.as_ref(), &result, now);
                result
            }
        }
    }

    fn reconcile(
        &self,
        reservation_id: ResourceReservationId,
        actual: ResourceUsage,
    ) -> Result<ResourceReceipt, ResourceError> {
        let authority = self.authority()?;
        authority.check_available()?;
        let now = self.clock.now();
        let result = {
            let mut reservations = authority.lock_reservations()?;
            let Some(record) = reservations.get(&reservation_id).cloned() else {
                return Err(ResourceError::UnknownReservation { id: reservation_id });
            };
            let accounts = record.accounts.clone();
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
            result
        };

        let receipt = match result {
            Ok(receipt) => receipt,
            Err(error) => return Err(error),
        };
        let delta = ResourceGovernorDelta::Reconcile {
            reservation_id,
            actual,
            at: now,
        };
        if let Err(error) = self.persist_delta(&authority, delta) {
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
        let now = self.clock.now();
        let result = {
            let mut reservations = authority.lock_reservations()?;
            let Some(record) = reservations.get(&reservation_id).cloned() else {
                return Err(ResourceError::UnknownReservation { id: reservation_id });
            };
            let accounts = record.accounts.clone();
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
            result
        };

        let receipt = match result {
            Ok(receipt) => receipt,
            Err(error) => return Err(error),
        };
        let delta = ResourceGovernorDelta::Release {
            reservation_id,
            at: now,
        };
        if let Err(error) = self.persist_delta(&authority, delta) {
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
        let (snapshot, changed) = {
            let mut locked = authority.lock_accounts(std::slice::from_ref(account))?;
            let before = locked.account_parts(account);
            let mut state =
                locked.state_for_accounts(std::slice::from_ref(account), HashMap::new());
            let snapshot = account_snapshot_in_state(&mut state, account, now);
            locked.write_accounts_from_state(std::slice::from_ref(account), &state);
            let after = locked.account_parts(account);
            (snapshot, before != after)
        };
        if changed {
            let delta = ResourceGovernorDelta::AccountSnapshot {
                account: account.clone(),
                at: now,
            };
            if let Err(error) = self.persist_delta(&authority, delta) {
                return self.poison(&authority, error);
            }
        }
        Ok(snapshot)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ResourceGovernorDelta {
    SetLimit {
        account: ResourceAccount,
        limits: ResourceLimits,
        at: DateTime<Utc>,
    },
    Reserve {
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
        at: DateTime<Utc>,
    },
    Reconcile {
        reservation_id: ResourceReservationId,
        actual: ResourceUsage,
        at: DateTime<Utc>,
    },
    Release {
        reservation_id: ResourceReservationId,
        at: DateTime<Utc>,
    },
    AccountSnapshot {
        account: ResourceAccount,
        at: DateTime<Utc>,
    },
}

impl ResourceGovernorDelta {
    fn apply_to(self, state: &mut ResourceState) -> Result<(), ResourceError> {
        match self {
            Self::SetLimit {
                account,
                limits,
                at,
            } => {
                set_limit_in_state(state, account, limits, at);
                Ok(())
            }
            Self::Reserve {
                scope,
                estimate,
                reservation_id,
                at,
            } => reserve_with_outcome_in_state(state, scope, estimate, reservation_id, at)
                .map(|_| ()),
            Self::Reconcile {
                reservation_id,
                actual,
                at,
            } => reconcile_in_state(state, reservation_id, actual, at).map(|_| ()),
            Self::Release { reservation_id, at } => {
                release_in_state(state, reservation_id, at).map(|_| ())
            }
            Self::AccountSnapshot { account, at } => {
                let _ = account_snapshot_in_state(state, &account, at);
                Ok(())
            }
        }
    }
}

struct ResourceDeltaJournal<F>
where
    F: RootFilesystem,
{
    sender: mpsc::Sender<DeltaJournalRequest>,
    _filesystem: std::marker::PhantomData<F>,
}

struct DeltaJournalRequest {
    delta: ResourceGovernorDelta,
    ack: mpsc::Sender<Result<SeqNo, ResourceError>>,
}

impl<F> ResourceDeltaJournal<F>
where
    F: RootFilesystem + 'static,
{
    fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        let (sender, receiver) = mpsc::channel();
        if let Err(error) = std::thread::Builder::new()
            .name("resource-governor-delta-journal".to_string())
            .spawn(move || run_delta_journal_flusher(filesystem, receiver))
        {
            warn!(reason = %error, "resource governor delta journal thread failed to start");
        }
        Self {
            sender,
            _filesystem: std::marker::PhantomData,
        }
    }

    fn persist(&self, delta: ResourceGovernorDelta) -> Result<SeqNo, ResourceError> {
        let (ack, receiver) = mpsc::channel();
        self.sender
            .send(DeltaJournalRequest { delta, ack })
            .map_err(|_| storage_error("resource governor delta journal stopped"))?;
        receiver
            .recv()
            .map_err(|_| storage_error("resource governor delta journal stopped"))?
    }
}

fn run_delta_journal_flusher<F>(
    filesystem: Arc<ScopedFilesystem<F>>,
    receiver: mpsc::Receiver<DeltaJournalRequest>,
) where
    F: RootFilesystem + 'static,
{
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            while let Ok(request) = receiver.recv() {
                let _ = request.ack.send(Err(storage_error(format!(
                    "resource governor delta journal runtime failed: {error}"
                ))));
            }
            return;
        }
    };
    while let Ok(first) = receiver.recv() {
        let mut requests = Vec::with_capacity(DELTA_JOURNAL_MAX_BATCH);
        requests.push(first);
        std::thread::yield_now();
        while requests.len() < DELTA_JOURNAL_MAX_BATCH {
            match receiver.try_recv() {
                Ok(request) => requests.push(request),
                Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
            }
        }
        let result = runtime.block_on(persist_delta_journal_batch(filesystem.as_ref(), &requests));
        match result {
            Ok(seqs) => {
                for (request, seq) in requests.into_iter().zip(seqs) {
                    let _ = request.ack.send(Ok(seq));
                }
            }
            Err(error) => {
                for request in requests {
                    let _ = request.ack.send(Err(error.clone()));
                }
            }
        }
    }
}

async fn persist_delta_journal_batch<F>(
    filesystem: &ScopedFilesystem<F>,
    requests: &[DeltaJournalRequest],
) -> Result<Vec<SeqNo>, ResourceError>
where
    F: RootFilesystem,
{
    let path = delta_log_path()?;
    let payloads = requests
        .iter()
        .map(|request| serde_json::to_vec(&request.delta).map_err(storage_error))
        .collect::<Result<Vec<_>, _>>()?;
    if let [payload] = payloads.as_slice() {
        return filesystem
            .append(&ResourceScope::system(), &path, payload.clone())
            .await
            .map(|seq| vec![seq])
            .map_err(fs_error);
    }
    let seqs = filesystem
        .append_batch(&ResourceScope::system(), &path, payloads)
        .await
        .map_err(fs_error)?;
    if seqs.len() != requests.len() {
        return Err(storage_error(
            "resource governor delta batch append returned an unexpected ack count",
        ));
    }
    Ok(seqs)
}

fn compact_resource_governor_snapshot<F>(
    snapshot_store: FilesystemResourceGovernorStore<F>,
    filesystem: Arc<ScopedFilesystem<F>>,
) -> Result<(), ResourceError>
where
    F: RootFilesystem + 'static,
{
    let snapshot = snapshot_store.inspect(|snapshot| Ok(snapshot.clone()))?;
    let from = SeqNo::from_backend(snapshot.journal_seq);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(storage_error)?;
    let (state, latest_seq) = runtime.block_on(replay_journal(filesystem, snapshot.state, from))?;
    snapshot_store.update(move |snapshot| {
        if snapshot.journal_seq > latest_seq.get() {
            return Ok(());
        }
        snapshot.schema_version = crate::RESOURCE_GOVERNOR_SNAPSHOT_SCHEMA_VERSION;
        snapshot.state = state.clone();
        snapshot.journal_seq = latest_seq.get();
        Ok(())
    })
}

async fn replay_journal<F>(
    filesystem: Arc<ScopedFilesystem<F>>,
    mut state: ResourceState,
    from: SeqNo,
) -> Result<(ResourceState, SeqNo), ResourceError>
where
    F: RootFilesystem,
{
    rebuild_tallies_from_reservations(&mut state);
    let path = delta_log_path()?;
    let records = match filesystem.tail(&ResourceScope::system(), &path, from).await {
        Ok(records) => records,
        Err(FilesystemError::NotFound { .. }) | Err(FilesystemError::Unsupported { .. }) => {
            Vec::new()
        }
        Err(error) => return Err(fs_error(error)),
    };
    let mut latest = from;
    for record in records {
        latest = record.seq;
        let delta: ResourceGovernorDelta = serde_json::from_slice(&record.payload)
            .map_err(|error| storage_error(format!("decode resource governor delta: {error}")))?;
        delta.apply_to(&mut state)?;
    }
    Ok((state, latest))
}

fn rebuild_tallies_from_reservations(state: &mut ResourceState) {
    state.reserved_by_account.clear();
    state.usage_by_account.clear();
    for record in state.reservations.values() {
        match record.status {
            ReservationStatus::Active => {
                for account in &record.accounts {
                    state
                        .reserved_by_account
                        .entry(account.clone())
                        .or_default()
                        .add_assign(&record.tally);
                }
            }
            ReservationStatus::Reconciled => {
                let Some(actual) = &record.actual else {
                    continue;
                };
                let spent = ResourceTally::from_usage(actual);
                for account in &record.accounts {
                    state
                        .usage_by_account
                        .entry(account.clone())
                        .or_default()
                        .add_assign(&spent);
                }
            }
            ReservationStatus::Released => {}
        }
    }
}

struct ResourceAuthority {
    shards: Vec<Mutex<AccountShard>>,
    reservations: Mutex<HashMap<ResourceReservationId, ReservationRecord>>,
    latest_seq: Mutex<SeqNo>,
    poisoned: Mutex<Option<String>>,
}

#[derive(Default)]
struct AccountShard {
    limits: HashMap<ResourceAccount, ResourceLimits>,
    reserved_by_account: HashMap<ResourceAccount, ResourceTally>,
    usage_by_account: HashMap<ResourceAccount, ResourceTally>,
    period_anchors: HashMap<ResourceAccount, DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
struct AccountParts {
    limits: Option<ResourceLimits>,
    reserved: Option<ResourceTally>,
    usage: Option<ResourceTally>,
    period_anchor: Option<DateTime<Utc>>,
}

impl ResourceAuthority {
    fn from_state(state: ResourceState, latest_seq: SeqNo) -> Self {
        let authority = Self {
            shards: (0..ACCOUNT_SHARDS)
                .map(|_| Mutex::new(AccountShard::default()))
                .collect(),
            reservations: Mutex::new(state.reservations),
            latest_seq: Mutex::new(latest_seq),
            poisoned: Mutex::new(None),
        };
        for (account, limits) in state.limits {
            authority
                .shard_for_account(&account)
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .limits
                .insert(account, limits);
        }
        for (account, tally) in state.reserved_by_account {
            authority
                .shard_for_account(&account)
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .reserved_by_account
                .insert(account, tally);
        }
        for (account, tally) in state.usage_by_account {
            authority
                .shard_for_account(&account)
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .usage_by_account
                .insert(account, tally);
        }
        for (account, anchor) in state.period_anchors {
            authority
                .shard_for_account(&account)
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .period_anchors
                .insert(account, anchor);
        }
        authority
    }

    fn check_available(&self) -> Result<(), ResourceError> {
        let poisoned = self.poisoned.lock().map_err(|_| ResourceError::Storage {
            reason: "resource governor poison lock poisoned".to_string(),
        })?;
        if let Some(reason) = poisoned.as_ref() {
            return Err(ResourceError::Storage {
                reason: reason.clone(),
            });
        }
        Ok(())
    }

    fn poison(&self, error: ResourceError) {
        if let ResourceError::Storage { reason } = error
            && let Ok(mut poisoned) = self.poisoned.lock()
        {
            *poisoned = Some(reason);
        }
    }

    fn set_latest_seq(&self, seq: SeqNo) -> Result<(), ResourceError> {
        *self.latest_seq.lock().map_err(|_| ResourceError::Storage {
            reason: "resource governor journal cursor lock poisoned".to_string(),
        })? = seq;
        Ok(())
    }

    fn lock_reservations(
        &self,
    ) -> Result<MutexGuard<'_, HashMap<ResourceReservationId, ReservationRecord>>, ResourceError>
    {
        self.reservations
            .lock()
            .map_err(|_| ResourceError::Storage {
                reason: "resource governor reservation map lock poisoned".to_string(),
            })
    }

    fn lock_accounts(
        &self,
        accounts: &[ResourceAccount],
    ) -> Result<LockedAccounts<'_>, ResourceError> {
        let mut indexes = BTreeSet::new();
        for account in accounts {
            indexes.insert(account_shard_index(account));
        }
        let mut guards = Vec::with_capacity(indexes.len());
        for index in indexes {
            let guard = self.shards[index]
                .lock()
                .map_err(|_| ResourceError::Storage {
                    reason: "resource governor account shard lock poisoned".to_string(),
                })?;
            guards.push((index, guard));
        }
        Ok(LockedAccounts { guards })
    }

    fn shard_for_account(&self, account: &ResourceAccount) -> &Mutex<AccountShard> {
        &self.shards[account_shard_index(account)]
    }
}

struct LockedAccounts<'a> {
    guards: Vec<(usize, MutexGuard<'a, AccountShard>)>,
}

impl LockedAccounts<'_> {
    fn state_for_accounts(
        &mut self,
        accounts: &[ResourceAccount],
        reservations: HashMap<ResourceReservationId, ReservationRecord>,
    ) -> ResourceState {
        let mut state = ResourceState {
            reservations,
            ..ResourceState::default()
        };
        for account in accounts {
            let shard = self.shard_mut(account);
            if let Some(limits) = shard.limits.get(account) {
                state.limits.insert(account.clone(), limits.clone());
            }
            if let Some(tally) = shard.reserved_by_account.get(account) {
                state
                    .reserved_by_account
                    .insert(account.clone(), tally.clone());
            }
            if let Some(tally) = shard.usage_by_account.get(account) {
                state
                    .usage_by_account
                    .insert(account.clone(), tally.clone());
            }
            if let Some(anchor) = shard.period_anchors.get(account) {
                state.period_anchors.insert(account.clone(), *anchor);
            }
        }
        state
    }

    fn write_accounts_from_state(&mut self, accounts: &[ResourceAccount], state: &ResourceState) {
        for account in accounts {
            let shard = self.shard_mut(account);
            write_optional(
                &mut shard.limits,
                account,
                state.limits.get(account).cloned(),
            );
            write_optional(
                &mut shard.reserved_by_account,
                account,
                state.reserved_by_account.get(account).cloned(),
            );
            write_optional(
                &mut shard.usage_by_account,
                account,
                state.usage_by_account.get(account).cloned(),
            );
            write_optional(
                &mut shard.period_anchors,
                account,
                state.period_anchors.get(account).copied(),
            );
        }
    }

    fn account_parts(&mut self, account: &ResourceAccount) -> AccountParts {
        let shard = self.shard_mut(account);
        AccountParts {
            limits: shard.limits.get(account).cloned(),
            reserved: shard.reserved_by_account.get(account).cloned(),
            usage: shard.usage_by_account.get(account).cloned(),
            period_anchor: shard.period_anchors.get(account).copied(),
        }
    }

    fn shard_mut(&mut self, account: &ResourceAccount) -> &mut AccountShard {
        let index = account_shard_index(account);
        self.guards
            .iter_mut()
            .find(|(candidate, _)| *candidate == index)
            .map(|(_, guard)| &mut **guard)
            // lock_accounts builds the guard list from exactly the account
            // shard indexes requested before LockedAccounts is constructed.
            .expect("account shard was locked")
    }
}

fn write_optional<T: Clone>(
    map: &mut HashMap<ResourceAccount, T>,
    account: &ResourceAccount,
    value: Option<T>,
) {
    match value {
        Some(value) => {
            map.insert(account.clone(), value);
        }
        None => {
            map.remove(account);
        }
    }
}

fn account_shard_index(account: &ResourceAccount) -> usize {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    account.hash(&mut hasher);
    (hasher.finish() as usize) % ACCOUNT_SHARDS
}

fn delta_log_path() -> Result<ScopedPath, ResourceError> {
    ScopedPath::new(DELTA_LOG_PATH.to_string()).map_err(|error| {
        storage_error(format!("invalid resource governor delta log path: {error}"))
    })
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
    use crate::{ResourceGovernorStore, ResourceLimits};

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
        let snapshot = reloaded.account_snapshot(&account).unwrap().unwrap();
        assert_eq!(snapshot.ledger.spent.usd, dec!(0.25));
        assert_eq!(snapshot.ledger.reserved.usd, dec!(0));
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
