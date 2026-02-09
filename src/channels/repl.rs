//! Interactive REPL channel with line editing and markdown rendering.
//!
//! Provides the primary CLI interface for interacting with the agent.
//! Uses rustyline for line editing, history, and tab-completion.
//! Uses termimad for rendering markdown responses inline.
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

use std::borrow::Cow;
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use rustyline::completion::Completer;
use rustyline::config::Config;
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{CompletionType, Editor, Helper};
use termimad::MadSkin;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;

/// Slash commands available in the REPL.
const SLASH_COMMANDS: &[&str] = &[
    "/help",
    "/quit",
    "/exit",
    "/debug",
    "/model",
    "/undo",
    "/redo",
    "/clear",
    "/compact",
    "/new",
    "/interrupt",
    "/version",
    "/tools",
    "/ping",
    "/job",
    "/status",
    "/cancel",
    "/list",
    "/heartbeat",
    "/summarize",
    "/suggest",
    "/thread",
    "/resume",
];

/// Rustyline helper for slash-command tab completion.
struct ReplHelper;

impl Completer for ReplHelper {
    type Candidate = String;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<String>)> {
        if !line.starts_with('/') {
            return Ok((0, vec![]));
        }

        let prefix = &line[..pos];
        let matches: Vec<String> = SLASH_COMMANDS
            .iter()
            .filter(|cmd| cmd.starts_with(prefix))
            .map(|cmd| cmd.to_string())
            .collect();

        Ok((0, matches))
    }
}

impl Hinter for ReplHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, _ctx: &rustyline::Context<'_>) -> Option<String> {
        if !line.starts_with('/') || pos < line.len() {
            return None;
        }

        SLASH_COMMANDS
            .iter()
            .find(|cmd| cmd.starts_with(line) && **cmd != line)
            .map(|cmd| cmd[line.len()..].to_string())
    }
}

impl Highlighter for ReplHelper {
    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Cow::Owned(format!("\x1b[90m{hint}\x1b[0m"))
    }
}

impl Validator for ReplHelper {}
impl Helper for ReplHelper {}

/// Build a termimad skin with our color scheme.
fn make_skin() -> MadSkin {
    let mut skin = MadSkin::default();
    skin.set_headers_fg(termimad::crossterm::style::Color::Yellow);
    skin.bold.set_fg(termimad::crossterm::style::Color::White);
    skin.italic
        .set_fg(termimad::crossterm::style::Color::Magenta);
    skin.inline_code
        .set_fg(termimad::crossterm::style::Color::Green);
    skin.code_block
        .set_fg(termimad::crossterm::style::Color::Green);
    skin
}

/// REPL channel with line editing and markdown rendering.
pub struct ReplChannel {
    /// Optional single message to send (for -m flag).
    single_message: Option<String>,
    /// Debug mode flag (shared with input thread).
    debug_mode: Arc<AtomicBool>,
    /// Whether we're currently streaming (chunks have been printed without a trailing newline).
    is_streaming: Arc<AtomicBool>,
}

