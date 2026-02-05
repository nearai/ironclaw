//! CLI command handling.
//!
//! Provides subcommands for:
//! - Running the agent (`run`)
//! - Interactive setup wizard (`setup`)
//! - Managing configuration (`config list`, `config get`, `config set`)
//! - Managing WASM tools (`tool install`, `tool list`, `tool remove`)

mod config;
mod tool;

pub use config::{ConfigCommand, run_config_command};
pub use tool::{ToolCommand, run_tool_command};

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "ironclaw")]
#[command(
    about = "Secure personal AI assistant that protects your data and expands its capabilities"
)]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Run in interactive CLI mode only (disable other channels)
    #[arg(long, global = true)]
    pub cli_only: bool,

    /// Skip database connection (for testing)
    #[arg(long, global = true)]
    pub no_db: bool,

    /// Simple REPL mode without TUI (for testing)
    #[arg(long, global = true)]
    pub repl: bool,

    /// Single message mode - send one message and exit
    #[arg(short, long, global = true)]
    pub message: Option<String>,

    /// Configuration file path (optional, uses env vars by default)
    #[arg(short, long, global = true)]
    pub config: Option<std::path::PathBuf>,

    /// Skip first-run setup check
    #[arg(long, global = true)]
    pub no_setup: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the agent (default if no subcommand given)
    Run,

    /// Interactive setup wizard
    Setup {
        /// Skip authentication (use existing session)
        #[arg(long)]
        skip_auth: bool,

        /// Reconfigure channels only
        #[arg(long)]
        channels_only: bool,
    },

    /// Manage configuration settings
    #[command(subcommand)]
    Config(ConfigCommand),

    /// Manage WASM tools
    #[command(subcommand)]
    Tool(ToolCommand),
}

impl Cli {
    /// Check if we should run the agent (default behavior or explicit `run` command).
    pub fn should_run_agent(&self) -> bool {
        matches!(self.command, None | Some(Command::Run))
    }
}
