use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

const FAILURE_CLASS_COUNT: usize = 8;

/// Typed class for operational repository failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TelemetryWriteFailureClass {
    StorageAdmission = 1,
    StoragePoolAdmission = 2,
    StorageOperation = 3,
    CounterOverflow = 4,
    InvalidRecord = 5,
    InvalidData = 6,
    ShutdownTimeout = 7,
    CollectorIdResolution = 8,
}

impl TelemetryWriteFailureClass {
    const fn index(self) -> usize {
        match self {
            Self::StorageAdmission => 0,
            Self::StoragePoolAdmission => 1,
            Self::StorageOperation => 2,
            Self::CounterOverflow => 3,
            Self::InvalidRecord => 4,
            Self::InvalidData => 5,
            Self::ShutdownTimeout => 6,
            Self::CollectorIdResolution => 7,
        }
    }

    pub(crate) const fn from_repr(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::StorageAdmission),
            2 => Some(Self::StoragePoolAdmission),
            3 => Some(Self::StorageOperation),
            4 => Some(Self::CounterOverflow),
            5 => Some(Self::InvalidRecord),
            6 => Some(Self::InvalidData),
            7 => Some(Self::ShutdownTimeout),
            8 => Some(Self::CollectorIdResolution),
            _ => None,
        }
    }
}

const _: [(); FAILURE_CLASS_COUNT] =
    [(); TelemetryWriteFailureClass::CollectorIdResolution.index() + 1];

pub(crate) type FailureClassCode = TelemetryWriteFailureClass;

/// Count-only worker and queue diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TelemetryDiagnostics {
    accepted_observation_count: u64,
    queue_full_drop_count: u64,
    closed_drop_count: u64,
    invalid_drop_count: u64,
    write_failed_observation_count: u64,
    repository_failure_count: u64,
    partial_batch_failure_count: u64,
    flushed_batch_count: u64,
    flushed_observation_count: u64,
    last_batch_size: u64,
    last_flush_latency_ms: u64,
    last_failure_class: Option<TelemetryWriteFailureClass>,
    failure_class_counts: [u64; FAILURE_CLASS_COUNT],
    shutdown_timeout_count: u64,
    shutdown_write_loss_count: u64,
    shutdown_abandoned_observation_count: u64,
    coverage_key_overflow_count: u64,
    collector_id_resolution_failure_count: u64,
}

impl TelemetryDiagnostics {
    pub const fn accepted_observation_count(self) -> u64 {
        self.accepted_observation_count
    }
    pub const fn queue_full_drop_count(self) -> u64 {
        self.queue_full_drop_count
    }
    pub const fn closed_drop_count(self) -> u64 {
        self.closed_drop_count
    }
    pub const fn invalid_drop_count(self) -> u64 {
        self.invalid_drop_count
    }
    pub const fn write_failed_observation_count(self) -> u64 {
        self.write_failed_observation_count
    }
    pub const fn repository_failure_count(self) -> u64 {
        self.repository_failure_count
    }
    pub const fn partial_batch_failure_count(self) -> u64 {
        self.partial_batch_failure_count
    }
    pub const fn flushed_batch_count(self) -> u64 {
        self.flushed_batch_count
    }
    pub const fn flushed_observation_count(self) -> u64 {
        self.flushed_observation_count
    }
    pub const fn last_batch_size(self) -> u64 {
        self.last_batch_size
    }
    pub const fn last_flush_latency_ms(self) -> u64 {
        self.last_flush_latency_ms
    }
    pub const fn last_failure_class(self) -> Option<TelemetryWriteFailureClass> {
        self.last_failure_class
    }
    pub const fn failure_class_count(self, class: TelemetryWriteFailureClass) -> u64 {
        self.failure_class_counts[class.index()]
    }
    pub const fn shutdown_timeout_count(self) -> u64 {
        self.shutdown_timeout_count
    }
    pub const fn shutdown_write_loss_count(self) -> u64 {
        self.shutdown_write_loss_count
    }
    pub const fn shutdown_abandoned_observation_count(self) -> u64 {
        self.shutdown_abandoned_observation_count
    }
    pub const fn coverage_key_overflow_count(self) -> u64 {
        self.coverage_key_overflow_count
    }
    pub const fn collector_id_resolution_failure_count(self) -> u64 {
        self.collector_id_resolution_failure_count
    }
}

