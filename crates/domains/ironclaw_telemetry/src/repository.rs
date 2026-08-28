use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use ironclaw_filesystem::{
    CasApply, ContentType, Entry, Filter, IndexKey, IndexKind, IndexName, IndexSpec, IndexValue,
    OrderedPage, OrderedQueryCursor, ScopedFilesystem, SortDirection, VersionedEntry, cas_update,
};
use ironclaw_host_api::{path::ScopedPath, resource::ResourceScope};
use ironclaw_telemetry_contracts::observation::{EffectiveModelId, ProviderId};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

use crate::{
    CollectorCoverage, HourlyAutomationUsage, HourlyModelUsage, HourlyRunFailure,
    HourlyUserActivity, LifecycleEvent, TelemetryBatch, error::TelemetryRepositoryError,
};
mod batch_sink;
mod repository_codec;
mod store;
use repository_codec::*;

pub const MAX_TELEMETRY_PAGE_SIZE: usize = 2_000;
/// Maximum UTF-8 byte length accepted for an opaque telemetry page cursor.
///
/// Cursors are parsed before they are used in backend query parameters. This
/// bound keeps malformed input from driving unbounded parser work or scratch
/// allocations while leaving ample room for the bounded cursor fields.
pub const MAX_TELEMETRY_CURSOR_BYTES: usize = 4_096;

const TELEMETRY_PREFIX: &str = "/tenant-shared/telemetry/v0";
const RECORD_SCHEMA_VERSION: u16 = 0;
const MAX_TELEMETRY_RANGE: chrono::Duration = chrono::Duration::days(366);

const FAMILY_ACTIVITY: &str = "activity";
const FAMILY_MODEL: &str = "model";
const FAMILY_FAILURE: &str = "failure";
const FAMILY_AUTOMATION: &str = "automation";
const FAMILY_LIFECYCLE: &str = "lifecycle";
const FAMILY_COVERAGE: &str = "coverage";

const INDEX_FAMILY_TIME: &str = "telemetry_family_time_v0";
const INDEX_PROVIDER_TIME: &str = "telemetry_provider_time_v0";
const INDEX_MODEL_TIME: &str = "telemetry_model_time_v0";
const INDEX_PROVIDER_MODEL_TIME: &str = "telemetry_provider_model_time_v0";
const INDEX_LIFECYCLE_TIME: &str = "telemetry_lifecycle_time_v0";

