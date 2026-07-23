use std::{num::NonZeroU32, sync::Arc};

use chrono::{NaiveTime, Timelike};
use chrono_tz::Tz;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, Timestamp, UserId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ulid::Ulid;

use crate::{
    TriggerDeliveryTargetId, TriggerError, TriggerId, TriggerRecord, TriggerRepository,
    TriggerSchedule, TriggerSourceKind, TriggerState, update_length_prefixed,
};

const HEARTBEAT_TRIGGER_ID_VERSION: &str = "ironclaw.heartbeat-trigger.v1";
const HEARTBEAT_TRIGGER_NAME: &str = "Heartbeat";
const HEARTBEAT_TRIGGER_PROMPT: &str =
    "Evaluate the current scoped HEARTBEAT.md checklist using the heartbeat run profile.";
const MAX_HEARTBEAT_FAILURE_LIMIT: u32 = 100;

/// Exact owner scope for one system-managed heartbeat schedule.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HeartbeatScope {
    pub tenant_id: TenantId,
    pub creator_user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
}

impl HeartbeatScope {
    /// Stable trigger identity for this complete scope.
    pub fn trigger_id(&self) -> TriggerId {
        let mut hasher = Sha256::new();
        hasher.update(HEARTBEAT_TRIGGER_ID_VERSION.as_bytes());
        hasher.update([0]);
        update_length_prefixed(&mut hasher, self.tenant_id.as_str().as_bytes());
        update_length_prefixed(&mut hasher, self.creator_user_id.as_str().as_bytes());
        update_optional_id(&mut hasher, self.agent_id.as_ref().map(AgentId::as_str));
        update_optional_id(&mut hasher, self.project_id.as_ref().map(ProjectId::as_str));
        let digest = hasher.finalize();
        let mut bytes = [0_u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        TriggerId(Ulid::from_bytes(bytes))
    }
}

fn update_optional_id(hasher: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            update_length_prefixed(hasher, value.as_bytes());
        }
        None => hasher.update([0]),
    }
}

/// Heartbeat cadence in whole minutes.
///
/// V1 accepts cron-representable fixed cadences that divide an hour, or whole
/// hour cadences that divide a day. This keeps the typed interval faithful to
/// the existing cron schedule rather than silently approximating it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct HeartbeatInterval(u32);

impl HeartbeatInterval {
    pub fn from_minutes(minutes: u32) -> Result<Self, TriggerError> {
        Self::try_from(minutes)
    }

    pub fn minutes(self) -> u32 {
        self.0
    }

    fn cron_expression(self) -> Result<String, TriggerError> {
        match self.0 {
            minutes if minutes <= 60 && 60 % minutes == 0 => Ok(format!("0 */{minutes} * * * *")),
            minutes if minutes % 60 == 0 => {
                let hours = minutes / 60;
                if hours <= 24 && 24 % hours == 0 {
                    Ok(format!("0 0 */{hours} * * *"))
                } else {
                    Err(invalid_config(
                        "interval must divide one hour or be a whole-hour interval that divides one day",
                    ))
                }
            }
            _ => Err(invalid_config(
                "interval must divide one hour or be a whole-hour interval that divides one day",
            )),
        }
    }
}

impl TryFrom<u32> for HeartbeatInterval {
    type Error = TriggerError;

    fn try_from(minutes: u32) -> Result<Self, Self::Error> {
        if minutes == 0 {
            return Err(invalid_config("interval must be non-zero"));
        }
        let interval = Self(minutes);
        interval.cron_expression()?;
        Ok(interval)
    }
}

impl From<HeartbeatInterval> for u32 {
    fn from(value: HeartbeatInterval) -> Self {
        value.0
    }
}

/// Local-time quiet window evaluated in the heartbeat timezone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatQuietHours {
    pub start: NaiveTime,
    pub end: NaiveTime,
}

impl HeartbeatQuietHours {
    pub fn new(start: NaiveTime, end: NaiveTime) -> Self {
        Self { start, end }
    }

    /// Parse operator-facing `HH:MM` or `HH:MM:SS` local times.
    pub fn parse(start: &str, end: &str) -> Result<Self, TriggerError> {
        fn parse_one(label: &str, value: &str) -> Result<NaiveTime, TriggerError> {
            ["%H:%M", "%H:%M:%S"]
                .into_iter()
                .find_map(|format| NaiveTime::parse_from_str(value, format).ok())
                .ok_or_else(|| {
                    invalid_config(format!(
                        "quiet_hours.{label} must be a local time in HH:MM or HH:MM:SS format"
                    ))
                })
        }

        Ok(Self {
            start: parse_one("start", start)?,
            end: parse_one("end", end)?,
        })
    }

