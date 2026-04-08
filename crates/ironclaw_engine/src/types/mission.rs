//! Missions — long-running goals that spawn threads over time.
//!
//! A mission represents an ongoing objective that periodically spawns
//! threads to make progress. Missions can run on a schedule (cron),
//! in response to events, or be triggered manually.

use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::error::EngineError;
use crate::types::project::ProjectId;
use crate::types::thread::ThreadId;

use super::{OwnerId, default_user_id};

pub use ironclaw_common::ValidTimezone;

/// Strongly-typed mission identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MissionId(pub Uuid);

impl MissionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for MissionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MissionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Lifecycle status of a mission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionStatus {
    /// Mission is actively spawning threads on cadence.
    Active,
    /// Mission is paused — no new threads will be spawned.
    Paused,
    /// Mission has achieved its goal.
    Completed,
    /// Mission has been abandoned or failed irrecoverably.
    Failed,
}

/// How a mission triggers new threads.
///
/// The engine defines the trigger *types*. The bridge/host implements the
/// actual trigger infrastructure (cron tickers, webhook endpoints, event
/// matchers). The engine just needs to be told "fire this mission now."
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MissionCadence {
    /// Spawn on a cron schedule (e.g., "0 */6 * * *" for every 6 hours).
    Cron {
        expression: String,
        #[serde(
            default,
            deserialize_with = "ironclaw_common::deserialize_option_lenient"
        )]
        timezone: Option<ValidTimezone>,
    },
    /// Spawn in response to a channel message matching a pattern.
    OnEvent { event_pattern: String },
    /// Spawn in response to a structured system event (from tools or external).
    OnSystemEvent { source: String, event_type: String },
    /// Spawn when an external webhook is received at a registered path.
    /// The bridge registers the webhook endpoint and routes payloads here.
    Webhook {
        path: String,
        secret: Option<String>,
    },
    /// Only spawn when manually triggered (via mission_fire tool or API).
    Manual,
}

/// A mission — a long-running goal that spawns threads over time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mission {
    pub id: MissionId,
    pub project_id: ProjectId,
    /// Tenant isolation: the user who owns this mission.
    #[serde(default = "default_user_id")]
    pub user_id: String,
    pub name: String,
    pub goal: String,
    pub status: MissionStatus,
    pub cadence: MissionCadence,

    // ── Evolving strategy ──
    /// What the next thread should focus on (updated after each thread).
    pub current_focus: Option<String>,
    /// What approaches have been tried and what happened.
    pub approach_history: Vec<String>,

    // ── Progress tracking ──
    /// History of threads spawned by this mission.
    pub thread_history: Vec<ThreadId>,
    /// Optional criteria for declaring the mission complete.
    pub success_criteria: Option<String>,

    // ── Notification ──
    /// Channels to notify when a mission thread completes (e.g. "gateway", "repl").
    /// Empty means no proactive notification (results only in approach_history).
    #[serde(default)]
    pub notify_channels: Vec<String>,

    // ── Budget ──
    /// Maximum threads per day (0 = unlimited).
    pub max_threads_per_day: u32,
    /// Threads spawned today (reset daily by the cron ticker).
    pub threads_today: u32,

    // ── Trigger payload ──
    /// Payload from the most recent trigger (webhook body, event data, etc.).
    /// Injected into the thread's context so the code can access it.
    pub last_trigger_payload: Option<serde_json::Value>,

    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// When the next thread should be spawned (for Cron cadence).
    pub next_fire_at: Option<DateTime<Utc>>,
}

impl Mission {
    pub fn new(
        project_id: ProjectId,
        user_id: impl Into<String>,
        name: impl Into<String>,
        goal: impl Into<String>,
        cadence: MissionCadence,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: MissionId::new(),
            project_id,
            user_id: user_id.into(),
            name: name.into(),
            goal: goal.into(),
            status: MissionStatus::Active,
            cadence,
            current_focus: None,
            approach_history: Vec::new(),
            thread_history: Vec::new(),
            success_criteria: None,
            notify_channels: Vec::new(),
            max_threads_per_day: 10,
            threads_today: 0,
            last_trigger_payload: None,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
            created_at: now,
            updated_at: now,
            next_fire_at: None,
        }
    }

    pub fn with_success_criteria(mut self, criteria: impl Into<String>) -> Self {
        self.success_criteria = Some(criteria.into());
        self
    }

    pub fn owner_id(&self) -> OwnerId<'_> {
        OwnerId::from_user_id(&self.user_id)
    }

    pub fn is_owned_by(&self, user_id: &str) -> bool {
        self.owner_id().matches_user(user_id)
    }

    /// Record that a thread was spawned for this mission.
    pub fn record_thread(&mut self, thread_id: ThreadId) {
        self.thread_history.push(thread_id);
        self.updated_at = Utc::now();
    }

    /// Whether the mission is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            MissionStatus::Completed | MissionStatus::Failed
        )
    }
}

