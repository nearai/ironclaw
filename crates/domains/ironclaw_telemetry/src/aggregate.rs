//! Pure UTC bucketing and order-independent observation aggregation.

use std::{borrow::Borrow, collections::BTreeMap};

use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};
use ironclaw_host_api::ids::{TenantId, UserId};
use ironclaw_telemetry_contracts::observation::{
    FailureCategory, LifecycleEventId, MAX_DURABLE_COUNTER, RunOutcome, ScopedTelemetryObservation,
    TelemetryObservation,
};

use crate::records::{
    HourlyAutomationUsage, HourlyModelUsage, HourlyRunFailure, HourlyUserActivity, LifecycleEvent,
    RecordError, TelemetryBatch,
};

#[derive(Debug, thiserror::Error)]
pub enum AggregationError {
    #[error("counter overflow while aggregating {field}")]
    CounterOverflow { field: &'static str },
    #[error("{field} value {value} exceeds signed BIGINT range")]
    CounterOutOfRange { field: &'static str, value: u64 },
    #[error(transparent)]
    InvalidRecord(#[from] RecordError),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ActivityKey(
    TenantId,
    DateTime<Utc>,
    UserId,
    ironclaw_telemetry_contracts::observation::OriginKind,
);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ModelKey(
    TenantId,
    UserId,
    DateTime<Utc>,
    ironclaw_telemetry_contracts::observation::ProviderId,
    ironclaw_telemetry_contracts::observation::EffectiveModelId,
);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FailureKey(TenantId, DateTime<Utc>, UserId, FailureCategory);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct AutomationKey(
    TenantId,
    DateTime<Utc>,
    UserId,
    ironclaw_telemetry_contracts::observation::AutomationKind,
);

#[derive(Debug)]
struct ActivityAccumulator {
    tenant_id: TenantId,
    window_start: DateTime<Utc>,
    user_id: UserId,
    origin_kind: ironclaw_telemetry_contracts::observation::OriginKind,
    run_count: u64,
    runs_with_reported_tool_calls_count: u64,
    tool_count_reported_run_count: u64,
    reported_tool_call_count: u64,
    completed_count: u64,
    failed_count: u64,
    cancelled_count: u64,
    recovery_required_count: u64,
    total_run_latency_ms: u64,
    first_observed_at: DateTime<Utc>,
    last_observed_at: DateTime<Utc>,
}

#[derive(Debug)]
struct ModelAccumulator {
    tenant_id: TenantId,
    user_id: UserId,
    window_start: DateTime<Utc>,
    provider_id: ironclaw_telemetry_contracts::observation::ProviderId,
    effective_model_id: ironclaw_telemetry_contracts::observation::EffectiveModelId,
    inference_count: u64,
    usage_reported_count: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_input_tokens: u64,
    cache_creation_input_tokens: u64,
    first_observed_at: DateTime<Utc>,
    last_observed_at: DateTime<Utc>,
}

#[derive(Debug)]
struct FailureAccumulator {
    tenant_id: TenantId,
    window_start: DateTime<Utc>,
    user_id: UserId,
    failure_category: FailureCategory,
    failure_count: u64,
    first_observed_at: DateTime<Utc>,
    last_observed_at: DateTime<Utc>,
}

#[derive(Debug)]
struct AutomationAccumulator {
    tenant_id: TenantId,
    window_start: DateTime<Utc>,
    user_id: UserId,
    automation_kind: ironclaw_telemetry_contracts::observation::AutomationKind,
    run_count: u64,
    completed_count: u64,
    failed_count: u64,
    cancelled_count: u64,
    recovery_required_count: u64,
    first_observed_at: DateTime<Utc>,
    last_observed_at: DateTime<Utc>,
}

fn checked_add(
    destination: &mut u64,
    amount: u64,
    field: &'static str,
) -> Result<(), AggregationError> {
    if *destination > MAX_DURABLE_COUNTER {
        return Err(AggregationError::CounterOutOfRange {
            field,
            value: *destination,
        });
    }
    if amount > MAX_DURABLE_COUNTER {
        return Err(AggregationError::CounterOutOfRange {
            field,
            value: amount,
        });
    }
    let value = destination
        .checked_add(amount)
        .ok_or(AggregationError::CounterOverflow { field })?;
    if value > MAX_DURABLE_COUNTER {
        return Err(AggregationError::CounterOutOfRange { field, value });
    }
    *destination = value;
    Ok(())
}

fn checked_value(value: u64, field: &'static str) -> Result<(), AggregationError> {
    if value > MAX_DURABLE_COUNTER {
        return Err(AggregationError::CounterOutOfRange { field, value });
    }
    Ok(())
}

fn update_range(
    first_observed_at: &mut DateTime<Utc>,
    last_observed_at: &mut DateTime<Utc>,
    occurred_at: DateTime<Utc>,
) {
    if occurred_at < *first_observed_at {
        *first_observed_at = occurred_at;
    }
    if occurred_at > *last_observed_at {
        *last_observed_at = occurred_at;
    }
}

fn add_terminal_counter(
    outcome: RunOutcome,
    completed_count: &mut u64,
    failed_count: &mut u64,
    cancelled_count: &mut u64,
    recovery_required_count: &mut u64,
) -> Result<(), AggregationError> {
    match outcome {
        RunOutcome::Completed => checked_add(completed_count, 1, "completed_count"),
        RunOutcome::Failed => checked_add(failed_count, 1, "failed_count"),
        RunOutcome::Cancelled => checked_add(cancelled_count, 1, "cancelled_count"),
        RunOutcome::RecoveryRequired => {
            checked_add(recovery_required_count, 1, "recovery_required_count")
        }
    }
}

fn accumulate_failure(
    run_failures: &mut BTreeMap<FailureKey, FailureAccumulator>,
    tenant_id: &TenantId,
    window_start: DateTime<Utc>,
    user_id: &UserId,
    failure_category: &FailureCategory,
    occurred_at: DateTime<Utc>,
) -> Result<(), AggregationError> {
    let key = FailureKey(
        tenant_id.clone(),
        window_start,
        user_id.clone(),
        failure_category.clone(),
    );
    if let Some(accumulator) = run_failures.get_mut(&key) {
        checked_add(&mut accumulator.failure_count, 1, "failure_count")?;
        update_range(
            &mut accumulator.first_observed_at,
            &mut accumulator.last_observed_at,
            occurred_at,
        );
    } else {
        run_failures.insert(
            key,
            FailureAccumulator {
                tenant_id: tenant_id.clone(),
                window_start,
                user_id: user_id.clone(),
                failure_category: failure_category.clone(),
                failure_count: 1,
                first_observed_at: occurred_at,
                last_observed_at: occurred_at,
            },
        );
    }
    Ok(())
}

/// Return the exact UTC start of the hour containing timestamp.
pub fn floor_utc_hour(timestamp: DateTime<Utc>) -> DateTime<Utc> {
    match Utc
        .with_ymd_and_hms(
            timestamp.year(),
            timestamp.month(),
            timestamp.day(),
            timestamp.hour(),
            0,
            0,
        )
        .single()
    {
        Some(floored) => floored,
        None => timestamp,
    }
}

/// Aggregate observations into deterministic, hourly records.
///
/// Borrow permits callers to pass either an owned iterator or a borrowed slice
/// without cloning observations. The maps and record vectors are ordered by
/// their durable keys, so input order cannot affect the result.
pub fn aggregate_batch<I, B>(observations: I) -> Result<TelemetryBatch, AggregationError>
where
    I: IntoIterator<Item = B>,
    B: Borrow<ScopedTelemetryObservation>,
{
    let mut activity = BTreeMap::<ActivityKey, ActivityAccumulator>::new();
    let mut model_usage = BTreeMap::<ModelKey, ModelAccumulator>::new();
    let mut run_failures = BTreeMap::<FailureKey, FailureAccumulator>::new();
    let mut automation_usage = BTreeMap::<AutomationKey, AutomationAccumulator>::new();
    let mut lifecycle_events = BTreeMap::<(TenantId, LifecycleEventId), LifecycleEvent>::new();

    for scoped_observation in observations {
        let scoped_observation = scoped_observation.borrow();
        let scope = scoped_observation.scope();
        match scoped_observation.observation() {
            TelemetryObservation::RunSettled(observation) => {
                checked_value(observation.duration_ms(), "total_run_latency_ms")?;
                if let Some(count) = observation.reported_tool_call_count() {
                    checked_value(count, "reported_tool_call_count")?;
                }
                let window_start = floor_utc_hour(observation.occurred_at());
                let key = ActivityKey(
                    scope.tenant_id.clone(),
                    window_start,
                    scope.user_id.clone(),
                    observation.origin(),
                );
                let window_start = key.1;
                if let Some(failure) = observation.failure() {
                    accumulate_failure(
                        &mut run_failures,
                        &scope.tenant_id,
                        window_start,
                        &scope.user_id,
                        failure,
                        observation.occurred_at(),
                    )?;
                }
                if let Some(accumulator) = activity.get_mut(&key) {
                    checked_add(&mut accumulator.run_count, 1, "run_count")?;
                    add_terminal_counter(
                        observation.outcome(),
                        &mut accumulator.completed_count,
                        &mut accumulator.failed_count,
                        &mut accumulator.cancelled_count,
                        &mut accumulator.recovery_required_count,
                    )?;
                    checked_add(
                        &mut accumulator.total_run_latency_ms,
                        observation.duration_ms(),
                        "total_run_latency_ms",
                    )?;
                    if let Some(count) = observation.reported_tool_call_count() {
                        checked_add(
                            &mut accumulator.tool_count_reported_run_count,
                            1,
                            "tool_count_reported_run_count",
                        )?;
                        checked_add(
                            &mut accumulator.reported_tool_call_count,
                            count,
                            "reported_tool_call_count",
                        )?;
                        if count > 0 {
                            checked_add(
                                &mut accumulator.runs_with_reported_tool_calls_count,
                                1,
                                "runs_with_reported_tool_calls_count",
                            )?;
                        }
                    }
                    update_range(
                        &mut accumulator.first_observed_at,
                        &mut accumulator.last_observed_at,
                        observation.occurred_at(),
                    );
                } else {
                    let mut accumulator = ActivityAccumulator {
                        tenant_id: scope.tenant_id.clone(),
                        window_start,
                        user_id: scope.user_id.clone(),
                        origin_kind: observation.origin(),
                        run_count: 1,
                        runs_with_reported_tool_calls_count: 0,
                        tool_count_reported_run_count: 0,
                        reported_tool_call_count: 0,
                        completed_count: 0,
                        failed_count: 0,
                        cancelled_count: 0,
                        recovery_required_count: 0,
                        total_run_latency_ms: observation.duration_ms(),
                        first_observed_at: observation.occurred_at(),
                        last_observed_at: observation.occurred_at(),
                    };
                    add_terminal_counter(
                        observation.outcome(),
                        &mut accumulator.completed_count,
                        &mut accumulator.failed_count,
                        &mut accumulator.cancelled_count,
                        &mut accumulator.recovery_required_count,
                    )?;
                    if let Some(count) = observation.reported_tool_call_count() {
                        accumulator.tool_count_reported_run_count = 1;
                        accumulator.reported_tool_call_count = count;
                        accumulator.runs_with_reported_tool_calls_count = u64::from(count > 0);
                    }
                    activity.insert(key, accumulator);
                }
            }
            TelemetryObservation::ModelCallCompleted(observation) => {
                checked_value(observation.input_tokens(), "input_tokens")?;
                checked_value(observation.output_tokens(), "output_tokens")?;
                checked_value(
                    observation.cache_read_input_tokens(),
                    "cache_read_input_tokens",
                )?;
                checked_value(
                    observation.cache_creation_input_tokens(),
                    "cache_creation_input_tokens",
                )?;
                let window_start = floor_utc_hour(observation.occurred_at());
                let key = ModelKey(
                    scope.tenant_id.clone(),
                    scope.user_id.clone(),
                    window_start,
                    observation.provider_id().clone(),
                    observation.effective_model_id().clone(),
                );
                if let Some(accumulator) = model_usage.get_mut(&key) {
                    checked_add(&mut accumulator.inference_count, 1, "inference_count")?;
                    if observation.usage_reported() {
                        checked_add(
                            &mut accumulator.usage_reported_count,
                            1,
                            "usage_reported_count",
                        )?;
                    }
                    checked_add(
                        &mut accumulator.input_tokens,
                        observation.input_tokens(),
                        "input_tokens",
                    )?;
                    checked_add(
                        &mut accumulator.output_tokens,
                        observation.output_tokens(),
                        "output_tokens",
                    )?;
                    checked_add(
                        &mut accumulator.cache_read_input_tokens,
                        observation.cache_read_input_tokens(),
                        "cache_read_input_tokens",
                    )?;
                    checked_add(
                        &mut accumulator.cache_creation_input_tokens,
                        observation.cache_creation_input_tokens(),
                        "cache_creation_input_tokens",
                    )?;
                    update_range(
                        &mut accumulator.first_observed_at,
                        &mut accumulator.last_observed_at,
                        observation.occurred_at(),
                    );
                } else {
                    model_usage.insert(
                        key,
                        ModelAccumulator {
                            tenant_id: scope.tenant_id.clone(),
                            user_id: scope.user_id.clone(),
                            window_start,
                            provider_id: observation.provider_id().clone(),
                            effective_model_id: observation.effective_model_id().clone(),
                            inference_count: 1,
                            usage_reported_count: u64::from(observation.usage_reported()),
                            input_tokens: observation.input_tokens(),
                            output_tokens: observation.output_tokens(),
                            cache_read_input_tokens: observation.cache_read_input_tokens(),
                            cache_creation_input_tokens: observation.cache_creation_input_tokens(),
                            first_observed_at: observation.occurred_at(),
                            last_observed_at: observation.occurred_at(),
                        },
                    );
                }
            }
            TelemetryObservation::AutomationSettled(observation) => {
                let window_start = floor_utc_hour(observation.occurred_at());
                let key = AutomationKey(
                    scope.tenant_id.clone(),
                    window_start,
                    scope.user_id.clone(),
                    observation.automation_kind(),
                );
                if let Some(accumulator) = automation_usage.get_mut(&key) {
                    checked_add(&mut accumulator.run_count, 1, "run_count")?;
                    add_terminal_counter(
                        observation.outcome(),
                        &mut accumulator.completed_count,
                        &mut accumulator.failed_count,
                        &mut accumulator.cancelled_count,
                        &mut accumulator.recovery_required_count,
                    )?;
                    update_range(
                        &mut accumulator.first_observed_at,
                        &mut accumulator.last_observed_at,
                        observation.occurred_at(),
                    );
                } else {
                    let mut accumulator = AutomationAccumulator {
                        tenant_id: scope.tenant_id.clone(),
                        window_start,
                        user_id: scope.user_id.clone(),
                        automation_kind: observation.automation_kind(),
                        run_count: 1,
                        completed_count: 0,
                        failed_count: 0,
                        cancelled_count: 0,
                        recovery_required_count: 0,
                        first_observed_at: observation.occurred_at(),
                        last_observed_at: observation.occurred_at(),
                    };
                    add_terminal_counter(
                        observation.outcome(),
                        &mut accumulator.completed_count,
                        &mut accumulator.failed_count,
                        &mut accumulator.cancelled_count,
                        &mut accumulator.recovery_required_count,
                    )?;
                    automation_usage.insert(key, accumulator);
                }
            }
            TelemetryObservation::LifecycleTransition(observation) => {
                let key = (scope.tenant_id.clone(), observation.event_id().clone());
                let candidate = LifecycleEvent::new(
                    scope.tenant_id.clone(),
                    observation.event_id().clone(),
                    observation.subject_user_id().cloned(),
                    observation.event_kind(),
                    observation.subject_kind(),
                    observation.subject_id().clone(),
                    observation.occurred_at(),
                )?;
                if let Some(existing) = lifecycle_events.get(&key) {
                    if candidate < *existing {
                        lifecycle_events.insert(key, candidate);
                    }
                } else {
                    lifecycle_events.insert(key, candidate);
                }
            }
        }
    }

    let activity = activity
        .into_values()
        .map(|accumulator| {
            HourlyUserActivity::new(
                accumulator.tenant_id,
                accumulator.window_start,
                accumulator.user_id,
                accumulator.origin_kind,
                accumulator.run_count,
                accumulator.runs_with_reported_tool_calls_count,
                accumulator.tool_count_reported_run_count,
                accumulator.reported_tool_call_count,
                accumulator.completed_count,
                accumulator.failed_count,
                accumulator.cancelled_count,
                accumulator.recovery_required_count,
                accumulator.total_run_latency_ms,
                accumulator.first_observed_at,
                accumulator.last_observed_at,
            )
            .map_err(AggregationError::InvalidRecord)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let model_usage = model_usage
        .into_values()
        .map(|accumulator| {
            HourlyModelUsage::new(
                accumulator.tenant_id,
                accumulator.user_id,
                accumulator.window_start,
                accumulator.provider_id,
                accumulator.effective_model_id,
                accumulator.inference_count,
                accumulator.usage_reported_count,
                accumulator.input_tokens,
                accumulator.output_tokens,
                accumulator.cache_read_input_tokens,
                accumulator.cache_creation_input_tokens,
                accumulator.first_observed_at,
                accumulator.last_observed_at,
            )
            .map_err(AggregationError::InvalidRecord)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let run_failures = run_failures
        .into_values()
        .map(|accumulator| {
            HourlyRunFailure::new(
                accumulator.tenant_id,
                accumulator.window_start,
                accumulator.user_id,
                accumulator.failure_category,
                accumulator.failure_count,
                accumulator.first_observed_at,
                accumulator.last_observed_at,
            )
            .map_err(AggregationError::InvalidRecord)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let automation_usage = automation_usage
        .into_values()
        .map(|accumulator| {
            HourlyAutomationUsage::new(
                accumulator.tenant_id,
                accumulator.window_start,
                accumulator.user_id,
                accumulator.automation_kind,
                accumulator.run_count,
                accumulator.completed_count,
                accumulator.failed_count,
                accumulator.cancelled_count,
                accumulator.recovery_required_count,
                accumulator.first_observed_at,
                accumulator.last_observed_at,
            )
            .map_err(AggregationError::InvalidRecord)
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(TelemetryBatch::new(
        activity,
        model_usage,
        run_failures,
        automation_usage,
        lifecycle_events.into_values().collect(),
        Vec::new(),
    )?)
}
