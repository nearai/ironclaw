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
//!   Approval prompts intentionally stay text-based for parity with other channels;
//!   there is no arrow-key selector path.
//! - `Esc` - Interrupt current operation

use std::borrow::Cow;
use std::io::{self, IsTerminal, Write};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use rustyline::completion::Completer;
use rustyline::config::Config;
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{
    Cmd as ReadlineCmd, CompletionType, ConditionalEventHandler, Editor, Event, EventContext,
    EventHandler, Helper, KeyCode, KeyEvent, Modifiers, RepeatCount,
};
use termimad::MadSkin;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use crate::agent::truncate_for_preview;
use crate::bootstrap::ironclaw_base_dir;
use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::cli::fmt;
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
    "/reasoning",
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
        Cow::Owned(format!("{}{hint}{}", fmt::dim(), fmt::reset()))
    }
}

impl Validator for ReplHelper {}
impl Helper for ReplHelper {}

struct EscInterruptHandler {
    triggered: Arc<AtomicBool>,
}

impl ConditionalEventHandler for EscInterruptHandler {
    fn handle(
        &self,
        _evt: &Event,
        _n: RepeatCount,
        _positive: bool,
        _ctx: &EventContext,
    ) -> Option<ReadlineCmd> {
        self.triggered.store(true, Ordering::Relaxed);
        Some(ReadlineCmd::Interrupt)
    }
}

// REPL approvals intentionally use the same plain-text yes/no/always path as
// other channels. That keeps the terminal flow aligned with SubmissionParser
// and avoids a selector-specific approval path that can diverge from engine
// gate handling.

/// Build a termimad skin with our color scheme.
fn make_skin() -> MadSkin {
    let mut skin = MadSkin::default();
    skin.set_headers_fg(crossterm::style::Color::Yellow);
    skin.bold.set_fg(crossterm::style::Color::White);
    skin.italic.set_fg(crossterm::style::Color::Magenta);
    skin.inline_code.set_fg(crossterm::style::Color::Green);
    skin.code_block.set_fg(crossterm::style::Color::Green);
    skin.code_block.left_margin = 2;
    skin
}

/// Truncate a string to `max_chars` using character boundaries.
///
/// For strings longer than `max_chars`, shows the first half and last half
/// separated by `...` so both ends are visible.
fn smart_truncate(s: &str, max_chars: usize) -> Cow<'_, str> {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        return Cow::Borrowed(s);
    }
    // Account for the 3-char "..." separator
    let budget = max_chars.saturating_sub(3);
    let head_len = budget / 2;
    let tail_len = budget - head_len;
    let head: String = s.chars().take(head_len).collect();
    let tail: String = s
        .chars()
        .skip(char_count.saturating_sub(tail_len))
        .collect();
    Cow::Owned(format!("{head}...{tail}"))
}

