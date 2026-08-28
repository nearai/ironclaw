//! Synchronous recorder port for best-effort telemetry capture.

use ironclaw_host_api::resource::ResourceScope;

use crate::observation::TelemetryObservation;

/// The result of attempting to enqueue one typed observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordOutcome {
    Accepted,
    DroppedQueueFull,
    DroppedClosed,
    DroppedInvalid,
}

/// A producer-facing, nonblocking telemetry sink.
pub trait TelemetryRecorder: Send + Sync {
    fn try_record(&self, scope: ResourceScope, observation: TelemetryObservation) -> RecordOutcome;
}

/// A deliberately unwired recorder for callers that do not have telemetry
/// enabled. It performs no work and reports the same non-blocking closed-drop
/// outcome as a recorder whose intake has been shut down.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopTelemetryRecorder;

impl TelemetryRecorder for NoopTelemetryRecorder {
    fn try_record(
        &self,
        _scope: ResourceScope,
        _observation: TelemetryObservation,
    ) -> RecordOutcome {
        RecordOutcome::DroppedClosed
    }
}
