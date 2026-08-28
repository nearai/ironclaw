use std::{
    collections::BTreeMap,
    num::TryFromIntError,
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
};

use chrono::{DateTime, Utc};
use ironclaw_host_api::ids::TenantId;
use ironclaw_telemetry_contracts::{
    observation::ScopedTelemetryObservation, recorder::RecordOutcome,
};
use tokio::sync::Notify;
use tokio::sync::mpsc;

use super::{DiagnosticsState, FailureClassCode, MAX_COVERAGE_SIDE_KEYS, checked_atomic_add};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct TenantHourKey {
    pub(crate) tenant_id: TenantId,
    pub(crate) window_start: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CoverageSideDelta {
    pub(crate) accepted_pending: u64,
    pub(crate) queue_full_drop_count: u64,
    pub(crate) closed_drop_count: u64,
    pub(crate) invalid_drop_count: u64,
    pub(crate) first_observed_at: Option<DateTime<Utc>>,
    pub(crate) last_observed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Default)]
struct CoverageSideAccumulator {
    entries: BTreeMap<TenantHourKey, CoverageSideDelta>,
}

impl CoverageSideAccumulator {
    fn record(
        &mut self,
        key: TenantHourKey,
        occurred_at: DateTime<Utc>,
        update: impl FnOnce(&mut CoverageSideDelta) -> Result<(), ()>,
    ) -> Result<bool, ()> {
        if let Some(delta) = self.entries.get_mut(&key) {
            update(delta)?;
            delta.record_timestamp(occurred_at);
            return Ok(true);
        }
        if self.entries.len() >= MAX_COVERAGE_SIDE_KEYS {
            return Ok(false);
        }
        let mut delta = CoverageSideDelta::default();
        update(&mut delta)?;
        delta.record_timestamp(occurred_at);
        self.entries.insert(key, delta);
        Ok(true)
    }

    fn take_drop_deltas(&mut self) -> BTreeMap<TenantHourKey, CoverageSideDelta> {
        let mut drops = BTreeMap::new();
        self.entries.retain(|key, delta| {
            let drop_delta = CoverageSideDelta {
                accepted_pending: 0,
                queue_full_drop_count: delta.queue_full_drop_count,
                closed_drop_count: delta.closed_drop_count,
                invalid_drop_count: delta.invalid_drop_count,
                first_observed_at: delta.first_observed_at,
                last_observed_at: delta.last_observed_at,
            };
            if drop_delta.queue_full_drop_count != 0
                || drop_delta.closed_drop_count != 0
                || drop_delta.invalid_drop_count != 0
            {
                drops.insert(key.clone(), drop_delta);
                delta.queue_full_drop_count = 0;
                delta.closed_drop_count = 0;
                delta.invalid_drop_count = 0;
                delta.first_observed_at = None;
                delta.last_observed_at = None;
            }
            delta.accepted_pending != 0
                || delta.queue_full_drop_count != 0
                || delta.closed_drop_count != 0
                || delta.invalid_drop_count != 0
        });
        drops
    }

    fn restore_drop_delta(
        &mut self,
        key: TenantHourKey,
        delta: CoverageSideDelta,
    ) -> Result<(), ()> {
        let Some(existing) = self.entries.get(&key) else {
            if self.entries.len() >= MAX_COVERAGE_SIDE_KEYS {
                return Err(());
            }
            self.entries.insert(key, delta);
            return Ok(());
        };
        let queue_full_drop_count = existing
            .queue_full_drop_count
            .checked_add(delta.queue_full_drop_count)
            .ok_or(())?;
        let closed_drop_count = existing
            .closed_drop_count
            .checked_add(delta.closed_drop_count)
            .ok_or(())?;
        let invalid_drop_count = existing
            .invalid_drop_count
            .checked_add(delta.invalid_drop_count)
            .ok_or(())?;
        let first_observed_at = match (existing.first_observed_at, delta.first_observed_at) {
            (Some(existing), Some(delta)) => Some(existing.min(delta)),
            (Some(existing), None) => Some(existing),
            (None, Some(delta)) => Some(delta),
            (None, None) => None,
        };
        let last_observed_at = match (existing.last_observed_at, delta.last_observed_at) {
            (Some(existing), Some(delta)) => Some(existing.max(delta)),
            (Some(existing), None) => Some(existing),
            (None, Some(delta)) => Some(delta),
            (None, None) => None,
        };
        let existing = self.entries.get_mut(&key).ok_or(())?;
        existing.queue_full_drop_count = queue_full_drop_count;
        existing.closed_drop_count = closed_drop_count;
        existing.invalid_drop_count = invalid_drop_count;
        existing.first_observed_at = first_observed_at;
        existing.last_observed_at = last_observed_at;
        Ok(())
    }

