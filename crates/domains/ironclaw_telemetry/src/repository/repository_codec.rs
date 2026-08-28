//! Private telemetry wire, projection, cursor, and additive-CAS grammar.

use chrono::{DateTime, SecondsFormat, Timelike, Utc};
use ironclaw_filesystem::{
    CasApply, CasUpdateError, ContentType, IndexKey, IndexValue, ScopedFilesystem, VersionedEntry,
    cas_update,
};
use ironclaw_host_api::ids::{TenantId, UserId};
use ironclaw_host_api::path::ScopedPath;
use ironclaw_telemetry_contracts::observation::{
    AutomationKind, CollectorInstanceId, EffectiveModelId, FailureCategory, LifecycleEventId,
    LifecycleEventKind, LifecycleSubjectKind, OriginKind, ProviderId, SubjectId,
};
use serde::{Serialize, de::DeserializeOwned};
use std::{collections::BTreeMap, sync::Arc};

use super::{
    FAMILY_ACTIVITY, FAMILY_AUTOMATION, FAMILY_COVERAGE, FAMILY_FAILURE, FAMILY_MODEL,
    MAX_TELEMETRY_CURSOR_BYTES, RECORD_SCHEMA_VERSION, ResourceScope, WireActivity, WireAutomation,
    WireCoverage, WireFailure, WireLifecycle, WireModel, checked_add, checked_counter, entry,
    json_error, parse_version,
};
use crate::{
    CollectorCoverage, HourlyAutomationUsage, HourlyModelUsage, HourlyRunFailure,
    HourlyUserActivity, LifecycleEvent, error::TelemetryRepositoryError,
};

pub(super) fn timestamp_text(timestamp: DateTime<Utc>) -> String {
    normalize_timestamp(timestamp).to_rfc3339_opts(SecondsFormat::Micros, true)
}

pub(super) fn normalize_timestamp(timestamp: DateTime<Utc>) -> DateTime<Utc> {
    let micros = timestamp.nanosecond() / 1_000;
    match timestamp.with_nanosecond(micros * 1_000) {
        Some(normalized) => normalized,
        None => timestamp,
    }
}

pub(super) fn parse_timestamp(
    value: &str,
    field: &'static str,
) -> Result<DateTime<Utc>, TelemetryRepositoryError> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|timestamp| normalize_timestamp(timestamp.with_timezone(&Utc)))
        .map_err(|source| TelemetryRepositoryError::InvalidTimestamp { field, source })
}

pub(super) fn encode_cursor(timestamp: DateTime<Utc>, fields: &[&str]) -> String {
    let timestamp = timestamp_text(timestamp);
    let mut cursor = String::new();
    append_length_prefixed_segment(&mut cursor, &timestamp);
    for field in fields {
        append_length_prefixed_segment(&mut cursor, field);
    }
    cursor
}

pub(super) fn append_length_prefixed_segment(cursor: &mut String, value: &str) {
    cursor.push_str(&value.len().to_string());
    cursor.push(':');
    cursor.push_str(value);
}

pub(super) fn decode_cursor(
    cursor: &str,
    expected_fields: usize,
) -> Result<(DateTime<Utc>, Vec<String>), TelemetryRepositoryError> {
    if cursor.len() > MAX_TELEMETRY_CURSOR_BYTES {
        return Err(TelemetryRepositoryError::InvalidCursor);
    }
    let mut cursor = cursor.as_bytes();
    let timestamp = parse_length_prefixed_segment(&mut cursor)?;
    let timestamp = parse_timestamp(&timestamp, "cursor timestamp")?;
    let mut fields = Vec::with_capacity(expected_fields);
    while !cursor.is_empty() {
        fields.push(parse_length_prefixed_segment(&mut cursor)?);
    }
    if fields.len() != expected_fields {
        return Err(TelemetryRepositoryError::InvalidCursor);
    }
    Ok((timestamp, fields))
}

