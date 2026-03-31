//! Widget trait and built-in widget implementations.
//!
//! All TUI panels (header, conversation, sidebar, status bar, input) are
//! widgets that implement [`TuiWidget`]. The trait receives a read-only
//! reference to [`AppState`] for rendering and can optionally handle key
//! events.

pub mod approval;
pub mod command_palette;
pub mod conversation;
pub mod header;
pub mod input_box;
pub mod logs;
pub mod registry;
pub mod status_bar;
pub mod thread_list;
pub mod tool_panel;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::event::LogRingBuffer;
use crate::layout::TuiSlot;
use command_palette::CommandPaletteState;

/// Which main content tab is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveTab {
    #[default]
    Conversation,
    Logs,
}

/// Shared application state visible to all widgets.
#[derive(Debug, Clone)]
pub struct AppState {
    /// IronClaw version string.
    pub version: String,

    /// Active LLM model name.
    pub model: String,

    /// Session start time.
    pub session_start: chrono::DateTime<chrono::Utc>,

    /// Cumulative input tokens this session.
    pub total_input_tokens: u64,

    /// Cumulative output tokens this session.
    pub total_output_tokens: u64,

    /// Cumulative cost (USD) this session.
    pub total_cost_usd: String,

    /// Conversation messages.
    pub messages: Vec<ChatMessage>,

    /// Scroll offset in the conversation (0 = bottom / most recent).
    pub scroll_offset: u16,

    /// Currently active tools (name -> started_at).
    pub active_tools: Vec<ToolActivity>,

    /// Recently completed tools.
    pub recent_tools: Vec<ToolActivity>,

    /// Active threads.
    pub threads: Vec<ThreadInfo>,

    /// Current thinking/status text.
    pub status_text: String,

    /// Whether a response is currently streaming.
    pub is_streaming: bool,

    /// Whether the sidebar is visible.
    pub sidebar_visible: bool,

    /// Pending approval request (if any).
    pub pending_approval: Option<ApprovalRequest>,

    /// Whether the TUI should quit.
    pub should_quit: bool,

    /// Currently active main content tab.
    pub active_tab: ActiveTab,

    /// Ring buffer of captured log entries.
    pub log_entries: LogRingBuffer,

    /// Scroll offset in the logs view (0 = bottom / most recent).
    pub log_scroll: u16,

    /// Maximum context window size in tokens for the active model.
    pub context_window: u64,

    /// Command palette state.
    pub command_palette: CommandPaletteState,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            version: String::new(),
            model: String::new(),
            session_start: chrono::Utc::now(),
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cost_usd: "$0.00".to_string(),
            messages: Vec::new(),
            scroll_offset: 0,
            active_tools: Vec::new(),
            recent_tools: Vec::new(),
            threads: Vec::new(),
            status_text: String::new(),
            is_streaming: false,
            sidebar_visible: true,
            pending_approval: None,
            should_quit: false,
            active_tab: ActiveTab::default(),
            log_entries: LogRingBuffer::new(500),
            log_scroll: 0,
            context_window: 128_000,
            command_palette: CommandPaletteState::default(),
        }
    }
}

/// A message in the conversation.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Who sent the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

/// Tool execution activity for the sidebar.
#[derive(Debug, Clone)]
pub struct ToolActivity {
    pub name: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub duration_ms: Option<u64>,
    pub status: ToolStatus,
    /// Short contextual summary (e.g., URL, path, query).
    pub detail: Option<String>,
    /// Brief preview of the tool's output.
    pub result_preview: Option<String>,
}

/// Tool execution status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Success,
    Failed,
}

/// Thread information for the sidebar.
#[derive(Debug, Clone)]
pub struct ThreadInfo {
    pub id: String,
    pub label: String,
    pub is_foreground: bool,
    pub is_running: bool,
    pub duration_secs: u64,
}

/// Pending approval request.
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub request_id: String,
    pub tool_name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub allow_always: bool,
    /// Currently selected option index (0=approve, 1=always, 2=deny).
    pub selected: usize,
}

/// Trait implemented by all TUI widgets.
pub trait TuiWidget: Send + Sync {
    /// Unique widget identifier.
    fn id(&self) -> &str;

    /// Which layout slot this widget occupies.
    fn slot(&self) -> TuiSlot;

    /// Render the widget into the given area.
    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState);

    /// Handle a key event. Returns `true` if the event was consumed.
    fn handle_key(
        &mut self,
        _key: ratatui::crossterm::event::KeyEvent,
        _state: &mut AppState,
    ) -> bool {
        false
    }

    /// Called on each tick for animations or time-based updates.
    fn tick(&mut self, _state: &AppState) {}
}
