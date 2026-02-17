//! Cognitive routines and memory guardian for agent behavior.
//!
//! This module implements two complementary systems:
//!
//! # Cognitive Routines (Prompt-Level Guidance)
//!
//! Behavioral instructions injected into the agent's system prompt:
//! - **Pre-game routine**: 5-step checklist before executing any non-trivial task
//! - **Checkpointing reminders**: Periodic nudges to save session state
//! - **After-action reviews**: Templates for post-task reflection
//!
//! # Memory Guardian (System-Level Enforcement)
//!
//! Automatic, compile-time memory discipline that doesn't rely on the agent
//! choosing to follow instructions:
//!
//! - **Pre-compaction breadcrumbs**: Before compaction compresses context, the
//!   guardian writes a raw state snapshot to daily notes. This is the "black box
//!   recorder" — even if the LLM-generated summary is lossy or wrong, the
//!   breadcrumb preserves tool call counts, recent tools, and session topics.
//!
//! - **Auto-breadcrumbs**: Every N tool calls, automatically append a breadcrumb
//!   to daily notes. This creates a paper trail even if the agent never explicitly
//!   writes a checkpoint.
//!
//! - **Checkpoint gate**: Escalating pressure injected into the system prompt
//!   when too many tool calls happen without a checkpoint write. Starts gentle
//!   ("consider checkpointing"), escalates to firm ("checkpoint needed"), then
//!   urgent ("STOP and checkpoint before doing anything else").
//!
//! - **Memory search tracking**: Counts turns since the last `memory_search`
//!   call and reminds the agent to search before answering memory questions.
//!
//! ## Why Both Layers Exist
//!
//! Cognitive routines are instructions — they work through the agent's attention
//! mechanism and degrade under long contexts or heavy tool use. The memory
//! guardian operates at the system level: it writes breadcrumbs regardless of
//! whether the agent follows instructions, and it blocks compaction until state
//! is preserved. Together they provide defense in depth: the routines guide
//! behavior when attention is available, the guardian catches failures when it
//! isn't.
//!
//! ## Design Decision: Escalating vs. Hard Blocking
//!
//! The checkpoint gate uses escalating pressure (gentle → firm → urgent) rather
//! than hard-blocking tool execution. Hard blocks would break multi-step
//! workflows where the agent legitimately needs many tool calls (e.g., building
//! a project). Escalating reminders work with the agent's attention mechanism
//! — the urgent-level message is emphatic enough to interrupt most workflows
//! without breaking them.
//!
//! ## Design Decision: Tool Calls vs. Exchanges as Metric
//!
//! We track tool calls (not just conversational exchanges) because tool calls
//! are a better proxy for "amount of work that could be lost." An agent might
//! have 3 exchanges but 30 tool calls (reading files, running commands, writing
//! code). Losing 30 tool calls of context is far more damaging than losing 3
//! chat turns.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::workspace::Workspace;

// ==================== Configuration ====================