impl ReplChannel {
    /// Create a new REPL channel.
    pub fn new() -> Self {
        Self {
            single_message: None,
            debug_mode: Arc::new(AtomicBool::new(false)),
            is_streaming: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Create a REPL channel that sends a single message and exits.
    pub fn with_message(message: String) -> Self {
        Self {
            single_message: Some(message),
            debug_mode: Arc::new(AtomicBool::new(false)),
            is_streaming: Arc::new(AtomicBool::new(false)),
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

/// Get the history file path (~/.ironclaw/history).
fn history_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".ironclaw")
        .join("history")
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
            // Single message mode: send it and return
            if let Some(msg) = single_message {
                let incoming = IncomingMessage::new("repl", "user", &msg);
                let _ = tx.blocking_send(incoming);
                return;
            }

            // Set up rustyline
            let config = Config::builder()
                .history_ignore_dups(true)
                .expect("valid config")
                .auto_add_history(true)
                .completion_type(CompletionType::List)
                .build();

            let mut rl = match Editor::with_config(config) {
                Ok(editor) => editor,
                Err(e) => {
                    eprintln!("Failed to initialize line editor: {e}");
                    return;
                }
            };

            rl.set_helper(Some(ReplHelper));

            // Load history
            let hist_path = history_path();
            if let Some(parent) = hist_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = rl.load_history(&hist_path);

            println!("IronClaw REPL - Type /help for commands, /quit to exit");
            println!();

            loop {
                let prompt = if debug_mode.load(Ordering::Relaxed) {
                    "\x1b[33m[debug]\x1b[0m \x1b[36m>\x1b[0m "
                } else {
                    "\x1b[36m>\x1b[0m "
                };

                match rl.readline(prompt) {
                    Ok(line) => {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }

                        // Handle local REPL commands (only commands that need
                        // immediate local handling stay here)
                        match line.to_lowercase().as_str() {
                            "/quit" | "/exit" => break,
                            "/debug" => {
                                let current = debug_mode.load(Ordering::Relaxed);
                                debug_mode.store(!current, Ordering::Relaxed);
                                if !current {
                                    println!("Debug mode ON");
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
                    Err(ReadlineError::Interrupted) => {
                        // Ctrl+C: send /interrupt
                        let msg = IncomingMessage::new("repl", "user", "/interrupt");
                        if tx.blocking_send(msg).is_err() {
                            break;
                        }
                    }
                    Err(ReadlineError::Eof) => {
                        // Ctrl+D: send /quit so the agent loop runs graceful shutdown
                        let msg = IncomingMessage::new("repl", "user", "/quit");
                        let _ = tx.blocking_send(msg);
                        break;
                    }
                    Err(e) => {
                        eprintln!("Input error: {e}");
                        break;
                    }
                }
            }

            // Save history on exit
            let _ = rl.save_history(&history_path());
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // If we were streaming, the content was already printed via StreamChunk.
        // Just finish the line and reset.
        if self.is_streaming.swap(false, Ordering::Relaxed) {
            println!();
            println!();
            return Ok(());
        }

        // Render markdown
        let skin = make_skin();
        let width = crossterm::terminal::size()
            .map(|(w, _)| w as usize)
            .unwrap_or(80);
        let text = termimad::FmtText::from(&skin, &response.content, Some(width));

        println!();
        print!("{text}");
        println!();
        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        let debug = self.is_debug();

        match status {
            StatusUpdate::Thinking(msg) => {
                if debug {
                    eprintln!("\x1b[90m[thinking] {msg}\x1b[0m");
                }
            }
            StatusUpdate::ToolStarted { name } => {
                eprintln!("  \x1b[33m>> {name}\x1b[0m");
            }
            StatusUpdate::ToolCompleted { name, success } => {
                if success {
                    eprintln!("  \x1b[32m<< {name}\x1b[0m");
                } else {
                    eprintln!("  \x1b[31m<< {name} failed\x1b[0m");
                }
            }
            StatusUpdate::StreamChunk(chunk) => {
                self.is_streaming.store(true, Ordering::Relaxed);
                print!("{chunk}");
                let _ = io::stdout().flush();
            }
            StatusUpdate::JobStarted {
                job_id,
                title,
                browse_url,
            } => {
                eprintln!(
                    "  \x1b[36m[job]\x1b[0m {title} \x1b[90m({job_id})\x1b[0m \x1b[4m{browse_url}\x1b[0m"
                );
            }
            StatusUpdate::Status(msg) => {
                if debug || msg.contains("approval") || msg.contains("Approval") {
                    eprintln!("\x1b[90m[status] {msg}\x1b[0m");
                }
            }
            StatusUpdate::ApprovalNeeded {
                request_id,
                tool_name,
                description,
                parameters,
            } => {
                let params_preview = serde_json::to_string_pretty(&parameters)
                    .unwrap_or_else(|_| parameters.to_string());
                let params_truncated = if params_preview.chars().count() > 200 {
                    format!(
                        "{}...",
                        params_preview.chars().take(200).collect::<String>()
                    )
                } else {
                    params_preview
                };
                eprintln!();
                eprintln!("\x1b[33m  Tool requires approval\x1b[0m");
                eprintln!("  \x1b[1mTool:\x1b[0m {tool_name}");
                eprintln!("  \x1b[1mDesc:\x1b[0m {description}");
                eprintln!(
                    "  \x1b[1mParams:\x1b[0m\n  {}",
                    params_truncated.replace('\n', "\n  ")
                );
                eprintln!();
                eprintln!(
                    "  Reply: \x1b[32myes\x1b[0m / \x1b[34malways\x1b[0m / \x1b[31mno\x1b[0m"
                );
                eprintln!("  \x1b[90mRequest ID: {request_id}\x1b[0m");
                eprintln!();
            }
        }
        Ok(())
    }

    async fn broadcast(
        &self,
        _user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let skin = make_skin();
        let width = crossterm::terminal::size()
            .map(|(w, _)| w as usize)
            .unwrap_or(80);

        eprintln!("\x1b[36m[notification]\x1b[0m");
        let text = termimad::FmtText::from(&skin, &response.content, Some(width));
        eprint!("{text}");
        eprintln!();
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}
