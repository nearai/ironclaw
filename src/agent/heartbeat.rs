//! Proactive heartbeat system for periodic execution.
//!
//! The heartbeat runner executes periodically (default: every 30 minutes) and:
//! 1. Reads the HEARTBEAT.md checklist
//! 2. Runs an agent turn to process the checklist
//! 3. Reports any findings to the configured channel
//!
//! If nothing needs attention, the agent replies "HEARTBEAT_OK" and no
//! message is sent to the user.
//!
//! # Usage
//!
//! Create a HEARTBEAT.md in the workspace with a checklist of things to monitor:
//!
//! ```markdown
//! # Heartbeat Checklist
//!
//! - [ ] Check for unread emails
//! - [ ] Review calendar for upcoming events
//! - [ ] Check project build status
//! ```
//!
//! The agent will process this checklist on each heartbeat and only notify
//! if action is needed.

use std::sync::Arc;
use std::time::Duration;

use chrono::TimeZone as _;
use chrono_tz::Tz;
use tokio::sync::mpsc;

use crate::channels::OutgoingResponse;
use crate::db::Database;
use crate::llm::{ChatMessage, CompletionRequest, LlmProvider, Reasoning};
use crate::workspace::Workspace;
use crate::workspace::hygiene::HygieneConfig;

/// Configuration for the heartbeat runner.
#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    /// Interval between heartbeat checks (used when fire_at is not set).
    pub interval: Duration,
    /// Whether heartbeat is enabled.
    pub enabled: bool,
    /// Maximum consecutive failures before disabling.
    pub max_failures: u32,
    /// User ID to notify on heartbeat findings.
    pub notify_user_id: Option<String>,
    /// Channel to notify on heartbeat findings.
    pub notify_channel: Option<String>,
    /// Fixed time-of-day to fire (24h). When set, interval is ignored.
    pub fire_at: Option<chrono::NaiveTime>,
    /// Hour (0-23) when quiet hours start.
    pub quiet_hours_start: Option<u32>,
    /// Hour (0-23) when quiet hours end.
    pub quiet_hours_end: Option<u32>,
    /// Timezone for fire_at and quiet hours evaluation (IANA name).
    pub timezone: Option<String>,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(30 * 60), // 30 minutes
            enabled: true,
            max_failures: 3,
            notify_user_id: None,
            notify_channel: None,
            fire_at: None,
            quiet_hours_start: None,
            quiet_hours_end: None,
            timezone: None,
        }
    }
}

impl HeartbeatConfig {
    /// Create a config with a specific interval.
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Disable heartbeat.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Check whether the current time falls within configured quiet hours.
    pub fn is_quiet_hours(&self) -> bool {
        use chrono::Timelike;
        let (Some(start), Some(end)) = (self.quiet_hours_start, self.quiet_hours_end) else {
            return false;
        };
        let tz = self
            .timezone
            .as_deref()
            .and_then(crate::timezone::parse_timezone)
            .unwrap_or(chrono_tz::UTC);
        let now_hour = crate::timezone::now_in_tz(tz).hour();
        if start <= end {
            now_hour >= start && now_hour < end
        } else {
            // Wraps midnight, e.g. 22..06
            now_hour >= start || now_hour < end
        }
    }

    /// Set the notification target.
    pub fn with_notify(mut self, user_id: impl Into<String>, channel: impl Into<String>) -> Self {
        self.notify_user_id = Some(user_id.into());
        self.notify_channel = Some(channel.into());
        self
    }

    /// Set a fixed time-of-day to fire (overrides interval).
    pub fn with_fire_at(mut self, time: chrono::NaiveTime, tz: Option<String>) -> Self {
        self.fire_at = Some(time);
        self.timezone = tz;
        self
    }

    /// Resolve timezone string to chrono_tz::Tz (defaults to UTC).
    fn resolved_tz(&self) -> Tz {
        self.timezone
            .as_deref()
            .and_then(crate::timezone::parse_timezone)
            .unwrap_or(chrono_tz::UTC)
    }
}

