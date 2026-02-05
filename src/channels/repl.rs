//! Interactive REPL channel for debugging and testing.
//!
//! Provides a command-line interface for interacting with the agent.
//!
//! ## Commands
//!
//! - `/help` - Show available commands
//! - `/quit` or `/exit` - Exit the REPL
//! - `/debug` - Toggle debug mode (verbose tool output)
//! - `/undo` - Undo the last turn
//! - `/redo` - Redo an undone turn
//! - `/clear` - Clear the conversation
//! - `/compact` - Compact the context
//! - `/new` - Start a new thread
//! - `yes`/`no`/`always` - Respond to tool approval prompts

use std::io::{self, BufRead, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;

/// REPL channel for interactive agent debugging.
pub struct ReplChannel {
    /// Optional single message to send (for -m flag).
    single_message: Option<String>,
    /// Debug mode flag (shared with input thread).
    debug_mode: Arc<AtomicBool>,
}

impl ReplChannel {
    /// Create a new REPL channel.
    pub fn new() -> Self {
        Self {
            single_message: None,
            debug_mode: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Create a REPL channel that sends a single message and exits.
    pub fn with_message(message: String) -> Self {
        Self {
            single_message: Some(message),
            debug_mode: Arc::new(AtomicBool::new(false)),
        }
    }

    fn is_debug(&self) -> bool {
        self.debug_mode.load(Ordering::Relaxed)
    }
}

impl Default for ReplChannel {
    fn default() -> Self {
        Self::new()
    }
}

fn print_help() {
    println!(
        r#"
IronClaw REPL - Interactive debugging mode

Commands:
  /help          Show this help message
  /quit, /exit   Exit the REPL
  /debug         Toggle debug mode (verbose output)
  /undo          Undo the last turn
  /redo          Redo an undone turn
  /clear         Clear the conversation
  /compact       Compact the context window
  /new           Start a new conversation thread
  /interrupt     Stop the current operation

Approval responses (when prompted):
  yes, y         Approve the tool execution
  no, n          Deny the tool execution
  always         Approve and auto-approve this tool for the session

Tips:
  - Tool calls requiring approval will pause and wait for your response
  - Use /debug to see detailed tool inputs and outputs
  - Press Ctrl+C to interrupt a long-running operation
"#
    );
}

#[async_trait]
impl Channel for ReplChannel {
    fn name(&self) -> &str {
        "repl"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = mpsc::channel(32);
        let single_message = self.single_message.clone();
        let debug_mode = Arc::clone(&self.debug_mode);

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

            println!("IronClaw REPL - Type /help for commands, /quit to exit");
            println!();

            loop {
                // Print prompt
                let prompt = if debug_mode.load(Ordering::Relaxed) {
                    "[debug] > "
                } else {
                    "> "
                };
                print!("{}", prompt);
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

                        // Handle local REPL commands
                        match line.to_lowercase().as_str() {
                            "/quit" | "/exit" => break,
                            "/help" | "/?" => {
                                print_help();
                                continue;
                            }
                            "/debug" => {
                                let current = debug_mode.load(Ordering::Relaxed);
                                debug_mode.store(!current, Ordering::Relaxed);
                                if !current {
                                    println!("Debug mode ON - showing verbose tool output");
                                } else {
                                    println!("Debug mode OFF");
                                }
                                continue;
                            }
                            _ => {}
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
        println!();
        println!("{}", response.content);
        println!();
        Ok(())
    }

    async fn send_status(&self, status: StatusUpdate) -> Result<(), ChannelError> {
        let debug = self.is_debug();

        match status {
            StatusUpdate::Thinking(msg) => {
                if debug {
                    eprintln!("\x1b[90m[thinking] {}\x1b[0m", msg);
                } else {
                    eprint!(".");
                    let _ = io::stderr().flush();
                }
            }
            StatusUpdate::ToolStarted { name } => {
                if debug {
                    eprintln!("\x1b[33m[tool:start] {}\x1b[0m", name);
                } else {
                    eprintln!("\x1b[33m⚡ {}\x1b[0m", name);
                }
            }
            StatusUpdate::ToolCompleted { name, success } => {
                if debug {
                    if success {
                        eprintln!("\x1b[32m[tool:done] {} ✓\x1b[0m", name);
                    } else {
                        eprintln!("\x1b[31m[tool:fail] {} ✗\x1b[0m", name);
                    }
                } else if !success {
                    eprintln!("\x1b[31m✗ {} failed\x1b[0m", name);
                }
            }
            StatusUpdate::StreamChunk(chunk) => {
                print!("{}", chunk);
                let _ = io::stdout().flush();
            }
            StatusUpdate::Status(msg) => {
                if debug || msg.contains("approval") || msg.contains("Approval") {
                    eprintln!("\x1b[90m[status] {}\x1b[0m", msg);
                }
            }
        }
        Ok(())
    }

    async fn broadcast(
        &self,
        _user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        println!();
        println!("\x1b[36m[notification]\x1b[0m {}", response.content);
        println!();
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}