pub(super) fn parse_length_prefixed_segment(
    cursor: &mut &[u8],
) -> Result<String, TelemetryRepositoryError> {
    let colon = cursor
        .iter()
        .position(|byte| *byte == b':')
        .ok_or(TelemetryRepositoryError::InvalidCursor)?;
    let length_text = std::str::from_utf8(&cursor[..colon])
        .map_err(TelemetryRepositoryError::invalid_cursor_encoding)?;
    let length = length_text.parse::<usize>().map_err(|source| {
        TelemetryRepositoryError::invalid_cursor_length(length_text.to_owned(), source)
    })?;
    // The tie-breaker is an encoded scoped path, not an identifier. Escaping
    // valid 128-byte identifiers can expand a path beyond the identifier
    // bound, so cap each segment by the already bounded whole-cursor budget.
    if length == 0 || length > MAX_TELEMETRY_CURSOR_BYTES {
        return Err(TelemetryRepositoryError::InvalidCursor);
    }
    let value_start = colon + 1;
    let value_end = value_start
        .checked_add(length)
        .ok_or(TelemetryRepositoryError::InvalidCursor)?;
    if value_end > cursor.len() {
        return Err(TelemetryRepositoryError::InvalidCursor);
    }
    let value = String::from_utf8(cursor[value_start..value_end].to_vec())
        .map_err(TelemetryRepositoryError::invalid_cursor_encoding)?;
    *cursor = &cursor[value_end..];
    Ok(value)
}

#[cfg(test)]
pub(super) fn checked_counter_sum(
    current: i64,
    incoming: u64,
    family: &'static str,
) -> Result<(), TelemetryRepositoryError> {
    let current =
        u64::try_from(current).map_err(|source| TelemetryRepositoryError::CounterConversion {
            family,
            value: current,
            source,
        })?;
    let total = current
        .checked_add(incoming)
        .ok_or(TelemetryRepositoryError::CounterOverflow { family })?;
    if total > ironclaw_telemetry_contracts::observation::MAX_DURABLE_COUNTER {
        return Err(TelemetryRepositoryError::CounterOverflow { family });
    }
    Ok(())
}

pub(super) fn decode_tenant_id(value: String) -> Result<TenantId, TelemetryRepositoryError> {
    TenantId::new(value.clone()).map_err(|source| {
        TelemetryRepositoryError::invalid_persisted_field("tenant_id", value, source)
    })
}

pub(super) fn decode_user_id(value: String) -> Result<UserId, TelemetryRepositoryError> {
    UserId::new(value.clone()).map_err(|source| {
        TelemetryRepositoryError::invalid_persisted_field("user_id", value, source)
    })
}

pub(super) fn decode_provider_id(value: String) -> Result<ProviderId, TelemetryRepositoryError> {
    ProviderId::new(value.clone()).map_err(|source| {
        TelemetryRepositoryError::invalid_persisted_field("provider_id", value, source)
    })
}

pub(super) fn decode_model_id(value: String) -> Result<EffectiveModelId, TelemetryRepositoryError> {
    EffectiveModelId::new(value.clone()).map_err(|source| {
        TelemetryRepositoryError::invalid_persisted_field("effective_model_id", value, source)
    })
}

pub(super) fn decode_failure_category(
    value: String,
) -> Result<FailureCategory, TelemetryRepositoryError> {
    FailureCategory::new(value.clone()).map_err(|source| {
        TelemetryRepositoryError::invalid_persisted_field("failure_category", value, source)
    })
}

pub(super) fn decode_event_id(value: String) -> Result<LifecycleEventId, TelemetryRepositoryError> {
    LifecycleEventId::new(value.clone()).map_err(|source| {
        TelemetryRepositoryError::invalid_persisted_field("event_id", value, source)
    })
}

pub(super) fn decode_subject_id(value: String) -> Result<SubjectId, TelemetryRepositoryError> {
    SubjectId::new(value.clone()).map_err(|source| {
        TelemetryRepositoryError::invalid_persisted_field("subject_id", value, source)
    })
}

pub(super) fn decode_collector_id(
    value: String,
) -> Result<CollectorInstanceId, TelemetryRepositoryError> {
    CollectorInstanceId::new(value.clone()).map_err(|source| {
        TelemetryRepositoryError::invalid_persisted_field("collector_instance_id", value, source)
    })
}

