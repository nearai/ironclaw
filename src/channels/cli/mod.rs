//! Interactive TUI channel using Ratatui.
//!
//! Provides a rich terminal interface with:
//! - Input history navigation
//! - Slash command completion
//! - Approval overlays for tool execution
//! - Streaming response display

mod app;
mod composer;
mod events;
mod overlay;
mod render;

use std::io;
use std::sync::Arc;

use async_trait::async_trait;
use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::{Mutex, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;

pub use app::{AppEvent, AppState, InputMode};
pub use composer::ChatComposer;
pub use overlay::{ApprovalOverlay, ApprovalRequest};

/// TUI channel for interactive terminal input with Ratatui.
pub struct TuiChannel {
    /// Channel for sending events to the TUI (created upfront for logging).
    event_tx: mpsc::Sender<AppEvent>,
    /// Receiver end, taken when start() is called.
    event_rx: Arc<Mutex<Option<mpsc::Receiver<AppEvent>>>>,
}

impl TuiChannel {
    /// Create a new TUI channel.
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::channel(64);
        Self {
            event_tx,
            event_rx: Arc::new(Mutex::new(Some(event_rx))),
        }
    }

    /// Get a log writer that sends messages to the TUI status line.
    /// Use this to redirect tracing output to the TUI.
    pub fn log_writer(&self) -> TuiLogWriter {
        TuiLogWriter::new(self.event_tx.clone())
    }
}

impl Default for TuiChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Channel for TuiChannel {
    fn name(&self) -> &str {
        "tui"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (msg_tx, msg_rx) = mpsc::channel(32);

        // Take the event receiver (can only start once)
        let event_rx = {
            let mut guard = self.event_rx.lock().await;
            guard.take().ok_or_else(|| ChannelError::StartupFailed {
                name: "tui".to_string(),
                reason: "TUI channel already started".to_string(),
            })?
        };

        tokio::task::spawn_blocking(move || {
            if let Err(e) = run_tui(msg_tx, event_rx) {
                // Try to restore terminal even on error
                let _ = disable_raw_mode();
                let _ = execute!(io::stdout(), LeaveAlternateScreen);
                eprintln!("TUI error: {}", e);
            }
        });

        Ok(Box::pin(ReceiverStream::new(msg_rx)))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.event_tx
            .send(AppEvent::Response(response.content))
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "tui".to_string(),
                reason: e.to_string(),
            })?;
        Ok(())
    }

    async fn send_status(&self, status: StatusUpdate) -> Result<(), ChannelError> {
        let event = match status {
            StatusUpdate::Thinking(msg) => AppEvent::ThinkingMessage(format!("🤔 {}", msg)),
            StatusUpdate::ToolStarted { name } => AppEvent::ToolStarted { name },
            StatusUpdate::ToolCompleted { name, success } => {
                AppEvent::ToolCompleted { name, success }
            }
            StatusUpdate::StreamChunk(chunk) => AppEvent::StreamChunk(chunk),
            StatusUpdate::Status(msg) => AppEvent::ThinkingMessage(msg),
        };
        self.event_tx
            .send(event)
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "tui".to_string(),
                reason: e.to_string(),
            })?;
        Ok(())
    }

    async fn broadcast(
        &self,
        _user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // For TUI, broadcasts appear as regular agent responses with a notification indicator
        self.event_tx
            .send(AppEvent::Response(response.content))
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "tui".to_string(),
                reason: e.to_string(),
            })?;
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        // Channel is healthy if we haven't been closed
        if self.event_tx.is_closed() {
            Err(ChannelError::HealthCheckFailed {
                name: "tui".to_string(),
            })
        } else {
            Ok(())
        }
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        let _ = self.event_tx.send(AppEvent::Quit).await;
        Ok(())
    }
}

/// Run the TUI event loop (blocking).
fn run_tui(
    msg_tx: mpsc::Sender<IncomingMessage>,
    event_rx: mpsc::Receiver<AppEvent>,
) -> io::Result<()> {
    // Setup terminal
    // Note: We don't enable mouse capture so users can select text normally
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = AppState::new();

    // Run event loop
    let result = events::run_event_loop(&mut terminal, &mut app, msg_tx, event_rx);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

/// TUI-compatible tracing writer that sends log messages to the TUI status line.
#[derive(Clone)]
pub struct TuiLogWriter {
    tx: mpsc::Sender<AppEvent>,
}

impl TuiLogWriter {
    pub fn new(tx: mpsc::Sender<AppEvent>) -> Self {
        Self { tx }
    }
}

impl std::io::Write for TuiLogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Ok(s) = std::str::from_utf8(buf) {
            let s = s.trim();
            if !s.is_empty() {
                // Fire and forget - don't block on logging
                let _ = self.tx.try_send(AppEvent::LogMessage(s.to_string()));
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TuiLogWriter {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