/// Result of a heartbeat check.
#[derive(Debug)]
pub enum HeartbeatResult {
    /// Nothing needs attention.
    Ok,
    /// Something needs attention, with the message to send.
    NeedsAttention(String),
    /// Heartbeat was skipped (no checklist or disabled).
    Skipped,
    /// Heartbeat failed.
    Failed(String),
}

/// Compute how long to sleep until the next occurrence of `fire_at` in `tz`.
///
/// If the target time today is still in the future, sleep until then.
/// Otherwise sleep until the same time tomorrow.
fn duration_until_next_fire(fire_at: chrono::NaiveTime, tz: Tz) -> Duration {
    let now = chrono::Utc::now().with_timezone(&tz);
    let today = now.date_naive();

    // Try to build today's target datetime in the given timezone.
    // `.earliest()` picks the first occurrence if DST creates ambiguity.
    let candidate = tz.from_local_datetime(&today.and_time(fire_at)).earliest();

    let target = match candidate {
        Some(t) if t > now => t,
        _ => {
            // Already past (or ambiguous) — schedule for tomorrow
            let tomorrow = today + chrono::Duration::days(1);
            tz.from_local_datetime(&tomorrow.and_time(fire_at))
                .earliest()
                .unwrap_or_else(|| now + chrono::Duration::days(1))
        }
    };

    let secs = (target - now).num_seconds().max(1) as u64;
    Duration::from_secs(secs)
}

/// Heartbeat runner for proactive periodic execution.
pub struct HeartbeatRunner {
    config: HeartbeatConfig,
    hygiene_config: HygieneConfig,
    workspace: Arc<Workspace>,
    llm: Arc<dyn LlmProvider>,
    response_tx: Option<mpsc::Sender<OutgoingResponse>>,
    store: Option<Arc<dyn Database>>,
    consecutive_failures: u32,
}

impl HeartbeatRunner {
    /// Create a new heartbeat runner.
    pub fn new(
        config: HeartbeatConfig,
        hygiene_config: HygieneConfig,
        workspace: Arc<Workspace>,
        llm: Arc<dyn LlmProvider>,
    ) -> Self {
        Self {
            config,
            hygiene_config,
            workspace,
            llm,
            response_tx: None,
            store: None,
            consecutive_failures: 0,
        }
    }

    /// Set the response channel for notifications.
    pub fn with_response_channel(mut self, tx: mpsc::Sender<OutgoingResponse>) -> Self {
        self.response_tx = Some(tx);
        self
    }

    /// Set the database store for persistent heartbeat conversations.
    pub fn with_store(mut self, store: Arc<dyn Database>) -> Self {
        self.store = Some(store);
        self
    }

