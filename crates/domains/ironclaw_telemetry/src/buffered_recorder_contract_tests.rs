use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::repository::{ScopedTelemetryBatch, TelemetryBatchSink};
use crate::{
    BatchApplyReport, BufferedRecorderConfig, BufferedTelemetryRecorder, RecordError,
    TelemetryBatch, TelemetryClock, TelemetryRepositoryError, TelemetryWriteFailureClass,
};
use chrono::{DateTime, TimeZone, Utc};
use ironclaw_host_api::{
    ids::{InvocationId, TenantId, UserId},
    resource::ResourceScope,
};
use ironclaw_telemetry_contracts::observation::{
    AutomationKind, AutomationSettledObservation, EffectiveModelId, LifecycleEventId,
    LifecycleEventKind, LifecycleSubjectKind, LifecycleTransitionObservation,
    ModelCallCompletedObservation, ModelUsage, ObservationContext, OriginKind, ProviderId,
    RunOutcome, RunSettledObservation, TelemetryObservation,
};
use ironclaw_telemetry_contracts::recorder::RecordOutcome;
use ironclaw_telemetry_contracts::recorder::TelemetryRecorder;

const START: i64 = 1_756_200_000;

#[derive(Clone)]
struct FixedClock {
    now: Arc<Mutex<DateTime<Utc>>>,
}

impl FixedClock {
    fn new(now: DateTime<Utc>) -> Self {
        Self {
            now: Arc::new(Mutex::new(now)),
        }
    }
}

impl TelemetryClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().expect("clock lock")
    }
}

#[derive(Default)]
struct FakeRepository {
    state: Mutex<FakeRepositoryState>,
}

#[derive(Default)]
struct FakeRepositoryState {
    batches: Vec<TelemetryBatch>,
    scopes: Vec<ironclaw_host_api::resource::ResourceScope>,
    failures_remaining: usize,
    always_fail: bool,
    commit_then_error: bool,
    next_error: Option<TelemetryRepositoryError>,
    fail_on_write: Option<usize>,
    write_count: usize,
    next_report: Option<BatchApplyReport>,
    active_writes: usize,
    max_active_writes: usize,
    write_started: Option<tokio::sync::oneshot::Sender<()>>,
    release_write: Option<tokio::sync::oneshot::Receiver<()>>,
}

impl FakeRepository {
    fn batches(&self) -> Vec<TelemetryBatch> {
        self.state.lock().expect("repository lock").batches.clone()
    }

    fn fail_next(&self) {
        self.state
            .lock()
            .expect("repository lock")
            .failures_remaining += 1;
    }

    fn set_fail_all(&self, fail: bool) {
        self.state.lock().expect("repository lock").always_fail = fail;
    }

    fn fail_next_with(&self, error: TelemetryRepositoryError) {
        self.state.lock().expect("repository lock").next_error = Some(error);
    }

    fn fail_on_write(&self, write_number: usize) {
        self.state.lock().expect("repository lock").fail_on_write = Some(write_number);
    }

    fn return_next_report(&self, report: BatchApplyReport) {
        self.state.lock().expect("repository lock").next_report = Some(report);
    }

    fn scopes(&self) -> Vec<ironclaw_host_api::resource::ResourceScope> {
        self.state.lock().expect("repository lock").scopes.clone()
    }

    fn fail_next_after_commit(&self) {
        let mut state = self.state.lock().expect("repository lock");
        state.failures_remaining += 1;
        state.commit_then_error = true;
    }

    fn max_active_writes(&self) -> usize {
        self.state
            .lock()
            .expect("repository lock")
            .max_active_writes
    }

    fn block_next_write(
        &self,
    ) -> (
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Sender<()>,
    ) {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let mut state = self.state.lock().expect("repository lock");
        state.write_started = Some(started_tx);
        state.release_write = Some(release_rx);
        (started_rx, release_tx)
    }
}

