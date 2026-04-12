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

use std::io::{self, Write};
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::Show;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event as CtEvent, KeyCode, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
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
use crate::widgets::conversation::ConversationWidget;
use crate::widgets::logs::LogsWidget;
use crate::widgets::help_overlay::HelpOverlayWidget;
use crate::widgets::model_picker::{ModelPickerState, ModelPickerWidget};
use crate::widgets::registry::{BuiltinWidgets, create_default_widgets};
use crate::widgets::thread_list::engine_thread_index_at;
use crate::widgets::thread_picker::ThreadPickerWidget;
use crate::widgets::{
    ActiveTab, AppState, ApprovalRequest, ChatMessage, ContextPressureInfo, CostGuardInfo,
    EngineThreadInfo, JobInfo, JobStatus, MessageRole, RoutineInfo, SandboxInfo, ScreenSnapshot,
    SecretsInfo, SelectionPoint, SkillCategory, TextSelection, ThreadStatus, Toast, ToastKind,
    ToolActivity, ToolCategory, ToolDetailModal, ToolStatus, TuiWidget, TurnCostSummary,
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
    /// Best-effort model list for the `/model` picker.
    pub available_models: Vec<String>,
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
    let mut restore_guard = TerminalRestoreGuard::new();
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
        model_picker: ModelPickerState::with_models(config.available_models),
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
                Ok(Some(CtEvent::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column,
                    row,
                    ..
                }))) => {
                    if poll_tx
                        .send(TuiEvent::MouseClick { column, row })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Some(CtEvent::Mouse(MouseEvent {
                    kind: MouseEventKind::Drag(MouseButton::Left),
                    column,
                    row,
                    ..
                }))) => {
                    if poll_tx
                        .send(TuiEvent::MouseDrag { column, row })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Some(CtEvent::Mouse(MouseEvent {
                    kind: MouseEventKind::Up(MouseButton::Left),
                    column,
                    row,
                    ..
                }))) => {
                    if poll_tx
                        .send(TuiEvent::MouseRelease { column, row })
                        .await
                        .is_err()
                    {
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

                // Drain all pending events from the channel, coalescing consecutive
                // MouseScroll events into a single net delta so that reversing scroll
                // direction takes effect immediately instead of unwinding a backlog.
                let mut pending = vec![event];
                while let Ok(e) = event_rx.try_recv() {
                    pending.push(e);
                }
                let mut coalesced: Vec<TuiEvent> = Vec::with_capacity(pending.len());
                for ev in pending {
                    if let TuiEvent::MouseScroll(delta) = &ev
                        && let Some(TuiEvent::MouseScroll(prev)) = coalesced.last_mut()
                    {
                        *prev += delta;
                        continue;
                    }
                    coalesced.push(ev);
                }
                // Drop scroll events that netted to zero
                for ev in coalesced {
                    if let TuiEvent::MouseScroll(0) = ev {
                        continue;
                    }
                    handle_event(ev, &mut state, &mut widgets, &msg_tx, &layout).await;
                }
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
    restore_guard.disarm();
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

fn outgoing_thread_scope(text: &str, current_thread_id: Option<&str>) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.eq_ignore_ascii_case("/new")
        || trimmed.eq_ignore_ascii_case("/clear")
        || trimmed.eq_ignore_ascii_case("/thread new")
        || trimmed.to_ascii_lowercase().starts_with("/thread ")
    {
        return None;
    }

    current_thread_id.map(str::to_owned)
}

fn update_local_thread_scope_after_submit(state: &mut AppState, text: &str) {
    let trimmed = text.trim();
    if trimmed.eq_ignore_ascii_case("/new")
        || trimmed.eq_ignore_ascii_case("/clear")
        || trimmed.eq_ignore_ascii_case("/thread new")
    {
        state.current_thread_id = None;
    }
}

fn parse_engine_thread_timestamp(
    raw: &str,
    field: &'static str,
    thread_id: &str,
) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Some(parsed.with_timezone(&chrono::Utc));
    }

    if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M") {
        return Some(chrono::DateTime::from_naive_utc_and_offset(
            parsed,
            chrono::Utc,
        ));
    }

    tracing::debug!(
        thread_id,
        field,
        raw,
        "Failed to parse engine thread timestamp"
    );
    None
}

