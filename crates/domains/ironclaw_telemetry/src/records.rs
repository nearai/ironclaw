//! Typed hourly records produced by telemetry aggregation.

use std::collections::BTreeSet;

use chrono::{DateTime, Timelike, Utc};
use ironclaw_host_api::ids::{TenantId, UserId};
use ironclaw_telemetry_contracts::observation::{
    AutomationKind, CollectorInstanceId, EffectiveModelId, LifecycleEventId, LifecycleEventKind,
    LifecycleSubjectKind, MAX_DURABLE_COUNTER, OriginKind, ProviderId, SubjectId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryBatchRowFamily {
    Activity,
    ModelUsage,
    RunFailure,
    AutomationUsage,
    LifecycleEvent,
    CollectorCoverage,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecordError {
    #[error("hourly record window_start must be an exact UTC hour")]
    InvalidWindowStart,
    #[error("record first_observed_at must not be after last_observed_at")]
    InvalidObservationRange,
    #[error("hourly activity terminal counts overflowed while being checked")]
    TerminalCountOverflow,
    #[error("hourly activity terminal counts must equal run_count")]
    TerminalCountMismatch,
    #[error("reported tool-call count must not exceed run_count")]
    ReportedToolCountExceedsRuns,
    #[error("reported usage count must not exceed inference_count")]
    ReportedUsageExceedsInferences,
    #[error("{field} value {value} exceeds signed BIGINT range")]
    CounterOutOfRange { field: &'static str, value: u64 },
    #[error("duplicate row in telemetry batch: {family:?}")]
    DuplicateRow { family: TelemetryBatchRowFamily },
    #[error("non-tenant lifecycle events require user attribution")]
    MissingUserAttribution,
}

fn validate_counter(value: u64, field: &'static str) -> Result<(), RecordError> {
    if value > MAX_DURABLE_COUNTER {
        return Err(RecordError::CounterOutOfRange { field, value });
    }
    Ok(())
}

fn validate_unique_rows<K>(
    keys: impl IntoIterator<Item = K>,
    family: TelemetryBatchRowFamily,
) -> Result<(), RecordError>
where
    K: Ord,
{
    let mut seen = BTreeSet::new();
    for key in keys {
        if !seen.insert(key) {
            return Err(RecordError::DuplicateRow { family });
        }
    }
    Ok(())
}

fn validate_hour(window_start: DateTime<Utc>) -> Result<(), RecordError> {
    if window_start.minute() != 0 || window_start.second() != 0 || window_start.nanosecond() != 0 {
        return Err(RecordError::InvalidWindowStart);
    }
    Ok(())
}

fn validate_range(
    first_observed_at: DateTime<Utc>,
    last_observed_at: DateTime<Utc>,
) -> Result<(), RecordError> {
    if first_observed_at > last_observed_at {
        return Err(RecordError::InvalidObservationRange);
    }
    Ok(())
}

fn validate_terminal_counts(
    run_count: u64,
    completed_count: u64,
    failed_count: u64,
    cancelled_count: u64,
    recovery_required_count: u64,
) -> Result<(), RecordError> {
    let terminal_count = completed_count
        .checked_add(failed_count)
        .and_then(|count| count.checked_add(cancelled_count))
        .and_then(|count| count.checked_add(recovery_required_count))
        .ok_or(RecordError::TerminalCountOverflow)?;
    validate_counter(terminal_count, "terminal_count")?;
    if terminal_count != run_count {
        return Err(RecordError::TerminalCountMismatch);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HourlyUserActivity {
    tenant_id: TenantId,
    window_start: DateTime<Utc>,
    user_id: UserId,
    origin_kind: OriginKind,
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

impl HourlyUserActivity {
    // arch-exempt: too_many_args, record constructor preserves explicit hourly activity fields, plan #7961
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: TenantId,
        window_start: DateTime<Utc>,
        user_id: UserId,
        origin_kind: OriginKind,
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
    ) -> Result<Self, RecordError> {
        validate_hour(window_start)?;
        validate_range(first_observed_at, last_observed_at)?;
        for (field, value) in [
            ("run_count", run_count),
            (
                "runs_with_reported_tool_calls_count",
                runs_with_reported_tool_calls_count,
            ),
            (
                "tool_count_reported_run_count",
                tool_count_reported_run_count,
            ),
            ("reported_tool_call_count", reported_tool_call_count),
            ("completed_count", completed_count),
            ("failed_count", failed_count),
            ("cancelled_count", cancelled_count),
            ("recovery_required_count", recovery_required_count),
            ("total_run_latency_ms", total_run_latency_ms),
        ] {
            validate_counter(value, field)?;
        }
        validate_terminal_counts(
            run_count,
            completed_count,
            failed_count,
            cancelled_count,
            recovery_required_count,
        )?;
        if runs_with_reported_tool_calls_count > tool_count_reported_run_count
            || tool_count_reported_run_count > run_count
        {
            return Err(RecordError::ReportedToolCountExceedsRuns);
        }
        Ok(Self {
            tenant_id,
            window_start,
            user_id,
            origin_kind,
            run_count,
            runs_with_reported_tool_calls_count,
            tool_count_reported_run_count,
            reported_tool_call_count,
            completed_count,
            failed_count,
            cancelled_count,
            recovery_required_count,
            total_run_latency_ms,
            first_observed_at,
            last_observed_at,
        })
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub const fn window_start(&self) -> DateTime<Utc> {
        self.window_start
    }

    pub fn user_id(&self) -> &UserId {
        &self.user_id
    }

    pub const fn origin_kind(&self) -> OriginKind {
        self.origin_kind
    }

    pub const fn run_count(&self) -> u64 {
        self.run_count
    }

    pub const fn runs_with_reported_tool_calls_count(&self) -> u64 {
        self.runs_with_reported_tool_calls_count
    }

    pub const fn tool_count_reported_run_count(&self) -> u64 {
        self.tool_count_reported_run_count
    }

    pub const fn reported_tool_call_count(&self) -> u64 {
        self.reported_tool_call_count
    }

    pub const fn completed_count(&self) -> u64 {
        self.completed_count
    }

    pub const fn failed_count(&self) -> u64 {
        self.failed_count
    }

    pub const fn cancelled_count(&self) -> u64 {
        self.cancelled_count
    }

    pub const fn recovery_required_count(&self) -> u64 {
        self.recovery_required_count
    }

    pub const fn total_run_latency_ms(&self) -> u64 {
        self.total_run_latency_ms
    }

    pub const fn first_observed_at(&self) -> DateTime<Utc> {
        self.first_observed_at
    }

    pub const fn last_observed_at(&self) -> DateTime<Utc> {
        self.last_observed_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HourlyModelUsage {
    tenant_id: TenantId,
    user_id: UserId,
    window_start: DateTime<Utc>,
    provider_id: ProviderId,
    effective_model_id: EffectiveModelId,
    inference_count: u64,
    usage_reported_count: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_input_tokens: u64,
    cache_creation_input_tokens: u64,
    first_observed_at: DateTime<Utc>,
    last_observed_at: DateTime<Utc>,
}

impl HourlyModelUsage {
    // arch-exempt: too_many_args, record constructor preserves explicit model usage fields, plan #7961
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: TenantId,
        user_id: UserId,
        window_start: DateTime<Utc>,
        provider_id: ProviderId,
        effective_model_id: EffectiveModelId,
        inference_count: u64,
        usage_reported_count: u64,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_input_tokens: u64,
        cache_creation_input_tokens: u64,
        first_observed_at: DateTime<Utc>,
        last_observed_at: DateTime<Utc>,
    ) -> Result<Self, RecordError> {
        validate_hour(window_start)?;
        validate_range(first_observed_at, last_observed_at)?;
        for (field, value) in [
            ("inference_count", inference_count),
            ("usage_reported_count", usage_reported_count),
            ("input_tokens", input_tokens),
            ("output_tokens", output_tokens),
            ("cache_read_input_tokens", cache_read_input_tokens),
            ("cache_creation_input_tokens", cache_creation_input_tokens),
        ] {
            validate_counter(value, field)?;
        }
        if usage_reported_count > inference_count {
            return Err(RecordError::ReportedUsageExceedsInferences);
        }
        Ok(Self {
            tenant_id,
            user_id,
            window_start,
            provider_id,
            effective_model_id,
            inference_count,
            usage_reported_count,
            input_tokens,
            output_tokens,
            cache_read_input_tokens,
            cache_creation_input_tokens,
            first_observed_at,
            last_observed_at,
        })
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn user_id(&self) -> &UserId {
        &self.user_id
    }

    pub const fn window_start(&self) -> DateTime<Utc> {
        self.window_start
    }

    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    pub fn effective_model_id(&self) -> &EffectiveModelId {
        &self.effective_model_id
    }

    pub const fn inference_count(&self) -> u64 {
        self.inference_count
    }

    pub const fn usage_reported_count(&self) -> u64 {
        self.usage_reported_count
    }

    pub const fn input_tokens(&self) -> u64 {
        self.input_tokens
    }

    pub const fn output_tokens(&self) -> u64 {
        self.output_tokens
    }

    pub const fn cache_read_input_tokens(&self) -> u64 {
        self.cache_read_input_tokens
    }

    pub const fn cache_creation_input_tokens(&self) -> u64 {
        self.cache_creation_input_tokens
    }

    pub const fn first_observed_at(&self) -> DateTime<Utc> {
        self.first_observed_at
    }

    pub const fn last_observed_at(&self) -> DateTime<Utc> {
        self.last_observed_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HourlyRunFailure {
    tenant_id: TenantId,
    window_start: DateTime<Utc>,
    user_id: UserId,
    failure_category: ironclaw_telemetry_contracts::observation::FailureCategory,
    failure_count: u64,
    first_observed_at: DateTime<Utc>,
    last_observed_at: DateTime<Utc>,
}

impl HourlyRunFailure {
    pub fn new(
        tenant_id: TenantId,
        window_start: DateTime<Utc>,
        user_id: UserId,
        failure_category: ironclaw_telemetry_contracts::observation::FailureCategory,
        failure_count: u64,
        first_observed_at: DateTime<Utc>,
        last_observed_at: DateTime<Utc>,
    ) -> Result<Self, RecordError> {
        validate_hour(window_start)?;
        validate_range(first_observed_at, last_observed_at)?;
        validate_counter(failure_count, "failure_count")?;
        Ok(Self {
            tenant_id,
            window_start,
            user_id,
            failure_category,
            failure_count,
            first_observed_at,
            last_observed_at,
        })
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub const fn window_start(&self) -> DateTime<Utc> {
        self.window_start
    }

    pub fn user_id(&self) -> &UserId {
        &self.user_id
    }

    pub fn failure_category(&self) -> &ironclaw_telemetry_contracts::observation::FailureCategory {
        &self.failure_category
    }

    pub const fn failure_count(&self) -> u64 {
        self.failure_count
    }

    pub const fn first_observed_at(&self) -> DateTime<Utc> {
        self.first_observed_at
    }

    pub const fn last_observed_at(&self) -> DateTime<Utc> {
        self.last_observed_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HourlyAutomationUsage {
    tenant_id: TenantId,
    window_start: DateTime<Utc>,
    user_id: UserId,
    automation_kind: AutomationKind,
    run_count: u64,
    completed_count: u64,
    failed_count: u64,
    cancelled_count: u64,
    recovery_required_count: u64,
    first_observed_at: DateTime<Utc>,
    last_observed_at: DateTime<Utc>,
}

impl HourlyAutomationUsage {
    // arch-exempt: too_many_args, record constructor preserves explicit automation fields, plan #7961
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: TenantId,
        window_start: DateTime<Utc>,
        user_id: UserId,
        automation_kind: AutomationKind,
        run_count: u64,
        completed_count: u64,
        failed_count: u64,
        cancelled_count: u64,
        recovery_required_count: u64,
        first_observed_at: DateTime<Utc>,
        last_observed_at: DateTime<Utc>,
    ) -> Result<Self, RecordError> {
        validate_hour(window_start)?;
        validate_range(first_observed_at, last_observed_at)?;
        for (field, value) in [
            ("run_count", run_count),
            ("completed_count", completed_count),
            ("failed_count", failed_count),
            ("cancelled_count", cancelled_count),
            ("recovery_required_count", recovery_required_count),
        ] {
            validate_counter(value, field)?;
        }
        validate_terminal_counts(
            run_count,
            completed_count,
            failed_count,
            cancelled_count,
            recovery_required_count,
        )?;
        Ok(Self {
            tenant_id,
            window_start,
            user_id,
            automation_kind,
            run_count,
            completed_count,
            failed_count,
            cancelled_count,
            recovery_required_count,
            first_observed_at,
            last_observed_at,
        })
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub const fn window_start(&self) -> DateTime<Utc> {
        self.window_start
    }

    pub fn user_id(&self) -> &UserId {
        &self.user_id
    }

    pub const fn automation_kind(&self) -> AutomationKind {
        self.automation_kind
    }

    pub const fn run_count(&self) -> u64 {
        self.run_count
    }

    pub const fn completed_count(&self) -> u64 {
        self.completed_count
    }

    pub const fn failed_count(&self) -> u64 {
        self.failed_count
    }

    pub const fn cancelled_count(&self) -> u64 {
        self.cancelled_count
    }

    pub const fn recovery_required_count(&self) -> u64 {
        self.recovery_required_count
    }

    pub const fn first_observed_at(&self) -> DateTime<Utc> {
        self.first_observed_at
    }

    pub const fn last_observed_at(&self) -> DateTime<Utc> {
        self.last_observed_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LifecycleEvent {
    tenant_id: TenantId,
    event_id: LifecycleEventId,
    user_id: Option<UserId>,
    event_kind: LifecycleEventKind,
    subject_kind: LifecycleSubjectKind,
    subject_id: SubjectId,
    occurred_at: DateTime<Utc>,
}

impl LifecycleEvent {
    pub fn new(
        tenant_id: TenantId,
        event_id: LifecycleEventId,
        user_id: Option<UserId>,
        event_kind: LifecycleEventKind,
        subject_kind: LifecycleSubjectKind,
        subject_id: SubjectId,
        occurred_at: DateTime<Utc>,
    ) -> Result<Self, RecordError> {
        if user_id.is_none() && subject_kind != LifecycleSubjectKind::Tenant {
            return Err(RecordError::MissingUserAttribution);
        }
        Ok(Self {
            tenant_id,
            event_id,
            user_id,
            event_kind,
            subject_kind,
            subject_id,
            occurred_at,
        })
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn event_id(&self) -> &LifecycleEventId {
        &self.event_id
    }

    pub fn user_id(&self) -> Option<&UserId> {
        self.user_id.as_ref()
    }

    pub const fn event_kind(&self) -> LifecycleEventKind {
        self.event_kind
    }

    pub const fn subject_kind(&self) -> LifecycleSubjectKind {
        self.subject_kind
    }

    pub fn subject_id(&self) -> &SubjectId {
        &self.subject_id
    }

    pub const fn occurred_at(&self) -> DateTime<Utc> {
        self.occurred_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectorCoverage {
    tenant_id: TenantId,
    window_start: DateTime<Utc>,
    collector_instance_id: CollectorInstanceId,
    accepted_observation_count: u64,
    queue_full_drop_count: u64,
    closed_drop_count: u64,
    invalid_drop_count: u64,
    write_failed_observation_count: u64,
    first_observed_at: DateTime<Utc>,
    last_observed_at: DateTime<Utc>,
}

impl CollectorCoverage {
    // arch-exempt: too_many_args, record constructor preserves explicit collector coverage fields, plan #7961
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: TenantId,
        window_start: DateTime<Utc>,
        collector_instance_id: CollectorInstanceId,
        accepted_observation_count: u64,
        queue_full_drop_count: u64,
        closed_drop_count: u64,
        invalid_drop_count: u64,
        write_failed_observation_count: u64,
        first_observed_at: DateTime<Utc>,
        last_observed_at: DateTime<Utc>,
    ) -> Result<Self, RecordError> {
        validate_hour(window_start)?;
        validate_range(first_observed_at, last_observed_at)?;
        for (field, value) in [
            ("accepted_observation_count", accepted_observation_count),
            ("queue_full_drop_count", queue_full_drop_count),
            ("closed_drop_count", closed_drop_count),
            ("invalid_drop_count", invalid_drop_count),
            (
                "write_failed_observation_count",
                write_failed_observation_count,
            ),
        ] {
            validate_counter(value, field)?;
        }
        Ok(Self {
            tenant_id,
            window_start,
            collector_instance_id,
            accepted_observation_count,
            queue_full_drop_count,
            closed_drop_count,
            invalid_drop_count,
            write_failed_observation_count,
            first_observed_at,
            last_observed_at,
        })
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub const fn window_start(&self) -> DateTime<Utc> {
        self.window_start
    }

    pub fn collector_instance_id(&self) -> &CollectorInstanceId {
        &self.collector_instance_id
    }

    pub const fn accepted_observation_count(&self) -> u64 {
        self.accepted_observation_count
    }

    pub const fn queue_full_drop_count(&self) -> u64 {
        self.queue_full_drop_count
    }

    pub const fn closed_drop_count(&self) -> u64 {
        self.closed_drop_count
    }

    pub const fn invalid_drop_count(&self) -> u64 {
        self.invalid_drop_count
    }

    pub const fn write_failed_observation_count(&self) -> u64 {
        self.write_failed_observation_count
    }

    pub const fn first_observed_at(&self) -> DateTime<Utc> {
        self.first_observed_at
    }

    pub const fn last_observed_at(&self) -> DateTime<Utc> {
        self.last_observed_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TelemetryBatch {
    activity: Vec<HourlyUserActivity>,
    model_usage: Vec<HourlyModelUsage>,
    run_failures: Vec<HourlyRunFailure>,
    automation_usage: Vec<HourlyAutomationUsage>,
    lifecycle_events: Vec<LifecycleEvent>,
    collector_coverage: Vec<CollectorCoverage>,
}

impl TelemetryBatch {
    pub fn new(
        activity: Vec<HourlyUserActivity>,
        model_usage: Vec<HourlyModelUsage>,
        run_failures: Vec<HourlyRunFailure>,
        automation_usage: Vec<HourlyAutomationUsage>,
        lifecycle_events: Vec<LifecycleEvent>,
        collector_coverage: Vec<CollectorCoverage>,
    ) -> Result<Self, RecordError> {
        validate_unique_rows(
            activity.iter().map(|row| {
                (
                    row.tenant_id().clone(),
                    row.window_start(),
                    row.user_id().clone(),
                    row.origin_kind(),
                )
            }),
            TelemetryBatchRowFamily::Activity,
        )?;
        validate_unique_rows(
            model_usage.iter().map(|row| {
                (
                    row.tenant_id().clone(),
                    row.window_start(),
                    row.user_id().clone(),
                    row.provider_id().clone(),
                    row.effective_model_id().clone(),
                )
            }),
            TelemetryBatchRowFamily::ModelUsage,
        )?;
        validate_unique_rows(
            run_failures.iter().map(|row| {
                (
                    row.tenant_id().clone(),
                    row.window_start(),
                    row.user_id().clone(),
                    row.failure_category().clone(),
                )
            }),
            TelemetryBatchRowFamily::RunFailure,
        )?;
        validate_unique_rows(
            automation_usage.iter().map(|row| {
                (
                    row.tenant_id().clone(),
                    row.window_start(),
                    row.user_id().clone(),
                    row.automation_kind(),
                )
            }),
            TelemetryBatchRowFamily::AutomationUsage,
        )?;
        validate_unique_rows(
            lifecycle_events
                .iter()
                .map(|row| (row.tenant_id().clone(), row.event_id().clone())),
            TelemetryBatchRowFamily::LifecycleEvent,
        )?;
        validate_unique_rows(
            collector_coverage.iter().map(|row| {
                (
                    row.tenant_id().clone(),
                    row.window_start(),
                    row.collector_instance_id().clone(),
                )
            }),
            TelemetryBatchRowFamily::CollectorCoverage,
        )?;
        Ok(Self {
            activity,
            model_usage,
            run_failures,
            automation_usage,
            lifecycle_events,
            collector_coverage,
        })
    }

    pub fn activity(&self) -> &[HourlyUserActivity] {
        &self.activity
    }

    pub fn model_usage(&self) -> &[HourlyModelUsage] {
        &self.model_usage
    }

    pub fn run_failures(&self) -> &[HourlyRunFailure] {
        &self.run_failures
    }

    pub fn automation_usage(&self) -> &[HourlyAutomationUsage] {
        &self.automation_usage
    }

    pub fn lifecycle_events(&self) -> &[LifecycleEvent] {
        &self.lifecycle_events
    }

    pub fn collector_coverage(&self) -> &[CollectorCoverage] {
        &self.collector_coverage
    }

    pub const fn record_count(&self) -> usize {
        self.activity.len()
            + self.model_usage.len()
            + self.run_failures.len()
            + self.automation_usage.len()
            + self.lifecycle_events.len()
            + self.collector_coverage.len()
    }
}