/// Configuration for cognitive routines and memory guardian.
///
/// Controls which features are active and their thresholds. All features
/// default to sensible values — most deployments won't need to change these.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitiveConfig {
    /// Enable pre-game routine injection into prompts.
    #[serde(default = "default_true")]
    pub pre_game_enabled: bool,

    /// Enable automatic checkpointing.
    #[serde(default = "default_true")]
    pub checkpointing_enabled: bool,

    /// Number of exchanges between checkpoint reminders.
    #[serde(default = "default_checkpoint_interval")]
    pub checkpoint_interval: u32,

    /// Enable after-action review prompts.
    #[serde(default)]
    pub after_action_enabled: bool,

    /// Enable memory guardian (auto-breadcrumbs, checkpoint gate, pre-compaction writes).
    ///
    /// When enabled, the guardian automatically:
    /// - Writes breadcrumbs to daily notes every `breadcrumb_interval` tool calls
    /// - Injects escalating checkpoint reminders into the system prompt
    /// - Writes state snapshots before compaction and reset
    /// - Tracks memory_search usage and reminds the agent to search
    #[serde(default = "default_true")]
    pub guardian_enabled: bool,

    /// Number of tool calls between automatic breadcrumbs.
    ///
    /// Lower values create more granular logs but add write overhead.
    /// Default: 10 (roughly every significant work unit).
    #[serde(default = "default_breadcrumb_interval")]
    pub breadcrumb_interval: u32,

    /// Tool call thresholds for checkpoint gate escalation.
    ///
    /// Three levels: gentle, firm, urgent. The agent sees increasingly
    /// emphatic reminders as tool calls accumulate without a checkpoint.
    /// Default: [12, 20, 30].
    #[serde(default = "default_gate_thresholds")]
    pub gate_thresholds: [u32; 3],

    /// Number of turns without memory_search before injecting a reminder.
    ///
    /// Default: 8. Set to 0 to disable memory search tracking.
    #[serde(default = "default_memory_search_interval")]
    pub memory_search_reminder_interval: u32,
}

fn default_true() -> bool {
    true
}

fn default_checkpoint_interval() -> u32 {
    15
}

fn default_breadcrumb_interval() -> u32 {
    10
}

fn default_gate_thresholds() -> [u32; 3] {
    [12, 20, 30]
}

fn default_memory_search_interval() -> u32 {
    8
}

impl Default for CognitiveConfig {
    fn default() -> Self {
        Self {
            pre_game_enabled: true,
            checkpointing_enabled: true,
            checkpoint_interval: 15,
            after_action_enabled: false,
            guardian_enabled: true,
            breadcrumb_interval: 10,
            gate_thresholds: [12, 20, 30],
            memory_search_reminder_interval: 8,
        }
    }
}

// ==================== Checkpoint Tracker ====================

/// Tracks checkpoint and tool call state for a session.
///
/// This is the core state machine for memory guardian. It tracks both
/// conversational exchanges (for checkpoint reminders) and individual
/// tool calls (for breadcrumbs and the checkpoint gate).
///
/// ## Why Track Both?
///
/// Exchanges (user → agent turns) are the natural unit for conversation flow,
/// but tool calls are the unit of *work*. An agent might have 2 exchanges but
/// execute 25 tool calls (reading files, running shell commands, writing code).
/// The checkpoint gate fires on tool calls because that's where context loss
/// hurts most.
#[derive(Debug, Clone)]
pub struct CheckpointTracker {
    /// Number of conversational exchanges since last checkpoint.
    pub exchanges_since_checkpoint: u32,

    /// Number of tool calls since the agent last wrote to daily notes.
    ///
    /// This is the primary metric for the checkpoint gate. Resets when
    /// the agent writes to `daily/*.md` (detected by `is_checkpoint_write`).
    pub tool_calls_since_checkpoint: u32,

    /// Number of tool calls since the last auto-breadcrumb was written.
    ///
    /// Separate from `tool_calls_since_checkpoint` because breadcrumbs are
    /// system-initiated (the guardian writes them), while checkpoints are
    /// agent-initiated (the agent writes to daily notes). An auto-breadcrumb
    /// resets this counter but does NOT reset `tool_calls_since_checkpoint`
    /// — only the agent writing its own checkpoint does that.
    ///
    /// ## Why Not Reset on Breadcrumbs?
    ///
    /// If breadcrumbs reset the checkpoint counter, the agent would never
    /// feel pressure to write its own checkpoints — the guardian would always
    /// relieve the pressure first. The escalating gate only works if the agent
    /// can't avoid it by doing nothing.
    pub tool_calls_since_breadcrumb: u32,

    /// Rolling window of recent tool names (last 10).
    ///
    /// Included in breadcrumbs and checkpoint gate messages so the agent
    /// (and anyone reading daily notes) can see what was happening.
    pub recent_tools: Vec<String>,