#[async_trait::async_trait]
impl TelemetryBatchSink for FakeRepository {
    async fn apply_batch(
        &self,
        batch: ScopedTelemetryBatch,
    ) -> Result<BatchApplyReport, TelemetryRepositoryError> {
        let (started, release, fail, committed_before_error, injected_error, report) = {
            let mut state = self.state.lock().expect("repository lock");
            state.write_count += 1;
            state.active_writes += 1;
            state.max_active_writes = state.max_active_writes.max(state.active_writes);
            let fail = state.next_error.is_some()
                || if state.always_fail {
                    true
                } else if state.failures_remaining > 0 {
                    state.failures_remaining -= 1;
                    true
                } else {
                    state.fail_on_write == Some(state.write_count)
                };
            let committed_before_error = fail && state.commit_then_error;
            if committed_before_error {
                state.commit_then_error = false;
            }
            (
                state.write_started.take(),
                state.release_write.take(),
                fail,
                committed_before_error,
                state.next_error.take(),
                state.next_report.take(),
            )
        };
        if let Some(started) = started {
            let _ = started.send(());
        }
        if let Some(release) = release {
            let _ = release.await;
        }
        {
            let mut state = self.state.lock().expect("repository lock");
            state.active_writes -= 1;
            if !fail || committed_before_error {
                state.scopes.push(batch.scope().clone());
                state.batches.push(batch.batch().clone());
            }
        }
        if fail {
            Err(
                injected_error.unwrap_or(TelemetryRepositoryError::StorageOperation {
                    operation: "fake batch write",
                    source: "injected failure".to_owned().into(),
                }),
            )
        } else {
            Ok(report.unwrap_or_else(|| BatchApplyReport::complete(batch.batch().record_count())))
        }
    }
}

fn timestamp(offset_seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(START + offset_seconds, 0)
        .single()
        .expect("valid timestamp")
}

fn context(offset_seconds: i64) -> ObservationContext {
    ObservationContext::new(timestamp(offset_seconds))
}

fn scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").expect("tenant"),
        user_id: UserId::new("user-a").expect("user"),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn scope_for(index: u64) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(format!("tenant-{index}")).expect("tenant"),
        user_id: UserId::new(format!("user-{index}")).expect("user"),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn completed_run(offset_seconds: i64) -> TelemetryObservation {
    TelemetryObservation::RunSettled(
        RunSettledObservation::new(
            context(offset_seconds),
            OriginKind::Human,
            RunOutcome::Completed,
            25,
            Some(1),
            None,
        )
        .expect("run"),
    )
}

fn completed_run_for_tenant_hour(index: u64) -> (ResourceScope, TelemetryObservation) {
    let occurred_at = timestamp((index as i64) * 3_600);
    (
        scope_for(index),
        TelemetryObservation::RunSettled(
            RunSettledObservation::new(
                ObservationContext::new(occurred_at),
                OriginKind::Human,
                RunOutcome::Completed,
                1,
                None,
                None,
            )
            .expect("run"),
        ),
    )
}

trait TestRecorderCall {
    fn try_record(&self, observation: TelemetryObservation) -> RecordOutcome;
    fn try_record_scoped(
        &self,
        scope: ResourceScope,
        observation: TelemetryObservation,
    ) -> RecordOutcome;
}

impl<T: TelemetryRecorder + ?Sized> TestRecorderCall for Arc<T> {
    fn try_record(&self, observation: TelemetryObservation) -> RecordOutcome {
        self.try_record_scoped(scope(), observation)
    }

    fn try_record_scoped(
        &self,
        scope: ResourceScope,
        observation: TelemetryObservation,
    ) -> RecordOutcome {
        TelemetryRecorder::try_record(self.as_ref(), scope, observation)
    }
}

fn model_call(offset_seconds: i64) -> TelemetryObservation {
    TelemetryObservation::ModelCallCompleted(
        ModelCallCompletedObservation::new(
            context(offset_seconds),
            ProviderId::new("provider-a").expect("provider"),
            EffectiveModelId::new("model-a").expect("model"),
            Some(ModelUsage::new(3, 4, 5, 6)),
        )
        .expect("model call"),
    )
}

fn automation(offset_seconds: i64) -> TelemetryObservation {
    TelemetryObservation::AutomationSettled(
        AutomationSettledObservation::new(
            context(offset_seconds),
            ironclaw_telemetry_contracts::observation::AutomationId::new("automation-a")
                .expect("automation"),
            AutomationKind::Cron,
            RunOutcome::Completed,
        )
        .expect("automation"),
    )
}

fn config() -> BufferedRecorderConfig {
    BufferedRecorderConfig::default()
        .with_queue_capacity(16)
        .with_max_batch_size(512)
        .with_max_wait(Duration::from_secs(1))
        .with_shutdown_timeout(Duration::from_secs(5))
}

async fn wait_for_batches(repository: &FakeRepository, count: usize) {
    for _ in 0..100 {
        if repository.batches().len() >= count {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("timed out waiting for {count} repository batches");
}

#[path = "buffered_recorder_contract_tests/failure_recovery.rs"]
mod failure_recovery;
#[path = "buffered_recorder_contract_tests/queue_coverage.rs"]
mod queue_coverage;
#[path = "buffered_recorder_contract_tests/shutdown_lifecycle.rs"]
mod shutdown_lifecycle;
#[path = "buffered_recorder_contract_tests/validation_attribution.rs"]
mod validation_attribution;
