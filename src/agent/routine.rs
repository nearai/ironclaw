//! Core types for the routines system.
//!
//! A routine is a named, persistent, user-owned task with a trigger and an action.
//! Each routine fires independently when its trigger condition is met, with only
//! that routine's prompt and context sent to the LLM.
//!
//! ```text
//! ┌──────────┐     ┌─────────┐     ┌──────────────────┐
//! │  Trigger  │────▶│ Engine  │────▶│  Execution Mode  │
//! │ cron/event│     │guardrail│     │lightweight│full_job│
//! │ system    │     │ check   │     └──────────────────┘
//! │ manual    │     └─────────┘              │
//! └──────────┘                               ▼
//!                                     ┌──────────────┐
//!                                     │  Notify user │
//!                                     │  if needed   │
//!                                     └──────────────┘
//! ```

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::RoutineError;

/// A routine is a named, persistent, user-owned task with a trigger and an action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Routine {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub user_id: String,
    pub enabled: bool,
    pub trigger: Trigger,
    pub action: RoutineAction,
    pub guardrails: RoutineGuardrails,
    pub notify: NotifyConfig,

    // Runtime state (DB-managed)
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_fire_at: Option<DateTime<Utc>>,
    pub run_count: u64,
    pub consecutive_failures: u32,
    pub state: serde_json::Value,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// When a routine should fire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Trigger {
    /// Fire on a cron schedule (e.g. "0 9 * * MON-FRI" or "every 2h").
    Cron {
        schedule: String,
        #[serde(default)]
        timezone: Option<String>,
    },
    /// Fire when a channel message matches a pattern.
    Event {
        /// Optional channel filter (e.g. "telegram", "slack").
        channel: Option<String>,
        /// Regex pattern to match against message content.
        pattern: String,
    },
    /// Fire when a structured system event is emitted.
    SystemEvent {
        /// Event source namespace (e.g. "github", "workflow", "tool").
        source: String,
        /// Event type within the source (e.g. "issue.opened").
        event_type: String,
        /// Optional exact-match filters against payload top-level fields.
        #[serde(default)]
        filters: std::collections::HashMap<String, String>,
    },
    /// Only fires via tool call or CLI.
    Manual,
}

impl Trigger {
    /// The string tag stored in the DB trigger_type column.
    pub fn type_tag(&self) -> &'static str {
        match self {
            Trigger::Cron { .. } => "cron",
            Trigger::Event { .. } => "event",
            Trigger::SystemEvent { .. } => "system_event",
            Trigger::Manual => "manual",
        }
    }

    /// Parse a trigger from its DB representation.
    pub fn from_db(trigger_type: &str, config: serde_json::Value) -> Result<Self, RoutineError> {
        match trigger_type {
            "cron" => {
                let schedule = config
                    .get("schedule")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RoutineError::MissingField {
                        context: "cron trigger".into(),
                        field: "schedule".into(),
                    })?
                    .to_string();
                let timezone = config
                    .get("timezone")
                    .and_then(|v| v.as_str())
                    .and_then(|tz| {
                        if crate::timezone::parse_timezone(tz).is_some() {
                            Some(tz.to_string())
                        } else {
                            tracing::warn!(
                                "Ignoring invalid timezone '{}' from DB for cron trigger",
                                tz
                            );
                            None
                        }
                    });
                Ok(Trigger::Cron { schedule, timezone })
            }
            "event" => {
                let pattern = config
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RoutineError::MissingField {
                        context: "event trigger".into(),
                        field: "pattern".into(),
                    })?
                    .to_string();
                let channel = config
                    .get("channel")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                Ok(Trigger::Event { channel, pattern })
            }
            "system_event" => {
                let source = config
                    .get("source")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RoutineError::MissingField {
                        context: "system_event trigger".into(),
                        field: "source".into(),
                    })?
                    .to_string();
                let event_type = config
                    .get("event_type")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RoutineError::MissingField {
                        context: "system_event trigger".into(),
                        field: "event_type".into(),
                    })?
                    .to_string();
                let filters = config
                    .get("filters")
                    .and_then(|v| v.as_object())
                    .map(|m| {
                        m.iter()
                            .filter_map(|(k, v)| {
                                json_value_as_filter_string(v).map(|s| (k.clone(), s))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Ok(Trigger::SystemEvent {
                    source,
                    event_type,
                    filters,
                })
            }
            "manual" => Ok(Trigger::Manual),
            other => Err(RoutineError::UnknownTriggerType {
                trigger_type: other.to_string(),
            }),
        }
    }

    /// Serialize trigger-specific config to JSON for DB storage.
    pub fn to_config_json(&self) -> serde_json::Value {
        match self {
            Trigger::Cron { schedule, timezone } => serde_json::json!({
                "schedule": schedule,
                "timezone": timezone,
            }),
            Trigger::Event { channel, pattern } => serde_json::json!({
                "pattern": pattern,
                "channel": channel,
            }),
            Trigger::SystemEvent {
                source,
                event_type,
                filters,
            } => serde_json::json!({
                "source": source,
                "event_type": event_type,
                "filters": filters,
            }),
            Trigger::Manual => serde_json::json!({}),
        }
    }
}

