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

/// A single image or file attachment pasted into the TUI.
#[derive(Debug, Clone)]
pub struct TuiAttachment {
    /// Raw file bytes (e.g. PNG-encoded image).
    pub data: Vec<u8>,
    /// MIME type (e.g. "image/png").
    pub mime_type: String,
    /// Display label shown in the input area (e.g. "Image 1").
    pub label: String,
}

/// A user message with optional attachments, sent from the TUI to the channel bridge.
#[derive(Debug, Clone)]
pub struct TuiUserMessage {
    /// The text content of the message.
    pub text: String,
    /// Pasted image attachments.
    pub attachments: Vec<TuiAttachment>,
    /// Active thread scope for this message, if the TUI has one selected.
    pub thread_id: Option<String>,
    /// Non-chat UI action to run through the bridge.
    pub ui_action: Option<TuiUiAction>,
}

/// Out-of-band UI actions emitted by the TUI.
#[derive(Debug, Clone)]
pub enum TuiUiAction {
    /// Load and show engine thread detail without sending chat text.
    OpenEngineThreadDetail { thread_id: String },
}

impl TuiUserMessage {
    /// Create a text-only message with no attachments.
    pub fn text_only(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            attachments: Vec::new(),
            thread_id: None,
            ui_action: None,
        }
    }

    /// Attach a thread scope to this message.
    pub fn with_thread_id(mut self, thread_id: Option<String>) -> Self {
        self.thread_id = thread_id;
        self
    }

    /// Request detail for an engine thread from the TUI bridge.
    pub fn open_engine_thread_detail(thread_id: impl Into<String>) -> Self {
        Self {
            text: String::new(),
            attachments: Vec::new(),
            thread_id: None,
            ui_action: Some(TuiUiAction::OpenEngineThreadDetail {
                thread_id: thread_id.into(),
            }),
        }
    }
}

/// A past conversation entry for the resume/thread picker.
#[derive(Debug, Clone)]
pub struct ThreadEntry {
    pub id: String,
    pub title: Option<String>,
    pub message_count: i64,
    pub last_activity: String,
    pub channel: String,
}

/// A single message from conversation history, for hydrating the TUI on thread resume.
#[derive(Debug, Clone)]
pub struct HistoryMessage {
    pub role: String,
    pub content: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// A pending approval restored alongside conversation history.
#[derive(Debug, Clone)]
pub struct HistoryApprovalRequest {
    pub request_id: String,
    pub tool_name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub allow_always: bool,
}

/// An engine v2 thread entry for the activity sidebar.
#[derive(Debug, Clone)]
pub struct EngineThreadEntry {
    pub id: String,
    pub goal: String,
    /// "Foreground", "Research", or "Mission".
    pub thread_type: String,
    /// Engine ThreadState as a string (e.g. "Running", "Waiting").
    pub state: String,
    pub step_count: usize,
    pub total_tokens: u64,
    pub created_at: String,
    pub updated_at: String,
}

/// A single message in engine thread detail.
#[derive(Debug, Clone)]
pub struct EngineThreadMessageEntry {
    pub role: String,
    pub content: String,
    pub timestamp: String,
}

/// Full engine thread detail for the sidebar modal.
#[derive(Debug, Clone)]
pub struct EngineThreadDetailEntry {
    pub id: String,
    pub goal: String,
    pub thread_type: String,
    pub state: String,
    pub project_id: String,
    pub parent_id: Option<String>,
    pub step_count: usize,
    pub total_tokens: u64,
    pub created_at: String,
    pub updated_at: String,
    pub max_iterations: usize,
    pub completed_at: Option<String>,
    pub total_cost_usd: f64,
    pub messages: Vec<EngineThreadMessageEntry>,
}

/// Events consumed by the TUI run loop.
#[derive(Debug, Clone)]
pub enum TuiEvent {
    /// A keyboard event from crossterm.
    Key(KeyEvent),

    /// Bracketed paste text from the terminal.
    Paste(String),

    /// Terminal was resized to (cols, rows).
    Resize(u16, u16),

    /// Mouse scroll (delta: negative = up, positive = down).
    MouseScroll(i16),

    /// Left mouse click at a terminal cell coordinate.
    MouseClick { column: u16, row: u16 },

    /// Mouse drag with the left button held.
    MouseDrag { column: u16, row: u16 },

    /// Left mouse button release.
    MouseRelease { column: u16, row: u16 },

    /// Periodic render tick (~30 fps).
    Tick,

    /// Agent is thinking / processing.
    Thinking(String),

    /// Tool execution started.
    ToolStarted {
        name: String,
        detail: Option<String>,
        call_id: Option<String>,
    },

    /// Tool execution completed.
    ToolCompleted {
        name: String,
        success: bool,
        error: Option<String>,
        call_id: Option<String>,
    },

    /// Brief preview of tool output.
    ToolResult {
        name: String,
        preview: String,
        call_id: Option<String>,
    },

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

    /// A sandbox job's status changed.
    JobStatus { job_id: String, status: String },

    /// A sandbox job completed with final result.
    JobResult { job_id: String, status: String },

    /// A routine was created, updated, or deleted.
    RoutineUpdate {
        id: String,
        name: String,
        trigger_type: String,
        enabled: bool,
        last_run: Option<String>,
        next_fire: Option<String>,
    },

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

    /// Context pressure update (token usage warning).
    ContextPressure {
        used_tokens: u64,
        max_tokens: u64,
        percentage: u8,
        warning: Option<String>,
    },

    /// Sandbox / Docker status update.
    SandboxStatus {
        docker_available: bool,
        running_containers: u32,
        status: String,
    },

    /// Secrets vault status update.
    SecretsStatus { count: u32, vault_unlocked: bool },

    /// Cost guard / budget status update.
    CostGuard {
        session_budget_usd: Option<String>,
        spent_usd: String,
        remaining_usd: Option<String>,
        limit_reached: bool,
    },

    /// A log entry captured from the tracing subscriber.
    Log {
        level: String,
        target: String,
        message: String,
        timestamp: String,
    },

    /// Thread list for the interactive resume picker.
    ThreadList { threads: Vec<ThreadEntry> },

    /// Engine v2 thread list update for the activity sidebar.
    EngineThreadList { threads: Vec<EngineThreadEntry> },

    /// Full engine v2 thread detail for the sidebar modal.
    EngineThreadDetail { detail: EngineThreadDetailEntry },

    /// Full conversation history for a resumed thread.
    ConversationHistory {
        thread_id: String,
        messages: Vec<HistoryMessage>,
        pending_approval: Option<HistoryApprovalRequest>,
    },
}
