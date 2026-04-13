//! Widget trait and built-in widget implementations.
//!
//! All TUI panels (header, conversation, sidebar, status bar, input) are
//! widgets that implement [`TuiWidget`]. The trait receives a read-only
//! reference to [`AppState`] for rendering and can optionally handle key
//! events.

pub mod approval;
pub mod codeblock;
pub mod command_palette;
pub mod conversation;
pub mod dashboard;
pub mod header;
pub mod help_overlay;
pub mod input_box;
pub mod logs;
pub mod model_picker;
pub mod plan;
pub mod registry;
pub mod status_bar;
pub mod tab_bar;
pub mod thread_list;
pub mod thread_picker;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::event::LogRingBuffer;
use crate::layout::TuiSlot;
use crate::spinner::{Spinner, SpinnerKind};
use command_palette::CommandPaletteState;
use model_picker::ModelPickerState;

/// Which main content tab is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveTab {
    #[default]
    Conversation,
    Dashboard,
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

    /// Whether the conversation auto-scrolls to follow new content.
    pub pinned_to_bottom: bool,

    /// Maximum valid scroll offset (set during render).
    pub max_scroll_offset: u16,

    /// Last known conversation area height in rows (set during render).
    pub conversation_height: u16,

    /// Currently active tools (name -> started_at).
    pub active_tools: Vec<ToolActivity>,

    /// Recently completed tools.
    pub recent_tools: Vec<ToolActivity>,

    /// Buffered previews for tool results whose ToolStarted was dropped
    /// (broadcast lag or batched CodeAct). Keyed by (name, call_id).
    pub orphaned_previews: std::collections::HashMap<(String, Option<String>), String>,

    /// Active threads (session-level, used by /resume picker).
    pub threads: Vec<ThreadInfo>,

    /// Engine v2 threads for the activity sidebar.
    pub engine_threads: Vec<EngineThreadInfo>,

    /// Currently selected conversation thread for outbound messages.
    pub current_thread_id: Option<String>,

    /// Tracked sandbox jobs.
    pub jobs: Vec<JobInfo>,

    /// Tracked routines.
    pub routines: Vec<RoutineInfo>,

    /// Current thinking/status text.
    pub status_text: String,

    /// Whether a response is currently streaming.
    pub is_streaming: bool,

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

    /// Model picker state, triggered from `/model`.
    pub model_picker: ModelPickerState,

    /// Whether the TUI is waiting for a `/model` response to hydrate the picker.
    pub awaiting_model_list: bool,

    /// Tick counter incremented each TUI tick (used for spinner timing).
    pub tick_count: usize,

    /// Active spinner definition.
    pub spinner: Spinner,

    /// Which spinner variant is selected.
    pub spinner_kind: SpinnerKind,

    /// History of sent messages for up-arrow recall.
    pub input_history: Vec<String>,
    /// Current position in input history (`None` = not browsing).
    pub history_index: Option<usize>,
    /// Saved draft when entering history mode.
    pub history_draft: String,

    /// Suggested follow-up messages.
    pub suggestions: Vec<String>,

    /// Search state for conversation.
    pub search: SearchState,

    /// Context pressure status (derived from token usage).
    pub context_pressure: Option<ContextPressureInfo>,

    /// Sandbox / Docker status.
    pub sandbox_status: Option<SandboxInfo>,

    /// Secrets vault status.
    pub secrets_status: Option<SecretsInfo>,

    /// Cost guard / budget status.
    pub cost_guard: Option<CostGuardInfo>,

    /// Tool categories for the welcome screen.
    pub welcome_tools: Vec<ToolCategory>,

    /// Skill categories for the welcome screen.
    pub welcome_skills: Vec<SkillCategory>,

    /// Workspace directory path (for display on welcome screen).
    pub workspace_path: String,

    /// Number of memory entries in the workspace.
    pub memory_count: usize,

    /// Identity files loaded at startup (e.g. "AGENTS.md", "SOUL.md").
    pub identity_files: Vec<String>,

    /// Whether the help overlay (F1) is visible.
    pub help_visible: bool,

    /// Log level filter for the Logs tab.
    pub log_level_filter: LogLevelFilter,

    /// Active notification toasts.
    pub toasts: Vec<Toast>,

    /// Tool detail modal (Ctrl+E).
    pub tool_detail_modal: Option<ToolDetailModal>,

    /// Expanded dashboard panel modal (click panel title to expand).
    pub expanded_dashboard_panel: Option<DashboardPanelModal>,

    /// Identity file viewer modal.
    pub identity_file_modal: Option<IdentityFileModal>,

    /// Identity file contents loaded at startup: (filename, content).
    pub identity_file_contents: Vec<(String, String)>,

    /// Memory entries for the Learnings panel.
    pub memory_entries: Vec<MemoryEntry>,

    /// Images pasted via Ctrl+V, pending submission with the next message.
    pub pending_attachments: Vec<crate::event::TuiAttachment>,

    /// Pending thread picker (from /resume).
    pub pending_thread_picker: Option<ThreadPickerState>,

    /// Last rendered terminal snapshot for text selection and copy.
    pub screen_snapshot: ScreenSnapshot,

    /// Active text selection in the rendered TUI.
    pub text_selection: Option<TextSelection>,

    /// Skills activated this session (deduplicated names).
    pub activated_skills: Vec<String>,

    /// Dashboard introspection data.
    pub dashboard: DashboardData,

    /// Animation frame counter for welcome screen gradient reveal.
    pub welcome_reveal_frame: u16,

    /// Active plan state (inline checklist display).
    pub plan_state: Option<PlanState>,
}

