//! Cognitive Guardian: proactive memory discipline for AI agents.
//!
//! # What
//!
//! A behavioral layer that nudges the agent to checkpoint its work and search
//! memory *before* context pressure forces compaction. Think of it as the
//! "pregame routine" — structured habits that prevent amnesia instead of
//! treating it after the fact.
//!
//! # Why
//!
//! IronClaw already has excellent reactive infrastructure:
//! - `context_monitor.rs` detects when context is nearly full
//! - `compaction.rs` summarizes and archives old turns
//! - `workspace/` provides persistent memory with search
//!
//! But reactive compaction is the ER — it kicks in when context is already
//! overflowing. By then, nuance is lost in summarization. The Cognitive
//! Guardian adds *preventive care*: it tracks the agent's behavior and
//! injects gentle nudges to encourage proactive memory hygiene.
//!
//! This is the difference between an agent that *has* memory and one that
//! *uses* memory well.
//!
//! # How
//!
//! The guardian tracks two counters per thread:
//!
//! 1. **Tool calls since last checkpoint** — how many tool calls since the
//!    agent last wrote to persistent storage (workspace daily log, MEMORY.md,
//!    or any workspace file). At configurable thresholds (default 12/20/30),
//!    the guardian injects escalating nudge messages into the LLM context.
//!
//! 2. **Turns since last `memory_search`** — how many user↔agent exchanges
//!    since the agent last searched its memory. When the agent is about to
//!    answer a question that might rely on past context, a reminder nudge
//!    fires.
//!
//! Additionally, the guardian can write **breadcrumbs** — lightweight state
//! snapshots written to the workspace daily log — at key moments:
//!
//! - **Pre-compaction**: Before context compaction runs, capture what's about
//!   to be summarized away. Insurance against lossy summarization.
//! - **Pre-reset**: Before a `/reset` or `/clear`, snapshot the current state
//!   so nothing is silently lost.
//! - **Auto-breadcrumb**: Every N tool calls, write a silent checkpoint with
//!   the session label, tool call count, and recent tool names.
//!
//! Integration points are minimal: the guardian hooks into the existing
//! `LoopDelegate` pattern via `before_llm_call()` for nudge injection and
//! `execute_tool_calls()` for counter tracking.
//!
//! # Configuration
//!
//! ```yaml
//! cognitive:
//!   enabled: true
//!   checkpoint_thresholds: [12, 20, 30]
//!   memory_search_reminder_turns: 8
//!   auto_breadcrumb_interval: 25
//!   pre_compaction_breadcrumb: true
//!   pre_reset_breadcrumb: true
//! ```
//!
//! All features are opt-in and off by default. The guardian is zero-cost when
//! disabled — no allocations, no counter tracking, no nudges.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::workspace::Workspace;

// ───────────────────────── Configuration ─────────────────────────

/// Configuration for the Cognitive Guardian.
///
/// All fields have safe defaults. When `enabled` is false, the guardian
/// is a no-op — all methods return immediately without allocations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitiveConfig {
    /// Master switch. When false, the guardian does nothing.
    #[serde(default)]
    pub enabled: bool,

    /// Tool-call counts at which checkpoint nudges fire.
    /// Each threshold triggers once per counter reset cycle.
    /// Default: [12, 20, 30].
    #[serde(default = "default_checkpoint_thresholds")]
    pub checkpoint_thresholds: Vec<usize>,

    /// Number of user↔agent turns without a `memory_search` call
    /// before a reminder nudge fires. Default: 8.
    #[serde(default = "default_memory_search_reminder_turns")]
    pub memory_search_reminder_turns: usize,

    /// Write an auto-breadcrumb to the workspace daily log every N
    /// tool calls. Set to 0 to disable. Default: 25.
    #[serde(default = "default_auto_breadcrumb_interval")]
    pub auto_breadcrumb_interval: usize,

    /// Write a state snapshot before context compaction runs.
    #[serde(default = "default_true")]
    pub pre_compaction_breadcrumb: bool,

    /// Write a state snapshot before `/reset` or `/clear`.
    #[serde(default = "default_true")]
    pub pre_reset_breadcrumb: bool,
}

