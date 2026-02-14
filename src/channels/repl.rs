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

use crate::agent::truncate_for_preview;
use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;

/// Max characters for tool result previews in the terminal.
const CLI_TOOL_RESULT_MAX: usize = 200;

/// Max characters for thinking/status messages in the terminal.
const CLI_STATUS_MAX: usize = 200;

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
    skin.code_block.left_margin = 2;
    skin
}

/// Format JSON params as `key: value` lines for the approval card.
fn format_json_params(params: &serde_json::Value, indent: &str) -> String {
    match params {
        serde_json::Value::Object(map) => {
            let mut lines = Vec::new();
            for (key, value) in map {
                let val_str = match value {
                    serde_json::Value::String(s) => {
                        let display = if s.len() > 120 { &s[..120] } else { s };
                        format!("\x1b[32m\"{display}\"\x1b[0m")
                    }
                    other => {
                        let rendered = other.to_string();
                        if rendered.len() > 120 {
                            format!("{}...", &rendered[..120])
                        } else {
                            rendered
                        }
                    }
                };
                lines.push(format!("{indent}\x1b[36m{key}\x1b[0m: {val_str}"));
            }
            lines.join("\n")
        }
        other => {
            let pretty = serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string());
            let truncated = if pretty.len() > 300 {
                format!("{}...", &pretty[..300])
            } else {
                pretty
            };
            truncated
                .lines()
                .map(|l| format!("{indent}\x1b[90m{l}\x1b[0m"))
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
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

fn print_help() {
    // Bold white for section headers, bold cyan for commands, dim gray for descriptions
    let h = "\x1b[1m"; // bold (section headers)
    let c = "\x1b[1;36m"; // bold cyan (commands)
    let d = "\x1b[90m"; // dim gray (descriptions)
    let r = "\x1b[0m"; // reset

    println!();
    println!("  {h}IronClaw REPL{r}");
    println!();
    println!("  {h}Commands{r}");
    println!("  {c}/help{r}              {d}show this help{r}");
    println!("  {c}/debug{r}             {d}toggle verbose output{r}");
    println!("  {c}/quit{r} {c}/exit{r}        {d}exit the repl{r}");
    println!();
    println!("  {h}Conversation{r}");
    println!("  {c}/undo{r}              {d}undo the last turn{r}");
    println!("  {c}/redo{r}              {d}redo an undone turn{r}");
    println!("  {c}/clear{r}             {d}clear conversation{r}");
    println!("  {c}/compact{r}           {d}compact context window{r}");
    println!("  {c}/new{r}               {d}new conversation thread{r}");
    println!("  {c}/interrupt{r}         {d}stop current operation{r}");
    println!();
    println!("  {h}Approval responses{r}");
    println!("  {c}yes{r} ({c}y{r})            {d}approve tool execution{r}");
    println!("  {c}no{r} ({c}n{r})             {d}deny tool execution{r}");
    println!("  {c}always{r} ({c}a{r})         {d}approve for this session{r}");
    println!();
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
                let incoming = IncomingMessage::new("repl", "default", &msg);
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

            println!("\x1b[1mIronClaw\x1b[0m  /help for commands, /quit to exit");
            println!();

            loop {
                let prompt = if debug_mode.load(Ordering::Relaxed) {
                    "\x1b[33m[debug]\x1b[0m \x1b[1;36m\u{203A}\x1b[0m "
                } else {
                    "\x1b[1;36m\u{203A}\x1b[0m "
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
                            "/help" => {
                                print_help();
                                continue;
                            }
                            "/debug" => {
                                let current = debug_mode.load(Ordering::Relaxed);
                                debug_mode.store(!current, Ordering::Relaxed);
                                if !current {
                                    println!("\x1b[90mdebug mode on\x1b[0m");
                                } else {
                                    println!("\x1b[90mdebug mode off\x1b[0m");
                                }
                                continue;
                            }
                            _ => {}
                        }

                        let msg = IncomingMessage::new("repl", "default", line);
                        if tx.blocking_send(msg).is_err() {
                            break;
                        }
                    }
                    Err(ReadlineError::Interrupted) => {
                        // Ctrl+C: send /interrupt
                        let msg = IncomingMessage::new("repl", "default", "/interrupt");
                        if tx.blocking_send(msg).is_err() {
                            break;
                        }
                    }
                    Err(ReadlineError::Eof) => {
                        // Ctrl+D: send /quit so the agent loop runs graceful shutdown
                        let msg = IncomingMessage::new("repl", "default", "/quit");
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
        let width = crossterm::terminal::size()
            .map(|(w, _)| w as usize)
            .unwrap_or(80);

        // If we were streaming, the content was already printed via StreamChunk.
        // Just finish the line and reset.
        if self.is_streaming.swap(false, Ordering::Relaxed) {
            println!();
            println!();
            return Ok(());
        }

        // Dim separator line before the response
        let sep_width = width.min(80);
        eprintln!("\x1b[90m{}\x1b[0m", "\u{2500}".repeat(sep_width));

        // Render markdown
        let skin = make_skin();
        let text = termimad::FmtText::from(&skin, &response.content, Some(width));

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
                let display = truncate_for_preview(&msg, CLI_STATUS_MAX);
                eprintln!("  \x1b[90m\u{25CB} {display}\x1b[0m");
            }
            StatusUpdate::ToolStarted { name } => {
                eprintln!("  \x1b[33m\u{25CB} {name}\x1b[0m");
            }
            StatusUpdate::ToolCompleted { name, success } => {
                if success {
                    eprintln!("  \x1b[32m\u{25CF} {name}\x1b[0m");
                } else {
                    eprintln!("  \x1b[31m\u{2717} {name} (failed)\x1b[0m");
                }
            }
            StatusUpdate::ToolResult { name: _, preview } => {
                let display = truncate_for_preview(&preview, CLI_TOOL_RESULT_MAX);
                eprintln!("    \x1b[90m{display}\x1b[0m");
            }
            StatusUpdate::StreamChunk(chunk) => {
                // Print separator on the false-to-true transition
                if !self.is_streaming.swap(true, Ordering::Relaxed) {
                    let width = crossterm::terminal::size()
                        .map(|(w, _)| w as usize)
                        .unwrap_or(80);
                    let sep_width = width.min(80);
                    eprintln!("\x1b[90m{}\x1b[0m", "\u{2500}".repeat(sep_width));
                }
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
                    let display = truncate_for_preview(&msg, CLI_STATUS_MAX);
                    eprintln!("  \x1b[90m{display}\x1b[0m");
                }
            }
            StatusUpdate::ApprovalNeeded {
                request_id,
                tool_name,
                description,
                parameters,
            } => {
                let term_width = crossterm::terminal::size()
                    .map(|(w, _)| w as usize)
                    .unwrap_or(80);
                let box_width = (term_width.saturating_sub(4)).clamp(40, 60);

                // Short request ID for the bottom border
                let short_id = if request_id.len() > 8 {
                    &request_id[..8]
                } else {
                    &request_id
                };

                // Top border: ┌ tool_name requires approval ───
                let top_label = format!(" {tool_name} requires approval ");
                let top_fill = box_width.saturating_sub(top_label.len() + 1);
                let top_border = format!(
                    "\u{250C}\x1b[33m{top_label}\x1b[0m{}",
                    "\u{2500}".repeat(top_fill)
                );

                // Bottom border: └─ short_id ─────
                let bot_label = format!(" {short_id} ");
                let bot_fill = box_width.saturating_sub(bot_label.len() + 2);
                let bot_border = format!(
                    "\u{2514}\u{2500}\x1b[90m{bot_label}\x1b[0m{}",
                    "\u{2500}".repeat(bot_fill)
                );

                eprintln!();
                eprintln!("  {top_border}");
                eprintln!("  \u{2502} \x1b[90m{description}\x1b[0m");
                eprintln!("  \u{2502}");

                // Params
                let param_lines = format_json_params(&parameters, "  \u{2502}   ");
                // The format_json_params already includes the indent prefix
                // but we need to handle the case where each line already starts with it
                for line in param_lines.lines() {
                    eprintln!("{line}");
                }

                eprintln!("  \u{2502}");
                eprintln!(
                    "  \u{2502} \x1b[32myes\x1b[0m (y) / \x1b[34malways\x1b[0m (a) / \x1b[31mno\x1b[0m (n)"
                );
                eprintln!("  {bot_border}");
                eprintln!();
            }
            StatusUpdate::AuthRequired {
                extension_name,
                instructions,
                setup_url,
                ..
            } => {
                eprintln!();
                eprintln!("\x1b[33m  Authentication required for {extension_name}\x1b[0m");
                if let Some(ref instr) = instructions {
                    eprintln!("  {instr}");
                }
                if let Some(ref url) = setup_url {
                    eprintln!("  \x1b[4m{url}\x1b[0m");
                }
                eprintln!();
            }
            StatusUpdate::AuthCompleted {
                extension_name,
                success,
                message,
            } => {
                if success {
                    eprintln!("\x1b[32m  {extension_name}: {message}\x1b[0m");
                } else {
                    eprintln!("\x1b[31m  {extension_name}: {message}\x1b[0m");
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
        let skin = make_skin();
        let width = crossterm::terminal::size()
            .map(|(w, _)| w as usize)
            .unwrap_or(80);

        eprintln!("\x1b[34m\u{25CF}\x1b[0m notification");
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
