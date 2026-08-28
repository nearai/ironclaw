//! The single consumer that aggregates and persists telemetry drains.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use ironclaw_host_api::{
    ids::{InvocationId, TenantId, UserId as WorkerUserId},
    resource::ResourceScope,
};
use ironclaw_telemetry_contracts::observation::{CollectorInstanceId, ScopedTelemetryObservation};
use tokio::{select, sync::mpsc};
use tokio_util::sync::CancellationToken;

use crate::{
    CollectorCoverage, ScopedTelemetryBatch, TelemetryBatch, aggregate_batch,
    buffered_recorder::{
        CoverageSideDelta, DiagnosticsState, Intake, MAX_COVERAGE_SIDE_KEYS, TelemetryClock,
        TenantHourKey, classify_aggregation_error, classify_record_error,
        classify_repository_error,
    },
    floor_utc_hour,
    repository::TelemetryBatchSink,
};

pub(crate) struct WorkerConfig {
    pub(crate) max_batch_size: usize,
    pub(crate) max_wait: Duration,
    pub(crate) collector_instance_id: Option<CollectorInstanceId>,
}

#[derive(Debug)]
struct CoverageAccumulator {
    tenant_id: TenantId,
    window_start: DateTime<Utc>,
    accepted_observation_count: u64,
    queue_full_drop_count: u64,
    closed_drop_count: u64,
    invalid_drop_count: u64,
    write_failed_observation_count: u64,
    first_observed_at: DateTime<Utc>,
    last_observed_at: DateTime<Utc>,
}

impl CoverageAccumulator {
    fn from_key(key: &TenantHourKey) -> Self {
        Self {
            tenant_id: key.tenant_id.clone(),
            window_start: key.window_start,
            accepted_observation_count: 0,
            queue_full_drop_count: 0,
            closed_drop_count: 0,
            invalid_drop_count: 0,
            write_failed_observation_count: 0,
            first_observed_at: key.window_start,
            last_observed_at: key.window_start,
        }
    }

    fn from_observation(observation: &ScopedTelemetryObservation) -> Self {
        let occurred_at = observation.occurred_at();
        Self {
            tenant_id: observation.scope().tenant_id.clone(),
            window_start: floor_utc_hour(occurred_at),
            accepted_observation_count: 1,
            queue_full_drop_count: 0,
            closed_drop_count: 0,
            invalid_drop_count: 0,
            write_failed_observation_count: 0,
            first_observed_at: occurred_at,
            last_observed_at: occurred_at,
        }
    }

    fn add_observation(&mut self, observation: &ScopedTelemetryObservation) -> Result<(), ()> {
        self.accepted_observation_count =
            self.accepted_observation_count.checked_add(1).ok_or(())?;
        self.first_observed_at = self.first_observed_at.min(observation.occurred_at());
        self.last_observed_at = self.last_observed_at.max(observation.occurred_at());
        Ok(())
    }

    fn add_side_delta(&mut self, delta: CoverageSideDelta) -> Result<(), ()> {
        let was_empty = self.accepted_observation_count == 0
            && self.queue_full_drop_count == 0
            && self.closed_drop_count == 0
            && self.invalid_drop_count == 0
            && self.write_failed_observation_count == 0;
        let accepted_observation_count = self
            .accepted_observation_count
            .checked_add(delta.accepted_pending)
            .ok_or(())?;
        let queue_full_drop_count = self
            .queue_full_drop_count
            .checked_add(delta.queue_full_drop_count)
            .ok_or(())?;
        let closed_drop_count = self
            .closed_drop_count
            .checked_add(delta.closed_drop_count)
            .ok_or(())?;
        let invalid_drop_count = self
            .invalid_drop_count
            .checked_add(delta.invalid_drop_count)
            .ok_or(())?;
        self.accepted_observation_count = accepted_observation_count;
        self.queue_full_drop_count = queue_full_drop_count;
        self.closed_drop_count = closed_drop_count;
        self.invalid_drop_count = invalid_drop_count;
        if let Some(first) = delta.first_observed_at {
            self.first_observed_at = if was_empty {
                first
            } else {
                self.first_observed_at.min(first)
            };
        }
        if let Some(last) = delta.last_observed_at {
            self.last_observed_at = if was_empty {
                last
            } else {
                self.last_observed_at.max(last)
            };
        }
        Ok(())
    }

