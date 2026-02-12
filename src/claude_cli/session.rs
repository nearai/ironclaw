//! High-level session abstractions for Claude CLI.
//!
//! Provides convenience functions for common usage patterns:
//! - [`oneshot`] - Run a single prompt and collect all events
//! - [`resume`] - Continue a previous session by ID
//! - [`Session`] - Persistent sidecar process for multi-turn conversations

use crate::claude_cli::config::ClaudeCodeConfig;
use crate::claude_cli::error::ClaudeCliError;
use crate::claude_cli::process::ClaudeProcess;
use crate::claude_cli::types::CliEvent;

/// Run Claude CLI in oneshot mode and collect all events.
///
/// Spawns the process, reads all NDJSON events, waits for exit, and returns
/// the collected events. This is the simplest way to call Claude Code.
///
/// ```rust,no_run
/// use ironclaw::claude_cli::{ClaudeCodeConfig, session};
///
/// # async fn example() -> Result<(), ironclaw::claude_cli::ClaudeCliError> {
/// let config = ClaudeCodeConfig::new()
///     .model("claude-sonnet-4-5-20250929")
///     .dangerously_skip_permissions();
///
/// let events = session::oneshot(&config, "What files are in the current directory?").await?;
/// for event in &events {
///     if let ironclaw::claude_cli::CliEvent::Assistant(asst) = event {
///         for block in &asst.content {
///             if let ironclaw::claude_cli::ContentBlock::Text { text } = block {
///                 println!("{}", text);
///             }
///         }
///     }
/// }
/// # Ok(())
/// # }
/// ```
pub async fn oneshot(
    config: &ClaudeCodeConfig,
    prompt: &str,
) -> Result<Vec<CliEvent>, ClaudeCliError> {
    let mut process = ClaudeProcess::spawn_oneshot(config, prompt)?;
    let events = collect_all(&mut process).await;
    process.wait().await?;
    Ok(events)
}

/// Resume a previous Claude CLI session by session ID.
///
/// Spawns a new process with `--resume <session_id>` and collects all events.
pub async fn resume(
    config: &ClaudeCodeConfig,
    session_id: &str,
    prompt: &str,
) -> Result<Vec<CliEvent>, ClaudeCliError> {
    let mut process = ClaudeProcess::spawn_resume(config, session_id, prompt)?;
    let events = collect_all(&mut process).await;
    process.wait().await?;
    Ok(events)
}

/// A persistent Claude CLI session (sidecar mode).
///
/// Keeps the `claude` process alive with `--input-format stream-json` and allows
/// sending multiple prompts over stdin. Each call to [`send`](Self::send) writes
/// a message and collects events until a result event is received.
///
/// ```rust,no_run
/// use ironclaw::claude_cli::{ClaudeCodeConfig, session::Session};
///
/// # async fn example() -> Result<(), ironclaw::claude_cli::ClaudeCliError> {
/// let config = ClaudeCodeConfig::new()
///     .model("claude-sonnet-4-5-20250929")
///     .dangerously_skip_permissions();
///
/// let mut session = Session::start(&config)?;
///
/// let events = session.send("What is 2 + 2?").await?;
/// let events = session.send("Now multiply that by 10").await?;
///
/// session.close().await?;
/// # Ok(())
/// # }
/// ```
pub struct Session {
    process: ClaudeProcess,
}

impl Session {
    /// Start a new sidecar session.
    pub fn start(config: &ClaudeCodeConfig) -> Result<Self, ClaudeCliError> {
        let process = ClaudeProcess::spawn_sidecar(config)?;
        Ok(Self { process })
    }

    /// Send a prompt and collect all events until the result.
    ///
    /// Blocks until a `result` event is received, then returns all events
    /// from this turn (including the result).
    pub async fn send(&mut self, prompt: &str) -> Result<Vec<CliEvent>, ClaudeCliError> {
        self.send_prompt(prompt).await?;

        let mut events = Vec::new();
        while let Some(event) = self.process.next_event().await {
            let is_result = event.is_result();
            events.push(event);
            if is_result {
                break;
            }
        }

        Ok(events)
    }

    /// Send a prompt without waiting for a response.
    ///
    /// Use [`next_event`](Self::next_event) to read the response events.
    pub async fn send_prompt(&mut self, prompt: &str) -> Result<(), ClaudeCliError> {
        let message = serde_json::json!({
            "type": "user",
            "content": prompt,
        });
        self.process.write_stdin(&message).await
    }

    /// Receive the next event from the session.
    ///
    /// Returns `None` if the process has exited.
    pub async fn next_event(&mut self) -> Option<CliEvent> {
        self.process.next_event().await
    }

    /// Close the session by dropping stdin and waiting for the process to exit.
    pub async fn close(self) -> Result<(), ClaudeCliError> {
        self.process.wait().await
    }
}

async fn collect_all(process: &mut ClaudeProcess) -> Vec<CliEvent> {
    let mut events = Vec::new();
    while let Some(event) = process.next_event().await {
        events.push(event);
    }
    events
}