/// What happens when a routine fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoutineAction {
    /// Single LLM call (optionally with tools). Cheap and fast.
    Lightweight {
        /// The prompt sent to the LLM.
        prompt: String,
        /// Workspace paths to load as context (e.g. ["context/priorities.md"]).
        #[serde(default)]
        context_paths: Vec<String>,
        /// Max output tokens (default: 4096).
        #[serde(default = "default_max_tokens")]
        max_tokens: u32,
        /// Enable tool access (default: false for backward compatibility).
        /// When true, the LLM can call tools during execution.
        /// Tools requiring approval are automatically filtered out.
        #[serde(default)]
        use_tools: bool,
        /// Max tool call rounds (default: 3). Only used when use_tools is true.
        #[serde(default = "default_max_tool_rounds")]
        max_tool_rounds: u32,
    },
    /// Full multi-turn worker job with tool access.
    FullJob {
        /// Job title for the scheduler.
        title: String,
        /// Job description / initial prompt.
        description: String,
        /// Max reasoning iterations (default: 10).
        #[serde(default = "default_max_iterations")]
        max_iterations: u32,
    },
}

fn default_max_tokens() -> u32 {
    4096
}

fn default_max_iterations() -> u32 {
    10
}

fn default_max_tool_rounds() -> u32 {
    3
}

/// Hard upper bound for max_tool_rounds to prevent runaway loops and cost explosion.
pub(crate) const MAX_TOOL_ROUNDS_LIMIT: u32 = 20;

/// Clamp max_tool_rounds to [1, MAX_TOOL_ROUNDS_LIMIT].
/// Accepts u64 to avoid truncation before clamping.
fn clamp_max_tool_rounds(value: u64) -> u32 {
    value.clamp(1, MAX_TOOL_ROUNDS_LIMIT as u64) as u32
}