pub(super) fn origin_text(value: OriginKind) -> &'static str {
    match value {
        OriginKind::Human => "human",
        OriginKind::ParentAgent => "parent_agent",
        OriginKind::System => "system",
        OriginKind::Automation => "automation",
        OriginKind::Other => "other",
    }
}

pub(super) fn parse_origin(value: &str) -> Result<OriginKind, TelemetryRepositoryError> {
    match value {
        "human" => Ok(OriginKind::Human),
        "parent_agent" => Ok(OriginKind::ParentAgent),
        "system" => Ok(OriginKind::System),
        "automation" => Ok(OriginKind::Automation),
        "other" => Ok(OriginKind::Other),
        value => Err(TelemetryRepositoryError::UnknownEnum {
            field: "origin_kind",
            value: value.to_owned(),
        }),
    }
}

pub(super) fn automation_text(value: AutomationKind) -> &'static str {
    match value {
        AutomationKind::Cron => "cron",
        AutomationKind::Once => "once",
        AutomationKind::Manual => "manual",
    }
}

pub(super) fn parse_automation(value: &str) -> Result<AutomationKind, TelemetryRepositoryError> {
    match value {
        "cron" => Ok(AutomationKind::Cron),
        "once" => Ok(AutomationKind::Once),
        "manual" => Ok(AutomationKind::Manual),
        value => Err(TelemetryRepositoryError::UnknownEnum {
            field: "automation_kind",
            value: value.to_owned(),
        }),
    }
}

pub(super) fn lifecycle_event_text(value: LifecycleEventKind) -> &'static str {
    match value {
        LifecycleEventKind::MemberAdded => "member_added",
        LifecycleEventKind::MemberRemoved => "member_removed",
        LifecycleEventKind::RoutineCreated => "routine_created",
        LifecycleEventKind::RoutineEnabled => "routine_enabled",
        LifecycleEventKind::RoutineDisabled => "routine_disabled",
        LifecycleEventKind::RoutineDeleted => "routine_deleted",
    }
}

pub(super) fn parse_event(value: &str) -> Result<LifecycleEventKind, TelemetryRepositoryError> {
    match value {
        "member_added" => Ok(LifecycleEventKind::MemberAdded),
        "member_removed" => Ok(LifecycleEventKind::MemberRemoved),
        "routine_created" => Ok(LifecycleEventKind::RoutineCreated),
        "routine_enabled" => Ok(LifecycleEventKind::RoutineEnabled),
        "routine_disabled" => Ok(LifecycleEventKind::RoutineDisabled),
        "routine_deleted" => Ok(LifecycleEventKind::RoutineDeleted),
        value => Err(TelemetryRepositoryError::UnknownEnum {
            field: "event_kind",
            value: value.to_owned(),
        }),
    }
}

pub(super) fn lifecycle_subject_text(value: LifecycleSubjectKind) -> &'static str {
    match value {
        LifecycleSubjectKind::Tenant => "tenant",
        LifecycleSubjectKind::User => "user",
        LifecycleSubjectKind::Routine => "routine",
    }
}

pub(super) fn parse_subject(value: &str) -> Result<LifecycleSubjectKind, TelemetryRepositoryError> {
    match value {
        "tenant" => Ok(LifecycleSubjectKind::Tenant),
        "user" => Ok(LifecycleSubjectKind::User),
        "routine" => Ok(LifecycleSubjectKind::Routine),
        value => Err(TelemetryRepositoryError::UnknownEnum {
            field: "subject_kind",
            value: value.to_owned(),
        }),
    }
}

