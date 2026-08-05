//! Placement-neutral process-execution vocabulary and the sandbox transport
//! port.
//!
//! The kernel decides *which* process port receives a command; a lane provides
//! the transport that runs it. Declaring both halves here is what lets a
//! `runtimes`-layer lane implement what the kernel consumes without an upward
//! dependency: `ironclaw_sandbox` (runtimes) implements
//! [`SandboxCommandTransport`], `ironclaw_host_runtime` (kernel) wraps it in
//! `UserSandboxProcessPort`. PROPOSAL §6.6.4 records that this home is
//! load-bearing, not cosmetic.
//!
//! `ironclaw_host_runtime` still owns the *behavior* — process spawning, output
//! capture, alias rewriting, and the local-host port. Only the shapes that
//! cross the kernel↔lane seam live here.

use std::{collections::HashMap, path::PathBuf, time::Duration};

use async_trait::async_trait;
use thiserror::Error;

use crate::{mount::MountView, resource::ResourceScope};

/// Metadata for command output persisted behind a saved-output reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedCommandOutput {
    pub path: PathBuf,
    pub sanitization: SavedCommandOutputSanitization,
    pub stream_was_capped: bool,
    pub max_saved_stream_size: usize,
    pub expires_at_unix_secs: u64,
}

/// Whether persisted command output required redaction or blocking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavedCommandOutputSanitization {
    Clean,
    Redacted,
    Blocked,
}

/// Placement-neutral command request handed to the selected process port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandExecutionRequest {
    pub scope: ResourceScope,
    pub mounts: Option<MountView>,
    pub command: String,
    pub workdir: Option<String>,
    pub timeout_secs: Option<u64>,
    pub extra_env: HashMap<String, String>,
}

/// Process-port command result normalized for capability handlers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandExecutionOutput {
    pub output: String,
    pub saved_output: Option<SavedCommandOutput>,
    pub exit_code: i64,
    pub sandboxed: bool,
    pub duration: Duration,
}

/// Stable redacted process-port failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuntimeProcessError {
    #[error("command timed out after {0:?}")]
    Timeout(Duration),
    #[error("process execution failed: {0}")]
    ExecutionFailed(String),
}

/// Transport for user-sandbox command execution.
///
/// This trait intentionally hides Docker/daemon details from host-runtime tool
/// code. A lane implements it with a container runtime or another runner that
/// isolates each authenticated user within the tenant boundary.
///
/// Implementations must enforce [`CommandExecutionRequest::timeout_secs`] and
/// clean up any remote process/container before returning
/// [`RuntimeProcessError::Timeout`].
#[async_trait]
pub trait SandboxCommandTransport: Send + Sync {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError>;
}