/// Data for the Dashboard introspection tab.
#[derive(Debug, Clone, Default)]
pub struct DashboardData {
    /// Memory usage: (current_bytes, max_bytes).
    pub memory_capacity: Option<(u64, u64)>,
    /// Number of memory entries by category: (category_name, count).
    pub memory_categories: Vec<(String, usize)>,
    /// Total memory entry count.
    pub total_memories: usize,
    /// Session history: (session_label, message_count, tool_calls, tokens).
    pub session_history: Vec<SessionSummary>,
    /// Token usage over time as sparkline data points (most recent last).
    pub token_sparkline: Vec<u64>,
    /// Tool usage frequency: (tool_name, call_count).
    pub tool_frequency: Vec<(String, usize)>,
    /// Skills summary: (total, custom_count, builtin_count).
    pub skills_summary: Option<(usize, usize, usize)>,
    /// Identity files loaded.
    pub identity_files: Vec<String>,
    /// Routine statuses for the routine monitor panel.
    pub routines: Vec<RoutineInfo>,
}

/// Summary of a past session for the dashboard.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub label: String,
    pub message_count: usize,
    pub tool_calls: usize,
    pub tokens: u64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// State for the thread resume picker modal.
#[derive(Debug, Clone)]
pub struct ThreadPickerState {
    /// Available threads to resume.
    pub threads: Vec<crate::event::ThreadEntry>,
    /// Currently selected index.
    pub selected: usize,
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
            pinned_to_bottom: true,
            max_scroll_offset: 0,
            conversation_height: 0,
            active_tools: Vec::new(),
            recent_tools: Vec::new(),
            orphaned_previews: std::collections::HashMap::new(),
            threads: Vec::new(),
            engine_threads: Vec::new(),
            current_thread_id: None,
            jobs: Vec::new(),
            routines: Vec::new(),
            status_text: String::new(),
            is_streaming: false,
            pending_approval: None,
            should_quit: false,
            active_tab: ActiveTab::default(),
            log_entries: LogRingBuffer::new(500),
            log_scroll: 0,
            context_window: 128_000,
            command_palette: CommandPaletteState::default(),
            model_picker: ModelPickerState::default(),
            awaiting_model_list: false,
            tick_count: 0,
            spinner: SpinnerKind::default().definition(),
            spinner_kind: SpinnerKind::default(),
            input_history: Vec::new(),
            history_index: None,
            history_draft: String::new(),
            suggestions: Vec::new(),
            search: SearchState::default(),
            context_pressure: None,
            sandbox_status: None,
            secrets_status: None,
            cost_guard: None,
            welcome_tools: Vec::new(),
            welcome_skills: Vec::new(),
            workspace_path: String::new(),
            memory_count: 0,
            identity_files: Vec::new(),
            help_visible: false,
            log_level_filter: LogLevelFilter::default(),
            toasts: Vec::new(),
            tool_detail_modal: None,
            expanded_dashboard_panel: None,
            identity_file_modal: None,
            identity_file_contents: Vec::new(),
            memory_entries: Vec::new(),
            pending_attachments: Vec::new(),
            pending_thread_picker: None,
            screen_snapshot: ScreenSnapshot::default(),
            text_selection: None,
            activated_skills: Vec::new(),
            dashboard: DashboardData::default(),
            welcome_reveal_frame: 0,
            plan_state: None,
        }
    }
}

/// Last rendered terminal contents.
#[derive(Debug, Clone)]
pub struct ScreenSnapshot {
    pub area: Rect,
    pub buffer: Buffer,
}

impl Default for ScreenSnapshot {
    fn default() -> Self {
        let area = Rect::default();
        Self {
            area,
            buffer: Buffer::empty(area),
        }
    }
}

/// A single terminal-cell coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionPoint {
    pub column: u16,
    pub row: u16,
}

/// Active text selection bounds and endpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSelection {
    pub anchor: SelectionPoint,
    pub focus: SelectionPoint,
    pub bounds: Rect,
}

