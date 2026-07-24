//! Scheduled trigger domain contracts for IronClaw Reborn.
//!
//! This crate owns trigger records, source-provider evaluation, deterministic
//! fire identity, trusted poller call sites, and in-memory test behavior. Poller
//! lifecycle wiring, first-party capabilities, and outbound delivery are owned
//! by later slices.

use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use chrono_tz::Tz;
use cron::Schedule;
use ironclaw_common::{AutomationName, AutomationNameError};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, Timestamp, UserId};
use ironclaw_turns::TurnRunId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use ulid::Ulid;
mod in_memory;
mod libsql;
mod postgres;
mod trusted_submit;
mod worker;

pub use ironclaw_host_api::OutboundDeliveryTargetId as TriggerDeliveryTargetId;
pub use trusted_submit::{
    TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
    TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerMaterializedPrompt,
    TriggerTrustedInboundBinding, is_trusted_trigger_adapter_kind,
};

const MIN_FIRE_CADENCE: Duration = Duration::from_secs(60);
const MAX_DUE_TRIGGER_POLL_LIMIT: usize = 128;
const MAX_TRIGGER_LIST_LIMIT: usize = 100;
const MAX_TRIGGER_RUN_HISTORY_LIMIT: usize = 500;
const MAX_TRIGGER_RUN_HISTORY_RETAINED: usize = 500;
pub const MAX_TRIGGER_NAME_BYTES: usize = ironclaw_common::MAX_AUTOMATION_NAME_BYTES;
pub const MAX_TRIGGER_PROMPT_BYTES: usize = 32 * 1024;
const IDENTITY_VERSION_LABEL: &str = "ironclaw.trigger-fire.v1";
const ROUTE_THREAD_DOMAIN: &str = "route-thread";
const EXTERNAL_EVENT_DOMAIN: &str = "external-event";

