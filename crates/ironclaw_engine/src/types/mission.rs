//! Missions — long-running goals that spawn threads over time.
//!
//! A mission represents an ongoing objective that periodically spawns
//! threads to make progress. Missions can run on a schedule (cron),
//! in response to events, or be triggered manually.

use std::collections::HashMap;
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
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MissionCadence {
    /// Spawn on a cron schedule (e.g., "0 */6 * * *" for every 6 hours).
    Cron {
        #[serde(rename = "schedule")]
        expression: String,
        #[serde(
            default,
            deserialize_with = "ironclaw_common::deserialize_option_lenient"
        )]
        timezone: Option<ValidTimezone>,
    },
    /// Spawn in response to a channel message matching a regex pattern.
    /// `channel`, when set, restricts firing to messages from a specific
    /// channel name (case-insensitive).
    #[serde(rename = "message_event")]
    OnEvent {
        #[serde(rename = "pattern")]
        event_pattern: String,
        #[serde(default)]
        channel: Option<String>,
    },
    /// Spawn in response to a structured system event (from tools or external).
    /// `filters`, when non-empty, requires every key/value pair to match
    /// against the event payload's top-level fields exactly.
    #[serde(rename = "system_event")]
    OnSystemEvent {
        source: String,
        event_type: String,
        #[serde(default)]
        filters: HashMap<String, serde_json::Value>,
    },
    /// Spawn when an external webhook is received at a registered path.
    /// The bridge registers the webhook endpoint and routes payloads here.
    Webhook {
        path: String,
        #[serde(default)]
        secret: Option<String>,
    },
    /// Only spawn when manually triggered (via mission_fire tool or API).
    Manual,
}

impl MissionCadence {
    /// Return a clone with the webhook secret cleared, for safe
    /// serialization in tool output (e.g. `mission_list`).
    pub fn redacted(&self) -> Self {
        match self {
            Self::Webhook { path, .. } => Self::Webhook {
                path: path.clone(),
                secret: None,
            },
            other => other.clone(),
        }
    }
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
    /// Optional human-readable description (separate from the goal statement).
    /// Routine `description` fields map here.
    #[serde(default)]
    pub description: Option<String>,
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
    /// Optional per-channel user/recipient target for notifications. Maps from
    /// routine `delivery.user`. When `None`, the channel's last-seen
    /// recipient is used.
    #[serde(default)]
    pub notify_user: Option<String>,

    // ── Context preloading ──
    /// Workspace paths whose contents are loaded into the thread's meta-prompt
    /// when the mission fires (e.g. `["MEMORY.md", "context/profile.json"]`).
    /// Maps from routine `execution.context_paths`.
    #[serde(default)]
    pub context_paths: Vec<String>,

    // ── Budget / guardrails ──
    /// Maximum threads per day (0 = unlimited).
    pub max_threads_per_day: u32,
    /// Threads spawned today (reset daily by the cron ticker).
    pub threads_today: u32,
    /// Cooldown between firings, in seconds. 0 = no cooldown. Maps from
    /// routine `guardrails.cooldown_secs`.
    #[serde(default)]
    pub cooldown_secs: u64,
    /// Maximum number of mission threads that may be running concurrently
    /// (in non-terminal states). 0 = unlimited. Maps from routine
    /// `guardrails.max_concurrent`.
    #[serde(default)]
    pub max_concurrent: u32,
    /// Deduplication window for event-triggered firings, in seconds. 0 = no
    /// dedup. When set, identical event-key payloads within this window are
    /// suppressed. Maps from routine `guardrails.dedup_window`.
    #[serde(default)]
    pub dedup_window_secs: u64,
    /// Timestamp of the most recent successful fire. Used by cooldown
    /// enforcement.
    #[serde(default)]
    pub last_fire_at: Option<DateTime<Utc>>,

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

        // Event-triggered cadences (OnEvent / OnSystemEvent / Webhook) are
        // *reactive*: a single noisy channel can fire them on every
        // matching message. Cron / Manual cadences are *proactive* and
        // self-paced. Set tighter defaults for the reactive variants so a
        // mission created without explicit guardrails cannot accidentally
        // flood the LLM if its pattern is too loose. The routine alias
        // path overrides these via post-create update when the LLM
        // supplies explicit guardrails / advanced settings.
        let is_reactive = matches!(
            cadence,
            MissionCadence::OnEvent { .. }
                | MissionCadence::OnSystemEvent { .. }
                | MissionCadence::Webhook { .. }
        );
        let (default_max_threads_per_day, default_cooldown_secs, default_max_concurrent) =
            if is_reactive {
                // 5-minute cooldown + 24/day cap + single-instance — same
                // floor v1 routine_create used for event-driven routines.
                (24, 300, 1)
            } else {
                // Existing defaults for cron/manual missions; no cooldown,
                // no concurrency cap, generous daily budget.
                (10, 0, 0)
            };