/// Handle a single TUI event.
async fn handle_event(
    event: TuiEvent,
    state: &mut AppState,
    widgets: &mut BuiltinWidgets,
    msg_tx: &mpsc::Sender<TuiUserMessage>,
    layout: &TuiLayout,
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

                update_input_overlays_from_input(&widgets.input_box, state);

                if state.search.active {
                    state.search.query = widgets.input_box.current_text();
                    state.search.match_count =
                        count_search_matches(&state.messages, &state.search.query);
                    state.search.current_match = 0;
                }
            }
        }
        TuiEvent::Key(key) => {
            let action = resolve_key_action(key, state, widgets);

            match action {
                InputAction::Submit => {
                    let selected_model = if state.model_picker.visible {
                        state.model_picker.selected_model().map(str::to_owned)
                    } else {
                        None
                    };
                    state.model_picker.close();
                    state.command_palette.close();
                    let text = widgets.input_box.take_input();
                    let trimmed = if let Some(ref model) = selected_model {
                        format!("/model {model}")
                    } else {
                        text.trim().to_string()
                    };
                    let attachments = std::mem::take(&mut state.pending_attachments);
                    if !trimmed.is_empty() || !attachments.is_empty() {
                        state.awaiting_model_list = selected_model.is_none()
                            && attachments.is_empty()
                            && trimmed == "/model";
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
                        state.pinned_to_bottom = true;
                        if let Some(model) = selected_model {
                            state.model = model;
                        }
                        // Send to agent
                        update_local_thread_scope_after_submit(state, &trimmed);
                        let thread_id =
                            outgoing_thread_scope(&trimmed, state.current_thread_id.as_deref());
                        let _ = msg_tx
                            .send(TuiUserMessage {
                                text: trimmed,
                                attachments,
                                thread_id,
                                ui_action: None,
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
                        let page = state.conversation_height.max(2).saturating_sub(2) as i16;
                        ConversationWidget::scroll(state, -page);
                    }
                    ActiveTab::Logs => {
                        LogsWidget::scroll(state, -5);
                    }
                },
                InputAction::ScrollDown => match state.active_tab {
                    ActiveTab::Conversation => {
                        let page = state.conversation_height.max(2).saturating_sub(2) as i16;
                        ConversationWidget::scroll(state, page);
                    }
                    ActiveTab::Logs => {
                        LogsWidget::scroll(state, 5);
                    }
                },
                InputAction::ScrollToBottom => {
                    state.pinned_to_bottom = true;
                }
                InputAction::Interrupt => {
                    let _ = msg_tx
                        .send(
                            TuiUserMessage::text_only("/interrupt")
                                .with_thread_id(state.current_thread_id.clone()),
                        )
                        .await;
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
                            .send(
                                TuiUserMessage::text_only(action.as_response())
                                    .with_thread_id(state.current_thread_id.clone()),
                            )
                            .await;
                        state.pending_approval = None;
                    }
                }
                InputAction::ApprovalCancel => {
                    if state.pending_approval.is_some() {
                        let _ = msg_tx
                            .send(
                                TuiUserMessage::text_only("n")
                                    .with_thread_id(state.current_thread_id.clone()),
                            )
                            .await;
                        state.pending_approval = None;
                    }
                }
                InputAction::QuickApprove => {
                    if state.pending_approval.is_some() {
                        let _ = msg_tx
                            .send(
                                TuiUserMessage::text_only("y")
                                    .with_thread_id(state.current_thread_id.clone()),
                            )
                            .await;
                        state.pending_approval = None;
                    }
                }
                InputAction::QuickAlways => {
                    if let Some(ref ap) = state.pending_approval {
                        if ap.allow_always {
                            let _ = msg_tx
                                .send(
                                    TuiUserMessage::text_only("a")
                                        .with_thread_id(state.current_thread_id.clone()),
                                )
                                .await;
                        } else {
                            let _ = msg_tx
                                .send(
                                    TuiUserMessage::text_only("y")
                                        .with_thread_id(state.current_thread_id.clone()),
                                )
                                .await;
                        }
                        state.pending_approval = None;
                    }
                }
                InputAction::QuickDeny => {
                    if state.pending_approval.is_some() {
                        let _ = msg_tx
                            .send(
                                TuiUserMessage::text_only("n")
                                    .with_thread_id(state.current_thread_id.clone()),
                            )
                            .await;
                        state.pending_approval = None;
                    }
                }
                InputAction::PaletteUp => {
                    if state.model_picker.visible {
                        state.model_picker.move_up();
                    } else {
                        state.command_palette.move_up();
                    }
                }
                InputAction::PaletteDown => {
                    if state.model_picker.visible {
                        state.model_picker.move_down();
                    } else {
                        state.command_palette.move_down();
                    }
                }
                InputAction::PaletteSelect => {
                    if state.model_picker.visible {
                        let command = state
                            .model_picker
                            .selected_model()
                            .map(|model| format!("/model {model}"))
                            .unwrap_or_else(|| widgets.input_box.current_text().trim().to_string());
                        let attachments = std::mem::take(&mut state.pending_attachments);
                        let _ = widgets.input_box.take_input();
                        state.model_picker.close();
                        state.command_palette.close();

                        if !command.is_empty() || !attachments.is_empty() {
                            state.awaiting_model_list =
                                attachments.is_empty() && command == "/model";
                            if !command.is_empty() {
                                state.input_history.push(command.clone());
                            }
                            state.history_index = None;
                            state.history_draft.clear();
                            state.suggestions.clear();

                            let display_content = if attachments.is_empty() {
                                command.clone()
                            } else {
                                let labels: Vec<&str> =
                                    attachments.iter().map(|a| a.label.as_str()).collect();
                                format!("{command} [{}]", labels.join("] ["))
                            };

                            state.messages.push(ChatMessage {
                                role: MessageRole::User,
                                content: display_content,
                                timestamp: chrono::Utc::now(),
                                cost_summary: None,
                            });
                            state.pinned_to_bottom = true;

                            if let Some(model) = command.strip_prefix("/model ") {
                                state.model = model.to_string();
                            }

                            update_local_thread_scope_after_submit(state, &command);
                            let thread_id =
                                outgoing_thread_scope(&command, state.current_thread_id.as_deref());
                            let _ = msg_tx
                                .send(TuiUserMessage {
                                    text: command,
                                    attachments,
                                    thread_id,
                                    ui_action: None,
                                })
                                .await;
                        }
                    } else if let Some(cmd) = state.command_palette.selected_command() {
                        state.command_palette.close();
                        if cmd == "/model" {
                            if state.model_picker.has_models() {
                                widgets.input_box.set_text("/model ");
                                state.model_picker.open("");
                            } else {
                                let command = cmd.to_string();
                                let attachments = std::mem::take(&mut state.pending_attachments);
                                let _ = widgets.input_box.take_input();

                                if !command.is_empty() || !attachments.is_empty() {
                                    state.awaiting_model_list =
                                        attachments.is_empty() && command == "/model";
                                    if !command.is_empty() {
                                        state.input_history.push(command.clone());
                                    }
                                    state.history_index = None;
                                    state.history_draft.clear();
                                    state.suggestions.clear();

                                    let display_content = if attachments.is_empty() {
                                        command.clone()
                                    } else {
                                        let labels: Vec<&str> =
                                            attachments.iter().map(|a| a.label.as_str()).collect();
                                        format!("{command} [{}]", labels.join("] ["))
                                    };

                                    state.messages.push(ChatMessage {
                                        role: MessageRole::User,
                                        content: display_content,
                                        timestamp: chrono::Utc::now(),
                                        cost_summary: None,
                                    });
                                    state.pinned_to_bottom = true;

                                    update_local_thread_scope_after_submit(state, &command);
                                    let thread_id = outgoing_thread_scope(
                                        &command,
                                        state.current_thread_id.as_deref(),
                                    );
                                    let _ = msg_tx
                                        .send(TuiUserMessage {
                                            text: command,
                                            attachments,
                                            thread_id,
                                            ui_action: None,
                                        })
                                        .await;
                                }
                            }
                        } else {
                            let text = format!("{cmd} ");
                            widgets.input_box.set_text(&text);
                        }
                    }
                }
                InputAction::PaletteClose => {
                    state.model_picker.close();
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
                            update_input_overlays_from_input(&widgets.input_box, state);
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
                            update_input_overlays_from_input(&widgets.input_box, state);
                        } else {
                            let new_idx = idx + 1;
                            state.history_index = Some(new_idx);
                            if let Some(text) = state.input_history.get(new_idx) {
                                widgets.input_box.set_text(text);
                                update_input_overlays_from_input(&widgets.input_box, state);
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
                InputAction::ThreadPickerUp => {
                    if let Some(ref mut picker) = state.pending_thread_picker {
                        crate::widgets::thread_picker::thread_picker_up(picker);
                    }
                }
                InputAction::ThreadPickerDown => {
                    if let Some(ref mut picker) = state.pending_thread_picker {
                        crate::widgets::thread_picker::thread_picker_down(picker);
                    }
                }
                InputAction::ThreadPickerSelect => {
                    if let Some(ref picker) = state.pending_thread_picker
                        && let Some(id) =
                            crate::widgets::thread_picker::thread_picker_selected_id(picker)
                    {
                        let cmd = format!("/thread {id}");
                        let _ = msg_tx
                            .send(TuiUserMessage::text_only(cmd).with_thread_id(None))
                            .await;
                        state.current_thread_id = Some(id.to_string());
                    }
                    state.pending_thread_picker = None;
                }
                InputAction::ThreadPickerClose => {
                    state.pending_thread_picker = None;
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
                    } else if key.code == KeyCode::Backspace
                        && widgets.input_box.is_empty()
                        && !state.pending_attachments.is_empty()
                    {
                        let removed = state.pending_attachments.pop();
                        if let Some(att) = removed {
                            state.toasts.push(Toast {
                                message: format!("Removed: {}", att.label),
                                kind: ToastKind::Info,
                                created_at: chrono::Utc::now(),
                            });
                        }
                    } else {
                        widgets.input_box.handle_key(key, state);
                        // Update command palette visibility based on input content
                        update_input_overlays_from_input(&widgets.input_box, state);
                    }
                }
            }
        }

        TuiEvent::MouseClick { column, row } => {
            handle_mouse_click(column, row, state, msg_tx, layout).await;
        }

        TuiEvent::MouseDrag { column, row } => {
            handle_mouse_drag(column, row, state);
        }

        TuiEvent::MouseRelease { column, row } => {
            handle_mouse_release(column, row, state);
        }

        TuiEvent::MouseScroll(delta) => {
            if let Some(ref mut modal) = state.tool_detail_modal {
                if delta < 0 {
                    // Scroll up — show earlier content (decrease Paragraph offset)
                    modal.scroll = modal.scroll.saturating_sub(delta.unsigned_abs());
                } else {
                    // Scroll down — show later content (increase Paragraph offset)
                    modal.scroll = modal.scroll.saturating_add(delta as u16);
                }
            } else if let Some(ref mut picker) = state.pending_thread_picker {
                if delta < 0 {
                    crate::widgets::thread_picker::thread_picker_up(picker);
                } else if delta > 0 {
                    crate::widgets::thread_picker::thread_picker_down(picker);
                }
            } else if let Some(ref mut approval) = state.pending_approval {
                let count = ApprovalWidget::options(approval.allow_always).len();
                if delta < 0 {
                    approval.selected = if approval.selected == 0 {
                        count - 1
                    } else {
                        approval.selected - 1
                    };
                } else if delta > 0 {
                    approval.selected = (approval.selected + 1) % count;
                }
            } else if !state.help_visible {
                match state.active_tab {
                    ActiveTab::Conversation => {
                        ConversationWidget::scroll(state, delta);
                    }
                    ActiveTab::Logs => {
                        LogsWidget::scroll(state, delta);
                    }
                }
            }
        }

        TuiEvent::Resize(_, _) => {
            // Terminal will re-render on next frame
        }

        TuiEvent::Tick => {
            state.tick_count = state.tick_count.wrapping_add(1);
        }

        TuiEvent::Thinking(msg) => {
            state.status_text = msg;
        }

        TuiEvent::ToolStarted {
            name,
            detail,
            call_id,
        } => {
            state.status_text = match &detail {
                Some(d) => format!("Running {name}: {d}"),
                None => format!("Running {name}..."),
            };
            state.active_tools.push(ToolActivity {
                call_id,
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
            call_id,
        } => {
            // Move from active to recent
            if let Some(pos) = state
                .active_tools
                .iter()
                .position(|t| tool_activity_matches(t, &name, call_id.as_deref()))
            {
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

        TuiEvent::ToolResult {
            name,
            preview,
            call_id,
        } => {
            if let Some(tool) = state
                .active_tools
                .iter_mut()
                .find(|t| tool_activity_matches(t, &name, call_id.as_deref()))
            {
                tool.result_preview = Some(preview);
            } else if let Some(tool) = state
                .recent_tools
                .iter_mut()
                .rev()
                .find(|t| tool_activity_matches(t, &name, call_id.as_deref()))
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
            // pinned_to_bottom is checked during render — no offset update needed
        }

        TuiEvent::Status(msg) => {
            state.status_text = msg;
        }

        TuiEvent::Response { content, thread_id } => {
            if let Some(thread_id) = thread_id {
                state.current_thread_id = Some(thread_id);
            }
            let was_streaming = state.is_streaming;
            state.is_streaming = false;
            state.status_text.clear();
            let parsed_model_response = if state.awaiting_model_list {
                parse_model_list_response(&content)
            } else {
                None
            };
            state.awaiting_model_list = false;
            // Streaming responses accumulate via StreamChunk; non-streaming
            // responses still need a fresh assistant message.
            if let Some(last) = state.messages.last_mut() {
                if last.role == MessageRole::Assistant && was_streaming {
                    // Streaming finished — content was already accumulated
                } else {
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
            // pinned_to_bottom is checked during render — no offset update needed
            state.active_tools.clear();

            if let Some((active_model, models)) = parsed_model_response {
                state.model = active_model;
                state.model_picker.set_models(models);
                widgets.input_box.set_text("/model ");
                update_input_overlays_from_input(&widgets.input_box, state);
            }
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
            // Update context_window from the engine's actual value
            if max_tokens > 0 {
                state.context_window = max_tokens;
            }
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

        TuiEvent::ThreadList { threads } => {
            // ThreadList only populates the /resume picker, not the sidebar.
            // The sidebar THREADS section uses EngineThreadList instead.
            state.pending_thread_picker = if threads.is_empty() {
                None
            } else {
                Some(super::widgets::ThreadPickerState {
                    threads,
                    selected: 0,
                })
            };
        }

        TuiEvent::EngineThreadList { threads } => {
            state.engine_threads = threads
                .iter()
                .map(|t| EngineThreadInfo {
                    id: t.id.clone(),
                    goal: t.goal.clone(),
                    thread_type: t.thread_type.clone(),
                    status: match t.state.as_str() {
                        "Running" => ThreadStatus::Active,
                        "Completed" | "Done" => ThreadStatus::Completed,
                        "Failed" => ThreadStatus::Failed,
                        _ => ThreadStatus::Idle,
                    },
                    step_count: t.step_count,
                    total_tokens: t.total_tokens,
                    started_at: parse_engine_thread_timestamp(&t.created_at, "created_at", &t.id),
                    updated_at: parse_engine_thread_timestamp(&t.updated_at, "updated_at", &t.id),
                })
                .collect();
        }

        TuiEvent::EngineThreadDetail { detail } => {
            state.tool_detail_modal = Some(ToolDetailModal {
                tool_name: format!("Thread {}", detail.thread_type),
                content: format_engine_thread_detail(&detail),
                scroll: 0,
            });
        }

        TuiEvent::ConversationHistory {
            thread_id,
            messages,
            pending_approval,
        } => {
            state.current_thread_id = Some(thread_id.clone());
            state.messages.clear();
            state.active_tools.clear();
            state.recent_tools.clear();
            state.is_streaming = false;
            state.status_text.clear();
            state.pending_approval = pending_approval.map(|approval| ApprovalRequest {
                request_id: approval.request_id,
                tool_name: approval.tool_name,
                description: approval.description,
                parameters: approval.parameters,
                allow_always: approval.allow_always,
                selected: 0,
            });
            state.suggestions.clear();
            for thread in &mut state.threads {
                thread.is_foreground = thread.id == thread_id;
                thread.status = if thread.is_foreground {
                    ThreadStatus::Active
                } else {
                    ThreadStatus::Idle
                };
            }

            for msg in &messages {
                let role = match msg.role.as_str() {
                    "user" => MessageRole::User,
                    "assistant" => MessageRole::Assistant,
                    _ => MessageRole::System,
                };
                state.messages.push(ChatMessage {
                    role,
                    content: msg.content.clone(),
                    timestamp: msg.timestamp,
                    cost_summary: None,
                });
            }

            state.pinned_to_bottom = true;
            state.toasts.push(Toast {
                message: format!("Resumed conversation ({} messages)", state.messages.len()),
                kind: ToastKind::Info,
                created_at: chrono::Utc::now(),
            });
        }
    }
}

fn resolve_key_action(
    key: event::KeyEvent,
    state: &AppState,
    widgets: &BuiltinWidgets,
) -> InputAction {
    let approval_active = state.pending_approval.is_some();
    let palette_active = state.command_palette.visible || state.model_picker.visible;
    let search_active = state.search.active;
    let help_active = state.help_visible;
    let tool_detail_active = state.tool_detail_modal.is_some();
    let logs_active = state.active_tab == ActiveTab::Logs;
    let thread_picker_active = state.pending_thread_picker.is_some();

    let action = map_key(
        key,
        approval_active,
        palette_active,
        search_active,
        help_active,
        tool_detail_active,
        logs_active,
        thread_picker_active,
    );

    if action != InputAction::Forward {
        return action;
    }

    if key.modifiers != KeyModifiers::NONE
        || approval_active
        || palette_active
        || search_active
        || help_active
        || tool_detail_active
        || thread_picker_active
    {
        return InputAction::Forward;
    }

    match key.code {
        KeyCode::Up if widgets.input_box.is_cursor_on_first_line() => InputAction::HistoryUp,
        KeyCode::Down
            if state.history_index.is_some() || widgets.input_box.is_cursor_on_last_line() =>
        {
            InputAction::HistoryDown
        }
        _ => InputAction::Forward,
    }
}

fn tool_activity_matches(tool: &ToolActivity, name: &str, call_id: Option<&str>) -> bool {
    match call_id {
        Some(call_id) => tool.call_id.as_deref() == Some(call_id),
        None => tool.name == name,
    }
}

fn parse_model_list_response(content: &str) -> Option<(String, Vec<String>)> {
    let mut lines = content.lines();
    let active_model = lines
        .next()?
        .strip_prefix("Active model: ")?
        .trim()
        .to_string();

    let mut in_model_section = false;
    let mut models = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "Available models:" {
            in_model_section = true;
            continue;
        }

        if !in_model_section {
            continue;
        }

        if trimmed.is_empty() || trimmed.starts_with("Use /model ") {
            break;
        }

        let model = trimmed
            .strip_suffix(" (active)")
            .unwrap_or(trimmed)
            .trim()
            .to_string();
        if !model.is_empty() {
            models.push(model);
        }
    }

    if models.is_empty() {
        None
    } else {
        Some((active_model, models))
    }
}

fn format_detail_timestamp(raw: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&chrono::Local))
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S %Z").to_string())
        .unwrap_or_else(|_| raw.to_string())
}

fn format_engine_thread_detail(detail: &crate::event::EngineThreadDetailEntry) -> String {
    use std::fmt::Write as _;

    let mut content = String::new();
    let _ = writeln!(content, "Goal");
    let _ = writeln!(content, "{}", detail.goal);
    let _ = writeln!(content);

    let _ = writeln!(content, "Overview");
    let _ = writeln!(content, "  Thread ID: {}", detail.id);
    let _ = writeln!(content, "  Type: {}", detail.thread_type);
    let _ = writeln!(content, "  State: {}", detail.state);
    let _ = writeln!(content, "  Steps: {}", detail.step_count);
    let _ = writeln!(content, "  Tokens: {}", detail.total_tokens);
    let _ = writeln!(content, "  Cost: ${:.4}", detail.total_cost_usd);
    let _ = writeln!(content, "  Max iterations: {}", detail.max_iterations);
    let _ = writeln!(
        content,
        "  Created: {}",
        format_detail_timestamp(&detail.created_at)
    );
    let _ = writeln!(
        content,
        "  Updated: {}",
        format_detail_timestamp(&detail.updated_at)
    );
    let completed = detail
        .completed_at
        .as_deref()
        .map(format_detail_timestamp)
        .unwrap_or_else(|| "-".to_string());
    let _ = writeln!(content, "  Completed: {completed}");
    let _ = writeln!(content, "  Project: {}", detail.project_id);
    let _ = writeln!(
        content,
        "  Parent: {}",
        detail.parent_id.as_deref().unwrap_or("-")
    );

    if detail.messages.is_empty() {
        return content;
    }

    let _ = writeln!(content);
    let _ = writeln!(content, "Messages ({})", detail.messages.len());
    for message in &detail.messages {
        let _ = writeln!(
            content,
            "\n[{}] {}",
            message.role,
            format_detail_timestamp(&message.timestamp)
        );
        let _ = writeln!(content, "{}", message.content);
    }

    content
}

struct TerminalRestoreGuard {
    active: bool,
}

impl TerminalRestoreGuard {
    fn new() -> Self {
        Self { active: true }
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for TerminalRestoreGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }

        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            Show,
            DisableBracketedPaste,
            ratatui::crossterm::event::DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = stdout.flush();
    }
}

#[cfg(test)]
fn terminal_area() -> Rect {
    Rect::new(0, 0, 80, 24)
}

#[cfg(not(test))]
fn terminal_area() -> Rect {
    ratatui::crossterm::terminal::size()
        .map(|(width, height)| Rect::new(0, 0, width, height))
        .unwrap_or_else(|_| Rect::new(0, 0, 80, 24))
}

#[cfg(test)]
static LAST_COPIED_TEXT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

#[cfg(test)]
fn take_last_copied_text_for_test() -> Option<String> {
    LAST_COPIED_TEXT
        .lock()
        .expect("copied text mutex poisoned")
        .take()
}

fn copy_text_to_clipboard(text: &str) -> bool {
    #[cfg(test)]
    {
        *LAST_COPIED_TEXT.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.to_string());
        true
    }

    #[cfg(all(not(test), feature = "clipboard"))]
    {
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(text.to_string()))
            .is_ok()
    }

    #[cfg(all(not(test), not(feature = "clipboard")))]
    {
        let _ = text;
        false
    }
}

async fn handle_mouse_click(
    column: u16,
    row: u16,
    state: &mut AppState,
    msg_tx: &mpsc::Sender<TuiUserMessage>,
    layout: &TuiLayout,
) {
    let terminal = terminal_area();

    if let Some(ref approval) = state.pending_approval
        && let Some(action) = approval_action_at(terminal, approval, column, row)
    {
        let _ = msg_tx
            .send(
                TuiUserMessage::text_only(action.as_response())
                    .with_thread_id(state.current_thread_id.clone()),
            )
            .await;
        state.pending_approval = None;
        state.text_selection = None;
        return;
    }

    if let Some(ref picker) = state.pending_thread_picker {
        if let Some(index) = thread_picker_index_at(terminal, picker, column, row) {
            if let Some(thread) = picker.threads.get(index) {
                let _ = msg_tx
                    .send(
                        TuiUserMessage::text_only(format!("/thread {}", thread.id))
                            .with_thread_id(None),
                    )
                    .await;
                state.current_thread_id = Some(thread.id.clone());
            }
            state.pending_thread_picker = None;
            state.text_selection = None;
            return;
        }

        if !rect_contains(
            ThreadPickerWidget::modal_area(terminal, picker.threads.len()),
            column,
            row,
        ) {
            state.pending_thread_picker = None;
        }
        state.text_selection = None;
        return;
    }

    if state.help_visible {
        state.help_visible = false;
        state.text_selection = None;
        return;
    }

    if let Some(tab) = tab_at(terminal, layout, state, column, row) {
        state.active_tab = tab;
        state.text_selection = None;
        return;
    }

    if state.tool_detail_modal.is_none()
        && let Some(area) = thread_list_sidebar_area(terminal, layout, state)
        && let Some(index) = engine_thread_index_at(area, state, column, row)
        && let Some(thread) = state.engine_threads.get(index)
    {
        let _ = msg_tx
            .send(TuiUserMessage::open_engine_thread_detail(thread.id.clone()))
            .await;
        state.text_selection = None;
        return;
    }

    if let Some(bounds) = selectable_area_at(terminal, layout, state, column, row) {
        state.text_selection = Some(TextSelection {
            anchor: SelectionPoint { column, row },
            focus: SelectionPoint { column, row },
            bounds,
        });
        return;
    }

    state.text_selection = None;
    if state.tool_detail_modal.is_some()
        && !rect_contains(tool_detail_modal_area(terminal), column, row)
    {
        state.tool_detail_modal = None;
    }
}

fn handle_mouse_drag(column: u16, row: u16, state: &mut AppState) {
    if let Some(ref mut selection) = state.text_selection {
        selection.focus = clamp_point_to_rect(SelectionPoint { column, row }, selection.bounds);
    }
}

fn handle_mouse_release(column: u16, row: u16, state: &mut AppState) {
    let Some(ref mut selection) = state.text_selection else {
        return;
    };

    selection.focus = clamp_point_to_rect(SelectionPoint { column, row }, selection.bounds);

    if selection.anchor == selection.focus {
        state.text_selection = None;
        return;
    }

    let text = extract_selected_text(&state.screen_snapshot, selection);
    if text.is_empty() {
        state.text_selection = None;
        return;
    }

    let copied = copy_text_to_clipboard(&text);
    state.toasts.push(Toast {
        message: if copied {
            format!("Copied {} chars", text.chars().count())
        } else {
            "Copy failed".to_string()
        },
        kind: if copied {
            ToastKind::Success
        } else {
            ToastKind::Error
        },
        created_at: chrono::Utc::now(),
    });
}

fn frame_sections(size: Rect, layout: &TuiLayout, state: &AppState) -> [Rect; 5] {
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

    [
        vertical[0],
        vertical[1],
        vertical[2],
        vertical[3],
        vertical[4],
    ]
}

fn tab_at(
    size: Rect,
    layout: &TuiLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<ActiveTab> {
    let tab_bar_area = frame_sections(size, layout, state)[1];
    if !rect_contains(tab_bar_area, column, row) {
        return None;
    }

    let relative_x = column.saturating_sub(tab_bar_area.x);
    if (2..6).contains(&relative_x) {
        Some(ActiveTab::Conversation)
    } else if (8..12).contains(&relative_x) {
        Some(ActiveTab::Logs)
    } else {
        None
    }
}

fn selectable_area_at(
    size: Rect,
    layout: &TuiLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<Rect> {
    if state.tool_detail_modal.is_some() {
        let inner = tool_detail_inner_area(tool_detail_modal_area(size));
        if rect_contains(inner, column, row) {
            return Some(inner);
        }
        return None;
    }

    let main_area = frame_sections(size, layout, state)[2];
    let selectable = match state.active_tab {
        ActiveTab::Logs => main_area,
        ActiveTab::Conversation => {
            if state.sidebar_visible && main_area.width > 40 {
                let sidebar_width =
                    (main_area.width as u32 * layout.sidebar.effective_width() as u32 / 100) as u16;
                let conversation_width = main_area.width.saturating_sub(sidebar_width + 1);

                Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Length(conversation_width),
                        Constraint::Length(1),
                        Constraint::Length(sidebar_width),
                    ])
                    .split(main_area)[0]
            } else {
                main_area
            }
        }
    };

    rect_contains(selectable, column, row).then_some(selectable)
}

fn thread_list_sidebar_area(size: Rect, layout: &TuiLayout, state: &AppState) -> Option<Rect> {
    if state.active_tab != ActiveTab::Conversation || !state.sidebar_visible {
        return None;
    }

    let main_area = frame_sections(size, layout, state)[2];
    if main_area.width <= 40 {
        return None;
    }

    let sidebar_width =
        (main_area.width as u32 * layout.sidebar.effective_width() as u32 / 100) as u16;
    let conversation_width = main_area.width.saturating_sub(sidebar_width + 1);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(conversation_width),
            Constraint::Length(1),
            Constraint::Length(sidebar_width),
        ])
        .split(main_area);
    let sidebar_area = horizontal[2];
    let sidebar_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(sidebar_area);
    Some(sidebar_split[1])
}

