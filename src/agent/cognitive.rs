//! Cognitive routines for agent behavior.
//!
//! Implements pre-game routines, checkpointing, and after-action reviews
//! to improve agent reliability and memory.
//!
//! # Pre-Game Routine
//!
//! Before executing any non-trivial task, the agent should:
//! 1. Restate the task in one sentence
//! 2. List constraints and success criteria
//! 3. Retrieve only minimum relevant memory
//! 4. Prefer tools over guessing when facts matter
//! 5. Identify mode (Preparation vs Execution)
//!
//! # Checkpointing
//!
//! During long conversations, periodically write state to daily notes:
//! - Every N exchanges (configurable, default 15)
//! - Before/after complex multi-step work
//! - When significant decisions are made
//!
//! # After-Action Review
//!
//! After completing major tasks, write a brief review:
//! - What happened (2-5 bullets)
//! - Tools used
//! - What would be done differently

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::workspace::Workspace;

/// Configuration for cognitive routines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitiveConfig {
    /// Enable pre-game routine injection into prompts.
    #[serde(default = "default_true")]
    pub pre_game_enabled: bool,

    /// Enable automatic checkpointing.
    #[serde(default = "default_true")]
    pub checkpointing_enabled: bool,

    /// Number of exchanges between checkpoints.
    #[serde(default = "default_checkpoint_interval")]
    pub checkpoint_interval: u32,

    /// Enable after-action review prompts.
    #[serde(default)]
    pub after_action_enabled: bool,
}

fn default_true() -> bool {
    true
}

fn default_checkpoint_interval() -> u32 {
    15
}

impl Default for CognitiveConfig {
    fn default() -> Self {
        Self {
            pre_game_enabled: true,
            checkpointing_enabled: true,
            checkpoint_interval: 15,
            after_action_enabled: false,
        }
    }
}

/// Tracks checkpoint state for a session.
#[derive(Debug, Clone, Default)]
pub struct CheckpointTracker {
    /// Number of exchanges since last checkpoint.
    pub exchanges_since_checkpoint: u32,
    /// Timestamp of last checkpoint.
    pub last_checkpoint: Option<chrono::DateTime<Utc>>,
    /// Topics discussed since last checkpoint.
    pub topics: Vec<String>,
    /// Key decisions made since last checkpoint.
    pub decisions: Vec<String>,
}

impl CheckpointTracker {
    /// Record a new exchange.
    pub fn record_exchange(&mut self) {
        self.exchanges_since_checkpoint += 1;
    }

    /// Add a topic being discussed.
    pub fn add_topic(&mut self, topic: &str) {
        if !self.topics.iter().any(|t| t == topic) {
            self.topics.push(topic.to_string());
        }
    }

    /// Add a decision that was made.
    pub fn add_decision(&mut self, decision: &str) {
        self.decisions.push(decision.to_string());
    }

    /// Check if a checkpoint is due.
    pub fn needs_checkpoint(&self, interval: u32) -> bool {
        self.exchanges_since_checkpoint >= interval
    }

    /// Reset after writing a checkpoint.
    pub fn reset(&mut self) {
        self.exchanges_since_checkpoint = 0;
        self.last_checkpoint = Some(Utc::now());
        self.topics.clear();
        self.decisions.clear();
    }

    /// Generate checkpoint content for daily notes.
    ///
    /// Sanitizes topics and decisions to prevent markdown/prompt injection.
    pub fn generate_checkpoint_content(&self) -> String {
        let timestamp = Utc::now().format("%H:%M UTC");
        let mut content = format!("## Conversation Checkpoint [{}]\n", timestamp);

        if !self.topics.is_empty() {
            let sanitized_topics: Vec<String> = self
                .topics
                .iter()
                .map(|t| sanitize_checkpoint_text(t))
                .collect();
            content.push_str(&format!(
                "- Currently discussing: {}\n",
                sanitized_topics.join(", ")
            ));
        }

        if !self.decisions.is_empty() {
            content.push_str("- Key decisions made:\n");
            for decision in &self.decisions {
                content.push_str(&format!("  - {}\n", sanitize_checkpoint_text(decision)));
            }
        }

        content.push('\n');
        content
    }
}