    /// Number of turns since the last `memory_search` tool call.
    ///
    /// Used to remind the agent to search before answering questions about
    /// past work, decisions, or preferences.
    pub turns_since_memory_search: u32,

    /// Timestamp of last checkpoint.
    pub last_checkpoint: Option<chrono::DateTime<Utc>>,

    /// Topics discussed since last checkpoint.
    pub topics: Vec<String>,

    /// Key decisions made since last checkpoint.
    pub decisions: Vec<String>,
}

impl Default for CheckpointTracker {
    fn default() -> Self {
        Self {
            exchanges_since_checkpoint: 0,
            tool_calls_since_checkpoint: 0,
            tool_calls_since_breadcrumb: 0,
            recent_tools: Vec::new(),
            turns_since_memory_search: 0,
            last_checkpoint: None,
            topics: Vec::new(),
            decisions: Vec::new(),
        }
    }
}

impl CheckpointTracker {
    /// Record a new conversational exchange (user → agent turn).
    pub fn record_exchange(&mut self) {
        self.exchanges_since_checkpoint += 1;
        self.turns_since_memory_search += 1;
    }

    /// Record a tool call. Returns `true` if an auto-breadcrumb should be written.
    ///
    /// Tracks the tool name in the rolling window and increments both the
    /// checkpoint and breadcrumb counters. The caller should check the return
    /// value and write a breadcrumb if `true`.
    pub fn record_tool_call(&mut self, tool_name: &str, breadcrumb_interval: u32) -> bool {
        self.tool_calls_since_checkpoint += 1;
        self.tool_calls_since_breadcrumb += 1;

        // Maintain rolling window of last 10 tool names
        self.recent_tools.push(tool_name.to_string());
        if self.recent_tools.len() > 10 {
            self.recent_tools.remove(0);
        }

        // Check if this was a memory_search call
        if tool_name == "memory_search" {
            self.turns_since_memory_search = 0;
        }

        // Signal breadcrumb needed
        breadcrumb_interval > 0 && self.tool_calls_since_breadcrumb >= breadcrumb_interval
    }

    /// Reset the breadcrumb counter after writing an auto-breadcrumb.
    ///
    /// Note: does NOT reset `tool_calls_since_checkpoint`. Only an agent-initiated
    /// checkpoint write should do that (via `on_agent_checkpoint`).
    pub fn on_breadcrumb_written(&mut self) {
        self.tool_calls_since_breadcrumb = 0;
    }

    /// Detect if a tool call represents an agent-initiated checkpoint.
    ///
    /// A checkpoint is any write/edit/append to `daily/*.md` files. This is how
    /// we detect the agent following checkpoint pressure — it doesn't need to
    /// call a special API, it just writes to its daily notes like normal.
    pub fn is_checkpoint_write(tool_name: &str, target: &str) -> bool {
        matches!(
            tool_name,
            "write" | "edit" | "append" | "memory_write" | "memory_append"
        ) && target.contains("daily/")
            && target.ends_with(".md")
    }