fn approval_action_at(
    size: Rect,
    approval: &ApprovalRequest,
    column: u16,
    row: u16,
) -> Option<ApprovalAction> {
    let area = ApprovalWidget::modal_area(size);
    if !rect_contains(area, column, row) {
        return None;
    }

    let params_count = approval
        .parameters
        .as_object()
        .map(|obj: &serde_json::Map<String, serde_json::Value>| obj.len().min(4) as u16)
        .unwrap_or(0);
    let options_start_y = area.y + 1 + 3 + params_count;
    let options = ApprovalWidget::options(approval.allow_always);
    let index = row.checked_sub(options_start_y)? as usize;
    options.get(index).copied()
}

fn thread_picker_index_at(
    size: Rect,
    picker: &crate::widgets::ThreadPickerState,
    column: u16,
    row: u16,
) -> Option<usize> {
    let area = ThreadPickerWidget::modal_area(size, picker.threads.len());
    if !rect_contains(area, column, row) {
        return None;
    }

    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    if inner.height < 2 || row >= inner.y + inner.height.saturating_sub(1) {
        return None;
    }

    let list_height = inner.height.saturating_sub(1) as usize;
    let scroll_offset = if picker.selected >= list_height {
        picker.selected - list_height + 1
    } else {
        0
    };

    let row_index = row.checked_sub(inner.y)? as usize;
    let thread_index = scroll_offset + row_index;
    picker.threads.get(thread_index)?;
    Some(thread_index)
}