pub(super) fn activity_wire(row: &HourlyUserActivity) -> WireActivity {
    WireActivity {
        schema_version: RECORD_SCHEMA_VERSION,
        tenant_id: row.tenant_id().as_str().to_owned(),
        window_start: timestamp_text(row.window_start()),
        user_id: row.user_id().as_str().to_owned(),
        origin_kind: origin_text(row.origin_kind()).to_owned(),
        run_count: row.run_count(),
        runs_with_reported_tool_calls_count: row.runs_with_reported_tool_calls_count(),
        tool_count_reported_run_count: row.tool_count_reported_run_count(),
        reported_tool_call_count: row.reported_tool_call_count(),
        completed_count: row.completed_count(),
        failed_count: row.failed_count(),
        cancelled_count: row.cancelled_count(),
        recovery_required_count: row.recovery_required_count(),
        total_run_latency_ms: row.total_run_latency_ms(),
        first_observed_at: timestamp_text(row.first_observed_at()),
        last_observed_at: timestamp_text(row.last_observed_at()),
    }
}

pub(super) fn activity_from_wire(
    wire: WireActivity,
) -> Result<HourlyUserActivity, TelemetryRepositoryError> {
    parse_version(wire.schema_version)?;
    Ok(HourlyUserActivity::new(
        decode_tenant_id(wire.tenant_id)?,
        parse_timestamp(&wire.window_start, "window_start")?,
        decode_user_id(wire.user_id)?,
        parse_origin(&wire.origin_kind)?,
        checked_counter(wire.run_count, FAMILY_ACTIVITY)?,
        checked_counter(wire.runs_with_reported_tool_calls_count, FAMILY_ACTIVITY)?,
        checked_counter(wire.tool_count_reported_run_count, FAMILY_ACTIVITY)?,
        checked_counter(wire.reported_tool_call_count, FAMILY_ACTIVITY)?,
        checked_counter(wire.completed_count, FAMILY_ACTIVITY)?,
        checked_counter(wire.failed_count, FAMILY_ACTIVITY)?,
        checked_counter(wire.cancelled_count, FAMILY_ACTIVITY)?,
        checked_counter(wire.recovery_required_count, FAMILY_ACTIVITY)?,
        checked_counter(wire.total_run_latency_ms, FAMILY_ACTIVITY)?,
        parse_timestamp(&wire.first_observed_at, "first_observed_at")?,
        parse_timestamp(&wire.last_observed_at, "last_observed_at")?,
    )?)
}

pub(super) fn add_activity(
    current: WireActivity,
    incoming: &HourlyUserActivity,
) -> Result<WireActivity, TelemetryRepositoryError> {
    let existing = activity_from_wire(current)?;
    let result = HourlyUserActivity::new(
        existing.tenant_id().clone(),
        existing.window_start(),
        existing.user_id().clone(),
        existing.origin_kind(),
        checked_add(existing.run_count(), incoming.run_count(), FAMILY_ACTIVITY)?,
        checked_add(
            existing.runs_with_reported_tool_calls_count(),
            incoming.runs_with_reported_tool_calls_count(),
            FAMILY_ACTIVITY,
        )?,
        checked_add(
            existing.tool_count_reported_run_count(),
            incoming.tool_count_reported_run_count(),
            FAMILY_ACTIVITY,
        )?,
        checked_add(
            existing.reported_tool_call_count(),
            incoming.reported_tool_call_count(),
            FAMILY_ACTIVITY,
        )?,
        checked_add(
            existing.completed_count(),
            incoming.completed_count(),
            FAMILY_ACTIVITY,
        )?,
        checked_add(
            existing.failed_count(),
            incoming.failed_count(),
            FAMILY_ACTIVITY,
        )?,
        checked_add(
            existing.cancelled_count(),
            incoming.cancelled_count(),
            FAMILY_ACTIVITY,
        )?,
        checked_add(
            existing.recovery_required_count(),
            incoming.recovery_required_count(),
            FAMILY_ACTIVITY,
        )?,
        checked_add(
            existing.total_run_latency_ms(),
            incoming.total_run_latency_ms(),
            FAMILY_ACTIVITY,
        )?,
        existing
            .first_observed_at()
            .min(incoming.first_observed_at()),
        existing.last_observed_at().max(incoming.last_observed_at()),
    )?;
    Ok(activity_wire(&result))
}