fn default_checkpoint_thresholds() -> Vec<usize> {
    vec![12, 20, 30]
}
fn default_memory_search_reminder_turns() -> usize {
    8
}
fn default_auto_breadcrumb_interval() -> usize {
    25
}
fn default_true() -> bool {
    true
}

impl Default for CognitiveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            checkpoint_thresholds: default_checkpoint_thresholds(),
            memory_search_reminder_turns: default_memory_search_reminder_turns(),
            auto_breadcrumb_interval: default_auto_breadcrumb_interval(),
            pre_compaction_breadcrumb: true,
            pre_reset_breadcrumb: true,
        }
    }
}

// ───────────────────────── Nudge types ─────────────────────────

/// A nudge is an injected system message that encourages the agent to
/// practice good memory hygiene. Nudges are informational — they never
/// block execution or force tool calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nudge {
    /// The agent has made many tool calls without checkpointing.
    CheckpointReminder { tool_calls: usize, threshold: usize },
    /// The agent hasn't searched memory in a while.
    MemorySearchReminder { turns_since_search: usize },
}

impl fmt::Display for Nudge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Nudge::CheckpointReminder {
                tool_calls,
                threshold,
            } => {
                let urgency = match *threshold {
                    t if t <= 12 => "📋 Consider checkpointing",
                    t if t <= 20 => "⚠️ You should checkpoint soon",
                    _ => "🚨 Checkpoint now — context loss risk is high",
                };
                write!(
                    f,
                    "{urgency} — {tool_calls} tool calls since your last memory write. \
                     Write key decisions, progress, and next steps to the \
                     workspace daily log or MEMORY.md before they're lost \
                     to compaction."
                )
            }
            Nudge::MemorySearchReminder { turns_since_search } => {
                write!(
                    f,
                    "📝 MEMORY REMINDER: You haven't called memory_search in \
                     {turns_since_search} turns. If you're about to answer a \
                     question about past work, decisions, or preferences — \
                     search first."
                )
            }
        }
    }
}

// ───────────────────────── Guardian state ─────────────────────────

/// Runtime state for the Cognitive Guardian, tracked per thread.
///
/// This struct is intentionally *not* serialized with the thread — it's
/// ephemeral session state. Counters reset on session restart, which is
/// fine: the point is to catch long-running sessions that drift, not to
/// persist nudge counts across restarts.
#[derive(Debug, Clone)]
pub struct CognitiveGuardian {
    config: CognitiveConfig,
    /// Tool calls since the last checkpoint (write to workspace).
    tool_calls_since_checkpoint: usize,
    /// User↔agent turns since the last `memory_search` tool call.
    turns_since_memory_search: usize,
    /// Which checkpoint thresholds have already fired (to avoid repeats).
    fired_thresholds: Vec<bool>,
    /// Recent tool names (ring buffer for breadcrumbs).
    recent_tools: Vec<String>,
    /// When the guardian was created (for breadcrumb timestamps).
    #[allow(dead_code)]
    created_at: DateTime<Utc>,
}

impl CognitiveGuardian {
    /// Create a new guardian with the given configuration.
    pub fn new(config: CognitiveConfig) -> Self {
        let fired_thresholds = vec![false; config.checkpoint_thresholds.len()];
        Self {
            config,
            tool_calls_since_checkpoint: 0,
            turns_since_memory_search: 0,
            fired_thresholds,
            recent_tools: Vec::new(),
            created_at: Utc::now(),
        }
    }

    /// Create a disabled (no-op) guardian.
    pub fn disabled() -> Self {
        Self::new(CognitiveConfig::default())
    }

    /// Whether the guardian is active.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    // ───────────── Event tracking ─────────────