    fn account_observations(
        &mut self,
        keys: impl IntoIterator<Item = TenantHourKey>,
    ) -> Result<(), ()> {
        let mut requested = BTreeMap::<TenantHourKey, u64>::new();
        for key in keys {
            let entry = requested.entry(key).or_default();
            *entry = entry.checked_add(1).ok_or(())?;
        }
        for (key, count) in &requested {
            if let Some(delta) = self.entries.get(key)
                && delta.accepted_pending < *count
            {
                return Err(());
            }
        }
        for (key, count) in requested {
            if let Some(delta) = self.entries.get_mut(&key) {
                delta.accepted_pending -= count;
            }
        }
        self.entries.retain(|_, delta| {
            delta.accepted_pending != 0
                || delta.queue_full_drop_count != 0
                || delta.closed_drop_count != 0
                || delta.invalid_drop_count != 0
        });
        Ok(())
    }

    fn take_unpersisted(&mut self) -> BTreeMap<TenantHourKey, u64> {
        let mut abandoned = BTreeMap::new();
        for (key, delta) in &self.entries {
            if delta.accepted_pending != 0 {
                abandoned.insert(key.clone(), delta.accepted_pending);
            }
        }
        for delta in self.entries.values_mut() {
            delta.accepted_pending = 0;
        }
        self.entries.retain(|_, delta| {
            delta.queue_full_drop_count != 0
                || delta.closed_drop_count != 0
                || delta.invalid_drop_count != 0
        });
        abandoned
    }
}

impl CoverageSideDelta {
    fn record_timestamp(&mut self, occurred_at: DateTime<Utc>) {
        self.first_observed_at = Some(match self.first_observed_at {
            Some(first) => first.min(occurred_at),
            None => occurred_at,
        });
        self.last_observed_at = Some(match self.last_observed_at {
            Some(last) => last.max(occurred_at),
            None => occurred_at,
        });
    }
}

struct IntakeState {
    sender: mpsc::Sender<ScopedTelemetryObservation>,
    closed: bool,
    coverage: CoverageSideAccumulator,
}

#[derive(Debug)]
pub(crate) enum IntakeAccountingError {
    KeyCountOverflow {
        count: usize,
        source: TryFromIntError,
    },
    PendingCountUnderflow {
        pending: u64,
        requested: u64,
    },
    CoveragePendingUnderflow,
}

impl IntakeAccountingError {
    pub(crate) const fn failure_class(&self) -> FailureClassCode {
        match self {
            Self::KeyCountOverflow { count, source } => {
                debug_assert!(*count > u64::MAX as usize);
                let _ = source;
                FailureClassCode::CounterOverflow
            }
            Self::PendingCountUnderflow { pending, requested } => {
                debug_assert!(*pending < *requested);
                FailureClassCode::CounterOverflow
            }
            Self::CoveragePendingUnderflow => FailureClassCode::CounterOverflow,
        }
    }
}

pub(crate) struct Intake {
    state: Mutex<IntakeState>,
    notify: Notify,
    pending_observation_count: AtomicU64,
}

fn record_coverage_event(
    state: &mut IntakeState,
    key: TenantHourKey,
    occurred_at: DateTime<Utc>,
    diagnostics: &DiagnosticsState,
    update: impl FnOnce(&mut CoverageSideDelta) -> Result<(), ()>,
) {
    match state.coverage.record(key, occurred_at, update) {
        Ok(true) => {}
        Ok(false) => diagnostics.record_coverage_key_overflow(),
        Err(()) => diagnostics.record_counter_overflow(),
    }
}

impl Intake {
    pub(crate) fn new(sender: mpsc::Sender<ScopedTelemetryObservation>) -> Self {
        Self {
            state: Mutex::new(IntakeState {
                sender,
                closed: false,
                coverage: CoverageSideAccumulator::default(),
            }),
            notify: Notify::new(),
            pending_observation_count: AtomicU64::new(0),
        }
    }

