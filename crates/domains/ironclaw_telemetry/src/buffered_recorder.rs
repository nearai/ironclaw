//! Non-blocking producer port for the telemetry batch worker.

use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Datelike, Timelike, Utc};
use ironclaw_host_api::resource::ResourceScope;
use ironclaw_telemetry_contracts::{
    observation::{
        CollectorInstanceId, MAX_DURABLE_COUNTER, ScopedTelemetryObservation, TelemetryObservation,
    },
    recorder::{RecordOutcome, TelemetryRecorder},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::repository::FilesystemTelemetryRepository;
use crate::{floor_utc_hour, repository::TelemetryBatchSink, worker};

pub const DEFAULT_TELEMETRY_QUEUE_CAPACITY: usize = 8_192;
pub(crate) const MAX_TELEMETRY_QUEUE_CAPACITY: usize = DEFAULT_TELEMETRY_QUEUE_CAPACITY;
pub const DEFAULT_TELEMETRY_MAX_BATCH_SIZE: usize = 512;
pub const DEFAULT_TELEMETRY_MAX_WAIT: Duration = Duration::from_secs(1);
pub const DEFAULT_TELEMETRY_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
/// Maximum distinct tenant/UTC-hour keys retained for count-only loss coverage.
///
/// Once this bound is reached, the global diagnostic still records the outcome,
/// but no unbounded producer-side state is allocated for a new key.
pub(crate) const MAX_COVERAGE_SIDE_KEYS: usize = 8_192;
const MAX_TELEMETRY_TIMESTAMP_YEAR: i32 = 9_999;

mod diagnostics;
mod intake;

pub(crate) use diagnostics::{DiagnosticsState, FailureClassCode, checked_atomic_add};
pub use diagnostics::{TelemetryDiagnostics, TelemetryWriteFailureClass};
pub(crate) use intake::{CoverageSideDelta, Intake, TenantHourKey};

/// Configuration for the bounded telemetry collector.
#[derive(Debug, Clone)]
pub struct BufferedRecorderConfig {
    pub queue_capacity: usize,
    pub max_batch_size: usize,
    pub max_wait: Duration,
    pub shutdown_timeout: Duration,
    pub collector_instance_id: Option<CollectorInstanceId>,
}

impl Default for BufferedRecorderConfig {
    fn default() -> Self {
        Self {
            queue_capacity: DEFAULT_TELEMETRY_QUEUE_CAPACITY,
            max_batch_size: DEFAULT_TELEMETRY_MAX_BATCH_SIZE,
            max_wait: DEFAULT_TELEMETRY_MAX_WAIT,
            shutdown_timeout: DEFAULT_TELEMETRY_SHUTDOWN_TIMEOUT,
            collector_instance_id: None,
        }
    }
}

impl BufferedRecorderConfig {
    pub fn with_queue_capacity(mut self, queue_capacity: usize) -> Self {
        self.queue_capacity = queue_capacity.clamp(1, MAX_TELEMETRY_QUEUE_CAPACITY);
        self
    }

    /// Returns the queue capacity that `spawn` will use, including for callers
    /// that construct this public config by setting fields directly.
    pub const fn effective_queue_capacity(&self) -> usize {
        if self.queue_capacity == 0 {
            1
        } else if self.queue_capacity > MAX_TELEMETRY_QUEUE_CAPACITY {
            MAX_TELEMETRY_QUEUE_CAPACITY
        } else {
            self.queue_capacity
        }
    }

    pub fn with_max_batch_size(mut self, max_batch_size: usize) -> Self {
        self.max_batch_size = max_batch_size.clamp(1, DEFAULT_TELEMETRY_MAX_BATCH_SIZE);
        self
    }

    pub fn with_max_wait(mut self, max_wait: Duration) -> Self {
        self.max_wait = if max_wait.is_zero() {
            Duration::from_millis(1)
        } else {
            max_wait.min(DEFAULT_TELEMETRY_MAX_WAIT)
        };
        self
    }

    pub fn with_shutdown_timeout(mut self, shutdown_timeout: Duration) -> Self {
        self.shutdown_timeout = if shutdown_timeout.is_zero() {
            Duration::from_millis(1)
        } else {
            shutdown_timeout.min(DEFAULT_TELEMETRY_SHUTDOWN_TIMEOUT)
        };
        self
    }

    pub fn with_collector_instance_id(
        mut self,
        collector_instance_id: CollectorInstanceId,
    ) -> Self {
        self.collector_instance_id = Some(collector_instance_id);
        self
    }
}

/// Clock used for coverage timestamps and count-only diagnostics.
pub trait TelemetryClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// Production wall clock implementation.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemTelemetryClock;

impl TelemetryClock for SystemTelemetryClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PreflightError {
    SystemScope,
    MissingSubjectUserAttribution,
    InvalidTimestamp,
    InvalidWindowStart,
    CounterOutOfRange,
}

impl PreflightError {
    pub(crate) const fn failure_class(self) -> FailureClassCode {
        match self {
            Self::SystemScope
            | Self::MissingSubjectUserAttribution
            | Self::InvalidTimestamp
            | Self::InvalidWindowStart => FailureClassCode::InvalidRecord,
            Self::CounterOutOfRange => FailureClassCode::CounterOverflow,
        }
    }
}

/// Shared non-blocking telemetry recorder.
pub struct BufferedTelemetryRecorder {
    intake: Arc<Intake>,
    diagnostics: Arc<DiagnosticsState>,
}

impl BufferedTelemetryRecorder {
    pub fn spawn<F>(
        config: BufferedRecorderConfig,
        repository: Arc<FilesystemTelemetryRepository<F>>,
        clock: Arc<dyn TelemetryClock>,
    ) -> (
        Arc<BufferedTelemetryRecorder>,
        BufferedTelemetryRecorderHandle,
    )
    where
        F: ironclaw_filesystem::RootFilesystem + ?Sized + 'static,
    {
        Self::spawn_with_sink(config, repository, clock)
    }

    fn spawn_with_sink(
        config: BufferedRecorderConfig,
        repository: Arc<dyn TelemetryBatchSink>,
        clock: Arc<dyn TelemetryClock>,
    ) -> (
        Arc<BufferedTelemetryRecorder>,
        BufferedTelemetryRecorderHandle,
    ) {
        let (sender, receiver) = mpsc::channel(config.effective_queue_capacity());
        let cancellation = CancellationToken::new();
        let diagnostics = Arc::new(DiagnosticsState::default());
        let collector_instance_id =
            resolve_collector_instance_id(config.collector_instance_id, &diagnostics);
        let intake = Arc::new(Intake::new(sender));
        let join = tokio::spawn(worker::run(
            worker::WorkerConfig {
                max_batch_size: config
                    .max_batch_size
                    .clamp(1, DEFAULT_TELEMETRY_MAX_BATCH_SIZE),
                max_wait: if config.max_wait.is_zero() {
                    Duration::from_millis(1)
                } else {
                    config.max_wait.min(DEFAULT_TELEMETRY_MAX_WAIT)
                },
                collector_instance_id,
            },
            receiver,
            Arc::clone(&intake),
            Arc::clone(&repository),
            clock,
            Arc::clone(&diagnostics),
            cancellation.clone(),
        ));
        let recorder = Arc::new(BufferedTelemetryRecorder {
            intake: Arc::clone(&intake),
            diagnostics: Arc::clone(&diagnostics),
        });
        let lifecycle = BufferedTelemetryRecorderHandle {
            cancellation,
            intake,
            diagnostics,
            join: Some(join),
            shutdown_timeout: if config.shutdown_timeout.is_zero() {
                Duration::from_millis(1)
            } else {
                config
                    .shutdown_timeout
                    .min(DEFAULT_TELEMETRY_SHUTDOWN_TIMEOUT)
            },
        };
        (recorder, lifecycle)
    }
}

fn resolve_collector_instance_id(
    configured: Option<CollectorInstanceId>,
    diagnostics: &DiagnosticsState,
) -> Option<CollectorInstanceId> {
    let candidate = match configured {
        Some(collector_instance_id) => Ok(collector_instance_id),
        None => CollectorInstanceId::new(format!("collector-{}", Uuid::new_v4())),
    };
    match candidate {
        Ok(collector_instance_id) => Some(collector_instance_id),
        Err(error) => {
            diagnostics.record_collector_id_resolution_failure(&error);
            match CollectorInstanceId::new("collector-fallback") {
                Ok(collector_instance_id) => Some(collector_instance_id),
                Err(fallback_error) => {
                    diagnostics.record_collector_id_resolution_failure(&fallback_error);
                    None
                }
            }
        }
    }
}

impl TelemetryRecorder for BufferedTelemetryRecorder {
    fn try_record(&self, scope: ResourceScope, observation: TelemetryObservation) -> RecordOutcome {
        let preflight = preflight_observation(&scope, &observation);
        if matches!(preflight, Err(PreflightError::SystemScope)) {
            let error = PreflightError::SystemScope;
            self.diagnostics.add_invalid(1);
            self.diagnostics.record_failure(error.failure_class());
            return RecordOutcome::DroppedInvalid;
        }
        let key = TenantHourKey {
            tenant_id: scope.tenant_id.clone(),
            window_start: floor_utc_hour(observation.occurred_at()),
        };
        self.intake.try_record(
            ScopedTelemetryObservation::new(scope, observation),
            key,
            self.diagnostics.as_ref(),
            preflight,
        )
    }
}

fn preflight_observation(
    scope: &ResourceScope,
    observation: &TelemetryObservation,
) -> Result<(), PreflightError> {
    if scope.is_system() {
        return Err(PreflightError::SystemScope);
    }
    let occurred_at = observation.occurred_at();
    if !(1..=MAX_TELEMETRY_TIMESTAMP_YEAR).contains(&occurred_at.year()) {
        return Err(PreflightError::InvalidTimestamp);
    }
    let window_start = floor_utc_hour(occurred_at);
    if window_start > occurred_at
        || window_start.minute() != 0
        || window_start.second() != 0
        || window_start.nanosecond() != 0
    {
        return Err(PreflightError::InvalidWindowStart);
    }
    match observation {
        TelemetryObservation::RunSettled(observation) => {
            if observation.duration_ms() > MAX_DURABLE_COUNTER
                || observation
                    .reported_tool_call_count()
                    .is_some_and(|count| count > MAX_DURABLE_COUNTER)
            {
                return Err(PreflightError::CounterOutOfRange);
            }
        }
        TelemetryObservation::ModelCallCompleted(observation) => {
            if [
                observation.input_tokens(),
                observation.output_tokens(),
                observation.cache_read_input_tokens(),
                observation.cache_creation_input_tokens(),
            ]
            .into_iter()
            .any(|value| value > MAX_DURABLE_COUNTER)
            {
                return Err(PreflightError::CounterOutOfRange);
            }
        }
        TelemetryObservation::AutomationSettled(_) => {}
        TelemetryObservation::LifecycleTransition(observation) => {
            if observation.subject_kind()
                != ironclaw_telemetry_contracts::observation::LifecycleSubjectKind::Tenant
                && observation.subject_user_id().is_none()
            {
                return Err(PreflightError::MissingSubjectUserAttribution);
            }
        }
    }
    Ok(())
}

/// Lifecycle owner for the single telemetry consumer task.
pub struct BufferedTelemetryRecorderHandle {
    cancellation: CancellationToken,
    intake: Arc<Intake>,
    diagnostics: Arc<DiagnosticsState>,
    join: Option<tokio::task::JoinHandle<()>>,
    shutdown_timeout: Duration,
}

impl BufferedTelemetryRecorderHandle {
    pub fn diagnostics(&self) -> TelemetryDiagnostics {
        self.diagnostics.snapshot()
    }

    pub fn close_intake(&self) {
        self.intake.close();
    }

    pub async fn shutdown(mut self) -> TelemetryDiagnostics {
        self.intake.close();
        self.cancellation.cancel();
        if let Some(mut join) = self.join.take()
            && tokio::time::timeout(self.shutdown_timeout, &mut join)
                .await
                .is_err()
        {
            join.abort();
            let _ = join.await;
            let (_, abandoned) = self.intake.take_unpersisted();
            self.diagnostics.record_shutdown_timeout(abandoned);
        }
        self.diagnostics.snapshot()
    }
}

impl Drop for BufferedTelemetryRecorderHandle {
    fn drop(&mut self) {
        self.intake.close();
        self.cancellation.cancel();
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}

pub(crate) fn classify_aggregation_error(error: &crate::AggregationError) -> FailureClassCode {
    match error {
        crate::AggregationError::CounterOverflow { .. }
        | crate::AggregationError::CounterOutOfRange { .. } => FailureClassCode::CounterOverflow,
        crate::AggregationError::InvalidRecord(error) => classify_record_error(error),
    }
}

pub(crate) fn classify_record_error(error: &crate::RecordError) -> FailureClassCode {
    match error {
        crate::RecordError::CounterOutOfRange { .. }
        | crate::RecordError::TerminalCountOverflow => FailureClassCode::CounterOverflow,
        crate::RecordError::InvalidWindowStart
        | crate::RecordError::InvalidObservationRange
        | crate::RecordError::TerminalCountMismatch
        | crate::RecordError::ReportedToolCountExceedsRuns
        | crate::RecordError::ReportedUsageExceedsInferences
        | crate::RecordError::DuplicateRow { .. }
        | crate::RecordError::MissingUserAttribution => FailureClassCode::InvalidRecord,
    }
}

pub(crate) fn classify_repository_error(
    error: &crate::TelemetryRepositoryError,
) -> FailureClassCode {
    match error {
        crate::TelemetryRepositoryError::StorageAdmission { .. } => {
            FailureClassCode::StorageAdmission
        }
        crate::TelemetryRepositoryError::StoragePoolAdmission { .. } => {
            FailureClassCode::StoragePoolAdmission
        }
        crate::TelemetryRepositoryError::StorageOperation { .. } => {
            FailureClassCode::StorageOperation
        }
        crate::TelemetryRepositoryError::CounterOverflow { .. } => {
            FailureClassCode::CounterOverflow
        }
        crate::TelemetryRepositoryError::CounterConversion { .. } => {
            FailureClassCode::CounterOverflow
        }
        crate::TelemetryRepositoryError::Record(_) => FailureClassCode::InvalidRecord,
        crate::TelemetryRepositoryError::InvalidScanRequest { .. }
        | crate::TelemetryRepositoryError::InvalidPageRequest { .. }
        | crate::TelemetryRepositoryError::InvalidCursor
        | crate::TelemetryRepositoryError::InvalidCursorEncoding { .. }
        | crate::TelemetryRepositoryError::InvalidCursorLength { .. }
        | crate::TelemetryRepositoryError::InvalidTimestamp { .. }
        | crate::TelemetryRepositoryError::InvalidPersistedField { .. }
        | crate::TelemetryRepositoryError::UnknownEnum { .. } => FailureClassCode::InvalidData,
        crate::TelemetryRepositoryError::ScopeMismatch
        | crate::TelemetryRepositoryError::InvalidProjection
        | crate::TelemetryRepositoryError::Serialization { .. } => FailureClassCode::InvalidData,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    #[test]
    fn checked_atomic_add_rejects_overflow_without_wrapping() {
        let counter = AtomicU64::new(u64::MAX);

        assert!(checked_atomic_add(&counter, 1).is_err());
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn diagnostic_counter_overflow_is_typed_and_loss_safe() {
        let diagnostics = DiagnosticsState::default();
        diagnostics
            .accepted_observation_count
            .store(u64::MAX, Ordering::Relaxed);

        diagnostics.increment_accepted();

        let snapshot = diagnostics.snapshot();
        assert_eq!(snapshot.accepted_observation_count(), u64::MAX);
        assert_eq!(
            snapshot.failure_class_count(TelemetryWriteFailureClass::CounterOverflow),
            1
        );
        assert_eq!(
            snapshot.last_failure_class(),
            Some(TelemetryWriteFailureClass::CounterOverflow)
        );

        diagnostics.failure_class_counts[FailureClassCode::CounterOverflow as usize - 1]
            .store(u64::MAX, Ordering::Relaxed);
        diagnostics.record_counter_overflow();
        assert_eq!(
            diagnostics
                .snapshot()
                .failure_class_count(TelemetryWriteFailureClass::CounterOverflow),
            u64::MAX
        );
    }
}

// Keep worker fakes behind the crate-private sink seam. They need to observe
// the worker's exact call shape, but that seam is intentionally not a public
// repository selector.
#[cfg(test)]
#[path = "buffered_recorder_contract_tests.rs"]
mod buffered_recorder_contract;