fn tool_detail_modal_area(size: Rect) -> Rect {
    let width = (size.width * 3 / 4)
        .max(40)
        .min(size.width.saturating_sub(4));
    let height = (size.height * 3 / 4)
        .max(10)
        .min(size.height.saturating_sub(4));
    let x = (size.width.saturating_sub(width)) / 2;
    let y = (size.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

fn tool_detail_inner_area(size: Rect) -> Rect {
    Rect::new(
        size.x.saturating_add(1),
        size.y.saturating_add(1),
        size.width.saturating_sub(2),
        size.height.saturating_sub(2),
    )
}

fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x && column < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}

fn clamp_point_to_rect(point: SelectionPoint, bounds: Rect) -> SelectionPoint {
    let max_column = bounds.x + bounds.width.saturating_sub(1);
    let max_row = bounds.y + bounds.height.saturating_sub(1);
    SelectionPoint {
        column: point.column.clamp(bounds.x, max_column),
        row: point.row.clamp(bounds.y, max_row),
    }
}

fn normalize_selection(selection: &TextSelection) -> (SelectionPoint, SelectionPoint) {
    if selection.anchor.row < selection.focus.row
        || (selection.anchor.row == selection.focus.row
            && selection.anchor.column <= selection.focus.column)
    {
        (selection.anchor, selection.focus)
    } else {
        (selection.focus, selection.anchor)
    }
}

fn extract_selected_text(snapshot: &ScreenSnapshot, selection: &TextSelection) -> String {
    let (start, end) = normalize_selection(selection);
    let mut lines = Vec::new();

    for row in start.row..=end.row {
        let start_col = if row == start.row {
            start.column
        } else {
            selection.bounds.x
        };
        let end_col = if row == end.row {
            end.column
        } else {
            selection.bounds.x + selection.bounds.width.saturating_sub(1)
        };

        let mut line = String::new();
        for column in start_col..=end_col {
            if let Some(symbol) = snapshot_symbol(snapshot, column, row) {
                line.push_str(symbol);
            }
        }
        lines.push(line.trim_end().to_string());
    }

    lines.join("\n").trim_end_matches('\n').to_string()
}

fn snapshot_symbol(snapshot: &ScreenSnapshot, column: u16, row: u16) -> Option<&str> {
    if !rect_contains(snapshot.area, column, row) {
        return None;
    }

    Some(snapshot.buffer[(column, row)].symbol())
}

/// Render a single frame.
fn render_frame(
    frame: &mut ratatui::Frame<'_>,
    state: &mut AppState,
    widgets: &BuiltinWidgets,
    layout: &TuiLayout,
) {
    let size = frame.area();
    let [
        header_area,
        tab_bar_area,
        main_area,
        input_area,
        status_area,
    ] = frame_sections(size, layout, state);

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

    // Track conversation area height for page-scroll calculations
    state.conversation_height = main_area.height;

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
            // Sync max_scroll_offset from the last render for scroll clamping
            state.max_scroll_offset = widgets.conversation.last_max_offset();
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

    if state.model_picker.visible {
        let modal_area = ModelPickerWidget::modal_area(size, state.model_picker.filtered.len());
        widgets
            .model_picker
            .render_picker(modal_area, frame.buffer_mut(), state);
    }

    // Approval modal (rendered on top of everything)
    if state.pending_approval.is_some() {
        let modal_area = ApprovalWidget::modal_area(size);
        widgets
            .approval
            .render(modal_area, frame.buffer_mut(), state);
    }

    // Thread picker modal (/resume)
    if let Some(ref picker) = state.pending_thread_picker {
        let modal_area = crate::widgets::thread_picker::ThreadPickerWidget::modal_area(
            size,
            picker.threads.len(),
        );
        widgets
            .thread_picker
            .render_picker(modal_area, frame.buffer_mut(), state);
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

    render_text_selection(frame, state, layout);

    // Notification toasts (bottom-right, above status bar)
    render_toasts(frame, size, state, layout);

    capture_screen_snapshot(frame, state);
}

/// Check input text and update slash-command overlays.
fn update_input_overlays_from_input(
    input_box: &crate::widgets::input_box::InputBoxWidget,
    state: &mut AppState,
) {
    let text = input_box.current_text();
    let trimmed = text.trim();

    if state.model_picker.has_models() && (trimmed == "/model" || trimmed.starts_with("/model ")) {
        let filter = trimmed
            .split_once(' ')
            .map(|(_, rest)| rest.trim())
            .unwrap_or("");
        state.command_palette.close();
        state.model_picker.open(filter);
        return;
    }

    state.model_picker.close();

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
    use ratatui::text::Span;
    use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

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

    let lines = crate::render::render_markdown(&modal.content, inner.width as usize, &theme);
    let total_lines = lines.len();
    let visible = inner.height as usize;

    let paragraph = Paragraph::new(lines).scroll((modal.scroll, 0));
    paragraph.render(inner, frame.buffer_mut());

    // Render scrollbar when content exceeds viewport
    if total_lines > visible {
        use rat_scrolled::{Scroll, ScrollState};
        use ratatui::widgets::StatefulWidget;
        let mut ss = ScrollState::default();
        ss.set_offset(modal.scroll as usize);
        ss.set_page_len(visible);
        ss.set_max_offset(total_lines.saturating_sub(visible));
        Scroll::vertical()
            .begin_symbol(None)
            .end_symbol(None)
            .render(inner, frame.buffer_mut(), &mut ss);
    }
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

fn render_text_selection(frame: &mut ratatui::Frame<'_>, state: &AppState, layout: &TuiLayout) {
    let Some(ref selection) = state.text_selection else {
        return;
    };

    let (start, end) = normalize_selection(selection);
    let theme = layout.resolve_theme();
    let selection_style = ratatui::style::Style::default()
        .bg(theme.accent.to_color())
        .fg(ratatui::style::Color::Black);

    for row in start.row..=end.row {
        let start_col = if row == start.row {
            start.column
        } else {
            selection.bounds.x
        };
        let end_col = if row == end.row {
            end.column
        } else {
            selection.bounds.x + selection.bounds.width.saturating_sub(1)
        };

        for column in start_col..=end_col {
            if let Some(cell) = frame.buffer_mut().cell_mut((column, row)) {
                cell.set_style(selection_style);
            }
        }
    }
}

fn capture_screen_snapshot(frame: &mut ratatui::Frame<'_>, state: &mut AppState) {
    state.screen_snapshot = ScreenSnapshot {
        area: frame.area(),
        buffer: frame.buffer_mut().clone(),
    };
}

/// Try to read an image from the system clipboard and return it as a PNG-encoded
/// [`TuiAttachment`]. Returns `None` if the clipboard has no image data or if
/// encoding fails.
#[cfg(feature = "clipboard")]
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

#[cfg(not(feature = "clipboard"))]
fn try_paste_clipboard_image(_state: &AppState) -> Option<TuiAttachment> {
    None
}

/// Encode raw RGBA pixel data to PNG. Returns `None` on invalid dimensions or
/// encoding failure.
#[cfg(feature = "clipboard")]
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
    use crate::event::{HistoryMessage, ThreadEntry};
    use crate::widgets::approval::ApprovalWidget;
    use crate::widgets::registry::create_default_widgets;
    use crate::widgets::thread_picker::ThreadPickerWidget;
    use crate::widgets::{ActiveTab, ApprovalRequest, MessageRole, ThreadStatus};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::layout::Rect;

    async fn apply_event(state: &mut AppState, event: TuiEvent) {
        let layout = TuiLayout::default();
        let mut widgets = create_default_widgets(&layout);
        let (msg_tx, _msg_rx) = mpsc::channel(4);
        handle_event(event, state, &mut widgets, &msg_tx, &layout).await;
    }

    async fn apply_event_and_take_messages(
        state: &mut AppState,
        event: TuiEvent,
    ) -> Vec<TuiUserMessage> {
        let layout = TuiLayout::default();
        let mut widgets = create_default_widgets(&layout);
        let (msg_tx, mut msg_rx) = mpsc::channel(4);
        handle_event(event, state, &mut widgets, &msg_tx, &layout).await;

        let mut messages = Vec::new();
        while let Ok(message) = msg_rx.try_recv() {
            messages.push(message);
        }
        messages
    }

    fn make_snapshot(width: u16, height: u16) -> ScreenSnapshot {
        let area = Rect::new(0, 0, width, height);
        ScreenSnapshot {
            area,
            buffer: ratatui::buffer::Buffer::empty(area),
        }
    }

    fn write_snapshot_text(snapshot: &mut ScreenSnapshot, column: u16, row: u16, text: &str) {
        for (offset, ch) in text.chars().enumerate() {
            snapshot.buffer[(column + offset as u16, row)].set_symbol(&ch.to_string());
        }
    }

    #[cfg(feature = "clipboard")]
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

    #[cfg(feature = "clipboard")]
    #[test]
    fn encode_rgba_to_png_bad_dimensions() {
        let rgba = vec![0u8; 16]; // 4 pixels
        // Claim 3x2 = 6 pixels, but only 4 are provided
        let png = encode_rgba_to_png(&rgba, 3, 2);
        assert!(png.is_none());
    }

    #[cfg(feature = "clipboard")]
    #[test]
    fn encode_rgba_to_png_zero_size() {
        // 0x0 image: the image crate rejects zero-dimension buffers
        let png = encode_rgba_to_png(&[], 0, 0);
        assert!(png.is_none());
    }

    #[tokio::test]
    async fn response_appends_after_existing_assistant_message_when_not_streaming() {
        let mut state = AppState::default();
        state.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: "first reply".to_string(),
            timestamp: chrono::Utc::now(),
            cost_summary: None,
        });

        apply_event(
            &mut state,
            TuiEvent::Response {
                content: "background notification".to_string(),
                thread_id: None,
            },
        )
        .await;

        assert_eq!(state.messages.len(), 2);
        assert_eq!(state.messages[1].content, "background notification");
    }

    #[tokio::test]
    async fn response_tracks_active_thread_id() {
        let mut state = AppState::default();

        apply_event(
            &mut state,
            TuiEvent::Response {
                content: "ok".to_string(),
                thread_id: Some("thread-42".to_string()),
            },
        )
        .await;

        assert_eq!(state.current_thread_id.as_deref(), Some("thread-42"));
    }

    #[tokio::test]
    async fn thread_list_only_populates_picker() {
        let mut state = AppState::default();

        apply_event(
            &mut state,
            TuiEvent::ThreadList {
                threads: vec![ThreadEntry {
                    id: "thread-1".to_string(),
                    title: Some("Bug bash".to_string()),
                    message_count: 3,
                    last_activity: "2026-04-03 12:00".to_string(),
                    channel: "repl".to_string(),
                }],
            },
        )
        .await;

        // ThreadList no longer populates the sidebar — only the picker.
        assert!(state.engine_threads.is_empty());
        assert!(state.pending_thread_picker.is_some());
        assert_eq!(
            state.pending_thread_picker.as_ref().unwrap().threads.len(),
            1
        );
    }

    #[tokio::test]
    async fn engine_thread_list_updates_sidebar() {
        let mut state = AppState::default();

        apply_event(
            &mut state,
            TuiEvent::EngineThreadList {
                threads: vec![crate::event::EngineThreadEntry {
                    id: "eng-1".to_string(),
                    goal: "fix login".to_string(),
                    thread_type: "Foreground".to_string(),
                    state: "Running".to_string(),
                    step_count: 3,
                    total_tokens: 800,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                }],
            },
        )
        .await;

        assert_eq!(state.engine_threads.len(), 1);
        assert_eq!(state.engine_threads[0].goal, "fix login");
        assert_eq!(state.engine_threads[0].thread_type, "Foreground");
        assert_eq!(state.engine_threads[0].status, ThreadStatus::Active);
    }

    #[tokio::test]
    async fn engine_thread_detail_opens_modal() {
        let mut state = AppState::default();

        apply_event(
            &mut state,
            TuiEvent::EngineThreadDetail {
                detail: crate::event::EngineThreadDetailEntry {
                    id: "eng-1".to_string(),
                    goal: "Send the top three Hacker News stories".to_string(),
                    thread_type: "Mission".to_string(),
                    state: "Running".to_string(),
                    project_id: "proj-1".to_string(),
                    parent_id: None,
                    step_count: 7,
                    total_tokens: 2_048,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                    max_iterations: 24,
                    completed_at: None,
                    total_cost_usd: 0.1234,
                    messages: vec![crate::event::EngineThreadMessageEntry {
                        role: "Assistant".to_string(),
                        content: "Fetching the latest stories.".to_string(),
                        timestamp: chrono::Utc::now().to_rfc3339(),
                    }],
                },
            },
        )
        .await;

        let modal = state
            .tool_detail_modal
            .as_ref()
            .expect("thread detail modal should open");
        assert_eq!(modal.tool_name, "Thread Mission");
        assert!(modal.content.contains("Goal"));
        assert!(
            modal
                .content
                .contains("Send the top three Hacker News stories")
        );
        assert!(modal.content.contains("Messages (1)"));
        assert!(modal.content.contains("Fetching the latest stories."));
    }

    #[tokio::test]
    async fn empty_thread_list_clears_picker() {
        let mut state = AppState::default();

        apply_event(
            &mut state,
            TuiEvent::ThreadList {
                threads: vec![ThreadEntry {
                    id: "thread-1".to_string(),
                    title: Some("Bug bash".to_string()),
                    message_count: 3,
                    last_activity: "2026-04-03 12:00".to_string(),
                    channel: "repl".to_string(),
                }],
            },
        )
        .await;
        assert!(state.pending_thread_picker.is_some());

        apply_event(&mut state, TuiEvent::ThreadList { threads: vec![] }).await;

        assert!(state.pending_thread_picker.is_none());
    }

    #[tokio::test]
    async fn job_events_do_not_populate_thread_sidebar() {
        let mut state = AppState::default();

        apply_event(
            &mut state,
            TuiEvent::JobStarted {
                job_id: "job-1".to_string(),
                title: "Backfill".to_string(),
            },
        )
        .await;

        assert_eq!(state.jobs.len(), 1);
        assert!(state.threads.is_empty());
    }

    #[tokio::test]
    async fn mouse_scroll_moves_thread_picker_selection() {
        let mut state = AppState {
            pending_thread_picker: Some(crate::widgets::ThreadPickerState {
                threads: vec![
                    ThreadEntry {
                        id: "thread-1".to_string(),
                        title: Some("Bug bash".to_string()),
                        message_count: 3,
                        last_activity: "2026-04-03 12:00".to_string(),
                        channel: "repl".to_string(),
                    },
                    ThreadEntry {
                        id: "thread-2".to_string(),
                        title: Some("Release prep".to_string()),
                        message_count: 8,
                        last_activity: "2026-04-03 13:00".to_string(),
                        channel: "repl".to_string(),
                    },
                ],
                selected: 0,
            }),
            ..Default::default()
        };

        apply_event(&mut state, TuiEvent::MouseScroll(3)).await;

        assert_eq!(
            state
                .pending_thread_picker
                .as_ref()
                .map(|picker| picker.selected),
            Some(1)
        );
    }

    #[tokio::test]
    async fn mouse_click_switches_active_tab() {
        let mut state = AppState::default();

        apply_event(&mut state, TuiEvent::MouseClick { column: 9, row: 0 }).await;

        assert_eq!(state.active_tab, ActiveTab::Logs);
    }

    #[tokio::test]
    async fn mouse_click_engine_thread_row_requests_detail_modal_data() {
        let now = chrono::Utc::now();
        let mut state = AppState {
            engine_threads: vec![EngineThreadInfo {
                id: "eng-1".to_string(),
                goal: "Check Hacker News hourly".to_string(),
                thread_type: "Mission".to_string(),
                status: ThreadStatus::Active,
                step_count: 5,
                total_tokens: 4_096,
                started_at: Some(now - chrono::Duration::minutes(9)),
                updated_at: Some(now),
            }],
            ..Default::default()
        };

        let layout = TuiLayout::default();
        let area = thread_list_sidebar_area(Rect::new(0, 0, 80, 24), &layout, &state)
            .expect("thread list area should exist");
        let click = (area.y..area.y + area.height)
            .find_map(|row| {
                (area.x..area.x + area.width).find_map(|column| {
                    (engine_thread_index_at(area, &state, column, row) == Some(0))
                        .then_some((column, row))
                })
            })
            .expect("expected a clickable engine thread row");

        let messages = apply_event_and_take_messages(
            &mut state,
            TuiEvent::MouseClick {
                column: click.0,
                row: click.1,
            },
        )
        .await;

        assert_eq!(messages.len(), 1);
        assert!(messages[0].text.is_empty());
        assert!(messages[0].thread_id.is_none());
        match &messages[0].ui_action {
            Some(crate::event::TuiUiAction::OpenEngineThreadDetail { thread_id }) => {
                assert_eq!(thread_id, "eng-1");
            }
            other => panic!("expected engine thread detail action, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn mouse_click_approval_option_submits_response() {
        let mut state = AppState {
            pending_approval: Some(ApprovalRequest {
                request_id: "req-1".to_string(),
                tool_name: "shell".to_string(),
                description: "Run a command".to_string(),
                parameters: serde_json::json!({}),
                allow_always: false,
                selected: 0,
            }),
            ..Default::default()
        };

        let area = ApprovalWidget::modal_area(Rect::new(0, 0, 80, 24));
        let messages = apply_event_and_take_messages(
            &mut state,
            TuiEvent::MouseClick {
                column: area.x + 3,
                row: area.y + 5,
            },
        )
        .await;

        assert!(state.pending_approval.is_none());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, "n");
    }

    #[tokio::test]
    async fn conversation_history_restores_pending_approval() {
        let mut state = AppState {
            pending_approval: Some(ApprovalRequest {
                request_id: "stale".to_string(),
                tool_name: "old-tool".to_string(),
                description: "stale approval".to_string(),
                parameters: serde_json::json!({"old": true}),
                allow_always: false,
                selected: 2,
            }),
            ..Default::default()
        };

        apply_event(
            &mut state,
            TuiEvent::ConversationHistory {
                thread_id: "thread-1".to_string(),
                messages: vec![HistoryMessage {
                    role: "assistant".to_string(),
                    content: "Waiting on approval".to_string(),
                    timestamp: chrono::Utc::now(),
                }],
                pending_approval: Some(crate::event::HistoryApprovalRequest {
                    request_id: "req-1".to_string(),
                    tool_name: "shell".to_string(),
                    description: "Run a command".to_string(),
                    parameters: serde_json::json!({"command": "[REDACTED]"}),
                    allow_always: true,
                }),
            },
        )
        .await;

        let approval = state
            .pending_approval
            .as_ref()
            .expect("pending approval should be restored");
        assert_eq!(approval.request_id, "req-1");
        assert_eq!(approval.tool_name, "shell");
        assert_eq!(approval.description, "Run a command");
        assert_eq!(
            approval.parameters,
            serde_json::json!({"command": "[REDACTED]"})
        );
        assert!(approval.allow_always);
        assert_eq!(approval.selected, 0);
    }

    #[tokio::test]
    async fn mouse_click_thread_picker_row_resumes_thread() {
        let mut state = AppState {
            pending_thread_picker: Some(crate::widgets::ThreadPickerState {
                threads: vec![
                    ThreadEntry {
                        id: "thread-1".to_string(),
                        title: Some("Bug bash".to_string()),
                        message_count: 3,
                        last_activity: "2026-04-03 12:00".to_string(),
                        channel: "repl".to_string(),
                    },
                    ThreadEntry {
                        id: "thread-2".to_string(),
                        title: Some("Release prep".to_string()),
                        message_count: 8,
                        last_activity: "2026-04-03 13:00".to_string(),
                        channel: "repl".to_string(),
                    },
                ],
                selected: 0,
            }),
            ..Default::default()
        };

        let area = ThreadPickerWidget::modal_area(Rect::new(0, 0, 80, 24), 2);
        let messages = apply_event_and_take_messages(
            &mut state,
            TuiEvent::MouseClick {
                column: area.x + 3,
                row: area.y + 2,
            },
        )
        .await;

        assert!(state.pending_thread_picker.is_none());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, "/thread thread-2");
        assert!(messages[0].thread_id.is_none());
        assert_eq!(state.current_thread_id.as_deref(), Some("thread-2"));
    }

    #[tokio::test]
    async fn mouse_drag_and_release_copies_selected_text() {
        let mut state = AppState {
            active_tab: ActiveTab::Logs,
            screen_snapshot: make_snapshot(80, 24),
            ..Default::default()
        };
        write_snapshot_text(&mut state.screen_snapshot, 1, 2, "hello world");
        take_last_copied_text_for_test();

        apply_event(&mut state, TuiEvent::MouseClick { column: 1, row: 2 }).await;
        apply_event(&mut state, TuiEvent::MouseDrag { column: 5, row: 2 }).await;
        apply_event(&mut state, TuiEvent::MouseRelease { column: 5, row: 2 }).await;

        assert_eq!(take_last_copied_text_for_test().as_deref(), Some("hello"));
        assert!(state.text_selection.is_some());
    }

    #[test]
    fn extract_selected_text_preserves_multiline_range() {
        let mut snapshot = make_snapshot(20, 4);
        write_snapshot_text(&mut snapshot, 0, 1, "first line");
        write_snapshot_text(&mut snapshot, 0, 2, "second line");

        let selection = TextSelection {
            anchor: SelectionPoint { column: 2, row: 1 },
            focus: SelectionPoint { column: 5, row: 2 },
            bounds: Rect::new(0, 1, 20, 2),
        };

        assert_eq!(
            extract_selected_text(&snapshot, &selection),
            "rst line\nsecond"
        );
    }

    #[test]
    fn parse_engine_thread_timestamp_accepts_rfc3339_and_legacy_format() {
        let rfc3339 =
            parse_engine_thread_timestamp("2026-04-06T05:56:16Z", "created_at", "thread-1");
        let legacy = parse_engine_thread_timestamp("2026-04-06 05:56", "created_at", "thread-1");

        assert_eq!(
            rfc3339,
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-04-06T05:56:16Z")
                    .expect("valid rfc3339")
                    .with_timezone(&chrono::Utc)
            )
        );
        assert_eq!(
            legacy,
            Some(
                chrono::NaiveDateTime::parse_from_str("2026-04-06 05:56", "%Y-%m-%d %H:%M")
                    .expect("valid legacy timestamp")
                    .and_utc()
            )
        );
    }

    #[test]
    fn parse_engine_thread_timestamp_returns_none_for_invalid_input() {
        assert_eq!(
            parse_engine_thread_timestamp("not-a-timestamp", "created_at", "thread-1"),
            None
        );
    }

    async fn apply_event_with_widgets(
        state: &mut AppState,
        widgets: &mut BuiltinWidgets,
        event: TuiEvent,
    ) {
        let layout = TuiLayout::default();
        let (msg_tx, _msg_rx) = mpsc::channel(4);
        handle_event(event, state, widgets, &msg_tx, &layout).await;
    }

    #[tokio::test]
    async fn up_arrow_recalls_latest_history_from_input_bar() {
        let mut state = AppState {
            input_history: vec!["first prompt".to_string(), "latest prompt".to_string()],
            ..Default::default()
        };
        let layout = TuiLayout::default();
        let mut widgets = create_default_widgets(&layout);

        apply_event_with_widgets(
            &mut state,
            &mut widgets,
            TuiEvent::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        )
        .await;

        assert_eq!(widgets.input_box.current_text(), "latest prompt");
        assert_eq!(state.history_index, Some(1));
    }

    #[tokio::test]
    async fn up_arrow_inside_multiline_draft_keeps_editing_instead_of_history() {
        let mut state = AppState {
            input_history: vec!["latest prompt".to_string()],
            ..Default::default()
        };
        let layout = TuiLayout::default();
        let mut widgets = create_default_widgets(&layout);
        widgets.input_box.set_text("first line\nsecond line");
        widgets
            .input_box
            .handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &mut state);

        apply_event_with_widgets(
            &mut state,
            &mut widgets,
            TuiEvent::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        )
        .await;

        assert_eq!(widgets.input_box.current_text(), "first line\nsecond line");
        assert_eq!(state.history_index, None);
    }

    #[tokio::test]
    async fn down_arrow_restores_draft_after_history_recall() {
        let mut state = AppState {
            input_history: vec!["older prompt".to_string()],
            ..Default::default()
        };
        let layout = TuiLayout::default();
        let mut widgets = create_default_widgets(&layout);
        widgets.input_box.set_text("draft prompt");

        apply_event_with_widgets(
            &mut state,
            &mut widgets,
            TuiEvent::Key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)),
        )
        .await;
        apply_event_with_widgets(
            &mut state,
            &mut widgets,
            TuiEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        )
        .await;

        assert_eq!(widgets.input_box.current_text(), "draft prompt");
        assert_eq!(state.history_index, None);
    }

    #[test]
    fn slash_model_opens_model_picker_instead_of_command_palette() {
        let mut state = AppState::default();
        state.model_picker.set_models(vec![
            "gpt-4o".to_string(),
            "gpt-5".to_string(),
            "claude-sonnet-4-6".to_string(),
        ]);

        let layout = TuiLayout::default();
        let mut widgets = create_default_widgets(&layout);
        widgets.input_box.set_text("/model gpt");

        update_input_overlays_from_input(&widgets.input_box, &mut state);

        assert!(state.model_picker.visible);
        assert_eq!(state.model_picker.filter, "gpt");
        assert_eq!(state.model_picker.filtered.len(), 2);
        assert!(!state.command_palette.visible);
    }

    #[tokio::test]
    async fn enter_on_model_picker_submits_selected_model_command() {
        let mut state = AppState {
            model: "gpt-4o".to_string(),
            ..Default::default()
        };
        state
            .model_picker
            .set_models(vec!["gpt-4o".to_string(), "gpt-5".to_string()]);

        let layout = TuiLayout::default();
        let mut widgets = create_default_widgets(&layout);
        let (msg_tx, mut msg_rx) = mpsc::channel(4);

        widgets.input_box.set_text("/model");
        update_input_overlays_from_input(&widgets.input_box, &mut state);

        handle_event(
            TuiEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            &mut state,
            &mut widgets,
            &msg_tx,
            &layout,
        )
        .await;
        handle_event(
            TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            &mut state,
            &mut widgets,
            &msg_tx,
            &layout,
        )
        .await;

        let message = msg_rx.try_recv().expect("model command sent");
        assert_eq!(message.text, "/model gpt-5");
        assert!(message.thread_id.is_none());
        assert_eq!(state.model, "gpt-5");
        assert!(!state.model_picker.visible);
    }

    #[tokio::test]
    async fn submit_uses_current_thread_scope() {
        let mut state = AppState {
            current_thread_id: Some("thread-123".to_string()),
            ..Default::default()
        };
        let layout = TuiLayout::default();
        let mut widgets = create_default_widgets(&layout);
        let (msg_tx, mut msg_rx) = mpsc::channel(4);

        widgets.input_box.set_text("run it");

        handle_event(
            TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            &mut state,
            &mut widgets,
            &msg_tx,
            &layout,
        )
        .await;

        let message = msg_rx.try_recv().expect("message sent");
        assert_eq!(message.text, "run it");
        assert_eq!(message.thread_id.as_deref(), Some("thread-123"));
    }

    #[tokio::test]
    async fn slash_model_without_available_models_submits_on_enter() {
        let mut state = AppState::default();
        let layout = TuiLayout::default();
        let mut widgets = create_default_widgets(&layout);
        let (msg_tx, mut msg_rx) = mpsc::channel(4);

        widgets.input_box.set_text("/model");
        update_input_overlays_from_input(&widgets.input_box, &mut state);
        assert!(state.command_palette.visible);
        assert!(!state.model_picker.visible);

        handle_event(
            TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            &mut state,
            &mut widgets,
            &msg_tx,
            &layout,
        )
        .await;

        let message = msg_rx.try_recv().expect("/model command sent");
        assert_eq!(message.text, "/model");
        assert!(state.awaiting_model_list);
        assert!(!state.command_palette.visible);
        assert!(widgets.input_box.is_empty());
    }

    #[tokio::test]
    async fn slash_palette_model_selection_opens_model_picker_when_models_exist() {
        let mut state = AppState::default();
        state
            .model_picker
            .set_models(vec!["gpt-4o".to_string(), "gpt-5".to_string()]);

        let layout = TuiLayout::default();
        let mut widgets = create_default_widgets(&layout);
        let (msg_tx, _msg_rx) = mpsc::channel(4);

        widgets.input_box.set_text("/mo");
        update_input_overlays_from_input(&widgets.input_box, &mut state);
        assert!(state.command_palette.visible);

        handle_event(
            TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            &mut state,
            &mut widgets,
            &msg_tx,
            &layout,
        )
        .await;

        assert_eq!(widgets.input_box.current_text(), "/model ");
        assert!(state.model_picker.visible);
        assert!(!state.command_palette.visible);
    }

    #[test]
    fn parse_model_response_extracts_active_and_available_models() {
        let parsed = parse_model_list_response(
            "Active model: gpt-5\n\nAvailable models:\n  gpt-5 (active)\n  gpt-4o\n\nUse /model <name> to switch.",
        )
        .expect("parsed model response");

        assert_eq!(parsed.0, "gpt-5");
        assert_eq!(parsed.1, vec!["gpt-5".to_string(), "gpt-4o".to_string()]);
    }

    #[tokio::test]
    async fn model_response_hydrates_picker_after_first_fetch() {
        let mut state = AppState {
            awaiting_model_list: true,
            ..Default::default()
        };
        let layout = TuiLayout::default();
        let mut widgets = create_default_widgets(&layout);

        handle_event(
            TuiEvent::Response {
                content: "Active model: gpt-5\n\nAvailable models:\n  gpt-5 (active)\n  gpt-4o\n\nUse /model <name> to switch.".to_string(),
                thread_id: None,
            },
            &mut state,
            &mut widgets,
            &mpsc::channel(1).0,
            &layout,
        )
        .await;

        assert_eq!(state.model, "gpt-5");
        assert_eq!(widgets.input_box.current_text(), "/model ");
        assert!(state.model_picker.visible);
        assert_eq!(state.model_picker.filtered.len(), 2);
        assert!(!state.awaiting_model_list);
    }

    #[tokio::test]
    async fn tool_updates_use_call_id_to_disambiguate_duplicate_names() {
        let mut state = AppState::default();

        apply_event(
            &mut state,
            TuiEvent::ToolStarted {
                name: "http".to_string(),
                detail: Some("first".to_string()),
                call_id: Some("call-1".to_string()),
            },
        )
        .await;
        apply_event(
            &mut state,
            TuiEvent::ToolStarted {
                name: "http".to_string(),
                detail: Some("second".to_string()),
                call_id: Some("call-2".to_string()),
            },
        )
        .await;
        apply_event(
            &mut state,
            TuiEvent::ToolResult {
                name: "http".to_string(),
                preview: "preview-2".to_string(),
                call_id: Some("call-2".to_string()),
            },
        )
        .await;
        apply_event(
            &mut state,
            TuiEvent::ToolCompleted {
                name: "http".to_string(),
                success: true,
                error: None,
                call_id: Some("call-2".to_string()),
            },
        )
        .await;

        assert_eq!(state.active_tools.len(), 1);
        assert_eq!(state.active_tools[0].call_id.as_deref(), Some("call-1"));
        assert_eq!(state.recent_tools.len(), 1);
        assert_eq!(state.recent_tools[0].call_id.as_deref(), Some("call-2"));
        assert_eq!(state.recent_tools[0].detail.as_deref(), Some("second"));
        assert_eq!(
            state.recent_tools[0].result_preview.as_deref(),
            Some("preview-2")
        );
    }
}