/// A message in the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Per-turn cost summary (if available).
    pub cost_summary: Option<TurnCostSummary>,
}

/// Per-turn token usage and cost information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnCostSummary {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: String,
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
    pub call_id: Option<String>,
    pub name: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub duration_ms: Option<u64>,
    pub status: ToolStatus,
    /// Short contextual summary (e.g., URL, path, query).
    pub detail: Option<String>,
    /// Brief preview of the tool's output.
    pub result_preview: Option<String>,
    /// Whether the inline code block is expanded (default: false).
    pub expanded: bool,
}

/// Tool execution status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Success,
    Failed,
}

/// Status of a sandbox job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JobStatus {
    #[default]
    Pending,
    Running,
    Completed,
    Failed,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "done"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// A sandbox job tracked in the sidebar.
#[derive(Debug, Clone)]
pub struct JobInfo {
    pub id: String,
    pub title: String,
    pub status: JobStatus,
    pub started_at: chrono::DateTime<chrono::Utc>,
}

/// A routine tracked in the sidebar.
#[derive(Debug, Clone)]
pub struct RoutineInfo {
    pub id: String,
    pub name: String,
    pub trigger_type: String,
    pub enabled: bool,
    pub last_run: Option<String>,
    pub next_fire: Option<String>,
}

/// Execution status of a thread or job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThreadStatus {
    /// Currently executing work.
    #[default]
    Active,
    /// Alive but waiting for input or a timer.
    Idle,
    /// Finished successfully.
    Completed,
    /// Terminated with an error.
    Failed,
}

impl std::fmt::Display for ThreadStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Idle => write!(f, "idle"),
            Self::Completed => write!(f, "done"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// Thread information for the sidebar.
#[derive(Debug, Clone)]
pub struct ThreadInfo {
    pub id: String,
    pub label: String,
    pub is_foreground: bool,
    pub is_running: bool,
    pub duration_secs: u64,
    /// Richer status indicator.
    pub status: ThreadStatus,
    /// When the thread was created / started.
    pub started_at: chrono::DateTime<chrono::Utc>,
}

/// Engine v2 thread information for the activity sidebar.
#[derive(Debug, Clone)]
pub struct EngineThreadInfo {
    pub id: String,
    pub goal: String,
    /// "Foreground", "Research", or "Mission".
    pub thread_type: String,
    pub status: ThreadStatus,
    pub step_count: usize,
    pub total_tokens: u64,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Context pressure status for the status bar.
#[derive(Debug, Clone)]
pub struct ContextPressureInfo {
    /// Tokens consumed so far.
    pub used_tokens: u64,
    /// Maximum context window.
    pub max_tokens: u64,
    /// Usage percentage (0–100).
    pub percentage: u8,
    /// Warning message when pressure is high.
    pub warning: Option<String>,
}

/// Sandbox / Docker status for the sidebar.
#[derive(Debug, Clone)]
pub struct SandboxInfo {
    /// Whether the Docker daemon is reachable.
    pub docker_available: bool,
    /// Number of currently running containers.
    pub running_containers: u32,
    /// Human-readable status summary.
    pub status: String,
}

/// Secrets vault status for the sidebar.
#[derive(Debug, Clone)]
pub struct SecretsInfo {
    /// Number of stored secrets.
    pub count: u32,
    /// Whether the vault is currently unlocked.
    pub vault_unlocked: bool,
}

/// A group of tools under a shared category for the welcome screen.
#[derive(Debug, Clone, Default)]
pub struct ToolCategory {
    /// Category name (e.g. "memory", "file", "browser").
    pub name: String,
    /// Tool names in this category.
    pub tools: Vec<String>,
}

/// A group of skills under a shared category for the welcome screen.
#[derive(Debug, Clone, Default)]
pub struct SkillCategory {
    /// Category name (e.g. "apple", "creative", "research").
    pub name: String,
    /// Skill names in this category.
    pub skills: Vec<String>,
}

/// Cost guard / budget information for the status bar.
#[derive(Debug, Clone)]
pub struct CostGuardInfo {
    /// Session spending budget (USD), if configured.
    pub session_budget_usd: Option<String>,
    /// Amount spent so far (USD).
    pub spent_usd: String,
    /// Amount remaining (USD), if budget is set.
    pub remaining_usd: Option<String>,
    /// Whether the spending limit has been reached.
    pub limit_reached: bool,
}

/// Log level filter for the Logs tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogLevelFilter {
    /// Show all log entries.
    #[default]
    All,
    /// Show only ERROR entries.
    Error,
    /// Show ERROR and WARN entries.
    Warn,
    /// Show ERROR, WARN, and INFO entries.
    Info,
    /// Show everything except TRACE.
    Debug,
}

impl std::fmt::Display for LogLevelFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::All => write!(f, "ALL"),
            Self::Error => write!(f, "ERROR"),
            Self::Warn => write!(f, "WARN+"),
            Self::Info => write!(f, "INFO+"),
            Self::Debug => write!(f, "DEBUG+"),
        }
    }
}

