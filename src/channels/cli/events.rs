//! Event handling for the TUI.

use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use crate::channels::IncomingMessage;
use crate::channels::cli::app::{AppEvent, AppState, InputMode};
use crate::channels::cli::render;

/// Tick rate for the event loop (50ms = 20fps).
const TICK_RATE: Duration = Duration::from_millis(50);

/// Run the main event loop.
pub fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut AppState,
    msg_tx: mpsc::Sender<IncomingMessage>,
    event_rx: &mut mpsc::Receiver<AppEvent>,
) -> io::Result<()> {
    loop {
        // Render
        terminal.draw(|f| render::render(f, app))?;

        // Check for quit
        if app.should_quit {
            return Ok(());
        }

        // Poll for events
        if event::poll(TICK_RATE)? {
            let evt = event::read()?;
            if let Err(e) = handle_event(app, evt, &msg_tx) {
                tracing::error!("Event handling error: {}", e);
            }
        }

        // Check for app events (non-blocking)
        while let Ok(app_event) = event_rx.try_recv() {
            handle_app_event(app, app_event);
        }
    }
}

/// Handle a crossterm event.
fn handle_event(
    app: &mut AppState,
    event: Event,
    msg_tx: &mpsc::Sender<IncomingMessage>,
) -> io::Result<()> {
    match event {
        Event::Key(key) => handle_key(app, key, msg_tx),
        Event::Mouse(_) => Ok(()),     // Could handle mouse scrolling here
        Event::Resize(_, _) => Ok(()), // Terminal will handle resize
        _ => Ok(()),
    }
}

/// Handle a key event.
fn handle_key(
    app: &mut AppState,
    key: KeyEvent,
    msg_tx: &mpsc::Sender<IncomingMessage>,
) -> io::Result<()> {
    // Global keybindings
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') => {
                if app.mode == InputMode::Approval {
                    // Cancel all pending approvals
                    app.clear_approvals();
                } else {
                    // Quit
                    app.should_quit = true;
                }
                return Ok(());
            }
            KeyCode::Char('d') => {
                app.should_quit = true;
                return Ok(());
            }
            _ => {}
        }
    }

    match app.mode {
        InputMode::Normal => handle_normal_mode(app, key),
        InputMode::Editing => handle_editing_mode(app, key, msg_tx),
        InputMode::Approval => handle_approval_mode(app, key),
    }
}

/// Handle keys in normal mode.
fn handle_normal_mode(app: &mut AppState, key: KeyEvent) -> io::Result<()> {
    match key.code {
        KeyCode::Char('i') | KeyCode::Char('a') => {
            app.mode = InputMode::Editing;
        }
        KeyCode::Char('q') => {
            app.should_quit = true;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.scroll_up(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.scroll_down(1);
        }
        KeyCode::PageUp => {
            app.scroll_up(10);
        }
        KeyCode::PageDown => {
            app.scroll_down(10);
        }
        KeyCode::Char('G') => {
            app.scroll_to_bottom();
        }
        _ => {}
    }
    Ok(())
}

/// Handle keys in editing mode.
fn handle_editing_mode(
    app: &mut AppState,
    key: KeyEvent,
    msg_tx: &mpsc::Sender<IncomingMessage>,
) -> io::Result<()> {
    match key.code {
        KeyCode::Enter => {
            if !app.composer.is_empty() {
                let input = app.composer.submit();
                app.add_user_message(&input);

                // Send message to agent
                let msg = IncomingMessage::new("tui", "local-user", &input);
                let _ = msg_tx.blocking_send(msg);
            }
        }
        KeyCode::Esc => {
            app.mode = InputMode::Normal;
        }
        KeyCode::Backspace => {
            app.composer.backspace();
        }
        KeyCode::Delete => {
            app.composer.delete();
        }
        KeyCode::Left => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                // Move word left (simplified: just move to start)
                app.composer.move_home();
            } else {
                app.composer.move_left();
            }
        }
        KeyCode::Right => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                // Move word right (simplified: just move to end)
                app.composer.move_end();
            } else {
                app.composer.move_right();
            }
        }
        KeyCode::Home => {
            app.composer.move_home();
        }
        KeyCode::End => {
            app.composer.move_end();
        }
        KeyCode::Up => {
            app.composer.history_prev();
        }
        KeyCode::Down => {
            app.composer.history_next();
        }
        KeyCode::Tab => {
            app.composer.complete();
        }
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match c {
                    'a' => app.composer.move_home(),
                    'e' => app.composer.move_end(),
                    'k' => app.composer.kill_line(),
                    'u' => app.composer.kill_to_start(),
                    'w' => {
                        // Delete word backwards (simplified: clear)
                        app.composer.clear();
                    }
                    _ => {}
                }
            } else {
                app.composer.insert(c);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Handle keys in approval mode.
fn handle_approval_mode(app: &mut AppState, key: KeyEvent) -> io::Result<()> {
    if let Some(ref mut overlay) = app.approval {
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => {
                overlay.select_prev();
            }
            KeyCode::Right | KeyCode::Char('l') => {
                overlay.select_next();
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                let (approved, _always) = overlay.confirm();
                app.handle_approval_response(approved);
                // TODO: If always, remember to auto-approve this tool
            }
            KeyCode::Char(c) => {
                if let Some(approved) = overlay.handle_shortcut(c) {
                    app.handle_approval_response(approved);
                }
            }
            KeyCode::Esc => {
                // Deny this approval
                app.handle_approval_response(false);
            }
            _ => {}
        }
    }
    Ok(())
}

/// Handle an application event.
fn handle_app_event(app: &mut AppState, event: AppEvent) {
    match event {
        AppEvent::Response(content) => {
            app.add_agent_message(content);
        }
        AppEvent::ToolStarted { name } => {
            app.set_status(format!("Running tool: {}...", name));
        }
        AppEvent::ToolCompleted { name, success } => {
            if success {
                app.set_status(format!("Tool {} completed", name));
            } else {
                app.set_status(format!("Tool {} failed", name));
            }
        }
        AppEvent::ApprovalRequested(request) => {
            app.queue_approval(request);
        }
        AppEvent::StreamChunk(chunk) => {
            if app.streaming_buffer.is_none() {
                app.start_streaming();
            }
            app.append_stream(&chunk);
        }
        AppEvent::Redraw => {
            // Just triggers a redraw on next loop iteration
        }
        AppEvent::Quit => {
            app.should_quit = true;
        }
        AppEvent::Input(_) => {
            // Already handled directly
        }
    }
}