    fn add_invalid(&mut self, count: u64) -> Result<(), ()> {
        self.invalid_drop_count = self.invalid_drop_count.checked_add(count).ok_or(())?;
        Ok(())
    }

    fn to_record(
        &self,
        collector_instance_id: &CollectorInstanceId,
    ) -> Result<CollectorCoverage, crate::records::RecordError> {
        CollectorCoverage::new(
            self.tenant_id.clone(),
            self.window_start,
            collector_instance_id.clone(),
            self.accepted_observation_count,
            self.queue_full_drop_count,
            self.closed_drop_count,
            self.invalid_drop_count,
            self.write_failed_observation_count,
            self.first_observed_at,
            self.last_observed_at,
        )
    }
}

#[derive(Default)]
struct TenantDrainRows {
    representative_scope: Option<ResourceScope>,
    observations: Vec<ScopedTelemetryObservation>,
    activity: Vec<crate::HourlyUserActivity>,
    model_usage: Vec<crate::HourlyModelUsage>,
    run_failures: Vec<crate::HourlyRunFailure>,
    automation_usage: Vec<crate::HourlyAutomationUsage>,
    lifecycle_events: Vec<crate::LifecycleEvent>,
    collector_coverage: Vec<CollectorCoverage>,
}

struct TenantDrain {
    tenant_id: TenantId,
    scope: ResourceScope,
    observations: Vec<ScopedTelemetryObservation>,
    batch: TelemetryBatch,
}

fn partition_drain(
    batch: &TelemetryBatch,
    observations: &[ScopedTelemetryObservation],
) -> Result<Vec<TenantDrain>, crate::RecordError> {
    let mut grouped = BTreeMap::<TenantId, TenantDrainRows>::new();
    for observation in observations {
        let tenant_id = observation.scope().tenant_id.clone();
        let rows = grouped.entry(tenant_id).or_default();
        if rows.representative_scope.is_none() {
            rows.representative_scope = Some(observation.scope().clone());
        }
        rows.observations.push(observation.clone());
    }
    for row in batch.activity() {
        grouped
            .entry(row.tenant_id().clone())
            .or_default()
            .activity
            .push(row.clone());
    }
    for row in batch.model_usage() {
        grouped
            .entry(row.tenant_id().clone())
            .or_default()
            .model_usage
            .push(row.clone());
    }
    for row in batch.run_failures() {
        grouped
            .entry(row.tenant_id().clone())
            .or_default()
            .run_failures
            .push(row.clone());
    }
    for row in batch.automation_usage() {
        grouped
            .entry(row.tenant_id().clone())
            .or_default()
            .automation_usage
            .push(row.clone());
    }
    for row in batch.lifecycle_events() {
        grouped
            .entry(row.tenant_id().clone())
            .or_default()
            .lifecycle_events
            .push(row.clone());
    }
    for row in batch.collector_coverage() {
        grouped
            .entry(row.tenant_id().clone())
            .or_default()
            .collector_coverage
            .push(row.clone());
    }

    grouped
        .into_iter()
        .map(|(tenant_id, rows)| {
            let scope = rows.representative_scope.unwrap_or_else(|| ResourceScope {
                tenant_id: tenant_id.clone(),
                user_id: WorkerUserId::from_trusted("__telemetry_worker__".to_owned()),
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            });
            let batch = TelemetryBatch::new(
                rows.activity,
                rows.model_usage,
                rows.run_failures,
                rows.automation_usage,
                rows.lifecycle_events,
                rows.collector_coverage,
            )?;
            Ok(TenantDrain {
                tenant_id,
                scope,
                observations: rows.observations,
                batch,
            })
        })
        .collect()
}

fn batch_record_count(batch: &TelemetryBatch) -> usize {
    batch.record_count()
}

enum FirstWork {
    Observation(Box<ScopedTelemetryObservation>),
    CoverageNotice,
    Shutdown,
}