    fn contains(self, local_time: NaiveTime) -> bool {
        if self.start == self.end {
            return true;
        }
        if self.start < self.end {
            local_time >= self.start && local_time < self.end
        } else {
            local_time >= self.start || local_time < self.end
        }
    }
}

/// Durable host-owned metadata attached only to managed heartbeat triggers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatSystemMetadata {
    pub quiet_hours: Option<HeartbeatQuietHours>,
    pub failure_limit: NonZeroU32,
}

/// Provenance for an ordinary user schedule or a host-managed automation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "config")]
pub enum TriggerAutomation {
    #[default]
    UserSchedule,
    Heartbeat(HeartbeatSystemMetadata),
}

impl TriggerAutomation {
    pub fn is_heartbeat(&self) -> bool {
        matches!(self, Self::Heartbeat(_))
    }

    pub(crate) fn validate_for_record(&self, record: &TriggerRecord) -> Result<(), TriggerError> {
        let Self::Heartbeat(metadata) = self else {
            return Ok(());
        };
        if !record.schedule.is_recurring() {
            return Err(invalid_config(
                "heartbeat must use a recurring trigger schedule",
            ));
        }
        if metadata.failure_limit.get() > MAX_HEARTBEAT_FAILURE_LIMIT {
            return Err(invalid_config(format!(
                "failure_limit must be at most {MAX_HEARTBEAT_FAILURE_LIMIT}"
            )));
        }
        Ok(())
    }

    pub(crate) fn is_quiet_at(
        &self,
        schedule: &TriggerSchedule,
        at: Timestamp,
    ) -> Result<bool, TriggerError> {
        let Self::Heartbeat(metadata) = self else {
            return Ok(false);
        };
        let Some(quiet_hours) = metadata.quiet_hours else {
            return Ok(false);
        };
        let timezone = schedule
            .timezone_text()
            .parse::<Tz>()
            .map_err(|error| invalid_config(format!("invalid timezone: {error}")))?;
        let local_time = at.with_timezone(&timezone).time();
        Ok(quiet_hours.contains(
            NaiveTime::from_hms_opt(local_time.hour(), local_time.minute(), local_time.second())
                .ok_or_else(|| invalid_config("invalid local quiet-hours time"))?,
        ))
    }
}

/// Typed, opt-in heartbeat configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatConfig {
    pub enabled: bool,
    pub interval: HeartbeatInterval,
    pub timezone: String,
    pub quiet_hours: Option<HeartbeatQuietHours>,
    pub delivery_target: Option<TriggerDeliveryTargetId>,
    pub failure_limit: NonZeroU32,
}

impl HeartbeatConfig {
    pub fn validate(&self) -> Result<(), TriggerError> {
        self.interval.cron_expression()?;
        self.timezone
            .parse::<Tz>()
            .map_err(|error| invalid_config(format!("invalid timezone: {error}")))?;
        if self.failure_limit.get() > MAX_HEARTBEAT_FAILURE_LIMIT {
            return Err(invalid_config(format!(
                "failure_limit must be at most {MAX_HEARTBEAT_FAILURE_LIMIT}"
            )));
        }
        Ok(())
    }

    fn schedule(&self) -> Result<TriggerSchedule, TriggerError> {
        TriggerSchedule::cron_with_timezone(self.interval.cron_expression()?, self.timezone.clone())
    }
}

/// Domain-owned reconciliation service over the existing trigger repository.
pub struct HeartbeatScheduleService {
    repository: Arc<dyn TriggerRepository>,
}

impl HeartbeatScheduleService {
    pub fn new(repository: Arc<dyn TriggerRepository>) -> Self {
        Self { repository }
    }