fn ceil_timestamp(timestamp: DateTime<Utc>) -> Result<DateTime<Utc>, TelemetryRepositoryError> {
    let floored = normalize_timestamp(timestamp);
    if floored == timestamp {
        return Ok(timestamp);
    }
    floored.checked_add_signed(Duration::microseconds(1)).ok_or(
        TelemetryRepositoryError::InvalidScanRequest {
            reason: "range lower bound cannot be represented at telemetry precision",
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedTelemetryBatch {
    scope: ResourceScope,
    batch: TelemetryBatch,
}

impl ScopedTelemetryBatch {
    pub fn new(scope: ResourceScope, batch: TelemetryBatch) -> Self {
        Self { scope, batch }
    }

    fn validate(&self) -> Result<(), TelemetryRepositoryError> {
        let tenant = self.scope.tenant_id.as_str();
        let tenants = self
            .batch
            .activity()
            .iter()
            .map(|row| row.tenant_id().as_str())
            .chain(
                self.batch
                    .model_usage()
                    .iter()
                    .map(|row| row.tenant_id().as_str()),
            )
            .chain(
                self.batch
                    .run_failures()
                    .iter()
                    .map(|row| row.tenant_id().as_str()),
            )
            .chain(
                self.batch
                    .automation_usage()
                    .iter()
                    .map(|row| row.tenant_id().as_str()),
            )
            .chain(
                self.batch
                    .lifecycle_events()
                    .iter()
                    .map(|row| row.tenant_id().as_str()),
            )
            .chain(
                self.batch
                    .collector_coverage()
                    .iter()
                    .map(|row| row.tenant_id().as_str()),
            );
        if tenants.into_iter().any(|row_tenant| row_tenant != tenant) {
            Err(TelemetryRepositoryError::ScopeMismatch)
        } else {
            Ok(())
        }
    }

    pub fn scope(&self) -> &ResourceScope {
        &self.scope
    }
    pub fn batch(&self) -> &TelemetryBatch {
        &self.batch
    }
    pub fn into_parts(self) -> (ResourceScope, TelemetryBatch) {
        (self.scope, self.batch)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
/// A repository's accounting for one already-scoped batch.
///
/// A successful report may account for a prefix, but only a report with no
/// failed records and a prefix equal to the submitted row count is complete.
/// Repository errors intentionally carry no report: a backend may have
/// committed an unknown prefix before returning an error, so the worker treats
/// the whole attempted batch as ambiguous and records count-only loss coverage.
pub struct BatchApplyReport {
    applied_prefix: usize,
    failed_record_count: usize,
}

impl BatchApplyReport {
    /// Build a report for a batch whose complete row prefix was applied.
    pub(crate) const fn complete(applied_prefix: usize) -> Self {
        Self::from_counts(applied_prefix, 0)
    }

    /// Build a conservative report for a batch that did not fully apply.
    pub(crate) const fn from_counts(applied_prefix: usize, failed_record_count: usize) -> Self {
        Self {
            applied_prefix,
            failed_record_count,
        }
    }

    /// Reports are complete only when the repository accounted for every row.
    pub(crate) const fn is_complete_for(self, expected_records: usize) -> bool {
        self.failed_record_count == 0 && self.applied_prefix == expected_records
    }

    pub const fn applied_prefix(self) -> usize {
        self.applied_prefix
    }
    pub const fn applied_record_count(self) -> usize {
        self.applied_prefix
    }
    pub const fn failed_record_count(self) -> usize {
        self.failed_record_count
    }
}

/// The worker needs only this behavior port. It is private so backend choice
/// cannot become a second repository contract.
#[async_trait]
pub(crate) trait TelemetryBatchSink: Send + Sync {
    async fn apply_batch(
        &self,
        batch: ScopedTelemetryBatch,
    ) -> Result<BatchApplyReport, TelemetryRepositoryError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryPageRequest {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    now: DateTime<Utc>,
    page_size: usize,
    after: Option<String>,
    include_partial: bool,
    provider_id: Option<ProviderId>,
    effective_model_id: Option<EffectiveModelId>,
}

impl TelemetryPageRequest {
    pub fn new(
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        now: DateTime<Utc>,
        page_size: usize,
        after: Option<String>,
    ) -> Result<Self, TelemetryRepositoryError> {
        if from >= to || to.signed_duration_since(from) > MAX_TELEMETRY_RANGE {
            return Err(TelemetryRepositoryError::InvalidScanRequest {
                reason: "range must be non-empty and at most 366 days",
            });
        }
        if page_size == 0 || page_size > MAX_TELEMETRY_PAGE_SIZE {
            return Err(TelemetryRepositoryError::InvalidPageRequest {
                reason: "page size must be between 1 and 2000",
            });
        }
        if after
            .as_deref()
            .is_some_and(|cursor| cursor.len() > MAX_TELEMETRY_CURSOR_BYTES)
        {
            return Err(TelemetryRepositoryError::InvalidPageRequest {
                reason: "cursor exceeds maximum length",
            });
        }
        Ok(Self {
            // Persisted timestamps are microsecond-precision. Ceil the
            // inclusive lower bound so a sub-microsecond request cannot
            // include the preceding persisted timestamp, while retaining the
            // caller's exclusive upper bound for exact local filtering.
            from: ceil_timestamp(from)?,
            to,
            now: normalize_timestamp(now),
            page_size,
            after,
            include_partial: false,
            provider_id: None,
            effective_model_id: None,
        })
    }

    pub fn with_include_partial(mut self, include_partial: bool) -> Self {
        self.include_partial = include_partial;
        self
    }
    pub fn with_provider_id(mut self, provider_id: Option<ProviderId>) -> Self {
        self.provider_id = provider_id;
        self
    }
    pub fn with_effective_model_id(mut self, model_id: Option<EffectiveModelId>) -> Self {
        self.effective_model_id = model_id;
        self
    }
    pub fn from(&self) -> DateTime<Utc> {
        self.from
    }
    pub fn to(&self) -> DateTime<Utc> {
        self.to
    }
    pub fn now(&self) -> DateTime<Utc> {
        self.now
    }
    pub fn page_size(&self) -> usize {
        self.page_size
    }
    pub fn after(&self) -> Option<&str> {
        self.after.as_deref()
    }
    pub fn include_partial(&self) -> bool {
        self.include_partial
    }
    pub fn provider_id(&self) -> Option<&ProviderId> {
        self.provider_id.as_ref()
    }
    pub fn effective_model_id(&self) -> Option<&EffectiveModelId> {
        self.effective_model_id.as_ref()
    }
    pub fn effective_to(&self) -> DateTime<Utc> {
        if self.include_partial {
            self.to
        } else {
            self.to.min(crate::floor_utc_hour(self.now))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct WireActivity {
    schema_version: u16,
    tenant_id: String,
    window_start: String,
    user_id: String,
    origin_kind: String,
    run_count: u64,
    runs_with_reported_tool_calls_count: u64,
    tool_count_reported_run_count: u64,
    reported_tool_call_count: u64,
    completed_count: u64,
    failed_count: u64,
    cancelled_count: u64,
    recovery_required_count: u64,
    total_run_latency_ms: u64,
    first_observed_at: String,
    last_observed_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct WireModel {
    schema_version: u16,
    tenant_id: String,
    user_id: String,
    window_start: String,
    provider_id: String,
    effective_model_id: String,
    inference_count: u64,
    usage_reported_count: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_input_tokens: u64,
    cache_creation_input_tokens: u64,
    first_observed_at: String,
    last_observed_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct WireFailure {
    schema_version: u16,
    tenant_id: String,
    window_start: String,
    user_id: String,
    failure_category: String,
    failure_count: u64,
    first_observed_at: String,
    last_observed_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct WireAutomation {
    schema_version: u16,
    tenant_id: String,
    window_start: String,
    user_id: String,
    automation_kind: String,
    run_count: u64,
    completed_count: u64,
    failed_count: u64,
    cancelled_count: u64,
    recovery_required_count: u64,
    first_observed_at: String,
    last_observed_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct WireLifecycle {
    schema_version: u16,
    tenant_id: String,
    event_id: String,
    user_id: Option<String>,
    event_kind: String,
    subject_kind: String,
    subject_id: String,
    occurred_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct WireCoverage {
    schema_version: u16,
    tenant_id: String,
    window_start: String,
    collector_instance_id: String,
    accepted_observation_count: u64,
    queue_full_drop_count: u64,
    closed_drop_count: u64,
    invalid_drop_count: u64,
    write_failed_observation_count: u64,
    first_observed_at: String,
    last_observed_at: String,
}

fn json_error(operation: &'static str, source: serde_json::Error) -> TelemetryRepositoryError {
    TelemetryRepositoryError::Serialization { operation, source }
}

fn checked_add(
    left: u64,
    right: u64,
    family: &'static str,
) -> Result<u64, TelemetryRepositoryError> {
    left.checked_add(right)
        .filter(|value| *value <= ironclaw_telemetry_contracts::observation::MAX_DURABLE_COUNTER)
        .ok_or(TelemetryRepositoryError::CounterOverflow { family })
}

fn checked_counter(value: u64, family: &'static str) -> Result<u64, TelemetryRepositoryError> {
    if value <= ironclaw_telemetry_contracts::observation::MAX_DURABLE_COUNTER {
        Ok(value)
    } else {
        Err(TelemetryRepositoryError::CounterOverflow { family })
    }
}

fn parse_version(version: u16) -> Result<(), TelemetryRepositoryError> {
    if version == RECORD_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(TelemetryRepositoryError::UnknownEnum {
            field: "schema_version",
            value: version.to_string(),
        })
    }
}

fn scoped_path(raw: String) -> Result<ScopedPath, TelemetryRepositoryError> {
    ScopedPath::new(raw).map_err(|source| TelemetryRepositoryError::StorageOperation {
        operation: "constructing telemetry scoped path",
        source: Box::new(source),
    })
}

fn escape_component(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            escaped.push(char::from(byte));
        } else {
            escaped.push('%');
            escaped.push(char::from(b"0123456789ABCDEF"[(byte >> 4) as usize]));
            escaped.push(char::from(b"0123456789ABCDEF"[(byte & 0x0f) as usize]));
        }
    }
    escaped
}

fn path_for(
    family: &str,
    hour: DateTime<Utc>,
    parts: &[&str],
) -> Result<ScopedPath, TelemetryRepositoryError> {
    let mut raw = format!(
        "{TELEMETRY_PREFIX}/hourly/{family}/{}",
        timestamp_text(hour)
    );
    for part in parts {
        raw.push('/');
        raw.push_str(&escape_component(part));
    }
    raw.push_str(".json");
    scoped_path(raw)
}

fn lifecycle_path(event_id: &str) -> Result<ScopedPath, TelemetryRepositoryError> {
    scoped_path(format!(
        "{TELEMETRY_PREFIX}/lifecycle/{}.json",
        escape_component(event_id)
    ))
}

fn coverage_path(row: &CollectorCoverage) -> Result<ScopedPath, TelemetryRepositoryError> {
    path_for(
        FAMILY_COVERAGE,
        row.window_start(),
        &[row.collector_instance_id().as_str()],
    )
}

fn index_key(value: &'static str) -> Result<IndexKey, TelemetryRepositoryError> {
    IndexKey::new(value).map_err(|source| TelemetryRepositoryError::StorageOperation {
        operation: "constructing telemetry index key",
        source: Box::new(source),
    })
}

fn index_name(value: &'static str) -> Result<IndexName, TelemetryRepositoryError> {
    IndexName::new(value).map_err(|source| TelemetryRepositoryError::StorageOperation {
        operation: "constructing telemetry index name",
        source: Box::new(source),
    })
}

fn exact_index(
    name: &'static str,
    keys: &[&'static str],
) -> Result<IndexSpec, TelemetryRepositoryError> {
    Ok(IndexSpec::new(
        index_name(name)?,
        keys.iter()
            .map(|key| index_key(key))
            .collect::<Result<Vec<_>, _>>()?,
        IndexKind::Exact,
    ))
}

fn projection(
    family: &str,
    tenant: &str,
    window: DateTime<Utc>,
    tie: &str,
    provider: Option<&str>,
    model: Option<&str>,
) -> Result<BTreeMap<IndexKey, IndexValue>, TelemetryRepositoryError> {
    let mut values = BTreeMap::new();
    values.insert(index_key("tenant_id")?, IndexValue::Text(tenant.to_owned()));
    values.insert(
        index_key("record_family")?,
        IndexValue::Text(family.to_owned()),
    );
    values.insert(
        index_key("window_start")?,
        IndexValue::Text(timestamp_text(window)),
    );
    values.insert(index_key("tie_breaker")?, IndexValue::Text(tie.to_owned()));
    if let Some(provider) = provider {
        values.insert(
            index_key("provider_id")?,
            IndexValue::Text(provider.to_owned()),
        );
    }
    if let Some(model) = model {
        values.insert(
            index_key("effective_model_id")?,
            IndexValue::Text(model.to_owned()),
        );
    }
    Ok(values)
}

fn lifecycle_projection(
    tenant: &str,
    occurred_at: DateTime<Utc>,
    tie: &str,
) -> Result<BTreeMap<IndexKey, IndexValue>, TelemetryRepositoryError> {
    let mut values = BTreeMap::new();
    values.insert(index_key("tenant_id")?, IndexValue::Text(tenant.to_owned()));
    values.insert(
        index_key("record_family")?,
        IndexValue::Text(FAMILY_LIFECYCLE.to_owned()),
    );
    values.insert(
        index_key("occurred_at")?,
        IndexValue::Text(timestamp_text(occurred_at)),
    );
    values.insert(index_key("tie_breaker")?, IndexValue::Text(tie.to_owned()));
    Ok(values)
}

fn entry<W: Serialize>(
    kind: &'static str,
    wire: &W,
    indexed: BTreeMap<IndexKey, IndexValue>,
) -> Result<Entry, TelemetryRepositoryError> {
    let body = serde_json::to_vec(wire)
        .map_err(|source| json_error("encoding telemetry record", source))?;
    let kind = ironclaw_filesystem::RecordKind::new(kind).map_err(|source| {
        TelemetryRepositoryError::StorageOperation {
            operation: "constructing telemetry record kind",
            source: Box::new(source),
        }
    })?;
    Ok(Entry {
        body,
        content_type: ContentType::json(),
        kind: Some(kind),
        indexed,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryPage<T> {
    rows: Vec<T>,
    next_cursor: Option<String>,
}

impl<T> TelemetryPage<T> {
    pub(crate) fn new(rows: Vec<T>, next_cursor: Option<String>) -> Self {
        Self { rows, next_cursor }
    }

    pub fn rows(&self) -> &[T] {
        &self.rows
    }

    pub fn into_rows(self) -> Vec<T> {
        self.rows
    }

    pub fn next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_deref()
    }
}

pub struct FilesystemTelemetryRepository<F: ?Sized> {
    filesystem: Arc<ScopedFilesystem<F>>,
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;

    use super::{decode_cursor, encode_cursor, timestamp_text};
    use crate::TelemetryRepositoryError;

    #[test]
    fn cursor_round_trip_accepts_identifier_delimiters() {
        let timestamp = DateTime::parse_from_rfc3339("2026-08-26T10:00:00.123456789Z")
            .expect("test timestamp")
            .with_timezone(&chrono::Utc);
        let cursor = encode_cursor(timestamp, &["user|one", "provider|two", "model|three"]);

        let (decoded_timestamp, fields) = decode_cursor(&cursor, 3).expect("cursor round trip");

        assert_eq!(
            timestamp_text(decoded_timestamp),
            "2026-08-26T10:00:00.123456Z"
        );
        assert_eq!(fields, ["user|one", "provider|two", "model|three"]);
    }

    #[test]
    fn timestamp_text_normalizes_to_postgres_precision() {
        let timestamp = DateTime::parse_from_rfc3339("2026-08-26T10:00:00.123456789Z")
            .expect("test timestamp")
            .with_timezone(&chrono::Utc);

        assert_eq!(timestamp_text(timestamp), "2026-08-26T10:00:00.123456Z");
    }

    #[test]
    fn persisted_decode_errors_preserve_field_causes() {
        let error = super::decode_tenant_id(String::new()).expect_err("empty tenant");
        assert!(matches!(
            error,
            TelemetryRepositoryError::InvalidPersistedField {
                field: "tenant_id",
                ..
            }
        ));
        assert_eq!(error.to_string(), "invalid persisted telemetry tenant_id");
        assert!(std::error::Error::source(&error).is_some());

        let error = super::parse_origin("not-a-real-origin").expect_err("unknown origin");
        assert!(matches!(
            error,
            TelemetryRepositoryError::UnknownEnum {
                field: "origin_kind",
                ..
            }
        ));
        assert_eq!(error.to_string(), "unknown persisted telemetry origin_kind");
    }

    #[test]
    fn cursor_timestamp_parse_preserves_source() {
        let error = decode_cursor("3:bad", 0).expect_err("invalid cursor timestamp");
        assert!(matches!(
            error,
            TelemetryRepositoryError::InvalidTimestamp {
                field: "cursor timestamp",
                ..
            }
        ));
        assert_eq!(
            error.to_string(),
            "invalid persisted telemetry timestamp in cursor timestamp"
        );
        assert!(
            std::error::Error::source(&error)
                .and_then(|source| source.downcast_ref::<chrono::ParseError>())
                .is_some()
        );
    }

    #[test]
    fn cursor_length_parse_preserves_source() {
        let error = decode_cursor("x:payload", 0).expect_err("invalid cursor length");
        assert!(matches!(
            error,
            TelemetryRepositoryError::InvalidCursorLength { ref value, .. } if value == "x"
        ));
        assert_eq!(error.to_string(), "invalid telemetry page cursor length");
        assert!(
            std::error::Error::source(&error)
                .and_then(|source| source.downcast_ref::<std::num::ParseIntError>())
                .is_some()
        );
    }

    #[test]
    fn cursor_payload_utf8_preserves_source() {
        let error = decode_cursor("1:\u{e9}", 1).expect_err("invalid cursor payload");
        assert!(matches!(
            error,
            TelemetryRepositoryError::InvalidCursorEncoding { .. }
        ));
        assert_eq!(error.to_string(), "invalid telemetry page cursor encoding");
        assert!(
            std::error::Error::source(&error)
                .and_then(|source| source.downcast_ref::<std::string::FromUtf8Error>())
                .is_some()
        );
    }

    #[test]
    fn persisted_counter_conversion_preserves_source_and_value() {
        let error = super::checked_counter_sum(-1, 0, "persisted").expect_err("negative counter");
        assert!(matches!(
            error,
            TelemetryRepositoryError::CounterConversion {
                family: "persisted",
                value: -1,
                ..
            }
        ));
        assert_eq!(
            error.to_string(),
            "telemetry counter conversion failed for persisted row"
        );
        assert!(
            std::error::Error::source(&error)
                .and_then(|source| source.downcast_ref::<std::num::TryFromIntError>())
                .is_some()
        );
    }
}