pub(crate) async fn run(
    config: WorkerConfig,
    mut receiver: mpsc::Receiver<ScopedTelemetryObservation>,
    intake: Arc<Intake>,
    repository: Arc<dyn TelemetryBatchSink>,
    clock: Arc<dyn TelemetryClock>,
    diagnostics: Arc<DiagnosticsState>,
    cancellation: CancellationToken,
) {
    let mut pending_coverage = BTreeMap::<(TenantId, DateTime<Utc>), CoverageAccumulator>::new();
    let mut shutting_down = false;
    loop {
        let first = receive_first(&mut receiver, &intake, &cancellation, shutting_down).await;
        let Some(first) = first else {
            break;
        };
        let FirstWork::Observation(first) = first else {
            if matches!(first, FirstWork::Shutdown) {
                shutting_down = true;
            }
            flush(
                &[],
                &mut pending_coverage,
                &intake,
                config.collector_instance_id.as_ref(),
                repository.as_ref(),
                clock.as_ref(),
                diagnostics.as_ref(),
            )
            .await;
            if shutting_down {
                break;
            }
            continue;
        };
        let mut observations = vec![*first];
        if !shutting_down {
            let deadline = tokio::time::sleep(config.max_wait);
            tokio::pin!(deadline);
            while observations.len() < config.max_batch_size {
                select! {
                    biased;
                    _ = cancellation.cancelled() => {
                        shutting_down = true;
                        break;
                    }
                    _ = &mut deadline => break,
                    next = receiver.recv() => match next {
                        Some(observation) => observations.push(observation),
                        None => {
                            shutting_down = true;
                            break;
                        }
                    },
                }
            }
        } else {
            while observations.len() < config.max_batch_size {
                match receiver.try_recv() {
                    Ok(observation) => observations.push(observation),
                    Err(
                        mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected,
                    ) => break,
                }
            }
        }

        flush(
            &observations,
            &mut pending_coverage,
            &intake,
            config.collector_instance_id.as_ref(),
            repository.as_ref(),
            clock.as_ref(),
            diagnostics.as_ref(),
        )
        .await;

        if cancellation.is_cancelled() {
            shutting_down = true;
        }
        if shutting_down && receiver.is_empty() {
            flush(
                &[],
                &mut pending_coverage,
                &intake,
                config.collector_instance_id.as_ref(),
                repository.as_ref(),
                clock.as_ref(),
                diagnostics.as_ref(),
            )
            .await;
            break;
        }
    }
}

async fn receive_first(
    receiver: &mut mpsc::Receiver<ScopedTelemetryObservation>,
    intake: &Intake,
    cancellation: &CancellationToken,
    shutting_down: bool,
) -> Option<FirstWork> {
    if shutting_down {
        return match receiver.try_recv() {
            Ok(observation) => Some(FirstWork::Observation(Box::new(observation))),
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => None,
        };
    }
    select! {
        biased;
        _ = cancellation.cancelled() => match receiver.try_recv() {
            Ok(observation) => Some(FirstWork::Observation(Box::new(observation))),
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) =>
                Some(FirstWork::Shutdown),
        },
        observation = receiver.recv() => observation.map(|observation| FirstWork::Observation(Box::new(observation))),
        _ = intake.notified() => Some(FirstWork::CoverageNotice),
    }
}

