//! Simple REPL channel for testing without TUI.
//!
//! Provides a basic stdin/stdout interface for testing the agent.

use std::io::{self, BufRead, Write};

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;

/// Simple REPL channel using stdin/stdout.
pub struct ReplChannel {
    /// Optional single message to send (for -m flag).
    single_message: Option<String>,
}

impl ReplChannel {
    /// Create a new REPL channel.
    pub fn new() -> Self {
        Self {
            single_message: None,
        }
    }

    /// Create a REPL channel that sends a single message and exits.
    pub fn with_message(message: String) -> Self {
        Self {
            single_message: Some(message),
        }
    }
}

impl Default for ReplChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Channel for ReplChannel {
    fn name(&self) -> &str {
        "repl"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = mpsc::channel(32);
        let single_message = self.single_message.clone();

        std::thread::spawn(move || {
            // If single message mode, send it and exit
            if let Some(msg) = single_message {
                let incoming = IncomingMessage::new("repl", "user", &msg);
                if tx.blocking_send(incoming).is_err() {
                    return;
                }
                // Wait a bit for response, then the channel will close
                return;
            }

            // Interactive REPL mode
            let stdin = io::stdin();
            let mut stdout = io::stdout();

            loop {
                // Print prompt
                print!("> ");
                let _ = stdout.flush();

                // Read line
                let mut line = String::new();
                match stdin.lock().read_line(&mut line) {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        if line == "/quit" || line == "/exit" {
                            break;
                        }

                        let msg = IncomingMessage::new("repl", "user", line);
                        if tx.blocking_send(msg).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        println!("\n{}\n", response.content);
        Ok(())
    }

    async fn send_status(&self, status: StatusUpdate) -> Result<(), ChannelError> {
        match status {
            StatusUpdate::Thinking(msg) => eprintln!("[thinking] {}", msg),
            StatusUpdate::ToolStarted { name } => eprintln!("[tool] Starting: {}", name),
            StatusUpdate::ToolCompleted { name, success } => {
                if success {
                    eprintln!("[tool] Completed: {}", name);
                } else {
                    eprintln!("[tool] Failed: {}", name);
                }
            }
            StatusUpdate::StreamChunk(chunk) => {
                print!("{}", chunk);
                let _ = io::stdout().flush();
            }
            StatusUpdate::Status(msg) => eprintln!("[status] {}", msg),
        }
        Ok(())
    }

    async fn broadcast(
        &self,
        _user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        println!("\n[broadcast] {}\n", response.content);
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}