    /// Reset checkpoint counters after detecting an agent-initiated checkpoint.
    ///
    /// This is the reward signal: the agent wrote to daily notes, so checkpoint
    /// pressure resets. Topics and decisions also clear since they've been captured.
    pub fn on_agent_checkpoint(&mut self) {
        self.tool_calls_since_checkpoint = 0;
        self.tool_calls_since_breadcrumb = 0;
        self.exchanges_since_checkpoint = 0;
        self.last_checkpoint = Some(Utc::now());
        self.topics.clear();
        self.decisions.clear();
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

    /// Check if an exchange-based checkpoint reminder is due.
    pub fn needs_checkpoint(&self, interval: u32) -> bool {
        self.exchanges_since_checkpoint >= interval
    }

    /// Reset after writing a checkpoint (legacy interface, calls `on_agent_checkpoint`).
    pub fn reset(&mut self) {
        self.on_agent_checkpoint();
    }

    /// Generate checkpoint content for daily notes.
    ///
    /// Sanitizes topics and decisions to prevent markdown/prompt injection
    /// when these notes are later included in system prompts.
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

// ==================== Memory Guardian ====================

/// Generate a pre-compaction breadcrumb for the daily log.
///
/// Called synchronously before compaction runs. This captures the raw session
/// state as a "black box recorder" that survives even if the LLM-generated
/// compaction summary is lossy or incomplete.
///
/// ## Why This Exists
///
/// IronClaw's compaction already writes an LLM-generated summary to daily notes
/// (via `write_summary_to_workspace` in compaction.rs). But that summary is:
/// 1. Generated *after* context is compressed — it may miss details
/// 2. Produced by the LLM — it can hallucinate or omit important state
/// 3. Focused on conversation content — it doesn't capture tool call patterns
///
/// The breadcrumb captures what the summary can't: how many tool calls happened,
/// what tools were used, and structured metadata about the session.
pub fn pre_compaction_breadcrumb(tracker: &CheckpointTracker) -> String {
    let timestamp = Utc::now().format("%H:%M UTC");
    let recent = if tracker.recent_tools.is_empty() {
        "none".to_string()
    } else {
        tracker.recent_tools.join(", ")
    };
    let topics = if tracker.topics.is_empty() {
        "none".to_string()
    } else {
        tracker
            .topics
            .iter()
            .map(|t| sanitize_checkpoint_text(t))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let decisions = if tracker.decisions.is_empty() {
        "none".to_string()
    } else {
        tracker
            .decisions
            .iter()
            .map(|d| sanitize_checkpoint_text(d))
            .collect::<Vec<_>>()
            .join("; ")
    };

    format!(
        "\n## Pre-Compaction Breadcrumb [{timestamp}]\n\
         - ⚠️ Context compaction about to run\n\
         - Exchanges since checkpoint: {exchanges}\n\
         - Tool calls since checkpoint: {tool_calls}\n\
         - Recent tools: {recent}\n\
         - Topics: {topics}\n\
         - Decisions: {decisions}\n",
        timestamp = timestamp,
        exchanges = tracker.exchanges_since_checkpoint,
        tool_calls = tracker.tool_calls_since_checkpoint,
        recent = recent,
        topics = topics,
        decisions = decisions,
    )
}

/// Generate a pre-reset breadcrumb for the daily log.
///
/// Similar to pre-compaction breadcrumb, but for `/new` and `/reset` commands
/// which clear session state entirely (worse than compaction, which at least
/// keeps recent turns).
pub fn pre_reset_breadcrumb(tracker: &CheckpointTracker) -> String {
    let timestamp = Utc::now().format("%H:%M UTC");
    let recent = if tracker.recent_tools.is_empty() {
        "none".to_string()
    } else {
        tracker.recent_tools.join(", ")
    };

    format!(
        "\n## Pre-Reset Breadcrumb [{timestamp}]\n\
         - 🔄 Session reset/new thread — all context being cleared\n\
         - Exchanges since checkpoint: {exchanges}\n\
         - Tool calls since checkpoint: {tool_calls}\n\
         - Recent tools: {recent}\n",
        timestamp = timestamp,
        exchanges = tracker.exchanges_since_checkpoint,
        tool_calls = tracker.tool_calls_since_checkpoint,
        recent = recent,
    )
}

/// Generate an auto-breadcrumb for the daily log.
///
/// Written automatically every N tool calls by the guardian. Unlike agent
/// checkpoints (which capture what the agent thinks is important), breadcrumbs
/// are mechanical records of what actually happened.
pub fn auto_breadcrumb(tracker: &CheckpointTracker) -> String {
    let timestamp = Utc::now().format("%H:%M UTC");
    let recent = if tracker.recent_tools.is_empty() {
        "none".to_string()
    } else {
        tracker.recent_tools.join(", ")
    };

    format!(
        "\n## Auto-Breadcrumb [{timestamp}]\n\
         - Tool calls since checkpoint: {tool_calls}\n\
         - Recent tools: {recent}\n",
        timestamp = timestamp,
        tool_calls = tracker.tool_calls_since_checkpoint,
        recent = recent,
    )
}

/// Generate a checkpoint gate message for the system prompt.
///
/// Returns `None` if tool calls are below the gentle threshold, or an
/// escalating reminder message at three levels:
///
/// - **Gentle** (default: 12 calls): "Consider checkpointing"
/// - **Firm** (default: 20 calls): "Checkpoint needed"
/// - **Urgent** (default: 30 calls): "STOP and checkpoint NOW"
///
/// ## Why Escalating, Not Binary?
///
/// A binary "checkpoint now" at a fixed threshold would fire during legitimate
/// multi-step workflows (e.g., building a project with 30+ tool calls). The
/// gentle level is easy to override, the firm level gets attention, and the
/// urgent level is hard to ignore. This matches how human managers escalate:
/// casual reminder → direct request → urgent demand.
pub fn checkpoint_gate_message(
    tracker: &CheckpointTracker,
    thresholds: [u32; 3],
) -> Option<String> {
    let calls = tracker.tool_calls_since_checkpoint;
    let [gentle, firm, urgent] = thresholds;

    if calls >= urgent {
        let recent: Vec<_> = tracker.recent_tools.iter().rev().take(5).cloned().collect();
        Some(format!(
            "🚨 CHECKPOINT OVERDUE ({calls} tool calls without writing to memory):\n\
             STOP. Before doing ANYTHING else, write a checkpoint to daily notes.\n\
             Include: what you've been working on, key decisions, current state, next steps.\n\
             Recent tools: {}\n\
             This is not optional.",
            recent.join(", ")
        ))
    } else if calls >= firm {
        Some(format!(
            "⚠️ CHECKPOINT NEEDED ({calls} tool calls without writing to memory):\n\
             Write a checkpoint to daily notes soon. Include current topic, decisions, and next steps."
        ))
    } else if calls >= gentle {
        Some(format!(
            "📋 Consider checkpointing — {calls} tool calls since your last memory write."
        ))
    } else {
        None
    }
}

/// Generate a memory search reminder for the system prompt.
///
/// Returns `None` if turns are below the threshold, or a reminder to search
/// before answering questions about past work.
pub fn memory_search_reminder(tracker: &CheckpointTracker, threshold: u32) -> Option<String> {
    if threshold == 0 {
        return None;
    }
    if tracker.turns_since_memory_search >= threshold {
        Some(format!(
            "📝 MEMORY REMINDER: You haven't called memory_search in {} turns. \
             If you're about to answer a question about past work, decisions, or preferences — search first.",
            tracker.turns_since_memory_search
        ))
    } else {
        None
    }
}

// ==================== Prompt Templates ====================

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

/// Write a breadcrumb to the workspace daily log.
///
/// Used by the guardian for pre-compaction, pre-reset, and auto-breadcrumbs.
pub async fn write_breadcrumb(
    workspace: &Workspace,
    content: &str,
) -> Result<(), crate::error::WorkspaceError> {
    workspace.append_daily_log(content).await
}

/// Post-compaction recovery instructions.
///
/// When the agent wakes up after compaction with incomplete context,
/// these instructions help recover the conversation state.
pub fn post_compaction_recovery() -> &'static str {
    r#"Context may be incomplete after compaction. To recover:
1. Read today's daily notes for recent checkpoints and breadcrumbs
2. Read BRIEFING.md for current context
3. Run memory_search for the last topic you can identify
4. Be honest — tell the user you lost the thread and what you recovered
Never pretend to remember something you don't."#
}

// ==================== Internal Helpers ====================

/// Sanitize text for checkpoint content to prevent markdown/prompt injection.
///
/// Removes or escapes characters that could break markdown structure
/// or inject instructions when logs are later included in system prompts.
fn sanitize_checkpoint_text(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == ' ')
        .collect::<String>()
        .replace('#', "\\#") // Escape markdown headers
        .replace('\n', " ") // Flatten newlines
        .replace("---", "–––") // Prevent horizontal rules
        .replace("```", "'''") // Prevent code blocks
        .trim()
        .to_string()
}

// ==================== Tests ====================

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
        assert!(config.guardian_enabled);
        assert_eq!(config.breadcrumb_interval, 10);
        assert_eq!(config.gate_thresholds, [12, 20, 30]);
        assert_eq!(config.memory_search_reminder_interval, 8);
    }

    // ==================== Memory Guardian Tests ====================

    #[test]
    fn test_tool_call_tracking_triggers_breadcrumb() {
        let mut tracker = CheckpointTracker::default();

        // First 9 calls should not trigger
        for i in 0..9 {
            assert!(
                !tracker.record_tool_call("shell", 10),
                "Should not trigger at call {}",
                i + 1
            );
        }

        // 10th call should trigger
        assert!(
            tracker.record_tool_call("shell", 10),
            "Should trigger at call 10"
        );
        assert_eq!(tracker.tool_calls_since_checkpoint, 10);
        assert_eq!(tracker.tool_calls_since_breadcrumb, 10);

        // After breadcrumb written, breadcrumb counter resets but checkpoint doesn't
        tracker.on_breadcrumb_written();
        assert_eq!(tracker.tool_calls_since_breadcrumb, 0);
        assert_eq!(
            tracker.tool_calls_since_checkpoint, 10,
            "Checkpoint counter should NOT reset on breadcrumb"
        );
    }

    #[test]
    fn test_agent_checkpoint_resets_all_counters() {
        let mut tracker = CheckpointTracker::default();
        for _ in 0..25 {
            tracker.record_tool_call("shell", 10);
        }
        tracker.add_topic("testing");
        tracker.add_decision("use guardian");

        tracker.on_agent_checkpoint();

        assert_eq!(tracker.tool_calls_since_checkpoint, 0);
        assert_eq!(tracker.tool_calls_since_breadcrumb, 0);
        assert_eq!(tracker.exchanges_since_checkpoint, 0);
        assert!(tracker.topics.is_empty());
        assert!(tracker.decisions.is_empty());
        assert!(tracker.last_checkpoint.is_some());
    }

    #[test]
    fn test_is_checkpoint_write() {
        assert!(CheckpointTracker::is_checkpoint_write(
            "write",
            "daily/2026-02-15.md"
        ));
        assert!(CheckpointTracker::is_checkpoint_write(
            "edit",
            "daily/2026-02-15.md"
        ));
        assert!(CheckpointTracker::is_checkpoint_write(
            "append",
            "daily/2026-01-01.md"
        ));
        assert!(CheckpointTracker::is_checkpoint_write(
            "memory_write",
            "daily/2026-02-15.md"
        ));

        // Should NOT match non-daily paths
        assert!(!CheckpointTracker::is_checkpoint_write(
            "write",
            "MEMORY.md"
        ));
        assert!(!CheckpointTracker::is_checkpoint_write("write", "notes.md"));
        assert!(!CheckpointTracker::is_checkpoint_write(
            "read",
            "daily/2026-02-15.md"
        ));
    }

    #[test]
    fn test_memory_search_resets_on_call() {
        let mut tracker = CheckpointTracker::default();
        tracker.record_exchange(); // turn 1
        tracker.record_exchange(); // turn 2
        assert_eq!(tracker.turns_since_memory_search, 2);

        tracker.record_tool_call("memory_search", 10);
        assert_eq!(tracker.turns_since_memory_search, 0);
    }

    #[test]
    fn test_checkpoint_gate_escalation() {
        let thresholds = [12, 20, 30];
        let mut tracker = CheckpointTracker::default();

        // Below gentle — no message
        for _ in 0..11 {
            tracker.record_tool_call("shell", 10);
        }
        assert!(checkpoint_gate_message(&tracker, thresholds).is_none());

        // At gentle threshold
        tracker.record_tool_call("shell", 10);
        let msg = checkpoint_gate_message(&tracker, thresholds).unwrap();
        assert!(msg.contains("📋"), "Should be gentle at 12 calls");
        assert!(msg.contains("12"));

        // At firm threshold
        for _ in 0..8 {
            tracker.record_tool_call("shell", 10);
        }
        let msg = checkpoint_gate_message(&tracker, thresholds).unwrap();
        assert!(msg.contains("⚠️"), "Should be firm at 20 calls");

        // At urgent threshold
        for _ in 0..10 {
            tracker.record_tool_call("shell", 10);
        }
        let msg = checkpoint_gate_message(&tracker, thresholds).unwrap();
        assert!(msg.contains("🚨"), "Should be urgent at 30 calls");
        assert!(msg.contains("STOP"));
    }

    #[test]
    fn test_memory_search_reminder() {
        let mut tracker = CheckpointTracker::default();

        // Below threshold — no reminder
        for _ in 0..7 {
            tracker.record_exchange();
        }
        assert!(memory_search_reminder(&tracker, 8).is_none());

        // At threshold
        tracker.record_exchange();
        let msg = memory_search_reminder(&tracker, 8).unwrap();
        assert!(msg.contains("8 turns"));

        // Disabled when threshold is 0
        assert!(memory_search_reminder(&tracker, 0).is_none());
    }

    #[test]
    fn test_pre_compaction_breadcrumb_content() {
        let mut tracker = CheckpointTracker::default();
        for _ in 0..5 {
            tracker.record_tool_call("shell", 10);
        }
        tracker.record_tool_call("write", 10);
        tracker.add_topic("memory system");
        tracker.add_decision("use guardian");

        let breadcrumb = pre_compaction_breadcrumb(&tracker);
        assert!(breadcrumb.contains("Pre-Compaction Breadcrumb"));
        assert!(breadcrumb.contains("Tool calls since checkpoint: 6"));
        assert!(breadcrumb.contains("shell"));
        assert!(breadcrumb.contains("memory system"));
        assert!(breadcrumb.contains("use guardian"));
    }

    #[test]
    fn test_auto_breadcrumb_content() {
        let mut tracker = CheckpointTracker::default();
        tracker.record_tool_call("read", 10);
        tracker.record_tool_call("shell", 10);
        tracker.record_tool_call("write", 10);

        let breadcrumb = auto_breadcrumb(&tracker);
        assert!(breadcrumb.contains("Auto-Breadcrumb"));
        assert!(breadcrumb.contains("Tool calls since checkpoint: 3"));
        assert!(breadcrumb.contains("read, shell, write"));
    }

    #[test]
    fn test_recent_tools_rolling_window() {
        let mut tracker = CheckpointTracker::default();

        // Add 15 tool calls — should only keep last 10
        for i in 0..15 {
            tracker.record_tool_call(&format!("tool_{}", i), 100);
        }

        assert_eq!(tracker.recent_tools.len(), 10);
        assert_eq!(tracker.recent_tools[0], "tool_5");
        assert_eq!(tracker.recent_tools[9], "tool_14");
    }

    #[test]
    fn test_checkpoint_content_sanitization() {
        let mut tracker = CheckpointTracker::default();
        tracker.add_topic("# Header Injection");
        tracker.add_decision("```code block```");

        let content = tracker.generate_checkpoint_content();
        // Headers should be escaped
        assert!(!content.contains("\n#"), "Headers should be escaped");
        // Code blocks should be neutralized
        assert!(
            !content.contains("```"),
            "Code blocks should be neutralized"
        );
    }

    #[test]
    fn test_breadcrumb_interval_zero_disables() {
        let mut tracker = CheckpointTracker::default();
        // With interval 0, should never trigger
        for _ in 0..100 {
            assert!(!tracker.record_tool_call("shell", 0));
        }
    }
}