pub(super) fn model_wire(row: &HourlyModelUsage) -> WireModel {
    WireModel {
        schema_version: RECORD_SCHEMA_VERSION,
        tenant_id: row.tenant_id().as_str().to_owned(),
        user_id: row.user_id().as_str().to_owned(),
        window_start: timestamp_text(row.window_start()),
        provider_id: row.provider_id().as_str().to_owned(),
        effective_model_id: row.effective_model_id().as_str().to_owned(),
        inference_count: row.inference_count(),
        usage_reported_count: row.usage_reported_count(),
        input_tokens: row.input_tokens(),
        output_tokens: row.output_tokens(),
        cache_read_input_tokens: row.cache_read_input_tokens(),
        cache_creation_input_tokens: row.cache_creation_input_tokens(),
        first_observed_at: timestamp_text(row.first_observed_at()),
        last_observed_at: timestamp_text(row.last_observed_at()),
    }
}

pub(super) fn model_from_wire(
    wire: WireModel,
) -> Result<HourlyModelUsage, TelemetryRepositoryError> {
    parse_version(wire.schema_version)?;
    Ok(HourlyModelUsage::new(
        decode_tenant_id(wire.tenant_id)?,
        decode_user_id(wire.user_id)?,
        parse_timestamp(&wire.window_start, "window_start")?,
        decode_provider_id(wire.provider_id)?,
        decode_model_id(wire.effective_model_id)?,
        checked_counter(wire.inference_count, FAMILY_MODEL)?,
        checked_counter(wire.usage_reported_count, FAMILY_MODEL)?,
        checked_counter(wire.input_tokens, FAMILY_MODEL)?,
        checked_counter(wire.output_tokens, FAMILY_MODEL)?,
        checked_counter(wire.cache_read_input_tokens, FAMILY_MODEL)?,
        checked_counter(wire.cache_creation_input_tokens, FAMILY_MODEL)?,
        parse_timestamp(&wire.first_observed_at, "first_observed_at")?,
        parse_timestamp(&wire.last_observed_at, "last_observed_at")?,
    )?)
}

pub(super) fn add_model(
    current: WireModel,
    incoming: &HourlyModelUsage,
) -> Result<WireModel, TelemetryRepositoryError> {
    let existing = model_from_wire(current)?;
    let result = HourlyModelUsage::new(
        existing.tenant_id().clone(),
        existing.user_id().clone(),
        existing.window_start(),
        existing.provider_id().clone(),
        existing.effective_model_id().clone(),
        checked_add(
            existing.inference_count(),
            incoming.inference_count(),
            FAMILY_MODEL,
        )?,
        checked_add(
            existing.usage_reported_count(),
            incoming.usage_reported_count(),
            FAMILY_MODEL,
        )?,
        checked_add(
            existing.input_tokens(),
            incoming.input_tokens(),
            FAMILY_MODEL,
        )?,
        checked_add(
            existing.output_tokens(),
            incoming.output_tokens(),
            FAMILY_MODEL,
        )?,
        checked_add(
            existing.cache_read_input_tokens(),
            incoming.cache_read_input_tokens(),
            FAMILY_MODEL,
        )?,
        checked_add(
            existing.cache_creation_input_tokens(),
            incoming.cache_creation_input_tokens(),
            FAMILY_MODEL,
        )?,
        existing
            .first_observed_at()
            .min(incoming.first_observed_at()),
        existing.last_observed_at().max(incoming.last_observed_at()),
    )?;
    Ok(model_wire(&result))
}

pub(super) fn failure_wire(row: &HourlyRunFailure) -> WireFailure {
    WireFailure {
        schema_version: RECORD_SCHEMA_VERSION,
        tenant_id: row.tenant_id().as_str().to_owned(),
        window_start: timestamp_text(row.window_start()),
        user_id: row.user_id().as_str().to_owned(),
        failure_category: row.failure_category().as_str().to_owned(),
        failure_count: row.failure_count(),
        first_observed_at: timestamp_text(row.first_observed_at()),
        last_observed_at: timestamp_text(row.last_observed_at()),
    }
}