/// Pre-game routine checklist.
///
/// Returns instructions to inject into the prompt before task execution.
pub fn pre_game_instructions() -> &'static str {
    r#"Before executing this task, mentally run through this checklist:
1. Restate the task in one sentence. If you can't, clarify before acting.
2. List constraints and success criteria. What does "done" look like?
3. Retrieve only minimum relevant memory. Don't dump everything — pull what matters.
4. Prefer tools over guessing when facts matter. Check the file, run the command, search first.
5. Identify your mode: Preparation (assembling context) or Execution (running tools). Don't mix them."#
}

/// After-action review template.
pub fn after_action_template(task_name: &str) -> String {
    let timestamp = Utc::now().format("%Y-%m-%d %H:%M UTC");
    format!(
        r#"## [{}] — [{}]
### What happened
- [2-5 bullet points of what was done]

### Tools used
- [which tools/scripts were key]

### What I'd do differently
- [lessons, mistakes, improvements]
"#,
        task_name, timestamp
    )
}

/// Write a checkpoint to the workspace daily log.
pub async fn write_checkpoint(
    workspace: &Workspace,
    tracker: &CheckpointTracker,
) -> Result<(), crate::error::WorkspaceError> {
    let content = tracker.generate_checkpoint_content();
    workspace.append_daily_log(&content).await
}

/// Post-compaction recovery instructions.
///
/// When the agent wakes up after compaction with incomplete context,
/// these instructions help recover the conversation state.
pub fn post_compaction_recovery() -> &'static str {
    r#"Context may be incomplete after compaction. To recover:
1. Read today's daily notes for recent checkpoints
2. Read BRIEFING.md for current context
3. Run memory_search for the last topic you can identify
4. Be honest — tell the user you lost the thread and what you recovered
Never pretend to remember something you don't."#
}

/// Sanitize text for checkpoint content to prevent markdown/prompt injection.
///
/// Removes or escapes characters that could break markdown structure
/// or inject instructions when logs are later included in system prompts.
fn sanitize_checkpoint_text(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == ' ')
        .collect::<String>()
        .replace('#', "\\#")       // Escape markdown headers
        .replace('\n', " ")        // Flatten newlines
        .replace("---", "–––")     // Prevent horizontal rules
        .replace("```", "'''")     // Prevent code blocks
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_tracker_needs_checkpoint() {
        let mut tracker = CheckpointTracker::default();
        assert!(!tracker.needs_checkpoint(15));

        for _ in 0..15 {
            tracker.record_exchange();
        }
        assert!(tracker.needs_checkpoint(15));
    }

    #[test]
    fn test_checkpoint_tracker_reset() {
        let mut tracker = CheckpointTracker::default();
        tracker.record_exchange();
        tracker.add_topic("IronClaw");
        tracker.add_decision("Use WASM sandbox");

        tracker.reset();

        assert_eq!(tracker.exchanges_since_checkpoint, 0);
        assert!(tracker.topics.is_empty());
        assert!(tracker.decisions.is_empty());
        assert!(tracker.last_checkpoint.is_some());
    }

    #[test]
    fn test_checkpoint_content_generation() {
        let mut tracker = CheckpointTracker::default();
        tracker.add_topic("memory system");
        tracker.add_topic("routines");
        tracker.add_decision("Add line numbers to chunks");

        let content = tracker.generate_checkpoint_content();

        assert!(content.contains("Conversation Checkpoint"));
        assert!(content.contains("memory system"));
        assert!(content.contains("routines"));
        assert!(content.contains("Add line numbers to chunks"));
    }

    #[test]
    fn test_pre_game_instructions() {
        let instructions = pre_game_instructions();
        assert!(instructions.contains("Restate the task"));
        assert!(instructions.contains("constraints"));
        assert!(instructions.contains("memory"));
    }

    #[test]
    fn test_after_action_template() {
        let template = after_action_template("Build IronClaw Skills");
        assert!(template.contains("Build IronClaw Skills"));
        assert!(template.contains("What happened"));
        assert!(template.contains("Tools used"));
    }

    #[test]
    fn test_cognitive_config_defaults() {
        let config = CognitiveConfig::default();
        assert!(config.pre_game_enabled);
        assert!(config.checkpointing_enabled);
        assert_eq!(config.checkpoint_interval, 15);
        assert!(!config.after_action_enabled);
    }
}