async fn flush(
    observations: &[ScopedTelemetryObservation],
    pending_coverage: &mut BTreeMap<(TenantId, DateTime<Utc>), CoverageAccumulator>,
    intake: &Intake,
    collector_instance_id: Option<&CollectorInstanceId>,
    repository: &dyn TelemetryBatchSink,
    clock: &dyn TelemetryClock,
    diagnostics: &DiagnosticsState,
) {
    let drop_deltas: Vec<_> = intake.take_drop_deltas().into_iter().collect();
    for (index, (key, delta)) in drop_deltas.iter().enumerate() {
        let Some(accumulator) = ensure_coverage_entry(key, pending_coverage, diagnostics) else {
            continue;
        };
        if accumulator.add_side_delta(*delta).is_err() {
            diagnostics.add_invalid(1);
            diagnostics.record_failure(crate::buffered_recorder::FailureClassCode::CounterOverflow);
            intake.restore_drop_deltas(
                drop_deltas[index..]
                    .iter()
                    .map(|(key, delta)| (key.clone(), *delta)),
                diagnostics,
            );
            account_observations(intake, observations, diagnostics);
            return;
        }
    }
    for observation in observations {
        let key = (
            observation.scope().tenant_id.clone(),
            floor_utc_hour(observation.occurred_at()),
        );
        if let Some(accumulator) = pending_coverage.get_mut(&key) {
            if accumulator.add_observation(observation).is_err() {
                diagnostics.add_invalid(1);
                diagnostics
                    .record_failure(crate::buffered_recorder::FailureClassCode::CounterOverflow);
                account_observations(intake, observations, diagnostics);
                return;
            }
        } else if pending_coverage.len() < MAX_COVERAGE_SIDE_KEYS {
            pending_coverage.insert(key, CoverageAccumulator::from_observation(observation));
        } else {
            diagnostics.record_coverage_key_overflow();
        }
    }

    let aggregate = aggregate_batch(observations);
    let mut batch = match aggregate {
        Ok(batch) => batch,
        Err(error) => {
            diagnostics.add_invalid(observations.len());
            diagnostics.record_failure(classify_aggregation_error(&error));
            for observation in observations {
                let key = (
                    observation.scope().tenant_id.clone(),
                    floor_utc_hour(observation.occurred_at()),
                );
                if let Some(accumulator) = pending_coverage.get_mut(&key)
                    && accumulator.add_invalid(1).is_err()
                {
                    diagnostics.record_failure(
                        crate::buffered_recorder::FailureClassCode::CounterOverflow,
                    );
                    return;
                }
            }
            account_observations(intake, observations, diagnostics);
            return;
        }
    };

    if let Some(collector_instance_id) = collector_instance_id {
        let mut coverage = Vec::with_capacity(pending_coverage.len());
        for accumulator in pending_coverage.values() {
            match accumulator.to_record(collector_instance_id) {
                Ok(row) => coverage.push(row),
                Err(error) => {
                    diagnostics.add_invalid(observations.len());
                    diagnostics.record_failure(classify_record_error(&error));
                    account_observations(intake, observations, diagnostics);
                    return;
                }
            }
        }
        batch = match TelemetryBatch::new(
            batch.activity().to_vec(),
            batch.model_usage().to_vec(),
            batch.run_failures().to_vec(),
            batch.automation_usage().to_vec(),
            batch.lifecycle_events().to_vec(),
            coverage,
        ) {
            Ok(batch) => batch,
            Err(error) => {
                diagnostics.add_invalid(observations.len());
                diagnostics.record_failure(classify_record_error(&error));
                account_observations(intake, observations, diagnostics);
                return;
            }
        };
    }

    if observations.is_empty() && pending_coverage.is_empty() {
        return;
    }

    let drains = match partition_drain(&batch, observations) {
        Ok(drains) => drains,
        Err(error) => {
            diagnostics.add_invalid(observations.len());
            diagnostics.record_failure(classify_record_error(&error));
            account_observations(intake, observations, diagnostics);
            return;
        }
    };
    if drains.is_empty() {
        account_observations(intake, observations, diagnostics);
        pending_coverage.clear();
        return;
    }

    let started = clock.now();
    let mut failed_tenants = Vec::new();
    for drain in &drains {
        let expected_records = batch_record_count(&drain.batch);
        let result = repository
            .apply_batch(ScopedTelemetryBatch::new(
                drain.scope.clone(),
                drain.batch.clone(),
            ))
            .await;
        match result {
            Ok(report) if report.is_complete_for(expected_records) => {}
            Ok(_) => {
                diagnostics.record_partial_batch_failure();
                failed_tenants.push(drain.tenant_id.clone());
                diagnostics.add_write_failed(drain.observations.len());
                replace_with_write_failure_coverage(
                    &drain.tenant_id,
                    &drain.observations,
                    pending_coverage,
                    diagnostics,
                );
            }
            Err(error) => {
                let class = classify_repository_error(&error);
                diagnostics.record_repository_failure(class);
                failed_tenants.push(drain.tenant_id.clone());
                diagnostics.add_write_failed(drain.observations.len());
                replace_with_write_failure_coverage(
                    &drain.tenant_id,
                    &drain.observations,
                    pending_coverage,
                    diagnostics,
                );
            }
        }
    }
    pending_coverage.retain(|(tenant_id, _), _| failed_tenants.contains(tenant_id));
    if !failed_tenants.is_empty() {
        account_observations(intake, observations, diagnostics);
        return;
    }
    let elapsed_ms = clock
        .now()
        .signed_duration_since(started)
        .num_milliseconds();
    diagnostics.record_flush(observations.len(), elapsed_ms.max(0) as u64);
    account_observations(intake, observations, diagnostics);
    pending_coverage.clear();
}

