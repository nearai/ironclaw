//! TuiApp: main event loop, frame rendering, and input dispatch.
//!
//! The TUI runs in a dedicated blocking thread (crossterm needs raw mode
//! control of stdin). It communicates with the agent via channels:
//!
//! - `event_rx`: receives [`TuiEvent`]s (key input, status updates, responses)
//! - `msg_tx`: sends user messages to the agent loop
//!
//! The app owns the terminal, manages alternate screen / raw mode, and
//! renders frames at ~30fps using a tick timer.

use std::io;
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event as CtEvent, KeyCode, KeyEventKind,
    KeyModifiers, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use tokio::sync::mpsc;

use crate::event::{TuiAttachment, TuiEvent, TuiLogEntry, TuiUserMessage};
use crate::input::{InputAction, map_key};
use crate::layout::TuiLayout;
use crate::widgets::approval::{ApprovalAction, ApprovalWidget};
use crate::widgets::command_palette::CommandPaletteWidget;
use crate::widgets::help_overlay::HelpOverlayWidget;
use crate::widgets::logs::LogsWidget;
use crate::widgets::registry::{BuiltinWidgets, create_default_widgets};
use crate::widgets::{
    ActiveTab, AppState, ChatMessage, ContextPressureInfo, CostGuardInfo, JobInfo, JobStatus,
    MessageRole, RoutineInfo, SandboxInfo, SecretsInfo, SkillCategory, ThreadInfo, ThreadStatus,
    Toast, ToastKind, ToolActivity, ToolCategory, ToolDetailModal, ToolStatus, TuiWidget,
    TurnCostSummary,
};

/// Handle returned when the TUI is started. The main crate uses this to
/// send events and receive user messages.
pub struct TuiAppHandle {
    /// Send events (status updates, responses) into the TUI.
    pub event_tx: mpsc::Sender<TuiEvent>,
    /// Receive user messages from the TUI input.
    pub msg_rx: mpsc::Receiver<TuiUserMessage>,
    /// Join handle for the TUI thread.
    pub join_handle: std::thread::JoinHandle<()>,
}

/// Configuration for creating a TuiApp.
pub struct TuiAppConfig {
    pub version: String,
    pub model: String,
    pub layout: TuiLayout,
    /// Maximum context window size in tokens (e.g., 128_000, 200_000).
    pub context_window: u64,
    /// Tool categories for the welcome screen.
    pub tools: Vec<ToolCategory>,
    /// Skill categories for the welcome screen.
    pub skills: Vec<SkillCategory>,
    /// Workspace directory path.
    pub workspace_path: String,
    /// Number of memory entries in the workspace.
    pub memory_count: usize,
    /// Identity files loaded at startup (e.g. "AGENTS.md", "SOUL.md").
    pub identity_files: Vec<String>,
}

/// Start the TUI application. Returns a handle for bi-directional communication.
///
/// The TUI runs in a dedicated OS thread because crossterm raw mode requires
/// exclusive stdin access.
pub fn start_tui(config: TuiAppConfig) -> TuiAppHandle {
    let (event_tx, event_rx) = mpsc::channel::<TuiEvent>(256);
    let (msg_tx, msg_rx) = mpsc::channel::<TuiUserMessage>(32);

    // Clone event_tx for the crossterm polling task
    let input_event_tx = event_tx.clone();

    let join_handle = std::thread::spawn(move || {
        // Build a single-threaded tokio runtime for the TUI thread
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                tracing::error!("Failed to build tokio runtime for TUI: {e}");
                return;
            }
        };

        rt.block_on(async move {
            if let Err(e) = run_tui(config, event_rx, input_event_tx, msg_tx).await {
                tracing::error!("TUI error: {}", e);
            }
        });
    });

    TuiAppHandle {
        event_tx,
        msg_rx,
        join_handle,
    }
}