#[derive(Debug, Error)]
pub enum TriggerError {
    #[error("invalid trigger id: {reason}")]
    InvalidTriggerId { reason: String },
    #[error("invalid fire identity component {label}: {reason}")]
    InvalidFireIdentityComponent { label: String, reason: String },
    #[error("invalid trigger record: {reason}")]
    InvalidRecord {
        kind: TriggerRecordValidationKind,
        reason: String,
    },
    #[error("invalid trigger poller configuration: {reason}")]
    InvalidPollerConfig { reason: String },
    #[error("invalid schedule: {reason}")]
    InvalidSchedule {
        kind: TriggerScheduleValidationKind,
        reason: String,
    },
    #[error("invalid trigger materialization: {reason}")]
    InvalidMaterialization { reason: String },
    #[error("trigger materialization blocked: {reason}")]
    BlockedMaterialization { reason: String },
    #[error("trigger repository backend unavailable: {reason}")]
    Backend { reason: String },
    #[error("trigger not found")]
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TriggerRecordValidationKind {
    NameEmpty,
    NameTooLong,
    PromptEmpty,
    PromptTooLong,
    DeliveryTargetInvalid,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TriggerScheduleValidationKind {
    InvalidTimezone,
    InvalidDateTime,
    AmbiguousDateTime,
    NonexistentDateTime,
    EmptyCronExpression,
    InvalidCronFieldCount,
    InvalidCronExpression,
    SecondLevelCadence,
    NoUpcomingFireTime,
    SubMinuteCadence,
    NoFutureFireTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TriggerId(Ulid);

impl TriggerId {
    pub fn new() -> Self {
        Self(Ulid::new())
    }

    pub fn parse(value: &str) -> Result<Self, TriggerError> {
        Ulid::from_str(value)
            .map(Self)
            .map_err(|error| TriggerError::InvalidTriggerId {
                reason: error.to_string(),
            })
    }
}

impl Default for TriggerId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TriggerId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TriggerRouteThreadId(String);

impl TriggerRouteThreadId {
    pub fn new(value: impl Into<String>) -> Result<Self, TriggerError> {
        Self::try_from(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn new_unchecked(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl AsRef<str> for TriggerRouteThreadId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for TriggerRouteThreadId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for TriggerRouteThreadId {
    type Error = TriggerError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate_lower_hex_identifier("route thread id", value).map(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TriggerExternalEventId(String);

impl TriggerExternalEventId {
    pub fn new(value: impl Into<String>) -> Result<Self, TriggerError> {
        Self::try_from(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn new_unchecked(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl AsRef<str> for TriggerExternalEventId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for TriggerExternalEventId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for TriggerExternalEventId {
    type Error = TriggerError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate_lower_hex_identifier("external event id", value).map(Self)
    }
}

/// Opaque reference to materialized trigger prompt content.
///
/// Values must be non-empty, at most 512 bytes, and free of control
/// characters. The concrete content store is owned by composition.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TriggerInboundContentRef(String);

impl TriggerInboundContentRef {
    /// Create a validated inbound content reference.
    ///
    /// Validation is byte-based: the value must be non-empty, at most 512
    /// bytes, and free of control characters.
    pub fn new(value: impl Into<String>) -> Result<Self, TriggerError> {
        let value = value.into();
        validate_inbound_content_ref(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for TriggerInboundContentRef {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for TriggerInboundContentRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for TriggerInboundContentRef {
    type Error = TriggerError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate_inbound_content_ref(&value)?;
        Ok(Self(value))
    }
}

impl Serialize for TriggerInboundContentRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for TriggerInboundContentRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

/// Validate an opaque outbound target at the trigger record boundary.
///
/// The identifier itself is the neutral host-API type used by outbound
/// inventory and mediated routing. This helper is the only trigger-owned
/// adaptation: it preserves the trigger repository's stable error taxonomy.
pub fn parse_trigger_delivery_target_id(
    value: impl Into<String>,
) -> Result<TriggerDeliveryTargetId, TriggerError> {
    TriggerDeliveryTargetId::new(value).map_err(|reason| TriggerError::InvalidRecord {
        kind: TriggerRecordValidationKind::DeliveryTargetInvalid,
        reason,
    })
}

impl Serialize for TriggerRouteThreadId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for TriggerRouteThreadId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

impl Serialize for TriggerExternalEventId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for TriggerExternalEventId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerRecord {
    pub trigger_id: TriggerId,
    pub tenant_id: TenantId,
    pub creator_user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub name: String,
    pub source: TriggerSourceKind,
    pub schedule: TriggerSchedule,
    pub prompt: String,
    /// Optional per-trigger outbound delivery target. When set, fires deliver
    /// their results to this target instead of the creator's user-global
    /// outbound delivery preference, so one automation's routing cannot be
    /// clobbered by another automation (or a later preference change).
    #[serde(default)]
    pub delivery_target: Option<TriggerDeliveryTargetId>,
    pub state: TriggerState,
    pub next_run_at: Timestamp,
    pub last_run_at: Option<Timestamp>,
    pub last_fired_slot: Option<Timestamp>,
    pub last_status: Option<TriggerRunStatus>,
    pub active_fire_slot: Option<Timestamp>,
    pub active_run_ref: Option<TurnRunId>,
    pub created_at: Timestamp,
}

impl TriggerRecord {
    pub fn validate(&self) -> Result<(), TriggerError> {
        validate_trigger_name(&self.name)?;
        if self.prompt.trim().is_empty() {
            return Err(TriggerError::InvalidRecord {
                kind: TriggerRecordValidationKind::PromptEmpty,
                reason: "trigger prompt must not be empty".to_string(),
            });
        }
        if self.prompt.len() > MAX_TRIGGER_PROMPT_BYTES {
            return Err(TriggerError::InvalidRecord {
                kind: TriggerRecordValidationKind::PromptTooLong,
                reason: format!("trigger prompt must be at most {MAX_TRIGGER_PROMPT_BYTES} bytes"),
            });
        }
        if self.active_run_ref.is_some() && self.active_fire_slot.is_none() {
            return Err(TriggerError::InvalidRecord {
                kind: TriggerRecordValidationKind::Other,
                reason: "active_run_ref requires active_fire_slot".to_string(),
            });
        }
        self.schedule.validate()?;
        Ok(())
    }

    pub fn is_due_at(&self, now: Timestamp) -> bool {
        self.state == TriggerState::Scheduled && self.next_run_at <= now
    }

    pub fn has_active_fire(&self) -> bool {
        self.active_fire_slot.is_some() || self.active_run_ref.is_some()
    }
}

pub(crate) fn validate_trigger_name(name: &str) -> Result<(), TriggerError> {
    AutomationName::new(name.to_string())
        .map(|_| ())
        .map_err(trigger_name_error)
}

fn trigger_name_error(error: AutomationNameError) -> TriggerError {
    let (kind, reason) = match error {
        AutomationNameError::Empty => (
            TriggerRecordValidationKind::NameEmpty,
            "trigger name must not be empty".to_string(),
        ),
        AutomationNameError::TooLong => (
            TriggerRecordValidationKind::NameTooLong,
            format!("trigger name must be at most {MAX_TRIGGER_NAME_BYTES} bytes"),
        ),
    };
    TriggerError::InvalidRecord { kind, reason }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TriggerSchedule {
    Cron {
        expression: String,
        timezone: String,
    },
    Once {
        at: chrono::DateTime<chrono::Utc>,
        timezone: String,
    },
}

impl TriggerSchedule {
    /// Create a cron schedule evaluated in UTC.
    pub fn cron(expression: impl Into<String>) -> Result<Self, TriggerError> {
        Self::cron_with_timezone(expression, "UTC")
    }

    /// Create a cron schedule evaluated in the given IANA timezone.
    pub fn cron_with_timezone(
        expression: impl Into<String>,
        timezone: impl Into<String>,
    ) -> Result<Self, TriggerError> {
        let schedule = Self::Cron {
            expression: expression.into(),
            timezone: timezone.into(),
        };
        schedule.validate()?;
        Ok(schedule)
    }

    /// Create a one-shot schedule that fires at the given UTC timestamp.
    pub fn once(
        at: chrono::DateTime<chrono::Utc>,
        timezone: impl Into<String>,
    ) -> Result<Self, TriggerError> {
        let schedule = Self::Once {
            at,
            timezone: timezone.into(),
        };
        schedule.validate()?;
        Ok(schedule)
    }
    pub(crate) fn timezone_text(&self) -> &str {
        match self {
            Self::Cron { timezone, .. } | Self::Once { timezone, .. } => timezone.as_str(),
        }
    }

    // Returns (kind, expression, schedule_at)
    pub(crate) fn to_storage(&self) -> (&'static str, &str, Option<String>) {
        match self {
            Self::Cron { expression, .. } => ("cron", expression.as_str(), None),
            Self::Once { at, .. } => (
                "once",
                "",
                Some(at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)),
            ),
        }
    }
    pub(crate) fn from_storage(
        kind: &str,
        expression: &str,
        schedule_at: Option<&str>,
        timezone: &str,
    ) -> Result<Self, TriggerError> {
        match kind {
            "cron" => Self::cron_with_timezone(expression, timezone),
            "once" => {
                let at_str = schedule_at.ok_or_else(|| TriggerError::InvalidRecord {
                    kind: TriggerRecordValidationKind::Other,
                    reason: "schedule_at: missing once timestamp".to_string(),
                })?;
                let at = chrono::DateTime::parse_from_rfc3339(at_str)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .map_err(|error| TriggerError::InvalidRecord {
                        kind: TriggerRecordValidationKind::Other,
                        reason: format!("schedule_at: invalid once timestamp: {error}"),
                    })?;
                Self::once(at, timezone)
            }
            other => Err(TriggerError::InvalidRecord {
                kind: TriggerRecordValidationKind::Other,
                reason: format!("schedule_kind: unsupported schedule kind `{other}`"),
            }),
        }
    }

    pub fn is_recurring(&self) -> bool {
        matches!(self, Self::Cron { .. })
    }

    /// Parse a naive wall-clock datetime string, attach the given IANA timezone,
    /// and convert to UTC. Rejects ambiguous (DST fall-back overlap) and
    /// non-existent (DST spring-forward gap) local times with InvalidSchedule.
    pub fn once_from_local(at_str: &str, timezone_str: &str) -> Result<Self, TriggerError> {
        use chrono::TimeZone as _;
        let tz = parse_timezone(timezone_str)?;
        let naive = chrono::NaiveDateTime::parse_from_str(at_str, "%Y-%m-%dT%H:%M:%S").map_err(
            |error| TriggerError::InvalidSchedule {
                kind: TriggerScheduleValidationKind::InvalidDateTime,
                reason: format!("invalid datetime '{at_str}': {error}"),
            },
        )?;
        let local = tz.from_local_datetime(&naive);
        let at = match local {
            chrono::LocalResult::Single(dt) => dt.with_timezone(&chrono::Utc),
            chrono::LocalResult::Ambiguous(_, _) => {
                return Err(TriggerError::InvalidSchedule {
                    kind: TriggerScheduleValidationKind::AmbiguousDateTime,
                    reason: format!(
                        "datetime '{at_str}' is ambiguous in timezone '{timezone_str}' (DST overlap); use an explicit UTC offset"
                    ),
                });
            }
            chrono::LocalResult::None => {
                return Err(TriggerError::InvalidSchedule {
                    kind: TriggerScheduleValidationKind::NonexistentDateTime,
                    reason: format!(
                        "datetime '{at_str}' does not exist in timezone '{timezone_str}' (DST gap); use an adjacent time"
                    ),
                });
            }
        };
        Self::once(at, timezone_str)
    }

    pub fn validate(&self) -> Result<(), TriggerError> {
        match self {
            Self::Cron {
                expression,
                timezone,
            } => {
                parse_timezone(timezone)?;
                parse_cron_schedule(expression)?;
                Ok(())
            }
            Self::Once { timezone, .. } => {
                parse_timezone(timezone)?;
                Ok(())
            }
        }
    }

    pub fn next_slot_after(&self, after: Timestamp) -> Result<Option<Timestamp>, TriggerError> {
        match self {
            Self::Cron {
                expression,
                timezone,
            } => {
                let tz = parse_timezone(timezone)?;
                let schedule = parse_cron_schedule(expression)?;
                Ok(next_cron_slot_after(&schedule, &tz, after))
            }
            Self::Once { at, .. } => Ok(if *at > after { Some(*at) } else { None }),
        }
    }

    /// Count elapsed schedule slots in `(after, now]` — not runs the poller
    /// attempted or skipped, just cron occurrences that have passed (#5886).
    /// Display-only derivation; stops at `cap` and reports the truncation so
    /// UIs can render "99+" instead of a false-exact count. `Once` schedules
    /// have no repeat slots to count.
    pub fn elapsed_occurrences_between(
        &self,
        after: Timestamp,
        now: Timestamp,
        cap: u32,
    ) -> Result<ElapsedOccurrenceCount, TriggerError> {
        let (expression, timezone) = match self {
            Self::Once { .. } => {
                return Ok(ElapsedOccurrenceCount {
                    count: 0,
                    capped: false,
                });
            }
            Self::Cron {
                expression,
                timezone,
            } => (expression, timezone),
        };
        // Parse the timezone and cron schedule once up front: the loop below
        // can call next_slot_after up to `cap` times, and re-parsing the same
        // loop-invariant schedule/timezone on every iteration is wasted work
        // (#5886 follow-up).
        let tz = parse_timezone(timezone)?;
        let schedule = parse_cron_schedule(expression)?;

        let mut count = 0u32;
        let mut cursor = after;
        while count < cap {
            match next_cron_slot_after(&schedule, &tz, cursor) {
                Some(slot) if slot <= now => {
                    count += 1;
                    cursor = slot;
                }
                _ => {
                    return Ok(ElapsedOccurrenceCount {
                        count,
                        capped: false,
                    });
                }
            }
        }
        let capped =
            matches!(next_cron_slot_after(&schedule, &tz, cursor), Some(slot) if slot <= now);
        Ok(ElapsedOccurrenceCount { count, capped })
    }
}

/// Cron-branch core of [`TriggerSchedule::next_slot_after`], factored to
/// accept an already-parsed `Schedule`/`Tz` so callers that need repeated
/// lookups (e.g. [`TriggerSchedule::elapsed_occurrences_between`]'s loop) can
/// parse once and reuse it instead of re-parsing the schedule string every
/// call.
fn next_cron_slot_after(schedule: &Schedule, tz: &Tz, after: Timestamp) -> Option<Timestamp> {
    if *tz == Tz::UTC {
        schedule.after(&after).next()
    } else {
        schedule
            .after(&after.with_timezone(tz))
            .next()
            .map(|dt| dt.with_timezone(&Utc))
    }
}

/// Result of [`TriggerSchedule::elapsed_occurrences_between`]: `capped` marks
/// a truncated count (the real number of elapsed occurrences exceeds
/// `count`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElapsedOccurrenceCount {
    pub count: u32,
    pub capped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerSourceKind {
    Schedule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerState {
    Scheduled,
    Paused,
    Completed,
}

fn validate_user_settable_trigger_state(state: TriggerState) -> Result<(), TriggerError> {
    match state {
        TriggerState::Scheduled | TriggerState::Paused => Ok(()),
        TriggerState::Completed => Err(TriggerError::InvalidRecord {
            kind: TriggerRecordValidationKind::Other,
            reason: "completed is a terminal trigger state and cannot be set directly".to_string(),
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerRunStatus {
    Ok,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerRunHistoryStatus {
    Running,
    Ok,
    Error,
}
pub(crate) fn trigger_run_history_status_text(value: TriggerRunHistoryStatus) -> &'static str {
    match value {
        TriggerRunHistoryStatus::Running => "running",
        TriggerRunHistoryStatus::Ok => "ok",
        TriggerRunHistoryStatus::Error => "error",
    }
}
pub(crate) fn state_text_codec(value: TriggerState) -> &'static str {
    match value {
        TriggerState::Scheduled => "scheduled",
        TriggerState::Paused => "paused",
        TriggerState::Completed => "completed",
    }
}
pub(crate) fn parse_state_codec(value: &str) -> Result<TriggerState, TriggerError> {
    match value {
        "scheduled" => Ok(TriggerState::Scheduled),
        "paused" => Ok(TriggerState::Paused),
        "completed" => Ok(TriggerState::Completed),
        other => Err(TriggerError::InvalidRecord {
            kind: TriggerRecordValidationKind::Other,
            reason: format!("state: unsupported trigger state `{other}`"),
        }),
    }
}
pub(crate) fn source_kind_text_codec(value: TriggerSourceKind) -> &'static str {
    match value {
        TriggerSourceKind::Schedule => "schedule",
    }
}
pub(crate) fn parse_source_kind_codec(value: &str) -> Result<TriggerSourceKind, TriggerError> {
    match value {
        "schedule" => Ok(TriggerSourceKind::Schedule),
        other => Err(TriggerError::InvalidRecord {
            kind: TriggerRecordValidationKind::Other,
            reason: format!("source: unsupported trigger source `{other}`"),
        }),
    }
}
pub(crate) fn status_text_codec(value: TriggerRunStatus) -> &'static str {
    match value {
        TriggerRunStatus::Ok => "ok",
        TriggerRunStatus::Error => "error",
    }
}
pub(crate) fn parse_run_status_codec(value: &str) -> Result<TriggerRunStatus, TriggerError> {
    match value {
        "ok" => Ok(TriggerRunStatus::Ok),
        "error" => Ok(TriggerRunStatus::Error),
        other => Err(TriggerError::InvalidRecord {
            kind: TriggerRecordValidationKind::Other,
            reason: format!("last_status: unsupported trigger run status `{other}`"),
        }),
    }
}
pub(crate) fn parse_run_history_status_codec(
    value: &str,
) -> Result<TriggerRunHistoryStatus, TriggerError> {
    match value {
        "running" => Ok(TriggerRunHistoryStatus::Running),
        "ok" => Ok(TriggerRunHistoryStatus::Ok),
        "error" => Ok(TriggerRunHistoryStatus::Error),
        other => Err(TriggerError::InvalidRecord {
            kind: TriggerRecordValidationKind::Other,
            reason: format!("status: unsupported trigger run history status `{other}`"),
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerRunRecord {
    pub tenant_id: TenantId,
    pub trigger_id: TriggerId,
    pub fire_slot: Timestamp,
    pub run_id: Option<TurnRunId>,
    /// Canonical thread id for this run, or `None` if no canonical conversation
    /// thread has been established yet.
    ///
    /// `None` is the initial state for claim-time rows and for runs that fail
    /// before fire acceptance. `Some(canonical_uuid)` is set by
    /// [`TriggerRepository::mark_fire_accepted`] (and optionally by
    /// [`TriggerRepository::mark_fire_replayed`] when the replayed outcome
    /// carries a canonical thread id). Only `Some` values correspond to a live
    /// chat thread that the WebUI panel can open.
    pub thread_id: Option<ThreadId>,
    pub status: TriggerRunHistoryStatus,
    pub submitted_at: Timestamp,
    pub completed_at: Option<Timestamp>,
}

impl TriggerRunRecord {
    /// Create a "running" run record with no canonical thread id yet.
    ///
    /// The `thread_id` field will be populated with the canonical UUID at
    /// fire-acceptance time via [`TriggerRepository::mark_fire_accepted`].
    /// Rows that never reach acceptance (e.g. pre-submit failures) retain
    /// `None` — the WebUI panel must not render a chat link for them.
    fn running(
        tenant_id: TenantId,
        trigger_id: TriggerId,
        fire_slot: Timestamp,
        run_id: Option<TurnRunId>,
        submitted_at: Timestamp,
    ) -> Self {
        Self {
            tenant_id,
            trigger_id,
            fire_slot,
            run_id,
            thread_id: None,
            status: TriggerRunHistoryStatus::Running,
            submitted_at,
            completed_at: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerFireIdentity {
    pub tenant_id: TenantId,
    pub trigger_id: TriggerId,
    pub fire_slot: Timestamp,
    pub route_thread_id: TriggerRouteThreadId,
    pub external_event_id: TriggerExternalEventId,
}

impl TriggerFireIdentity {
    pub fn new(tenant_id: TenantId, trigger_id: TriggerId, fire_slot: Timestamp) -> Self {
        let route_thread_id = TriggerRouteThreadId::new_unchecked(derive_fire_digest(
            ROUTE_THREAD_DOMAIN,
            &tenant_id,
            trigger_id,
            fire_slot,
        ));
        let external_event_id = TriggerExternalEventId::new_unchecked(derive_fire_digest(
            EXTERNAL_EVENT_DOMAIN,
            &tenant_id,
            trigger_id,
            fire_slot,
        ));
        Self {
            tenant_id,
            trigger_id,
            fire_slot,
            route_thread_id,
            external_event_id,
        }
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn trigger_id(&self) -> TriggerId {
        self.trigger_id
    }

    pub fn fire_slot(&self) -> Timestamp {
        self.fire_slot
    }

    pub fn route_thread_id(&self) -> &TriggerRouteThreadId {
        &self.route_thread_id
    }

    pub fn external_event_id(&self) -> &TriggerExternalEventId {
        &self.external_event_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerFire {
    pub identity: TriggerFireIdentity,
    pub creator_user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub prompt: String,
    /// Per-trigger outbound delivery target carried from the record so the
    /// delivery layer can honor it without re-reading the trigger row.
    #[serde(default)]
    pub delivery_target: Option<TriggerDeliveryTargetId>,
}

#[async_trait]
pub trait TriggerPromptMaterializer: Send + Sync {
    async fn materialize_prompt(
        &self,
        fire: TriggerFire,
    ) -> Result<TriggerMaterializedPrompt, TriggerError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimDueFireRequest {
    pub tenant_id: TenantId,
    pub trigger_id: TriggerId,
    pub fire_slot: Timestamp,
    pub now: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedTriggerFire {
    pub record: TriggerRecord,
    pub fire_slot: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimDueFireOutcome {
    Claimed(ClaimedTriggerFire),
    NotFound,
    NotDue {
        record: TriggerRecord,
    },
    AlreadyActive {
        active_fire_slot: Option<Timestamp>,
        active_run_ref: Option<TurnRunId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FireAcceptedRequest {
    pub tenant_id: TenantId,
    pub trigger_id: TriggerId,
    pub fire_slot: Timestamp,
    pub run_id: TurnRunId,
    /// Canonical thread id minted by the conversation binding layer for the
    /// accepted run. Persisted into the run-history row so the WebUI Automations
    /// panel can open the correct chat thread from `recent_runs[].thread_id`.
    pub thread_id: ThreadId,
    pub submitted_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FireReplayedRequest {
    pub tenant_id: TenantId,
    pub trigger_id: TriggerId,
    pub fire_slot: Timestamp,
    pub original_run_id: TurnRunId,
    /// Canonical thread id for the replayed fire, if one is known.
    ///
    /// The submission path resolves conversation binding before determining
    /// whether a fire is new or replayed, so the replayed outcome can carry
    /// the canonical `ThreadId` (UUID). `None` means the submission path
    /// did not resolve a canonical thread — the run-history row will have
    /// no chat link.
    pub thread_id: Option<ThreadId>,
    pub replayed_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FireRetryableFailedRequest {
    pub tenant_id: TenantId,
    pub trigger_id: TriggerId,
    pub fire_slot: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirePermanentFailedRequest {
    pub tenant_id: TenantId,
    pub trigger_id: TriggerId,
    pub fire_slot: Timestamp,
    pub next_run_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FireTerminalFailedRequest {
    pub tenant_id: TenantId,
    pub trigger_id: TriggerId,
    pub fire_slot: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClearActiveFireRequest {
    pub tenant_id: TenantId,
    pub trigger_id: TriggerId,
    pub fire_slot: Timestamp,
    pub run_id: TurnRunId,
    pub status: TriggerRunHistoryStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveTriggerScanCursor {
    active_fire_slot: Timestamp,
    tenant_id: TenantId,
    trigger_id: TriggerId,
}

impl ActiveTriggerScanCursor {
    pub fn from_active_record(record: &TriggerRecord) -> Option<Self> {
        Some(Self {
            active_fire_slot: record.active_fire_slot?,
            tenant_id: record.tenant_id.clone(),
            trigger_id: record.trigger_id,
        })
    }

    pub fn active_fire_slot(&self) -> Timestamp {
        self.active_fire_slot
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn trigger_id(&self) -> TriggerId {
        self.trigger_id
    }
}

#[async_trait]
pub trait TriggerSourceProvider: Send + Sync {
    async fn evaluate(
        &self,
        record: &TriggerRecord,
        now: Timestamp,
    ) -> Result<Option<TriggerFire>, TriggerError>;
}

#[derive(Debug, Default, Clone)]
pub struct ScheduleTriggerSourceProvider;

#[async_trait]
impl TriggerSourceProvider for ScheduleTriggerSourceProvider {
    async fn evaluate(
        &self,
        record: &TriggerRecord,
        now: Timestamp,
    ) -> Result<Option<TriggerFire>, TriggerError> {
        record.validate()?;
        if record.source != TriggerSourceKind::Schedule || !record.is_due_at(now) {
            return Ok(None);
        }
        let identity = TriggerFireIdentity::new(
            record.tenant_id.clone(),
            record.trigger_id,
            record.next_run_at,
        );
        Ok(Some(TriggerFire {
            identity,
            creator_user_id: record.creator_user_id.clone(),
            agent_id: record.agent_id.clone(),
            project_id: record.project_id.clone(),
            prompt: record.prompt.clone(),
            delivery_target: record.delivery_target.clone(),
        }))
    }
}

#[async_trait]
pub trait TriggerRepository: Send + Sync {
    async fn upsert_trigger(&self, record: TriggerRecord) -> Result<(), TriggerError>;

    async fn get_trigger(
        &self,
        tenant_id: TenantId,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError>;

    /// Returns all triggers for a tenant in creation order.
    ///
    /// This method is currently unbounded. Callers must apply any product or
    /// API pagination before exposing user-facing list surfaces.
    async fn list_triggers(&self, tenant_id: TenantId) -> Result<Vec<TriggerRecord>, TriggerError>;

    /// Returns caller-scoped triggers in creation order, capped for user-facing surfaces.
    ///
    /// `excluded_states` is a slice of states to omit from the result. Pass
    /// `&[]` to include all states (model-facing paths) or
    /// `&[TriggerState::Completed]` to exclude soft-completed one-shots from
    /// user-facing panels.
    async fn list_scoped_triggers(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        limit: usize,
        excluded_states: &[TriggerState],
    ) -> Result<Vec<TriggerRecord>, TriggerError>;

    async fn remove_trigger(
        &self,
        tenant_id: TenantId,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError>;

    /// Removes a trigger only when the full caller scope matches the stored record.
    async fn remove_scoped_trigger(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError>;

    /// Sets the lifecycle state for a caller-scoped trigger.
    ///
    /// This user-facing mutation may only set non-terminal states
    /// (`Scheduled` or `Paused`). A stored `Completed` trigger is terminal and
    /// must not be moved back into the scheduler by this method; implementations
    /// return `Ok(None)` for that case so callers do not leak trigger existence
    /// across invalid lifecycle transitions.
    async fn set_scoped_trigger_state(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        trigger_id: TriggerId,
        state: TriggerState,
    ) -> Result<Option<TriggerRecord>, TriggerError>;

    /// Renames a trigger only when the full caller scope matches the stored record.
    ///
    /// This user-facing mutation updates only the human-readable label. It
    /// must not alter schedule state, active-fire metadata, run history, or
    /// prompt content.
    async fn rename_scoped_trigger(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _trigger_id: TriggerId,
        _name: AutomationName,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Err(TriggerError::Backend {
            reason: "rename_scoped_trigger not implemented by this repository".to_string(),
        })
    }

    /// Lists due triggers across all tenants for the trusted poller path.
    ///
    /// # Safety / Authorization
    ///
    /// This is a global repository query and must not be surfaced as a
    /// tenant-scoped or user-facing capability. Host-owned poller code should
    /// keep this call on explicit worker-local trusted poller call sites so the
    /// trust boundary remains visible.
    async fn list_due_triggers(
        &self,
        now: Timestamp,
        limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError>;

    /// Lists active trigger fires across all tenants for trusted poller cleanup.
    ///
    /// # Safety / Authorization
    ///
    /// This is a global repository query and must not be surfaced as a
    /// tenant-scoped or user-facing capability. Host-owned poller code should
    /// keep this call on explicit worker-local trusted poller call sites so the
    /// trust boundary remains visible.
    async fn list_active_triggers(&self, limit: usize) -> Result<Vec<TriggerRecord>, TriggerError>;

    /// Lists active trigger fires after a previous scan cursor.
    ///
    /// # Safety / Authorization
    ///
    /// This has the same trusted-poller-only authorization constraints as
    /// [`TriggerRepository::list_active_triggers`]. The cursor must be derived
    /// from a previous trusted active scan result, not from user input.
    ///
    /// Cursor pagination is required for every repository implementation so the
    /// poller cannot advance successfully on the first tick and then fail when
    /// it resumes from a stored cursor.
    async fn list_active_triggers_after(
        &self,
        after: Option<ActiveTriggerScanCursor>,
        limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError>;

    async fn claim_due_fire(
        &self,
        request: ClaimDueFireRequest,
    ) -> Result<ClaimDueFireOutcome, TriggerError>;

    async fn mark_fire_accepted(
        &self,
        request: FireAcceptedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError>;

    async fn mark_fire_replayed(
        &self,
        request: FireReplayedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError>;

    async fn mark_fire_retryable_failed(
        &self,
        request: FireRetryableFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError>;

    async fn mark_fire_permanently_failed(
        &self,
        request: FirePermanentFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError>;

    /// Marks a trusted poller-owned claimed fire as terminally failed.
    ///
    /// # Safety / Authorization
    ///
    /// This clears active-fire state and completes the trigger when a claimed
    /// fire cannot advance to another schedule slot. Callers must derive the
    /// tenant, trigger id, and fire slot from a trusted claimed record, not from
    /// user input or a tenant-scoped list path.
    async fn mark_fire_terminally_failed(
        &self,
        request: FireTerminalFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError>;

    async fn clear_active_fire(
        &self,
        request: ClearActiveFireRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError>;

    /// Looks up the run-history row and its parent trigger by `thread_id`.
    ///
    /// Returns `Some((trigger_record, run_record))` when a run with the given
    /// `thread_id` exists for the tenant, `None` when no match is found.
    ///
    /// # Authorization
    ///
    /// This method performs a pure storage lookup with **no authorization
    /// filtering**. The caller is responsible for applying any caller-visibility
    /// or scope predicate before acting on the returned record (e.g., checking
    /// that the trigger belongs to the expected `creator_user_id`, `agent_id`,
    /// and `project_id`).
    ///
    /// Required (no default body): this lookup feeds the authorization path
    /// for opening trigger-owned threads from the Automations panel. A
    /// silently inherited `Ok(None)` would degrade every timeline/SSE/gate/
    /// cancel access check to 404 on a backend that forgot to implement it.
    async fn find_trigger_run_by_thread_id(
        &self,
        tenant_id: TenantId,
        thread_id: &ThreadId,
    ) -> Result<Option<(TriggerRecord, TriggerRunRecord)>, TriggerError>;

    /// Returns recent run-history rows for one tenant-scoped trigger.
    ///
    /// Rows are ordered newest first by fire slot. Implementations must clamp
    /// the caller-provided limit to the repository maximum, and a limit of zero
    /// must return an empty list without touching storage.
    async fn list_trigger_run_history(
        &self,
        _tenant_id: TenantId,
        _trigger_id: TriggerId,
        _limit: usize,
    ) -> Result<Vec<TriggerRunRecord>, TriggerError> {
        Ok(Vec::new())
    }

    /// Returns recent run-history rows for several tenant-scoped triggers.
    ///
    /// Each entry is ordered newest first by fire slot and truncated to `limit`.
    ///
    /// The default implementation is a non-production fallback: it issues one
    /// serial [`list_trigger_run_history`] call per trigger id and logs when it
    /// is exercised. Storage-backed repositories used by list-page or UI paths
    /// must override this with a true batch query so callers do not
    /// accidentally introduce N sequential round-trips.
    async fn list_trigger_run_history_batch(
        &self,
        tenant_id: TenantId,
        trigger_ids: &[TriggerId],
        limit: usize,
    ) -> Result<HashMap<TriggerId, Vec<TriggerRunRecord>>, TriggerError> {
        let mut runs_by_trigger = HashMap::with_capacity(trigger_ids.len());
        if limit == 0 {
            return Ok(runs_by_trigger);
        }
        if !trigger_ids.is_empty() {
            tracing::warn!(
                trigger_count = trigger_ids.len(),
                "default trigger run-history batch fallback is issuing serial per-trigger lookups"
            );
        }
        for trigger_id in trigger_ids {
            runs_by_trigger.insert(
                *trigger_id,
                self.list_trigger_run_history(tenant_id.clone(), *trigger_id, limit)
                    .await?,
            );
        }
        Ok(runs_by_trigger)
    }
}

/// Feature-gated durable libSQL repository type for composition/test wiring.
pub use libsql::LibSqlTriggerRepository;
/// Feature-gated durable PostgreSQL repository type for composition/test wiring.
pub use postgres::PostgresTriggerRepository;
pub use worker::{
    ACTIVE_HOLD_ELAPSED_OCCURRENCES_CAP, ACTIVE_HOLD_LOOKUP_TIMEOUT, ActiveHoldProjection,
    ActiveHoldReason, BlockedActiveRunKind, MissingTriggerActiveRunLookup,
    NoopTriggerFireSettlementObserver, TriggerAcceptedFireSettlement, TriggerActiveRunLookup,
    TriggerActiveRunState, TriggerActiveRunStateRequest, TriggerFireSettlementObserver,
    TriggerPollerFailureReason, TriggerPollerFireOutcome, TriggerPollerFireReport,
    TriggerPollerTickReport, TriggerPollerWorker, TriggerPollerWorkerConfig,
    TriggerPollerWorkerDeps, TrustedTriggerFireSubmitOutcome, TrustedTriggerFireSubmitter,
    TrustedTriggerSubmitRequest, active_hold_projection, active_holds_for_records,
};

#[derive(Clone, Default)]
pub struct InMemoryTriggerRepository {
    state: Arc<Mutex<in_memory::InMemoryTriggerRepositoryState>>,
}

pub(crate) fn reject_non_future_next_run_at(
    fire_slot: Timestamp,
    next_run_at: Timestamp,
) -> Result<(), TriggerError> {
    if next_run_at > fire_slot {
        return Ok(());
    }
    Err(TriggerError::InvalidRecord {
        kind: TriggerRecordValidationKind::Other,
        reason: "fire result next_run_at must be after the claimed fire slot".to_string(),
    })
}

pub(crate) fn reject_run_ref_rewrite(
    active_run_ref: TurnRunId,
    incoming_run_ref: TurnRunId,
) -> Result<(), TriggerError> {
    if active_run_ref == incoming_run_ref {
        return Ok(());
    }
    Err(TriggerError::InvalidRecord {
        kind: TriggerRecordValidationKind::Other,
        reason: "fire result must not rewrite an existing active_run_ref".to_string(),
    })
}

pub(crate) fn reject_failed_result_after_active_run(
    active_run_ref: Option<TurnRunId>,
) -> Result<(), TriggerError> {
    if active_run_ref.is_none() {
        return Ok(());
    }
    Err(TriggerError::InvalidRecord {
        kind: TriggerRecordValidationKind::Other,
        reason: "fire failure result must not clear an accepted active_run_ref".to_string(),
    })
}

fn parse_timezone(timezone: &str) -> Result<Tz, TriggerError> {
    timezone.parse::<Tz>().map_err(|_| TriggerError::InvalidSchedule {
        kind: TriggerScheduleValidationKind::InvalidTimezone,
        reason: format!(
            "invalid timezone '{timezone}': must be a valid IANA timezone name (e.g. 'America/New_York', 'UTC')"
        ),
    })
}

fn normalize_cron_expression(expression: &str) -> Result<String, TriggerError> {
    let trimmed = expression.trim();
    if trimmed.is_empty() {
        return Err(TriggerError::InvalidSchedule {
            kind: TriggerScheduleValidationKind::EmptyCronExpression,
            reason: "cron expression must not be empty".to_string(),
        });
    }
    let fields = trimmed.split_whitespace().collect::<Vec<_>>();
    match fields.len() {
        5 => Ok(format!("0 {} *", fields.join(" "))),
        6 => {
            reject_sub_minute_seconds_field(fields[0])?;
            Ok(format!("{} *", fields.join(" ")))
        }
        7 => {
            reject_sub_minute_seconds_field(fields[0])?;
            Ok(trimmed.to_string())
        }
        count => Err(TriggerError::InvalidSchedule {
            kind: TriggerScheduleValidationKind::InvalidCronFieldCount,
            reason: format!("expected 5, 6, or 7 cron fields, got {count}"),
        }),
    }
}

fn parse_cron_schedule(expression: &str) -> Result<Schedule, TriggerError> {
    let normalized = normalize_cron_expression(expression)?;
    let schedule =
        Schedule::from_str(&normalized).map_err(|error| TriggerError::InvalidSchedule {
            kind: TriggerScheduleValidationKind::InvalidCronExpression,
            reason: format!("invalid cron expression: {error}"),
        })?;
    reject_sub_minute_cadence(&schedule)?;
    Ok(schedule)
}

fn reject_sub_minute_seconds_field(field: &str) -> Result<(), TriggerError> {
    if field.trim().parse::<u32>() == Ok(0) {
        return Ok(());
    }
    Err(TriggerError::InvalidSchedule {
        kind: TriggerScheduleValidationKind::SecondLevelCadence,
        reason: "cron schedules must not use second-level cadence; use second field `0`"
            .to_string(),
    })
}

fn reject_sub_minute_cadence(schedule: &Schedule) -> Result<(), TriggerError> {
    let mut upcoming = schedule.upcoming(Utc);
    let Some(first) = upcoming.next() else {
        return Err(TriggerError::InvalidSchedule {
            kind: TriggerScheduleValidationKind::NoUpcomingFireTime,
            reason: "cron expression has no upcoming fire time".to_string(),
        });
    };
    let Some(second) = upcoming.next() else {
        return Ok(());
    };
    if (second - first).num_seconds() < MIN_FIRE_CADENCE.as_secs() as i64 {
        return Err(TriggerError::InvalidSchedule {
            kind: TriggerScheduleValidationKind::SubMinuteCadence,
            reason: "schedule can fire more frequently than once per minute".to_string(),
        });
    }
    Ok(())
}

fn validate_lower_hex_identifier(label: &str, value: String) -> Result<String, TriggerError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Ok(value);
    }
    Err(TriggerError::InvalidFireIdentityComponent {
        label: label.to_string(),
        reason: "must be 64 lowercase hex characters".to_string(),
    })
}

fn validate_inbound_content_ref(value: &str) -> Result<(), TriggerError> {
    if value.is_empty() {
        return Err(TriggerError::InvalidMaterialization {
            reason: "inbound content ref must not be empty".to_string(),
        });
    }
    if value.len() > 512 {
        return Err(TriggerError::InvalidMaterialization {
            reason: "inbound content ref must be at most 512 bytes".to_string(),
        });
    }
    if value.chars().any(|ch| ch == '\0' || ch.is_control()) {
        return Err(TriggerError::InvalidMaterialization {
            reason: "inbound content ref must not contain control characters".to_string(),
        });
    }
    Ok(())
}

fn derive_fire_digest(
    domain_label: &str,
    tenant_id: &TenantId,
    trigger_id: TriggerId,
    fire_slot: Timestamp,
) -> String {
    let slot = fire_slot
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Nanos, true);
    let mut hasher = Sha256::new();
    hasher.update(IDENTITY_VERSION_LABEL.as_bytes());
    hasher.update([0]);
    hasher.update(domain_label.as_bytes());
    hasher.update([0]);
    update_length_prefixed(&mut hasher, tenant_id.as_str().as_bytes());
    update_length_prefixed(&mut hasher, trigger_id.to_string().as_bytes());
    update_length_prefixed(&mut hasher, slot.as_bytes());
    hex::encode(hasher.finalize())
}

fn update_length_prefixed(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

#[cfg(test)]
mod tests;