fn replace_with_write_failure_coverage(
    tenant_id: &TenantId,
    observations: &[ScopedTelemetryObservation],
    pending_coverage: &mut BTreeMap<(TenantId, DateTime<Utc>), CoverageAccumulator>,
    diagnostics: &DiagnosticsState,
) {
    // The repository may have applied any part of the batch before returning
    // the error. Never retry its additive counters; carry only one fresh loss
    // marker for each tenant/hour key touched by this attempted drain.
    let mut attempted_keys =
        BTreeMap::<(TenantId, DateTime<Utc>), (DateTime<Utc>, DateTime<Utc>, u64, u64)>::new();
    for ((pending_tenant_id, window_start), accumulator) in pending_coverage.iter() {
        if pending_tenant_id != tenant_id {
            continue;
        }
        attempted_keys.insert(
            (pending_tenant_id.clone(), *window_start),
            (
                accumulator.first_observed_at,
                accumulator.last_observed_at,
                accumulator.write_failed_observation_count,
                0,
            ),
        );
    }
    for observation in observations {
        if observation.scope().tenant_id != *tenant_id {
            continue;
        }
        let key = (tenant_id.clone(), floor_utc_hour(observation.occurred_at()));
        let occurred_at = observation.occurred_at();
        if let Some((first_observed_at, last_observed_at, _, current_count)) =
            attempted_keys.get_mut(&key)
        {
            *first_observed_at = (*first_observed_at).min(occurred_at);
            *last_observed_at = (*last_observed_at).max(occurred_at);
            *current_count = current_count.saturating_add(1);
        } else if attempted_keys.len() < MAX_COVERAGE_SIDE_KEYS {
            attempted_keys.insert(key, (occurred_at, occurred_at, 0, 1));
        } else {
            diagnostics.record_coverage_key_overflow();
        }
    }

    pending_coverage.retain(|(pending_tenant_id, _), _| pending_tenant_id != tenant_id);
    for (
        (tenant_id, window_start),
        (first_observed_at, last_observed_at, prior_count, current_count),
    ) in attempted_keys
    {
        if pending_coverage.len() >= MAX_COVERAGE_SIDE_KEYS {
            diagnostics.record_coverage_key_overflow();
            break;
        }
        pending_coverage.insert(
            (tenant_id.clone(), window_start),
            CoverageAccumulator {
                tenant_id,
                window_start,
                accepted_observation_count: 0,
                queue_full_drop_count: 0,
                closed_drop_count: 0,
                invalid_drop_count: 0,
                write_failed_observation_count: if current_count == 0 {
                    prior_count
                } else {
                    current_count
                },
                first_observed_at,
                last_observed_at,
            },
        );
    }
}

fn observation_keys(observations: &[ScopedTelemetryObservation]) -> Vec<TenantHourKey> {
    observations
        .iter()
        .map(|observation| TenantHourKey {
            tenant_id: observation.scope().tenant_id.clone(),
            window_start: floor_utc_hour(observation.occurred_at()),
        })
        .collect()
}

fn ensure_coverage_entry<'a>(
    key: &TenantHourKey,
    pending_coverage: &'a mut BTreeMap<(TenantId, DateTime<Utc>), CoverageAccumulator>,
    diagnostics: &DiagnosticsState,
) -> Option<&'a mut CoverageAccumulator> {
    let pending_key = (key.tenant_id.clone(), key.window_start);
    if !pending_coverage.contains_key(&pending_key) {
        if pending_coverage.len() >= MAX_COVERAGE_SIDE_KEYS {
            diagnostics.record_coverage_key_overflow();
            return None;
        }
        pending_coverage.insert(pending_key.clone(), CoverageAccumulator::from_key(key));
    }
    pending_coverage.get_mut(&pending_key)
}