pub(crate) struct DiagnosticsState {
    pub(super) accepted_observation_count: AtomicU64,
    queue_full_drop_count: AtomicU64,
    closed_drop_count: AtomicU64,
    invalid_drop_count: AtomicU64,
    write_failed_observation_count: AtomicU64,
    repository_failure_count: AtomicU64,
    partial_batch_failure_count: AtomicU64,
    flushed_batch_count: AtomicU64,
    flushed_observation_count: AtomicU64,
    last_batch_size: AtomicU64,
    last_flush_latency_ms: AtomicU64,
    last_failure_class: AtomicU8,
    pub(super) failure_class_counts: [AtomicU64; FAILURE_CLASS_COUNT],
    shutdown_timeout_count: AtomicU64,
    shutdown_write_loss_count: AtomicU64,
    shutdown_abandoned_observation_count: AtomicU64,
    coverage_key_overflow_count: AtomicU64,
    collector_id_resolution_failure_count: AtomicU64,
}

impl Default for DiagnosticsState {
    fn default() -> Self {
        Self {
            accepted_observation_count: AtomicU64::new(0),
            queue_full_drop_count: AtomicU64::new(0),
            closed_drop_count: AtomicU64::new(0),
            invalid_drop_count: AtomicU64::new(0),
            write_failed_observation_count: AtomicU64::new(0),
            repository_failure_count: AtomicU64::new(0),
            partial_batch_failure_count: AtomicU64::new(0),
            flushed_batch_count: AtomicU64::new(0),
            flushed_observation_count: AtomicU64::new(0),
            last_batch_size: AtomicU64::new(0),
            last_flush_latency_ms: AtomicU64::new(0),
            last_failure_class: AtomicU8::new(0),
            failure_class_counts: std::array::from_fn(|_| AtomicU64::new(0)),
            shutdown_timeout_count: AtomicU64::new(0),
            shutdown_write_loss_count: AtomicU64::new(0),
            shutdown_abandoned_observation_count: AtomicU64::new(0),
            coverage_key_overflow_count: AtomicU64::new(0),
            collector_id_resolution_failure_count: AtomicU64::new(0),
        }
    }
}

impl DiagnosticsState {
    pub(crate) fn snapshot(&self) -> TelemetryDiagnostics {
        let last_failure_class =
            TelemetryWriteFailureClass::from_repr(self.last_failure_class.load(Ordering::Relaxed));
        TelemetryDiagnostics {
            accepted_observation_count: self.accepted_observation_count.load(Ordering::Relaxed),
            queue_full_drop_count: self.queue_full_drop_count.load(Ordering::Relaxed),
            closed_drop_count: self.closed_drop_count.load(Ordering::Relaxed),
            invalid_drop_count: self.invalid_drop_count.load(Ordering::Relaxed),
            write_failed_observation_count: self
                .write_failed_observation_count
                .load(Ordering::Relaxed),
            repository_failure_count: self.repository_failure_count.load(Ordering::Relaxed),
            partial_batch_failure_count: self.partial_batch_failure_count.load(Ordering::Relaxed),
            flushed_batch_count: self.flushed_batch_count.load(Ordering::Relaxed),
            flushed_observation_count: self.flushed_observation_count.load(Ordering::Relaxed),
            last_batch_size: self.last_batch_size.load(Ordering::Relaxed),
            last_flush_latency_ms: self.last_flush_latency_ms.load(Ordering::Relaxed),
            last_failure_class,
            failure_class_counts: std::array::from_fn(|index| {
                self.failure_class_counts[index].load(Ordering::Relaxed)
            }),
            shutdown_timeout_count: self.shutdown_timeout_count.load(Ordering::Relaxed),
            shutdown_write_loss_count: self.shutdown_write_loss_count.load(Ordering::Relaxed),
            shutdown_abandoned_observation_count: self
                .shutdown_abandoned_observation_count
                .load(Ordering::Relaxed),
            coverage_key_overflow_count: self.coverage_key_overflow_count.load(Ordering::Relaxed),
            collector_id_resolution_failure_count: self
                .collector_id_resolution_failure_count
                .load(Ordering::Relaxed),
        }
    }