    /// Record that a tool was called. Updates counters and recent-tools
    /// ring buffer. Call this after every tool execution.
    ///
    /// Returns `true` if the tool call was a checkpoint (workspace write),
    /// which resets the checkpoint counter.
    pub fn on_tool_call(&mut self, tool_name: &str) -> bool {
        if !self.config.enabled {
            return false;
        }

        self.tool_calls_since_checkpoint += 1;

        // Keep last 5 tool names for breadcrumbs
        if self.recent_tools.len() >= 5 {
            self.recent_tools.remove(0);
        }
        self.recent_tools.push(tool_name.to_string());

        // Check if this tool call is a checkpoint (workspace write)
        let is_checkpoint = is_checkpoint_tool(tool_name);
        if is_checkpoint {
            self.reset_checkpoint_counter();
        }

        // Track memory engagement separately — resets the search reminder
        // counter but does NOT reset the checkpoint counter (reads aren't writes).
        if is_memory_engagement_tool(tool_name) {
            self.turns_since_memory_search = 0;
        }

        is_checkpoint
    }

    /// Record that a new user↔agent turn has started. Increments the
    /// memory search counter.
    pub fn on_new_turn(&mut self) {
        if !self.config.enabled {
            return;
        }
        self.turns_since_memory_search += 1;
    }

    /// Reset checkpoint counters (e.g., after a manual checkpoint).
    fn reset_checkpoint_counter(&mut self) {
        self.tool_calls_since_checkpoint = 0;
        for fired in &mut self.fired_thresholds {
            *fired = false;
        }
    }

    // ───────────── Nudge generation ─────────────

    /// Get the highest-priority nudge that should be injected, if any.
    ///
    /// Returns at most one nudge per call. Priority order:
    /// 1. Checkpoint reminders (highest unfired threshold)
    /// 2. Memory search reminders
    ///
    /// Each threshold fires only once per cycle to avoid spamming.
    pub fn get_nudge(&mut self) -> Option<Nudge> {
        if !self.config.enabled {
            return None;
        }

        // Check checkpoint thresholds (highest first for urgency)
        for (i, threshold) in self.config.checkpoint_thresholds.iter().enumerate().rev() {
            if self.tool_calls_since_checkpoint >= *threshold && !self.fired_thresholds[i] {
                self.fired_thresholds[i] = true;
                return Some(Nudge::CheckpointReminder {
                    tool_calls: self.tool_calls_since_checkpoint,
                    threshold: *threshold,
                });
            }
        }

        // Check memory search reminder
        if self.config.memory_search_reminder_turns > 0
            && self.turns_since_memory_search >= self.config.memory_search_reminder_turns
        {
            // Only fire once per threshold crossing (reset on next search)
            if self.turns_since_memory_search == self.config.memory_search_reminder_turns {
                return Some(Nudge::MemorySearchReminder {
                    turns_since_search: self.turns_since_memory_search,
                });
            }
        }

        None
    }

    /// Check whether an auto-breadcrumb should be written at this point.
    /// Returns `true` if the tool call count is a multiple of the configured
    /// interval. The caller is responsible for actually writing the breadcrumb.
    pub fn should_auto_breadcrumb(&self) -> bool {
        if !self.config.enabled || self.config.auto_breadcrumb_interval == 0 {
            return false;
        }
        self.tool_calls_since_checkpoint > 0
            && self
                .tool_calls_since_checkpoint
                .is_multiple_of(self.config.auto_breadcrumb_interval)
    }

    // ───────────── Breadcrumbs ─────────────

    /// Write a pre-compaction breadcrumb to the workspace.
    ///
    /// Captures current session state before compaction runs, so that
    /// even if summarization is lossy, the key context is preserved in
    /// the daily log.
    pub async fn write_pre_compaction_breadcrumb(
        &self,
        workspace: &Workspace,
        session_label: &str,
        topic_summary: Option<&str>,
    ) {
        if !self.config.enabled || !self.config.pre_compaction_breadcrumb {
            return;
        }

        let entry = format!(
            "### Pre-Compaction Breadcrumb\n\
             - Session: {session_label}\n\
             - Tool calls since checkpoint: {}\n\
             - Recent tools: {}\n\
             {}",
            self.tool_calls_since_checkpoint,
            self.recent_tools.join(", "),
            topic_summary
                .map(|s| format!("- Context: {s}\n"))
                .unwrap_or_default(),
        );

        if let Err(e) = workspace.append_daily_log(&entry).await {
            tracing::warn!("Failed to write pre-compaction breadcrumb: {}", e);
        }
    }