pub(super) fn failure_from_wire(
    wire: WireFailure,
) -> Result<HourlyRunFailure, TelemetryRepositoryError> {
    parse_version(wire.schema_version)?;
    Ok(HourlyRunFailure::new(
        decode_tenant_id(wire.tenant_id)?,
        parse_timestamp(&wire.window_start, "window_start")?,
        decode_user_id(wire.user_id)?,
        decode_failure_category(wire.failure_category)?,
        checked_counter(wire.failure_count, FAMILY_FAILURE)?,
        parse_timestamp(&wire.first_observed_at, "first_observed_at")?,
        parse_timestamp(&wire.last_observed_at, "last_observed_at")?,
    )?)
}

pub(super) fn add_failure(
    current: WireFailure,
    incoming: &HourlyRunFailure,
) -> Result<WireFailure, TelemetryRepositoryError> {
    let existing = failure_from_wire(current)?;
    let result = HourlyRunFailure::new(
        existing.tenant_id().clone(),
        existing.window_start(),
        existing.user_id().clone(),
        existing.failure_category().clone(),
        checked_add(
            existing.failure_count(),
            incoming.failure_count(),
            FAMILY_FAILURE,
        )?,
        existing
            .first_observed_at()
            .min(incoming.first_observed_at()),
        existing.last_observed_at().max(incoming.last_observed_at()),
    )?;
    Ok(failure_wire(&result))
}

pub(super) fn automation_wire(row: &HourlyAutomationUsage) -> WireAutomation {
    WireAutomation {
        schema_version: RECORD_SCHEMA_VERSION,
        tenant_id: row.tenant_id().as_str().to_owned(),
        window_start: timestamp_text(row.window_start()),
        user_id: row.user_id().as_str().to_owned(),
        automation_kind: automation_text(row.automation_kind()).to_owned(),
        run_count: row.run_count(),
        completed_count: row.completed_count(),
        failed_count: row.failed_count(),
        cancelled_count: row.cancelled_count(),
        recovery_required_count: row.recovery_required_count(),
        first_observed_at: timestamp_text(row.first_observed_at()),
        last_observed_at: timestamp_text(row.last_observed_at()),
    }
}

pub(super) fn automation_from_wire(
    wire: WireAutomation,
) -> Result<HourlyAutomationUsage, TelemetryRepositoryError> {
    parse_version(wire.schema_version)?;
    Ok(HourlyAutomationUsage::new(
        decode_tenant_id(wire.tenant_id)?,
        parse_timestamp(&wire.window_start, "window_start")?,
        decode_user_id(wire.user_id)?,
        parse_automation(&wire.automation_kind)?,
        checked_counter(wire.run_count, FAMILY_AUTOMATION)?,
        checked_counter(wire.completed_count, FAMILY_AUTOMATION)?,
        checked_counter(wire.failed_count, FAMILY_AUTOMATION)?,
        checked_counter(wire.cancelled_count, FAMILY_AUTOMATION)?,
        checked_counter(wire.recovery_required_count, FAMILY_AUTOMATION)?,
        parse_timestamp(&wire.first_observed_at, "first_observed_at")?,
        parse_timestamp(&wire.last_observed_at, "last_observed_at")?,
    )?)
}

pub(super) fn add_automation(
    current: WireAutomation,
    incoming: &HourlyAutomationUsage,
) -> Result<WireAutomation, TelemetryRepositoryError> {
    let existing = automation_from_wire(current)?;
    let result = HourlyAutomationUsage::new(
        existing.tenant_id().clone(),
        existing.window_start(),
        existing.user_id().clone(),
        existing.automation_kind(),
        checked_add(
            existing.run_count(),
            incoming.run_count(),
            FAMILY_AUTOMATION,
        )?,
        checked_add(
            existing.completed_count(),
            incoming.completed_count(),
            FAMILY_AUTOMATION,
        )?,
        checked_add(
            existing.failed_count(),
            incoming.failed_count(),
            FAMILY_AUTOMATION,
        )?,
        checked_add(
            existing.cancelled_count(),
            incoming.cancelled_count(),
            FAMILY_AUTOMATION,
        )?,
        checked_add(
            existing.recovery_required_count(),
            incoming.recovery_required_count(),
            FAMILY_AUTOMATION,
        )?,
        existing
            .first_observed_at()
            .min(incoming.first_observed_at()),
        existing.last_observed_at().max(incoming.last_observed_at()),
    )?;
    Ok(automation_wire(&result))
}