    /// Reconcile exactly one deterministic managed trigger for `scope`.
    ///
    /// The repository upsert is keyed by the deterministic trigger id, so
    /// concurrent or repeated reconciliation cannot create duplicate rows.
    pub async fn reconcile(
        &self,
        scope: HeartbeatScope,
        config: HeartbeatConfig,
        now: Timestamp,
    ) -> Result<TriggerRecord, TriggerError> {
        config.validate()?;
        let trigger_id = scope.trigger_id();
        let schedule = config.schedule()?;
        let next_run_at = schedule
            .next_slot_after(now)?
            .ok_or_else(|| invalid_config("heartbeat schedule has no future slot"))?;
        let existing = self
            .repository
            .get_trigger(scope.tenant_id.clone(), trigger_id)
            .await?;
        if let Some(record) = existing.as_ref()
            && (record.creator_user_id != scope.creator_user_id
                || record.agent_id != scope.agent_id
                || record.project_id != scope.project_id)
        {
            return Err(TriggerError::InvalidRecord {
                kind: crate::TriggerRecordValidationKind::Other,
                reason: "managed heartbeat trigger scope does not match its deterministic identity"
                    .to_string(),
            });
        }

        let record = TriggerRecord {
            trigger_id,
            tenant_id: scope.tenant_id,
            creator_user_id: scope.creator_user_id,
            agent_id: scope.agent_id,
            project_id: scope.project_id,
            name: HEARTBEAT_TRIGGER_NAME.to_string(),
            source: TriggerSourceKind::Schedule,
            schedule,
            prompt: HEARTBEAT_TRIGGER_PROMPT.to_string(),
            delivery_target: config.delivery_target,
            automation: TriggerAutomation::Heartbeat(HeartbeatSystemMetadata {
                quiet_hours: config.quiet_hours,
                failure_limit: config.failure_limit,
            }),
            state: if config.enabled {
                TriggerState::Scheduled
            } else {
                TriggerState::Paused
            },
            next_run_at,
            last_run_at: existing.as_ref().and_then(|record| record.last_run_at),
            last_fired_slot: existing.as_ref().and_then(|record| record.last_fired_slot),
            last_status: existing.as_ref().and_then(|record| record.last_status),
            active_fire_slot: existing.as_ref().and_then(|record| record.active_fire_slot),
            active_run_ref: existing.as_ref().and_then(|record| record.active_run_ref),
            created_at: existing.as_ref().map_or(now, |record| record.created_at),
        };
        record.validate()?;
        self.repository.upsert_trigger(record.clone()).await?;
        Ok(record)
    }
}

fn invalid_config(reason: impl Into<String>) -> TriggerError {
    TriggerError::InvalidHeartbeatConfig {
        reason: reason.into(),
    }
}

pub(crate) fn automation_to_storage(
    automation: &TriggerAutomation,
) -> Result<Option<String>, TriggerError> {
    match automation {
        TriggerAutomation::UserSchedule => Ok(None),
        TriggerAutomation::Heartbeat(_) => {
            serde_json::to_string(automation)
                .map(Some)
                .map_err(|error| TriggerError::InvalidRecord {
                    kind: crate::TriggerRecordValidationKind::Other,
                    reason: format!("automation metadata serialization failed: {error}"),
                })
        }
    }
}

