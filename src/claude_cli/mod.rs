//! Clean Claude CLI module for calling Claude Code from any part of IronClaw.
//!
//! Provides strongly typed NDJSON event parsing, process management, and
//! high-level session abstractions for three execution modes:
//!
//! - **Oneshot**: `claude -p "prompt" --output-format stream-json`
//! - **Sidecar**: persistent process via `--input-format stream-json`
//! - **Resume**: `--resume <session_id>` to continue a previous session
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use ironclaw::claude_cli::{ClaudeCodeConfig, session};
//!
//! # async fn example() -> Result<(), ironclaw::claude_cli::ClaudeCliError> {
//! let config = ClaudeCodeConfig::new()
//!     .model("claude-sonnet-4-5-20250929")
//!     .max_turns(50)
//!     .dangerously_skip_permissions();
//!
//! // Oneshot: run a single prompt
//! let events = session::oneshot(&config, "List all TODO comments").await?;
//!
//! // Resume: continue a previous session
//! if let Some(sid) = events.iter().find_map(|e| e.session_id()) {
//!     let more = session::resume(&config, sid, "Now fix them").await?;
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Streaming
//!
//! For streaming access to events as they arrive, use [`ClaudeProcess`] directly:
//!
//! ```rust,no_run
//! use ironclaw::claude_cli::{ClaudeCodeConfig, ClaudeProcess, CliEvent, ContentBlock};
//!
//! # async fn example() -> Result<(), ironclaw::claude_cli::ClaudeCliError> {
//! let config = ClaudeCodeConfig::new().dangerously_skip_permissions();
//! let mut proc = ClaudeProcess::spawn_oneshot(&config, "Hello")?;
//!
//! while let Some(event) = proc.next_event().await {
//!     if let CliEvent::Assistant(asst) = &event {
//!         for block in &asst.content {
//!             if let ContentBlock::Text { text } = block {
//!                 print!("{}", text);
//!             }
//!         }
//!     }
//! }
//! proc.wait().await?;
//! # Ok(())
//! # }
//! ```

pub mod config;
pub mod error;
pub mod process;
pub mod session;
pub mod types;

pub use config::{ClaudeCodeConfig, PermissionMode};
pub use error::ClaudeCliError;
pub use process::ClaudeProcess;
pub use session::Session;
pub use types::{
    AssistantEvent, CliEvent, ContentBlock, ResultEvent, ResultInfo, SystemEvent, ToolInfo,
    UserEvent,
};