pub(super) fn lifecycle_wire(row: &LifecycleEvent) -> WireLifecycle {
    WireLifecycle {
        schema_version: RECORD_SCHEMA_VERSION,
        tenant_id: row.tenant_id().as_str().to_owned(),
        event_id: row.event_id().as_str().to_owned(),
        user_id: row.user_id().map(|id| id.as_str().to_owned()),
        event_kind: lifecycle_event_text(row.event_kind()).to_owned(),
        subject_kind: lifecycle_subject_text(row.subject_kind()).to_owned(),
        subject_id: row.subject_id().as_str().to_owned(),
        occurred_at: timestamp_text(row.occurred_at()),
    }
}

pub(super) fn lifecycle_from_wire(
    wire: WireLifecycle,
) -> Result<LifecycleEvent, TelemetryRepositoryError> {
    parse_version(wire.schema_version)?;
    Ok(LifecycleEvent::new(
        decode_tenant_id(wire.tenant_id)?,
        decode_event_id(wire.event_id)?,
        wire.user_id.map(decode_user_id).transpose()?,
        parse_event(&wire.event_kind)?,
        parse_subject(&wire.subject_kind)?,
        decode_subject_id(wire.subject_id)?,
        parse_timestamp(&wire.occurred_at, "occurred_at")?,
    )?)
}

pub(super) fn coverage_wire(row: &CollectorCoverage) -> WireCoverage {
    WireCoverage {
        schema_version: RECORD_SCHEMA_VERSION,
        tenant_id: row.tenant_id().as_str().to_owned(),
        window_start: timestamp_text(row.window_start()),
        collector_instance_id: row.collector_instance_id().as_str().to_owned(),
        accepted_observation_count: row.accepted_observation_count(),
        queue_full_drop_count: row.queue_full_drop_count(),
        closed_drop_count: row.closed_drop_count(),
        invalid_drop_count: row.invalid_drop_count(),
        write_failed_observation_count: row.write_failed_observation_count(),
        first_observed_at: timestamp_text(row.first_observed_at()),
        last_observed_at: timestamp_text(row.last_observed_at()),
    }
}

pub(super) fn coverage_from_wire(
    wire: WireCoverage,
) -> Result<CollectorCoverage, TelemetryRepositoryError> {
    parse_version(wire.schema_version)?;
    Ok(CollectorCoverage::new(
        decode_tenant_id(wire.tenant_id)?,
        parse_timestamp(&wire.window_start, "window_start")?,
        decode_collector_id(wire.collector_instance_id)?,
        checked_counter(wire.accepted_observation_count, FAMILY_COVERAGE)?,
        checked_counter(wire.queue_full_drop_count, FAMILY_COVERAGE)?,
        checked_counter(wire.closed_drop_count, FAMILY_COVERAGE)?,
        checked_counter(wire.invalid_drop_count, FAMILY_COVERAGE)?,
        checked_counter(wire.write_failed_observation_count, FAMILY_COVERAGE)?,
        parse_timestamp(&wire.first_observed_at, "first_observed_at")?,
        parse_timestamp(&wire.last_observed_at, "last_observed_at")?,
    )?)
}

pub(super) fn add_coverage(
    current: WireCoverage,
    incoming: &CollectorCoverage,
) -> Result<WireCoverage, TelemetryRepositoryError> {
    let existing = coverage_from_wire(current)?;
    let result = CollectorCoverage::new(
        existing.tenant_id().clone(),
        existing.window_start(),
        existing.collector_instance_id().clone(),
        checked_add(
            existing.accepted_observation_count(),
            incoming.accepted_observation_count(),
            FAMILY_COVERAGE,
        )?,
        checked_add(
            existing.queue_full_drop_count(),
            incoming.queue_full_drop_count(),
            FAMILY_COVERAGE,
        )?,
        checked_add(
            existing.closed_drop_count(),
            incoming.closed_drop_count(),
            FAMILY_COVERAGE,
        )?,
        checked_add(
            existing.invalid_drop_count(),
            incoming.invalid_drop_count(),
            FAMILY_COVERAGE,
        )?,
        checked_add(
            existing.write_failed_observation_count(),
            incoming.write_failed_observation_count(),
            FAMILY_COVERAGE,
        )?,
        existing
            .first_observed_at()
            .min(incoming.first_observed_at()),
        existing.last_observed_at().max(incoming.last_observed_at()),
    )?;
    Ok(coverage_wire(&result))
}

