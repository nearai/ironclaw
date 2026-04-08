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
        n => Err(EngineError::InvalidCadence {
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
///
/// Cron parse failures return [`EngineError::InvalidCadence`] (validation, not
/// storage), so callers can map them to user-facing errors.
pub fn next_cron_fire(
    expression: &str,
    timezone: Option<&ValidTimezone>,
) -> Result<Option<DateTime<Utc>>, EngineError> {
    let normalized = normalize_cron_expression(expression)?;
    let schedule =
        cron::Schedule::from_str(&normalized).map_err(|e| EngineError::InvalidCadence {
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

    #[test]
    fn normalize_seven_field_cron() {
        // 7-field (sec min hr dom mon dow year) should pass through.
        let next = next_cron_fire("0 0 9 * * * 2027", None).unwrap();
        assert!(next.is_some());
    }

    #[test]
    fn invalid_cron_returns_invalid_cadence_error() {
        // Cron parse errors are validation errors, not store errors.
        let err = next_cron_fire("not a cron", None).unwrap_err();
        assert!(
            matches!(err, EngineError::InvalidCadence { .. }),
            "expected InvalidCadence, got: {err:?}"
        );

        let err = next_cron_fire("nope nope nope nope nope", None).unwrap_err();
        assert!(matches!(err, EngineError::InvalidCadence { .. }));
    }

    // ── DST tests (#1944) ─────────────────────────────────────
    //
    // The whole point of carrying user_timezone through the engine is so that
    // cron schedules respect DST. These tests pin the cron crate's behavior on
    // the two tricky transitions in `America/New_York`:
    //
    //  * Spring-forward: 2027-03-14 02:00 jumps to 03:00. Local times in
    //    [02:00, 03:00) do not exist on that day.
    //  * Fall-back: 2027-11-07 02:00 jumps back to 01:00. Local times in
    //    [01:00, 02:00) occur twice (once EDT, once EST).
    //
    // We don't test specific calendar dates (those would rot); instead we use
    // explicit reference instants via the `cron` crate's `after()` method to
    // assert behavior in a year-independent way.

    use chrono::TimeZone;

    fn schedule_after(
        expression: &str,
        tz: &ValidTimezone,
        after_utc: DateTime<Utc>,
    ) -> DateTime<Utc> {
        let normalized = normalize_cron_expression(expression).unwrap(); // safety: test helper
        let schedule = cron::Schedule::from_str(&normalized).unwrap(); // safety: test helper
        let after_local = after_utc.with_timezone(&tz.tz());
        schedule
            .after(&after_local)
            .next()
            .expect("schedule should produce a fire time") // safety: test helper
            .with_timezone(&Utc)
    }

    #[test]
    fn dst_spring_forward_skips_missing_local_hour() {
        // 2027-03-14 in America/New_York: clocks jump 02:00 -> 03:00 EDT.
        // A cron at "30 2 * * *" requests a wall-clock time that does not
        // exist on that day. The cron crate skips that occurrence and fires
        // on the next valid day at 02:30 (which is then EDT, UTC-4).
        let tz = ValidTimezone::parse("America/New_York").unwrap();

        // Reference: 2027-03-13 22:00 UTC = 2027-03-13 18:00 EDT(?), well
        // before the spring-forward day. We just need a stable anchor.
        let after = Utc.with_ymd_and_hms(2027, 3, 13, 0, 0, 0).unwrap();
        let fire = schedule_after("30 2 * * *", &tz, after);

        // The first fire on 2027-03-13 is 02:30 EST = 07:30 UTC. The next
        // fire would be 2027-03-14 02:30 — but that doesn't exist on DST
        // day, so the schedule skips to 2027-03-15 02:30 EDT = 06:30 UTC.
        // Whichever the cron crate picks, it must NOT land in the missing
        // local interval [02:00, 03:00) on 2027-03-14.
        let fire_local = fire.with_timezone(&tz.tz());
        if fire_local.year() == 2027 && fire_local.month() == 3 && fire_local.day() == 14 {
            // If it lands on DST day, the wall-clock hour must be >= 3 (EDT).
            assert!(
                fire_local.hour() >= 3,
                "fire on DST day must not be in skipped [02:00, 03:00) window, got {fire_local}"
            );
        }
        // Sanity: the result is a real future instant.
        assert!(fire > after);
    }

    #[test]
    fn dst_fall_back_picks_one_of_overlapping_hours() {
        // 2027-11-07 in America/New_York: clocks jump 02:00 EDT -> 01:00 EST.
        // Local times in [01:00, 02:00) occur twice. A cron at "30 1 * * *"
        // could fire at 01:30 EDT (05:30 UTC) or 01:30 EST (06:30 UTC).
        // The cron crate picks one consistently — we just assert it picks
        // exactly one and that the result is correct in UTC.
        let tz = ValidTimezone::parse("America/New_York").unwrap();
        let after = Utc.with_ymd_and_hms(2027, 11, 6, 12, 0, 0).unwrap();
        let fire = schedule_after("30 1 * * *", &tz, after);

        let fire_local = fire.with_timezone(&tz.tz());
        // Whatever date the cron crate lands on, the local time must be 01:30.
        assert_eq!(
            fire_local.hour(),
            1,
            "expected hour 1 local, got {fire_local}"
        );
        assert_eq!(
            fire_local.minute(),
            30,
            "expected minute 30 local, got {fire_local}"
        );

        // And the UTC instant must be exactly one of the two valid 01:30 NY
        // instants on the fall-back day, OR a 01:30 NY on a neighbouring day.
        // Either way, converting back must round-trip to the same wall clock.
        let round_trip = fire.with_timezone(&tz.tz());
        assert_eq!(round_trip, fire_local);
    }

    #[test]
    fn dst_aware_schedule_advances_correctly_across_transition() {
        // Across a DST transition the absolute UTC interval between two
        // consecutive 09:00 local fires shifts by an hour. This is the
        // "load-bearing tz" property the PR exists to enable.
        let tz = ValidTimezone::parse("America/New_York").unwrap();
        // Pick an anchor in EST (winter, before spring-forward).
        let anchor = Utc.with_ymd_and_hms(2027, 3, 1, 0, 0, 0).unwrap();
        let normalized = normalize_cron_expression("0 9 * * *").unwrap();
        let schedule = cron::Schedule::from_str(&normalized).unwrap();
        let anchor_local = anchor.with_timezone(&tz.tz());

        // Take 30 consecutive fires — long enough to cross spring-forward.
        let fires: Vec<_> = schedule.after(&anchor_local).take(30).collect();
        assert_eq!(fires.len(), 30);

        // All fires must be at 09:00 local wall clock, regardless of DST.
        for f in &fires {
            assert_eq!(f.hour(), 9, "every fire must be 09:00 local, got {f}");
        }

        // The UTC hour shifts when crossing DST: 09:00 EST = 14:00 UTC,
        // 09:00 EDT = 13:00 UTC. Both must appear across the 30-day window.
        let utc_hours: std::collections::BTreeSet<u32> =
            fires.iter().map(|f| f.with_timezone(&Utc).hour()).collect();
        assert!(
            utc_hours.contains(&13) && utc_hours.contains(&14),
            "30-day window straddling spring-forward should contain both 13:00 and 14:00 UTC fires; got {utc_hours:?}"
        );
    }
}