    /// Run the heartbeat loop.
    ///
    /// This runs forever, checking periodically based on the configured interval.
    pub async fn run(&mut self) {
        if !self.config.enabled {
            tracing::info!("Heartbeat is disabled, not starting loop");
            return;
        }

        if let Some(fire_at) = self.config.fire_at {
            tracing::info!(
                "Starting heartbeat loop: fire daily at {:?} {:?}",
                fire_at,
                self.config.timezone
            );
        } else {
            tracing::info!(
                "Starting heartbeat loop with interval {:?}",
                self.config.interval
            );
        }

        loop {
            // Sleep until the next fire time
            let sleep_dur = if let Some(fire_at) = self.config.fire_at {
                duration_until_next_fire(fire_at, self.config.resolved_tz())
            } else {
                self.config.interval
            };
            tracing::info!("Next heartbeat in {:.1}h", sleep_dur.as_secs_f64() / 3600.0);
            tokio::time::sleep(sleep_dur).await;

            // Skip during quiet hours
            if self.config.is_quiet_hours() {
                tracing::trace!("Heartbeat skipped: quiet hours");
                continue;
            }

            // Run memory hygiene in the background so it never delays the
            // heartbeat checklist. Failures are logged inside run_if_due.
            let hygiene_workspace = Arc::clone(&self.workspace);
            let hygiene_config = self.hygiene_config.clone();
            tokio::spawn(async move {
                let report =
                    crate::workspace::hygiene::run_if_due(&hygiene_workspace, &hygiene_config)
                        .await;
                if report.had_work() {
                    tracing::info!(
                        daily_logs_deleted = report.daily_logs_deleted,
                        conversation_docs_deleted = report.conversation_docs_deleted,
                        "heartbeat: memory hygiene deleted stale documents"
                    );
                }
            });

            match self.check_heartbeat().await {
                HeartbeatResult::Ok => {
                    tracing::trace!("Heartbeat OK");
                    self.consecutive_failures = 0;
                }
                HeartbeatResult::NeedsAttention(message) => {
                    tracing::info!("Heartbeat needs attention: {}", message);
                    self.consecutive_failures = 0;
                    self.send_notification(&message).await;
                }
                HeartbeatResult::Skipped => {
                    tracing::trace!("Heartbeat skipped");
                }
                HeartbeatResult::Failed(error) => {
                    tracing::error!("Heartbeat failed: {}", error);
                    self.consecutive_failures += 1;

                    if self.consecutive_failures >= self.config.max_failures {
                        tracing::error!(
                            "Heartbeat disabled after {} consecutive failures",
                            self.consecutive_failures
                        );
                        break;
                    }
                }
            }
        }
    }