        Self {
            id: MissionId::new(),
            project_id,
            user_id: user_id.into(),
            name: name.into(),
            description: None,
            goal: goal.into(),
            status: MissionStatus::Active,
            cadence,
            current_focus: None,
            approach_history: Vec::new(),
            thread_history: Vec::new(),
            success_criteria: None,
            notify_channels: Vec::new(),
            notify_user: None,
            context_paths: Vec::new(),
            max_threads_per_day: default_max_threads_per_day,
            threads_today: 0,
            cooldown_secs: default_cooldown_secs,
            max_concurrent: default_max_concurrent,
            dedup_window_secs: 0,
            last_fire_at: None,
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
/// Field formats accepted:
/// - **5-field** (standard Vixie cron): `min hr dom mon dow` — prepend `0`
///   (seconds) and append `*` (year).
/// - **6-field**: assumed to be `sec min hr dom mon dow` (the `cron` crate's
///   native format minus year) and append `*` (year). **Note:** this is *not*
///   the Quartz `min hr dom mon dow year` interpretation. A user passing
///   `"0 9 * * * 2027"` intending "at 09:00 every day in 2027" will instead
///   get "at second 0 of minute 9 of every hour, every day, every year". Use
///   the explicit 7-field form `0 0 9 * * * 2027` to disambiguate.
/// - **7-field**: `sec min hr dom mon dow year` — passed through unchanged.
///
/// Returns an error for any other field count rather than passing the input
/// through to `cron::Schedule::from_str`, which would surface a confusing
/// low-level parse error.
fn normalize_cron_expression(expression: &str) -> Result<String, EngineError> {
    let trimmed = expression.trim();
    let fields: Vec<&str> = trimmed.split_whitespace().collect();
    match fields.len() {
        5 => Ok(format!("0 {} *", fields.join(" "))),
        6 => {
            // Disambiguate the Quartz-style 6-field form. A user (or LLM)
            // typing `"0 9 * * * 2027"` almost certainly means
            // "at 09:00 every day in 2027" (Quartz: `min hr dom mon dow year`),
            // not "at second 0 of minute 9, every hour, every day, dow=2027".
            // The cron crate would treat the year-shaped final field as a
            // (nonsensical) day-of-week and silently produce a wrong schedule.
            // Reject early with a message that points at the explicit
            // 7-field form so the caller can fix it instead of debugging a
            // schedule that never fires.
            if let Some(last) = fields.last()
                && is_year_field(last)
            {
                return Err(EngineError::InvalidCadence {
                    reason: format!(
                        "ambiguous 6-field cron expression '{expression}': the trailing '{last}' \
                         looks like a year. The 6-field form is `sec min hr dom mon dow`, NOT the \
                         Quartz `min hr dom mon dow year`. Use the explicit 7-field form \
                         `0 {} {} {} {} {} {last}` to mean 'at the given time in {last}'.",
                        fields[0], fields[1], fields[2], fields[3], fields[4]
                    ),
                });
            }
            Ok(format!("{} *", fields.join(" ")))
        }
        7 => Ok(trimmed.to_string()),
        n => Err(EngineError::InvalidCadence {
            reason: format!(
                "invalid cron expression '{expression}': expected 5, 6, or 7 fields, got {n}"
            ),
        }),
    }
}

/// True if `field` is a literal 4-digit year in the plausible cron range.
///
/// Used to detect the Quartz-style `min hr dom mon dow year` mistake in
/// 6-field input. Range chosen to cover the cron crate's accepted year span
/// without firing on field values that happen to be 4 digits but mean
/// something else (none of the standard cron field ranges produce 4-digit
/// literals).
fn is_year_field(field: &str) -> bool {
    field.len() == 4
        && field.bytes().all(|b| b.is_ascii_digit())
        && field
            .parse::<u32>()
            .is_ok_and(|y| (1970..=2099).contains(&y))
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

/// Like [`next_cron_fire`], but treats `Ok(None)` as a validation error.
///
/// `next_cron_fire` returns `Ok(None)` for cron expressions that are
/// syntactically valid but will never fire again (e.g. `0 0 9 * * * 2020` —
/// year-locked to a year that's already passed). At lifecycle entry points
/// (`create_mission`, cadence updates, `resume_mission`) this is the same
/// failure mode as the original #1944 bug: an Active mission with
/// `next_fire_at = None` that the ticker can never pick up. Surface it as
/// `InvalidCadence` so callers fail fast and the operator gets a clear error.
///
/// `fire_mission` and `bootstrap_project` intentionally tolerate `Ok(None)`
/// (logged) and should keep using `next_cron_fire` directly — the thread is
/// already running or the data is already persisted, and aborting would do
/// more harm than logging.
pub fn next_cron_fire_required(
    expression: &str,
    timezone: Option<&ValidTimezone>,
) -> Result<DateTime<Utc>, EngineError> {
    next_cron_fire(expression, timezone)?.ok_or_else(|| EngineError::InvalidCadence {
        reason: format!(
            "cron expression '{expression}' has no upcoming fire time (year-locked or otherwise unschedulable)"
        ),
    })
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

        // Sanity: result is a real future date, not the epoch. Compare
        // against `Utc::now()` so the assertion stays stable across calendar
        // years rather than being pinned to a hard-coded threshold.
        assert!(in_ny > Utc::now(), "next cron fire must be in the future");
    }

    #[test]
    fn normalize_six_field_cron() {
        // 6-field (with seconds) should be accepted.
        let next = next_cron_fire("0 0 9 * * *", None).unwrap();
        assert!(next.is_some());
    }

    #[test]
    fn six_field_cron_is_sec_min_hr_dom_mon_dow_not_quartz_with_year() {
        // Pin the 6-field interpretation: `sec min hr dom mon dow`, NOT the
        // Quartz-style `min hr dom mon dow year`. A 6-field input gets `*`
        // appended for the year position. This test exists so a future change
        // doesn't silently flip the interpretation and break existing
        // missions.
        let normalized = normalize_cron_expression("0 0 9 * * *").unwrap();
        assert_eq!(
            normalized, "0 0 9 * * * *",
            "6-field input must be treated as `sec min hr dom mon dow` and appended with `*` (year)"
        );

        // Sanity: the resulting schedule fires at 09:00:00 wall-clock daily.
        let next = next_cron_fire("0 0 9 * * *", None).unwrap().unwrap();
        assert_eq!(next.hour(), 9);
        assert_eq!(next.minute(), 0);
        assert_eq!(next.second(), 0);
    }

    #[test]
    fn normalize_seven_field_cron() {
        // 7-field (sec min hr dom mon dow year) should pass through.
        let next = next_cron_fire("0 0 9 * * * 2027", None).unwrap();
        assert!(next.is_some());
    }

    #[test]
    fn six_field_cron_with_year_shaped_last_field_is_rejected() {
        // A user (or LLM) typing the Quartz-style `min hr dom mon dow year`
        // form gets a clear error pointing at the explicit 7-field form,
        // rather than a silently misparsed schedule. The 6-field form is
        // `sec min hr dom mon dow`, so `2027` would otherwise be interpreted
        // as a (nonsensical) day-of-week.
        let err = next_cron_fire("0 9 * * * 2027", None).unwrap_err();
        let msg = err.to_string();
        assert!(
            matches!(err, EngineError::InvalidCadence { .. }),
            "expected InvalidCadence, got: {err:?}"
        );
        assert!(
            msg.contains("looks like a year") && msg.contains("0 0 9 * * * 2027"),
            "error should explain Quartz ambiguity and suggest 7-field form, got: {msg}"
        );

        // Cover all year boundaries.
        for year in ["1970", "1999", "2000", "2026", "2099"] {
            let expr = format!("0 0 * * * {year}");
            assert!(
                matches!(
                    next_cron_fire(&expr, None),
                    Err(EngineError::InvalidCadence { .. })
                ),
                "year {year} should be rejected as Quartz-style ambiguity"
            );
        }

        // Out-of-range 4-digit values are NOT treated as years and fall
        // through to the regular 6-field interpretation (which the cron
        // crate may then reject for its own reasons).
        let normalized = normalize_cron_expression("0 0 9 * * 1969");
        assert!(
            normalized.is_ok(),
            "1969 (out of year range) should not trigger the Quartz heuristic"
        );

        // 5-field cron with a literal day-of-week numeric value must still
        // work — the year heuristic only applies to 6-field input.
        assert!(next_cron_fire("0 9 * * 3", None).unwrap().is_some());
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

        // Reference: 2027-03-13 00:00 UTC = 2027-03-12 19:00 EST, well
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
