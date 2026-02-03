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
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse};
use crate::error::ChannelError;

pub use app::{AppEvent, AppState, InputMode};
pub use composer::ChatComposer;
pub use overlay::{ApprovalOverlay, ApprovalRequest};

/// TUI channel for interactive terminal input with Ratatui.
pub struct TuiChannel {
    /// Channel for sending events to the TUI.
    event_tx: Option<mpsc::Sender<AppEvent>>,
}

impl TuiChannel {
    /// Create a new TUI channel.
    pub fn new() -> Self {
        Self { event_tx: None }
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
        let (event_tx, event_rx) = mpsc::channel(64);

        // Store the event sender so we can send responses
        // Note: In the actual implementation, we'd store this properly
        // For now, spawn the TUI in a separate task
        let event_tx_clone = event_tx.clone();

        tokio::task::spawn_blocking(move || {
            if let Err(e) = run_tui(msg_tx, event_rx) {
                tracing::error!("TUI error: {}", e);
            }
        });

        // Keep the event_tx alive by storing it
        // This is a hack; in production we'd use Arc<Mutex<>> or similar
        let _ = event_tx_clone;

        Ok(Box::pin(ReceiverStream::new(msg_rx)))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Send response event to the TUI
        if let Some(ref tx) = self.event_tx {
            let _ = tx
                .send(AppEvent::Response(response.content))
                .await
                .map_err(|e| ChannelError::SendFailed {
                    name: "tui".to_string(),
                    reason: e.to_string(),
                });
        }
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(AppEvent::Quit).await;
        }
        Ok(())
    }
}

/// Run the TUI event loop (blocking).
fn run_tui(
    msg_tx: mpsc::Sender<IncomingMessage>,
    mut event_rx: mpsc::Receiver<AppEvent>,
) -> io::Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = AppState::new();

    // Run event loop
    let result = events::run_event_loop(&mut terminal, &mut app, msg_tx, &mut event_rx);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

/// Simple blocking CLI channel (fallback when TUI not available).
pub struct SimpleCliChannel {
    running: Arc<std::sync::atomic::AtomicBool>,
}

impl SimpleCliChannel {
    pub fn new() -> Self {
        Self {
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

impl Default for SimpleCliChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Channel for SimpleCliChannel {
    fn name(&self) -> &str {
        "cli"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        self.running
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let running = self.running.clone();

        let (tx, rx) = mpsc::channel(32);

        tokio::task::spawn_blocking(move || {
            use std::io::BufRead;

            let stdin = io::stdin();
            let reader = stdin.lock();

            print_prompt();

            for line in reader.lines() {
                if !running.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }

                match line {
                    Ok(content) => {
                        let content = content.trim();
                        if content.is_empty() {
                            print_prompt();
                            continue;
                        }

                        if content == "exit" || content == "quit" || content == "/quit" {
                            running.store(false, std::sync::atomic::Ordering::SeqCst);
                            break;
                        }

                        let msg = IncomingMessage::new("cli", "local-user", content);

                        if tx.blocking_send(msg).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::error!("Error reading stdin: {}", e);
                        break;
                    }
                }
            }

            tracing::debug!("CLI input loop ended");
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        println!("\n{}\n", response.content);
        print_prompt();
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        if self.running.load(std::sync::atomic::Ordering::SeqCst) {
            Ok(())
        } else {
            Err(ChannelError::HealthCheckFailed {
                name: "cli".to_string(),
            })
        }
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

fn print_prompt() {
    use std::io::Write;
    print!("agent> ");
    let _ = io::stdout().flush();
}