pub(super) fn map_cas_error(
    error: CasUpdateError<TelemetryRepositoryError>,
) -> TelemetryRepositoryError {
    match error {
        CasUpdateError::Apply(error) => error,
        CasUpdateError::Backend(source) => TelemetryRepositoryError::StorageOperation {
            operation: "updating telemetry record",
            source: Box::new(source),
        },
        CasUpdateError::CasUnsupported => TelemetryRepositoryError::StorageOperation {
            operation: "updating telemetry record with CAS",
            source: Box::new(std::io::Error::other("filesystem CAS is unsupported")),
        },
        CasUpdateError::RetriesExhausted => TelemetryRepositoryError::StorageOperation {
            operation: "retrying telemetry record CAS update",
            source: Box::new(std::io::Error::other("filesystem CAS retries exhausted")),
        },
        CasUpdateError::Timeout => TelemetryRepositoryError::StorageOperation {
            operation: "waiting for telemetry record CAS update",
            source: Box::new(std::io::Error::other("filesystem CAS update timed out")),
        },
    }
}

// The codec callbacks stay explicit so each record family can supply its own
// closed wire type without a second repository abstraction.
// arch-exempt: too_many_args, additive update keeps storage callbacks and typed record inputs explicit, plan #7961
#[allow(clippy::too_many_arguments)]
pub(super) async fn additive_update<F, W, R, M, C>(
    filesystem: &Arc<ScopedFilesystem<F>>,
    scope: &ResourceScope,
    path: &ScopedPath,
    kind: &'static str,
    indexed: BTreeMap<IndexKey, IndexValue>,
    incoming: R,
    merge: M,
    to_wire: C,
) -> Result<(), TelemetryRepositoryError>
where
    F: ironclaw_filesystem::RootFilesystem + ?Sized,
    W: Serialize + DeserializeOwned + Clone + PartialEq,
    R: Clone,
    M: Fn(W, &R) -> Result<W, TelemetryRepositoryError> + Copy,
    C: Fn(&R) -> W + Copy,
{
    let incoming_wire = to_wire(&incoming);
    let decode = |body: &[u8]| {
        serde_json::from_slice::<W>(body)
            .map_err(|source| json_error("decoding telemetry record", source))
    };
    let encode = move |wire: &W| entry(kind, wire, indexed.clone());
    cas_update::<F, W, (), TelemetryRepositoryError, _, _, _, _>(
        filesystem.as_ref(),
        scope,
        path,
        decode,
        encode,
        move |current| {
            let incoming = incoming.clone();
            let incoming_wire = incoming_wire.clone();
            async move {
                let next = match current {
                    Some(current) => merge(current, &incoming)?,
                    None => incoming_wire,
                };
                Ok(CasApply::new(next, ()))
            }
        },
    )
    .await
    .map_err(map_cas_error)
}

pub(super) fn validate_entry_shape(
    entry: &VersionedEntry,
    kind: &'static str,
    expected: BTreeMap<IndexKey, IndexValue>,
) -> Result<(), TelemetryRepositoryError> {
    let actual_kind = entry.entry.kind.as_ref().map(|value| value.as_str());
    if actual_kind != Some(kind) || entry.entry.content_type.as_str() != ContentType::JSON {
        return Err(TelemetryRepositoryError::InvalidProjection);
    }
    if entry.entry.indexed != expected {
        return Err(TelemetryRepositoryError::InvalidProjection);
    }
    Ok(())
}
