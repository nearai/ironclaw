//! Inert recording process port for the integration-test harness (slice 5).
//!
//! `RecordingProcessPort` implements `RuntimeProcessPort` but never spawns a
//! real OS process. Every `run_command` call records the command string and
//! returns a benign success response (exit 0, empty stdout/stderr). The
//! recorder is the default for the `BuiltinHttpTools` capability backend so
//! that `builtin.shell` test turns are safe to run without any system setup.

// Not every test binary that mounts this support tree exercises the recording
// process port — mirrors the `#![allow(dead_code)]` used in sibling modules.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_host_runtime::{
    CommandExecutionOutput, CommandExecutionRequest, RuntimeProcessError, RuntimeProcessPort,
};

/// A scripted `run_command` result for the recording process port. Sticky:
/// once set, EVERY `run_command` call returns it (after recording the command),
/// so a retryable failure surfaces consistently across the loop's retry budget
/// instead of being consumed once from a FIFO queue.
#[derive(Debug, Clone)]
pub enum ScriptedProcessResult {
    /// Return a benign success with this exit code (non-zero drives the tool's
    /// `success: false` / `exit_code` model-visible output — still a Completed
    /// tool result, not an error).
    ExitCode(i64),
    /// Return `Err(RuntimeProcessError::Timeout(..))` — the tool maps this to a
    /// recoverable `Failed{Resource}` capability error.
    Timeout,
}

/// Inert process port: records every `CommandExecutionRequest` and, by default,
/// returns a benign success (`exit_code = 0`, empty stdout/stderr) without
/// spawning any OS process. A scripted result (via [`set_scripted`]) overrides
/// the default for every subsequent call.
///
/// [`set_scripted`]: RecordingProcessPort::set_scripted
#[derive(Debug, Clone, Default)]
pub struct RecordingProcessPort {
    commands: Arc<Mutex<Vec<String>>>,
    scripted: Arc<Mutex<Option<ScriptedProcessResult>>>,
}

impl RecordingProcessPort {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of every command string recorded so far, in call order.
    pub fn commands(&self) -> Vec<String> {
        self.commands
            .lock()
            .expect("recording process port lock poisoned")
            .clone()
    }

    /// Install a sticky scripted result returned by every subsequent
    /// `run_command` call (the command is still recorded first).
    pub fn set_scripted(&self, result: ScriptedProcessResult) {
        *self
            .scripted
            .lock()
            .expect("recording process port lock poisoned") = Some(result);
    }
}

#[async_trait]
impl RuntimeProcessPort for RecordingProcessPort {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        self.commands
            .lock()
            .expect("recording process port lock poisoned")
            .push(request.command.clone());
        let scripted = self
            .scripted
            .lock()
            .expect("recording process port lock poisoned")
            .clone();
        let exit_code = match scripted {
            None => 0,
            Some(ScriptedProcessResult::ExitCode(code)) => code,
            Some(ScriptedProcessResult::Timeout) => {
                let timeout_secs = request.timeout_secs.unwrap_or(1);
                return Err(RuntimeProcessError::Timeout(Duration::from_secs(
                    timeout_secs,
                )));
            }
        };
        Ok(CommandExecutionOutput {
            output: String::new(),
            saved_output: None,
            exit_code,
            sandboxed: false,
            duration: Duration::from_millis(0),
        })
    }
}