/// Format JSON params as `key: value` lines for the approval card.
fn format_json_params(params: &serde_json::Value, indent: &str) -> String {
    let max_val_len = fmt::term_width().saturating_sub(8);

    match params {
        serde_json::Value::Object(map) => {
            let mut lines = Vec::new();
            for (key, value) in map {
                let val_str = match value {
                    serde_json::Value::String(s) => {
                        let display = smart_truncate(s, max_val_len);
                        format!("{}\"{display}\"{}", fmt::success(), fmt::reset())
                    }
                    other => {
                        let rendered = other.to_string();
                        smart_truncate(&rendered, max_val_len).into_owned()
                    }
                };
                lines.push(format!(
                    "{indent}{}{key}{}: {val_str}",
                    fmt::accent(),
                    fmt::reset()
                ));
            }
            lines.join("\n")
        }
        other => {
            let pretty = serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string());
            let truncated = smart_truncate(&pretty, 300);
            truncated
                .lines()
                .map(|l| format!("{indent}{}{l}{}", fmt::dim(), fmt::reset()))
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
}

/// REPL channel with line editing and markdown rendering.
pub struct ReplChannel {
    /// Stable owner scope for this REPL instance.
    user_id: String,
    /// Optional single message to send (for -m flag).
    single_message: Option<String>,
    /// Debug mode flag (shared with input thread).
    debug_mode: Arc<AtomicBool>,
    /// Whether we're currently streaming (chunks have been printed without a trailing newline).
    is_streaming: Arc<AtomicBool>,
    /// When true, the one-liner startup banner is suppressed (boot screen shown instead).
    suppress_banner: Arc<AtomicBool>,
    /// Sender to inject messages into the agent loop (set after start()).
    msg_tx: Arc<Mutex<Option<mpsc::Sender<IncomingMessage>>>>,
    /// Pending approval request currently shown in the REPL.
    pending_approval: Arc<Mutex<Option<Uuid>>>,
    /// When true, the readline thread must yield stdin (approval selector or agent processing).
    stdin_locked: Arc<AtomicBool>,
    /// Number of transient status lines (Thinking) to erase on next output.
    transient_lines: std::sync::atomic::AtomicU8,
}

impl ReplChannel {
    /// Create a new REPL channel.
    pub fn new() -> Self {
        Self::with_user_id("default")
    }

    /// Create a new REPL channel for a specific owner scope.
    pub fn with_user_id(user_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            single_message: None,
            debug_mode: Arc::new(AtomicBool::new(false)),
            is_streaming: Arc::new(AtomicBool::new(false)),
            suppress_banner: Arc::new(AtomicBool::new(false)),
            msg_tx: Arc::new(Mutex::new(None)),
            pending_approval: Arc::new(Mutex::new(None)),
            stdin_locked: Arc::new(AtomicBool::new(false)),
            transient_lines: std::sync::atomic::AtomicU8::new(0),
        }
    }

    /// Create a REPL channel that sends a single message and exits.
    pub fn with_message(message: String) -> Self {
        Self::with_message_for_user("default", message)
    }

    /// Create a REPL channel that sends a single message for a specific owner scope and exits.
    pub fn with_message_for_user(user_id: impl Into<String>, message: String) -> Self {
        Self {
            user_id: user_id.into(),
            single_message: Some(message),
            debug_mode: Arc::new(AtomicBool::new(false)),
            is_streaming: Arc::new(AtomicBool::new(false)),
            suppress_banner: Arc::new(AtomicBool::new(false)),
            msg_tx: Arc::new(Mutex::new(None)),
            pending_approval: Arc::new(Mutex::new(None)),
            stdin_locked: Arc::new(AtomicBool::new(false)),
            transient_lines: std::sync::atomic::AtomicU8::new(0),
        }
    }

    /// Suppress the one-liner startup banner (boot screen will be shown instead).
    pub fn suppress_banner(&self) {
        self.suppress_banner.store(true, Ordering::Relaxed);
    }

    fn is_debug(&self) -> bool {
        self.debug_mode.load(Ordering::Relaxed)
    }

    /// Erase transient status lines (Thinking indicators) from the terminal.
    fn clear_transient(&self) {
        use crossterm::{cursor, execute, terminal};
        let n = self.transient_lines.swap(0, Ordering::Relaxed);
        if n > 0 {
            let mut stderr = io::stderr();
            let _ = execute!(stderr, cursor::MoveUp(n as u16));
            let _ = execute!(stderr, terminal::Clear(terminal::ClearType::FromCursorDown));
        }
    }

    async fn finish_single_message_turn(&self) {
        if self.single_message.is_none() {
            return;
        }

        let tx = self.msg_tx.lock().ok().and_then(|mut guard| guard.take());
        if let Some(tx) = tx {
            let msg = IncomingMessage::new("repl", &self.user_id, "/quit");
            let _ = tx.send(msg).await;
        }
    }
}

impl Default for ReplChannel {
    fn default() -> Self {
        Self::new()
    }
}

fn rewrite_repl_approval_submission(
    line: &str,
    pending_approval: &Arc<Mutex<Option<Uuid>>>,
) -> Option<String> {
    let submission = crate::agent::SubmissionParser::parse(line);
    let crate::agent::Submission::ApprovalResponse { approved, always } = submission else {
        return None;
    };

    let request_id = pending_approval
        .lock()
        .ok()
        .and_then(|mut guard| guard.take())?;
    serde_json::to_string(&crate::agent::Submission::ExecApproval {
        request_id,
        approved,
        always,
    })
    .ok()
}

fn print_help() {
    let h = fmt::bold();
    let c = fmt::bold_accent();
    let d = fmt::dim();
    let r = fmt::reset();
    let hi = fmt::hint();

    println!();
    println!("  {h}IronClaw REPL{r}");
    println!();
    println!("  {h}Quick start{r}");
    println!("    {c}/new{r}         {hi}Start a new thread{r}");
    println!("    {c}/compact{r}     {hi}Compress context window{r}");
    println!("    {c}/quit{r}        {hi}Exit{r}");
    println!();
    println!("  {h}All commands{r}");
    println!(
        "    {d}Conversation{r}  {c}/new{r} {c}/clear{r} {c}/compact{r} {c}/undo{r} {c}/redo{r} {c}/summarize{r} {c}/suggest{r}"
    );
    println!("    {d}Threads{r}       {c}/thread{r} {c}/resume{r} {c}/list{r}");
    println!("    {d}Execution{r}     {c}/interrupt{r} {d}(esc){r} {c}/cancel{r}");
    println!(
        "    {d}System{r}        {c}/tools{r} {c}/model{r} {c}/version{r} {c}/status{r} {c}/debug{r} {c}/heartbeat{r}"
    );
    println!("    {d}Session{r}       {c}/help{r} {c}/quit{r}");
    println!();
}

