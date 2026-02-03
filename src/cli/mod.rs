//! CLI command handling.
//!
//! Provides subcommands for:
//! - Running the agent (`run`)
//! - Managing WASM tools (`tool install`, `tool list`, `tool remove`)
//! - Managing secrets (`secret set`, `secret list`, `secret remove`)

mod tool;

pub use tool::{ToolCommand, run_tool_command};

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "near-agent")]
#[command(about = "LLM-powered autonomous agent for the NEAR AI marketplace")]
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
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the agent (default if no subcommand given)
    Run,

    /// Manage WASM tools
    #[command(subcommand)]
    Tool(ToolCommand),
    // Future: Secret management
    // #[command(subcommand)]
    // Secret(SecretCommand),
}

impl Cli {
    /// Check if we should run the agent (default behavior or explicit `run` command).
    pub fn should_run_agent(&self) -> bool {
        matches!(self.command, None | Some(Command::Run))
    }
}
