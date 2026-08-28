//! Tenant-scoped BI telemetry domain boundary.

pub mod aggregate;
mod buffered_recorder;
mod error;
pub mod records;
pub mod repository;
mod worker;

pub use aggregate::{AggregationError, aggregate_batch, floor_utc_hour};
pub use buffered_recorder::{
    BufferedRecorderConfig, BufferedTelemetryRecorder, BufferedTelemetryRecorderHandle,
    DEFAULT_TELEMETRY_MAX_BATCH_SIZE, DEFAULT_TELEMETRY_MAX_WAIT, DEFAULT_TELEMETRY_QUEUE_CAPACITY,
    DEFAULT_TELEMETRY_SHUTDOWN_TIMEOUT, SystemTelemetryClock, TelemetryClock, TelemetryDiagnostics,
    TelemetryWriteFailureClass,
};
pub use error::TelemetryRepositoryError;
pub use records::{
    CollectorCoverage, HourlyAutomationUsage, HourlyModelUsage, HourlyRunFailure,
    HourlyUserActivity, LifecycleEvent, RecordError, TelemetryBatch, TelemetryBatchRowFamily,
};

pub use repository::{
    BatchApplyReport, FilesystemTelemetryRepository, MAX_TELEMETRY_PAGE_SIZE, ScopedTelemetryBatch,
    TelemetryPage, TelemetryPageRequest,
};
