//! CLI command handling.
//!
//! Provides subcommands for:
//! - Running the agent (`run`)
//! - Interactive onboarding wizard (`onboard`)
//! - Managing configuration (`config list`, `config get`, `config set`)
//! - Managing WASM tools (`tool install`, `tool list`, `tool remove`)
//! - Managing MCP servers (`mcp add`, `mcp auth`, `mcp list`, `mcp test`)
//! - Querying workspace memory (`memory search`, `memory read`, `memory write`)
//! - Checking system health (`status`)

mod config;
mod mcp;
pub mod memory;
pub mod oauth_defaults;
mod pairing;
pub mod status;
mod tool;

pub use config::{ConfigCommand, run_config_command};
pub use mcp::{McpCommand, run_mcp_command};
pub use memory::MemoryCommand;
#[cfg(feature = "postgres")]
pub use memory::run_memory_command;
pub use memory::run_memory_command_with_db;
pub use pairing::{PairingCommand, run_pairing_command, run_pairing_command_with_store};
pub use status::run_status_command;
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

    /// Single message mode - send one message and exit
    #[arg(short, long, global = true)]
    pub message: Option<String>,

    /// Configuration file path (optional, uses env vars by default)
    #[arg(short, long, global = true)]
    pub config: Option<std::path::PathBuf>,

    /// Skip first-run onboarding check
    #[arg(long, global = true)]
    pub no_onboard: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the agent (default if no subcommand given)
    Run,

    /// Interactive onboarding wizard
    Onboard {
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

    /// Manage MCP servers (hosted tool providers)
    #[command(subcommand)]
    Mcp(McpCommand),

    /// Query and manage workspace memory
    #[command(subcommand)]
    Memory(MemoryCommand),

    /// DM pairing (approve inbound requests from unknown senders)
    #[command(subcommand)]
    Pairing(PairingCommand),

    /// Show system health and diagnostics
    Status,

    /// Run as a sandboxed worker inside a Docker container (internal use).
    /// This is invoked automatically by the orchestrator, not by users directly.
    Worker {
        /// Job ID to execute.
        #[arg(long)]
        job_id: uuid::Uuid,

        /// URL of the orchestrator's internal API.
        #[arg(long, default_value = "http://host.docker.internal:50051")]
        orchestrator_url: String,

        /// Maximum iterations before stopping.
        #[arg(long, default_value = "50")]
        max_iterations: u32,
    },

    /// Run as a Claude Code bridge inside a Docker container (internal use).
    /// Spawns the `claude` CLI and streams output back to the orchestrator.
    ClaudeBridge {
        /// Job ID to execute.
        #[arg(long)]
        job_id: uuid::Uuid,

        /// URL of the orchestrator's internal API.
        #[arg(long, default_value = "http://host.docker.internal:50051")]
        orchestrator_url: String,

        /// Maximum agentic turns for Claude Code.
        #[arg(long, default_value = "50")]
        max_turns: u32,

        /// Claude model to use (e.g. "sonnet", "opus").
        #[arg(long, default_value = "sonnet")]
        model: String,
    },
}

impl Cli {
    /// Check if we should run the agent (default behavior or explicit `run` command).
    pub fn should_run_agent(&self) -> bool {
        matches!(self.command, None | Some(Command::Run))
    }
}