impl RoutineAction {
    /// The string tag stored in the DB action_type column.
    pub fn type_tag(&self) -> &'static str {
        match self {
            RoutineAction::Lightweight { .. } => "lightweight",
            RoutineAction::FullJob { .. } => "full_job",
        }
    }

    /// Parse an action from its DB representation.
    pub fn from_db(action_type: &str, config: serde_json::Value) -> Result<Self, RoutineError> {
        match action_type {
            "lightweight" => {
                let prompt = config
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RoutineError::MissingField {
                        context: "lightweight action".into(),
                        field: "prompt".into(),
                    })?
                    .to_string();
                let context_paths = config
                    .get("context_paths")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let max_tokens = config
                    .get("max_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(default_max_tokens() as u64) as u32;
                let use_tools = config
                    .get("use_tools")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let max_tool_rounds = clamp_max_tool_rounds(
                    config
                        .get("max_tool_rounds")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(default_max_tool_rounds() as u64),
                );
                Ok(RoutineAction::Lightweight {
                    prompt,
                    context_paths,
                    max_tokens,
                    use_tools,
                    max_tool_rounds,
                })
            }
            "full_job" => {
                let title = config
                    .get("title")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RoutineError::MissingField {
                        context: "full_job action".into(),
                        field: "title".into(),
                    })?
                    .to_string();
                let description = config
                    .get("description")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RoutineError::MissingField {
                        context: "full_job action".into(),
                        field: "description".into(),
                    })?
                    .to_string();
                let max_iterations = config
                    .get("max_iterations")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(default_max_iterations() as u64)
                    as u32;
                Ok(RoutineAction::FullJob {
                    title,
                    description,
                    max_iterations,
                })
            }
            other => Err(RoutineError::UnknownActionType {
                action_type: other.to_string(),
            }),
        }
    }

    /// Serialize action config to JSON for DB storage.
    pub fn to_config_json(&self) -> serde_json::Value {
        match self {
            RoutineAction::Lightweight {
                prompt,
                context_paths,
                max_tokens,
                use_tools,
                max_tool_rounds,
            } => serde_json::json!({
                "prompt": prompt,
                "context_paths": context_paths,
                "max_tokens": max_tokens,
                "use_tools": use_tools,
                "max_tool_rounds": max_tool_rounds,
            }),
            RoutineAction::FullJob {
                title,
                description,
                max_iterations,
            } => serde_json::json!({
                "title": title,
                "description": description,
                "max_iterations": max_iterations,
            }),
        }
    }
}

/// Guardrails to prevent runaway execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineGuardrails {
    /// Minimum time between fires.
    pub cooldown: Duration,
    /// Max simultaneous runs of this routine.
    pub max_concurrent: u32,
    /// Window for content-hash dedup (event triggers). None = no dedup.
    pub dedup_window: Option<Duration>,
}

impl Default for RoutineGuardrails {
    fn default() -> Self {
        Self {
            cooldown: Duration::from_secs(300),
            max_concurrent: 1,
            dedup_window: None,
        }
    }
}

/// Notification preferences for a routine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyConfig {
    /// Channel to notify on (None = default/broadcast all).
    pub channel: Option<String>,
    /// Explicit target to notify. None means "resolve the owner's last-seen target".
    pub user: Option<String>,
    /// Notify when routine produces actionable output.
    pub on_attention: bool,
    /// Notify when routine errors.
    pub on_failure: bool,
    /// Notify when routine runs with no findings.
    pub on_success: bool,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            channel: None,
            user: None,
            on_attention: true,
            on_failure: true,
            on_success: false,
        }
    }
}

/// Status of a routine run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Ok,
    Attention,
    Failed,
}

impl std::fmt::Display for RunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunStatus::Running => write!(f, "running"),
            RunStatus::Ok => write!(f, "ok"),
            RunStatus::Attention => write!(f, "attention"),
            RunStatus::Failed => write!(f, "failed"),
        }
    }
}

impl FromStr for RunStatus {
    type Err = RoutineError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "running" => Ok(RunStatus::Running),
            "ok" => Ok(RunStatus::Ok),
            "attention" => Ok(RunStatus::Attention),
            "failed" => Ok(RunStatus::Failed),
            other => Err(RoutineError::UnknownRunStatus {
                status: other.to_string(),
            }),
        }
    }
}

/// A single execution of a routine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineRun {
    pub id: Uuid,
    pub routine_id: Uuid,
    pub trigger_type: String,
    pub trigger_detail: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub status: RunStatus,
    pub result_summary: Option<String>,
    pub tokens_used: Option<i32>,
    pub job_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// Convert a JSON value to a string for filter storage.