/// Normalize a cron expression to the 7-field format expected by the `cron` crate.
///
/// - 5-field (standard) -> prepend `0` (seconds) and append `*` (year)
/// - 6-field -> append `*` (year)
/// - 7-field -> pass through unchanged
///
/// Returns an error for any other field count rather than passing the input
/// through to `cron::Schedule::from_str`, which would surface a confusing
/// low-level parse error.
fn normalize_cron_expression(expression: &str) -> Result<String, EngineError> {
    let trimmed = expression.trim();
    let fields: Vec<&str> = trimmed.split_whitespace().collect();
    match fields.len() {
        5 => Ok(format!("0 {} *", fields.join(" "))),
        6 => Ok(format!("{} *", fields.join(" "))),
        7 => Ok(trimmed.to_string()),
        n => Err(EngineError::Store {
            reason: format!(
                "invalid cron expression '{expression}': expected 5, 6, or 7 fields, got {n}"
            ),
        }),
    }
}

/// Parse a cron expression and compute the next fire time from now.
///
/// Accepts standard 5-field, 6-field, or 7-field cron expressions (auto-normalized).
/// When a [`ValidTimezone`] is provided, the schedule is evaluated in that
/// timezone and the result is converted back to UTC. Otherwise UTC is used.
pub fn next_cron_fire(
    expression: &str,
    timezone: Option<&ValidTimezone>,
) -> Result<Option<DateTime<Utc>>, EngineError> {
    let normalized = normalize_cron_expression(expression)?;
    let schedule = cron::Schedule::from_str(&normalized).map_err(|e| EngineError::Store {
        reason: format!("invalid cron expression '{expression}': {e}"),
    })?;
    if let Some(vtz) = timezone {
        Ok(schedule
            .upcoming(vtz.tz())
            .next()
            .map(|dt| dt.with_timezone(&Utc)))
    } else {
        Ok(schedule.upcoming(Utc).next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    #[test]
    fn rejects_four_field_cron() {
        // Four-field input is not a recognized cron format. Surface a clear
        // error rather than passing through to a low-level parse failure.
        let err = next_cron_fire("* * * *", None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("expected 5, 6, or 7 fields"), "got: {msg}");
    }

    #[test]
    fn accepts_five_field_cron() {
        let next = next_cron_fire("0 9 * * *", None).unwrap();
        assert!(next.is_some(), "5-field cron should produce a fire time");
    }

    #[test]
    fn next_cron_fire_respects_timezone() {
        // "0 9 * * *" in America/New_York should produce a UTC instant whose
        // wall-clock time in NY is 09:00 on some date — and the resulting UTC
        // hour should differ from a UTC-evaluated schedule (since NY is offset
        // from UTC year-round).
        let tz = ValidTimezone::parse("America/New_York").unwrap();
        let in_ny = next_cron_fire("0 9 * * *", Some(&tz))
            .unwrap()
            .expect("schedule should produce a fire time");
        let in_utc = next_cron_fire("0 9 * * *", None)
            .unwrap()
            .expect("schedule should produce a fire time");

        // NY 09:00 in UTC is either 13:00 (EDT) or 14:00 (EST). UTC 09:00 is 09:00.
        let ny_utc_hour = in_ny.hour();
        assert!(
            ny_utc_hour == 13 || ny_utc_hour == 14,
            "NY 09:00 should map to UTC 13 or 14, got {ny_utc_hour}"
        );
        assert_eq!(in_utc.hour(), 9, "UTC schedule should fire at hour 9");
        assert_ne!(
            in_ny.hour(),
            in_utc.hour(),
            "tz-aware and tz-naive schedules should differ"
        );

        // Sanity: result is a real future date, not the epoch.
        assert!(in_ny.year() >= 2026);
    }

    #[test]
    fn normalize_six_field_cron() {
        // 6-field (with seconds) should be accepted.
        let next = next_cron_fire("0 0 9 * * *", None).unwrap();
        assert!(next.is_some());
    }
}
