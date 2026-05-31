//! Scheduled trigger domain contracts for IronClaw Reborn.
//!
//! This crate owns trigger records, source-provider evaluation, deterministic
//! fire identity, and in-memory test behavior. Durable persistence, poller
//! lifecycle wiring, first-party capabilities, and outbound delivery are owned
//! by later slices.

use std::{
    collections::HashMap,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use cron::Schedule;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, Timestamp, UserId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use ulid::Ulid;

const MIN_FIRE_CADENCE: Duration = Duration::from_secs(60);
const MAX_DUE_TRIGGER_POLL_LIMIT: usize = 128;
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
    #[error("invalid schedule: {reason}")]
    InvalidSchedule { reason: String },
    #[error("trigger repository backend unavailable")]
    Backend,
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

    pub fn from_ulid(value: Ulid) -> Self {
        Self(value)
    }

    pub fn parse(value: &str) -> Result<Self, TriggerError> {
        Ulid::from_str(value)
            .map(Self)
            .map_err(|error| TriggerError::InvalidTriggerId {
                reason: error.to_string(),
            })
    }

    pub fn as_ulid(self) -> Ulid {
        self.0
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
    pub created_at: Timestamp,
}

impl TriggerRecord {
    pub fn validate(&self) -> Result<(), TriggerError> {
        if self.name.trim().is_empty() {
            return Err(TriggerError::InvalidRecord {
                reason: "trigger name must not be empty".to_string(),
            });
        }
        if self.prompt.trim().is_empty() {
            return Err(TriggerError::InvalidRecord {
                reason: "trigger prompt must not be empty".to_string(),
            });
        }
        self.schedule.validate()?;
        Ok(())
    }

    pub fn is_due_at(&self, now: Timestamp) -> bool {
        self.state == TriggerState::Scheduled && self.next_run_at <= now
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TriggerSchedule {
    Cron { expression: String },
}

impl TriggerSchedule {
    pub fn cron(expression: impl Into<String>) -> Result<Self, TriggerError> {
        let schedule = Self::Cron {
            expression: expression.into(),
        };
        schedule.validate()?;
        Ok(schedule)
    }

    pub fn validate(&self) -> Result<(), TriggerError> {
        match self {
            Self::Cron { expression } => {
                parse_cron_schedule(expression)?;
                Ok(())
            }
        }
    }

    pub fn next_slot_after(&self, after: Timestamp) -> Result<Option<Timestamp>, TriggerError> {
        match self {
            Self::Cron { expression } => Ok(parse_cron_schedule(expression)?.after(&after).next()),
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

    async fn list_triggers(&self, tenant_id: TenantId) -> Result<Vec<TriggerRecord>, TriggerError>;

    async fn remove_trigger(
        &self,
        tenant_id: TenantId,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError>;

    async fn list_due_triggers(
        &self,
        now: Timestamp,
        limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError>;
}

#[derive(Clone, Default)]
pub struct InMemoryTriggerRepository {
    state: Arc<Mutex<HashMap<TriggerRepositoryKey, TriggerRecord>>>,
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

#[async_trait]
impl TriggerRepository for InMemoryTriggerRepository {
    async fn upsert_trigger(&self, record: TriggerRecord) -> Result<(), TriggerError> {
        record.validate()?;
        let mut state = self.lock_state()?;
        state.insert(
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
            .get(&TriggerRepositoryKey::new(&tenant_id, trigger_id))
            .cloned())
    }

    async fn list_triggers(&self, tenant_id: TenantId) -> Result<Vec<TriggerRecord>, TriggerError> {
        let mut keys = {
            let state = self.lock_state()?;
            state
                .iter()
                .filter(|(_, record)| record.tenant_id == tenant_id)
                .map(|(key, record)| (record.created_at, record.trigger_id, key.clone()))
                .collect::<Vec<_>>()
        };
        keys.sort_by_key(|(created_at, trigger_id, _)| (*created_at, *trigger_id));
        let state = self.lock_state()?;
        Ok(keys
            .into_iter()
            .filter_map(|(_, _, key)| state.get(&key).cloned())
            .collect())
    }

    async fn remove_trigger(
        &self,
        tenant_id: TenantId,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Ok(self
            .lock_state()?
            .remove(&TriggerRepositoryKey::new(&tenant_id, trigger_id)))
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
        let mut keys = {
            let state = self.lock_state()?;
            state
                .iter()
                .filter(|(_, record)| record.is_due_at(now))
                .map(|(key, record)| (record.next_run_at, record.trigger_id, key.clone()))
                .collect::<Vec<_>>()
        };
        keys.sort_by_key(|(next_run_at, trigger_id, _)| (*next_run_at, *trigger_id));
        keys.truncate(limit);
        let state = self.lock_state()?;
        Ok(keys
            .into_iter()
            .filter_map(|(_, _, key)| state.get(&key).cloned())
            .collect())
    }
}

impl InMemoryTriggerRepository {
    fn lock_state(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, HashMap<TriggerRepositoryKey, TriggerRecord>>, TriggerError>
    {
        self.state.lock().map_err(|_| TriggerError::Backend)
    }
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
    use chrono::TimeZone;
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
        assert_eq!(tenant_records.len(), 2);

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
        assert!(matches!(error, TriggerError::Backend));
    }
}
