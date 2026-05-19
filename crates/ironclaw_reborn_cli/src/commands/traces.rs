//! Stub `traces` subcommand for the standalone reborn CLI.
//!
//! The full TraceCommons CLI port is tracked separately. For now this
//! subcommand exposes the clap surface (`opt-in`, `status`, `queue-status`)
//! and points the user at the legacy `ironclaw traces` binary until the
//! reborn-cli port lands. The two read-only subcommands (`status` and
//! `queue-status`) call into the extracted `ironclaw_reborn_traces` crate
//! using the *anonymous* (no-scope) trace contribution directory so the
//! wiring is exercised end-to-end at compile time.

use clap::{Args, Subcommand};
use ironclaw_reborn_traces::contribution as traces;

#[derive(Debug, Args)]
pub(crate) struct TracesCommand {
    #[command(subcommand)]
    command: TracesSubcommand,
}

#[derive(Debug, Subcommand)]
enum TracesSubcommand {
    /// Opt in to TraceCommons contributions.
    OptIn(OptInCommand),
    /// Show the standing trace contribution policy for the anonymous scope.
    Status(StatusCommand),
    /// Show how many trace envelopes are queued for submission.
    QueueStatus(QueueStatusCommand),
}

#[derive(Debug, Args)]
struct OptInCommand {}

#[derive(Debug, Args)]
struct StatusCommand {}

#[derive(Debug, Args)]
struct QueueStatusCommand {}

impl TracesCommand {
    pub(crate) fn execute(self) -> anyhow::Result<()> {
        match self.command {
            TracesSubcommand::OptIn(_) => {
                println!(
                    "Trace opt-in is not yet implemented in the reborn CLI \
                     — use the legacy `ironclaw traces opt-in` binary until \
                     the reborn-cli port is complete."
                );
                Ok(())
            }
            TracesSubcommand::Status(_) => {
                let policy = traces::read_trace_policy_for_scope(None)?;
                println!("Trace contribution policy (anonymous scope): {policy:?}");
                Ok(())
            }
            TracesSubcommand::QueueStatus(_) => {
                let queued = traces::queued_trace_envelope_paths_for_scope(None)?;
                println!(
                    "Queued trace envelopes (anonymous scope): {} pending",
                    queued.len()
                );
                Ok(())
            }
        }
    }
}
