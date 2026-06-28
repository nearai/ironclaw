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

/// Inert process port: records every `CommandExecutionRequest` and returns a
/// benign success (`exit_code = 0`, empty stdout/stderr) without spawning any
/// OS process.
#[derive(Debug, Clone, Default)]
pub struct RecordingProcessPort {
    commands: Arc<Mutex<Vec<String>>>,
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
        Ok(CommandExecutionOutput {
            output: String::new(),
            saved_output: None,
            exit_code: 0,
            sandboxed: false,
            duration: Duration::from_millis(0),
        })
    }
}