    /// Run a single heartbeat check.
    pub async fn check_heartbeat(&self) -> HeartbeatResult {
        // Get the heartbeat checklist
        let checklist = match self.workspace.heartbeat_checklist().await {
            Ok(Some(content)) if !is_effectively_empty(&content) => content,
            Ok(_) => return HeartbeatResult::Skipped,
            Err(e) => return HeartbeatResult::Failed(format!("Failed to read checklist: {}", e)),
        };

        // Build the heartbeat prompt
        let prompt = format!(
            "Read the HEARTBEAT.md checklist below and follow it strictly. \
             Do not infer or repeat old tasks. Check each item and report findings.\n\
             \n\
             If nothing needs attention, reply EXACTLY with: HEARTBEAT_OK\n\
             \n\
             If something needs attention, provide a concise summary of what needs action.\n\
             \n\
             ## HEARTBEAT.md\n\
             \n\
             {}",
            checklist
        );

        // Get the system prompt for context
        let system_prompt = match self.workspace.system_prompt().await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Failed to get system prompt for heartbeat: {}", e);
                String::new()
            }
        };

        // Run the agent turn
        let messages = if system_prompt.is_empty() {
            vec![ChatMessage::user(&prompt)]
        } else {
            vec![
                ChatMessage::system(&system_prompt),
                ChatMessage::user(&prompt),
            ]
        };

        // Use the model's context_length to set max_tokens. The API returns
        // the total context window; we cap output at half of that (the rest is
        // the prompt) with a floor of 4096.
        let max_tokens = match self.llm.model_metadata().await {
            Ok(meta) => {
                let from_api = meta.context_length.map(|ctx| ctx / 2).unwrap_or(4096);
                from_api.max(4096)
            }
            Err(e) => {
                tracing::warn!(
                    "Could not fetch model metadata, using default max_tokens: {}",
                    e
                );
                4096
            }
        };

        let request = CompletionRequest::new(messages)
            .with_max_tokens(max_tokens)
            .with_temperature(0.3);

        let reasoning =
            Reasoning::new(self.llm.clone()).with_model_name(self.llm.active_model_name());
        let (content, _usage) = match reasoning.complete(request).await {
            Ok(r) => r,
            Err(e) => return HeartbeatResult::Failed(format!("LLM call failed: {}", e)),
        };

        let content = content.trim();

        // Guard against empty content. Reasoning models (e.g. GLM-4.7) may
        // burn all output tokens on chain-of-thought and return content: null.
        if content.is_empty() {
            return HeartbeatResult::Failed("LLM returned empty content.".to_string());
        }

        // Check if nothing needs attention
        if content == "HEARTBEAT_OK" || content.contains("HEARTBEAT_OK") {
            return HeartbeatResult::Ok;
        }

        HeartbeatResult::NeedsAttention(content.to_string())
    }

    /// Send a notification about heartbeat findings.
    async fn send_notification(&self, message: &str) {
        let Some(ref tx) = self.response_tx else {
            tracing::debug!("No response channel configured for heartbeat notifications");
            return;
        };

        let user_id = self.config.notify_user_id.as_deref().unwrap_or("default");

        // Persist to heartbeat conversation and get thread_id
        let thread_id = if let Some(ref store) = self.store {
            match store.get_or_create_heartbeat_conversation(user_id).await {
                Ok(conv_id) => {
                    if let Err(e) = store
                        .add_conversation_message(conv_id, "assistant", message)
                        .await
                    {
                        tracing::error!("Failed to persist heartbeat message: {}", e);
                    }
                    Some(conv_id.to_string())
                }
                Err(e) => {
                    tracing::error!("Failed to get heartbeat conversation: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let response = OutgoingResponse {
            content: format!("🔔 *Heartbeat Alert*\n\n{}", message),
            thread_id,
            attachments: Vec::new(),
            metadata: serde_json::json!({
                "source": "heartbeat",
            }),
        };

        if let Err(e) = tx.send(response).await {
            tracing::error!("Failed to send heartbeat notification: {}", e);
        }
    }
}

/// Check if heartbeat content is effectively empty.
///
/// Returns true if the content contains only:
/// - Whitespace
/// - Markdown headers (lines starting with #)
/// - HTML comments (`<!-- ... -->`)
/// - Empty list items (`- [ ]`, `- [x]`, `-`, `*`)
///
/// This skips the LLM call when the user hasn't added real tasks yet,
/// saving API costs.
fn is_effectively_empty(content: &str) -> bool {
    let without_comments = strip_html_comments(content);

    without_comments.lines().all(|line| {
        let trimmed = line.trim();
        trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed == "- [ ]"
            || trimmed == "- [x]"
            || trimmed == "-"
            || trimmed == "*"
    })
}

/// Remove HTML comments from content.
fn strip_html_comments(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find("<!--") {
        result.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => return result, // unclosed comment, treat rest as comment
        }
    }
    result.push_str(rest);
    result
}

/// Spawn the heartbeat runner as a background task.
///
/// Returns a handle that can be used to stop the runner.
pub fn spawn_heartbeat(
    config: HeartbeatConfig,
    hygiene_config: HygieneConfig,
    workspace: Arc<Workspace>,
    llm: Arc<dyn LlmProvider>,
    response_tx: Option<mpsc::Sender<OutgoingResponse>>,
    store: Option<Arc<dyn Database>>,
) -> tokio::task::JoinHandle<()> {
    let mut runner = HeartbeatRunner::new(config, hygiene_config, workspace, llm);
    if let Some(tx) = response_tx {
        runner = runner.with_response_channel(tx);
    }
    if let Some(s) = store {
        runner = runner.with_store(s);
    }

    tokio::spawn(async move {
        runner.run().await;
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heartbeat_config_defaults() {
        let config = HeartbeatConfig::default();
        assert!(config.enabled);
        assert_eq!(config.interval, Duration::from_secs(30 * 60));
        assert_eq!(config.max_failures, 3);
    }

    #[test]
    fn test_heartbeat_config_builders() {
        let config = HeartbeatConfig::default()
            .with_interval(Duration::from_secs(60))
            .with_notify("user1", "telegram");

        assert_eq!(config.interval, Duration::from_secs(60));
        assert_eq!(config.notify_user_id, Some("user1".to_string()));
        assert_eq!(config.notify_channel, Some("telegram".to_string()));

        let disabled = HeartbeatConfig::default().disabled();
        assert!(!disabled.enabled);
    }

    // ==================== strip_html_comments ====================

    #[test]
    fn test_strip_html_comments_no_comments() {
        assert_eq!(strip_html_comments("hello world"), "hello world");
    }

    #[test]
    fn test_strip_html_comments_single() {
        assert_eq!(
            strip_html_comments("before<!-- gone -->after"),
            "beforeafter"
        );
    }

    #[test]
    fn test_strip_html_comments_multiple() {
        let input = "a<!-- 1 -->b<!-- 2 -->c";
        assert_eq!(strip_html_comments(input), "abc");
    }

    #[test]
    fn test_strip_html_comments_multiline() {
        let input = "# Title\n<!-- multi\nline\ncomment -->\nreal content";
        assert_eq!(strip_html_comments(input), "# Title\n\nreal content");
    }

    #[test]
    fn test_strip_html_comments_unclosed() {
        let input = "before<!-- never closed";
        assert_eq!(strip_html_comments(input), "before");
    }

    // ==================== is_effectively_empty ====================

    #[test]
    fn test_effectively_empty_empty_string() {
        assert!(is_effectively_empty(""));
    }

    #[test]
    fn test_effectively_empty_whitespace() {
        assert!(is_effectively_empty("   \n\n  \n  "));
    }

    #[test]
    fn test_effectively_empty_headers_only() {
        assert!(is_effectively_empty("# Title\n## Subtitle\n### Section"));
    }

    #[test]
    fn test_effectively_empty_html_comments_only() {
        assert!(is_effectively_empty("<!-- this is a comment -->"));
    }

    #[test]
    fn test_effectively_empty_empty_checkboxes() {
        assert!(is_effectively_empty("# Checklist\n- [ ]\n- [x]"));
    }

    #[test]
    fn test_effectively_empty_bare_list_markers() {
        assert!(is_effectively_empty("-\n*\n-"));
    }

    #[test]
    fn test_effectively_empty_seeded_template() {
        let template = "\
# Heartbeat Checklist

<!-- Keep this file empty to skip heartbeat API calls.
     Add tasks below when you want the agent to check something periodically.

     Example:
     - [ ] Check for unread emails needing a reply
     - [ ] Review today's calendar for upcoming meetings
     - [ ] Check CI build status for main branch
-->";
        assert!(is_effectively_empty(template));
    }

    #[test]
    fn test_effectively_empty_real_checklist() {
        let content = "\
# Heartbeat Checklist

- [ ] Check for unread emails needing a reply
- [ ] Review today's calendar for upcoming meetings";
        assert!(!is_effectively_empty(content));
    }

    #[test]
    fn test_effectively_empty_mixed_real_and_headers() {
        let content = "# Title\n\nDo something important";
        assert!(!is_effectively_empty(content));
    }

    #[test]
    fn test_effectively_empty_comment_plus_real_content() {
        let content = "<!-- comment -->\nActual task here";
        assert!(!is_effectively_empty(content));
    }

    // ==================== quiet hours ====================

    #[test]
    fn test_quiet_hours_inside() {
        use chrono::{Timelike, Utc};

        let now_utc = Utc::now();
        let hour = now_utc.hour();
        let start = hour;
        let end = (hour + 1) % 24;

        let config = HeartbeatConfig {
            quiet_hours_start: Some(start),
            quiet_hours_end: Some(end),
            timezone: Some("UTC".to_string()),
            ..HeartbeatConfig::default()
        };
        // Current UTC hour is inside [start, end) by construction
        assert!(config.is_quiet_hours());
    }

    #[test]
    fn test_quiet_hours_outside() {
        use chrono::{Timelike, Utc};

        let now_utc = Utc::now();
        let hour = now_utc.hour();
        let start = (hour + 1) % 24;
        let end = (hour + 2) % 24;

        let config = HeartbeatConfig {
            quiet_hours_start: Some(start),
            quiet_hours_end: Some(end),
            timezone: Some("UTC".to_string()),
            ..HeartbeatConfig::default()
        };
        // Current UTC hour is outside [start, end) by construction
        assert!(!config.is_quiet_hours());
    }

    #[test]
    fn test_quiet_hours_wraparound_excludes_now() {
        use chrono::{Timelike, Utc};

        let now_utc = Utc::now();
        let hour = now_utc.hour();
        // Window covers all hours except the current one
        let start = (hour + 1) % 24;
        let end = hour;

        let config = HeartbeatConfig {
            quiet_hours_start: Some(start),
            quiet_hours_end: Some(end),
            timezone: Some("UTC".to_string()),
            ..HeartbeatConfig::default()
        };
        assert!(!config.is_quiet_hours());
    }

    #[test]
    fn test_quiet_hours_none_configured() {
        let config = HeartbeatConfig::default();
        assert!(!config.is_quiet_hours());
    }

    #[test]
    fn test_quiet_hours_same_start_end() {
        let config = HeartbeatConfig {
            quiet_hours_start: Some(10),
            quiet_hours_end: Some(10),
            timezone: Some("UTC".to_string()),
            ..HeartbeatConfig::default()
        };
        // start == end means zero-width window, should be false
        assert!(!config.is_quiet_hours());
    }

    #[test]
    fn test_spawn_heartbeat_accepts_store_param() {
        // Regression: spawn_heartbeat must accept an optional Database store
        // for persisting heartbeat notifications to a dedicated conversation.
        // Compile-time check: the 7th parameter is `Option<Arc<dyn Database>>`.
        #[allow(clippy::type_complexity)]
        let _fn_ptr: fn(
            HeartbeatConfig,
            HygieneConfig,
            Arc<crate::workspace::Workspace>,
            Arc<dyn crate::llm::LlmProvider>,
            Option<tokio::sync::mpsc::Sender<crate::channels::OutgoingResponse>>,
            Option<Arc<dyn crate::db::Database>>,
        ) -> tokio::task::JoinHandle<()> = spawn_heartbeat;
        let _ = _fn_ptr;
    }

    // ==================== fire_at scheduling ====================

    #[test]
    fn test_default_config_has_no_fire_at() {
        let config = HeartbeatConfig::default();
        assert!(config.fire_at.is_none());
        // Interval-based scheduling should be the default
        assert_eq!(config.interval, Duration::from_secs(30 * 60));
    }

    #[test]
    fn test_with_fire_at_builder() {
        let time = chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap();
        let config =
            HeartbeatConfig::default().with_fire_at(time, Some("Pacific/Auckland".to_string()));
        assert_eq!(config.fire_at, Some(time));
        assert_eq!(config.timezone, Some("Pacific/Auckland".to_string()));
    }

    #[test]
    fn test_duration_until_next_fire_is_bounded() {
        // Result must always be between 1 second and ~24 hours
        let time = chrono::NaiveTime::from_hms_opt(14, 0, 0).unwrap();
        let dur = duration_until_next_fire(time, chrono_tz::UTC);
        assert!(dur.as_secs() >= 1, "duration must be at least 1 second");
        assert!(
            dur.as_secs() <= 86_401,
            "duration must be at most ~24 hours, got {}s",
            dur.as_secs()
        );
    }

    #[test]
    fn test_duration_until_next_fire_dst_timezone_no_panic() {
        // Use a timezone with DST (US Eastern) — should never panic
        let tz: Tz = "America/New_York".parse().unwrap();
        // Test a range of times including midnight boundaries
        for hour in [0, 2, 3, 12, 23] {
            let time = chrono::NaiveTime::from_hms_opt(hour, 30, 0).unwrap();
            let dur = duration_until_next_fire(time, tz);
            assert!(dur.as_secs() >= 1);
            assert!(dur.as_secs() <= 86_401);
        }
    }

    #[test]
    fn test_resolved_tz_defaults_to_utc() {
        let config = HeartbeatConfig::default();
        assert_eq!(config.resolved_tz(), chrono_tz::UTC);
    }

    #[test]
    fn test_resolved_tz_parses_iana() {
        let time = chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap();
        let config =
            HeartbeatConfig::default().with_fire_at(time, Some("Europe/London".to_string()));
        assert_eq!(config.resolved_tz(), chrono_tz::Europe::London);
    }
}
