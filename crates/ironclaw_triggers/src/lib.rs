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
use ironclaw_host_api::{AgentId, ProjectId, TenantId, Timestamp, UserId};
use ironclaw_turns::TurnRunId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use ulid::Ulid;

#[cfg(feature = "libsql")]
mod libsql;
#[cfg(feature = "postgres")]
mod postgres;
mod trusted_submit;
mod worker;

pub use trusted_submit::{
    TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
    TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerMaterializedPrompt,
    TriggerTrustedInboundBinding,
};

const MIN_FIRE_CADENCE: Duration = Duration::from_secs(60);
const MAX_DUE_TRIGGER_POLL_LIMIT: usize = 128;
const MAX_TRIGGER_LIST_LIMIT: usize = 100;
const MAX_TRIGGER_RUN_HISTORY_LIMIT: usize = 500;
const MAX_TRIGGER_RUN_HISTORY_RETAINED: usize = 500;
pub const MAX_TRIGGER_NAME_BYTES: usize = 256;
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
    InvalidRecord { reason: String },
    #[error("invalid trigger poller configuration: {reason}")]
    InvalidPollerConfig { reason: String },
    #[error("invalid schedule: {reason}")]
    InvalidSchedule { reason: String },
    #[error("invalid trigger materialization: {reason}")]
    InvalidMaterialization { reason: String },
    #[error("trigger repository backend unavailable: {reason}")]
    Backend { reason: String },
    #[error("trigger not found")]
    NotFound,
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
    pub completion_policy: TriggerCompletionPolicy,
    pub prompt: String,
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
        if self.name.trim().is_empty() {
            return Err(TriggerError::InvalidRecord {
                reason: "trigger name must not be empty".to_string(),
            });
        }
        if self.name.len() > MAX_TRIGGER_NAME_BYTES {
            return Err(TriggerError::InvalidRecord {
                reason: format!("trigger name must be at most {MAX_TRIGGER_NAME_BYTES} bytes"),
            });
        }
        if self.prompt.trim().is_empty() {
            return Err(TriggerError::InvalidRecord {
                reason: "trigger prompt must not be empty".to_string(),
            });
        }
        if self.prompt.len() > MAX_TRIGGER_PROMPT_BYTES {
            return Err(TriggerError::InvalidRecord {
                reason: format!("trigger prompt must be at most {MAX_TRIGGER_PROMPT_BYTES} bytes"),
            });
        }
        if self.active_run_ref.is_some() && self.active_fire_slot.is_none() {
            return Err(TriggerError::InvalidRecord {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TriggerSchedule {
    Cron {
        expression: String,
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
        }
    }

    pub fn next_slot_after(&self, after: Timestamp) -> Result<Option<Timestamp>, TriggerError> {
        match self {
            Self::Cron {
                expression,
                timezone,
            } => {
                let tz = parse_timezone(timezone)?;
                let next = if tz == Tz::UTC {
                    parse_cron_schedule(expression)?.after(&after).next()
                } else {
                    parse_cron_schedule(expression)?
                        .after(&after.with_timezone(&tz))
                        .next()
                        .map(|dt| dt.with_timezone(&Utc))
                };
                Ok(next)
            }
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerCompletionPolicy {
    Recurring,
    CompleteAfterFirstFire,
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

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn trigger_run_history_status_text(value: TriggerRunHistoryStatus) -> &'static str {
    match value {
        TriggerRunHistoryStatus::Running => "running",
        TriggerRunHistoryStatus::Ok => "ok",
        TriggerRunHistoryStatus::Error => "error",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerRunRecord {
    pub tenant_id: TenantId,
    pub trigger_id: TriggerId,
    pub fire_slot: Timestamp,
    pub run_id: Option<TurnRunId>,
    pub thread_id: TriggerRouteThreadId,
    pub status: TriggerRunHistoryStatus,
    pub submitted_at: Timestamp,
    pub completed_at: Option<Timestamp>,
}

impl TriggerRunRecord {
    fn running(
        tenant_id: TenantId,
        trigger_id: TriggerId,
        fire_slot: Timestamp,
        run_id: Option<TurnRunId>,
        submitted_at: Timestamp,
    ) -> Self {
        let identity = TriggerFireIdentity::new(tenant_id.clone(), trigger_id, fire_slot);
        Self {
            tenant_id,
            trigger_id,
            fire_slot,
            run_id,
            thread_id: identity.route_thread_id,
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
    pub submitted_at: Timestamp,
    pub next_run_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FireReplayedRequest {
    pub tenant_id: TenantId,
    pub trigger_id: TriggerId,
    pub fire_slot: Timestamp,
    pub original_run_id: TurnRunId,
    pub replayed_at: Timestamp,
    pub next_run_at: Timestamp,
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
    async fn list_scoped_triggers(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        limit: usize,
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
#[cfg(feature = "libsql")]
pub use libsql::LibSqlTriggerRepository;
/// Feature-gated durable PostgreSQL repository type for composition/test wiring.
#[cfg(feature = "postgres")]
pub use postgres::PostgresTriggerRepository;
pub use worker::{
    TriggerActiveRunLookup, TriggerActiveRunState, TriggerActiveRunStateRequest,
    TriggerPollerFailureReason, TriggerPollerFireOutcome, TriggerPollerFireReport,
    TriggerPollerTickReport, TriggerPollerWorker, TriggerPollerWorkerConfig,
    TriggerPollerWorkerDeps, TrustedTriggerFireSubmitOutcome, TrustedTriggerFireSubmitter,
    TrustedTriggerSubmitRequest,
};

#[derive(Clone, Default)]
pub struct InMemoryTriggerRepository {
    state: Arc<Mutex<InMemoryTriggerRepositoryState>>,
}

#[derive(Debug, Default)]
struct InMemoryTriggerRepositoryState {
    records: HashMap<TriggerRepositoryKey, TriggerRecord>,
    runs: HashMap<TriggerRunRepositoryKey, TriggerRunRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TriggerRepositoryKey {
    tenant_id: TenantId,
    trigger_id: TriggerId,
}

impl TriggerRepositoryKey {
    fn new(tenant_id: &TenantId, trigger_id: TriggerId) -> Self {
        Self {
            tenant_id: tenant_id.clone(),
            trigger_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TriggerRunRepositoryKey {
    tenant_id: TenantId,
    trigger_id: TriggerId,
    fire_slot: Timestamp,
}

impl TriggerRunRepositoryKey {
    fn new(tenant_id: &TenantId, trigger_id: TriggerId, fire_slot: Timestamp) -> Self {
        Self {
            tenant_id: tenant_id.clone(),
            trigger_id,
            fire_slot,
        }
    }
}

#[async_trait]
impl TriggerRepository for InMemoryTriggerRepository {
    async fn upsert_trigger(&self, record: TriggerRecord) -> Result<(), TriggerError> {
        record.validate()?;
        let mut state = self.lock_state()?;
        state.records.insert(
            TriggerRepositoryKey::new(&record.tenant_id, record.trigger_id),
            record,
        );
        Ok(())
    }

    async fn get_trigger(
        &self,
        tenant_id: TenantId,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Ok(self
            .lock_state()?
            .records
            .get(&TriggerRepositoryKey::new(&tenant_id, trigger_id))
            .cloned())
    }

    async fn list_triggers(&self, tenant_id: TenantId) -> Result<Vec<TriggerRecord>, TriggerError> {
        let state = self.lock_state()?;
        let mut records = state
            .records
            .values()
            .filter(|record| record.tenant_id == tenant_id)
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by_key(|record| (record.created_at, record.trigger_id));
        Ok(records)
    }

    async fn list_scoped_triggers(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(MAX_TRIGGER_LIST_LIMIT);
        let state = self.lock_state()?;
        let mut records = state
            .records
            .values()
            .filter(|record| {
                record.tenant_id == tenant_id
                    && record.creator_user_id == creator_user_id
                    && record.agent_id == agent_id
                    && record.project_id == project_id
            })
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by_key(|record| (record.created_at, record.trigger_id));
        records.truncate(limit);
        Ok(records)
    }

    async fn remove_trigger(
        &self,
        tenant_id: TenantId,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Ok(self
            .lock_state()?
            .records
            .remove(&TriggerRepositoryKey::new(&tenant_id, trigger_id)))
    }

    async fn remove_scoped_trigger(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let mut state = self.lock_state()?;
        let key = TriggerRepositoryKey::new(&tenant_id, trigger_id);
        let Some(record) = state.records.get(&key) else {
            return Ok(None);
        };
        if record.creator_user_id != creator_user_id
            || record.agent_id != agent_id
            || record.project_id != project_id
        {
            return Ok(None);
        }
        Ok(state.records.remove(&key))
    }

    async fn list_due_triggers(
        &self,
        now: Timestamp,
        limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(MAX_DUE_TRIGGER_POLL_LIMIT);
        let state = self.lock_state()?;
        let mut selected_keys = state
            .records
            .iter()
            .filter(|(_, record)| record.is_due_at(now) && !record.has_active_fire())
            .map(|(key, record)| {
                (
                    record.next_run_at,
                    record.tenant_id.clone(),
                    record.trigger_id,
                    key.clone(),
                )
            })
            .collect::<Vec<_>>();
        selected_keys.sort_by_key(|(next_run_at, tenant_id, trigger_id, _)| {
            (*next_run_at, tenant_id.clone(), *trigger_id)
        });
        selected_keys.truncate(limit);
        Ok(selected_keys
            .into_iter()
            .filter_map(|(_, _, _, key)| state.records.get(&key).cloned())
            .collect())
    }

    async fn list_active_triggers(&self, limit: usize) -> Result<Vec<TriggerRecord>, TriggerError> {
        self.list_active_triggers_after(None, limit).await
    }

    async fn list_active_triggers_after(
        &self,
        after: Option<ActiveTriggerScanCursor>,
        limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(MAX_DUE_TRIGGER_POLL_LIMIT);
        let mut selected_records = {
            let state = self.lock_state()?;
            state
                .records
                .values()
                .filter_map(|record| {
                    let active_fire_slot = record.active_fire_slot?;
                    Some((
                        active_fire_slot,
                        record.tenant_id.clone(),
                        record.trigger_id,
                        record.clone(),
                    ))
                })
                .filter(
                    |(active_fire_slot, tenant_id, trigger_id, _record)| match after.as_ref() {
                        Some(cursor) => {
                            (*active_fire_slot, tenant_id, *trigger_id)
                                > (
                                    cursor.active_fire_slot(),
                                    cursor.tenant_id(),
                                    cursor.trigger_id(),
                                )
                        }
                        None => true,
                    },
                )
                .collect::<Vec<_>>()
        };
        selected_records.sort_by_key(|(active_fire_slot, tenant_id, trigger_id, _record)| {
            (*active_fire_slot, tenant_id.clone(), *trigger_id)
        });
        selected_records.truncate(limit);
        Ok(selected_records
            .into_iter()
            .map(|(_, _, _, record)| record)
            .collect())
    }

    async fn claim_due_fire(
        &self,
        request: ClaimDueFireRequest,
    ) -> Result<ClaimDueFireOutcome, TriggerError> {
        let mut state = self.lock_state()?;
        let key = TriggerRepositoryKey::new(&request.tenant_id, request.trigger_id);
        let Some(record) = state.records.get_mut(&key) else {
            return Ok(ClaimDueFireOutcome::NotFound);
        };

        if record.state != TriggerState::Scheduled
            || record.next_run_at != request.fire_slot
            || request.fire_slot > request.now
        {
            return Ok(ClaimDueFireOutcome::NotDue {
                record: record.clone(),
            });
        }

        if record.has_active_fire() {
            return Ok(ClaimDueFireOutcome::AlreadyActive {
                active_fire_slot: record.active_fire_slot,
                active_run_ref: record.active_run_ref,
            });
        }

        record.active_fire_slot = Some(request.fire_slot);
        record.active_run_ref = None;
        let record = record.clone();
        state.runs.insert(
            TriggerRunRepositoryKey::new(&request.tenant_id, request.trigger_id, request.fire_slot),
            TriggerRunRecord::running(
                request.tenant_id,
                request.trigger_id,
                request.fire_slot,
                None,
                request.now,
            ),
        );
        prune_run_history_locked(&mut state, &record.tenant_id, record.trigger_id);
        Ok(ClaimDueFireOutcome::Claimed(ClaimedTriggerFire {
            record,
            fire_slot: request.fire_slot,
        }))
    }

    async fn mark_fire_accepted(
        &self,
        request: FireAcceptedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let Some(record) = self.update_claimed_fire(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            |record| {
                if let Some(active_run_ref) = record.active_run_ref {
                    reject_run_ref_rewrite(active_run_ref, request.run_id)?;
                    return Ok(());
                }
                reject_non_future_next_run_at(request.fire_slot, request.next_run_at)?;
                record.last_run_at = Some(request.submitted_at);
                record.last_fired_slot = Some(request.fire_slot);
                record.last_status = Some(TriggerRunStatus::Ok);
                record.next_run_at = request.next_run_at;
                record.active_fire_slot = Some(request.fire_slot);
                record.active_run_ref = Some(request.run_id);
                Ok(())
            },
        )?
        else {
            return Ok(None);
        };
        self.upsert_running_run_history(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            request.run_id,
            record.last_run_at.unwrap_or(request.submitted_at),
        )?;
        Ok(Some(record))
    }

    async fn mark_fire_replayed(
        &self,
        request: FireReplayedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let Some(record) = self.update_claimed_fire(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            |record| {
                if let Some(active_run_ref) = record.active_run_ref {
                    reject_run_ref_rewrite(active_run_ref, request.original_run_id)?;
                    return Ok(());
                }
                reject_non_future_next_run_at(request.fire_slot, request.next_run_at)?;
                record.last_run_at = Some(request.replayed_at);
                record.last_fired_slot = Some(request.fire_slot);
                record.last_status = Some(TriggerRunStatus::Ok);
                record.next_run_at = request.next_run_at;
                record.active_fire_slot = Some(request.fire_slot);
                record.active_run_ref = Some(request.original_run_id);
                Ok(())
            },
        )?
        else {
            return Ok(None);
        };
        self.upsert_running_run_history(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            request.original_run_id,
            record.last_run_at.unwrap_or(request.replayed_at),
        )?;
        Ok(Some(record))
    }

    async fn mark_fire_retryable_failed(
        &self,
        request: FireRetryableFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let Some(record) = self.update_claimed_fire(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            |record| {
                reject_failed_result_after_active_run(record.active_run_ref)?;
                if record.next_run_at > request.fire_slot {
                    return Err(TriggerError::InvalidRecord {
                        reason: "retryable fire failure must leave next_run_at at or before the failed fire slot"
                            .to_string(),
                    });
                }
                record.last_status = Some(TriggerRunStatus::Error);
                record.active_fire_slot = None;
                record.active_run_ref = None;
                Ok(())
            },
        )?
        else {
            return Ok(None);
        };
        self.complete_run_history(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            None,
            TriggerRunHistoryStatus::Error,
            Utc::now(),
        )?;
        Ok(Some(record))
    }

    async fn mark_fire_permanently_failed(
        &self,
        request: FirePermanentFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let Some(record) = self.update_claimed_fire(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            |record| {
                reject_failed_result_after_active_run(record.active_run_ref)?;
                reject_non_future_next_run_at(request.fire_slot, request.next_run_at)?;
                record.last_status = Some(TriggerRunStatus::Error);
                record.next_run_at = request.next_run_at;
                record.active_fire_slot = None;
                record.active_run_ref = None;
                Ok(())
            },
        )?
        else {
            return Ok(None);
        };
        self.complete_run_history(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            None,
            TriggerRunHistoryStatus::Error,
            Utc::now(),
        )?;
        Ok(Some(record))
    }

    async fn mark_fire_terminally_failed(
        &self,
        request: FireTerminalFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let Some(record) = self.update_claimed_fire(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            |record| {
                reject_failed_result_after_active_run(record.active_run_ref)?;
                record.state = TriggerState::Completed;
                record.last_status = Some(TriggerRunStatus::Error);
                record.active_fire_slot = None;
                record.active_run_ref = None;
                Ok(())
            },
        )?
        else {
            return Ok(None);
        };
        self.complete_run_history(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            None,
            TriggerRunHistoryStatus::Error,
            Utc::now(),
        )?;
        Ok(Some(record))
    }

    async fn clear_active_fire(
        &self,
        request: ClearActiveFireRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let mut state = self.lock_state()?;
        let key = TriggerRepositoryKey::new(&request.tenant_id, request.trigger_id);
        let Some(record) = state.records.get_mut(&key) else {
            return Ok(None);
        };
        if record.active_fire_slot != Some(request.fire_slot)
            || record.active_run_ref != Some(request.run_id)
        {
            return Ok(None);
        }
        record.active_fire_slot = None;
        record.active_run_ref = None;
        let record = record.clone();
        let completed_at = Utc::now();
        state
            .runs
            .entry(TriggerRunRepositoryKey::new(
                &request.tenant_id,
                request.trigger_id,
                request.fire_slot,
            ))
            .and_modify(|run| {
                run.run_id = Some(request.run_id);
                run.status = request.status;
                run.completed_at = Some(completed_at);
            })
            .or_insert_with(|| {
                let mut run = TriggerRunRecord::running(
                    request.tenant_id.clone(),
                    request.trigger_id,
                    request.fire_slot,
                    Some(request.run_id),
                    completed_at,
                );
                run.status = request.status;
                run.completed_at = Some(completed_at);
                run
            });
        prune_run_history_locked(&mut state, &request.tenant_id, request.trigger_id);
        Ok(Some(record))
    }

    async fn list_trigger_run_history(
        &self,
        tenant_id: TenantId,
        trigger_id: TriggerId,
        limit: usize,
    ) -> Result<Vec<TriggerRunRecord>, TriggerError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(MAX_TRIGGER_RUN_HISTORY_LIMIT);
        let state = self.lock_state()?;
        let mut runs = state
            .runs
            .values()
            .filter(|run| run.tenant_id == tenant_id && run.trigger_id == trigger_id)
            .cloned()
            .collect::<Vec<_>>();
        runs.sort_by_key(|run| std::cmp::Reverse(run.fire_slot));
        runs.truncate(limit);
        Ok(runs)
    }

    async fn list_trigger_run_history_batch(
        &self,
        tenant_id: TenantId,
        trigger_ids: &[TriggerId],
        limit: usize,
    ) -> Result<HashMap<TriggerId, Vec<TriggerRunRecord>>, TriggerError> {
        let mut runs_by_trigger = HashMap::with_capacity(trigger_ids.len());
        if limit == 0 || trigger_ids.is_empty() {
            return Ok(runs_by_trigger);
        }
        let limit = limit.min(MAX_TRIGGER_RUN_HISTORY_LIMIT);
        let state = self.lock_state()?;
        let trigger_id_set = trigger_ids.iter().copied().collect::<HashSet<_>>();
        for trigger_id in trigger_ids {
            runs_by_trigger.insert(*trigger_id, Vec::new());
        }
        for run in state
            .runs
            .values()
            .filter(|run| run.tenant_id == tenant_id && trigger_id_set.contains(&run.trigger_id))
        {
            runs_by_trigger
                .entry(run.trigger_id)
                .or_default()
                .push(run.clone());
        }
        for runs in runs_by_trigger.values_mut() {
            runs.sort_by_key(|run| std::cmp::Reverse(run.fire_slot));
            runs.truncate(limit);
        }
        Ok(runs_by_trigger)
    }
}

impl InMemoryTriggerRepository {
    fn lock_state(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, InMemoryTriggerRepositoryState>, TriggerError> {
        self.state.lock().map_err(|_| TriggerError::Backend {
            reason: "trigger repository mutex poisoned".to_string(),
        })
    }

    fn update_claimed_fire(
        &self,
        tenant_id: &TenantId,
        trigger_id: TriggerId,
        fire_slot: Timestamp,
        update: impl FnOnce(&mut TriggerRecord) -> Result<(), TriggerError>,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let mut state = self.lock_state()?;
        let key = TriggerRepositoryKey::new(tenant_id, trigger_id);
        let Some(record) = state.records.get_mut(&key) else {
            return Ok(None);
        };
        if record.active_fire_slot != Some(fire_slot) {
            return Ok(None);
        }
        update(record)?;
        Ok(Some(record.clone()))
    }

    fn upsert_running_run_history(
        &self,
        tenant_id: &TenantId,
        trigger_id: TriggerId,
        fire_slot: Timestamp,
        run_id: TurnRunId,
        submitted_at: Timestamp,
    ) -> Result<(), TriggerError> {
        let mut state = self.lock_state()?;
        let key = TriggerRunRepositoryKey::new(tenant_id, trigger_id, fire_slot);
        if state
            .runs
            .get(&key)
            .is_some_and(|run| run.completed_at.is_some())
        {
            return Ok(());
        }
        state.runs.insert(
            key,
            TriggerRunRecord::running(
                tenant_id.clone(),
                trigger_id,
                fire_slot,
                Some(run_id),
                submitted_at,
            ),
        );
        prune_run_history_locked(&mut state, tenant_id, trigger_id);
        Ok(())
    }

    fn complete_run_history(
        &self,
        tenant_id: &TenantId,
        trigger_id: TriggerId,
        fire_slot: Timestamp,
        run_id: Option<TurnRunId>,
        status: TriggerRunHistoryStatus,
        completed_at: Timestamp,
    ) -> Result<(), TriggerError> {
        let mut state = self.lock_state()?;
        state
            .runs
            .entry(TriggerRunRepositoryKey::new(
                tenant_id, trigger_id, fire_slot,
            ))
            .and_modify(|run| {
                if run.run_id.is_none() {
                    run.run_id = run_id;
                }
                run.status = status;
                run.completed_at = Some(completed_at);
            })
            .or_insert_with(|| {
                let mut run = TriggerRunRecord::running(
                    tenant_id.clone(),
                    trigger_id,
                    fire_slot,
                    run_id,
                    completed_at,
                );
                run.status = status;
                run.completed_at = Some(completed_at);
                run
            });
        prune_run_history_locked(&mut state, tenant_id, trigger_id);
        Ok(())
    }
}

fn prune_run_history_locked(
    state: &mut InMemoryTriggerRepositoryState,
    tenant_id: &TenantId,
    trigger_id: TriggerId,
) {
    let mut keys = state
        .runs
        .keys()
        .filter(|key| key.tenant_id == *tenant_id && key.trigger_id == trigger_id)
        .cloned()
        .collect::<Vec<_>>();
    keys.sort_by_key(|key| std::cmp::Reverse(key.fire_slot));
    for key in keys.into_iter().skip(MAX_TRIGGER_RUN_HISTORY_RETAINED) {
        state.runs.remove(&key);
    }
}

pub(crate) fn reject_non_future_next_run_at(
    fire_slot: Timestamp,
    next_run_at: Timestamp,
) -> Result<(), TriggerError> {
    if next_run_at > fire_slot {
        return Ok(());
    }
    Err(TriggerError::InvalidRecord {
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
        reason: "fire failure result must not clear an accepted active_run_ref".to_string(),
    })
}

fn parse_timezone(timezone: &str) -> Result<Tz, TriggerError> {
    timezone.parse::<Tz>().map_err(|_| TriggerError::InvalidSchedule {
        reason: format!(
            "invalid timezone '{timezone}': must be a valid IANA timezone name (e.g. 'America/New_York', 'UTC')"
        ),
    })
}

fn normalize_cron_expression(expression: &str) -> Result<String, TriggerError> {
    let trimmed = expression.trim();
    if trimmed.is_empty() {
        return Err(TriggerError::InvalidSchedule {
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
            reason: format!("expected 5, 6, or 7 cron fields, got {count}"),
        }),
    }
}

fn parse_cron_schedule(expression: &str) -> Result<Schedule, TriggerError> {
    let normalized = normalize_cron_expression(expression)?;
    let schedule =
        Schedule::from_str(&normalized).map_err(|error| TriggerError::InvalidSchedule {
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
        reason: "cron schedules must not use second-level cadence; use second field `0`"
            .to_string(),
    })
}

fn reject_sub_minute_cadence(schedule: &Schedule) -> Result<(), TriggerError> {
    let mut upcoming = schedule.upcoming(Utc);
    let Some(first) = upcoming.next() else {
        return Err(TriggerError::InvalidSchedule {
            reason: "cron expression has no upcoming fire time".to_string(),
        });
    };
    let Some(second) = upcoming.next() else {
        return Ok(());
    };
    if (second - first).num_seconds() < MIN_FIRE_CADENCE.as_secs() as i64 {
        return Err(TriggerError::InvalidSchedule {
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
mod tests {
    use chrono::{Datelike, TimeZone};
    use serde_json::{from_value, json, to_value};

    use super::*;

    fn ts(seconds: i64) -> Timestamp {
        Utc.timestamp_opt(seconds, 0)
            .single()
            .expect("valid timestamp")
    }

    fn tenant(value: &str) -> TenantId {
        TenantId::new(value).expect("valid tenant")
    }

    fn poison_in_memory_repo(repo: &InMemoryTriggerRepository) {
        let poison_repo = repo.clone();
        let _ = std::panic::catch_unwind(move || {
            let _guard = poison_repo.state.lock().expect("lock before poison");
            panic!("poison trigger repository mutex");
        });
    }

    fn user(value: &str) -> UserId {
        UserId::new(value).expect("valid user")
    }

    fn sample_record(
        trigger_id: TriggerId,
        tenant_id: TenantId,
        next_run_at: Timestamp,
    ) -> TriggerRecord {
        TriggerRecord {
            trigger_id,
            tenant_id,
            creator_user_id: user("user-a"),
            agent_id: Some(AgentId::new("agent-a").expect("valid agent")),
            project_id: Some(ProjectId::new("project-a").expect("valid project")),
            name: "daily summary".to_string(),
            source: TriggerSourceKind::Schedule,
            schedule: TriggerSchedule::cron("0 8 * * *").expect("valid cron"),
            completion_policy: TriggerCompletionPolicy::Recurring,
            prompt: "summarize unread mail".to_string(),
            state: TriggerState::Scheduled,
            next_run_at,
            last_run_at: None,
            last_fired_slot: None,
            last_status: None,
            active_fire_slot: None,
            active_run_ref: None,
            created_at: ts(1_704_067_200),
        }
    }

    #[test]
    fn cron_schedule_accepts_minute_cadence_and_computes_next_slot() {
        let schedule = TriggerSchedule::cron("*/5 * * * *").expect("minute cadence is valid");
        let next = schedule
            .next_slot_after(Utc.with_ymd_and_hms(2026, 5, 30, 12, 3, 0).unwrap())
            .expect("next slot")
            .expect("future slot");
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 5, 30, 12, 5, 0).unwrap());
    }

    #[test]
    fn cron_schedule_rejects_exhausted_finite_year() {
        let past_year = Utc::now().year() - 1;
        let error = TriggerSchedule::cron(format!("0 0 8 * * * {past_year}"))
            .expect_err("exhausted finite cron rejected");
        assert!(
            error
                .to_string()
                .contains("cron expression has no upcoming fire time"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn cron_schedule_rejects_wrong_field_count() {
        let error = TriggerSchedule::cron("0 8 * *").expect_err("cron field count rejected");
        assert!(
            error
                .to_string()
                .contains("expected 5, 6, or 7 cron fields"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn trigger_id_parse_rejects_invalid_ulid() {
        let error = TriggerId::parse("not-a-ulid").expect_err("malformed ulid rejected");
        assert!(
            error.to_string().contains("invalid trigger id"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn public_fire_id_wrappers_validate_hex_accessors_and_serde_round_trip() {
        let route_value = "a".repeat(64);
        let event_value = "b".repeat(64);
        let route = TriggerRouteThreadId::new(route_value.clone()).expect("valid route id");
        let event = TriggerExternalEventId::new(event_value.clone()).expect("valid event id");

        assert_eq!(route.as_str(), route_value);
        assert_eq!(route.as_ref(), route_value);
        assert_eq!(route.to_string(), route_value);
        assert_eq!(event.as_str(), event_value);
        assert_eq!(event.as_ref(), event_value);
        assert_eq!(event.to_string(), event_value);
        assert!(TriggerRouteThreadId::new("route-1").is_err());
        assert!(TriggerExternalEventId::new("event-1").is_err());
        assert_eq!(to_value(&route).unwrap(), json!(route_value));
        assert_eq!(to_value(&event).unwrap(), json!(event_value));
        assert_eq!(
            from_value::<TriggerRouteThreadId>(json!(route_value)).unwrap(),
            route
        );
        assert_eq!(
            from_value::<TriggerExternalEventId>(json!(event_value)).unwrap(),
            event
        );
        assert!(matches!(
            TriggerRouteThreadId::new("route-1"),
            Err(TriggerError::InvalidFireIdentityComponent { .. })
        ));
        assert!(matches!(
            TriggerExternalEventId::new("event-1"),
            Err(TriggerError::InvalidFireIdentityComponent { .. })
        ));
    }

    #[test]
    fn cron_schedule_rejects_sub_minute_seconds_fields() {
        for expression in [
            "*/30 * * * * *",
            "1 * * * * *",
            "0/15 * * * * * *",
            "00/15 * * * * *",
        ] {
            let error = TriggerSchedule::cron(expression).expect_err("sub-minute cron rejected");
            assert!(
                error.to_string().contains("second-level cadence"),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    fn cron_schedule_accepts_zero_and_zero_padded_seconds_fields() {
        for expression in ["0 0 * * * *", "00 0 * * * *"] {
            TriggerSchedule::cron(expression).expect("zero seconds accepted");
        }
    }

    #[test]
    fn cron_schedule_accepts_far_future_recurring_dates() {
        TriggerSchedule::cron("0 8 31 12 *").expect("annual schedule accepted");
    }

    #[test]
    fn trigger_enums_serialize_as_snake_case() {
        assert_eq!(
            to_value(TriggerSourceKind::Schedule).unwrap(),
            json!("schedule")
        );
        assert_eq!(
            to_value(TriggerState::Scheduled).unwrap(),
            json!("scheduled")
        );
        assert_eq!(
            to_value(TriggerCompletionPolicy::CompleteAfterFirstFire).unwrap(),
            json!("complete_after_first_fire")
        );
        assert_eq!(to_value(TriggerRunStatus::Ok).unwrap(), json!("ok"));
        assert_eq!(
            from_value::<TriggerRunStatus>(json!("error")).unwrap(),
            TriggerRunStatus::Error
        );
        assert!(from_value::<TriggerRunStatus>(json!("timed_out")).is_err());
        assert!(from_value::<TriggerRunStatus>(json!("approval_blocked")).is_err());
    }

    #[test]
    fn fire_identity_is_stable_domain_separated_and_tenant_scoped() {
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let slot = Utc.with_ymd_and_hms(2026, 5, 30, 8, 0, 0).unwrap();
        let first = TriggerFireIdentity::new(tenant("tenant-a"), trigger_id, slot);
        let second = TriggerFireIdentity::new(tenant("tenant-a"), trigger_id, slot);
        let other_slot = TriggerFireIdentity::new(
            tenant("tenant-a"),
            trigger_id,
            slot + chrono::Duration::minutes(1),
        );
        let other_tenant = TriggerFireIdentity::new(tenant("tenant-b"), trigger_id, slot);

        assert_eq!(first, second);
        assert_ne!(
            first.route_thread_id.as_str(),
            first.external_event_id.as_str()
        );
        assert_ne!(first.route_thread_id, other_slot.route_thread_id);
        assert_ne!(first.external_event_id, other_slot.external_event_id);
        assert_ne!(first.route_thread_id, other_tenant.route_thread_id);
    }

    #[test]
    fn fire_identity_length_prefixing_avoids_component_boundary_collisions() {
        let slot = Utc.with_ymd_and_hms(2026, 5, 30, 8, 0, 0).unwrap();
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let similar_trigger_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
        let short_tenant = TriggerFireIdentity::new(tenant("ab"), trigger_id, slot);
        let prefix_tenant = TriggerFireIdentity::new(tenant("a"), similar_trigger_id, slot);

        assert_ne!(short_tenant.route_thread_id, prefix_tenant.route_thread_id);
        assert_eq!(short_tenant.route_thread_id.as_str().len(), 64);
        assert_eq!(short_tenant.external_event_id.as_str().len(), 64);
    }

    #[tokio::test]
    async fn schedule_provider_emits_due_fire_only() {
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let record = sample_record(trigger_id, tenant("tenant-a"), ts(1_704_067_200));
        let provider = ScheduleTriggerSourceProvider;

        assert!(
            provider
                .evaluate(&record, ts(1_704_067_199))
                .await
                .expect("not due")
                .is_none()
        );
        let fire = provider
            .evaluate(&record, ts(1_704_067_200))
            .await
            .expect("due")
            .expect("fire");
        assert_eq!(fire.identity.trigger_id, trigger_id);
        assert_eq!(fire.identity.fire_slot, record.next_run_at);
        assert_eq!(fire.prompt, record.prompt);
    }

    #[test]
    fn trigger_inbound_content_ref_is_opaque_validated_materialization_output() {
        let content_ref =
            TriggerInboundContentRef::new("content:trigger-fire-01").expect("valid content ref");

        assert_eq!(content_ref.as_str(), "content:trigger-fire-01");
        assert_eq!(content_ref.as_ref(), "content:trigger-fire-01");
        assert_eq!(content_ref.to_string(), "content:trigger-fire-01");
        assert_eq!(
            to_value(&content_ref).unwrap(),
            json!("content:trigger-fire-01")
        );
        assert_eq!(
            from_value::<TriggerInboundContentRef>(json!("content:trigger-fire-01")).unwrap(),
            content_ref
        );
        assert!(TriggerInboundContentRef::new("x".repeat(512)).is_ok());

        assert!(TriggerInboundContentRef::new("").is_err());
        assert!(TriggerInboundContentRef::new("content:\ntrigger").is_err());
        assert!(TriggerInboundContentRef::new("x".repeat(513)).is_err());

        assert!(from_value::<TriggerInboundContentRef>(json!("")).is_err());
        assert!(from_value::<TriggerInboundContentRef>(json!("content:\ntrigger")).is_err());
        assert!(from_value::<TriggerInboundContentRef>(json!("x".repeat(513))).is_err());
    }

    #[tokio::test]
    async fn prompt_materializer_port_receives_fire_and_returns_materialized_prompt() {
        struct RecordingMaterializer;

        #[async_trait]
        impl TriggerPromptMaterializer for RecordingMaterializer {
            async fn materialize_prompt(
                &self,
                fire: TriggerFire,
            ) -> Result<TriggerMaterializedPrompt, TriggerError> {
                assert_eq!(fire.creator_user_id, user("user-a"));
                assert_eq!(fire.agent_id, Some(AgentId::new("agent-a").unwrap()));
                assert_eq!(fire.project_id, Some(ProjectId::new("project-a").unwrap()));
                assert_eq!(fire.prompt, "summarize unread mail");
                let content_ref = TriggerInboundContentRef::new(format!(
                    "content:{}",
                    fire.identity.external_event_id
                ))?;
                Ok(TriggerMaterializedPrompt::for_fire(&fire, content_ref))
            }
        }

        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let record = sample_record(trigger_id, tenant("tenant-a"), ts(1_704_067_200));
        let fire = ScheduleTriggerSourceProvider
            .evaluate(&record, ts(1_704_067_200))
            .await
            .expect("due")
            .expect("fire");

        let materialized = RecordingMaterializer
            .materialize_prompt(fire.clone())
            .await
            .expect("materialized");

        assert_eq!(
            materialized.content_ref().as_str(),
            format!("content:{}", fire.identity.external_event_id)
        );
        assert_eq!(
            materialized.trusted_inbound_binding().external_event_id(),
            fire.identity.external_event_id().as_str()
        );
    }

    #[tokio::test]
    async fn schedule_provider_uses_state_as_fire_gate() {
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let mut record = sample_record(trigger_id, tenant("tenant-a"), ts(1_704_067_200));
        let provider = ScheduleTriggerSourceProvider;

        assert!(
            provider
                .evaluate(&record, ts(1_704_067_200))
                .await
                .expect("scheduled state remains due")
                .is_some()
        );

        record.state = TriggerState::Paused;
        assert!(
            provider
                .evaluate(&record, ts(1_704_067_200))
                .await
                .expect("paused state is not due")
                .is_none()
        );

        record.state = TriggerState::Completed;
        assert!(
            provider
                .evaluate(&record, ts(1_704_067_200))
                .await
                .expect("completed state is not due")
                .is_none()
        );
    }

    #[tokio::test]
    async fn schedule_provider_rejects_invalid_record() {
        let mut record = sample_record(
            TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
            tenant("tenant-a"),
            ts(1_704_067_200),
        );
        record.prompt.clear();

        let error = ScheduleTriggerSourceProvider
            .evaluate(&record, ts(1_704_067_200))
            .await
            .expect_err("invalid record rejected");
        assert!(
            error
                .to_string()
                .contains("trigger prompt must not be empty"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn in_memory_repository_lists_and_removes_scoped_records() {
        let repo = InMemoryTriggerRepository::default();
        let due = sample_record(
            TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
            tenant("tenant-a"),
            ts(1_704_067_200),
        );
        let later = sample_record(
            TriggerId::parse("01J00000000000000000000000").expect("ulid"),
            tenant("tenant-a"),
            ts(1_704_067_260),
        );
        let other_tenant = sample_record(
            TriggerId::parse("01J00000000000000000000001").expect("ulid"),
            tenant("tenant-b"),
            ts(1_704_067_200),
        );
        let other_tenant_id = other_tenant.trigger_id;
        repo.upsert_trigger(due.clone()).await.expect("insert due");
        repo.upsert_trigger(later.clone())
            .await
            .expect("insert later");
        repo.upsert_trigger(other_tenant)
            .await
            .expect("insert other tenant");

        let due_records = repo
            .list_due_triggers(ts(1_704_067_200), 10)
            .await
            .expect("list due");
        assert_eq!(
            due_records
                .iter()
                .map(|record| record.trigger_id)
                .collect::<Vec<_>>(),
            vec![due.trigger_id, other_tenant_id]
        );

        let tenant_records = repo
            .list_triggers(tenant("tenant-a"))
            .await
            .expect("list tenant");
        assert_eq!(
            tenant_records
                .iter()
                .map(|record| record.trigger_id)
                .collect::<Vec<_>>(),
            vec![due.trigger_id, later.trigger_id]
        );

        let removed = repo
            .remove_trigger(tenant("tenant-a"), due.trigger_id)
            .await
            .expect("remove")
            .expect("record removed");
        assert_eq!(removed.trigger_id, due.trigger_id);
        assert!(
            repo.get_trigger(tenant("tenant-a"), due.trigger_id)
                .await
                .expect("lookup")
                .is_none()
        );
    }

    #[tokio::test]
    async fn in_memory_repository_remove_missing_key_returns_none() {
        let repo = InMemoryTriggerRepository::default();
        assert!(
            repo.remove_trigger(
                tenant("tenant-a"),
                TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid")
            )
            .await
            .expect("remove missing")
            .is_none()
        );
    }

    #[tokio::test]
    async fn in_memory_repository_rejects_invalid_record_on_upsert() {
        let repo = InMemoryTriggerRepository::default();
        let mut record = sample_record(
            TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
            tenant("tenant-a"),
            ts(1_704_067_200),
        );
        record.name.clear();
        assert!(matches!(
            repo.upsert_trigger(record).await,
            Err(TriggerError::InvalidRecord { .. })
        ));

        let mut record = sample_record(
            TriggerId::parse("01J00000000000000000000000").expect("ulid"),
            tenant("tenant-a"),
            ts(1_704_067_200),
        );
        record.prompt.clear();
        assert!(matches!(
            repo.upsert_trigger(record).await,
            Err(TriggerError::InvalidRecord { .. })
        ));

        let mut record = sample_record(
            TriggerId::parse("01J00000000000000000000001").expect("ulid"),
            tenant("tenant-a"),
            ts(1_704_067_200),
        );
        record.name = "x".repeat(MAX_TRIGGER_NAME_BYTES + 1);
        assert!(matches!(
            repo.upsert_trigger(record).await,
            Err(TriggerError::InvalidRecord { .. })
        ));

        let mut record = sample_record(
            TriggerId::parse("01J00000000000000000000002").expect("ulid"),
            tenant("tenant-a"),
            ts(1_704_067_200),
        );
        record.prompt = "x".repeat(MAX_TRIGGER_PROMPT_BYTES + 1);
        assert!(matches!(
            repo.upsert_trigger(record).await,
            Err(TriggerError::InvalidRecord { .. })
        ));
    }

    #[tokio::test]
    async fn in_memory_repository_list_due_triggers_handles_zero_limit() {
        let repo = InMemoryTriggerRepository::default();
        repo.upsert_trigger(sample_record(
            TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
            tenant("tenant-a"),
            ts(1_704_067_200),
        ))
        .await
        .expect("insert due");

        let due_records = repo
            .list_due_triggers(ts(1_704_067_200), 0)
            .await
            .expect("list due");
        assert!(due_records.is_empty());
    }

    #[tokio::test]
    async fn in_memory_repository_list_due_triggers_truncates_to_limit_one() {
        let repo = InMemoryTriggerRepository::default();
        let first = sample_record(
            TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
            tenant("tenant-a"),
            ts(1_704_067_200),
        );
        let mut second = sample_record(
            TriggerId::parse("01J00000000000000000000000").expect("ulid"),
            tenant("tenant-a"),
            ts(1_704_067_260),
        );
        second.created_at = ts(1_704_067_201);
        repo.upsert_trigger(first.clone())
            .await
            .expect("insert first");
        repo.upsert_trigger(second).await.expect("insert second");

        let due_records = repo
            .list_due_triggers(ts(1_704_067_260), 1)
            .await
            .expect("list due");
        assert_eq!(due_records.len(), 1);
        assert_eq!(due_records[0].trigger_id, first.trigger_id);
    }

    #[tokio::test]
    async fn in_memory_repository_list_due_triggers_orders_same_slot_by_tenant_then_trigger_id() {
        let repo = InMemoryTriggerRepository::default();
        let due_slot = ts(1_704_067_200);
        let tenant_a_high = sample_record(
            TriggerId::parse("01J00000000000000000000000").expect("ulid"),
            tenant("tenant-a"),
            due_slot,
        );
        let tenant_b_low = sample_record(
            TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
            tenant("tenant-b"),
            due_slot,
        );
        let tenant_a_low = sample_record(
            TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZY").expect("ulid"),
            tenant("tenant-a"),
            due_slot,
        );
        repo.upsert_trigger(tenant_b_low.clone())
            .await
            .expect("insert tenant b");
        repo.upsert_trigger(tenant_a_high.clone())
            .await
            .expect("insert tenant a high");
        repo.upsert_trigger(tenant_a_low.clone())
            .await
            .expect("insert tenant a low");

        let due_records = repo
            .list_due_triggers(due_slot, 10)
            .await
            .expect("list due");

        assert_eq!(
            due_records
                .iter()
                .map(|record| (record.tenant_id.clone(), record.trigger_id))
                .collect::<Vec<_>>(),
            vec![
                (tenant_a_low.tenant_id.clone(), tenant_a_low.trigger_id),
                (tenant_a_high.tenant_id.clone(), tenant_a_high.trigger_id),
                (tenant_b_low.tenant_id.clone(), tenant_b_low.trigger_id),
            ]
        );
    }

    #[tokio::test]
    async fn in_memory_repository_list_due_triggers_clamps_large_limit() {
        let repo = InMemoryTriggerRepository::default();
        for _ in 0..=MAX_DUE_TRIGGER_POLL_LIMIT {
            repo.upsert_trigger(sample_record(
                TriggerId::new(),
                tenant("tenant-a"),
                ts(1_704_067_200),
            ))
            .await
            .expect("insert due");
        }

        let due_records = repo
            .list_due_triggers(ts(1_704_067_200), MAX_DUE_TRIGGER_POLL_LIMIT + 10)
            .await
            .expect("list due");
        assert_eq!(due_records.len(), MAX_DUE_TRIGGER_POLL_LIMIT);
    }

    #[tokio::test]
    async fn in_memory_repository_running_history_does_not_overwrite_terminal_history() {
        let repo = InMemoryTriggerRepository::default();
        let tenant_id = tenant("tenant-a");
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("valid run");
        let completed_at = fire_slot + chrono::Duration::seconds(30);
        let later_submitted_at = fire_slot + chrono::Duration::seconds(45);

        repo.complete_run_history(
            &tenant_id,
            trigger_id,
            fire_slot,
            Some(run_id),
            TriggerRunHistoryStatus::Ok,
            completed_at,
        )
        .expect("seed terminal history");

        repo.upsert_running_run_history(
            &tenant_id,
            trigger_id,
            fire_slot,
            run_id,
            later_submitted_at,
        )
        .expect("late running upsert is ignored");

        let runs = repo
            .list_trigger_run_history(tenant_id, trigger_id, 10)
            .await
            .expect("list run history");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, TriggerRunHistoryStatus::Ok);
        assert_eq!(runs[0].submitted_at, completed_at);
        assert_eq!(runs[0].completed_at, Some(completed_at));
    }

    #[test]
    fn in_memory_repository_returns_backend_error_when_mutex_is_poisoned() {
        let repo = InMemoryTriggerRepository::default();
        let poison_repo = repo.clone();
        let _ = std::panic::catch_unwind(move || {
            let _guard = poison_repo.state.lock().expect("lock before poison");
            panic!("poison trigger repository mutex");
        });

        let error = repo
            .lock_state()
            .expect_err("poisoned mutex maps to backend");
        assert!(matches!(error, TriggerError::Backend { .. }));
    }

    #[tokio::test]
    async fn in_memory_repository_claim_due_fire_returns_backend_error_when_mutex_is_poisoned() {
        let repo = InMemoryTriggerRepository::default();
        poison_in_memory_repo(&repo);

        let error = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant("tenant-a"),
                trigger_id: TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
                fire_slot: ts(1_704_067_200),
                now: ts(1_704_067_200),
            })
            .await
            .expect_err("poisoned mutex maps to backend through claim API");
        assert!(matches!(error, TriggerError::Backend { .. }));
    }

    #[tokio::test]
    async fn in_memory_repository_mark_fire_accepted_returns_backend_error_when_mutex_is_poisoned()
    {
        let repo = InMemoryTriggerRepository::default();
        poison_in_memory_repo(&repo);

        let fire_slot = ts(1_704_067_200);
        let error = repo
            .mark_fire_accepted(FireAcceptedRequest {
                tenant_id: tenant("tenant-a"),
                trigger_id: TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
                fire_slot,
                run_id: TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a")
                    .expect("valid run"),
                submitted_at: fire_slot,
                next_run_at: ts(1_704_067_260),
            })
            .await
            .expect_err("poisoned mutex maps to backend through accepted-result API");
        assert!(matches!(error, TriggerError::Backend { .. }));
    }

    #[tokio::test]
    async fn in_memory_repository_mark_fire_replayed_returns_backend_error_when_mutex_is_poisoned()
    {
        let repo = InMemoryTriggerRepository::default();
        poison_in_memory_repo(&repo);

        let fire_slot = ts(1_704_067_200);
        let error = repo
            .mark_fire_replayed(FireReplayedRequest {
                tenant_id: tenant("tenant-a"),
                trigger_id: TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
                fire_slot,
                original_run_id: TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a")
                    .expect("valid run"),
                replayed_at: fire_slot,
                next_run_at: ts(1_704_067_260),
            })
            .await
            .expect_err("poisoned mutex maps to backend through replayed-result API");
        assert!(matches!(error, TriggerError::Backend { .. }));
    }

    #[tokio::test]
    async fn in_memory_repository_mark_fire_retryable_failed_returns_backend_error_when_mutex_is_poisoned()
     {
        let repo = InMemoryTriggerRepository::default();
        poison_in_memory_repo(&repo);

        let fire_slot = ts(1_704_067_200);
        let error = repo
            .mark_fire_retryable_failed(FireRetryableFailedRequest {
                tenant_id: tenant("tenant-a"),
                trigger_id: TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
                fire_slot,
            })
            .await
            .expect_err("poisoned mutex maps to backend through retryable-failure API");
        assert!(matches!(error, TriggerError::Backend { .. }));
    }

    #[tokio::test]
    async fn in_memory_repository_mark_fire_permanently_failed_returns_backend_error_when_mutex_is_poisoned()
     {
        let repo = InMemoryTriggerRepository::default();
        poison_in_memory_repo(&repo);

        let fire_slot = ts(1_704_067_200);
        let error = repo
            .mark_fire_permanently_failed(FirePermanentFailedRequest {
                tenant_id: tenant("tenant-a"),
                trigger_id: TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
                fire_slot,
                next_run_at: ts(1_704_067_260),
            })
            .await
            .expect_err("poisoned mutex maps to backend through permanent-failure API");
        assert!(matches!(error, TriggerError::Backend { .. }));
    }

    #[test]
    fn cron_schedule_rejects_invalid_timezone() {
        let error = TriggerSchedule::cron_with_timezone("0 9 * * *", "Not/A/Timezone")
            .expect_err("invalid timezone rejected");
        assert!(
            error.to_string().contains("invalid timezone"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn cron_schedule_evaluates_in_named_timezone() {
        // "0 9 * * *" = 9am local time
        // America/New_York is UTC-5 in winter (no DST in January)
        // 9am New York = 14:00 UTC
        let schedule = TriggerSchedule::cron_with_timezone("0 9 * * *", "America/New_York")
            .expect("valid schedule");
        let after = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(); // midnight UTC, Jan 1
        let next = schedule
            .next_slot_after(after)
            .expect("next slot")
            .expect("future slot");
        // 9am NY on 2026-01-01 = 14:00 UTC (EST = UTC-5)
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 1, 1, 14, 0, 0).unwrap());
    }

    #[test]
    fn cron_schedule_dst_spring_forward_does_not_panic() {
        // US clocks spring forward 2nd Sunday of March at 2am
        // 2026-03-08 is the second Sunday in March 2026
        // "0 2 * * *" = 2am NY — this hour is skipped on spring-forward day
        let schedule = TriggerSchedule::cron_with_timezone("0 2 * * *", "America/New_York")
            .expect("valid schedule");
        // just before spring forward: 2026-03-08 06:59:00 UTC = 1:59am EST
        let before_gap = Utc.with_ymd_and_hms(2026, 3, 8, 6, 59, 0).unwrap();
        // Should not panic; result may be None or next day
        let _ = schedule
            .next_slot_after(before_gap)
            .expect("no error during DST gap");
    }
}