    /// Write a pre-reset breadcrumb to the workspace.
    ///
    /// Captures state before a `/reset` or `/clear` so the user doesn't
    /// silently lose context.
    pub async fn write_pre_reset_breadcrumb(
        &self,
        workspace: &Workspace,
        session_label: &str,
        turn_count: usize,
    ) {
        if !self.config.enabled || !self.config.pre_reset_breadcrumb {
            return;
        }

        let entry = format!(
            "### Pre-Reset Breadcrumb\n\
             - Session: {session_label}\n\
             - Turns in thread: {turn_count}\n\
             - Tool calls since checkpoint: {}\n\
             - Recent tools: {}",
            self.tool_calls_since_checkpoint,
            self.recent_tools.join(", "),
        );

        if let Err(e) = workspace.append_daily_log(&entry).await {
            tracing::warn!("Failed to write pre-reset breadcrumb: {}", e);
        }
    }

    /// Write an auto-breadcrumb to the workspace.
    ///
    /// A lightweight, periodic state snapshot. Silent — the agent doesn't
    /// see this in its context window. Written directly to the daily log
    /// for future reference.
    pub async fn write_auto_breadcrumb(&self, workspace: &Workspace, session_label: &str) {
        if !self.config.enabled {
            return;
        }

        let entry = format!(
            "### Auto-Breadcrumb\n\
             - Session: {session_label}\n\
             - Tool calls since checkpoint: {}\n\
             - Recent: {}",
            self.tool_calls_since_checkpoint,
            self.recent_tools.join(", "),
        );

        if let Err(e) = workspace.append_daily_log(&entry).await {
            tracing::warn!("Failed to write auto-breadcrumb: {}", e);
        }
    }

    // ───────────── Accessors (for testing / metrics) ─────────────

    /// Current tool calls since last checkpoint.
    pub fn tool_calls_since_checkpoint(&self) -> usize {
        self.tool_calls_since_checkpoint
    }

    /// Current turns since last memory search.
    pub fn turns_since_memory_search(&self) -> usize {
        self.turns_since_memory_search
    }

    /// Recent tool names (for breadcrumb snapshots).
    pub fn recent_tools(&self) -> &[String] {
        &self.recent_tools
    }
}

// ───────────────────────── Helpers ─────────────────────────

/// Determine whether a tool call counts as a "checkpoint" — a write to
/// persistent workspace storage that the agent can later search/recall.
///
/// This covers the standard workspace tools. Agents that use custom
/// checkpoint tools can extend this list via configuration (future work).
fn is_checkpoint_tool(name: &str) -> bool {
    matches!(
        name,
        "workspace_write"
            | "workspace_append"
            | "write" // OpenClaw file write
            | "edit" // OpenClaw file edit
    )
}

