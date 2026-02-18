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

use tokio::sync::mpsc;

use crate::channels::OutgoingResponse;
use crate::llm::{ChatMessage, CompletionRequest, FinishReason, LlmProvider};
use crate::workspace::hygiene::HygieneConfig;
use crate::workspace::Workspace;

/// Configuration for the heartbeat runner.
#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    /// Interval between heartbeat checks.
    pub interval: Duration,
    /// Whether heartbeat is enabled.
    pub enabled: bool,
    /// Maximum consecutive failures before disabling.
    pub max_failures: u32,
    /// User ID to notify on heartbeat findings.
    pub notify_user_id: Option<String>,
    /// Channel to notify on heartbeat findings.
    pub notify_channel: Option<String>,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(30 * 60), // 30 minutes
            enabled: true,
            max_failures: 3,
            notify_user_id: None,
            notify_channel: None,
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

    /// Set the notification target.
    pub fn with_notify(mut self, user_id: impl Into<String>, channel: impl Into<String>) -> Self {
        self.notify_user_id = Some(user_id.into());
        self.notify_channel = Some(channel.into());
        self
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

/// Heartbeat runner for proactive periodic execution.
pub struct HeartbeatRunner {
    config: HeartbeatConfig,
    hygiene_config: HygieneConfig,
    workspace: Arc<Workspace>,
    llm: Arc<dyn LlmProvider>,
    response_tx: Option<mpsc::Sender<OutgoingResponse>>,
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
            consecutive_failures: 0,
        }
    }

    /// Set the response channel for notifications.
    pub fn with_response_channel(mut self, tx: mpsc::Sender<OutgoingResponse>) -> Self {
        self.response_tx = Some(tx);
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

        tracing::info!(
            "Starting heartbeat loop with interval {:?}",
            self.config.interval
        );

        let mut interval = tokio::time::interval(self.config.interval);
        // Don't run immediately on startup
        interval.tick().await;

        loop {
            interval.tick().await;

            // Run memory hygiene (best-effort; never blocks the heartbeat loop).
            let report = crate::workspace::hygiene::run_if_due(
                &self.workspace,
                &self.hygiene_config,
            )
            .await;
            if report.had_work() {
                tracing::info!(
                    daily_logs_deleted = report.daily_logs_deleted,
                    "heartbeat: memory hygiene deleted stale documents"
                );
            }

            match self.check_heartbeat().await {
                HeartbeatResult::Ok => {
                    tracing::debug!("Heartbeat OK");
                    self.consecutive_failures = 0;
                }
                HeartbeatResult::NeedsAttention(message) => {
                    tracing::info!("Heartbeat needs attention: {}", message);
                    self.consecutive_failures = 0;
                    self.send_notification(&message).await;
                }
                HeartbeatResult::Skipped => {
                    tracing::debug!("Heartbeat skipped");
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

        let response = match self.llm.complete(request).await {
            Ok(r) => r,
            Err(e) => return HeartbeatResult::Failed(format!("LLM call failed: {}", e)),
        };

        let content = response.content.trim();

        // Guard against empty content. Reasoning models (e.g. GLM-4.7) may
        // burn all output tokens on chain-of-thought and return content: null.
        if content.is_empty() {
            return if response.finish_reason == FinishReason::Length {
                HeartbeatResult::Failed(
                    "LLM response was truncated (finish_reason=length) with no content. \
                     The model may have exhausted its token budget on reasoning."
                        .to_string(),
                )
            } else {
                HeartbeatResult::Failed("LLM returned empty content.".to_string())
            };
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

        let response = OutgoingResponse {
            content: format!("🔔 *Heartbeat Alert*\n\n{}", message),
            thread_id: None,
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
) -> tokio::task::JoinHandle<()> {
    let mut runner = HeartbeatRunner::new(config, hygiene_config, workspace, llm);
    if let Some(tx) = response_tx {
        runner = runner.with_response_channel(tx);
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
}