    fn lock(&self) -> MutexGuard<'_, IntakeState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub(crate) fn close(&self) {
        let mut state = self.lock();
        state.closed = true;
        drop(state);
        self.notify.notify_one();
    }

    pub(crate) fn notified(&self) -> impl std::future::Future<Output = ()> + '_ {
        self.notify.notified()
    }

    pub(crate) fn take_drop_deltas(&self) -> BTreeMap<TenantHourKey, CoverageSideDelta> {
        self.lock().coverage.take_drop_deltas()
    }

    pub(crate) fn restore_drop_deltas(
        &self,
        deltas: impl IntoIterator<Item = (TenantHourKey, CoverageSideDelta)>,
        diagnostics: &DiagnosticsState,
    ) {
        let mut state = self.lock();
        let mut restored_any = false;
        for (key, delta) in deltas {
            if state.coverage.restore_drop_delta(key, delta).is_err() {
                diagnostics.record_coverage_key_overflow();
            } else {
                restored_any = true;
            }
        }
        drop(state);
        if restored_any {
            self.notify.notify_one();
        }
    }

    pub(crate) fn account_observations(
        &self,
        keys: impl IntoIterator<Item = TenantHourKey>,
    ) -> Result<(), IntakeAccountingError> {
        let keys: Vec<_> = keys.into_iter().collect();
        let count = u64::try_from(keys.len()).map_err(|source| {
            IntakeAccountingError::KeyCountOverflow {
                count: keys.len(),
                source,
            }
        })?;
        let mut state = self.lock();
        if state.coverage.account_observations(keys.clone()).is_err() {
            return Err(IntakeAccountingError::CoveragePendingUnderflow);
        }
        match self.pending_observation_count.fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |pending| pending.checked_sub(count),
        ) {
            Ok(_) => Ok(()),
            Err(pending) => Err(IntakeAccountingError::PendingCountUnderflow {
                pending,
                requested: count,
            }),
        }
    }

    pub(crate) fn take_unpersisted(&self) -> (BTreeMap<TenantHourKey, u64>, u64) {
        let mut state = self.lock();
        let unpersisted = state.coverage.take_unpersisted();
        let pending = self.pending_observation_count.swap(0, Ordering::AcqRel);
        (unpersisted, pending)
    }

    pub(crate) fn try_record(
        &self,
        observation: ScopedTelemetryObservation,
        key: TenantHourKey,
        diagnostics: &DiagnosticsState,
        preflight: Result<(), super::PreflightError>,
    ) -> RecordOutcome {
        let occurred_at = observation.occurred_at();
        let mut state = self.lock();
        if state.closed {
            record_coverage_event(&mut state, key, occurred_at, diagnostics, |delta| {
                delta.closed_drop_count = delta.closed_drop_count.checked_add(1).ok_or(())?;
                Ok(())
            });
            diagnostics.increment_closed();
            drop(state);
            self.notify.notify_one();
            return RecordOutcome::DroppedClosed;
        }
        if let Err(error) = preflight {
            record_coverage_event(&mut state, key, occurred_at, diagnostics, |delta| {
                delta.invalid_drop_count = delta.invalid_drop_count.checked_add(1).ok_or(())?;
                Ok(())
            });
            diagnostics.add_invalid(1);
            diagnostics.record_failure(error.failure_class());
            drop(state);
            self.notify.notify_one();
            return RecordOutcome::DroppedInvalid;
        }
        let outcome = state.sender.try_send(observation);
        match outcome {
            Ok(()) => {
                record_coverage_event(&mut state, key, occurred_at, diagnostics, |delta| {
                    delta.accepted_pending = delta.accepted_pending.checked_add(1).ok_or(())?;
                    Ok(())
                });
                if checked_atomic_add(&self.pending_observation_count, 1).is_err() {
                    diagnostics.record_counter_overflow();
                }
                diagnostics.increment_accepted();
                RecordOutcome::Accepted
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                record_coverage_event(&mut state, key, occurred_at, diagnostics, |delta| {
                    delta.queue_full_drop_count =
                        delta.queue_full_drop_count.checked_add(1).ok_or(())?;
                    Ok(())
                });
                diagnostics.increment_queue_full();
                drop(state);
                self.notify.notify_one();
                RecordOutcome::DroppedQueueFull
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                record_coverage_event(&mut state, key, occurred_at, diagnostics, |delta| {
                    delta.closed_drop_count = delta.closed_drop_count.checked_add(1).ok_or(())?;
                    Ok(())
                });
                diagnostics.increment_closed();
                drop(state);
                self.notify.notify_one();
                RecordOutcome::DroppedClosed
            }
        }
    }
}