///
/// Handles strings, numbers, and booleans — consistent with the matching
/// logic in `routine_engine::json_value_as_string`.
pub fn json_value_as_filter_string(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Compute a content hash for event dedup.
pub fn content_hash(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

/// Normalize a cron expression to the 7-field format expected by the `cron` crate.
///
/// The `cron` crate requires: `sec min hour day-of-month month day-of-week year`.
/// Standard cron uses 5 fields: `min hour day-of-month month day-of-week`.
/// This function auto-expands:
/// - 5-field → prepend `0` (seconds) and append `*` (year)
/// - 6-field → append `*` (year)
/// - 7-field → pass through unchanged
pub fn normalize_cron_expression(schedule: &str) -> String {
    let trimmed = schedule.trim();
    let fields: Vec<&str> = trimmed.split_whitespace().collect();
    match fields.len() {
        5 => format!("0 {} *", trimmed),
        6 => format!("{} *", trimmed),
        _ => trimmed.to_string(),
    }
}

/// Parse a cron expression and compute the next fire time from now.
///
/// Accepts standard 5-field, 6-field, or 7-field cron expressions (auto-normalized).
/// When `timezone` is provided and valid, the schedule is evaluated in that
/// timezone and the result is converted back to UTC. Otherwise UTC is used.
pub fn next_cron_fire(
    schedule: &str,
    timezone: Option<&str>,
) -> Result<Option<DateTime<Utc>>, RoutineError> {
    let normalized = normalize_cron_expression(schedule);
    let cron_schedule =
        cron::Schedule::from_str(&normalized).map_err(|e| RoutineError::InvalidCron {
            reason: e.to_string(),
        })?;
    if let Some(tz) = timezone.and_then(crate::timezone::parse_timezone) {
        Ok(cron_schedule
            .upcoming(tz)
            .next()
            .map(|dt| dt.with_timezone(&Utc)))
    } else {
        Ok(cron_schedule.upcoming(Utc).next())
    }
}

/// Describe common routine cron patterns in plain English.
///
/// Falls back to `cron: <raw>` for malformed or complex expressions.
pub fn describe_cron(schedule: &str, timezone: Option<&str>) -> String {
    fn fallback(raw: &str) -> String {
        if raw.trim().is_empty() {
            "cron: (empty)".to_string()
        } else {
            format!("cron: {}", raw.trim())
        }
    }

    fn parse_u8_token(token: &str) -> Option<u8> {
        token.parse::<u8>().ok()
    }

    fn parse_step(token: &str) -> Option<u8> {
        token
            .strip_prefix("*/")
            .and_then(parse_u8_token)
            .filter(|n| *n > 0)
    }

    fn weekday_name(dow: &str) -> Option<&'static str> {
        let normalized = dow.trim().to_ascii_uppercase();
        match normalized.as_str() {
            "MON" | "1" => Some("Monday"),
            "TUE" | "2" => Some("Tuesday"),
            "WED" | "3" => Some("Wednesday"),
            "THU" | "4" => Some("Thursday"),
            "FRI" | "5" => Some("Friday"),
            "SAT" | "6" => Some("Saturday"),
            "SUN" | "0" | "7" => Some("Sunday"),
            _ => None,
        }
    }

    fn format_time(hour: u8, minute: u8) -> String {
        if hour == 0 && minute == 0 {
            return "midnight".to_string();
        }
        let (display_hour, am_pm) = match hour {
            0 => (12, "AM"),
            1..=11 => (hour, "AM"),
            12 => (12, "PM"),
            _ => (hour - 12, "PM"),
        };
        format!("{display_hour}:{minute:02} {am_pm}")
    }

    fn ordinal(n: u8) -> String {
        let suffix = if (11..=13).contains(&(n % 100)) {
            "th"
        } else {
            match n % 10 {
                1 => "st",
                2 => "nd",
                3 => "rd",
                _ => "th",
            }
        };
        format!("{n}{suffix}")
    }

    fn describe_inner(raw: &str) -> Option<String> {
        let fields: Vec<&str> = raw.split_whitespace().collect();
        let (sec, min, hour, dom, month, dow, year) = match fields.len() {
            5 => (
                "0", fields[0], fields[1], fields[2], fields[3], fields[4], None,
            ),
            6 => (
                fields[0], fields[1], fields[2], fields[3], fields[4], fields[5], None,
            ),
            7 => (
                fields[0],
                fields[1],
                fields[2],
                fields[3],
                fields[4],
                fields[5],
                Some(fields[6]),
            ),
            _ => return None,
        };

        if year.is_some_and(|v| v != "*") {
            return None;
        }

        if sec == "0"
            && hour == "*"
            && dom == "*"
            && month == "*"
            && dow == "*"
            && let Some(step) = parse_step(min)
        {
            return Some(match step {
                1 => "Every minute".to_string(),
                n => format!("Every {n} minutes"),
            });
        }

        if sec == "0"
            && min == "0"
            && dom == "*"
            && month == "*"
            && dow == "*"
            && let Some(step) = parse_step(hour)
        {
            return Some(match step {
                1 => "Every hour".to_string(),
                n => format!("Every {n} hours"),
            });
        }

        let hour = parse_u8_token(hour).filter(|h| *h <= 23)?;
        let minute = parse_u8_token(min).filter(|m| *m <= 59)?;
        let time = format_time(hour, minute);
        let time_phrase = if time == "midnight" {
            "at midnight".to_string()
        } else {
            format!("at {time}")
        };

        if sec == "0" && dom == "*" && month == "*" && dow == "*" {
            return Some(format!("Daily {time_phrase}"));
        }

        if sec == "0" && dom == "*" && month == "*" && dow.eq_ignore_ascii_case("MON-FRI") {
            return Some(format!("Weekdays {time_phrase}"));
        }

        if sec == "0"
            && dom == "*"
            && month == "*"
            && let Some(day_name) = weekday_name(dow)
        {
            return Some(format!("Every {day_name} {time_phrase}"));
        }

        if sec == "0"
            && month == "*"
            && dow == "*"
            && let Some(day_of_month) = parse_u8_token(dom).filter(|d| (1..=31).contains(d))
        {
            return Some(format!(
                "{} of every month {time_phrase}",
                ordinal(day_of_month)
            ));
        }

        None
    }

    let mut description = describe_inner(schedule).unwrap_or_else(|| fallback(schedule));
    if let Some(tz) = timezone.map(str::trim).filter(|tz| !tz.is_empty()) {
        description.push_str(" (");
        description.push_str(tz);
        description.push(')');
    }
    description
}

#[cfg(test)]
mod tests {
    use crate::agent::routine::{
        MAX_TOOL_ROUNDS_LIMIT, RoutineAction, RoutineGuardrails, RunStatus, Trigger, content_hash,
        describe_cron, next_cron_fire, normalize_cron_expression,
    };

    #[test]
    fn test_trigger_roundtrip() {
        let trigger = Trigger::Cron {
            schedule: "0 9 * * MON-FRI".to_string(),
            timezone: None,
        };
        let json = trigger.to_config_json();
        let parsed = Trigger::from_db("cron", json).expect("parse cron");
        assert!(matches!(parsed, Trigger::Cron { schedule, .. } if schedule == "0 9 * * MON-FRI"));
    }

    #[test]
    fn test_event_trigger_roundtrip() {
        let trigger = Trigger::Event {
            channel: Some("telegram".to_string()),
            pattern: r"deploy\s+\w+".to_string(),
        };
        let json = trigger.to_config_json();
        let parsed = Trigger::from_db("event", json).expect("parse event");
        assert!(matches!(parsed, Trigger::Event { channel, pattern }
            if channel == Some("telegram".to_string()) && pattern == r"deploy\s+\w+"));
    }

    #[test]
    fn test_system_event_trigger_roundtrip() {
        let mut filters = std::collections::HashMap::new();
        filters.insert("repo".to_string(), "nearai/ironclaw".to_string());
        filters.insert("action".to_string(), "opened".to_string());
        let trigger = Trigger::SystemEvent {
            source: "github".to_string(),
            event_type: "issue".to_string(),
            filters: filters.clone(),
        };
        let json = trigger.to_config_json();
        let parsed = Trigger::from_db("system_event", json).expect("parse system_event");
        assert!(
            matches!(parsed, Trigger::SystemEvent { source, event_type, filters: f }
            if source == "github" && event_type == "issue" && f == filters)
        );
    }

    #[test]
    fn test_action_lightweight_roundtrip() {
        let action = RoutineAction::Lightweight {
            prompt: "Check PRs".to_string(),
            context_paths: vec!["context/priorities.md".to_string()],
            max_tokens: 2048,
            use_tools: false,
            max_tool_rounds: 3,
        };
        let json = action.to_config_json();
        let parsed = RoutineAction::from_db("lightweight", json).expect("parse lightweight");
        assert!(
            matches!(parsed, RoutineAction::Lightweight { prompt, context_paths, max_tokens, .. }
            if prompt == "Check PRs" && context_paths.len() == 1 && max_tokens == 2048)
        );
    }

    #[test]
    fn test_action_full_job_roundtrip() {
        let action = RoutineAction::FullJob {
            title: "Deploy review".to_string(),
            description: "Review and deploy pending changes".to_string(),
            max_iterations: 5,
        };
        let json = action.to_config_json();
        let parsed = RoutineAction::from_db("full_job", json).expect("parse full_job");
        assert!(
            matches!(parsed, RoutineAction::FullJob { title, max_iterations, .. }
            if title == "Deploy review"
                && max_iterations == 5)
        );
    }

    #[test]
    fn test_action_full_job_ignores_legacy_permission_fields() {
        let parsed = RoutineAction::from_db(
            "full_job",
            serde_json::json!({
                "title": "Deploy review",
                "description": "Review and deploy pending changes",
                "max_iterations": 5,
                "tool_permissions": ["shell"],
                "permission_mode": "inherit_owner"
            }),
        )
        .expect("parse full_job");
        assert!(matches!(
            parsed,
            RoutineAction::FullJob {
                ref title,
                ref description,
                max_iterations,
                ..
            } if title == "Deploy review"
                && description == "Review and deploy pending changes"
                && max_iterations == 5
        ));
        assert_eq!(
            parsed.to_config_json(),
            serde_json::json!({
                "title": "Deploy review",
                "description": "Review and deploy pending changes",
                "max_iterations": 5,
            })
        );
    }

    #[test]
    fn test_run_status_display_parse() {
        for status in [
            RunStatus::Running,
            RunStatus::Ok,
            RunStatus::Attention,
            RunStatus::Failed,
        ] {
            let s = status.to_string();
            let parsed: RunStatus = s.parse().expect("parse status");
            assert_eq!(parsed, status);
        }
    }

    #[test]
    fn test_content_hash_deterministic() {
        let h1 = content_hash("deploy production");
        let h2 = content_hash("deploy production");
        assert_eq!(h1, h2);

        let h3 = content_hash("deploy staging");
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_next_cron_fire_valid() {
        // Every minute should always have a next fire
        let next = next_cron_fire("* * * * * *", None).expect("valid cron");
        assert!(next.is_some());
    }

    #[test]
    fn test_next_cron_fire_invalid() {
        let result = next_cron_fire("not a cron", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_trigger_cron_timezone_roundtrip() {
        let trigger = Trigger::Cron {
            schedule: "0 9 * * MON-FRI".to_string(),
            timezone: Some("America/New_York".to_string()),
        };
        let json = trigger.to_config_json();
        let parsed = Trigger::from_db("cron", json).expect("parse cron");
        assert!(matches!(parsed, Trigger::Cron { schedule, timezone }
                if schedule == "0 9 * * MON-FRI"
                && timezone.as_deref() == Some("America/New_York")));
    }

    #[test]
    fn test_trigger_cron_no_timezone_backward_compat() {
        let json = serde_json::json!({"schedule": "0 9 * * *"});
        let parsed = Trigger::from_db("cron", json).expect("parse cron");
        assert!(matches!(parsed, Trigger::Cron { timezone, .. } if timezone.is_none()));
    }

    #[test]
    fn test_trigger_cron_invalid_timezone_coerced_to_none() {
        let json = serde_json::json!({"schedule": "0 9 * * *", "timezone": "Fake/Zone"});
        let parsed = Trigger::from_db("cron", json).expect("parse cron");
        assert!(
            matches!(parsed, Trigger::Cron { timezone, .. } if timezone.is_none()),
            "invalid timezone should be coerced to None"
        );
    }

    #[test]
    fn test_next_cron_fire_with_timezone() {
        let next_utc = next_cron_fire("0 0 9 * * * *", None)
            .expect("valid cron")
            .expect("has next");
        let next_est = next_cron_fire("0 0 9 * * * *", Some("America/New_York"))
            .expect("valid cron")
            .expect("has next");
        // EST is UTC-5 (or EDT UTC-4), so the UTC result should differ
        assert_ne!(next_utc, next_est, "timezone should shift the fire time");
    }

    #[test]
    fn test_describe_cron_common_patterns() {
        let cases = vec![
            ("0 */30 * * * *", None, "Every 30 minutes"),
            ("0 0 9 * * *", None, "Daily at 9:00 AM"),
            ("0 0 9 * * MON-FRI", None, "Weekdays at 9:00 AM"),
            ("0 0 */2 * * *", None, "Every 2 hours"),
            ("0 0 0 * * *", None, "Daily at midnight"),
            ("0 0 9 * * 1", None, "Every Monday at 9:00 AM"),
            ("0 0 9 1 * *", None, "1st of every month at 9:00 AM"),
            (
                "0 0 9 * * MON-FRI",
                Some("America/New_York"),
                "Weekdays at 9:00 AM (America/New_York)",
            ),
            ("1 2 3 4 5 6", None, "cron: 1 2 3 4 5 6"),
        ];

        for (schedule, timezone, expected) in cases {
            let actual = describe_cron(schedule, timezone);
            assert_eq!(actual, expected); // safety: test-only assertion in #[cfg(test)] module
        }
    }

    #[test]
    fn test_describe_cron_edge_cases() {
        assert_eq!(describe_cron("", None), "cron: (empty)"); // safety: test-only assertion in #[cfg(test)] module
        assert_eq!(describe_cron("not a cron", None), "cron: not a cron"); // safety: test-only assertion in #[cfg(test)] module
        let weekdays_5_field = describe_cron("0 9 * * MON-FRI", None);
        assert_eq!(weekdays_5_field, "Weekdays at 9:00 AM"); // safety: test-only assertion in #[cfg(test)] module
        let weekdays_7_field = describe_cron("0 0 9 * * MON-FRI *", None);
        assert_eq!(weekdays_7_field, "Weekdays at 9:00 AM"); // safety: test-only assertion in #[cfg(test)] module
    }

    #[test]
    fn test_guardrails_default() {
        let g = RoutineGuardrails::default();
        assert_eq!(g.cooldown.as_secs(), 300);
        assert_eq!(g.max_concurrent, 1);
        assert!(g.dedup_window.is_none());
    }

    #[test]
    fn test_trigger_type_tag() {
        assert_eq!(
            Trigger::Cron {
                schedule: String::new(),
                timezone: None,
            }
            .type_tag(),
            "cron"
        );
        assert_eq!(
            Trigger::Event {
                channel: None,
                pattern: String::new()
            }
            .type_tag(),
            "event"
        );
        assert_eq!(
            Trigger::SystemEvent {
                source: String::new(),
                event_type: String::new(),
                filters: std::collections::HashMap::new(),
            }
            .type_tag(),
            "system_event"
        );
        assert_eq!(Trigger::Manual.type_tag(), "manual");
    }

    #[test]
    fn test_normalize_cron_5_field() {
        // Standard cron: min hour dom month dow
        assert_eq!(normalize_cron_expression("0 9 * * 1"), "0 0 9 * * 1 *");
        assert_eq!(
            normalize_cron_expression("0 9 * * MON-FRI"),
            "0 0 9 * * MON-FRI *"
        );
    }

    #[test]
    fn test_normalize_cron_6_field() {
        // 6-field: sec min hour dom month dow
        assert_eq!(
            normalize_cron_expression("0 0 9 * * MON-FRI"),
            "0 0 9 * * MON-FRI *"
        );
    }

    #[test]
    fn test_normalize_cron_7_field_passthrough() {
        // Already 7-field: no change
        assert_eq!(
            normalize_cron_expression("0 0 9 * * MON-FRI *"),
            "0 0 9 * * MON-FRI *"
        );
    }

    #[test]
    fn test_next_cron_fire_5_field_accepted() {
        // Standard 5-field cron should now work through normalization
        let result = next_cron_fire("0 9 * * 1", None);
        assert!(
            result.is_ok(),
            "5-field cron should be accepted: {result:?}"
        );
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_next_cron_fire_5_field_with_timezone() {
        let result = next_cron_fire("0 9 * * MON-FRI", Some("America/New_York"));
        assert!(
            result.is_ok(),
            "5-field cron with timezone should be accepted: {result:?}"
        );
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_action_lightweight_backward_compat_no_use_tools() {
        // Simulate old DB record without use_tools field
        let json = serde_json::json!({
            "prompt": "old routine",
            "context_paths": [],
            "max_tokens": 4096
        });
        let parsed = RoutineAction::from_db("lightweight", json).expect("parse lightweight");
        assert!(
            matches!(parsed, RoutineAction::Lightweight { use_tools, max_tool_rounds, .. }
            if !use_tools && max_tool_rounds == 3),
            "missing use_tools should default to false, max_tool_rounds to 3"
        );
    }

    #[test]
    fn test_max_tool_rounds_clamped_to_upper_bound() {
        let json = serde_json::json!({
            "prompt": "test",
            "use_tools": true,
            "max_tool_rounds": 9999
        });
        let parsed = RoutineAction::from_db("lightweight", json).expect("parse");
        match parsed {
            RoutineAction::Lightweight {
                max_tool_rounds, ..
            } => {
                assert_eq!(
                    max_tool_rounds, MAX_TOOL_ROUNDS_LIMIT,
                    "should clamp to MAX_TOOL_ROUNDS_LIMIT"
                );
            }
            _ => panic!("expected Lightweight"),
        }
    }

    #[test]
    fn test_max_tool_rounds_clamped_to_lower_bound() {
        let json = serde_json::json!({
            "prompt": "test",
            "use_tools": true,
            "max_tool_rounds": 0
        });
        let parsed = RoutineAction::from_db("lightweight", json).expect("parse");
        match parsed {
            RoutineAction::Lightweight {
                max_tool_rounds, ..
            } => {
                assert_eq!(max_tool_rounds, 1, "should clamp 0 to 1");
            }
            _ => panic!("expected Lightweight"),
        }
    }

    #[test]
    fn test_max_tool_rounds_normal_value_passes_through() {
        let json = serde_json::json!({
            "prompt": "test",
            "use_tools": true,
            "max_tool_rounds": 10
        });
        let parsed = RoutineAction::from_db("lightweight", json).expect("parse");
        match parsed {
            RoutineAction::Lightweight {
                max_tool_rounds, ..
            } => {
                assert_eq!(max_tool_rounds, 10, "normal value should pass through");
            }
            _ => panic!("expected Lightweight"),
        }
    }
}