/// Get the history file path (~/.ironclaw/history).
fn history_path() -> std::path::PathBuf {
    ironclaw_base_dir().join("history")
}

#[async_trait]
impl Channel for ReplChannel {
    fn name(&self) -> &str {
        "repl"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = mpsc::channel(32);
        // Store tx so send_status can inject approval responses directly.
        // Skip for single-message mode — no interactive approval is needed
        // and the extra sender would keep the stream open after /quit.
        if self.single_message.is_none()
            && let Ok(mut guard) = self.msg_tx.lock()
        {
            *guard = Some(tx.clone());
        }
        let single_message = self.single_message.clone();
        let user_id = self.user_id.clone();
        let debug_mode = Arc::clone(&self.debug_mode);
        let suppress_banner = Arc::clone(&self.suppress_banner);
        let pending_approval = Arc::clone(&self.pending_approval);
        let stdin_locked = Arc::clone(&self.stdin_locked);
        let esc_interrupt_triggered_for_thread = Arc::new(AtomicBool::new(false));

        std::thread::spawn(move || {
            let sys_tz = crate::timezone::detect_system_timezone().name().to_string();

            // Single message mode: send it and return
            if let Some(msg) = single_message {
                let incoming = IncomingMessage::new("repl", &user_id, &msg)
                    .with_metadata(serde_json::json!({ "single_message_mode": true }))
                    .with_timezone(&sys_tz);
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

            rl.bind_sequence(
                KeyEvent(KeyCode::Esc, Modifiers::NONE),
                EventHandler::Conditional(Box::new(EscInterruptHandler {
                    triggered: Arc::clone(&esc_interrupt_triggered_for_thread),
                })),
            );

            // Load history
            let hist_path = history_path();
            if let Some(parent) = hist_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = rl.load_history(&hist_path);

            if !suppress_banner.load(Ordering::Relaxed) {
                println!(
                    "{}IronClaw{}  /help for commands, /quit to exit",
                    fmt::bold(),
                    fmt::reset()
                );
                println!();
            }

            loop {
                // Yield stdin while approval selector or agent processing locks it
                while stdin_locked.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }

                let prompt = if debug_mode.load(Ordering::Relaxed) {
                    format!(
                        "{}[debug]{} {}\u{203A}{} ",
                        fmt::warning(),
                        fmt::reset(),
                        fmt::bold_accent(),
                        fmt::reset()
                    )
                } else {
                    format!("{}\u{203A}{} ", fmt::bold_accent(), fmt::reset())
                };

                match rl.readline(&prompt) {
                    Ok(line) => {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }

                        // Handle local REPL commands (only commands that need
                        // immediate local handling stay here)
                        match line.to_lowercase().as_str() {
                            "/quit" | "/exit" => {
                                // Forward shutdown command so the agent loop exits even
                                // when other channels (e.g. web gateway) are still active.
                                let msg = IncomingMessage::new("repl", &user_id, "/quit")
                                    .with_timezone(&sys_tz);
                                let _ = tx.blocking_send(msg);
                                break;
                            }
                            "/help" => {
                                print_help();
                                continue;
                            }
                            "/debug" => {
                                let current = debug_mode.load(Ordering::Relaxed);
                                debug_mode.store(!current, Ordering::Relaxed);
                                if !current {
                                    println!("{}debug mode on{}", fmt::dim(), fmt::reset());
                                } else {
                                    println!("{}debug mode off{}", fmt::dim(), fmt::reset());
                                }
                                continue;
                            }
                            _ => {}
                        }

                        let content = rewrite_repl_approval_submission(line, &pending_approval)
                            .unwrap_or_else(|| line.to_string());
                        let msg =
                            IncomingMessage::new("repl", &user_id, content).with_timezone(&sys_tz);
                        // Lock stdin before sending so readline doesn't restart
                        // while the agent is processing (approval selector needs stdin)
                        stdin_locked.store(true, Ordering::Relaxed);
                        if tx.blocking_send(msg).is_err() {
                            stdin_locked.store(false, Ordering::Relaxed);
                            break;
                        }
                    }
                    Err(ReadlineError::Interrupted) => {
                        if esc_interrupt_triggered_for_thread.swap(false, Ordering::Relaxed) {
                            // Esc: interrupt current operation and keep REPL open.
                            let msg = IncomingMessage::new("repl", &user_id, "/interrupt")
                                .with_timezone(&sys_tz);
                            if tx.blocking_send(msg).is_err() {
                                break;
                            }
                        } else {
                            // Ctrl+C (VINTR): request graceful shutdown.
                            let msg = IncomingMessage::new("repl", &user_id, "/quit")
                                .with_timezone(&sys_tz);
                            let _ = tx.blocking_send(msg);
                            break;
                        }
                    }
                    Err(ReadlineError::Eof) => {
                        // Ctrl+D in interactive mode: graceful shutdown.
                        // In daemon mode (stdin = /dev/null, no TTY), EOF arrives
                        // immediately — just drop the REPL thread silently so other
                        // channels (gateway, telegram, …) keep running.
                        if std::io::stdin().is_terminal() {
                            let msg = IncomingMessage::new("repl", &user_id, "/quit")
                                .with_timezone(&sys_tz);
                            let _ = tx.blocking_send(msg);
                        }
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
        let approval_prompt_already_rendered = self
            .pending_approval
            .lock()
            .ok()
            .and_then(|guard| *guard)
            .is_some()
            && response.content.contains("requires approval");
        if approval_prompt_already_rendered {
            self.stdin_locked.store(false, Ordering::Relaxed);
            self.finish_single_message_turn().await;
            return Ok(());
        }

        let width = fmt::term_width();

        // If we were streaming, the content was already printed via StreamChunk.
        // Just finish the line and reset.
        if self.is_streaming.swap(false, Ordering::Relaxed) {
            println!();
            println!();
            self.stdin_locked.store(false, Ordering::Relaxed);
            self.finish_single_message_turn().await;
            return Ok(());
        }

        // Clear any leftover thinking indicators
        self.clear_transient();

        // Dim separator line before the response
        let sep_width = width.min(80);
        eprintln!("{}", fmt::separator(sep_width));

        // Render markdown
        let skin = make_skin();
        let text = termimad::FmtText::from(&skin, &response.content, Some(width));

        print!("{text}");
        println!();
        // Unlock stdin so readline can resume
        self.stdin_locked.store(false, Ordering::Relaxed);
        self.finish_single_message_turn().await;
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
                self.clear_transient();
                let display = truncate_for_preview(&msg, CLI_STATUS_MAX);
                eprintln!("  {}\u{25CB} {display}{}", fmt::dim(), fmt::reset());
                self.transient_lines.store(1, Ordering::Relaxed);
            }
            StatusUpdate::ToolStarted { name } => {
                self.clear_transient();
                eprintln!("  {}\u{25CB} {name}{}", fmt::dim(), fmt::reset());
                self.transient_lines.store(1, Ordering::Relaxed);
            }
            StatusUpdate::ToolCompleted { name, success, .. } => {
                self.clear_transient();
                if success {
                    eprintln!("  {}\u{25CF} {name}{}", fmt::success(), fmt::reset());
                } else {
                    eprintln!("  {}\u{2717} {name} (failed){}", fmt::error(), fmt::reset());
                }
            }
            StatusUpdate::ToolResult { name: _, preview } => {
                let display = truncate_for_preview(&preview, CLI_TOOL_RESULT_MAX);
                eprintln!("    {}{display}{}", fmt::dim(), fmt::reset());
            }
            StatusUpdate::StreamChunk(chunk) => {
                // Print separator on the false-to-true transition
                if !self.is_streaming.swap(true, Ordering::Relaxed) {
                    self.clear_transient();
                    let sep_width = fmt::term_width().min(80);
                    eprintln!("{}", fmt::separator(sep_width));
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
                    "  {}[job]{} {title} {}({job_id}){} {}{browse_url}{}",
                    fmt::accent(),
                    fmt::reset(),
                    fmt::dim(),
                    fmt::reset(),
                    fmt::link(),
                    fmt::reset()
                );
            }
            StatusUpdate::Status(msg) => {
                if debug || msg.contains("approval") || msg.contains("Approval") {
                    let display = truncate_for_preview(&msg, CLI_STATUS_MAX);
                    eprintln!("  {}{display}{}", fmt::dim(), fmt::reset());
                }
            }
            StatusUpdate::ApprovalNeeded {
                request_id,
                tool_name,
                description: _,
                parameters,
                allow_always,
            } => {
                if let Ok(parsed) = Uuid::parse_str(&request_id)
                    && let Ok(mut guard) = self.pending_approval.lock()
                {
                    *guard = Some(parsed);
                }
                self.clear_transient();
                let pipe = format!("{}│{}", fmt::accent(), fmt::reset());

                // Header: ◆ tool requires approval
                eprintln!();
                eprintln!(
                    "  {}\u{25C6}  {}{tool_name}{} requires approval",
                    fmt::accent(),
                    fmt::bold(),
                    fmt::reset()
                );

                // Params: │  key  value
                let param_lines = format_json_params(&parameters, &format!("  {pipe}  "));
                if !param_lines.is_empty() {
                    eprintln!("  {pipe}");
                    for line in param_lines.lines() {
                        eprintln!("{line}");
                    }
                }
                eprintln!("  {pipe}");
                let choices = if allow_always {
                    "Reply 'yes' to approve, 'always' to auto-approve, or 'no' to deny."
                } else {
                    "Reply 'yes' to approve or 'no' to deny."
                };
                eprintln!("  {}{choices}{}", fmt::dim(), fmt::reset());
                // Resume readline so the user can answer via the normal
                // submission parser path.
                self.stdin_locked.store(false, Ordering::Relaxed);
            }
            StatusUpdate::AuthRequired {
                extension_name,
                instructions,
                setup_url,
                ..
            } => {
                if let Ok(mut guard) = self.pending_approval.lock() {
                    *guard = None;
                }
                eprintln!();
                eprintln!(
                    "{}  Authentication required for {extension_name}{}",
                    fmt::warning(),
                    fmt::reset()
                );
                if let Some(ref instr) = instructions {
                    eprintln!("  {instr}");
                }
                if let Some(ref url) = setup_url {
                    eprintln!("  {}{url}{}", fmt::link(), fmt::reset());
                }
                eprintln!();
            }
            StatusUpdate::AuthCompleted {
                extension_name,
                success,
                message,
            } => {
                if let Ok(mut guard) = self.pending_approval.lock() {
                    *guard = None;
                }
                if success {
                    eprintln!(
                        "{}  {extension_name}: {message}{}",
                        fmt::success(),
                        fmt::reset()
                    );
                } else {
                    eprintln!(
                        "{}  {extension_name}: {message}{}",
                        fmt::error(),
                        fmt::reset()
                    );
                }
            }
            StatusUpdate::ImageGenerated { path, .. } => {
                if let Some(ref p) = path {
                    eprintln!("{}  [image] {p}{}", fmt::accent(), fmt::reset());
                } else {
                    eprintln!("{}  [image generated]{}", fmt::accent(), fmt::reset());
                }
            }
            StatusUpdate::Suggestions { .. } => {
                // Suggestions are only rendered by the web gateway
            }
            StatusUpdate::ReasoningUpdate {
                narrative,
                decisions,
            } => {
                if !narrative.is_empty() {
                    let display = truncate_for_preview(&narrative, CLI_STATUS_MAX);
                    eprintln!("  \x1b[94m\u{25B6} {display}\x1b[0m");
                }
                for d in &decisions {
                    let display = truncate_for_preview(&d.rationale, CLI_STATUS_MAX);
                    eprintln!("    \x1b[90m\u{2192} {}: {display}\x1b[0m", d.tool_name);
                }
            }
            StatusUpdate::TurnCost { .. } => {
                // Cost display is handled by the TUI channel
            }
            StatusUpdate::SkillActivated { skill_names } => {
                if !skill_names.is_empty() {
                    eprintln!(
                        "  \x1b[36m\u{25C8} skills: {}\x1b[0m",
                        skill_names.join(", ")
                    );
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
        let width = fmt::term_width();

        eprintln!("{}\u{25CF}{} notification", fmt::accent(), fmt::reset());
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

#[cfg(test)]
mod tests {
    use futures::StreamExt;
    use tokio::time::{Duration, timeout};

    use super::*;

    /// Regression: single-message mode must close the stream after the one
    /// message so callers (and tests) don't hang forever.
    #[tokio::test]
    async fn single_message_mode_sends_message_and_closes_stream() {
        let repl = ReplChannel::with_message("hi".to_string());
        let mut stream = repl.start().await.expect("repl start should succeed");

        let first = timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("timed out waiting for first message")
            .expect("first message missing");
        assert_eq!(first.channel, "repl");
        assert_eq!(first.content, "hi");

        // The spawned thread sent the message and returned, dropping its
        // sender. Because we skip storing a clone in msg_tx for single-
        // message mode, the stream should close immediately.
        assert!(
            timeout(Duration::from_secs(1), stream.next())
                .await
                .expect("timed out waiting for stream to close")
                .is_none(),
            "stream should end after the single message"
        );
    }
}