impl LogLevelFilter {
    /// Returns true if the given log level passes this filter.
    pub fn accepts(&self, level: &str) -> bool {
        match self {
            Self::All => true,
            Self::Error => level == "ERROR",
            Self::Warn => matches!(level, "ERROR" | "WARN"),
            Self::Info => matches!(level, "ERROR" | "WARN" | "INFO"),
            Self::Debug => matches!(level, "ERROR" | "WARN" | "INFO" | "DEBUG"),
        }
    }
}

/// A notification toast displayed briefly in the bottom-right corner.
#[derive(Debug, Clone)]
pub struct Toast {
    /// Short message to display.
    pub message: String,
    /// Visual style of the toast.
    pub kind: ToastKind,
    /// When the toast was created (for auto-dismiss).
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Visual style for notification toasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

/// Which dashboard panel is expanded (if any).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashboardPanel {
    TokenUsage,
    TopTools,
    System,
    Workspace,
    Jobs,
    Threads,
    Skills,
    Learnings,
    Missions,
}

/// State for an expanded dashboard panel modal.
#[derive(Debug, Clone)]
pub struct DashboardPanelModal {
    pub panel: DashboardPanel,
    pub scroll: u16,
}

/// Modal for viewing an identity file's content.
#[derive(Debug, Clone)]
pub struct IdentityFileModal {
    /// File name (e.g. "AGENTS.md").
    pub name: String,
    /// File content.
    pub content: String,
    /// Scroll offset within the modal.
    pub scroll: u16,
}

/// A single memory entry for the Learnings panel.
#[derive(Debug, Clone)]
pub struct MemoryEntry {
    /// Document path (e.g. "daily/2025-05-01.md").
    pub path: String,
    /// First ~100 chars of content for preview.
    pub snippet: String,
    /// Last updated timestamp.
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Modal showing full tool output, scrollable.
#[derive(Debug, Clone)]
pub struct ToolDetailModal {
    /// Name of the tool whose output is shown.
    pub tool_name: String,
    /// Full tool output content.
    pub content: String,
    /// Scroll offset within the modal.
    pub scroll: u16,
}

/// Status of an individual plan step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

impl PlanStepStatus {
    /// Parse from a status string (case-insensitive).
    pub fn parse_status(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "in_progress" | "inprogress" | "running" => Self::InProgress,
            "completed" | "done" => Self::Completed,
            "failed" | "error" => Self::Failed,
            _ => Self::Pending,
        }
    }

    /// Icon character for this status.
    pub fn icon(self) -> &'static str {
        match self {
            Self::Pending => "\u{25CB}",    // ○
            Self::InProgress => "\u{25D0}", // ◐ (base; caller animates)
            Self::Completed => "\u{25CF}",  // ●
            Self::Failed => "\u{2715}",     // ✕
        }
    }
}

/// Overall plan status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStatus {
    Draft,
    Approved,
    Executing,
    Completed,
    Failed,
}

impl PlanStatus {
    /// Parse from a status string (case-insensitive).
    pub fn parse_status(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "approved" => Self::Approved,
            "executing" | "running" | "in_progress" => Self::Executing,
            "completed" | "done" => Self::Completed,
            "failed" | "error" => Self::Failed,
            _ => Self::Draft,
        }
    }

    /// Human-readable label for the status badge.
    pub fn label(self) -> &'static str {
        match self {
            Self::Draft => "DRAFT",
            Self::Approved => "APPROVED",
            Self::Executing => "EXECUTING",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
        }
    }

    /// Whether this is a terminal state (no more updates expected).
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

/// A single step in a plan.
#[derive(Debug, Clone)]
pub struct PlanStep {
    pub index: usize,
    pub title: String,
    pub status: PlanStepStatus,
    pub result: Option<String>,
}

/// Active plan tracked by the TUI.
#[derive(Debug, Clone)]
pub struct PlanState {
    pub plan_id: String,
    pub title: String,
    pub status: PlanStatus,
    pub steps: Vec<PlanStep>,
    pub mission_id: Option<String>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl PlanState {
    /// Number of completed steps.
    pub fn completed_count(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| s.status == PlanStepStatus::Completed)
            .count()
    }

    /// Whether the plan is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }
}

/// Search state for Ctrl+F in conversation.
#[derive(Debug, Clone, Default)]
pub struct SearchState {
    /// Whether search mode is active.
    pub active: bool,
    /// Current search query.
    pub query: String,
    /// Total number of matches found.
    pub match_count: usize,
    /// Current match index (0-based).
    pub current_match: usize,
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
