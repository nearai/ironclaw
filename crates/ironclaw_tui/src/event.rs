//! Unified event type for the TUI event loop.
//!
//! All external inputs (keyboard, terminal resize, engine status updates,
//! agent responses) are funnelled into a single `TuiEvent` enum so the
//! main loop can `select!` on one receiver.

use std::collections::VecDeque;

use ratatui::crossterm::event::KeyEvent;

/// A single log entry displayed in the TUI Logs tab.
///
/// This mirrors `LogEntry` from the main crate but is self-contained
/// so `ironclaw_tui` has no dependency on the main crate.
#[derive(Debug, Clone)]
pub struct TuiLogEntry {
    pub level: String,
    pub target: String,
    pub message: String,
    pub timestamp: String,
}

/// Ring buffer of log entries with a fixed capacity.
#[derive(Debug, Clone)]
pub struct LogRingBuffer {
    entries: VecDeque<TuiLogEntry>,
    capacity: usize,
}

impl LogRingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, entry: TuiLogEntry) {
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &TuiLogEntry> {
        self.entries.iter()
    }
}

/// Events consumed by the TUI run loop.
#[derive(Debug, Clone)]
pub enum TuiEvent {
    /// A keyboard event from crossterm.
    Key(KeyEvent),

    /// Terminal was resized to (cols, rows).
    Resize(u16, u16),

    /// Mouse scroll (delta: negative = up, positive = down).
    MouseScroll(i16),

    /// Periodic render tick (~30 fps).
    Tick,

    /// Agent is thinking / processing.
    Thinking(String),

    /// Tool execution started.
    ToolStarted {
        name: String,
        detail: Option<String>,
    },

    /// Tool execution completed.
    ToolCompleted {
        name: String,
        success: bool,
        error: Option<String>,
    },

    /// Brief preview of tool output.
    ToolResult { name: String, preview: String },

    /// Streaming text chunk from the LLM.
    StreamChunk(String),

    /// General status message.
    Status(String),

    /// Full agent response ready to display.
    Response {
        content: String,
        thread_id: Option<String>,
    },

    /// A sandbox job started.
    JobStarted { job_id: String, title: String },

    /// Tool requires user approval.
    ApprovalNeeded {
        request_id: String,
        tool_name: String,
        description: String,
        parameters: serde_json::Value,
        allow_always: bool,
    },

    /// Extension needs user authentication.
    AuthRequired {
        extension_name: String,
        instructions: Option<String>,
    },

    /// Extension auth completed.
    AuthCompleted {
        extension_name: String,
        success: bool,
        message: String,
    },

    /// Agent reasoning update.
    ReasoningUpdate { narrative: String },

    /// Per-turn token/cost summary.
    TurnCost {
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: String,
    },

    /// Suggestions for follow-up messages.
    Suggestions { suggestions: Vec<String> },

    /// A log entry captured from the tracing subscriber.
    Log {
        level: String,
        target: String,
        message: String,
        timestamp: String,
    },
}