fn account_observations(
    intake: &Intake,
    observations: &[ScopedTelemetryObservation],
    diagnostics: &DiagnosticsState,
) {
    if let Err(error) = intake.account_observations(observation_keys(observations)) {
        diagnostics.record_failure(error.failure_class());
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use ironclaw_telemetry_contracts::observation::{
        ObservationContext, OriginKind, RunOutcome, RunSettledObservation, TelemetryObservation,
    };
    use ironclaw_telemetry_contracts::recorder::RecordOutcome;

    use super::*;
    use crate::SystemTelemetryClock;

    struct NeverSink;

    #[async_trait::async_trait]
    impl TelemetryBatchSink for NeverSink {
        async fn apply_batch(
            &self,
            _batch: ScopedTelemetryBatch,
        ) -> Result<crate::BatchApplyReport, crate::TelemetryRepositoryError> {
            panic!("overflow path must return before writing");
        }
    }

    #[test]
    fn coverage_accumulator_rejects_counter_overflow_without_mutation() {
        let window_start = Utc
            .with_ymd_and_hms(2025, 8, 26, 9, 0, 0)
            .single()
            .expect("valid test timestamp");
        let key = TenantHourKey {
            tenant_id: TenantId::new("tenant-a").expect("valid tenant"),
            window_start,
        };
        let mut accumulator = CoverageAccumulator::from_key(&key);
        accumulator.accepted_observation_count = u64::MAX;

        let delta = CoverageSideDelta {
            accepted_pending: 1,
            ..CoverageSideDelta::default()
        };
        assert!(accumulator.add_side_delta(delta).is_err());
        assert_eq!(accumulator.accepted_observation_count, u64::MAX);
    }

    #[tokio::test]
    async fn drop_delta_overflow_restores_remaining_deltas_and_accounts_observations() {
        let (sender, _receiver) = mpsc::channel(1);
        let intake = Intake::new(sender);
        let diagnostics = DiagnosticsState::default();
        let tenant_id = TenantId::new("tenant-a").expect("valid tenant");
        let user_id = WorkerUserId::new("user-a").expect("valid user");
        let scope = ResourceScope {
            tenant_id: tenant_id.clone(),
            user_id,
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        let occurred_at = Utc
            .timestamp_opt(1_756_200_000, 0)
            .single()
            .expect("valid timestamp");
        let observation = ScopedTelemetryObservation::new(
            scope,
            TelemetryObservation::RunSettled(
                RunSettledObservation::new(
                    ObservationContext::new(occurred_at),
                    OriginKind::Human,
                    RunOutcome::Completed,
                    1,
                    Some(0),
                    None,
                )
                .expect("valid observation"),
            ),
        );
        let key = TenantHourKey {
            tenant_id: tenant_id.clone(),
            window_start: floor_utc_hour(occurred_at),
        };
        assert_eq!(
            intake.try_record(observation.clone(), key.clone(), &diagnostics, Ok(())),
            RecordOutcome::Accepted
        );
        assert_eq!(
            intake.try_record(observation.clone(), key.clone(), &diagnostics, Ok(())),
            RecordOutcome::DroppedQueueFull
        );
        intake.notified().await;

        let mut pending_coverage = BTreeMap::new();
        let mut accumulator = CoverageAccumulator::from_observation(&observation);
        accumulator.queue_full_drop_count = u64::MAX;
        pending_coverage.insert((tenant_id, key.window_start), accumulator);
        let clock = SystemTelemetryClock;

        flush(
            &[observation],
            &mut pending_coverage,
            &intake,
            None,
            &NeverSink,
            &clock,
            &diagnostics,
        )
        .await;

        tokio::time::timeout(Duration::from_millis(10), intake.notified())
            .await
            .expect("restoring drop deltas must wake the idle worker");

        let (_, pending_observation_count) = intake.take_unpersisted();
        assert_eq!(pending_observation_count, 0);
        let restored = intake.take_drop_deltas();
        assert_eq!(restored[&key].queue_full_drop_count, 1);
    }
}