pub(crate) fn automation_from_storage(
    value: Option<&str>,
) -> Result<TriggerAutomation, TriggerError> {
    value.map_or(Ok(TriggerAutomation::UserSchedule), |value| {
        serde_json::from_str(value).map_err(|error| TriggerError::InvalidRecord {
            kind: crate::TriggerRecordValidationKind::Other,
            reason: format!("automation metadata is invalid: {error}"),
        })
    })
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveTime, TimeZone, Utc};

    use super::*;
    use crate::{InMemoryTriggerRepository, ScheduleTriggerSourceProvider, TriggerSourceProvider};

    fn scope(user: &str) -> HeartbeatScope {
        HeartbeatScope {
            tenant_id: TenantId::new("tenant-a").expect("valid tenant"),
            creator_user_id: UserId::new(user).expect("valid user"),
            agent_id: Some(AgentId::new("agent-a").expect("valid agent")),
            project_id: Some(ProjectId::new("project-a").expect("valid project")),
        }
    }

    fn config(enabled: bool) -> HeartbeatConfig {
        HeartbeatConfig {
            enabled,
            interval: HeartbeatInterval::from_minutes(30).expect("valid interval"),
            timezone: "America/Los_Angeles".to_string(),
            quiet_hours: None,
            delivery_target: None,
            failure_limit: NonZeroU32::new(3).expect("non-zero"),
        }
    }

    #[test]
    fn stable_id_covers_the_complete_owner_scope() {
        assert_eq!(scope("user-a").trigger_id(), scope("user-a").trigger_id());
        assert_ne!(scope("user-a").trigger_id(), scope("user-b").trigger_id());

        let mut other_project = scope("user-a");
        other_project.project_id = Some(ProjectId::new("project-b").expect("valid project"));
        assert_ne!(scope("user-a").trigger_id(), other_project.trigger_id());
    }

    #[test]
    fn interval_rejects_cron_inexact_values() {
        assert!(HeartbeatInterval::from_minutes(30).is_ok());
        assert!(HeartbeatInterval::from_minutes(120).is_ok());
        assert!(HeartbeatInterval::from_minutes(0).is_err());
        assert!(HeartbeatInterval::from_minutes(7).is_err());
        assert!(HeartbeatInterval::from_minutes(1_500).is_err());
    }

    #[test]
    fn quiet_hours_cover_cross_midnight_and_equal_full_day_windows() {
        let cross_midnight = HeartbeatQuietHours::new(
            NaiveTime::from_hms_opt(22, 0, 0).expect("valid time"),
            NaiveTime::from_hms_opt(7, 0, 0).expect("valid time"),
        );
        assert!(cross_midnight.contains(NaiveTime::from_hms_opt(23, 30, 0).expect("valid time")));
        assert!(cross_midnight.contains(NaiveTime::from_hms_opt(6, 59, 0).expect("valid time")));
        assert!(!cross_midnight.contains(NaiveTime::from_hms_opt(7, 0, 0).expect("valid time")));

        let all_day = HeartbeatQuietHours::new(
            NaiveTime::from_hms_opt(9, 0, 0).expect("valid time"),
            NaiveTime::from_hms_opt(9, 0, 0).expect("valid time"),
        );
        assert!(all_day.contains(NaiveTime::from_hms_opt(12, 0, 0).expect("valid time")));
    }

    #[tokio::test]
    async fn reconcile_is_idempotent_opt_in_and_scope_isolated() {
        let repository: Arc<dyn TriggerRepository> = Arc::new(InMemoryTriggerRepository::default());
        let service = HeartbeatScheduleService::new(Arc::clone(&repository));
        let now = Utc
            .with_ymd_and_hms(2026, 7, 23, 12, 0, 0)
            .single()
            .expect("valid timestamp");

        let paused = service
            .reconcile(scope("user-a"), config(false), now)
            .await
            .expect("disabled heartbeat reconciles");
        assert_eq!(paused.state, TriggerState::Paused);
        assert!(paused.automation.is_heartbeat());

        let enabled = service
            .reconcile(scope("user-a"), config(true), now)
            .await
            .expect("enabled heartbeat reconciles");
        assert_eq!(enabled.trigger_id, paused.trigger_id);
        assert_eq!(enabled.state, TriggerState::Scheduled);

        let other = service
            .reconcile(scope("user-b"), config(true), now)
            .await
            .expect("other user heartbeat reconciles");
        assert_ne!(other.trigger_id, enabled.trigger_id);
        assert_eq!(
            repository
                .list_triggers(scope("user-a").tenant_id)
                .await
                .expect("list tenant")
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn quiet_due_slot_is_suppressed_before_turn_materialization() {
        let now = Utc
            .with_ymd_and_hms(2026, 7, 23, 6, 0, 0)
            .single()
            .expect("valid timestamp");
        let mut heartbeat_config = config(true);
        heartbeat_config.timezone = "UTC".to_string();
        heartbeat_config.quiet_hours = Some(HeartbeatQuietHours::new(
            NaiveTime::from_hms_opt(0, 0, 0).expect("valid time"),
            NaiveTime::from_hms_opt(7, 0, 0).expect("valid time"),
        ));
        let repository: Arc<dyn TriggerRepository> = Arc::new(InMemoryTriggerRepository::default());
        let record = HeartbeatScheduleService::new(repository)
            .reconcile(scope("user-a"), heartbeat_config, now)
            .await
            .expect("heartbeat reconciles");
        let mut due = record;
        due.next_run_at = now;

        let fire = ScheduleTriggerSourceProvider
            .evaluate(&due, now)
            .await
            .expect("source evaluation succeeds");
        assert!(fire.is_none(), "quiet heartbeat must not submit a fire");
    }
}