/// Internal TUI run loop.
async fn run_tui(
    config: TuiAppConfig,
    mut event_rx: mpsc::Receiver<TuiEvent>,
    input_event_tx: mpsc::Sender<TuiEvent>,
    msg_tx: mpsc::Sender<TuiUserMessage>,
) -> io::Result<()> {
    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        ratatui::crossterm::event::EnableMouseCapture,
        EnableBracketedPaste
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // State
    let mut state = AppState {
        version: config.version,
        model: config.model,
        sidebar_visible: config.layout.sidebar.visible,
        context_window: config.context_window,
        welcome_tools: config.tools,
        welcome_skills: config.skills,
        workspace_path: config.workspace_path,
        memory_count: config.memory_count,
        identity_files: config.identity_files,
        ..AppState::default()
    };

    let mut widgets = create_default_widgets(&config.layout);
    let layout = config.layout;

    // Spawn crossterm input poller
    let poll_tx = input_event_tx;
    tokio::spawn(async move {
        loop {
            // Poll crossterm events with a short timeout
            match tokio::task::spawn_blocking(|| {
                if event::poll(Duration::from_millis(33)).unwrap_or(false) {
                    event::read().ok()
                } else {
                    None
                }
            })
            .await
            {
                Ok(Some(CtEvent::Key(key))) => {
                    if key.kind == KeyEventKind::Press
                        && poll_tx.send(TuiEvent::Key(key)).await.is_err()
                    {
                        break;
                    }
                }
                Ok(Some(CtEvent::Resize(w, h))) => {
                    if poll_tx.send(TuiEvent::Resize(w, h)).await.is_err() {
                        break;
                    }
                }
                Ok(Some(CtEvent::Mouse(MouseEvent {
                    kind: MouseEventKind::ScrollUp,
                    ..
                }))) => {
                    if poll_tx.send(TuiEvent::MouseScroll(-3)).await.is_err() {
                        break;
                    }
                }
                Ok(Some(CtEvent::Mouse(MouseEvent {
                    kind: MouseEventKind::ScrollDown,
                    ..
                }))) => {
                    if poll_tx.send(TuiEvent::MouseScroll(3)).await.is_err() {
                        break;
                    }
                }
                Ok(Some(CtEvent::Paste(text))) => {
                    if poll_tx.send(TuiEvent::Paste(text)).await.is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    let mut tick_interval = tokio::time::interval(Duration::from_millis(33));

    // Main loop
    loop {
        // Render
        terminal.draw(|frame| {
            render_frame(frame, &mut state, &widgets, &layout);
        })?;

        // Wait for event
        tokio::select! {
            _ = tick_interval.tick() => {
                // Tick — just triggers a re-render
            }
            event = event_rx.recv() => {
                let Some(event) = event else {
                    break; // Channel closed
                };
                handle_event(event, &mut state, &mut widgets, &msg_tx).await;
            }
        }

        if state.should_quit {
            break;
        }
    }

    // Teardown
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        ratatui::crossterm::event::DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

/// Count the number of case-insensitive matches of `query` across all messages.
fn count_search_matches(messages: &[ChatMessage], query: &str) -> usize {
    if query.is_empty() {
        return 0;
    }
    let query_lower = query.to_lowercase();
    messages
        .iter()
        .map(|m| {
            let content_lower = m.content.to_lowercase();
            content_lower.matches(&query_lower).count()
        })
        .sum()
}

/// Handle a single TUI event.
async fn handle_event(
    event: TuiEvent,
    state: &mut AppState,
    widgets: &mut BuiltinWidgets,
    msg_tx: &mpsc::Sender<TuiUserMessage>,
) {
    match event {
        TuiEvent::Paste(text) => {
            let approval_active = state.pending_approval.is_some();
            let help_active = state.help_visible;
            let tool_detail_active = state.tool_detail_modal.is_some();

            if !approval_active && !help_active && !tool_detail_active {
                widgets.input_box.insert_text(&text);

                if state.history_index.is_some() {
                    state.history_index = None;
                    state.history_draft = widgets.input_box.current_text();
                }

                if !state.command_palette.visible {
                    if let Some(cmd) =
                        crate::input::parse_slash_command(&widgets.input_box.current_text())
                    {
                        state.command_palette.open(cmd);
                    }
                } else {
                    let input = widgets.input_box.current_text();
                    if input.starts_with('/') {
                        state.command_palette.open(input.trim());
                    } else {
                        state.command_palette.close();
                    }
                }

                if state.search.active {
                    state.search.query = widgets.input_box.current_text();
                    state.search.match_count =
                        count_search_matches(&state.messages, &state.search.query);
                    state.search.current_match = 0;
                }
            }
        }
        TuiEvent::Key(key) => {
            let approval_active = state.pending_approval.is_some();
            let palette_active = state.command_palette.visible;
            let search_active = state.search.active;
            let help_active = state.help_visible;
            let tool_detail_active = state.tool_detail_modal.is_some();
            let logs_active = state.active_tab == ActiveTab::Logs;
            let action = map_key(
                key,
                approval_active,
                palette_active,
                search_active,
                help_active,
                tool_detail_active,
                logs_active,
            );

            match action {
                InputAction::Submit => {
                    // Close palette if open
                    state.command_palette.close();
                    let text = widgets.input_box.take_input();
                    let trimmed = text.trim().to_string();
                    let attachments = std::mem::take(&mut state.pending_attachments);
                    if !trimmed.is_empty() || !attachments.is_empty() {
                        // Push to input history
                        if !trimmed.is_empty() {
                            state.input_history.push(trimmed.clone());
                        }
                        state.history_index = None;
                        state.history_draft.clear();
                        // Clear follow-up suggestions from previous turn
                        state.suggestions.clear();
                        // Build display content with attachment labels
                        let display_content = if attachments.is_empty() {
                            trimmed.clone()
                        } else {
                            let labels: Vec<&str> =
                                attachments.iter().map(|a| a.label.as_str()).collect();
                            if trimmed.is_empty() {
                                format!("[{}]", labels.join("] ["))
                            } else {
                                format!("{trimmed} [{}]", labels.join("] ["))
                            }
                        };
                        // Add user message to conversation
                        state.messages.push(ChatMessage {
                            role: MessageRole::User,
                            content: display_content,
                            timestamp: chrono::Utc::now(),
                            cost_summary: None,
                        });
                        state.scroll_offset = 0;
                        // Send to agent
                        let _ = msg_tx
                            .send(TuiUserMessage {
                                text: trimmed,
                                attachments,
                            })
                            .await;
                    }
                }
                InputAction::Quit => {
                    let _ = msg_tx.send(TuiUserMessage::text_only("/quit")).await;
                    state.should_quit = true;
                }
                InputAction::ToggleSidebar => {
                    state.sidebar_visible = !state.sidebar_visible;
                }
                InputAction::ToggleLogs => {
                    state.active_tab = match state.active_tab {
                        ActiveTab::Conversation => ActiveTab::Logs,
                        ActiveTab::Logs => ActiveTab::Conversation,
                    };
                }
                InputAction::ScrollUp => match state.active_tab {
                    ActiveTab::Conversation => {
                        widgets.conversation.scroll(state, -5);
                    }
                    ActiveTab::Logs => {
                        LogsWidget::scroll(state, -5);
                    }
                },
                InputAction::ScrollDown => match state.active_tab {
                    ActiveTab::Conversation => {
                        widgets.conversation.scroll(state, 5);
                    }
                    ActiveTab::Logs => {
                        LogsWidget::scroll(state, 5);
                    }
                },
                InputAction::Interrupt => {
                    let _ = msg_tx.send(TuiUserMessage::text_only("/interrupt")).await;
                    state.status_text.clear();
                }
                InputAction::ApprovalUp => {
                    if let Some(ref mut ap) = state.pending_approval {
                        let count = ApprovalWidget::options(ap.allow_always).len();
                        ap.selected = if ap.selected == 0 {
                            count - 1
                        } else {
                            ap.selected - 1
                        };
                    }
                }
                InputAction::ApprovalDown => {
                    if let Some(ref mut ap) = state.pending_approval {
                        let count = ApprovalWidget::options(ap.allow_always).len();
                        ap.selected = (ap.selected + 1) % count;
                    }
                }
                InputAction::ApprovalConfirm => {
                    if let Some(ref ap) = state.pending_approval {
                        let options = ApprovalWidget::options(ap.allow_always);
                        let action = options
                            .get(ap.selected)
                            .copied()
                            .unwrap_or(ApprovalAction::Deny);
                        let _ = msg_tx
                            .send(TuiUserMessage::text_only(action.as_response()))
                            .await;
                        state.pending_approval = None;
                    }
                }
                InputAction::ApprovalCancel => {
                    if state.pending_approval.is_some() {
                        let _ = msg_tx.send(TuiUserMessage::text_only("n")).await;
                        state.pending_approval = None;
                    }
                }
                InputAction::QuickApprove => {
                    if state.pending_approval.is_some() {
                        let _ = msg_tx.send(TuiUserMessage::text_only("y")).await;
                        state.pending_approval = None;
                    }
                }
                InputAction::QuickAlways => {
                    if let Some(ref ap) = state.pending_approval {
                        if ap.allow_always {
                            let _ = msg_tx.send(TuiUserMessage::text_only("a")).await;
                        } else {
                            let _ = msg_tx.send(TuiUserMessage::text_only("y")).await;
                        }
                        state.pending_approval = None;
                    }
                }
                InputAction::QuickDeny => {
                    if state.pending_approval.is_some() {
                        let _ = msg_tx.send(TuiUserMessage::text_only("n")).await;
                        state.pending_approval = None;
                    }
                }
                InputAction::PaletteUp => {
                    state.command_palette.move_up();
                }
                InputAction::PaletteDown => {
                    state.command_palette.move_down();
                }
                InputAction::PaletteSelect => {
                    if let Some(cmd) = state.command_palette.selected_command() {
                        let text = format!("{cmd} ");
                        widgets.input_box.set_text(&text);
                    }
                    state.command_palette.close();
                }
                InputAction::PaletteClose => {
                    state.command_palette.close();
                }
                InputAction::SearchToggle => {
                    state.search.active = !state.search.active;
                    if !state.search.active {
                        state.search.query.clear();
                        state.search.match_count = 0;
                        state.search.current_match = 0;
                    }
                }
                InputAction::SearchNext => {
                    if state.search.match_count > 0 {
                        state.search.current_match =
                            (state.search.current_match + 1) % state.search.match_count;
                    }
                }
                InputAction::SearchPrev => {
                    if state.search.match_count > 0 {
                        state.search.current_match = if state.search.current_match == 0 {
                            state.search.match_count - 1
                        } else {
                            state.search.current_match - 1
                        };
                    }
                }
                InputAction::HistoryUp => {
                    if !state.input_history.is_empty() {
                        let new_idx = match state.history_index {
                            None => {
                                // Save current draft, start from most recent
                                state.history_draft = widgets.input_box.current_text();
                                state.input_history.len() - 1
                            }
                            Some(idx) => idx.saturating_sub(1),
                        };
                        state.history_index = Some(new_idx);
                        if let Some(text) = state.input_history.get(new_idx) {
                            widgets.input_box.set_text(text);
                        }
                    }
                }
                InputAction::HistoryDown => {
                    if let Some(idx) = state.history_index {
                        if idx + 1 >= state.input_history.len() {
                            // Back to draft
                            state.history_index = None;
                            let draft = state.history_draft.clone();
                            widgets.input_box.set_text(&draft);
                        } else {
                            let new_idx = idx + 1;
                            state.history_index = Some(new_idx);
                            if let Some(text) = state.input_history.get(new_idx) {
                                widgets.input_box.set_text(text);
                            }
                        }
                    }
                }
                InputAction::ToggleHelp => {
                    state.help_visible = !state.help_visible;
                }
                InputAction::ExpandTool => {
                    // Show the most recent tool with a result preview
                    if let Some(tool) = state
                        .recent_tools
                        .iter()
                        .rev()
                        .find(|t| t.result_preview.is_some())
                    {
                        state.tool_detail_modal = Some(ToolDetailModal {
                            tool_name: tool.name.clone(),
                            content: tool.result_preview.clone().unwrap_or_default(),
                            scroll: 0,
                        });
                    }
                }
                InputAction::ToolDetailClose => {
                    state.tool_detail_modal = None;
                }
                InputAction::ToolDetailScrollUp => {
                    if let Some(ref mut modal) = state.tool_detail_modal {
                        modal.scroll = modal.scroll.saturating_add(5);
                    }
                }
                InputAction::ToolDetailScrollDown => {
                    if let Some(ref mut modal) = state.tool_detail_modal {
                        modal.scroll = modal.scroll.saturating_sub(5);
                    }
                }
                InputAction::LogFilter(level) => {
                    state.log_level_filter = level;
                }
                InputAction::ClipboardPaste => {
                    if let Some(attachment) = try_paste_clipboard_image(state) {
                        state.toasts.push(Toast {
                            message: format!("Pasted: {}", attachment.label),
                            kind: ToastKind::Info,
                            created_at: chrono::Utc::now(),
                        });
                        state.pending_attachments.push(attachment);
                    }
                }
                InputAction::Forward => {
                    if state.search.active {
                        // Update the search query with the key event
                        match (key.code, key.modifiers) {
                            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                                state.search.query.push(c);
                            }
                            (KeyCode::Backspace, _) => {
                                state.search.query.pop();
                            }
                            _ => {}
                        }
                        // Recount matches
                        state.search.match_count =
                            count_search_matches(&state.messages, &state.search.query);
                        // Clamp current_match
                        if state.search.match_count == 0 {
                            state.search.current_match = 0;
                        } else if state.search.current_match >= state.search.match_count {
                            state.search.current_match = state.search.match_count - 1;
                        }
                    } else {
                        widgets.input_box.handle_key(key, state);
                        // Update command palette visibility based on input content
                        update_palette_from_input(&widgets.input_box, state);
                    }
                }
            }
        }

        TuiEvent::MouseScroll(delta) => match state.active_tab {
            ActiveTab::Conversation => {
                widgets.conversation.scroll(state, delta);
            }
            ActiveTab::Logs => {
                LogsWidget::scroll(state, delta);
            }
        },

        TuiEvent::Resize(_, _) => {
            // Terminal will re-render on next frame
        }

        TuiEvent::Tick => {
            state.spinner_frame = (state.spinner_frame + 1) % 10;
        }

        TuiEvent::Thinking(msg) => {
            state.status_text = msg;
        }

        TuiEvent::ToolStarted { name, detail } => {
            state.status_text = match &detail {
                Some(d) => format!("Running {name}: {d}"),
                None => format!("Running {name}..."),
            };
            state.active_tools.push(ToolActivity {
                name,
                started_at: chrono::Utc::now(),
                duration_ms: None,
                status: ToolStatus::Running,
                detail,
                result_preview: None,
            });
        }

        TuiEvent::ToolCompleted {
            name,
            success,
            error: _,
        } => {
            // Move from active to recent
            if let Some(pos) = state.active_tools.iter().position(|t| t.name == name) {
                let mut tool = state.active_tools.remove(pos);
                tool.duration_ms = Some(
                    chrono::Utc::now()
                        .signed_duration_since(tool.started_at)
                        .num_milliseconds()
                        .unsigned_abs(),
                );
                tool.status = if success {
                    ToolStatus::Success
                } else {
                    ToolStatus::Failed
                };
                state.recent_tools.push(tool);
                // Keep recent list bounded
                if state.recent_tools.len() > 20 {
                    state.recent_tools.remove(0);
                }
            }
            if state.active_tools.is_empty() {
                state.status_text.clear();
            }
        }

        TuiEvent::ToolResult { name, preview } => {
            if let Some(tool) = state.active_tools.iter_mut().find(|t| t.name == name) {
                tool.result_preview = Some(preview);
            } else if let Some(tool) = state.recent_tools.iter_mut().rev().find(|t| t.name == name)
            {
                tool.result_preview = Some(preview);
            }
        }

        TuiEvent::StreamChunk(chunk) => {
            state.is_streaming = true;
            // Append to the last assistant message, or create one
            if let Some(last) = state.messages.last_mut() {
                if last.role == MessageRole::Assistant {
                    last.content.push_str(&chunk);
                } else {
                    state.messages.push(ChatMessage {
                        role: MessageRole::Assistant,
                        content: chunk,
                        timestamp: chrono::Utc::now(),
                        cost_summary: None,
                    });
                }
            } else {
                state.messages.push(ChatMessage {
                    role: MessageRole::Assistant,
                    content: chunk,
                    timestamp: chrono::Utc::now(),
                    cost_summary: None,
                });
            }
            state.scroll_offset = 0;
        }

        TuiEvent::Status(msg) => {
            state.status_text = msg;
        }

        TuiEvent::Response { content, .. } => {
            state.is_streaming = false;
            state.status_text.clear();
            // If streaming already appended content, replace; otherwise add new
            if let Some(last) = state.messages.last_mut() {
                if last.role == MessageRole::Assistant && state.is_streaming {
                    // Streaming finished — content was already accumulated
                } else if last.role != MessageRole::Assistant {
                    state.messages.push(ChatMessage {
                        role: MessageRole::Assistant,
                        content,
                        timestamp: chrono::Utc::now(),
                        cost_summary: None,
                    });
                }
            } else {
                state.messages.push(ChatMessage {
                    role: MessageRole::Assistant,
                    content,
                    timestamp: chrono::Utc::now(),
                    cost_summary: None,
                });
            }
            state.scroll_offset = 0;
            state.active_tools.clear();
        }

        TuiEvent::JobStarted { job_id, title } => {
            let now = chrono::Utc::now();
            state.messages.push(ChatMessage {
                role: MessageRole::System,
                content: format!("[job] {title} ({job_id})"),
                timestamp: now,
                cost_summary: None,
            });
            state.toasts.push(Toast {
                message: format!("Job started: {title}"),
                kind: ToastKind::Info,
                created_at: now,
            });
            state.jobs.push(JobInfo {
                id: job_id.clone(),
                title: title.clone(),
                status: JobStatus::Running,
                started_at: now,
            });
            // Mirror to threads for the sidebar widget
            state.threads.push(ThreadInfo {
                id: job_id,
                label: title,
                is_foreground: false,
                is_running: true,
                duration_secs: 0,
                status: ThreadStatus::Active,
                started_at: now,
            });
        }

        TuiEvent::JobStatus { job_id, status } => {
            let new_status = match status.as_str() {
                "running" | "in_progress" => JobStatus::Running,
                "completed" | "done" => JobStatus::Completed,
                "failed" => JobStatus::Failed,
                _ => JobStatus::Running,
            };
            if let Some(job) = state.jobs.iter_mut().find(|j| j.id == job_id) {
                job.status = new_status;
            }
            // Update matching thread
            if let Some(thread) = state.threads.iter_mut().find(|t| t.id == job_id) {
                match new_status {
                    JobStatus::Running => {
                        thread.status = ThreadStatus::Active;
                        thread.is_running = true;
                    }
                    JobStatus::Completed => {
                        thread.status = ThreadStatus::Completed;
                        thread.is_running = false;
                    }
                    JobStatus::Failed => {
                        thread.status = ThreadStatus::Failed;
                        thread.is_running = false;
                    }
                    JobStatus::Pending => {
                        thread.status = ThreadStatus::Idle;
                        thread.is_running = false;
                    }
                }
            }
        }

        TuiEvent::JobResult { job_id, status } => {
            let new_status = if status == "failed" {
                JobStatus::Failed
            } else {
                JobStatus::Completed
            };
            if let Some(job) = state.jobs.iter_mut().find(|j| j.id == job_id) {
                job.status = new_status;
            }
            // Update matching thread
            if let Some(thread) = state.threads.iter_mut().find(|t| t.id == job_id) {
                thread.is_running = false;
                thread.status = if new_status == JobStatus::Failed {
                    ThreadStatus::Failed
                } else {
                    ThreadStatus::Completed
                };
            }
        }

        TuiEvent::RoutineUpdate {
            id,
            name,
            trigger_type,
            enabled,
            last_run,
            next_fire,
        } => {
            // Upsert: update existing or insert new
            if let Some(routine) = state.routines.iter_mut().find(|r| r.id == id) {
                routine.name = name;
                routine.trigger_type = trigger_type;
                routine.enabled = enabled;
                routine.last_run = last_run;
                routine.next_fire = next_fire;
            } else {
                state.routines.push(RoutineInfo {
                    id,
                    name,
                    trigger_type,
                    enabled,
                    last_run,
                    next_fire,
                });
            }
        }

        TuiEvent::ApprovalNeeded {
            request_id,
            tool_name,
            description,
            parameters,
            allow_always,
        } => {
            state.pending_approval = Some(super::widgets::ApprovalRequest {
                request_id,
                tool_name,
                description,
                parameters,
                allow_always,
                selected: 0,
            });
        }

        TuiEvent::AuthRequired {
            extension_name,
            instructions,
        } => {
            let msg = if let Some(instr) = instructions {
                format!("Authentication required for {extension_name}: {instr}")
            } else {
                format!("Authentication required for {extension_name}")
            };
            state.toasts.push(Toast {
                message: format!("Auth needed: {extension_name}"),
                kind: ToastKind::Warning,
                created_at: chrono::Utc::now(),
            });
            state.messages.push(ChatMessage {
                role: MessageRole::System,
                content: msg,
                timestamp: chrono::Utc::now(),
                cost_summary: None,
            });
        }

        TuiEvent::AuthCompleted {
            extension_name,
            success,
            message,
        } => {
            let prefix = if success { "\u{2713}" } else { "\u{2717}" };
            state.toasts.push(Toast {
                message: format!("{prefix} {extension_name}"),
                kind: if success {
                    ToastKind::Success
                } else {
                    ToastKind::Error
                },
                created_at: chrono::Utc::now(),
            });
            state.messages.push(ChatMessage {
                role: MessageRole::System,
                content: format!("{prefix} {extension_name}: {message}"),
                timestamp: chrono::Utc::now(),
                cost_summary: None,
            });
        }

        TuiEvent::ReasoningUpdate { narrative } => {
            if !narrative.is_empty() {
                state.status_text = narrative;
            }
        }

        TuiEvent::TurnCost {
            input_tokens,
            output_tokens,
            cost_usd,
        } => {
            state.total_input_tokens += input_tokens;
            state.total_output_tokens += output_tokens;
            state.total_cost_usd = cost_usd.clone();
            // Attach to last assistant message
            if let Some(msg) = state
                .messages
                .iter_mut()
                .rev()
                .find(|m| m.role == MessageRole::Assistant)
            {
                msg.cost_summary = Some(TurnCostSummary {
                    input_tokens,
                    output_tokens,
                    cost_usd,
                });
            }
        }

        TuiEvent::Suggestions { suggestions } => {
            state.suggestions = suggestions;
        }

        TuiEvent::ContextPressure {
            used_tokens,
            max_tokens,
            percentage,
            warning,
        } => {
            state.context_pressure = Some(ContextPressureInfo {
                used_tokens,
                max_tokens,
                percentage,
                warning,
            });
        }

        TuiEvent::SandboxStatus {
            docker_available,
            running_containers,
            status,
        } => {
            state.sandbox_status = Some(SandboxInfo {
                docker_available,
                running_containers,
                status,
            });
        }

        TuiEvent::SecretsStatus {
            count,
            vault_unlocked,
        } => {
            state.secrets_status = Some(SecretsInfo {
                count,
                vault_unlocked,
            });
        }

        TuiEvent::CostGuard {
            session_budget_usd,
            spent_usd,
            remaining_usd,
            limit_reached,
        } => {
            if limit_reached {
                state.toasts.push(Toast {
                    message: "Cost limit reached".to_string(),
                    kind: ToastKind::Error,
                    created_at: chrono::Utc::now(),
                });
            }
            state.cost_guard = Some(CostGuardInfo {
                session_budget_usd,
                spent_usd,
                remaining_usd,
                limit_reached,
            });
        }

        TuiEvent::Log {
            level,
            target,
            message,
            timestamp,
        } => {
            state.log_entries.push(TuiLogEntry {
                level,
                target,
                message,
                timestamp,
            });
        }
    }
}

/// Render a single frame.
fn render_frame(
    frame: &mut ratatui::Frame<'_>,
    state: &mut AppState,
    widgets: &BuiltinWidgets,
    layout: &TuiLayout,
) {
    let size = frame.area();

    // Vertical layout: header (0-1) | tab bar (1) | main | input (3) | status (1)
    let header_height = if layout.header.visible { 1 } else { 0 };
    let status_height = if layout.status_bar.visible { 1 } else { 0 };
    let tab_bar_height = 1u16;
    let input_height = if state.pending_attachments.is_empty() {
        3u16
    } else {
        4u16
    };

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Length(tab_bar_height),
            Constraint::Min(4),
            Constraint::Length(input_height),
            Constraint::Length(status_height),
        ])
        .split(size);

    let header_area = vertical[0];
    let tab_bar_area = vertical[1];
    let main_area = vertical[2];
    let input_area = vertical[3];
    let status_area = vertical[4];

    // Header
    if layout.header.visible {
        widgets
            .header
            .render(header_area, frame.buffer_mut(), state);
    }

    // Tab bar
    widgets
        .tab_bar
        .render(tab_bar_area, frame.buffer_mut(), state);

    // Main area: conversation/logs | sidebar
    match state.active_tab {
        ActiveTab::Logs => {
            // Logs tab takes the full main area (no sidebar)
            widgets.logs.render(main_area, frame.buffer_mut(), state);
        }
        ActiveTab::Conversation => {
            if state.sidebar_visible && main_area.width > 40 {
                let sidebar_width =
                    (main_area.width as u32 * layout.sidebar.effective_width() as u32 / 100) as u16;
                let conversation_width = main_area.width.saturating_sub(sidebar_width + 1);

                let horizontal = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Length(conversation_width),
                        Constraint::Length(1), // border
                        Constraint::Length(sidebar_width),
                    ])
                    .split(main_area);

                let conv_area = horizontal[0];
                let border_area = horizontal[1];
                let sidebar_area = horizontal[2];

                widgets
                    .conversation
                    .render(conv_area, frame.buffer_mut(), state);

                // Vertical border
                render_vertical_border(frame, border_area, layout);

                // Split sidebar into tool panel and thread list
                let sidebar_split = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(sidebar_area);

                widgets
                    .tool_panel
                    .render(sidebar_split[0], frame.buffer_mut(), state);
                widgets
                    .thread_list
                    .render(sidebar_split[1], frame.buffer_mut(), state);
            } else {
                widgets
                    .conversation
                    .render(main_area, frame.buffer_mut(), state);
            }
        }
    }

    // Input area with top border
    let input_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(input_area);

    render_horizontal_border(frame, input_split[0], layout);
    widgets
        .input_box
        .render(input_split[1], frame.buffer_mut(), state);

    // Status bar
    if layout.status_bar.visible {
        render_horizontal_border(frame, status_area, layout);
        // Status bar renders on same line as border (overwriting)
        widgets
            .status_bar
            .render(status_area, frame.buffer_mut(), state);
    }

    // Command palette overlay (above input area)
    if state.command_palette.visible && !state.command_palette.filtered.is_empty() {
        let palette_area = CommandPaletteWidget::palette_area(
            size,
            input_area,
            state.command_palette.filtered.len(),
        );
        if palette_area.height > 0 {
            widgets.command_palette.render_palette(
                palette_area,
                frame.buffer_mut(),
                &state.command_palette,
            );
        }
    }

    // Approval modal (rendered on top of everything)
    if state.pending_approval.is_some() {
        let modal_area = ApprovalWidget::modal_area(size);
        widgets
            .approval
            .render(modal_area, frame.buffer_mut(), state);
    }

    // Tool detail modal (Ctrl+E)
    if state.tool_detail_modal.is_some() {
        render_tool_detail_modal(frame, size, state, layout);
    }

    // Help overlay (F1)
    if state.help_visible {
        let help_area = HelpOverlayWidget::modal_area(size);
        widgets.help.render(help_area, frame.buffer_mut(), state);
    }

    // Notification toasts (bottom-right, above status bar)
    render_toasts(frame, size, state, layout);
}

/// Check input text and open/close the command palette accordingly.
fn update_palette_from_input(
    input_box: &crate::widgets::input_box::InputBoxWidget,
    state: &mut AppState,
) {
    let text = input_box.current_text();
    let trimmed = text.trim();
    if trimmed.starts_with('/') && !trimmed.contains(' ') {
        // Text after the leading '/'
        let filter = &trimmed[1..];
        state.command_palette.open(filter);
    } else {
        state.command_palette.close();
    }
}

/// Render a vertical border line.
fn render_vertical_border(frame: &mut ratatui::Frame<'_>, area: Rect, layout: &TuiLayout) {
    let theme = layout.resolve_theme();
    let border_style = theme.border_style();

    for y in area.y..area.y + area.height {
        if let Some(cell) = frame.buffer_mut().cell_mut((area.x, y)) {
            cell.set_symbol("\u{2502}");
            cell.set_style(border_style);
        }
    }
}

/// Render a horizontal border line.
fn render_horizontal_border(frame: &mut ratatui::Frame<'_>, area: Rect, layout: &TuiLayout) {
    let theme = layout.resolve_theme();
    let border_style = theme.border_style();

    for x in area.x..area.x + area.width {
        if let Some(cell) = frame.buffer_mut().cell_mut((x, area.y)) {
            cell.set_symbol("\u{2500}");
            cell.set_style(border_style);
        }
    }
}

/// Render the tool detail modal (Ctrl+E).
#[allow(clippy::cast_possible_truncation)]
fn render_tool_detail_modal(
    frame: &mut ratatui::Frame<'_>,
    size: Rect,
    state: &AppState,
    layout: &TuiLayout,
) {
    use ratatui::style::Modifier;
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

    let Some(ref modal) = state.tool_detail_modal else {
        return;
    };
    let theme = layout.resolve_theme();

    let width = (size.width * 3 / 4)
        .max(40)
        .min(size.width.saturating_sub(4));
    let height = (size.height * 3 / 4)
        .max(10)
        .min(size.height.saturating_sub(4));
    let x = (size.width.saturating_sub(width)) / 2;
    let y = (size.height.saturating_sub(height)) / 2;
    let area = Rect::new(x, y, width, height);

    Clear.render(area, frame.buffer_mut());

    let title = format!(" {} ", modal.tool_name);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.accent_style())
        .title(Span::styled(
            title,
            theme.accent_style().add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());

    let lines: Vec<Line<'_>> = modal
        .content
        .lines()
        .map(|l| Line::from(l.to_string()))
        .collect();

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((modal.scroll, 0));
    paragraph.render(inner, frame.buffer_mut());
}

/// Render notification toasts in the bottom-right corner.
fn render_toasts(
    frame: &mut ratatui::Frame<'_>,
    size: Rect,
    state: &mut AppState,
    layout: &TuiLayout,
) {
    use ratatui::style::Modifier;
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

    // Prune expired toasts (older than 5 seconds)
    let now = chrono::Utc::now();
    state
        .toasts
        .retain(|t| now.signed_duration_since(t.created_at).num_seconds() < 5);

    if state.toasts.is_empty() {
        return;
    }

    let theme = layout.resolve_theme();
    let max_toasts = 3usize;
    let toast_width = 40u16.min(size.width.saturating_sub(2));

    // Stack toasts from bottom up, above status bar
    let start_y = size.height.saturating_sub(3); // above status bar + input
    let visible_toasts = state.toasts.iter().rev().take(max_toasts);

    for (i, toast) in visible_toasts.enumerate() {
        let y = start_y.saturating_sub((i as u16) * 3);
        let x = size.width.saturating_sub(toast_width + 1);
        let area = Rect::new(x, y, toast_width, 3);

        if area.y == 0 {
            continue;
        }

        Clear.render(area, frame.buffer_mut());

        let (icon, border_style) = match toast.kind {
            ToastKind::Info => ("\u{2139}", theme.accent_style()),
            ToastKind::Success => ("\u{2713}", theme.success_style()),
            ToastKind::Warning => ("\u{26A0}", theme.warning_style()),
            ToastKind::Error => ("\u{2717}", theme.error_style()),
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style);
        let inner = block.inner(area);
        block.render(area, frame.buffer_mut());

        let msg_width = inner.width as usize;
        let display_msg = if toast.message.len() > msg_width.saturating_sub(3) {
            format!(
                "{}...",
                &toast.message[..msg_width.saturating_sub(6).min(toast.message.len())]
            )
        } else {
            toast.message.clone()
        };

        let line = Line::from(vec![
            Span::styled(
                format!(" {icon} "),
                border_style.add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                display_msg,
                ratatui::style::Style::default().fg(theme.fg.to_color()),
            ),
        ]);
        let paragraph = Paragraph::new(line);
        paragraph.render(inner, frame.buffer_mut());
    }
}

/// Try to read an image from the system clipboard and return it as a PNG-encoded
/// [`TuiAttachment`]. Returns `None` if the clipboard has no image data or if
/// encoding fails.
fn try_paste_clipboard_image(state: &AppState) -> Option<TuiAttachment> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    let img_data = clipboard.get_image().ok()?;

    let png_bytes = encode_rgba_to_png(
        &img_data.bytes,
        img_data.width as u32,
        img_data.height as u32,
    )?;

    let n = state.pending_attachments.len() + 1;
    Some(TuiAttachment {
        data: png_bytes,
        mime_type: "image/png".to_string(),
        label: format!("Image {n}"),
    })
}