/// Determine whether a tool call indicates memory engagement — the agent
/// is actively interacting with its memory system. This resets the
/// memory-search reminder counter but does NOT reset the checkpoint
/// counter (reading memory is not the same as writing a checkpoint).
fn is_memory_engagement_tool(name: &str) -> bool {
    matches!(
        name,
        "memory_search" | "memory_get" | "workspace_search" | "workspace_read"
    )
}

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_config() -> CognitiveConfig {
        CognitiveConfig {
            enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn test_disabled_guardian_is_noop() {
        let mut guardian = CognitiveGuardian::disabled();
        for _ in 0..50 {
            guardian.on_tool_call("exec");
        }
        guardian.on_new_turn();
        assert_eq!(guardian.get_nudge(), None);
        assert!(!guardian.should_auto_breadcrumb());
    }

    #[test]
    fn test_checkpoint_nudge_fires_at_thresholds() {
        let mut guardian = CognitiveGuardian::new(enabled_config());

        // 11 tool calls — no nudge yet
        for _ in 0..11 {
            guardian.on_tool_call("exec");
        }
        assert_eq!(guardian.get_nudge(), None);

        // 12th tool call — first threshold fires
        guardian.on_tool_call("exec");
        let nudge = guardian.get_nudge();
        assert!(matches!(
            nudge,
            Some(Nudge::CheckpointReminder {
                tool_calls: 12,
                threshold: 12,
            })
        ));

        // Calling again at 12 — already fired, no repeat
        assert_eq!(guardian.get_nudge(), None);

        // Reach 20 — second threshold
        for _ in 0..8 {
            guardian.on_tool_call("exec");
        }
        let nudge = guardian.get_nudge();
        assert!(matches!(
            nudge,
            Some(Nudge::CheckpointReminder {
                tool_calls: 20,
                threshold: 20,
            })
        ));
    }

    #[test]
    fn test_checkpoint_resets_on_workspace_write() {
        let mut guardian = CognitiveGuardian::new(enabled_config());

        for _ in 0..15 {
            guardian.on_tool_call("exec");
        }
        assert_eq!(guardian.tool_calls_since_checkpoint(), 15);

        // A workspace write resets the counter
        let was_checkpoint = guardian.on_tool_call("workspace_write");
        assert!(was_checkpoint);
        assert_eq!(guardian.tool_calls_since_checkpoint(), 0);

        // Nudge no longer fires
        assert_eq!(guardian.get_nudge(), None);
    }

    #[test]
    fn test_memory_search_resets_reminder_not_checkpoint() {
        let config = CognitiveConfig {
            enabled: true,
            memory_search_reminder_turns: 3,
            ..Default::default()
        };
        let mut guardian = CognitiveGuardian::new(config);

        // Accumulate some tool calls
        for _ in 0..15 {
            guardian.on_tool_call("exec");
        }
        assert_eq!(guardian.tool_calls_since_checkpoint(), 15);

        // memory_search should reset the search reminder but NOT the checkpoint counter
        guardian.on_new_turn();
        guardian.on_new_turn();
        guardian.on_new_turn();
        guardian.on_tool_call("memory_search");

        // Search reminder is reset
        assert_eq!(guardian.turns_since_memory_search(), 0);
        // Checkpoint counter is NOT reset (memory_search is a read, not a write)
        assert_eq!(guardian.tool_calls_since_checkpoint(), 16);
    }

    #[test]
    fn test_memory_search_reminder() {
        let config = CognitiveConfig {
            enabled: true,
            memory_search_reminder_turns: 3,
            ..Default::default()
        };
        let mut guardian = CognitiveGuardian::new(config);

        guardian.on_new_turn();
        guardian.on_new_turn();
        assert_eq!(guardian.get_nudge(), None);

        // 3rd turn — reminder fires
        guardian.on_new_turn();
        let nudge = guardian.get_nudge();
        assert!(matches!(
            nudge,
            Some(Nudge::MemorySearchReminder {
                turns_since_search: 3,
            })
        ));

        // A memory_search call resets it
        guardian.on_tool_call("memory_search");
        assert_eq!(guardian.turns_since_memory_search(), 0);
    }

    #[test]
    fn test_memory_engagement_tools_reset_reminder() {
        let config = CognitiveConfig {
            enabled: true,
            memory_search_reminder_turns: 3,
            ..Default::default()
        };
        let mut guardian = CognitiveGuardian::new(config);

        guardian.on_new_turn();
        guardian.on_new_turn();
        guardian.on_new_turn();

        // memory_get also counts as memory engagement
        guardian.on_tool_call("memory_get");
        assert_eq!(guardian.turns_since_memory_search(), 0);
    }

    #[test]
    fn test_auto_breadcrumb_interval() {
        let config = CognitiveConfig {
            enabled: true,
            auto_breadcrumb_interval: 10,
            ..Default::default()
        };
        let mut guardian = CognitiveGuardian::new(config);

        for i in 1..=25 {
            guardian.on_tool_call("exec");
            if i == 10 || i == 20 {
                assert!(guardian.should_auto_breadcrumb(), "should fire at {}", i);
            } else {
                assert!(
                    !guardian.should_auto_breadcrumb(),
                    "should not fire at {}",
                    i
                );
            }
        }
    }

    #[test]
    fn test_recent_tools_ring_buffer() {
        let mut guardian = CognitiveGuardian::new(enabled_config());

        for name in &["read", "exec", "write", "search", "edit", "deploy", "test"] {
            guardian.on_tool_call(name);
        }

        // Should keep last 5: write, search, edit, deploy, test
        // (note: "write" resets checkpoint counter since it's a checkpoint tool,
        // but the ring buffer still records it)
        assert_eq!(guardian.recent_tools.len(), 5);
        assert_eq!(guardian.recent_tools[4], "test");
    }

    #[test]
    fn test_nudge_display_formatting() {
        let nudge = Nudge::CheckpointReminder {
            tool_calls: 12,
            threshold: 12,
        };
        let text = nudge.to_string();
        assert!(text.contains("📋"));
        assert!(text.contains("12 tool calls"));

        let nudge = Nudge::CheckpointReminder {
            tool_calls: 30,
            threshold: 30,
        };
        let text = nudge.to_string();
        assert!(text.contains("🚨"));

        let nudge = Nudge::MemorySearchReminder {
            turns_since_search: 8,
        };
        let text = nudge.to_string();
        assert!(text.contains("📝"));
        assert!(text.contains("8 turns"));
    }

    #[test]
    fn test_edit_and_write_tools_are_checkpoints() {
        let mut guardian = CognitiveGuardian::new(enabled_config());

        for _ in 0..15 {
            guardian.on_tool_call("exec");
        }
        assert_eq!(guardian.tool_calls_since_checkpoint(), 15);

        // "edit" (OpenClaw file edit) should reset
        guardian.on_tool_call("edit");
        assert_eq!(guardian.tool_calls_since_checkpoint(), 0);

        for _ in 0..15 {
            guardian.on_tool_call("exec");
        }

        // "write" (OpenClaw file write) should reset
        guardian.on_tool_call("write");
        assert_eq!(guardian.tool_calls_since_checkpoint(), 0);
    }

    #[test]
    fn test_memory_search_is_not_a_checkpoint() {
        let mut guardian = CognitiveGuardian::new(enabled_config());

        for _ in 0..15 {
            guardian.on_tool_call("exec");
        }
        assert_eq!(guardian.tool_calls_since_checkpoint(), 15);

        // memory_search should NOT reset the checkpoint counter
        let was_checkpoint = guardian.on_tool_call("memory_search");
        assert!(!was_checkpoint);
        assert_eq!(guardian.tool_calls_since_checkpoint(), 16);
    }

    #[test]
    fn test_checkpoint_thresholds_fire_highest_first() {
        // If we jump straight to 30, only the 30 threshold fires
        // (not 12 and 20 as well — we want the most urgent one).
        let mut guardian = CognitiveGuardian::new(enabled_config());

        for _ in 0..30 {
            guardian.on_tool_call("exec");
        }

        let nudge = guardian.get_nudge();
        assert!(matches!(
            nudge,
            Some(Nudge::CheckpointReminder { threshold: 30, .. })
        ));

        // Next call should give 20 (second highest unfired)
        let nudge = guardian.get_nudge();
        assert!(matches!(
            nudge,
            Some(Nudge::CheckpointReminder { threshold: 20, .. })
        ));

        // Then 12
        let nudge = guardian.get_nudge();
        assert!(matches!(
            nudge,
            Some(Nudge::CheckpointReminder { threshold: 12, .. })
        ));

        // All fired, nothing left
        assert_eq!(guardian.get_nudge(), None);
    }
}
