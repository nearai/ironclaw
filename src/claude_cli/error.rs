//! Error types for the Claude CLI module.

use std::io;

/// Errors from Claude CLI process management and event parsing.
#[derive(Debug, thiserror::Error)]
pub enum ClaudeCliError {
    #[error("failed to spawn claude process: {0}")]
    SpawnFailed(#[source] io::Error),

    #[error("claude process exited with code {code}")]
    ProcessFailed { code: i32, stderr: String },

    #[error("claude process was killed by signal")]
    ProcessKilled,

    #[error("failed to parse NDJSON event: {reason}")]
    ParseError { reason: String, raw: String },

    #[error("stdin write failed: {0}")]
    StdinWriteFailed(#[source] io::Error),

    #[error("session not in sidecar mode (no stdin available)")]
    NotSidecarMode,

    #[error("session already closed")]
    SessionClosed,

    #[error("io error: {0}")]
    Io(#[from] io::Error),
}