/// Encode raw RGBA pixel data to PNG. Returns `None` on invalid dimensions or
/// encoding failure.
fn encode_rgba_to_png(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let expected_len = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    if rgba.len() != expected_len {
        return None;
    }

    let buf: image::ImageBuffer<image::Rgba<u8>, &[u8]> =
        image::ImageBuffer::from_raw(width, height, rgba)?;
    let mut png_bytes: Vec<u8> = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut png_bytes);
    buf.write_to(&mut cursor, image::ImageFormat::Png).ok()?;
    Some(png_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_rgba_to_png_valid() {
        // 2x2 red image
        let rgba = vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];
        let png = encode_rgba_to_png(&rgba, 2, 2);
        assert!(png.is_some());
        let bytes = png.unwrap();
        // PNG signature starts with 0x89 'P' 'N' 'G'
        assert!(bytes.len() > 8);
        assert_eq!(&bytes[..4], &[0x89, b'P', b'N', b'G']);
    }

    #[test]
    fn encode_rgba_to_png_bad_dimensions() {
        let rgba = vec![0u8; 16]; // 4 pixels
        // Claim 3x2 = 6 pixels, but only 4 are provided
        let png = encode_rgba_to_png(&rgba, 3, 2);
        assert!(png.is_none());
    }

    #[test]
    fn encode_rgba_to_png_zero_size() {
        // 0x0 image: the image crate rejects zero-dimension buffers
        let png = encode_rgba_to_png(&[], 0, 0);
        assert!(png.is_none());
    }
}