    pub(crate) fn increment_accepted(&self) {
        self.add_counter(&self.accepted_observation_count, 1);
    }
    pub(crate) fn increment_queue_full(&self) {
        self.add_counter(&self.queue_full_drop_count, 1);
    }
    pub(crate) fn increment_closed(&self) {
        self.add_counter(&self.closed_drop_count, 1);
    }
    pub(crate) fn add_invalid(&self, count: usize) {
        let Ok(count) = u64::try_from(count) else {
            self.record_counter_overflow();
            return;
        };
        self.add_counter(&self.invalid_drop_count, count);
    }
    pub(crate) fn add_write_failed(&self, count: usize) {
        let Ok(count) = u64::try_from(count) else {
            self.record_counter_overflow();
            return;
        };
        self.add_counter(&self.write_failed_observation_count, count);
    }
    pub(crate) fn record_repository_failure(&self, class: FailureClassCode) {
        self.add_counter(&self.repository_failure_count, 1);
        self.record_failure(class);
    }

    pub(crate) fn record_partial_batch_failure(&self) {
        self.add_counter(&self.partial_batch_failure_count, 1);
        self.record_failure(FailureClassCode::StorageOperation);
    }

    pub(crate) fn record_failure(&self, class: FailureClassCode) {
        let index = class.index();
        if checked_atomic_add(&self.failure_class_counts[index], 1).is_err()
            && class != FailureClassCode::CounterOverflow
        {
            self.record_counter_overflow();
        }
        self.last_failure_class
            .store(class as u8, Ordering::Relaxed);
    }
    pub(crate) fn record_flush(&self, batch_size: usize, latency_ms: u64) {
        self.add_counter(&self.flushed_batch_count, 1);
        let Ok(batch_size) = u64::try_from(batch_size) else {
            self.record_counter_overflow();
            return;
        };
        self.add_counter(&self.flushed_observation_count, batch_size);
        self.last_batch_size.store(batch_size, Ordering::Relaxed);
        self.last_flush_latency_ms
            .store(latency_ms, Ordering::Relaxed);
    }

    pub(crate) fn record_shutdown_timeout(&self, abandoned: u64) {
        self.add_counter(&self.shutdown_timeout_count, 1);
        self.add_counter(&self.shutdown_write_loss_count, abandoned);
        self.add_counter(&self.shutdown_abandoned_observation_count, abandoned);
        self.add_counter(&self.write_failed_observation_count, abandoned);
        self.record_failure(FailureClassCode::ShutdownTimeout);
    }

    pub(crate) fn record_coverage_key_overflow(&self) {
        self.add_counter(&self.coverage_key_overflow_count, 1);
    }

    pub(crate) fn record_collector_id_resolution_failure(
        &self,
        error: &ironclaw_telemetry_contracts::observation::BoundedIdentifierError,
    ) {
        self.add_counter(&self.collector_id_resolution_failure_count, 1);
        self.record_failure(classify_collector_id_error(error));
    }

    fn add_counter(&self, counter: &AtomicU64, amount: u64) {
        if checked_atomic_add(counter, amount).is_err() {
            self.record_counter_overflow();
        }
    }

    pub(crate) fn record_counter_overflow(&self) {
        let index = FailureClassCode::CounterOverflow.index();
        let _ = checked_atomic_add(&self.failure_class_counts[index], 1);
        self.last_failure_class
            .store(FailureClassCode::CounterOverflow as u8, Ordering::Relaxed);
    }
}

pub(crate) fn checked_atomic_add(counter: &AtomicU64, amount: u64) -> Result<u64, u64> {
    counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
        current.checked_add(amount)
    })
}

fn classify_collector_id_error(
    error: &ironclaw_telemetry_contracts::observation::BoundedIdentifierError,
) -> FailureClassCode {
    match error {
        ironclaw_telemetry_contracts::observation::BoundedIdentifierError::Empty { .. }
        | ironclaw_telemetry_contracts::observation::BoundedIdentifierError::TooLong { .. }
        | ironclaw_telemetry_contracts::observation::BoundedIdentifierError::ControlCharacters {
            ..
        } => FailureClassCode::CollectorIdResolution,
    }
}
